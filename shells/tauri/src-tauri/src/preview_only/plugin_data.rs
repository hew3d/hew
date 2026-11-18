//! What a running plugin leaves beside its package, as the desktop keeps
//! it (docs/agents/PLUGINS.md "Plugin data"): per-document storage, the
//! state each window's host reports, the activity log, the console, and
//! the two live API commands that read or reload. The files are
//! `plugin_host::data`'s; this file decides who may call it, how often,
//! and with what.
//!
//! - Every command checks the calling window's label first (Tauri supplies
//!   it, not the page), then the plugin id against the grammar, then the
//!   document id where there is one, before any path is built. A window
//!   label recorded in a state report or the activity log is the label
//!   Tauri gave the command, never one from the page's arguments.
//! - Every write refuses `not_installed` for a plugin with no record, under
//!   the store lock, so nothing is kept for a plugin that is gone, and an
//!   uninstall forgets what the hosts reported about it in the same hold.
//! - The shell is the only writer of the console and activity files.
//!   State reports are rate-bounded per window per plugin; the activity log
//!   is written at most once a second per plugin, batched; console lines
//!   are bounded per call and by a byte budget per plugin, and what is over
//!   it is refused, never queued.
//! - What the page sends is cleaned and bounded here again: the closed
//!   sets of states and levels, reasons and texts cut, times clamped to
//!   the shell's clock.
//! - The live API reads the activity log and can ask the hosts to reload a
//!   developer-folder plugin; it cannot start a plugin that stopped or is
//!   blocked, and nothing else about plugins can be done from the socket.

use super::plugins::{
    bus_refusal, check_id, check_window, install_error, lock, refusal, BusRefusal, DevChanged,
    PluginState, Ui,
};
use plugin_host::data::{
    self, ActivityEntry, ConsoleLine, CONSOLE_LEVELS, MAX_ACTIVITY_ENTRIES, PLUGIN_STATES,
};
use plugin_host::store::{self, DevMode};
use plugin_host::InstallError;
use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

/// The global event a state report that changed something emits, with
/// [`PluginEvent`]: a hint to read `plugin_list` again.
pub const STATE_EVENT: &str = "plugins-state";
/// The global event new console lines emit, with [`PluginEvent`]: a hint
/// to read `plugin_console_read` again. It carries no line.
pub const CONSOLE_EVENT: &str = "plugins-console";

/// How long one count of state reports runs, and how many one window may
/// send about one plugin in it.
pub const REPORT_WINDOW_MS: u64 = 10_000;
pub const MAX_REPORTS_PER_WINDOW: u32 = 30;
/// A `seq` further than this past the shell's own clock is not believed.
pub const MAX_SEQ_AHEAD_MS: u64 = 86_400_000;
/// How often, at most, a plugin's activity log is written.
pub const ACTIVITY_WRITE_INTERVAL_MS: u64 = 1_000;
/// The longest reason read from a report, in bytes, before it is cleaned.
const MAX_REASON_INPUT_BYTES: usize = 1024;
/// The most lines one `plugin_console_append` may carry.
pub const MAX_CONSOLE_LINES_PER_CALL: usize = data::MAX_CONSOLE_LINES_PER_APPEND;
/// The console bytes one plugin may write per [`CONSOLE_BUDGET_WINDOW_MS`],
/// across windows; lines past it are refused.
pub const CONSOLE_BYTES_PER_WINDOW: u64 = 256 * 1024;
pub const CONSOLE_BUDGET_WINDOW_MS: u64 = 60_000;
/// How far in the past a console line's time may be; an older or later
/// one takes the shell's clock.
const CONSOLE_AT_SLACK_MS: u64 = 10 * 60_000;
/// The console lines `plugin_console_read` answers when not told.
pub const DEFAULT_CONSOLE_READ: usize = 200;
/// The least time between two reloads of one plugin from the live API.
pub const RELOAD_INTERVAL_MS: u64 = 2_000;

/// [`STATE_EVENT`]'s and [`CONSOLE_EVENT`]'s payload.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct PluginEvent {
    pub id: String,
}

/// `plugins-storage-changed`'s payload for per-document storage:
/// `{id, scope: "document", document_id, keys}`. Plugin storage keeps its
/// own payload, `{id, keys}`, with no `scope`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct DocStorageChanged {
    pub id: String,
    pub scope: &'static str,
    pub document_id: String,
    pub keys: Vec<String>,
}

/// The latest report one window's host made about one plugin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub state: &'static str,
    pub reason: Option<String>,
    pub at: u64,
    /// The highest sequence number a host sent with a report for this
    /// window; a report with a lower one is older and is ignored.
    pub seq: Option<u64>,
}

/// Uses of a budget since a moment.
#[derive(Debug, Clone, Copy, Default)]
struct Budget {
    since: u64,
    used: u64,
}

impl Budget {
    /// Starts a new count once `window_ms` has passed, or once the clock
    /// stepped back by a whole window. A smaller step back counts in the
    /// current window: two calls can take their time and then reach the
    /// count out of order, and that must not start the count again.
    fn at(&mut self, now: u64, window_ms: u64) -> &mut Budget {
        if now.saturating_sub(self.since) >= window_ms
            || self.since.saturating_sub(now) >= window_ms
        {
            *self = Budget {
                since: now,
                used: 0,
            };
        }
        self
    }
}

/// Activity entries waiting to be written, and when the plugin's log was
/// last written.
#[derive(Debug, Default)]
struct Queued {
    entries: Vec<ActivityEntry>,
    written_at: Option<u64>,
}

/// The state behind this file's commands, kept in [`PluginState`].
#[derive(Debug, Default)]
pub struct Activity {
    /// The latest report, by plugin id and then window label.
    latest: Mutex<BTreeMap<String, BTreeMap<String, Report>>>,
    /// State reports counted per (plugin, window).
    reports: Mutex<HashMap<(String, String), Budget>>,
    /// Activity entries waiting for the writer, by plugin id.
    queue: Mutex<BTreeMap<String, Queued>>,
    /// Console bytes counted per plugin.
    console: Mutex<HashMap<String, Budget>>,
    /// When each plugin was last reloaded from the live API.
    reloads: Mutex<HashMap<String, u64>>,
    /// Set when the app exits: the writer thread stops.
    stopped: AtomicBool,
}

/// What `plugin_list` adds to each row about the plugin's hosts: in how
/// many open windows it runs (`activating` or `activated`), is stopped
/// (`crashed`), is blocked (`blocked`) or was unloaded for sitting idle
/// (`idle`, and starts again when it is needed), and the latest stop among
/// them.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Aggregate {
    pub running_in: usize,
    pub stopped_in: usize,
    pub blocked_in: usize,
    pub idle_in: usize,
    /// The newest `crashed` report still standing: its reason and time.
    pub last_stop: Option<(Option<String>, u64)>,
}

impl Aggregate {
    /// Adds `running_in`, `stopped_in`, `blocked_in`, `idle_in` and
    /// `last_stop` (`{reason, at}` or `null`) to a row.
    pub fn insert_into(&self, row: &mut serde_json::Map<String, serde_json::Value>) {
        row.insert("running_in".into(), self.running_in.into());
        row.insert("stopped_in".into(), self.stopped_in.into());
        row.insert("blocked_in".into(), self.blocked_in.into());
        row.insert("idle_in".into(), self.idle_in.into());
        row.insert(
            "last_stop".into(),
            match &self.last_stop {
                Some((reason, at)) => serde_json::json!({ "reason": reason, "at": at }),
                None => serde_json::Value::Null,
            },
        );
    }
}

/// What the hosts in windows `window_open` says are open report about
/// plugin `id`.
pub fn aggregate(state: &PluginState, id: &str, window_open: &dyn Fn(&str) -> bool) -> Aggregate {
    let latest = lock(&state.activity.latest);
    let mut out = Aggregate::default();
    for (label, report) in latest.get(id).into_iter().flatten() {
        if !window_open(label) {
            continue;
        }
        match report.state {
            "activating" | "activated" => out.running_in += 1,
            "crashed" => {
                out.stopped_in += 1;
                if out
                    .last_stop
                    .as_ref()
                    .is_none_or(|(_, at)| report.at >= *at)
                {
                    out.last_stop = Some((report.reason.clone(), report.at));
                }
            }
            "blocked" => out.blocked_in += 1,
            "idle" => out.idle_in += 1,
            _ => {}
        }
    }
    out
}

/// A window's host registered again (a page started): the sequence numbers
/// held for that window are dropped for every plugin, since a new page
/// counts from the start and the clock may have stepped back.
pub fn window_registered(state: &PluginState, label: &str) {
    let _store = state.store_lock();
    for windows in lock(&state.activity.latest).values_mut() {
        if let Some(r) = windows.get_mut(label) {
            r.seq = None;
        }
    }
}

/// A window closed: what its host reported goes with it, and every window
/// is told for each plugin it had reported about.
pub fn window_closed(state: &PluginState, label: &str, ui: &dyn Ui) {
    // Under the store lock, as a report is recorded: one that checked the
    // window was open lands before this clears it, never after.
    let _store = state.store_lock();
    let changed: Vec<String> = {
        let mut latest = lock(&state.activity.latest);
        let ids = latest
            .iter_mut()
            .filter_map(|(id, windows)| windows.remove(label).map(|_| id.clone()))
            .collect();
        latest.retain(|_, windows| !windows.is_empty());
        ids
    };
    lock(&state.activity.reports).retain(|(_, window), _| window != label);
    drop(_store);
    for id in changed {
        ui.state_changed(&id);
    }
}

/// Forgets everything held in memory about plugin `id`: its reports, its
/// unwritten activity, its budgets. Called by uninstall under the store
/// lock, after the plugin's files are gone.
pub fn forget(state: &PluginState, id: &str) {
    lock(&state.activity.latest).remove(id);
    lock(&state.activity.queue).remove(id);
    lock(&state.activity.reports).retain(|(plugin, _), _| plugin != id);
    lock(&state.activity.console).remove(id);
    lock(&state.activity.reloads).remove(id);
}

/// Refuses `not_installed` unless the plugin has a record. The caller holds
/// the store lock.
fn require_installed(dir: &store::PluginDir, id: &str) -> Result<(), String> {
    store::require_record(dir, id).map_err(install_error)
}

/// `text` cut to at most `max` bytes at a character boundary, before it is
/// cleaned, so a huge string costs no more than a short one.
fn cut(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

// ---------------------------------------------------------------------------
// Per-document storage
// ---------------------------------------------------------------------------

fn check_document(doc: &str) -> Result<(), String> {
    data::checked_document_id(doc).map_err(install_error)
}

/// `plugin_doc_storage_get`: the value plugin `id` saved under `key` for
/// document `doc`, if any. Refused in order: `window_not_allowed`, the id
/// grammar, `document_id_invalid`, `plugins_disabled` / `no_plugin_dir`,
/// `storage_key_invalid`, `not_installed`, `storage_unreadable`.
pub fn doc_storage_get(
    label: &str,
    state: &PluginState,
    id: &str,
    doc: &str,
    key: &str,
) -> Result<Option<String>, String> {
    check_window(label)?;
    check_id(id)?;
    check_document(doc)?;
    let dir = state.dir()?;
    let _store = state.store_lock();
    data::doc_storage_get(dir, id, doc, key).map_err(install_error)
}

/// `plugin_doc_storage_list`: every key plugin `id` saved for document
/// `doc`, sorted.
pub fn doc_storage_list(
    label: &str,
    state: &PluginState,
    id: &str,
    doc: &str,
) -> Result<Vec<String>, String> {
    check_window(label)?;
    check_id(id)?;
    check_document(doc)?;
    let dir = state.dir()?;
    let _store = state.store_lock();
    data::doc_storage_list(dir, id, doc).map_err(install_error)
}

/// `plugin_doc_storage_set`: saves `value` under `key` for document `doc`,
/// then tells every window ([`DocStorageChanged`]). Refused
/// `storage_quota` past the document's or the plugin's quota and
/// `storage_key_invalid` for an empty or overlong key, with nothing
/// written.
pub fn doc_storage_set(
    label: &str,
    state: &PluginState,
    id: &str,
    doc: &str,
    key: &str,
    value: &str,
    ui: &dyn Ui,
) -> Result<(), String> {
    check_window(label)?;
    check_id(id)?;
    check_document(doc)?;
    let dir = state.dir()?;
    {
        let _store = state.store_lock();
        data::doc_storage_set(dir, id, doc, key, value).map_err(install_error)?;
    }
    ui.doc_storage_changed(DocStorageChanged {
        id: id.to_string(),
        scope: "document",
        document_id: doc.to_string(),
        keys: vec![key.to_string()],
    });
    Ok(())
}

/// `plugin_doc_storage_delete`: removes `key` from document `doc`'s saved
/// data; tells every window only when it was there.
pub fn doc_storage_delete(
    label: &str,
    state: &PluginState,
    id: &str,
    doc: &str,
    key: &str,
    ui: &dyn Ui,
) -> Result<(), String> {
    check_window(label)?;
    check_id(id)?;
    check_document(doc)?;
    let dir = state.dir()?;
    let removed = {
        let _store = state.store_lock();
        data::doc_storage_delete(dir, id, doc, key).map_err(install_error)?
    };
    if removed {
        ui.doc_storage_changed(DocStorageChanged {
            id: id.to_string(),
            scope: "document",
            document_id: doc.to_string(),
            keys: vec![key.to_string()],
        });
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// State reports and activity
// ---------------------------------------------------------------------------

fn invalid_state() -> String {
    refusal(
        "invalid_state",
        "A plugin's state is one of enabled, activating, activated, crashed, blocked, idle or unloaded.",
    )
}

/// `plugin_report_state`: the calling window's host says what plugin `id`
/// is doing there. `label` is the window Tauri says called; the page has no
/// say in it. `state` must be one of [`PLUGIN_STATES`]; `reason` is cut and
/// cleaned ([`data::clean_reason`]). A report that changes the window's
/// latest (state or reason) is kept, queued for the activity log, and told
/// to every window ([`STATE_EVENT`]); the same report again changes
/// nothing and spends none of the budget.
///
/// `seq` is a number the host raises with every report it sends from one
/// page. Reports of one window can arrive out of order (each call runs on
/// its own thread), so one whose `seq` is lower than the highest held for
/// that window is ignored (`Ok`, nothing recorded). No `seq` is never
/// ignored. A report from a window `window_open` says is not open (it is
/// being destroyed) is ignored too, so nothing stale is left behind.
///
/// Refused in order: `window_not_allowed`, the id grammar,
/// `invalid_state`, `plugins_disabled` / `no_plugin_dir`, `not_installed`,
/// and `report_too_often` past [`MAX_REPORTS_PER_WINDOW`] reports in
/// [`REPORT_WINDOW_MS`] from one window about one plugin. A report that
/// changes the window's state to `crashed` or `blocked` from a state that
/// is not already one of those two is never refused for that: the first
/// stop must not be lost to the budget. A change from one stop to the other
/// is budgeted like any other, so one window cannot alternate them forever.
///
/// A `seq` more than [`MAX_SEQ_AHEAD_MS`] past `now` is treated as absent,
/// so a page cannot pin its window with one huge number.
///
/// Lock order, at every site: the store lock first; under it `latest`, and
/// while `latest` is held, `reports` and then `queue` (nested, in that
/// order). `window_closed` and `forget` take them one at a time, so they
/// never invert it.
#[allow(clippy::too_many_arguments)]
pub fn report_state_from(
    label: &str,
    state: &PluginState,
    id: &str,
    reported: &str,
    reason: Option<&str>,
    seq: Option<u64>,
    now: u64,
    window_open: &dyn Fn(&str) -> bool,
    ui: &dyn Ui,
) -> Result<(), String> {
    check_window(label)?;
    check_id(id)?;
    let reported = PLUGIN_STATES
        .iter()
        .copied()
        .find(|s| *s == reported)
        .ok_or_else(invalid_state)?;
    let reason = data::clean_reason(reason.map(|r| cut(r, MAX_REASON_INPUT_BYTES)));
    let dir = state.dir()?;
    let changed = {
        // Under the store lock, so an uninstall that forgets the plugin
        // cannot fall between the check and the record.
        let _store = state.store_lock();
        require_installed(dir, id)?;
        if !window_open(label) {
            return Ok(());
        }
        let seq = seq.filter(|s| *s <= now.saturating_add(MAX_SEQ_AHEAD_MS));
        let mut latest = lock(&state.activity.latest);
        let held = latest.get(id).and_then(|w| w.get(label));
        let held_terminal = held.is_some_and(|r| matches!(r.state, "crashed" | "blocked"));
        if let (Some(new), Some(old)) = (seq, held.and_then(|r| r.seq)) {
            if new < old {
                return Ok(());
            }
        }
        let held_seq = held.and_then(|r| r.seq);
        let held_changed = held.is_none_or(|r| r.state != reported || r.reason != reason);
        if !held_changed {
            // Nothing new: no budget spent, but the newest `seq` is kept.
            if let (Some(new), Some(r)) = (seq, latest.get_mut(id).and_then(|w| w.get_mut(label))) {
                r.seq = Some(new.max(r.seq.unwrap_or(0)));
            }
            return Ok(());
        }
        {
            let mut reports = lock(&state.activity.reports);
            let budget = reports
                .entry((id.to_string(), label.to_string()))
                .or_default()
                .at(now, REPORT_WINDOW_MS);
            let terminal = matches!(reported, "crashed" | "blocked");
            if budget.used >= u64::from(MAX_REPORTS_PER_WINDOW) {
                if !terminal || held_terminal {
                    return Err(refusal(
                        "report_too_often",
                        "This window reported this plugin's state too often. Try again shortly.",
                    ));
                }
            } else {
                budget.used += 1;
            }
        }
        let report = Report {
            state: reported,
            reason: reason.clone(),
            at: now,
            seq: seq.or(held_seq),
        };
        latest
            .entry(id.to_string())
            .or_default()
            .insert(label.to_string(), report);
        let mut queue = lock(&state.activity.queue);
        let queued = queue.entry(id.to_string()).or_default();
        queued.entries.push(ActivityEntry {
            at: now,
            window: label.to_string(),
            state: reported.to_string(),
            reason,
        });
        let over = queued.entries.len().saturating_sub(MAX_ACTIVITY_ENTRIES);
        queued.entries.drain(..over);
        true
    };
    if changed {
        ui.state_changed(id);
        flush_activity(state, now, false);
    }
    Ok(())
}

/// [`report_state_from`] with no sequence number, from a window that is
/// open.
#[cfg(test)]
pub fn report_state(
    label: &str,
    state: &PluginState,
    id: &str,
    reported: &str,
    reason: Option<&str>,
    now: u64,
    ui: &dyn Ui,
) -> Result<(), String> {
    report_state_from(label, state, id, reported, reason, None, now, &|_| true, ui)
}

/// Writes the queued activity of every plugin whose log was last written
/// at least [`ACTIVITY_WRITE_INTERVAL_MS`] ago (all of them with `force`).
/// What cannot be written is dropped, the batch whole: a plugin that is
/// gone, a tree that is refused, and also a write that fails only this
/// once. The log is a display history; the live state each window reported
/// stays in memory either way.
pub fn flush_activity(state: &PluginState, now: u64, force: bool) {
    let Ok(dir) = state.dir() else {
        return;
    };
    let is_due = |q: &Queued| {
        !q.entries.is_empty()
            && (force
                || q.written_at.is_none_or(|at| {
                    now.saturating_sub(at) >= ACTIVITY_WRITE_INTERVAL_MS || now < at
                }))
    };
    {
        // A look only, so the store lock is not taken every quarter second
        // for nothing. Nothing waits and the interval has passed: nothing
        // to remember.
        let mut queue = lock(&state.activity.queue);
        queue.retain(|_, q| {
            !q.entries.is_empty()
                || q.written_at
                    .is_some_and(|at| now.saturating_sub(at) < ACTIVITY_WRITE_INTERVAL_MS)
        });
        if !queue.values().any(is_due) {
            return;
        }
    }
    // The store lock is held across the drain and the write, so a read
    // (which holds it too) finds the entries in the queue or in the file,
    // never in neither. Lock order: the store lock, then the queue's.
    let _store = state.store_lock();
    let due: Vec<(String, Vec<ActivityEntry>)> = {
        let mut queue = lock(&state.activity.queue);
        queue
            .iter_mut()
            .filter(|(_, q)| is_due(q))
            .map(|(id, q)| {
                q.written_at = Some(now);
                (id.clone(), std::mem::take(&mut q.entries))
            })
            .collect()
    };
    for (id, entries) in due {
        let _ = data::activity_append(dir, &id, &entries);
    }
}

/// How many entries of plugin `id` wait in the queue.
#[cfg(test)]
pub fn queued_len(state: &PluginState, id: &str) -> usize {
    lock(&state.activity.queue)
        .get(id)
        .map_or(0, |q| q.entries.len())
}

/// The activity of plugin `id`, newest first: what is queued, then what
/// is written, at most [`MAX_ACTIVITY_ENTRIES`].
fn activity_entries(
    state: &PluginState,
    dir: &store::PluginDir,
    id: &str,
) -> Result<Vec<ActivityEntry>, InstallError> {
    let _store = state.store_lock();
    let written = data::activity_read(dir, id)?;
    let mut out: Vec<ActivityEntry> = lock(&state.activity.queue)
        .get(id)
        .map(|q| q.entries.iter().rev().cloned().collect())
        .unwrap_or_default();
    out.extend(written);
    out.truncate(MAX_ACTIVITY_ENTRIES);
    Ok(out)
}

/// `plugin_activity_read`: plugin `id`'s activity, newest first, at most
/// [`MAX_ACTIVITY_ENTRIES`]: `{at, window, state, reason}` each.
pub fn activity_read(
    label: &str,
    state: &PluginState,
    id: &str,
) -> Result<Vec<ActivityEntry>, String> {
    check_window(label)?;
    check_id(id)?;
    let dir = state.dir()?;
    activity_entries(state, dir, id).map_err(install_error)
}

/// Starts the thread that writes queued activity once its interval has
/// passed. Called once, from the app's setup.
pub fn start_activity_writer(app: &tauri::AppHandle) {
    use tauri::Manager;
    let handle = app.clone();
    let spawned = std::thread::Builder::new()
        .name("hew-plugin-activity".into())
        .spawn(move || loop {
            std::thread::sleep(std::time::Duration::from_millis(250));
            let Some(state) = handle.try_state::<PluginState>() else {
                return;
            };
            if state.activity.stopped.load(Ordering::Acquire) {
                return;
            }
            flush_activity(&state, super::plugins::now_ms(), false);
        });
    if let Err(e) = spawned {
        eprintln!("hew: the plugin activity writer did not start: {e}");
    }
}

/// Stops the writer and writes whatever is still queued. Called at app
/// exit.
pub fn stop_activity_writer(app: &tauri::AppHandle) {
    use tauri::Manager;
    if let Some(state) = app.try_state::<PluginState>() {
        state.activity.stopped.store(true, Ordering::Release);
        flush_activity(&state, super::plugins::now_ms(), true);
    }
}

// ---------------------------------------------------------------------------
// Console
// ---------------------------------------------------------------------------

/// One console line as the page sends it. `at` may be any number; it is
/// clamped to the shell's clock.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct ConsoleLineIn {
    pub level: String,
    pub text: String,
    pub at: f64,
}

/// What `plugin_console_append` did: how many lines it wrote and how many
/// it refused for being over the plugin's budget.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ConsoleAppended {
    pub accepted: usize,
    pub dropped: usize,
}

/// `plugin_console_append`: the calling window's host adds plugin `id`'s
/// console lines (it batches them, about once a second). Each level must be
/// one of [`CONSOLE_LEVELS`]; each text is cut and cleaned here; each time
/// is clamped to the last [`CONSOLE_AT_SLACK_MS`] of the shell's clock.
/// Lines past the plugin's budget ([`CONSOLE_BYTES_PER_WINDOW`] per
/// [`CONSOLE_BUDGET_WINDOW_MS`], across windows) are refused, not queued:
/// the answer counts them. Refused in order: `window_not_allowed`, the id
/// grammar, `too_many_lines` over [`MAX_CONSOLE_LINES_PER_CALL`],
/// `invalid_level`, `plugins_disabled` / `no_plugin_dir`, `not_installed`,
/// `console_rate_limited` when no line fits the budget, then the file's
/// refusals (`storage_quota`, `storage_unreadable`, `io`). Lines written
/// are told to every window ([`CONSOLE_EVENT`]).
pub fn console_append(
    label: &str,
    state: &PluginState,
    id: &str,
    lines: &[ConsoleLineIn],
    now: u64,
    ui: &dyn Ui,
) -> Result<ConsoleAppended, String> {
    check_window(label)?;
    check_id(id)?;
    if lines.len() > MAX_CONSOLE_LINES_PER_CALL {
        return Err(refusal(
            "too_many_lines",
            "A plugin's console lines are sent at most 64 at a time.",
        ));
    }
    if lines
        .iter()
        .any(|l| !CONSOLE_LEVELS.contains(&l.level.as_str()))
    {
        return Err(refusal(
            "invalid_level",
            "A console line's level is one of log, info, warn or error.",
        ));
    }
    let dir = state.dir()?;
    if lines.is_empty() {
        let _store = state.store_lock();
        require_installed(dir, id)?;
        return Ok(ConsoleAppended {
            accepted: 0,
            dropped: 0,
        });
    }
    let earliest = now.saturating_sub(CONSOLE_AT_SLACK_MS);
    let cleaned: Vec<ConsoleLine> = lines
        .iter()
        .map(|l| {
            let at = if l.at.is_finite() && l.at >= earliest as f64 && l.at <= now as f64 {
                l.at as u64
            } else {
                now
            };
            ConsoleLine {
                at,
                level: l.level.clone(),
                text: plugin_host::clean_text(
                    cut(&l.text, data::MAX_CONSOLE_TEXT_CHARS * 4),
                    data::MAX_CONSOLE_TEXT_CHARS,
                ),
            }
        })
        .collect();
    let _store = state.store_lock();
    require_installed(dir, id)?;
    let accepted: Vec<ConsoleLine> = {
        let mut budgets = lock(&state.activity.console);
        let budget = budgets
            .entry(id.to_string())
            .or_default()
            .at(now, CONSOLE_BUDGET_WINDOW_MS);
        cleaned
            .into_iter()
            .filter(|line| {
                // The line as the log stores it: its JSON and the newline.
                let bytes = serde_json::to_vec(line).map_or(u64::MAX, |json| json.len() as u64 + 1);
                if budget.used.saturating_add(bytes) > CONSOLE_BYTES_PER_WINDOW {
                    return false;
                }
                budget.used += bytes;
                true
            })
            .collect()
    };
    let dropped = lines.len() - accepted.len();
    if accepted.is_empty() {
        return Err(refusal(
            "console_rate_limited",
            "This plugin wrote too much to its console. Its lines are dropped for a while.",
        ));
    }
    let written = data::console_append(dir, id, &accepted).map_err(install_error)?;
    drop(_store);
    ui.console_changed(id);
    Ok(ConsoleAppended {
        accepted: written,
        dropped,
    })
}

/// `plugin_console_read`: plugin `id`'s newest console lines, newest first:
/// `limit` of them (default [`DEFAULT_CONSOLE_READ`], at most
/// [`data::MAX_CONSOLE_READ`]).
pub fn console_read(
    label: &str,
    state: &PluginState,
    id: &str,
    limit: Option<usize>,
) -> Result<Vec<ConsoleLine>, String> {
    check_window(label)?;
    check_id(id)?;
    let dir = state.dir()?;
    let _store = state.store_lock();
    data::console_read(dir, id, limit.unwrap_or(DEFAULT_CONSOLE_READ)).map_err(install_error)
}

// ---------------------------------------------------------------------------
// The live API
// ---------------------------------------------------------------------------

/// The plugin id a live API call names, or `invalid_params`.
fn bus_id<'a>(params: &'a serde_json::Value, method: &str) -> Result<&'a str, BusRefusal> {
    params
        .get("id")
        .and_then(serde_json::Value::as_str)
        .filter(|id| plugin_manifest::validate_plugin_id(id).is_ok())
        .ok_or_else(|| {
            BusRefusal::new(
                "invalid_params",
                &format!("{method} takes {{id}}, an installed plugin's id."),
            )
        })
}

/// A store refusal, for the live API.
fn bus_install_error(e: InstallError) -> BusRefusal {
    let name = match e {
        InstallError::NotInstalled(_) => "not_installed",
        // Its reason can name a path on this machine, which the live API
        // never gives out (`hew.plugin.list` says only `unreadable`).
        InstallError::InstalledUnreadable { .. } => {
            return BusRefusal::new(
                "installed_unreadable",
                "Hew cannot read this plugin's record. A newer Hew may have written it; it is left as it is.",
            )
        }
        InstallError::StorageUnreadable => "storage_unreadable",
        _ => "io",
    };
    BusRefusal::new(name, &e.explanation())
}

/// `hew.plugin.activity {id}` (live API, read-only): plugin `id`'s activity
/// as [`activity_read`] gives it, as `{entries: [{at, window, state,
/// reason}]}`, newest first. Refused `plugins_disabled` / `no_plugin_dir`,
/// `invalid_params` (no id, or not a plugin id), `not_installed`, and
/// `storage_unreadable` / `installed_unreadable` / `io` when the log or
/// the record cannot be read.
pub fn bus_activity(
    state: &PluginState,
    params: &serde_json::Value,
) -> Result<serde_json::Value, BusRefusal> {
    let dir = state.dir().map_err(bus_refusal)?;
    let id = bus_id(params, "hew.plugin.activity")?;
    let entries = activity_entries(state, dir, id).map_err(bus_install_error)?;
    Ok(serde_json::json!({ "entries": entries }))
}

/// `hew.plugin.reload {id}` (live API): asks every host to reload a
/// developer-folder plugin, answering the change event to send
/// (`plugins-dev-changed` with `manifest_changed: false`, which hosts
/// reload on). Refused, in order and changing nothing:
///
/// - `plugins_disabled` / `no_plugin_dir`; `invalid_params` (no plugin id);
/// - `plugins_off`: no open window runs a plugin host;
/// - `developer_mode_off`;
/// - `not_installed` (`installed_unreadable` for a record that cannot be
///   read); `not_a_developer_folder` for a plugin installed from a package;
/// - `plugin_blocked` while its folder's change waits for the person's
///   review, or while a host reports it blocked; `plugin_stopped` while a
///   host reports it crashed. A plugin that stopped stays stopped until a
///   person turns it off and on; a reload is not a way around that.
/// - `plugin_turned_off` while the person has it turned off: no host acts
///   on it, so nothing would reload.
/// - `reload_too_soon`: less than [`RELOAD_INTERVAL_MS`] since the last.
///
/// A rehearsal (`dry_run`) is refused by the router before this runs.
pub fn bus_reload(
    state: &PluginState,
    params: &serde_json::Value,
    now: u64,
    window_open: &dyn Fn(&str) -> bool,
) -> Result<DevChanged, BusRefusal> {
    let dir = state.dir().map_err(bus_refusal)?;
    let id = bus_id(params, "hew.plugin.reload")?;
    let any_host = {
        let mut hosts = lock(&state.hosts);
        hosts.retain(|label| window_open(label));
        !hosts.is_empty()
    };
    if !any_host {
        return Err(BusRefusal::new(
            "plugins_off",
            "Plugins are turned off in Hew, so no plugin is running to reload. Turn them on in Settings, then try again.",
        ));
    }
    if state.dev() == DevMode::Off {
        return Err(BusRefusal::new(
            "developer_mode_off",
            &InstallError::DeveloperModeOff.explanation(),
        ));
    }
    let record = {
        let _store = state.store_lock();
        store::installed(dir, id).map_err(bus_install_error)?.0
    };
    if !matches!(record.source, plugin_host::InstallSource::Dir { .. }) {
        return Err(BusRefusal::new(
            "not_a_developer_folder",
            "Only a plugin loaded from a developer folder can be reloaded this way.",
        ));
    }
    let blocked = || {
        BusRefusal::new(
            "plugin_blocked",
            "This plugin is waiting for the person to review a change in Settings > Plugins, so it is not reloaded.",
        )
    };
    if store::needs_consent(&record) {
        return Err(blocked());
    }
    let seen = aggregate(state, id, window_open);
    if seen.stopped_in > 0 {
        return Err(BusRefusal::new(
            "plugin_stopped",
            "This plugin stopped. It starts again only when the person turns it off and on in Settings > Plugins.",
        ));
    }
    if seen.blocked_in > 0 {
        return Err(blocked());
    }
    if !record.enabled {
        // No host acts on a plugin that is turned off, so nothing would
        // reload, and the two-second slot would be spent for nothing. After
        // the blocked checks, so the documented order holds.
        return Err(BusRefusal::new(
            "plugin_turned_off",
            "This plugin is turned off. It runs again only when the person turns it on in Settings > Plugins.",
        ));
    }
    {
        let mut reloads = lock(&state.activity.reloads);
        if let Some(last) = reloads.get(id) {
            if now.saturating_sub(*last) < RELOAD_INTERVAL_MS && now >= *last {
                return Err(BusRefusal::new(
                    "reload_too_soon",
                    "This plugin was reloaded a moment ago. Try again in two seconds.",
                ));
            }
        }
        reloads.insert(id.to_string(), now);
    }
    Ok(DevChanged {
        id: id.to_string(),
        manifest_changed: false,
        reason: None,
        problem: None,
    })
}

// ---------------------------------------------------------------------------
// The Tauri commands
// ---------------------------------------------------------------------------

use super::plugins::off_main;

#[tauri::command]
pub async fn plugin_doc_storage_get(
    window: tauri::WebviewWindow,
    id: String,
    doc_id: String,
    key: String,
) -> Result<Option<String>, String> {
    off_main(window, move |label, state, _| {
        doc_storage_get(label, state, &id, &doc_id, &key)
    })
    .await
}

#[tauri::command]
pub async fn plugin_doc_storage_set(
    window: tauri::WebviewWindow,
    id: String,
    doc_id: String,
    key: String,
    value: String,
) -> Result<(), String> {
    off_main(window, move |label, state, ui| {
        doc_storage_set(label, state, &id, &doc_id, &key, &value, ui)
    })
    .await
}

#[tauri::command]
pub async fn plugin_doc_storage_delete(
    window: tauri::WebviewWindow,
    id: String,
    doc_id: String,
    key: String,
) -> Result<(), String> {
    off_main(window, move |label, state, ui| {
        doc_storage_delete(label, state, &id, &doc_id, &key, ui)
    })
    .await
}

#[tauri::command]
pub async fn plugin_doc_storage_list(
    window: tauri::WebviewWindow,
    id: String,
    doc_id: String,
) -> Result<Vec<String>, String> {
    off_main(window, move |label, state, _| {
        doc_storage_list(label, state, &id, &doc_id)
    })
    .await
}

/// The window label is the calling window's, from Tauri; the page passes
/// only the plugin, the state, the reason and a sequence number that rises
/// with each report from the page.
#[tauri::command]
pub async fn plugin_report_state(
    window: tauri::WebviewWindow,
    id: String,
    state: String,
    reason: Option<String>,
    seq: Option<u64>,
) -> Result<(), String> {
    use tauri::Manager;
    let now = super::plugins::now_ms();
    let app = window.app_handle().clone();
    off_main(window, move |label, plugins, ui| {
        let open = |l: &str| app.get_webview_window(l).is_some();
        report_state_from(
            label,
            plugins,
            &id,
            &state,
            reason.as_deref(),
            seq,
            now,
            &open,
            ui,
        )
    })
    .await
}

#[tauri::command]
pub async fn plugin_activity_read(
    window: tauri::WebviewWindow,
    id: String,
) -> Result<Vec<ActivityEntry>, String> {
    off_main(window, move |label, state, _| {
        activity_read(label, state, &id)
    })
    .await
}

#[tauri::command]
pub async fn plugin_console_append(
    window: tauri::WebviewWindow,
    id: String,
    lines: Vec<ConsoleLineIn>,
) -> Result<ConsoleAppended, String> {
    let now = super::plugins::now_ms();
    off_main(window, move |label, state, ui| {
        console_append(label, state, &id, &lines, now, ui)
    })
    .await
}

#[tauri::command]
pub async fn plugin_console_read(
    window: tauri::WebviewWindow,
    id: String,
    limit: Option<usize>,
) -> Result<Vec<ConsoleLine>, String> {
    off_main(window, move |label, state, _| {
        console_read(label, state, &id, limit)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::Budget;

    #[test]
    fn a_budget_counts_a_small_step_back_in_its_window_and_starts_again_after_a_whole_one() {
        let mut b = Budget::default();
        b.at(1_005, 1_000).used = 7;
        assert_eq!(b.at(1_000, 1_000).used, 7, "a call that reached it late");
        assert_eq!(b.since, 1_005);
        assert_eq!(b.at(2_004, 1_000).used, 7, "still within the window");
        assert_eq!(b.at(2_005, 1_000).used, 0, "the window passed");
        b.used = 3;
        assert_eq!(b.at(1_006, 1_000).used, 3, "less than a window back");
        assert_eq!(b.at(1_005, 1_000).used, 0, "a whole window back");
        assert_eq!(b.since, 1_005);
    }
}

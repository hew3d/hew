//! The desktop's plugin commands (docs/agents/PLUGINS.md "Installing"):
//! the Tauri commands Settings ▸ Plugins calls to pick, stage, confirm,
//! list, change and remove installed plugins, and the four a running
//! plugin's host calls to keep the plugin's saved data
//! (`plugins/<id>/storage.json`). The store itself is `plugin_host::store`;
//! this file decides who may call it and with what.
//!
//! These commands are the only thing between a web page and installing
//! other people's code, so every argument from the page is treated as
//! hostile:
//!
//! - Every command first checks the calling window's label (`main`,
//!   `main-<n>` or `settings`), which Tauri supplies, not the page. The
//!   Library window and any future plugin window are refused.
//! - Every plugin id is checked against the plugin id grammar before it is
//!   used for anything, the store folder included.
//! - A package or folder is staged only from a path the user picked in this
//!   run's native picker, recorded here in a set of its own and consumed on
//!   use. `ApprovedPaths` is not consulted: drag-drop and neighbor-folder
//!   approval widen it, and neither is a choice to install.
//! - The page only ever sees `{token, consent}`; the package stays here.
//!   Accepting shows a native confirmation whose text is built from the
//!   consent payload this side holds, so a page cannot install anything the
//!   user was not shown by Hew itself. Widening a grant, rolling back,
//!   turning a plugin on and uninstalling ask the same way, one dialog at a
//!   time ([`super::confirm`]: only the accept button says yes), and re-read
//!   the store after the answer so what is written is what was shown.
//!   Turning a plugin off and narrowing a grant ask nothing.
//! - Saved data is only ever a value in one JSON object: a key is never a
//!   path, and the file is found from the checked id alone.
//! - A picked path is matched by exact string against what the picker
//!   returned; the page's string is never resolved on the filesystem.
//! - Plugin code is read from a folder only while developer mode is on.
//!   Developer mode is this side's state, kept in a small file in the app's
//!   config folder; turning it on asks natively, and anything in that file
//!   but the exact "on" form reads as off. With it off, picking, staging,
//!   reading, watching and reviewing a folder are all refused
//!   `developer_mode_off`, for folders installed earlier too.
//! - A folder plugin's folder is watched ([`super::dev_watch`]). A change
//!   that leaves its permissions as they were reloads it; any other change
//!   to what its manifest asks for marks its record `needs_consent`, after
//!   which nothing is read from the folder until the person reviews the
//!   change ([`review`]) and accepts it in the native confirmation.
//! - A folder plugin's manifest is also compared with the stored one
//!   whenever the plugin starts being watched (the app starts, developer
//!   mode turns on, the plugin is turned on, installed, reviewed or rolled
//!   back), so an edit made while nobody watched is never the starting
//!   point ([`compare_folders`]).
//! - The live API can only ask: `hew.plugin.dev` records one request and
//!   shows it in every window ([`bus_dev`]); it never picks, stages or
//!   reads. The folder picker it leads to is the person's own pick, opened
//!   for that request by its id. While no window runs a plugin host (the
//!   Plugins switch is off) it is refused `plugins_off`; while one request
//!   waits another is refused `request_waiting`; after Dismiss the socket
//!   is refused `request_dismissed` for a while.
//!
//! Each command is a thin wrapper over a plain function that takes the
//! window label, the state, the arguments and a [`Ui`] (the native dialog
//! and the change event), so the rules are tested without a running app.

use super::dev_watch::{DevWatch, WatchEvent, Watching};
use plugin_host::store::{
    self, DevMode, FolderOutcome, Listed, Pending, PluginDir, PENDING_TTL_MS,
};
use plugin_host::{ConsentPayload, InstallError, InstallSource, Installed};
use plugin_manifest::PermissionName;
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::Duration;
use tauri::{Emitter, Manager};

/// The global event every command that changes the store emits.
pub const CHANGED_EVENT: &str = "plugins-changed";
/// The global event a change to a plugin's saved data emits, with
/// [`StorageChanged`], so every window's host can tell that plugin.
pub const STORAGE_CHANGED_EVENT: &str = "plugins-storage-changed";
/// The global event a change in a folder plugin's folder emits, with
/// [`DevChanged`].
pub const DEV_CHANGED_EVENT: &str = "plugins-dev-changed";
/// The global event that tells every window the live API's waiting
/// request: [`DevRequested`] when one arrives, `null` when it is taken,
/// dismissed or dropped.
pub const DEV_REQUEST_EVENT: &str = "plugins-dev-request";
/// How long a command waits for folder reads it does itself (the manifest
/// comparison when a plugin starts being watched). What does not finish in
/// time is left to the watcher, which has its own limit.
pub const FOLDER_READ_BUDGET: Duration = Duration::from_secs(2);
/// Reads the `hew-plugin.json` of the folder recorded at a path.
type ManifestReader = fn(&Path) -> Result<Vec<u8>, InstallError>;

/// How long Dismiss keeps the live API from asking again.
pub const REQUEST_QUIET_MS: u64 = 60_000;
/// The file in the app's config folder that keeps developer mode.
pub const DEV_MODE_FILE: &str = "plugin-developer-mode";
/// The longest path a live API request may carry, in bytes, before it is
/// refused; and the most characters of it the page is shown.
pub const MAX_REQUEST_PATH_BYTES: usize = 4096;
pub const MAX_REQUEST_PATH_CHARS: usize = 512;
/// The package file extension, compared without regard to case.
pub const PACKAGE_EXTENSION: &str = "hewplugin";
/// How many staged installs may wait for an answer at once. Each holds a
/// whole package in memory, so a page cannot pile them up.
pub const MAX_PENDING: usize = 4;
/// How many unstaged picks are remembered; the oldest is forgotten first.
pub const MAX_PICKS: usize = 8;

/// What a native picker returned: a package file or an unpacked folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PickKind {
    Package,
    Folder,
}

/// The commands' state, managed by Tauri in a preview build.
pub struct PluginState {
    /// The store's root, or `None` when no data folder could be found.
    dir: Option<PluginDir>,
    /// Paths the user picked in a native picker and has not yet staged,
    /// oldest first.
    picked: Mutex<VecDeque<Pick>>,
    /// Set while a native confirmation is open, so a page cannot stack
    /// dialogs behind the one the user is reading.
    asking: AtomicBool,
    /// Staged installs waiting for the user's answer, by token.
    pending: Mutex<HashMap<String, Pending>>,
    /// Staged installs that came from a live API request's picker, by
    /// token: when staged, and whether the person's native answer is being
    /// asked now (the install has then left `pending`). While one is
    /// live, the request counts as still waiting ([`bus_list`]).
    request_flows: Mutex<HashMap<String, (u64, bool)>>,
    /// Held around every change to the store, so two commands never
    /// rearrange the same folders at once. Never held while a dialog shows.
    store: Mutex<()>,
    /// Why every command is refused, when the build is not one that can
    /// run plugins; `None` when it can.
    disabled: Option<&'static str>,
    /// Developer mode: whether plugin code may be read from a folder.
    dev_mode: AtomicBool,
    /// Where developer mode is kept, when a config folder was found.
    dev_mode_file: Option<PathBuf>,
    /// Bumped each time developer mode goes off, so a folder read or a
    /// stage that began before can tell, after it, that it must be dropped.
    dev_epoch: AtomicU64,
    /// The one live API request to load a folder, and the quiet after a
    /// Dismiss.
    dev_request: Mutex<RequestSlot>,
    /// The windows whose plugin host is running (the Plugins switch is on
    /// there), by label.
    pub(crate) hosts: Mutex<BTreeSet<String>>,
    /// What the hosts reported about running plugins, the activity and
    /// console writers' state, and the live API's reload clock
    /// ([`super::plugin_data`]).
    pub(crate) activity: super::plugin_data::Activity,
    /// The ids watched as of the last sync. Its lock is held while the
    /// watched set is decided and handed to the watcher, so a sync that saw
    /// developer mode on cannot hand its set over after one that saw it off.
    watched: Mutex<BTreeSet<String>>,
    /// The folder plugins whose folder manifest has been compared with the
    /// stored one since they last started being watched. Changed and read
    /// only under the store lock. A folder plugin outside it is unchecked:
    /// every read of its folder is refused, so nothing runs uncompared.
    /// Developer mode going off empties it, and a plugin leaves it when it
    /// stops being watched or a command rewrites its record.
    checked: Mutex<BTreeSet<String>>,
    /// Unchecked folder plugins whose folder a read was refused: when their
    /// comparison completes the host is told to try again.
    refused_unchecked: Mutex<BTreeSet<String>>,
    /// How a folder manifest is read for the comparison, and how long a
    /// command waits for it ([`FOLDER_READ_BUDGET`]); tests stand in.
    manifest_reader: ManifestReader,
    read_budget: Duration,
    /// The folder watcher.
    pub watch: DevWatch,
    /// Set once the page has listed plugins or read developer mode, which
    /// it does only while its Plugins switch is on. Until then nothing is
    /// watched, so with the switch off no folder is read and the store is
    /// never written by the watcher.
    armed: AtomicBool,
}

/// A live API request to load a plugin from a folder: its random id, the
/// folder the picker may open near (only a plain local absolute path that
/// cleaning left as it was; never resolved here), the path as the page
/// shows it (cleaned and cut, as the event carries it), and when it came.
#[derive(Debug, Clone)]
struct DevRequest {
    id: String,
    start: Option<PathBuf>,
    shown: String,
    created_at_ms: u64,
}

/// The waiting request, and until when the live API is refused after the
/// person dismissed one.
#[derive(Debug, Default)]
struct RequestSlot {
    current: Option<DevRequest>,
    quiet_until_ms: u64,
}

impl RequestSlot {
    /// The waiting request, dropped once it is older than
    /// [`PENDING_TTL_MS`].
    fn fresh(&mut self, now_ms: u64) -> Option<&DevRequest> {
        if self
            .current
            .as_ref()
            .is_some_and(|r| now_ms.saturating_sub(r.created_at_ms) > PENDING_TTL_MS)
        {
            self.current = None;
        }
        self.current.as_ref()
    }
}

impl PluginState {
    pub fn new(dir: Option<PluginDir>) -> PluginState {
        PluginState {
            dir,
            picked: Mutex::new(VecDeque::new()),
            asking: AtomicBool::new(false),
            pending: Mutex::new(HashMap::new()),
            request_flows: Mutex::new(HashMap::new()),
            store: Mutex::new(()),
            disabled: None,
            dev_mode: AtomicBool::new(false),
            dev_mode_file: None,
            dev_epoch: AtomicU64::new(0),
            dev_request: Mutex::new(RequestSlot::default()),
            hosts: Mutex::new(BTreeSet::new()),
            activity: super::plugin_data::Activity::default(),
            watched: Mutex::new(BTreeSet::new()),
            checked: Mutex::new(BTreeSet::new()),
            refused_unchecked: Mutex::new(BTreeSet::new()),
            manifest_reader: store::folder_manifest,
            read_budget: FOLDER_READ_BUDGET,
            watch: DevWatch::new(super::dev_watch::TIMING),
            armed: AtomicBool::new(false),
        }
    }

    /// The state with developer mode read from `file` (see
    /// [`read_dev_mode`]) and kept there.
    pub fn with_dev_mode_file(mut self, file: Option<PathBuf>) -> PluginState {
        let on = file.as_deref().is_some_and(read_dev_mode);
        self.dev_mode = AtomicBool::new(on);
        self.dev_mode_file = file;
        self
    }

    /// Developer mode as the store takes it.
    pub fn dev(&self) -> DevMode {
        if self.dev_mode.load(Ordering::Acquire) {
            DevMode::On
        } else {
            DevMode::Off
        }
    }

    /// The state for the running app: the store under the app's data
    /// folder, unless the content security policy this shell was started
    /// with does not allow the realm's boot script (`boot_hash`, the
    /// base64 SHA-256 the realm frame is pinned to). A shell built without
    /// the preview policy (the Tauri CLI run directly instead of through
    /// `scripts/tauri-policy.mjs`) would otherwise offer plugins whose
    /// realm can never start; it refuses every command and says why.
    pub fn resolve(csp: Option<&str>, boot_hash: &str, config_dir: Option<&Path>) -> PluginState {
        if policy_allows_boot(csp, boot_hash) {
            return PluginState::new(PluginDir::resolve())
                .with_dev_mode_file(config_dir.map(|d| d.join(DEV_MODE_FILE)));
        }
        eprintln!(
            "hew: ERROR: the content security policy does not allow the plugin realm's boot script; plugins are disabled. Build a preview shell with `pnpm --dir shells/tauri build` (or dev), not the Tauri CLI directly."
        );
        let mut state = PluginState::new(None);
        state.disabled = Some(
            "This build of Hew was not given the policy plugins need, so plugins are turned off.",
        );
        state
    }

    pub(crate) fn dir(&self) -> Result<&PluginDir, String> {
        if let Some(why) = self.disabled {
            return Err(refusal("plugins_disabled", why));
        }
        self.dir.as_ref().ok_or_else(|| {
            refusal(
                "no_plugin_dir",
                "Hew could not find a folder to keep plugins in.",
            )
        })
    }

    pub(crate) fn store_lock(&self) -> MutexGuard<'_, ()> {
        lock(&self.store)
    }
}

/// Whether `csp`'s `script-src` allows the script whose base64 SHA-256 is
/// `boot_hash`.
pub fn policy_allows_boot(csp: Option<&str>, boot_hash: &str) -> bool {
    let source = format!("'sha256-{boot_hash}'");
    !boot_hash.is_empty()
        && csp.is_some_and(|csp| {
            csp.split(';')
                .map(str::trim)
                .filter(|d| d.starts_with("script-src "))
                .any(|d| d.split_whitespace().any(|s| s == source))
        })
}

/// A lock that a panic elsewhere does not turn into a refusal forever. The
/// data behind these locks stays consistent across a panic: each is
/// changed by a single insert or remove.
pub(crate) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// What a staged install shows the page: its token and the consent payload.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PendingView {
    pub token: String,
    pub consent: ConsentPayload,
}

/// [`DEV_CHANGED_EVENT`]'s payload. `manifest_changed: false` with no
/// `problem`: the folder's files changed and its permissions did not (the
/// stored manifest copy was refreshed if its other fields changed); the
/// host reloads the plugin. `manifest_changed: true`: the record is now
/// marked `needs_consent` for `reason`; the host stops the plugin and the
/// pane offers Review. `problem`: the folder cannot be watched right now,
/// named by a refusal (`folder_missing`, `package_path_invalid`,
/// `package_too_large`); nothing else changed.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct DevChanged {
    pub id: String,
    pub manifest_changed: bool,
    pub reason: Option<String>,
    pub problem: Option<String>,
}

/// [`DEV_REQUEST_EVENT`]'s payload and `plugin_dev_request`'s answer: the
/// request's id (the page hands it back to open the picker for it or to
/// dismiss it) and the requested path, cleaned and cut to
/// [`MAX_REQUEST_PATH_CHARS`]. The path is for the page to show, nothing
/// more.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct DevRequested {
    pub id: String,
    pub path: String,
}

/// [`STORAGE_CHANGED_EVENT`]'s payload: the plugin and the keys that
/// changed.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct StorageChanged {
    pub id: String,
    pub keys: Vec<String>,
}

/// The native side of a command: the confirmation dialog and the change
/// events. The app implements it with [`super::confirm::confirm_native`]
/// and global events; tests stub it.
pub trait Ui {
    /// Shows a native confirmation and blocks until it is answered. True
    /// only when the user activated the button labeled `accept`.
    fn confirm(&self, title: &str, body: &str, accept: &str) -> bool;
    /// Tells every window the store changed.
    fn changed(&self);
    /// Tells every window a plugin's saved data changed.
    fn storage_changed(&self, change: StorageChanged);
    /// Tells every window a folder plugin's folder changed.
    fn dev_changed(&self, change: DevChanged);
    /// Tells every window the live API's waiting request: a new one, or
    /// `None` when it was taken, dismissed or dropped.
    fn dev_request(&self, _request: Option<DevRequested>) {}
    /// Tells every window a plugin's per-document saved data changed.
    fn doc_storage_changed(&self, _change: super::plugin_data::DocStorageChanged) {}
    /// Tells every window what the hosts reported about a plugin changed.
    fn state_changed(&self, _id: &str) {}
    /// Tells every window a plugin's console has new lines.
    fn console_changed(&self, _id: &str) {}
}

pub(crate) fn refusal(name: &str, text: &str) -> String {
    format!("{name}: {text}")
}

pub(crate) fn install_error(e: InstallError) -> String {
    format!("{}: {}", e.name(), e.explanation())
}

/// [`install_error`] for a failure reading a plugin's folder: the words say
/// what is true of a folder (fix the file, it is read again), not of a
/// package. Chosen here, at the folder call sites, and nowhere else.
fn folder_error(e: InstallError) -> String {
    format!("{}: {}", e.name(), e.explanation_for_folder())
}

/// Milliseconds since the Unix epoch, the clock pending installs expire by.
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// This build's version, the one a manifest's `hew` range is checked
/// against: the release tag's version when the release workflow stamped
/// one (a preview build's package version is still the last release's),
/// else the package's own ([`crate::version::VERSION`]). A stamp that is
/// not a semantic version falls back to the package's version rather than
/// taking the app down at the first plugin check.
fn running_version() -> &'static plugin_host::semver::Version {
    static RUNNING: OnceLock<plugin_host::semver::Version> = OnceLock::new();
    RUNNING.get_or_init(|| {
        plugin_host::semver::Version::parse(crate::version::VERSION).unwrap_or_else(|_| {
            plugin_host::semver::Version::parse(crate::version::PACKAGE)
                .expect("the package version is a semantic version")
        })
    })
}

/// Whether a window may manage plugins: a document window (`main`,
/// `main-<n>`) or Settings. Everything else is refused, the Library window
/// and any window a plugin may one day get included.
pub fn window_allowed(label: &str) -> bool {
    label == "main"
        || label == "settings"
        || label
            .strip_prefix("main-")
            .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

pub(crate) fn check_window(label: &str) -> Result<(), String> {
    if window_allowed(label) {
        Ok(())
    } else {
        Err(refusal(
            "window_not_allowed",
            "Plugins can be managed only from a model window or Settings.",
        ))
    }
}

pub(crate) fn check_id(id: &str) -> Result<(), String> {
    plugin_manifest::validate_plugin_id(id).map_err(|e| install_error(InstallError::Manifest(e)))
}

fn developer_mode_off() -> String {
    install_error(InstallError::DeveloperModeOff)
}

/// Refuses `developer_mode_off` unless developer mode is on.
fn require_dev(state: &PluginState) -> Result<(), String> {
    match state.dev() {
        DevMode::On => Ok(()),
        DevMode::Off => Err(developer_mode_off()),
    }
}

/// Whether developer mode's file says on: a regular file (not a link) of
/// at most a few bytes holding exactly `on`, with or without one line
/// break. Anything else, a damaged or missing file included, is off.
pub fn read_dev_mode(path: &Path) -> bool {
    let regular = std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_file());
    if !regular {
        return false;
    }
    let mut bytes = Vec::new();
    let read = std::fs::File::open(path).and_then(|f| f.take(16).read_to_end(&mut bytes));
    read.is_ok() && matches!(bytes.as_slice(), b"on" | b"on\n")
}

/// Keeps developer mode off in `path`: written as off, else removed (no
/// file reads as off), else refused `developer_mode_not_saved`.
fn keep_dev_mode_off(path: &Path) -> Result<(), String> {
    let Err(write) = write_dev_mode(path, false) else {
        return Ok(());
    };
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => {
            eprintln!(
                "hew: developer mode is off, but it could not be saved ({write}; removing the file: {e})"
            );
            Err(refusal(
                "developer_mode_not_saved",
                "Developer mode is off for now, but Hew could not save that, so it may be on again the next time Hew starts.",
            ))
        }
    }
}

/// Keeps developer mode in `path`, through a temporary file and a rename.
fn write_dev_mode(path: &Path, on: bool) -> Result<(), String> {
    let io = |e: std::io::Error| install_error(InstallError::Io(e.to_string()));
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(io)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, if on { "on\n" } else { "off\n" }).map_err(io)?;
    std::fs::rename(&tmp, path).map_err(io)
}

/// What should be watched now: once the page has started using plugins
/// ([`arm_watch`]) and with developer mode on, every enabled folder plugin,
/// marked ones included (comparing a marked plugin's folder is what clears
/// its mark when the change is undone); otherwise nothing.
fn watch_targets(state: &PluginState) -> BTreeMap<String, PathBuf> {
    let mut targets = BTreeMap::new();
    if state.armed.load(Ordering::Acquire) && state.dev() == DevMode::On && state.disabled.is_none()
    {
        if let Some(dir) = &state.dir {
            let rows = {
                let _store = state.store_lock();
                store::list(dir)
            };
            for row in rows {
                if let Listed::Installed { installed, .. } = row {
                    if let InstallSource::Dir { path } = &installed.source {
                        if installed.enabled {
                            targets.insert(installed.id.clone(), PathBuf::from(path));
                        }
                    }
                }
            }
        }
    }
    targets
}

/// Points the watcher at what it should watch now ([`watch_targets`]);
/// nothing stops the watcher's thread. Every folder plugin that was not
/// watched at the last sync, and is not in `settled` (compared already by
/// the command that let it in), has its manifest compared with the stored
/// one first ([`compare_folders`]), so its first listing is never an
/// unexamined starting point; what that found is returned for the caller
/// to tell the windows. The decision and the hand-over happen under one
/// lock, and turning developer mode off stores the mode before it takes
/// that lock, so a sync can never start watching after developer mode went
/// off. Never called with the store lock held.
fn sync_watch_settling(state: &PluginState, settled: &[&str]) -> Settled {
    let mut watched = lock(&state.watched);
    let targets = watch_targets(state);
    let entering: Vec<String> = {
        // Leaving the watched set leaves the checked set: coming back is a
        // new entry, compared again before anything is read. Anything
        // still unchecked is compared again too. Before the page has
        // started using plugins nothing is watched yet, and what a command
        // compared stays checked.
        let _store = state.store_lock();
        let mut checked = lock(&state.checked);
        if state.armed.load(Ordering::Acquire) {
            checked.retain(|id| targets.contains_key(id));
        }
        targets
            .keys()
            .filter(|id| {
                (!watched.contains(*id) || !checked.contains(*id))
                    && !settled.contains(&id.as_str())
            })
            .cloned()
            .collect()
    };
    let found = compare_folders(state, &entering, true);
    *watched = targets.keys().cloned().collect();
    let unchecked: Vec<String> = {
        let _store = state.store_lock();
        let checked = lock(&state.checked);
        targets
            .keys()
            .filter(|id| !checked.contains(*id))
            .cloned()
            .collect()
    };
    state.watch.watch(targets);
    // The watcher tries again, every pass, until one comparison completes.
    state.watch.recheck(&unchecked);
    found
}

/// [`sync_watch_settling`], telling the windows through `ui` what the
/// comparison found.
pub fn sync_watch(state: &PluginState, ui: &dyn Ui) {
    let found = sync_watch_settling(state, &[]);
    found.tell(ui);
}

/// Lets the watcher run from now on: the first time the page lists plugins
/// or reads developer mode. The folders it starts to watch are compared
/// before this returns, so the list the page reads next carries any mark.
fn arm_watch(state: &PluginState, ui: &dyn Ui) {
    if !state.armed.swap(true, Ordering::AcqRel) {
        sync_watch(state, ui);
    }
}

/// After a command changed the store: watch what should be watched now
/// (comparing what starts being watched), then tell every window, the
/// command's own comparison (`found`, for the plugins in `settled`)
/// included.
fn after_change_with(state: &PluginState, ui: &dyn Ui, mut found: Settled, settled: &[&str]) {
    found.merge(sync_watch_settling(state, settled));
    found.changed = true;
    found.tell(ui);
}

/// After a command changed the store: tell every window, and watch what
/// should be watched now.
fn after_change(state: &PluginState, ui: &dyn Ui) {
    after_change_with(state, ui, Settled::default(), &[]);
}

/// What comparing folder manifests found, for the windows: whether the
/// store changed (`plugins-changed`) and the `plugins-dev-changed` events,
/// in order.
#[derive(Debug, Default)]
struct Settled {
    changed: bool,
    events: Vec<DevChanged>,
}

impl Settled {
    fn merge(&mut self, other: Settled) {
        self.changed |= other.changed;
        self.events.extend(other.events);
    }

    /// `plugins-changed` first, so a host reads the store as it is now,
    /// then each folder event.
    fn tell(self, ui: &dyn Ui) {
        if self.changed {
            ui.changed();
        }
        for e in self.events {
            ui.dev_changed(e);
        }
    }

    /// Notes one comparison. `entering`: the plugin is starting to be
    /// watched, so an unchanged manifest says nothing (no reload of what
    /// has not started) and a problem is left to the watcher, which reports
    /// it once.
    /// A comparison that completed (any outcome but a refusal) makes the
    /// plugin checked; if a read of its folder was refused while it was
    /// not, the host is told to try again. Called under the store lock.
    fn note(
        &mut self,
        state: &PluginState,
        dir: &PluginDir,
        id: &str,
        outcome: Result<FolderOutcome, InstallError>,
        entering: bool,
    ) {
        if outcome.is_ok() {
            lock(&state.checked).insert(id.to_string());
            let marked = store::installed(dir, id).map_or(true, |(r, _)| store::needs_consent(&r));
            if lock(&state.refused_unchecked).remove(id)
                && !marked
                && matches!(outcome, Ok(FolderOutcome::Unchanged))
            {
                self.events.push(DevChanged {
                    id: id.to_string(),
                    manifest_changed: false,
                    reason: None,
                    problem: None,
                });
                return;
            }
        }
        let event =
            |manifest_changed: bool, reason: Option<&str>, problem: Option<&str>| DevChanged {
                id: id.to_string(),
                manifest_changed,
                reason: reason.map(str::to_string),
                problem: problem.map(str::to_string),
            };
        match outcome {
            Ok(FolderOutcome::Unchanged) => {
                // Files changed under a plugin that may run: reload it. A
                // plugin still marked does not run; nothing to say.
                let marked =
                    store::installed(dir, id).map_or(true, |(r, _)| store::needs_consent(&r));
                if !entering && !marked {
                    self.events.push(event(false, None, None));
                }
            }
            Ok(FolderOutcome::Refreshed | FolderOutcome::Cleared) => {
                self.changed = true;
                self.events.push(event(false, None, None));
            }
            Ok(FolderOutcome::Marked(reason)) => {
                self.changed = true;
                self.events.push(event(true, Some(reason), None));
            }
            Err(e) => {
                if !entering {
                    self.events.push(event(false, None, Some(e.name())));
                }
            }
        }
    }
}

/// Runs `read` for each item on a thread of its own and collects what
/// finished within `budget`; the rest are let go (their answers are
/// dropped), so a folder that stopped answering holds nothing up for
/// longer than that.
fn read_bounded<T, R>(
    items: Vec<(String, T)>,
    budget: Duration,
    read: fn(&T) -> R,
) -> Vec<(String, R)>
where
    T: Send + 'static,
    R: Send + 'static,
{
    let (tx, rx) = std::sync::mpsc::channel();
    let mut asked = 0usize;
    for (id, item) in items {
        let tx = tx.clone();
        let spawned = std::thread::Builder::new()
            .name("hew-plugin-folder-read".to_string())
            .spawn(move || {
                let _ = tx.send((id, read(&item)));
            });
        if spawned.is_ok() {
            asked += 1;
        }
    }
    drop(tx);
    let deadline = std::time::Instant::now() + budget;
    let mut out = Vec::new();
    while out.len() < asked {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        match rx.recv_timeout(left) {
            Ok(answer) => out.push(answer),
            Err(_) => break,
        }
    }
    out
}

/// Whether developer mode is on and has not gone off since `epoch` was
/// read: a folder read that began before must be dropped otherwise.
fn dev_still_on(state: &PluginState, epoch: u64) -> bool {
    state.dev() == DevMode::On && state.dev_epoch.load(Ordering::Acquire) == epoch
}

/// Compares the folder manifests of plugins `ids` with the stored ones,
/// acting on each exactly as on a change in its folder: refreshed or
/// cleared, the host reloads it; marked, the host stops it. The records
/// are read under the store lock, the manifests without it (each read
/// bounded by [`FOLDER_READ_BUDGET`]; one that does not finish is left to
/// the watcher, whose first listing of a folder compares it again), and
/// [`store::apply_folder_manifest`] decides under the lock, only while
/// developer mode is still on.
fn compare_folders(state: &PluginState, ids: &[String], entering: bool) -> Settled {
    let mut found = Settled::default();
    let Ok(dir) = state.dir() else {
        return found;
    };
    if ids.is_empty() {
        return found;
    }
    let epoch = state.dev_epoch.load(Ordering::Acquire);
    let reader = state.manifest_reader;
    let sources: Vec<(String, (PathBuf, ManifestReader))> = {
        let _store = state.store_lock();
        ids.iter()
            .filter_map(|id| match store::folder_of(dir, id, state.dev()) {
                Ok(Some(source)) => Some((id.clone(), (source.root().to_path_buf(), reader))),
                _ => None,
            })
            .collect()
    };
    let reads = read_bounded(
        sources,
        state.read_budget,
        |(root, read): &(PathBuf, ManifestReader)| read(root),
    );
    let _store = state.store_lock();
    if !dev_still_on(state, epoch) {
        return found;
    }
    for (id, read) in reads {
        let outcome = store::apply_folder_manifest(dir, &id, read, running_version());
        found.note(state, dir, &id, outcome, entering);
    }
    found
}

/// The manifest of the folder recorded at `path`, read now (bounded) so a
/// command can compare it with the stored one under the same hold of the
/// store lock as its own write. `None` while developer mode is off, or
/// when the read did not finish in time (the comparison is then left to
/// the sync after the command and to the watcher).
fn read_ahead(state: &PluginState, path: &str) -> Option<Result<Vec<u8>, InstallError>> {
    if state.dev() == DevMode::Off {
        return None;
    }
    read_bounded(
        vec![(String::new(), (PathBuf::from(path), state.manifest_reader))],
        state.read_budget,
        |(root, read): &(PathBuf, ManifestReader)| read(root),
    )
    .pop()
    .map(|(_, read)| read)
}

/// Inside a command's hold of the store lock, after its write: compares
/// what [`read_ahead`] read with the record now in place. Answers whether
/// it was compared.
fn compare_ahead(
    state: &PluginState,
    dir: &PluginDir,
    id: &str,
    ahead: Option<Result<Vec<u8>, InstallError>>,
    epoch: u64,
    found: &mut Settled,
) -> bool {
    // Whatever the record was checked against is gone: it is unchecked
    // until this comparison, or a later one, completes.
    lock(&state.checked).remove(id);
    let Some(read) = ahead else {
        return false;
    };
    if !dev_still_on(state, epoch) {
        return false;
    }
    let outcome = store::apply_folder_manifest(dir, id, read, running_version());
    found.note(state, dir, id, outcome, true);
    true
}

fn not_picked() -> String {
    refusal(
        "path_not_picked",
        "Choose the plugin again with the Install button.",
    )
}

/// One remembered pick: the canonical path as the pick command returned it
/// to the page, the path itself, and what kind of picker produced it.
#[derive(Debug, Clone)]
pub struct Pick {
    shown: String,
    path: PathBuf,
    kind: PickKind,
    /// The live API request whose picker produced this pick. It is used up
    /// only when the folder stages; a cancel or a refusal leaves it waiting.
    request: Option<String>,
}

/// Records a path the user picked in a native picker, canonical, and
/// returns it as the string the page must hand back. A package must be a
/// file and a folder a folder. Past [`MAX_PICKS`] the oldest is forgotten.
pub fn record_pick(state: &PluginState, path: &Path, kind: PickKind) -> Result<String, String> {
    record_pick_for(state, path, kind, None)
}

/// [`record_pick`] for a pick that came from the picker a live API request
/// opened (`request` is its id).
fn record_pick_for(
    state: &PluginState,
    path: &Path,
    kind: PickKind,
    request: Option<String>,
) -> Result<String, String> {
    let canonical = std::fs::canonicalize(path).map_err(|_| not_picked())?;
    let right_kind = match kind {
        PickKind::Package => canonical.is_file(),
        PickKind::Folder => canonical.is_dir(),
    };
    if !right_kind {
        return Err(not_picked());
    }
    // A folder install records its path as text and reads it back by that
    // text, so a path that is not text could never be found again; and the
    // path is shown in Hew's own dialogs, so one holding a line break, a
    // control character or one that turns text around is refused too.
    if kind == PickKind::Folder && !plain_path(&canonical) {
        return Err(refusal(
            "folder_path_invalid",
            "Hew can't use this folder because its name, or the name of a folder it is in, has characters Hew can't use. Rename it, then choose it again.",
        ));
    }
    let shown = canonical.to_string_lossy().into_owned();
    remember_pick(
        &mut lock(&state.picked),
        Pick {
            shown: shown.clone(),
            path: canonical,
            kind,
            request,
        },
    );
    Ok(shown)
}

/// Whether a folder's path is text with no control character and none that
/// [`plugin_host::clean_text`] removes (direction changes, zero-width and
/// the like).
fn plain_path(path: &Path) -> bool {
    path.to_str().is_some_and(|text| {
        !text.chars().any(|c| {
            c.is_control()
                || plugin_host::CLEAN_TEXT_REMOVED
                    .iter()
                    .any(|&(lo, hi)| (lo..=hi).contains(&u32::from(c)))
        })
    })
}

/// Adds a pick, replacing an equal one and forgetting the oldest past the
/// cap.
fn remember_pick(picks: &mut VecDeque<Pick>, pick: Pick) {
    picks.retain(|p| !(p.shown == pick.shown && p.kind == pick.kind));
    picks.push_back(pick);
    while picks.len() > MAX_PICKS {
        picks.pop_front();
    }
}

/// Removes and returns the pick whose string is exactly `path`, of `kind`.
/// Pure: the page's string is only compared, never resolved, so a path
/// that was not picked costs no filesystem work at all.
fn take_from(
    picks: &mut VecDeque<Pick>,
    path: &str,
    kind: PickKind,
) -> Option<(PathBuf, Option<String>)> {
    let at = picks
        .iter()
        .position(|p| p.shown == path && p.kind == kind)?;
    picks.remove(at).map(|p| (p.path, p.request))
}

/// Removes and returns a picked path of the given kind; anything not
/// picked, or picked as the other kind, is `path_not_picked`.
fn take_pick(state: &PluginState, path: &str, kind: PickKind) -> Result<PathBuf, String> {
    take_pick_with_request(state, path, kind).map(|(path, _)| path)
}

/// [`take_pick`], and the id of the request whose picker made the pick.
fn take_pick_with_request(
    state: &PluginState,
    path: &str,
    kind: PickKind,
) -> Result<(PathBuf, Option<String>), String> {
    take_from(&mut lock(&state.picked), path, kind).ok_or_else(not_picked)
}

/// Marks a native confirmation as open until dropped, a panic included.
struct Asking<'a>(&'a AtomicBool);

impl Drop for Asking<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

/// Claims the one native confirmation, or refuses `confirmation_open`
/// while another is showing.
fn begin_asking(state: &PluginState) -> Result<Asking<'_>, String> {
    state
        .asking
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .map(|_| Asking(&state.asking))
        .map_err(|_| {
            refusal(
                "confirmation_open",
                "Answer the question Hew is already asking first.",
            )
        })
}

fn not_a_package() -> String {
    refusal(
        "not_a_package",
        "Hew installs plugins from .hewplugin files.",
    )
}

/// Reads a package file of at most the package size cap, measured on the
/// bytes read.
fn read_package_file(path: &Path) -> Result<Vec<u8>, String> {
    let file =
        std::fs::File::open(path).map_err(|e| install_error(InstallError::Io(e.to_string())))?;
    let is_file = file
        .metadata()
        .map_err(|e| install_error(InstallError::Io(e.to_string())))?
        .is_file();
    if !is_file {
        return Err(not_a_package());
    }
    let cap = plugin_host::MAX_PACKAGE_BYTES;
    let mut bytes = Vec::new();
    file.take(cap as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| install_error(InstallError::Io(e.to_string())))?;
    if bytes.len() > cap {
        return Err(install_error(InstallError::PackageTooLarge));
    }
    Ok(bytes)
}

/// Drops every pending install older than the time limit.
fn prune(pending: &mut HashMap<String, Pending>, now_ms: u64) {
    pending.retain(|_, p| now_ms.saturating_sub(p.created_at_ms) <= PENDING_TTL_MS);
}

/// Checks a read package against the store and holds it for an answer. A
/// newer stage of the same plugin replaces any older one still waiting.
/// `epoch` is developer mode's epoch when the stage began (see
/// [`hold_staged`]).
fn hold(
    state: &PluginState,
    package: plugin_host::Package,
    source: InstallSource,
    now_ms: u64,
    epoch: u64,
) -> Result<PendingView, String> {
    let dir = state.dir()?;
    let staged = {
        let _store = state.store_lock();
        store::stage(dir, package, source, running_version(), now_ms).map_err(install_error)?
    };
    hold_staged(state, staged, now_ms, epoch)
}

/// Holds a staged install for an answer, in place of any older one of the
/// same plugin. A folder stage (an install or a review) is refused
/// `developer_mode_off` when developer mode is off now or went off since
/// `epoch`, the moment the stage began: it is checked under the lock that
/// turning developer mode off takes, after storing the mode, to drop the
/// folder stages, so a stage that read its folder while the mode went off
/// is never left holding a slot.
fn hold_staged(
    state: &PluginState,
    staged: Pending,
    now_ms: u64,
    epoch: u64,
) -> Result<PendingView, String> {
    let mut pending = lock(&state.pending);
    if matches!(staged.source, InstallSource::Dir { .. }) && !dev_still_on(state, epoch) {
        return Err(developer_mode_off());
    }
    prune(&mut pending, now_ms);
    pending.retain(|_, p| p.consent.id != staged.consent.id);
    if pending.len() >= MAX_PENDING {
        return Err(refusal(
            "too_many_pending",
            "Too many installs are waiting for an answer. Finish or cancel one first.",
        ));
    }
    let view = PendingView {
        token: staged.token.clone(),
        consent: staged.consent.clone(),
    };
    pending.insert(staged.token.clone(), staged);
    Ok(view)
}

/// `plugin_list`: every plugin in the store. The first list arms the
/// watcher, and the folders it starts to watch are compared before the
/// rows are read, so a folder edited while Hew was closed is already
/// marked in what this answers.
pub fn list(label: &str, state: &PluginState, ui: &dyn Ui) -> Result<Vec<Listed>, String> {
    check_window(label)?;
    let dir = state.dir()?;
    arm_watch(state, ui);
    // Under the store lock, so a change in another window is seen whole or
    // not at all, never half way through its renames. No command holds the
    // lock while a dialog is open, so this waits only for a rearrangement.
    let _store = state.store_lock();
    Ok(store::list(dir))
}

/// The folder problems the page shows (the watcher's refusal names).
const DEV_PROBLEMS: &[&str] = &[
    "folder_missing",
    "package_path_invalid",
    "package_too_large",
];

/// The `dev_problem` of an enabled folder plugin whose folder has not been
/// compared yet (and the watcher has no other problem with it): nothing of
/// it is read until that comparison completes.
pub const FOLDER_NOT_CHECKED: &str = "folder_not_checked";

/// The `dev_problem` of an enabled folder plugin whose stored manifest names
/// a `main` that is not a regular file in its folder now.
pub const MAIN_MISSING: &str = "main_missing";

/// `plugin_list`'s rows: [`list`], each folder plugin's row with
/// `dev_problem`, the problem the watcher has with its folder now
/// (`folder_missing`, `package_path_invalid`, `package_too_large`), else
/// [`MAIN_MISSING`] when the stored manifest's main file is not in the
/// folder, else [`FOLDER_NOT_CHECKED`] while it is enabled, developer mode is on and
/// its folder has not been compared yet, else `null`, so a window that
/// opens after the watcher reported it still shows it. Read from the
/// watcher's state without waiting for a walk. Every row also carries what
/// the windows' hosts reported ([`super::plugin_data::Aggregate`]):
/// `running_in`, `stopped_in`, `blocked_in`, `idle_in` and `last_stop`.
pub fn list_rows(
    label: &str,
    state: &PluginState,
    ui: &dyn Ui,
) -> Result<Vec<serde_json::Value>, String> {
    let rows = list(label, state, ui)?;
    let problems = state.watch.problems();
    let checked = {
        let _store = state.store_lock();
        lock(&state.checked).clone()
    };
    let dev = state.dev();
    rows.into_iter()
        .map(|row| {
            let folder = match &row {
                Listed::Installed {
                    installed,
                    manifest,
                    ..
                } => match &installed.source {
                    InstallSource::Dir { path } => Some((
                        installed.id.clone(),
                        installed.enabled,
                        // Whether the stored manifest's main file is there:
                        // names only, nothing opened.
                        dev == DevMode::On
                            && installed.enabled
                            && !store::needs_consent(installed)
                            && store::checked_root(Path::new(path)).is_ok()
                            && !store::jailed_file_exists(Path::new(path), &manifest.main),
                    )),
                    _ => None,
                },
                Listed::Unreadable { .. } => None,
            };
            let mut value = serde_json::to_value(&row)
                .map_err(|e| install_error(InstallError::Io(e.to_string())))?;
            let row_id = match &row {
                Listed::Installed { installed, .. } => installed.id.as_str(),
                Listed::Unreadable { id, .. } => id.as_str(),
            };
            if let Some(object) = value.as_object_mut() {
                super::plugin_data::aggregate(state, row_id, &|_| true).insert_into(object);
            }
            if let (Some((id, enabled, main_missing)), Some(object)) =
                (folder, value.as_object_mut())
            {
                let problem = problems
                    .get(&id)
                    .copied()
                    .filter(|p| DEV_PROBLEMS.contains(p))
                    .or_else(|| main_missing.then_some(MAIN_MISSING))
                    .or_else(|| {
                        (enabled && dev == DevMode::On && !checked.contains(&id))
                            .then_some(FOLDER_NOT_CHECKED)
                    });
                object.insert("dev_problem".to_string(), serde_json::json!(problem));
            }
            Ok(value)
        })
        .collect()
}

/// `plugin_stage_file`: reads a picked `.hewplugin` and holds it.
pub fn stage_file(
    label: &str,
    state: &PluginState,
    path: &str,
    now_ms: u64,
) -> Result<PendingView, String> {
    check_window(label)?;
    let path = take_pick(state, path, PickKind::Package)?;
    let is_package = path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case(PACKAGE_EXTENSION));
    if !is_package {
        return Err(not_a_package());
    }
    state.dir()?;
    let bytes = read_package_file(&path)?;
    let package = plugin_host::read_package_zip(&bytes).map_err(install_error)?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    hold(
        state,
        package,
        InstallSource::File { name },
        now_ms,
        state.dev_epoch.load(Ordering::Acquire),
    )
}

/// `plugin_stage_dir`: reads a picked unpacked plugin folder and holds it.
/// The command itself is [`stage_dir_for_request`].
#[cfg(test)]
pub fn stage_dir(
    label: &str,
    state: &PluginState,
    path: &str,
    now_ms: u64,
) -> Result<PendingView, String> {
    stage_dir_inner(label, state, path, now_ms).map(|(view, _)| view)
}

/// [`stage_dir`] for the command: a folder picked from a live API request's
/// picker that stages uses that request up, and every window is told. A
/// refusal leaves the request waiting.
pub fn stage_dir_for_request(
    label: &str,
    state: &PluginState,
    path: &str,
    now_ms: u64,
    ui: &dyn Ui,
) -> Result<PendingView, String> {
    let (view, request) = stage_dir_inner(label, state, path, now_ms)?;
    if request.is_some() {
        lock(&state.request_flows).insert(view.token.clone(), (now_ms, false));
    }
    if let Some(id) = request {
        let took = {
            let mut slot = lock(&state.dev_request);
            if slot.fresh(now_ms).is_some_and(|r| r.id == id) {
                slot.current = None;
                true
            } else {
                false
            }
        };
        if took {
            ui.dev_request(None);
        }
    }
    Ok(view)
}

fn stage_dir_inner(
    label: &str,
    state: &PluginState,
    path: &str,
    now_ms: u64,
) -> Result<(PendingView, Option<String>), String> {
    check_window(label)?;
    state.dir()?;
    let epoch = state.dev_epoch.load(Ordering::Acquire);
    require_dev(state)?;
    let (path, request) = take_pick_with_request(state, path, PickKind::Folder)?;
    state.dir()?;
    // `record_pick` refused a folder path that is not text.
    let Some(text) = path.to_str() else {
        return Err(not_picked());
    };
    let source = InstallSource::Dir {
        path: text.to_string(),
    };
    let package = store::read_package_dir(&path).map_err(folder_error)?;
    hold(state, package, source, now_ms, epoch).map(|view| (view, request))
}

/// Ends a request's flow when the commit that answers its staged install
/// returns, by any road. A refusal that leaves the install waiting (another
/// dialog open) keeps it.
struct RequestFlowEnd<'a> {
    state: &'a PluginState,
    token: &'a str,
}

impl Drop for RequestFlowEnd<'_> {
    fn drop(&mut self) {
        let waiting = lock(&self.state.pending).contains_key(self.token);
        if !waiting {
            lock(&self.state.request_flows).remove(self.token);
        }
    }
}

/// `plugin_commit`: `accept: false` drops the pending install. `accept:
/// true` takes it out of the waiting set, asks the user natively with the
/// consent this side holds, and installs only on yes; a no returns `None`.
/// Either way the token cannot be used again.
pub fn commit(
    label: &str,
    state: &PluginState,
    token: &str,
    accept: bool,
    ui: &dyn Ui,
    now_ms: u64,
) -> Result<Option<Installed>, String> {
    check_window(label)?;
    let _flow = RequestFlowEnd { state, token };
    // Claimed before the pending install is taken, so a refusal here leaves
    // it waiting.
    let asking = if accept {
        Some(begin_asking(state)?)
    } else {
        None
    };
    let taken = {
        let mut pending = lock(&state.pending);
        let taken = pending.remove(token);
        prune(&mut pending, now_ms);
        taken
    };
    let Some(taken) = taken else {
        return Err(install_error(InstallError::PendingUnknown));
    };
    if !accept {
        return Ok(None);
    }
    if let Some(flow) = lock(&state.request_flows).get_mut(token) {
        flow.1 = true;
    }
    if now_ms.saturating_sub(taken.created_at_ms) > PENDING_TTL_MS {
        return Err(install_error(InstallError::PendingExpired));
    }
    let dir = state.dir()?;
    // A folder is installed, and a review accepted, only while developer
    // mode is on: checked before asking and again after the answer.
    let folder = matches!(taken.source, InstallSource::Dir { .. });
    if folder {
        require_dev(state)?;
    }
    let review = taken.consent.review;
    let (title, accept_label) = if review {
        plugin_host::review_confirmation()
    } else {
        plugin_host::install_confirmation(&taken.consent)
    };
    let body = plugin_host::consent_summary(&taken.consent);
    let yes = ui.confirm(title, &body, accept_label);
    drop(asking);
    if !yes {
        return Ok(None);
    }
    let id = taken.consent.id.clone();
    // The folder may have changed since it was staged: its manifest is
    // read now and compared with what was just accepted under the same
    // hold of the store lock as the write, so the record never stands
    // unmarked over a folder that asks for more.
    let epoch = state.dev_epoch.load(Ordering::Acquire);
    let ahead = match &taken.source {
        InstallSource::Dir { path } => read_ahead(state, path),
        _ => None,
    };
    let mut found = Settled::default();
    // The store refuses `pending_stale` if the installed record changed
    // since staging; that error returns here before anything is emitted.
    let (installed, compared) = {
        let _store = state.store_lock();
        if folder {
            require_dev(state)?;
        }
        let installed = if review {
            Some(store::commit_review(dir, taken, now_ms).map_err(install_error)?)
        } else {
            store::commit(dir, taken, true, now_ms).map_err(install_error)?
        };
        let compared = compare_ahead(state, dir, &id, ahead, epoch, &mut found);
        (installed, compared)
    };
    // The marker is gone, unless the folder changed again: the host starts
    // the plugin again.
    if review && !found.events.iter().any(|e| e.manifest_changed) {
        found.events.retain(|e| e.id != id);
        found.events.push(DevChanged {
            id: id.clone(),
            manifest_changed: false,
            reason: None,
            problem: None,
        });
    }
    let settled: &[&str] = if compared { &[id.as_str()] } else { &[] };
    after_change_with(state, ui, found, settled);
    Ok(installed)
}

/// `plugin_dev_mode`: whether developer mode is on.
pub fn dev_mode(label: &str, state: &PluginState, ui: &dyn Ui) -> Result<bool, String> {
    check_window(label)?;
    state.dir()?;
    arm_watch(state, ui);
    Ok(state.dev() == DevMode::On)
}

/// `plugin_set_dev_mode`: turns developer mode on or off and answers what
/// it is now. Turning it on asks natively first (one dialog at a time); a
/// no is `developer_mode_declined` and leaves it off. It is kept in its
/// file before it takes effect, so a failed write leaves it off; once on,
/// every folder plugin that starts being watched has its manifest compared
/// with the stored one before the windows are told. Turning it off asks
/// nothing and takes effect at once: every folder stops being read and
/// watched, staged folder installs and any live API request are dropped.
/// Then it is kept: written as off, or, if that fails, the file is removed
/// (no file reads as off); if that fails too, it stays off for this run
/// and the answer is `developer_mode_not_saved`, so the person knows it may
/// be on again at the next start. Either way every window is told.
pub fn set_dev_mode(
    label: &str,
    state: &PluginState,
    on: bool,
    ui: &dyn Ui,
) -> Result<bool, String> {
    check_window(label)?;
    state.dir()?;
    if !on {
        state.dev_mode.store(false, Ordering::Release);
        state.dev_epoch.fetch_add(1, Ordering::AcqRel);
        {
            // Every folder is unchecked again: turning it back on compares
            // before anything is read.
            let _store = state.store_lock();
            lock(&state.checked).clear();
        }
        if lock(&state.dev_request).current.take().is_some() {
            ui.dev_request(None);
        }
        lock(&state.pending).retain(|_, p| !matches!(p.source, InstallSource::Dir { .. }));
        after_change(state, ui);
        if let Some(file) = &state.dev_mode_file {
            keep_dev_mode_off(file)?;
        }
        return Ok(false);
    }
    if state.dev() == DevMode::On {
        return Ok(true);
    }
    let yes = {
        let _asking = begin_asking(state)?;
        ui.confirm(
            "Turn on developer mode?",
            "Hew will run plugin code straight from a folder, where the files can change at any time. Turn this on only if you are writing a plugin.",
            "Turn On",
        )
    };
    if !yes {
        return Err(refusal(
            "developer_mode_declined",
            "Developer mode was left off.",
        ));
    }
    if let Some(file) = &state.dev_mode_file {
        write_dev_mode(file, true)?;
    }
    state.dev_mode.store(true, Ordering::Release);
    after_change(state, ui);
    Ok(true)
}

/// `plugin_dev_review`: stages the review of a folder plugin whose manifest
/// changed, from the folder recorded at install (no new pick). The page
/// shows the consent sheet; `plugin_commit` then asks natively with the
/// review's own title and, on yes, replaces the stored manifest and grant
/// in place and clears the marker. Shown whatever the version. Refused
/// `developer_mode_off`, `no_review_pending`, `folder_id_changed`, and as
/// a read of the folder or an install would be.
pub fn review(
    label: &str,
    state: &PluginState,
    id: &str,
    now_ms: u64,
) -> Result<PendingView, String> {
    check_window(label)?;
    check_id(id)?;
    let dir = state.dir()?;
    let epoch = state.dev_epoch.load(Ordering::Acquire);
    require_dev(state)?;
    // The folder is read without the store lock held, then staged under
    // it against the record as it is then.
    let (package, source) = store::read_review(dir, id, state.dev()).map_err(folder_error)?;
    let staged = {
        let _store = state.store_lock();
        require_dev(state)?;
        store::stage_review(dir, package, source, running_version(), now_ms)
            .map_err(install_error)?
    };
    hold_staged(state, staged, now_ms, epoch)
}

/// The watcher's handler: a problem is passed on as it is; a folder's
/// first listing and a change are settled by [`folder_changed`]. Answers
/// what the watcher does next: keep watching, let go, or (while the
/// plugin is still unchecked, its comparison not completed) report its
/// entry again on the next pass.
pub fn on_watch(state: &PluginState, event: WatchEvent, ui: &dyn Ui) -> Watching {
    match event {
        WatchEvent::Problem { id, refusal } => {
            if state.dev() == DevMode::Off {
                return Watching::Drop;
            }
            let checked = is_checked(state, &id);
            ui.dev_changed(DevChanged {
                id,
                manifest_changed: false,
                reason: None,
                problem: Some(refusal.to_string()),
            });
            if checked {
                Watching::Keep
            } else {
                Watching::Again
            }
        }
        WatchEvent::Entered { id } => settle_folder(state, &id, ui, true),
        WatchEvent::Changed { id } => settle_folder(state, &id, ui, false),
    }
}

/// Whether folder plugin `id` has been compared since it started being
/// watched (see `PluginState::checked`).
fn is_checked(state: &PluginState, id: &str) -> bool {
    let _store = state.store_lock();
    lock(&state.checked).contains(id)
}

/// Settles a change in folder plugin `id`'s folder: its `hew-plugin.json`
/// is read through the same reader as every read of the folder (the record
/// checked under the store lock, the file read without it and bounded),
/// then [`store::apply_folder_manifest`] under the lock decides: unchanged
/// or refreshed, the host reloads it (`manifest_changed: false`); marked,
/// the host stops it (`manifest_changed: true`); a mark the folder no
/// longer earns is cleared and the host starts it again. A marked plugin
/// stays watched, so undoing the change clears the mark. The stored
/// manifest's permissions and the grant never change here. Answers
/// whether to keep watching it.
#[cfg(test)]
pub fn folder_changed(state: &PluginState, id: &str, ui: &dyn Ui) -> bool {
    settle_folder(state, id, ui, false) != Watching::Drop
}

/// [`folder_changed`]; with `entering`, for a folder's first listing,
/// where an unchanged manifest says nothing.
fn settle_folder(state: &PluginState, id: &str, ui: &dyn Ui, entering: bool) -> Watching {
    let Ok(dir) = state.dir() else {
        return Watching::Drop;
    };
    let source = {
        let _store = state.store_lock();
        store::folder_of(dir, id, state.dev())
    };
    // Off, uninstalled, or not a folder: nothing to settle.
    if !matches!(source, Ok(Some(_))) {
        return Watching::Drop;
    }
    let found = compare_folders(state, &[id.to_string()], entering);
    if state.dev() == DevMode::Off {
        return Watching::Drop;
    }
    found.tell(ui);
    if is_checked(state, id) {
        Watching::Keep
    } else {
        Watching::Again
    }
}

/// A refusal for the live API: its name and one sentence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BusRefusal {
    pub name: &'static str,
    pub explanation: String,
}

impl BusRefusal {
    pub(crate) fn new(name: &'static str, explanation: &str) -> BusRefusal {
        BusRefusal {
            name,
            explanation: explanation.to_string(),
        }
    }
}

/// The refusal a command string carries, for the live API.
pub(crate) fn bus_refusal(e: String) -> BusRefusal {
    let name = match e.split(':').next().unwrap_or("") {
        "plugins_disabled" => "plugins_disabled",
        "no_plugin_dir" => "no_plugin_dir",
        _ => "io",
    };
    let explanation = e
        .split_once(": ")
        .map_or(e.as_str(), |(_, t)| t)
        .to_string();
    BusRefusal { name, explanation }
}

/// `hew.plugin.list` (live API, read-only): every installed plugin as
/// `{id, name, version, source, enabled, state}` and whether developer
/// mode is on. `name` is the cleaned display name; `source` the source
/// kind (`file`, `dir`, …); `state` what this side can know:
/// `needs_developer_mode` (a folder plugin while developer mode is off),
/// `needs_review` (marked `needs_consent`), `unreadable`, `checking` (an
/// enabled folder plugin whose folder has not been compared yet, so
/// nothing of it is read), `plugins_off` (the same, while no window runs a
/// plugin host, so nothing will compare it), else `installed`. Also
/// `request_waiting`: whether a live API request waits for the person
/// (no path, no id). Whether a plugin is running is the page's to know
/// and is not here. No path is given out.
pub fn bus_list(
    state: &PluginState,
    now_ms: u64,
    window_open: &dyn Fn(&str) -> bool,
) -> Result<serde_json::Value, BusRefusal> {
    let dir = state.dir().map_err(bus_refusal)?;
    let dev = state.dev();
    let hosts = lock(&state.hosts).iter().any(|label| window_open(label));
    // A request is waiting from the moment it is accepted until the flow it
    // started ends: also while its staged install is pending or being
    // confirmed.
    let request_waiting = lock(&state.dev_request).fresh(now_ms).is_some() || {
        let pending = lock(&state.pending);
        let mut flows = lock(&state.request_flows);
        flows.retain(|token, (at, asking)| {
            now_ms.saturating_sub(*at) <= PENDING_TTL_MS && (*asking || pending.contains_key(token))
        });
        !flows.is_empty()
    };
    let (rows, checked) = {
        let _store = state.store_lock();
        (store::list(dir), lock(&state.checked).clone())
    };
    let plugins: Vec<serde_json::Value> = rows
        .into_iter()
        .map(|row| match row {
            Listed::Installed {
                installed,
                manifest,
                ..
            } => {
                let folder = matches!(installed.source, InstallSource::Dir { .. });
                let state = if folder && dev == DevMode::Off {
                    "needs_developer_mode"
                } else if store::needs_consent(&installed) {
                    "needs_review"
                } else if folder && installed.enabled && !checked.contains(&installed.id) {
                    // Nothing arms the watcher while no window runs a
                    // plugin host, so waiting would never end.
                    if hosts {
                        "checking"
                    } else {
                        "plugins_off"
                    }
                } else {
                    "installed"
                };
                let source = match installed.source {
                    InstallSource::File { .. } => "file",
                    InstallSource::Dir { .. } => "dir",
                    InstallSource::Index { .. } => "index",
                    InstallSource::Git { .. } => "git",
                    InstallSource::Url { .. } => "url",
                };
                serde_json::json!({
                    "id": installed.id,
                    "name": plugin_host::display_name(&manifest),
                    "version": installed.version.to_string(),
                    "source": source,
                    "enabled": installed.enabled,
                    "state": state,
                })
            }
            Listed::Unreadable { id, .. } => serde_json::json!({
                "id": id,
                "name": id,
                "version": null,
                "source": null,
                "enabled": false,
                "state": "unreadable",
            }),
        })
        .collect();
    Ok(serde_json::json!({
        "plugins": plugins,
        "developer_mode": dev == DevMode::On,
        "request_waiting": request_waiting,
    }))
}

/// `plugin_host_present`: the page says whether this window's plugin host
/// is running (it starts and stops with the Plugins switch). While no open
/// window has one, `hew.plugin.dev` is refused `plugins_off`; when the last
/// one goes, a waiting request is dropped, since nothing would show it.
pub fn host_present(
    label: &str,
    state: &PluginState,
    present: bool,
    ui: &dyn Ui,
) -> Result<(), String> {
    check_window(label)?;
    state.dir()?;
    if present {
        // A page that starts counts its report numbers from the start.
        super::plugin_data::window_registered(state, label);
    }
    host_set(state, label, present, ui);
    Ok(())
}

/// A window closed: its plugin host went with it, whatever the page last
/// said ([`host_present`] with `present: false`).
pub fn window_gone(app: &tauri::AppHandle, label: &str) {
    if let Some(state) = app.try_state::<PluginState>() {
        let ui = AppEvents { app: app.clone() };
        host_set(&state, label, false, &ui);
        // The rest waits on the store lock, which a plugin write can hold
        // for a while: not on the thread that handles the close.
        let app = app.clone();
        let label = label.to_string();
        tauri::async_runtime::spawn_blocking(move || {
            if let Some(state) = app.try_state::<PluginState>() {
                let ui = AppEvents { app: app.clone() };
                super::plugin_data::window_closed(&state, &label, &ui);
            }
        });
    }
}

/// Records whether window `label` runs a plugin host; when none is left,
/// the waiting request is dropped and every window is told.
fn host_set(state: &PluginState, label: &str, present: bool, ui: &dyn Ui) {
    let none = {
        let mut hosts = lock(&state.hosts);
        if present {
            hosts.insert(label.to_string());
        } else {
            hosts.remove(label);
        }
        hosts.is_empty()
    };
    if none && lock(&state.dev_request).current.take().is_some() {
        ui.dev_request(None);
    }
}

/// An id for a live API request, as 32 lowercase hex digits, made from the
/// standard library's randomly keyed hasher (the keys are random per
/// process; the input is the clock). It only ties a window's picker or
/// Dismiss to the request that window shows: it is unpredictable to a
/// local client, not a secret, and never answered to the caller of
/// `hew.plugin.dev`.
fn new_request_id() -> String {
    use std::hash::{BuildHasher, Hasher};
    (0..2u64)
        .map(|i| {
            let mut h = std::collections::hash_map::RandomState::new().build_hasher();
            h.write_u64(i);
            h.write_u64(now_ms());
            format!("{:016x}", h.finish())
        })
        .collect()
}

/// What [`bus_dev`] answers when it accepts: the request to show in every
/// window, or `None` when the same request was already waiting (shown
/// already; nothing to say again).
pub type BusDevAnswer = Option<DevRequested>;

/// `hew.plugin.dev {path}` (live API): records one request to load a
/// plugin from a folder and answers it for every window to show
/// ([`DevRequested`]); the caller shows it and answers `{status:
/// "awaiting_user"}`. It never picks, stages, or reads the folder, and
/// never records the path as picked: the path is a hint the page shows,
/// and at most where the person's own picker opens. Refused, recording
/// nothing:
///
/// - `invalid_params`: no string path, empty, over
///   [`MAX_REQUEST_PATH_BYTES`], or nothing left once cleaned;
/// - `plugins_off`: no open window runs a plugin host
///   (`window_open` says which labels are still open);
/// - `developer_mode_off`; `confirmation_open` while a native dialog (a
///   confirmation or a picker) is showing;
/// - `request_dismissed`: the person dismissed a request less than
///   [`REQUEST_QUIET_MS`] ago;
/// - `request_waiting`: another path's request is waiting (a request lives
///   [`PENDING_TTL_MS`]). The same path again is accepted with nothing new
///   to show, so a client that asks repeatedly brings nothing back.
pub fn bus_dev(
    state: &PluginState,
    params: &serde_json::Value,
    now_ms: u64,
    window_open: &dyn Fn(&str) -> bool,
) -> Result<BusDevAnswer, BusRefusal> {
    state.dir().map_err(bus_refusal)?;
    let any_host = {
        let mut hosts = lock(&state.hosts);
        // A label whose window is gone counts for nothing, and is forgotten.
        hosts.retain(|label| window_open(label));
        !hosts.is_empty()
    };
    if !any_host {
        return Err(BusRefusal::new(
            "plugins_off",
            "Plugins are turned off in Hew, so there is nowhere to show the request. Turn them on in Settings, then try again.",
        ));
    }
    let invalid = || {
        BusRefusal::new(
            "invalid_params",
            "hew.plugin.dev takes {path}, the plugin's folder.",
        )
    };
    let raw = params
        .get("path")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(invalid)?;
    if raw.is_empty() || raw.len() > MAX_REQUEST_PATH_BYTES {
        return Err(invalid());
    }
    if state.dev() == DevMode::Off {
        return Err(BusRefusal::new(
            "developer_mode_off",
            &InstallError::DeveloperModeOff.explanation(),
        ));
    }
    if state.asking.load(Ordering::Acquire) {
        return Err(BusRefusal::new(
            "confirmation_open",
            "Hew is already asking the person something. Try again after it is answered.",
        ));
    }
    let shown = plugin_host::clean_text(raw, MAX_REQUEST_PATH_CHARS);
    if shown.is_empty() {
        return Err(invalid());
    }
    let mut slot = lock(&state.dev_request);
    if now_ms < slot.quiet_until_ms {
        return Err(BusRefusal::new(
            "request_dismissed",
            "The person dismissed the last request. Try again in a minute.",
        ));
    }
    if let Some(waiting) = slot.fresh(now_ms) {
        if waiting.shown == shown {
            return Ok(None);
        }
        return Err(BusRefusal::new(
            "request_waiting",
            "Another request to load a plugin is waiting for the person. Try again after it is answered.",
        ));
    }
    let request = DevRequest {
        id: new_request_id(),
        start: start_folder_hint(raw, &shown, &no_start_roots(state)),
        shown,
        created_at_ms: now_ms,
    };
    let answer = DevRequested {
        id: request.id.clone(),
        path: request.shown.clone(),
    };
    slot.current = Some(request);
    Ok(Some(answer))
}

/// `plugin_dev_request`: the waiting live API request as the page shows it
/// (its id and the same cleaned, cut path the event carried), or `None`
/// when there is none or it is older than [`PENDING_TTL_MS`]. It is only
/// looked at, never taken, so a page that missed the event can still show
/// it; the folder picker takes it ([`pick_dir_start`]).
pub fn dev_request(
    label: &str,
    state: &PluginState,
    now_ms: u64,
) -> Result<Option<DevRequested>, String> {
    check_window(label)?;
    state.dir()?;
    if state.dev() == DevMode::Off {
        return Ok(None);
    }
    Ok(lock(&state.dev_request)
        .fresh(now_ms)
        .map(|r| DevRequested {
            id: r.id.clone(),
            path: r.shown.clone(),
        }))
}

/// `plugin_dev_request_dismiss {id}`: the person dismissed request `id`.
/// It is forgotten, every window is told, and the live API is refused
/// `request_dismissed` for [`REQUEST_QUIET_MS`], so a client that keeps
/// asking cannot keep a notice on screen. A request that is no longer the
/// waiting one is left alone (the windows are told what waits instead);
/// with none waiting, the quiet starts all the same.
pub fn dismiss_request(
    label: &str,
    state: &PluginState,
    id: &str,
    now_ms: u64,
    ui: &dyn Ui,
) -> Result<(), String> {
    check_window(label)?;
    state.dir()?;
    let mut slot = lock(&state.dev_request);
    match slot.fresh(now_ms) {
        Some(waiting) if waiting.id != id => {
            let shown = DevRequested {
                id: waiting.id.clone(),
                path: waiting.shown.clone(),
            };
            drop(slot);
            ui.dev_request(Some(shown));
        }
        _ => {
            slot.current = None;
            slot.quiet_until_ms = now_ms.saturating_add(REQUEST_QUIET_MS);
            drop(slot);
            ui.dev_request(None);
        }
    }
    Ok(())
}

/// The folders a request may never open the picker in: the plugin store,
/// the app's data folder and its config folder (where developer mode is
/// kept).
fn no_start_roots(state: &PluginState) -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Some(dir) = &state.dir {
        roots.push(dir.root().to_path_buf());
    }
    if let Some(data) = store::app_data_dir() {
        roots.push(data);
    }
    if let Some(config) = state.dev_mode_file.as_deref().and_then(Path::parent) {
        roots.push(config.to_path_buf());
    }
    roots
}

/// Whether `path` is `root` or inside it, by text. Without regard to case
/// where the usual file systems ignore it (macOS, Windows).
fn inside(path: &Path, root: &Path) -> bool {
    if cfg!(any(target_os = "macos", windows)) {
        let lower = |p: &Path| PathBuf::from(p.to_string_lossy().to_lowercase());
        lower(path).starts_with(lower(root))
    } else {
        path.starts_with(root)
    }
}

/// The folder a request's picker may open near, decided from the text
/// alone (nothing is looked up, except a Windows drive's type):
///
/// - only when cleaning for display left the path exactly as it came
///   (`shown`), so the picker never opens at a folder the notice does not
///   name in full;
/// - an absolute path written plainly: no `.`, `..` or empty segment, no
///   repeated or trailing separator (refused, not cleaned);
/// - not a network place, as far as the text can tell: on Unix not under
///   `//`, `/net`, `/Network`, `/afs`, or a desktop's network mounts
///   (`/run/user/<n>/gvfs`, `/run/user/<n>/doc`); on Windows only a drive
///   letter whose drive is a fixed disk (no `\\server`, `\\?\`, device
///   path, mapped network drive or removable drive). A network share
///   mounted elsewhere (`/Volumes/<name>`, `/mnt/<name>`) cannot be told
///   from the text and is not refused;
/// - not inside the plugin store or the app's data or config folders
///   (`no_start`).
///
/// Refused, the picker opens with no start folder; the notice still shows
/// the path.
fn start_folder_hint(raw: &str, shown: &str, no_start: &[PathBuf]) -> Option<PathBuf> {
    if raw != shown || raw.chars().any(char::is_control) {
        return None;
    }
    let plain = |segments: &[&str]| {
        segments
            .iter()
            .all(|s| !s.is_empty() && *s != "." && *s != "..")
    };
    let path = if cfg!(windows) {
        let b = raw.as_bytes();
        let drive = b.len() >= 3
            && b[0].is_ascii_alphabetic()
            && b[1] == b':'
            && (b[2] == b'\\' || b[2] == b'/');
        if !drive {
            return None;
        }
        let rest = &raw[3..];
        if !rest.is_empty() && !plain(&rest.split(['\\', '/']).collect::<Vec<_>>()) {
            return None;
        }
        if !fixed_drive(b[0]) {
            return None;
        }
        PathBuf::from(raw)
    } else {
        let rest = raw.strip_prefix('/')?;
        if !rest.is_empty() {
            let segments: Vec<&str> = rest.split('/').collect();
            if !plain(&segments) {
                return None;
            }
            let first = segments[0].to_ascii_lowercase();
            if matches!(first.as_str(), "net" | "network" | "afs") {
                return None;
            }
            if segments.len() >= 4
                && segments[0] == "run"
                && segments[1] == "user"
                && matches!(segments[3], "gvfs" | "doc")
            {
                return None;
            }
        }
        PathBuf::from(raw)
    };
    if no_start.iter().any(|root| inside(&path, root)) {
        return None;
    }
    Some(path)
}

/// Whether drive `letter` is a fixed disk (not a network, removable or
/// optical drive), asked of Windows.
#[cfg(windows)]
fn fixed_drive(letter: u8) -> bool {
    use windows_sys::Win32::Storage::FileSystem::GetDriveTypeW;
    /// `DRIVE_FIXED`.
    const FIXED: u32 = 3;
    let root: Vec<u16> = [u16::from(letter), u16::from(b':'), u16::from(b'\\'), 0].to_vec();
    // SAFETY: `root` is a NUL-terminated UTF-16 string that outlives the
    // call, which only reads it.
    unsafe { GetDriveTypeW(root.as_ptr()) == FIXED }
}

#[cfg(not(windows))]
fn fixed_drive(_letter: u8) -> bool {
    false
}

fn request_gone() -> String {
    refusal(
        "request_gone",
        "That request is no longer waiting. Ask again from the program that sent it.",
    )
}

/// The folder `plugin_pick_dir` opens at. With no request id, none. With
/// one, it must name the waiting request (not expired); anything else is
/// `request_gone`, so the picker never opens for a request other than the
/// one the window showed. The request is only looked at here: it is used
/// up when the folder it led to stages ([`stage_dir_for_request`]), so a
/// cancel or a refused folder leaves it waiting. The picker opens at the
/// requested folder itself, when it passes the same filters as before (a
/// plain local absolute path, really there, not Hew's own folders); the
/// consent sheet and the native confirmation that follow show the folder
/// picked on its own line. The page passes only the id, never a path.
pub fn pick_dir_start(
    state: &PluginState,
    use_request: Option<&str>,
    now_ms: u64,
) -> Result<Option<PathBuf>, String> {
    let Some(id) = use_request else {
        return Ok(None);
    };
    let request = {
        let mut slot = lock(&state.dev_request);
        match slot.fresh(now_ms) {
            Some(r) if r.id == id => r.clone(),
            _ => return Err(request_gone()),
        }
    };
    Ok(request
        .start
        .and_then(|start| real_start(&start, &no_start_roots(state))))
}

/// The folder the picker opens at, by where it really is: `parent`
/// canonicalized (a link into the plugin store or the app's folders is
/// refused by what it points at), and refused inside `no_start` (each
/// canonicalized too where it exists). `None` when it cannot be
/// canonicalized.
fn real_start(parent: &Path, no_start: &[PathBuf]) -> Option<PathBuf> {
    let real = std::fs::canonicalize(parent).ok()?;
    let refused = no_start.iter().any(|root| {
        let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.clone());
        inside(&real, &root)
    });
    (!refused).then_some(real)
}

/// `plugin_pick_dir`, without the window: refused `developer_mode_off`
/// first, then `confirmation_open` while any native dialog is showing (the
/// picker counts as one for as long as it is open), then
/// [`pick_dir_start`]'s `request_gone`; then `show` runs the picker from
/// the start folder and the person's pick is recorded. `None` when
/// canceled.
pub fn pick_dir(
    label: &str,
    state: &PluginState,
    use_request: Option<&str>,
    now_ms: u64,
    show: impl FnOnce(Option<PathBuf>) -> Option<PathBuf>,
) -> Result<Option<String>, String> {
    check_window(label)?;
    state.dir()?;
    require_dev(state)?;
    let _asking = begin_asking(state)?;
    let start = pick_dir_start(state, use_request, now_ms)?;
    let Some(path) = show(start) else {
        return Ok(None);
    };
    require_dev(state)?;
    record_pick_for(
        state,
        &path,
        PickKind::Folder,
        use_request.map(str::to_string),
    )
    .map(Some)
}

/// `plugin_pick_package`, without the window: refused `confirmation_open`
/// while any native dialog is showing; then `show` runs the picker and the
/// pick is recorded. `None` when canceled.
pub fn pick_package(
    label: &str,
    state: &PluginState,
    show: impl FnOnce() -> Option<PathBuf>,
) -> Result<Option<String>, String> {
    check_window(label)?;
    state.dir()?;
    let _asking = begin_asking(state)?;
    let Some(path) = show() else {
        return Ok(None);
    };
    record_pick(state, &path, PickKind::Package).map(Some)
}

/// Parses permission names from the page, dropping repeats.
fn permission_names(names: &[String]) -> Result<Vec<PermissionName>, String> {
    let mut out: Vec<PermissionName> = Vec::with_capacity(names.len());
    for name in names {
        let p =
            PermissionName::parse(name).map_err(|e| install_error(InstallError::Manifest(e)))?;
        if !out.contains(&p) {
            out.push(p);
        }
    }
    Ok(out)
}

/// `plugin_set_grant`: replaces a plugin's grant. Anything the manifest does
/// not declare is refused. When the new grant adds anything the stored
/// record does not hold, the user is asked natively, listing what is added
/// and the warnings the resulting grant brings; a no leaves the record as
/// it was and returns it. The record is read again after the answer and
/// the change refused `grant_changed` if it moved meanwhile. Narrowing asks
/// nothing and is decided and written under one hold of the store lock.
pub fn set_grant(
    label: &str,
    state: &PluginState,
    id: &str,
    grant: &[String],
    optional: &[String],
    ui: &dyn Ui,
) -> Result<Installed, String> {
    check_window(label)?;
    check_id(id)?;
    let dir = state.dir()?;
    let grant = permission_names(grant)?;
    let optional = permission_names(optional)?;
    let (current, manifest, added) = {
        let _store = state.store_lock();
        let (current, manifest) = store::installed(dir, id).map_err(install_error)?;
        // A record held back for the person's review is changed only by the
        // review: no grant, no dialog, for a manifest nobody has approved.
        if store::needs_consent(&current) {
            return Err(install_error(InstallError::DevConsentPending(
                id.to_string(),
            )));
        }
        let added = grant_added(&current, &manifest, &grant, &optional)?;
        if added.is_empty() {
            let installed = store::set_grant(dir, id, grant, optional).map_err(install_error)?;
            drop(_store);
            after_change(state, ui);
            return Ok(installed);
        }
        (current, manifest, added)
    };
    let name_version = format!(
        "{} {}",
        plugin_host::display_name(&manifest),
        current.version
    );
    let resulting: Vec<PermissionName> = grant.iter().chain(&optional).cloned().collect();
    let body = plugin_host::grant_change_summary(&name_version, &added, &resulting);
    let yes = {
        let _asking = begin_asking(state)?;
        ui.confirm("Allow more for this plugin?", &body, "Allow")
    };
    if !yes {
        return Ok(current);
    }
    let installed = {
        let _store = state.store_lock();
        let (now_current, now_manifest) = store::installed(dir, id).map_err(install_error)?;
        if now_current != current || now_manifest != manifest {
            return Err(refusal(
                "grant_changed",
                "The plugin's permissions changed while Hew was asking. Try again.",
            ));
        }
        store::set_grant(dir, id, grant, optional).map_err(install_error)?
    };
    after_change(state, ui);
    Ok(installed)
}

/// What `grant` and `optional` add to the stored record, after refusing
/// anything the manifest does not declare (`unknown_permission`).
fn grant_added(
    current: &Installed,
    manifest: &plugin_manifest::Manifest,
    grant: &[PermissionName],
    optional: &[PermissionName],
) -> Result<Vec<PermissionName>, String> {
    let undeclared = grant
        .iter()
        .find(|p| !manifest.permissions.contains(p))
        .or_else(|| {
            optional
                .iter()
                .find(|p| !manifest.optional_permissions.contains(p))
        });
    if let Some(p) = undeclared {
        return Err(install_error(InstallError::Manifest(
            plugin_manifest::ManifestError::UnknownPermission(p.as_str().to_string()),
        )));
    }
    Ok(grant
        .iter()
        .filter(|p| !current.grant.contains(p))
        .chain(
            optional
                .iter()
                .filter(|p| !current.optional_granted.contains(p)),
        )
        .cloned()
        .collect())
}

/// `plugin_set_enabled`: turns a plugin on or off; returns the plugins that
/// were turned off with it.
///
/// Turning one off asks nothing. Turning one on lets it run, and start
/// with Hew if it asks to, so the user is asked natively with what it
/// holds; a no is `enable_declined` and changes nothing. A plugin already
/// on and allowed to start is left as it is without asking. The record is
/// read again after the answer and the change refused `plugin_changed` if
/// it moved meanwhile.
pub fn set_enabled(
    label: &str,
    state: &PluginState,
    id: &str,
    enabled: bool,
    ui: &dyn Ui,
) -> Result<Vec<String>, String> {
    check_window(label)?;
    check_id(id)?;
    let dir = state.dir()?;
    if !enabled {
        let (_, also) = {
            let _store = state.store_lock();
            store::set_enabled(dir, id, false).map_err(install_error)?
        };
        after_change(state, ui);
        return Ok(also);
    }
    let (current, manifest) = {
        let _store = state.store_lock();
        store::installed(dir, id).map_err(install_error)?
    };
    if current.enabled && current.startup_allowed {
        return Ok(Vec::new());
    }
    let body = plugin_host::enable_summary(&current, &manifest);
    let yes = {
        let _asking = begin_asking(state)?;
        ui.confirm("Turn on this plugin?", &body, "Turn On")
    };
    if !yes {
        return Err(refusal("enable_declined", "The plugin was left off."));
    }
    // A folder plugin starts being watched: its manifest is read now and
    // compared under the same hold of the store lock as the write.
    let epoch = state.dev_epoch.load(Ordering::Acquire);
    let ahead = match &current.source {
        InstallSource::Dir { path } => read_ahead(state, path),
        _ => None,
    };
    let mut found = Settled::default();
    let (also, compared) = {
        let _store = state.store_lock();
        let now = store::installed(dir, id).map_err(install_error)?;
        if now != (current, manifest) {
            return Err(plugin_changed());
        }
        let also = store::set_enabled(dir, id, true).map_err(install_error)?.1;
        (
            also,
            compare_ahead(state, dir, id, ahead, epoch, &mut found),
        )
    };
    let settled: &[&str] = if compared { &[id] } else { &[] };
    after_change_with(state, ui, found, settled);
    Ok(also)
}

fn plugin_changed() -> String {
    refusal(
        "plugin_changed",
        "The plugin changed while Hew was asking. Try again.",
    )
}

/// `plugin_uninstall`: removes a plugin and everything the store holds
/// for it, its saved data included, after asking natively. A no is
/// `uninstall_declined` and removes nothing. Refusals the store would make
/// (not installed, still required by enabled plugins) come before the
/// question. A plugin whose record cannot be read is named by its id.
pub fn uninstall(label: &str, state: &PluginState, id: &str, ui: &dyn Ui) -> Result<(), String> {
    check_window(label)?;
    check_id(id)?;
    let dir = state.dir()?;
    let shown = {
        let _store = state.store_lock();
        let shown = match store::installed(dir, id) {
            Ok(found) => Some(found),
            Err(InstallError::NotInstalled(_)) => {
                return Err(install_error(InstallError::NotInstalled(id.to_string())));
            }
            // Unreadable records can still be removed.
            Err(_) => None,
        };
        let dependents = store::dependents_of(dir, id);
        if !dependents.is_empty() {
            return Err(install_error(InstallError::DependencyMissing(dependents)));
        }
        shown
    };
    let body = match &shown {
        Some((installed, manifest)) => plugin_host::uninstall_summary(installed, manifest),
        // The id passed the plugin id grammar, so it is plain text.
        None => format!(
            "{id}\nHew cannot read this plugin's record. Removes the plugin and everything it saved.\n"
        ),
    };
    let yes = {
        let _asking = begin_asking(state)?;
        ui.confirm("Uninstall this plugin?", &body, "Uninstall")
    };
    if !yes {
        return Err(refusal(
            "uninstall_declined",
            "The plugin was left installed.",
        ));
    }
    {
        let _store = state.store_lock();
        store::uninstall(dir, id).map_err(install_error)?;
        // Under the same hold, so no report about the plugin that is gone
        // lands after it is forgotten.
        super::plugin_data::forget(state, id);
    }
    after_change(state, ui);
    Ok(())
}

/// `plugin_rollback`: goes back to the version installed before. The
/// previous version's record carries the grant it had, which may be wider
/// than the one the user has since narrowed to, so the user is asked
/// natively with what it will be able to do. A no leaves everything as it
/// is. The records are read again after the answer, and the rollback is
/// refused `plugin_changed` if either changed while the dialog was open, so
/// what runs is what was shown. A folder plugin's folder is compared with
/// the version that comes back under the same hold of the store lock as
/// the rollback, so the restored record is marked again, before anything
/// can read it, when the folder now asks for more than it did.
pub fn rollback(
    label: &str,
    state: &PluginState,
    id: &str,
    ui: &dyn Ui,
) -> Result<Installed, String> {
    check_window(label)?;
    check_id(id)?;
    let dir = state.dir()?;
    let shown = {
        let _store = state.store_lock();
        let (current, _) = store::installed(dir, id).map_err(install_error)?;
        let previous = store::previous(dir, id).map_err(install_error)?;
        // Refused by the install rules before the person is asked.
        store::check_rollback(dir, id, running_version()).map_err(install_error)?;
        (current, previous)
    };
    let (current, (previous, previous_manifest)) = &shown;
    let body = plugin_host::rollback_summary(current, previous, previous_manifest);
    let yes = {
        let _asking = begin_asking(state)?;
        ui.confirm("Roll back this plugin?", &body, "Roll back")
    };
    if !yes {
        return Err(rollback_declined());
    }
    let epoch = state.dev_epoch.load(Ordering::Acquire);
    let ahead = match &previous.source {
        InstallSource::Dir { path } => read_ahead(state, path),
        _ => None,
    };
    let mut found = Settled::default();
    let (installed, compared) = {
        let _store = state.store_lock();
        let (now_current, _) = store::installed(dir, id).map_err(install_error)?;
        let now_previous = store::previous(dir, id).map_err(install_error)?;
        if &now_current != current || now_previous != (previous.clone(), previous_manifest.clone())
        {
            return Err(plugin_changed());
        }
        store::rollback(dir, id, running_version()).map_err(install_error)?;
        let compared = compare_ahead(state, dir, id, ahead, epoch, &mut found);
        let (installed, _) = store::installed(dir, id).map_err(install_error)?;
        (installed, compared)
    };
    let settled: &[&str] = if compared { &[id] } else { &[] };
    after_change_with(state, ui, found, settled);
    Ok(installed)
}

fn rollback_declined() -> String {
    refusal("rollback_declined", "The plugin was left as it is.")
}

/// `plugin_read_file`: one file of an installed plugin, through the
/// store's jail (no path outside the plugin's files, no links).
pub fn read_file(label: &str, state: &PluginState, id: &str, rel: &str) -> Result<Vec<u8>, String> {
    check_window(label)?;
    check_id(id)?;
    let dir = state.dir()?;
    plugin_host::normalize_path(rel).map_err(install_error)?;
    // The record is read under the store lock; the file is read without it,
    // so a slow or stuck folder holds up no other command. Bytes read from a
    // folder while developer mode went off are dropped.
    let epoch = state.dev_epoch.load(Ordering::Acquire);
    let source = {
        let _store = state.store_lock();
        let source = store::file_source(dir, id, state.dev()).map_err(install_error)?;
        if source.is_folder() && !lock(&state.checked).contains(id) {
            lock(&state.refused_unchecked).insert(id.to_string());
            return Err(not_checked());
        }
        source
    };
    read_checked(state, &source, epoch, || source.read(rel))
}

/// The refusal of a read of a folder plugin whose folder has not been
/// compared since it started being watched. `folder_missing`, so the host
/// holds it as not startable (not crashed), and starts it once told.
fn not_checked() -> String {
    refusal(
        "folder_missing",
        "Hew is still checking this plugin's folder. It starts once the check is done.",
    )
}

/// Runs `read` and, for a folder, drops what it read when developer mode
/// is off now or went off since `epoch`.
fn read_checked(
    state: &PluginState,
    source: &store::FileSource,
    epoch: u64,
    read: impl FnOnce() -> Result<Vec<u8>, InstallError>,
) -> Result<Vec<u8>, String> {
    let bytes = read().map_err(install_error)?;
    if source.is_folder() && !dev_still_on(state, epoch) {
        return Err(developer_mode_off());
    }
    Ok(bytes)
}

/// `plugin_storage_get`: the value plugin `id` saved under `key`, if any.
pub fn storage_get(
    label: &str,
    state: &PluginState,
    id: &str,
    key: &str,
) -> Result<Option<String>, String> {
    check_window(label)?;
    check_id(id)?;
    let dir = state.dir()?;
    let _store = state.store_lock();
    store::storage_get(dir, id, key).map_err(install_error)
}

/// `plugin_storage_list`: every key plugin `id` has saved, sorted.
pub fn storage_list(label: &str, state: &PluginState, id: &str) -> Result<Vec<String>, String> {
    check_window(label)?;
    check_id(id)?;
    let dir = state.dir()?;
    let _store = state.store_lock();
    store::storage_list(dir, id).map_err(install_error)
}

/// `plugin_storage_set`: saves `value` under `key` for plugin `id`, then
/// tells every window. Refused `storage_quota` past the plugin's quota and
/// `storage_key_invalid` for an empty or overlong key, with nothing
/// written.
pub fn storage_set(
    label: &str,
    state: &PluginState,
    id: &str,
    key: &str,
    value: &str,
    ui: &dyn Ui,
) -> Result<(), String> {
    check_window(label)?;
    check_id(id)?;
    let dir = state.dir()?;
    {
        let _store = state.store_lock();
        store::storage_set(dir, id, key, value).map_err(install_error)?;
    }
    ui.storage_changed(StorageChanged {
        id: id.to_string(),
        keys: vec![key.to_string()],
    });
    Ok(())
}

/// `plugin_storage_delete`: removes `key` from plugin `id`'s saved data;
/// tells every window only when it was there.
pub fn storage_delete(
    label: &str,
    state: &PluginState,
    id: &str,
    key: &str,
    ui: &dyn Ui,
) -> Result<(), String> {
    check_window(label)?;
    check_id(id)?;
    let dir = state.dir()?;
    let removed = {
        let _store = state.store_lock();
        store::storage_delete(dir, id, key).map_err(install_error)?
    };
    if removed {
        ui.storage_changed(StorageChanged {
            id: id.to_string(),
            keys: vec![key.to_string()],
        });
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// The Tauri commands. Each runs its plain function off the main thread
// (packages are read whole, and the confirmations block until answered).
// ---------------------------------------------------------------------------

/// The app's [`Ui`]: Hew's native confirmation over the calling window,
/// and global events.
pub(crate) struct WindowUi {
    window: tauri::WebviewWindow,
}

impl Ui for WindowUi {
    fn confirm(&self, title: &str, body: &str, accept: &str) -> bool {
        super::confirm::confirm_native(&self.window, title, body, accept)
    }

    fn changed(&self) {
        let _ = self.window.app_handle().emit(CHANGED_EVENT, ());
    }

    fn storage_changed(&self, change: StorageChanged) {
        let _ = self.window.app_handle().emit(STORAGE_CHANGED_EVENT, change);
    }

    fn dev_changed(&self, change: DevChanged) {
        let _ = self.window.app_handle().emit(DEV_CHANGED_EVENT, change);
    }

    fn dev_request(&self, request: Option<DevRequested>) {
        let _ = self.window.app_handle().emit(DEV_REQUEST_EVENT, request);
    }

    fn doc_storage_changed(&self, change: super::plugin_data::DocStorageChanged) {
        let _ = self.window.app_handle().emit(STORAGE_CHANGED_EVENT, change);
    }

    fn state_changed(&self, id: &str) {
        let _ = self.window.app_handle().emit(
            super::plugin_data::STATE_EVENT,
            super::plugin_data::PluginEvent { id: id.to_string() },
        );
    }

    fn console_changed(&self, id: &str) {
        let _ = self.window.app_handle().emit(
            super::plugin_data::CONSOLE_EVENT,
            super::plugin_data::PluginEvent { id: id.to_string() },
        );
    }
}

/// The [`Ui`] of work no window asked for (the folder watcher): global
/// events only. It never asks anything; a question would be a no.
struct AppEvents {
    app: tauri::AppHandle,
}

impl Ui for AppEvents {
    fn confirm(&self, _: &str, _: &str, _: &str) -> bool {
        false
    }

    fn changed(&self) {
        let _ = self.app.emit(CHANGED_EVENT, ());
    }

    fn storage_changed(&self, change: StorageChanged) {
        let _ = self.app.emit(STORAGE_CHANGED_EVENT, change);
    }

    fn dev_changed(&self, change: DevChanged) {
        let _ = self.app.emit(DEV_CHANGED_EVENT, change);
    }

    fn dev_request(&self, request: Option<DevRequested>) {
        let _ = self.app.emit(DEV_REQUEST_EVENT, request);
    }

    fn doc_storage_changed(&self, change: super::plugin_data::DocStorageChanged) {
        let _ = self.app.emit(STORAGE_CHANGED_EVENT, change);
    }

    fn state_changed(&self, id: &str) {
        let _ = self.app.emit(
            super::plugin_data::STATE_EVENT,
            super::plugin_data::PluginEvent { id: id.to_string() },
        );
    }

    fn console_changed(&self, id: &str) {
        let _ = self.app.emit(
            super::plugin_data::CONSOLE_EVENT,
            super::plugin_data::PluginEvent { id: id.to_string() },
        );
    }
}

/// The app's [`Ui`] for work no window asked for: global events only.
pub fn app_events(app: &tauri::AppHandle) -> impl Ui {
    AppEvents { app: app.clone() }
}

/// Gives the folder watcher its handler. Called once, from the app's
/// setup, after the state is managed; nothing is watched until the page
/// first lists plugins ([`arm_watch`]).
pub fn start_watching(app: &tauri::AppHandle) {
    let Some(state) = app.try_state::<PluginState>() else {
        return;
    };
    let handle = app.clone();
    state.watch.set_handler(std::sync::Arc::new(move |event| {
        let Some(state) = handle.try_state::<PluginState>() else {
            return Watching::Drop;
        };
        on_watch(
            &state,
            event,
            &AppEvents {
                app: handle.clone(),
            },
        )
    }));
}

/// Stops the folder watcher for good and waits for its thread. Called at
/// app exit.
pub fn stop_watching(app: &tauri::AppHandle) {
    if let Some(state) = app.try_state::<PluginState>() {
        state.watch.shutdown();
    }
}

/// Runs `f` on a blocking thread with the window's label and state.
pub(crate) async fn off_main<T, F>(window: tauri::WebviewWindow, f: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce(&str, &PluginState, &WindowUi) -> Result<T, String> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(move || {
        let state = window.state::<PluginState>();
        let label = window.label().to_string();
        let ui = WindowUi {
            window: window.clone(),
        };
        f(&label, &state, &ui)
    })
    .await
    .map_err(|e| install_error(InstallError::Io(e.to_string())))?
}

#[tauri::command]
pub async fn plugin_list(window: tauri::WebviewWindow) -> Result<Vec<serde_json::Value>, String> {
    off_main(window, |label, state, ui| list_rows(label, state, ui)).await
}

/// Shows the native open panel filtered to `.hewplugin` and records the
/// pick for `plugin_stage_file` ([`pick_package`]). `None` when canceled.
#[tauri::command]
pub async fn plugin_pick_package(window: tauri::WebviewWindow) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    check_window(window.label())?;
    let dialog = window
        .dialog()
        .file()
        .set_title("Install a Plugin")
        .add_filter("Hew plugin", &[PACKAGE_EXTENSION]);
    off_main(window, move |label, state, _| {
        pick_package(label, state, move || {
            dialog.blocking_pick_file().and_then(|p| p.into_path().ok())
        })
    })
    .await
}

/// Shows the native folder picker and records the pick for
/// `plugin_stage_dir` ([`pick_dir`]). `None` when canceled. With
/// `use_request`, the id of the live API request the window shows, the
/// picker opens near that request's folder, or is refused `request_gone`
/// when it is no longer the one waiting; the page passes only the id,
/// never a path, and what is recorded is whatever the person picks.
#[tauri::command]
pub async fn plugin_pick_dir(
    window: tauri::WebviewWindow,
    use_request: Option<String>,
) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    check_window(window.label())?;
    let dialog = window
        .dialog()
        .file()
        .set_title("Install from a Developer Folder");
    let now = now_ms();
    off_main(window, move |label, state, _| {
        pick_dir(label, state, use_request.as_deref(), now, move |start| {
            let dialog = match start {
                Some(start) => dialog.set_directory(start),
                None => dialog,
            };
            dialog
                .blocking_pick_folder()
                .and_then(|p| p.into_path().ok())
        })
    })
    .await
}

#[tauri::command]
pub async fn plugin_stage_file(
    window: tauri::WebviewWindow,
    path: String,
) -> Result<PendingView, String> {
    let now = now_ms();
    off_main(window, move |label, state, _| {
        stage_file(label, state, &path, now)
    })
    .await
}

#[tauri::command]
pub async fn plugin_stage_dir(
    window: tauri::WebviewWindow,
    path: String,
) -> Result<PendingView, String> {
    let now = now_ms();
    off_main(window, move |label, state, ui| {
        stage_dir_for_request(label, state, &path, now, ui)
    })
    .await
}

#[tauri::command]
pub async fn plugin_commit(
    window: tauri::WebviewWindow,
    token: String,
    accept: bool,
) -> Result<Option<Installed>, String> {
    let now = now_ms();
    off_main(window, move |label, state, ui| {
        commit(label, state, &token, accept, ui, now)
    })
    .await
}

#[tauri::command]
pub async fn plugin_dev_mode(window: tauri::WebviewWindow) -> Result<bool, String> {
    off_main(window, |label, state, ui| dev_mode(label, state, ui)).await
}

#[tauri::command]
pub async fn plugin_host_present(
    window: tauri::WebviewWindow,
    present: bool,
) -> Result<(), String> {
    off_main(window, move |label, state, ui| {
        host_present(label, state, present, ui)
    })
    .await
}

#[tauri::command]
pub async fn plugin_dev_request_dismiss(
    window: tauri::WebviewWindow,
    id: String,
) -> Result<(), String> {
    let now = now_ms();
    off_main(window, move |label, state, ui| {
        dismiss_request(label, state, &id, now, ui)
    })
    .await
}

#[tauri::command]
pub async fn plugin_set_dev_mode(window: tauri::WebviewWindow, on: bool) -> Result<bool, String> {
    off_main(window, move |label, state, ui| {
        set_dev_mode(label, state, on, ui)
    })
    .await
}

#[tauri::command]
pub async fn plugin_dev_request(
    window: tauri::WebviewWindow,
) -> Result<Option<DevRequested>, String> {
    let now = now_ms();
    off_main(window, move |label, state, _| {
        dev_request(label, state, now)
    })
    .await
}

#[tauri::command]
pub async fn plugin_dev_review(
    window: tauri::WebviewWindow,
    id: String,
) -> Result<PendingView, String> {
    let now = now_ms();
    off_main(window, move |label, state, _| {
        review(label, state, &id, now)
    })
    .await
}

#[tauri::command]
pub async fn plugin_set_enabled(
    window: tauri::WebviewWindow,
    id: String,
    enabled: bool,
) -> Result<Vec<String>, String> {
    off_main(window, move |label, state, ui| {
        set_enabled(label, state, &id, enabled, ui)
    })
    .await
}

#[tauri::command]
pub async fn plugin_set_grant(
    window: tauri::WebviewWindow,
    id: String,
    grant: Vec<String>,
    optional: Vec<String>,
) -> Result<Installed, String> {
    off_main(window, move |label, state, ui| {
        set_grant(label, state, &id, &grant, &optional, ui)
    })
    .await
}

#[tauri::command]
pub async fn plugin_uninstall(window: tauri::WebviewWindow, id: String) -> Result<(), String> {
    off_main(window, move |label, state, ui| {
        uninstall(label, state, &id, ui)
    })
    .await
}

#[tauri::command]
pub async fn plugin_rollback(
    window: tauri::WebviewWindow,
    id: String,
) -> Result<Installed, String> {
    off_main(window, move |label, state, ui| {
        rollback(label, state, &id, ui)
    })
    .await
}

#[tauri::command]
pub async fn plugin_read_file(
    window: tauri::WebviewWindow,
    id: String,
    rel: String,
) -> Result<tauri::ipc::Response, String> {
    off_main(window, move |label, state, _| {
        read_file(label, state, &id, &rel)
    })
    .await
    .map(tauri::ipc::Response::new)
}

#[tauri::command]
pub async fn plugin_storage_get(
    window: tauri::WebviewWindow,
    id: String,
    key: String,
) -> Result<Option<String>, String> {
    off_main(window, move |label, state, _| {
        storage_get(label, state, &id, &key)
    })
    .await
}

#[tauri::command]
pub async fn plugin_storage_set(
    window: tauri::WebviewWindow,
    id: String,
    key: String,
    value: String,
) -> Result<(), String> {
    off_main(window, move |label, state, ui| {
        storage_set(label, state, &id, &key, &value, ui)
    })
    .await
}

#[tauri::command]
pub async fn plugin_storage_delete(
    window: tauri::WebviewWindow,
    id: String,
    key: String,
) -> Result<(), String> {
    off_main(window, move |label, state, ui| {
        storage_delete(label, state, &id, &key, ui)
    })
    .await
}

#[tauri::command]
pub async fn plugin_storage_list(
    window: tauri::WebviewWindow,
    id: String,
) -> Result<Vec<String>, String> {
    off_main(window, move |label, state, _| {
        storage_list(label, state, &id)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    #[test]
    fn the_boot_script_needs_its_hash_in_script_src() {
        let with = "default-src 'self'; script-src 'self' 'sha256-abc='; style-src 'self'";
        assert!(policy_allows_boot(Some(with), "abc="));
        assert!(!policy_allows_boot(Some(with), "abd="));
        assert!(!policy_allows_boot(Some("script-src 'self'"), "abc="));
        // A hash in another directive allows no script.
        assert!(!policy_allows_boot(Some("style-src 'sha256-abc='"), "abc="));
        assert!(!policy_allows_boot(None, "abc="));
        assert!(!policy_allows_boot(Some(with), ""));
    }

    #[test]
    fn a_shell_without_the_preview_policy_refuses_every_command() {
        let state = PluginState::resolve(Some("script-src 'self'"), "abc=", None);
        let e = state.dir().expect_err("refused");
        assert_eq!(refusal_name(&e), "plugins_disabled");
    }

    /// A [`Ui`] with a fixed answer that records what it was asked.
    struct Stub {
        answer: bool,
        asked: RefCell<Vec<(String, String, String)>>,
        changed: Cell<usize>,
        stored: RefCell<Vec<StorageChanged>>,
        dev: RefCell<Vec<DevChanged>>,
        requests: RefCell<Vec<Option<DevRequested>>>,
        doc_stored: RefCell<Vec<super::super::plugin_data::DocStorageChanged>>,
        states: RefCell<Vec<String>>,
        consoles: RefCell<Vec<String>>,
    }

    impl Stub {
        fn answering(answer: bool) -> Stub {
            Stub {
                answer,
                asked: RefCell::new(Vec::new()),
                changed: Cell::new(0),
                stored: RefCell::new(Vec::new()),
                dev: RefCell::new(Vec::new()),
                requests: RefCell::new(Vec::new()),
                doc_stored: RefCell::new(Vec::new()),
                states: RefCell::new(Vec::new()),
                consoles: RefCell::new(Vec::new()),
            }
        }
        fn asked(&self) -> usize {
            self.asked.borrow().len()
        }
    }

    impl Ui for Stub {
        fn confirm(&self, title: &str, body: &str, accept: &str) -> bool {
            self.asked
                .borrow_mut()
                .push((title.to_string(), body.to_string(), accept.to_string()));
            self.answer
        }
        fn changed(&self) {
            self.changed.set(self.changed.get() + 1);
        }
        fn storage_changed(&self, change: StorageChanged) {
            self.stored.borrow_mut().push(change);
        }
        fn dev_changed(&self, change: DevChanged) {
            self.dev.borrow_mut().push(change);
        }
        fn dev_request(&self, request: Option<DevRequested>) {
            self.requests.borrow_mut().push(request);
        }
        fn doc_storage_changed(&self, change: super::super::plugin_data::DocStorageChanged) {
            self.doc_stored.borrow_mut().push(change);
        }
        fn state_changed(&self, id: &str) {
            self.states.borrow_mut().push(id.to_string());
        }
        fn console_changed(&self, id: &str) {
            self.consoles.borrow_mut().push(id.to_string());
        }
    }

    /// A folder removed when the test ends.
    struct Temp(PathBuf);

    impl Temp {
        fn new(name: &str) -> Temp {
            use std::sync::atomic::{AtomicUsize, Ordering};
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "hew-shell-plugins-{}-{name}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("temp folder");
            Temp(path)
        }
    }

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    const NOW: u64 = 1_000_000;

    fn manifest(id: &str, version: &str, permissions: &[&str], optional: &[&str]) -> String {
        serde_json::json!({
            "manifest": 1,
            "id": id,
            "name": "Test Plugin",
            "version": version,
            "api": 1,
            "main": "main.js",
            "permissions": permissions,
            "optionalPermissions": optional,
        })
        .to_string()
    }

    /// An unpacked plugin folder under `root`.
    fn folder(root: &Path, name: &str, manifest: &str) -> PathBuf {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("hew-plugin.json"), manifest).unwrap();
        std::fs::write(dir.join("main.js"), "// nothing\n").unwrap();
        dir
    }

    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = !0u32;
        for &b in bytes {
            crc ^= u32::from(b);
            for _ in 0..8 {
                crc = if crc & 1 == 1 {
                    (crc >> 1) ^ 0xEDB8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }

    /// A stored (uncompressed) zip of `files`, written by hand so the tests
    /// need no zip writer.
    fn stored_zip(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut central = Vec::new();
        for (name, data) in files {
            let offset = out.len() as u32;
            let crc = crc32(data);
            let size = data.len() as u32;
            let name_len = name.len() as u16;
            out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
            out.extend_from_slice(&20u16.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes()); // flags
            out.extend_from_slice(&0u16.to_le_bytes()); // stored
            out.extend_from_slice(&0u16.to_le_bytes()); // time
            out.extend_from_slice(&0x21u16.to_le_bytes()); // 1980-01-01
            out.extend_from_slice(&crc.to_le_bytes());
            out.extend_from_slice(&size.to_le_bytes());
            out.extend_from_slice(&size.to_le_bytes());
            out.extend_from_slice(&name_len.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(data);

            central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
            central.extend_from_slice(&20u16.to_le_bytes()); // made by (no unix mode)
            central.extend_from_slice(&20u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0x21u16.to_le_bytes());
            central.extend_from_slice(&crc.to_le_bytes());
            central.extend_from_slice(&size.to_le_bytes());
            central.extend_from_slice(&size.to_le_bytes());
            central.extend_from_slice(&name_len.to_le_bytes());
            central.extend_from_slice(&[0; 12]); // extra, comment, disk, attributes
            central.extend_from_slice(&offset.to_le_bytes());
            central.extend_from_slice(name.as_bytes());
        }
        let central_offset = out.len() as u32;
        let central_size = central.len() as u32;
        let count = files.len() as u16;
        out.extend_from_slice(&central);
        out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&count.to_le_bytes());
        out.extend_from_slice(&count.to_le_bytes());
        out.extend_from_slice(&central_size.to_le_bytes());
        out.extend_from_slice(&central_offset.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out
    }

    /// A `.hewplugin` file under `root`.
    fn package(root: &Path, file: &str, manifest: &str) -> PathBuf {
        let path = root.join(file);
        let bytes = stored_zip(&[
            ("hew-plugin.json", manifest.as_bytes()),
            ("main.js", b"// nothing\n"),
            ("icon.svg", b"<svg/>"),
        ]);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    /// A temp folder holding a store and sources, and the state over it,
    /// with developer mode on so folders install (the tests that need it
    /// off turn it off).
    fn setup(name: &str) -> (Temp, PluginState) {
        let temp = Temp::new(name);
        let state = PluginState::new(Some(PluginDir::at(temp.0.join("store"))));
        state.dev_mode.store(true, Ordering::Release);
        (temp, state)
    }

    fn refusal_name(e: &str) -> &str {
        e.split(':').next().unwrap_or(e)
    }

    /// Picks and stages an unpacked folder from the main window.
    fn stage_folder(state: &PluginState, path: &Path, now: u64) -> Result<PendingView, String> {
        let picked = record_pick(state, path, PickKind::Folder)?;
        stage_dir("main", state, &picked, now)
    }

    /// Installs `id` at `version` from a folder, answering yes.
    fn install(
        state: &PluginState,
        root: &Path,
        id: &str,
        version: &str,
        perms: &[&str],
        opt: &[&str],
    ) -> Installed {
        let src = folder(
            root,
            &format!("{id}-{version}"),
            &manifest(id, version, perms, opt),
        );
        let view = stage_folder(state, &src, NOW).expect("stage");
        commit(
            "main",
            state,
            &view.token,
            true,
            &Stub::answering(true),
            NOW,
        )
        .expect("commit")
        .expect("installed")
    }

    fn installed_ids(state: &PluginState) -> Vec<String> {
        list("main", state, &Stub::answering(true))
            .unwrap()
            .into_iter()
            .map(|l| match l {
                Listed::Installed { installed, .. } => installed.id,
                Listed::Unreadable { id, .. } => id,
            })
            .collect()
    }

    #[test]
    fn only_document_windows_and_settings_may_call() {
        for ok in ["main", "main-2", "main-17", "settings"] {
            assert!(window_allowed(ok), "{ok}");
        }
        for refused in [
            "library",
            "plugin-x",
            "",
            "main-",
            "main-x",
            "mainx",
            "Main",
            "settings-2",
        ] {
            assert!(!window_allowed(refused), "{refused:?}");
        }
    }

    #[test]
    fn every_command_refuses_another_window_first() {
        let (temp, state) = setup("label");
        let ui = Stub::answering(true);
        let src = folder(
            &temp.0,
            "src",
            &manifest("com.example.a", "1.0.0", &[], &[]),
        );
        let pkg = package(
            &temp.0,
            "a.hewplugin",
            &manifest("com.example.a", "1.0.0", &[], &[]),
        );
        let file = record_pick(&state, &pkg, PickKind::Package).unwrap();
        let dir_pick = record_pick(&state, &src, PickKind::Folder).unwrap();
        let id = "com.example.a";
        for label in ["library", "plugin-x", ""] {
            let results: Vec<Result<(), String>> = vec![
                list(label, &state, &Stub::answering(true)).map(drop),
                stage_file(label, &state, &file, NOW).map(drop),
                stage_dir(label, &state, &dir_pick, NOW).map(drop),
                commit(label, &state, "t", true, &ui, NOW).map(drop),
                set_enabled(label, &state, id, true, &ui).map(drop),
                set_grant(label, &state, id, &[], &[], &ui).map(drop),
                uninstall(label, &state, id, &ui),
                rollback(label, &state, id, &ui).map(drop),
                read_file(label, &state, id, "main.js").map(drop),
                storage_get(label, &state, id, "k").map(drop),
                storage_set(label, &state, id, "k", "v", &ui),
                storage_delete(label, &state, id, "k", &ui),
                storage_list(label, &state, id).map(drop),
            ];
            for r in results {
                assert_eq!(refusal_name(&r.unwrap_err()), "window_not_allowed");
            }
        }
        assert_eq!(ui.asked(), 0);
        assert_eq!(ui.changed.get(), 0);
        assert!(ui.stored.borrow().is_empty());
        // A refused window does not use up the user's picks.
        stage_file("settings", &state, &file, NOW).expect("the pick is still there");
        stage_dir("main-2", &state, &dir_pick, NOW).expect("the pick is still there");
        assert!(!temp.0.join("store").join(id).exists());
    }

    #[test]
    fn a_bad_id_is_refused_before_anything_else() {
        // No store folder at all: an id check that came after the folder
        // lookup would answer no_plugin_dir instead.
        let state = PluginState::new(None);
        let ui = Stub::answering(true);
        for bad in [
            "../x", "a", "A.b", "hew.x", "", "a.b/../c", "a..b", "a.b\\c",
        ] {
            let results: Vec<Result<(), String>> = vec![
                set_enabled("main", &state, bad, true, &ui).map(drop),
                set_grant("main", &state, bad, &["model.read".into()], &[], &ui).map(drop),
                uninstall("main", &state, bad, &ui),
                rollback("main", &state, bad, &ui).map(drop),
                read_file("main", &state, bad, "main.js").map(drop),
                storage_get("main", &state, bad, "k").map(drop),
                storage_set("main", &state, bad, "k", "v", &ui),
                storage_delete("main", &state, bad, "k", &ui),
                storage_list("main", &state, bad).map(drop),
            ];
            for r in results {
                let e = r.unwrap_err();
                assert!(
                    matches!(
                        refusal_name(&e),
                        "invalid_plugin_id" | "reserved_plugin_id" | "device_plugin_id"
                    ),
                    "{bad:?}: {e}"
                );
            }
        }
        assert_eq!(ui.asked(), 0);
        assert_eq!(ui.changed.get(), 0);

        // And with a real store, nothing is created or read for a bad id.
        let (temp, state) = setup("bad-id");
        for bad in ["../x", "a", "A.b", "hew.x"] {
            assert!(uninstall("main", &state, bad, &ui).is_err());
            assert!(set_enabled("main", &state, bad, false, &ui).is_err());
            assert!(rollback("main", &state, bad, &ui).is_err());
            assert!(storage_set("main", &state, bad, "k", "v", &ui).is_err());
        }
        assert!(!temp.0.join("store").exists());
    }

    #[test]
    fn staging_an_unpicked_path_is_refused() {
        let (temp, state) = setup("unpicked");
        let src = folder(
            &temp.0,
            "src",
            &manifest("com.example.a", "1.0.0", &[], &[]),
        );
        let pkg = package(
            &temp.0,
            "a.hewplugin",
            &manifest("com.example.a", "1.0.0", &[], &[]),
        );
        let src_s = src.to_string_lossy().into_owned();
        let pkg_s = pkg.to_string_lossy().into_owned();
        for r in [
            stage_file("main", &state, &pkg_s, NOW),
            stage_dir("main", &state, &src_s, NOW),
            stage_file("main", &state, "/nonexistent/a.hewplugin", NOW),
        ] {
            assert_eq!(refusal_name(&r.unwrap_err()), "path_not_picked");
        }
        // A pick of one kind does not stage as the other.
        record_pick(&state, &pkg, PickKind::Package).unwrap();
        record_pick(&state, &src, PickKind::Folder).unwrap();
        assert_eq!(
            refusal_name(&stage_dir("main", &state, &pkg_s, NOW).unwrap_err()),
            "path_not_picked"
        );
        assert_eq!(
            refusal_name(&stage_file("main", &state, &src_s, NOW).unwrap_err()),
            "path_not_picked"
        );
        // A folder is not recorded as a package pick, nor a file as a folder.
        assert!(record_pick(&state, &src, PickKind::Package).is_err());
        assert!(record_pick(&state, &pkg, PickKind::Folder).is_err());
        assert!(installed_ids(&state).is_empty());
    }

    #[test]
    fn a_pick_is_used_once() {
        let (temp, state) = setup("once");
        let pkg = package(
            &temp.0,
            "a.hewplugin",
            &manifest("com.example.a", "1.0.0", &[], &[]),
        );
        let picked = record_pick(&state, &pkg, PickKind::Package).unwrap();
        let view = stage_file("main", &state, &picked, NOW).expect("first stage");
        assert_eq!(view.consent.id, "com.example.a");
        assert_eq!(
            refusal_name(&stage_file("main", &state, &picked, NOW).unwrap_err()),
            "path_not_picked"
        );

        // Used up even when the package is refused.
        let junk = temp.0.join("junk.hewplugin");
        std::fs::write(&junk, b"not a zip").unwrap();
        let picked = record_pick(&state, &junk, PickKind::Package).unwrap();
        assert_eq!(
            refusal_name(&stage_file("main", &state, &picked, NOW).unwrap_err()),
            "package_unreadable"
        );
        assert_eq!(
            refusal_name(&stage_file("main", &state, &picked, NOW).unwrap_err()),
            "path_not_picked"
        );

        let src = folder(
            &temp.0,
            "src",
            &manifest("com.example.b", "1.0.0", &[], &[]),
        );
        let picked = record_pick(&state, &src, PickKind::Folder).unwrap();
        stage_dir("main", &state, &picked, NOW).expect("first stage");
        assert_eq!(
            refusal_name(&stage_dir("main", &state, &picked, NOW).unwrap_err()),
            "path_not_picked"
        );
    }

    #[test]
    fn a_picked_file_must_be_a_hewplugin() {
        let (temp, state) = setup("ext");
        let text = manifest("com.example.a", "1.0.0", &[], &[]);
        for name in ["a.zip", "a", "a.hewplugin.zip", "a.hewplug"] {
            let path = package(&temp.0, name, &text);
            let picked = record_pick(&state, &path, PickKind::Package).unwrap();
            assert_eq!(
                refusal_name(&stage_file("main", &state, &picked, NOW).unwrap_err()),
                "not_a_package",
                "{name}"
            );
            // Refused and used up.
            assert_eq!(
                refusal_name(&stage_file("main", &state, &picked, NOW).unwrap_err()),
                "path_not_picked"
            );
        }
        let upper = package(&temp.0, "B.HEWPLUGIN", &text);
        let picked = record_pick(&state, &upper, PickKind::Package).unwrap();
        stage_file("main", &state, &picked, NOW).expect("the extension is matched without case");
    }

    #[test]
    fn a_package_over_the_cap_is_refused_on_the_bytes_read() {
        let (temp, state) = setup("big");
        let path = temp.0.join("big.hewplugin");
        let file = std::fs::File::create(&path).unwrap();
        file.set_len(plugin_host::MAX_PACKAGE_BYTES as u64 + 1)
            .unwrap();
        drop(file);
        let picked = record_pick(&state, &path, PickKind::Package).unwrap();
        assert_eq!(
            refusal_name(&stage_file("main", &state, &picked, NOW).unwrap_err()),
            "package_too_large"
        );
    }

    #[test]
    fn a_no_in_the_native_dialog_installs_nothing() {
        let (temp, state) = setup("no");
        let pkg = package(
            &temp.0,
            "a.hewplugin",
            &manifest("com.example.a", "1.0.0", &["model.read"], &[]),
        );
        let picked = record_pick(&state, &pkg, PickKind::Package).unwrap();
        let view = stage_file("main", &state, &picked, NOW).unwrap();
        let ui = Stub::answering(false);
        assert_eq!(
            commit("main", &state, &view.token, true, &ui, NOW),
            Ok(None)
        );
        assert_eq!(ui.asked(), 1);
        {
            let asked = ui.asked.borrow();
            let (title, body, accept) = &asked[0];
            assert_eq!(title, "Install this plugin?");
            assert_eq!(accept, "Install");
            // The body is what this side holds, not anything from the page.
            assert_eq!(body, &plugin_host::consent_summary(&view.consent));
            assert!(body.contains(plugin_host::UNSIGNED_LINE));
            assert!(body.contains("Read the model"));
        }
        assert_eq!(ui.changed.get(), 0);
        assert!(installed_ids(&state).is_empty());
        assert!(!temp.0.join("store").join("com.example.a").exists());
        // The pending install is gone.
        assert_eq!(
            refusal_name(
                &commit(
                    "main",
                    &state,
                    &view.token,
                    true,
                    &Stub::answering(true),
                    NOW
                )
                .unwrap_err()
            ),
            "pending_unknown"
        );
    }

    #[test]
    fn a_yes_installs_and_tells_every_window() {
        let (temp, state) = setup("yes");
        let pkg = package(
            &temp.0,
            "a.hewplugin",
            &manifest("com.example.a", "1.0.0", &["model.read"], &[]),
        );
        let picked = record_pick(&state, &pkg, PickKind::Package).unwrap();
        let view = stage_file("main", &state, &picked, NOW).unwrap();
        assert_eq!(
            view.consent.source,
            InstallSource::File {
                name: "a.hewplugin".into()
            }
        );
        let ui = Stub::answering(true);
        let installed = commit("main", &state, &view.token, true, &ui, NOW)
            .unwrap()
            .expect("installed");
        assert_eq!(installed.id, "com.example.a");
        assert_eq!(ui.asked(), 1);
        assert_eq!(ui.changed.get(), 1);
        assert_eq!(installed_ids(&state), ["com.example.a"]);
        assert_eq!(
            read_file("main", &state, "com.example.a", "main.js").unwrap(),
            b"// nothing\n"
        );
        // The token is spent.
        assert_eq!(
            refusal_name(&commit("main", &state, &view.token, true, &ui, NOW).unwrap_err()),
            "pending_unknown"
        );
        assert_eq!(ui.changed.get(), 1);
    }

    #[test]
    fn accept_false_drops_the_pending_without_asking() {
        let (temp, state) = setup("decline");
        let src = folder(
            &temp.0,
            "src",
            &manifest("com.example.a", "1.0.0", &[], &[]),
        );
        let view = stage_folder(&state, &src, NOW).unwrap();
        let ui = Stub::answering(true);
        assert_eq!(
            commit("main", &state, &view.token, false, &ui, NOW),
            Ok(None)
        );
        assert_eq!(ui.asked(), 0);
        assert_eq!(ui.changed.get(), 0);
        assert_eq!(
            refusal_name(&commit("main", &state, &view.token, true, &ui, NOW).unwrap_err()),
            "pending_unknown"
        );
        assert!(installed_ids(&state).is_empty());
    }

    #[test]
    fn an_unknown_token_is_refused_without_asking() {
        let (_temp, state) = setup("unknown");
        let ui = Stub::answering(true);
        for token in ["", "0123456789abcdef0123456789abcdef", "../x"] {
            assert_eq!(
                refusal_name(&commit("main", &state, token, true, &ui, NOW).unwrap_err()),
                "pending_unknown"
            );
        }
        assert_eq!(ui.asked(), 0);
    }

    #[test]
    fn an_expired_pending_is_refused() {
        let (temp, state) = setup("expired");
        let src = folder(
            &temp.0,
            "src",
            &manifest("com.example.a", "1.0.0", &[], &[]),
        );
        let view = stage_folder(&state, &src, NOW).unwrap();
        let ui = Stub::answering(true);
        let late = NOW + PENDING_TTL_MS + 1;
        assert_eq!(
            refusal_name(&commit("main", &state, &view.token, true, &ui, late).unwrap_err()),
            "pending_expired"
        );
        assert_eq!(ui.asked(), 0);
        assert!(installed_ids(&state).is_empty());
        // Expired installs are dropped whenever the waiting set is touched.
        let src_b = folder(&temp.0, "b", &manifest("com.example.b", "1.0.0", &[], &[]));
        let b = stage_folder(&state, &src_b, NOW).unwrap();
        let src_c = folder(&temp.0, "c", &manifest("com.example.c", "1.0.0", &[], &[]));
        stage_folder(&state, &src_c, late).unwrap();
        assert_eq!(lock(&state.pending).len(), 1);
        assert_eq!(
            refusal_name(&commit("main", &state, &b.token, true, &ui, late).unwrap_err()),
            "pending_unknown"
        );
    }

    #[test]
    fn only_a_few_installs_may_wait_at_once() {
        let (temp, state) = setup("cap");
        let mut tokens = Vec::new();
        for i in 0..MAX_PENDING {
            let id = format!("com.example.p{i}");
            let src = folder(&temp.0, &id, &manifest(&id, "1.0.0", &[], &[]));
            tokens.push(stage_folder(&state, &src, NOW).unwrap().token);
        }
        let extra = folder(
            &temp.0,
            "extra",
            &manifest("com.example.extra", "1.0.0", &[], &[]),
        );
        assert_eq!(
            refusal_name(&stage_folder(&state, &extra, NOW).unwrap_err()),
            "too_many_pending"
        );
        // Staging a plugin that is already waiting replaces it.
        let again = folder(
            &temp.0,
            "again",
            &manifest("com.example.p0", "1.0.1", &[], &[]),
        );
        let replaced = stage_folder(&state, &again, NOW).unwrap();
        assert_eq!(lock(&state.pending).len(), MAX_PENDING);
        assert_eq!(
            refusal_name(
                &commit(
                    "main",
                    &state,
                    &tokens[0],
                    false,
                    &Stub::answering(true),
                    NOW
                )
                .unwrap_err()
            ),
            "pending_unknown"
        );
        // Answering one makes room.
        commit(
            "main",
            &state,
            &replaced.token,
            false,
            &Stub::answering(true),
            NOW,
        )
        .unwrap();
        stage_folder(&state, &extra, NOW).expect("room again");
        // So does time.
        let late = NOW + PENDING_TTL_MS + 1;
        let later = folder(
            &temp.0,
            "later",
            &manifest("com.example.later", "1.0.0", &[], &[]),
        );
        stage_folder(&state, &later, late).expect("the old ones expired");
        assert_eq!(lock(&state.pending).len(), 1);
    }

    #[test]
    fn widening_a_grant_asks_and_narrowing_does_not() {
        let (temp, state) = setup("grant");
        let id = "com.example.g";
        let installed = install(
            &state,
            &temp.0,
            id,
            "1.0.0",
            &["model.read", "view"],
            &["documents.save"],
        );
        assert_eq!(installed.grant.len(), 2);

        // Narrowing: no dialog.
        let ui = Stub::answering(false);
        let narrowed = set_grant("main", &state, id, &["model.read".into()], &[], &ui).unwrap();
        assert_eq!(narrowed.grant.len(), 1);
        assert_eq!(ui.asked(), 0);
        assert_eq!(ui.changed.get(), 1);

        // Widening with a no: asked, nothing changes, nothing emitted.
        let no = Stub::answering(false);
        let wide: Vec<String> = vec!["model.read".into(), "view".into()];
        let kept = set_grant("main", &state, id, &wide, &[], &no).unwrap();
        assert_eq!(kept.grant.len(), 1);
        assert_eq!(no.asked(), 1);
        assert_eq!(no.changed.get(), 0);
        {
            let asked = no.asked.borrow();
            let (title, body, accept) = &asked[0];
            assert_eq!(title, "Allow more for this plugin?");
            assert_eq!(accept, "Allow");
            let view_line =
                plugin_host::permission_line(&PermissionName::parse("view").unwrap()).text;
            assert!(body.contains(&view_line), "{body}");
            let read_line =
                plugin_host::permission_line(&PermissionName::parse("model.read").unwrap()).text;
            assert!(!body.contains(&read_line), "only the added lines: {body}");
        }
        let (on_disk, _) = store::installed(state.dir().unwrap(), id).unwrap();
        assert_eq!(on_disk.grant.len(), 1);

        // Widening with a yes.
        let yes = Stub::answering(true);
        let widened =
            set_grant("main", &state, id, &wide, &["documents.save".into()], &yes).unwrap();
        assert_eq!(widened.grant.len(), 2);
        assert_eq!(widened.optional_granted.len(), 1);
        assert_eq!(yes.asked(), 1);
        assert_eq!(yes.changed.get(), 1);

        // The same grant again adds nothing: no dialog.
        let same = Stub::answering(false);
        set_grant("main", &state, id, &wide, &["documents.save".into()], &same).unwrap();
        assert_eq!(same.asked(), 0);

        // Anything the manifest does not declare is refused before asking.
        let ask = Stub::answering(true);
        for (grant, optional) in [
            (vec!["documents.save".to_string()], vec![]),
            (vec![], vec!["view".to_string()]),
            (vec!["network:evil.example".to_string()], vec![]),
            (vec!["not-a-permission".to_string()], vec![]),
        ] {
            let e = set_grant("main", &state, id, &grant, &optional, &ask).unwrap_err();
            assert_eq!(
                refusal_name(&e),
                "unknown_permission",
                "{grant:?} {optional:?}"
            );
        }
        assert_eq!(ask.asked(), 0);
        assert_eq!(ask.changed.get(), 0);
    }

    #[test]
    fn changes_to_the_store_tell_every_window() {
        let (temp, state) = setup("emit");
        let id = "com.example.e";
        install(&state, &temp.0, id, "1.0.0", &[], &[]);

        // An update asks as an update.
        let src = folder(&temp.0, "v2", &manifest(id, "2.0.0", &[], &[]));
        let view = stage_folder(&state, &src, NOW).unwrap();
        let ui = Stub::answering(true);
        commit("main", &state, &view.token, true, &ui, NOW).unwrap();
        assert_eq!(ui.asked.borrow()[0].0, "Update this plugin?");
        assert_eq!(ui.asked.borrow()[0].2, "Update");

        let ui = Stub::answering(true);
        assert_eq!(
            set_enabled("settings", &state, id, false, &ui).unwrap(),
            Vec::<String>::new()
        );
        assert_eq!(ui.changed.get(), 1);
        let back = rollback("settings", &state, id, &ui).unwrap();
        assert_eq!(back.version.to_string(), "1.0.0");
        assert_eq!(ui.changed.get(), 2);
        uninstall("settings", &state, id, &ui).unwrap();
        assert_eq!(ui.changed.get(), 3);
        // The rollback and the uninstall asked; turning it off did not.
        assert_eq!(ui.asked(), 2);
        assert!(installed_ids(&state).is_empty());

        // A refused change tells no one.
        assert!(uninstall("settings", &state, id, &ui).is_err());
        assert!(rollback("settings", &state, id, &ui).is_err());
        assert!(set_enabled("settings", &state, id, true, &ui).is_err());
        assert_eq!(ui.changed.get(), 3);
        assert_eq!(ui.asked(), 2, "refused before asking");
    }

    /// Installs `id` 1.0.0 with `model.read` and `view`, updates it to
    /// 2.0.0, then narrows 2.0.0's grant to `model.read`.
    fn two_versions(state: &PluginState, root: &Path, id: &str) {
        let perms = ["model.read", "view"];
        install(state, root, id, "1.0.0", &perms, &[]);
        install(state, root, id, "2.0.0", &perms, &[]);
        set_grant(
            "main",
            state,
            id,
            &["model.read".into()],
            &[],
            &Stub::answering(true),
        )
        .unwrap();
    }

    #[test]
    fn a_rollback_asks_first_and_a_yes_rolls_back() {
        let (temp, state) = setup("rollback-yes");
        let id = "com.example.r";
        two_versions(&state, &temp.0, id);
        let ui = Stub::answering(true);
        let back = rollback("main", &state, id, &ui).unwrap();
        assert_eq!(back.version.to_string(), "1.0.0");
        assert_eq!(ui.asked(), 1);
        assert_eq!(ui.changed.get(), 1);
        let asked = ui.asked.borrow();
        let (title, body, accept) = &asked[0];
        assert_eq!(title, "Roll back this plugin?");
        assert_eq!(accept, "Roll back");
        // The wider grant the old version brings back is shown.
        let view_line = plugin_host::permission_line(&PermissionName::parse("view").unwrap()).text;
        assert!(body.contains(&view_line), "{body}");
        assert!(body.contains("2.0.0") && body.contains("1.0.0"), "{body}");
    }

    #[test]
    fn a_no_to_a_rollback_changes_nothing() {
        let (temp, state) = setup("rollback-no");
        let id = "com.example.r";
        two_versions(&state, &temp.0, id);
        let dir = state.dir().unwrap();
        let before = (
            store::installed(dir, id).unwrap(),
            store::previous(dir, id).unwrap(),
        );
        let ui = Stub::answering(false);
        assert_eq!(
            rollback("main", &state, id, &ui).unwrap_err(),
            "rollback_declined: The plugin was left as it is."
        );
        assert_eq!(ui.asked(), 1);
        assert_eq!(ui.changed.get(), 0);
        let after = (
            store::installed(dir, id).unwrap(),
            store::previous(dir, id).unwrap(),
        );
        assert_eq!(before, after);
        assert_eq!(after.0 .0.version.to_string(), "2.0.0");
    }

    #[test]
    fn a_rollback_without_a_previous_version_is_refused_before_asking() {
        let (temp, state) = setup("rollback-none");
        let id = "com.example.r";
        install(&state, &temp.0, id, "1.0.0", &[], &[]);
        let ui = Stub::answering(true);
        let e = rollback("main", &state, id, &ui).unwrap_err();
        assert_eq!(refusal_name(&e), "no_previous_version");
        let e = rollback("main", &state, "com.example.missing", &ui).unwrap_err();
        assert_eq!(refusal_name(&e), "not_installed");
        assert_eq!(ui.asked(), 0);
        assert_eq!(ui.changed.get(), 0);
    }

    /// A [`Ui`] that changes the store while its dialog is open, then says
    /// yes.
    struct Meddler<'a> {
        state: &'a PluginState,
        id: &'a str,
    }

    impl Ui for Meddler<'_> {
        fn confirm(&self, _: &str, _: &str, _: &str) -> bool {
            // Another window rolls back while this dialog is up.
            store::rollback(self.state.dir().unwrap(), self.id, running_version()).unwrap();
            true
        }
        fn changed(&self) {
            panic!("a refused rollback must not emit");
        }
        fn storage_changed(&self, _: StorageChanged) {
            panic!("a rollback changes no saved data");
        }
        fn dev_changed(&self, _: DevChanged) {
            panic!("a refused rollback must not emit");
        }
    }

    #[test]
    fn a_rollback_is_refused_if_the_versions_change_while_asking() {
        let (temp, state) = setup("rollback-race");
        let id = "com.example.r";
        two_versions(&state, &temp.0, id);
        let ui = Meddler { state: &state, id };
        assert_eq!(
            refusal_name(&rollback("main", &state, id, &ui).unwrap_err()),
            "plugin_changed"
        );
        // Only the meddler's own rollback happened.
        let (current, _) = store::installed(state.dir().unwrap(), id).unwrap();
        assert_eq!(current.version.to_string(), "1.0.0");
    }

    #[test]
    fn the_pick_lookup_only_compares_strings() {
        // Pure over strings: none of these exists, and none is resolved.
        let mut picks = VecDeque::new();
        remember_pick(
            &mut picks,
            Pick {
                shown: "/picked/a.hewplugin".into(),
                path: PathBuf::from("/picked/a.hewplugin"),
                kind: PickKind::Package,
                request: None,
            },
        );
        for path in [
            "/nonexistent/a.hewplugin",
            "/picked/./a.hewplugin",
            "/picked//a.hewplugin",
            "/picked/a.hewplugin/",
            "/PICKED/a.hewplugin",
            "",
        ] {
            assert_eq!(
                take_from(&mut picks, path, PickKind::Package),
                None,
                "{path:?}"
            );
        }
        assert_eq!(
            take_from(&mut picks, "/picked/a.hewplugin", PickKind::Folder),
            None
        );
        assert_eq!(picks.len(), 1);
        assert_eq!(
            take_from(&mut picks, "/picked/a.hewplugin", PickKind::Package),
            Some((PathBuf::from("/picked/a.hewplugin"), None))
        );
        assert!(picks.is_empty());
    }

    #[test]
    fn only_the_exact_picked_string_stages() {
        let (temp, state) = setup("exact");
        let pkg = package(
            &temp.0,
            "a.hewplugin",
            &manifest("com.example.a", "1.0.0", &[], &[]),
        );
        let picked = record_pick(&state, &pkg, PickKind::Package).unwrap();
        // Another string that opens the picked file: a symlink, or on
        // Windows, where making one takes a privilege, the same path in
        // capitals.
        #[cfg(unix)]
        let alias = {
            let link = temp.0.join("link.hewplugin");
            std::os::unix::fs::symlink(&pkg, &link).unwrap();
            link.to_string_lossy().into_owned()
        };
        #[cfg(windows)]
        let alias = picked.to_uppercase();
        assert_ne!(alias, picked);
        assert!(
            std::path::Path::new(&alias).is_file(),
            "{alias} opens the pick"
        );
        for path in [alias, "/nonexistent/a.hewplugin".into()] {
            assert_eq!(
                refusal_name(&stage_file("main", &state, &path, NOW).unwrap_err()),
                "path_not_picked",
                "{path}"
            );
        }
        stage_file("main", &state, &picked, NOW).expect("the pick was not used up");
    }

    #[test]
    fn only_a_few_picks_are_remembered() {
        let (temp, state) = setup("picks");
        let text = manifest("com.example.a", "1.0.0", &[], &[]);
        let picks: Vec<String> = (0..=MAX_PICKS)
            .map(|i| {
                let path = package(&temp.0, &format!("p{i}.hewplugin"), &text);
                record_pick(&state, &path, PickKind::Package).unwrap()
            })
            .collect();
        assert_eq!(lock(&state.picked).len(), MAX_PICKS);
        assert_eq!(
            refusal_name(&stage_file("main", &state, &picks[0], NOW).unwrap_err()),
            "path_not_picked"
        );
        stage_file("main", &state, &picks[MAX_PICKS], NOW).expect("the newest is kept");
    }

    /// A [`Ui`] that runs `during` while its dialog is open, then answers
    /// `answer`, and counts change events.
    struct During<'a> {
        during: RefCell<Option<Box<dyn FnOnce() + 'a>>>,
        answer: bool,
        changed: Cell<usize>,
    }

    impl<'a> During<'a> {
        fn new(answer: bool, during: impl FnOnce() + 'a) -> During<'a> {
            During {
                during: RefCell::new(Some(Box::new(during))),
                answer,
                changed: Cell::new(0),
            }
        }
    }

    impl Ui for During<'_> {
        fn confirm(&self, _: &str, _: &str, _: &str) -> bool {
            if let Some(f) = self.during.borrow_mut().take() {
                f();
            }
            self.answer
        }
        fn changed(&self) {
            self.changed.set(self.changed.get() + 1);
        }
        fn storage_changed(&self, _: StorageChanged) {}
        fn dev_changed(&self, _: DevChanged) {}
    }

    #[test]
    fn one_native_confirmation_at_a_time() {
        let (temp, state) = setup("one-dialog");
        let g = "com.example.g";
        install(&state, &temp.0, g, "1.0.0", &["model.read", "view"], &[]);
        set_grant(
            "main",
            &state,
            g,
            &["model.read".into()],
            &[],
            &Stub::answering(true),
        )
        .unwrap();
        let r = "com.example.r";
        two_versions(&state, &temp.0, r);
        let off = "com.example.off";
        install(&state, &temp.0, off, "1.0.0", &[], &[]);
        set_enabled("main", &state, off, false, &Stub::answering(true)).unwrap();
        let gone = "com.example.gone";
        install(&state, &temp.0, gone, "1.0.0", &[], &[]);
        let src = folder(
            &temp.0,
            "waiting",
            &manifest("com.example.w", "1.0.0", &[], &[]),
        );
        let waiting = stage_folder(&state, &src, NOW).unwrap();
        let other = folder(
            &temp.0,
            "first",
            &manifest("com.example.f", "1.0.0", &[], &[]),
        );
        let first = stage_folder(&state, &other, NOW).unwrap();

        let inner: RefCell<Vec<String>> = RefCell::new(Vec::new());
        let ui = During::new(false, || {
            let yes = Stub::answering(true);
            let wide = ["model.read".to_string(), "view".to_string()];
            let results = [
                commit("main", &state, &waiting.token, true, &yes, NOW).map(drop),
                set_grant("main", &state, g, &wide, &[], &yes).map(drop),
                rollback("main", &state, r, &yes).map(drop),
                set_enabled("main", &state, off, true, &yes).map(drop),
                uninstall("main", &state, gone, &yes),
            ];
            // Turning off asks nothing, so it is not held up.
            set_enabled("main", &state, gone, false, &yes).expect("no question");
            for result in results {
                inner.borrow_mut().push(result.unwrap_err());
            }
            assert_eq!(yes.asked(), 0);
            assert_eq!(yes.changed.get(), 1, "only the turn-off");
        });
        assert_eq!(
            commit("main", &state, &first.token, true, &ui, NOW),
            Ok(None)
        );
        drop(ui);
        let inner = inner.into_inner();
        assert_eq!(inner.len(), 5);
        for e in &inner {
            assert_eq!(
                e,
                "confirmation_open: Answer the question Hew is already asking first."
            );
        }
        // The install refused for the open dialog is still waiting, and the
        // flag is released once the first dialog is answered.
        let yes = Stub::answering(true);
        commit("main", &state, &waiting.token, true, &yes, NOW)
            .unwrap()
            .expect("installed");
        assert_eq!(yes.asked(), 1);
        uninstall("main", &state, gone, &yes).expect("asked and removed");
        set_enabled("main", &state, off, true, &yes).expect("asked and on");
        assert_eq!(yes.asked(), 3);
    }

    /// A [`Ui`] whose dialog panics.
    struct Panics;

    impl Ui for Panics {
        fn confirm(&self, _: &str, _: &str, _: &str) -> bool {
            panic!("the dialog failed");
        }
        fn changed(&self) {}
        fn storage_changed(&self, _: StorageChanged) {}
        fn dev_changed(&self, _: DevChanged) {}
    }

    #[test]
    fn a_panic_in_the_dialog_releases_it() {
        let (temp, state) = setup("panic");
        let r = "com.example.r";
        two_versions(&state, &temp.0, r);
        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            rollback("main", &state, r, &Panics)
        }));
        assert!(panicked.is_err());
        let yes = Stub::answering(true);
        rollback("main", &state, r, &yes).expect("no dialog is held open");
        assert_eq!(yes.asked(), 1);
    }

    #[test]
    fn a_grant_that_changes_while_asking_is_refused() {
        let (temp, state) = setup("grant-race");
        let id = "com.example.g";
        install(&state, &temp.0, id, "1.0.0", &["model.read", "view"], &[]);
        set_grant(
            "main",
            &state,
            id,
            &["model.read".into()],
            &[],
            &Stub::answering(true),
        )
        .unwrap();
        let ui = During::new(true, || {
            // Another window turns it off while the dialog is up.
            store::set_enabled(state.dir().unwrap(), id, false).unwrap();
        });
        let wide = ["model.read".to_string(), "view".to_string()];
        assert_eq!(
            set_grant("main", &state, id, &wide, &[], &ui).unwrap_err(),
            "grant_changed: The plugin's permissions changed while Hew was asking. Try again."
        );
        assert_eq!(ui.changed.get(), 0);
        let (record, _) = store::installed(state.dir().unwrap(), id).unwrap();
        assert_eq!(record.grant.len(), 1);
    }

    #[test]
    fn a_stale_pending_is_refused_after_the_yes() {
        let (temp, state) = setup("stale");
        let id = "com.example.s";
        install(&state, &temp.0, id, "1.0.0", &[], &[]);
        let src = folder(&temp.0, "v2", &manifest(id, "2.0.0", &[], &[]));
        let view = stage_folder(&state, &src, NOW).unwrap();
        set_enabled("main", &state, id, false, &Stub::answering(true)).unwrap();
        let ui = Stub::answering(true);
        let e = commit("main", &state, &view.token, true, &ui, NOW).unwrap_err();
        assert_eq!(refusal_name(&e), "pending_stale");
        assert_eq!(ui.asked(), 1);
        assert_eq!(ui.changed.get(), 0);
        let (record, _) = store::installed(state.dir().unwrap(), id).unwrap();
        assert_eq!(record.version.to_string(), "1.0.0");
    }

    #[test]
    fn reading_a_file_cannot_leave_the_plugin() {
        let (temp, state) = setup("jail");
        let pkg = package(
            &temp.0,
            "a.hewplugin",
            &manifest("com.example.a", "1.0.0", &[], &[]),
        );
        let picked = record_pick(&state, &pkg, PickKind::Package).unwrap();
        let view = stage_file("main", &state, &picked, NOW).unwrap();
        commit(
            "main",
            &state,
            &view.token,
            true,
            &Stub::answering(true),
            NOW,
        )
        .unwrap();
        // A folder install reads from its folder, under the same rules.
        install(&state, &temp.0, "com.example.d", "1.0.0", &[], &[]);
        let outside = temp.0.join("store").join("secret.txt");
        std::fs::write(&outside, b"secret").unwrap();

        for id in ["com.example.a", "com.example.d"] {
            assert_eq!(
                read_file("main", &state, id, "main.js").unwrap(),
                b"// nothing\n"
            );
            let abs = outside.to_string_lossy().into_owned();
            for rel in [
                "../installed.json",
                "../hew-plugin.json",
                "../../secret.txt",
                "files/../../secret.txt",
                abs.as_str(),
                "/etc/passwd",
                "..\\installed.json",
                "a\\..\\..\\installed.json",
                "C:\\Windows\\win.ini",
                "./main.js",
                "",
                "missing.js",
            ] {
                let e = read_file("main", &state, id, rel).unwrap_err();
                assert_eq!(
                    refusal_name(&e),
                    "package_path_invalid",
                    "{id} {rel:?}: {e}"
                );
            }
        }
        // Another plugin's id does not reach this one's files.
        assert!(read_file("main", &state, "com.example.zz", "main.js").is_err());
    }

    #[test]
    fn listing_needs_an_allowed_window_and_a_folder() {
        let (_temp, state) = setup("dir");
        assert!(list("main", &state, &Stub::answering(true))
            .unwrap()
            .is_empty());
        let none = PluginState::new(None);
        assert_eq!(
            refusal_name(&list("main", &none, &Stub::answering(true)).unwrap_err()),
            "no_plugin_dir"
        );
        let ui = Stub::answering(true);
        for r in [
            storage_list("main", &none, "com.example.a").map(drop),
            storage_set("main", &none, "com.example.a", "k", "v", &ui),
        ] {
            assert_eq!(refusal_name(&r.unwrap_err()), "no_plugin_dir");
        }
    }

    #[test]
    fn the_store_folder_is_not_a_command() {
        // The page has no use for where the store is, so no command says.
        let source = include_str!("plugins.rs");
        let gone = ["plugin", "_dir("].concat();
        assert!(!source.contains(&format!("fn {gone}")));
    }

    #[test]
    fn the_running_version_is_the_stamped_one_not_the_crates() {
        // A release build stamps HEW_RELEASE_VERSION; the crate's own
        // version is the last release's and must not be what is checked.
        assert_eq!(running_version().to_string(), crate::version::VERSION);
        let source = include_str!("plugins.rs");
        let crate_version = ["env!(\"CARGO_PKG", "_VERSION\")"].concat();
        let non_test = source.split("#[cfg(test)]").next().unwrap();
        assert!(!non_test.contains(&crate_version));
        if let Some(stamp) = option_env!("HEW_RELEASE_VERSION").filter(|v| !v.is_empty()) {
            assert_eq!(running_version().to_string(), stamp);
        }
    }

    #[test]
    fn reinstalling_the_installed_version_asks_as_a_reinstall() {
        let (temp, state) = setup("again");
        let id = "com.example.again";
        install(&state, &temp.0, id, "1.0.0", &[], &[]);
        let src = folder(&temp.0, "again", &manifest(id, "1.0.0", &[], &[]));
        let view = stage_folder(&state, &src, NOW).unwrap();
        let ui = Stub::answering(true);
        commit("main", &state, &view.token, true, &ui, NOW)
            .unwrap()
            .expect("installed");
        let asked = ui.asked.borrow();
        assert_eq!(asked[0].0, "Install this plugin again?");
        assert_eq!(asked[0].2, "Install");
        assert!(
            asked[0]
                .1
                .contains("Replaces the installed copy of v1.0.0."),
            "{}",
            asked[0].1
        );
    }

    #[test]
    fn uninstalling_asks_and_a_no_removes_nothing() {
        let (temp, state) = setup("uninstall");
        let id = "com.example.u";
        install(&state, &temp.0, id, "1.0.0", &["model.read"], &[]);
        storage_set("main", &state, id, "k", "v", &Stub::answering(true)).unwrap();

        let no = Stub::answering(false);
        assert_eq!(
            uninstall("main", &state, id, &no).unwrap_err(),
            "uninstall_declined: The plugin was left installed."
        );
        assert_eq!(no.asked(), 1);
        assert_eq!(no.changed.get(), 0);
        {
            let asked = no.asked.borrow();
            let (title, body, accept) = &asked[0];
            assert_eq!(title, "Uninstall this plugin?");
            assert_eq!(accept, "Uninstall");
            let (record, manifest) = store::installed(state.dir().unwrap(), id).unwrap();
            assert_eq!(body, &plugin_host::uninstall_summary(&record, &manifest));
        }
        assert_eq!(installed_ids(&state), [id]);
        assert_eq!(
            storage_get("main", &state, id, "k").unwrap().as_deref(),
            Some("v")
        );

        let yes = Stub::answering(true);
        uninstall("main", &state, id, &yes).unwrap();
        assert_eq!(yes.changed.get(), 1);
        assert!(installed_ids(&state).is_empty());
        // Its saved data went with it.
        assert!(!temp.0.join("store").join(id).exists());
        assert_eq!(
            refusal_name(&storage_get("main", &state, id, "k").unwrap_err()),
            "not_installed"
        );
    }

    #[test]
    fn an_uninstall_the_store_would_refuse_is_refused_before_asking() {
        let (temp, state) = setup("uninstall-refused");
        let ui = Stub::answering(true);
        assert_eq!(
            refusal_name(&uninstall("main", &state, "com.example.none", &ui).unwrap_err()),
            "not_installed"
        );
        let base = "com.example.base";
        install(&state, &temp.0, base, "1.0.0", &[], &[]);
        let needs = serde_json::json!({
            "manifest": 1, "id": "com.example.needs", "name": "Needs", "version": "1.0.0",
            "api": 1, "main": "main.js", "requires": [{ "id": base, "range": "^1" }]
        })
        .to_string();
        let src = folder(&temp.0, "needs", &needs);
        let view = stage_folder(&state, &src, NOW).unwrap();
        commit(
            "main",
            &state,
            &view.token,
            true,
            &Stub::answering(true),
            NOW,
        )
        .unwrap();
        assert_eq!(
            refusal_name(&uninstall("main", &state, base, &ui).unwrap_err()),
            "dependency_missing"
        );
        assert_eq!(ui.asked(), 0);
        assert_eq!(installed_ids(&state).len(), 2);
    }

    #[test]
    fn a_plugin_whose_record_is_unreadable_can_be_uninstalled_by_its_id() {
        let (temp, state) = setup("uninstall-unreadable");
        let id = "com.example.newer";
        install(&state, &temp.0, id, "1.0.0", &[], &[]);
        std::fs::write(
            temp.0.join("store").join(id).join("installed.json"),
            br#"{"schema": 99, "id": "com.example.newer"}"#,
        )
        .unwrap();
        let ui = Stub::answering(true);
        uninstall("main", &state, id, &ui).unwrap();
        assert!(ui.asked.borrow()[0].1.starts_with("com.example.newer\n"));
        assert!(installed_ids(&state).is_empty());
    }

    #[test]
    fn turning_a_plugin_on_asks_and_turning_it_off_does_not() {
        let (temp, state) = setup("enable");
        let id = "com.example.on";
        install(&state, &temp.0, id, "1.0.0", &["model.read"], &[]);

        // Freshly installed: on, but not yet allowed to start with Hew, so
        // turning it on still asks.
        let off = Stub::answering(false);
        assert_eq!(set_enabled("main", &state, id, false, &off), Ok(vec![]));
        assert_eq!(off.asked(), 0, "turning off asks nothing");
        assert_eq!(off.changed.get(), 1);

        let no = Stub::answering(false);
        assert_eq!(
            set_enabled("main", &state, id, true, &no).unwrap_err(),
            "enable_declined: The plugin was left off."
        );
        assert_eq!(no.changed.get(), 0);
        let (record, manifest) = store::installed(state.dir().unwrap(), id).unwrap();
        assert!(!record.enabled);
        {
            let asked = no.asked.borrow();
            let (title, body, accept) = &asked[0];
            assert_eq!(title, "Turn on this plugin?");
            assert_eq!(accept, "Turn On");
            assert_eq!(body, &plugin_host::enable_summary(&record, &manifest));
            assert!(body.contains("Read the model"), "{body}");
        }

        let yes = Stub::answering(true);
        assert_eq!(set_enabled("main", &state, id, true, &yes), Ok(vec![]));
        assert_eq!(yes.asked(), 1);
        assert_eq!(yes.changed.get(), 1);
        let (record, _) = store::installed(state.dir().unwrap(), id).unwrap();
        assert!(record.enabled && record.startup_allowed);

        // Already on and allowed to start: nothing to ask, nothing changes.
        let again = Stub::answering(false);
        assert_eq!(set_enabled("main", &state, id, true, &again), Ok(vec![]));
        assert_eq!(again.asked(), 0);
        assert_eq!(again.changed.get(), 0);

        // Not installed: refused before asking.
        let e = set_enabled("main", &state, "com.example.none", true, &again).unwrap_err();
        assert_eq!(refusal_name(&e), "not_installed");
        assert_eq!(again.asked(), 0);
    }

    #[test]
    fn turning_on_a_plugin_that_changes_while_asking_is_refused() {
        let (temp, state) = setup("enable-race");
        let id = "com.example.on";
        install(&state, &temp.0, id, "1.0.0", &["model.read", "view"], &[]);
        set_enabled("main", &state, id, false, &Stub::answering(true)).unwrap();
        let ui = During::new(true, || {
            // Another window narrows its grant while the dialog is up.
            store::set_grant(
                state.dir().unwrap(),
                id,
                vec![PermissionName::parse("model.read").unwrap()],
                vec![],
            )
            .unwrap();
        });
        assert_eq!(
            refusal_name(&set_enabled("main", &state, id, true, &ui).unwrap_err()),
            "plugin_changed"
        );
        assert_eq!(ui.changed.get(), 0);
        let (record, _) = store::installed(state.dir().unwrap(), id).unwrap();
        assert!(!record.enabled);
    }

    #[test]
    fn saved_data_round_trips_and_tells_every_window() {
        let (temp, state) = setup("storage");
        let id = "com.example.s";
        install(&state, &temp.0, id, "1.0.0", &[], &[]);
        let ui = Stub::answering(false);
        assert_eq!(storage_get("main", &state, id, "a"), Ok(None));
        assert_eq!(storage_list("main-2", &state, id), Ok(vec![]));
        storage_set("main", &state, id, "a", "{\"n\":1}", &ui).unwrap();
        storage_set("settings", &state, id, "b", "\"x\"", &ui).unwrap();
        assert_eq!(
            storage_get("main-2", &state, id, "a").unwrap().as_deref(),
            Some("{\"n\":1}")
        );
        assert_eq!(
            storage_list("main", &state, id).unwrap(),
            ["a".to_string(), "b".to_string()]
        );
        storage_delete("main", &state, id, "a", &ui).unwrap();
        // Deleting what is not there changes nothing and tells no one.
        storage_delete("main", &state, id, "a", &ui).unwrap();
        assert_eq!(
            *ui.stored.borrow(),
            [
                StorageChanged {
                    id: id.into(),
                    keys: vec!["a".into()]
                },
                StorageChanged {
                    id: id.into(),
                    keys: vec!["b".into()]
                },
                StorageChanged {
                    id: id.into(),
                    keys: vec!["a".into()]
                },
            ]
        );
        // Saved data is not a change to the store, and asks nothing.
        assert_eq!(ui.changed.get(), 0);
        assert_eq!(ui.asked(), 0);
        assert_eq!(
            serde_json::to_value(&ui.stored.borrow()[0]).unwrap(),
            serde_json::json!({ "id": id, "keys": ["a"] })
        );
    }

    #[test]
    fn saved_data_refusals_carry_their_names_and_write_nothing() {
        let (temp, state) = setup("storage-refused");
        let id = "com.example.s";
        install(&state, &temp.0, id, "1.0.0", &[], &[]);
        let ui = Stub::answering(true);
        let big = "x".repeat(plugin_host::MAX_STORAGE_BYTES);
        assert_eq!(
            refusal_name(&storage_set("main", &state, id, "a", &big, &ui).unwrap_err()),
            "storage_quota"
        );
        let long = "k".repeat(plugin_host::MAX_STORAGE_KEY_BYTES + 1);
        for key in ["", long.as_str()] {
            assert_eq!(
                refusal_name(&storage_set("main", &state, id, key, "v", &ui).unwrap_err()),
                "storage_key_invalid"
            );
            assert_eq!(
                refusal_name(&storage_get("main", &state, id, key).unwrap_err()),
                "storage_key_invalid"
            );
        }
        // A plugin that is not installed has no saved data, and asking for
        // it creates nothing.
        let none = "com.example.none";
        assert_eq!(
            refusal_name(&storage_set("main", &state, none, "a", "v", &ui).unwrap_err()),
            "not_installed"
        );
        assert!(!temp.0.join("store").join(none).exists());
        assert!(ui.stored.borrow().is_empty());
        assert!(!temp.0.join("store").join(id).join("storage.json").exists());
    }

    // ---------------------------------------------------------------------
    // Developer mode, folder changes, reviews, the live API.
    // ---------------------------------------------------------------------

    const DEV: &str = "com.example.dev";

    /// A manifest for [`DEV`] with what a row of the change table varies.
    fn dev_manifest(
        id: &str,
        version: &str,
        perms: &[&str],
        opt: &[&str],
        startup: bool,
        command: bool,
    ) -> String {
        let mut m = serde_json::json!({
            "manifest": 1, "id": id, "name": "Dev Plugin", "version": version,
            "api": 1, "main": "main.js",
            "permissions": perms, "optionalPermissions": opt,
        });
        if startup {
            m["activation"] = serde_json::json!(["on_command", "on_startup"]);
            m["activationReason"] = serde_json::json!("Keeps a count.");
        }
        if command {
            m["contributes"] = serde_json::json!({ "commands": [{
                "id": "count", "class": "read_only", "summary": "Counts.",
                "params": { "type": "object", "additionalProperties": false },
                "result": { "type": "object", "additionalProperties": false }
            }]});
        }
        m.to_string()
    }

    fn base_manifest() -> String {
        dev_manifest(DEV, "1.0.0", &["model.read"], &["clipboard"], false, false)
    }

    /// Installs [`DEV`] from `root/src`, turns it on (so it may start with
    /// Hew) and grants its optional `clipboard`. Answers the folder.
    fn dev_install(state: &PluginState, root: &Path) -> PathBuf {
        let src = folder(root, "src", &base_manifest());
        let view = stage_folder(state, &src, NOW).expect("stage");
        let yes = Stub::answering(true);
        commit("main", state, &view.token, true, &yes, NOW)
            .expect("commit")
            .expect("installed");
        set_enabled("main", state, DEV, true, &yes).expect("on");
        set_grant(
            "main",
            state,
            DEV,
            &["model.read".into()],
            &["clipboard".into()],
            &yes,
        )
        .expect("optional granted");
        src
    }

    fn record(state: &PluginState) -> (Installed, plugin_manifest::Manifest) {
        store::installed(state.dir().unwrap(), DEV).unwrap()
    }

    fn names(p: &[PermissionName]) -> Vec<&str> {
        p.iter().map(PermissionName::as_str).collect()
    }

    fn lines(l: &[plugin_host::PermissionLine]) -> Vec<&str> {
        l.iter().map(|l| l.name.as_str()).collect()
    }

    /// One row of the change table: what is written into the folder, the
    /// marker reason expected (none: a reload), and a check of the review
    /// and of the record after the review is accepted.
    struct Row {
        name: &'static str,
        edit: fn(&Path),
        marked: Option<&'static str>,
        review: Option<fn(&PendingView, &Installed, &plugin_manifest::Manifest)>,
    }

    fn write_manifest(src: &Path, text: &str) {
        std::fs::write(src.join("hew-plugin.json"), text).unwrap();
    }

    #[test]
    fn what_a_change_in_the_folder_does() {
        let rows = [
            Row {
                name: "code only",
                edit: |src| std::fs::write(src.join("main.js"), b"// edited").unwrap(),
                marked: None,
                review: None,
            },
            Row {
                name: "commands changed",
                edit: |src| {
                    write_manifest(
                        src,
                        &dev_manifest(DEV, "1.0.0", &["model.read"], &["clipboard"], false, true),
                    )
                },
                marked: None,
                review: None,
            },
            Row {
                name: "version only",
                edit: |src| {
                    write_manifest(
                        src,
                        &dev_manifest(DEV, "1.1.0", &["model.read"], &["clipboard"], false, false),
                    )
                },
                marked: None,
                review: None,
            },
            Row {
                name: "permission added",
                edit: |src| {
                    write_manifest(
                        src,
                        &dev_manifest(
                            DEV,
                            "1.0.0",
                            &["model.read", "view"],
                            &["clipboard"],
                            false,
                            false,
                        ),
                    )
                },
                marked: Some("permissions_changed"),
                review: Some(|view, after, _| {
                    let u = view.consent.update.as_ref().unwrap();
                    assert_eq!(lines(&u.added), ["view"]);
                    assert!(u.removed.is_empty());
                    assert_eq!(names(&after.grant), ["model.read", "view"]);
                    assert_eq!(names(&after.optional_granted), ["clipboard"]);
                }),
            },
            Row {
                name: "permission removed",
                edit: |src| {
                    write_manifest(
                        src,
                        &dev_manifest(DEV, "1.0.0", &[], &["clipboard"], false, false),
                    )
                },
                marked: Some("permissions_changed"),
                review: Some(|view, after, _| {
                    let u = view.consent.update.as_ref().unwrap();
                    assert!(u.added.is_empty());
                    assert_eq!(lines(&u.removed), ["model.read"]);
                    assert!(after.grant.is_empty());
                }),
            },
            Row {
                name: "optional added",
                edit: |src| {
                    write_manifest(
                        src,
                        &dev_manifest(
                            DEV,
                            "1.0.0",
                            &["model.read"],
                            &["clipboard", "library"],
                            false,
                            false,
                        ),
                    )
                },
                marked: Some("permissions_changed"),
                review: Some(|view, after, _| {
                    let u = view.consent.update.as_ref().unwrap();
                    assert_eq!(lines(&u.new_optional), ["library"]);
                    assert_eq!(names(&after.optional_granted), ["clipboard"]);
                }),
            },
            Row {
                name: "optional removed",
                edit: |src| {
                    write_manifest(
                        src,
                        &dev_manifest(DEV, "1.0.0", &["model.read"], &[], false, false),
                    )
                },
                marked: Some("permissions_changed"),
                review: Some(|view, after, _| {
                    let u = view.consent.update.as_ref().unwrap();
                    assert_eq!(lines(&u.removed_optional), ["clipboard"]);
                    assert!(after.optional_granted.is_empty());
                }),
            },
            Row {
                name: "on_startup added",
                edit: |src| {
                    write_manifest(
                        src,
                        &dev_manifest(DEV, "1.0.0", &["model.read"], &["clipboard"], true, false),
                    )
                },
                marked: Some("startup_added"),
                review: Some(|view, after, stored| {
                    assert!(view.consent.update.as_ref().unwrap().startup_newly);
                    assert!(!after.startup_allowed, "asks to start again");
                    assert!(stored
                        .activation
                        .contains(&plugin_manifest::Activation::OnStartup));
                }),
            },
            Row {
                name: "manifest broken",
                edit: |src| write_manifest(src, "{ not json"),
                marked: Some("manifest_unreadable"),
                review: None,
            },
            Row {
                name: "id changed",
                edit: |src| {
                    write_manifest(
                        src,
                        &dev_manifest(
                            "com.example.other",
                            "1.0.0",
                            &["model.read"],
                            &["clipboard"],
                            false,
                            false,
                        ),
                    )
                },
                marked: Some("id_changed"),
                review: None,
            },
        ];
        for row in rows {
            let (temp, state) = setup("change-table");
            let src = dev_install(&state, &temp.0);
            let (before, before_manifest) = record(&state);
            assert!(before.startup_allowed && names(&before.optional_granted) == ["clipboard"]);
            (row.edit)(&src);
            let ui = Stub::answering(true);
            let keep = folder_changed(&state, DEV, &ui);
            let (after, after_manifest) = record(&state);
            let events = ui.dev.borrow().clone();
            assert_eq!(events.len(), 1, "{}", row.name);
            let event = &events[0];
            assert_eq!(event.id, DEV);
            assert_eq!(event.problem, None, "{}", row.name);
            // The grant never changes before the person accepts.
            assert_eq!(after.grant, before.grant, "{}", row.name);
            assert_eq!(
                after.optional_granted, before.optional_granted,
                "{}",
                row.name
            );
            assert_eq!(
                after.installed_at_ms, before.installed_at_ms,
                "{}",
                row.name
            );
            match row.marked {
                None => {
                    assert!(keep, "{}", row.name);
                    assert!(!event.manifest_changed, "{}", row.name);
                    assert!(!store::needs_consent(&after), "{}", row.name);
                    assert_eq!(after_manifest.permissions, before_manifest.permissions);
                    match row.name {
                        "code only" => {
                            assert_eq!(after_manifest, before_manifest);
                            assert_eq!(ui.changed.get(), 0, "nothing in the store changed");
                            assert_eq!(
                                read_file("main", &state, DEV, "main.js").unwrap(),
                                b"// edited"
                            );
                        }
                        "commands changed" => {
                            assert_eq!(after_manifest.contributes.commands.len(), 1);
                            assert_eq!(ui.changed.get(), 1);
                        }
                        "version only" => {
                            assert_eq!(after.version.to_string(), "1.1.0");
                            assert_eq!(after_manifest.version.to_string(), "1.1.0");
                            assert_eq!(ui.changed.get(), 1);
                        }
                        other => unreachable!("{other}"),
                    }
                }
                Some(reason) => {
                    assert!(keep, "{}: still watched, so undoing it clears it", row.name);
                    assert!(event.manifest_changed, "{}", row.name);
                    assert_eq!(event.reason.as_deref(), Some(reason), "{}", row.name);
                    assert!(store::needs_consent(&after), "{}", row.name);
                    assert_eq!(store::needs_consent_reason(&after), Some(reason));
                    assert_eq!(
                        after_manifest, before_manifest,
                        "{}: stored copy kept",
                        row.name
                    );
                    assert_eq!(after.version, before.version);
                    for rel in ["main.js", "hew-plugin.json"] {
                        assert_eq!(
                            refusal_name(&read_file("main", &state, DEV, rel).unwrap_err()),
                            "dev_consent_pending",
                            "{}",
                            row.name
                        );
                    }
                    // A second change with the same reason says nothing more.
                    let again = Stub::answering(true);
                    assert!(folder_changed(&state, DEV, &again));
                    assert!(again.dev.borrow().is_empty());
                    assert!(store::needs_consent(&record(&state).0));
                }
            }
            match (row.marked, row.review) {
                (Some(reason), Some(check)) => {
                    let view = review("main", &state, DEV, NOW).expect("review");
                    assert!(view.consent.review);
                    let yes = Stub::answering(true);
                    let installed = commit("main", &state, &view.token, true, &yes, NOW)
                        .expect("accepted")
                        .expect("a record");
                    {
                        let asked = yes.asked.borrow();
                        let (title, body, accept) = &asked[0];
                        assert_eq!(title, "Allow this plugin's changes?");
                        assert_eq!(accept, "Allow");
                        assert!(body.contains(plugin_host::FOLDER_LINE), "{body}");
                        let line = view.consent.review_line.clone().expect("a review says why");
                        if reason != "command_name_taken" {
                            assert_eq!(
                                line,
                                plugin_host::review_line(Some(reason), None),
                                "{}",
                                row.name
                            );
                        }
                        assert!(body.contains(&line), "{body}");
                    }
                    assert_eq!(
                        yes.dev.borrow().as_slice(),
                        [DevChanged {
                            id: DEV.into(),
                            manifest_changed: false,
                            reason: None,
                            problem: None
                        }]
                    );
                    let (now, stored) = record(&state);
                    assert_eq!(now, installed);
                    assert!(!store::needs_consent(&now), "{}", row.name);
                    assert_eq!(now.installed_at_ms, before.installed_at_ms);
                    assert_eq!(stored.permissions, now.grant, "{}", row.name);
                    assert!(!temp.0.join("store").join(DEV).join("previous").exists());
                    check(&view, &now, &stored);
                    read_file("main", &state, DEV, "main.js").expect("reads again");
                }
                (Some("manifest_unreadable" | "id_changed"), None) => {
                    let e = review("main", &state, DEV, NOW).unwrap_err();
                    if row.marked == Some("id_changed") {
                        assert_eq!(refusal_name(&e), "folder_id_changed");
                    } else {
                        assert_eq!(refusal_name(&e), "manifest_malformed", "{e}");
                    }
                    let (still, stored) = record(&state);
                    assert!(store::needs_consent(&still));
                    assert_eq!(
                        (still.grant, stored),
                        (before.grant.clone(), before_manifest.clone())
                    );
                }
                (None, None) => {
                    assert_eq!(
                        refusal_name(&review("main", &state, DEV, NOW).unwrap_err()),
                        "no_review_pending"
                    );
                }
                _ => unreachable!(),
            }
        }
    }

    #[test]
    fn a_declined_or_expired_review_leaves_the_marker() {
        let (temp, state) = setup("review-no");
        let src = dev_install(&state, &temp.0);
        write_manifest(
            &src,
            &dev_manifest(
                DEV,
                "1.0.0",
                &["model.read", "view"],
                &["clipboard"],
                false,
                false,
            ),
        );
        folder_changed(&state, DEV, &Stub::answering(true));
        let (marked, stored) = record(&state);
        // Declined in the native dialog.
        let view = review("main", &state, DEV, NOW).unwrap();
        let no = Stub::answering(false);
        assert_eq!(
            commit("main", &state, &view.token, true, &no, NOW),
            Ok(None)
        );
        assert_eq!(record(&state), (marked.clone(), stored.clone()));
        // Dropped by the page.
        let view = review("main", &state, DEV, NOW).unwrap();
        assert_eq!(
            commit("main", &state, &view.token, false, &no, NOW),
            Ok(None)
        );
        assert_eq!(record(&state), (marked.clone(), stored.clone()));
        // Left too long: refused, and a new review can be asked for.
        let view = review("main", &state, DEV, NOW).unwrap();
        let late = NOW + PENDING_TTL_MS + 1;
        assert_eq!(
            refusal_name(
                &commit(
                    "main",
                    &state,
                    &view.token,
                    true,
                    &Stub::answering(true),
                    late
                )
                .unwrap_err()
            ),
            "pending_expired"
        );
        assert_eq!(record(&state), (marked, stored));
        let view = review("main", &state, DEV, late).expect("asked again");
        commit(
            "main",
            &state,
            &view.token,
            true,
            &Stub::answering(true),
            late,
        )
        .unwrap()
        .unwrap();
        assert!(!store::needs_consent(&record(&state).0));
    }

    #[test]
    fn with_developer_mode_off_no_folder_is_read() {
        let (temp, state) = setup("dev-off");
        let src = dev_install(&state, &temp.0);
        let pkg = package(
            &temp.0,
            "p.hewplugin",
            &manifest("com.example.p", "1.0.0", &[], &[]),
        );
        let picked = record_pick(&state, &pkg, PickKind::Package).unwrap();
        let view = stage_file("main", &state, &picked, NOW).unwrap();
        commit(
            "main",
            &state,
            &view.token,
            true,
            &Stub::answering(true),
            NOW,
        )
        .unwrap();
        let other = folder(
            &temp.0,
            "waiting",
            &manifest("com.example.w", "1.0.0", &[], &[]),
        );
        let waiting = stage_folder(&state, &other, NOW).unwrap();
        let next = folder(
            &temp.0,
            "next",
            &manifest("com.example.n", "1.0.0", &[], &[]),
        );
        let next_pick = record_pick(&state, &next, PickKind::Folder).unwrap();

        let ui = Stub::answering(true);
        assert_eq!(set_dev_mode("main", &state, false, &ui), Ok(false));
        assert_eq!(ui.asked(), 0, "turning off asks nothing");
        assert_eq!(ui.changed.get(), 1);
        let off = |r: Result<(), String>| refusal_name(&r.unwrap_err()).to_string();
        assert_eq!(
            off(stage_dir("main", &state, &next_pick, NOW).map(drop)),
            "developer_mode_off"
        );
        assert_eq!(
            off(read_file("main", &state, DEV, "main.js").map(drop)),
            "developer_mode_off"
        );
        assert_eq!(
            off(review("main", &state, DEV, NOW).map(drop)),
            "developer_mode_off"
        );
        // The folder install staged before is dropped.
        assert_eq!(
            off(commit("main", &state, &waiting.token, true, &ui, NOW).map(drop)),
            "pending_unknown"
        );
        // The watcher settles nothing and says nothing.
        std::fs::write(src.join("main.js"), b"// edited").unwrap();
        assert!(!folder_changed(&state, DEV, &ui));
        assert!(ui.dev.borrow().is_empty());
        // A package is read as before.
        assert_eq!(
            read_file("main", &state, "com.example.p", "main.js").unwrap(),
            b"// nothing\n"
        );
        // The refused stage did not use up the pick.
        state.dev_mode.store(true, Ordering::Release);
        stage_dir("main", &state, &next_pick, NOW).expect("still picked");
    }

    #[test]
    fn a_folder_install_answered_after_developer_mode_went_off_is_refused() {
        let (temp, state) = setup("dev-off-asking");
        let src = folder(&temp.0, "src", &base_manifest());
        let view = stage_folder(&state, &src, NOW).unwrap();
        let ui = During::new(true, || {
            set_dev_mode("main", &state, false, &Stub::answering(true)).unwrap();
        });
        assert_eq!(
            refusal_name(&commit("main", &state, &view.token, true, &ui, NOW).unwrap_err()),
            "developer_mode_off"
        );
        assert!(installed_ids(&state).is_empty());
    }

    #[test]
    fn turning_developer_mode_on_asks_and_is_kept_in_its_file() {
        let (temp, state) = setup("dev-on");
        let file = temp.0.join("config").join(DEV_MODE_FILE);
        let state = PluginState {
            dev_mode_file: Some(file.clone()),
            ..state
        };
        state.dev_mode.store(false, Ordering::Release);
        assert_eq!(dev_mode("main", &state, &Stub::answering(true)), Ok(false));
        let no = Stub::answering(false);
        assert_eq!(
            refusal_name(&set_dev_mode("main", &state, true, &no).unwrap_err()),
            "developer_mode_declined"
        );
        {
            let asked = no.asked.borrow();
            assert_eq!(
                asked[0],
                (
                    "Turn on developer mode?".to_string(),
                    "Hew will run plugin code straight from a folder, where the files can change at any time. Turn this on only if you are writing a plugin.".to_string(),
                    "Turn On".to_string()
                )
            );
        }
        assert_eq!(dev_mode("main", &state, &Stub::answering(true)), Ok(false));
        assert!(!file.exists());
        assert_eq!(no.changed.get(), 0);

        let yes = Stub::answering(true);
        assert_eq!(set_dev_mode("main", &state, true, &yes), Ok(true));
        assert_eq!(std::fs::read(&file).unwrap(), b"on\n");
        assert_eq!(yes.changed.get(), 1);
        assert!(read_dev_mode(&file));
        // Already on: nothing asked.
        assert_eq!(set_dev_mode("main", &state, true, &yes), Ok(true));
        assert_eq!(yes.asked(), 1);
        assert_eq!(set_dev_mode("main", &state, false, &yes), Ok(false));
        assert_eq!(std::fs::read(&file).unwrap(), b"off\n");
        assert!(!read_dev_mode(&file));
        // Read back at start.
        std::fs::write(&file, b"on\n").unwrap();
        let again = PluginState::new(state.dir.clone()).with_dev_mode_file(Some(file.clone()));
        assert_eq!(again.dev(), DevMode::On);

        // One native confirmation at a time.
        let state = PluginState::new(state.dir.clone());
        let inner = RefCell::new(None);
        let ui = During::new(false, || {
            *inner.borrow_mut() = Some(set_dev_mode("main", &state, true, &Stub::answering(true)));
        });
        let pkg = package(
            &temp.0,
            "x.hewplugin",
            &manifest("com.example.x", "1.0.0", &[], &[]),
        );
        let picked = record_pick(&state, &pkg, PickKind::Package).unwrap();
        let view = stage_file("main", &state, &picked, NOW).unwrap();
        let _ = commit("main", &state, &view.token, true, &ui, NOW);
        drop(ui);
        assert_eq!(
            refusal_name(&inner.into_inner().unwrap().unwrap_err()),
            "confirmation_open"
        );
        assert_eq!(state.dev(), DevMode::Off);

        // Another window is refused first.
        for label in ["library", ""] {
            assert_eq!(
                refusal_name(&dev_mode(label, &state, &Stub::answering(true)).unwrap_err()),
                "window_not_allowed"
            );
            assert_eq!(
                refusal_name(&set_dev_mode(label, &state, true, &yes).unwrap_err()),
                "window_not_allowed"
            );
            assert_eq!(
                refusal_name(&review(label, &state, DEV, NOW).unwrap_err()),
                "window_not_allowed"
            );
        }
    }

    #[test]
    fn only_the_exact_on_form_turns_developer_mode_on() {
        let temp = Temp::new("dev-file");
        let file = temp.0.join(DEV_MODE_FILE);
        assert!(!read_dev_mode(&file), "missing");
        for (bytes, on) in [
            (&b"on"[..], true),
            (b"on\n", true),
            (b"on\n\n", false),
            (b" on", false),
            (b"ON", false),
            (b"on\r\n", false),
            (b"yes", false),
            (b"1", false),
            (b"true", false),
            (b"", false),
            (b"\xff\xfe", false),
        ] {
            std::fs::write(&file, bytes).unwrap();
            assert_eq!(read_dev_mode(&file), on, "{bytes:?}");
        }
        let mut long = b"on\n".to_vec();
        long.extend(vec![b' '; 64]);
        std::fs::write(&file, &long).unwrap();
        assert!(!read_dev_mode(&file));
        std::fs::remove_file(&file).unwrap();
        std::fs::create_dir(&file).unwrap();
        assert!(!read_dev_mode(&file), "a folder");
        #[cfg(unix)]
        {
            std::fs::remove_dir(&file).unwrap();
            let target = temp.0.join("target");
            std::fs::write(&target, b"on\n").unwrap();
            std::os::unix::fs::symlink(&target, &file).unwrap();
            assert!(!read_dev_mode(&file), "a link");
        }
    }

    /// A [`Ui`] the watcher's thread can use: it records the events.
    #[derive(Default)]
    struct Recorder {
        dev: Mutex<Vec<DevChanged>>,
    }

    impl Ui for Recorder {
        fn confirm(&self, _: &str, _: &str, _: &str) -> bool {
            false
        }
        fn changed(&self) {}
        fn storage_changed(&self, _: StorageChanged) {}
        fn dev_changed(&self, change: DevChanged) {
            lock(&self.dev).push(change);
        }
    }

    #[test]
    fn the_watcher_follows_developer_mode_and_the_store() {
        let (temp, mut state) = setup("watch-life");
        state.watch = DevWatch::new(super::super::dev_watch::Timing {
            poll: std::time::Duration::from_millis(20),
            quiet: std::time::Duration::from_millis(80),
            budget: std::time::Duration::from_millis(500),
            join: std::time::Duration::from_secs(1),
        });
        let state = std::sync::Arc::new(state);
        let recorder = std::sync::Arc::new(Recorder::default());
        {
            let weak = std::sync::Arc::downgrade(&state);
            let recorder = recorder.clone();
            state.watch.set_handler(std::sync::Arc::new(move |event| {
                weak.upgrade().map_or(Watching::Drop, |state| {
                    on_watch(&state, event, recorder.as_ref())
                })
            }));
        }
        let src = dev_install(&state, &temp.0);
        assert!(
            !state.watch.is_running(),
            "nothing is watched before the page uses plugins"
        );
        list("main", &state, &Stub::answering(true)).unwrap();
        assert_eq!(
            state.watch.watching(),
            [DEV],
            "an enabled folder plugin is watched"
        );
        let yes = Stub::answering(true);
        set_enabled("main", &state, DEV, false, &yes).unwrap();
        assert!(!state.watch.is_running(), "off: not watched");
        set_enabled("main", &state, DEV, true, &yes).unwrap();
        assert_eq!(state.watch.watching(), [DEV]);
        set_dev_mode("main", &state, false, &yes).unwrap();
        assert!(
            !state.watch.is_running(),
            "developer mode off stops the thread"
        );
        state.dev_mode.store(true, Ordering::Release);
        sync_watch(&state, &Stub::answering(true));
        assert_eq!(state.watch.watching(), [DEV]);

        // A save reaches the host as a reload.
        std::thread::sleep(std::time::Duration::from_millis(100));
        std::fs::write(src.join("main.js"), b"// saved, longer").unwrap();
        let wait = |n: usize| {
            for _ in 0..250 {
                if lock(&recorder.dev).len() >= n {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            panic!("no event: {:?}", lock(&recorder.dev));
        };
        wait(1);
        assert_eq!(
            lock(&recorder.dev)[0],
            DevChanged {
                id: DEV.into(),
                manifest_changed: false,
                reason: None,
                problem: None
            }
        );
        // A wider manifest marks it.
        write_manifest(
            &src,
            &dev_manifest(
                DEV,
                "1.0.0",
                &["model.read", "view"],
                &["clipboard"],
                false,
                false,
            ),
        );
        wait(2);
        assert!(lock(&recorder.dev)[1].manifest_changed);
        assert!(store::needs_consent(&record(&state).0));
        sync_watch(&state, &Stub::answering(true));
        assert_eq!(
            state.watch.watching(),
            [DEV],
            "a marked plugin stays watched, so undoing the change clears it"
        );

        // Uninstalled: nothing is watched.
        let view = review("main", &state, DEV, NOW).unwrap();
        commit("main", &state, &view.token, true, &yes, NOW).unwrap();
        assert_eq!(state.watch.watching(), [DEV], "accepted: watched again");
        uninstall("main", &state, DEV, &yes).unwrap();
        assert!(!state.watch.is_running());
        state.watch.shutdown();
    }

    /// A request waiting for a folder, with its folder made.
    fn waiting_request(name: &str) -> (Temp, PluginState, PathBuf, DevRequested) {
        let (temp, state) = setup(name);
        let src = std::fs::canonicalize(folder(&temp.0, "src", &base_manifest())).unwrap();
        let params = serde_json::json!({ "path": src.to_string_lossy() });
        host_present("main", &state, true, &Stub::answering(true)).unwrap();
        let shown = bus_dev(&state, &params, NOW, &|_: &str| true)
            .unwrap()
            .unwrap();
        (temp, state, src, shown)
    }

    #[test]
    fn a_cancel_or_a_refused_folder_leaves_the_request_waiting_and_a_staged_one_takes_it() {
        // The picker opens at the folder itself.
        let (_temp, state, src, shown) = waiting_request("req-keep");
        let ui = Stub::answering(true);
        let opened = RefCell::new(None);
        // A cancel keeps it.
        let none = pick_dir("main", &state, Some(&shown.id), NOW, |start| {
            *opened.borrow_mut() = start;
            None
        });
        assert_eq!(none, Ok(None));
        if cfg!(unix) {
            assert_eq!(opened.into_inner(), Some(src.clone()));
        }
        assert_eq!(
            dev_request("main", &state, NOW).unwrap().as_ref(),
            Some(&shown)
        );
        // A folder the store refuses keeps it too: the parent holds no
        // manifest at its top level.
        let parent = src.parent().unwrap().to_path_buf();
        let picked = pick_dir("main", &state, Some(&shown.id), NOW, |_| {
            Some(parent.clone())
        })
        .unwrap()
        .unwrap();
        let refused = stage_dir_for_request("main", &state, &picked, NOW, &ui).unwrap_err();
        assert_eq!(refusal_name(&refused), "manifest_missing");
        assert!(
            refused.contains("This folder has no hew-plugin.json"),
            "{refused}"
        );
        assert_eq!(
            dev_request("main", &state, NOW).unwrap().as_ref(),
            Some(&shown)
        );
        assert!(
            ui.requests.borrow().is_empty(),
            "nobody was told it is gone"
        );
        // A pick that is not a folder at all is refused and keeps it.
        let file = src.join("main.js");
        let e = pick_dir("main", &state, Some(&shown.id), NOW, |_| Some(file.clone())).unwrap_err();
        assert_eq!(refusal_name(&e), "path_not_picked");
        assert_eq!(
            dev_request("main", &state, NOW).unwrap().as_ref(),
            Some(&shown)
        );
        // A staged folder takes it, and every window is told.
        let picked = pick_dir("main", &state, Some(&shown.id), NOW, |_| Some(src.clone()))
            .unwrap()
            .unwrap();
        stage_dir_for_request("main", &state, &picked, NOW, &ui).expect("stages");
        assert_eq!(dev_request("main", &state, NOW).unwrap(), None);
        assert_eq!(ui.requests.borrow().as_slice(), [None]);
    }

    #[test]
    fn a_folder_staged_without_a_request_does_not_touch_the_waiting_one() {
        let (temp, state, _src, shown) = waiting_request("req-other");
        let other = folder(&temp.0, "other", &base_manifest());
        let ui = Stub::answering(true);
        let picked = record_pick(&state, &other, PickKind::Folder).unwrap();
        stage_dir_for_request("main", &state, &picked, NOW, &ui).expect("stages");
        assert_eq!(
            dev_request("main", &state, NOW).unwrap().as_ref(),
            Some(&shown)
        );
        assert!(ui.requests.borrow().is_empty());
    }

    #[test]
    fn the_live_api_can_only_ask() {
        let (temp, state) = setup("bus");
        let src = std::fs::canonicalize(folder(&temp.0, "src", &base_manifest())).unwrap();
        let path = src.to_string_lossy().into_owned();
        let params = serde_json::json!({ "path": path });
        let open = |_: &str| true;

        // While no window runs a plugin host (the Plugins switch is off
        // everywhere), refused and nothing is recorded.
        assert_eq!(
            bus_dev(&state, &params, NOW, &open).unwrap_err().name,
            "plugins_off"
        );
        assert!(
            lock(&state.dev_request).current.is_none(),
            "nothing recorded"
        );
        assert_eq!(dev_request("main", &state, NOW).unwrap(), None);
        let ui = Stub::answering(true);
        host_present("main", &state, true, &ui).unwrap();
        // A host in a window that is gone does not count, and is forgotten.
        assert_eq!(
            bus_dev(&state, &params, NOW, &|_: &str| false)
                .unwrap_err()
                .name,
            "plugins_off"
        );
        assert!(lock(&state.hosts).is_empty());
        host_present("main", &state, true, &ui).unwrap();

        // Refused with developer mode off, and while a dialog is open.
        state.dev_mode.store(false, Ordering::Release);
        assert_eq!(
            bus_dev(&state, &params, NOW, &open).unwrap_err().name,
            "developer_mode_off"
        );
        state.dev_mode.store(true, Ordering::Release);
        {
            let _asking = begin_asking(&state).unwrap();
            assert_eq!(
                bus_dev(&state, &params, NOW, &open).unwrap_err().name,
                "confirmation_open"
            );
        }
        for bad in [
            serde_json::json!({}),
            serde_json::json!({ "path": 7 }),
            serde_json::json!({ "path": "" }),
            serde_json::json!({ "path": "\u{200B}" }),
            serde_json::json!({ "path": "x".repeat(MAX_REQUEST_PATH_BYTES + 1) }),
        ] {
            assert_eq!(
                bus_dev(&state, &bad, NOW, &open).unwrap_err().name,
                "invalid_params",
                "{bad}"
            );
        }

        // Asked: the path comes back cleaned, with an id, and nothing else
        // happens.
        let shown = bus_dev(&state, &params, NOW, &open).unwrap().unwrap();
        assert_eq!(
            shown.path,
            plugin_host::clean_text(&path, MAX_REQUEST_PATH_CHARS)
        );
        assert_eq!(shown.id.len(), 32);
        assert!(lock(&state.picked).is_empty(), "not recorded as picked");
        assert!(lock(&state.pending).is_empty(), "nothing staged");
        assert!(!temp.0.join("store").exists(), "the store is untouched");
        assert_eq!(
            refusal_name(&stage_dir("main", &state, &path, NOW).unwrap_err()),
            "path_not_picked"
        );
        assert_eq!(
            dev_request("main", &state, NOW).unwrap(),
            Some(shown.clone())
        );

        // While it waits: the same path asks nothing new; another path is
        // refused, so the waiting one is never replaced under the notice.
        assert_eq!(bus_dev(&state, &params, NOW + 1, &open).unwrap(), None);
        let other = serde_json::json!({ "path": "/elsewhere/plugin" });
        assert_eq!(
            bus_dev(&state, &other, NOW + 1, &open).unwrap_err().name,
            "request_waiting"
        );
        assert_eq!(
            dev_request("main", &state, NOW).unwrap(),
            Some(shown.clone())
        );

        // A picker opens only for the request the window shows, at the
        // folder itself; looking at the request does not use it up.
        let ui = Stub::answering(true);
        assert_eq!(
            refusal_name(&pick_dir_start(&state, Some("0123"), NOW).unwrap_err()),
            "request_gone"
        );
        assert_eq!(pick_dir_start(&state, None, NOW), Ok(None), "no request");
        if cfg!(unix) {
            let want = std::fs::canonicalize(&src).ok();
            assert_eq!(
                pick_dir_start(&state, Some(&shown.id), NOW),
                Ok(want.clone())
            );
            assert_eq!(
                pick_dir_start(&state, Some(&shown.id), NOW),
                Ok(want),
                "still waiting"
            );
            assert!(dev_request("main", &state, NOW).unwrap().is_some());
        }
        assert!(ui.requests.borrow().is_empty());
        // Expired: gone, and another may come.
        lock(&state.dev_request).current = None;
        let again = bus_dev(&state, &params, NOW, &open).unwrap().unwrap();
        assert_eq!(
            refusal_name(
                &pick_dir_start(&state, Some(&again.id), NOW + PENDING_TTL_MS + 1).unwrap_err()
            ),
            "request_gone"
        );
        assert!(bus_dev(&state, &other, NOW + PENDING_TTL_MS + 2, &open)
            .unwrap()
            .is_some());

        // Turning developer mode off drops a waiting request and tells
        // every window.
        let off = Stub::answering(true);
        set_dev_mode("main", &state, false, &off).unwrap();
        assert_eq!(off.requests.borrow().as_slice(), [None]);
        assert_eq!(dev_request("main", &state, NOW).unwrap(), None);
        state.dev_mode.store(true, Ordering::Release);

        // The last host going drops it too.
        let r = bus_dev(&state, &params, NOW, &open).unwrap().unwrap();
        let gone = Stub::answering(true);
        host_present("settings", &state, true, &gone).unwrap();
        host_present("main", &state, false, &gone).unwrap();
        assert_eq!(dev_request("main", &state, NOW).unwrap(), Some(r));
        host_present("settings", &state, false, &gone).unwrap();
        assert_eq!(gone.requests.borrow().as_slice(), [None]);
        assert_eq!(dev_request("main", &state, NOW).unwrap(), None);
        assert_eq!(
            refusal_name(&host_present("library", &state, true, &gone).unwrap_err()),
            "window_not_allowed"
        );
    }

    #[test]
    fn a_dismissed_request_is_gone_everywhere_and_the_socket_waits() {
        let (_temp, state) = setup("dismiss");
        let ui = Stub::answering(true);
        host_present("main", &state, true, &ui).unwrap();
        let open = |_: &str| true;
        let params = serde_json::json!({ "path": "/home/me/plugin" });
        let r = bus_dev(&state, &params, NOW, &open).unwrap().unwrap();
        // Dismissing another id leaves the waiting one and says what waits.
        dismiss_request("main", &state, "elsewhere", NOW, &ui).unwrap();
        assert_eq!(ui.requests.borrow().as_slice(), [Some(r.clone())]);
        assert_eq!(dev_request("main", &state, NOW).unwrap(), Some(r.clone()));
        dismiss_request("settings", &state, &r.id, NOW, &ui).unwrap();
        assert_eq!(ui.requests.borrow().last(), Some(&None));
        assert_eq!(dev_request("main", &state, NOW).unwrap(), None);
        // A flood cannot bring it back for the quiet.
        for at in [NOW, NOW + 1, NOW + REQUEST_QUIET_MS - 1] {
            assert_eq!(
                bus_dev(&state, &params, at, &open).unwrap_err().name,
                "request_dismissed"
            );
        }
        assert_eq!(dev_request("main", &state, NOW).unwrap(), None);
        assert!(bus_dev(&state, &params, NOW + REQUEST_QUIET_MS, &open)
            .unwrap()
            .is_some());
        assert_eq!(
            refusal_name(&dismiss_request("library", &state, &r.id, NOW, &ui).unwrap_err()),
            "window_not_allowed"
        );
    }

    #[test]
    fn a_page_can_look_at_the_waiting_request_without_taking_it() {
        let (_temp, state) = setup("dev-request");
        host_present("main", &state, true, &Stub::answering(true)).unwrap();
        let open = |_: &str| true;
        assert_eq!(
            dev_request("main", &state, NOW).unwrap(),
            None,
            "none waiting"
        );
        let hostile = serde_json::json!({ "path": "\u{202E}/home/me/plugin\n\u{1b}[31m" });
        let seen = bus_dev(&state, &hostile, NOW, &open).unwrap().unwrap();
        // The same text the event carried, as often as it is asked.
        assert_eq!(
            dev_request("main", &state, NOW).unwrap(),
            Some(seen.clone())
        );
        assert_eq!(
            dev_request("settings", &state, NOW + 5).unwrap(),
            Some(seen.clone())
        );
        assert_eq!(seen.path, "/home/me/plugin [31m");
        // Cleaning changed the path, so the picker gets no start folder.
        assert_eq!(
            lock(&state.dev_request).current.as_ref().unwrap().start,
            None
        );
        // Every page check applies.
        assert_eq!(
            refusal_name(&dev_request("library", &state, NOW).unwrap_err()),
            "window_not_allowed"
        );
        assert_eq!(
            refusal_name(&dev_request("main", &PluginState::new(None), NOW).unwrap_err()),
            "no_plugin_dir"
        );
        // Expired: not shown, and dropped.
        assert_eq!(
            dev_request("main", &state, NOW + PENDING_TTL_MS + 1).unwrap(),
            None
        );
        let long =
            serde_json::json!({ "path": format!("/{}", "a".repeat(MAX_REQUEST_PATH_BYTES - 1)) });
        bus_dev(&state, &long, NOW, &open).unwrap().unwrap();
        let shown = dev_request("main", &state, NOW).unwrap().unwrap();
        assert_eq!(
            shown.path.chars().count(),
            MAX_REQUEST_PATH_CHARS,
            "cut as the event is"
        );
        assert_eq!(
            lock(&state.dev_request).current.as_ref().unwrap().start,
            None,
            "a cut path opens no start folder"
        );
        pick_dir_start(&state, Some(&shown.id), NOW).unwrap();
        assert!(
            dev_request("main", &state, NOW).unwrap().is_some(),
            "looking does not use it up"
        );
        // With developer mode off nothing is waiting.
        lock(&state.dev_request).current = None;
        bus_dev(&state, &hostile, NOW, &open).unwrap();
        state.dev_mode.store(false, Ordering::Release);
        assert_eq!(dev_request("main", &state, NOW).unwrap(), None);
    }

    #[cfg(unix)]
    #[test]
    fn a_request_opens_the_picker_only_at_a_plain_local_folder() {
        let no_start = [
            PathBuf::from("/home/someone/.local/share/com.hew3d.Hew"),
            PathBuf::from("/home/someone/.config/com.hew3d.Hew"),
        ];
        for (raw, ok) in [
            ("/home/someone/plugin", true),
            ("/Users/someone/My Plugin", true),
            ("/", true),
            ("relative/plugin", false),
            ("/home/../etc", false),
            ("/home/./plugin", false),
            ("/./net/server/share", false),
            ("/home//plugin", false),
            ("/home/plugin/", false),
            ("//server/share", false),
            ("/net", false),
            ("/net/server/share", false),
            ("/NET/server", false),
            ("/Network/Servers/x", false),
            ("/afs/cell/x", false),
            ("/run/user/1000/gvfs/smb-share:server=x", false),
            ("/run/user/1000/doc/abc", false),
            ("/run/user/1000/other", true),
            ("/home/a\nb", false),
            ("/home/a\u{200B}b", false),
            ("~/plugin", false),
            ("/home/someone/.local/share/com.hew3d.Hew", false),
            (
                "/home/someone/.local/share/com.hew3d.Hew/plugins/com.x",
                false,
            ),
            ("/home/someone/.config/com.hew3d.Hew/x", false),
            ("/home/someone/.config/other", true),
        ] {
            let shown = plugin_host::clean_text(raw, MAX_REQUEST_PATH_CHARS);
            assert_eq!(
                start_folder_hint(raw, &shown, &no_start).is_some(),
                ok,
                "{raw:?}"
            );
        }
        // The store itself, wherever it is kept.
        let (temp, state) = setup("no-start");
        let root = state
            .dir
            .as_ref()
            .unwrap()
            .root()
            .to_string_lossy()
            .into_owned();
        let inner = format!("{root}/com.example.x");
        assert_eq!(
            start_folder_hint(&inner, &inner, &no_start_roots(&state)),
            None
        );
        drop(temp);
    }

    #[test]
    fn the_folder_picker_takes_no_path_from_the_page() {
        let source = include_str!("plugins.rs");
        let start = source
            .find("pub async fn plugin_pick_dir(")
            .expect("the command");
        let signature = &source[start..start + source[start..].find("->").unwrap()];
        // The page passes a request's id, never a path: the parameters are
        // exactly the window and that id, so a new `path: String` fails.
        let open = signature.find('(').unwrap();
        let close = signature.rfind(')').unwrap();
        let params: Vec<String> = signature[open + 1..close]
            .split(',')
            .map(|p| p.split_whitespace().collect::<Vec<_>>().join(" "))
            .filter(|p| !p.is_empty())
            .collect();
        assert_eq!(
            params,
            [
                "window: tauri::WebviewWindow",
                "use_request: Option<String>"
            ],
            "{signature}"
        );
        let body = &source[start..start + source[start..].find("\n}\n").unwrap()];
        // Only the plain function sees the id, and it only compares it.
        assert!(body.contains("pick_dir("), "{body}");
        assert!(body.contains("use_request.as_deref()"), "{body}");
    }

    #[test]
    fn a_picker_is_one_native_dialog_and_refused_first() {
        let (temp, state) = setup("pickers");
        let src = folder(&temp.0, "src", &base_manifest());
        let ui = Stub::answering(true);
        // Developer mode off: refused before anything shows.
        state.dev_mode.store(false, Ordering::Release);
        let shown = Cell::new(false);
        let e = pick_dir("main", &state, None, NOW, |_| {
            shown.set(true);
            None
        })
        .unwrap_err();
        assert_eq!(refusal_name(&e), "developer_mode_off");
        assert!(!shown.get());
        state.dev_mode.store(true, Ordering::Release);
        // While another native dialog is open, neither picker shows.
        {
            let _asking = begin_asking(&state).unwrap();
            let e = pick_dir("main", &state, None, NOW, |_| {
                shown.set(true);
                None
            })
            .unwrap_err();
            assert_eq!(refusal_name(&e), "confirmation_open");
            let e = pick_package("main", &state, || {
                shown.set(true);
                None
            })
            .unwrap_err();
            assert_eq!(refusal_name(&e), "confirmation_open");
            assert!(!shown.get());
        }
        // While a picker is open, a confirmation (and another picker) is
        // refused.
        let pkg = package(
            &temp.0,
            "p.hewplugin",
            &manifest("com.example.p", "1.0.0", &[], &[]),
        );
        let staged = stage_file(
            "main",
            &state,
            &record_pick(&state, &pkg, PickKind::Package).unwrap(),
            NOW,
        )
        .unwrap();
        let during = RefCell::new(None);
        let picked = pick_dir("main", &state, None, NOW, |_| {
            *during.borrow_mut() = Some((
                commit(
                    "main",
                    &state,
                    &staged.token,
                    true,
                    &Stub::answering(true),
                    NOW,
                ),
                pick_package("main-2", &state, || None),
            ));
            Some(src.clone())
        })
        .unwrap();
        let (commit_during, pick_during) = during.into_inner().unwrap();
        assert_eq!(
            refusal_name(&commit_during.unwrap_err()),
            "confirmation_open"
        );
        assert_eq!(refusal_name(&pick_during.unwrap_err()), "confirmation_open");
        // The pick is the person's, and the flow goes on in order.
        let picked = picked.unwrap();
        stage_dir("main", &state, &picked, NOW).expect("stages");
        assert!(commit("main", &state, &staged.token, true, &ui, NOW)
            .unwrap()
            .is_some());
        // A request that is gone refuses before the picker shows.
        let e = pick_dir("main", &state, Some("gone"), NOW, |_| {
            shown.set(true);
            None
        })
        .unwrap_err();
        assert_eq!(refusal_name(&e), "request_gone");
        assert!(!shown.get());
    }

    /// The manifest of [`DEV`] with one more permission.
    fn wider() -> String {
        dev_manifest(
            DEV,
            "1.0.0",
            &["model.read", "view"],
            &["clipboard"],
            false,
            false,
        )
    }

    fn marked_reason(state: &PluginState) -> Option<&'static str> {
        let (r, _) = record(state);
        store::needs_consent(&r).then(|| store::needs_consent_reason(&r).unwrap_or("?"))
    }

    #[test]
    fn a_folder_edited_while_not_watched_is_compared_when_watching_starts() {
        // Turned off and on again.
        let (temp, state) = setup("enter-enable");
        let src = dev_install(&state, &temp.0);
        let yes = Stub::answering(true);
        set_enabled("main", &state, DEV, false, &yes).unwrap();
        write_manifest(&src, &wider());
        let on = Stub::answering(true);
        set_enabled("main", &state, DEV, true, &on).unwrap();
        assert_eq!(marked_reason(&state), Some("permissions_changed"));
        assert_eq!(
            refusal_name(&read_file("main", &state, DEV, "main.js").unwrap_err()),
            "dev_consent_pending"
        );
        // The windows hear the store change, then the mark.
        assert_eq!(on.changed.get(), 1);
        assert!(on.dev.borrow().iter().any(|e| e.manifest_changed));

        // Developer mode off, then on again.
        let (temp, state) = setup("enter-dev");
        let src = dev_install(&state, &temp.0);
        list("main", &state, &Stub::answering(true)).unwrap();
        set_dev_mode("main", &state, false, &Stub::answering(true)).unwrap();
        write_manifest(&src, &wider());
        let on = Stub::answering(true);
        set_dev_mode("main", &state, true, &on).unwrap();
        assert_eq!(marked_reason(&state), Some("permissions_changed"));
        assert!(on.dev.borrow().iter().any(|e| e.manifest_changed));
        state.watch.shutdown();

        // Hew started again over an edited folder: the first list already
        // carries the mark.
        let (temp, state) = setup("enter-start");
        let src = dev_install(&state, &temp.0);
        write_manifest(&src, &wider());
        let fresh = PluginState::new(state.dir.clone());
        fresh.dev_mode.store(true, Ordering::Release);
        let rows = list("main", &fresh, &Stub::answering(true)).unwrap();
        let Listed::Installed { installed, .. } = &rows[0] else {
            panic!("installed")
        };
        assert!(store::needs_consent(installed), "marked before the rows");
        fresh.watch.shutdown();

        // A review accepted, then the folder changed again before the yes.
        let (temp, state) = setup("enter-review");
        let src = dev_install(&state, &temp.0);
        write_manifest(&src, &wider());
        folder_changed(&state, DEV, &Stub::answering(true));
        let view = review("main", &state, DEV, NOW).unwrap();
        write_manifest(
            &src,
            &dev_manifest(
                DEV,
                "1.0.0",
                &["model.read", "view", "library"],
                &["clipboard"],
                false,
                false,
            ),
        );
        let yes = Stub::answering(true);
        commit("main", &state, &view.token, true, &yes, NOW).unwrap();
        assert_eq!(marked_reason(&state), Some("permissions_changed"));
        assert!(yes.dev.borrow().iter().all(|e| e.manifest_changed));

        // Removed and installed again over an edited folder is compared
        // from the folder as it is at the yes.
        let (temp, state) = setup("enter-readd");
        let src = folder(&temp.0, "src", &base_manifest());
        let view = stage_folder(&state, &src, NOW).unwrap();
        write_manifest(&src, &wider());
        commit(
            "main",
            &state,
            &view.token,
            true,
            &Stub::answering(true),
            NOW,
        )
        .unwrap();
        assert_eq!(marked_reason(&state), Some("permissions_changed"));
        drop(temp);
    }

    #[test]
    fn a_rollback_of_a_folder_plugin_compares_the_folder_with_the_version_restored() {
        let (temp, state) = setup("rollback-mark");
        let src = folder(&temp.0, "src", &base_manifest());
        let yes = Stub::answering(true);
        let view = stage_folder(&state, &src, NOW).unwrap();
        commit("main", &state, &view.token, true, &yes, NOW).unwrap();
        write_manifest(
            &src,
            &dev_manifest(DEV, "1.1.0", &["model.read"], &["clipboard"], false, false),
        );
        let view = stage_folder(&state, &src, NOW).unwrap();
        commit("main", &state, &view.token, true, &yes, NOW).unwrap();
        // The folder now asks for more; the watcher marks the 1.1 record.
        write_manifest(
            &src,
            &dev_manifest(
                DEV,
                "1.1.0",
                &["model.read", "view"],
                &["clipboard"],
                false,
                false,
            ),
        );
        folder_changed(&state, DEV, &Stub::answering(true));
        assert_eq!(marked_reason(&state), Some("permissions_changed"));
        // Rolling back restores the 1.0 record, which carries no mark; the
        // folder is compared with it before anything can read.
        let back = Stub::answering(true);
        let restored = rollback("main", &state, DEV, &back).unwrap();
        assert_eq!(restored.version.to_string(), "1.0.0");
        assert!(store::needs_consent(&restored), "marked again");
        assert_eq!(marked_reason(&state), Some("permissions_changed"));
        for rel in ["main.js", "hew-plugin.json"] {
            assert_eq!(
                refusal_name(&read_file("main", &state, DEV, rel).unwrap_err()),
                "dev_consent_pending"
            );
        }
        assert!(back.dev.borrow().iter().any(|e| e.manifest_changed));
        // The review compares the folder with the restored manifest.
        let view = review("main", &state, DEV, NOW).unwrap();
        let u = view.consent.update.as_ref().unwrap();
        assert_eq!(u.from_version, "1.0.0");
        assert_eq!(lines(&u.added), ["view"]);
    }

    #[test]
    fn turning_developer_mode_off_is_kept_or_said_to_be_not_kept() {
        // The file cannot be written but can be removed: removed, off.
        let (temp, state) = setup("off-remove");
        let config = temp.0.join("config");
        std::fs::create_dir_all(&config).unwrap();
        let file = config.join(DEV_MODE_FILE);
        std::fs::write(&file, b"on\n").unwrap();
        std::fs::create_dir_all(file.with_extension("tmp")).unwrap();
        let state = PluginState {
            dev_mode_file: Some(file.clone()),
            ..state
        };
        assert_eq!(
            set_dev_mode("main", &state, false, &Stub::answering(true)),
            Ok(false)
        );
        assert!(!file.exists(), "no file reads as off");
        assert!(!PluginState::new(None)
            .with_dev_mode_file(Some(file.clone()))
            .dev_mode
            .load(Ordering::Acquire));
        #[cfg(unix)]
        {
            // Neither written nor removed: off for now, and said so.
            use std::os::unix::fs::PermissionsExt;
            std::fs::remove_dir_all(file.with_extension("tmp")).unwrap();
            std::fs::write(&file, b"on\n").unwrap();
            state.dev_mode.store(true, Ordering::Release);
            std::fs::set_permissions(&config, std::fs::Permissions::from_mode(0o555)).unwrap();
            let writable = std::fs::write(config.join("probe"), b"").is_ok();
            if !writable {
                let e = set_dev_mode("main", &state, false, &Stub::answering(true)).unwrap_err();
                assert_eq!(refusal_name(&e), "developer_mode_not_saved");
                assert!(e.contains("off for now"), "{e}");
                assert_eq!(state.dev(), DevMode::Off, "off in this run");
            }
            std::fs::set_permissions(&config, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_folder_whose_path_is_not_text_is_refused_at_the_pick() {
        use std::os::unix::ffi::OsStrExt;
        let (temp, state) = setup("bytes");
        let name = [b's', 0xff, b'c'];
        let parent = temp.0.join(std::ffi::OsStr::from_bytes(&name));
        let src = folder(&parent, "src", &base_manifest());
        assert_eq!(
            refusal_name(&record_pick(&state, &src, PickKind::Folder).unwrap_err()),
            "folder_path_invalid"
        );
    }

    #[test]
    fn a_folder_stage_that_read_while_developer_mode_went_off_holds_nothing() {
        let (temp, state) = setup("stage-off");
        let src = folder(&temp.0, "src", &base_manifest());
        // What stage_dir does, with developer mode going off during the read.
        let epoch = state.dev_epoch.load(Ordering::Acquire);
        let package = store::read_package_dir(&src).unwrap();
        set_dev_mode("main", &state, false, &Stub::answering(true)).unwrap();
        let source = InstallSource::Dir {
            path: src.to_string_lossy().into_owned(),
        };
        assert_eq!(
            refusal_name(&hold(&state, package.clone(), source.clone(), NOW, epoch).unwrap_err()),
            "developer_mode_off"
        );
        assert!(lock(&state.pending).is_empty(), "no slot held");
        // Off and on again during the read is refused too.
        state.dev_mode.store(true, Ordering::Release);
        assert_eq!(
            refusal_name(&hold(&state, package.clone(), source.clone(), NOW, epoch).unwrap_err()),
            "developer_mode_off"
        );
        let now = state.dev_epoch.load(Ordering::Acquire);
        hold(&state, package, source, NOW, now).expect("a stage begun while on");
    }

    #[test]
    fn a_folder_read_in_flight_when_developer_mode_goes_off_is_dropped() {
        let (temp, state) = setup("read-off");
        dev_install(&state, &temp.0);
        let epoch = state.dev_epoch.load(Ordering::Acquire);
        let source = store::file_source(state.dir().unwrap(), DEV, DevMode::On).unwrap();
        let e = read_checked(&state, &source, epoch, || {
            let bytes = source.read("main.js");
            set_dev_mode("main", &state, false, &Stub::answering(true)).unwrap();
            bytes
        })
        .unwrap_err();
        assert_eq!(refusal_name(&e), "developer_mode_off");
        // A package is not a folder: read either way.
        let pkg = package(
            &temp.0,
            "p.hewplugin",
            &manifest("com.example.p", "1.0.0", &[], &[]),
        );
        let view = stage_file(
            "main",
            &state,
            &record_pick(&state, &pkg, PickKind::Package).unwrap(),
            NOW,
        )
        .unwrap();
        commit(
            "main",
            &state,
            &view.token,
            true,
            &Stub::answering(true),
            NOW,
        )
        .unwrap();
        let source =
            store::file_source(state.dir().unwrap(), "com.example.p", DevMode::Off).unwrap();
        assert!(read_checked(&state, &source, 0, || source.read("main.js")).is_ok());
    }

    #[test]
    fn a_sync_that_saw_developer_mode_on_cannot_watch_after_it_went_off() {
        let (temp, state) = setup("sync-race");
        state
            .watch
            .set_handler(std::sync::Arc::new(|_| Watching::Keep));
        dev_install(&state, &temp.0);
        list("main", &state, &Stub::answering(true)).unwrap();
        assert_eq!(state.watch.watching(), [DEV]);
        // A sync holds the decision while developer mode goes off in
        // another window: the off waits for it, then stops the watcher.
        std::thread::scope(|scope| {
            let held = lock(&state.watched);
            let off = scope.spawn(|| set_dev_mode("main", &state, false, &Stub::answering(true)));
            // The off stores the mode, then waits for the lock this holds.
            eventually("the mode is stored off", || state.dev() == DevMode::Off);
            assert!(!off.is_finished(), "waits for the sync");
            // The sync that saw it on hands its set over now.
            state
                .watch
                .watch(BTreeMap::from([(DEV.to_string(), temp.0.join("src"))]));
            drop(held);
            off.join().unwrap().unwrap();
        });
        assert!(!state.watch.is_running(), "off wins");
        state.watch.shutdown();
    }

    #[test]
    fn a_marked_plugin_whose_change_is_undone_runs_again() {
        let (temp, state) = setup("unmark");
        let src = dev_install(&state, &temp.0);
        write_manifest(&src, "{ not json");
        folder_changed(&state, DEV, &Stub::answering(true));
        assert_eq!(marked_reason(&state), Some("manifest_unreadable"));
        write_manifest(&src, &base_manifest());
        let ui = Stub::answering(true);
        assert!(folder_changed(&state, DEV, &ui));
        assert_eq!(marked_reason(&state), None);
        assert_eq!(ui.changed.get(), 1);
        assert_eq!(
            ui.dev.borrow().as_slice(),
            [DevChanged {
                id: DEV.into(),
                manifest_changed: false,
                reason: None,
                problem: None
            }]
        );
        read_file("main", &state, DEV, "main.js").expect("reads again");
    }

    #[test]
    fn the_live_list_says_what_the_shell_knows() {
        let (temp, state) = setup("bus-list");
        dev_install(&state, &temp.0);
        let pkg = package(
            &temp.0,
            "p.hewplugin",
            &manifest("com.example.p", "1.0.0", &[], &[]),
        );
        let picked = record_pick(&state, &pkg, PickKind::Package).unwrap();
        let view = stage_file("main", &state, &picked, NOW).unwrap();
        commit(
            "main",
            &state,
            &view.token,
            true,
            &Stub::answering(true),
            NOW,
        )
        .unwrap();
        let states = |state: &PluginState| -> Vec<(String, String)> {
            let v = bus_list(state, NOW, &|_: &str| true).unwrap();
            v["plugins"]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| {
                    (
                        p["id"].as_str().unwrap().to_string(),
                        p["state"].as_str().unwrap().to_string(),
                    )
                })
                .collect()
        };
        let row = bus_list(&state, NOW, &|_: &str| true).unwrap()["plugins"][0].clone();
        assert_eq!(
            row,
            serde_json::json!({
                "id": DEV, "name": "Dev Plugin", "version": "1.0.0",
                "source": "dir", "enabled": true, "state": "installed"
            })
        );
        assert_eq!(
            bus_list(&state, NOW, &|_: &str| true).unwrap()["developer_mode"],
            true
        );
        state.dev_mode.store(false, Ordering::Release);
        assert_eq!(
            states(&state),
            [
                (DEV.to_string(), "needs_developer_mode".to_string()),
                ("com.example.p".to_string(), "installed".to_string())
            ]
        );
        state.dev_mode.store(true, Ordering::Release);
        let src = temp.0.join("src");
        write_manifest(&src, "{");
        folder_changed(&state, DEV, &Stub::answering(true));
        assert_eq!(states(&state)[0].1, "needs_review");
        let none = PluginState::new(None);
        assert_eq!(
            bus_list(&none, NOW, &|_: &str| true).unwrap_err().name,
            "no_plugin_dir"
        );
    }

    #[test]
    fn the_live_list_says_whether_a_request_is_waiting_and_nothing_more() {
        let (_temp, state, _src, shown) = waiting_request("list-waiting");
        let v = bus_list(&state, NOW, &|_: &str| true).unwrap();
        assert_eq!(v["request_waiting"], true);
        assert!(!v.to_string().contains(&shown.id), "no id: {v}");
        assert!(!v.to_string().contains("src"), "no path: {v}");
        assert_eq!(
            bus_list(&state, NOW + PENDING_TTL_MS + 1, &|_: &str| true).unwrap()["request_waiting"],
            false,
            "expired"
        );
        let ui = Stub::answering(true);
        dismiss_request("main", &state, &shown.id, NOW, &ui).unwrap();
        assert_eq!(
            bus_list(&state, NOW, &|_: &str| true).unwrap()["request_waiting"],
            false,
            "dismissed"
        );
    }

    #[test]
    fn a_request_stays_waiting_in_the_list_until_the_flow_it_started_ends() {
        let waiting = |state: &PluginState, at: u64| {
            bus_list(state, at, &|_: &str| true).unwrap()["request_waiting"].clone()
        };
        // Staged from the request, then answered each way.
        for (answer, installed) in [(Some(true), true), (Some(false), false), (None, false)] {
            let (_temp, state, src, shown) = waiting_request("flow");
            let ui = Stub::answering(answer.unwrap_or(true));
            let picked = pick_dir("main", &state, Some(&shown.id), NOW, |_| Some(src.clone()))
                .unwrap()
                .unwrap();
            let view = stage_dir_for_request("main", &state, &picked, NOW, &ui).unwrap();
            assert_eq!(
                dev_request("main", &state, NOW).unwrap(),
                None,
                "the card is gone"
            );
            assert_eq!(waiting(&state, NOW), true, "the sheet is open");
            match answer {
                Some(_) => {
                    commit("main", &state, &view.token, true, &ui, NOW).unwrap();
                }
                // Cancelled at the sheet.
                None => {
                    commit("main", &state, &view.token, false, &ui, NOW).unwrap();
                }
            }
            assert_eq!(waiting(&state, NOW), false, "{answer:?}");
            let rows = bus_list(&state, NOW, &|_: &str| true).unwrap();
            assert_eq!(
                rows["plugins"].as_array().unwrap().len(),
                usize::from(installed)
            );
        }
        // Expired unanswered.
        let (_temp, state, src, shown) = waiting_request("flow-late");
        let ui = Stub::answering(true);
        let picked = pick_dir("main", &state, Some(&shown.id), NOW, |_| Some(src.clone()))
            .unwrap()
            .unwrap();
        stage_dir_for_request("main", &state, &picked, NOW, &ui).unwrap();
        assert_eq!(waiting(&state, NOW + 1), true);
        assert_eq!(waiting(&state, NOW + PENDING_TTL_MS + 1), false);
    }

    #[test]
    fn a_plugin_whose_main_file_is_gone_says_so_in_the_list() {
        let (temp, state) = setup("main-missing");
        let src = dev_install(&state, &temp.0);
        let problem = |state: &PluginState| {
            list_rows("main", state, &Stub::answering(true)).unwrap()[0]["dev_problem"].clone()
        };
        // dev_install puts the manifest's main in the folder.
        assert_ne!(problem(&state), "main_missing");
        std::fs::remove_file(src.join("main.js")).unwrap();
        assert_eq!(problem(&state), "main_missing");
        // A folder in its place, or a link, is not a regular file.
        std::fs::create_dir(src.join("main.js")).unwrap();
        assert_eq!(problem(&state), "main_missing");
        std::fs::remove_dir(src.join("main.js")).unwrap();
        std::fs::write(src.join("main.js"), b"//").unwrap();
        assert_ne!(problem(&state), "main_missing");
        state.watch.shutdown();
    }

    #[test]
    fn the_list_carries_the_folder_problem_the_watcher_has_now() {
        let (temp, mut state) = setup("list-problem");
        state.watch = DevWatch::new(super::super::dev_watch::Timing {
            poll: std::time::Duration::from_millis(20),
            quiet: std::time::Duration::from_millis(80),
            budget: std::time::Duration::from_millis(500),
            join: std::time::Duration::from_secs(1),
        });
        state
            .watch
            .set_handler(std::sync::Arc::new(|_| Watching::Keep));
        let src = dev_install(&state, &temp.0);
        let problem = |state: &PluginState| {
            list_rows("main", state, &Stub::answering(true)).unwrap()[0]["dev_problem"].clone()
        };
        assert_eq!(problem(&state), serde_json::Value::Null);
        std::fs::rename(&src, temp.0.join("moved")).unwrap();
        let mut seen = serde_json::Value::Null;
        for _ in 0..250 {
            seen = problem(&state);
            if !seen.is_null() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(seen, "folder_missing");
        std::fs::rename(temp.0.join("moved"), &src).unwrap();
        for _ in 0..250 {
            seen = problem(&state);
            if seen.is_null() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(seen, serde_json::Value::Null);
        state.watch.shutdown();
    }

    static STALL_ENTRY: AtomicBool = AtomicBool::new(false);
    static STALL_WINDOWS: AtomicBool = AtomicBool::new(false);
    static STALL_DEV_ON: AtomicBool = AtomicBool::new(false);

    fn stalled_while(flag: &AtomicBool, root: &Path) -> Result<Vec<u8>, InstallError> {
        while flag.load(Ordering::Acquire) {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        store::folder_manifest(root)
    }

    /// The state over `store` as the app has it at start, with a manifest
    /// reader that waits while `flag` is set and a command read budget of
    /// `budget_ms`, and a watcher that tells `recorder`.
    fn restarted(
        dir: Option<PluginDir>,
        reader: ManifestReader,
        budget_ms: u64,
        recorder: std::sync::Arc<Recorder>,
    ) -> std::sync::Arc<PluginState> {
        let mut state = PluginState::new(dir);
        state.dev_mode.store(true, Ordering::Release);
        state.manifest_reader = reader;
        state.read_budget = std::time::Duration::from_millis(budget_ms);
        state.watch = DevWatch::new(super::super::dev_watch::Timing {
            poll: std::time::Duration::from_millis(20),
            quiet: std::time::Duration::from_millis(60),
            budget: std::time::Duration::from_millis(500),
            join: std::time::Duration::from_secs(1),
        });
        let state = std::sync::Arc::new(state);
        let weak = std::sync::Arc::downgrade(&state);
        state.watch.set_handler(std::sync::Arc::new(move |event| {
            weak.upgrade().map_or(Watching::Drop, |state| {
                on_watch(&state, event, recorder.as_ref())
            })
        }));
        state
    }

    fn eventually(what: &str, mut ok: impl FnMut() -> bool) {
        for _ in 0..400 {
            if ok() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("never: {what}");
    }

    #[test]
    fn a_folder_plugin_whose_comparison_stalls_reads_nothing_until_one_completes() {
        let (temp, state) = setup("unchecked-stall");
        dev_install(&state, &temp.0);
        let recorder = std::sync::Arc::new(Recorder::default());
        STALL_ENTRY.store(true, Ordering::Release);
        let state = restarted(
            state.dir.clone(),
            |root| stalled_while(&STALL_ENTRY, root),
            100,
            recorder.clone(),
        );
        // The first list waits its budget for the comparison, then answers.
        let rows = list_rows("main", &state, &Stub::answering(true)).unwrap();
        assert_eq!(rows[0]["dev_problem"], FOLDER_NOT_CHECKED);
        let e = read_file("main", &state, DEV, "main.js").unwrap_err();
        assert_eq!(refusal_name(&e), "folder_missing");
        // With no window running a plugin host nothing will compare it.
        assert_eq!(
            bus_list(&state, NOW, &|_: &str| true).unwrap()["plugins"][0]["state"],
            "plugins_off"
        );
        lock(&state.hosts).insert("main".to_string());
        assert_eq!(
            bus_list(&state, NOW, &|_: &str| true).unwrap()["plugins"][0]["state"],
            "checking"
        );
        assert_eq!(
            bus_list(&state, NOW, &|_: &str| false).unwrap()["plugins"][0]["state"],
            "plugins_off",
            "a host in a window that is gone does not count"
        );
        // Still nothing while the read stays stuck: the watcher keeps trying.
        std::thread::sleep(std::time::Duration::from_millis(200));
        assert!(read_file("main", &state, DEV, "main.js").is_err());
        // The folder answers: compared, readable, and the host is told to
        // try again (its read was refused).
        STALL_ENTRY.store(false, Ordering::Release);
        eventually("readable", || {
            read_file("main", &state, DEV, "main.js").is_ok()
        });
        let rows = list_rows("main", &state, &Stub::answering(true)).unwrap();
        assert_eq!(rows[0]["dev_problem"], serde_json::Value::Null);
        assert_eq!(
            bus_list(&state, NOW, &|_: &str| true).unwrap()["plugins"][0]["state"],
            "installed"
        );
        assert!(lock(&recorder.dev)
            .iter()
            .any(|e| e.id == DEV && !e.manifest_changed && e.problem.is_none()));
        state.watch.shutdown();
    }

    #[test]
    fn a_second_window_listing_during_the_first_comparison_reads_nothing() {
        let (temp, state) = setup("unchecked-windows");
        dev_install(&state, &temp.0);
        STALL_WINDOWS.store(true, Ordering::Release);
        let state = restarted(
            state.dir.clone(),
            |root| stalled_while(&STALL_WINDOWS, root),
            10_000,
            std::sync::Arc::new(Recorder::default()),
        );
        std::thread::scope(|scope| {
            let first = scope.spawn(|| list("main", &state, &Stub::answering(true)).map(drop));
            eventually("the first window armed", || {
                state.armed.load(Ordering::Acquire)
            });
            // The second window's list does not wait, and its host may not
            // read what the first window's comparison has not finished.
            list("main-2", &state, &Stub::answering(true)).unwrap();
            for label in ["main", "main-2"] {
                assert_eq!(
                    refusal_name(&read_file(label, &state, DEV, "main.js").unwrap_err()),
                    "folder_missing"
                );
            }
            STALL_WINDOWS.store(false, Ordering::Release);
            first.join().unwrap().unwrap();
        });
        read_file("main-2", &state, DEV, "main.js").expect("compared, then read");
        state.watch.shutdown();
    }

    #[test]
    fn turning_developer_mode_on_reads_no_folder_before_its_comparison() {
        let (temp, state) = setup("unchecked-dev-on");
        dev_install(&state, &temp.0);
        let state = restarted(
            state.dir.clone(),
            |root| stalled_while(&STALL_DEV_ON, root),
            10_000,
            std::sync::Arc::new(Recorder::default()),
        );
        list("main", &state, &Stub::answering(true)).unwrap();
        read_file("main", &state, DEV, "main.js").expect("compared at the first list");
        set_dev_mode("main", &state, false, &Stub::answering(true)).unwrap();
        STALL_DEV_ON.store(true, Ordering::Release);
        std::thread::scope(|scope| {
            let on = scope.spawn(|| set_dev_mode("main", &state, true, &Stub::answering(true)));
            eventually("the mode is on", || state.dev() == DevMode::On);
            assert_eq!(
                refusal_name(&read_file("main", &state, DEV, "main.js").unwrap_err()),
                "folder_missing",
                "on, but not compared yet"
            );
            STALL_DEV_ON.store(false, Ordering::Release);
            on.join().unwrap().unwrap();
        });
        read_file("main", &state, DEV, "main.js").expect("compared");
        state.watch.shutdown();
    }

    #[test]
    fn a_closed_window_takes_its_host_and_the_last_one_takes_the_request() {
        let (_temp, state) = setup("host-gone");
        let ui = Stub::answering(true);
        host_present("main", &state, true, &ui).unwrap();
        host_present("main-2", &state, true, &ui).unwrap();
        let open = |_: &str| true;
        bus_dev(
            &state,
            &serde_json::json!({ "path": "/home/me/p" }),
            NOW,
            &open,
        )
        .unwrap()
        .unwrap();
        // The window closes without its page saying anything.
        host_set(&state, "main", false, &ui);
        assert!(dev_request("main-2", &state, NOW).unwrap().is_some());
        assert!(ui.requests.borrow().is_empty());
        host_set(&state, "main-2", false, &ui);
        assert_eq!(ui.requests.borrow().as_slice(), [None]);
        assert_eq!(dev_request("main", &state, NOW).unwrap(), None);
        assert_eq!(
            bus_dev(
                &state,
                &serde_json::json!({ "path": "/home/me/p" }),
                NOW,
                &open
            )
            .unwrap_err()
            .name,
            "plugins_off"
        );
        // A page that reloads says present again under the same label: one
        // entry, gone with the window.
        host_present("main", &state, true, &ui).unwrap();
        host_present("main", &state, true, &ui).unwrap();
        host_set(&state, "main", false, &ui);
        assert!(lock(&state.hosts).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn the_picker_start_is_judged_by_where_it_really_is() {
        let temp = Temp::new("real-start");
        let store_root = temp.0.join("store");
        std::fs::create_dir_all(store_root.join("com.example.x")).unwrap();
        let elsewhere = temp.0.join("work");
        std::fs::create_dir_all(&elsewhere).unwrap();
        // A link that looks like a plain folder, pointing into the store.
        let link = temp.0.join("innocent");
        std::os::unix::fs::symlink(&store_root, &link).unwrap();
        let roots = [store_root.clone()];
        assert_eq!(real_start(&link, &roots), None);
        assert_eq!(real_start(&store_root.join("com.example.x"), &roots), None);
        assert_eq!(
            real_start(&elsewhere, &roots),
            Some(std::fs::canonicalize(&elsewhere).unwrap())
        );
        assert_eq!(real_start(&temp.0.join("gone"), &roots), None, "not there");
    }

    #[cfg(unix)]
    #[test]
    fn a_folder_path_with_a_break_or_a_direction_change_is_refused_at_the_pick() {
        let (temp, state) = setup("plain-path");
        for name in [
            "new\nline",
            "turn\u{202E}around",
            "bell\u{7}",
            "zero\u{200B}width",
        ] {
            let src = folder(&temp.0, name, &base_manifest());
            assert_eq!(
                refusal_name(&record_pick(&state, &src, PickKind::Folder).unwrap_err()),
                "folder_path_invalid",
                "{name:?}"
            );
        }
        let src = folder(&temp.0, "a \"quoted\" name", &base_manifest());
        record_pick(&state, &src, PickKind::Folder).expect("quotes are plain text");
    }

    #[test]
    fn a_rollback_the_install_rules_refuse_is_refused_before_asking() {
        let (temp, state) = setup("rollback-rules");
        let with = |id: &str, version: &str, command: &str| {
            let mut m: serde_json::Value =
                serde_json::from_str(&manifest(id, version, &[], &[])).unwrap();
            if !command.is_empty() {
                m["contributes"] = serde_json::json!({ "commands": [{
                    "id": command, "class": "read_only", "summary": "Runs.",
                    "params": { "type": "object", "additionalProperties": false },
                    "result": { "type": "object", "additionalProperties": false }
                }] });
            }
            m.to_string()
        };
        let yes = Stub::answering(true);
        for (file, text) in [
            ("a1.hewplugin", with("com.example.a", "1.0.0", "tool.run")),
            ("a2.hewplugin", with("com.example.a", "2.0.0", "")),
            ("t.hewplugin", with("com.example.a.tool", "1.0.0", "run")),
        ] {
            let pkg = package(&temp.0, file, &text);
            let picked = record_pick(&state, &pkg, PickKind::Package).unwrap();
            let view = stage_file("main", &state, &picked, NOW).unwrap();
            commit("main", &state, &view.token, true, &yes, NOW).unwrap();
        }
        let ui = Stub::answering(true);
        assert_eq!(
            refusal_name(&rollback("main", &state, "com.example.a", &ui).unwrap_err()),
            "command_name_taken"
        );
        assert_eq!(ui.asked(), 0, "refused before asking");
    }

    // -----------------------------------------------------------------------
    // Plugin data: per-document storage, state reports, activity, console,
    // and the two live API commands (`super::super::plugin_data`).
    // -----------------------------------------------------------------------

    use super::super::plugin_data as pd;

    const DOC: &str = "3f2c9107-5ed4-40a8-806b-1ec297405d83";

    /// Installs `id` from a `.hewplugin` package (not a folder).
    fn install_package(state: &PluginState, root: &Path, id: &str) {
        let pkg = package(
            root,
            &format!("{id}.hewplugin"),
            &manifest(id, "1.0.0", &[], &[]),
        );
        let picked = record_pick(state, &pkg, PickKind::Package).unwrap();
        let view = stage_file("main", state, &picked, NOW).unwrap();
        commit(
            "main",
            state,
            &view.token,
            true,
            &Stub::answering(true),
            NOW,
        )
        .unwrap();
    }

    fn row(state: &PluginState, id: &str) -> serde_json::Value {
        list_rows("main", state, &Stub::answering(true))
            .unwrap()
            .into_iter()
            .find(|r| r["installed"]["id"] == id || r["id"] == id)
            .expect("listed")
    }

    #[test]
    fn document_storage_round_trips_and_tells_every_window_which_document() {
        let (temp, state) = setup("doc-storage");
        let id = "com.example.d";
        install(&state, &temp.0, id, "1.0.0", &[], &[]);
        let ui = Stub::answering(false);
        assert_eq!(pd::doc_storage_get("main", &state, id, DOC, "a"), Ok(None));
        pd::doc_storage_set("main", &state, id, DOC, "a", "1", &ui).unwrap();
        pd::doc_storage_set("main-2", &state, id, DOC, "b", "2", &ui).unwrap();
        assert_eq!(
            pd::doc_storage_get("settings", &state, id, DOC, "a")
                .unwrap()
                .as_deref(),
            Some("1")
        );
        assert_eq!(
            pd::doc_storage_list("main", &state, id, DOC).unwrap(),
            ["a".to_string(), "b".to_string()]
        );
        pd::doc_storage_delete("main", &state, id, DOC, "a", &ui).unwrap();
        // Deleting what is not there tells no one.
        pd::doc_storage_delete("main", &state, id, DOC, "a", &ui).unwrap();
        assert_eq!(ui.doc_stored.borrow().len(), 3);
        assert_eq!(
            serde_json::to_value(&ui.doc_stored.borrow()[0]).unwrap(),
            serde_json::json!({ "id": id, "scope": "document", "document_id": DOC, "keys": ["a"] })
        );
        // Plugin storage's own payload is unchanged.
        assert!(ui.stored.borrow().is_empty());
        assert_eq!(ui.changed.get(), 0);
        assert_eq!(ui.asked(), 0);
        assert!(temp
            .0
            .join("store.plugin-data")
            .join(id)
            .join("documents")
            .join(format!("{DOC}.json"))
            .is_file());
    }

    #[test]
    fn document_storage_refuses_in_order_and_writes_nothing() {
        let (temp, state) = setup("doc-storage-refused");
        let id = "com.example.d";
        install(&state, &temp.0, id, "1.0.0", &[], &[]);
        let ui = Stub::answering(true);
        // The window first, whatever else is wrong.
        let e = pd::doc_storage_set("library", &state, "../x", "../x", "", "v", &ui).unwrap_err();
        assert_eq!(refusal_name(&e), "window_not_allowed");
        // Then the plugin id, then the document id, before any path.
        let e = pd::doc_storage_set("main", &state, "../x", "../x", "a", "v", &ui).unwrap_err();
        assert_ne!(refusal_name(&e), "document_id_invalid");
        for bad in [
            "",
            "../../plugins",
            "3F2C9107-5ED4-40A8-806B-1EC297405D83",
            "x",
        ] {
            for e in [
                pd::doc_storage_set("main", &state, id, bad, "a", "v", &ui).unwrap_err(),
                pd::doc_storage_get("main", &state, id, bad, "a").unwrap_err(),
                pd::doc_storage_list("main", &state, id, bad).unwrap_err(),
                pd::doc_storage_delete("main", &state, id, bad, "a", &ui).unwrap_err(),
            ] {
                assert_eq!(refusal_name(&e), "document_id_invalid", "{bad:?}");
            }
        }
        let e = pd::doc_storage_set("main", &state, id, DOC, "", "v", &ui).unwrap_err();
        assert_eq!(refusal_name(&e), "storage_key_invalid");
        let big = "x".repeat(plugin_host::MAX_STORAGE_BYTES);
        let e = pd::doc_storage_set("main", &state, id, DOC, "a", &big, &ui).unwrap_err();
        assert_eq!(refusal_name(&e), "storage_quota");
        let none = "com.example.none";
        let e = pd::doc_storage_set("main", &state, none, DOC, "a", "v", &ui).unwrap_err();
        assert_eq!(refusal_name(&e), "not_installed");
        assert!(!temp.0.join("store.plugin-data").join(none).exists());
        assert!(!temp
            .0
            .join("store.plugin-data")
            .join(id)
            .join("documents")
            .join(format!("{DOC}.json"))
            .exists());
        assert!(ui.doc_stored.borrow().is_empty());
    }

    #[test]
    fn state_reports_are_kept_per_window_and_summed_on_the_list() {
        let (temp, state) = setup("states");
        let id = "com.example.r";
        install(&state, &temp.0, id, "1.0.0", &[], &[]);
        let ui = Stub::answering(true);
        let fresh = row(&state, id);
        assert_eq!(fresh["running_in"], 0);
        assert_eq!(fresh["stopped_in"], 0);
        assert_eq!(fresh["blocked_in"], 0);
        assert_eq!(fresh["idle_in"], 0);
        assert_eq!(fresh["last_stop"], serde_json::Value::Null);

        pd::report_state("main", &state, id, "activated", None, 1_000, &ui).unwrap();
        pd::report_state(
            "main-2",
            &state,
            id,
            "crashed",
            Some(" out of\nmemory "),
            2_000,
            &ui,
        )
        .unwrap();
        // The same report again changes nothing and tells no one.
        pd::report_state(
            "main-2",
            &state,
            id,
            "crashed",
            Some("out of memory"),
            2_500,
            &ui,
        )
        .unwrap();
        assert_eq!(*ui.states.borrow(), [id, id]);
        let r = row(&state, id);
        assert_eq!(r["running_in"], 1);
        assert_eq!(r["stopped_in"], 1);
        assert_eq!(
            r["last_stop"],
            serde_json::json!({ "reason": "out of memory", "at": 2_000 })
        );
        // The activity carries the window that called, as Tauri named it.
        let entries = pd::activity_read("settings", &state, id).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(
            (entries[0].window.as_str(), entries[0].state.as_str()),
            ("main-2", "crashed")
        );
        assert_eq!(
            (entries[1].window.as_str(), entries[1].state.as_str()),
            ("main", "activated")
        );

        // A window that closes takes its reports with it.
        pd::window_closed(&state, "main-2", &ui);
        assert_eq!(ui.states.borrow().len(), 3);
        let r = row(&state, id);
        assert_eq!(r["stopped_in"], 0);
        assert_eq!(r["last_stop"], serde_json::Value::Null);
        pd::report_state(
            "main",
            &state,
            id,
            "blocked",
            Some("needs_review"),
            3_000,
            &ui,
        )
        .unwrap();
        assert_eq!(row(&state, id)["blocked_in"], 1);
        assert_eq!(row(&state, id)["running_in"], 0);
        assert_eq!(row(&state, id)["idle_in"], 0);
        // A plugin unloaded for sitting idle is counted apart: it starts
        // again when it is needed.
        pd::report_state("main", &state, id, "idle", Some("idle"), 4_000, &ui).unwrap();
        let r = row(&state, id);
        assert_eq!(r["idle_in"], 1);
        assert_eq!(r["blocked_in"], 0);
        assert_eq!(r["running_in"], 0);
    }

    #[test]
    fn state_reports_refuse_in_order_and_are_rate_bounded() {
        let (temp, state) = setup("states-refused");
        let id = "com.example.r";
        install(&state, &temp.0, id, "1.0.0", &[], &[]);
        let ui = Stub::answering(true);
        let name = |r: Result<(), String>| refusal_name(&r.unwrap_err()).to_string();
        assert_eq!(
            name(pd::report_state(
                "library", &state, "../x", "nope", None, 1, &ui
            )),
            "window_not_allowed"
        );
        assert_ne!(
            name(pd::report_state(
                "main", &state, "../x", "nope", None, 1, &ui
            )),
            "invalid_state"
        );
        assert_eq!(
            name(pd::report_state(
                "main", &state, id, "running", None, 1, &ui
            )),
            "invalid_state"
        );
        assert_eq!(
            name(pd::report_state(
                "main",
                &state,
                "com.example.none",
                "crashed",
                None,
                1,
                &ui
            )),
            "not_installed"
        );
        assert!(!temp
            .0
            .join("store.plugin-data")
            .join("com.example.none")
            .exists());
        // A window may report about a plugin so many times in a while.
        let states = ["activating", "activated"];
        for n in 0..pd::MAX_REPORTS_PER_WINDOW {
            pd::report_state(
                "main",
                &state,
                id,
                states[n as usize % 2],
                None,
                10_000 + u64::from(n),
                &ui,
            )
            .unwrap();
        }
        assert_eq!(
            name(pd::report_state(
                "main", &state, id, "idle", None, 10_100, &ui
            )),
            "report_too_often"
        );
        // A stop is never refused for the budget.
        pd::report_state("main", &state, id, "crashed", None, 10_100, &ui).unwrap();
        assert_eq!(row(&state, id)["stopped_in"], 1);
        // Another window has its own count; the count starts again later.
        pd::report_state("main-2", &state, id, "crashed", None, 10_100, &ui).unwrap();
        pd::report_state(
            "main",
            &state,
            id,
            "crashed",
            None,
            10_000 + pd::REPORT_WINDOW_MS,
            &ui,
        )
        .unwrap();
        // A long reason is cut before it is cleaned, and kept short.
        let long = "why ".repeat(10_000);
        pd::report_state("main", &state, id, "idle", Some(&long), 30_000, &ui).unwrap();
        let entries = pd::activity_read("main", &state, id).unwrap();
        assert!(
            entries[0].reason.as_ref().unwrap().chars().count()
                <= plugin_host::data::MAX_STATE_REASON_CHARS
        );
    }

    #[test]
    fn a_stop_gets_through_a_spent_budget_and_an_unchanged_report_spends_none() {
        let (temp, state) = setup("states-budget");
        let id = "com.example.r";
        install(&state, &temp.0, id, "1.0.0", &[], &[]);
        let ui = Stub::answering(true);
        // The same report again, far past the budget, is never refused.
        for n in 0..(3 * pd::MAX_REPORTS_PER_WINDOW) {
            pd::report_state(
                "main",
                &state,
                id,
                "activated",
                None,
                10_000 + u64::from(n),
                &ui,
            )
            .unwrap();
        }
        // So the budget is untouched: a full set of changes still fits.
        let states = ["activating", "activated"];
        for n in 0..pd::MAX_REPORTS_PER_WINDOW - 1 {
            pd::report_state(
                "main",
                &state,
                id,
                states[n as usize % 2],
                None,
                10_200,
                &ui,
            )
            .unwrap();
        }
        // Spent now: a change is refused, a stop and a block are not.
        let err = pd::report_state("main", &state, id, "idle", None, 10_300, &ui).unwrap_err();
        assert_eq!(refusal_name(&err), "report_too_often");
        pd::report_state("main", &state, id, "blocked", None, 10_300, &ui).unwrap();
        assert_eq!(row(&state, id)["blocked_in"], 1);
        // Stop to stop is budgeted like any change.
        let err =
            pd::report_state("main", &state, id, "crashed", Some("boom"), 10_400, &ui).unwrap_err();
        assert_eq!(refusal_name(&err), "report_too_often");
        assert_eq!(row(&state, id)["blocked_in"], 1);
    }

    #[test]
    fn one_window_cannot_alternate_stops_through_a_spent_budget() {
        let (temp, state) = setup("states-alternate");
        let id = "com.example.r";
        install(&state, &temp.0, id, "1.0.0", &[], &[]);
        let ui = Stub::answering(true);
        let states = ["activating", "activated"];
        for n in 0..pd::MAX_REPORTS_PER_WINDOW {
            pd::report_state(
                "main",
                &state,
                id,
                states[n as usize % 2],
                None,
                10_000,
                &ui,
            )
            .unwrap();
        }
        pd::report_state("main", &state, id, "crashed", None, 10_001, &ui).unwrap();
        let logged = pd::activity_read("main", &state, id).unwrap().len();
        // The held stop never changes, so each try to flip it is refused
        // (before the fix each one was let through and logged).
        for _ in 0..20 {
            let err =
                pd::report_state("main", &state, id, "blocked", None, 10_002, &ui).unwrap_err();
            assert_eq!(refusal_name(&err), "report_too_often");
        }
        assert_eq!(pd::activity_read("main", &state, id).unwrap().len(), logged);
        assert_eq!(row(&state, id)["stopped_in"], 1);
    }

    #[test]
    fn a_page_start_clears_the_held_sequence_and_a_huge_one_is_not_believed() {
        let (temp, state) = setup("states-seq-reset");
        let id = "com.example.r";
        install(&state, &temp.0, id, "1.0.0", &[], &[]);
        let ui = Stub::answering(true);
        host_present("main", &state, true, &ui).unwrap();
        let open = |_: &str| true;
        let send = |name: &str, seq: Option<u64>, now: u64| {
            pd::report_state_from("main", &state, id, name, None, seq, now, &open, &ui)
        };
        // A huge number is as if there were none: it pins nothing.
        send("activated", Some(u64::MAX), 1_000).unwrap();
        send("idle", Some(1), 1_001).unwrap();
        assert_eq!(row(&state, id)["idle_in"], 1);
        // A number the clock could have made is kept and orders reports.
        send("activated", Some(500), 1_002).unwrap();
        send("crashed", Some(400), 1_003).unwrap();
        assert_eq!(row(&state, id)["stopped_in"], 0);
        // The page reloads and counts from the start again.
        host_present("main", &state, true, &ui).unwrap();
        send("crashed", Some(1), 1_004).unwrap();
        assert_eq!(row(&state, id)["stopped_in"], 1);
    }

    #[test]
    fn an_older_report_by_sequence_number_is_ignored() {
        let (temp, state) = setup("states-seq");
        let id = "com.example.r";
        install(&state, &temp.0, id, "1.0.0", &[], &[]);
        let ui = Stub::answering(true);
        let open = |_: &str| true;
        let send = |state_name: &str, seq: Option<u64>, now: u64| {
            pd::report_state_from("main", &state, id, state_name, None, seq, now, &open, &ui)
        };
        // The stop arrives first, then the activation sent before it.
        send("crashed", Some(200), 1_000).unwrap();
        send("activated", Some(100), 1_001).unwrap();
        assert_eq!(row(&state, id)["stopped_in"], 1);
        assert_eq!(row(&state, id)["running_in"], 0);
        assert_eq!(pd::activity_read("main", &state, id).unwrap().len(), 1);
        // An unchanged report with a higher number raises the held one.
        send("crashed", Some(300), 1_002).unwrap();
        send("idle", Some(250), 1_003).unwrap();
        assert_eq!(row(&state, id)["idle_in"], 0);
        // Equal is not older; none at all behaves as before.
        send("idle", Some(300), 1_004).unwrap();
        assert_eq!(row(&state, id)["idle_in"], 1);
        send("activated", None, 1_005).unwrap();
        assert_eq!(row(&state, id)["running_in"], 1);
        // The held number survives a report without one.
        send("crashed", Some(299), 1_006).unwrap();
        assert_eq!(row(&state, id)["running_in"], 1);
        // Each window has its own.
        pd::report_state_from(
            "main-2",
            &state,
            id,
            "crashed",
            None,
            Some(1),
            1_007,
            &open,
            &ui,
        )
        .unwrap();
        assert_eq!(row(&state, id)["stopped_in"], 1);
    }

    #[test]
    fn a_report_from_a_window_that_is_gone_leaves_nothing_behind() {
        let (temp, state) = setup("states-gone");
        let id = "com.example.r";
        install(&state, &temp.0, id, "1.0.0", &[], &[]);
        let ui = Stub::answering(true);
        pd::window_closed(&state, "main-2", &ui);
        pd::report_state_from(
            "main-2",
            &state,
            id,
            "crashed",
            None,
            None,
            1_000,
            &|l: &str| l != "main-2",
            &ui,
        )
        .unwrap();
        assert_eq!(row(&state, id)["stopped_in"], 0);
        assert!(pd::activity_read("main", &state, id).unwrap().is_empty());
        assert!(ui.states.borrow().is_empty());
        // An open window's report is kept, and closing the window clears it.
        pd::report_state("main-2", &state, id, "crashed", None, 1_100, &ui).unwrap();
        assert_eq!(row(&state, id)["stopped_in"], 1);
        pd::window_closed(&state, "main-2", &ui);
        assert_eq!(row(&state, id)["stopped_in"], 0);
    }

    #[test]
    fn a_flush_leaves_no_moment_when_a_read_misses_the_entries() {
        let (temp, state) = setup("flush-lock");
        let id = "com.example.r";
        install(&state, &temp.0, id, "1.0.0", &[], &[]);
        let ui = Stub::answering(true);
        pd::report_state("main", &state, id, "activated", None, 1_000, &ui).unwrap();
        pd::report_state("main", &state, id, "idle", None, 1_100, &ui).unwrap();
        // The idle entry is queued (written at most once a second). Holding
        // the store lock, as a read does, a flush must wait for it, so it
        // cannot have drained the queue before the read.
        let guard = state.store_lock();
        let queued = std::thread::scope(|s| {
            let flusher = s.spawn(|| pd::flush_activity(&state, 5_000, true));
            std::thread::sleep(std::time::Duration::from_millis(200));
            let seen = pd::queued_len(&state, id);
            drop(guard);
            flusher.join().unwrap();
            seen
        });
        assert_eq!(
            queued, 1,
            "the flush drained the queue before taking the store lock"
        );
        assert_eq!(pd::activity_read("main", &state, id).unwrap().len(), 2);
    }

    #[test]
    fn activity_is_written_at_most_once_a_second_and_forgotten_on_uninstall() {
        let (temp, state) = setup("activity");
        let id = "com.example.r";
        install(&state, &temp.0, id, "1.0.0", &[], &[]);
        let ui = Stub::answering(true);
        let dir = state.dir().unwrap().clone();
        let on_disk = || plugin_host::data::activity_read(&dir, id).unwrap().len();
        pd::report_state("main", &state, id, "activating", None, 100_000, &ui).unwrap();
        assert_eq!(on_disk(), 1, "the first report is written at once");
        pd::report_state("main", &state, id, "activated", None, 100_200, &ui).unwrap();
        pd::report_state("main", &state, id, "idle", None, 100_400, &ui).unwrap();
        assert_eq!(on_disk(), 1, "within the second, queued");
        assert_eq!(
            pd::activity_read("main", &state, id).unwrap().len(),
            3,
            "read with the queue"
        );
        pd::flush_activity(&state, 100_900, false);
        assert_eq!(on_disk(), 1);
        pd::flush_activity(&state, 101_000, false);
        assert_eq!(on_disk(), 3, "one write for both");
        pd::report_state("main", &state, id, "crashed", None, 101_100, &ui).unwrap();
        pd::flush_activity(&state, 101_200, true);
        assert_eq!(on_disk(), 4, "the exit writes what waits");
        assert!(temp
            .0
            .join("store.plugin-data")
            .join(id)
            .join("activity.json")
            .is_file());
        // Uninstalling removes the log and forgets what was reported.
        pd::report_state("main", &state, id, "activated", None, 101_300, &ui).unwrap();
        uninstall("main", &state, id, &Stub::answering(true)).unwrap();
        assert!(!temp.0.join("store.plugin-data").join(id).exists());
        pd::flush_activity(&state, 200_000, true);
        assert!(
            !temp.0.join("store.plugin-data").join(id).exists(),
            "nothing written for a plugin that is gone"
        );
        install(&state, &temp.0, id, "1.0.0", &[], &[]);
        let r = row(&state, id);
        assert_eq!(
            (r["running_in"].clone(), r["stopped_in"].clone()),
            (0.into(), 0.into())
        );
        assert_eq!(pd::activity_read("main", &state, id).unwrap(), vec![]);
    }

    #[test]
    fn console_lines_are_cleaned_bounded_and_refused_past_the_budget() {
        let (temp, state) = setup("console");
        let id = "com.example.c";
        install(&state, &temp.0, id, "1.0.0", &[], &[]);
        let ui = Stub::answering(true);
        let now = 10_000_000;
        let line = |level: &str, text: &str, at: f64| pd::ConsoleLineIn {
            level: level.into(),
            text: text.into(),
            at,
        };
        let done = pd::console_append(
            "main",
            &state,
            id,
            &[
                line("log", "one", (now - 5) as f64),
                line("error", "two\u{202e}\nlines", 1e300),
            ],
            now,
            &ui,
        )
        .unwrap();
        assert_eq!(
            done,
            pd::ConsoleAppended {
                accepted: 2,
                dropped: 0
            }
        );
        assert_eq!(*ui.consoles.borrow(), [id]);
        let read = pd::console_read("main", &state, id, None).unwrap();
        assert_eq!(read.len(), 2);
        assert_eq!(
            (read[0].text.as_str(), read[0].at),
            ("two lines", now),
            "a time from the page is clamped"
        );
        assert_eq!(read[1].at, now - 5);
        let name =
            |r: Result<pd::ConsoleAppended, String>| refusal_name(&r.unwrap_err()).to_string();
        assert_eq!(
            name(pd::console_append("library", &state, id, &[], now, &ui)),
            "window_not_allowed"
        );
        let many: Vec<pd::ConsoleLineIn> = (0..=pd::MAX_CONSOLE_LINES_PER_CALL)
            .map(|_| line("log", "x", 0.0))
            .collect();
        assert_eq!(
            name(pd::console_append("main", &state, id, &many, now, &ui)),
            "too_many_lines"
        );
        assert_eq!(
            name(pd::console_append(
                "main",
                &state,
                id,
                &[line("trace", "x", 0.0)],
                now,
                &ui
            )),
            "invalid_level"
        );
        assert_eq!(
            name(pd::console_append(
                "main",
                &state,
                "com.example.none",
                &[line("log", "x", 0.0)],
                now,
                &ui
            )),
            "not_installed"
        );
        // Past the budget, lines are refused and counted, not queued.
        let text = "y".repeat(2_000);
        // Batches that do not divide the budget evenly, so one is cut short.
        let batch: Vec<pd::ConsoleLineIn> = (0..50).map(|_| line("info", &text, 0.0)).collect();
        let mut accepted = 0;
        let mut dropped = 0;
        loop {
            match pd::console_append("main-2", &state, id, &batch, now + 1, &ui) {
                Ok(done) => {
                    accepted += done.accepted;
                    dropped += done.dropped;
                }
                Err(e) => {
                    assert_eq!(refusal_name(&e), "console_rate_limited");
                    break;
                }
            }
        }
        assert!(dropped > 0);
        assert!((accepted as u64) * 2_000 <= pd::CONSOLE_BYTES_PER_WINDOW);
        // The budget comes back after its window.
        pd::console_append(
            "main",
            &state,
            id,
            &[line("log", "later", 0.0)],
            now + pd::CONSOLE_BUDGET_WINDOW_MS + 1,
            &ui,
        )
        .unwrap();
        assert_eq!(
            pd::console_read("main", &state, id, Some(1)).unwrap()[0].text,
            "later"
        );
    }

    #[test]
    fn an_empty_console_batch_still_needs_the_plugin_installed() {
        let (temp, state) = setup("console-empty");
        let id = "com.example.c";
        install(&state, &temp.0, id, "1.0.0", &[], &[]);
        let ui = Stub::answering(true);
        let e =
            pd::console_append("main", &state, "com.example.none", &[], 1_000, &ui).unwrap_err();
        assert_eq!(refusal_name(&e), "not_installed");
        assert_eq!(
            pd::console_append("main", &state, id, &[], 1_000, &ui).unwrap(),
            pd::ConsoleAppended {
                accepted: 0,
                dropped: 0
            }
        );
    }

    #[test]
    fn the_console_budget_counts_each_line_as_the_log_stores_it() {
        let (temp, state) = setup("console-escaped");
        let id = "com.example.c";
        install(&state, &temp.0, id, "1.0.0", &[], &[]);
        let ui = Stub::answering(true);
        let now = 10_000_000;
        // A quote takes two bytes in the log's JSON.
        let text = "\"".repeat(2_000);
        let stored = serde_json::to_vec(&plugin_host::data::ConsoleLine {
            at: now,
            level: "info".into(),
            text: text.clone(),
        })
        .unwrap()
        .len() as u64
            + 1;
        assert!(stored > 4_000);
        let batch: Vec<pd::ConsoleLineIn> = (0..64)
            .map(|_| pd::ConsoleLineIn {
                level: "info".into(),
                text: text.clone(),
                at: 0.0,
            })
            .collect();
        let mut accepted = 0u64;
        loop {
            match pd::console_append("main", &state, id, &batch, now, &ui) {
                Ok(done) => accepted += done.accepted as u64,
                Err(e) => {
                    assert_eq!(refusal_name(&e), "console_rate_limited");
                    break;
                }
            }
        }
        assert!(accepted * stored <= pd::CONSOLE_BYTES_PER_WINDOW);
        assert!(
            (accepted + 1) * stored > pd::CONSOLE_BYTES_PER_WINDOW,
            "refused only once one more line would pass the budget"
        );
    }

    #[test]
    fn reloading_a_plugin_whose_record_cannot_be_read_gives_out_no_path() {
        let (temp, state) = setup("bus-reload-unreadable");
        let id = "com.example.folder";
        install(&state, &temp.0, id, "1.0.0", &[], &[]);
        let ui = Stub::answering(true);
        host_present("main", &state, true, &ui).unwrap();
        let record = temp.0.join("store").join(id).join("installed.json");
        std::fs::remove_file(&record).unwrap();
        std::fs::create_dir(&record).unwrap();
        let e = pd::bus_reload(&state, &serde_json::json!({ "id": id }), 10_000, &|_| true)
            .unwrap_err();
        assert_eq!(e.name, "installed_unreadable");
        let root = temp.0.to_string_lossy().into_owned();
        assert!(!e.explanation.contains(&root), "{}", e.explanation);
        assert!(
            !e.explanation.contains('/') && !e.explanation.contains('\\'),
            "{}",
            e.explanation
        );
    }

    #[test]
    fn reloading_a_turned_off_folder_is_refused_and_spends_no_slot() {
        let (temp, state) = setup("bus-reload-off");
        let id = "com.example.folder";
        install(&state, &temp.0, id, "1.0.0", &[], &[]);
        let ui = Stub::answering(true);
        host_present("main", &state, true, &ui).unwrap();
        let params = serde_json::json!({ "id": id });
        set_enabled("main", &state, id, false, &ui).unwrap();
        let e = pd::bus_reload(&state, &params, 10_000, &|_| true).unwrap_err();
        assert_eq!(e.name, "plugin_turned_off");
        // Turned on again, the same moment's reload is not "too soon".
        set_enabled("main", &state, id, true, &ui).unwrap();
        assert!(pd::bus_reload(&state, &params, 10_000, &|_| true).is_ok());
    }

    #[test]
    fn a_grant_for_a_record_waiting_for_review_is_refused_before_any_dialog() {
        let (temp, state) = setup("grant-review");
        let src = dev_install(&state, &temp.0);
        list("main", &state, &Stub::answering(true)).unwrap();
        write_manifest(&src, &wider());
        folder_changed(&state, DEV, &Stub::answering(true));
        assert_eq!(marked_reason(&state), Some("permissions_changed"));
        let ui = Stub::answering(true);
        let e = set_grant("main", &state, DEV, &["model.read".into()], &[], &ui).unwrap_err();
        assert_eq!(refusal_name(&e), "dev_consent_pending");
        assert_eq!(ui.asked(), 0);
        state.watch.shutdown();
    }

    #[test]
    fn the_live_api_reads_activity_and_reloads_only_a_running_developer_folder() {
        let (temp, state) = setup("bus-reload");
        let folder_id = "com.example.folder";
        let package_id = "com.example.package";
        install(&state, &temp.0, folder_id, "1.0.0", &[], &[]);
        install_package(&state, &temp.0, package_id);
        let ui = Stub::answering(true);
        let all_open = |_: &str| true;
        let params = |id: &str| serde_json::json!({ "id": id });
        let name = |r: Result<DevChanged, BusRefusal>| r.unwrap_err().name;

        // hew.plugin.activity
        for bad in [
            serde_json::json!({}),
            serde_json::json!({ "id": 1 }),
            params("../x"),
        ] {
            assert_eq!(
                pd::bus_activity(&state, &bad).unwrap_err().name,
                "invalid_params"
            );
        }
        assert_eq!(
            pd::bus_activity(&state, &params("com.example.none"))
                .unwrap_err()
                .name,
            "not_installed"
        );
        pd::report_state("main", &state, folder_id, "activated", None, 1_000, &ui).unwrap();
        let answer = pd::bus_activity(&state, &params(folder_id)).unwrap();
        assert_eq!(answer["entries"][0]["state"], "activated");
        assert_eq!(answer["entries"][0]["window"], "main");

        // hew.plugin.reload
        assert_eq!(
            name(pd::bus_reload(
                &state,
                &serde_json::json!({}),
                10_000,
                &all_open
            )),
            "invalid_params"
        );
        assert_eq!(
            name(pd::bus_reload(
                &state,
                &params(folder_id),
                10_000,
                &all_open
            )),
            "plugins_off"
        );
        host_present("main", &state, true, &ui).unwrap();
        // A host in a window that is gone counts for nothing.
        assert_eq!(
            name(pd::bus_reload(&state, &params(folder_id), 10_000, &|_| {
                false
            })),
            "plugins_off"
        );
        host_present("main", &state, true, &ui).unwrap();
        state.dev_mode.store(false, Ordering::Release);
        assert_eq!(
            name(pd::bus_reload(
                &state,
                &params(folder_id),
                10_000,
                &all_open
            )),
            "developer_mode_off"
        );
        state.dev_mode.store(true, Ordering::Release);
        assert_eq!(
            name(pd::bus_reload(
                &state,
                &params("com.example.none"),
                10_000,
                &all_open
            )),
            "not_installed"
        );
        assert_eq!(
            name(pd::bus_reload(
                &state,
                &params(package_id),
                10_000,
                &all_open
            )),
            "not_a_developer_folder"
        );
        // A plugin that stopped stays stopped; one that is blocked stays so.
        pd::report_state(
            "main-2",
            &state,
            folder_id,
            "crashed",
            Some("timeout"),
            2_000,
            &ui,
        )
        .unwrap();
        assert_eq!(
            name(pd::bus_reload(
                &state,
                &params(folder_id),
                10_000,
                &all_open
            )),
            "plugin_stopped"
        );
        // Only open windows count.
        assert!(
            pd::bus_reload(&state, &params(folder_id), 10_000, &|l: &str| l != "main-2").is_ok()
        );
        pd::window_closed(&state, "main-2", &ui);
        pd::report_state("main", &state, folder_id, "blocked", None, 3_000, &ui).unwrap();
        assert_eq!(
            name(pd::bus_reload(
                &state,
                &params(folder_id),
                20_000,
                &all_open
            )),
            "plugin_blocked"
        );
        pd::report_state("main", &state, folder_id, "activated", None, 4_000, &ui).unwrap();
        let changed = pd::bus_reload(&state, &params(folder_id), 20_000, &all_open).unwrap();
        assert_eq!(
            serde_json::to_value(&changed).unwrap(),
            serde_json::json!({ "id": folder_id, "manifest_changed": false, "reason": null, "problem": null })
        );
        assert_eq!(
            name(pd::bus_reload(
                &state,
                &params(folder_id),
                20_000 + pd::RELOAD_INTERVAL_MS - 1,
                &all_open
            )),
            "reload_too_soon"
        );
        assert!(pd::bus_reload(
            &state,
            &params(folder_id),
            20_000 + pd::RELOAD_INTERVAL_MS,
            &all_open
        )
        .is_ok());
        // A folder change waiting for review blocks it too.
        let src = temp.0.join(format!("{folder_id}-1.0.0"));
        std::fs::write(
            src.join("hew-plugin.json"),
            manifest(folder_id, "1.0.0", &["model.write"], &[]),
        )
        .unwrap();
        let bytes = std::fs::read(src.join("hew-plugin.json")).unwrap();
        let outcome = store::apply_folder_manifest(
            state.dir().unwrap(),
            folder_id,
            Ok(bytes),
            running_version(),
        )
        .unwrap();
        assert!(matches!(outcome, FolderOutcome::Marked(_)));
        assert_eq!(
            name(pd::bus_reload(
                &state,
                &params(folder_id),
                60_000,
                &all_open
            )),
            "plugin_blocked"
        );
        // Turned off as well as blocked: blocked is named first; turned off
        // alone is plugin_turned_off, and neither spends the reload slot.
        set_enabled("main", &state, folder_id, false, &ui).unwrap();
        assert_eq!(
            name(pd::bus_reload(
                &state,
                &params(folder_id),
                70_000,
                &all_open
            )),
            "plugin_blocked"
        );
    }
}

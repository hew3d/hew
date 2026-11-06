//! Follow a bug report, desktop side (docs/design/follow-a-report.md §4.2,
//! §5.3, §5.4, §6.2, §6.3).
//!
//! A reporter who ticks "follow" at submit gets a 43-character secret from
//! the intake service. It is kept here, in Rust only, in
//! `<app config dir>/followed-reports.json` (written atomically, `0600` on
//! Unix). Every thread operation is a Tauri command that reads the secret
//! from that file; the webview only ever sees `FollowedReport`, which has no
//! secret field. Nothing here logs a secret, a message body, or a URL with
//! one in it (the secret travels only in the `Hew-Follow-Secret` header).
//!
//! The pure parts (store round trip, merge, seen/done transitions, the
//! 6-hour / 365-day launch policy, header building) take explicit paths and
//! clocks so they unit-test without an app handle or a network.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager};

use crate::report_client::{self, ReportError, ReportErrorKind, ReportSubmitResult};

const STORE_FILE: &str = "followed-reports.json";
/// Hint to the store format; bump only with a migration.
const STORE_FORMAT: u32 = 1;

/// Launch policy (§6.3).
const LAUNCH_DELAY: Duration = Duration::from_secs(15);
const LAUNCH_MIN_INTERVAL_MS: i64 = 6 * 60 * 60 * 1000;
const AUTO_POLL_MAX_AGE_MS: i64 = 365 * 24 * 60 * 60 * 1000;
const LAUNCH_TIMEOUT: Duration = Duration::from_secs(10);
/// Interactive thread calls (manual check, post, seen, forget).
const THREAD_TIMEOUT: Duration = Duration::from_secs(20);

/// `ReportError::message` for a 403 with the service's `forbidden` body.
pub(crate) const FORBIDDEN_MESSAGE: &str = "forbidden";
const EXCERPT_MAX_CHARS: usize = 200;
const OPTIONS_HEADER: &str = "hew-options";

/// Native menu id and label keys of Help ▸ Sent Reports…
pub const MENU_ID: &str = "help-sent-reports";
const LABEL_KEY: &str = "help-sent-reports";
const LABEL_KEY_NEW: &str = "help-sent-reports-new";

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    Open,
    NeedInfo,
    Fixed,
    Closed,
    Gone,
}

impl Status {
    fn parse(text: &str) -> Option<Self> {
        match text {
            "open" => Some(Self::Open),
            "need-info" => Some(Self::NeedInfo),
            "fixed" => Some(Self::Fixed),
            "closed" => Some(Self::Closed),
            "gone" => Some(Self::Gone),
            _ => None,
        }
    }

    fn is_final(self) -> bool {
        matches!(self, Self::Fixed | Self::Closed | Self::Gone)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum State {
    Following,
    Done,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub seq: u64,
    pub at: i64,
    pub author: String,
    pub kind: String,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachment_report_id: Option<String>,
}

/// What the webview sees: everything about a followed report except the
/// secret.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FollowedReport {
    pub id: String,
    pub sent_at: i64,
    pub app_version: String,
    pub excerpt: String,
    #[serde(default)]
    pub file_name: Option<String>,
    pub status: Status,
    #[serde(default)]
    pub status_version: Option<String>,
    #[serde(default)]
    pub status_at: Option<i64>,
    #[serde(default)]
    pub messages: Vec<Message>,
    #[serde(default)]
    pub latest: u64,
    #[serde(default)]
    pub seen_up_to: u64,
    #[serde(default)]
    pub last_checked_at: Option<i64>,
    pub state: State,
}

/// A stored record: the secret plus the view. Never serialized to the
/// webview.
#[derive(Clone, Serialize, Deserialize)]
struct Record {
    secret: String,
    /// Highest `seen_up_to` the service has acknowledged; internal, so the
    /// webview never sees it. Old files read as 0 (the mark is resent).
    #[serde(default, rename = "seenSynced")]
    seen_synced: u64,
    #[serde(flatten)]
    report: FollowedReport,
}

impl std::fmt::Debug for Record {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Record")
            .field("secret", &"<redacted>")
            .field("report", &self.report)
            .finish()
    }
}

#[derive(Serialize, Deserialize)]
struct StoreFile {
    format: u32,
    reports: Vec<Record>,
}

/// The `options` the webview passes alongside `report_submit`'s raw body.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SubmitOptions {
    pub follow: bool,
    pub excerpt: String,
    pub file_name: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckSummary {
    /// Records a request was made for.
    pub checked: u32,
    /// Records that received at least one new unread message.
    pub with_news: u32,
    /// Total new unread messages across those records.
    pub new_count: u32,
    /// Requests that failed for a reason other than 403 (offline, 5xx, 429).
    pub failed: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckResult {
    pub reports: Vec<FollowedReport>,
    pub summary: CheckSummary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CheckReason {
    Launch,
    Manual,
}

/// The GET thread body (§5.4).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ThreadView {
    #[serde(default)]
    status: String,
    #[serde(default)]
    status_version: Option<String>,
    #[serde(default)]
    status_at: Option<i64>,
    #[serde(default)]
    latest: u64,
    #[serde(default)]
    messages: Vec<Message>,
}

// ---------------------------------------------------------------------------
// Pure helpers: time, policy, state transitions
// ---------------------------------------------------------------------------

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// §6.3: the launch check polls a record only while it is `following`, was
/// last checked at least 6 hours ago (or never), and was sent no more than
/// 365 days ago. A last-check time in the future (clock set back) counts as
/// due, so a bad clock cannot silence a record forever.
fn due_for_launch(report: &FollowedReport, now: i64) -> bool {
    if report.state != State::Following {
        return false;
    }
    if now.saturating_sub(report.sent_at) > AUTO_POLL_MAX_AGE_MS {
        return false;
    }
    match report.last_checked_at {
        None => true,
        Some(t) => t > now || now - t >= LAUNCH_MIN_INTERVAL_MS,
    }
}

/// The seq of the newest `status` message, or 0 when there is none.
fn last_status_seq(report: &FollowedReport) -> u64 {
    report
        .messages
        .iter()
        .filter(|m| m.kind == "status")
        .map(|m| m.seq)
        .max()
        .unwrap_or(0)
}

/// A message the reporter should be told about: anything from the
/// maintainer, or a status/system change.
fn is_news(message: &Message) -> bool {
    message.author == "maintainer" || message.author == "system" || message.kind == "status"
}

fn unread_count(report: &FollowedReport) -> u32 {
    report
        .messages
        .iter()
        .filter(|m| m.seq > report.seen_up_to && is_news(m))
        .count() as u32
}

/// Folds a GET thread response into the record. Returns how many newly
/// arrived messages are unread news. A reply that reopens a `done` record
/// (status back to open or need-info) puts it back to `following`.
fn merge_thread(report: &mut FollowedReport, view: ThreadView, now: i64) -> u32 {
    let mut fresh = 0;
    // Judged against the stored `latest` before this response's messages
    // raise it.
    let fresh_view = view.latest >= report.latest;
    for message in view.messages {
        if report.messages.iter().any(|m| m.seq == message.seq) {
            continue;
        }
        if message.seq > report.seen_up_to && is_news(&message) {
            fresh += 1;
        }
        report.latest = report.latest.max(message.seq);
        report.messages.push(message);
    }
    report.messages.sort_by_key(|m| m.seq);
    // A view older than what is stored (a slow concurrent refresh) must not
    // roll the status back.
    report.latest = report.latest.max(view.latest);
    if fresh_view {
        if let Some(status) = Status::parse(&view.status) {
            report.status = status;
            report.status_version = view.status_version;
            report.status_at = view.status_at;
        }
    }
    report.last_checked_at = Some(now);
    if report.state == State::Done && !report.status.is_final() {
        report.state = State::Following;
    }
    fresh
}

/// The reporter viewed the thread through `up_to`. Moves the record to
/// `done` when its status is final and the final status message has been
/// viewed.
fn mark_seen(report: &mut FollowedReport, up_to: u64) {
    let up_to = up_to.min(report.latest);
    report.seen_up_to = report.seen_up_to.max(up_to);
    if report.status.is_final() && report.seen_up_to >= last_status_seq(report) {
        report.state = State::Done;
    }
}

/// A 403: the service no longer knows this thread (§4.1).
fn mark_gone(report: &mut FollowedReport, now: i64) {
    report.status = Status::Gone;
    report.state = State::Done;
    report.last_checked_at = Some(now);
}

fn truncate_excerpt(text: &str) -> String {
    text.trim().chars().take(EXCERPT_MAX_CHARS).collect()
}

// ---------------------------------------------------------------------------
// Headers
// ---------------------------------------------------------------------------

/// The extra START headers: `Hew-Follow: 1` for a new report that wants a
/// thread, or `Hew-Follow-Up` + `Hew-Follow-Secret` for an attachment to a
/// followed report (a follow-up never sends `Hew-Follow`, §4.3).
pub fn start_headers(follow: bool, follow_up: Option<(&str, &str)>) -> Vec<(&'static str, String)> {
    match follow_up {
        Some((parent, secret)) => vec![
            ("Hew-Follow-Up", parent.to_owned()),
            ("Hew-Follow-Secret", secret.to_owned()),
        ],
        None if follow => vec![("Hew-Follow", "1".to_owned())],
        None => Vec::new(),
    }
}

/// Decodes `value`, a `encodeURIComponent`'d JSON string, into options.
fn parse_options(value: &str) -> Option<SubmitOptions> {
    // form_urlencoded's decoder handles %XX; encodeURIComponent never emits
    // a literal '+' (it becomes %2B), so '+' -> ' ' cannot corrupt it.
    let probe = format!("o={value}");
    let (_, decoded) = url::form_urlencoded::parse(probe.as_bytes()).next()?;
    serde_json::from_str(&decoded)
        .ok()
        .or_else(|| serde_json::from_str(&replace_lone_surrogates(&decoded)).ok())
}

/// `JSON.stringify` writes a lone UTF-16 surrogate (a code unit cut out of
/// a pair) as a `\udXXX` escape, which serde_json rejects. Replaces each
/// such escape with `\ufffd`; a valid high+low pair is left alone.
fn replace_lone_surrogates(json: &str) -> String {
    fn hex4(b: &[u8]) -> Option<u32> {
        let text = std::str::from_utf8(b.get(..4)?).ok()?;
        u32::from_str_radix(text, 16).ok()
    }
    let bytes = json.as_bytes();
    let mut out = String::with_capacity(json.len());
    let mut i = 0;
    let mut start = 0;
    while i < bytes.len() {
        if bytes[i] != b'\\' {
            i += 1;
            continue;
        }
        // An escape: copy what came before, then decide.
        if bytes.get(i + 1) == Some(&b'u') {
            if let Some(unit) = bytes.get(i + 2..).and_then(hex4) {
                let low_follows = bytes.get(i + 6..i + 8) == Some(b"\\u".as_slice())
                    && bytes
                        .get(i + 8..)
                        .and_then(hex4)
                        .is_some_and(|l| (0xDC00..=0xDFFF).contains(&l));
                if (0xD800..=0xDBFF).contains(&unit) && low_follows {
                    i += 12; // a valid pair
                    continue;
                }
                if (0xD800..=0xDFFF).contains(&unit) {
                    out.push_str(&json[start..i]);
                    out.push_str("\\ufffd");
                    i += 6;
                    start = i;
                    continue;
                }
            }
        }
        i += 2; // skip the escaped character (`\\`, `\"`, ...)
    }
    out.push_str(&json[start..]);
    out
}

/// Options for a raw-body command ride in the `hew-options` header as
/// URL-encoded JSON (a raw invoke has no named arguments). Absent or
/// unparseable means defaults, so existing callers keep working.
pub fn parse_options_header(headers: &tauri::http::HeaderMap) -> SubmitOptions {
    headers
        .get(OPTIONS_HEADER)
        .and_then(|v| v.to_str().ok())
        .and_then(parse_options)
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Store (path based)
// ---------------------------------------------------------------------------

/// Serializes read-modify-write cycles on the store file.
static STORE_LOCK: Mutex<()> = Mutex::new(());

fn store_path(app: &tauri::AppHandle) -> Option<PathBuf> {
    app.path().app_config_dir().ok().map(|d| d.join(STORE_FILE))
}

/// The stored records. A missing file is an empty store. Any other read
/// error is returned (callers that write must not overwrite what they could
/// not read). A file that is not JSON, or has no `reports` array, is renamed
/// to `followed-reports.json.corrupt-<ms>` (best effort) and reads as empty,
/// so the next write cannot destroy the only copy. A single record that does
/// not parse is dropped and the others are kept.
fn load_records(path: &Path) -> std::io::Result<Vec<Record>> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let records = serde_json::from_slice::<serde_json::Value>(&bytes)
        .ok()
        .and_then(|v| match v {
            serde_json::Value::Object(mut o) => match o.remove("reports") {
                Some(serde_json::Value::Array(a)) => Some(a),
                _ => None,
            },
            _ => None,
        });
    let Some(items) = records else {
        eprintln!("hew: quarantining unreadable {STORE_FILE}");
        let stamp = now_ms();
        let mut quarantine = path.as_os_str().to_owned();
        quarantine.push(format!(".corrupt-{stamp}"));
        let _ = std::fs::rename(path, PathBuf::from(quarantine));
        return Ok(Vec::new());
    };
    // One record from a newer build (an unknown status) or a damaged one
    // must not hide the rest, so each parses on its own.
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        match serde_json::from_value::<Record>(item) {
            Ok(r) => out.push(r),
            Err(_) => eprintln!("hew: dropping an unreadable record in {STORE_FILE}"),
        }
    }
    Ok(out)
}

/// Temp file + rename, `0600` from creation on Unix.
fn save_records(path: &Path, records: &[Record]) -> std::io::Result<()> {
    use std::io::Write;
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(dir)?;
    let text = serde_json::to_string_pretty(&StoreFile {
        format: STORE_FORMAT,
        reports: records.to_vec(),
    })
    .map_err(std::io::Error::other)?;
    let tmp = dir.join(format!("{STORE_FILE}.tmp-{}", std::process::id()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = (|| {
        let mut file = options.open(&tmp)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// Runs `edit` on the stored records under the store lock and writes them
/// back. Returns what `edit` returned. An unreadable store is an error and
/// nothing is written.
fn with_records<T>(
    app: &tauri::AppHandle,
    edit: impl FnOnce(&mut Vec<Record>) -> T,
) -> Result<T, ReportError> {
    let path = store_path(app)
        .ok_or_else(|| ReportError::new(ReportErrorKind::Invalid, "no app config directory"))?;
    let _guard = STORE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut records = load_records(&path).map_err(|_| {
        ReportError::new(ReportErrorKind::Invalid, "could not read followed reports")
    })?;
    let out = edit(&mut records);
    save_records(&path, &records).map_err(|_| {
        ReportError::new(ReportErrorKind::Invalid, "could not save followed reports")
    })?;
    Ok(out)
}

fn read_records(app: &tauri::AppHandle) -> Vec<Record> {
    let Some(path) = store_path(app) else {
        return Vec::new();
    };
    let _guard = STORE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    load_records(&path).unwrap_or_default()
}

fn views(records: &[Record]) -> Vec<FollowedReport> {
    records.iter().map(|r| r.report.clone()).collect()
}

fn secret_for(app: &tauri::AppHandle, id: &str) -> Result<String, ReportError> {
    if !report_client::is_report_id(id) {
        return Err(ReportError::new(
            ReportErrorKind::Invalid,
            "not a report id",
        ));
    }
    read_records(app)
        .into_iter()
        .find(|r| r.report.id == id)
        .map(|r| r.secret)
        .ok_or_else(|| ReportError::status(reqwest::StatusCode::NOT_FOUND, "not a followed report"))
}

fn changed(app: &tauri::AppHandle) {
    let _ = app.emit("follow-changed", ());
    refresh_menu_label(app);
}

// ---------------------------------------------------------------------------
// Menu label
// ---------------------------------------------------------------------------

/// The Help item's text for `count` reports with unread news.
fn menu_text(labels: &crate::menu_labels::MenuLabels, count: u32) -> String {
    menu_text_with(labels.get(LABEL_KEY), labels.get(LABEL_KEY_NEW), count)
}

/// Picks the plain or the "(n new)" label; `new` carries a literal `{n}`.
pub(crate) fn menu_text_with(plain: &str, new: &str, count: u32) -> String {
    if count == 0 {
        plain.to_owned()
    } else {
        new.replace("{n}", &count.to_string())
    }
}

fn unread_reports(records: &[Record]) -> u32 {
    records
        .iter()
        .filter(|r| r.report.status != Status::Gone && unread_count(&r.report) > 0)
        .count() as u32
}

/// Relabels Help ▸ Sent Reports… from the store: "(n new)" when any report
/// has unread news. Also called after a language switch, which resets every
/// native label to its plain form.
pub fn refresh_menu_label(app: &tauri::AppHandle) {
    let count = unread_reports(&read_records(app));
    let labels = crate::current_menu_labels(app);
    let text = menu_text(&labels, count);
    let Some(menu) = app.menu() else {
        return;
    };
    if let Some(tauri::menu::MenuItemKind::Submenu(help)) = menu.get("submenu:help") {
        if let Some(tauri::menu::MenuItemKind::MenuItem(item)) = help.get(MENU_ID) {
            let _ = item.set_text(text);
        }
    }
}

// ---------------------------------------------------------------------------
// Network
// ---------------------------------------------------------------------------

fn thread_url(id: &str) -> String {
    format!("{}{id}/thread", report_client::report_url())
}

/// §5.4 error mapping for thread routes. 403 stays a `Status` error with
/// `status: 403`; callers turn that into gone/done.
fn thread_error(status: reqwest::StatusCode, retry_after: Option<u64>, body: &str) -> ReportError {
    if status == reqwest::StatusCode::CONFLICT {
        let mut e = ReportError::new(ReportErrorKind::Full, "this conversation is full");
        e.status = Some(409);
        return e;
    }
    let mut e = ReportError::from_status(status, body);
    e.retry_after = retry_after;
    e
}

async fn thread_failure(response: reqwest::Response) -> ReportError {
    let status = response.status();
    let retry_after = response
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok());
    let body = response.text().await.unwrap_or_default();
    thread_error(status, retry_after, &body)
}

/// Only a 403 whose body was exactly `{"error":"forbidden"}` (which
/// `ReportError::from_status` labels `forbidden`) means the thread is gone;
/// any other 403 (a proxy, a captive portal) is a transient failure.
fn is_forbidden(e: &ReportError) -> bool {
    e.status == Some(403) && e.message == FORBIDDEN_MESSAGE
}

enum CheckOutcome {
    Updated(u32),
    Gone,
    Failed(ReportError),
}

/// Whether the service still needs this record's seen mark: it has not
/// acknowledged the local mark, and the thread still exists.
fn seen_resend_due(seen_synced: u64, seen_up_to: u64, gone: bool) -> bool {
    !gone && seen_synced < seen_up_to
}

/// Posts the seen mark for `id` up to `up_to` (best effort) and records the
/// service's acknowledgment. A 403 marks the report gone.
async fn send_seen(
    app: &tauri::AppHandle,
    client: &reqwest::Client,
    id: &str,
    secret: &str,
    up_to: u64,
) {
    let sent = client
        .post(format!("{}/seen", thread_url(id)))
        .header("Hew-Follow-Secret", secret)
        .timeout(THREAD_TIMEOUT)
        .json(&serde_json::json!({ "upTo": up_to }))
        .send()
        .await;
    if let Ok(response) = sent {
        if response.status().is_success() {
            let _ = with_records(app, |records| {
                if let Some(r) = records.iter_mut().find(|r| r.report.id == id) {
                    r.seen_synced = r.seen_synced.max(up_to);
                }
            });
        } else if is_forbidden(&thread_failure(response).await) {
            mark_record_gone(app, id);
            changed(app);
        }
    }
}

/// One GET for one record, merged into the store.
async fn check_one(
    app: &tauri::AppHandle,
    client: &reqwest::Client,
    id: &str,
    timeout: Duration,
) -> CheckOutcome {
    let Some(record) = read_records(app).into_iter().find(|r| r.report.id == id) else {
        return CheckOutcome::Failed(ReportError::new(ReportErrorKind::Invalid, "not followed"));
    };
    let url = format!("{}?after={}", thread_url(id), record.report.latest);
    let sent = client
        .get(&url)
        .header("Hew-Follow-Secret", &record.secret)
        .timeout(timeout)
        .send()
        .await;
    let response = match sent {
        Ok(r) => r,
        Err(e) => return CheckOutcome::Failed(ReportError::transport(&e)),
    };
    if response.status() == reqwest::StatusCode::OK {
        let view = match response.text().await {
            Ok(text) => serde_json::from_str::<ThreadView>(&text).ok(),
            Err(_) => None,
        };
        let Some(view) = view else {
            return CheckOutcome::Failed(ReportError::malformed());
        };
        let now = now_ms();
        let fresh = with_records(app, |records| {
            records.iter_mut().find(|r| r.report.id == id).map(|r| {
                let n = merge_thread(&mut r.report, view, now);
                let gone = r.report.status == Status::Gone;
                (
                    n,
                    seen_resend_due(r.seen_synced, r.report.seen_up_to, gone),
                    r.report.seen_up_to,
                )
            })
        });
        return match fresh {
            Ok(Some((n, resend, up_to))) => {
                // An earlier seen mark that never got through (a network
                // blip) is retried here, so the service learns the final
                // status was seen.
                if resend {
                    send_seen(app, client, id, &record.secret, up_to).await;
                }
                CheckOutcome::Updated(n)
            }
            Ok(None) => CheckOutcome::Failed(ReportError::new(
                ReportErrorKind::Invalid,
                "report was forgotten",
            )),
            Err(e) => CheckOutcome::Failed(e),
        };
    }
    let error = thread_failure(response).await;
    if is_forbidden(&error) {
        mark_record_gone(app, id);
        return CheckOutcome::Gone;
    }
    CheckOutcome::Failed(error)
}

fn mark_record_gone(app: &tauri::AppHandle, id: &str) {
    let now = now_ms();
    let _ = with_records(app, |records| {
        if let Some(r) = records.iter_mut().find(|r| r.report.id == id) {
            mark_gone(&mut r.report, now);
        }
    });
}

/// Checks `ids` one after another (no retry) and totals the outcome.
async fn check_many(
    app: &tauri::AppHandle,
    ids: Vec<String>,
    timeout: Duration,
) -> (CheckSummary, Option<ReportError>) {
    let client = report_client::build_client();
    let mut summary = CheckSummary::default();
    let mut last_error = None;
    for id in ids {
        summary.checked += 1;
        match check_one(app, &client, &id, timeout).await {
            CheckOutcome::Updated(fresh) => {
                if fresh > 0 {
                    summary.with_news += 1;
                    summary.new_count += fresh;
                }
            }
            CheckOutcome::Gone => {}
            CheckOutcome::Failed(e) => {
                summary.failed += 1;
                last_error = Some(e);
            }
        }
    }
    (summary, last_error)
}

// ---------------------------------------------------------------------------
// Submit hook
// ---------------------------------------------------------------------------

/// Stores the record for a just-committed report. Returns whether a record
/// was written.
pub fn record_submitted(
    app: &tauri::AppHandle,
    id: &str,
    secret: String,
    options: SubmitOptions,
) -> bool {
    let record = Record {
        secret,
        seen_synced: 0,
        report: FollowedReport {
            id: id.to_owned(),
            sent_at: now_ms(),
            app_version: env!("CARGO_PKG_VERSION").to_owned(),
            excerpt: truncate_excerpt(&options.excerpt),
            file_name: options.file_name,
            status: Status::Open,
            status_version: None,
            status_at: None,
            messages: Vec::new(),
            latest: 0,
            seen_up_to: 0,
            last_checked_at: None,
            state: State::Following,
        },
    };
    let ok = with_records(app, |records| {
        records.retain(|r| r.report.id != record.report.id);
        records.push(record);
    })
    .is_ok();
    if ok {
        changed(app);
    }
    ok
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn follow_list(app: tauri::AppHandle) -> Vec<FollowedReport> {
    views(&read_records(&app))
}

/// `reason`: `"launch"` applies the §6.3 policy; `"manual"` ignores the
/// 6-hour rule and the 365-day cap. With `id`, only that report is checked
/// (and a failure is returned as the error); without, every `following`
/// report is.
#[tauri::command]
pub async fn follow_check(
    app: tauri::AppHandle,
    id: Option<String>,
    reason: CheckReason,
) -> Result<CheckResult, ReportError> {
    let now = now_ms();
    let timeout = match reason {
        CheckReason::Launch => LAUNCH_TIMEOUT,
        CheckReason::Manual => THREAD_TIMEOUT,
    };
    let ids: Vec<String> = match &id {
        Some(id) => {
            secret_for(&app, id)?;
            vec![id.clone()]
        }
        None => read_records(&app)
            .into_iter()
            .filter(|r| match reason {
                CheckReason::Launch => due_for_launch(&r.report, now),
                CheckReason::Manual => r.report.state == State::Following,
            })
            .map(|r| r.report.id)
            .collect(),
    };
    let (summary, error) = check_many(&app, ids, timeout).await;
    changed(&app);
    if id.is_some() {
        if let Some(e) = error {
            return Err(e);
        }
    }
    Ok(CheckResult {
        reports: views(&read_records(&app)),
        summary,
    })
}

#[derive(Serialize)]
struct PostBody<'a> {
    text: &'a str,
}

#[tauri::command]
pub async fn follow_post(
    app: tauri::AppHandle,
    id: String,
    text: String,
) -> Result<FollowedReport, ReportError> {
    let secret = secret_for(&app, &id)?;
    if text.trim().is_empty() {
        return Err(ReportError::new(ReportErrorKind::Invalid, "empty message"));
    }
    let client = report_client::build_client();
    let response = client
        .post(thread_url(&id))
        .header("Hew-Follow-Secret", &secret)
        .timeout(THREAD_TIMEOUT)
        .json(&PostBody { text: &text })
        .send()
        .await
        .map_err(|e| ReportError::transport(&e))?;
    if response.status() != reqwest::StatusCode::CREATED {
        let error = thread_failure(response).await;
        if is_forbidden(&error) {
            mark_record_gone(&app, &id);
            changed(&app);
        }
        return Err(error);
    }
    // Pull the thread so our own message (and a reopen) appears; the post
    // itself succeeded, so a failed refresh is not an error.
    let _ = check_one(&app, &client, &id, THREAD_TIMEOUT).await;
    changed(&app);
    report_view(&app, &id)
}

fn report_view(app: &tauri::AppHandle, id: &str) -> Result<FollowedReport, ReportError> {
    read_records(app)
        .into_iter()
        .find(|r| r.report.id == id)
        .map(|r| r.report)
        .ok_or_else(|| ReportError::status(reqwest::StatusCode::NOT_FOUND, "not a followed report"))
}

#[tauri::command]
pub async fn follow_seen(
    app: tauri::AppHandle,
    id: String,
    up_to: u64,
) -> Result<FollowedReport, ReportError> {
    let secret = secret_for(&app, &id)?;
    let (effective, synced, gone) = with_records(&app, |records| {
        records.iter_mut().find(|r| r.report.id == id).map(|r| {
            mark_seen(&mut r.report, up_to);
            (
                r.report.seen_up_to,
                r.seen_synced,
                r.report.status == Status::Gone,
            )
        })
    })?
    .unwrap_or((up_to, up_to, true));
    changed(&app);
    // Tell the service best-effort (resending when an earlier mark never
    // got through); the local record is already updated.
    if seen_resend_due(synced, effective, gone) {
        let client = report_client::build_client();
        send_seen(&app, &client, &id, &secret, effective).await;
    }
    report_view(&app, &id)
}

/// A follow-up upload: the same raw gzip body as `report_submit` (the file
/// name travels inside the bundle), with the parent report id in the
/// `hew-follow-id` header. Emits the same `report-progress` events.
#[tauri::command]
pub async fn follow_attach(
    app: tauri::AppHandle,
    request: tauri::ipc::Request<'_>,
) -> Result<ReportSubmitResult, ReportError> {
    // The report id rides the `hew-follow-id` header; a raw body has no
    // named arguments.
    let id = request
        .headers()
        .get("hew-follow-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let bytes = report_client::raw_body(&request)?;
    let secret = secret_for(&app, &id)?;
    let headers = start_headers(false, Some((&id, &secret)));
    let uploaded = match report_client::upload_pieces(&app, &bytes, &headers).await {
        Ok(u) => u,
        Err(e) => {
            if is_forbidden(&e) {
                mark_record_gone(&app, &id);
                changed(&app);
            }
            return Err(e);
        }
    };
    // Show the attachment message right away; best-effort.
    let client = report_client::build_client();
    let _ = check_one(&app, &client, &id, THREAD_TIMEOUT).await;
    changed(&app);
    Ok(ReportSubmitResult {
        id: uploaded.id,
        following: false,
    })
}

#[tauri::command]
pub async fn follow_forget(app: tauri::AppHandle, id: String) -> Result<(), ReportError> {
    let secret = secret_for(&app, &id)?;
    // Best-effort revoke: a 403 (already gone) or a dead network still ends
    // with the local record removed.
    let client = report_client::build_client();
    let _ = client
        .delete(thread_url(&id))
        .header("Hew-Follow-Secret", &secret)
        .timeout(THREAD_TIMEOUT)
        .send()
        .await;
    with_records(&app, |records| records.retain(|r| r.report.id != id))?;
    changed(&app);
    Ok(())
}

// ---------------------------------------------------------------------------
// Launch check
// ---------------------------------------------------------------------------

static LAUNCH_STARTED: AtomicBool = AtomicBool::new(false);

/// Called from `setup` once the first window is up: 15 seconds later, run
/// the §6.3 policy once for this process. Silent on every failure.
pub fn spawn_launch_check(app: tauri::AppHandle) {
    if LAUNCH_STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    // Reflect any unread news from the last session immediately.
    refresh_menu_label(&app);
    std::thread::spawn(move || {
        std::thread::sleep(LAUNCH_DELAY);
        tauri::async_runtime::block_on(async move {
            let now = now_ms();
            let records = read_records(&app);
            let ids: Vec<String> = records
                .iter()
                .filter(|r| due_for_launch(&r.report, now))
                .map(|r| r.report.id.clone())
                .collect();
            // A done report is never fetched again, but its final seen mark
            // may still be unacknowledged (the blip happened on the last
            // view). The service's 30-day clock starts only from that mark
            // (design §5.5), so the launch check resends it without a fetch.
            let unsynced: Vec<(String, String, u64)> = records
                .iter()
                .filter(|r| r.report.state == State::Done)
                .filter(|r| {
                    seen_resend_due(
                        r.seen_synced,
                        r.report.seen_up_to,
                        r.report.status == Status::Gone,
                    )
                })
                .map(|r| (r.report.id.clone(), r.secret.clone(), r.report.seen_up_to))
                .collect();
            if ids.is_empty() && unsynced.is_empty() {
                return;
            }
            if !ids.is_empty() {
                let _ = check_many(&app, ids, LAUNCH_TIMEOUT).await;
            }
            if !unsynced.is_empty() {
                let client = report_client::build_client();
                for (id, secret, up_to) in unsynced {
                    send_seen(&app, &client, &id, &secret, up_to).await;
                }
            }
            changed(&app);
        });
    });
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR: i64 = 60 * 60 * 1000;
    const DAY: i64 = 24 * HOUR;
    const SECRET: &str = "sEcReT-sEcReT-sEcReT-sEcReT-sEcReT-abcde";

    fn report(id: &str) -> FollowedReport {
        FollowedReport {
            id: id.into(),
            sent_at: 1_000_000,
            app_version: "1.3.1".into(),
            excerpt: "crash on save".into(),
            file_name: None,
            status: Status::Open,
            status_version: None,
            status_at: None,
            messages: Vec::new(),
            latest: 0,
            seen_up_to: 0,
            last_checked_at: None,
            state: State::Following,
        }
    }

    fn msg(seq: u64, author: &str, kind: &str) -> Message {
        Message {
            seq,
            at: seq as i64,
            author: author.into(),
            kind: kind.into(),
            text: format!("m{seq}"),
            attachment_report_id: None,
        }
    }

    fn view(status: &str, latest: u64, messages: Vec<Message>) -> ThreadView {
        ThreadView {
            status: status.into(),
            status_version: None,
            status_at: Some(5),
            latest,
            messages,
        }
    }

    fn temp_path(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("hew-follow-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join(STORE_FILE)
    }

    #[test]
    fn store_round_trips_and_keeps_the_secret() {
        let path = temp_path("rt");
        let records = vec![Record {
            secret: SECRET.into(),
            seen_synced: 0,
            report: report("HEW-7K3F-Q9XB"),
        }];
        save_records(&path, &records).unwrap();
        let back = load_records(&path).unwrap();
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].secret, SECRET);
        assert_eq!(back[0].report, records[0].report);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn store_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let path = temp_path("perm");
        save_records(&path, &[]).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        // A rewrite keeps it that way.
        save_records(&path, &[]).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn write_is_atomic_no_temp_left_and_replaces_whole_file() {
        let path = temp_path("atomic");
        save_records(&path, &[]).unwrap();
        let one = vec![Record {
            secret: SECRET.into(),
            seen_synced: 0,
            report: report("HEW-7K3F-Q9XB"),
        }];
        save_records(&path, &one).unwrap();
        let names: Vec<String> = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec![STORE_FILE.to_string()]);
        assert_eq!(load_records(&path).unwrap().len(), 1);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn missing_store_reads_empty() {
        let path = temp_path("missing");
        assert!(load_records(&path).unwrap().is_empty());
    }

    #[test]
    fn corrupt_store_is_quarantined_and_reads_empty() {
        let path = temp_path("corrupt");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "{ not json").unwrap();
        assert!(load_records(&path).unwrap().is_empty());
        assert!(!path.exists());
        let prefix = format!("{STORE_FILE}.corrupt-");
        let quarantined: Vec<_> = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| {
                p.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with(&prefix)
            })
            .collect();
        assert_eq!(quarantined.len(), 1);
        assert_eq!(
            std::fs::read_to_string(&quarantined[0]).unwrap(),
            "{ not json"
        );
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn file_without_a_reports_array_is_quarantined() {
        let path = temp_path("noreports");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, r#"{"format":1}"#).unwrap();
        assert!(load_records(&path).unwrap().is_empty());
        assert!(!path.exists());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn one_bad_record_keeps_the_good_ones_and_the_file() {
        let path = temp_path("badrec");
        let records = vec![
            Record {
                secret: SECRET.into(),
                seen_synced: 0,
                report: report("HEW-AAAA-AAAA"),
            },
            Record {
                secret: SECRET.into(),
                seen_synced: 0,
                report: report("HEW-BBBB-BBBB"),
            },
            Record {
                secret: SECRET.into(),
                seen_synced: 0,
                report: report("HEW-CCCC-CCCC"),
            },
        ];
        save_records(&path, &records).unwrap();
        let mut value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        // A status this build does not know, and a record missing a field.
        value["reports"][0]["status"] = "from-the-future".into();
        value["reports"][2]
            .as_object_mut()
            .unwrap()
            .remove("sentAt");
        std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        let back = load_records(&path).unwrap();
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].report.id, "HEW-BBBB-BBBB");
        assert!(path.exists(), "not quarantined");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn seen_resend_is_due_only_for_an_unacknowledged_live_mark() {
        assert!(seen_resend_due(0, 3, false));
        assert!(seen_resend_due(2, 3, false));
        assert!(!seen_resend_due(3, 3, false));
        assert!(!seen_resend_due(0, 0, false));
        assert!(!seen_resend_due(0, 3, true));
    }

    #[test]
    fn stale_view_does_not_roll_the_status_back() {
        let mut r = report("HEW-7K3F-Q9XB");
        merge_thread(
            &mut r,
            view("fixed", 4, vec![msg(4, "system", "status")]),
            10,
        );
        assert_eq!(r.status, Status::Fixed);
        // A slower, older response lands afterward.
        merge_thread(
            &mut r,
            view("open", 2, vec![msg(2, "maintainer", "text")]),
            11,
        );
        assert_eq!(r.status, Status::Fixed);
        assert_eq!(r.latest, 4);
        assert_eq!(r.messages.len(), 2);
    }

    #[test]
    fn unreadable_store_is_an_error_not_an_empty_store() {
        // A directory where the file should be: read fails with something
        // other than NotFound, so a writer must not proceed.
        let path = temp_path("unreadable");
        std::fs::create_dir_all(&path).unwrap();
        assert!(load_records(&path).is_err());
        assert!(path.is_dir(), "left untouched");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn old_records_without_seen_synced_default_to_zero() {
        let path = temp_path("oldrec");
        let records = vec![Record {
            secret: SECRET.into(),
            seen_synced: 3,
            report: report("HEW-7K3F-Q9XB"),
        }];
        save_records(&path, &records).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"seenSynced\": 3"));
        std::fs::write(&path, text.replace("\"seenSynced\": 3,", "")).unwrap();
        assert_eq!(load_records(&path).unwrap()[0].seen_synced, 0);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn the_view_never_contains_the_secret() {
        let record = Record {
            secret: SECRET.into(),
            seen_synced: 0,
            report: report("HEW-7K3F-Q9XB"),
        };
        let view_json = serde_json::to_string(&record.report).unwrap();
        assert!(!view_json.contains(SECRET));
        assert!(!view_json.contains("secret"));
        let result = serde_json::to_string(&CheckResult {
            reports: vec![record.report.clone()],
            summary: CheckSummary::default(),
        })
        .unwrap();
        assert!(!result.contains(SECRET));
        assert!(!format!("{record:?}").contains(SECRET));
        // The stored form does carry it (that is its job).
        assert!(serde_json::to_string(&record).unwrap().contains(SECRET));
    }

    #[test]
    fn view_serializes_camel_case_with_kebab_enums() {
        let mut r = report("HEW-7K3F-Q9XB");
        r.status = Status::NeedInfo;
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["status"], "need-info");
        assert_eq!(v["state"], "following");
        assert!(v.get("sentAt").is_some() && v.get("seenUpTo").is_some());
        assert!(v.get("lastCheckedAt").is_some());
    }

    #[test]
    fn start_headers_for_new_report_follow_up_and_plain() {
        assert!(start_headers(false, None).is_empty());
        assert_eq!(
            start_headers(true, None),
            vec![("Hew-Follow", "1".to_string())]
        );
        let h = start_headers(true, Some(("HEW-7K3F-Q9XB", SECRET)));
        assert!(h.iter().all(|(n, _)| *n != "Hew-Follow"));
        assert!(h.contains(&("Hew-Follow-Up", "HEW-7K3F-Q9XB".to_string())));
        assert!(h.contains(&("Hew-Follow-Secret", SECRET.to_string())));
    }

    #[test]
    fn options_header_decodes_url_encoded_json_and_defaults_when_bad() {
        let json = r#"{"follow":true,"excerpt":"Größe + 100% \"x\"","fileName":"k.skp"}"#;
        let encoded: String = json
            .bytes()
            .map(|b| {
                if b.is_ascii_alphanumeric() {
                    (b as char).to_string()
                } else {
                    format!("%{b:02X}")
                }
            })
            .collect();
        let o = parse_options(&encoded).unwrap();
        assert!(o.follow);
        assert_eq!(o.excerpt, "Größe + 100% \"x\"");
        assert_eq!(o.file_name.as_deref(), Some("k.skp"));
        assert!(parse_options("%7B%7D").is_some_and(|o| !o.follow));
        assert!(parse_options("garbage").is_none());
    }

    #[test]
    fn options_header_tolerates_lone_surrogate_escapes() {
        fn enc(json: &str) -> String {
            json.bytes().map(|b| format!("%{b:02X}")).collect()
        }
        // A 200-unit slice can cut an emoji in half; JSON.stringify then
        // writes the orphan as an escape.
        let o = parse_options(&enc(
            r#"{"follow":true,"excerpt":"ab\ud83d","fileName":"k.skp"}"#,
        ))
        .unwrap();
        assert!(o.follow);
        assert_eq!(o.excerpt, "ab\u{fffd}");
        assert_eq!(o.file_name.as_deref(), Some("k.skp"));
        let o = parse_options(&enc(r#"{"excerpt":"\ude00x\ud83d\ude00 \\ud83d"}"#)).unwrap();
        assert_eq!(o.excerpt, "\u{fffd}x\u{1F600} \\ud83d");
    }

    #[test]
    fn launch_policy_time_arithmetic() {
        let now = 400 * DAY;
        let mut r = report("HEW-7K3F-Q9XB");
        r.sent_at = now - 10 * DAY;
        // Never checked: due.
        assert!(due_for_launch(&r, now));
        // Checked 5h59m ago: not due; exactly 6h: due.
        r.last_checked_at = Some(now - 6 * HOUR + 60_000);
        assert!(!due_for_launch(&r, now));
        r.last_checked_at = Some(now - 6 * HOUR);
        assert!(due_for_launch(&r, now));
        // Clock set back: a future check time is due.
        r.last_checked_at = Some(now + DAY);
        assert!(due_for_launch(&r, now));
        // 365 days exactly is still polled; one ms past is not.
        r.last_checked_at = None;
        r.sent_at = now - 365 * DAY;
        assert!(due_for_launch(&r, now));
        r.sent_at = now - 365 * DAY - 1;
        assert!(!due_for_launch(&r, now));
        // Done records are never polled.
        r.sent_at = now - DAY;
        r.state = State::Done;
        assert!(!due_for_launch(&r, now));
    }

    #[test]
    fn merge_adds_messages_once_and_counts_unread_news() {
        let mut r = report("HEW-7K3F-Q9XB");
        let fresh = merge_thread(
            &mut r,
            view(
                "need-info",
                2,
                vec![msg(1, "reporter", "text"), msg(2, "maintainer", "text")],
            ),
            777,
        );
        assert_eq!(fresh, 1);
        assert_eq!(r.status, Status::NeedInfo);
        assert_eq!(r.latest, 2);
        assert_eq!(r.last_checked_at, Some(777));
        assert_eq!(unread_count(&r), 1);
        // Same messages again: nothing new.
        let again = merge_thread(
            &mut r,
            view("need-info", 2, vec![msg(2, "maintainer", "text")]),
            800,
        );
        assert_eq!(again, 0);
        assert_eq!(r.messages.len(), 2);
        // Out-of-order arrival is sorted.
        merge_thread(
            &mut r,
            view(
                "need-info",
                4,
                vec![msg(4, "system", "status"), msg(3, "reporter", "attachment")],
            ),
            900,
        );
        let seqs: Vec<u64> = r.messages.iter().map(|m| m.seq).collect();
        assert_eq!(seqs, vec![1, 2, 3, 4]);
        // An unknown status string leaves the status alone.
        merge_thread(&mut r, view("weird", 4, vec![]), 901);
        assert_eq!(r.status, Status::NeedInfo);
    }

    #[test]
    fn seen_moves_to_done_only_after_the_final_status_is_viewed() {
        let mut r = report("HEW-7K3F-Q9XB");
        merge_thread(
            &mut r,
            view(
                "fixed",
                3,
                vec![
                    msg(1, "maintainer", "text"),
                    msg(2, "maintainer", "text"),
                    msg(3, "system", "status"),
                ],
            ),
            1,
        );
        assert_eq!(unread_count(&r), 3);
        mark_seen(&mut r, 2);
        assert_eq!(r.state, State::Following);
        assert_eq!(unread_count(&r), 1);
        mark_seen(&mut r, 3);
        assert_eq!(r.state, State::Done);
        assert_eq!(unread_count(&r), 0);
        // Beyond latest clamps; seen never moves backward.
        mark_seen(&mut r, 99);
        assert_eq!(r.seen_up_to, 3);
        mark_seen(&mut r, 1);
        assert_eq!(r.seen_up_to, 3);
    }

    #[test]
    fn seen_on_an_open_thread_never_finishes_it() {
        let mut r = report("HEW-7K3F-Q9XB");
        merge_thread(
            &mut r,
            view("open", 1, vec![msg(1, "maintainer", "text")]),
            1,
        );
        mark_seen(&mut r, 1);
        assert_eq!(r.state, State::Following);
    }

    #[test]
    fn a_reply_after_done_reopens_following() {
        let mut r = report("HEW-7K3F-Q9XB");
        merge_thread(
            &mut r,
            view("closed", 1, vec![msg(1, "system", "status")]),
            1,
        );
        mark_seen(&mut r, 1);
        assert_eq!(r.state, State::Done);
        merge_thread(&mut r, view("open", 2, vec![msg(2, "system", "status")]), 2);
        assert_eq!(r.state, State::Following);
    }

    #[test]
    fn forbidden_means_gone_and_done() {
        let mut r = report("HEW-7K3F-Q9XB");
        mark_gone(&mut r, 42);
        assert_eq!(r.status, Status::Gone);
        assert_eq!(r.state, State::Done);
        assert!(!due_for_launch(&r, 43));
        assert!(is_forbidden(&thread_error(
            reqwest::StatusCode::FORBIDDEN,
            None,
            r#"{"error":"forbidden"}"#
        )));
        // A plain 403 (proxy page, other body) is transient, not gone.
        assert!(!is_forbidden(&thread_error(
            reqwest::StatusCode::FORBIDDEN,
            None,
            "<html>blocked</html>"
        )));
        assert!(!is_forbidden(&thread_error(
            reqwest::StatusCode::FORBIDDEN,
            None,
            r#"{"error":"other"}"#
        )));
    }

    #[test]
    fn thread_errors_map_per_the_contract() {
        let e = thread_error(
            reqwest::StatusCode::TOO_MANY_REQUESTS,
            Some(30),
            r#"{"error":"rate-limited"}"#,
        );
        assert!(matches!(e.kind, ReportErrorKind::RateLimited));
        assert_eq!(e.retry_after, Some(30));
        let e = thread_error(reqwest::StatusCode::CONFLICT, None, r#"{"error":"full"}"#);
        assert!(matches!(e.kind, ReportErrorKind::Full));
        assert_eq!(e.status, Some(409));
        let e = thread_error(
            reqwest::StatusCode::BAD_REQUEST,
            None,
            r#"{"error":"invalid"}"#,
        );
        assert!(matches!(e.kind, ReportErrorKind::Invalid));
    }

    #[test]
    fn menu_text_switches_on_unread() {
        let labels = crate::menu_labels::MenuLabels::for_locale("en");
        assert_eq!(menu_text(&labels, 0), "Sent Reports…");
        assert_eq!(menu_text(&labels, 2), "Sent Reports (2 new)…");
    }

    #[test]
    fn unread_reports_counts_reports_not_messages_and_skips_gone() {
        let mut a = report("HEW-AAAA-AAAA");
        merge_thread(
            &mut a,
            view(
                "open",
                2,
                vec![msg(1, "maintainer", "text"), msg(2, "maintainer", "text")],
            ),
            1,
        );
        let b = report("HEW-BBBB-BBBB");
        let mut c = report("HEW-CCCC-CCCC");
        merge_thread(
            &mut c,
            view("open", 1, vec![msg(1, "maintainer", "text")]),
            1,
        );
        mark_gone(&mut c, 2);
        let recs: Vec<Record> = [a, b, c]
            .into_iter()
            .map(|report| Record {
                seen_synced: 0,
                secret: SECRET.into(),
                report,
            })
            .collect();
        assert_eq!(unread_reports(&recs), 1);
    }

    #[test]
    fn excerpt_is_trimmed_and_capped() {
        assert_eq!(truncate_excerpt("  hi  "), "hi");
        assert_eq!(
            truncate_excerpt(&"é".repeat(500)).chars().count(),
            EXCERPT_MAX_CHARS
        );
    }
}

//! In-app updater (compiled in via the `updater` feature, see Cargo.toml and
//! build.rs; absent from preview builds).
//!
//! Talks to the signed `latest.json` published on GitHub Releases: a silent
//! check on launch, plus the manual "Check for Updates" menu item. The check,
//! download, and install live shell-side; the webview's update window
//! (`index.html#update`) is the only UI for an available update and reaches
//! the shell through the commands below.
//!
//! What a check does with a found update depends on how this copy was
//! installed (install_channel.rs): the macOS app, the Windows installer and
//! the AppImage replace themselves; a native .deb or .rpm installs the
//! package after the system asks for a password; a Flatpak, pacman, Gentoo or
//! other package-manager copy only shows the version and the command that
//! updates it (tell-only). Every branch reads the `UpdatePlan` and nothing
//! else, so no code here inspects the platform. "You're up to date" and
//! "Could not check for updates" stay native message boxes.
//!
//! # IPC contract (the React side codes against this)
//!
//! The window: label `update`, URL `index.html#update`, opened by a check
//! that finds an update, never by the webview. At most one; a second check
//! brings it forward (never switching an open window to the What's New
//! form). It is not a document window. The person closing it (button or
//! Escape) while the offer is genuinely open counts as "Remind Me Later";
//! closing a failure, the What's New form, or the shell closing it (the last
//! document went away, Hew quit) does not (`remind_on_close`).
//!
//! Commands (all return `Result<_, String>`; the string is a plain message).
//! Each is checked against the calling window's label at runtime, because
//! custom commands are not scoped by capability files; the capability file
//! `capabilities/update.json` grants the update window its plugin
//! permissions only.
//!
//! | command | callable from | argument | returns |
//! |---|---|---|---|
//! | `update_info` | `update` | none | `UpdateInfo` |
//! | `update_install` | `update` | none | `InstallResult`, also emitted |
//! | `update_skip` | `update` | none | `()`; window closes |
//! | `update_remind` | `update` | none | `()`; window closes |
//! | `update_restart` | `update`, document windows | none | `RestartOutcome`; on success it never returns (the app restarts) |
//! | `close_cancelled` | document windows | none | `()`: the close guard's Cancel (see "Restart") |
//! | `update_copy_command` | `update` | none | `()` (tell-only: copies `plan.command`) |
//! | `update_show_notes` | document windows | none | `()`: opens the window read-only |
//! | `update_ready_get` | document windows | none | `UpdateReady` or `null` |
//! | `update_auto_get` | `update`, `settings`, document windows | none | `AutoInstall` |
//! | `update_auto_set` | same | `{ enabled: bool }` | `AutoInstall` |
//! | `update_open_fixture` | any (debug builds only) | `{ mode?, notes?, version?, phase?, readOnly?, failRestart?, noticeBar? }` | `()` |
//!
//! `UpdateInfo` (camelCase JSON): `{ version, currentVersion, notes|null,
//! date|null ("YYYY-MM-DD"), plan: { mode: "selfInstall" |
//! "selfInstallElevated" | "notifyOnly", channel, command|null, helpUrl,
//! closesApp }, phase: "offer" | "downloading" | "ready" | "installed",
//! readOnly, autoInstall, autoSupported, changelogUrl, downloadUrl }`.
//! `notes` is Markdown (`Update.body`), null when empty. The window shows the
//! checkbox only when `autoSupported`. Tell-only: `mode == "notifyOnly"`
//! (Update Help button, `command` with Copy, no install). Elevated:
//! `mode == "selfInstallElevated"` (Install Update plus the password line, no
//! checkbox). `readOnly`: notes and Close only. If `phase` is "installed" the
//! window opens on the restart step; "downloading" on the progress bar.
//!
//! Events (Tauri events, JSON payloads, camelCase):
//!  - `update-progress` to the `update` window: `{ phase: "downloading",
//!    version, downloaded: number, total: number|null }` while downloading
//!    (throttled), then once `{ phase: "installing", version, closesApp }`.
//!    Where the installer closes Hew (Windows) the "installing" event comes
//!    from `update_restart`, after every document has closed, and the
//!    process exits right after; show "Hew will close to finish installing".
//!  - `update-result` to the `update` window (also the return value of
//!    `update_install`): `{ status: "installed" | "ready" | "cancelled" |
//!    "failed", version, error: string|null, closesApp }`. "installed" means
//!    offer the restart step; "ready" means downloaded and verified but not
//!    installed, because the installer would close Hew (Windows): offer
//!    "Install and Restart", which is `update_restart`; "cancelled" means the
//!    person dismissed the system password prompt (back to the offer, no
//!    error); "failed" carries the error text, and the window shows Open
//!    Download Page (`downloadUrl`). `update_restart` also emits "failed" when
//!    the Windows installer could not start after every document closed.
//!  - `update-changed` to the `update` window, no payload: the pending
//!    update changed (a newer check, the window brought forward, or an
//!    automatic download finished or failed); call `update_info` again. The
//!    window keeps what only it knows while the phase has not moved on (a
//!    restart waiting on the documents, download progress). An automatic
//!    download also sends `update-progress`, so a window opened during one
//!    shows its progress.
//!  - `update-ready` to every document window: an automatic launch install
//!    finished. Payload `UpdateReady` `{ version, currentVersion, installed,
//!    closesApp }`. `installed: true` (Mac, AppImage): the bar says it will
//!    open next time; `installed: false` (Windows): "ready to install", and
//!    nothing is installed until the person clicks "Install and Restart"
//!    (`update_restart`). Windows created later call `update_ready_get`.
//!
//!
//! # Restart
//!
//! Automatic mode never restarts by itself, and nothing here ends the
//! process except `update_restart`: `update_install` never runs an installer
//! that closes Hew (it stops at "ready" instead).
//!
//! `update_restart` first closes every document window, one at a time,
//! through the same close guard a click on a window's close button runs
//! (App.tsx): the shell focuses the window and asks it to close, the guard
//! shows the unsaved-changes dialog if the document has changes, and the
//! shell waits, with no timeout, for that window to be destroyed or to
//! answer Cancel (`close_cancelled`). Windows with unsaved changes go first;
//! the list is read again after each close, so a window opened meanwhile is
//! included. Cancel in any window stops the pass, including a prompt the
//! person opened meanwhile by closing another window by hand: windows
//! already closed stay closed (as with File ▸ Exit), the update stays
//! installed or downloaded, and the command returns `{ status: "cancelled"
//! }`. A window whose prompt is already open when the pass reaches it keeps
//! that prompt (the guard ignores a second close request), and the
//! person's answer to it counts. A second
//! call while a pass runs brings the awaited window forward and returns
//! `{ status: "inProgress" }`. Once the last document has closed it restarts
//! (installed in place) or runs the Windows installer (downloaded). While a
//! pass runs, the last window closing does not end the app (main.rs's
//! `ExitRequested` arm). If the Windows installer cannot start, a new empty
//! document window and the update window open with the failure message.

use crate::install_channel::{self, UpdateMode, UpdatePlan};
use crate::update_state::{
    self as state, AutoInstall, LaunchAction, Offer, Phase, UpdateInfo, UpdateReady,
};
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Mutex};
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
use tauri_plugin_updater::UpdaterExt;

const WINDOW: &str = "update";

/// Single-flight guard (managed state). The silent launch check and the
/// manual "Check for Updates" run independently; without this, an update
/// that appears while the user also clicks the menu item would stack two
/// windows and, on accept, start two concurrent installs writing the same
/// target (the macOS bundle swap is not atomic). One flag serializes them.
#[derive(Default)]
pub struct UpdateGuard(pub AtomicBool);

/// The update a check found, held until it is installed or dismissed.
#[derive(Default)]
pub struct Pending(Mutex<Option<PendingUpdate>>);

struct PendingUpdate {
    offer: Offer,
    plan: UpdatePlan,
    /// None for a debug fixture, which has nothing to download.
    update: Option<tauri_plugin_updater::Update>,
    phase: Phase,
    /// Downloaded package waiting for a click (Windows automatic mode).
    bytes: Option<Vec<u8>>,
    read_only: bool,
    fixture: bool,
    /// Debug fixture only: a "ready" restart takes the installer-failed path
    /// instead of exiting, so that path can be driven on any system.
    fixture_fail_restart: bool,
    /// Why the last install or restart failed (the window opens on the
    /// failure step); cleared when an install starts.
    last_error: Option<String>,
    /// The shell itself is closing the update window (the last document
    /// went away, or Hew is quitting): that close is not "Remind Me Later".
    quiet_close: bool,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct InstallResult {
    status: &'static str,
    version: String,
    error: Option<String>,
    closes_app: bool,
}

/// `update_restart`'s answer when it returns at all: `"cancelled"` (someone
/// pressed Cancel in a save prompt; nothing changed) or `"inProgress"` (a
/// restart is already closing the documents).
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RestartOutcome {
    status: &'static str,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn config_dir(app: &AppHandle) -> Option<std::path::PathBuf> {
    app.path().app_config_dir().ok()
}

fn load_state(app: &AppHandle) -> state::UpdateState {
    config_dir(app)
        .map(|dir| state::load(&dir))
        .unwrap_or_default()
}

fn save_state(app: &AppHandle, update: impl FnOnce(&mut state::UpdateState)) {
    let Some(dir) = config_dir(app) else {
        return;
    };
    let mut current = state::load(&dir);
    update(&mut current);
    // Best effort: a failed write only means the choice is forgotten.
    let _ = state::save(&dir, &current);
}

fn require_window(window: &tauri::WebviewWindow, allow_documents: bool) -> Result<(), String> {
    let label = window.label();
    if label == WINDOW || (allow_documents && crate::is_document_window_label(label)) {
        Ok(())
    } else {
        Err("not available from this window".into())
    }
}

/// The restart was put off after an install; another check offers it again
/// instead of re-offering the same update.
static INSTALLED_PENDING_RESTART: AtomicBool = AtomicBool::new(false);

/// Run an update check against the configured endpoint, at most one at a
/// time. A second call while one is in flight backs off: the launch check
/// silently, a manual click with a note.
///
/// `interactive` splits the two entry points:
///  - the manual menu item (`true`): always give feedback (the window, "up
///    to date", or the error), ignoring Skip and Remind.
///  - the launch check (`false`): stay quiet unless `launch_action` says
///    otherwise; no popup when current, no error dialog when offline.
pub async fn run_check(app: AppHandle, interactive: bool) {
    if app.state::<UpdateGuard>().0.swap(true, Ordering::SeqCst) {
        if interactive {
            let labels = crate::current_menu_labels(&app);
            info_dialog(
                &app,
                labels.get("updater:in-progress-title"),
                labels.get("updater:in-progress-body"),
            );
        }
        return;
    }
    check_and_install(app.clone(), interactive).await;
    app.state::<UpdateGuard>().0.store(false, Ordering::SeqCst);
}

/// The check, then the window (or the automatic install); `run_check`
/// serializes calls to it.
async fn check_and_install(app: AppHandle, interactive: bool) {
    let labels = crate::current_menu_labels(&app);
    // The retired `last-update-notice` file becomes a skipped version once.
    let saved = config_dir(&app)
        .map(|dir| state::load_with_handover(&dir))
        .unwrap_or_default();
    // Resolved before anything can replace the binary on disk; see
    // `install_channel::current_channel`.
    let plan = install_channel::current_plan();
    if INSTALLED_PENDING_RESTART.load(Ordering::SeqCst) {
        if interactive {
            let _ = show_window(&app, false).await;
        }
        return;
    }
    let updater = match app.updater() {
        Ok(updater) => updater,
        Err(err) => {
            if interactive {
                error_dialog(
                    &app,
                    &labels
                        .get("updater:start-failed-body")
                        .replace("{error}", &err.to_string()),
                );
            }
            return;
        }
    };
    match updater.check().await {
        Ok(Some(update)) => {
            if !interactive {
                match state::launch_action(&saved, &update.version, now_ms(), &plan) {
                    LaunchAction::Quiet => return,
                    LaunchAction::AutoInstall => {
                        auto_install(app, plan, update).await;
                        return;
                    }
                    LaunchAction::ShowWindow => {}
                }
            }
            offer(&app, plan, update).await;
        }
        Ok(None) => {
            if interactive {
                info_dialog(
                    &app,
                    labels.get("updater:up-to-date-title"),
                    labels.get("updater:up-to-date-body"),
                );
            }
        }
        Err(err) => {
            if interactive {
                error_dialog(
                    &app,
                    &labels
                        .get("updater:check-failed-body")
                        .replace("{error}", &err.to_string()),
                );
            }
        }
    }
}

fn offer_from(update: &tauri_plugin_updater::Update) -> Offer {
    Offer {
        version: update.version.clone(),
        current_version: update.current_version.clone(),
        notes: update.body.clone(),
        date: update.date.map(|d| d.date().to_string()),
    }
}

/// Hold the found update and show the window. Work already under way
/// (downloading, downloaded, installed) is never replaced by a re-check;
/// the window just comes forward.
async fn offer(app: &AppHandle, plan: UpdatePlan, update: tauri_plugin_updater::Update) {
    {
        let pending = app.state::<Pending>();
        let mut slot = pending.0.lock().unwrap_or_else(|e| e.into_inner());
        match slot.as_mut() {
            Some(held) if held.phase != Phase::Offer => {}
            _ => {
                *slot = Some(PendingUpdate {
                    offer: offer_from(&update),
                    plan,
                    update: Some(update),
                    phase: Phase::Offer,
                    bytes: None,
                    read_only: false,
                    fixture: false,
                    fixture_fail_restart: false,
                    last_error: None,
                    quiet_close: false,
                });
            }
        }
    }
    let _ = show_window(app, false).await;
}

/// Open the update window, or bring the existing one forward. `read_only`
/// is the "What's New" form (notes and Close).
async fn show_window(app: &AppHandle, read_only: bool) -> Result<(), String> {
    let existing = app.get_webview_window(WINDOW);
    {
        let pending = app.state::<Pending>();
        let mut slot = pending.0.lock().unwrap_or_else(|e| e.into_inner());
        let held = slot.as_mut().ok_or("no update is pending")?;
        held.read_only = form_after_show(existing.is_some(), held.read_only, read_only);
        if existing.is_none() {
            held.quiet_close = false;
        }
    }
    if let Some(existing) = existing {
        let _ = existing.emit("update-changed", ());
        let _ = existing.unminimize();
        return existing.set_focus().map_err(|e| e.to_string());
    }
    let title = crate::current_menu_labels(app)
        .get("updater:window-title")
        .to_string();
    tauri::WebviewWindowBuilder::new(
        app,
        WINDOW,
        tauri::WebviewUrl::App("index.html#update".into()),
    )
    .title(title)
    .inner_size(560.0, 560.0)
    .min_inner_size(440.0, 420.0)
    .resizable(true)
    .minimizable(false)
    .maximizable(false)
    .build()
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Whether the window shows the read-only What's New form after a
/// `show_window` that asked for `requested`. A new window takes the form
/// asked for. An open window is never turned into What's New: that would
/// take its Restart button (or a restart under way) out from under the
/// person; it only comes forward. An open What's New window does become the
/// full window when a check asks for it.
fn form_after_show(window_open: bool, current: bool, requested: bool) -> bool {
    if window_open {
        current && requested
    } else {
        requested
    }
}

/// Download (and where possible install) without asking, then tell the
/// document windows. Never restarts. On any failure it stays quiet and the
/// next launch tries again.
async fn auto_install(app: AppHandle, plan: UpdatePlan, update: tauri_plugin_updater::Update) {
    let version = update.version.clone();
    let current_version = update.current_version.clone();
    let closes_app = state::plan_info(&plan).closes_app;
    {
        let pending = app.state::<Pending>();
        let mut slot = pending.0.lock().unwrap_or_else(|e| e.into_inner());
        if slot.as_ref().is_some_and(|held| held.phase != Phase::Offer) {
            return;
        }
        *slot = Some(PendingUpdate {
            offer: offer_from(&update),
            plan,
            update: Some(update.clone()),
            phase: Phase::Downloading,
            bytes: None,
            read_only: false,
            fixture: false,
            fixture_fail_restart: false,
            last_error: None,
            quiet_close: false,
        });
    }
    // Progress goes to the update window too: Check for Updates or What's
    // New can open it mid-download, on the progress bar.
    let outcome = if closes_app {
        // The Windows installer closes Hew, so only download; the install
        // waits for a click.
        update
            .download(progress_reporter(&app, &version), || {})
            .await
            .map(Some)
    } else {
        let (app_installing, version_installing) = (app.clone(), version.clone());
        update
            .download_and_install(progress_reporter(&app, &version), move || {
                emit_installing(&app_installing, &version_installing)
            })
            .await
            .map(|()| None)
    };
    let ready = {
        let pending = app.state::<Pending>();
        let mut slot = pending.0.lock().unwrap_or_else(|e| e.into_inner());
        let Some(held) = slot.as_mut() else {
            return;
        };
        match outcome {
            Ok(bytes) => {
                let installed = bytes.is_none();
                held.phase = if installed {
                    Phase::Installed
                } else {
                    Phase::Ready
                };
                held.bytes = bytes;
                if installed {
                    INSTALLED_PENDING_RESTART.store(true, Ordering::SeqCst);
                }
                Some(UpdateReady {
                    version,
                    current_version,
                    installed,
                    closes_app,
                })
            }
            Err(err) => {
                // Shown only by an update window already open; a later
                // check replaces this offer with a fresh one.
                held.phase = Phase::Offer;
                held.last_error = Some(err.to_string());
                None
            }
        }
    };
    // An update window opened mid-download sits on the progress bar until
    // told the download is over.
    let _ = app.emit_to(WINDOW, "update-changed", ());
    if let Some(ready) = ready {
        emit_ready(&app, &ready);
    }
}

/// The throttled `update-progress` reporter for a download (a chunk is a
/// few KB).
fn progress_reporter(app: &AppHandle, version: &str) -> impl FnMut(usize, Option<u64>) {
    let app = app.clone();
    let version = version.to_string();
    let mut downloaded = 0u64;
    let mut last_emit = Instant::now();
    move |chunk, total| {
        downloaded += chunk as u64;
        if last_emit.elapsed().as_millis() >= 100 {
            last_emit = Instant::now();
            let _ = app.emit_to(
                WINDOW,
                "update-progress",
                serde_json::json!({
                    "phase": "downloading",
                    "version": version,
                    "downloaded": downloaded,
                    "total": total,
                }),
            );
        }
    }
}

/// The download is over and the install in place starts.
fn emit_installing(app: &AppHandle, version: &str) {
    let _ = app.emit_to(
        WINDOW,
        "update-progress",
        serde_json::json!({ "phase": "installing", "version": version, "closesApp": false }),
    );
}

fn emit_ready(app: &AppHandle, ready: &UpdateReady) {
    for (label, _) in app.webview_windows() {
        if crate::is_document_window_label(&label) {
            let _ = app.emit_to(label.as_str(), "update-ready", ready);
        }
    }
}

/// `update_ready_get`'s value for the current pending update.
fn ready_of(held: &PendingUpdate) -> Option<UpdateReady> {
    if held.fixture || !matches!(held.phase, Phase::Ready | Phase::Installed) {
        return None;
    }
    Some(UpdateReady {
        version: held.offer.version.clone(),
        current_version: held.offer.current_version.clone(),
        installed: held.phase == Phase::Installed,
        closes_app: state::plan_info(&held.plan).closes_app,
    })
}

/// Whether the update window closing now means "Remind Me Later". The rule
/// (docs/design D3: the close button or Escape on the offer): only the
/// person closing a genuinely open offer. So not the What's New form, not a
/// failed install or restart (its own Remind Me Later button is the way to
/// ask for one), not while work is under way or done (the phase is not
/// `Offer`), not a debug fixture, and not a close the shell made itself
/// (`quiet_close`). The buttons that settle the offer (Skip, Remind) clear
/// the slot before they close the window, so their close records nothing
/// further here.
fn remind_on_close(held: &PendingUpdate) -> bool {
    held.phase == Phase::Offer
        && !held.read_only
        && held.last_error.is_none()
        && !held.fixture
        && !held.quiet_close
}

/// The update window closed: record "Remind Me Later" when
/// `remind_on_close` says so. Called from the window-event hook.
pub fn on_window_closed(app: &AppHandle) {
    let pending = app.state::<Pending>();
    let mut slot = pending.0.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(held) = slot.as_mut() {
        let remind = remind_on_close(held);
        held.read_only = false;
        held.quiet_close = false;
        if remind {
            let until = now_ms() + state::REMIND_AFTER_MS;
            drop(slot);
            save_state(app, |s| s.remind_after = Some(until));
        }
    }
}

/// The next close of the update window is the shell's, not the person's:
/// it records no reminder. For Hew quitting with the window open.
pub fn mark_quiet_close(app: &AppHandle) {
    if let Some(pending) = app.try_state::<Pending>() {
        if let Some(held) = pending.0.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            held.quiet_close = true;
        }
    }
}

/// Close the update window for the shell's own reasons (the last document
/// window went away). Not "Remind Me Later": the person did not choose it.
pub fn close_window_quietly(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(WINDOW) {
        mark_quiet_close(app);
        let _ = window.close();
    }
}

// --- Commands --------------------------------------------------------------

#[tauri::command]
pub fn update_info(
    app: AppHandle,
    window: tauri::WebviewWindow,
    pending: tauri::State<'_, Pending>,
) -> Result<UpdateInfo, String> {
    require_window(&window, false)?;
    let saved = load_state(&app);
    let slot = pending.0.lock().unwrap_or_else(|e| e.into_inner());
    let held = slot.as_ref().ok_or("no update is pending")?;
    let mut info = state::build_info(&held.offer, &held.plan, held.phase, held.read_only, &saved);
    info.error = held.last_error.clone();
    Ok(info)
}

/// Download with progress, then install the way the plan says. A `.deb` or
/// `.rpm` copy downloads (and signature-checks) through the plugin but
/// installs with our own single `pkexec` call (see
/// `install_channel::install_package_elevated`); every other copy uses the
/// plugin's installer. Where that installer would close Hew (Windows) this
/// only downloads: the package is held, the phase becomes `Ready`, and the
/// install waits for `update_restart`, which closes the documents first. So
/// this command never ends the process.
#[tauri::command]
pub async fn update_install(
    app: AppHandle,
    window: tauri::WebviewWindow,
) -> Result<InstallResult, String> {
    require_window(&window, false)?;
    if app.state::<RestartPass>().is_active() {
        return Err("Hew is already closing to restart".into());
    }
    let (update, plan, version, bytes, fixture) = {
        let pending = app.state::<Pending>();
        let mut slot = pending.0.lock().unwrap_or_else(|e| e.into_inner());
        let held = slot.as_mut().ok_or("no update is pending")?;
        if held.plan.mode == UpdateMode::NotifyOnly {
            return Err("this copy of Hew is updated outside the app".into());
        }
        if held.phase == Phase::Downloading || held.phase == Phase::Installed {
            return Err("the update is already being installed".into());
        }
        held.phase = Phase::Downloading;
        held.last_error = None;
        (
            held.update.clone(),
            held.plan.clone(),
            held.offer.version.clone(),
            held.bytes.take(),
            held.fixture,
        )
    };
    let closes_app = state::plan_info(&plan).closes_app;
    let outcome = if fixture {
        fixture_install(&app, &version).await;
        Ok(None)
    } else {
        match update {
            Some(update) => run_install(&app, &plan, &update, bytes, &version).await,
            None => Err(Some("no update is pending".to_string())),
        }
    };
    let (status, error) = {
        let pending = app.state::<Pending>();
        let mut slot = pending.0.lock().unwrap_or_else(|e| e.into_inner());
        settle_install(slot.as_mut(), closes_app, outcome)
    };
    if status == "installed" && !fixture {
        INSTALLED_PENDING_RESTART.store(true, Ordering::SeqCst);
    }
    let result = InstallResult {
        status,
        version,
        error,
        closes_app,
    };
    let _ = app.emit_to(WINDOW, "update-result", &result);
    Ok(result)
}

/// Debug fixture: fake download progress so the UI can be driven.
async fn fixture_install(app: &AppHandle, version: &str) {
    let total = 40_000_000u64;
    for step in 1..=10u64 {
        sleep_ms(150).await;
        let _ = app.emit_to(
            WINDOW,
            "update-progress",
            serde_json::json!({
                "phase": "downloading", "version": version,
                "downloaded": total * step / 10, "total": total,
            }),
        );
    }
}

async fn sleep_ms(ms: u64) {
    let _ = tauri::async_runtime::spawn_blocking(move || {
        std::thread::sleep(std::time::Duration::from_millis(ms))
    })
    .await;
}

/// What `update_install` does with the result of a download or install:
/// the phase it leaves, the package it keeps, and the status it reports.
/// `outcome` is `Ok(Some(bytes))` for a download that waits for the restart
/// (a plan whose installer closes Hew), `Ok(None)` for an install in place,
/// and `Err(None)` for a dismissed password prompt.
fn settle_install(
    held: Option<&mut PendingUpdate>,
    closes_app: bool,
    outcome: Result<Option<Vec<u8>>, Option<String>>,
) -> (&'static str, Option<String>) {
    let (phase, bytes, status, error) = match outcome {
        Ok(bytes) if closes_app => (Phase::Ready, bytes, "ready", None),
        Ok(_) => (Phase::Installed, None, "installed", None),
        // `None` is the person dismissing the password prompt: not a failure.
        Err(None) => (Phase::Offer, None, "cancelled", None),
        Err(Some(message)) => (Phase::Offer, None, "failed", Some(message)),
    };
    if let Some(held) = held {
        held.phase = phase;
        held.bytes = bytes;
        // The window shows the failure; closing it is not a reminder
        // (`remind_on_close`), and a reload (`update-changed`) keeps it.
        held.last_error = error.clone();
    }
    (status, error)
}

/// Download (or use the package already held), then install unless the
/// plan's installer closes Hew: then return the package for
/// `update_restart`. `Err(None)` means the person backed out at the
/// system's password prompt.
async fn run_install(
    app: &AppHandle,
    plan: &UpdatePlan,
    update: &tauri_plugin_updater::Update,
    held_bytes: Option<Vec<u8>>,
    version: &str,
) -> Result<Option<Vec<u8>>, Option<String>> {
    let bytes = match held_bytes {
        Some(bytes) => bytes,
        None => update
            .download(progress_reporter(app, version), || {})
            .await
            .map_err(|err| Some(err.to_string()))?,
    };
    if state::plan_info(plan).closes_app {
        // The Windows installer exits the process: it runs only from
        // `update_restart`, once every document has closed.
        return Ok(Some(bytes));
    }
    emit_installing(app, version);
    #[cfg(target_os = "linux")]
    if plan.mode == UpdateMode::SelfInstallElevated {
        use install_channel::PackageInstall;
        let work_dir = app
            .path()
            .app_cache_dir()
            .map_err(|err| Some(err.to_string()))?
            .join("update");
        let channel = plan.channel.clone();
        let outcome = tauri::async_runtime::spawn_blocking(move || {
            install_channel::install_package_elevated(&channel, &bytes, &work_dir)
        })
        .await
        .map_err(|err| Some(err.to_string()))?;
        return match outcome {
            PackageInstall::Installed => Ok(None),
            PackageInstall::Cancelled => Err(None),
            PackageInstall::Failed(message) => Err(Some(message)),
        };
    }
    // macOS and AppImage: replaces the app in place and returns; the restart
    // step is offered.
    update
        .install(bytes)
        .map(|()| None)
        .map_err(|err| Some(err.to_string()))
}

#[tauri::command]
pub fn update_skip(
    app: AppHandle,
    window: tauri::WebviewWindow,
    pending: tauri::State<'_, Pending>,
) -> Result<(), String> {
    require_window(&window, false)?;
    let fixture;
    let version;
    {
        let mut slot = pending.0.lock().unwrap_or_else(|e| e.into_inner());
        let held = slot.as_ref().ok_or("no update is pending")?;
        if held.phase != Phase::Offer {
            return Err("the update is already being installed".into());
        }
        fixture = held.fixture;
        version = held.offer.version.clone();
        // Cleared first so the close below does not count as a reminder.
        *slot = None;
    }
    if !fixture {
        save_state(&app, |s| s.skipped_version = Some(version));
    }
    let _ = window.close();
    Ok(())
}

#[tauri::command]
pub fn update_remind(
    app: AppHandle,
    window: tauri::WebviewWindow,
    pending: tauri::State<'_, Pending>,
) -> Result<(), String> {
    require_window(&window, false)?;
    {
        let mut slot = pending.0.lock().unwrap_or_else(|e| e.into_inner());
        let held = slot.as_ref().ok_or("no update is pending")?;
        if held.phase == Phase::Offer {
            if !held.fixture {
                let until = now_ms() + state::REMIND_AFTER_MS;
                save_state(&app, |s| s.remind_after = Some(until));
            }
            // The offer is dropped; the next check finds it again.
            *slot = None;
        }
    }
    let _ = window.close();
    Ok(())
}

// --- The restart pass (see "Restart" in the module doc) --------------------

/// What a document window reports while a restart pass is running.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PassEvent {
    /// The window was destroyed: saved, discarded, or nothing to save.
    Closed(String),
    /// The window's unsaved-changes prompt was answered Cancel.
    Cancelled(String),
}

/// How a restart pass ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PassOutcome {
    /// No document window is left; the restart can go ahead.
    AllClosed,
    /// Someone pressed Cancel; the restart is off.
    Cancelled,
}

/// The running restart pass, if any (managed state). Present from the moment
/// `update_restart` starts closing documents until a Cancel, a failed
/// installer, or the end of the process.
#[derive(Default)]
pub struct RestartPass(Mutex<Option<Pass>>);

struct Pass {
    /// The document window the pass is waiting on; empty before the first.
    awaiting: String,
    tx: mpsc::Sender<PassEvent>,
}

impl RestartPass {
    fn slot(&self) -> std::sync::MutexGuard<'_, Option<Pass>> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn is_active(&self) -> bool {
        self.slot().is_some()
    }

    /// Start a pass and return the receiving end of its reports. When one is
    /// already running, `Err` carries the label of the window it waits on
    /// (empty if it has not asked one yet).
    fn begin(&self) -> Result<mpsc::Receiver<PassEvent>, String> {
        let mut slot = self.slot();
        if let Some(pass) = slot.as_ref() {
            return Err(pass.awaiting.clone());
        }
        let (tx, rx) = mpsc::channel();
        *slot = Some(Pass {
            awaiting: String::new(),
            tx,
        });
        Ok(rx)
    }

    fn set_awaiting(&self, label: &str) {
        if let Some(pass) = self.slot().as_mut() {
            pass.awaiting = label.to_string();
        }
    }

    /// Hand a window's report to the running pass. Nothing happens when no
    /// pass runs; `PassDriver::on_event` drops a close of a window the pass
    /// is not waiting on (and stops on a Cancel from any window).
    fn deliver(&self, event: PassEvent) {
        if let Some(pass) = self.slot().as_ref() {
            let _ = pass.tx.send(event);
        }
    }

    fn end(&self) {
        *self.slot() = None;
    }
}

/// The document window the pass closes next: those with unsaved changes
/// first, then the rest, each group in window order. `None` when no
/// document window is open.
fn next_to_close(open: &[(String, bool)]) -> Option<String> {
    open.iter()
        .min_by(|(a, a_dirty), (b, b_dirty)| {
            b_dirty
                .cmp(a_dirty)
                .then_with(|| crate::window_sort_key(a).cmp(&crate::window_sort_key(b)))
                .then_with(|| a.cmp(b))
        })
        .map(|(label, _)| label.clone())
}

/// What the pass does next, from `PassDriver::next`.
#[derive(Debug, Clone, PartialEq, Eq)]
enum PassStep {
    /// Ask this window to close, then wait for its report.
    Close(String),
    Finished(PassOutcome),
}

/// What a report means to the pass, from `PassDriver::on_event`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PassWake {
    /// The awaited window closed: look at the open windows again.
    Advance,
    /// A window said Cancel: the pass is over.
    Stop,
    /// Another window closed (or a stale report): keep waiting.
    Ignore,
}

/// The pass's decisions, with no Tauri in them (unit tested). The caller
/// lists the open document windows afresh before every `next`, so a window
/// opened during the pass is closed too.
#[derive(Default)]
struct PassDriver {
    awaiting: Option<String>,
}

impl PassDriver {
    fn next(&mut self, open: &[(String, bool)]) -> PassStep {
        match next_to_close(open) {
            Some(label) => {
                self.awaiting = Some(label.clone());
                PassStep::Close(label)
            }
            None => {
                self.awaiting = None;
                PassStep::Finished(PassOutcome::AllClosed)
            }
        }
    }

    fn on_event(&mut self, event: &PassEvent) -> PassWake {
        let Some(awaiting) = self.awaiting.as_deref() else {
            return PassWake::Ignore;
        };
        match event {
            PassEvent::Closed(label) if label == awaiting => {
                self.awaiting = None;
                PassWake::Advance
            }
            // Cancel in any window stops the pass, not only in the awaited
            // one. The pass asks one window at a time, so a prompt in another
            // window is one the person opened by closing it by hand; their
            // Cancel there still means "not now", and the pass must not go
            // on to ask that same window again as if they had not answered.
            PassEvent::Cancelled(_) => {
                self.awaiting = None;
                PassWake::Stop
            }
            PassEvent::Closed(_) => PassWake::Ignore,
        }
    }
}

/// The open document windows and whether each has unsaved changes, as the
/// webview last reported (`set_document_dirty`).
fn open_documents(app: &AppHandle) -> Vec<(String, bool)> {
    // The window list first, then the dirty flags: never both locks at once.
    let labels: Vec<String> = app
        .webview_windows()
        .into_keys()
        .filter(|label| crate::is_document_window_label(label))
        .collect();
    let live = app.try_state::<crate::live::LiveApi>();
    let docs = live
        .as_ref()
        .map(|live| live.docs.lock().unwrap_or_else(|e| e.into_inner()));
    labels
        .into_iter()
        .map(|label| {
            let dirty = docs.as_ref().is_some_and(|docs| docs.is_dirty(&label));
            (label, dirty)
        })
        .collect()
}

/// Close every document window, one at a time, through its own close
/// guard: focus it, ask it to close (which shows its unsaved-changes dialog
/// when it has changes), and wait with no timeout for it to be destroyed or
/// to answer Cancel. Never destroys a window itself. On `Cancelled` the
/// pass is ended here; on `AllClosed` it stays active so the last window
/// closing does not end the app before the restart.
async fn close_documents_for_restart(
    app: &AppHandle,
    mut rx: mpsc::Receiver<PassEvent>,
) -> PassOutcome {
    let pass = app.state::<RestartPass>();
    let mut driver = PassDriver::default();
    loop {
        let label = match driver.next(&open_documents(app)) {
            PassStep::Finished(outcome) => return outcome,
            PassStep::Close(label) => label,
        };
        pass.set_awaiting(&label);
        let Some(window) = app.get_webview_window(&label) else {
            // Gone between the listing and now; list again.
            continue;
        };
        let _ = crate::focus_window_by_label(app, &label);
        if window.close().is_err() {
            // The close could not even be asked for: stop rather than wait
            // on a window that may never answer. Nothing is lost.
            pass.end();
            return PassOutcome::Cancelled;
        }
        loop {
            let received = tauri::async_runtime::spawn_blocking(move || (rx.recv().ok(), rx)).await;
            let Ok((event, back)) = received else {
                pass.end();
                return PassOutcome::Cancelled;
            };
            rx = back;
            let Some(event) = event else {
                pass.end();
                return PassOutcome::Cancelled;
            };
            match driver.on_event(&event) {
                PassWake::Ignore => continue,
                PassWake::Advance => break,
                PassWake::Stop => {
                    pass.end();
                    return PassOutcome::Cancelled;
                }
            }
        }
    }
}

/// Whether a restart pass is running (main.rs keeps the app alive when the
/// last window closes during one, and keeps the update window open).
pub fn restart_pass_active(app: &AppHandle) -> bool {
    app.try_state::<RestartPass>()
        .is_some_and(|pass| pass.is_active())
}

/// A document window was destroyed (main.rs's window-event hook). A no-op
/// unless a restart pass is running.
pub fn on_document_destroyed(app: &AppHandle, label: &str) {
    if let Some(pass) = app.try_state::<RestartPass>() {
        pass.deliver(PassEvent::Closed(label.to_string()));
    }
}

/// The close guard's Cancel (App.tsx): the window stays open, and a running
/// restart pass stops. A no-op when no pass is running.
#[tauri::command]
pub fn close_cancelled(app: AppHandle, window: tauri::WebviewWindow) -> Result<(), String> {
    if !crate::is_document_window_label(window.label()) {
        return Err("not available from this window".into());
    }
    if let Some(pass) = app.try_state::<RestartPass>() {
        pass.deliver(PassEvent::Cancelled(window.label().to_string()));
    }
    Ok(())
}

/// Whether `held` can restart now. Only reads: the package is taken once
/// every document has closed.
fn restart_ready(held: &PendingUpdate) -> Result<(), String> {
    match held.phase {
        Phase::Installed => Ok(()),
        Phase::Ready if held.fixture || (held.update.is_some() && held.bytes.is_some()) => Ok(()),
        Phase::Ready => Err("the downloaded update is no longer available".into()),
        _ => Err("the update is not ready yet".into()),
    }
}

/// What the restart does once every document has closed.
enum Finish {
    /// Installed in place: start the new version.
    Restart,
    /// Debug fixture: end the process instead of restarting or installing.
    Exit,
    /// Downloaded (Windows): run the installer, which ends the process.
    Install {
        update: Box<tauri_plugin_updater::Update>,
        bytes: Vec<u8>,
        version: String,
    },
    /// Nothing can be installed after all.
    Fail { version: String, error: String },
}

fn take_finish(held: Option<&mut PendingUpdate>) -> Finish {
    let Some(held) = held else {
        return Finish::Fail {
            version: String::new(),
            error: "no update is pending".into(),
        };
    };
    let version = held.offer.version.clone();
    match held.phase {
        Phase::Installed if held.fixture => Finish::Exit,
        Phase::Installed => Finish::Restart,
        Phase::Ready if held.fixture && held.fixture_fail_restart => Finish::Fail {
            version,
            error: "The installer could not be started (test fixture).".into(),
        },
        Phase::Ready if held.fixture => Finish::Exit,
        Phase::Ready => match (held.update.clone(), held.bytes.take()) {
            (Some(update), Some(bytes)) => Finish::Install {
                update: Box::new(update),
                bytes,
                version,
            },
            _ => Finish::Fail {
                version,
                error: "the downloaded update is no longer available".into(),
            },
        },
        _ => Finish::Fail {
            version,
            error: "the update is not ready yet".into(),
        },
    }
}

/// Every document has closed: restart, or run the installer. Returns only
/// when that failed, after reopening a document window and the update
/// window with the failure.
async fn finish_restart(app: &AppHandle) -> Result<RestartOutcome, String> {
    let finish = {
        let pending = app.state::<Pending>();
        let mut slot = pending.0.lock().unwrap_or_else(|e| e.into_inner());
        take_finish(slot.as_mut())
    };
    match finish {
        Finish::Restart => app.restart(),
        Finish::Exit => {
            app.exit(0);
            std::future::pending().await
        }
        Finish::Install {
            update,
            bytes,
            version,
        } => {
            let _ = app.emit_to(
                WINDOW,
                "update-progress",
                serde_json::json!({
                    "phase": "installing", "version": version, "closesApp": true,
                }),
            );
            // The installer ends the process with `exit`, which skips
            // RunEvent::Exit and so main.rs's cleanup.
            crate::live::cleanup();
            match update.install(bytes) {
                // Only where the installer does not end the process itself.
                Ok(()) => app.restart(),
                Err(err) => restart_failed(app, version, err.to_string()).await,
            }
        }
        Finish::Fail { version, error } => restart_failed(app, version, error).await,
    }
}

/// The installer could not start after every document closed: the update
/// goes back to an offer, an empty document window and the update window
/// open, and the window shows the failure with Open Download Page.
async fn restart_failed(
    app: &AppHandle,
    version: String,
    error: String,
) -> Result<RestartOutcome, String> {
    let closes_app = {
        let pending = app.state::<Pending>();
        let mut slot = pending.0.lock().unwrap_or_else(|e| e.into_inner());
        slot.as_mut().is_some_and(|held| {
            held.phase = Phase::Offer;
            held.bytes = None;
            held.last_error = Some(error.clone());
            state::plan_info(&held.plan).closes_app
        })
    };
    // Opened while the pass still holds the app alive.
    if open_documents(app).is_empty() {
        let _ = crate::create_document_window(app, None, None, true).await;
    }
    let _ = show_window(app, false).await;
    app.state::<RestartPass>().end();
    let result = InstallResult {
        status: "failed",
        version,
        error: Some(error.clone()),
        closes_app,
    };
    let _ = app.emit_to(WINDOW, "update-result", &result);
    Err(error)
}

impl RestartOutcome {
    const CANCELLED: RestartOutcome = RestartOutcome {
        status: "cancelled",
    };
    const IN_PROGRESS: RestartOutcome = RestartOutcome {
        status: "inProgress",
    };
}

/// Restart into the update: close every document through its save prompt,
/// then restart into the installed version, or (Windows, downloaded) run the
/// installer, which closes Hew and starts it again. Returns only when the
/// restart did not happen: `cancelled`, `inProgress`, or an error.
#[tauri::command]
pub async fn update_restart(
    app: AppHandle,
    window: tauri::WebviewWindow,
) -> Result<RestartOutcome, String> {
    require_window(&window, true)?;
    {
        let pending = app.state::<Pending>();
        let slot = pending.0.lock().unwrap_or_else(|e| e.into_inner());
        restart_ready(slot.as_ref().ok_or("no update is pending")?)?;
    }
    let rx = match app.state::<RestartPass>().begin() {
        Ok(rx) => rx,
        Err(awaiting) => {
            if !awaiting.is_empty() {
                let _ = crate::focus_window_by_label(&app, &awaiting);
            }
            return Ok(RestartOutcome::IN_PROGRESS);
        }
    };
    match close_documents_for_restart(&app, rx).await {
        // The update stays installed or downloaded, exactly as it was.
        PassOutcome::Cancelled => Ok(RestartOutcome::CANCELLED),
        PassOutcome::AllClosed => finish_restart(&app).await,
    }
}

/// Copy the tell-only command to the clipboard.
#[tauri::command]
pub fn update_copy_command(
    app: AppHandle,
    window: tauri::WebviewWindow,
    pending: tauri::State<'_, Pending>,
) -> Result<(), String> {
    require_window(&window, false)?;
    let command = pending
        .0
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .and_then(|held| held.plan.command.clone())
        .ok_or("there is no command to copy")?;
    app.clipboard()
        .write_text(command)
        .map_err(|e| e.to_string())
}

/// "What's New" on the notice bar: the window in its read-only form.
#[tauri::command]
pub async fn update_show_notes(app: AppHandle, window: tauri::WebviewWindow) -> Result<(), String> {
    if !crate::is_document_window_label(window.label()) {
        return Err("not available from this window".into());
    }
    show_window(&app, true).await
}

#[tauri::command]
pub fn update_ready_get(
    window: tauri::WebviewWindow,
    pending: tauri::State<'_, Pending>,
) -> Result<Option<UpdateReady>, String> {
    if !crate::is_document_window_label(window.label()) {
        return Err("not available from this window".into());
    }
    Ok(pending
        .0
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .and_then(ready_of))
}

fn auto_value(app: &AppHandle) -> AutoInstall {
    let supported = state::auto_install_supported(&install_channel::current_plan());
    AutoInstall {
        enabled: supported && load_state(app).auto_install,
        supported,
    }
}

fn require_auto_window(window: &tauri::WebviewWindow) -> Result<(), String> {
    if window.label() == "settings" {
        return Ok(());
    }
    require_window(window, true)
}

#[tauri::command]
pub fn update_auto_get(
    app: AppHandle,
    window: tauri::WebviewWindow,
) -> Result<AutoInstall, String> {
    require_auto_window(&window)?;
    Ok(auto_value(&app))
}

/// Saved the moment it is toggled. Refused where it cannot work.
#[tauri::command]
pub fn update_auto_set(
    app: AppHandle,
    window: tauri::WebviewWindow,
    enabled: bool,
) -> Result<AutoInstall, String> {
    require_auto_window(&window)?;
    if !state::auto_install_supported(&install_channel::current_plan()) {
        return Err("this copy of Hew cannot update itself automatically".into());
    }
    save_state(&app, |s| s.auto_install = enabled);
    Ok(auto_value(&app))
}

/// Debug builds only: open the update window from a made-up update, so the
/// UI can be driven without a signed release. `mode` is `"selfInstall"`
/// (default), `"selfInstallElevated"`, `"notifyOnly"` (with a Flatpak-style
/// command), or `"windows"` (self-install that closes the app). `phase` is
/// `"offer"` (default), `"installed"` or `"ready"`. `readOnly` opens the
/// What's New form. Install on a fixture simulates the progress events and
/// installs nothing; skip and remind do not write `update.json`. Restart on
/// a fixture runs the real document-closing pass and then exits instead of
/// restarting or installing; with `failRestart` a `"ready"` restart takes
/// the installer-failed path instead. `noticeBar` (phase `"installed"` or
/// `"ready"`) shows the document windows' notice bar instead of the window.
#[cfg(debug_assertions)]
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn update_open_fixture(
    app: AppHandle,
    mode: Option<String>,
    notes: Option<String>,
    version: Option<String>,
    phase: Option<String>,
    read_only: Option<bool>,
    fail_restart: Option<bool>,
    notice_bar: Option<bool>,
) -> Result<(), String> {
    use install_channel::{plan_for, InstallChannel};
    if app.state::<RestartPass>().is_active() {
        return Err("Hew is already closing to restart".into());
    }
    let channel = match mode.as_deref().unwrap_or("selfInstall") {
        "selfInstall" => InstallChannel::MacApp,
        "windows" => InstallChannel::WindowsInstaller,
        "selfInstallElevated" => InstallChannel::Deb,
        "notifyOnly" => InstallChannel::Flatpak,
        "notifyOnlyNoCommand" => InstallChannel::Unknown,
        other => return Err(format!("unknown fixture mode {other}")),
    };
    let phase = match phase.as_deref().unwrap_or("offer") {
        "offer" => Phase::Offer,
        "installed" => Phase::Installed,
        "ready" => Phase::Ready,
        other => return Err(format!("unknown fixture phase {other}")),
    };
    let plan = plan_for(channel);
    let version_shown = version.unwrap_or_else(|| "9.9.9".into());
    let notes = notes.or_else(|| {
        Some(
            "### Added\n\n- **Lock:** keep an object from being moved or edited.\n- Update window with release notes.\n  - Skip and remind options.\n\n### Fixed\n\n- A `fixture` bug. See [the site](https://hew3d.com).\n".to_string(),
        )
    });
    {
        let pending = app.state::<Pending>();
        *pending.0.lock().unwrap_or_else(|e| e.into_inner()) = Some(PendingUpdate {
            offer: Offer {
                version: version_shown.clone(),
                current_version: env!("CARGO_PKG_VERSION").into(),
                notes,
                date: Some("2026-12-01".into()),
            },
            plan: plan.clone(),
            update: None,
            phase,
            bytes: None,
            read_only: false,
            fixture: true,
            fixture_fail_restart: fail_restart.unwrap_or(false),
            last_error: None,
            quiet_close: false,
        });
    }
    if notice_bar.unwrap_or(false) {
        if phase != Phase::Installed && phase != Phase::Ready {
            return Err("the notice bar needs phase installed or ready".into());
        }
        emit_ready(
            &app,
            &UpdateReady {
                version: version_shown,
                current_version: env!("CARGO_PKG_VERSION").into(),
                installed: phase == Phase::Installed,
                closes_app: state::plan_info(&plan).closes_app,
            },
        );
        return Ok(());
    }
    show_window(&app, read_only.unwrap_or(false)).await
}

/// Debug builds only: `HEW_UPDATE_FIXTURE` set to the JSON arguments of
/// `update_open_fixture` (camelCase) opens that fixture a few seconds after
/// launch (`delayMs`, default 4000), so a real-app check can be scripted
/// without the devtools. An array of them runs in order, each `delayMs`
/// after the one before (say, the notice bar first, then the window).
#[cfg(debug_assertions)]
pub fn spawn_fixture_from_env(app: &AppHandle) {
    let Ok(text) = std::env::var("HEW_UPDATE_FIXTURE") else {
        return;
    };
    let steps = match serde_json::from_str::<serde_json::Value>(&text) {
        Ok(serde_json::Value::Array(steps)) => steps,
        Ok(step) => vec![step],
        Err(_) => {
            eprintln!("hew: HEW_UPDATE_FIXTURE is not JSON");
            return;
        }
    };
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        for args in steps {
            let delay = args.get("delayMs").and_then(|v| v.as_u64()).unwrap_or(4000);
            sleep_ms(delay).await;
            let text_of = |key: &str| args.get(key).and_then(|v| v.as_str()).map(str::to_string);
            let flag = |key: &str| args.get(key).and_then(|v| v.as_bool());
            let result = update_open_fixture(
                app.clone(),
                text_of("mode"),
                text_of("notes"),
                text_of("version"),
                text_of("phase"),
                flag("readOnly"),
                flag("failRestart"),
                flag("noticeBar"),
            )
            .await;
            if let Err(err) = result {
                eprintln!("hew: HEW_UPDATE_FIXTURE: {err}");
            }
        }
    });
}

// --- Native dialogs (the two that stay native) -----------------------------

/// A non-blocking informational dialog (no decision to wait on).
fn info_dialog(app: &AppHandle, title: &str, body: &str) {
    app.dialog()
        .message(body)
        .title(title)
        .kind(MessageDialogKind::Info)
        .show(|_| {});
}

/// A non-blocking error dialog.
fn error_dialog(app: &AppHandle, body: &str) {
    let labels = crate::current_menu_labels(app);
    app.dialog()
        .message(body)
        .title(labels.get("updater:title"))
        .kind(MessageDialogKind::Error)
        .show(|_| {});
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::install_channel::{plan_for, InstallChannel};

    fn open(list: &[(&str, bool)]) -> Vec<(String, bool)> {
        list.iter().map(|(l, d)| (l.to_string(), *d)).collect()
    }

    fn held(channel: InstallChannel, phase: Phase, bytes: Option<Vec<u8>>) -> PendingUpdate {
        PendingUpdate {
            offer: Offer {
                version: "1.6.0".into(),
                current_version: "1.5.0".into(),
                notes: None,
                date: None,
            },
            plan: plan_for(channel),
            update: None,
            phase,
            bytes,
            read_only: false,
            fixture: true,
            fixture_fail_restart: false,
            last_error: None,
            quiet_close: false,
        }
    }

    #[test]
    fn unsaved_windows_close_first_then_window_order() {
        assert_eq!(next_to_close(&[]), None);
        assert_eq!(
            next_to_close(&open(&[("main-2", false), ("main", false)])).as_deref(),
            Some("main")
        );
        assert_eq!(
            next_to_close(&open(&[
                ("main", false),
                ("main-3", true),
                ("main-2", true)
            ]))
            .as_deref(),
            Some("main-2")
        );
        // Creation order, not text order: main-10 comes after main-9.
        assert_eq!(
            next_to_close(&open(&[("main-10", true), ("main-9", true)])).as_deref(),
            Some("main-9")
        );
        assert_eq!(
            next_to_close(&open(&[("main-10", false), ("main", true)])).as_deref(),
            Some("main")
        );
    }

    #[test]
    fn the_pass_advances_only_on_the_awaited_window() {
        let mut windows = open(&[("main", false), ("main-2", true)]);
        let mut pass = PassDriver::default();
        assert_eq!(pass.next(&windows), PassStep::Close("main-2".into()));
        // Another window closing (by hand) does not advance the pass.
        assert_eq!(
            pass.on_event(&PassEvent::Closed("main".into())),
            PassWake::Ignore
        );
        assert_eq!(
            pass.on_event(&PassEvent::Closed("main-2".into())),
            PassWake::Advance
        );
        // A stale second report for the same window changes nothing.
        assert_eq!(
            pass.on_event(&PassEvent::Closed("main-2".into())),
            PassWake::Ignore
        );
        windows.retain(|(l, _)| l != "main-2");
        // A window opened during the pass is closed too.
        windows.push(("main-3".into(), false));
        assert_eq!(pass.next(&windows), PassStep::Close("main".into()));
        assert_eq!(
            pass.on_event(&PassEvent::Closed("main".into())),
            PassWake::Advance
        );
        windows.retain(|(l, _)| l != "main");
        assert_eq!(pass.next(&windows), PassStep::Close("main-3".into()));
        assert_eq!(
            pass.on_event(&PassEvent::Closed("main-3".into())),
            PassWake::Advance
        );
        windows.clear();
        assert_eq!(
            pass.next(&windows),
            PassStep::Finished(PassOutcome::AllClosed)
        );
    }

    /// The person closes a window by hand while the pass waits on another
    /// one, and answers Cancel in that window's save prompt: the pass stops
    /// there, and never goes on to ask that window again.
    #[test]
    fn a_cancel_in_any_window_stops_the_pass() {
        let windows = open(&[("main", true), ("main-2", true)]);
        let mut pass = PassDriver::default();
        assert_eq!(pass.next(&windows), PassStep::Close("main".into()));
        assert_eq!(
            pass.on_event(&PassEvent::Cancelled("main-2".into())),
            PassWake::Stop
        );
        // Over: later reports change nothing.
        assert_eq!(
            pass.on_event(&PassEvent::Closed("main".into())),
            PassWake::Ignore
        );
        // Also after an advance, before the next window was asked: the
        // report waits in the channel and stops the pass at the next wait.
        let mut pass = PassDriver::default();
        assert_eq!(pass.next(&windows), PassStep::Close("main".into()));
        assert_eq!(
            pass.on_event(&PassEvent::Closed("main".into())),
            PassWake::Advance
        );
        assert_eq!(
            pass.next(&open(&[("main-2", true), ("main-3", false)])),
            PassStep::Close("main-2".into())
        );
        assert_eq!(
            pass.on_event(&PassEvent::Cancelled("main-3".into())),
            PassWake::Stop
        );
    }

    #[test]
    fn closing_means_remind_only_for_a_genuinely_open_offer() {
        let mut offer = held(InstallChannel::MacApp, Phase::Offer, None);
        offer.fixture = false;
        assert!(remind_on_close(&offer), "the person closing the offer");
        // The shell closing it (last document gone, Hew quitting).
        let mut shell = held(InstallChannel::MacApp, Phase::Offer, None);
        shell.fixture = false;
        shell.quiet_close = true;
        assert!(!remind_on_close(&shell));
        // A failed install, and a failed restart, both back at Offer.
        let mut failed = held(InstallChannel::MacApp, Phase::Downloading, None);
        failed.fixture = false;
        assert_eq!(
            settle_install(Some(&mut failed), false, Err(Some("offline".into()))).0,
            "failed"
        );
        assert_eq!(failed.phase, Phase::Offer);
        assert!(!remind_on_close(&failed));
        let mut no_installer = held(InstallChannel::WindowsInstaller, Phase::Offer, None);
        no_installer.fixture = false;
        no_installer.last_error = Some("installer did not start".into());
        assert!(!remind_on_close(&no_installer));
        // A dismissed password prompt is back at a real offer.
        let mut back = held(InstallChannel::Deb, Phase::Downloading, None);
        back.fixture = false;
        back.last_error = Some("stale".into());
        assert_eq!(
            settle_install(Some(&mut back), false, Err(None)).0,
            "cancelled"
        );
        assert!(remind_on_close(&back), "the error was cleared");
        // What's New, work under way or done, a debug fixture.
        let mut notes = held(InstallChannel::MacApp, Phase::Offer, None);
        notes.fixture = false;
        notes.read_only = true;
        assert!(!remind_on_close(&notes));
        for phase in [Phase::Downloading, Phase::Installed, Phase::Ready] {
            let mut busy = held(InstallChannel::MacApp, phase, None);
            busy.fixture = false;
            assert!(!remind_on_close(&busy));
        }
        assert!(!remind_on_close(&held(
            InstallChannel::MacApp,
            Phase::Offer,
            None
        )));
    }

    #[test]
    fn an_open_window_never_turns_into_whats_new() {
        // A new window takes the form asked for.
        assert!(form_after_show(false, false, true));
        assert!(!form_after_show(false, true, false));
        // What's New on an open full window (Restart Now showing): stays full.
        assert!(!form_after_show(true, false, true));
        // A check on an open What's New window: becomes the full window.
        assert!(!form_after_show(true, true, false));
        assert!(form_after_show(true, true, true));
    }

    #[test]
    fn no_documents_means_all_closed_at_once() {
        let mut pass = PassDriver::default();
        assert_eq!(pass.next(&[]), PassStep::Finished(PassOutcome::AllClosed));
        assert_eq!(
            pass.on_event(&PassEvent::Cancelled("main".into())),
            PassWake::Ignore
        );
    }

    #[test]
    fn cancel_in_the_awaited_window_stops_and_keeps_the_update() {
        let mut pending = held(
            InstallChannel::WindowsInstaller,
            Phase::Ready,
            Some(vec![1, 2, 3]),
        );
        assert_eq!(restart_ready(&pending), Ok(()), "validating takes nothing");
        assert_eq!(pending.bytes.as_deref(), Some(&[1u8, 2, 3][..]));
        let mut pass = PassDriver::default();
        let windows = open(&[("main", true), ("main-2", false)]);
        assert_eq!(pass.next(&windows), PassStep::Close("main".into()));
        assert_eq!(
            pass.on_event(&PassEvent::Cancelled("main".into())),
            PassWake::Stop
        );
        // `update_restart` returns "cancelled" without touching the pending
        // update: still ready, package still held, restartable again.
        assert_eq!(pending.phase, Phase::Ready);
        assert_eq!(pending.bytes.as_deref(), Some(&[1u8, 2, 3][..]));
        assert_eq!(restart_ready(&pending), Ok(()));
        // And only after every document closed is the package taken.
        pending.fixture = false;
        assert!(
            matches!(take_finish(Some(&mut pending)), Finish::Fail { .. }),
            "a real package needs its update handle"
        );
    }

    #[test]
    fn a_second_restart_while_one_runs_is_in_progress() {
        let pass = RestartPass::default();
        assert!(!pass.is_active());
        let rx = pass.begin().expect("first start");
        assert!(pass.is_active());
        assert_eq!(
            pass.begin().err().as_deref(),
            Some(""),
            "not waiting on anyone yet"
        );
        pass.set_awaiting("main-2");
        assert_eq!(pass.begin().err().as_deref(), Some("main-2"));
        pass.deliver(PassEvent::Cancelled("main-2".into()));
        assert_eq!(rx.try_recv(), Ok(PassEvent::Cancelled("main-2".into())));
        pass.end();
        assert!(!pass.is_active());
        // With no pass, reports go nowhere and nothing panics.
        pass.deliver(PassEvent::Closed("main".into()));
        assert!(rx.try_recv().is_err());
        assert!(pass.begin().is_ok(), "a new pass can start after one ended");
    }

    #[test]
    fn restart_needs_an_installed_or_downloaded_update() {
        let mut pending = held(InstallChannel::MacApp, Phase::Offer, None);
        assert!(restart_ready(&pending).is_err());
        pending.phase = Phase::Downloading;
        assert!(restart_ready(&pending).is_err());
        pending.phase = Phase::Installed;
        assert_eq!(restart_ready(&pending), Ok(()));
        // A real (non-fixture) download without its package is refused up
        // front, before any window is asked to close.
        let mut win = held(InstallChannel::WindowsInstaller, Phase::Ready, None);
        win.fixture = false;
        assert!(restart_ready(&win).is_err());
    }

    #[test]
    fn a_fixture_restart_exits_or_takes_the_failure_path() {
        let mut installed = held(InstallChannel::MacApp, Phase::Installed, None);
        assert!(matches!(take_finish(Some(&mut installed)), Finish::Exit));
        let mut ready = held(InstallChannel::WindowsInstaller, Phase::Ready, None);
        assert!(matches!(take_finish(Some(&mut ready)), Finish::Exit));
        ready.fixture_fail_restart = true;
        assert!(matches!(take_finish(Some(&mut ready)), Finish::Fail { .. }));
        assert!(matches!(take_finish(None), Finish::Fail { .. }));
    }

    #[test]
    fn install_on_a_plan_that_closes_hew_stops_at_ready_with_the_package() {
        let mut pending = held(InstallChannel::WindowsInstaller, Phase::Downloading, None);
        let closes_app = state::plan_info(&pending.plan).closes_app;
        assert!(closes_app);
        let (status, error) = settle_install(Some(&mut pending), closes_app, Ok(Some(vec![7; 4])));
        assert_eq!((status, error), ("ready", None));
        assert_eq!(pending.phase, Phase::Ready);
        assert_eq!(pending.bytes.as_deref(), Some(&[7u8; 4][..]));
        // The debug fixture downloads nothing but still stops at ready.
        let mut fixture = held(InstallChannel::WindowsInstaller, Phase::Downloading, None);
        assert_eq!(
            settle_install(Some(&mut fixture), true, Ok(None)).0,
            "ready"
        );
        assert_eq!(fixture.phase, Phase::Ready);
    }

    #[test]
    fn install_in_place_cancel_and_failure_settle_as_before() {
        let mut mac = held(InstallChannel::MacApp, Phase::Downloading, None);
        assert_eq!(
            settle_install(Some(&mut mac), false, Ok(None)),
            ("installed", None)
        );
        assert_eq!(mac.phase, Phase::Installed);
        let mut deb = held(InstallChannel::Deb, Phase::Downloading, None);
        assert_eq!(
            settle_install(Some(&mut deb), false, Err(None)),
            ("cancelled", None)
        );
        assert_eq!(deb.phase, Phase::Offer);
        let mut win = held(InstallChannel::WindowsInstaller, Phase::Downloading, None);
        assert_eq!(
            settle_install(Some(&mut win), true, Err(Some("offline".into()))),
            ("failed", Some("offline".into()))
        );
        assert_eq!(win.phase, Phase::Offer);
        assert!(win.bytes.is_none());
    }

    #[test]
    fn restart_outcome_shape() {
        assert_eq!(
            serde_json::to_value(RestartOutcome::CANCELLED).unwrap(),
            serde_json::json!({ "status": "cancelled" })
        );
        assert_eq!(
            serde_json::to_value(RestartOutcome::IN_PROGRESS).unwrap(),
            serde_json::json!({ "status": "inProgress" })
        );
    }
}

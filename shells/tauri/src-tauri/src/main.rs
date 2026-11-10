// Prevents an additional console window on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
// Outside the determinism-critical kernel scope (kernel / inference /
// tessellate / mesh-heal). The workspace `clippy.toml` bans HashMap/HashSet for
// kernel determinism, but it also applies to this desktop shell, where the only
// hit is HashMap inside `tauri::generate_context!()`'s macro-generated code —
// not kernel output, and not ours to change. Suppress the ban for this crate
// exactly as wasm-api / dae-import / gltf-import do. (Previously latent: the
// Tauri-shell clippy result was cache-masked until touched main.rs.)
#![allow(clippy::disallowed_types)]

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;

use menu_labels::MenuLabels;
use tauri::{
    menu::{
        CheckMenuItem, CheckMenuItemBuilder, MenuBuilder, MenuItem, MenuItemBuilder, MenuItemKind,
        PredefinedMenuItem, SubmenuBuilder,
    },
    Emitter, Manager,
};

// System font enumeration + reading for the 3D Text font picker — see
// fonts.rs's top-of-file doc comment for the security model
// (`read_font_file` takes an opaque token, never a path).
mod fly_cursor;
mod fonts;
mod menu_labels;

// The desktop half of `--live` (docs/agents/HEW_API.md §11.2): the local socket
// transport, the discovery file, and the token gate. See live.rs's
// top-of-file doc comment for the full contract with
// crates/wasm-api/src/live.rs (the webview-side dispatch half).
mod live;
mod live_router;

// "Open on Phone" QR handoff (docs/design/shop-mode.md §4, workers/
// share-relay/README.md): the desktop encrypts the document and uploads it
// to a Cloudflare Worker dead-drop; this module only renders the resulting
// URL as an SVG QR code (the encryption and upload happen entirely in the
// webview — see qr.rs's own doc comment).
mod print;
mod qr;
// The native save panel with the format list (and its options) in it.
mod save_panel;
// The "Open on Phone" relay client + the server setting it is bound to
// (Settings ▸ Advanced ▸ Server) — see its module doc for why the requests
// live in Rust rather than behind an HTTP plugin scope.
mod follow;
mod relay_client;
mod report_client;

// ---------------------------------------------------------------------------
// In-app auto-updater (compiled in via the `updater` feature — see Cargo.toml).
//
// Talks to the signed `latest.json` published on GitHub Releases: a silent
// check on launch, plus a manual "Check for Updates" menu item. The whole
// flow lives shell-side — check, native confirm, download, restart prompt —
// so the webview is granted no update-related capability, and a
// `--no-default-features` build (Flathub/winget/AUR, which own their own
// updates) drops the module, the menu item, and the launch check together.
// ---------------------------------------------------------------------------
#[cfg(feature = "updater")]
mod updater {
    use std::sync::atomic::{AtomicBool, Ordering};
    use tauri::{AppHandle, Manager};
    use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
    use tauri_plugin_updater::UpdaterExt;

    /// Single-flight guard (managed state). The silent launch check and the
    /// manual "Check for Updates" run independently; without this, an update
    /// that appears while the user also clicks the menu item would stack two
    /// dialogs and — on accept — start two concurrent installs writing the same
    /// target (the macOS bundle swap is not atomic). One flag serializes them.
    #[derive(Default)]
    pub struct UpdateGuard(pub AtomicBool);

    /// Run an update check against the configured endpoint — at most one at a
    /// time. A second call while one is in flight backs off: the launch check
    /// silently, a manual click with a note.
    ///
    /// `interactive` splits the two entry points:
    ///  - the manual menu item (`true`): always give feedback — report "up to
    ///    date" and surface errors, so a click is never silent.
    ///  - the launch check (`false`): stay quiet unless an update is actually
    ///    available — no popup when current, no error dialog on an offline
    ///    launch.
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

    /// The check → prompt → download → restart flow itself; `run_check`
    /// serializes calls to it.
    async fn check_and_install(app: AppHandle, interactive: bool) {
        let updater = match app.updater() {
            Ok(updater) => updater,
            Err(err) => {
                if interactive {
                    error_dialog(&app, &format!("Could not start the updater.\n\n{err}"));
                }
                return;
            }
        };
        match updater.check().await {
            Ok(Some(update)) => prompt_and_install(app, update).await,
            Ok(None) => {
                if interactive {
                    info_dialog(
                        &app,
                        "You’re up to date",
                        "Hew is running the latest version.",
                    );
                }
            }
            Err(err) => {
                if interactive {
                    error_dialog(&app, &format!("Could not check for updates.\n\n{err}"));
                }
            }
        }
    }

    /// Offer the found update; on acceptance, download it and offer a restart.
    /// Each prompt is a native modal, and the install replaces the running app
    /// in place (the `.app` bundle on macOS, the NSIS install on Windows, the
    /// AppImage on Linux).
    async fn prompt_and_install(app: AppHandle, update: tauri_plugin_updater::Update) {
        let labels = crate::current_menu_labels(&app);
        let install = confirm(
            &app,
            labels.get("updater:available-title"),
            &labels
                .get("updater:available-body")
                .replace("{version}", &update.version)
                .replace("{current}", &update.current_version),
            labels.get("updater:install"),
            labels.get("updater:later"),
        )
        .await;
        if !install {
            return;
        }
        // Platform note: on macOS and Linux the install returns here and the
        // restart below is offered (the user may choose "Later"). On Windows
        // the plugin launches the NSIS/MSI installer and exits the process
        // immediately, so the restart prompt is macOS/Linux-only in practice —
        // the Windows path ends the app here. Autosave/crash-recovery covers a
        // dirty document if that happens.
        if let Err(err) = update
            .download_and_install(|_chunk, _total| {}, || {})
            .await
        {
            error_dialog(
                &app,
                &labels
                    .get("updater:failed-body")
                    .replace("{error}", &err.to_string()),
            );
            return;
        }
        let restart = confirm(
            &app,
            labels.get("updater:installed-title"),
            labels.get("updater:installed-body"),
            labels.get("updater:restart"),
            labels.get("updater:later"),
        )
        .await;
        if restart {
            app.restart();
        }
    }

    /// A modal OK/Cancel, run off the event loop: the dialog plugin forbids its
    /// blocking API on the main thread (the file pickers take the same
    /// `spawn_blocking` route). Returns true when the primary button is chosen.
    async fn confirm(app: &AppHandle, title: &str, body: &str, ok: &str, cancel: &str) -> bool {
        let app = app.clone();
        let title = title.to_string();
        let body = body.to_string();
        let ok = ok.to_string();
        let cancel = cancel.to_string();
        tauri::async_runtime::spawn_blocking(move || {
            app.dialog()
                .message(body)
                .title(title)
                .kind(MessageDialogKind::Info)
                .buttons(MessageDialogButtons::OkCancelCustom(ok, cancel))
                .blocking_show()
        })
        .await
        .unwrap_or(false)
    }

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
        app.dialog()
            .message(body)
            .title("Update")
            .kind(MessageDialogKind::Error)
            .show(|_| {});
    }
}

/// Manually trigger an update check — the entry point for the in-app menu
/// bar's "Check for Updates" item (Windows/Linux, where the webview drives the
/// menu). macOS routes its native menu item through `on_menu_event` instead. A
/// no-op when the updater was compiled out.
#[tauri::command]
async fn check_for_updates(app: tauri::AppHandle) {
    #[cfg(feature = "updater")]
    updater::run_check(app, true).await;
    #[cfg(not(feature = "updater"))]
    let _ = app;
}

/// Whether this build carries the auto-updater. The in-app menu bar shows the
/// "Check for Updates" item only when true, so package-manager builds (which
/// compile the updater out) never advertise an update path they don't have.
#[tauri::command]
fn updater_available() -> bool {
    cfg!(feature = "updater")
}

// ---------------------------------------------------------------------------
// Recent files — stored as JSON in the app config dir.
// Max 10 entries, most-recent first, deduped by path.
// ---------------------------------------------------------------------------

const RECENTS_MAX: usize = 10;

/// Load the recent-files list from the JSON store file.
fn load_recents(app: &tauri::AppHandle) -> Vec<String> {
    let Some(config_dir) = app.path().app_config_dir().ok() else {
        return Vec::new();
    };
    let path = config_dir.join("recents.json");
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    serde_json::from_str::<Vec<String>>(&text).unwrap_or_default()
}

/// Persist the recent-files list to the JSON store file.
fn save_recents(app: &tauri::AppHandle, recents: &[String]) {
    let Some(config_dir) = app.path().app_config_dir().ok() else {
        return;
    };
    let _ = std::fs::create_dir_all(&config_dir);
    let path = config_dir.join("recents.json");
    if let Ok(text) = serde_json::to_string(recents) {
        let _ = std::fs::write(&path, text);
    }
}

/// Extract the filename (basename) from an absolute path.
fn basename(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

/// Attach the per-platform menu accelerator: macOS keeps its long-standing
/// Cmd-combo scheme; every other platform advertises the
/// SketchUp-for-Windows scheme so the native menu, the tool rail, and
/// the TS keydown handler all show the same keys for the same tools.
/// `None` = no accelerator on that platform.
fn accel(
    builder: tauri::menu::MenuItemBuilder,
    mac: Option<&str>,
    win: Option<&str>,
) -> tauri::menu::MenuItemBuilder {
    match if cfg!(target_os = "macos") { mac } else { win } {
        Some(a) => builder.accelerator(a),
        None => builder,
    }
}

/// Build a checkable menu item (per-platform accelerator like `accel`) and
/// register its handle in `checks` so `sync_menu_state` can reach it later.
/// Used for every stateful item: the active tool radio group, View toggles,
/// and the Window pane toggles.
fn check_item(
    handle: &tauri::AppHandle,
    checks: &mut HashMap<String, CheckMenuItem<tauri::Wry>>,
    id: &str,
    label: &str,
    mac: Option<&str>,
    win: Option<&str>,
) -> tauri::Result<CheckMenuItem<tauri::Wry>> {
    let builder = CheckMenuItemBuilder::with_id(id, label).checked(false);
    let builder = match if cfg!(target_os = "macos") { mac } else { win } {
        Some(a) => builder.accelerator(a),
        None => builder,
    };
    let item = builder.build(handle)?;
    checks.insert(id.to_string(), item.clone());
    Ok(item)
}

/// An Edit-menu clipboard item: on macOS an UNGATED item carrying its
/// standard key equivalent (a disabled item has no key equivalent, and the
/// key must reach a focused text field's own editing command — see the
/// Edit menu comment where these are built); elsewhere a gated,
/// accelerator-free item exactly like `gated_item`.
fn edit_item(
    handle: &tauri::AppHandle,
    items: &mut HashMap<String, MenuItem<tauri::Wry>>,
    id: &str,
    label: &str,
    mac_accel: &str,
) -> tauri::Result<MenuItem<tauri::Wry>> {
    if cfg!(target_os = "macos") {
        MenuItemBuilder::with_id(id, label)
            .accelerator(mac_accel)
            .build(handle)
    } else {
        gated_item(handle, items, id, label, None, None)
    }
}

/// Build a plain menu item and register its handle in `items` so
/// `sync_menu_state` can enable/disable it (selection-dependent Edit
/// commands: Group, Explode, …).
fn gated_item(
    handle: &tauri::AppHandle,
    items: &mut HashMap<String, MenuItem<tauri::Wry>>,
    id: &str,
    label: &str,
    mac: Option<&str>,
    win: Option<&str>,
) -> tauri::Result<MenuItem<tauri::Wry>> {
    let builder = accel(MenuItemBuilder::with_id(id, label), mac, win);
    let item = builder.build(handle)?;
    items.insert(id.to_string(), item.clone());
    Ok(item)
}

// ---------------------------------------------------------------------------
// Managed state
// ---------------------------------------------------------------------------

/// Holds the mutable recents list + a handle to the "Open Recent" submenu so
/// commands can rebuild it without recreating the whole menu bar.
struct RecentState {
    paths: Vec<String>,
    submenu: tauri::menu::Submenu<tauri::Wry>,
}

/// Pending path to open on app startup (cold-start file association). Only
/// populated while no webview has signaled readiness (`frontend_ready`) —
/// once one has, opens are delivered live via `menu-open-path` instead, so a
/// consumed-once buffer can never go stale and leak into a later File ▸ New
/// window's mount-time poll.
struct PendingOpen(Option<String>);

/// Labels of webviews that have registered their `menu-open-path` listener
/// (via the `frontend_ready` command). Gates live delivery vs. buffering for
/// file-association opens; labels are removed when their window is destroyed.
struct ReadyWindows(HashSet<String>);

/// The most recently focused document (non-settings) window. Menu events and
/// warm file-association opens are delivered here when the OS reports no
/// focused webview (all windows minimized, Settings focused, menu used while
/// the app is inactive) — never broadcast: a broadcast reaches every document
/// window and runs destructive actions (Undo, Delete, Open) in all of them.
struct ActiveWindow(Option<String>);

/// Snapshot slots assigned to not-yet-mounted recovery windows: `new_window`
/// with a `recover_slot` records `new label → slot` here, and the new
/// webview claims it at mount via `take_pending_recovery`.
struct PendingRecovery(HashMap<String, String>);

/// Snapshot slots holding crash work the user has not yet decided about —
/// seeded by `recovery_list` at startup, removed as slots are claimed or
/// discarded. While a window's own slot is protected (the user pressed
/// Escape on the recovery dialog: decide later), that window's autosaves
/// divert to a `<label>-deferred` slot and its post-save clear touches only
/// the deferred pair, so "the offer returns next launch" stays true even if
/// the user keeps working — the undecided snapshot is never overwritten.
struct ProtectedSlots(HashSet<String>);

/// The slot a window's own snapshot writes/clears should target: its label,
/// or the `<label>-deferred` side-slot while the label's own snapshot is
/// still protected (see [`ProtectedSlots`]).
fn effective_own_slot(app: &tauri::AppHandle, label: &str) -> String {
    let protected = app
        .state::<Mutex<ProtectedSlots>>()
        .lock()
        .map(|p| p.0.contains(label))
        .unwrap_or(false);
    if protected {
        format!("{label}-deferred")
    } else {
        label.to_string()
    }
}

/// Filesystem paths the user has approved through a real interaction — a
/// native dialog pick, a file-association open, a drag-drop, or the persisted
/// recents list (approved in a past session). The `read_file` / `write_file`
/// / `list_dir` commands refuse everything else, so a compromised webview
/// cannot use them as arbitrary file I/O: every reachable path traces back to
/// an explicit user gesture.
///
/// `read_dirs` approves a directory for listing and for reading its files
/// (and one subdirectory level below it) — requested by `pick_open_path`'s
/// `approve_dir` flag, which every import-format pick sets (their texture
/// scan legitimately reads siblings like `textures/wood.png`, though only
/// COLLADA's actually does) and which the unified Open dialog's pick also
/// sets unconditionally, since the picked extension isn't known until the
/// dialog resolves — so a plain `.hew` open now grants this too, unlike the
/// narrower `.hew`-only open path. A real but bounded widening (read-only,
/// scoped to a directory the user just navigated to via a native dialog);
/// tightening `pick_open_path` to gate this on the resolved path's own
/// extension, so a `.hew`/`.skp`/`.glb`/`.gltf`/`.stl` pick never grants it
/// at all, is a reasonable follow-up but changes this trust boundary and
/// hasn't been signed off.
#[derive(Default)]
struct ApprovedPaths {
    read_files: HashSet<PathBuf>,
    write_files: HashSet<PathBuf>,
    read_dirs: HashSet<PathBuf>,
}

/// Throttle + maximize bookkeeping for window-state persistence.
struct WindowStateCache {
    /// The most recent non-maximized geometry seen this session — what a
    /// quit-while-maximized should persist alongside `maximized: true`.
    last_normal: Option<WindowState>,
    /// When the state file was last written (Moved/Resized fire dozens of
    /// times per drag; unforced writes are throttled).
    last_write: Option<Instant>,
}

/// Handles to the stateful native menu items, so the webview can reflect its
/// state (active tool, Axes/Guides, open panes, selection-gated commands)
/// into the menu bar via `sync_menu_state`.
struct MenuHandles {
    checks: HashMap<String, CheckMenuItem<tauri::Wry>>,
    items: HashMap<String, MenuItem<tauri::Wry>>,
    /// The macOS app-menu items Tauri supplies (About, Hide, Quit, …),
    /// keyed `predefined:<name>`: they carry no id of their own, so
    /// `set_menu_locale` relabels them through these handles.
    predefined: Vec<(String, PredefinedMenuItem<tauri::Wry>)>,
}

/// In-memory desktop clipboard for Copy/Cut/Paste/Paste In Place (Lane D,
/// v1.1-cycle.md): raw `.hew` item bytes from `Scene::extract_item`, shared
/// across every open document window in this process so a copy in one
/// window pastes in another. Plain app state — never touches the OS
/// clipboard, cleared on quit. `None` until the first copy.
struct ClipboardState(Mutex<Option<Vec<u8>>>);

/// Store `bytes` as the shared desktop clipboard (Copy/Cut).
#[tauri::command]
fn clipboard_set(app: tauri::AppHandle, bytes: Vec<u8>) -> Result<(), String> {
    let state = app.state::<ClipboardState>();
    let mut guard = state.0.lock().map_err(|e| e.to_string())?;
    *guard = Some(bytes);
    Ok(())
}

/// Read the shared desktop clipboard (Paste/Paste In Place) — `None` if
/// nothing has been copied yet in this process.
#[tauri::command]
fn clipboard_get(app: tauri::AppHandle) -> Result<Option<Vec<u8>>, String> {
    let state = app.state::<ClipboardState>();
    let guard = state.0.lock().map_err(|e| e.to_string())?;
    Ok(guard.clone())
}

/// Monotonic counter for extra document windows ("main-2", "main-3", …).
struct WindowCounter(u32);

/// Path queued by `open_in_new_window` for a not-yet-mounted document
/// window: `new label -> path`. Claimed once, at mount, via
/// `take_pending_window_open` — the same one-shot shape as
/// [`PendingRecovery`], just for a plain open instead of a recovery slot.
struct PendingWindowOpen(HashMap<String, PendingWindowOpenEntry>);

/// Which `.hew` file each document window currently has open (label →
/// canonical path), reported by the webview through `set_document_path`
/// whenever its document session changes (open, Save As, New, Close). Read
/// by every open entry point so opening a path that is ALREADY open in some
/// window focuses that window instead of minting a second one — two windows
/// editing one file would be confusing at best and, at worst, two writers
/// that don't know about each other. Entries drop with their window.
struct DocumentPaths(HashMap<String, String>);

/// A comparable form of `path`: the filesystem's canonical path when it
/// resolves (symlinks, `..`, case-insensitive volumes), else the raw string
/// — good enough to keep two windows apart on a path that no longer exists.
fn canonical_document_path(path: &str) -> String {
    std::fs::canonicalize(path)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| path.to_string())
}

/// The label of the document window that has `path` open — or is ABOUT to:
/// a window minted by `open_in_new_window` whose queued path is still in
/// `PendingWindowOpen` (created, not yet mounted) counts too, so a second
/// open of the same file racing the first one's load focuses the window
/// already on its way instead of minting another. `as_copy` entries are a
/// fresh untitled document and never claim the file.
fn window_label_for_path(app: &tauri::AppHandle, path: &str) -> Option<String> {
    let wanted = canonical_document_path(path);
    {
        let state = app.state::<Mutex<DocumentPaths>>();
        let guard = state.lock().ok()?;
        if let Some((label, _)) = guard
            .0
            .iter()
            .find(|(label, p)| **p == wanted && app.get_webview_window(label).is_some())
        {
            return Some(label.clone());
        }
    }
    let state = app.state::<Mutex<PendingWindowOpen>>();
    let guard = state.lock().ok()?;
    guard
        .0
        .iter()
        .find(|(label, entry)| {
            !entry.as_copy
                && canonical_document_path(&entry.path) == wanted
                && app.get_webview_window(label).is_some()
        })
        .map(|(label, _)| label.clone())
}

/// Every document window's open `.hew` path (label → path), for the live
/// API's document list.
fn document_paths_snapshot(app: &tauri::AppHandle) -> HashMap<String, String> {
    app.state::<Mutex<DocumentPaths>>()
        .lock()
        .map(|g| g.0.clone())
        .unwrap_or_default()
}

/// Record that window `label` holds `path` (see [`DocumentPaths`]) — the
/// shell-side twin of `set_document_path`, for a window the live API opens.
fn claim_document_path(app: &tauri::AppHandle, label: &str, path: &str) {
    if let Ok(mut guard) = app.state::<Mutex<DocumentPaths>>().lock() {
        guard
            .0
            .insert(label.to_string(), canonical_document_path(path));
    }
}

/// Report whether this window's document has unsaved changes — the one
/// piece of document state the shell cannot see for itself, and the live
/// API's `hew.meta.documents` `dirty` field. The webview pushes it on every
/// change (App.tsx).
#[tauri::command]
fn set_document_dirty(app: tauri::AppHandle, window: tauri::WebviewWindow, dirty: bool) {
    if let Ok(mut docs) = app.state::<live::LiveApi>().docs.lock() {
        docs.set_dirty(window.label(), dirty);
    }
}

/// Record (or clear, with `None`) the `.hew` path this window has open —
/// see [`DocumentPaths`].
#[tauri::command]
fn set_document_path(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    path: Option<String>,
) -> Result<(), String> {
    let state = app.state::<Mutex<DocumentPaths>>();
    let mut guard = state.lock().map_err(|e| e.to_string())?;
    match path {
        Some(p) => {
            guard
                .0
                .insert(window.label().to_string(), canonical_document_path(&p));
        }
        None => {
            guard.0.remove(window.label());
        }
    }
    Ok(())
}

/// If some OTHER document window already has `path` open, raise and focus
/// it and return `true` — the caller then skips its own open. `false` when
/// no window has it, or when the only window that does is the CALLER
/// itself: a window minted for a queued open holds the claim on its own
/// path before it loads (`take_pending_window_open`), and asking "is this
/// open elsewhere?" from that very window must not answer "yes, in you"
/// and abort the load it is about to do (the regression that left every
/// second-opened file as a blank Untitled window).
#[tauri::command]
fn focus_window_with_path(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    path: String,
) -> Result<bool, String> {
    match window_label_for_path(&app, &path) {
        Some(label) if label != window.label() => {
            focus_window_by_label(&app, &label)?;
            Ok(true)
        }
        _ => Ok(false),
    }
}

/// One queued open for a freshly created window: the path, whether the
/// window should treat it as a COPY (library "Open as Document" — import
/// semantics, Save always prompts) rather than a path-bound open, and the
/// display name a copy adopts.
#[derive(Clone, serde::Serialize)]
struct PendingWindowOpenEntry {
    path: String,
    as_copy: bool,
    name: Option<String>,
}

/// One entry in the Window menu's "open windows" tail, and in the
/// `window-list` event the in-app React menu bar (Windows/Linux) renders it
/// from.
#[derive(Clone, serde::Serialize)]
struct WindowInfo {
    label: String,
    title: String,
    focused: bool,
}

/// Handle to the native Window submenu's dynamic tail (the open-windows
/// list) plus how many entries it currently holds (a separator + one item
/// per window) — a rebuild removes exactly that many from the end before
/// appending the fresh set, leaving the static entries above (minimize /
/// maximize / pane toggles) undisturbed. Built on every platform (the
/// submenu itself exists everywhere, for menu parity — see the `set_menu`
/// note in `main()`), but only actually rebuilt into a visible native menu
/// on macOS; Windows/Linux read the same list from the `window-list`
/// broadcast instead.
// The fields' only reader, `rebuild_window_menu_tail`, is compiled on
// macOS alone, so everywhere else they are (intentionally) never read.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
struct WindowMenuState {
    submenu: tauri::menu::Submenu<tauri::Wry>,
    dynamic_count: usize,
}

// ---------------------------------------------------------------------------
// Window state — main-window size/position persisted across launches.
// First run (no state file): ~2/3 of the current monitor, centered.
// ---------------------------------------------------------------------------

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy)]
struct WindowState {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    maximized: bool,
}

fn window_state_path(app: &tauri::AppHandle) -> Option<std::path::PathBuf> {
    app.path()
        .app_config_dir()
        .ok()
        .map(|d| d.join("window-state.json"))
}

fn load_window_state(app: &tauri::AppHandle) -> Option<WindowState> {
    let text = std::fs::read_to_string(window_state_path(app)?).ok()?;
    serde_json::from_str(&text).ok()
}

/// Write `text` to `path` atomically (temp file + rename), so a crash
/// mid-write can never leave a half-written file — the reader sees either
/// the old content or the new, never a torn one.
fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension(match path.extension().and_then(|e| e.to_str()) {
        Some(ext) => format!("{ext}.tmp"),
        None => "tmp".to_string(),
    });
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

/// Minimum interval between unforced window-state writes; Moved/Resized fire
/// continuously during a drag, and each write is synchronous on the event
/// loop. The trailing edge is covered by a forced write on
/// CloseRequested/ExitRequested.
const WINDOW_STATE_WRITE_INTERVAL_MS: u128 = 500;

/// Capture the window's current logical geometry and persist it. Logical
/// (scale-independent) units keep the state portable across DPI changes.
/// While maximized, the last non-maximized geometry seen this session (or the
/// prior state file) is persisted with `maximized: true`, so both a
/// quit-while-maximized and a maximize-on-first-run restore sensibly.
/// Unforced calls are throttled; `force` (window close / app exit) always
/// writes so the final geometry is never lost to the throttle.
fn save_window_state(app: &tauri::AppHandle, window: &tauri::Window, force: bool) {
    let Some(path) = window_state_path(app) else {
        return;
    };
    let cache = app.state::<Mutex<WindowStateCache>>();
    let Ok(mut cache) = cache.lock() else { return };
    if !force {
        if let Some(last) = cache.last_write {
            if last.elapsed().as_millis() < WINDOW_STATE_WRITE_INTERVAL_MS {
                return;
            }
        }
    }
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let maximized = window.is_maximized().unwrap_or(false);
    let state = if maximized {
        // Keep the restore geometry from this session, or the prior file;
        // with neither (maximized since the very first event), fall back to
        // the first-run default so the flag itself still persists.
        let mut prev = cache
            .last_normal
            .or_else(|| load_window_state(app))
            .unwrap_or_else(|| default_window_state(window));
        prev.maximized = true;
        prev
    } else {
        let scale = window.scale_factor().unwrap_or(1.0);
        let (Ok(size), Ok(pos)) = (window.inner_size(), window.outer_position()) else {
            return;
        };
        // Ignore the degenerate geometry some platforms report mid-teardown.
        if size.width == 0 || size.height == 0 {
            return;
        }
        let state = WindowState {
            x: f64::from(pos.x) / scale,
            y: f64::from(pos.y) / scale,
            width: f64::from(size.width) / scale,
            height: f64::from(size.height) / scale,
            maximized: false,
        };
        cache.last_normal = Some(state);
        state
    };
    if let Ok(text) = serde_json::to_string(&state) {
        if write_atomic(&path, text.as_bytes()).is_ok() {
            cache.last_write = Some(Instant::now());
        }
    }
}

/// The first-run default geometry: ~2/3 of the window's current monitor,
/// centered — proportional to the screen instead of a fixed 1280x800.
fn default_window_state(window: &tauri::Window) -> WindowState {
    if let Ok(Some(monitor)) = window.current_monitor() {
        let scale = monitor.scale_factor();
        let mw = f64::from(monitor.size().width) / scale;
        let mh = f64::from(monitor.size().height) / scale;
        let mx = f64::from(monitor.position().x) / scale;
        let my = f64::from(monitor.position().y) / scale;
        let width = mw * (2.0 / 3.0);
        let height = mh * (2.0 / 3.0);
        return WindowState {
            x: mx + (mw - width) / 2.0,
            y: my + (mh - height) / 2.0,
            width,
            height,
            maximized: false,
        };
    }
    WindowState {
        x: 100.0,
        y: 100.0,
        width: 1280.0,
        height: 800.0,
        maximized: false,
    }
}

/// 1-D overlap length of `[a0, a1)` and `[b0, b1)`, 0 when disjoint.
fn overlap(a0: i32, a1: i32, b0: i32, b1: i32) -> i32 {
    (a1.min(b1) - a0.max(b0)).max(0)
}

/// Whether enough of the window's top (title/drag) strip is visible on some
/// monitor to grab it with the mouse. `current_monitor()` alone cannot answer
/// this: on Windows it maps to `MONITOR_DEFAULTTONEAREST`, which always
/// returns a monitor even for fully off-screen coordinates, and a window
/// hanging almost entirely off one edge still "has" a monitor everywhere.
fn window_reachably_on_screen(window: &tauri::WebviewWindow) -> bool {
    let (Ok(pos), Ok(size)) = (window.outer_position(), window.outer_size()) else {
        return true; // can't tell — don't fight the WM
    };
    let Ok(monitors) = window.available_monitors() else {
        return true;
    };
    let scale = window.scale_factor().unwrap_or(1.0);
    // The window is grabbable when ≥100x30 logical px of its top 64-px strip
    // land on a monitor (the drag region on the borderless shells, the title
    // bar on macOS).
    let strip_h = (64.0 * scale) as i32;
    let need_w = (100.0 * scale) as i32;
    let need_h = (30.0 * scale) as i32;
    for m in monitors {
        let mp = m.position();
        let ms = m.size();
        let vis_w = overlap(
            pos.x,
            pos.x + size.width as i32,
            mp.x,
            mp.x + ms.width as i32,
        );
        let vis_h = overlap(pos.y, pos.y + strip_h, mp.y, mp.y + ms.height as i32);
        if vis_w >= need_w && vis_h >= need_h.min(strip_h) {
            return true;
        }
    }
    false
}

/// Apply persisted state to the main window, or fall back to ~2/3 of the
/// current monitor, centered — a first-run default proportional to the
/// screen instead of a fixed 1280x800.
fn apply_initial_window_state(app: &tauri::AppHandle, window: &tauri::WebviewWindow) {
    if let Some(state) = load_window_state(app) {
        if state.width >= 400.0 && state.height >= 300.0 {
            let _ = window.set_size(tauri::LogicalSize::new(state.width, state.height));
            let _ = window.set_position(tauri::LogicalPosition::new(state.x, state.y));
            // If the saved position no longer leaves a grabbable strip on any
            // monitor (display unplugged, resolution change, or a mostly
            // off-screen drag persisted), pull the window back into view.
            if !window_reachably_on_screen(window) {
                let _ = window.center();
            }
            if state.maximized {
                let _ = window.maximize();
            }
            return;
        }
    }
    if let Ok(Some(monitor)) = window.current_monitor() {
        let scale = monitor.scale_factor();
        let width = f64::from(monitor.size().width) / scale * (2.0 / 3.0);
        let height = f64::from(monitor.size().height) / scale * (2.0 / 3.0);
        let _ = window.set_size(tauri::LogicalSize::new(width, height));
        let _ = window.center();
    }
}

/// Sort key for document window labels ("main", "main-2", "main-3", …) in
/// creation order — a lexical sort would put "main-10" before "main-2".
fn window_sort_key(label: &str) -> u32 {
    if label == "main" {
        0
    } else {
        label
            .strip_prefix("main-")
            .and_then(|s| s.parse().ok())
            .unwrap_or(u32::MAX)
    }
}

/// Strip Hew's constant " — Hew" title suffix (see `window_list`'s doc
/// comment) for display in the Window-menu list.
fn strip_hew_suffix(raw: &str) -> String {
    raw.strip_suffix(" — Hew").unwrap_or(raw).to_string()
}

/// A window's title as just applied via `window.set_title`, not yet
/// reflected by that same window's `w.title()` getter — see
/// `refresh_window_list_after_retitle`'s doc comment for why the two can
/// disagree. A named struct rather than a bare `(&str, &str)` tuple so a
/// `label`/`title` argument-order mistake at a call site is a compile-time
/// field-name mismatch, not a silent bug an override can mask.
#[derive(Clone, Copy)]
struct Retitled<'a> {
    label: &'a str,
    title: &'a str,
}

/// Resolve the display title for the window `label`, whose OS-reported
/// title is `os_title` — `retitled`, when it names this same `label`,
/// wins over `os_title` unconditionally. Pure and unit-tested directly
/// (see the `tests` module below); `window_list_impl` is its only caller
/// and needs a live `AppHandle` to get an `os_title` to pass in, but the
/// override-vs-OS-value decision itself needs neither.
fn resolved_window_title(label: &str, os_title: &str, retitled: Option<Retitled>) -> String {
    match retitled {
        Some(r) if r.label == label => strip_hew_suffix(r.title),
        _ => strip_hew_suffix(os_title),
    }
}

/// Snapshot of every open document window (Settings excluded — it manages no
/// document and was never meant to appear in a document-window list), in
/// creation order. `title` is whatever the webview last pushed via
/// `set_window_title` (Hew's "<mark><name> — Hew" format, `deriveTitle` in
/// App.tsx); stripping the constant " — Hew" suffix here keeps the
/// Window-menu list — already inside the Hew app — from repeating the app
/// name on every line. Purely cosmetic: if `deriveTitle`'s format ever
/// changes this stops matching and the raw title shows instead, so nothing
/// breaks, it just gets a little more verbose.
///
/// `retitled` overrides one window's title with a value the caller already
/// knows to be authoritative rather than reading it back from `w.title()` —
/// see `refresh_window_list_after_retitle`'s doc comment for why that
/// matters right after a `set_title` call specifically.
fn window_list(app: &tauri::AppHandle) -> Vec<WindowInfo> {
    window_list_impl(app, None)
}

fn window_list_impl(app: &tauri::AppHandle, retitled: Option<Retitled>) -> Vec<WindowInfo> {
    let mut list: Vec<WindowInfo> = app
        .webview_windows()
        .values()
        .filter(|w| is_document_window_label(w.label()))
        .map(|w| {
            let os_title = w.title().unwrap_or_default();
            let title = resolved_window_title(w.label(), &os_title, retitled);
            WindowInfo {
                label: w.label().to_string(),
                title,
                focused: w.is_focused().unwrap_or(false),
            }
        })
        .collect();
    list.sort_by_key(|w| window_sort_key(&w.label));
    list
}

/// Rebuild the native Window submenu's dynamic tail from `list` (macOS only
/// — see [`WindowMenuState`]). Each entry's id is `win-focus:<label>`,
/// dispatched in `on_menu_event` below; the focused window's entry is
/// checked.
#[cfg(target_os = "macos")]
fn rebuild_window_menu_tail(app: &tauri::AppHandle, list: &[WindowInfo]) {
    let Some(state) = app.try_state::<Mutex<WindowMenuState>>() else {
        return;
    };
    let Ok(mut guard) = state.lock() else { return };
    for _ in 0..guard.dynamic_count {
        let last = match guard.submenu.items() {
            Ok(items) if !items.is_empty() => items.len() - 1,
            _ => break,
        };
        let _ = guard.submenu.remove_at(last);
    }
    guard.dynamic_count = 0;
    if list.is_empty() {
        return;
    }
    if let Ok(sep) = PredefinedMenuItem::separator(app) {
        if guard.submenu.append(&sep).is_ok() {
            guard.dynamic_count += 1;
        }
    }
    let labels = current_menu_labels(app);
    for w in list {
        let label = if w.title.is_empty() {
            labels.get("fallback:untitled").to_string()
        } else {
            w.title.clone()
        };
        if let Ok(item) = CheckMenuItemBuilder::with_id(format!("win-focus:{}", w.label), label)
            .checked(w.focused)
            .build(app)
        {
            if guard.submenu.append(&item).is_ok() {
                guard.dynamic_count += 1;
            }
        }
    }
}

/// Refresh every observer of the open-window list after a window is created,
/// destroyed, focused, or retitled: rebuild the native Window menu's tail
/// (macOS) and broadcast the list to every webview (all platforms — the
/// in-app React menu bar on Windows/Linux renders it from here). A plain
/// `emit` (not `emit_to_active`) is correct: the list is identical for every
/// window and reading it is never destructive, unlike the targeted commands
/// `emit_to_active` guards.
///
/// The macOS rebuild is dispatched via `run_on_main_thread` rather than
/// called directly, and deliberately does NOT block waiting for it to run.
/// `create_document_window` calls this from an async command, which Tauri
/// runs off the main thread (see that function's doc comment); every
/// `Submenu` mutation (`items`/`remove_at`/`append`, inside
/// `rebuild_window_menu_tail`) blocks its calling thread on a main-thread
/// round trip. Locking `WindowMenuState` for that whole round trip from a
/// background thread — while `WindowEvent::Focused`/`Destroyed` can lock the
/// very same mutex synchronously ON the main thread at any moment — is a
/// real deadlock: the main thread can end up blocked acquiring the mutex
/// inside its own event dispatch, which is the only place that could ever
/// service the background thread's queued main-thread task, so neither side
/// can proceed. Posting the whole rebuild (lock included) as one
/// `run_on_main_thread` closure keeps the mutex main-thread-only in
/// practice: `run_on_main_thread` itself never blocks its caller (it either
/// runs synchronously in place when already on the main thread — the
/// Focused/Destroyed/set_window_title call sites — or posts the closure and
/// returns immediately otherwise), so it can never be the far end of a
/// cross-thread wait cycle.
///
/// The closure re-fetches `window_list(app)` itself rather than closing over
/// the snapshot taken above: back-to-back calls to this function (e.g. a
/// window's `Focused` handler firing moments after `create_document_window`
/// posts its own rebuild) can have their `run_on_main_thread` closures
/// execute out of POST order — a call already on the main thread runs its
/// closure synchronously in place, while a call from a background thread
/// only gets its closure queued, so an EARLIER call's rebuild can execute
/// AFTER a LATER call's. Rebuilding from a snapshot captured at post time
/// would let that stale, earlier snapshot silently overwrite the menu with
/// out-of-date window titles/focus state; re-fetching inside the closure
/// means whichever rebuild runs last always reflects the true state at that
/// moment, regardless of post order.
///
/// `retitled`, when set, is threaded through every re-fetch inside this call
/// (the broadcast above and the macOS closure's own re-fetch) — see
/// `refresh_window_list_after_retitle`'s doc comment for why a freshly-set
/// title can't just be read back from the window.
fn refresh_window_list(app: &tauri::AppHandle) {
    refresh_window_list_impl(app, None);
}

fn refresh_window_list_impl(app: &tauri::AppHandle, retitled: Option<Retitled>) {
    let list = window_list_impl(app, retitled);
    let _ = app.emit("window-list", &list);
    #[cfg(target_os = "macos")]
    {
        let app_for_closure = app.clone();
        let retitled_for_closure = retitled.map(|r| (r.label.to_string(), r.title.to_string()));
        let _ = app.run_on_main_thread(move || {
            let retitled_ref = retitled_for_closure
                .as_ref()
                .map(|(label, title)| Retitled { label, title });
            let list = window_list_impl(&app_for_closure, retitled_ref);
            rebuild_window_menu_tail(&app_for_closure, &list);
        });
    }
}

/// `refresh_window_list`, but for the window that was JUST retitled via
/// `window.set_title(title)` (called immediately before this) — the only
/// caller that needs it. On macOS, `tao::window::Window::set_title` never
/// applies the OS-level title change synchronously, even when already
/// called from the main thread: it unconditionally marshals through
/// `DispatchQueue::main().exec_async` (`tao`'s
/// `platform_impl::macos::util::set_title_async`), which only enqueues the
/// real `NSWindow.setTitle:` call on the main dispatch queue — it runs once
/// this whole command handler returns control to the run loop, not before.
/// `set_window_title`'s own command handler *does* run on the main thread
/// (it is a non-async `#[tauri::command]`), so re-reading this window's
/// title via `w.title()` in the very same call — exactly what a plain
/// `refresh_window_list(app)` would do — reads back the value from BEFORE
/// this call, not the one just set: a real ordering race, not a threading
/// one. It self-corrects on the next window-list refresh (any focus/
/// create/destroy event re-reads the by-then-actually-applied OS title),
/// which is why the native Window-menu entry and the broadcast list both
/// showed the stale title until an unrelated focus switch forced another
/// rebuild. Passing the just-set title through as `retitled` sidesteps the
/// race entirely instead of trying to wait for the queued dispatch: nothing
/// downstream needs to ask the OS for a value the caller already knows.
///
/// Takes a `Retitled` (named fields) rather than two bare `&str` arguments
/// on purpose: `set_window_title` is this function's only caller, and a
/// `label`/`title` argument-order slip at that one call site is exactly the
/// kind of mistake that would silently defeat the whole fix (the wrong
/// window would show the wrong title) without failing any test that only
/// exercises `resolved_window_title` in isolation — see the `tests` module.
/// Named fields turn that slip into a call site that reads
/// `Retitled { label: title, title: label }`, which is a lot harder to
/// write by accident.
fn refresh_window_list_after_retitle(app: &tauri::AppHandle, retitled: Retitled) {
    refresh_window_list_impl(app, Some(retitled));
}

/// Put `label`'s window back in front and give it key focus WITHOUT
/// unminimizing it — what the live API does for the user's window after
/// creating a background one where there is no `order_window_behind`.
/// A minimized window stays minimized.
#[cfg(not(target_os = "macos"))]
fn refocus_window(app: &tauri::AppHandle, label: &str) -> Result<(), String> {
    match app.get_webview_window(label) {
        Some(w) if !w.is_minimized().unwrap_or(false) => w.set_focus().map_err(|e| e.to_string()),
        _ => Ok(()),
    }
}

/// Slot `label`'s window directly BEHIND `anchor`'s in the window order —
/// without activating the app, taking key status, or moving anything else
/// — so a window the live API creates for background work never covers the
/// one the user is using, whether or not Hew is the active app. macOS only
/// (`orderWindow:relativeTo:` is exactly this operation); AppKit insists on
/// the main thread, hence the hop. Elsewhere there is no such primitive, and
/// the caller falls back to [`refocus_window`].
#[cfg(target_os = "macos")]
fn order_window_behind(app: &tauri::AppHandle, label: &str, anchor: &str) {
    use objc2_app_kit::{NSWindow, NSWindowOrderingMode};
    let handle = app.clone();
    let (label, anchor) = (label.to_string(), anchor.to_string());
    let _ = app.run_on_main_thread(move || {
        let (Some(win), Some(anchor_win)) = (
            handle.get_webview_window(&label),
            handle.get_webview_window(&anchor),
        ) else {
            return;
        };
        let (Ok(win_ptr), Ok(anchor_ptr)) = (win.ns_window(), anchor_win.ns_window()) else {
            return;
        };
        if win_ptr.is_null() || anchor_ptr.is_null() {
            return;
        }
        // SAFETY: both pointers are live NSWindows owned by their Tauri
        // windows, which `handle` keeps alive for this call, and this
        // closure runs on the main thread.
        unsafe {
            let win: &NSWindow = &*(win_ptr as *const NSWindow);
            let anchor: &NSWindow = &*(anchor_ptr as *const NSWindow);
            win.orderWindow_relativeTo(NSWindowOrderingMode::Below, anchor.windowNumber());
        }
    });
}

/// Raise and focus the document window with the given label. Shared by the
/// `focus_window` command (the in-app React Window-menu tail) and the native
/// macOS Window-menu tail's own `win-focus:<label>` dispatch.
fn focus_window_by_label(app: &tauri::AppHandle, label: &str) -> Result<(), String> {
    if let Some(w) = app.get_webview_window(label) {
        let _ = w.unminimize();
        w.set_focus().map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Focus (raise) an open document window by label — the in-app React
/// Window-menu tail's click handler (Windows/Linux/web multi-window).
#[tauri::command]
fn focus_window(app: tauri::AppHandle, label: String) -> Result<(), String> {
    focus_window_by_label(&app, &label)
}

/// Return the current open-window list. Called once by a freshly-mounted
/// webview for its initial state; the `window-list` broadcast
/// (`refresh_window_list`) keeps it fresh after that.
#[tauri::command]
fn list_windows(app: tauri::AppHandle) -> Vec<WindowInfo> {
    window_list(&app)
}

/// Set this window's OS title and refresh every open-window observer — the
/// one path the webview uses to push its document title (App.tsx's
/// `deriveTitle` effect), replacing a bare Tauri `setTitle` call so the
/// Window-menu list (native and broadcast) never goes stale.
///
/// Refreshes via `refresh_window_list_after_retitle`, not a plain
/// `refresh_window_list(&app)` — see that function's doc comment for why a
/// freshly-set title can't be read back from `window` in this same call.
#[tauri::command]
fn set_window_title(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    title: String,
) -> Result<(), String> {
    window.set_title(&title).map_err(|e| e.to_string())?;
    refresh_window_list_after_retitle(
        &app,
        Retitled {
            label: window.label(),
            title: &title,
        },
    );
    Ok(())
}

/// Create a new document window cascaded from the calling window, returning
/// its label. Shared by `new_window` (File ▸ New / startup crash recovery)
/// and `open_in_new_window` (File ▸ Open onto a non-pristine window); with
/// `recover_slot`, the new window claims that recovery snapshot at mount
/// (via `take_pending_recovery`) instead of starting blank.
///
/// `async` is load-bearing, not style: on Windows, creating a webview inside a
/// synchronous command deadlocks WebView2 initialization (the sync command
/// holds the main thread the webview needs to finish creating itself — see the
/// "Known issues" note on `WebviewWindowBuilder::build`). The result is a
/// zombie window: a frame with no content whose close button does nothing.
/// An async command runs off the main thread, so the build can complete.
async fn create_document_window(
    app: &tauri::AppHandle,
    window: &tauri::WebviewWindow,
    recover_slot: Option<String>,
    focus: bool,
) -> Result<String, String> {
    let label = {
        let state = app.state::<Mutex<WindowCounter>>();
        let mut guard = state.lock().map_err(|e| e.to_string())?;
        guard.0 += 1;
        format!("main-{}", guard.0)
    };
    if let Some(slot) = recover_slot {
        let state = app.state::<Mutex<PendingRecovery>>();
        let mut guard = state.lock().map_err(|e| e.to_string())?;
        guard.0.insert(label.clone(), slot);
    }
    let scale = window.scale_factor().unwrap_or(1.0);
    let size = window.inner_size().map_err(|e| e.to_string())?;
    let pos = window.outer_position().map_err(|e| e.to_string())?;
    // Standard macOS document-window cascade offset.
    const CASCADE: f64 = 28.0;
    let builder =
        tauri::WebviewWindowBuilder::new(app, &label, tauri::WebviewUrl::App("index.html".into()))
            .title("Hew")
            .focused(focus)
            .inner_size(
                f64::from(size.width) / scale,
                f64::from(size.height) / scale,
            )
            .position(
                f64::from(pos.x) / scale + CASCADE,
                f64::from(pos.y) / scale + CASCADE,
            );
    // Linux/WebKitGTK draws its own chrome (see the setup() note); Windows and
    // macOS keep native decorations.
    #[cfg(target_os = "linux")]
    let builder = builder.decorations(false);
    builder.build().map_err(|e| e.to_string())?;
    refresh_window_list(app);
    Ok(label)
}

/// Open a new document window cascaded from the calling window — File → New
/// when the current model isn't blank, and startup crash recovery when more
/// than one snapshot exists. See [`create_document_window`].
#[tauri::command]
async fn new_window(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    recover_slot: Option<String>,
) -> Result<(), String> {
    create_document_window(&app, &window, recover_slot, true).await?;
    Ok(())
}

/// Open `path` in a brand-new document window rather than the calling one —
/// File ▸ Open (or a recent/dropped/associated path) onto a window that
/// isn't pristine. Mints the window via [`create_document_window`] and
/// queues the path for it in [`PendingWindowOpen`]; the new window claims it
/// at startup via `take_pending_window_open`, reusing the same ready
/// handshake `deliver_open` already established rather than re-deriving one.
#[tauri::command]
async fn open_in_new_window(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    path: String,
    as_copy: Option<bool>,
    name: Option<String>,
) -> Result<(), String> {
    // Already open somewhere (and not a Library "open as copy", which is a
    // fresh untitled document that merely STARTS from that file)? Focus that
    // window instead of opening the same file twice — see `DocumentPaths`.
    if !as_copy.unwrap_or(false) {
        if let Some(label) = window_label_for_path(&app, &path) {
            return focus_window_by_label(&app, &label);
        }
    }
    let label = create_document_window(&app, &window, None, true).await?;
    // Propagate a lock failure (matches the sibling recover_slot insert in
    // create_document_window) rather than swallowing it: on a poisoned
    // mutex, silently returning Ok would leave the freshly opened window
    // blank forever — take_pending_window_open would find nothing to
    // deliver, and the caller would have no idea the open never happened.
    let state = app.state::<Mutex<PendingWindowOpen>>();
    let mut guard = state.lock().map_err(|e| e.to_string())?;
    guard.0.insert(
        label,
        PendingWindowOpenEntry {
            path,
            as_copy: as_copy.unwrap_or(false),
            name,
        },
    );
    Ok(())
}

/// Claim (remove) the path queued by `open_in_new_window` for this window, if
/// any — called once at startup by every window, mirroring
/// `take_pending_recovery`. A window not created via `open_in_new_window`
/// (the initial `main` window, a File ▸ New window, a recovery window) never
/// has an entry and this is a no-op `None`.
#[tauri::command]
fn take_pending_window_open(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> Option<PendingWindowOpenEntry> {
    let entry = app
        .state::<Mutex<PendingWindowOpen>>()
        .lock()
        .ok()
        .and_then(|mut guard| guard.0.remove(window.label()))?;
    // Carry the claim over: from here until the webview reports its loaded
    // document, this window is the one opening `path` (see `DocumentPaths`
    // and `window_label_for_path`). An open-as-copy claims nothing.
    if !entry.as_copy {
        if let Ok(mut paths) = app.state::<Mutex<DocumentPaths>>().lock() {
            paths.0.insert(
                window.label().to_string(),
                canonical_document_path(&entry.path),
            );
        }
    }
    Some(entry)
}

/// The label table the menu bar currently shows (English if the state is
/// somehow unavailable), for code that rebuilds part of the menu at runtime.
fn current_menu_labels(app: &tauri::AppHandle) -> MenuLabels {
    app.try_state::<Mutex<MenuLabels>>()
        .and_then(|s| s.lock().ok().map(|g| g.clone()))
        .unwrap_or_else(|| MenuLabels::for_locale("en"))
}

/// Relabel every id-bearing item under `items` from `labels`, recursing
/// into submenus. Ids with no label entry (`recent:<path>` file names, the
/// dynamic undo/redo text the frontend pushes) are left alone.
fn relabel_menu_items(items: Vec<MenuItemKind<tauri::Wry>>, labels: &MenuLabels) {
    for item in items {
        match item {
            MenuItemKind::MenuItem(i) => {
                if let Some(text) = labels.lookup(i.id().as_ref()) {
                    let _ = i.set_text(text);
                }
            }
            MenuItemKind::Check(i) => {
                if let Some(text) = labels.lookup(i.id().as_ref()) {
                    let _ = i.set_text(text);
                }
            }
            MenuItemKind::Submenu(sub) => {
                if let Some(text) = labels.lookup(sub.id().as_ref()) {
                    let _ = sub.set_text(text);
                }
                if let Ok(children) = sub.items() {
                    relabel_menu_items(children, labels);
                }
            }
            MenuItemKind::Predefined(_) | MenuItemKind::Icon(_) => {}
        }
    }
}

/// Switch the native menu bar to `locale`'s labels in place — no rebuild,
/// so ids, accelerators, check states, and enabled states all survive —
/// and remember the locale so the next launch starts in it
/// (docs/LOCALIZATION.md "Native menus"). An unshipped locale shows
/// English. The frontend calls this after activating a catalog; every
/// window does, so repeated calls with the same locale must be cheap and
/// harmless.
#[tauri::command]
fn set_menu_locale(app: tauri::AppHandle, locale: String) -> Result<(), String> {
    let labels = MenuLabels::for_locale(&locale);
    if let Ok(config_dir) = app.path().app_config_dir() {
        menu_labels::write_persisted_locale(&config_dir, labels.locale());
    }
    let already = app
        .try_state::<Mutex<MenuLabels>>()
        .and_then(|s| s.lock().ok().map(|g| g.locale() == labels.locale()))
        .unwrap_or(false);
    if !already {
        if let Some(menu) = app.menu() {
            let items = menu.items().map_err(|e| e.to_string())?;
            relabel_menu_items(items, &labels);
        }
        if let Some(state) = app.try_state::<Mutex<MenuHandles>>() {
            if let Ok(guard) = state.lock() {
                for (key, item) in &guard.predefined {
                    let _ = item.set_text(labels.get(key));
                }
            }
        }
        // The shell-built tool windows carry fixed titles; the document
        // windows retitle themselves from the webview.
        if let Some(window) = app.get_webview_window("settings") {
            let _ = window.set_title(labels.get("window:settings"));
        }
        if let Some(window) = app.get_webview_window("library") {
            let _ = window.set_title(labels.get("win-library"));
        }
        if let Some(state) = app.try_state::<Mutex<MenuLabels>>() {
            if let Ok(mut guard) = state.lock() {
                *guard = labels;
            }
        }
        // The relabel above reset Sent Reports to its plain form; put the
        // unread count back (in the new language).
        follow::refresh_menu_label(&app);
    }
    Ok(())
}

/// Reflect webview state into the native menu bar: `checked` drives check
/// marks (active tool radio group, Axes/Guides, open panes); `enabled` gates
/// selection-dependent items (Group, Explode, …). Unknown ids are ignored so
/// the webview can stay ahead of the shell.
///
/// There is one menu bar and many document windows, so only the focused
/// window's push may win — a background window whose state changes (or whose
/// sync races a focus switch) must not repaint the foreground window's menu.
/// Pushes are accepted from an unfocused window only while NO document window
/// has focus (the startup sync, before the first focus event lands).
#[tauri::command]
fn sync_menu_state(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    checked: HashMap<String, bool>,
    enabled: HashMap<String, bool>,
    // Lane C (docs/design/v1.1-cycle.md): "Undo Push/Pull" / "Redo Move" —
    // the top undo/redo entry's label, derived from `Scene::
    // history_entries_json`. Reaches only items registered in `guard.items`
    // (edit-undo/edit-redo, via `gated_item` — see their construction
    // above) since `MenuItem::set_text` isn't offered on a `CheckMenuItem`.
    text: HashMap<String, String>,
) -> Result<(), String> {
    if !window.is_focused().unwrap_or(false) {
        let any_doc_focused = app
            .webview_windows()
            .into_values()
            .any(|w| is_document_window_label(w.label()) && w.is_focused().unwrap_or(false));
        // With no document window focused, accept only the window menu
        // events would be routed to (the last-focused one) — otherwise any
        // background window's async state change repaints the shared menu
        // for a window that won't receive the resulting actions. Before any
        // focus event has landed (startup), every push is welcome.
        let is_menu_target = app
            .state::<Mutex<ActiveWindow>>()
            .lock()
            .map(|a| match a.0.as_deref() {
                Some(label) => label == window.label(),
                None => true,
            })
            .unwrap_or(false);
        if any_doc_focused || !is_menu_target {
            return Ok(());
        }
    }
    let state = app.state::<Mutex<MenuHandles>>();
    let guard = state.lock().map_err(|e| e.to_string())?;
    for (id, value) in &checked {
        if let Some(item) = guard.checks.get(id) {
            let _ = item.set_checked(*value);
        }
    }
    for (id, value) in &enabled {
        if let Some(item) = guard.checks.get(id) {
            let _ = item.set_enabled(*value);
        } else if let Some(item) = guard.items.get(id) {
            let _ = item.set_enabled(*value);
        }
    }
    for (id, value) in &text {
        if let Some(item) = guard.items.get(id) {
            let _ = item.set_text(value);
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Autosave / crash-recovery store.
//
// One snapshot pair per document window in the app config dir:
// recovery-<label>.hew (geometry bytes, the same format scene.save()
// produces) and recovery-<label>.json (the RecoveryMeta JSON verbatim,
// opaque to Rust — we never parse it here). The un-suffixed
// "recovery.hew"/"recovery.json" pair is the pre-multi-window legacy name,
// listed under the reserved slot name "legacy".
//
// Lifecycle: `recovery_list` enumerates every snapshot at startup (the
// `main` window offers them all — with N crashed documents all N are
// recoverable, one per window); `recovery_claim` re-homes a snapshot to the
// claiming window by RENAMING its files (so it is never shadowed or
// overwritten by the claimer's next autosave — that autosave now targets the
// same slot); `recovery_clear` drops only the calling window's own snapshot
// (after a successful save — one window's save must never destroy a sibling
// window's snapshot); `recovery_discard_all` is the startup dialog's
// explicit "Discard All".
// ---------------------------------------------------------------------------

/// Reserved slot name for the legacy un-suffixed snapshot pair.
const LEGACY_SLOT: &str = "legacy";

/// The `.hew`/`.json` file pair backing a snapshot slot. Pure path mapping —
/// no filesystem access (unit-tested below).
fn slot_paths(config_dir: &Path, slot: &str) -> (PathBuf, PathBuf) {
    if slot == LEGACY_SLOT {
        (
            config_dir.join("recovery.hew"),
            config_dir.join("recovery.json"),
        )
    } else {
        (
            config_dir.join(format!("recovery-{slot}.hew")),
            config_dir.join(format!("recovery-{slot}.json")),
        )
    }
}

/// The `.skpsrc` companion of a snapshot slot: the `.skp` source bytes a
/// SketchUp document saves back onto (see the `Scene` source in wasm-api).
/// Written only when it changes, so unlike the pair it is not part of every
/// autosave. Pure path mapping, derived from the slot's `.hew` path.
fn slot_source_path(hew_path: &Path) -> PathBuf {
    hew_path.with_extension("skpsrc")
}

/// The slot name for a snapshot `.hew` filename, or None for non-snapshots.
/// Pure (unit-tested below).
fn slot_of_filename(name: &str) -> Option<String> {
    if name == "recovery.hew" {
        return Some(LEGACY_SLOT.to_string());
    }
    let label = name.strip_prefix("recovery-")?.strip_suffix(".hew")?;
    // Never let a stray file alias the reserved legacy slot.
    if label.is_empty() || label == LEGACY_SLOT {
        return None;
    }
    Some(label.to_string())
}

/// The numeric suffix a document-window slot name implies for the window
/// counter: "main" → 1, "main-7" / "main-7-deferred" → 7, anything else →
/// None. Pure (unit-tested below). Used to seed [`WindowCounter`] past every
/// slot left by a crashed session, so a fresh window's label can never alias
/// an unclaimed snapshot slot — `recovery_claim` renames into the caller's
/// own slot, and an aliased label would make that rename destroy the
/// unclaimed sibling snapshot.
fn main_slot_suffix(slot: &str) -> Option<u32> {
    if slot == "main" {
        return Some(1);
    }
    let rest = slot.strip_prefix("main-")?;
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() {
        return None;
    }
    digits.parse().ok()
}

/// The highest window-counter value implied by snapshot slots on disk (≥ 1).
fn seed_window_counter(config_dir: Option<&Path>) -> u32 {
    let mut max = 1;
    let Some(dir) = config_dir else { return max };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return max;
    };
    for entry in entries.flatten() {
        let Some(name) = entry.file_name().to_str().map(String::from) else {
            continue;
        };
        if let Some(n) = slot_of_filename(&name)
            .as_deref()
            .and_then(main_slot_suffix)
        {
            max = max.max(n);
        }
    }
    max
}

/// One recoverable snapshot as listed at startup. `meta` is the RecoveryMeta
/// JSON when its sidecar file is readable; a snapshot whose sidecar was lost
/// (crash between the pair's writes on an old build) is still listed — the
/// geometry is recoverable even if the display name is not.
#[derive(serde::Serialize)]
struct RecoveryEntry {
    slot: String,
    meta: Option<String>,
    modified_ms: u64,
}

/// Payload returned by `recovery_claim`.
#[derive(serde::Serialize)]
struct RecoveryPayload {
    contents: Vec<u8>,
    meta: Option<String>,
    modified_ms: u64,
}

/// Persist a recovery snapshot, overwriting any previous one from the same
/// window. Snapshots are namespaced per window label so concurrent document
/// windows don't clobber each other's autosaves. Both files are written
/// atomically (temp + rename), sidecar first, so a crash mid-write — this
/// feature's operating condition — can never pair new bytes with stale meta:
/// the `.hew` rename is the commit point and its sidecar is already current.
#[tauri::command]
fn recovery_write(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    contents: Vec<u8>,
    meta: String,
) -> Result<(), String> {
    let Some(config_dir) = app.path().app_config_dir().ok() else {
        return Err("could not resolve app config dir".into());
    };
    std::fs::create_dir_all(&config_dir).map_err(|e| e.to_string())?;
    let slot = effective_own_slot(&app, window.label());
    let (hew_path, json_path) = slot_paths(&config_dir, &slot);
    write_atomic(&json_path, meta.as_bytes()).map_err(|e| e.to_string())?;
    write_atomic(&hew_path, &contents).map_err(|e| e.to_string())?;
    Ok(())
}

/// Persist the `.skp` source of the calling window's document beside its
/// snapshot (raw IPC body, atomic). Sources are tens to hundreds of MB, so
/// this is called only when the source changes, never per autosave.
#[tauri::command]
fn recovery_source_write(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    request: tauri::ipc::Request<'_>,
) -> Result<(), String> {
    let bytes = match request.body() {
        tauri::ipc::InvokeBody::Raw(bytes) => bytes,
        tauri::ipc::InvokeBody::Json(_) => {
            return Err("recovery_source_write expects a raw byte body".to_owned());
        }
    };
    let Some(config_dir) = app.path().app_config_dir().ok() else {
        return Err("could not resolve app config dir".into());
    };
    std::fs::create_dir_all(&config_dir).map_err(|e| e.to_string())?;
    let slot = effective_own_slot(&app, window.label());
    let (hew_path, _) = slot_paths(&config_dir, &slot);
    write_atomic(&slot_source_path(&hew_path), bytes).map_err(|e| e.to_string())
}

/// Remove the calling window's `.skp` source companion (no-op if none).
#[tauri::command]
fn recovery_source_clear(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> Result<(), String> {
    let Some(config_dir) = app.path().app_config_dir().ok() else {
        return Ok(());
    };
    let slot = effective_own_slot(&app, window.label());
    let (hew_path, _) = slot_paths(&config_dir, &slot);
    let _ = std::fs::remove_file(slot_source_path(&hew_path));
    Ok(())
}

/// Read the `.skp` source companion of `slot` as a raw response. Errors when
/// there is none (the caller treats that as "no source"). Called before
/// `recovery_claim`, which re-homes the slot's files.
#[tauri::command]
fn recovery_source_read(
    app: tauri::AppHandle,
    slot: String,
) -> Result<tauri::ipc::Response, String> {
    if slot.is_empty() || slot.contains(['/', '\\']) || slot.contains("..") {
        return Err(format!("recovery_source_read: invalid slot {slot:?}"));
    }
    let config_dir = app
        .path()
        .app_config_dir()
        .map_err(|_| "could not resolve app config dir".to_owned())?;
    let (hew_path, _) = slot_paths(&config_dir, &slot);
    let path = slot_source_path(&hew_path);
    let bytes =
        std::fs::read(&path).map_err(|e| format!("no recovery source for {slot:?}: {e}"))?;
    Ok(tauri::ipc::Response::new(bytes))
}

/// Enumerate every stored recovery snapshot (all window labels + legacy),
/// newest first. Called by the `main` window at startup to offer recovery of
/// every crashed document, not just the newest one.
#[tauri::command]
fn recovery_list(app: tauri::AppHandle) -> Vec<RecoveryEntry> {
    let Ok(config_dir) = app.path().app_config_dir() else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(&config_dir) else {
        return Vec::new();
    };
    let mut result: Vec<RecoveryEntry> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some(slot) = slot_of_filename(name) else {
            continue;
        };
        let modified_ms = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let meta = std::fs::read_to_string(path.with_extension("json")).ok();
        result.push(RecoveryEntry {
            slot,
            meta,
            modified_ms,
        });
    }
    result.sort_by_key(|e| std::cmp::Reverse(e.modified_ms));
    // Every listed slot is undecided until claimed or discarded — protect it
    // from being overwritten by its own label's future autosaves (Escape on
    // the dialog means "decide later", and the main window then starts a new
    // document under the same label).
    if let Ok(mut protected) = app.state::<Mutex<ProtectedSlots>>().lock() {
        for entry in &result {
            protected.0.insert(entry.slot.clone());
        }
    }
    result
}

/// Claim the snapshot in `slot` for the calling window: read it, then move
/// its file pair to the caller's own slot so the claimer's next autosave
/// overwrites what it adopted and the source ceases to exist. Reading comes
/// FIRST — an unreadable snapshot is left exactly where it was — and the
/// move is skipped entirely when the caller's own slot still holds someone
/// else's undecided snapshot (it must never be clobbered; the caller's
/// autosaves divert to the deferred side-slot in that case, so the adopted
/// document is still protected). Recovering a document whose meta carries an
/// original file path also approves that path for write, since Save on a
/// recovered document writes back to it without a new dialog.
#[tauri::command]
fn recovery_claim(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    slot: String,
) -> Option<RecoveryPayload> {
    let config_dir = app.path().app_config_dir().ok()?;
    let (src_hew, src_json) = slot_paths(&config_dir, &slot);
    let (dst_hew, dst_json) = slot_paths(&config_dir, window.label());
    let modified_ms = std::fs::metadata(&src_hew)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let contents = std::fs::read(&src_hew).ok()?;
    // The sidecar may be missing (torn write on an old build) — the
    // geometry alone is still worth recovering.
    let meta = std::fs::read_to_string(&src_json).ok();
    let mut own_slot_adopted = true;
    if src_hew != dst_hew {
        let dst_protected = app
            .state::<Mutex<ProtectedSlots>>()
            .lock()
            .map(|p| p.0.contains(window.label()))
            .unwrap_or(false);
        if dst_protected && dst_hew.exists() {
            // The caller's own slot holds another window's undecided
            // snapshot. Leave both slots on disk (the adopted document is
            // re-offered next launch if this session crashes before its
            // first autosave) and keep the label slot protected so the
            // caller's autosaves stay diverted to the deferred side-slot.
            own_slot_adopted = false;
        } else {
            let _ = std::fs::remove_file(&dst_hew);
            let _ = std::fs::remove_file(&dst_json);
            let _ = std::fs::remove_file(slot_source_path(&dst_hew));
            if std::fs::rename(&src_hew, &dst_hew).is_ok() {
                let _ = std::fs::rename(&src_json, &dst_json);
                let _ = std::fs::rename(slot_source_path(&src_hew), slot_source_path(&dst_hew));
            }
        }
    }
    // The claimed slot is decided; the caller's own label is releasable only
    // if it doesn't still shelter someone else's undecided snapshot.
    if let Ok(mut protected) = app.state::<Mutex<ProtectedSlots>>().lock() {
        protected.0.remove(&slot);
        if own_slot_adopted {
            protected.0.remove(window.label());
        }
    }
    // Save writes back to the recovered document's original path.
    if let Some(meta_json) = &meta {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(meta_json) {
            if let Some(path) = value.get("path").and_then(|p| p.as_str()) {
                approve_file(&app, Path::new(path), true);
            }
        }
    }
    Some(RecoveryPayload {
        contents,
        meta,
        modified_ms,
    })
}

/// Discard the calling window's own snapshot pair, ignoring not-found
/// errors. Called after a successful Save/Save As — scoped to the caller so
/// saving in one window can never destroy a sibling window's autosave of a
/// different, still-dirty document. While the caller's label slot is still
/// protected (an undecided crash snapshot), only the deferred side-slot is
/// cleared — a Save of the new document must not delete the old one.
#[tauri::command]
fn recovery_clear(app: tauri::AppHandle, window: tauri::WebviewWindow) -> Result<(), String> {
    clear_recovery_for_label(&app, window.label());
    Ok(())
}

/// [`recovery_clear`] by label — for the live API closing a window whose
/// webview no longer answers, so it cannot clear its own snapshot.
fn clear_recovery_for_label(app: &tauri::AppHandle, label: &str) {
    let Some(config_dir) = app.path().app_config_dir().ok() else {
        return;
    };
    let slot = effective_own_slot(app, label);
    let (hew_path, json_path) = slot_paths(&config_dir, &slot);
    let _ = std::fs::remove_file(slot_source_path(&hew_path));
    let _ = std::fs::remove_file(hew_path);
    let _ = std::fs::remove_file(json_path);
}

/// Discard every stored recovery snapshot (all window labels + legacy),
/// ignoring not-found errors. Only the startup recovery dialog's explicit
/// "Discard All" calls this — the dialog listed every snapshot, so the user
/// declined each of them by name.
#[tauri::command]
fn recovery_discard_all(app: tauri::AppHandle) -> Result<(), String> {
    let Some(config_dir) = app.path().app_config_dir().ok() else {
        return Ok(());
    };
    let Ok(entries) = std::fs::read_dir(&config_dir) else {
        return Ok(());
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let is_recovery = name == "recovery.hew"
            || name == "recovery.json"
            || (name.starts_with("recovery-")
                && (name.ends_with(".hew")
                    || name.ends_with(".json")
                    || name.ends_with(".skpsrc")));
        if is_recovery {
            let _ = std::fs::remove_file(&path);
        }
    }
    // Everything is gone; nothing is left to protect.
    if let Ok(mut protected) = app.state::<Mutex<ProtectedSlots>>().lock() {
        protected.0.clear();
    }
    Ok(())
}

/// Take the recovery slot assigned to this window by `new_window` — returns
/// Some once for a recovery window's mount, then None.
#[tauri::command]
fn take_pending_recovery(app: tauri::AppHandle, window: tauri::WebviewWindow) -> Option<String> {
    app.state::<Mutex<PendingRecovery>>()
        .lock()
        .ok()?
        .0
        .remove(window.label())
}

// ---------------------------------------------------------------------------
// Diagnostic log — rolling file (docs/dev/DEVELOPMENT.md).
//
// One file in the app log dir: diagnostic.log. On rotation (size cap
// exceeded), it's renamed to diagnostic.1.log (replacing any prior backup)
// and a fresh diagnostic.log is started — one backup kept, kept simple.
// ---------------------------------------------------------------------------

/// Size cap (bytes) before `log_rotate` rolls `diagnostic.log` to `diagnostic.1.log`.
const LOG_ROTATE_BYTES: u64 = 10 * 1024 * 1024;

/// Append `lines` (already-formatted NDJSON, newline-terminated) to the
/// rolling diagnostic log file, creating the log dir/file if needed.
#[tauri::command]
fn log_append(app: tauri::AppHandle, lines: String) -> Result<(), String> {
    let Some(log_dir) = app.path().app_log_dir().ok() else {
        return Err("could not resolve app log dir".into());
    };
    std::fs::create_dir_all(&log_dir).map_err(|e| e.to_string())?;
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_dir.join("diagnostic.log"))
        .map_err(|e| e.to_string())?;
    file.write_all(lines.as_bytes())
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Rotate `diagnostic.log` to `diagnostic.1.log` (replacing any prior backup)
/// if it has grown past `LOG_ROTATE_BYTES`. No-op if the file is missing or
/// under the cap.
#[tauri::command]
fn log_rotate(app: tauri::AppHandle) -> Result<(), String> {
    let Some(log_dir) = app.path().app_log_dir().ok() else {
        return Err("could not resolve app log dir".into());
    };
    let current = log_dir.join("diagnostic.log");
    let Ok(metadata) = std::fs::metadata(&current) else {
        return Ok(()); // nothing to rotate yet
    };
    if metadata.len() <= LOG_ROTATE_BYTES {
        return Ok(());
    }
    let backup = log_dir.join("diagnostic.1.log");
    let _ = std::fs::remove_file(&backup);
    std::fs::rename(&current, &backup).map_err(|e| e.to_string())?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Auto-reproducer dump (docs/dev/DEVELOPMENT.md) — on a failure, the app
// bundles {recorded command stream + serialized .hew + diagnostic-log tail}
// into one JSON file under the app log dir, so "it broke" becomes "here is a
// model + an input log that reproduces it". See app/src/log/reproducerDump.ts.
// ---------------------------------------------------------------------------

/// Write a reproducer bundle (`contents`, already-serialized JSON) to
/// `<app_log_dir>/reproducers/<name>`, creating the directory if needed.
/// Returns the absolute path. `name` is restricted to a plain filename (no
/// path separators or `..`) since it crosses the invoke boundary from the
/// webview.
#[tauri::command]
fn reproducer_write(
    app: tauri::AppHandle,
    name: String,
    contents: String,
) -> Result<String, String> {
    if name.is_empty() || name.contains(['/', '\\']) || name.contains("..") {
        return Err("invalid reproducer file name".into());
    }
    let Some(log_dir) = app.path().app_log_dir().ok() else {
        return Err("could not resolve app log dir".into());
    };
    let dir = log_dir.join("reproducers");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(&name);
    std::fs::write(&path, contents).map_err(|e| e.to_string())?;
    Ok(path.to_string_lossy().into_owned())
}

// ---------------------------------------------------------------------------
// Custom file-I/O commands, gated by the ApprovedPaths registry.
//
// These bypass the fs plugin so we need no fs capability entries — but
// app-defined commands also require no capability grant, which makes them
// reachable from ANY window's webview. Ungated, they would be arbitrary
// user-file read/write for a compromised webview; every path must therefore
// trace back to an explicit user gesture (dialog pick, file association,
// drag-drop, recents) recorded via the approve_* helpers.
// ---------------------------------------------------------------------------

/// Canonicalize a path for registry membership checks. Falls back to the
/// raw path when canonicalization fails (nonexistent file) — such paths can
/// still be approved for WRITE via their canonicalized parent + file name.
fn canonical_or_synthesized(path: &Path) -> Option<PathBuf> {
    if let Ok(canon) = std::fs::canonicalize(path) {
        return Some(canon);
    }
    let parent = std::fs::canonicalize(path.parent()?).ok()?;
    Some(parent.join(path.file_name()?))
}

/// Record read (and optionally write) approval for a user-designated file.
fn approve_file(app: &tauri::AppHandle, path: &Path, write: bool) {
    let Some(canon) = canonical_or_synthesized(path) else {
        return;
    };
    let state = app.state::<Mutex<ApprovedPaths>>();
    let Ok(mut approved) = state.lock() else {
        return;
    };
    approved.read_files.insert(canon.clone());
    if write {
        approved.write_files.insert(canon);
    }
}

/// Record read approval for a directory (listing + reading files in it and
/// one level below it). Called by `pick_open_path` whenever its caller sets
/// `approve_dir` — every import-format pick (whose COLLADA texture scan
/// reads siblings of the picked model and `textures/`-style subfolders) and,
/// since the unified Open dialog, every plain `.hew` pick too (see the
/// `ApprovedPaths::read_dirs` doc comment for why, and the open follow-up).
fn approve_dir_reads(app: &tauri::AppHandle, dir: &Path) {
    let Ok(canon) = std::fs::canonicalize(dir) else {
        return;
    };
    let state = app.state::<Mutex<ApprovedPaths>>();
    if let Ok(mut approved) = state.lock() {
        approved.read_dirs.insert(canon);
    };
}

/// Whether `path` may be read: itself approved (read or write), or inside an
/// approved directory (directly, or one subdirectory level below it).
fn read_allowed(app: &tauri::AppHandle, path: &Path) -> bool {
    let Ok(canon) = std::fs::canonicalize(path) else {
        return false;
    };
    let state = app.state::<Mutex<ApprovedPaths>>();
    let Ok(approved) = state.lock() else {
        return false;
    };
    if approved.read_files.contains(&canon) || approved.write_files.contains(&canon) {
        return true;
    }
    let mut dir = canon.parent();
    for _ in 0..2 {
        let Some(d) = dir else { break };
        if approved.read_dirs.contains(d) {
            return true;
        }
        dir = d.parent();
    }
    false
}

/// Whether `dir` may be listed: approved itself, or one level below an
/// approved directory (the import texture scan lists `<dir>/textures` etc.).
fn list_allowed(app: &tauri::AppHandle, dir: &Path) -> bool {
    let Ok(canon) = std::fs::canonicalize(dir) else {
        return false;
    };
    let state = app.state::<Mutex<ApprovedPaths>>();
    let Ok(approved) = state.lock() else {
        return false;
    };
    approved.read_dirs.contains(&canon)
        || canon
            .parent()
            .is_some_and(|p| approved.read_dirs.contains(p))
}

/// Whether `path` may be written: approved for write via a save dialog, an
/// open pick (Save writes back to the opened file), a file association,
/// drag-drop, or the recents list.
fn write_allowed(app: &tauri::AppHandle, path: &Path) -> bool {
    let Some(canon) = canonical_or_synthesized(path) else {
        return false;
    };
    let state = app.state::<Mutex<ApprovedPaths>>();
    let Ok(approved) = state.lock() else {
        return false;
    };
    approved.write_files.contains(&canon)
}

/// Read a user-approved file and return its raw bytes.
///
/// Returns the bytes as a raw IPC response (`tauri::ipc::Response`) rather than
/// a `Vec<u8>`: a `Vec<u8>` return is marshalled as a JSON array of numbers,
/// which for a multi-megabyte model is a huge JSON string that blocks the
/// webview main thread while it parses. A raw response transfers the bytes as
/// an `ArrayBuffer` with no JSON round-trip. `Err` is still serialized and
/// rejects the JS promise, preserving the error contract.
#[tauri::command]
fn read_file(app: tauri::AppHandle, path: String) -> Result<tauri::ipc::Response, String> {
    if !read_allowed(&app, Path::new(&path)) {
        return Err(format!("read_file: {path:?} is not a user-approved path"));
    }
    let bytes = std::fs::read(&path).map_err(|e| format!("read_file failed for {path:?}: {e}"))?;
    Ok(tauri::ipc::Response::new(bytes))
}

/// Write raw bytes to a user-approved file, creating or overwriting it.
#[tauri::command]
fn write_file(app: tauri::AppHandle, path: String, contents: Vec<u8>) -> Result<(), String> {
    if !write_allowed(&app, Path::new(&path)) {
        return Err(format!("write_file: {path:?} is not a user-approved path"));
    }
    std::fs::write(&path, &contents).map_err(|e| format!("write_file failed for {path:?}: {e}"))
}

/// Percent-decode (the inverse of JS's `encodeURIComponent`) a string carried
/// in an IPC request header. Header values reach Rust through the same
/// ASCII/Latin-1-only `Headers` model the browser Fetch API uses, so a path
/// containing any non-ASCII character would arrive silently mangled (or, for
/// most non-Latin-1 text, outright corrupted — JS's `Headers` coerces to a
/// "byte string" by truncating each UTF-16 code unit) if sent raw; the caller
/// percent-encodes it first, and this undoes that.
fn percent_decode(s: &str) -> Result<String, String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = s
                .get(i + 1..i + 3)
                .ok_or("percent_decode: truncated %XX escape")?;
            let byte = u8::from_str_radix(hex, 16)
                .map_err(|_| format!("percent_decode: invalid escape %{hex}"))?;
            out.push(byte);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).map_err(|_| "percent_decode: not valid UTF-8 after decoding".to_owned())
}

/// The raw-IPC-body twin of `write_file`, for a payload where `Vec<u8>` JSON
/// marshalling (roughly 4 bytes of IPC JSON per file byte — every byte
/// becomes a decimal number 0-255 plus a comma) would be prohibitively slow:
/// the Report Bug dialog's Save to file… writes exactly this way, and a
/// bundle carrying an imported model easily reaches hundreds of megabytes of
/// JSON for what's really a few hundred megabytes of bytes, which is what
/// hung the webview.
///
/// A raw-body invoke's argument position IS the byte buffer (see
/// `relay_client.rs::relay_put`, the same pattern), so the destination path
/// can't ride along as a normal argument — it comes in the `path` request
/// header instead, percent-encoded by the caller. Gated by the exact same
/// `write_allowed` approval check `write_file` uses, not a looser one: this
/// is still arbitrary-file-write from the webview's perspective if that
/// check were skipped or reimplemented more permissively.
#[tauri::command]
fn write_file_bytes(app: tauri::AppHandle, request: tauri::ipc::Request<'_>) -> Result<(), String> {
    let path_header = request
        .headers()
        .get("path")
        .ok_or_else(|| "write_file_bytes: missing path header".to_owned())?
        .to_str()
        .map_err(|_| "write_file_bytes: path header is not valid ASCII".to_owned())?;
    let path = percent_decode(path_header)?;

    let bytes = match request.body() {
        tauri::ipc::InvokeBody::Raw(bytes) => bytes,
        tauri::ipc::InvokeBody::Json(_) => {
            return Err("write_file_bytes expects a raw byte body".to_owned());
        }
    };

    if !write_allowed(&app, Path::new(&path)) {
        return Err(format!(
            "write_file_bytes: {path:?} is not a user-approved path"
        ));
    }
    std::fs::write(&path, bytes).map_err(|e| format!("write_file_bytes failed for {path:?}: {e}"))
}

/// List the direct children of a user-approved directory, returning their
/// absolute paths. Only regular files are included (directories are omitted).
#[tauri::command]
fn list_dir(app: tauri::AppHandle, path: String) -> Result<Vec<String>, String> {
    if !list_allowed(&app, Path::new(&path)) {
        return Err(format!("list_dir: {path:?} is not a user-approved path"));
    }
    let entries =
        std::fs::read_dir(&path).map_err(|e| format!("list_dir failed for {path:?}: {e}"))?;
    let mut result = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        if entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
            result.push(entry.path().to_string_lossy().to_string());
        }
    }
    Ok(result)
}

/// One filter row for the native pick dialogs.
#[derive(serde::Deserialize)]
struct DialogFilter {
    name: String,
    extensions: Vec<String>,
}

/// Show the native open-file dialog and, when the user picks a file, record
/// its approval before returning the path. Approval happens HERE, at the
/// only place that knows a real dialog interaction occurred — the JS side
/// merely receives the already-approved path. `write` additionally approves
/// write-back (opening a .hew implies Save may later overwrite it);
/// `approve_dir` extends read approval to the picked file's directory
/// (import texture scans).
#[tauri::command]
async fn pick_open_path(
    app: tauri::AppHandle,
    filters: Vec<DialogFilter>,
    write: bool,
    approve_dir: bool,
) -> Option<String> {
    use tauri_plugin_dialog::DialogExt;
    let mut dialog = app.dialog().file();
    for f in &filters {
        let exts: Vec<&str> = f.extensions.iter().map(String::as_str).collect();
        dialog = dialog.add_filter(&f.name, &exts);
    }
    // Blocking dialog off the event loop (the dialog plugin forbids its
    // blocking API on the main thread).
    let picked = tauri::async_runtime::spawn_blocking(move || dialog.blocking_pick_file())
        .await
        .ok()
        .flatten()?;
    let path = picked.into_path().ok()?;
    approve_file(&app, &path, write);
    if approve_dir {
        if let Some(dir) = path.parent() {
            approve_dir_reads(&app, dir);
        }
    }
    Some(path.to_string_lossy().into_owned())
}

/// Show the native save-file dialog and, when the user confirms a target,
/// record write approval before returning the path (same trust model as
/// `pick_open_path`).
#[tauri::command]
async fn pick_save_path(
    app: tauri::AppHandle,
    default_name: String,
    filters: Vec<DialogFilter>,
) -> Option<String> {
    use tauri_plugin_dialog::DialogExt;
    let mut dialog = app.dialog().file().set_file_name(&default_name);
    for f in &filters {
        let exts: Vec<&str> = f.extensions.iter().map(String::as_str).collect();
        dialog = dialog.add_filter(&f.name, &exts);
    }
    let picked = tauri::async_runtime::spawn_blocking(move || dialog.blocking_save_file())
        .await
        .ok()
        .flatten()?;
    let path = picked.into_path().ok()?;
    approve_file(&app, &path, true);
    Some(path.to_string_lossy().into_owned())
}

/// Show the platform's save panel with the format list in it (and the
/// chosen format's options), and record write approval for the path the
/// user confirmed. `None` when canceled; an error when the panel could
/// not be shown, so the webview says so rather than treating it as a
/// cancel. The webview writes the bytes afterwards, since they depend on
/// the format picked.
#[tauri::command]
async fn pick_save_target(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    title: String,
    default_name: String,
    formats: Vec<save_panel::PanelFormat>,
    initial: String,
) -> Result<Option<save_panel::PanelResult>, String> {
    let initial = formats.iter().position(|f| f.id == initial).unwrap_or(0);
    let request = save_panel::PanelRequest {
        title,
        default_name,
        formats,
        initial,
    };
    let picked = tauri::async_runtime::spawn_blocking(move || save_panel::run(&window, request))
        .await
        .map_err(|e| e.to_string())??;
    if let Some(p) = &picked {
        approve_file(&app, Path::new(&p.path), true);
    }
    Ok(picked)
}

/// Take the pending-open path (cold-start file association) — returns Some once,
/// then None on subsequent calls.
#[tauri::command]
fn take_pending_open(app: tauri::AppHandle) -> Option<String> {
    app.state::<Mutex<PendingOpen>>().lock().ok()?.0.take()
}

/// Mark the calling webview as ready to receive `menu-open-path` events.
/// Called once per window after its listener is registered; from then on,
/// file-association opens are delivered live instead of buffered.
#[tauri::command]
fn frontend_ready(app: tauri::AppHandle, window: tauri::WebviewWindow) {
    if let Ok(mut ready) = app.state::<Mutex<ReadyWindows>>().lock() {
        ready.0.insert(window.label().to_string());
    }
}
/// Whether a window label names a DOCUMENT window — the windows that hold a
/// model and can receive menu actions, file opens, and library actions.
/// The settings window and the library browser window are auxiliary: they
/// must never be the "active document" target, never appear in the Window
/// menu's focus list, and never absorb a routed open.
fn is_document_window_label(label: &str) -> bool {
    label != "settings" && label != "library"
}

/// The document window menu/open events should land in: the focused
/// non-settings window, else the most recently focused one that still
/// exists, else `main`, else any document window.
fn active_document_window(app: &tauri::AppHandle) -> Option<tauri::WebviewWindow> {
    let windows = app.webview_windows();
    if let Some(w) = windows
        .values()
        .find(|w| is_document_window_label(w.label()) && w.is_focused().unwrap_or(false))
    {
        return Some(w.clone());
    }
    if let Ok(active) = app.state::<Mutex<ActiveWindow>>().lock() {
        if let Some(label) = active.0.as_deref() {
            if let Some(w) = windows.get(label) {
                return Some(w.clone());
            }
        }
    }
    windows.get("main").cloned().or_else(|| {
        windows
            .values()
            .find(|w| is_document_window_label(w.label()))
            .cloned()
    })
}

/// Emit an event to exactly one document window (see
/// `active_document_window`). Never broadcasts: `app.emit` reaches every
/// window-scoped listener, so a broadcast Undo/Delete/Open would mutate
/// every open document at once. With no document window alive the event is
/// dropped — there is nothing meaningful to apply it to.
fn emit_to_active(app: &tauri::AppHandle, event: &str, payload: &str) {
    if let Some(w) = active_document_window(app) {
        let _ = app.emit_to(w.label(), event, payload);
    }
}

/// Route a file-association/second-instance open: approve the path (it came
/// from the OS on the user's behalf), then deliver it live if the window it
/// would target has its listener registered, else buffer it for a mount-time
/// poll. The readiness check is on the TARGET window, not "any window" — a
/// freshly created File ▸ New window can be focused before its listener
/// exists, and an event emitted at it would vanish. The buffer is only ever
/// written pre-readiness, so it cannot go stale and leak into a later
/// File ▸ New window.
fn deliver_open(app: &tauri::AppHandle, path: &str) {
    // Cold start: the macOS "open document" Apple event (or a racing
    // second-instance open) can arrive BEFORE `setup` has managed the rest
    // of the app state, so none of it can be consulted yet. Buffer the path
    // — the buffer itself is managed on the builder — and let `setup`
    // approve it; the first window's mount-time poll then opens it. The
    // buffer lock is held across the check so it pairs with `setup`.
    {
        let pending_state = app.state::<Mutex<PendingOpen>>();
        let Ok(mut pending) = pending_state.lock() else {
            return;
        };
        if app.try_state::<Mutex<WindowMenuState>>().is_none() {
            pending.0 = Some(path.to_string());
            return;
        }
    }
    // Already open in some window (a second double-click on the same file
    // in Finder/Explorer, say)? Focus it — never a second copy.
    if let Some(label) = window_label_for_path(app, path) {
        let _ = focus_window_by_label(app, &label);
        return;
    }
    approve_file(app, Path::new(path), true);
    let target_ready = active_document_window(app).is_some_and(|w| {
        app.state::<Mutex<ReadyWindows>>()
            .lock()
            .map(|r| r.0.contains(w.label()))
            .unwrap_or(false)
    });
    if target_ready {
        emit_to_active(app, "menu-open-path", path);
    } else if let Ok(mut guard) = app.state::<Mutex<PendingOpen>>().lock() {
        guard.0 = Some(path.to_string());
    }
}

/// Open (or focus) the Settings window. Lives here rather than in the
/// webview so the capability set needs no window-creation grants — a
/// compromised webview must not be able to mint windows (new document
/// windows are likewise created by the `new_window` command).
///
/// `async` is load-bearing: a synchronous command deadlocks WebView2 creation
/// on Windows and yields a zombie window — see the note on [`new_window`].
#[tauri::command]
async fn open_settings_window(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(existing) = app.get_webview_window("settings") {
        return existing.set_focus().map_err(|e| e.to_string());
    }
    // Fixed-size, per macOS Settings convention (HIG: settings windows
    // aren't resizable; each pane determines its own compact height).
    let title = current_menu_labels(&app).get("window:settings").to_string();
    tauri::WebviewWindowBuilder::new(
        &app,
        "settings",
        tauri::WebviewUrl::App("index.html#settings".into()),
    )
    .title(title)
    .inner_size(520.0, 360.0)
    .resizable(false)
    .minimizable(false)
    .maximizable(false)
    .build()
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Prepend `path` to the recents list (dedup, cap RECENTS_MAX), persist, and
/// rebuild the "Open Recent" submenu.
/// Opens (or focuses) the Library browser as a real native window — the
/// Settings-window pattern, but resizable: the Library is a working
/// surface, not a fixed-form preferences pane.
#[tauri::command]
async fn open_library_window(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(existing) = app.get_webview_window("library") {
        return existing.set_focus().map_err(|e| e.to_string());
    }
    let title = current_menu_labels(&app).get("win-library").to_string();
    tauri::WebviewWindowBuilder::new(
        &app,
        "library",
        tauri::WebviewUrl::App("index.html#library".into()),
    )
    .title(title)
    .inner_size(920.0, 632.0)
    .min_inner_size(640.0, 420.0)
    .resizable(true)
    .build()
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Routes a Library-window action (insert / open-as-document / paint /
/// add-to-palette) to the ACTIVE DOCUMENT window — the same delivery the
/// native menus use (`emit_to_active`), which is exactly why clicking in
/// the focused Library window still reaches the right model: the library
/// label is excluded from active-document tracking, so the last document
/// window the user worked in stays the target. `focus_target` pulls that
/// window forward (an insert continues with cursor placement there).
/// Forwards a document window's placements/palette push to the Library
/// window — ONLY when the sender is the active document window, so the
/// Library's badges always describe the same document its actions target
/// (`library_dispatch` routes through the same `active_document_window`).
/// A background window's docRev churn or a stale request answer is
/// silently discarded here rather than racing the active window's data.
#[tauri::command]
fn library_push_placements(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    payload: String,
) -> Result<(), String> {
    let is_active =
        active_document_window(&app).is_some_and(|active| active.label() == window.label());
    if is_active {
        let _ = app.emit_to("library", "library-placements-json", payload);
    }
    Ok(())
}

#[tauri::command]
fn library_dispatch(
    app: tauri::AppHandle,
    payload: String,
    focus_target: bool,
) -> Result<bool, String> {
    let Some(target) = active_document_window(&app) else {
        // No document window exists (all closed) — report honestly so the
        // Library window can tell the user instead of silently no-oping.
        return Ok(false);
    };
    if focus_target {
        let _ = target.set_focus();
    }
    let _ = app.emit_to(target.label(), "library-action", payload);
    Ok(true)
}

/// One row of a native library context menu (`library_popup_menu`).
#[derive(serde::Deserialize)]
struct LibraryMenuEntry {
    id: String,
    label: String,
    #[serde(default)]
    disabled: bool,
    #[serde(default)]
    separator: bool,
}

/// Pops a NATIVE context menu on the Library window (the item tiles' `⋯`
/// and right-click menu — stock menus, not hand-drawn DOM). Selection is
/// delivered asynchronously through the app's menu-event handler, which
/// forwards any `libmenu:`-prefixed id back to the library window as a
/// `library-menu` event; dismissal delivers nothing, exactly like every
/// native menu.
#[tauri::command]
fn library_popup_menu(
    window: tauri::WebviewWindow,
    entries: Vec<LibraryMenuEntry>,
) -> Result<(), String> {
    use tauri::menu::{MenuBuilder, MenuItemBuilder, PredefinedMenuItem};
    let app = window.app_handle().clone();
    let win = window.clone();
    window
        .run_on_main_thread(move || {
            let mut builder = MenuBuilder::new(&app);
            for e in &entries {
                if e.separator {
                    if let Ok(sep) = PredefinedMenuItem::separator(&app) {
                        builder = builder.item(&sep);
                    }
                    continue;
                }
                let item = MenuItemBuilder::new(&e.label)
                    .id(format!("libmenu:{}", e.id))
                    .enabled(!e.disabled)
                    .build(&app);
                if let Ok(item) = item {
                    builder = builder.item(&item);
                }
            }
            if let Ok(menu) = builder.build() {
                let _ = win.popup_menu(&menu);
            }
        })
        .map_err(|e| e.to_string())
}

/// A native confirmation dialog on the Library window (the destructive
/// delete's stock confirm — OK is `action_label`, cancel is Cancel).
#[tauri::command]
async fn library_confirm(
    window: tauri::WebviewWindow,
    title: String,
    message: String,
    action_label: String,
) -> Result<bool, String> {
    use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};
    let dialog = window.dialog().message(message).title(title).buttons(
        MessageDialogButtons::OkCancelCustom(action_label, "Cancel".to_string()),
    );
    tauri::async_runtime::spawn_blocking(move || dialog.blocking_show())
        .await
        .map_err(|e| e.to_string())
}

/// True when `name` is a path Hew opens as a document: `.hew` or SketchUp's
/// `.skp` (case-insensitive). Pure; the extension test every open route
/// (argv, single-instance, macOS Open With) and the recents gate share.
fn is_document_name(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.ends_with(".hew") || lower.ends_with(".skp")
}

/// True only for an existing regular file whose name ends `.hew` or `.skp`
/// (case-insensitive), with symlinks resolved. Gate for what may enter or
/// stay in the recents list: recents are auto-approved (read+write) at
/// launch, and their path — unlike every other `ApprovedPaths` input —
/// originates from persisted state / a JS `push_recent` argument, not a
/// native picker. This keeps a compromised webview from persisting an
/// arbitrary path (a launch agent, `~/.zshrc`, `/etc/...`) that the next
/// launch would then approve.
fn is_recentable_document_file(path: &str) -> bool {
    let Ok(canon) = std::fs::canonicalize(path) else {
        return false;
    };
    canon.is_file()
        && canon
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("hew") || e.eq_ignore_ascii_case("skp"))
}

#[tauri::command]
fn push_recent(path: String, app: tauri::AppHandle) -> Result<(), String> {
    // A recent must be a real .hew or .skp document — never an arbitrary path a
    // compromised webview could plant for the launch-time auto-approval to
    // pick up (audit sec-desktop). Silently ignore anything else: a genuine
    // just-opened/just-saved .hew always qualifies, so this never rejects a
    // legitimate call.
    if !is_recentable_document_file(&path) {
        return Ok(());
    }
    let state = app.state::<Mutex<RecentState>>();
    let mut guard = state.lock().map_err(|e| e.to_string())?;
    // Dedup: remove existing entry for this path, then prepend.
    guard.paths.retain(|p| p != &path);
    guard.paths.insert(0, path);
    guard.paths.truncate(RECENTS_MAX);
    let paths = guard.paths.clone();
    save_recents(&app, &paths);
    rebuild_recent_submenu(&guard.submenu, &app, &paths, &current_menu_labels(&app))
}

/// Return the recents list (most-recent first) for the in-app web menu (Linux,
/// where the native "Open Recent" submenu isn't used).
#[tauri::command]
fn get_recents(app: tauri::AppHandle) -> Result<Vec<String>, String> {
    let state = app.state::<Mutex<RecentState>>();
    let guard = state.lock().map_err(|e| e.to_string())?;
    Ok(guard.paths.clone())
}

/// Clear the recents list, persist, and rebuild the submenu.
#[tauri::command]
fn clear_recent(app: tauri::AppHandle) -> Result<(), String> {
    let state = app.state::<Mutex<RecentState>>();
    let mut guard = state.lock().map_err(|e| e.to_string())?;
    guard.paths.clear();
    save_recents(&app, &[]);
    rebuild_recent_submenu(&guard.submenu, &app, &[], &current_menu_labels(&app))
}

// ---------------------------------------------------------------------------
// Library — a folder of `.hew` files (default `$HOME/Hew Library`) organized
// by type into `Components/`, `Materials/`, `Models/` subfolders (legacy
// flat items directly in the folder still work), plus a thumbnail cache in
// `<folder>/.thumbnails/`. The configured folder is stored as JSON in the
// app config dir (`library.json`, `{"dir": "..."}`), mirroring
// `recents.json` above.
//
// The on-disk layout itself — folder resolution, item-name/thumbnail-key
// validation, atomic list/read/write/remove — is owned by `crates/library`
// (docs/design/v1.1-cycle.md's Lane B), the SAME crate `hew-cli` uses for
// `hew.library.*`: this shell used to carry its own copy of every one of
// those rules, which is exactly the two-implementations-that-can-drift
// problem that crate exists to close. What stays HERE, necessarily
// Tauri-specific: the native folder-picker dialog, the `settings-changed`/
// `library-changed` cross-window broadcast, and the config file's own
// read/write (`crates/library` only ever READS it, via `library::LibraryDir::
// resolve` — the desktop app is what WRITES a user's folder choice, so this
// half stays the shell's job, at the exact path `library::config_path()`
// gives it, so the two directions can never resolve to different files).
// ---------------------------------------------------------------------------

#[derive(serde::Serialize, serde::Deserialize)]
struct LibraryConfig {
    dir: String,
}

/// Persist the configured library folder to `library::config_path()` — the
/// exact file `library::LibraryDir::resolve` reads back.
fn save_library_dir(dir: &Path) -> Result<(), String> {
    let Some(path) = library::config_path() else {
        return Err("could not resolve the app config directory".into());
    };
    let Some(config_dir) = path.parent() else {
        return Err("could not resolve the app config directory".into());
    };
    std::fs::create_dir_all(config_dir).map_err(|e| e.to_string())?;
    let config = LibraryConfig {
        dir: dir.to_string_lossy().into_owned(),
    };
    let text = serde_json::to_string(&config).map_err(|e| e.to_string())?;
    std::fs::write(&path, text).map_err(|e| e.to_string())
}

/// The effective library folder: `HEW_LIBRARY_DIR` (documented override,
/// shared with `hew-cli`), else the configured one, else `$HOME/Hew
/// Library` — `library::LibraryDir::resolve`'s own fallback chain. Never
/// creates it.
fn resolved_library_dir() -> PathBuf {
    library::LibraryDir::resolve(None).root().to_path_buf()
}

/// The configured (or default) library folder as a display path.
#[tauri::command]
fn library_get_dir() -> String {
    resolved_library_dir().to_string_lossy().into_owned()
}

/// Show a native folder picker; on a pick, persist it as the library folder
/// and broadcast `settings-changed` (`{"key": "libraryFolder"}`) to every
/// window, so the browser and any other open Settings window pick it up
/// without polling. Returns `None` on cancel — not an error.
#[tauri::command]
async fn library_choose_dir(app: tauri::AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let current = resolved_library_dir();
    let title = current_menu_labels(&app)
        .get("dialog:choose-library-folder")
        .to_string();
    let dialog = app.dialog().file().set_title(title).set_directory(&current);
    let picked = tauri::async_runtime::spawn_blocking(move || dialog.blocking_pick_folder())
        .await
        .map_err(|e| e.to_string())?;
    let Some(picked) = picked else {
        return Ok(None);
    };
    let path = picked.into_path().map_err(|e| e.to_string())?;
    save_library_dir(&path)?;
    let _ = app.emit(
        "settings-changed",
        serde_json::json!({ "key": "libraryFolder" }),
    );
    Ok(Some(path.to_string_lossy().into_owned()))
}

/// One `.hew` file in the library folder, as `library_list` reports it —
/// `tauriLibraryStore.ts`'s `LibraryListEntry` shape, unchanged by the
/// switch to `crates/library` underneath.
#[derive(serde::Serialize)]
struct LibraryEntry {
    name: String,
    size: u64,
    mtime_ms: u64,
}

/// List every `.hew` item in the library folder: legacy flat files directly
/// in the folder (kept working) plus each of the three category subfolders
/// (`Components/`, `Materials/`, `Models/`), reported as `"Category/
/// name.hew"`. A missing folder — nothing saved to the library yet — lists
/// as empty, not an error. A file that fails to parse (`library::list`'s
/// `error` entries) is still reported by name/size/mtime — the web app's
/// own `libraryModel.ts` is what turns that into an error tile, so this
/// stays a bare listing exactly as it was before the switch.
#[tauri::command]
fn library_list() -> Vec<LibraryEntry> {
    let dir = library::LibraryDir::resolve(None);
    library::list(&dir)
        .into_iter()
        .map(|item| LibraryEntry {
            name: item.rel_path,
            size: item.size,
            mtime_ms: item.mtime_ms,
        })
        .collect()
}

/// Read a library item's raw bytes. Same `tauri::ipc::Response` shape as
/// `read_file` — no JSON round-trip for multi-megabyte models.
#[tauri::command]
fn library_read(name: String) -> Result<tauri::ipc::Response, String> {
    let dir = library::LibraryDir::resolve(None);
    let bytes =
        library::read(&dir, &name).map_err(|e| format!("library_read failed for {name:?}: {e}"))?;
    Ok(tauri::ipc::Response::new(bytes))
}

/// Write a library item, creating the library folder — and, for a
/// two-segment name, its category subfolder — on first use.
#[tauri::command]
fn library_write(app: tauri::AppHandle, name: String, contents: Vec<u8>) -> Result<(), String> {
    let dir = library::LibraryDir::resolve(None);
    library::write(&dir, &name, &contents)
        .map_err(|e| format!("library_write failed for {name:?}: {e}"))?;
    // Content changed: every window's listing (an open Library window, the
    // command palette's index in other document windows) refreshes off this.
    let _ = app.emit("library-changed", ());
    Ok(())
}

/// Delete a library item. Deleting an already-absent file is not an error —
/// the caller's goal ("this name is gone") is already satisfied.
#[tauri::command]
fn library_delete(app: tauri::AppHandle, name: String) -> Result<(), String> {
    let dir = library::LibraryDir::resolve(None);
    library::remove(&dir, &name).map_err(|e| format!("library_delete failed for {name:?}: {e}"))?;
    let _ = app.emit("library-changed", ());
    Ok(())
}

/// Reveal a library item in the OS file manager (Finder/Explorer/etc).
#[tauri::command]
fn library_reveal(name: String) -> Result<(), String> {
    if !library::valid_item_name(&name) {
        return Err(format!("library_reveal: invalid item name {name:?}"));
    }
    let dir = library::LibraryDir::resolve(None);
    tauri_plugin_opener::reveal_item_in_dir(dir.root().join(&name)).map_err(|e| e.to_string())
}

/// Resolve a library item's absolute filesystem path — the Library ▸ Open
/// as Document flow's non-pristine-window case needs a real path to hand
/// `open_in_new_window` (`LibraryStore` otherwise only deals in bare names,
/// deliberately: everywhere else a path would be an unvalidated escape
/// hatch). Same name validation as every other `library_*` command; the
/// path is never checked to exist — the caller (`open_in_new_window`'s
/// eventual read) reports that failure.
#[tauri::command]
fn library_item_path(app: tauri::AppHandle, name: String) -> Result<String, String> {
    if !library::valid_item_name(&name) {
        return Err(format!("library_item_path: invalid item name {name:?}"));
    }
    let dir = library::LibraryDir::resolve(None);
    let path = dir.root().join(&name);
    // The caller's next step is handing this path to the ORDINARY open
    // machinery (`open_in_new_window` → the new window's `read_file`),
    // which enforces the approved-paths registry the `library_*` commands
    // themselves bypass. The request is the user's own click on a library
    // item — approve it like any picked/OS-delivered open (`deliver_open`'s
    // rationale), or the fresh window refuses the read and strands as an
    // Untitled document (playtest round 2).
    approve_file(&app, &path, false);
    Ok(path.to_string_lossy().into_owned())
}

/// Read a cached thumbnail PNG by content-hash key. Errs with the literal
/// string `"not_found"` when absent (vs. any other message on a real I/O
/// failure) — the one signal `tauriLibraryStore.ts` treats as "no thumbnail
/// yet" and maps to `null`, rather than surfacing a Vec<u8>-empty-means-
/// missing convention that a genuinely empty (0-byte) cached PNG could
/// collide with.
#[tauri::command]
fn library_thumb_read(key: String) -> Result<tauri::ipc::Response, String> {
    let dir = library::LibraryDir::resolve(None);
    match library::read_thumbnail(&dir, &key) {
        Ok(Some(bytes)) => Ok(tauri::ipc::Response::new(bytes)),
        Ok(None) => Err("not_found".to_string()),
        Err(e) => Err(format!("library_thumb_read failed for {key:?}: {e}")),
    }
}

/// Write a cached thumbnail PNG, creating `.thumbnails/` on first use.
#[tauri::command]
fn library_thumb_write(key: String, contents: Vec<u8>) -> Result<(), String> {
    let dir = library::LibraryDir::resolve(None);
    library::write_thumbnail(&dir, &key, &contents)
        .map_err(|e| format!("library_thumb_write failed for {key:?}: {e}"))
}

// ---------------------------------------------------------------------------
// Submenu rebuild helper.
// Removes all current items from the submenu and repopulates from `paths`.
// Layout:
//   <path items>
//   ----  (separator, only when paths non-empty)
//   Clear Recent
// ---------------------------------------------------------------------------
fn rebuild_recent_submenu(
    submenu: &tauri::menu::Submenu<tauri::Wry>,
    app: &tauri::AppHandle,
    paths: &[String],
    labels: &MenuLabels,
) -> Result<(), String> {
    // Remove every existing item.
    while let Ok(Some(_)) = submenu.remove_at(0) {}

    // Append one item per path.
    for path in paths {
        let label = basename(path);
        let item = MenuItemBuilder::with_id(format!("recent:{path}"), label)
            .build(app)
            .map_err(|e| e.to_string())?;
        submenu.append(&item).map_err(|e| e.to_string())?;
    }

    // Separator + "Clear Recent" (always present so users know there's a menu).
    let sep = PredefinedMenuItem::separator(app).map_err(|e| e.to_string())?;
    submenu.append(&sep).map_err(|e| e.to_string())?;

    let clear = MenuItemBuilder::with_id("recent-clear", labels.get("recent-clear"))
        .build(app)
        .map_err(|e| e.to_string())?;
    submenu.append(&clear).map_err(|e| e.to_string())?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

fn main() {
    // Check argv for a .hew or .skp path on Windows/Linux (best-effort, first launch).
    // macOS uses RunEvent::Opened (Apple event), so we skip argv there.
    let argv_path: Option<String> = {
        #[cfg(not(target_os = "macos"))]
        {
            std::env::args().skip(1).find(|a| is_document_name(a))
        }
        #[cfg(target_os = "macos")]
        {
            None
        }
    };

    let builder = tauri::Builder::default()
        // Managed here, not in `setup`: a macOS cold-start open event can
        // fire before `setup` runs and needs somewhere to land (see
        // `deliver_open`). `setup` seeds it from argv on Windows/Linux.
        .manage(Mutex::new(PendingOpen(None)))
        // Single-instance guard, registered first (its docs require it): a
        // second launch — a Windows/Linux file-association double-click —
        // joins the running instance instead of spawning a second process
        // whose "main" window would fight this one over recovery-main.hew
        // and window-state.json.
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            if let Some(path) = args.iter().skip(1).find(|a| is_document_name(a)) {
                deliver_open(app, path);
            }
            if let Some(w) = active_document_window(app) {
                let _ = w.unminimize();
                let _ = w.set_focus();
            }
        }))
        // Register the dialog plugin (open/save native dialogs).
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        // Opener: lets the webview hand a URL (the getting-started guide link on
        // the welcome screen) to the OS default browser instead of trying to
        // navigate the app's own webview.
        .plugin(tauri_plugin_opener::init());

    // The updater plugin ships only in updater-enabled builds (the `updater`
    // feature); package-manager builds compile it out.
    #[cfg(feature = "updater")]
    let builder = builder.plugin(tauri_plugin_updater::Builder::new().build());

    // Register custom file-I/O commands.
    builder
        .invoke_handler(tauri::generate_handler![
            read_file,
            write_file,
            write_file_bytes,
            list_dir,
            pick_open_path,
            pick_save_path,
            pick_save_target,
            take_pending_open,
            take_pending_recovery,
            frontend_ready,
            new_window,
            open_in_new_window,
            take_pending_window_open,
            list_windows,
            focus_window,
            set_document_path,
            set_document_dirty,
            focus_window_with_path,
            set_window_title,
            fly_cursor::set_fly_cursor_lock,
            open_settings_window,
            open_library_window,
            library_dispatch,
            library_push_placements,
            library_popup_menu,
            library_confirm,
            sync_menu_state,
            set_menu_locale,
            clipboard_set,
            clipboard_get,
            push_recent,
            get_recents,
            clear_recent,
            library_get_dir,
            library_choose_dir,
            library_list,
            library_read,
            library_write,
            library_delete,
            library_reveal,
            library_item_path,
            library_thumb_read,
            library_thumb_write,
            recovery_write,
            recovery_list,
            recovery_claim,
            recovery_clear,
            recovery_source_write,
            recovery_source_clear,
            recovery_source_read,
            recovery_discard_all,
            log_append,
            log_rotate,
            reproducer_write,
            check_for_updates,
            updater_available,
            fonts::list_system_fonts,
            fonts::read_font_file,
            qr::qr_svg,
            print::print_document,
            print::print_defaults,
            print::screen_mm_per_px,
            relay_client::get_server_setting,
            relay_client::set_server_setting,
            relay_client::relay_identity,
            relay_client::relay_put,
            relay_client::relay_peek,
            relay_client::relay_delete,
            report_client::report_submit,
            follow::follow_list,
            follow::follow_check,
            follow::follow_post,
            follow::follow_seen,
            follow::follow_attach,
            follow::follow_forget,
        ])
        // Build and attach the native menu bar; wire menu-item clicks to
        // `menu-action` events emitted to the webview.
        .setup(move |app| {
            let handle = app.handle();

            // Handle registries for stateful menu items (filled in as the
            // menus below are built; managed as MenuHandles at the end).
            let mut checks: HashMap<String, CheckMenuItem<tauri::Wry>> = HashMap::new();
            let mut gated: HashMap<String, MenuItem<tauri::Wry>> = HashMap::new();

            // Every label below comes from the per-locale table
            // (menu_labels.rs; docs/LOCALIZATION.md "Native menus"), in the
            // language the last session recorded so the first frame is
            // already right. `set_menu_locale` relabels in place later.
            let labels = MenuLabels::for_locale(
                menu_labels::read_persisted_locale(handle.path().app_config_dir().ok().as_deref())
                    .as_deref()
                    .unwrap_or("en"),
            );
            // A macro rather than a closure: a closure returning a borrow of
            // `labels` cannot name that lifetime, and every use here is a
            // plain inline lookup.
            macro_rules! lbl {
                ($key:expr) => {
                    labels.get($key)
                };
            }
            let mut predefined: Vec<(String, PredefinedMenuItem<tauri::Wry>)> = Vec::new();
            let mut predef = |key: &str, item: PredefinedMenuItem<tauri::Wry>| {
                predefined.push((key.to_string(), item.clone()));
                item
            };
            let predef_about = predef(
                "predefined:about",
                PredefinedMenuItem::about(handle, Some(lbl!("predefined:about")), None)?,
            );
            let predef_services = predef(
                "predefined:services",
                PredefinedMenuItem::services(handle, Some(lbl!("predefined:services")))?,
            );
            let predef_hide = predef(
                "predefined:hide",
                PredefinedMenuItem::hide(handle, Some(lbl!("predefined:hide")))?,
            );
            let predef_hide_others = predef(
                "predefined:hide-others",
                PredefinedMenuItem::hide_others(handle, Some(lbl!("predefined:hide-others")))?,
            );
            let predef_show_all = predef(
                "predefined:show-all",
                PredefinedMenuItem::show_all(handle, Some(lbl!("predefined:show-all")))?,
            );
            let predef_quit = predef(
                "predefined:quit",
                PredefinedMenuItem::quit(handle, Some(lbl!("predefined:quit")))?,
            );
            let predef_minimize = predef(
                "predefined:minimize",
                PredefinedMenuItem::minimize(handle, Some(lbl!("predefined:minimize")))?,
            );
            let predef_maximize = predef(
                "predefined:maximize",
                PredefinedMenuItem::maximize(handle, Some(lbl!("predefined:maximize")))?,
            );

            // ----------------------------------------------------------------
            // App menu (macOS — About + Quit)
            // ----------------------------------------------------------------
            let app_settings = MenuItemBuilder::with_id("app-settings", lbl!("app-settings"))
                .accelerator("CmdOrCtrl+,")
                .build(handle)?;

            // "Check for Updates…" follows About (macOS convention), present
            // only in updater-enabled builds.
            let app_menu_builder =
                SubmenuBuilder::with_id(handle, "submenu:app", lbl!("submenu:app"))
                    .item(&predef_about);
            #[cfg(feature = "updater")]
            let app_menu_builder = app_menu_builder.item(
                &MenuItemBuilder::with_id("app-check-updates", lbl!("app-check-updates"))
                    .build(handle)?,
            );
            let app_menu = app_menu_builder
                .separator()
                .item(&app_settings)
                .separator()
                .item(&predef_services)
                .separator()
                .item(&predef_hide)
                .item(&predef_hide_others)
                .item(&predef_show_all)
                .separator()
                .item(&predef_quit)
                .build()?;

            // ----------------------------------------------------------------
            // Open Recent submenu — populated from persisted recents.
            // ----------------------------------------------------------------
            let recents = load_recents(handle);

            // Build an empty submenu first, then populate via the helper so
            // the rebuild logic stays in one place.
            let open_recent_submenu =
                SubmenuBuilder::with_id(handle, "submenu:open-recent", lbl!("submenu:open-recent"))
                    .build()?;
            rebuild_recent_submenu(&open_recent_submenu, handle, &recents, &labels)
                .map_err(|e| tauri::Error::Anyhow(anyhow::anyhow!(e)))?;

            // ----------------------------------------------------------------
            // File menu
            // ----------------------------------------------------------------
            let file_new = MenuItemBuilder::with_id("file-new", lbl!("file-new"))
                .accelerator("CmdOrCtrl+N")
                .build(handle)?;
            let file_open = MenuItemBuilder::with_id("file-open", lbl!("file-open"))
                .accelerator("CmdOrCtrl+O")
                .build(handle)?;
            // Registered as a GATED item: Import refuses while an explode
            // session is open (ExplodeSessionScope), and the frontend's
            // sync_menu_state push disables the native item to match the
            // web MenuBar/palette gating.
            let file_import = gated_item(
                handle,
                &mut gated,
                "file-import",
                lbl!("file-import"),
                None,
                None,
            )?;
            // Place Image: gated like Import (a picture is placed as an
            // instance, which an open component refuses).
            let file_import_image = gated_item(
                handle,
                &mut gated,
                "file-import-image",
                lbl!("file-import-image"),
                None,
                None,
            )?;
            // Export… opens the unified Export dialog (format — glTF/GLB or
            // STL — chosen there); there used to be a second "Export STL…"
            // item here, but the design calls for format choice handled in-dialog like
            // every other app.
            let file_export =
                MenuItemBuilder::with_id("file-export", lbl!("file-export")).build(handle)?;
            let file_print = MenuItemBuilder::with_id("file-print", lbl!("file-print"))
                .accelerator("CmdOrCtrl+P")
                .build(handle)?;
            // "Open on Phone…" (qr.rs, workers/share-relay/README.md):
            // encrypts the document and uploads it to a cloud dead-drop,
            // showing a QR the phone scans to fetch and decrypt it. Always
            // enabled here — like Save to Library above, macOS gets no live
            // disabled-state push for this item; the frontend refuses an
            // empty document with a toast instead (the web MenuBar's
            // `openOnPhoneDisabled` gating, App.tsx's "open-on-phone"
            // action).
            let file_open_on_phone =
                MenuItemBuilder::with_id("file-open-on-phone", lbl!("file-open-on-phone"))
                    .build(handle)?;
            let file_save = MenuItemBuilder::with_id("file-save", lbl!("file-save"))
                .accelerator("CmdOrCtrl+S")
                .build(handle)?;
            let file_save_as = MenuItemBuilder::with_id("file-save-as", lbl!("file-save-as"))
                .accelerator("Shift+CmdOrCtrl+S")
                .build(handle)?;
            // Throws away every change since the last save after one
            // confirmation (App.tsx's "revert-to-saved"). A GATED item: the
            // frontend's sync_menu_state push enables it only while the
            // document has a file and unsaved changes, as the web MenuBar
            // does. Starts disabled: nothing is open at launch.
            let file_revert = gated_item(
                handle,
                &mut gated,
                "file-revert",
                lbl!("file-revert"),
                None,
                None,
            )?;
            let _ = file_revert.set_enabled(false);
            // Prompts for a name (SaveToLibraryPopover's centered "modal"
            // variant) then writes the whole document as a library item —
            // always enabled here (macOS has no equivalent of the web
            // MenuBar's `disabled` prop); the frontend no-ops with a toast on
            // a platform without a library backend, same posture as every
            // other native item that has no web-gated counterpart.
            let file_save_to_library =
                MenuItemBuilder::with_id("file-save-to-library", lbl!("file-save-to-library"))
                    .build(handle)?;
            // Deletes every unused palette material and component
            // definition as one undoable step (v1.1 assets lane). No
            // accelerator — a housekeeping action, not a frequent one. The
            // frontend confirms with a preview modal before committing; the
            // native item just opens that same flow (App.tsx's
            // "purge-unused" action).
            let file_purge_unused =
                MenuItemBuilder::with_id("file-purge-unused", lbl!("file-purge-unused"))
                    .build(handle)?;
            let file_close = MenuItemBuilder::with_id("file-close", lbl!("file-close"))
                .accelerator("CmdOrCtrl+W")
                .build(handle)?;

            // HIG File order (TextEdit/Pages shape): the document block
            // (New/Open/Open Recent), then Close + the save block, then
            // interchange (Import/Export), then sharing, with Print… last —
            // as the platform guidelines put it.
            let file_menu = SubmenuBuilder::with_id(handle, "submenu:file", lbl!("submenu:file"))
                .item(&file_new)
                .item(&file_open)
                .item(&open_recent_submenu)
                .separator()
                .item(&file_close)
                .item(&file_save)
                .item(&file_save_as)
                .item(&file_revert)
                .item(&file_save_to_library)
                .separator()
                .item(&file_purge_unused)
                .separator()
                .item(&file_import)
                .item(&file_import_image)
                .item(&file_export)
                .separator()
                .item(&file_open_on_phone)
                .separator()
                .item(&file_print)
                .build()?;

            // ----------------------------------------------------------------
            // Edit menu
            // ----------------------------------------------------------------
            // Registered via `gated_item` (not a bare `MenuItemBuilder`) so
            // `sync_menu_state` can reach them through `guard.items` — Lane C
            // (docs/design/v1.1-cycle.md) wants their TEXT to track the top
            // undo/redo entry's label ("Undo Push/Pull"), which needs the
            // same handle enable/disable already uses.
            let edit_undo = gated_item(
                handle,
                &mut gated,
                "edit-undo",
                lbl!("edit-undo"),
                Some("CmdOrCtrl+Z"),
                Some("CmdOrCtrl+Z"),
            )?;
            let edit_redo = gated_item(
                handle,
                &mut gated,
                "edit-redo",
                lbl!("edit-redo"),
                Some("Shift+CmdOrCtrl+Z"),
                Some("Shift+CmdOrCtrl+Z"),
            )?;
            // No accelerator: a bare Backspace/Delete binding here would be a
            // global OS-level shortcut that fires even while typing in a text
            // field or dialog, unlike the JS keydown handler (App.tsx), which
            // is scoped to the Select tool and guards typing contexts. Relying
            // on the JS handler for the keyboard path is the documented
            // fallback when the menu lib's accelerator can't be scoped.
            let edit_delete = gated_item(
                handle,
                &mut gated,
                "edit-delete",
                lbl!("edit-delete"),
                None,
                None,
            )?;
            // Cut / Copy / Select All / Paste In Place carry their standard
            // key equivalents on macOS ONLY, ungated there (`edit_item`):
            // WebKit hands a Command key equivalent to the page — and to a
            // focused text field — before AppKit consults the menu bar, so
            // the JS keydown handler (App.tsx) keeps the viewport path, and
            // the menu item only fires when neither claimed the key. The
            // dispatcher then forwards a fire that lands while a text field
            // has focus to that field's own editing command, which is what
            // makes Cmd+C/X/A work in a rename field at all: a WebKit text
            // field has no editing key equivalents of its own on macOS and
            // relies on the Edit menu for them. A gated (disabled) item has
            // no key equivalent, hence ungated on macOS. Paste is the same
            // kind of item: a fire that lands in a text field reads the OS
            // clipboard through the clipboard-manager plugin and inserts the
            // text, so every Edit item is an ordinary one (no native paste
            // action with its own icon). On Windows and Linux the menu
            // library's accelerator
            // table runs BEFORE the webview, so these items stay
            // accelerator-free and gated there (the JS handler owns the
            // keyboard path), exactly as before.
            let edit_cut = edit_item(
                handle,
                &mut gated,
                "edit-cut",
                lbl!("edit-cut"),
                "CmdOrCtrl+X",
            )?;
            let edit_copy = edit_item(
                handle,
                &mut gated,
                "edit-copy",
                lbl!("edit-copy"),
                "CmdOrCtrl+C",
            )?;
            let edit_paste = edit_item(
                handle,
                &mut gated,
                "edit-paste",
                lbl!("edit-paste"),
                "CmdOrCtrl+V",
            )?;
            let edit_paste_in_place = edit_item(
                handle,
                &mut gated,
                "edit-paste-in-place",
                lbl!("edit-paste-in-place"),
                "Shift+CmdOrCtrl+V",
            )?;
            let edit_select_all = accel(
                MenuItemBuilder::with_id("edit-select-all", lbl!("edit-select-all")),
                Some("CmdOrCtrl+A"),
                None,
            )
            .build(handle)?;
            let edit_select_none = accel(
                MenuItemBuilder::with_id("edit-select-none", lbl!("edit-select-none")),
                Some("Shift+CmdOrCtrl+A"),
                None,
            )
            .build(handle)?;
            // No accelerator on any platform: SketchUp's own Shift+Cmd+I /
            // Ctrl+Shift+I is already claimed by Hew's Outliner toggle, so
            // Invert Selection is menu- and palette-only.
            let edit_invert_selection =
                MenuItemBuilder::with_id("edit-invert-selection", lbl!("edit-invert-selection"))
                    .build(handle)?;
            let edit_delete_guides =
                MenuItemBuilder::with_id("edit-delete-guides", lbl!("edit-delete-guides"))
                    .build(handle)?;

            // Object commands (previously buttons inside the Outliner panel —
            // they belong in the menu bar, enabled per selection via
            // `sync_menu_state`). Start disabled: nothing is selected at launch.
            let edit_group = gated_item(
                handle,
                &mut gated,
                "edit-group",
                lbl!("edit-group"),
                Some("CmdOrCtrl+G"),
                Some("CmdOrCtrl+G"),
            )?;
            let edit_ungroup = gated_item(
                handle,
                &mut gated,
                "edit-ungroup",
                lbl!("edit-ungroup"),
                Some("Shift+CmdOrCtrl+G"),
                Some("Shift+CmdOrCtrl+G"),
            )?;
            let edit_make_component = gated_item(
                handle,
                &mut gated,
                "edit-make-component",
                lbl!("edit-make-component"),
                None,
                None,
            )?;
            let edit_place_copy = gated_item(
                handle,
                &mut gated,
                "edit-place-copy",
                lbl!("edit-place-copy"),
                None,
                None,
            )?;
            let edit_explode = gated_item(
                handle,
                &mut gated,
                "edit-explode",
                lbl!("edit-explode"),
                None,
                None,
            )?;
            // The contextual dock's Save to Library verb, here too so it
            // stays reachable with the dock turned off (View ▸ Contextual
            // Dock). Distinct from File ▸ Save to Library…, which saves the
            // whole document.
            let edit_save_selection_to_library = gated_item(
                handle,
                &mut gated,
                "edit-save-selection-to-library",
                lbl!("edit-save-selection-to-library"),
                None,
                None,
            )?;
            let edit_make_unique = gated_item(
                handle,
                &mut gated,
                "edit-make-unique",
                lbl!("edit-make-unique"),
                None,
                None,
            )?;
            let edit_union = gated_item(
                handle,
                &mut gated,
                "edit-union",
                lbl!("edit-union"),
                None,
                None,
            )?;
            let edit_subtract = gated_item(
                handle,
                &mut gated,
                "edit-subtract",
                lbl!("edit-subtract"),
                None,
                None,
            )?;
            let edit_intersect = gated_item(
                handle,
                &mut gated,
                "edit-intersect",
                lbl!("edit-intersect"),
                None,
                None,
            )?;
            for item in [
                &edit_group,
                &edit_ungroup,
                &edit_make_component,
                &edit_place_copy,
                &edit_explode,
                &edit_save_selection_to_library,
                &edit_make_unique,
                &edit_union,
                &edit_subtract,
                &edit_intersect,
            ] {
                let _ = item.set_enabled(false);
            }

            // Edit stays textbook HIG (undo block, delete/select block, then
            // the specialized guide cleanup); everything object-level lives in
            // the Object menu below.
            let edit_menu = SubmenuBuilder::with_id(handle, "submenu:edit", lbl!("submenu:edit"))
                .item(&edit_undo)
                .item(&edit_redo)
                .separator()
                .item(&edit_cut)
                .item(&edit_copy)
                .item(&edit_paste)
                .item(&edit_paste_in_place)
                .separator()
                .item(&edit_delete)
                .item(&edit_select_all)
                .item(&edit_select_none)
                .item(&edit_invert_selection)
                .separator()
                .item(&edit_delete_guides)
                .build()?;

            // ----------------------------------------------------------------
            // Object menu — commands that act on the selected Objects/Groups/
            // Components (containers, component lifecycle, booleans). Split
            // out of Edit so the HIG-standard Edit menu stays small and
            // future object commands (mirror, flip, lock, …) have an obvious
            // home. Item ids keep their historical `edit-` prefix — they are
            // wire ids shared with the dispatch switch, sync_menu_state, and
            // the palette, not user-visible text.
            // ----------------------------------------------------------------
            // Make Solid… — always enabled (it works on the selection, or the whole
            // model when nothing is selected), so a plain item rather than `gated_item`.
            let object_make_solid =
                MenuItemBuilder::with_id("object-make-solid", lbl!("object-make-solid"))
                    .build(handle)?;
            // Model Health… opens the floating window (docs/design/import-health.md),
            // or brings it forward; also always enabled, so a plain item.
            let object_model_health =
                MenuItemBuilder::with_id("object-model-health", lbl!("object-model-health"))
                    .build(handle)?;
            let object_menu =
                SubmenuBuilder::with_id(handle, "submenu:object", lbl!("submenu:object"))
                    .item(&edit_group)
                    .item(&edit_ungroup)
                    .separator()
                    .item(&edit_make_component)
                    .item(&edit_make_unique)
                    .item(&edit_place_copy)
                    .item(&edit_explode)
                    .item(&edit_save_selection_to_library)
                    .separator()
                    .item(&edit_union)
                    .item(&edit_subtract)
                    .item(&edit_intersect)
                    .separator()
                    .item(&object_make_solid)
                    .item(&object_model_health)
                    .build()?;

            // ----------------------------------------------------------------
            // View menu
            // ----------------------------------------------------------------
            let view_axes = check_item(
                handle,
                &mut checks,
                "view-axes",
                lbl!("view-axes"),
                None,
                None,
            )?;
            // Reset the movable drawing axes (tool-parity §4) back to world
            // identity — a plain action, not a checkable toggle like Axes/
            // Grid/Guides above, so it's built with `MenuItemBuilder`
            // directly (same shape as "edit-delete-guides" below) rather
            // than `check_item`.
            let view_reset_axes =
                MenuItemBuilder::with_id("view-reset-axes", lbl!("view-reset-axes"))
                    .build(handle)?;
            let view_grid = check_item(
                handle,
                &mut checks,
                "view-grid",
                lbl!("view-grid"),
                None,
                None,
            )?;
            let view_guides = check_item(
                handle,
                &mut checks,
                "view-guides",
                lbl!("view-guides"),
                None,
                None,
            )?;
            // Section Plane's active (clipping) toggle — moved here from Tools
            // (section-plane-polish D3); same "toggle-section-active" dispatch
            // it always had. Checked only when a section is BOTH placed and
            // active; `sync_menu_state`'s `enabled` map (driven by the same
            // `view-section-plane` id) disables it while nothing is placed.
            // Labelled "Section Cut", not "Section Plane": the TOOL (which
            // places the plane) keeps that name under Tools; this toggle
            // clips/unclips the placed cut — the same naming split SketchUp
            // draws (View ▸ Section Cuts vs Tools ▸ Section Plane). The id
            // stays `view-section-plane` (wire id, not user-visible).
            let view_section_plane = check_item(
                handle,
                &mut checks,
                "view-section-plane",
                lbl!("view-section-plane"),
                None,
                None,
            )?;
            // Whole-model render modes: independent checkable toggles, no
            // accelerators (SketchUp binds none), per window, never saved.
            let view_xray = check_item(
                handle,
                &mut checks,
                "view-xray",
                lbl!("view-xray"),
                None,
                None,
            )?;
            let view_wireframe = check_item(
                handle,
                &mut checks,
                "view-wireframe",
                lbl!("view-wireframe"),
                None,
                None,
            )?;
            // Command palette — `Cmd/Ctrl+/` on every platform, matching the
            // JS keydown handler's binding in App.tsx so one shortcut is true
            // in both shells and the web build. `/` rather than `K` because
            // `Cmd+K` is Rectangle's classic macOS SketchUp accelerator and
            // keeps that job; the palette isn't a tool and has no claim on the
            // SketchUp key scheme. (Ctrl+K still opens the palette off-macOS
            // as an unadvertised alias — the JS handler keeps it.)
            let view_palette = accel(
                MenuItemBuilder::with_id("view-palette", lbl!("view-palette")),
                Some("CmdOrCtrl+/"),
                Some("CmdOrCtrl+/"),
            )
            .build(handle)?;

            // The window's chrome — the tool rail's icons-only mode, the
            // whole right-hand tray, and the contextual dock — ahead of the
            // tray's own sections. No accelerators: these are
            // set-and-forget layout choices, not worth a reserved chord
            // (SketchUp keeps none for its trays either).
            let view_compact_rail = check_item(
                handle,
                &mut checks,
                "view-compact-rail",
                lbl!("view-compact-rail"),
                None,
                None,
            )?;
            let view_tray = check_item(
                handle,
                &mut checks,
                "view-tray",
                lbl!("view-tray"),
                None,
                None,
            )?;
            let view_dock = check_item(
                handle,
                &mut checks,
                "view-dock",
                lbl!("view-dock"),
                None,
                None,
            )?;

            // Panel toggles — HIG puts show/hide of interface panels in the
            // View menu (Window holds actual windows). Ids keep their
            // historical `win-` prefix from when these lived on the Window
            // menu — wire ids, not user-visible text.
            let win_model_info = check_item(
                handle,
                &mut checks,
                "win-model-info",
                lbl!("win-model-info"),
                Some("Shift+CmdOrCtrl+I"),
                Some("Shift+CmdOrCtrl+I"),
            )?;
            let win_object_info = check_item(
                handle,
                &mut checks,
                "win-object-info",
                lbl!("win-object-info"),
                Some("Shift+CmdOrCtrl+O"),
                Some("Shift+CmdOrCtrl+O"),
            )?;
            let win_materials = check_item(
                handle,
                &mut checks,
                "win-materials",
                lbl!("win-materials"),
                Some("Shift+CmdOrCtrl+C"),
                Some("Shift+CmdOrCtrl+C"),
            )?;
            let win_components = check_item(
                handle,
                &mut checks,
                "win-components",
                lbl!("win-components"),
                Some("Shift+CmdOrCtrl+M"),
                Some("Shift+CmdOrCtrl+M"),
            )?;
            let win_tags = check_item(
                handle,
                &mut checks,
                "win-tags",
                lbl!("win-tags"),
                Some("Shift+CmdOrCtrl+T"),
                Some("Shift+CmdOrCtrl+T"),
            )?;
            // The Scenes panel toggle lives inside the Scenes submenu below,
            // keeping the whole feature (panel + actions + transitions) in
            // one place. No accelerator, same reasoning as win-debug-log: a
            // sixth Shift+Cmd pane chord isn't worth reserving.
            let win_scenes = check_item(
                handle,
                &mut checks,
                "win-scenes",
                lbl!("win-scenes"),
                None,
                None,
            )?;
            // Changes panel (Lane C, docs/design/v1.1-cycle.md): this
            // session's undo/redo entries since the last save. No
            // accelerator, same reasoning as win-scenes/win-debug-log.
            let win_changes = check_item(
                handle,
                &mut checks,
                "win-changes",
                lbl!("win-changes"),
                None,
                None,
            )?;

            // View ▸ Scenes (docs/design/scenes.md §5): Add/Update/Next/
            // Previous plus the Scene Transitions checkmark (a SPEC.md
            // deviation — the design put transitions in Settings; it lives
            // here at a fixed 600ms instead, App.tsx's own comment on why).
            // Next/Previous carry NO native accelerator on purpose: the JS
            // keydown handler already binds bare Page Down/Page Up on every
            // platform (no CmdOrCtrl, so it isn't gated behind
            // `nativeMenuOwnsModCombos`) — a native accelerator here would
            // double-fire alongside it on macOS.
            let scenes_add =
                MenuItemBuilder::with_id("scenes-add", lbl!("scenes-add")).build(handle)?;
            let scenes_update = gated_item(
                handle,
                &mut gated,
                "scenes-update",
                lbl!("scenes-update"),
                None,
                None,
            )?;
            let _ = scenes_update.set_enabled(false); // nothing active at launch
            let scenes_next = gated_item(
                handle,
                &mut gated,
                "scenes-next",
                lbl!("scenes-next"),
                None,
                None,
            )?;
            let _ = scenes_next.set_enabled(false); // no Scenes at launch
            let scenes_previous = gated_item(
                handle,
                &mut gated,
                "scenes-previous",
                lbl!("scenes-previous"),
                None,
                None,
            )?;
            let _ = scenes_previous.set_enabled(false);
            let scenes_transitions = check_item(
                handle,
                &mut checks,
                "scenes-transitions",
                lbl!("scenes-transitions"),
                None,
                None,
            )?;
            let scenes_menu =
                SubmenuBuilder::with_id(handle, "submenu:scenes", lbl!("submenu:scenes"))
                    .item(&win_scenes)
                    .item(&PredefinedMenuItem::separator(handle)?)
                    .item(&scenes_add)
                    .item(&scenes_update)
                    .item(&PredefinedMenuItem::separator(handle)?)
                    .item(&scenes_next)
                    .item(&scenes_previous)
                    .item(&PredefinedMenuItem::separator(handle)?)
                    .item(&scenes_transitions)
                    .build()?;

            let view_menu = SubmenuBuilder::with_id(handle, "submenu:view", lbl!("submenu:view"))
                .item(&view_axes)
                .item(&view_reset_axes)
                .item(&view_grid)
                .item(&view_guides)
                .item(&view_section_plane)
                .item(&view_xray)
                .item(&view_wireframe)
                .item(&PredefinedMenuItem::separator(handle)?)
                .item(&view_compact_rail)
                .item(&view_tray)
                .item(&view_dock)
                .item(&PredefinedMenuItem::separator(handle)?)
                .item(&win_model_info)
                .item(&win_object_info)
                .item(&win_materials)
                .item(&win_components)
                .item(&win_tags)
                .item(&PredefinedMenuItem::separator(handle)?)
                .item(&scenes_menu)
                .item(&win_changes)
                .item(&PredefinedMenuItem::separator(handle)?)
                .item(&view_palette)
                .build()?;

            // ----------------------------------------------------------------
            // Draw menu
            // ----------------------------------------------------------------
            let draw_rect = check_item(
                handle,
                &mut checks,
                "draw-rectangle",
                lbl!("draw-rectangle"),
                Some("CmdOrCtrl+K"),
                Some("R"),
            )?;

            let draw_circle = check_item(
                handle,
                &mut checks,
                "draw-circle",
                lbl!("draw-circle"),
                None,
                Some("C"),
            )?;

            // Polygon: SketchUp assigns no default key (it lives in the
            // shapes flyout) — no accelerator on either platform.
            let draw_polygon = check_item(
                handle,
                &mut checks,
                "draw-polygon",
                lbl!("draw-polygon"),
                None,
                None,
            )?;

            // Arc : Cmd+J is SketchUp's arc-family
            // key on macOS, even though Hew's Arc is the simpler 2-point
            // gesture rather than SketchUp's multi-mode arc tool family.
            let draw_arc = check_item(
                handle,
                &mut checks,
                "draw-arc",
                lbl!("draw-arc"),
                Some("CmdOrCtrl+J"),
                Some("A"),
            )?;

            let draw_line = check_item(
                handle,
                &mut checks,
                "draw-line",
                lbl!("draw-line"),
                Some("CmdOrCtrl+L"),
                Some("L"),
            )?;

            // "3D Text…" opens a dialog rather than arming a tool, so it's a
            // plain one-off item (no checked state — see `cam_zoom_extents`
            // above for the same pattern), not `check_item`. Draw placement
            // matches the web MenuBar and command palette (docs/design/3d-text.md).
            // Registered as a GATED item like Import above: 3D Text refuses
            // while an explode session is open.
            let draw_3d_text = gated_item(
                handle,
                &mut gated,
                "draw-3d-text",
                lbl!("draw-3d-text"),
                None,
                None,
            )?;

            // Flat, in two groups: stroke tools (open paths), then closed
            // shapes — future curve tools join the first group, new
            // primitives the second. Submenus return only if the menu ever
            // outgrows ~10 items (the old "Lines" submenu held ONE item).
            let draw_menu = SubmenuBuilder::with_id(handle, "submenu:draw", lbl!("submenu:draw"))
                .item(&draw_line)
                .item(&draw_arc)
                .separator()
                .item(&draw_rect)
                .item(&draw_circle)
                .item(&draw_polygon)
                .separator()
                .item(&draw_3d_text)
                .build()?;

            // ----------------------------------------------------------------
            // Tools menu
            // ----------------------------------------------------------------
            let tool_select = check_item(
                handle,
                &mut checks,
                "tool-select",
                lbl!("tool-select"),
                None,
                Some("Space"),
            )?;
            let tool_paint = check_item(
                handle,
                &mut checks,
                "tool-paint",
                lbl!("tool-paint"),
                None,
                Some("B"),
            )?;
            let tool_position_texture = check_item(
                handle,
                &mut checks,
                "tool-position-texture",
                lbl!("tool-position-texture"),
                None,
                None,
            )?;
            let tool_move = check_item(
                handle,
                &mut checks,
                "tool-move",
                lbl!("tool-move"),
                Some("CmdOrCtrl+0"),
                Some("M"),
            )?;
            let tool_rotate = check_item(
                handle,
                &mut checks,
                "tool-rotate",
                lbl!("tool-rotate"),
                Some("CmdOrCtrl+8"),
                Some("Q"),
            )?;
            let tool_scale = check_item(
                handle,
                &mut checks,
                "tool-scale",
                lbl!("tool-scale"),
                Some("CmdOrCtrl+9"),
                Some("S"),
            )?;
            let tool_pushpull = check_item(
                handle,
                &mut checks,
                "tool-pushpull",
                lbl!("tool-pushpull"),
                Some("CmdOrCtrl+="),
                Some("P"),
            )?;
            let tool_follow_me = check_item(
                handle,
                &mut checks,
                "tool-follow-me",
                lbl!("tool-follow-me"),
                None,
                None,
            )?;

            let tool_offset = check_item(
                handle,
                &mut checks,
                "tool-offset",
                lbl!("tool-offset"),
                None,
                Some("F"),
            )?;
            let tool_tape_measure = check_item(
                handle,
                &mut checks,
                "tool-tape-measure",
                lbl!("tool-tape-measure"),
                Some("CmdOrCtrl+D"),
                Some("T"),
            )?;
            let tool_dimension = check_item(
                handle,
                &mut checks,
                "tool-dimension",
                lbl!("tool-dimension"),
                None,
                None,
            )?;
            let tool_protractor = check_item(
                handle,
                &mut checks,
                "tool-protractor",
                lbl!("tool-protractor"),
                None,
                None,
            )?;
            let tool_slice = check_item(
                handle,
                &mut checks,
                "tool-slice",
                lbl!("tool-slice"),
                None,
                None,
            )?;
            let tool_section_plane = check_item(
                handle,
                &mut checks,
                "tool-section-plane",
                lbl!("tool-section-plane"),
                None,
                None,
            )?;
            let tool_edit_vertex = check_item(
                handle,
                &mut checks,
                "tool-edit-vertex",
                lbl!("tool-edit-vertex"),
                None,
                None,
            )?;
            let tool_axes = check_item(
                handle,
                &mut checks,
                "tool-axes",
                lbl!("tool-axes"),
                None,
                None,
            )?;
            let tool_text = check_item(
                handle,
                &mut checks,
                "tool-text",
                lbl!("tool-text"),
                None,
                None,
            )?;

            // Grouped by job, so future tools have an obvious slot: select,
            // transform (mirror/array variants land here), shape geometry
            // (chamfer/fillet land here), measure & annotate, materials,
            // model-space utilities.
            let tools_menu =
                SubmenuBuilder::with_id(handle, "submenu:tools", lbl!("submenu:tools"))
                    .item(&tool_select)
                    .separator()
                    .item(&tool_move)
                    .item(&tool_rotate)
                    .item(&tool_scale)
                    .separator()
                    .item(&tool_pushpull)
                    .item(&tool_follow_me)
                    .item(&tool_offset)
                    .item(&tool_slice)
                    .item(&tool_edit_vertex)
                    .separator()
                    .item(&tool_tape_measure)
                    .item(&tool_protractor)
                    .item(&tool_dimension)
                    .item(&tool_text)
                    .separator()
                    .item(&tool_paint)
                    .item(&tool_position_texture)
                    .separator()
                    .item(&tool_axes)
                    .item(&tool_section_plane)
                    .build()?;

            // ----------------------------------------------------------------
            // Camera menu
            // ----------------------------------------------------------------
            // Camera tools: SketchUp's real O / H / Z on non-Mac (
            // verified against the official 2024 Windows Quick Reference
            // Card); macOS keeps its pre-existing Cmd-combos.
            let cam_orbit = check_item(
                handle,
                &mut checks,
                "cam-orbit",
                lbl!("cam-orbit"),
                Some("CmdOrCtrl+B"),
                Some("O"),
            )?;
            let cam_pan = check_item(
                handle,
                &mut checks,
                "cam-pan",
                lbl!("cam-pan"),
                Some("CmdOrCtrl+R"),
                Some("H"),
            )?;
            let cam_zoom = check_item(
                handle,
                &mut checks,
                "cam-zoom",
                lbl!("cam-zoom"),
                Some("CmdOrCtrl+\\"),
                Some("Z"),
            )?;
            // Position Camera / Walk / Look Around (docs/design/camera.md §4):
            // the first REAL camera tools (Orbit/Pan/Zoom above are just
            // OrbitControls mouse-button remaps). Sticky like Orbit/Pan/Zoom
            // (`check_item`, not a one-shot `MenuItemBuilder` — they stay
            // active until Escape/another tool switch, unlike Zoom Window
            // below), and — like Zoom Window — no default bare-letter
            // shortcut in v1 (SketchUp ships none for these either).
            let cam_position_camera = check_item(
                handle,
                &mut checks,
                "cam-position-camera",
                lbl!("cam-position-camera"),
                None,
                None,
            )?;
            let cam_walk = check_item(
                handle,
                &mut checks,
                "cam-walk",
                lbl!("cam-walk"),
                None,
                None,
            )?;
            let cam_look_around = check_item(
                handle,
                &mut checks,
                "cam-look-around",
                lbl!("cam-look-around"),
                None,
                None,
            )?;
            // Fly (docs/design/fly-navigation.md): holding the right mouse
            // button flies under any tool with no menu involved; this entry
            // LATCHES the same flight on for trackpads (§10) — a CHECK item
            // (on/off, synced from the webview), not a tool. No accelerator:
            // its keys (W A S D Q E) are live only while flying.
            let cam_fly = check_item(handle, &mut checks, "cam-fly", lbl!("cam-fly"), None, None)?;
            // Zoom Window (docs/design/camera.md §3): one-shot rectangle-drag
            // reframe, springs back to Select — no default shortcut in v1
            // (SketchUp ships none for it either), same as the tool registry.
            // A PLAIN item, not `check_item` — unlike Orbit/Pan/Zoom this
            // mode is never "sticky": it always springs back to Select right
            // after the drag, so a persistent checkmark would be misleading
            // (matches the web `MenuBar`'s plain `MenuItem` for the same
            // entry).
            let cam_zoom_window =
                MenuItemBuilder::with_id("cam-zoom-window", lbl!("cam-zoom-window"))
                    .build(handle)?;
            let cam_zoom_extents =
                MenuItemBuilder::with_id("cam-zoom-extents", lbl!("cam-zoom-extents"))
                    .build(handle)?;
            // No standalone fov-entry menu item (camera-playtest2.md §2 —
            // Kurt's playtest call): the lens is reachable only through
            // Zoom (typed-degree/mm entry, or Shift-drag/wheel — design
            // §2-3), never a separate menu entry.
            // Parallel Projection (design §1): perspective <-> parallel
            // toggle, visually stable at the orbit target.
            let cam_parallel_projection = check_item(
                handle,
                &mut checks,
                "cam-parallel-projection",
                lbl!("cam-parallel-projection"),
                None,
                None,
            )?;

            // Standard Views — axis-aligned + isometric framings.
            let view_top =
                MenuItemBuilder::with_id("cam-view-top", lbl!("cam-view-top")).build(handle)?;
            let view_bottom = MenuItemBuilder::with_id("cam-view-bottom", lbl!("cam-view-bottom"))
                .build(handle)?;
            let view_front =
                MenuItemBuilder::with_id("cam-view-front", lbl!("cam-view-front")).build(handle)?;
            let view_back =
                MenuItemBuilder::with_id("cam-view-back", lbl!("cam-view-back")).build(handle)?;
            let view_left =
                MenuItemBuilder::with_id("cam-view-left", lbl!("cam-view-left")).build(handle)?;
            let view_right =
                MenuItemBuilder::with_id("cam-view-right", lbl!("cam-view-right")).build(handle)?;
            let view_iso =
                MenuItemBuilder::with_id("cam-view-iso", lbl!("cam-view-iso")).build(handle)?;
            let standard_views = SubmenuBuilder::with_id(
                handle,
                "submenu:standard-views",
                lbl!("submenu:standard-views"),
            )
            .item(&view_top)
            .item(&view_bottom)
            .item(&view_front)
            .item(&view_back)
            .item(&view_left)
            .item(&view_right)
            .separator()
            .item(&view_iso)
            .build()?;

            let camera_menu =
                SubmenuBuilder::with_id(handle, "submenu:camera", lbl!("submenu:camera"))
                    .item(&cam_orbit)
                    .item(&cam_pan)
                    .item(&cam_zoom)
                    .separator()
                    .item(&cam_position_camera)
                    .item(&cam_walk)
                    .item(&cam_look_around)
                    .item(&cam_fly)
                    .separator()
                    .item(&cam_zoom_window)
                    .item(&cam_zoom_extents)
                    .separator()
                    .item(&cam_parallel_projection)
                    .separator()
                    .item(&standard_views)
                    .build()?;

            // ----------------------------------------------------------------
            // Window menu
            // ----------------------------------------------------------------
            // Window holds actual windows (HIG): Minimize/Zoom, the Library
            // (a real second window on desktop, unlike the View-menu panes),
            // and the dynamic open-windows tail appended by
            // rebuild_window_menu_tail. The pane toggles that used to sit
            // here live in View now.
            //
            // Bare Shift+L (the in-app bare-letter shortcut) can't be a safe
            // native accelerator — it would fire while typing in any native
            // text field — so the menu advertises the same modifier chord
            // the pane toggles use instead.
            let win_library = check_item(
                handle,
                &mut checks,
                "win-library",
                lbl!("win-library"),
                Some("Shift+CmdOrCtrl+L"),
                Some("Shift+CmdOrCtrl+L"),
            )?;
            let window_menu =
                SubmenuBuilder::with_id(handle, "submenu:window", lbl!("submenu:window"))
                    .item(&predef_minimize)
                    .item(&predef_maximize)
                    .separator()
                    .item(&win_library)
                    .build()?;

            // ----------------------------------------------------------------
            // Help menu
            // ----------------------------------------------------------------
            // Debug Log lives here rather than among the document panes above
            // — it isn't a common pane, it's a diagnostic tool.
            let win_debug_log = check_item(
                handle,
                &mut checks,
                "win-debug-log",
                lbl!("win-debug-log"),
                None,
                None,
            )?;
            let help_report_bug =
                MenuItemBuilder::with_id("help-report-bug", lbl!("help-report-bug"))
                    .build(handle)?;
            // Opens the online user guide in the default browser — dispatched
            // to the frontend ("open-guide"), which routes through the opener
            // plugin (the WelcomeScreen's guide-link pattern).
            let help_guide =
                MenuItemBuilder::with_id("help-guide", lbl!("help-guide")).build(handle)?;
            // Follow a bug report: opens the Sent Reports list. Relabeled
            // "Sent Reports (n new)…" from follow.rs when replies are unread.
            let help_sent_reports = MenuItemBuilder::with_id(
                follow::MENU_ID,
                follow::menu_text_with(lbl!("help-sent-reports"), lbl!("help-sent-reports-new"), 0),
            )
            .build(handle)?;

            let help_menu = SubmenuBuilder::with_id(handle, "submenu:help", lbl!("submenu:help"))
                .item(&help_guide)
                .separator()
                .item(&help_report_bug)
                .item(&help_sent_reports)
                .item(&win_debug_log)
                .build()?;
            // ----------------------------------------------------------------
            // Assemble and set the menu bar
            // ----------------------------------------------------------------
            let menu = MenuBuilder::new(handle)
                .item(&app_menu)
                .item(&file_menu)
                .item(&edit_menu)
                .item(&object_menu)
                .item(&view_menu)
                .item(&camera_menu)
                .item(&draw_menu)
                .item(&tools_menu)
                .item(&window_menu)
                .item(&help_menu)
                .build()?;

            // Attach the menu. macOS shows a single global menu bar owned by the
            // app, so the app-level menu is correct there.
            #[cfg(target_os = "macos")]
            app.set_menu(menu)?;
            // Registering as THE help menu makes macOS add its standard
            // search field (which searches menu items and help) at the top.
            // MUST run after set_menu: muda resolves the NSMenu through the
            // app's attached main menu, so an earlier call silently no-ops.
            #[cfg(target_os = "macos")]
            help_menu.set_as_help_menu_for_nsapp()?;
            // Windows and Linux both drive the menus from the in-app HTML
            // MenuBar (the native menu is built for parity but not attached):
            // Linux settled on this after the experiments — the native GTK
            // menubar can only stack ABOVE the custom title bar and can't match
            // Hew's theme beyond a dark/light flip.
            //
            // Window chrome, however, splits by platform. Linux/WebKitGTK can't
            // repaint the server-side titlebar after `setTitle` (Wayland's
            // stale-title bug), so it goes borderless and draws its own
            // TitleBar. Windows keeps NATIVE decorations: WebView2 repaints the
            // native caption fine, the OS title bar reflects `setTitle`, and the
            // native caption buttons behave exactly as users expect — no reason
            // to reimplement min/maximize/close in-app there.
            #[cfg(any(target_os = "windows", target_os = "linux"))]
            {
                let _ = menu; // built for parity; not attached on Windows/Linux.
                #[cfg(target_os = "linux")]
                if let Some(main_window) = app.webview_windows().values().next() {
                    let _ = main_window.set_decorations(false);
                }
            }

            // ----------------------------------------------------------------
            // Managed state: approved paths — seeded from the persisted
            // recents (each entry was user-picked in a past session) and the
            // argv open; every other approval flows through a dialog pick,
            // file association, or drag-drop at the moment it happens.
            // ----------------------------------------------------------------
            app.manage(Mutex::new(ApprovedPaths::default()));
            // System font enumeration state for the 3D Text picker — see
            // fonts.rs's top-of-file doc comment for the token registry's
            // security role.
            app.manage(fonts::SystemFontState::default());
            // "Open on Phone" server setting + HTTP client — loaded once from
            // server.json; only `set_server_setting` ever changes it.
            app.manage(relay_client::RelayState::new(relay_client::load_setting(
                handle,
            )));
            // Recents are read+write approved so opening one and saving back
            // works with no dialog, exactly as before — but ONLY when the
            // entry still resolves to a real .hew file. Combined with
            // push_recent's own gate, this keeps the auto-approval from ever
            // covering a non-document path (a launch agent, a shell rc file),
            // even if recents.json was hand-edited on disk (audit sec-desktop).
            for path in &recents {
                if is_recentable_document_file(path) {
                    approve_file(handle, Path::new(path), true);
                }
            }
            // ----------------------------------------------------------------
            // Managed state: recents + submenu handle
            // ----------------------------------------------------------------
            app.manage(Mutex::new(RecentState {
                paths: recents,
                submenu: open_recent_submenu,
            }));

            // ----------------------------------------------------------------
            // Managed state: stateful menu handles, document-window counter,
            // multi-window routing/recovery bookkeeping, window-state cache
            // ----------------------------------------------------------------
            app.manage(Mutex::new(MenuHandles {
                checks,
                items: gated,
                predefined,
            }));
            app.manage(Mutex::new(labels));
            app.manage(ClipboardState(Mutex::new(None)));
            app.manage(fly_cursor::FlyCursorState::default());
            // Seeded past any main-N recovery slot a crashed session left, so
            // this session's fresh window labels never alias an unclaimed
            // snapshot slot (see main_slot_suffix).
            let config_dir = handle.path().app_config_dir().ok();
            app.manage(Mutex::new(WindowCounter(seed_window_counter(
                config_dir.as_deref(),
            ))));
            app.manage(Mutex::new(ReadyWindows(HashSet::new())));
            app.manage(Mutex::new(DocumentPaths(HashMap::new())));
            app.manage(Mutex::new(ActiveWindow(None)));
            app.manage(Mutex::new(PendingRecovery(HashMap::new())));
            app.manage(Mutex::new(PendingWindowOpen(HashMap::new())));
            app.manage(Mutex::new(ProtectedSlots(HashSet::new())));
            app.manage(Mutex::new(WindowStateCache {
                last_normal: None,
                last_write: None,
            }));
            // Handle to the Window submenu's dynamic tail (see
            // `rebuild_window_menu_tail`) — built on every platform since
            // `window_menu` itself is (menu parity), even though only macOS
            // actually attaches and rebuilds it.
            // `WindowMenuState` is the LAST state `setup` manages, so its
            // presence is `deliver_open`'s "setup is done" signal. The
            // pending-open lock is held across seeding, approving, and
            // managing it: a Windows/Linux second-instance open (another
            // thread) either buffers first — and is approved here — or
            // waits and then sees a fully set-up app.
            {
                let pending_state = app.state::<Mutex<PendingOpen>>();
                let mut pending = pending_state.lock().unwrap_or_else(|e| e.into_inner());
                // The argv path (Windows/Linux) seeds the buffer; a macOS
                // cold-start open event may already be sitting in it. Either
                // way the path came from the OS on the user's behalf, so
                // approve it like any other file-association open.
                if pending.0.is_none() {
                    pending.0 = argv_path.clone();
                }
                if let Some(path) = &pending.0 {
                    approve_file(handle, Path::new(path), true);
                }
                app.manage(Mutex::new(WindowMenuState {
                    submenu: window_menu,
                    dynamic_count: 0,
                }));
            }
            // Seed the Window-menu tail / broadcast with this launch's first
            // window before anything triggers the usual create/focus refresh.
            refresh_window_list(handle);

            // ----------------------------------------------------------------
            // Main window geometry: restore the persisted size/position, or
            // default to ~2/3 of the screen on first run.
            // ----------------------------------------------------------------
            if let Some(main_window) = app.get_webview_window("main") {
                apply_initial_window_state(handle, &main_window);
                let _ = main_window.show();
            }

            // Serialize update checks (launch + manual) behind one flag, then
            // kick off the silent launch check (updater-enabled builds only):
            // quiet unless an update is actually available, at which point the
            // user is prompted to download and restart.
            #[cfg(feature = "updater")]
            {
                app.manage(updater::UpdateGuard::default());
                let handle = handle.clone();
                tauri::async_runtime::spawn(async move { updater::run_check(handle, false).await });
            }

            // Follow a bug report (docs/design/follow-a-report.md §6.3): once
            // per process, 15 s from now, poll followed reports that are due.
            follow::spawn_launch_check(handle.clone());

            // `--live` (docs/agents/HEW_API.md §11.2): sweep stale discovery files
            // from a previous unclean exit, then bind this launch's own
            // socket and publish its discovery file. Best-effort — see
            // live::start's doc comment for why a failure here never blocks
            // the app from starting normally.
            live::start(handle);

            Ok(())
        })
        // Per-window bookkeeping: focus tracking for menu/open routing,
        // drag-drop path approval, readiness cleanup, and main-window
        // geometry persistence (extra "main-N" document windows are
        // transient and don't overwrite the primary window's saved state).
        .on_window_event(|window, event| {
            let app = window.app_handle();
            match event {
                tauri::WindowEvent::Focused(true) if is_document_window_label(window.label()) => {
                    if let Ok(mut active) = app.state::<Mutex<ActiveWindow>>().lock() {
                        active.0 = Some(window.label().to_string());
                    }
                    refresh_window_list(app);
                }
                // A flight's hidden, frozen cursor must not outlive the
                // window's focus (src/fly_cursor.rs).
                tauri::WindowEvent::Focused(false) => {
                    fly_cursor::unlock_if_locked(app, window);
                }
                // Dropped files are a user gesture — approve them like a
                // dialog pick (write too: a dropped .hew is opened, and Save
                // then writes back to it).
                tauri::WindowEvent::DragDrop(tauri::DragDropEvent::Drop { paths, .. }) => {
                    for path in paths {
                        approve_file(app, path, true);
                    }
                }
                tauri::WindowEvent::Destroyed => {
                    fly_cursor::unlock_if_locked(app, window);
                    if let Ok(mut ready) = app.state::<Mutex<ReadyWindows>>().lock() {
                        ready.0.remove(window.label());
                    }
                    if let Ok(mut paths) = app.state::<Mutex<DocumentPaths>>().lock() {
                        paths.0.remove(window.label());
                    }
                    if let Ok(mut active) = app.state::<Mutex<ActiveWindow>>().lock() {
                        if active.0.as_deref() == Some(window.label()) {
                            active.0 = None;
                        }
                    }
                    // The Library window may be holding the destroyed
                    // document's placements — ask the surviving active
                    // window to re-push (the gate in
                    // `library_push_placements` picks the right one).
                    if is_document_window_label(window.label()) {
                        let _ = app.emit("library-placements-request", ());
                        // An auxiliary browser must not outlive the last
                        // document window (playtest round 2: quitting the
                        // documents left a live Library window keeping the
                        // app running) — its whole purpose is acting on a
                        // document.
                        let docs_remaining = app
                            .webview_windows()
                            .values()
                            .any(|w| is_document_window_label(w.label()));
                        if !docs_remaining {
                            if let Some(lib) = app.get_webview_window("library") {
                                let _ = lib.close();
                            }
                            // Settings is declared equally auxiliary by
                            // is_document_window_label — same rule.
                            if let Some(settings) = app.get_webview_window("settings") {
                                let _ = settings.close();
                            }
                        }
                    }
                    // A recovery window that dies before claiming leaves its
                    // assignment unconsumed; drop it (the slot files stay on
                    // disk and are re-offered next launch).
                    if let Ok(mut pending) = app.state::<Mutex<PendingRecovery>>().lock() {
                        pending.0.remove(window.label());
                    }
                    // Likewise for a window closed before claiming its
                    // open_in_new_window path — nothing left to deliver it to.
                    if let Ok(mut pending) = app.state::<Mutex<PendingWindowOpen>>().lock() {
                        pending.0.remove(window.label());
                    }
                    refresh_window_list(app);
                }
                _ => {}
            }
            if window.label() != "main" {
                return;
            }
            match event {
                tauri::WindowEvent::Resized(_) | tauri::WindowEvent::Moved(_) => {
                    save_window_state(app, window, false);
                }
                // Forced (unthrottled): the final geometry of a close must
                // never be dropped by the Moved/Resized throttle.
                tauri::WindowEvent::CloseRequested { .. } => {
                    save_window_state(app, window, true);
                }
                _ => {}
            }
        })
        // Map menu-item ids to action strings and emit them to exactly one
        // document window (focused, else last-focused — see emit_to_active;
        // a broadcast would run Undo/Delete/Open in every open document).
        .on_menu_event(|app, event| {
            let id = event.id().as_ref();
            // Library context-menu picks route back to the library window
            // (the only place that pops `libmenu:` menus).
            if let Some(action) = id.strip_prefix("libmenu:") {
                let _ = app.emit_to("library", "library-menu", action);
                return;
            }
            // macOS's menu bar is APP-wide: its accelerators fire whichever
            // window is key. With an AUXILIARY window focused (Library,
            // Settings), document-directed actions must not reach the
            // hidden document window — Cmd+W there would close a window
            // the user isn't looking at (adversarial review). The
            // Mac-correct split: Close closes the FOCUSED auxiliary
            // window; window-management/new-window actions pass through;
            // every document-mutating action is dropped.
            if let Some(focused) = app
                .webview_windows()
                .into_values()
                .find(|w| w.is_focused().unwrap_or(false))
            {
                if !is_document_window_label(focused.label()) {
                    match id {
                        "file-close" => {
                            let _ = focused.close();
                            return;
                        }
                        // The text-editing items belong to whichever text
                        // field is focused — an auxiliary window's own
                        // (Settings' server address, Library's search). A
                        // WebKit field on macOS gets Cmd+C/X/V/A only
                        // through these items, so they go to the FOCUSED
                        // window, which applies them to its field (see
                        // useTextFieldMenuActions.ts); the document window
                        // never sees them.
                        "edit-cut" | "edit-copy" | "edit-paste" | "edit-select-all" => {
                            let _ = app.emit_to(focused.label(), "menu-action", id);
                            return;
                        }
                        // Window management and new-window creation behave
                        // sensibly from anywhere.
                        "file-new" | "file-open" | "app-settings" | "win-library"
                        | "app-check-updates" => {}
                        _ if id.starts_with("win-focus:") || id.starts_with("recent:") => {}
                        // Anything else would mutate or reframe a document
                        // the user cannot see — drop it.
                        _ => return,
                    }
                }
            }
            if let Some(path) = id.strip_prefix("recent:") {
                emit_to_active(app, "menu-open-path", path);
                return;
            }
            if id == "recent-clear" {
                let _ = clear_recent(app.clone());
                return;
            }
            if let Some(label) = id.strip_prefix("win-focus:") {
                let _ = focus_window_by_label(app, label);
                return;
            }
            // The macOS "Check for Updates…" item runs the whole flow shell-side
            // rather than emitting to the webview (Windows/Linux reach the same
            // `check_for_updates` command from the in-app menu bar).
            #[cfg(feature = "updater")]
            if id == "app-check-updates" {
                let app = app.clone();
                tauri::async_runtime::spawn(async move { updater::run_check(app, true).await });
                return;
            }
            let action = match id {
                "app-settings" => "open-settings",
                "file-new" => "new",
                "file-open" => "open",
                "file-import" => "import",
                "file-import-image" => "import-image",
                "file-export" => "export",
                "file-print" => "print",
                "file-open-on-phone" => "open-on-phone",
                "file-save" => "save",
                "file-save-as" => "save-as",
                "file-revert" => "revert-to-saved",
                "file-save-to-library" => "save-to-library-doc",
                "file-purge-unused" => "purge-unused",
                "file-close" => "close",
                "edit-undo" => "undo",
                "edit-redo" => "redo",
                "edit-cut" => "edit-cut",
                "edit-copy" => "edit-copy",
                "edit-paste" => "edit-paste",
                "edit-paste-in-place" => "edit-paste-in-place",
                "edit-delete" => "edit-delete",
                "edit-delete-guides" => "edit-delete-guides",
                "edit-select-all" => "edit-select-all",
                "edit-select-none" => "edit-select-none",
                "edit-invert-selection" => "edit-invert-selection",
                "edit-group" => "edit-group",
                "edit-ungroup" => "edit-ungroup",
                "edit-make-component" => "edit-make-component",
                "edit-place-copy" => "edit-place-copy",
                "edit-save-selection-to-library" => "save-to-library",
                "edit-explode" => "edit-explode",
                "edit-make-unique" => "edit-make-unique",
                "edit-union" => "edit-union",
                "edit-subtract" => "edit-subtract",
                "edit-intersect" => "edit-intersect",
                "object-make-solid" => "make-solid",
                "object-model-health" => "model-health",
                "view-axes" => "toggle-axes",
                "view-reset-axes" => "reset-axes",
                "view-grid" => "toggle-grid",
                "view-guides" => "toggle-guides",
                "view-section-plane" => "toggle-section-active",
                "view-xray" => "toggle-xray",
                "view-wireframe" => "toggle-wireframe",
                "scenes-add" => "scenes-add",
                "scenes-update" => "scenes-update",
                "scenes-next" => "scenes-next",
                "scenes-previous" => "scenes-previous",
                "scenes-transitions" => "scenes-transitions",
                "view-palette" => "open-palette",
                "draw-rectangle" => "tool-rectangle",
                "draw-circle" => "tool-circle",
                "draw-polygon" => "tool-polygon",
                "draw-arc" => "tool-arc",
                "draw-line" => "tool-line",
                "draw-3d-text" => "draw-3d-text",
                "tool-select" => "tool-select",
                "tool-paint" => "tool-paint",
                "tool-position-texture" => "tool-position-texture",
                "tool-move" => "tool-move",
                "tool-rotate" => "tool-rotate",
                "tool-scale" => "tool-scale",
                "tool-pushpull" => "tool-pushpull",
                "tool-follow-me" => "tool-follow-me",
                "tool-offset" => "tool-offset",
                "tool-tape-measure" => "tool-tape-measure",
                "tool-dimension" => "tool-dimension",
                "tool-protractor" => "tool-protractor",
                "tool-slice" => "tool-slice",
                "tool-section-plane" => "tool-section-plane",
                "tool-edit-vertex" => "tool-edit-vertex",
                "tool-axes" => "tool-axes",
                "tool-text" => "tool-text",
                "cam-orbit" => "tool-orbit",
                "cam-pan" => "tool-pan",
                "cam-zoom" => "tool-zoom",
                "cam-position-camera" => "tool-position-camera",
                "cam-walk" => "tool-walk",
                "cam-look-around" => "tool-look-around",
                "cam-fly" => "toggle-fly",
                "cam-zoom-window" => "tool-zoom-window",
                "cam-zoom-extents" => "zoom-extents",
                "cam-parallel-projection" => "toggle-parallel-projection",
                "cam-view-top" => "view-top",
                "cam-view-bottom" => "view-bottom",
                "cam-view-front" => "view-front",
                "cam-view-back" => "view-back",
                "cam-view-left" => "view-left",
                "cam-view-right" => "view-right",
                "cam-view-iso" => "view-iso",
                "view-compact-rail" => "toggle-compact-rail",
                "view-tray" => "toggle-tray",
                "view-dock" => "toggle-dock",
                "win-model-info" => "toggle-model-info",
                "win-materials" => "toggle-materials",
                "win-components" => "toggle-components",
                "win-tags" => "toggle-tags",
                "win-scenes" => "toggle-scenes",
                "win-changes" => "toggle-changes",
                "win-object-info" => "toggle-object-info",
                "win-library" => "toggle-library",
                "win-debug-log" => "toggle-debug-log",
                "help-report-bug" => "report-bug",
                "help-sent-reports" => "sent-reports",
                "help-guide" => "open-guide",
                _ => return,
            };
            emit_to_active(app, "menu-action", action);
        })
        .build(tauri::generate_context!())
        .expect("error while building Hew desktop")
        .run(|app, event| {
            // A quit that never routes through CloseRequested must still
            // capture the final main-window geometry. macOS Cmd+Q uses the
            // predefined Quit item's `terminate:` selector, which surfaces
            // only as RunEvent::Exit (never ExitRequested) with the windows
            // still alive — hook both.
            if matches!(
                &event,
                tauri::RunEvent::ExitRequested { .. } | tauri::RunEvent::Exit
            ) {
                if let Some(main_window) = app.get_webview_window("main") {
                    save_window_state(app, &main_window.as_ref().window_ref(), true);
                }
                // "removes it on exit" (docs/agents/HEW_API.md §11.2) — a crash
                // (no RunEvent at all) is instead cleaned up by the NEXT
                // launch's live::start sweeping stale discovery files.
                live::cleanup();
            }
            // macOS: intercept "open document" Apple events (warm + cold
            // start). deliver_open emits to the active document window once
            // some webview is ready, and buffers only before that — so a
            // warm open can never strand a stale path in the buffer for a
            // later File ▸ New window to swallow, and an open while the app
            // is inactive lands in one window instead of broadcasting.
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Opened { urls } = &event {
                // This arm runs INSIDE tao's `application:openURLs:` objc
                // callback — an `extern "C"` frame a Rust panic cannot
                // unwind through, so any panic here aborts the whole app
                // (observed in the wild as a SIGABRT double-clicking a
                // `.hew` at cold launch). Opening a file must never be
                // able to take the app down: contain and log instead.
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    for url in urls {
                        if url.scheme() != "file" {
                            continue;
                        }
                        // Convert file:// URL to filesystem path (percent-decode).
                        let Ok(path) = url.to_file_path() else {
                            continue;
                        };
                        let path_str = path.to_string_lossy().to_string();
                        if !is_document_name(&path_str) {
                            continue;
                        }
                        deliver_open(app, &path_str);
                    }
                }));
                if result.is_err() {
                    eprintln!("open-urls delivery panicked; the open was dropped");
                }
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    // `write_file_bytes` itself needs a live `AppHandle` for its
    // `write_allowed` check, which a plain `#[test]` can't construct (see
    // the library_* tests below for the same limitation) — `percent_decode`
    // is the part of it that's pure and worth pinning directly. The
    // approval gate itself is exactly `write_file`'s own `write_allowed`
    // call, unchanged and untouched by this addition.
    #[test]
    fn percent_decode_round_trips_encodeuricomponent_output() {
        let cases = [
            "/Users/kurt/Desktop/hew-bug-report-2026-01-01.json",
            "C:\\Users\\kurt\\Desktop\\report.json",
            "/tmp/héllo wörld/plus+sign/report.json",
            "/tmp/100%/report.json",
        ];
        for path in cases {
            // JS-side encoding this test stands in for: `encodeURIComponent(path)`.
            let encoded = path
                .bytes()
                .map(|b| {
                    let c = b as char;
                    if c.is_ascii_alphanumeric() || "-_.!~*'()".contains(c) {
                        c.to_string()
                    } else {
                        format!("%{b:02X}")
                    }
                })
                .collect::<String>();
            assert_eq!(
                percent_decode(&encoded).unwrap(),
                path,
                "round-trip of {path:?}"
            );
        }
    }

    #[test]
    fn percent_decode_rejects_a_truncated_or_invalid_escape() {
        assert!(percent_decode("abc%2").is_err());
        assert!(percent_decode("abc%zz").is_err());
    }

    #[test]
    fn slot_paths_maps_labels_and_legacy() {
        let dir = Path::new("/cfg");
        let (hew, json) = slot_paths(dir, "main");
        assert_eq!(hew, Path::new("/cfg/recovery-main.hew"));
        assert_eq!(json, Path::new("/cfg/recovery-main.json"));
        let (hew, json) = slot_paths(dir, "main-3");
        assert_eq!(hew, Path::new("/cfg/recovery-main-3.hew"));
        assert_eq!(json, Path::new("/cfg/recovery-main-3.json"));
        let (hew, json) = slot_paths(dir, LEGACY_SLOT);
        assert_eq!(hew, Path::new("/cfg/recovery.hew"));
        assert_eq!(json, Path::new("/cfg/recovery.json"));
    }

    #[test]
    fn slot_source_path_sits_beside_the_snapshot() {
        let (hew, _) = slot_paths(Path::new("/cfg"), "main-3");
        assert_eq!(
            slot_source_path(&hew),
            Path::new("/cfg/recovery-main-3.skpsrc")
        );
        // The companion is never mistaken for a snapshot.
        assert_eq!(slot_of_filename("recovery-main-3.skpsrc"), None);
    }

    #[test]
    fn document_names_are_hew_or_skp() {
        assert!(is_document_name("/a/b/Model.hew"));
        assert!(is_document_name("C:\\x\\House.SKP"));
        assert!(!is_document_name("/a/b/Model.skb"));
        assert!(!is_document_name("/a/b/Model.skp.bak"));
    }

    #[test]
    fn slot_of_filename_recognizes_snapshots_only() {
        assert_eq!(
            slot_of_filename("recovery.hew").as_deref(),
            Some(LEGACY_SLOT)
        );
        assert_eq!(
            slot_of_filename("recovery-main.hew").as_deref(),
            Some("main")
        );
        assert_eq!(
            slot_of_filename("recovery-main-12.hew").as_deref(),
            Some("main-12")
        );
        // Sidecars, non-snapshots, and slot-name aliasing are all rejected.
        assert_eq!(slot_of_filename("recovery-main.json"), None);
        assert_eq!(slot_of_filename("recovery.json"), None);
        assert_eq!(slot_of_filename("recents.json"), None);
        assert_eq!(slot_of_filename("recovery-.hew"), None);
        assert_eq!(slot_of_filename("recovery-legacy.hew"), None);
    }

    #[test]
    fn slot_roundtrip_survives_list_then_claim() {
        // Every filename produced by slot_paths must list back to the same
        // slot — a mismatch would make a snapshot unclaimable.
        let dir = Path::new("/cfg");
        for slot in ["main", "main-2", LEGACY_SLOT] {
            let (hew, _) = slot_paths(dir, slot);
            let name = hew.file_name().unwrap().to_str().unwrap();
            assert_eq!(slot_of_filename(name).as_deref(), Some(slot));
        }
    }

    #[test]
    fn main_slot_suffix_reads_document_window_labels() {
        assert_eq!(main_slot_suffix("main"), Some(1));
        assert_eq!(main_slot_suffix("main-2"), Some(2));
        assert_eq!(main_slot_suffix("main-17"), Some(17));
        // Deferred side-slots still pin the counter past their base label.
        assert_eq!(main_slot_suffix("main-3-deferred"), Some(3));
        assert_eq!(main_slot_suffix("main-deferred"), None);
        assert_eq!(main_slot_suffix(LEGACY_SLOT), None);
        assert_eq!(main_slot_suffix("settings"), None);
        assert_eq!(main_slot_suffix("main-"), None);
    }

    #[test]
    fn overlap_measures_intersection_length() {
        assert_eq!(overlap(0, 10, 5, 20), 5);
        assert_eq!(overlap(0, 10, 10, 20), 0); // touching, not overlapping
        assert_eq!(overlap(0, 10, -5, 3), 3);
        assert_eq!(overlap(0, 10, 20, 30), 0); // disjoint clamps to 0
        assert_eq!(overlap(-10, -2, -8, -4), 4); // fully negative coords
    }

    // `resolved_window_title` is the fix for the window-title-refresh bug:
    // `window.set_title` on macOS (`tao`'s `set_title_async`) unconditionally
    // marshals the real `NSWindow.setTitle:` call through
    // `DispatchQueue::main().exec_async`, even when the caller is already on
    // the main thread — so re-reading `w.title()` in the same synchronous
    // command (`set_window_title` -> `refresh_window_list`) observes the
    // title from BEFORE the call, not the one just set. These pin the
    // override that sidesteps that race by using the just-set title
    // directly instead of asking the OS to confirm it.
    //
    // Honest scope note: these tests pin `resolved_window_title` itself —
    // the override-vs-OS-getter precedence, the per-label targeting, and the
    // formatting consistency between the two paths. They do NOT exercise
    // `set_window_title`'s one-line call into `refresh_window_list_after_retitle`
    // (that would need a live `AppHandle`, which the concrete, non-generic
    // Tauri command type can't get from a mock runtime without a much larger
    // refactor of this deadlock-sensitive menu code — considered and
    // rejected as disproportionate to a one-line wiring call). `Retitled`
    // being a named-field struct rather than a bare tuple (see its doc
    // comment) closes the argument-order half of that residual risk at the
    // type level; whether the call itself still happens is unverifiable by
    // any test in this repo and is exactly what the maintainer's real
    // desktop hand-check (already required to prove the fix at all, since
    // no automated test can drive the native NSMenu) re-confirms.

    #[test]
    fn resolved_window_title_prefers_override_for_matching_label() {
        // The retitled window's OS-reported title is stale (this is exactly
        // the race: the OS hasn't applied the real title yet) — the override
        // must win regardless.
        assert_eq!(
            resolved_window_title(
                "main",
                "Untitled — Hew",
                Some(Retitled {
                    label: "main",
                    title: "sketch.hew — Hew"
                })
            ),
            "sketch.hew"
        );
    }

    #[test]
    fn resolved_window_title_ignores_override_for_a_different_label() {
        // A sibling window's own retitle must never leak into this window's
        // entry — only an exact label match applies the override.
        assert_eq!(
            resolved_window_title(
                "main-2",
                "Untitled — Hew",
                Some(Retitled {
                    label: "main",
                    title: "sketch.hew — Hew"
                })
            ),
            "Untitled"
        );
    }

    #[test]
    fn resolved_window_title_falls_back_to_os_title_with_no_override() {
        assert_eq!(
            resolved_window_title("main", "• sketch.hew — Hew", None),
            "• sketch.hew"
        );
    }

    #[test]
    fn resolved_window_title_strips_suffix_on_both_paths_identically() {
        // The override path and the OS-getter fallback path must format
        // identically (both go through `strip_hew_suffix`) — a mismatch here
        // would make the Window-menu list's formatting depend on WHICH path
        // supplied a given entry, which is exactly the kind of inconsistency
        // this fix must not introduce.
        let via_override = resolved_window_title(
            "main",
            "ignored",
            Some(Retitled {
                label: "main",
                title: "x.hew — Hew",
            }),
        );
        let via_os = resolved_window_title("main", "x.hew — Hew", None);
        assert_eq!(via_override, via_os);
    }

    /// `std::env` is process-global; every test below that sets
    /// `HEW_LIBRARY_DIR` holds this lock for its whole scenario (`cargo
    /// test` runs a crate's tests concurrently by default).
    static LIBRARY_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn library_scratch_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "hew-tauri-library-test-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// `library_list`/`library_read`/`library_write` (`main.rs`, above)
    /// are thin wrappers over `library::list`/`library::read`/
    /// `library::write` — this exercises the SAME crate-level calls those
    /// commands make, with `HEW_LIBRARY_DIR` pointed at a real scratch
    /// folder, so a regression in the delegation (a wrong argument order,
    /// a swapped path) would show up here without needing a live Tauri
    /// `AppHandle` (which a plain `#[test]` cannot construct).
    #[test]
    fn library_list_and_read_delegate_to_the_shared_crate_over_a_real_scratch_folder() {
        let _env = LIBRARY_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = library_scratch_dir("list-read");
        unsafe {
            std::env::set_var("HEW_LIBRARY_DIR", &dir);
        }

        // Nothing saved yet: an empty listing, not an error — the same
        // "missing folder lists as empty" contract `library_list` (above)
        // promises.
        assert!(library_list().is_empty());

        let lib_dir = library::LibraryDir::resolve(None);
        assert_eq!(lib_dir.root(), dir.as_path());
        // `library_list`'s `name` field is the raw relative path
        // regardless of whether the file parses as a valid `.hew`
        // container (`ListedItem::rel_path` is set the same way on both
        // the `Ok` and `Err` arms of `crates/library::ops::list_into`),
        // so arbitrary bytes exercise the delegation this test targets
        // without this crate needing a `kernel` dependency of its own.
        let item_bytes = b"not a real .hew container, and that's fine here".to_vec();
        library::write(&lib_dir, "Models/scratch-item.hew", &item_bytes).unwrap();

        let listed = library_list();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "Models/scratch-item.hew");
        assert_eq!(listed[0].size, item_bytes.len() as u64);

        let read_back = library_read("Models/scratch-item.hew".to_string())
            .expect("library_read should find what library_list just listed");
        // `tauri::ipc::Response` doesn't expose its bytes for direct
        // comparison outside the IPC layer, but a successful `Ok` here IS
        // the delegation this test targets — `library_read`'s body is
        // exactly `library::read(&dir, &name)`, already covered byte-for-
        // byte by `crates/library`'s own `write_read_remove_round_trip`.
        let _ = read_back;

        unsafe {
            std::env::remove_var("HEW_LIBRARY_DIR");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `library_thumb_read`/`library_thumb_write` (above) delegate to
    /// `library::read_thumbnail`/`library::write_thumbnail` — the
    /// "not_found" sentinel `tauriLibraryStore.ts` specifically pattern-
    /// matches on is the one behavior worth pinning at the Tauri-command
    /// boundary (the crate's own `Option<Vec<u8>>` return doesn't carry
    /// that string at all; `library_thumb_read` is what invents it).
    #[test]
    fn library_thumb_read_and_write_delegate_and_preserve_the_not_found_sentinel() {
        let _env = LIBRARY_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = library_scratch_dir("thumb");
        unsafe {
            std::env::set_var("HEW_LIBRARY_DIR", &dir);
        }

        let key = "deadbeefcafef00d";
        let missing = library_thumb_read(key.to_string());
        // `tauri::ipc::Response` doesn't implement `Debug`, so
        // `unwrap_err()` (which needs `T: Debug` for its own panic
        // message) doesn't typecheck here — `.err().unwrap()` sidesteps
        // that without losing the assertion.
        assert_eq!(missing.err().unwrap(), "not_found");

        library_thumb_write(key.to_string(), vec![1, 2, 3, 4]).unwrap();
        assert!(library_thumb_read(key.to_string()).is_ok());

        // An invalid key is rejected before it ever reaches
        // `library::read_thumbnail`/`write_thumbnail` — `valid_thumb_key`
        // in `crates/library` is what draws this line; this just confirms
        // the Tauri command actually surfaces that refusal rather than
        // silently reading/writing something.
        assert!(library_thumb_write("not-hex!!".to_string(), vec![]).is_err());

        unsafe {
            std::env::remove_var("HEW_LIBRARY_DIR");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    // `valid_library_item_name` itself moved to `crates/library`
    // (`library::valid_item_name`) in the switch to a shared on-disk-layout
    // crate (docs/design/v1.1-cycle.md's Lane B) — these regression cases
    // stay here as a pin on the SHELL's expectations of that shared
    // validator (full coverage of the validator's own behavior lives in
    // `crates/library/src/naming.rs`'s own tests).
    #[test]
    fn valid_library_item_name_accepts_legacy_flat_names() {
        assert!(library::valid_item_name("chair.hew"));
        assert!(library::valid_item_name("theater-chair-3f2a.hew"));
        assert!(library::valid_item_name("UPPER.HEW"));
    }

    #[test]
    fn valid_library_item_name_accepts_the_three_category_subfolders() {
        assert!(library::valid_item_name(
            "Components/theater-chair-3f2a.hew"
        ));
        assert!(library::valid_item_name("Materials/oak.hew"));
        assert!(library::valid_item_name("Models/house.hew"));
    }

    #[test]
    fn valid_library_item_name_rejects_any_other_first_segment() {
        assert!(!library::valid_item_name("components/chair.hew")); // wrong case
        assert!(!library::valid_item_name("Textures/chair.hew"));
        assert!(!library::valid_item_name(".thumbnails/x.hew"));
    }

    #[test]
    fn valid_library_item_name_rejects_more_than_one_subfolder_segment() {
        assert!(!library::valid_item_name("Components/sub/chair.hew"));
    }

    #[test]
    fn valid_library_item_name_rejects_traversal_and_escapes() {
        assert!(!library::valid_item_name(""));
        assert!(!library::valid_item_name("../chair.hew"));
        assert!(!library::valid_item_name("Components/../chair.hew"));
        assert!(!library::valid_item_name("Components/..hew"));
        assert!(!library::valid_item_name("a\\b.hew"));
        assert!(!library::valid_item_name("Components/a\\b.hew"));
        assert!(!library::valid_item_name("C:foo.hew"));
        assert!(!library::valid_item_name("Components/C:foo.hew"));
        assert!(!library::valid_item_name(".hidden.hew"));
        assert!(!library::valid_item_name("Components/.hidden.hew"));
        assert!(!library::valid_item_name("Components/"));
        assert!(!library::valid_item_name("chair.txt"));
        assert!(!library::valid_item_name("Components//chair.hew"));
    }
}

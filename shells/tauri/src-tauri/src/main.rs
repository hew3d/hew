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
use tauri::{
    menu::{
        CheckMenuItem, CheckMenuItemBuilder, MenuBuilder, MenuItem, MenuItemBuilder,
        PredefinedMenuItem, SubmenuBuilder,
    },
    Emitter, Manager,
};

// ---------------------------------------------------------------------------
// In-app auto-updater (compiled in via the `updater` feature — see Cargo.toml).
//
// Talks to the signed `latest.json` published on GitHub Releases: a silent
// check on launch, plus a manual "Check for Updates" menu item. The whole
// flow lives shell-side — check, native confirm, download, restart prompt —
// so the webview is granted no update-related capability, and a
// `--no-default-features` build (Flathub/Homebrew/winget/AUR, which own their
// own updates) drops the module, the menu item, and the launch check together.
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
                info_dialog(
                    &app,
                    "Update in progress",
                    "Hew is already checking for updates.",
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
        let install = confirm(
            &app,
            "Update available",
            &format!(
                "Hew {} is available — you have {}.\n\nDownload and install it now?",
                update.version, update.current_version
            ),
            "Install",
            "Later",
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
                &format!("The update could not be installed.\n\n{err}"),
            );
            return;
        }
        let restart = confirm(
            &app,
            "Update installed",
            "The update has been installed. Restart Hew now to use it?",
            "Restart",
            "Later",
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
/// (and one subdirectory level below it) — granted only by import picks,
/// whose texture scan legitimately reads siblings like `textures/wood.png`.
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
}

/// Monotonic counter for extra document windows ("main-2", "main-3", …).
struct WindowCounter(u32);

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

/// Open a new document window cascaded from the calling window — File → New
/// when the current model isn't blank, and startup crash recovery when more
/// than one snapshot exists. With `recover_slot`, the new window claims that
/// recovery snapshot at mount (via `take_pending_recovery`) instead of
/// starting blank.
///
/// `async` is load-bearing, not style: on Windows, creating a webview inside a
/// synchronous command deadlocks WebView2 initialization (the sync command
/// holds the main thread the webview needs to finish creating itself — see the
/// "Known issues" note on `WebviewWindowBuilder::build`). The result is a
/// zombie window: a frame with no content whose close button does nothing.
/// An async command runs off the main thread, so the build can complete.
#[tauri::command]
async fn new_window(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    recover_slot: Option<String>,
) -> Result<(), String> {
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
        tauri::WebviewWindowBuilder::new(&app, &label, tauri::WebviewUrl::App("index.html".into()))
            .title("Hew")
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
) -> Result<(), String> {
    if !window.is_focused().unwrap_or(false) {
        let any_doc_focused = app
            .webview_windows()
            .into_values()
            .any(|w| w.label() != "settings" && w.is_focused().unwrap_or(false));
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
            if std::fs::rename(&src_hew, &dst_hew).is_ok() {
                let _ = std::fs::rename(&src_json, &dst_json);
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
    let Some(config_dir) = app.path().app_config_dir().ok() else {
        return Ok(());
    };
    let slot = effective_own_slot(&app, window.label());
    let (hew_path, json_path) = slot_paths(&config_dir, &slot);
    let _ = std::fs::remove_file(hew_path);
    let _ = std::fs::remove_file(json_path);
    Ok(())
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
                && (name.ends_with(".hew") || name.ends_with(".json")));
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
// Diagnostic log — rolling file (docs/DEVELOPMENT.md).
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
// Auto-reproducer dump (docs/DEVELOPMENT.md) — on a failure, the app
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
/// one level below it). Granted only by import picks — their texture scan
/// reads siblings of the picked model and `textures/`-style subfolders.
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

/// The document window menu/open events should land in: the focused
/// non-settings window, else the most recently focused one that still
/// exists, else `main`, else any document window.
fn active_document_window(app: &tauri::AppHandle) -> Option<tauri::WebviewWindow> {
    let windows = app.webview_windows();
    if let Some(w) = windows
        .values()
        .find(|w| w.label() != "settings" && w.is_focused().unwrap_or(false))
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
    windows
        .get("main")
        .cloned()
        .or_else(|| windows.values().find(|w| w.label() != "settings").cloned())
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
    tauri::WebviewWindowBuilder::new(
        &app,
        "settings",
        tauri::WebviewUrl::App("index.html#settings".into()),
    )
    .title("Settings")
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
#[tauri::command]
fn push_recent(path: String, app: tauri::AppHandle) -> Result<(), String> {
    let state = app.state::<Mutex<RecentState>>();
    let mut guard = state.lock().map_err(|e| e.to_string())?;
    // Dedup: remove existing entry for this path, then prepend.
    guard.paths.retain(|p| p != &path);
    guard.paths.insert(0, path);
    guard.paths.truncate(RECENTS_MAX);
    let paths = guard.paths.clone();
    save_recents(&app, &paths);
    rebuild_recent_submenu(&guard.submenu, &app, &paths)
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
    rebuild_recent_submenu(&guard.submenu, &app, &[])
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

    let clear = MenuItemBuilder::with_id("recent-clear", "Clear Recent")
        .build(app)
        .map_err(|e| e.to_string())?;
    submenu.append(&clear).map_err(|e| e.to_string())?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

fn main() {
    // Check argv for a .hew path on Windows/Linux (best-effort, first launch).
    // macOS uses RunEvent::Opened (Apple event), so we skip argv there.
    let argv_path: Option<String> = {
        #[cfg(not(target_os = "macos"))]
        {
            std::env::args()
                .skip(1)
                .find(|a| a.to_lowercase().ends_with(".hew"))
        }
        #[cfg(target_os = "macos")]
        {
            None
        }
    };

    let builder = tauri::Builder::default()
        // Single-instance guard, registered first (its docs require it): a
        // second launch — a Windows/Linux file-association double-click —
        // joins the running instance instead of spawning a second process
        // whose "main" window would fight this one over recovery-main.hew
        // and window-state.json.
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            if let Some(path) = args
                .iter()
                .skip(1)
                .find(|a| a.to_lowercase().ends_with(".hew"))
            {
                deliver_open(app, path);
            }
            if let Some(w) = active_document_window(app) {
                let _ = w.unminimize();
                let _ = w.set_focus();
            }
        }))
        // Register the dialog plugin (open/save native dialogs).
        .plugin(tauri_plugin_dialog::init())
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
            list_dir,
            pick_open_path,
            pick_save_path,
            take_pending_open,
            take_pending_recovery,
            frontend_ready,
            new_window,
            open_settings_window,
            sync_menu_state,
            push_recent,
            get_recents,
            clear_recent,
            recovery_write,
            recovery_list,
            recovery_claim,
            recovery_clear,
            recovery_discard_all,
            log_append,
            log_rotate,
            reproducer_write,
            check_for_updates,
            updater_available,
        ])
        // Build and attach the native menu bar; wire menu-item clicks to
        // `menu-action` events emitted to the webview.
        .setup(move |app| {
            let handle = app.handle();

            // Handle registries for stateful menu items (filled in as the
            // menus below are built; managed as MenuHandles at the end).
            let mut checks: HashMap<String, CheckMenuItem<tauri::Wry>> = HashMap::new();
            let mut gated: HashMap<String, MenuItem<tauri::Wry>> = HashMap::new();

            // ----------------------------------------------------------------
            // App menu (macOS — About + Quit)
            // ----------------------------------------------------------------
            let app_settings = MenuItemBuilder::with_id("app-settings", "Settings…")
                .accelerator("CmdOrCtrl+,")
                .build(handle)?;

            // "Check for Updates…" follows About (macOS convention), present
            // only in updater-enabled builds.
            let app_menu_builder = SubmenuBuilder::new(handle, "Hew")
                .item(&PredefinedMenuItem::about(handle, None, None)?);
            #[cfg(feature = "updater")]
            let app_menu_builder = app_menu_builder.item(
                &MenuItemBuilder::with_id("app-check-updates", "Check for Updates…")
                    .build(handle)?,
            );
            let app_menu = app_menu_builder
                .separator()
                .item(&app_settings)
                .separator()
                .item(&PredefinedMenuItem::services(handle, None)?)
                .separator()
                .item(&PredefinedMenuItem::hide(handle, None)?)
                .item(&PredefinedMenuItem::hide_others(handle, None)?)
                .item(&PredefinedMenuItem::show_all(handle, None)?)
                .separator()
                .item(&PredefinedMenuItem::quit(handle, None)?)
                .build()?;

            // ----------------------------------------------------------------
            // Open Recent submenu — populated from persisted recents.
            // ----------------------------------------------------------------
            let recents = load_recents(handle);

            // Build an empty submenu first, then populate via the helper so
            // the rebuild logic stays in one place.
            let open_recent_submenu = SubmenuBuilder::new(handle, "Open Recent").build()?;
            rebuild_recent_submenu(&open_recent_submenu, handle, &recents)
                .map_err(|e| tauri::Error::Anyhow(anyhow::anyhow!(e)))?;

            // ----------------------------------------------------------------
            // File menu
            // ----------------------------------------------------------------
            let file_new = MenuItemBuilder::with_id("file-new", "New")
                .accelerator("CmdOrCtrl+N")
                .build(handle)?;
            let file_open = MenuItemBuilder::with_id("file-open", "Open…")
                .accelerator("CmdOrCtrl+O")
                .build(handle)?;
            let file_import = MenuItemBuilder::with_id("file-import", "Import…").build(handle)?;
            // Export… opens the unified Export dialog (format — glTF/GLB or
            // STL — chosen there); there used to be a second "Export STL…"
            // item here, but the design calls for format choice handled in-dialog like
            // every other app.
            let file_export = MenuItemBuilder::with_id("file-export", "Export…").build(handle)?;
            let file_save = MenuItemBuilder::with_id("file-save", "Save")
                .accelerator("CmdOrCtrl+S")
                .build(handle)?;
            let file_save_as = MenuItemBuilder::with_id("file-save-as", "Save As…")
                .accelerator("Shift+CmdOrCtrl+S")
                .build(handle)?;
            let file_close = MenuItemBuilder::with_id("file-close", "Close")
                .accelerator("CmdOrCtrl+W")
                .build(handle)?;

            let file_menu = SubmenuBuilder::new(handle, "File")
                .item(&file_new)
                .item(&file_open)
                .item(&open_recent_submenu)
                .separator()
                .item(&file_import)
                .item(&file_export)
                .separator()
                .item(&file_save)
                .item(&file_save_as)
                .separator()
                .item(&file_close)
                .build()?;

            // ----------------------------------------------------------------
            // Edit menu
            // ----------------------------------------------------------------
            let edit_undo = MenuItemBuilder::with_id("edit-undo", "Undo")
                .accelerator("CmdOrCtrl+Z")
                .build(handle)?;
            let edit_redo = MenuItemBuilder::with_id("edit-redo", "Redo")
                .accelerator("Shift+CmdOrCtrl+Z")
                .build(handle)?;
            // No accelerator: a bare Backspace/Delete binding here would be a
            // global OS-level shortcut that fires even while typing in a text
            // field or dialog, unlike the JS keydown handler (App.tsx), which
            // is scoped to the Select tool and guards typing contexts. Relying
            // on the JS handler for the keyboard path is the documented
            // fallback when the menu lib's accelerator can't be scoped.
            let edit_delete = gated_item(handle, &mut gated, "edit-delete", "Delete", None, None)?;
            // No accelerator, for the same reason as Delete above: a native
            // CmdOrCtrl+A would fire even while typing in a text field,
            // hijacking select-all-text into a scene-wide selection. The JS
            // keydown handler (App.tsx) owns the keyboard path with a typing
            // guard on every platform.
            let edit_select_all =
                MenuItemBuilder::with_id("edit-select-all", "Select All").build(handle)?;
            let edit_delete_guides =
                MenuItemBuilder::with_id("edit-delete-guides", "Delete Guide Lines")
                    .build(handle)?;

            // Object commands (previously buttons inside the Outliner panel —
            // they belong in the menu bar, enabled per selection via
            // `sync_menu_state`). Start disabled: nothing is selected at launch.
            let edit_group = gated_item(
                handle,
                &mut gated,
                "edit-group",
                "Group",
                Some("CmdOrCtrl+G"),
                Some("CmdOrCtrl+G"),
            )?;
            let edit_ungroup = gated_item(
                handle,
                &mut gated,
                "edit-ungroup",
                "Ungroup",
                Some("Shift+CmdOrCtrl+G"),
                Some("Shift+CmdOrCtrl+G"),
            )?;
            let edit_make_component = gated_item(
                handle,
                &mut gated,
                "edit-make-component",
                "Make Component",
                None,
                None,
            )?;
            let edit_place_copy = gated_item(
                handle,
                &mut gated,
                "edit-place-copy",
                "Place Copy",
                None,
                None,
            )?;
            let edit_explode =
                gated_item(handle, &mut gated, "edit-explode", "Explode", None, None)?;
            let edit_make_unique = gated_item(
                handle,
                &mut gated,
                "edit-make-unique",
                "Make Unique",
                None,
                None,
            )?;
            let edit_union = gated_item(handle, &mut gated, "edit-union", "Union", None, None)?;
            let edit_subtract =
                gated_item(handle, &mut gated, "edit-subtract", "Subtract", None, None)?;
            let edit_intersect = gated_item(
                handle,
                &mut gated,
                "edit-intersect",
                "Intersect",
                None,
                None,
            )?;
            for item in [
                &edit_group,
                &edit_ungroup,
                &edit_make_component,
                &edit_place_copy,
                &edit_explode,
                &edit_make_unique,
                &edit_union,
                &edit_subtract,
                &edit_intersect,
            ] {
                let _ = item.set_enabled(false);
            }

            let edit_menu = SubmenuBuilder::new(handle, "Edit")
                .item(&edit_undo)
                .item(&edit_redo)
                .separator()
                .item(&edit_select_all)
                .item(&edit_delete)
                .item(&edit_delete_guides)
                .separator()
                .item(&edit_group)
                .item(&edit_ungroup)
                .separator()
                .item(&edit_make_component)
                .item(&edit_place_copy)
                .item(&edit_explode)
                .item(&edit_make_unique)
                .separator()
                .item(&edit_union)
                .item(&edit_subtract)
                .item(&edit_intersect)
                .build()?;

            // ----------------------------------------------------------------
            // View menu
            // ----------------------------------------------------------------
            let view_axes = check_item(handle, &mut checks, "view-axes", "Axes", None, None)?;
            let view_grid = check_item(handle, &mut checks, "view-grid", "Grid", None, None)?;
            let view_guides = check_item(handle, &mut checks, "view-guides", "Guides", None, None)?;
            // Command palette. `Cmd+K` is already Rectangle's
            // accelerator (preserved unchanged) — `Cmd+/` is free and is
            // the same binding Windows/Linux/web reach via the JS keydown
            // handler's Ctrl+K (that platform's Rectangle moved to a bare
            // `R` in, freeing Ctrl+K there; macOS keeps Cmd+K on
            // Rectangle, so the palette needs a different key here).
            let view_palette = accel(
                MenuItemBuilder::with_id("view-palette", "Command Palette…"),
                Some("CmdOrCtrl+/"),
                Some("CmdOrCtrl+K"),
            )
            .build(handle)?;

            let view_menu = SubmenuBuilder::new(handle, "View")
                .item(&view_axes)
                .item(&view_grid)
                .item(&view_guides)
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
                "Rectangle",
                Some("CmdOrCtrl+K"),
                Some("R"),
            )?;

            let draw_circle = check_item(
                handle,
                &mut checks,
                "draw-circle",
                "Circle",
                None,
                Some("C"),
            )?;

            // Arc : Cmd+J is SketchUp's arc-family
            // key on macOS, even though Hew's Arc is the simpler 2-point
            // gesture rather than SketchUp's multi-mode arc tool family.
            let draw_arc = check_item(
                handle,
                &mut checks,
                "draw-arc",
                "Arc",
                Some("CmdOrCtrl+J"),
                Some("A"),
            )?;

            let draw_shapes = SubmenuBuilder::new(handle, "Shapes")
                .item(&draw_rect)
                .item(&draw_circle)
                .item(&draw_arc)
                .build()?;

            let draw_line = check_item(
                handle,
                &mut checks,
                "draw-line",
                "Line",
                Some("CmdOrCtrl+L"),
                Some("L"),
            )?;

            let draw_lines = SubmenuBuilder::new(handle, "Lines")
                .item(&draw_line)
                .build()?;

            let draw_menu = SubmenuBuilder::new(handle, "Draw")
                .item(&draw_shapes)
                .item(&draw_lines)
                .build()?;

            // ----------------------------------------------------------------
            // Tools menu
            // ----------------------------------------------------------------
            let tool_select = check_item(
                handle,
                &mut checks,
                "tool-select",
                "Select",
                None,
                Some("Space"),
            )?;
            let tool_paint =
                check_item(handle, &mut checks, "tool-paint", "Paint", None, Some("B"))?;
            let tool_move = check_item(
                handle,
                &mut checks,
                "tool-move",
                "Move",
                Some("CmdOrCtrl+0"),
                Some("M"),
            )?;
            let tool_rotate = check_item(
                handle,
                &mut checks,
                "tool-rotate",
                "Rotate",
                Some("CmdOrCtrl+8"),
                Some("Q"),
            )?;
            let tool_scale = check_item(
                handle,
                &mut checks,
                "tool-scale",
                "Scale",
                Some("CmdOrCtrl+9"),
                Some("S"),
            )?;
            let tool_pushpull = check_item(
                handle,
                &mut checks,
                "tool-pushpull",
                "Push/Pull",
                Some("CmdOrCtrl+="),
                Some("P"),
            )?;
            let tool_follow_me = check_item(
                handle,
                &mut checks,
                "tool-follow-me",
                "Follow Me",
                None,
                None,
            )?;

            let tool_offset = check_item(
                handle,
                &mut checks,
                "tool-offset",
                "Offset",
                None,
                Some("F"),
            )?;
            let tool_tape_measure = check_item(
                handle,
                &mut checks,
                "tool-tape-measure",
                "Tape Measure",
                Some("CmdOrCtrl+D"),
                Some("T"),
            )?;
            let tool_protractor = check_item(
                handle,
                &mut checks,
                "tool-protractor",
                "Protractor",
                None,
                None,
            )?;
            let tool_slice = check_item(handle, &mut checks, "tool-slice", "Slice", None, None)?;
            let tool_edit_vertex = check_item(
                handle,
                &mut checks,
                "tool-edit-vertex",
                "Edit Vertex",
                None,
                None,
            )?;

            let tools_menu = SubmenuBuilder::new(handle, "Tools")
                .item(&tool_select)
                .item(&tool_paint)
                .item(&tool_move)
                .item(&tool_rotate)
                .item(&tool_scale)
                .item(&tool_pushpull)
                .item(&tool_follow_me)
                .item(&tool_offset)
                .separator()
                .item(&tool_tape_measure)
                .item(&tool_protractor)
                .item(&tool_slice)
                .item(&tool_edit_vertex)
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
                "Orbit",
                Some("CmdOrCtrl+B"),
                Some("O"),
            )?;
            let cam_pan = check_item(
                handle,
                &mut checks,
                "cam-pan",
                "Pan",
                Some("CmdOrCtrl+R"),
                Some("H"),
            )?;
            let cam_zoom = check_item(
                handle,
                &mut checks,
                "cam-zoom",
                "Zoom",
                Some("CmdOrCtrl+\\"),
                Some("Z"),
            )?;
            let cam_zoom_extents =
                MenuItemBuilder::with_id("cam-zoom-extents", "Zoom Extents").build(handle)?;

            // Standard Views — axis-aligned + isometric framings.
            let view_top = MenuItemBuilder::with_id("cam-view-top", "Top").build(handle)?;
            let view_bottom =
                MenuItemBuilder::with_id("cam-view-bottom", "Bottom").build(handle)?;
            let view_front = MenuItemBuilder::with_id("cam-view-front", "Front").build(handle)?;
            let view_back = MenuItemBuilder::with_id("cam-view-back", "Back").build(handle)?;
            let view_left = MenuItemBuilder::with_id("cam-view-left", "Left").build(handle)?;
            let view_right = MenuItemBuilder::with_id("cam-view-right", "Right").build(handle)?;
            let view_iso = MenuItemBuilder::with_id("cam-view-iso", "Iso").build(handle)?;
            let standard_views = SubmenuBuilder::new(handle, "Standard Views")
                .item(&view_top)
                .item(&view_bottom)
                .item(&view_front)
                .item(&view_back)
                .item(&view_left)
                .item(&view_right)
                .separator()
                .item(&view_iso)
                .build()?;

            let camera_menu = SubmenuBuilder::new(handle, "Camera")
                .item(&cam_orbit)
                .item(&cam_pan)
                .item(&cam_zoom)
                .separator()
                .item(&cam_zoom_extents)
                .item(&standard_views)
                .build()?;

            // ----------------------------------------------------------------
            // Window menu
            // ----------------------------------------------------------------
            let win_model_info = check_item(
                handle,
                &mut checks,
                "win-model-info",
                "Model Info",
                Some("Shift+CmdOrCtrl+I"),
                Some("Shift+CmdOrCtrl+I"),
            )?;
            let win_materials = check_item(
                handle,
                &mut checks,
                "win-materials",
                "Materials",
                Some("Shift+CmdOrCtrl+C"),
                Some("Shift+CmdOrCtrl+C"),
            )?;
            let win_tags = check_item(
                handle,
                &mut checks,
                "win-tags",
                "Tags",
                Some("Shift+CmdOrCtrl+T"),
                Some("Shift+CmdOrCtrl+T"),
            )?;
            let win_object_info = check_item(
                handle,
                &mut checks,
                "win-object-info",
                "Object Info",
                Some("Shift+CmdOrCtrl+O"),
                Some("Shift+CmdOrCtrl+O"),
            )?;
            let win_debug_log = check_item(
                handle,
                &mut checks,
                "win-debug-log",
                "Debug Log",
                None,
                None,
            )?;

            let window_menu = SubmenuBuilder::new(handle, "Window")
                // Standard macOS window management first (HIG: the Window
                // menu manages windows; the pane toggles follow below).
                .item(&PredefinedMenuItem::minimize(handle, None)?)
                .item(&PredefinedMenuItem::maximize(handle, Some("Zoom"))?)
                .separator()
                .item(&win_model_info)
                .item(&win_materials)
                .item(&win_tags)
                .item(&win_object_info)
                .item(&win_debug_log)
                .build()?;

            // ----------------------------------------------------------------
            // Help menu
            // ----------------------------------------------------------------
            let help_report_bug =
                MenuItemBuilder::with_id("help-report-bug", "Report Bug…").build(handle)?;

            let help_menu = SubmenuBuilder::new(handle, "Help")
                .item(&help_report_bug)
                .build()?;

            // ----------------------------------------------------------------
            // Assemble and set the menu bar
            // ----------------------------------------------------------------
            let menu = MenuBuilder::new(handle)
                .item(&app_menu)
                .item(&file_menu)
                .item(&edit_menu)
                .item(&view_menu)
                .item(&draw_menu)
                .item(&tools_menu)
                .item(&camera_menu)
                .item(&window_menu)
                .item(&help_menu)
                .build()?;

            // Attach the menu. macOS shows a single global menu bar owned by the
            // app, so the app-level menu is correct there.
            #[cfg(target_os = "macos")]
            app.set_menu(menu)?;
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
            for path in &recents {
                approve_file(handle, Path::new(path), true);
            }
            if let Some(path) = &argv_path {
                approve_file(handle, Path::new(path), true);
            }

            // ----------------------------------------------------------------
            // Managed state: recents + submenu handle
            // ----------------------------------------------------------------
            app.manage(Mutex::new(RecentState {
                paths: recents,
                submenu: open_recent_submenu,
            }));

            // ----------------------------------------------------------------
            // Managed state: pending-open (seeded from argv on Windows/Linux)
            // ----------------------------------------------------------------
            app.manage(Mutex::new(PendingOpen(argv_path)));

            // ----------------------------------------------------------------
            // Managed state: stateful menu handles, document-window counter,
            // multi-window routing/recovery bookkeeping, window-state cache
            // ----------------------------------------------------------------
            app.manage(Mutex::new(MenuHandles {
                checks,
                items: gated,
            }));
            // Seeded past any main-N recovery slot a crashed session left, so
            // this session's fresh window labels never alias an unclaimed
            // snapshot slot (see main_slot_suffix).
            let config_dir = handle.path().app_config_dir().ok();
            app.manage(Mutex::new(WindowCounter(seed_window_counter(
                config_dir.as_deref(),
            ))));
            app.manage(Mutex::new(ReadyWindows(HashSet::new())));
            app.manage(Mutex::new(ActiveWindow(None)));
            app.manage(Mutex::new(PendingRecovery(HashMap::new())));
            app.manage(Mutex::new(ProtectedSlots(HashSet::new())));
            app.manage(Mutex::new(WindowStateCache {
                last_normal: None,
                last_write: None,
            }));

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

            Ok(())
        })
        // Per-window bookkeeping: focus tracking for menu/open routing,
        // drag-drop path approval, readiness cleanup, and main-window
        // geometry persistence (extra "main-N" document windows are
        // transient and don't overwrite the primary window's saved state).
        .on_window_event(|window, event| {
            let app = window.app_handle();
            match event {
                tauri::WindowEvent::Focused(true) if window.label() != "settings" => {
                    if let Ok(mut active) = app.state::<Mutex<ActiveWindow>>().lock() {
                        active.0 = Some(window.label().to_string());
                    }
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
                    if let Ok(mut ready) = app.state::<Mutex<ReadyWindows>>().lock() {
                        ready.0.remove(window.label());
                    }
                    if let Ok(mut active) = app.state::<Mutex<ActiveWindow>>().lock() {
                        if active.0.as_deref() == Some(window.label()) {
                            active.0 = None;
                        }
                    }
                    // A recovery window that dies before claiming leaves its
                    // assignment unconsumed; drop it (the slot files stay on
                    // disk and are re-offered next launch).
                    if let Ok(mut pending) = app.state::<Mutex<PendingRecovery>>().lock() {
                        pending.0.remove(window.label());
                    }
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
            if let Some(path) = id.strip_prefix("recent:") {
                emit_to_active(app, "menu-open-path", path);
                return;
            }
            if id == "recent-clear" {
                let _ = clear_recent(app.clone());
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
                "file-export" => "export",
                "file-save" => "save",
                "file-save-as" => "save-as",
                "file-close" => "close",
                "edit-undo" => "undo",
                "edit-redo" => "redo",
                "edit-delete" => "edit-delete",
                "edit-delete-guides" => "edit-delete-guides",
                "edit-select-all" => "edit-select-all",
                "edit-group" => "edit-group",
                "edit-ungroup" => "edit-ungroup",
                "edit-make-component" => "edit-make-component",
                "edit-place-copy" => "edit-place-copy",
                "edit-explode" => "edit-explode",
                "edit-make-unique" => "edit-make-unique",
                "edit-union" => "edit-union",
                "edit-subtract" => "edit-subtract",
                "edit-intersect" => "edit-intersect",
                "view-axes" => "toggle-axes",
                "view-grid" => "toggle-grid",
                "view-guides" => "toggle-guides",
                "view-palette" => "open-palette",
                "draw-rectangle" => "tool-rectangle",
                "draw-circle" => "tool-circle",
                "draw-arc" => "tool-arc",
                "draw-line" => "tool-line",
                "tool-select" => "tool-select",
                "tool-paint" => "tool-paint",
                "tool-move" => "tool-move",
                "tool-rotate" => "tool-rotate",
                "tool-scale" => "tool-scale",
                "tool-pushpull" => "tool-pushpull",
                "tool-follow-me" => "tool-follow-me",
                "tool-offset" => "tool-offset",
                "tool-tape-measure" => "tool-tape-measure",
                "tool-protractor" => "tool-protractor",
                "tool-slice" => "tool-slice",
                "tool-edit-vertex" => "tool-edit-vertex",
                "cam-orbit" => "tool-orbit",
                "cam-pan" => "tool-pan",
                "cam-zoom" => "tool-zoom",
                "cam-zoom-extents" => "zoom-extents",
                "cam-view-top" => "view-top",
                "cam-view-bottom" => "view-bottom",
                "cam-view-front" => "view-front",
                "cam-view-back" => "view-back",
                "cam-view-left" => "view-left",
                "cam-view-right" => "view-right",
                "cam-view-iso" => "view-iso",
                "win-model-info" => "toggle-model-info",
                "win-materials" => "toggle-materials",
                "win-tags" => "toggle-tags",
                "win-object-info" => "toggle-object-info",
                "win-debug-log" => "toggle-debug-log",
                "help-report-bug" => "report-bug",
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
            }
            // macOS: intercept "open document" Apple events (warm + cold
            // start). deliver_open emits to the active document window once
            // some webview is ready, and buffers only before that — so a
            // warm open can never strand a stale path in the buffer for a
            // later File ▸ New window to swallow, and an open while the app
            // is inactive lands in one window instead of broadcasting.
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Opened { urls } = &event {
                for url in urls {
                    if url.scheme() != "file" {
                        continue;
                    }
                    // Convert file:// URL to filesystem path (percent-decode).
                    let Ok(path) = url.to_file_path() else {
                        continue;
                    };
                    let path_str = path.to_string_lossy().to_string();
                    if !path_str.to_lowercase().ends_with(".hew") {
                        continue;
                    }
                    deliver_open(app, &path_str);
                }
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

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
}

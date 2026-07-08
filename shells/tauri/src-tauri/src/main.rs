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

use std::collections::HashMap;
use std::sync::Mutex;
use tauri::{
    menu::{
        CheckMenuItem, CheckMenuItemBuilder, MenuBuilder, MenuItem, MenuItemBuilder,
        PredefinedMenuItem, SubmenuBuilder,
    },
    Emitter, Manager,
};

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

/// Pending path to open on app startup (cold-start file association).
struct PendingOpen(Option<String>);

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

/// Capture the window's current logical geometry and persist it. Logical
/// (scale-independent) units keep the state portable across DPI changes.
/// While maximized, only the flag is updated so the restored "un-maximized"
/// geometry survives a quit-while-maximized.
fn save_window_state(app: &tauri::AppHandle, window: &tauri::Window) {
    let Some(path) = window_state_path(app) else {
        return;
    };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let maximized = window.is_maximized().unwrap_or(false);
    if maximized {
        if let Some(mut prev) = load_window_state(app) {
            prev.maximized = true;
            if let Ok(text) = serde_json::to_string(&prev) {
                let _ = std::fs::write(&path, text);
            }
        }
        return;
    }
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
    if let Ok(text) = serde_json::to_string(&state) {
        let _ = std::fs::write(&path, text);
    }
}

/// Apply persisted state to the main window, or fall back to ~2/3 of the
/// current monitor, centered — a first-run default proportional to the
/// screen instead of a fixed 1280x800.
fn apply_initial_window_state(app: &tauri::AppHandle, window: &tauri::WebviewWindow) {
    if let Some(state) = load_window_state(app) {
        if state.width >= 400.0 && state.height >= 300.0 {
            let _ = window.set_size(tauri::LogicalSize::new(state.width, state.height));
            let _ = window.set_position(tauri::LogicalPosition::new(state.x, state.y));
            // If the saved position no longer lands on any monitor (display
            // unplugged), pull the window back into view.
            if window.current_monitor().ok().flatten().is_none() {
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

/// Open a new, empty document window (File → New when the current model
/// isn't blank), cascaded from the calling window.
#[tauri::command]
fn new_window(app: tauri::AppHandle, window: tauri::WebviewWindow) -> Result<(), String> {
    let label = {
        let state = app.state::<Mutex<WindowCounter>>();
        let mut guard = state.lock().map_err(|e| e.to_string())?;
        guard.0 += 1;
        format!("main-{}", guard.0)
    };
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
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    let builder = builder.decorations(false);
    builder.build().map_err(|e| e.to_string())?;
    Ok(())
}

/// Reflect webview state into the native menu bar: `checked` drives check
/// marks (active tool radio group, Axes/Guides, open panes); `enabled` gates
/// selection-dependent items (Group, Explode, …). Unknown ids are ignored so
/// the webview can stay ahead of the shell.
#[tauri::command]
fn sync_menu_state(
    app: tauri::AppHandle,
    checked: HashMap<String, bool>,
    enabled: HashMap<String, bool>,
) -> Result<(), String> {
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
// Two files in the app config dir: recovery.hew (geometry bytes, the same
// format scene.save() produces) and recovery.json (the RecoveryMeta JSON
// verbatim, opaque to Rust — we never parse it here).
// ---------------------------------------------------------------------------

/// Payload returned by `recovery_read`.
#[derive(serde::Serialize)]
struct RecoveryPayload {
    contents: Vec<u8>,
    meta: String,
}

/// Persist a recovery snapshot, overwriting any previous one from the same
/// window. Snapshots are namespaced per window label ("recovery-main.hew")
/// so concurrent document windows don't clobber each other's autosaves;
/// the un-suffixed "recovery.hew" is the pre-multi-window legacy name.
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
    let label = window.label();
    std::fs::write(config_dir.join(format!("recovery-{label}.hew")), &contents)
        .map_err(|e| e.to_string())?;
    std::fs::write(config_dir.join(format!("recovery-{label}.json")), &meta)
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Read back the newest recovery snapshot across all windows (plus the
/// legacy un-suffixed pair), or None when nothing recoverable exists. Only
/// the first window at startup runs recovery, so it adopts whichever
/// window's snapshot is freshest.
#[tauri::command]
fn recovery_read(app: tauri::AppHandle) -> Option<RecoveryPayload> {
    let config_dir = app.path().app_config_dir().ok()?;
    let entries = std::fs::read_dir(&config_dir).ok()?;
    let mut newest: Option<(std::time::SystemTime, std::path::PathBuf)> = None;
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let is_snapshot =
            name == "recovery.hew" || (name.starts_with("recovery-") && name.ends_with(".hew"));
        if !is_snapshot {
            continue;
        }
        let Ok(modified) = entry.metadata().and_then(|m| m.modified()) else {
            continue;
        };
        if newest.as_ref().is_none_or(|(t, _)| modified > *t) {
            newest = Some((modified, path));
        }
    }
    let (_, hew_path) = newest?;
    let contents = std::fs::read(&hew_path).ok()?;
    let meta = std::fs::read_to_string(hew_path.with_extension("json")).ok()?;
    Some(RecoveryPayload { contents, meta })
}

/// Discard every stored recovery snapshot (all window labels + legacy),
/// ignoring not-found errors. Called once recovery is accepted or declined.
#[tauri::command]
fn recovery_clear(app: tauri::AppHandle) -> Result<(), String> {
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
    Ok(())
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
// Custom file-I/O commands.
//
// These bypass the fs plugin so we need no fs capability entries — app-defined
// commands require no capability grant.
// ---------------------------------------------------------------------------

/// Read a file from the filesystem and return its raw bytes.
#[tauri::command]
fn read_file(path: String) -> Result<Vec<u8>, String> {
    std::fs::read(&path).map_err(|e| format!("read_file failed for {path:?}: {e}"))
}

/// Write raw bytes to a file, creating or overwriting it.
#[tauri::command]
fn write_file(path: String, contents: Vec<u8>) -> Result<(), String> {
    std::fs::write(&path, &contents).map_err(|e| format!("write_file failed for {path:?}: {e}"))
}

/// List the direct children of a directory, returning their absolute paths.
/// Only regular files are included (directories are omitted).
#[tauri::command]
fn list_dir(path: String) -> Result<Vec<String>, String> {
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

/// Take the pending-open path (cold-start file association) — returns Some once,
/// then None on subsequent calls.
#[tauri::command]
fn take_pending_open(app: tauri::AppHandle) -> Option<String> {
    app.state::<Mutex<PendingOpen>>().lock().ok()?.0.take()
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

    tauri::Builder::default()
        // Register the dialog plugin (open/save native dialogs).
        .plugin(tauri_plugin_dialog::init())
        // Register custom file-I/O commands.
        .invoke_handler(tauri::generate_handler![
            read_file,
            write_file,
            list_dir,
            take_pending_open,
            new_window,
            sync_menu_state,
            push_recent,
            get_recents,
            clear_recent,
            recovery_write,
            recovery_read,
            recovery_clear,
            log_append,
            log_rotate,
            reproducer_write,
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

            let app_menu = SubmenuBuilder::new(handle, "Hew")
                .item(&PredefinedMenuItem::about(handle, None, None)?)
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
            // Windows and Linux both go borderless with fully in-app chrome
            // (custom TitleBar + HTML MenuBar — App renders both when
            // isLinux || isWindows). Linux settled on this after the
            //  experiments: native decorations can't return
            // on Wayland ('s stale-title bug still reproduces; X11 works
            // but is backend-regressive), and the native GTK menubar —
            // trialed in  — was rejected because GTK can only stack it
            // ABOVE the custom title bar and can't match Hew's theme beyond
            // a dark/light flip. The menu below is still built on every
            // platform: macOS attaches it (above), and it keeps parity ready
            // if Windows ever goes native chrome ( leaves that open —
            // Windows hasn't been examined yet); `accel()` keeps its
            // per-platform accelerator labels correct for that day.
            #[cfg(any(target_os = "windows", target_os = "linux"))]
            {
                let _ = menu; // built for parity; not attached on Windows/Linux.
                if let Some(main_window) = app.webview_windows().values().next() {
                    let _ = main_window.set_decorations(false);
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
            // Managed state: pending-open (seeded from argv on Windows/Linux)
            // ----------------------------------------------------------------
            app.manage(Mutex::new(PendingOpen(argv_path)));

            // ----------------------------------------------------------------
            // Managed state: stateful menu handles + document-window counter
            // ----------------------------------------------------------------
            app.manage(Mutex::new(MenuHandles {
                checks,
                items: gated,
            }));
            app.manage(Mutex::new(WindowCounter(1)));

            // ----------------------------------------------------------------
            // Main window geometry: restore the persisted size/position, or
            // default to ~2/3 of the screen on first run.
            // ----------------------------------------------------------------
            if let Some(main_window) = app.get_webview_window("main") {
                apply_initial_window_state(handle, &main_window);
                let _ = main_window.show();
            }

            Ok(())
        })
        // Persist main-window geometry as it changes so the next launch
        // restores it (extra "main-N" document windows are transient and
        // don't overwrite the primary window's saved state).
        .on_window_event(|window, event| {
            if window.label() != "main" {
                return;
            }
            match event {
                tauri::WindowEvent::Resized(_)
                | tauri::WindowEvent::Moved(_)
                | tauri::WindowEvent::CloseRequested { .. } => {
                    save_window_state(window.app_handle(), window);
                }
                _ => {}
            }
        })
        // Map menu-item ids to action strings and emit them to the webview.
        // With multiple document windows, an action targets the focused one;
        // the fallback broadcast covers the moment when the menu is used
        // while no webview reports focus (e.g. right after a dialog closes).
        .on_menu_event(|app, event| {
            let emit_to_active = |name: &str, payload: &str| {
                let focused = app
                    .webview_windows()
                    .into_values()
                    .find(|w| w.label() != "settings" && w.is_focused().unwrap_or(false));
                match focused {
                    Some(w) => {
                        let _ = app.emit_to(w.label(), name, payload);
                    }
                    None => {
                        let _ = app.emit(name, payload);
                    }
                }
            };
            let id = event.id().as_ref();
            if let Some(path) = id.strip_prefix("recent:") {
                emit_to_active("menu-open-path", path);
                return;
            }
            if id == "recent-clear" {
                let _ = clear_recent(app.clone());
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
            emit_to_active("menu-action", action);
        })
        .build(tauri::generate_context!())
        .expect("error while building Hew desktop")
        .run(|app, event| {
            // `app`/`event` are only read in the macOS Apple-event path below;
            // on every other platform this closure is a no-op, so mark them used
            // to keep `clippy -D warnings` (verify.sh) green on Linux/Windows.
            #[cfg(not(target_os = "macos"))]
            let _ = (&app, &event);
            // macOS: intercept "open document" Apple events (warm + cold start).
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
                    // Always update the pending-open buffer (covers cold start
                    // where the webview listener may not be registered yet).
                    if let Ok(mut guard) = app.state::<Mutex<PendingOpen>>().lock() {
                        guard.0 = Some(path_str.clone());
                    }
                    // Also emit for the warm case (app already running) —
                    // targeted at the focused document window so a Finder
                    // double-click doesn't open the file in every window.
                    let focused = app
                        .webview_windows()
                        .into_values()
                        .find(|w| w.label() != "settings" && w.is_focused().unwrap_or(false));
                    match focused {
                        Some(w) => {
                            let _ = app.emit_to(w.label(), "menu-open-path", &path_str);
                        }
                        None => {
                            let _ = app.emit("menu-open-path", &path_str);
                        }
                    }
                }
            }
        });
}

// Prevents an additional console window on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Mutex;
use tauri::{
    menu::{MenuBuilder, MenuItemBuilder, PredefinedMenuItem, SubmenuBuilder},
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

/// Persist a recovery snapshot, overwriting any previous one.
#[tauri::command]
fn recovery_write(app: tauri::AppHandle, contents: Vec<u8>, meta: String) -> Result<(), String> {
    let Some(config_dir) = app.path().app_config_dir().ok() else {
        return Err("could not resolve app config dir".into());
    };
    std::fs::create_dir_all(&config_dir).map_err(|e| e.to_string())?;
    std::fs::write(config_dir.join("recovery.hew"), &contents).map_err(|e| e.to_string())?;
    std::fs::write(config_dir.join("recovery.json"), &meta).map_err(|e| e.to_string())?;
    Ok(())
}

/// Read back the most recent recovery snapshot, or None if either file is
/// missing/unreadable.
#[tauri::command]
fn recovery_read(app: tauri::AppHandle) -> Option<RecoveryPayload> {
    let config_dir = app.path().app_config_dir().ok()?;
    let contents = std::fs::read(config_dir.join("recovery.hew")).ok()?;
    let meta = std::fs::read_to_string(config_dir.join("recovery.json")).ok()?;
    Some(RecoveryPayload { contents, meta })
}

/// Discard the stored recovery snapshot, ignoring not-found errors.
#[tauri::command]
fn recovery_clear(app: tauri::AppHandle) -> Result<(), String> {
    let Some(config_dir) = app.path().app_config_dir().ok() else {
        return Ok(());
    };
    let _ = std::fs::remove_file(config_dir.join("recovery.hew"));
    let _ = std::fs::remove_file(config_dir.join("recovery.json"));
    Ok(())
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
            push_recent,
            clear_recent,
            recovery_write,
            recovery_read,
            recovery_clear,
        ])
        // Build and attach the native menu bar; wire menu-item clicks to
        // `menu-action` events emitted to the webview.
        .setup(move |app| {
            let handle = app.handle();

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

            let edit_menu = SubmenuBuilder::new(handle, "Edit")
                .item(&edit_undo)
                .item(&edit_redo)
                .build()?;

            // ----------------------------------------------------------------
            // Draw menu
            // ----------------------------------------------------------------
            let draw_rect = MenuItemBuilder::with_id("draw-rectangle", "Rectangle")
                .accelerator("CmdOrCtrl+K")
                .build(handle)?;

            let draw_shapes = SubmenuBuilder::new(handle, "Shapes")
                .item(&draw_rect)
                .build()?;

            let draw_menu = SubmenuBuilder::new(handle, "Draw")
                .item(&draw_shapes)
                .build()?;

            // ----------------------------------------------------------------
            // Tools menu
            // ----------------------------------------------------------------
            let tool_select = MenuItemBuilder::with_id("tool-select", "Select").build(handle)?;
            let tool_paint = MenuItemBuilder::with_id("tool-paint", "Paint").build(handle)?;
            let tool_move = MenuItemBuilder::with_id("tool-move", "Move")
                .accelerator("CmdOrCtrl+0")
                .build(handle)?;
            let tool_rotate = MenuItemBuilder::with_id("tool-rotate", "Rotate")
                .accelerator("CmdOrCtrl+8")
                .build(handle)?;
            let tool_scale = MenuItemBuilder::with_id("tool-scale", "Scale")
                .accelerator("CmdOrCtrl+9")
                .build(handle)?;
            let tool_pushpull = MenuItemBuilder::with_id("tool-pushpull", "Push/Pull")
                .accelerator("CmdOrCtrl+=")
                .build(handle)?;

            let tools_menu = SubmenuBuilder::new(handle, "Tools")
                .item(&tool_select)
                .item(&tool_paint)
                .item(&tool_move)
                .item(&tool_rotate)
                .item(&tool_scale)
                .item(&tool_pushpull)
                .build()?;

            // ----------------------------------------------------------------
            // Camera menu
            // ----------------------------------------------------------------
            let cam_orbit = MenuItemBuilder::with_id("cam-orbit", "Orbit")
                .accelerator("CmdOrCtrl+B")
                .build(handle)?;
            let cam_pan = MenuItemBuilder::with_id("cam-pan", "Pan")
                .accelerator("CmdOrCtrl+R")
                .build(handle)?;
            let cam_zoom = MenuItemBuilder::with_id("cam-zoom", "Zoom")
                .accelerator("CmdOrCtrl+\\")
                .build(handle)?;
            let cam_zoom_extents =
                MenuItemBuilder::with_id("cam-zoom-extents", "Zoom Extents").build(handle)?;

            let camera_menu = SubmenuBuilder::new(handle, "Camera")
                .item(&cam_orbit)
                .item(&cam_pan)
                .item(&cam_zoom)
                .separator()
                .item(&cam_zoom_extents)
                .build()?;

            // ----------------------------------------------------------------
            // Window menu
            // ----------------------------------------------------------------
            let win_model_info = MenuItemBuilder::with_id("win-model-info", "Model Info")
                .accelerator("Shift+CmdOrCtrl+I")
                .build(handle)?;
            let win_materials = MenuItemBuilder::with_id("win-materials", "Materials")
                .accelerator("Shift+CmdOrCtrl+C")
                .build(handle)?;
            let win_tags = MenuItemBuilder::with_id("win-tags", "Tags")
                .accelerator("Shift+CmdOrCtrl+T")
                .build(handle)?;
            let win_object_info = MenuItemBuilder::with_id("win-object-info", "Object Info")
                .accelerator("Shift+CmdOrCtrl+O")
                .build(handle)?;
            let win_debug_log =
                MenuItemBuilder::with_id("win-debug-log", "Debug Log").build(handle)?;

            let window_menu = SubmenuBuilder::new(handle, "Window")
                .item(&win_model_info)
                .item(&win_materials)
                .item(&win_tags)
                .item(&win_object_info)
                .item(&win_debug_log)
                .build()?;

            // ----------------------------------------------------------------
            // Assemble and set the menu bar
            // ----------------------------------------------------------------
            let menu = MenuBuilder::new(handle)
                .item(&app_menu)
                .item(&file_menu)
                .item(&edit_menu)
                .item(&draw_menu)
                .item(&tools_menu)
                .item(&camera_menu)
                .item(&window_menu)
                .build()?;

            app.set_menu(menu)?;

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

            Ok(())
        })
        // Map menu-item ids to action strings and emit them to the webview.
        .on_menu_event(|app, event| {
            let id = event.id().as_ref();
            if let Some(path) = id.strip_prefix("recent:") {
                let _ = app.emit("menu-open-path", path);
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
                "file-save" => "save",
                "file-save-as" => "save-as",
                "file-close" => "close",
                "edit-undo" => "undo",
                "edit-redo" => "redo",
                "draw-rectangle" => "tool-rectangle",
                "tool-select" => "tool-select",
                "tool-paint" => "tool-paint",
                "tool-move" => "tool-move",
                "tool-rotate" => "tool-rotate",
                "tool-scale" => "tool-scale",
                "tool-pushpull" => "tool-pushpull",
                "cam-orbit" => "tool-orbit",
                "cam-pan" => "tool-pan",
                "cam-zoom" => "tool-zoom",
                "cam-zoom-extents" => "zoom-extents",
                "win-model-info" => "toggle-model-info",
                "win-materials" => "toggle-materials",
                "win-tags" => "toggle-tags",
                "win-object-info" => "toggle-object-info",
                "win-debug-log" => "toggle-debug-log",
                _ => return,
            };
            let _ = app.emit("menu-action", action);
        })
        .build(tauri::generate_context!())
        .expect("error while building Hew desktop")
        .run(|app, event| {
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
                    // Also emit for the warm case (app already running).
                    let _ = app.emit("menu-open-path", &path_str);
                }
            }
        });
}

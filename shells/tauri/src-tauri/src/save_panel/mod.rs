//! The platform's own save panel with the format choice in it — the
//! convention every desktop app follows: macOS's "Format:" popup in the
//! panel, Windows' "Save as type" list, GTK's file-type filter — and the
//! chosen format's options beside it (SVG's view and scale, STL's curve
//! resolution). The stock dialog plugin can show a type list but never says
//! which entry was picked, and two formats can share an extension (SketchUp
//! 2026 and SketchUp 2017 are both `.skp`), so the choice has to come back
//! from the panel itself.
//!
//! The webview describes the formats ([`PanelFormat`]: label, extension,
//! options) and gets back the path, the format and every option's value
//! ([`PanelResult`]); it writes the bytes only after, since what to write
//! depends on the format picked.

mod model;
pub use model::{PanelFormat, PanelRequest, PanelResult};

#[cfg(all(unix, not(target_os = "macos")))]
mod gtk;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

/// What answers a request: the user's choice, `None` when they canceled,
/// or why the panel could not be shown.
pub type PanelAnswer = Result<Option<PanelResult>, String>;

/// Show the panel over `window` and wait for the user. Whatever the
/// platform's panel allowed, the path ends in the chosen format's
/// extension.
pub fn run(window: &tauri::WebviewWindow, request: PanelRequest) -> PanelAnswer {
    if request.formats.is_empty() {
        return Err("no formats to save in".into());
    }
    let extensions: Vec<(String, String)> = request
        .formats
        .iter()
        .map(|f| (f.id.clone(), f.extension.clone()))
        .collect();
    let mut answer = platform::run(window, request)?;
    if let Some(r) = answer.as_mut() {
        if let Some((_, ext)) = extensions.iter().find(|(id, _)| id == &r.format) {
            let typed = r.path.clone();
            r.path = model::with_extension(&r.path, ext);
            // The platform panel asked about replacing the name the user
            // typed, not the one the extension rule made of it (GTK re-asks
            // itself; macOS and Windows do not): never overwrite unasked.
            if let Some(refusal) =
                model::unasked_overwrite(&typed, &r.path, |p| std::path::Path::new(p).exists())
            {
                return Err(refusal);
            }
        }
    }
    Ok(answer)
}

/// The panel's answer from the main thread, or why none came.
fn received(rx: std::sync::mpsc::Receiver<PanelAnswer>) -> PanelAnswer {
    rx.recv()
        .map_err(|_| "the save panel closed without answering".to_string())?
}

#[cfg(target_os = "macos")]
mod platform {
    pub use super::macos::run;
}

#[cfg(windows)]
mod platform {
    use super::{received, PanelAnswer, PanelRequest};
    use std::sync::mpsc;

    pub fn run(window: &tauri::WebviewWindow, request: PanelRequest) -> PanelAnswer {
        let owner = window.hwnd().map_err(|e| e.to_string())?;
        let (tx, rx) = mpsc::channel::<PanelAnswer>();
        // The dialog is modal to the window and runs on its (main) thread.
        let owner = owner.0 as isize;
        window
            .run_on_main_thread(move || {
                let owner = ::windows::Win32::Foundation::HWND(owner as *mut _);
                let _ = tx.send(super::windows::show(owner, request));
            })
            .map_err(|e| e.to_string())?;
        received(rx)
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
mod platform {
    use super::{received, PanelAnswer, PanelRequest};
    use gtk::prelude::*;
    use std::sync::mpsc;

    pub fn run(window: &tauri::WebviewWindow, request: PanelRequest) -> PanelAnswer {
        let (tx, rx) = mpsc::channel::<PanelAnswer>();
        let parent = window.clone();
        window
            .run_on_main_thread(move || {
                // GTK objects live on the main thread only.
                let app_window = parent.gtk_window().ok();
                let parent = app_window.as_ref().map(|w| w.upcast_ref::<gtk::Window>());
                let _ = tx.send(Ok(super::gtk::show(parent, request)));
            })
            .map_err(|e| e.to_string())?;
        received(rx)
    }
}

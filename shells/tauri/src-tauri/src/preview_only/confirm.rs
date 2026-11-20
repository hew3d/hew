//! The native confirmation the plugin commands show before anything that
//! widens what a plugin may do or removes it: installing, widening a
//! grant, rolling back, turning a plugin on, and uninstalling.
//!
//! One contract on every platform: [`confirm_native`] answers true only
//! when the user activated the accept button itself. Cancel is the default
//! button and holds the focus, so Enter on a fresh dialog answers no;
//! Escape, closing the dialog or its window, any error showing it, and a
//! dialog that went away without answering are all no.
//!
//! The dialog plugin cannot be used for this: it has no default-button
//! setting, its default is the first button, and it reports a plain
//! cancel (Escape, closing the window) as the second label, so no button
//! order makes Enter and Escape both mean no.
//!
//! - Linux: a `gtk::MessageDialog`, modal and transient for the calling
//!   window, with Cancel added first, made the default response and given
//!   the focus. Shown from the main thread; the calling thread waits.
//! - Windows: `TaskDialogIndirect` with the accept button and the common
//!   Cancel button, Cancel the default, and dialog cancellation allowed
//!   (Escape and the close box answer Cancel).
//! - macOS: the dialog plugin's message box, as before (Enter there
//!   answers the accept button).

/// Shows a modal confirmation over `window` and blocks until it is
/// answered. True only when the button labeled `accept` was activated.
/// Must not be called on the main thread, which shows the dialog; called
/// there it answers false without showing anything.
pub fn confirm_native(
    window: &tauri::WebviewWindow,
    title: &str,
    body: &str,
    accept: &str,
) -> bool {
    platform::confirm(window, title, body, accept)
}

/// The answer from the main thread; a dialog that went away without
/// answering (its sender dropped) is a no. macOS shows the dialog plugin's
/// box and never waits this way.
#[cfg(not(target_os = "macos"))]
fn received(rx: std::sync::mpsc::Receiver<bool>) -> bool {
    rx.recv().unwrap_or(false)
}

#[cfg(all(unix, not(target_os = "macos")))]
mod platform {
    use gtk::prelude::*;
    use std::cell::Cell;
    use std::sync::mpsc;

    pub fn confirm(window: &tauri::WebviewWindow, title: &str, body: &str, accept: &str) -> bool {
        // The dialog runs on the main thread while this one waits, so on
        // the main thread itself it could never answer.
        if gtk::is_initialized_main_thread() {
            return false;
        }
        let (tx, rx) = mpsc::channel::<bool>();
        let parent = window.clone();
        let (title, body, accept) = (title.to_owned(), body.to_owned(), accept.to_owned());
        let shown = window.run_on_main_thread(move || {
            // GTK objects live on the main thread only.
            let app_window = parent.gtk_window().ok();
            let parent = app_window.as_ref().map(|w| w.upcast_ref::<gtk::Window>());
            show(parent, &title, &body, &accept, tx);
        });
        if shown.is_err() {
            return false;
        }
        super::received(rx)
    }

    /// Builds and shows the dialog; `tx` gets the one answer.
    pub fn show(
        parent: Option<&gtk::Window>,
        title: &str,
        body: &str,
        accept: &str,
        tx: mpsc::Sender<bool>,
    ) {
        // Plain text only: neither label reads markup.
        let dialog = gtk::MessageDialog::builder()
            .message_type(gtk::MessageType::Warning)
            .buttons(gtk::ButtonsType::None)
            .title(title)
            .text(title)
            .secondary_text(body)
            .use_markup(false)
            .secondary_use_markup(false)
            .modal(true)
            .destroy_with_parent(true)
            .build();
        if let Some(parent) = parent {
            dialog.set_transient_for(Some(parent));
        }
        let cancel = dialog.add_button("Cancel", gtk::ResponseType::Cancel);
        dialog.add_button(accept, gtk::ResponseType::Accept);
        dialog.set_default_response(gtk::ResponseType::Cancel);
        let answer = Cell::new(Some(tx));
        dialog.connect_response(move |dialog, response| {
            if let Some(tx) = answer.take() {
                // Escape, the window's close box and a destroyed parent all
                // come here as something other than Accept.
                let _ = tx.send(response == gtk::ResponseType::Accept);
            }
            dialog.close();
        });
        dialog.show_all();
        // After showing, so nothing GTK focuses on map takes it back.
        dialog.set_focus(Some(&cancel));
        cancel.grab_focus();
    }
}

#[cfg(windows)]
mod platform {
    use std::sync::mpsc;
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::UI::Controls::{
        TaskDialogIndirect, TASKDIALOGCONFIG, TASKDIALOG_BUTTON, TDCBF_CANCEL_BUTTON,
        TDF_ALLOW_DIALOG_CANCELLATION, TDF_POSITION_RELATIVE_TO_WINDOW, TD_WARNING_ICON,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::IDCANCEL;

    /// The accept button's id: not `IDOK`, so nothing but that button can
    /// produce it.
    const ACCEPT_ID: i32 = 1000;

    pub fn confirm(window: &tauri::WebviewWindow, title: &str, body: &str, accept: &str) -> bool {
        let Ok(owner) = window.hwnd() else {
            return false;
        };
        // A window handle is not `Send`; it crosses as a number.
        let owner = owner.0 as isize;
        let (tx, rx) = mpsc::channel::<bool>();
        let (title, body, accept) = (wide(title), wide(body), wide(accept));
        let shown = window.run_on_main_thread(move || {
            let _ = tx.send(show(owner as HWND, &title, &body, &accept));
        });
        if shown.is_err() {
            return false;
        }
        super::received(rx)
    }

    /// UTF-16 with one terminating NUL; any NUL inside is dropped so the
    /// text cannot end early.
    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16()
            .filter(|&c| c != 0)
            .chain(std::iter::once(0))
            .collect()
    }

    fn show(owner: HWND, title: &[u16], body: &[u16], accept: &[u16]) -> bool {
        let buttons = [TASKDIALOG_BUTTON {
            nButtonID: ACCEPT_ID,
            pszButtonText: accept.as_ptr(),
        }];
        let mut config = TASKDIALOGCONFIG {
            cbSize: std::mem::size_of::<TASKDIALOGCONFIG>() as u32,
            hwndParent: owner,
            dwFlags: TDF_ALLOW_DIALOG_CANCELLATION | TDF_POSITION_RELATIVE_TO_WINDOW,
            dwCommonButtons: TDCBF_CANCEL_BUTTON,
            pszWindowTitle: title.as_ptr(),
            pszMainInstruction: title.as_ptr(),
            pszContent: body.as_ptr(),
            cButtons: buttons.len() as u32,
            pButtons: buttons.as_ptr(),
            nDefaultButton: IDCANCEL,
            ..Default::default()
        };
        config.Anonymous1.pszMainIcon = TD_WARNING_ICON;
        let mut pressed: i32 = 0;
        // SAFETY: `config` and every string and button it points to live
        // until the call returns; the out-pointers we do not want are null,
        // which the API allows.
        let result = unsafe {
            TaskDialogIndirect(
                &config,
                &mut pressed,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        result == 0 && pressed == ACCEPT_ID
    }
}

#[cfg(target_os = "macos")]
mod platform {
    pub fn confirm(window: &tauri::WebviewWindow, title: &str, body: &str, accept: &str) -> bool {
        use tauri_plugin_dialog::{
            DialogExt, MessageDialogButtons, MessageDialogKind, MessageDialogResult,
        };
        let answer = window
            .dialog()
            .message(body)
            .title(title)
            .kind(MessageDialogKind::Warning)
            .buttons(MessageDialogButtons::OkCancelCustom(
                accept.to_string(),
                "Cancel".to_string(),
            ))
            .blocking_show_with_result();
        // Only the accept button itself counts; closing the dialog, Escape
        // and Cancel all come back as something else.
        answer == MessageDialogResult::Custom(accept.to_string())
    }
}

/// The Linux dialog under real input. Ignored in every ordinary run: it
/// needs a display, and something outside the test to press the keys. A
/// driver starts it on a display with `HEW_CONFIRM_DRIVEN=1`, reads the
/// dialog's window title and button positions from stdout, presses keys or
/// clicks, and reads the `ANSWER=` line.
#[cfg(all(test, unix, not(target_os = "macos")))]
mod tests {
    use gtk::prelude::*;
    use std::sync::mpsc;

    #[test]
    #[ignore = "needs a display and an input driver"]
    fn the_dialog_under_real_input() {
        if std::env::var_os("HEW_CONFIRM_DRIVEN").is_none() {
            return;
        }
        gtk::init().expect("a display");
        let (tx, rx) = mpsc::channel::<bool>();
        super::platform::show(
            None,
            "Install this plugin?",
            "Fixture 1.0.0\nUnsigned. Hew cannot tell who made this.",
            "Install",
            tx,
        );
        // Report where the two buttons are once the dialog is laid out.
        let mut reported = false;
        let answer = loop {
            if let Ok(answer) = rx.try_recv() {
                break answer;
            }
            if !reported {
                let dialog = gtk::Window::list_toplevels()
                    .into_iter()
                    .filter_map(|w| w.downcast::<gtk::MessageDialog>().ok())
                    .find(|d| d.is_mapped());
                if let Some(dialog) = dialog {
                    let buttons: Vec<gtk::Widget> =
                        [gtk::ResponseType::Cancel, gtk::ResponseType::Accept]
                            .into_iter()
                            .filter_map(|r| dialog.widget_for_response(r))
                            .collect();
                    let mut lines = Vec::new();
                    for b in &buttons {
                        let Some(label) = b.downcast_ref::<gtk::Button>().and_then(|b| b.label())
                        else {
                            continue;
                        };
                        let a = b.allocation();
                        if a.width() <= 1 {
                            continue;
                        }
                        let (x, y) = b
                            .translate_coordinates(&dialog, 0, 0)
                            .unwrap_or((a.x(), a.y()));
                        lines.push(format!(
                            "BUTTON {label} {} {} {} {}",
                            x,
                            y,
                            a.width(),
                            a.height()
                        ));
                    }
                    if lines.len() == 2 {
                        let focused = dialog
                            .focused_widget()
                            .and_then(|w| w.downcast::<gtk::Button>().ok())
                            .and_then(|b| b.label())
                            .map_or_else(|| "none".to_string(), |l| l.to_string());
                        for line in lines {
                            println!("{line}");
                        }
                        println!("FOCUS {focused}");
                        println!("READY");
                        use std::io::Write;
                        let _ = std::io::stdout().flush();
                        reported = true;
                    }
                }
            }
            gtk::main_iteration_do(false);
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        println!("ANSWER={answer}");
    }
}

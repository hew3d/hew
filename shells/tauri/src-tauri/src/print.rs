//! System printing for File ▸ Print… (docs/design/printing.md §9).
//!
//! The webview's DOM already holds the composed pages (`#hew-print-root`,
//! print-media CSS hides the rest of the app). All this module does is open
//! the platform's print operation for that DOM with the page geometry Hew
//! composed for — paper size, orientation, zero margins, 100 % — so a scaled
//! print comes out at exactly its scale, and hand the user the system dialog
//! (printer, copies, PDF destination). Nothing is rasterized here.
//!
//! Why Rust at all: WKWebView does not implement `window.print()`, and wry's
//! own `print()` is not exposed to JS and cannot set paper size (macOS
//! margins only). Per platform:
//!
//! - **macOS**: `NSPrintInfo.sharedPrintInfo` configured, then the
//!   `WKWebView` `printOperationWithPrintInfo:` operation (macOS 11+) run
//!   modally as a sheet on the window — the standard panel with the PDF menu.
//!   The selector is not in objc2-web-kit's generated `WKWebView`, so it is
//!   sent with `msg_send!` (wry hand-declares it the same way).
//! - **Windows**: WebView2 `ICoreWebView2_16::ShowPrintUI(SYSTEM)` — the
//!   system print dialog prints the DOM at 100 % honouring CSS `@page`.
//!   Deliberately not the browser-style Edge preview, whose default "Fit to
//!   printable area" quietly shrinks a scaled print.
//! - **Linux**: WebKitGTK `PrintOperation` with a `PageSetup` (paper,
//!   orientation, zero margins) and `run_dialog` — the GTK dialog with
//!   "Print to File".
//!
//! Any failure returns `Err(String)`; the JS side falls back to
//! `window.print()` where that works (Windows/Linux) and shows the error on
//! macOS.

use serde::Deserialize;

/// Page geometry the app composed for; sizes are already ORIENTED (width >
/// height for landscape).
#[derive(Debug, Clone, Deserialize)]
pub struct PrintSetup {
    pub paper_w_mm: f64,
    pub paper_h_mm: f64,
    // Only read on macOS and Linux; Windows' system print dialog owns
    // orientation itself.
    #[cfg_attr(target_os = "windows", allow(dead_code))]
    pub landscape: bool,
    // Only read on macOS, to set the print operation's job title; the other
    // platforms' native dialogs don't expose an equivalent field.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub job_title: String,
}

/// Default paper for the OS/locale, to seed the app's print preference once
/// (`settings/print.ts` — never overrides a later user choice).
#[derive(Debug, Clone, serde::Serialize)]
pub struct PrintDefaults {
    pub paper_w_mm: f64,
    pub paper_h_mm: f64,
    pub landscape: bool,
}

// Only used on macOS, converting to/from `NSPrintInfo`'s point-based sizes;
// the other platforms' print APIs are already millimetre-based.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
const MM_PER_PT: f64 = 25.4 / 72.0;

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn mm_to_pt(mm: f64) -> f64 {
    mm / MM_PER_PT
}

#[tauri::command]
pub async fn print_document(window: tauri::WebviewWindow, setup: PrintSetup) -> Result<(), String> {
    if !(setup.paper_w_mm.is_finite()
        && setup.paper_h_mm.is_finite()
        && setup.paper_w_mm > 10.0
        && setup.paper_h_mm > 10.0)
    {
        return Err("invalid paper size".into());
    }
    platform::print_document(&window, setup)
}

#[tauri::command]
pub async fn print_defaults(window: tauri::WebviewWindow) -> Result<Option<PrintDefaults>, String> {
    platform::print_defaults(&window)
}

/// Physical millimetres per LOGICAL (CSS) pixel of the display, so the print
/// preview's "100 %" can be a true 100 % on this screen instead of the CSS
/// 96 dpi assumption. `None` where the platform can't say (the CSS
/// assumption stands then).
#[tauri::command]
pub async fn screen_mm_per_px(window: tauri::WebviewWindow) -> Result<Option<f64>, String> {
    platform::screen_mm_per_px(&window)
}

// ---------------------------------------------------------------- macOS
#[cfg(target_os = "macos")]
mod platform {
    use super::{mm_to_pt, PrintDefaults, PrintSetup};
    use objc2::rc::Retained;
    use objc2::runtime::{NSObjectProtocol, ProtocolObject};
    use objc2::{msg_send, sel};
    use objc2_app_kit::{
        NSPaperOrientation, NSPrintInfo, NSPrintJobSavingURL, NSPrintOperation, NSPrintSaveJob,
        NSWindow,
    };
    use objc2_foundation::{NSNumber, NSSize, NSString, NSURL};
    use objc2_web_kit::WKWebView;
    use std::sync::mpsc;

    pub fn print_document(window: &tauri::WebviewWindow, setup: PrintSetup) -> Result<(), String> {
        let (tx, rx) = mpsc::channel::<Result<(), String>>();
        window
            .with_webview(move |pw| {
                let result = (|| -> Result<(), String> {
                    let wk_ptr = pw.inner() as *const WKWebView;
                    let ns_window_ptr = pw.ns_window() as *const NSWindow;
                    if wk_ptr.is_null() || ns_window_ptr.is_null() {
                        return Err("no webview".into());
                    }
                    // SAFETY: `with_webview` runs on the main thread and hands
                    // us the live WKWebView / NSWindow pointers for the
                    // duration of the closure.
                    unsafe {
                        let wk: &WKWebView = &*wk_ptr;
                        let ns_window: &NSWindow = &*ns_window_ptr;
                        if !wk.respondsToSelector(sel!(printOperationWithPrintInfo:)) {
                            return Err("printing needs macOS 11 or later (WKWebView printOperationWithPrintInfo:)".into());
                        }
                        let info = NSPrintInfo::sharedPrintInfo();
                        // Orientation first, then the ORIENTED paper size —
                        // AppKit swaps `paperSize` when the orientation flips.
                        info.setOrientation(if setup.landscape { NSPaperOrientation::Landscape } else { NSPaperOrientation::Portrait });
                        info.setPaperSize(NSSize::new(mm_to_pt(setup.paper_w_mm), mm_to_pt(setup.paper_h_mm)));
                        info.setTopMargin(0.0);
                        info.setBottomMargin(0.0);
                        info.setLeftMargin(0.0);
                        info.setRightMargin(0.0);
                        info.setScalingFactor(1.0);
                        info.setHorizontallyCentered(false);
                        info.setVerticallyCentered(false);
                        // Debug/test hook: `HEW_PRINT_PDF_PATH=/path/out.pdf` saves the
                        // job straight to a PDF with no panel — how the print path is
                        // verified with a ruler without clicking through the sheet.
                        // Debug builds only; a release binary ignores the variable.
                        let mut show_panel = true;
                        if cfg!(debug_assertions) {
                            if let Ok(path) = std::env::var("HEW_PRINT_PDF_PATH") {
                                if !path.is_empty() {
                                    // The real AppKit constants (their string values are NOT
                                    // the constant names — a hand-typed key silently falls back
                                    // to the default disposition, i.e. the printer).
                                    info.setJobDisposition(NSPrintSaveJob);
                                    let url = NSURL::fileURLWithPath(&NSString::from_str(&path));
                                    info.dictionary().setObject_forKey(&*url, ProtocolObject::from_ref(NSPrintJobSavingURL));
                                    if !info.jobDisposition().isEqualToString(NSPrintSaveJob) {
                                        // Never let a test hook reach a real printer.
                                        return Err("HEW_PRINT_PDF_PATH: could not set the save-to-PDF disposition".into());
                                    }
                                    show_panel = false;
                                }
                            }
                        }
                        let op: Retained<NSPrintOperation> = msg_send![wk, printOperationWithPrintInfo: &*info];
                        op.setJobTitle(Some(&NSString::from_str(&setup.job_title)));
                        op.setShowsPrintPanel(show_panel);
                        op.setShowsProgressPanel(show_panel);
                        op.setCanSpawnSeparateThread(true);
                        op.runOperationModalForWindow_delegate_didRunSelector_contextInfo(ns_window, None, None, std::ptr::null_mut());
                    }
                    Ok(())
                })();
                let _ = tx.send(result);
            })
            .map_err(|e| e.to_string())?;
        rx.recv().map_err(|e| e.to_string())?
    }

    pub fn print_defaults(_window: &tauri::WebviewWindow) -> Result<Option<PrintDefaults>, String> {
        // `sharedPrintInfo` reflects the user's default page setup (System
        // Settings ▸ Printers, or the last Page Setup). Read on the main
        // thread like everything AppKit; the command is async, so hop over.
        let (tx, rx) = mpsc::channel::<Option<PrintDefaults>>();
        _window
            .with_webview(move |_pw| {
                let d = {
                    let info = NSPrintInfo::sharedPrintInfo();
                    let size = info.paperSize();
                    let landscape = info.orientation() == NSPaperOrientation::Landscape;
                    PrintDefaults {
                        paper_w_mm: size.width * super::MM_PER_PT,
                        paper_h_mm: size.height * super::MM_PER_PT,
                        landscape,
                    }
                };
                let _ = tx.send(Some(d));
            })
            .map_err(|e| e.to_string())?;
        Ok(rx.recv().unwrap_or(None))
    }

    #[repr(C)]
    struct CGSize {
        width: f64,
        height: f64,
    }
    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGMainDisplayID() -> u32;
        fn CGDisplayScreenSize(display: u32) -> CGSize;
        fn CGDisplayPixelsWide(display: u32) -> usize;
    }

    /// The WINDOW's display: its physical width (EDID, mm) over its logical
    /// width (device pixels ÷ that screen's backing scale). Falls back to the
    /// main display when the window has no screen (off-screen).
    pub fn screen_mm_per_px(window: &tauri::WebviewWindow) -> Result<Option<f64>, String> {
        let (tx, rx) = mpsc::channel::<Option<f64>>();
        window
            .with_webview(move |pw| {
                let value = (|| -> Option<f64> {
                    let ns_window_ptr = pw.ns_window() as *const NSWindow;
                    if ns_window_ptr.is_null() {
                        return None;
                    }
                    // SAFETY: `with_webview` runs on the main thread with a
                    // live window; the CoreGraphics calls are plain C reads
                    // of the display configuration.
                    unsafe {
                        let ns_window = &*ns_window_ptr;
                        let (id, scale) = match ns_window.screen() {
                            Some(screen) => {
                                let key = NSString::from_str("NSScreenNumber");
                                let id = screen
                                    .deviceDescription()
                                    .objectForKey(&key)
                                    .and_then(|obj| obj.downcast::<NSNumber>().ok())
                                    .map(|n| n.unsignedIntValue())
                                    .unwrap_or_else(|| CGMainDisplayID());
                                (id, screen.backingScaleFactor())
                            }
                            None => (CGMainDisplayID(), ns_window.backingScaleFactor()),
                        };
                        let size = CGDisplayScreenSize(id);
                        let px = CGDisplayPixelsWide(id) as f64;
                        // NaN-safe: each must be a positive finite number.
                        let ok = |v: f64| v.is_finite() && v > 0.0;
                        if !ok(scale) || !ok(size.width) || !ok(px) {
                            return None;
                        }
                        let mm_per_px = size.width / (px / scale);
                        // A plausible desktop display is 0.1–0.5 mm per logical px.
                        if !(0.05..=1.0).contains(&mm_per_px) {
                            return None;
                        }
                        Some(mm_per_px)
                    }
                })();
                let _ = tx.send(value);
            })
            .map_err(|e| e.to_string())?;
        Ok(rx.recv().unwrap_or(None))
    }
}

// -------------------------------------------------------------- Windows
#[cfg(windows)]
mod platform {
    use super::{PrintDefaults, PrintSetup};

    /// Not measured here yet: the CSS 96 dpi assumption stands.
    pub fn screen_mm_per_px(_window: &tauri::WebviewWindow) -> Result<Option<f64>, String> {
        Ok(None)
    }
    use std::sync::mpsc;
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        ICoreWebView2_16, COREWEBVIEW2_PRINT_DIALOG_KIND_SYSTEM,
    };
    use windows_core::Interface;

    pub fn print_document(window: &tauri::WebviewWindow, _setup: PrintSetup) -> Result<(), String> {
        let (tx, rx) = mpsc::channel::<Result<(), String>>();
        window
            .with_webview(move |pw| {
                let result = (|| -> Result<(), String> {
                    // SAFETY: the controller is live for the closure; COM
                    // calls happen on the UI thread `with_webview` runs on.
                    unsafe {
                        let core = pw.controller().CoreWebView2().map_err(|e| e.to_string())?;
                        let wv16: ICoreWebView2_16 = core.cast().map_err(|e| {
                            format!("WebView2 runtime too old for ShowPrintUI: {e}")
                        })?;
                        wv16.ShowPrintUI(COREWEBVIEW2_PRINT_DIALOG_KIND_SYSTEM)
                            .map_err(|e| e.to_string())?;
                    }
                    Ok(())
                })();
                let _ = tx.send(result);
            })
            .map_err(|e| e.to_string())?;
        rx.recv().map_err(|e| e.to_string())?
    }

    pub fn print_defaults(_window: &tauri::WebviewWindow) -> Result<Option<PrintDefaults>, String> {
        // DEVMODE.dmPaperSize of the default printer (DMPAPER_* constants):
        // Letter 1, Legal 5, Tabloid 3, A3 8, A4 9, A5 11; dmOrientation
        // 1 = portrait, 2 = landscape. Anything else → None (locale rule).
        use windows_sys::Win32::Graphics::Gdi::{DEVMODEW, DMORIENT_LANDSCAPE};
        use windows_sys::Win32::Graphics::Printing::{
            ClosePrinter, DocumentPropertiesW, GetDefaultPrinterW, OpenPrinterW, PRINTER_HANDLE,
        };
        unsafe {
            let mut len: u32 = 0;
            GetDefaultPrinterW(std::ptr::null_mut(), &mut len);
            if len == 0 {
                return Ok(None);
            }
            let mut name = vec![0u16; len as usize];
            if GetDefaultPrinterW(name.as_mut_ptr(), &mut len) == 0 {
                return Ok(None);
            }
            let mut handle = PRINTER_HANDLE::default();
            if OpenPrinterW(name.as_mut_ptr(), &mut handle, std::ptr::null()) == 0 {
                return Ok(None);
            }
            let size = DocumentPropertiesW(
                std::ptr::null_mut(),
                handle,
                name.as_mut_ptr(),
                std::ptr::null_mut(),
                std::ptr::null(),
                0,
            );
            if size <= 0 {
                ClosePrinter(handle);
                return Ok(None);
            }
            // u64-backed so the DEVMODEW view is aligned.
            let mut buf = vec![0u64; (size as usize).div_ceil(8)];
            let dm = buf.as_mut_ptr() as *mut DEVMODEW;
            let rc = DocumentPropertiesW(
                std::ptr::null_mut(),
                handle,
                name.as_mut_ptr(),
                dm,
                std::ptr::null(),
                2, /* DM_OUT_BUFFER */
            );
            ClosePrinter(handle);
            if rc < 0 {
                return Ok(None);
            }
            let dm = &*dm;
            let paper = dm.Anonymous1.Anonymous1.dmPaperSize;
            let landscape = dm.Anonymous1.Anonymous1.dmOrientation == DMORIENT_LANDSCAPE as i16;
            let (w, h) = match paper {
                1 => (215.9, 279.4),
                5 => (215.9, 355.6),
                3 => (279.4, 431.8),
                8 => (297.0, 420.0),
                9 => (210.0, 297.0),
                11 => (148.0, 210.0),
                _ => return Ok(None),
            };
            Ok(Some(PrintDefaults {
                paper_w_mm: if landscape { h } else { w },
                paper_h_mm: if landscape { w } else { h },
                landscape,
            }))
        }
    }
}

// ---------------------------------------------------------------- Linux
#[cfg(all(unix, not(target_os = "macos")))]
mod platform {
    use super::{PrintDefaults, PrintSetup};

    /// Not measured here yet: the CSS 96 dpi assumption stands.
    pub fn screen_mm_per_px(_window: &tauri::WebviewWindow) -> Result<Option<f64>, String> {
        Ok(None)
    }
    use std::sync::mpsc;
    use webkit2gtk::PrintOperationExt;

    /// `with_webview`'s closure must be `Send`, but GTK types wrap a raw
    /// pointer and are intentionally `!Send` since GTK is single-threaded.
    /// Tauri guarantees the closure always runs on the main thread (see its
    /// `with_webview` doc comment), so it's safe to smuggle the window
    /// through this wrapper rather than relying on the compiler.
    struct MainThreadOnly<T>(T);
    unsafe impl<T> Send for MainThreadOnly<T> {}
    impl<T> MainThreadOnly<T> {
        // Takes `self` by value so the closure below is forced to capture
        // the whole wrapper (and thus its `Send` impl) rather than, under
        // 2021 disjoint closure capture, reaching in and capturing just the
        // inner field on its own — which would be `!Send` again.
        fn into_inner(self) -> T {
            self.0
        }
    }

    pub fn print_document(window: &tauri::WebviewWindow, setup: PrintSetup) -> Result<(), String> {
        let (tx, rx) = mpsc::channel::<Result<(), String>>();
        let gtk_window = MainThreadOnly(window.gtk_window().ok());
        window
            .with_webview(move |pw| {
                let gtk_window = gtk_window.into_inner();
                let result: Result<(), String> = {
                    let webview = pw.inner();
                    let op = webkit2gtk::PrintOperation::new(&webview);
                    let page_setup = gtk::PageSetup::new();
                    // Named GTK sizes keep the dialog's paper menu in sync;
                    // anything else is a custom size in mm.
                    let (w, h) = if setup.landscape {
                        (setup.paper_h_mm, setup.paper_w_mm)
                    } else {
                        (setup.paper_w_mm, setup.paper_h_mm)
                    };
                    let paper = match named_paper(w, h) {
                        Some(name) => gtk::PaperSize::new(Some(name)),
                        None => {
                            gtk::PaperSize::new_custom("hew-custom", "Custom", w, h, gtk::Unit::Mm)
                        }
                    };
                    page_setup.set_paper_size(&paper);
                    page_setup.set_orientation(if setup.landscape {
                        gtk::PageOrientation::Landscape
                    } else {
                        gtk::PageOrientation::Portrait
                    });
                    page_setup.set_top_margin(0.0, gtk::Unit::Mm);
                    page_setup.set_bottom_margin(0.0, gtk::Unit::Mm);
                    page_setup.set_left_margin(0.0, gtk::Unit::Mm);
                    page_setup.set_right_margin(0.0, gtk::Unit::Mm);
                    op.set_page_setup(&page_setup);
                    let settings = gtk::PrintSettings::new();
                    settings.set_scale(100.0);
                    op.set_print_settings(&settings);
                    op.run_dialog(gtk_window.as_ref());
                    Ok(())
                };
                let _ = tx.send(result);
            })
            .map_err(|e| e.to_string())?;
        rx.recv().map_err(|e| e.to_string())?
    }

    fn named_paper(w_mm: f64, h_mm: f64) -> Option<&'static str> {
        let near = |a: f64, b: f64| (a - b).abs() < 0.6;
        let (lo, hi) = if w_mm < h_mm {
            (w_mm, h_mm)
        } else {
            (h_mm, w_mm)
        };
        if near(lo, 215.9) && near(hi, 279.4) {
            Some("na_letter")
        } else if near(lo, 215.9) && near(hi, 355.6) {
            Some("na_legal")
        } else if near(lo, 279.4) && near(hi, 431.8) {
            Some("na_ledger")
        } else if near(lo, 210.0) && near(hi, 297.0) {
            Some("iso_a4")
        } else if near(lo, 297.0) && near(hi, 420.0) {
            Some("iso_a3")
        } else if near(lo, 148.0) && near(hi, 210.0) {
            Some("iso_a5")
        } else {
            None
        }
    }

    pub fn print_defaults(_window: &tauri::WebviewWindow) -> Result<Option<PrintDefaults>, String> {
        // GTK's locale default paper (`gtk_paper_size_get_default`).
        // (`gtk_paper_size_get_default` returns the default paper's NAME.)
        let Some(name) = gtk::PaperSize::default() else {
            return Ok(None);
        };
        let ps = gtk::PaperSize::new(Some(name.as_str()));
        let w = ps.width(gtk::Unit::Mm);
        let h = ps.height(gtk::Unit::Mm);
        if w > 0.0 && h > 0.0 {
            Ok(Some(PrintDefaults {
                paper_w_mm: w,
                paper_h_mm: h,
                landscape: false,
            }))
        } else {
            Ok(None)
        }
    }
}

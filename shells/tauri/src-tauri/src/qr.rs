//! QR rendering for the "Open on Phone" handoff (workers/share-relay's
//! README.md has the full design: the desktop encrypts the document in the
//! webview, uploads ciphertext to that Worker, and builds a
//! `https://app.hew3d.com/#recv=…` URL carrying the drop's token and
//! decryption key). Everything about the upload, the URL, and the
//! encryption itself lives entirely in the webview
//! (`app/src/panels/PhoneShareDialog.tsx`, `app/src/io/shareCrypto.ts`) —
//! this module's only job is turning that URL string into an SVG QR code,
//! which needs a native crate (`qrcode`) the webview doesn't have.
//!
//! This replaces the former `phone_share` module, which used to run a whole
//! LAN HTTP server (serving the bundled PWA plus the document bytes) so a
//! phone could fetch same-origin and dodge mixed-content restrictions. The
//! cloud dead-drop above makes that server unnecessary: the QR now points
//! at the already-HTTPS hosted app, which fetches the ciphertext itself.

use qrcode::render::svg;
use qrcode::QrCode;

/// Renders `text` as an SVG QR code string, embedded directly into the
/// dialog's markup by the frontend (an `<img src="data:image/svg+xml,…">`,
/// same as the LAN-server design used before it).
#[tauri::command]
pub fn qr_svg(text: String) -> Result<String, String> {
    render_qr_svg(&text)
}

fn render_qr_svg(text: &str) -> Result<String, String> {
    let code =
        QrCode::new(text.as_bytes()).map_err(|e| format!("could not build the QR code: {e}"))?;
    Ok(code.render::<svg::Color>().build())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_a_url_to_svg_markup() {
        let svg = render_qr_svg("https://app.hew3d.com/#recv=tok.key.name").expect("renders");
        assert!(svg.contains("<svg"));
    }

    #[test]
    fn rejects_data_too_large_for_a_qr_code() {
        // qrcode::QrCode::new refuses input beyond what any QR version can
        // encode — surfaced as an Err, not a panic, matching this module's
        // "no panic" posture the same way phone_share.rs's own render did.
        let huge = "a".repeat(10_000);
        assert!(render_qr_svg(&huge).is_err());
    }
}

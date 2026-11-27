fn main() {
    // The Preview wall's build-time flag (docs/dev/DEVELOPMENT.md "Preview
    // features"): `HEW_PREVIEW=1` builds a preview shell (`cfg(hew_preview)`),
    // which keeps every Preview feature's native menu item; any other value
    // is a production build, which prunes them (src/preview_menu.rs). The
    // same variable reaches `crates/api` through the webview's WASM and the
    // hew-cli sidecar, which are built in the same environment.
    println!("cargo::rustc-check-cfg=cfg(hew_preview)");
    println!("cargo::rustc-check-cfg=cfg(hew_updater)");
    println!("cargo::rerun-if-env-changed=HEW_PREVIEW");
    let preview = std::env::var_os("HEW_PREVIEW").is_some_and(|v| v == "1");
    if preview {
        println!("cargo::rustc-cfg=hew_preview");
        // The base64 SHA-256 of the plugin realm's boot script, which the
        // preview content security policy must allow (shells/csp). The
        // shell compares it with the policy it was configured with.
        let hash = std::path::Path::new("../../../app/src/plugins/boot/boot-1.hash");
        println!("cargo::rerun-if-changed={}", hash.display());
        let text = std::fs::read_to_string(hash)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", hash.display()));
        println!("cargo::rustc-env=HEW_PLUGIN_BOOT_HASH={}", text.trim());
    }
    // The in-app updater runs in a production build with the `updater`
    // feature, and never in a preview build: the one update feed is the
    // production `latest.json`, and a preview install must not replace
    // itself with a production release (nor the reverse). Preview testers
    // install each preview build themselves.
    if std::env::var_os("CARGO_FEATURE_UPDATER").is_some() && !preview {
        println!("cargo::rustc-cfg=hew_updater");
    }
    tauri_build::build()
}

//! The Preview wall's build-time flag (docs/dev/DEVELOPMENT.md "Preview
//! features"). `HEW_PREVIEW=1` in the build environment compiles this crate
//! with `cfg(hew_preview)`: the commands of every entry in
//! `preview-features.json` are registered and run. Any other value (unset,
//! `0`, empty) is a production build, where they are absent from every
//! listing and refuse `preview_only`. Nothing reads the variable at run
//! time.

fn main() {
    println!("cargo::rustc-check-cfg=cfg(hew_preview)");
    println!("cargo::rerun-if-env-changed=HEW_PREVIEW");
    if std::env::var_os("HEW_PREVIEW").is_some_and(|v| v == "1") {
        println!("cargo::rustc-cfg=hew_preview");
    }
}

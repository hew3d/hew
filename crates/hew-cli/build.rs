//! The Preview wall's build-time flag for this crate (docs/dev/DEVELOPMENT.md
//! "Preview features"). `HEW_PREVIEW=1` in the build environment compiles
//! this crate with `cfg(hew_preview)`, the same rule `crates/api/build.rs`
//! applies, so code that exists only for a Preview feature that loads code
//! is left out of a production build rather than refused in it. The crate
//! asserts at compile time that its flag agrees with the API crate's.

fn main() {
    println!("cargo::rustc-check-cfg=cfg(hew_preview)");
    println!("cargo::rerun-if-env-changed=HEW_PREVIEW");
    if std::env::var_os("HEW_PREVIEW").is_some_and(|v| v == "1") {
        println!("cargo::rustc-cfg=hew_preview");
    }
}

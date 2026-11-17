//! `hew-cli`'s logic, as a library — `src/main.rs` is a thin argv shell
//! over this crate so `crates/hew-cli/tests/cli.rs` can exercise the real
//! `run`/`dispatch`/`mcp` code paths directly, spawn-free (no subprocess,
//! no piping stdio).
//!
//! Headless is the default (docs/agents/HEW_API.md §12): every subcommand embeds
//! `crates/api` and the kernel directly through [`host::CliHost`]. `--live`
//! (discovering and forwarding to a running desktop instance over the
//! local socket transport, §11.2) is implemented in [`live`]; it never
//! touches a filesystem-backed document at all — every envelope goes to
//! the running app instead.

pub mod host;
pub mod live;
mod live_library;
pub mod mcp;
#[cfg(hew_preview)]
pub mod plugin;
pub mod print;
pub mod run;

pub use host::CliHost;

// This crate's own `cfg(hew_preview)` (build.rs) and the API crate's are set
// from the same variable by two build scripts; a build where they disagree
// would compile a Preview subcommand into a binary whose API refuses
// Preview commands, or the reverse, so it does not compile.
const _: () = assert!(
    cfg!(hew_preview) == api::preview::PREVIEW_BUILD,
    "hew-cli and api disagree about HEW_PREVIEW"
);

/// The version this binary reports: the release tag's version when the
/// release workflow stamped one at compile time (`HEW_RELEASE_VERSION`),
/// else the crate's own.
pub const VERSION: &str = version_from(
    option_env!("HEW_RELEASE_VERSION"),
    env!("CARGO_PKG_VERSION"),
);

/// `VERSION` from explicit inputs: the stamp when there is one, else the
/// package version. An empty stamp counts as unset.
pub const fn version_from(stamp: Option<&'static str>, package: &'static str) -> &'static str {
    match stamp {
        Some(v) if !v.is_empty() => v,
        _ => package,
    }
}

#[cfg(test)]
mod version_tests {
    use super::version_from;

    #[test]
    fn version_from_prefers_the_stamp() {
        assert_eq!(version_from(Some("1.5.0-pre.x"), "1.3.1"), "1.5.0-pre.x");
    }

    #[test]
    fn version_from_treats_an_empty_stamp_as_unset() {
        assert_eq!(version_from(Some(""), "1.3.1"), "1.3.1");
    }

    #[test]
    fn version_from_falls_back_to_the_package_version() {
        assert_eq!(version_from(None, "1.3.1"), "1.3.1");
    }
}

//! The plugin install store (docs/agents/PLUGINS.md "Installing").
//!
//! A plugin arrives as a `.hewplugin` package (a zip whose entries are all
//! stored, never compressed) or as an unpacked folder. This crate reads
//! either one into a [`Package`], refusing anything that is not a plain
//! tree of files with safe names; builds the [`ConsentPayload`] the consent
//! sheet shows; and, with the `fs` feature, keeps the on-disk store:
//! `plugins/<id>/{hew-plugin.json, installed.json, files/…, previous/}`.
//!
//! Plugins are a Preview feature that loads code. Everything here is
//! compiled only in a preview build; a production build compiles an empty
//! crate, so neither the store nor `HEW_PLUGIN_DIR` exists there.

// This crate's own `cfg(hew_preview)` (build.rs) and the API crate's are set
// from the same variable by two build scripts. A build where they disagree
// would compile the install store into a binary whose API refuses the
// plugin commands, or the reverse, so it does not compile.
const _: () = assert!(
    cfg!(hew_preview) == api::preview::PREVIEW_BUILD,
    "plugin-host and api disagree about HEW_PREVIEW"
);

/// The version types `hew_version_ok` and `stage` take, so a host needs no
/// direct dependency of its own.
#[cfg(hew_preview)]
pub use semver;

#[cfg(hew_preview)]
mod error;
#[cfg(hew_preview)]
pub use error::{InstallError, MAX_STORAGE_BYTES, MAX_STORAGE_KEY_BYTES};

#[cfg(hew_preview)]
pub mod package;
#[cfg(hew_preview)]
pub use package::{
    EntrySpan, MANIFEST_FILE, MAX_ENTRY_BYTES, MAX_PACKAGE_BYTES, MAX_PACKAGE_ENTRIES,
    MAX_PATH_BYTES, MAX_PERMISSIONS, Package, normalize_path, package_from_files, read_package_zip,
    read_package_zip_spans,
};

#[cfg(hew_preview)]
pub mod consent;
#[cfg(hew_preview)]
pub use consent::{
    CLEAN_TEXT_COMBINING, CLEAN_TEXT_REMOVED, ConsentPayload, FOLDER_LINE, MAX_COMBINING_RUN,
    MAX_NAME_CHARS, MAX_REASON_CHARS, MAX_SUMMARY_LINES, MAX_WARNING_HOSTS, PermissionLine,
    Replaces, UNSIGNED_LINE, UpdateDiff, clean_text, consent, consent_summary, display_name,
    enable_summary, grant_change_summary, hew_version_ok, install_confirmation, is_reinstall,
    permission_line, permission_warnings, reinstall_line, replaces_line, rollback_summary,
    uninstall_summary,
};

#[cfg(hew_preview)]
pub mod installed;
#[cfg(hew_preview)]
pub use installed::{
    INSTALLED_SCHEMA, InstallSource, Installed, RESERVED_ID_PREFIX, check_requires, check_source_id,
};

#[cfg(all(hew_preview, feature = "fs"))]
pub mod store;

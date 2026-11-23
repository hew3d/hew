//! `library` — the single owner of the Hew Library's on-disk layout
//! (docs/agents/HEW_API.md §8.1; docs/design/v1.1-cycle.md's Lane B).
//!
//! Before this crate, the on-disk convention (folder resolution, file
//! naming, item validation) was duplicated in TypeScript
//! (`app/src/library/fileNaming.ts`, `libraryModel.ts`,
//! `app/src/io/tauriLibraryStore.ts`) and in the Tauri shell
//! (`shells/tauri/src-tauri/src/main.rs`'s `library_*` commands). This
//! crate is the SECOND copy — the one hosts written in Rust share
//! (`hew-cli`'s `CliHost` for `hew.library.*`, and the Tauri shell's own
//! `library_*` commands, switched over in the same change) — while the web
//! shell keeps its TypeScript copy (`crates/wasm-api` has no filesystem of
//! its own, so there is nothing for a WASM-side crate to own there).
//!
//! Does real filesystem I/O (`std::fs`) — this crate is a HOST-side
//! dependency, not a kernel-class one. `crates/api` never depends on it
//! (see `crates/api/src/host.rs`'s module doc): a host implements
//! `api::Host`'s `library_*` methods over this crate's functions, and
//! `crates/api` only ever sees the plain data those methods hand back.

pub mod dir;
pub mod meta;
pub mod naming;
pub mod ops;
pub mod sources;

pub use dir::{LibraryDir, config_path};
pub use meta::{Category, ItemMeta, derive_category, display_name, parse_item_meta};
pub use naming::{
    CATEGORY_DIRS, item_file_name, valid_item_name, valid_source_cache_key, valid_thumb_key,
};
pub use ops::{
    ListedItem, list, read, read_thumbnail, remove, sha256_hex, thumbnail_path, write,
    write_thumbnail,
};
pub use sources::{
    SOURCES_CACHE_DIR, SOURCES_FILE, merge_source_entry, read_source_cache, read_sources,
    remove_source_cache, source_cache_path, write_source_cache, write_sources,
};

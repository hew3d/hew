//! Shared Library sources: the `hew.json` index, the catalog, source
//! references, request URLs and the public-host check
//! (docs/design/shared-library-sources.md, sections 6.1 to 6.4).
//!
//! The crate is pure: no I/O, no clock, no network. Hostile input (a pasted
//! reference, a downloaded index, a resolve response) yields a typed error
//! or `None`, never a panic.

use std::collections::BTreeMap;

use serde::Serialize;

mod build;
mod host;
mod index;
mod paths;
mod refs;
mod urls;

pub use build::build_index;
pub use host::is_public_host;
pub use index::{parse_catalog, parse_index};
pub use refs::{parse_source_ref, same_source_ignoring_ref};
pub use urls::{commit_from_response, file_url, index_url, resolve_url};

/// The newest index/catalog `hew` version this build understands.
pub const SUPPORTED_HEW: u64 = 1;
/// Largest accepted index or catalog, in bytes (6.1, 6.3).
pub const MAX_INDEX_BYTES: usize = 4 * 1024 * 1024;

/// The four kinds of source reference (6.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Github,
    Gitlab,
    Forgejo,
    Web,
}

/// Why an index or catalog was refused (6.1). `code()` is the stable
/// machine name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IndexError {
    /// `hew` is greater than [`SUPPORTED_HEW`].
    TooNew { found: u64 },
    /// The document is larger than [`MAX_INDEX_BYTES`].
    TooLarge,
    /// Any other violation, with the item's 0-based position where it applies.
    Invalid { reason: String, item: Option<usize> },
}

impl IndexError {
    /// `index_too_new`, `index_too_large` or `index_invalid`.
    pub fn code(&self) -> &'static str {
        match self {
            IndexError::TooNew { .. } => "index_too_new",
            IndexError::TooLarge => "index_too_large",
            IndexError::Invalid { .. } => "index_invalid",
        }
    }
}

/// Why a pasted source reference was refused (6.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefError {
    Empty,
    /// A query string is refused outright.
    QueryRefused,
    /// Any other reason (unknown scheme, `http`, missing repo, bad `dir`, ...).
    Invalid {
        reason: String,
    },
}

/// Why a relative path was refused (6.1 `path` rule, `file_url`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathError {
    Invalid { reason: String },
}

/// One source reference in canonical, parsed form (6.2). For `web`, `repo`
/// is the URL path of the folder holding `hew.json`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct SourceRef {
    pub kind: Kind,
    pub host: String,
    pub repo: String,
    pub dir: String,
    pub git_ref: Option<String>,
    pub canonical: String,
}

/// The index category, exact case (6.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Category {
    Components,
    Materials,
    Models,
    Profiles,
}

/// One entry of `items` (6.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IndexItem {
    pub id: String,
    pub path: String,
    pub name: String,
    pub category: Category,
    pub keywords: Vec<String>,
    pub collection: Option<String>,
    pub sha256: String,
    pub bytes: u64,
    pub thumbnail: Option<String>,
    pub license: Option<String>,
}

/// One well-shaped entry of `plugins` (section 9). Wrong-shaped entries are
/// dropped by `parse_index`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PluginEntry {
    pub id: String,
    pub version: String,
    pub package: String,
    pub sha256: String,
    pub bytes: u64,
    pub hew: u64,
    pub signature: String,
    pub key: String,
}

/// A parsed, validated `hew.json` (6.1). Serializes (camelCase, e.g.
/// `licenseUrl`) to a document `parse_index` accepts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceIndex {
    pub hew: u64,
    pub name: String,
    pub description: Option<String>,
    pub author: Option<String>,
    pub homepage: Option<String>,
    pub license: Option<String>,
    pub license_url: Option<String>,
    pub items: Vec<IndexItem>,
    pub plugins: Vec<PluginEntry>,
}

/// The header fields of an index (6.4), supplied to `build_index`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct IndexHeader {
    pub name: String,
    pub description: Option<String>,
    pub author: Option<String>,
    pub homepage: Option<String>,
    pub license: Option<String>,
    pub license_url: Option<String>,
}

/// `severity` of a `removed.json` entry (6.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Removed,
    Critical,
}

/// One entry of `removed` (6.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RemovedEntry {
    pub source: String,
    pub severity: Severity,
    pub reason: String,
    pub link: Option<String>,
}

/// One entry of the catalog's `sources` (6.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogSource {
    pub source: String,
    pub kind: Kind,
    pub commit: Option<String>,
    pub name: String,
    pub description: Option<String>,
    pub author: Option<String>,
    pub homepage: Option<String>,
    pub license: Option<String>,
    pub license_url: Option<String>,
    pub items: u64,
    pub bytes: u64,
    pub categories: BTreeMap<String, u64>,
    pub stale: bool,
    pub snapshot: String,
}

/// A parsed `catalog.json` (6.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Catalog {
    pub hew: u64,
    pub built: String,
    pub sources: Vec<CatalogSource>,
    pub removed: Vec<RemovedEntry>,
}

/// The metadata `build_index` reads from a local item (6.4). The crate does
/// not depend on `library`; the caller maps its own metadata into this.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LocalMeta {
    pub id: Option<String>,
    pub name: Option<String>,
    pub category: Option<Category>,
    pub keywords: Vec<String>,
    pub collection: Option<String>,
}

/// A `.hew` file read from a folder, for `build_index` (6.4).
#[derive(Debug, Clone, PartialEq)]
pub struct LocalItem {
    pub path: String,
    pub bytes: Vec<u8>,
    pub meta: LocalMeta,
}

//! `build_index`, the core of `hew-cli library index` (6.4).

use sha2::{Digest, Sha256};

use crate::{Category, IndexHeader, IndexItem, LocalItem, SUPPORTED_HEW, SourceIndex};

/// Builds the index for a folder's items.
///
/// Infallible by design: the caller guarantees valid input (every item has
/// `meta.id`, `meta.name` and `meta.category`; paths and ids are valid and
/// unique). Missing metadata falls back to an empty string or `Components`
/// rather than panicking; such an index fails `parse_index`, so a caller that
/// broke the contract finds out at the next read.
///
/// `items` are sorted by `path` in UTF-8 byte order, so the output never
/// depends on the order of the folder walk. Each entry carries the SHA-256
/// and length of its file. No thumbnail is named: rendering one is the CLI's
/// work, and it sets `thumbnail` itself when one exists.
pub fn build_index(items: &[LocalItem], header: IndexHeader) -> SourceIndex {
    let mut entries: Vec<IndexItem> = items.iter().map(entry).collect();
    entries.sort_by(|a, b| a.path.as_bytes().cmp(b.path.as_bytes()));
    SourceIndex {
        hew: SUPPORTED_HEW,
        name: header.name,
        description: header.description,
        author: header.author,
        homepage: header.homepage,
        license: header.license,
        license_url: header.license_url,
        items: entries,
        plugins: Vec::new(),
    }
}

fn entry(item: &LocalItem) -> IndexItem {
    let meta = &item.meta;
    IndexItem {
        id: meta.id.clone().unwrap_or_default(),
        path: item.path.clone(),
        name: meta.name.clone().unwrap_or_default(),
        category: meta.category.unwrap_or(Category::Components),
        keywords: meta.keywords.clone(),
        collection: meta.collection.clone(),
        sha256: sha256_hex(&item.bytes),
        bytes: item.bytes.len() as u64,
        thumbnail: None,
        license: None,
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

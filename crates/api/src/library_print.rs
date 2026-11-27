//! Content prints (OW-2, Library updates): the kernel writes a definition's
//! canonical content stream ([`kernel::Document::content_print_stream`])
//! and this module hashes it with SHA-256, so the kernel gains no
//! dependency. A print is stored as text, `p1:<hex>`.
//!
//! Where prints live: a linked definition's `hew.library` stamp holds its
//! base under `print`; a Library item's own `hew.library` metadata holds
//! `prints`, a map from each definition's stable id in the item file (as a
//! decimal string) to its print, written by every save to the Library.

use std::collections::BTreeMap;

use kernel::{AttrTarget, AttrValue, ComponentId, Document, DocumentError};
use sha2::{Digest, Sha256};

/// The item-metadata key of an item's prints (`hew.library` / `prints`).
pub const PRINTS_KEY: &str = "prints";

/// SHA-256 of `bytes`: the digest every content print is made with
/// ([`kernel::PrintDigest`]).
pub fn sha256_digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

/// Definition `def`'s content print, `p1:<hex>`.
///
/// # Errors
/// [`DocumentError::UnknownComponent`] for a stale or hidden definition.
pub fn content_print(doc: &Document, def: ComponentId) -> Result<String, DocumentError> {
    doc.content_print(def, sha256_digest)
}

/// Every live definition's print by its stable id: the `prints` map an
/// item carries, computed from the item itself.
pub fn item_prints(doc: &Document) -> BTreeMap<u64, String> {
    doc.content_prints_by_sid(sha256_digest)
}

/// `prints` as the attribute value an item stores: a map keyed by the
/// decimal stable id.
pub fn prints_value(prints: &BTreeMap<u64, String>) -> AttrValue {
    AttrValue::Map(
        prints
            .iter()
            .map(|(sid, p)| (sid.to_string(), AttrValue::Text(p.clone())))
            .collect(),
    )
}

/// Writes `doc`'s own prints into its `hew.library` metadata (`prints`),
/// replacing any it carried: what every save to the Library does just
/// before writing the item.
///
/// # Errors
/// As [`kernel::Document::attr_set`] (none for a valid map).
pub fn stamp_item_prints(doc: &mut Document) -> Result<BTreeMap<u64, String>, DocumentError> {
    let prints = item_prints(doc);
    doc.attr_set(
        AttrTarget::Document,
        "hew.library",
        PRINTS_KEY,
        prints_value(&prints),
    )?;
    Ok(prints)
}

/// The prints an item's `hew.library` metadata carries, read from the
/// manifest summary's document attributes (JSON, `namespace → key →
/// value`). `None` when the item has no `prints` (saved before 1.5), when
/// the map is malformed, or when any print is under another scheme than
/// this build's: the caller then computes them from the item file.
pub fn stored_prints_from_json(doc_attrs: &serde_json::Value) -> Option<BTreeMap<u64, String>> {
    let map = doc_attrs.get("hew.library")?.get(PRINTS_KEY)?.as_object()?;
    let mut out = BTreeMap::new();
    for (sid, print) in map {
        let sid: u64 = sid.parse().ok()?;
        let print = print.as_str()?;
        if !kernel::is_current_print(print) {
            return None;
        }
        out.insert(sid, print.to_string());
    }
    Some(out)
}

/// [`stored_prints_from_json`] for a loaded item.
pub fn stored_prints(doc: &Document) -> Option<BTreeMap<u64, String>> {
    let dict = doc.attr_get(&AttrTarget::Document).ok().flatten()?;
    let AttrValue::Map(map) = dict.get("hew.library")?.get(PRINTS_KEY)? else {
        return None;
    };
    let mut out = BTreeMap::new();
    for (sid, print) in map {
        let sid: u64 = sid.parse().ok()?;
        let AttrValue::Text(print) = print else {
            return None;
        };
        if !kernel::is_current_print(print) {
            return None;
        }
        out.insert(sid, print.clone());
    }
    Some(out)
}

/// The provenance a Library insert or update stamps: the item's id, the
/// file's fingerprint, and the item's prints computed from the item
/// itself (never trusted from its metadata, so a base is always what the
/// copy really holds).
pub fn provenance(
    item: &Document,
    source_id: String,
    content_hash: String,
) -> kernel::LibraryProvenance {
    kernel::LibraryProvenance {
        source_id,
        content_hash,
        prints: item_prints(item),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_print_is_p1_and_sixty_four_hex_digits() {
        let text = kernel::content_print_text(&sha256_digest(b"x"));
        assert!(text.starts_with("p1:"));
        assert_eq!(text.len(), 3 + 64);
        assert!(kernel::is_current_print(&text));
    }

    #[test]
    fn stored_prints_read_back_and_a_foreign_scheme_reads_as_none() {
        let ok = serde_json::json!({ "hew.library": { "prints": { "7": "p1:ab" } } });
        assert_eq!(
            stored_prints_from_json(&ok),
            Some(BTreeMap::from([(7, "p1:ab".to_string())]))
        );
        let foreign = serde_json::json!({ "hew.library": { "prints": { "7": "p2:ab" } } });
        assert_eq!(stored_prints_from_json(&foreign), None);
        let missing = serde_json::json!({ "hew.library": { "id": "x" } });
        assert_eq!(stored_prints_from_json(&missing), None);
    }
}

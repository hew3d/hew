//! Item metadata: the `hew.library` document attribute dictionary parsed
//! the SAME way `app/src/library/libraryModel.ts` does (`parseLibraryMeta`,
//! `deriveCategory`), so a listing built here and a listing built by the web
//! app agree on every item's category and display name. The on-disk keys
//! are the exact camelCase ones the UI writes (`App.tsx`'s save flow):
//! `id`, `name`, `category`, `keywords`, `collection`, `savedAt`,
//! `sourceDoc`.

use kernel::ItemSummary;

/// The curatorial category of a library item — mirrors
/// `app/src/library/types.ts`'s `LibraryCategory`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Category {
    Component,
    Material,
    Model,
    Profile,
}

impl Category {
    /// Parses the wire/JSON spelling (`"component"` | `"material"` |
    /// `"model"` | `"profile"`); `None` for anything else. Named `parse` rather than
    /// `from_str` to avoid the inherent-method/`FromStr`-trait-method
    /// name clash clippy flags (`should_implement_trait`).
    pub fn parse(s: &str) -> Option<Category> {
        Some(match s {
            "component" => Category::Component,
            "material" => Category::Material,
            "model" => Category::Model,
            "profile" => Category::Profile,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Category::Component => "component",
            Category::Material => "material",
            Category::Model => "model",
            Category::Profile => "profile",
        }
    }
}

/// The `hew.library` metadata carried in an item file's own document
/// attribute dictionary (docs/agents/HEW_API.md §8.1) — mirrors
/// `LibraryItemMeta` in `app/src/library/types.ts`. Every field optional: a
/// bare `.hew` dropped into the library folder is a valid item with
/// everything derived.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ItemMeta {
    /// Stable library identity (a UUID string minted at save). Drives
    /// idempotent re-insert and "in this model" matching.
    pub id: Option<String>,
    /// Display name. Falls back to the item's own definition/root name,
    /// then the file name (see [`display_name`]).
    pub name: Option<String>,
    pub category: Option<Category>,
    pub keywords: Vec<String>,
    /// User collection name (`None` = uncollected).
    pub collection: Option<String>,
    /// ISO-8601 save timestamp.
    pub saved_at: Option<String>,
    /// The document the item was saved out of (display only).
    pub source_doc: Option<String>,
    /// The content print of each definition the item holds, by its stable
    /// id in the item file (`prints`, written by every save since Library
    /// updates, OW-2): what a model using the item compares its own
    /// definitions against. `None` for an item saved before prints existed
    /// (or a malformed map): its prints are computed from the file.
    pub prints: Option<std::collections::BTreeMap<u64, String>>,
}

/// Parses a `hew.library` attribute-dictionary value (as JSON — e.g.
/// `ItemSummary::doc_attrs["hew.library"]`) into an [`ItemMeta`]. Never
/// fails: anything not shaped like an object yields the default (empty)
/// meta, and each field is only carried across if it has the right JSON
/// type — a malformed field (say, `id` as a number) is dropped rather than
/// poisoning the whole item. Mirrors `parseLibraryMeta` in
/// `app/src/library/libraryModel.ts` exactly.
pub fn parse_item_meta(raw: &serde_json::Value) -> ItemMeta {
    let serde_json::Value::Object(obj) = raw else {
        return ItemMeta::default();
    };
    let mut meta = ItemMeta::default();
    if let Some(id) = obj.get("id").and_then(|v| v.as_str()) {
        meta.id = Some(id.to_string());
    }
    if let Some(name) = obj.get("name").and_then(|v| v.as_str()) {
        meta.name = Some(name.to_string());
    }
    if let Some(category) = obj
        .get("category")
        .and_then(|v| v.as_str())
        .and_then(Category::parse)
    {
        meta.category = Some(category);
    }
    if let Some(keywords) = obj.get("keywords").and_then(|v| v.as_array()) {
        // All-or-nothing, like the TS `isStringArray` guard: a keywords
        // array with even one non-string entry is not "keywords" at all.
        if keywords.iter().all(|k| k.is_string()) {
            meta.keywords = keywords
                .iter()
                .map(|k| k.as_str().unwrap_or_default().to_string())
                .collect();
        }
    }
    if let Some(collection) = obj.get("collection").and_then(|v| v.as_str()) {
        meta.collection = Some(collection.to_string());
    }
    if let Some(saved_at) = obj.get("savedAt").and_then(|v| v.as_str()) {
        meta.saved_at = Some(saved_at.to_string());
    }
    if let Some(source_doc) = obj.get("sourceDoc").and_then(|v| v.as_str()) {
        meta.source_doc = Some(source_doc.to_string());
    }
    // All-or-nothing, like keywords: one malformed entry and the item
    // reads as having no prints (they are then computed from the file).
    if let Some(prints) = obj.get("prints").and_then(|v| v.as_object()) {
        meta.prints = prints
            .iter()
            .map(|(sid, p)| Some((sid.parse::<u64>().ok()?, p.as_str()?.to_string())))
            .collect();
    }
    meta
}

/// Derives a category from manifest shape alone, for an item whose
/// metadata doesn't already pin one (a caller's explicit `meta.category`
/// always wins over this). Mirrors `deriveCategory` in
/// `app/src/library/libraryModel.ts`: a material-only file (no solids, no
/// component definitions, at least one palette entry) is a material; a
/// file holding exactly one component definition and no top-level groups
/// is a component; everything else is a model.
pub fn derive_category(summary: &ItemSummary) -> Category {
    // A profile file: one open drawn chain and nothing else.
    if summary.objects == 0
        && summary.components == 0
        && summary.materials == 0
        && summary.profile_chain_only
    {
        return Category::Profile;
    }
    let material_only = summary.objects == 0 && summary.components == 0 && summary.materials > 0;
    if material_only {
        return Category::Material;
    }
    let single_definition = summary.components == 1 && summary.groups == 0;
    if single_definition {
        return Category::Component;
    }
    Category::Model
}

/// The bare file name minus any category subfolder and its `.hew`
/// extension, case-INsensitively — matching
/// [`crate::naming::valid_item_name`]'s own
/// `to_ascii_lowercase().ends_with(".hew")` check, so a name this module
/// accepts as valid (`Chair.HEW`, say) gets its extension actually
/// stripped here too rather than surviving into the displayed name — the
/// last-resort display name for an item with no metadata and no named
/// definition/root. Mirrors `fileStem` in `libraryModel.ts`. `pub(crate)`:
/// [`crate::ops::list_into`]'s errored-item fallback uses this exact
/// helper rather than a second hand-rolled copy.
pub(crate) fn file_stem(rel_path: &str) -> String {
    let base = rel_path.rsplit('/').next().unwrap_or(rel_path);
    let stem = if base.len() >= 4 && base[base.len() - 4..].eq_ignore_ascii_case(".hew") {
        &base[..base.len() - 4]
    } else {
        base
    };
    if stem.is_empty() {
        rel_path.to_string()
    } else {
        stem.to_string()
    }
}

/// The display-name fallback chain: explicit meta name, then the first
/// component definition's name, then the first root node's name, then the
/// bare file stem — mirrors `buildLibraryItem`'s `displayName` derivation.
pub fn display_name(meta: &ItemMeta, summary: &ItemSummary, rel_path: &str) -> String {
    meta.name
        .clone()
        .or_else(|| summary.first_component_name.clone())
        .or_else(|| summary.first_root_name.clone())
        .unwrap_or_else(|| file_stem(rel_path))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A profile item's bytes: one bundled shape as an open drawn chain.
    fn profile_summary() -> ItemSummary {
        let (_, _, make) = kernel::bundled_profiles()[0];
        let doc = kernel::Document::profile_document_from_shape(&make(), 0.05).unwrap();
        kernel::read_item_summary(&doc.save()).unwrap()
    }

    #[test]
    fn profile_is_a_category_and_derives_from_one_open_chain() {
        assert_eq!(Category::parse("profile"), Some(Category::Profile));
        assert_eq!(Category::Profile.as_str(), "profile");
        let mut summary = profile_summary();
        assert!(summary.profile_chain_only);
        assert_eq!(derive_category(&summary), Category::Profile);
        // What an old reader computes: ignoring the new flag, a file with
        // no objects, components or materials is a plain model.
        summary.profile_chain_only = false;
        assert_eq!(derive_category(&summary), Category::Model);
    }

    #[test]
    fn parse_item_meta_reads_every_field_of_a_well_formed_dict() {
        let raw = serde_json::json!({
            "id": "abc-123",
            "name": "Chair",
            "category": "component",
            "keywords": ["seat", "wood"],
            "collection": "Furniture/Seating",
            "savedAt": "2026-01-01T00:00:00.000Z",
            "sourceDoc": "house.hew",
        });
        let meta = parse_item_meta(&raw);
        assert_eq!(meta.id.as_deref(), Some("abc-123"));
        assert_eq!(meta.name.as_deref(), Some("Chair"));
        assert_eq!(meta.category, Some(Category::Component));
        assert_eq!(meta.keywords, vec!["seat".to_string(), "wood".to_string()]);
        assert_eq!(meta.collection.as_deref(), Some("Furniture/Seating"));
        assert_eq!(meta.saved_at.as_deref(), Some("2026-01-01T00:00:00.000Z"));
        assert_eq!(meta.source_doc.as_deref(), Some("house.hew"));
        // Saved before Library updates: no prints.
        assert_eq!(meta.prints, None);
    }

    #[test]
    fn parse_item_meta_reads_the_prints_map_whole_or_not_at_all() {
        let raw = serde_json::json!({ "prints": { "3": "p1:aa", "9": "p1:bb" } });
        let prints = parse_item_meta(&raw).prints.expect("well formed");
        assert_eq!(prints.get(&3).map(String::as_str), Some("p1:aa"));
        assert_eq!(prints.get(&9).map(String::as_str), Some("p1:bb"));
        let bad_key = serde_json::json!({ "prints": { "x": "p1:aa" } });
        assert_eq!(parse_item_meta(&bad_key).prints, None);
        let bad_value = serde_json::json!({ "prints": { "3": 5 } });
        assert_eq!(parse_item_meta(&bad_value).prints, None);
        let empty = serde_json::json!({ "prints": {} });
        assert_eq!(parse_item_meta(&empty).prints, Some(Default::default()));
    }

    #[test]
    fn parse_item_meta_degrades_gracefully_on_malformed_input() {
        assert_eq!(
            parse_item_meta(&serde_json::json!(null)),
            ItemMeta::default()
        );
        assert_eq!(
            parse_item_meta(&serde_json::json!([1, 2])),
            ItemMeta::default()
        );
        // Wrong-typed id, invalid category, mixed-type keywords: each
        // field drops independently rather than poisoning the whole item.
        let raw = serde_json::json!({
            "id": 42,
            "name": "Still Named",
            "category": "not-a-real-category",
            "keywords": ["ok", 5],
        });
        let meta = parse_item_meta(&raw);
        assert_eq!(meta.id, None);
        assert_eq!(meta.name.as_deref(), Some("Still Named"));
        assert_eq!(meta.category, None);
        assert!(meta.keywords.is_empty());
    }

    fn summary_with(
        objects: usize,
        components: usize,
        groups: usize,
        materials: usize,
    ) -> ItemSummary {
        ItemSummary {
            format_version: 1,
            objects,
            materials,
            components,
            instances: 0,
            groups,
            world_sketches: 0,
            annotations: 0,
            guides: 0,
            first_component_name: None,
            first_component_sid: None,
            first_root_name: None,
            doc_attrs: serde_json::json!({}),
            material_entries: Vec::new(),
            profile_chain_only: false,
        }
    }

    #[test]
    fn derive_category_matches_the_ts_rules() {
        assert_eq!(
            derive_category(&summary_with(0, 0, 0, 1)),
            Category::Material
        );
        assert_eq!(
            derive_category(&summary_with(1, 1, 0, 0)),
            Category::Component
        );
        // A group present disqualifies the single-definition shortcut.
        assert_eq!(derive_category(&summary_with(1, 1, 1, 0)), Category::Model);
        assert_eq!(derive_category(&summary_with(3, 0, 0, 0)), Category::Model);
        assert_eq!(derive_category(&summary_with(0, 0, 0, 0)), Category::Model);
    }

    #[test]
    fn display_name_follows_the_fallback_chain() {
        let summary = summary_with(1, 1, 0, 0);
        let mut with_name_summary = summary.clone();
        with_name_summary.first_component_name = Some("Widget".to_string());
        let meta = ItemMeta {
            name: Some("Explicit".to_string()),
            ..Default::default()
        };
        assert_eq!(
            display_name(&meta, &with_name_summary, "Components/x-abcdef.hew"),
            "Explicit"
        );

        let meta = ItemMeta::default();
        assert_eq!(
            display_name(&meta, &with_name_summary, "Components/x-abcdef.hew"),
            "Widget"
        );

        let mut root_only = summary.clone();
        root_only.first_root_name = Some("Root".to_string());
        assert_eq!(
            display_name(&meta, &root_only, "Components/x-abcdef.hew"),
            "Root"
        );

        assert_eq!(
            display_name(&meta, &summary, "Components/x-abcdef.hew"),
            "x-abcdef"
        );
    }
}

//! `hew.library.*` — the Hew Library for hosts with a filesystem
//! (docs/agents/HEW_API.md §8.1; docs/design/v1.1-cycle.md's Lane B).
//!
//! `list`/`describe`/`remove`/`update_meta` never touch the attached
//! document — they run entirely through [`crate::host::Host`]'s
//! `library_*` methods, which a filesystem-less host (the wasm live
//! boundary) refuses `host_capability_missing`. `insert` and `save` DO
//! touch the document, through the same kernel entry points
//! (`Document::insert_document`, `Document::extract_item`,
//! `Document::stamp_library_source`) `crates/wasm-api`'s own
//! `Scene::insert_item`/`extract_item`/`stamp_library_source` wrap for the
//! UI — an API insert or save is indistinguishable from a UI one.
//!
//! Every `item` parameter below (an id or a relative path) is resolved
//! against ONE [`Host::library_list`] call — `crates/api` never asks a
//! host to resolve an id itself, so the id-or-path matching rule lives in
//! exactly one place ([`find_item`]).

use super::doc::{decode_base64, encode_base64};
use super::entity::resolve_node;
use super::{CmdError, Ctx, Handler};
use crate::host::{LibraryItemEntry, LibraryWriteTarget};
use crate::refusal::Refusal;
use kernel::{
    AttrTarget, AttrValue, Document, EntityRef, InsertOptions, LibraryProvenance, NodeId, Transform,
};
use serde::Deserialize;
use serde_json::Value;

/// This namespace's slice of the handler table.
pub fn handler(name: &str) -> Option<Handler> {
    Some(match name {
        "hew.library.list" => list_items,
        "hew.library.describe" => describe_item,
        "hew.library.insert" => insert_item,
        "hew.library.save" => save_item,
        "hew.library.remove" => remove_item,
        "hew.library.update_meta" => update_meta,
        _ => return None,
    })
}

fn parse<T: for<'de> Deserialize<'de>>(params: &Value) -> Result<T, CmdError> {
    serde_json::from_value(params.clone()).map_err(|e| CmdError::Params(e.to_string()))
}

/// A `hew.library.*` `item` (or `hew.library.list`'s `category`) that
/// doesn't name a live library item / a valid category.
fn unknown_library_item(item: &str) -> CmdError {
    CmdError::Refusal(
        Refusal::api(
            "unknown_library_item",
            &format!("'{item}' does not name an item in the library (by id or path)."),
        )
        .with_detail(serde_json::json!({ "item": item })),
    )
}

/// Parses `s` as one of the three category strings, refusing typed
/// (`CmdError::Params`) for anything else — the enum every `category`
/// param in this module shares.
fn parse_category(s: &str) -> Result<&'static str, CmdError> {
    match s {
        "component" => Ok("component"),
        "material" => Ok("material"),
        "model" => Ok("model"),
        _ => Err(CmdError::Params(format!(
            "category must be one of \"component\", \"material\", \"model\" — got {s:?}"
        ))),
    }
}

/// The entry whose `path` or `id` equals `item` — path checked first (a
/// path can never collide with a minted UUID in practice, but checking it
/// first is the deterministic tie-break either way).
fn find_item<'a>(entries: &'a [LibraryItemEntry], item: &str) -> Option<&'a LibraryItemEntry> {
    entries
        .iter()
        .find(|e| e.path == item)
        .or_else(|| entries.iter().find(|e| e.id.as_deref() == Some(item)))
}

fn resolve_entry(ctx: &Ctx, item: &str) -> Result<LibraryItemEntry, CmdError> {
    let listing = ctx.host.library_list().map_err(CmdError::Refusal)?;
    find_item(&listing.items, item)
        .cloned()
        .ok_or_else(|| unknown_library_item(item))
}

// --------------------------------------------------------- shared shaping

fn entry_to_json(e: &LibraryItemEntry) -> Value {
    let mut v = serde_json::json!({
        "path": e.path,
        "name": e.name,
        "category": e.category,
        "keywords": e.keywords,
        "size": e.size,
        "mtime_ms": e.mtime_ms,
    });
    let obj = v.as_object_mut().expect("built as an object above");
    if let Some(id) = &e.id {
        obj.insert("id".to_string(), Value::String(id.clone()));
    }
    if let Some(collection) = &e.collection {
        obj.insert("collection".to_string(), Value::String(collection.clone()));
    }
    if let Some(saved_at) = &e.saved_at {
        obj.insert("saved_at".to_string(), Value::String(saved_at.clone()));
    }
    if let Some(summary) = &e.summary {
        obj.insert(
            "summary".to_string(),
            serde_json::to_value(summary).expect("ItemSummary serializes"),
        );
    }
    if let Some(error) = &e.error {
        obj.insert("error".to_string(), Value::String(error.clone()));
    }
    v
}

fn collection_segments(raw: &str) -> Vec<&str> {
    raw.split('/')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect()
}

/// True when `item_collection` is `filter_path` itself or lives anywhere
/// in its subtree — matched on whole path segments, mirroring
/// `collectionMatchesSubtree` in `app/src/library/libraryModel.ts`.
fn collection_matches_subtree(item_collection: Option<&str>, filter_path: &str) -> bool {
    let Some(item_collection) = item_collection else {
        return false;
    };
    let filter_segments = collection_segments(filter_path);
    if filter_segments.is_empty() {
        return false;
    }
    let item_segments = collection_segments(item_collection);
    if item_segments.len() < filter_segments.len() {
        return false;
    }
    filter_segments
        .iter()
        .zip(item_segments.iter())
        .all(|(a, b)| a == b)
}

fn matches_query(entry: &LibraryItemEntry, query: &str) -> bool {
    if entry.name.to_lowercase().contains(query) {
        return true;
    }
    entry
        .keywords
        .iter()
        .any(|k| k.to_lowercase().contains(query))
}

// --------------------------------------------------------- hew.library.list

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ListParams {
    #[serde(default)]
    category: Option<String>,
    #[serde(default)]
    query: Option<String>,
    #[serde(default)]
    collection: Option<String>,
}

fn list_items(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let p: ListParams = parse(params)?;
    let category = p.category.as_deref().map(parse_category).transpose()?;
    let query = p.query.as_deref().map(str::to_lowercase);
    let listing = ctx.host.library_list().map_err(CmdError::Refusal)?;
    let items: Vec<Value> = listing
        .items
        .iter()
        .filter(|e| category.is_none_or(|c| e.category == c))
        .filter(|e| {
            p.collection
                .as_deref()
                .is_none_or(|f| collection_matches_subtree(e.collection.as_deref(), f))
        })
        .filter(|e| query.as_deref().is_none_or(|q| matches_query(e, q)))
        .map(entry_to_json)
        .collect();
    Ok(serde_json::json!({ "folder": listing.folder, "items": items }))
}

// ----------------------------------------------------- hew.library.describe

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ItemParams {
    item: String,
}

fn describe_item(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let p: ItemParams = parse(params)?;
    let entry = resolve_entry(ctx, &p.item)?;
    let mut v = entry_to_json(&entry);
    let attrs = entry
        .summary
        .as_ref()
        .map(|s| s.doc_attrs.clone())
        .unwrap_or_else(|| serde_json::json!({}));
    v.as_object_mut()
        .expect("built as an object above")
        .insert("attrs".to_string(), attrs);
    Ok(v)
}

// ------------------------------------------------------- hew.library.remove

fn remove_item(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let p: ItemParams = parse(params)?;
    let entry = resolve_entry(ctx, &p.item)?;
    ctx.host
        .library_remove(&entry.path)
        .map_err(CmdError::Refusal)?;
    Ok(serde_json::json!({ "removed": entry.path }))
}

// -------------------------------------------------- hew.library.update_meta

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateMetaParams {
    item: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    keywords: Option<Vec<String>>,
    #[serde(default)]
    collection: Option<String>,
}

fn load_failed(e: impl std::fmt::Debug) -> CmdError {
    CmdError::Refusal(Refusal::api(
        "load_failed",
        &format!("this item's bytes are not a valid .hew document: {e:?}"),
    ))
}

fn update_meta(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let p: UpdateMetaParams = parse(params)?;
    let entry = resolve_entry(ctx, &p.item)?;
    let read = ctx
        .host
        .library_read(&entry.path)
        .map_err(CmdError::Refusal)?;
    let mut item_doc = Document::load(&read.bytes).map_err(load_failed)?;

    if let Some(name) = &p.name {
        item_doc.attr_set(
            AttrTarget::Document,
            "hew.library",
            "name",
            AttrValue::Text(name.clone()),
        )?;
    }
    if let Some(keywords) = &p.keywords {
        let list = AttrValue::List(keywords.iter().cloned().map(AttrValue::Text).collect());
        item_doc.attr_set(AttrTarget::Document, "hew.library", "keywords", list)?;
    }
    if let Some(collection) = &p.collection {
        item_doc.attr_set(
            AttrTarget::Document,
            "hew.library",
            "collection",
            AttrValue::Text(collection.clone()),
        )?;
    }

    let bytes = item_doc.save();
    ctx.host
        .library_write(LibraryWriteTarget::Existing { path: &entry.path }, &bytes)
        .map_err(CmdError::Refusal)?;

    let dict = item_doc
        .attr_get(&AttrTarget::Document)
        .ok()
        .flatten()
        .and_then(|d| d.get("hew.library"));
    let text = |key: &str| {
        dict.and_then(|d| d.get(key)).and_then(|v| match v {
            AttrValue::Text(s) => Some(s.clone()),
            _ => None,
        })
    };
    let keywords = dict
        .and_then(|d| d.get("keywords"))
        .and_then(|v| match v {
            AttrValue::List(items) => Some(
                items
                    .iter()
                    .filter_map(|i| match i {
                        AttrValue::Text(s) => Some(s.clone()),
                        _ => None,
                    })
                    .collect::<Vec<_>>(),
            ),
            _ => None,
        })
        .unwrap_or_else(|| entry.keywords.clone());

    let mut result = serde_json::json!({
        "name": text("name").unwrap_or(entry.name),
        "category": text("category").unwrap_or(entry.category),
        "keywords": keywords,
    });
    let obj = result.as_object_mut().expect("built as an object above");
    if let Some(id) = text("id").or(entry.id) {
        obj.insert("id".to_string(), Value::String(id));
    }
    if let Some(collection) = text("collection").or(entry.collection) {
        obj.insert("collection".to_string(), Value::String(collection));
    }
    if let Some(saved_at) = text("savedAt").or(entry.saved_at) {
        obj.insert("saved_at".to_string(), Value::String(saved_at));
    }
    Ok(result)
}

// ----------------------------------------------------- hew.library.insert

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct InsertParams {
    #[serde(default)]
    item: Option<String>,
    #[serde(default)]
    bytes_base64: Option<String>,
    #[serde(default)]
    content_hash: Option<String>,
    #[serde(default)]
    at: Option<Value>,
    #[serde(default)]
    name: Option<String>,
}

fn library_meta_text(item_doc: &Document, key: &str) -> Option<String> {
    item_doc
        .attr_get(&AttrTarget::Document)
        .ok()
        .flatten()
        .and_then(|d| d.get("hew.library"))
        .and_then(|d| d.get(key))
        .and_then(|v| match v {
            AttrValue::Text(s) => Some(s.clone()),
            _ => None,
        })
}

/// A material item goes into the palette, as the Library's "Add to palette"
/// does (`Document::insert_palette`): each of its materials is added, or
/// reused when the palette already holds an identical one. Nothing is
/// placed, and like `hew.material.create` it records no undo entry.
fn insert_material_item(ctx: &mut Ctx, item_doc: &Document) -> Result<Value, CmdError> {
    // The palette's own growth, so duplicates inside the item count once.
    let before = ctx.doc.materials().len();
    let ids = ctx.doc.insert_palette(item_doc);
    let added = ctx.doc.materials().len() - before;
    let resolver = ctx.resolver();
    let materials: Vec<String> = ids
        .iter()
        .map(|&m| {
            resolver
                .public_of(ctx.doc, &EntityRef::Material(m))
                .expect("in the palette")
        })
        .collect();
    Ok(serde_json::json!({
        "roots": [],
        "materials": materials,
        "definitions_added": 0,
        "definitions_reused": 0,
        "materials_added": added,
        "materials_reused": ids.len() - added,
        "objects_added": 0,
        "guides_added": 0,
        "world_sketches_skipped": 0,
        "annotations_skipped": 0,
    }))
}

fn insert_item(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let p: InsertParams = parse(params)?;
    if p.item.is_some() == p.bytes_base64.is_some() {
        return Err(CmdError::Params(
            "exactly one of item/bytes_base64 is required".into(),
        ));
    }

    let (bytes, content_hash) = if let Some(item) = &p.item {
        let entry = resolve_entry(ctx, item)?;
        let read = ctx
            .host
            .library_read(&entry.path)
            .map_err(CmdError::Refusal)?;
        (read.bytes, Some(read.content_hash))
    } else {
        let b64 = p.bytes_base64.as_deref().expect("checked above");
        let bytes = decode_base64(b64)
            .ok_or_else(|| CmdError::Params("bytes_base64 is not valid base64".into()))?;
        (bytes, p.content_hash.clone())
    };

    let item_doc = Document::load(&bytes).map_err(load_failed)?;
    if library_meta_text(&item_doc, "category").as_deref() == Some("material") {
        if p.at.is_some() || p.name.is_some() {
            return Err(CmdError::Params(
                "a material item goes into the palette; \"at\" and \"name\" place geometry".into(),
            ));
        }
        return insert_material_item(ctx, &item_doc);
    }
    let source_id = library_meta_text(&item_doc, "id");
    let provenance = match (source_id, content_hash) {
        (Some(source_id), Some(content_hash)) => Some(LibraryProvenance {
            source_id,
            content_hash,
        }),
        _ => None,
    };

    let pose = match &p.at {
        Some(v) => Transform::translation(crate::locate::resolve_point(ctx, v)?.to_vec()),
        None => Transform::IDENTITY,
    };

    let (report, _change) = ctx.doc.insert_document(
        &item_doc,
        &InsertOptions {
            pose,
            provenance,
            origin: None,
        },
    )?;

    if let Some(name) = p.name.as_deref().map(str::trim).filter(|n| !n.is_empty())
        && let [only] = report.roots.as_slice()
    {
        ctx.doc.set_node_name(*only, Some(name.to_string()))?;
    }

    let resolver = ctx.resolver();
    let roots: Vec<String> = report
        .roots
        .iter()
        .map(|&nid| {
            let entity = match nid {
                NodeId::Object(o) => EntityRef::Object(o),
                NodeId::Group(g) => EntityRef::Group(g),
                NodeId::Instance(i) => EntityRef::Instance(i),
            };
            resolver.public_of(ctx.doc, &entity).expect("just created")
        })
        .collect();

    Ok(serde_json::json!({
        "roots": roots,
        "definitions_added": report.definitions_added,
        "definitions_reused": report.definitions_reused,
        "materials_added": report.materials_added,
        "materials_reused": report.materials_reused,
        "objects_added": report.objects_added,
        "guides_added": report.guides_added,
        "world_sketches_skipped": report.world_sketches_skipped,
        "annotations_skipped": report.annotations_skipped,
    }))
}

// ------------------------------------------------------- hew.library.save

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SaveParams {
    #[serde(default)]
    selection: Option<Vec<String>>,
    /// One palette material, saved alone as a material item — the
    /// palette's "Save to Library".
    #[serde(default)]
    material: Option<String>,
    name: String,
    #[serde(default)]
    category: Option<String>,
    #[serde(default)]
    keywords: Option<Vec<String>>,
    #[serde(default)]
    collection: Option<String>,
    #[serde(default)]
    return_bytes: Option<bool>,
    /// Display-only "saved from" bookkeeping (`hew.library`'s `sourceDoc`
    /// key, docs/agents/HEW_API.md §8.1) — an explicit value always wins;
    /// omitted, it defaults to [`crate::host::Host::working_document_path`]
    /// (the file a `--file`-dispatched `hew-cli` opened this document
    /// from), and is left off the item entirely when neither is known (a
    /// fresh, never-saved document, or a host with no notion of "current
    /// file").
    #[serde(default)]
    source_doc: Option<String>,
}

fn save_item(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let p: SaveParams = parse(params)?;

    let selection = p.selection.filter(|s| !s.is_empty());
    if selection.is_some() && p.material.is_some() {
        return Err(CmdError::Params(
            "library.save takes \"selection\" or \"material\", not both".into(),
        ));
    }
    let (mut item_doc, default_category, source_nodes) = match &selection {
        None if p.material.is_some() => {
            let public = p.material.as_deref().expect("checked above");
            let Some(EntityRef::Material(mid)) = ctx.resolver().resolve(public) else {
                return Err(super::entity::unknown_entity(public));
            };
            let material = ctx
                .doc
                .material(mid)
                .ok_or_else(|| super::entity::unknown_entity(public))?
                .clone();
            // As the palette's Save to Library does: the material alone.
            let mut item = Document::new();
            item.add_material(material);
            (item, "material", Vec::new())
        }
        Some(ids) => {
            let nodes: Vec<NodeId> = ids
                .iter()
                .map(|id| resolve_node(ctx, id))
                .collect::<Result<_, _>>()?;
            let extracted = ctx.doc.extract_item(&nodes, true)?;
            (extracted, "component", nodes)
        }
        None => {
            let bytes = ctx.doc.save_for_persistence();
            let whole = Document::load(&bytes).map_err(load_failed)?;
            (whole, "model", Vec::new())
        }
    };

    let category = match &p.category {
        Some(c) => parse_category(c)?,
        None => default_category,
    };
    if default_category == "material" && category != "material" {
        return Err(CmdError::Params(
            "a saved \"material\" is a material item; leave \"category\" off or make it \"material\"".into(),
        ));
    }

    let id = mint_item_id(ctx.host);
    let mut set = |key: &str, value: AttrValue| -> Result<(), CmdError> {
        item_doc
            .attr_set(AttrTarget::Document, "hew.library", key, value)
            .map_err(CmdError::from)
            .map(|_| ())
    };
    set("id", AttrValue::Text(id.clone()))?;
    set("name", AttrValue::Text(p.name.clone()))?;
    set("category", AttrValue::Text(category.to_string()))?;
    if let Some(keywords) = &p.keywords {
        set(
            "keywords",
            AttrValue::List(keywords.iter().cloned().map(AttrValue::Text).collect()),
        )?;
    }
    if let Some(collection) = &p.collection {
        set("collection", AttrValue::Text(collection.clone()))?;
    }
    // An explicit `source_doc` always wins; absent, fall back to the
    // host's own notion of "the file this document came from" (`--file`
    // dispatch's opened path) — display-only, so a host with neither (a
    // fresh, never-saved document; a host with no filesystem) simply
    // leaves the key off rather than refusing.
    let source_doc = p
        .source_doc
        .clone()
        .or_else(|| ctx.host.working_document_path().map(str::to_string));
    if let Some(source_doc) = source_doc {
        set("sourceDoc", AttrValue::Text(source_doc))?;
    }
    set("savedAt", AttrValue::Text(iso_now(ctx.host.unix_time_ms())))?;

    let bytes = item_doc.save();
    let write = ctx
        .host
        .library_write(
            LibraryWriteTarget::New {
                category,
                name: &p.name,
                id: &id,
            },
            &bytes,
        )
        .map_err(CmdError::Refusal)?;

    if !source_nodes.is_empty() {
        // The definition's stable id (`def_sid`), for an instance
        // selection wrapped as a component — mirrors
        // `Scene::stamp_library_source`'s use of
        // `summary.first_component_sid`, read the same way: round-trip
        // through the saved bytes rather than walking `item_doc`'s
        // in-memory ids (sids are only guaranteed stable post-save).
        let summary = kernel::read_item_summary(&bytes).map_err(|e| {
            CmdError::Internal(format!("just-saved item failed to re-parse: {e:?}"))
        })?;
        let def_sid = summary
            .first_component_sid
            .as_deref()
            .and_then(|s| s.parse::<u64>().ok());
        let provenance = LibraryProvenance {
            source_id: id.clone(),
            content_hash: write.content_hash.clone(),
        };
        ctx.doc
            .stamp_library_source(&source_nodes, &provenance, def_sid);
    }

    let mut result = serde_json::json!({ "path": write.path, "id": id });
    if p.return_bytes.unwrap_or(false) {
        result
            .as_object_mut()
            .expect("built as an object above")
            .insert(
                "bytes_base64".to_string(),
                Value::String(encode_base64(&bytes)),
            );
    }
    Ok(result)
}

// ---------------------------------------------------------------- helpers

/// Mints a fresh library item id: 128 bits from the host's
/// [`Host::random_u64`] — good enough uniqueness for a library item's own
/// identity (never used as a security token, and a collision would only
/// cost a re-save), without pulling a `uuid`/rand crate into this pure
/// crate (`crates/api` stays std + kernel + serde — see `host.rs`'s module
/// doc). The host supplies the randomness because the live app's WASM
/// kernel has no OS entropy, clock, or pid to read. Formatted as an RFC
/// 4122-shaped string (version 4, variant bits set) purely for
/// familiarity — nothing here or in `crates/library` parses that shape
/// back out.
fn mint_item_id(host: &dyn crate::host::Host) -> String {
    let a = host.random_u64();
    let b = host.random_u64() ^ 0x9E37_79B9_7F4A_7C15;

    let mut bytes = [0u8; 16];
    bytes[..8].copy_from_slice(&a.to_be_bytes());
    bytes[8..].copy_from_slice(&b.to_be_bytes());
    bytes[6] = (bytes[6] & 0x0F) | 0x40; // version 4
    bytes[8] = (bytes[8] & 0x3F) | 0x80; // RFC 4122 variant

    let hex: Vec<String> = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        hex[0..4].concat(),
        hex[4..6].concat(),
        hex[6..8].concat(),
        hex[8..10].concat(),
        hex[10..16].concat()
    )
}

/// The current UTC instant as an ISO-8601 string
/// (`YYYY-MM-DDTHH:MM:SS.mmmZ`) — `hew.library` metadata's `savedAt`
/// convention (`App.tsx`'s `new Date().toISOString()`). No `chrono`
/// dependency (this crate stays std + kernel + serde): a Gregorian
/// civil-date conversion from a Unix-epoch day count is ~15 lines
/// (Howard Hinnant's `civil_from_days`, public domain), well-trodden
/// enough not to need a crate for it.
fn iso_now(unix_ms: u64) -> String {
    let total_ms = u128::from(unix_ms);
    let secs = (total_ms / 1000) as i64;
    let ms = (total_ms % 1000) as u32;
    let days = secs.div_euclid(86_400);
    let sod = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    let hh = sod / 3600;
    let mm = (sod % 3600) / 60;
    let ss = sod % 60;
    format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}.{ms:03}Z")
}

/// Days-since-Unix-epoch to a proleptic Gregorian (year, month, day) —
/// Howard Hinnant's `civil_from_days` algorithm.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mint_item_id_looks_like_a_uuid_and_is_never_repeated() {
        let a = mint_item_id(&crate::host::NoHost);
        let b = mint_item_id(&crate::host::NoHost);
        assert_ne!(a, b);
        for id in [&a, &b] {
            let parts: Vec<&str> = id.split('-').collect();
            assert_eq!(
                parts.iter().map(|p| p.len()).collect::<Vec<_>>(),
                vec![8, 4, 4, 4, 12],
                "{id}"
            );
        }
    }

    #[test]
    fn iso_now_looks_like_iso_8601_and_round_trips_a_known_day() {
        // 2024-01-01T00:00:00.000Z is exactly 19723 days after the epoch —
        // a fixed point to pin `civil_from_days` against, independent of
        // whatever moment the test actually runs.
        assert_eq!(civil_from_days(19_723), (2024, 1, 1));
        assert_eq!(civil_from_days(0), (1970, 1, 1));

        let now = iso_now(1_700_000_000_123);
        assert_eq!(now.len(), 24, "{now}");
        assert!(now.ends_with('Z'), "{now}");
        assert_eq!(now.as_bytes()[4], b'-');
        assert_eq!(now.as_bytes()[10], b'T');
    }

    #[test]
    fn collection_matches_subtree_matches_segments_not_prefixes() {
        assert!(collection_matches_subtree(Some("Hardware"), "Hardware"));
        assert!(collection_matches_subtree(
            Some("Hardware/Fasteners"),
            "Hardware"
        ));
        assert!(!collection_matches_subtree(Some("HardwareX"), "Hardware"));
        assert!(!collection_matches_subtree(
            Some("Hardware"),
            "Hardware/Fasteners"
        ));
        assert!(!collection_matches_subtree(None, "Hardware"));
    }
}

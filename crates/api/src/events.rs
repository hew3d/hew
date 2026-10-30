//! Server-originated notifications (docs/agents/HEW_API.md §4.5): the
//! closed event inventory and the frame builders a host emits through.
//!
//! A notification shares the JSON-RPC envelope with no `id`:
//! `{"jsonrpc": "2.0", "method": "hew.event.<name>", "params": {...}}`.
//! Which connection receives which frame is
//! [`crate::Connection::wants`]; the builders here only shape params, from
//! a `&kernel::Document` and what the host knows about it.

use kernel::{AttrScope, AttrTarget, ChangeKind, ChangeRecord, ChangeSet, HistoryOrigin};
use serde_json::{Value, json};

use crate::ids;

/// Every event a connection may subscribe to. An unknown name in
/// `hew.event.subscribe` is `-32602`.
pub const EVENT_NAMES: &[&str] = &[
    "document_changed",
    "selection_changed",
    "context_changed",
    "document_will_save",
    "document_saved",
    "document_opened",
    "document_closed",
    "units_changed",
    "scenes_changed",
    "tool_changed",
    "plugin_state",
    "slot_changed",
    "settings_changed",
    "ui_cancel",
    "window_closed",
    "surface_closed",
];

/// The document lifecycle events: delivered for the connection's own
/// document only, without `documents.other`, and with `path` only under
/// `documents.save`.
pub const LIFECYCLE_EVENTS: &[&str] = &[
    "document_will_save",
    "document_saved",
    "document_opened",
    "document_closed",
];

/// How many `document_changed` records a host sends one by one between
/// two drains; more collapse into one `full` frame.
pub const MAX_RECORDS_PER_DRAIN: usize = 64;

/// The notification frame for `hew.event.<method_suffix>`, serialized.
pub fn notification(method_suffix: &str, params: Value) -> String {
    json!({
        "jsonrpc": "2.0",
        "method": format!("hew.event.{method_suffix}"),
        "params": params,
    })
    .to_string()
}

fn origin_json(origin: &HistoryOrigin) -> Value {
    match origin {
        HistoryOrigin::User => json!("user"),
        HistoryOrigin::Connection(id) => json!({ "connection": id }),
    }
}

fn kind_name(kind: ChangeKind) -> &'static str {
    match kind {
        ChangeKind::Commit => "commit",
        ChangeKind::Undo => "undo",
        ChangeKind::Redo => "redo",
        ChangeKind::Abort => "abort",
        ChangeKind::Reset => "reset",
    }
}

/// A change set on the wire: public ids in each list, attribute entries
/// as `{id, ns, scope}` (`id` is `"document"` for the document's own
/// dictionary, else the entity's public id, resolved through `doc`; a
/// face or edge dictionary is named by its object's id with `scope`
/// `"face"` or `"edge"`, every other entry has `scope` `"entity"`).
pub fn change_set_json(doc: &kernel::Document, changes: &ChangeSet) -> Value {
    let ids = |list: &[kernel::ChangeRef]| -> Vec<String> {
        list.iter()
            .map(|r| ids::public_id(&r.entity, r.sid))
            .collect()
    };
    let attrs: Vec<Value> = changes
        .attrs
        .iter()
        .filter_map(|change| {
            let ns = &change.ns;
            let scope = match change.scope {
                AttrScope::Entity => "entity",
                AttrScope::Face => "face",
                AttrScope::Edge => "edge",
            };
            // A face or edge dictionary is named by its Object, by the
            // stable id recorded with the change.
            if let Some(object) = &change.object {
                let id = ids::public_id(&object.entity, object.sid);
                return Some(json!({ "id": id, "ns": ns, "scope": scope }));
            }
            match change.target.as_ref()? {
                AttrTarget::Document => Some(json!({ "id": "document", "ns": ns, "scope": scope })),
                // Stable ids outlive their entities, so a written target
                // always resolves.
                AttrTarget::Entity(e) => doc
                    .sid_of(e)
                    .map(|sid| json!({ "id": ids::public_id(e, sid), "ns": ns, "scope": scope })),
            }
        })
        .collect();
    json!({
        "added": ids(&changes.added),
        "removed": ids(&changes.removed),
        "modified": ids(&changes.modified),
        "attrs": attrs,
        "document": changes.document,
        "full": changes.full,
    })
}

/// The JSON a change set is answered with when the history no longer
/// covers the range asked about.
pub fn full_change_set_json() -> Value {
    json!({
        "added": [],
        "removed": [],
        "modified": [],
        "attrs": [],
        "document": false,
        "full": true,
    })
}

/// `hew.query.changes_since`'s `changes` for `generation`.
pub fn changes_since_json(doc: &kernel::Document, generation: u64) -> Value {
    match doc.changes_since_generation(generation) {
        Some(changes) => change_set_json(doc, &changes),
        None => full_change_set_json(),
    }
}

/// `document_changed` params for one record. An undo's lists arrive
/// swapped: `added` names what the undo brought back, `removed` what it
/// took away.
pub fn document_changed(doc: &kernel::Document, document: &str, record: &ChangeRecord) -> Value {
    let changes = if record.kind == ChangeKind::Undo {
        record.changes.swapped()
    } else {
        record.changes.clone()
    };
    json!({
        "document": document,
        "generation": record.generation,
        "epoch": record.epoch,
        "origin": origin_json(&record.origin),
        "label": record.label,
        "kind": kind_name(record.kind),
        "bookkeeping": record.bookkeeping,
        "changes": change_set_json(doc, &changes),
    })
}

/// `context_changed` params: `hew.query.context`'s `stack`.
pub fn context_changed(doc: &kernel::Document, document: &str) -> Value {
    let stack = crate::commands::query::context_stack_json(doc).unwrap_or_else(|_| json!([]));
    json!({ "document": document, "stack": stack })
}

/// `scenes_changed` params: `hew.scenes.list`'s entries and the Scene
/// last activated (its public id), if any.
pub fn scenes_changed(doc: &kernel::Document, document: &str, active: Option<u64>) -> Value {
    json!({
        "document": document,
        "scenes": crate::commands::scenes::scene_entries_json(doc),
        "active": active.map(ids::scene_id),
    })
}

/// `units_changed` params: the display length format.
pub fn units_changed(document: &str, units: &str) -> Value {
    json!({ "document": document, "units": units })
}

/// `selection_changed` params: `{ document, ids, primary, generation,
/// epoch }`, the live entries only, by public id.
pub fn selection_changed(
    doc: &kernel::Document,
    document: &str,
    entries: &[crate::host::SelectionEntry],
) -> Value {
    let mut params = crate::selection::selection_json(doc, entries);
    params["document"] = json!(document);
    params
}

/// `tool_changed` params: `{ tool }`, the active tool's name.
pub fn tool_changed(tool: &str) -> Value {
    json!({ "tool": tool })
}

/// `document_will_save`, `document_saved`, `document_opened`, and
/// `document_closed` params. `path` and `format` are left out when
/// absent; the emitter strips `path` for a connection without
/// `documents.save`. `saved` and `opened` also carry `document_id`, the
/// document's persistent UUID or `null` when it has none.
pub fn document_lifecycle(
    kind: &str,
    document: &str,
    title: &str,
    path: Option<&str>,
    format: Option<&str>,
    document_id: Option<&str>,
) -> Value {
    let mut out = json!({ "document": document, "title": title });
    if matches!(kind, "saved" | "opened") {
        out["document_id"] = json!(document_id);
    }
    if let Some(path) = path {
        out["path"] = json!(path);
    }
    if let Some(format) = format {
        out["format"] = json!(format);
    }
    out
}

/// The record a host emits instead of more than [`MAX_RECORDS_PER_DRAIN`]
/// records, or when the ring no longer reaches back to its last drain:
/// one commit with `full`, at the document's current clock.
pub fn collapsed_record(doc: &kernel::Document) -> ChangeRecord {
    ChangeRecord::collapsed(doc.history_generation(), doc.document_epoch())
}

#[cfg(test)]
mod tests {
    use super::*;
    use kernel::{AttrValue, Document, EntityRef, Plane, Point3};

    fn ground() -> Plane {
        Plane::from_polygon(&[
            Point3::ORIGIN,
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(0.0, 1.0, 0.0),
        ])
        .unwrap()
    }

    fn rect(doc: &mut Document) -> kernel::SketchId {
        let s = doc.add_sketch(ground());
        doc.begin_sketch_gesture(s).unwrap();
        let sk = doc.sketch_mut(s).unwrap();
        let c = [
            Point3::ORIGIN,
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(1.0, 1.0, 0.0),
            Point3::new(0.0, 1.0, 0.0),
        ];
        for i in 0..4 {
            sk.add_segment(c[i], c[(i + 1) % 4]).unwrap();
        }
        doc.end_sketch_gesture(s).unwrap();
        s
    }

    fn last(doc: &Document) -> ChangeRecord {
        doc.change_records_since_epoch(0)
            .unwrap()
            .last()
            .unwrap()
            .clone()
    }

    #[test]
    fn notification_is_an_id_less_frame() {
        let frame: Value =
            serde_json::from_str(&notification("units_changed", json!({ "a": 1 }))).unwrap();
        assert_eq!(
            frame,
            json!({ "jsonrpc": "2.0", "method": "hew.event.units_changed", "params": { "a": 1 } })
        );
        assert!(frame.get("id").is_none());
    }

    #[test]
    fn document_changed_names_public_ids_and_swaps_undo() {
        let mut doc = Document::new();
        let s = rect(&mut doc);
        let id = ids::public_id(
            &EntityRef::Sketch(s),
            doc.sid_of(&EntityRef::Sketch(s)).unwrap(),
        );
        let commit = document_changed(&doc, "doc_1", &last(&doc));
        assert_eq!(commit["document"], "doc_1");
        assert_eq!(commit["kind"], "commit");
        assert_eq!(commit["origin"], "user");
        assert_eq!(commit["bookkeeping"], false);
        assert_eq!(commit["generation"], doc.history_generation());
        assert_eq!(commit["epoch"], doc.document_epoch());
        assert_eq!(commit["changes"]["added"], json!([id]));
        assert_eq!(commit["changes"]["removed"], json!([]));
        doc.undo().unwrap();
        let undo = document_changed(&doc, "doc_1", &last(&doc));
        assert_eq!(undo["kind"], "undo");
        assert_eq!(undo["changes"]["added"], json!([]));
        assert_eq!(undo["changes"]["removed"], json!([id]));
    }

    #[test]
    fn change_set_json_shapes_attrs_and_flags() {
        let mut doc = Document::new();
        doc.attr_set(
            kernel::AttrTarget::Document,
            "com.example.t",
            "k",
            AttrValue::Int(1),
        )
        .unwrap();
        let s = rect(&mut doc);
        doc.attr_set(
            kernel::AttrTarget::Entity(EntityRef::Sketch(s)),
            "com.example.t",
            "k",
            AttrValue::Int(1),
        )
        .unwrap();
        let since = changes_since_json(&doc, 0);
        let sketch = ids::public_id(
            &EntityRef::Sketch(s),
            doc.sid_of(&EntityRef::Sketch(s)).unwrap(),
        );
        assert_eq!(
            since["attrs"],
            json!([
                { "id": "document", "ns": "com.example.t", "scope": "entity" },
                { "id": sketch, "ns": "com.example.t", "scope": "entity" }
            ])
        );
        assert_eq!(since["document"], false);
        assert_eq!(since["full"], false);
        assert_eq!(
            full_change_set_json(),
            json!({ "added": [], "removed": [], "modified": [], "attrs": [], "document": false, "full": true })
        );
        // A generation the ring does not know answers full.
        assert_eq!(changes_since_json(&doc, 999), full_change_set_json());
    }

    #[test]
    fn context_and_scenes_reuse_the_query_shapes() {
        let mut doc = Document::new();
        assert_eq!(
            context_changed(&doc, "doc_1"),
            json!({ "document": "doc_1", "stack": [] })
        );
        let sid = doc
            .add_scene(
                Some("Front".into()),
                kernel::SceneProps::default(),
                None,
                None,
                None,
            )
            .unwrap();
        let scenes = scenes_changed(&doc, "doc_1", Some(sid));
        assert_eq!(scenes["scenes"].as_array().unwrap().len(), 1);
        assert_eq!(scenes["scenes"][0]["name"], "Front");
        assert_eq!(scenes["active"], ids::scene_id(sid));
        assert_eq!(scenes_changed(&doc, "doc_1", None)["active"], Value::Null);
    }

    #[test]
    fn units_and_lifecycle_params() {
        assert_eq!(
            units_changed("doc_1", "mm"),
            json!({ "document": "doc_1", "units": "mm" })
        );
        let id = "3f2c9107-5ed4-40a8-806b-1ec297405d83";
        assert_eq!(
            document_lifecycle(
                "saved",
                "doc_1",
                "Box",
                Some("/a/b.hew"),
                Some("hew"),
                Some(id)
            ),
            json!({ "document": "doc_1", "title": "Box", "path": "/a/b.hew", "format": "hew", "document_id": id })
        );
        assert_eq!(
            document_lifecycle("opened", "doc_1", "Box", None, None, None),
            json!({ "document": "doc_1", "title": "Box", "document_id": null })
        );
        // Only `saved` and `opened` carry the id.
        assert_eq!(
            document_lifecycle("will_save", "doc_1", "Box", None, None, Some(id)),
            json!({ "document": "doc_1", "title": "Box" })
        );
        assert_eq!(
            document_lifecycle("closed", "doc_1", "Box", None, None, Some(id)),
            json!({ "document": "doc_1", "title": "Box" })
        );
    }

    #[test]
    fn event_names_are_closed_and_gated() {
        assert_eq!(EVENT_NAMES.len(), 16);
        let read = crate::Grant::plugin(
            "com.example.sub",
            Some("doc_1"),
            [crate::Permission::ModelRead].into_iter().collect(),
        );
        for name in EVENT_NAMES {
            // Every name in the inventory is one the grant table knows.
            assert!(crate::Grant::core().may_receive(name), "{name}");
        }
        assert!(read.may_receive("document_changed"));
        assert!(!read.may_receive("selection_changed"));
    }
}

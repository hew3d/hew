// Test-only: the sets below are order-independent equality assertions on id
// collections, not kernel output, so HashSet is fine here. Suppress the
// workspace clippy.toml ban for this integration-test crate.
#![allow(clippy::disallowed_types)]

//! Acceptance specs for the library copy engine (DEVELOPMENT.md rule 3):
//! [`Document::extract_item`] (save a selection to a standalone item
//! document) and [`Document::insert_document`] (graft an item document into
//! a live one, losslessly, as one undo step, with provenance-based
//! idempotent re-insert and content-deduplicated materials).

use kernel::{
    CurveGeom, Document, DocumentError, InsertOptions, InsertReport, LibraryProvenance, Material,
    NodeId, Plane, Point3, Rgba8, Transform, TransformError, Vec3, WatertightState,
};
use proptest::prelude::*;
use std::collections::HashSet;

// ----------------------------------------------------------------- helpers

/// The ground (z = 0) plane.
fn ground() -> Plane {
    Plane::from_polygon(&[
        Point3::new(0.0, 0.0, 0.0),
        Point3::new(1.0, 0.0, 0.0),
        Point3::new(0.0, 1.0, 0.0),
    ])
    .expect("ground plane is well-defined")
}

/// The single region of a sketch (panics if there isn't exactly one).
fn only_region(doc: &Document, s: kernel::SketchId) -> kernel::SketchRegionId {
    let regions = doc.extrudable_regions(s).expect("sketch is live");
    assert_eq!(regions.len(), 1, "expected exactly one extrudable region");
    regions[0]
}

/// Extrude an axis-aligned box on the ground with the given footprint/height.
fn extrude_box(doc: &mut Document, x0: f64, y0: f64, x1: f64, y1: f64, h: f64) -> kernel::ObjectId {
    let s = doc.add_sketch(ground());
    let corners = [
        (Point3::new(x0, y0, 0.0), Point3::new(x1, y0, 0.0)),
        (Point3::new(x1, y0, 0.0), Point3::new(x1, y1, 0.0)),
        (Point3::new(x1, y1, 0.0), Point3::new(x0, y1, 0.0)),
        (Point3::new(x0, y1, 0.0), Point3::new(x0, y0, 0.0)),
    ];
    let sk = doc.sketch_mut(s).expect("sketch is live");
    for (a, b) in corners {
        sk.add_segment(a, b).expect("rectangle segment");
    }
    let r = only_region(doc, s);
    doc.extrude_region(s, r, h).expect("extrude box").0
}

/// Extrude a true-circle cylinder (analytic curve chain) on the ground.
fn extrude_cylinder(doc: &mut Document, center: Point3, radius: f64, h: f64) -> kernel::ObjectId {
    let s = doc.add_sketch(ground());
    let sk = doc.sketch_mut(s).expect("sketch is live");
    sk.begin_curve_with(CurveGeom { center, radius }).unwrap();
    let n = 24usize;
    let p = |i: usize| {
        let a = 2.0 * std::f64::consts::PI * (i as f64) / (n as f64);
        Point3::new(
            center.x + radius * a.cos(),
            center.y + radius * a.sin(),
            0.0,
        )
    };
    for i in 0..n {
        sk.add_segment(p(i), p(i + 1)).unwrap();
    }
    sk.end_curve();
    let r = only_region(doc, s);
    doc.extrude_region(s, r, h).expect("extrude cylinder").0
}

/// Axis-aligned bounding box over an object's vertices.
fn object_bbox(doc: &Document, id: kernel::ObjectId) -> (Point3, Point3) {
    let obj = doc.object(id).expect("object is live");
    let mut lo = Point3::new(f64::INFINITY, f64::INFINITY, f64::INFINITY);
    let mut hi = Point3::new(f64::NEG_INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    for v in obj.vertices().values() {
        let p = v.position;
        lo = Point3::new(lo.x.min(p.x), lo.y.min(p.y), lo.z.min(p.z));
        hi = Point3::new(hi.x.max(p.x), hi.y.max(p.y), hi.z.max(p.z));
    }
    (lo, hi)
}

fn provenance(id: &str, hash: &str) -> Option<LibraryProvenance> {
    Some(LibraryProvenance {
        source_id: id.to_string(),
        content_hash: hash.to_string(),
    })
}

fn insert_at_identity(prov: Option<LibraryProvenance>) -> InsertOptions {
    InsertOptions {
        pose: Transform::IDENTITY,
        provenance: prov,
    }
}

// ----------------------------------------------------------- extraction

/// A single bare object, wrapped: the item is a component-item file —
/// exactly one definition (named after the object) plus one identity-posed
/// instance carrying the object's tags.
#[test]
fn extract_wrapped_object_is_definition_plus_identity_instance() {
    let mut doc = Document::new();
    let oid = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    doc.set_node_name(NodeId::Object(oid), Some("Chair".into()))
        .unwrap();

    let item = doc.extract_item(&[NodeId::Object(oid)], true).unwrap();

    assert_eq!(item.component_ids().len(), 1);
    assert_eq!(item.instance_ids().len(), 1);
    assert!(
        item.visible_object_ids().is_empty(),
        "the solid lives in the definition, not the world tree"
    );
    let cid = item.component_ids()[0];
    assert_eq!(item.component_name(cid), Some("Chair"));
    // The item re-origins to the selection's bbox BOTTOM CENTER (axes at
    // identity): the unit box saved at (0..1)² carries a pose that puts
    // its bottom center on the item origin.
    let iid = item.instance_ids()[0];
    let pose = item.instance_pose(iid).unwrap().to_affine();
    assert_eq!((pose[3], pose[7], pose[11]), (-0.5, -0.5, 0.0));
}

/// Extraction is read-only: the source document's saved bytes are untouched.
#[test]
fn extract_leaves_the_source_document_untouched() {
    let mut doc = Document::new();
    let oid = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let before = doc.state_hash();

    let _item = doc.extract_item(&[NodeId::Object(oid)], true).unwrap();

    assert_eq!(doc.state_hash(), before);
}

/// The item is re-expressed in the drawing-axes frame: with axes moved to
/// (2, 3, 0), a unit box at the world origin lands at (−2, −3, 0) in the
/// item ("insertion point = current axes origin").
#[test]
fn extract_reexpresses_geometry_in_the_axes_frame() {
    let mut doc = Document::new();
    let oid = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    doc.set_axes(
        Point3::new(2.0, 3.0, 0.0),
        Vec3::new(1.0, 0.0, 0.0),
        Vec3::new(0.0, 1.0, 0.0),
    )
    .unwrap();

    let item = doc.extract_item(&[NodeId::Object(oid)], true).unwrap();

    let cid = item.component_ids()[0];
    let member = item.def_members(cid).unwrap()[0];
    let (lo, hi) = object_bbox(&item, member);
    assert!(lo.approx_eq(Point3::new(-2.0, -3.0, 0.0), 1e-12));
    assert!(hi.approx_eq(Point3::new(-1.0, -2.0, 1.0), 1e-12));
}

/// Extracting an instance carries its definition and conjugates the pose
/// into the item frame; the definition's geometry copies verbatim.
#[test]
fn extract_instance_carries_definition_and_pose() {
    let mut doc = Document::new();
    let oid = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let (cid, iid, _) = doc.make_component(&[NodeId::Object(oid)]).unwrap();
    let shift = Transform::translation(Vec3::new(5.0, 0.0, 0.0));
    doc.transform_instance(iid, &shift).unwrap();

    let item = doc.extract_item(&[NodeId::Instance(iid)], false).unwrap();

    assert_eq!(item.component_ids().len(), 1);
    assert_eq!(item.instance_ids().len(), 1);
    let item_cid = item.component_ids()[0];
    assert_eq!(
        item.def_members(item_cid).unwrap().len(),
        doc.def_members(cid).unwrap().len()
    );
    // Re-origined to the placement's bbox bottom center: the world pose's
    // (5, 0, 0) translation becomes (-0.5, -0.5, 0) in item coordinates
    // (unit box centered on the origin's XY, sitting on Z=0).
    let item_pose = item
        .instance_pose(item.instance_ids()[0])
        .unwrap()
        .to_affine();
    assert_eq!(
        (item_pose[3], item_pose[7], item_pose[11]),
        (-0.5, -0.5, 0.0)
    );
}

/// Extracting a group copies its structure and slices the palette down to
/// the materials the copied content actually references.
#[test]
fn extract_group_copies_structure_and_slices_materials() {
    let mut doc = Document::new();
    let used = doc.add_material(Material {
        name: "Oak".into(),
        color: Rgba8::rgb(180, 140, 90),
        texture: None,
    });
    let _unused = doc.add_material(Material {
        name: "Unused".into(),
        color: Rgba8::rgb(1, 2, 3),
        texture: None,
    });
    let a = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let b = extrude_box(&mut doc, 2.0, 0.0, 3.0, 1.0, 1.0);
    doc.set_object_material(a, Some(used)).unwrap();
    let (gid, _) = doc
        .group_nodes(&[NodeId::Object(a), NodeId::Object(b)])
        .unwrap();
    doc.set_node_name(NodeId::Group(gid), Some("Table".into()))
        .unwrap();

    let item = doc.extract_item(&[NodeId::Group(gid)], true).unwrap();

    // wrap flag is ignored for groups: the item is a plain group copy.
    assert!(item.component_ids().is_empty());
    assert_eq!(item.visible_object_ids().len(), 2);
    assert_eq!(item.material_ids().len(), 1, "only the referenced material");
    let mid = item.material_ids()[0];
    assert_eq!(item.material(mid).unwrap().name, "Oak");
}

/// Refusals: empty and duplicate selections, stale nodes — all typed, all
/// before anything is built.
#[test]
fn extract_refusals_are_typed() {
    let mut doc = Document::new();
    let oid = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);

    assert!(matches!(
        doc.extract_item(&[], true),
        Err(DocumentError::EmptyComponent)
    ));
    assert!(matches!(
        doc.extract_item(&[NodeId::Object(oid), NodeId::Object(oid)], false),
        Err(DocumentError::DuplicateMember)
    ));
    let (_, iid, _) = doc.make_component(&[NodeId::Object(oid)]).unwrap();
    // `oid` is now a definition member, not a world solid.
    assert!(matches!(
        doc.extract_item(&[NodeId::Object(oid)], true),
        Err(DocumentError::UnknownObject)
    ));
    let _ = iid;
}

// ------------------------------------------------------------- insertion

/// The round trip: extract → save → load → insert into a fresh document.
/// One watertight solid arrives as an instance of one definition; the
/// report accounts for everything.
#[test]
fn insert_round_trip_lands_one_instance() {
    let mut src = Document::new();
    let oid = extrude_box(&mut src, 0.0, 0.0, 1.0, 1.0, 1.0);
    src.set_node_name(NodeId::Object(oid), Some("Chair".into()))
        .unwrap();
    let item = src.extract_item(&[NodeId::Object(oid)], true).unwrap();
    let item = Document::load(&item.save()).expect("item file round-trips");

    let mut dst = Document::new();
    let (report, change) = dst
        .insert_document(&item, &insert_at_identity(provenance("lib-1", "hash-1")))
        .unwrap();

    assert_eq!(report.roots.len(), 1);
    assert!(matches!(report.roots[0], NodeId::Instance(_)));
    assert_eq!(report.definitions_added, 1);
    assert_eq!(report.definitions_reused, 0);
    assert_eq!(report.objects_added, 1);
    assert_eq!(dst.instance_ids().len(), 1);
    assert_eq!(dst.component_ids().len(), 1);
    assert_eq!(dst.component_name(dst.component_ids()[0]), Some("Chair"));
    let member = dst.def_members(dst.component_ids()[0]).unwrap()[0];
    assert_eq!(
        dst.object(member).unwrap().watertight(),
        WatertightState::Watertight
    );
    assert!(!change.components_touched.is_empty());
}

/// Losslessness — the reason insert does NOT go through `ImportScene`: a
/// true-circle cylinder's analytic wall surfaces survive extract → save →
/// load → insert.
#[test]
fn insert_preserves_analytic_surfaces() {
    let mut src = Document::new();
    let oid = extrude_cylinder(&mut src, Point3::new(1.0, 2.0, 0.0), 0.5, 1.0);
    let count_surfaces = |doc: &Document, id: kernel::ObjectId| {
        doc.object(id)
            .unwrap()
            .faces()
            .values()
            .filter(|f| f.surface.is_some())
            .count()
    };
    let src_surfaces = count_surfaces(&src, oid);
    assert!(src_surfaces > 0, "cylinder walls carry their surface");

    let item = src.extract_item(&[NodeId::Object(oid)], true).unwrap();
    let item = Document::load(&item.save()).expect("item file round-trips");
    let mut dst = Document::new();
    dst.insert_document(&item, &insert_at_identity(None))
        .unwrap();

    let member = dst.def_members(dst.component_ids()[0]).unwrap()[0];
    assert_eq!(count_surfaces(&dst, member), src_surfaces);
}

/// The idempotent re-insert: same provenance twice → the second insert
/// REUSES the definition (one definition, two instances, no new objects).
#[test]
fn insert_same_provenance_twice_reuses_the_definition() {
    let mut src = Document::new();
    let oid = extrude_box(&mut src, 0.0, 0.0, 1.0, 1.0, 1.0);
    let item = src.extract_item(&[NodeId::Object(oid)], true).unwrap();

    let mut dst = Document::new();
    let prov = provenance("lib-1", "hash-1");
    dst.insert_document(&item, &insert_at_identity(prov.clone()))
        .unwrap();
    let (second, _) = dst
        .insert_document(
            &item,
            &InsertOptions {
                pose: Transform::translation(Vec3::new(3.0, 0.0, 0.0)),
                provenance: prov,
            },
        )
        .unwrap();

    assert_eq!(second.definitions_reused, 1);
    assert_eq!(second.definitions_added, 0);
    assert_eq!(second.objects_added, 0);
    assert_eq!(dst.component_ids().len(), 1, "one shared definition");
    assert_eq!(dst.instance_ids().len(), 2, "two placements");
    // The second placement sits at the requested pose.
    let poses: Vec<[f64; 12]> = dst
        .instance_ids()
        .iter()
        .map(|&i| dst.instance_pose(i).unwrap().to_affine())
        .collect();
    // Item pose carries the bottom-center re-origin (-0.5, -0.5, 0); the
    // second insert's (3, 0, 0) composes on top.
    assert!(
        poses
            .iter()
            .any(|p| p[3] == 2.5 && p[7] == -0.5 && p[11] == 0.0)
    );
}

/// An EDITED item (same source id, new content hash) does NOT reuse the
/// stale definition.
#[test]
fn insert_edited_item_gets_a_fresh_definition() {
    let mut src = Document::new();
    let oid = extrude_box(&mut src, 0.0, 0.0, 1.0, 1.0, 1.0);
    let item = src.extract_item(&[NodeId::Object(oid)], true).unwrap();

    let mut dst = Document::new();
    dst.insert_document(&item, &insert_at_identity(provenance("lib-1", "hash-1")))
        .unwrap();
    let (second, _) = dst
        .insert_document(&item, &insert_at_identity(provenance("lib-1", "hash-2")))
        .unwrap();

    assert_eq!(second.definitions_reused, 0);
    assert_eq!(second.definitions_added, 1);
    assert_eq!(dst.component_ids().len(), 2);
}

/// Without provenance there is no reuse: two inserts, two definitions.
#[test]
fn insert_without_provenance_never_reuses() {
    let mut src = Document::new();
    let oid = extrude_box(&mut src, 0.0, 0.0, 1.0, 1.0, 1.0);
    let item = src.extract_item(&[NodeId::Object(oid)], true).unwrap();

    let mut dst = Document::new();
    dst.insert_document(&item, &insert_at_identity(None))
        .unwrap();
    dst.insert_document(&item, &insert_at_identity(None))
        .unwrap();
    assert_eq!(dst.component_ids().len(), 2);
}

/// Materials dedupe by content: inserting an item whose material is
/// content-equal to a palette entry reuses it; a second insert of the same
/// item adds nothing to the palette either.
#[test]
fn insert_dedupes_materials_by_content() {
    let mut src = Document::new();
    let oak = src.add_material(Material {
        name: "Oak".into(),
        color: Rgba8::rgb(180, 140, 90),
        texture: None,
    });
    let a = extrude_box(&mut src, 0.0, 0.0, 1.0, 1.0, 1.0);
    src.set_object_material(a, Some(oak)).unwrap();
    let item = src.extract_item(&[NodeId::Object(a)], true).unwrap();

    // dst already has a content-equal "Oak".
    let mut dst = Document::new();
    dst.add_material(Material {
        name: "Oak".into(),
        color: Rgba8::rgb(180, 140, 90),
        texture: None,
    });
    let (first, _) = dst
        .insert_document(&item, &insert_at_identity(None))
        .unwrap();
    assert_eq!(first.materials_reused, 1);
    assert_eq!(first.materials_added, 0);
    assert_eq!(dst.material_ids().len(), 1);

    let (second, _) = dst
        .insert_document(&item, &insert_at_identity(None))
        .unwrap();
    assert_eq!(second.materials_reused, 1);
    assert_eq!(dst.material_ids().len(), 1, "palette never grows");
}

/// One undo step: undo hides everything the insert created (the document
/// LOOKS exactly as before), redo restores it, and a save in the undone
/// state round-trips (no dangling references from tombstoned entities).
#[test]
fn insert_is_one_undo_step_and_undone_state_saves() {
    let mut src = Document::new();
    let oid = extrude_box(&mut src, 0.0, 0.0, 1.0, 1.0, 1.0);
    let item = src.extract_item(&[NodeId::Object(oid)], true).unwrap();

    let mut dst = Document::new();
    let _keep = extrude_box(&mut dst, 5.0, 5.0, 6.0, 6.0, 1.0);
    let before_hash = dst.state_hash();
    let visible_before: HashSet<kernel::ObjectId> = dst.visible_object_ids().into_iter().collect();

    dst.insert_document(&item, &insert_at_identity(provenance("lib-1", "h")))
        .unwrap();
    dst.undo().unwrap();

    let visible_after: HashSet<kernel::ObjectId> = dst.visible_object_ids().into_iter().collect();
    assert_eq!(visible_before, visible_after);
    assert!(dst.instance_ids().is_empty());
    assert!(dst.component_ids().is_empty());
    // Palette additions deliberately survive undo (add_material's posture),
    // so the byte-level hash may differ; the undone state must still SAVE
    // and LOAD cleanly.
    let reloaded = Document::load(&dst.save()).expect("undone state round-trips");
    assert!(reloaded.instance_ids().is_empty());
    let _ = before_hash;

    dst.redo().unwrap();
    assert_eq!(dst.instance_ids().len(), 1);
    assert_eq!(dst.component_ids().len(), 1);
}

/// A reflecting pose is refused up front, document untouched (the strong
/// guarantee: reflections cannot bake into world objects).
#[test]
fn insert_refuses_reflecting_pose_untouched() {
    let mut src = Document::new();
    let oid = extrude_box(&mut src, 0.0, 0.0, 1.0, 1.0, 1.0);
    let item = src.extract_item(&[NodeId::Object(oid)], false).unwrap();

    let mut dst = Document::new();
    let before = dst.state_hash();
    let mirror = Transform::scale(Vec3::new(-1.0, 1.0, 1.0));
    let result = dst.insert_document(
        &item,
        &InsertOptions {
            pose: mirror,
            provenance: None,
        },
    );
    assert!(matches!(
        result,
        Err(DocumentError::Transform(TransformError::Reflection))
    ));
    assert_eq!(dst.state_hash(), before);
}

/// A plain (unwrapped) object item inserts as a world object at the pose.
#[test]
fn insert_unwrapped_object_lands_as_world_object_at_pose() {
    let mut src = Document::new();
    let oid = extrude_box(&mut src, 0.0, 0.0, 1.0, 1.0, 1.0);
    let item = src.extract_item(&[NodeId::Object(oid)], false).unwrap();

    let mut dst = Document::new();
    let (report, _) = dst
        .insert_document(
            &item,
            &InsertOptions {
                pose: Transform::translation(Vec3::new(10.0, 0.0, 0.0)),
                provenance: None,
            },
        )
        .unwrap();

    assert_eq!(report.roots.len(), 1);
    let NodeId::Object(new_oid) = report.roots[0] else {
        panic!("expected a world object root");
    };
    let (lo, hi) = object_bbox(&dst, new_oid);
    // The item was re-origined to its bottom center, so the insert pose
    // places that center — not the box's old min corner — at (10, 0, 0).
    assert!(lo.approx_eq(Point3::new(9.5, -0.5, 0.0), 1e-12));
    assert!(hi.approx_eq(Point3::new(10.5, 0.5, 1.0), 1e-12));
}

/// Group items round-trip structurally: names, nesting, and tags survive,
/// and the group inserts as a deep copy.
#[test]
fn insert_group_item_round_trips_structure() {
    let mut src = Document::new();
    let a = extrude_box(&mut src, 0.0, 0.0, 1.0, 1.0, 1.0);
    let b = extrude_box(&mut src, 2.0, 0.0, 3.0, 1.0, 1.0);
    let (gid, _) = src
        .group_nodes(&[NodeId::Object(a), NodeId::Object(b)])
        .unwrap();
    src.set_node_name(NodeId::Group(gid), Some("Table".into()))
        .unwrap();
    src.add_node_tag(NodeId::Group(gid), vec!["Furniture".into()])
        .unwrap();
    let item = src.extract_item(&[NodeId::Group(gid)], false).unwrap();
    let item = Document::load(&item.save()).expect("item file round-trips");

    let mut dst = Document::new();
    let (report, _) = dst
        .insert_document(&item, &insert_at_identity(provenance("lib-g", "h")))
        .unwrap();

    assert_eq!(report.roots.len(), 1);
    let NodeId::Group(new_gid) = report.roots[0] else {
        panic!("expected a group root");
    };
    assert_eq!(dst.group_name(new_gid), Some("Table"));
    assert_eq!(dst.visible_object_ids().len(), 2);
    assert!(
        dst.tag_meta()
            .any(|(path, _)| path == ["Furniture".to_string()]),
        "carried tag path is registered"
    );
}

/// The insert's whole product saves and loads: a full byte-level round trip
/// of the destination document right after an insert.
#[test]
fn inserted_document_saves_and_loads() {
    let mut src = Document::new();
    let oak = src.add_material(Material {
        name: "Oak".into(),
        color: Rgba8::rgb(180, 140, 90),
        texture: None,
    });
    let a = extrude_box(&mut src, 0.0, 0.0, 1.0, 1.0, 1.0);
    src.set_object_material(a, Some(oak)).unwrap();
    let item = src.extract_item(&[NodeId::Object(a)], true).unwrap();

    let mut dst = Document::new();
    dst.insert_document(&item, &insert_at_identity(provenance("lib-1", "h")))
        .unwrap();
    let bytes = dst.save();
    let reloaded = Document::load(&bytes).expect("inserted document round-trips");
    assert_eq!(reloaded.save(), bytes, "byte-stable across the round trip");
}

// ------------------------------------------------------- property tests

proptest! {
    /// Extract-then-insert round-trips a box's world-space bounding box
    /// through an arbitrary translation pose (rule 3: the geometric
    /// contract, not just examples).
    #[test]
    fn prop_extract_insert_round_trips_bbox(
        w in 0.1f64..4.0,
        d in 0.1f64..4.0,
        h in 0.1f64..4.0,
        tx in -10.0f64..10.0,
        ty in -10.0f64..10.0,
        tz in -10.0f64..10.0,
    ) {
        let mut src = Document::new();
        let oid = extrude_box(&mut src, 0.0, 0.0, w, d, h);
        let item = src.extract_item(&[NodeId::Object(oid)], false).unwrap();

        let mut dst = Document::new();
        let pose = Transform::translation(Vec3::new(tx, ty, tz));
        let (report, _) = dst
            .insert_document(&item, &InsertOptions { pose, provenance: None })
            .unwrap();
        let NodeId::Object(new_oid) = report.roots[0] else {
            panic!("expected a world object root");
        };
        let (lo, hi) = object_bbox(&dst, new_oid);
        // Bottom-center anchoring: the insert pose lands the item's bbox
        // bottom center at (tx, ty, tz).
        prop_assert!(lo.approx_eq(Point3::new(tx - w / 2.0, ty - d / 2.0, tz), 1e-9));
        prop_assert!(hi.approx_eq(Point3::new(tx + w / 2.0, ty + d / 2.0, tz + h), 1e-9));
        prop_assert_eq!(
            dst.object(new_oid).unwrap().watertight(),
            WatertightState::Watertight
        );
    }
}

// ------------------------------------------------- palette & summary

/// Material items: `insert_palette` copies the item's palette with content
/// dedupe and returns resolved ids in item order.
#[test]
fn insert_palette_dedupes_and_returns_resolved_ids() {
    let mut item = Document::new();
    item.add_material(Material {
        name: "Oak".into(),
        color: Rgba8::rgb(180, 140, 90),
        texture: None,
    });
    item.add_material(Material {
        name: "Glass".into(),
        color: Rgba8::rgba(200, 220, 255, 100),
        texture: None,
    });

    let mut dst = Document::new();
    let existing = dst.add_material(Material {
        name: "Oak".into(),
        color: Rgba8::rgb(180, 140, 90),
        texture: None,
    });

    let resolved = dst.insert_palette(&item);
    assert_eq!(resolved.len(), 2);
    assert_eq!(resolved[0], existing, "content-equal entry is reused");
    assert_eq!(dst.material_ids().len(), 2, "only Glass was added");
    assert_eq!(dst.material(resolved[1]).unwrap().name, "Glass");

    // Idempotent: a second insert adds nothing.
    let again = dst.insert_palette(&item);
    assert_eq!(again, resolved);
    assert_eq!(dst.material_ids().len(), 2);
}

/// The manifest-only summary reports counts, names, and doc attrs without
/// touching geometry buffers.
#[test]
fn read_item_summary_reports_counts_and_attrs() {
    let mut src = Document::new();
    let oak = src.add_material(Material {
        name: "Oak".into(),
        color: Rgba8::rgb(180, 140, 90),
        texture: None,
    });
    let oid = extrude_box(&mut src, 0.0, 0.0, 1.0, 1.0, 1.0);
    src.set_object_material(oid, Some(oak)).unwrap();
    src.set_node_name(NodeId::Object(oid), Some("Chair".into()))
        .unwrap();
    let mut item = src.extract_item(&[NodeId::Object(oid)], true).unwrap();
    item.attr_set(
        kernel::AttrTarget::Document,
        "hew.library",
        "id",
        kernel::AttrValue::Text("lib-42".into()),
    )
    .unwrap();

    let summary = kernel::read_item_summary(&item.save()).unwrap();
    assert_eq!(summary.objects, 1);
    assert_eq!(summary.materials, 1);
    assert_eq!(summary.components, 1);
    assert_eq!(summary.instances, 1);
    assert_eq!(summary.first_component_name, Some("Chair".to_string()));
    assert_eq!(summary.material_entries[0].name, "Oak");
    assert_eq!(summary.material_entries[0].color, [180, 140, 90, 255]);
    assert_eq!(
        summary.doc_attrs["hew.library"]["id"],
        serde_json::json!("lib-42")
    );

    // Not a container → typed error, mirroring load.
    assert!(kernel::read_item_summary(b"not a zip").is_err());
}

// -------------------------------------------- review-hardening specs

/// Definition-owned sketches carry with their definition through extract →
/// save → load → insert, and the insert's UNDO tombstones them — without
/// that, a save in the undone state would write a sketch whose `owner`
/// names a definition the save excludes (a dangling reference no reader
/// accepts). Redo revives the same sketch.
#[test]
fn insert_carries_def_sketches_and_undo_tombstones_them() {
    let mut src = Document::new();
    let oid = extrude_box(&mut src, 0.0, 0.0, 1.0, 1.0, 1.0);
    let (_, iid, _) = src.make_component(&[NodeId::Object(oid)]).unwrap();
    // Draw into the definition through the instance (a def-owned sketch).
    let (sk, _) = src
        .begin_sketch_on_plane_in_instance(iid, ground())
        .unwrap();
    let sketch = src.sketch_mut(sk).unwrap();
    sketch
        .add_segment(Point3::new(0.2, 0.2, 0.0), Point3::new(0.8, 0.2, 0.0))
        .unwrap();

    let item = src.extract_item(&[NodeId::Instance(iid)], false).unwrap();
    let item = Document::load(&item.save()).expect("item with def sketch round-trips");

    let mut dst = Document::new();
    let (_, change) = dst
        .insert_document(&item, &insert_at_identity(provenance("lib-ds", "h")))
        .unwrap();
    assert_eq!(change.sketches_touched.len(), 1, "the def sketch arrived");
    let def = dst.component_ids()[0];
    assert_eq!(dst.def_member_sketches(def).unwrap().len(), 1);

    // Undo: the whole insert disappears, def sketch included, and the
    // undone state must still SAVE and LOAD (the dangling-owner trap).
    dst.undo().unwrap();
    let reloaded = Document::load(&dst.save()).expect("undone state round-trips");
    assert!(reloaded.component_ids().is_empty());

    // Redo revives the definition with its sketch.
    dst.redo().unwrap();
    assert_eq!(dst.def_member_sketches(def).unwrap().len(), 1);
}

/// make_unique must NOT clone the `hew.library` provenance stamp onto the
/// private copy — a stale identity claim there would let a later insert of
/// the pristine item silently reuse the user's diverged geometry once the
/// original definition is gone (adversarial review S1).
#[test]
fn make_unique_strips_the_library_provenance_stamp() {
    let mut src = Document::new();
    let oid = extrude_box(&mut src, 0.0, 0.0, 1.0, 1.0, 1.0);
    let item = src.extract_item(&[NodeId::Object(oid)], true).unwrap();

    let mut dst = Document::new();
    let prov = provenance("lib-1", "hash-1");
    dst.insert_document(&item, &insert_at_identity(prov.clone()))
        .unwrap();
    let first_instance = dst.instance_ids()[0];
    let (unique_def, _) = dst.make_unique(first_instance).unwrap();

    // The private copy carries no identity claim…
    let attrs = dst
        .attr_get(&kernel::AttrTarget::Entity(kernel::EntityRef::Component(
            unique_def,
        )))
        .unwrap();
    assert!(
        attrs.is_none_or(|d| !d.contains_key("hew.library")),
        "make_unique's copy must not claim to BE the library item"
    );
    // …so a fresh insert copies the pristine item rather than reusing the
    // diverged private definition, even with the original def gone.
    let (report, _) = dst
        .insert_document(&item, &insert_at_identity(prov))
        .unwrap();
    // The original stamped definition is still live here, so reuse hits
    // THAT (correct); the private copy never matches.
    assert_eq!(report.definitions_reused, 1);
    assert!(dst.component_ids().len() >= 2);
}

/// Extraction refuses a selection that lists both an ancestor group and a
/// node inside it — the nested node would otherwise copy twice.
#[test]
fn extract_refuses_root_nested_under_another_root() {
    let mut doc = Document::new();
    let a = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let b = extrude_box(&mut doc, 2.0, 0.0, 3.0, 1.0, 1.0);
    let (gid, _) = doc
        .group_nodes(&[NodeId::Object(a), NodeId::Object(b)])
        .unwrap();
    assert!(matches!(
        doc.extract_item(&[NodeId::Group(gid), NodeId::Object(a)], false),
        Err(DocumentError::DuplicateMember)
    ));
}

/// A user-hidden selection saves as a VISIBLE item — hide state is view
/// state of the source document, not of the library part; an item that
/// inserted invisibly would read as data loss.
#[test]
fn extract_clears_user_hidden_on_the_item_root() {
    let mut doc = Document::new();
    let oid = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    doc.set_node_user_hidden(NodeId::Object(oid), true);

    let item = doc.extract_item(&[NodeId::Object(oid)], false).unwrap();
    let root = item.top_level_nodes()[0];
    assert!(!item.node_user_hidden(root), "the item's root is visible");
}

/// `insert_palette` carries a material's attribute dictionaries, exactly
/// as `insert_document`'s material copies do.
#[test]
fn insert_palette_carries_material_attrs() {
    let mut item = Document::new();
    let mid = item.add_material(Material {
        name: "Oak".into(),
        color: Rgba8::rgb(180, 140, 90),
        texture: None,
    });
    item.attr_set(
        kernel::AttrTarget::Entity(kernel::EntityRef::Material(mid)),
        "com.example.paint",
        "sku",
        kernel::AttrValue::Text("OAK-7".into()),
    )
    .unwrap();

    let mut dst = Document::new();
    let resolved = dst.insert_palette(&item);
    let dict = dst
        .attr_get(&kernel::AttrTarget::Entity(kernel::EntityRef::Material(
            resolved[0],
        )))
        .unwrap()
        .expect("dictionary carried");
    assert!(dict.contains_key("com.example.paint"));
}

/// `stamp_library_source` marks a saved selection's instance AND its
/// definition, so (a) the source model immediately counts the item as "in
/// this model" material, and (b) re-inserting the just-saved item reuses
/// the definition it was saved FROM instead of minting a duplicate — the
/// playtest expectation: "I saved this component from this model."
#[test]
fn stamp_library_source_makes_reinsert_reuse_the_source_definition() {
    let mut doc = Document::new();
    let oid = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let (cid, iid, _) = doc.make_component(&[NodeId::Object(oid)]).unwrap();

    // Save-to-library: extract, then stamp the source with the item's
    // provenance (the app computes the hash from the item bytes and reads
    // def_sid from the item's summary).
    let item = doc.extract_item(&[NodeId::Instance(iid)], false).unwrap();
    let item_bytes = item.save();
    let summary = kernel::read_item_summary(&item_bytes).unwrap();
    let def_sid: u64 = summary.first_component_sid.unwrap().parse().unwrap();
    let prov = LibraryProvenance {
        source_id: "lib-src".into(),
        content_hash: "hash-src".into(),
    };
    doc.stamp_library_source(&[NodeId::Instance(iid)], &prov, Some(def_sid));

    // (b) Re-inserting the item reuses the ORIGINAL definition.
    let item = Document::load(&item_bytes).unwrap();
    let (report, _) = doc
        .insert_document(
            &item,
            &InsertOptions {
                pose: Transform::translation(Vec3::new(3.0, 0.0, 0.0)),
                provenance: Some(prov),
            },
        )
        .unwrap();
    assert_eq!(report.definitions_reused, 1);
    assert_eq!(doc.component_ids().len(), 1, "no duplicate definition");
    assert_eq!(doc.instance_ids().len(), 2);
    let _ = cid;

    // The stamp survives save/load (it is serialized document state).
    let reloaded = Document::load(&doc.save()).unwrap();
    assert_eq!(reloaded.component_ids().len(), 1);
}

/// The stamp is NOT an undo step: it pushes nothing onto the stack, so the
/// next undo steps over it to the last real model edit, and the label
/// survives an undo/redo cycle of that edit.
#[test]
fn stamp_library_source_is_not_undoable() {
    let mut doc = Document::new();
    let oid = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let prov = LibraryProvenance {
        source_id: "lib-src".into(),
        content_hash: "h".into(),
    };
    doc.stamp_library_source(&[NodeId::Object(oid)], &prov, None);

    // The top of the stack is the EXTRUSION, not the stamp: one undo hides
    // the box (stamp contributed no entry of its own)…
    doc.undo().unwrap();
    assert!(doc.visible_object_ids().is_empty());
    // …and after redo the label is still on the object.
    doc.redo().unwrap();
    let dict = doc
        .attr_get(&kernel::AttrTarget::Entity(kernel::EntityRef::Object(oid)))
        .unwrap()
        .expect("stamped");
    assert!(dict.contains_key("hew.library"));
    // Nothing further to undo beyond the extrusion+sketch history — the
    // stamp never becomes an undoable step of its own.
}

/// Material content hashes agree across the two sides that compare them:
/// a live palette entry and the same material read back out of an item
/// file's manifest-level summary.
#[test]
fn material_content_hash_agrees_between_palette_and_summary() {
    let mut item = Document::new();
    let mid = item.add_material(Material {
        name: "Oak".into(),
        color: Rgba8::rgb(180, 140, 90),
        texture: None,
    });
    let live_hash = kernel::material_content_hash(item.material(mid).unwrap());
    let summary = kernel::read_item_summary(&item.save()).unwrap();
    assert_eq!(
        summary.material_entries[0].content_hash.as_deref(),
        Some(live_hash.to_string().as_str())
    );
}

/// The "in this model" association must survive the source document's own
/// save/load round trip: the stamp is ordinary v14 attrs and must come
/// back on the SAME entities (playtest round 3: the badge died on
/// reopen).
#[test]
fn stamp_survives_source_document_save_and_load() {
    let mut doc = Document::new();
    let oid = extrude_box(&mut doc, 0.0, 0.0, 1.0, 1.0, 1.0);
    let (_, iid, _) = doc.make_component(&[NodeId::Object(oid)]).unwrap();
    let prov = LibraryProvenance {
        source_id: "lib-rt".into(),
        content_hash: "h-rt".into(),
    };
    doc.stamp_library_source(&[NodeId::Instance(iid)], &prov, Some(7));

    let reloaded = Document::load(&doc.save()).unwrap();
    let new_iid = reloaded.instance_ids()[0];
    let new_cid = reloaded.component_ids()[0];

    let has_stamp = |target: kernel::AttrTarget| -> bool {
        reloaded.attr_get(&target).unwrap().is_some_and(|d| {
            d.get("hew.library").is_some_and(|ns| {
                matches!(ns.get("source_id"), Some(kernel::AttrValue::Text(s)) if s == "lib-rt")
            })
        })
    };
    assert!(
        has_stamp(kernel::AttrTarget::Entity(kernel::EntityRef::Instance(
            new_iid
        ))),
        "instance stamp lost across save/load"
    );
    assert!(
        has_stamp(kernel::AttrTarget::Entity(kernel::EntityRef::Component(
            new_cid
        ))),
        "definition stamp lost across save/load"
    );
}

// A compile-time reminder that the report type stays exhaustive: adding a
// field forces this constructor (and the specs above) to acknowledge it.
#[allow(dead_code)]
fn report_shape(r: &InsertReport) -> usize {
    r.roots.len()
        + r.definitions_added
        + r.definitions_reused
        + r.materials_added
        + r.materials_reused
        + r.objects_added
        + r.guides_added
        + r.world_sketches_skipped
        + r.annotations_skipped
}

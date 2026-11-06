//! Acceptance specs for model health at the document level
//! (`Document::health_report`, the import report's counts): what an import
//! leaves open, by kind, with the importer's split pieces grouped as one
//! family and a shared definition counted once.

use kernel::health::{SURFACE_KEY, SURFACE_NS, Verdict};
use kernel::{
    AttrTarget, AttrValue, DefRecipe, Document, EntityRef, ImportNode, ImportScene, MeshRecipe,
    Point3, Transform, Vec3,
};

fn scene(roots: Vec<ImportNode>, defs: Vec<DefRecipe>) -> ImportScene {
    ImportScene {
        materials: Vec::new(),
        defs,
        roots,
        guides: Vec::new(),
        tags: Vec::new(),
        camera: None,
        scenes: Vec::new(),
        section: None,
        view_style: None,
        extra: Default::default(),
        geo: None,
    }
}

fn mesh(name: &str, positions: Vec<Point3>, faces: Vec<Vec<usize>>) -> MeshRecipe {
    let n = faces.len();
    MeshRecipe {
        name: name.into(),
        positions,
        faces,
        face_materials: vec![kernel::NO_MATERIAL; n],
        face_uv_frames: vec![None; n],
        face_holes: vec![Vec::new(); n],
        base_material: kernel::NO_MATERIAL,
        tags: Vec::new(),
        extra: Default::default(),
    }
}

/// The unit-high box from `x0` to `x1`, without face `drop` (3 is the
/// x = x1 side, 5 the x = x0 side).
fn half(name: &str, x0: f64, x1: f64, drop: usize) -> MeshRecipe {
    open_half(name, x0, x1, &[drop])
}

/// [`half`] without every face in `drops`.
fn open_half(name: &str, x0: f64, x1: f64, drops: &[usize]) -> MeshRecipe {
    let p = |x: f64, y: f64, z: f64| Point3::new(x, y, z);
    let positions = vec![
        p(x0, 0.0, 0.0),
        p(x1, 0.0, 0.0),
        p(x1, 1.0, 0.0),
        p(x0, 1.0, 0.0),
        p(x0, 0.0, 1.0),
        p(x1, 0.0, 1.0),
        p(x1, 1.0, 1.0),
        p(x0, 1.0, 1.0),
    ];
    let faces: Vec<Vec<usize>> = [
        vec![0, 3, 2, 1],
        vec![4, 5, 6, 7],
        vec![0, 1, 5, 4],
        vec![1, 2, 6, 5],
        vec![2, 3, 7, 6],
        vec![3, 0, 4, 7],
    ]
    .into_iter()
    .enumerate()
    .filter(|(i, _)| !drops.contains(i))
    .map(|(_, f)| f)
    .collect();
    mesh(name, positions, faces)
}

fn partition() -> MeshRecipe {
    let p = |x: f64, y: f64, z: f64| Point3::new(x, y, z);
    mesh(
        "Partition",
        vec![
            p(1.0, 0.0, 0.0),
            p(1.0, 1.0, 0.0),
            p(1.0, 1.0, 1.0),
            p(1.0, 0.0, 1.0),
        ],
        vec![vec![0, 1, 2, 3]],
    )
}

/// What the importer's non-manifold split emits for a box with an internal
/// partition at x = 1: two open halves and the partition sheet, sharing
/// the partition's rim.
fn split_box() -> ImportScene {
    scene(
        vec![
            ImportNode::Mesh(half("Left", 0.0, 1.0, 3)),
            ImportNode::Mesh(half("Right", 1.0, 2.0, 5)),
            ImportNode::Mesh(partition()),
        ],
        Vec::new(),
    )
}

#[test]
fn a_split_solid_reads_as_one_family_of_two_cappable_halves_and_a_sheet() {
    let mut doc = Document::new();
    let (report, _) = doc.ingest(split_box(), Vec::new()).unwrap();
    let health = doc.health_report(None);
    assert_eq!(health.rows.len(), 3);
    let mut verdicts: Vec<Verdict> = health.rows.iter().map(|r| r.diagnosis.verdict).collect();
    verdicts.sort();
    assert_eq!(
        verdicts,
        vec![
            Verdict::FlatSheet { holes: 0 },
            Verdict::Cappable,
            Verdict::Cappable
        ]
    );
    assert_eq!(health.families.len(), 1);
    assert_eq!(health.families[0].members.len(), 3);
    assert_eq!(health.families[0].shared_rims, 1);
    assert!(health.rows.iter().all(|r| r.seam_family == Some(0)));
    assert!(health.rows.iter().all(|r| r.shared_rims == vec![0]));
    assert!(
        health
            .rows
            .iter()
            .all(|r| r.placements == 1 && r.definition.is_none())
    );

    // The import report says the same, per kind.
    let c = report.health;
    assert_eq!((c.objects, c.solids), (3, 0));
    assert_eq!((c.cappable, c.flat_sheets, c.seam_families), (2, 1, 1));
    assert_eq!((c.surfaces(), c.needing_attention()), (1, 2));
    assert_eq!(report.leaky, 3, "`leaky` keeps counting every open object");
}

#[test]
fn a_split_pieces_seam_is_shared_and_its_own_opening_is_not() {
    // A sleeve open at both ends (x = 0 and x = 2) with a partition at
    // x = 1: each half shares only its x = 1 rim; its far end is an opening
    // the sleeve was drawn with.
    let mut doc = Document::new();
    let s = scene(
        vec![
            ImportNode::Mesh(open_half("Left", 0.0, 1.0, &[3, 5])),
            ImportNode::Mesh(open_half("Right", 1.0, 2.0, &[3, 5])),
            ImportNode::Mesh(partition()),
        ],
        Vec::new(),
    );
    doc.ingest(s, Vec::new()).unwrap();
    let health = doc.health_report(None);
    assert_eq!(health.families.len(), 1);
    for row in &health.rows {
        let obj = doc.object(row.object).unwrap();
        let rim_x = |i: usize| {
            let h = row.diagnosis.rims[i].half_edges[0];
            obj.vertices()[obj.half_edges()[h].origin].position.x
        };
        assert_eq!(row.shared_rims.len(), 1, "one seam per member");
        assert!(
            (rim_x(row.shared_rims[0]) - 1.0).abs() < 1e-9,
            "the seam is the x = 1 rim"
        );
        let others = (0..row.diagnosis.rims.len()).filter(|i| !row.shared_rims.contains(i));
        assert!(others.into_iter().all(|i| (rim_x(i) - 1.0).abs() > 0.5));
    }
}

#[test]
fn rims_in_different_coordinate_spaces_never_join_a_family() {
    // The same two halves, but one is a component member: its rim lives in
    // definition space, so even a coinciding position is no seam.
    let mut doc = Document::new();
    let s = scene(
        vec![
            ImportNode::Mesh(half("Left", 0.0, 1.0, 3)),
            ImportNode::Instance {
                def: 0,
                pose: Transform::IDENTITY,
                name: None,
                tags: Vec::new(),
                hidden: false,
                key: None,
            },
        ],
        vec![DefRecipe {
            name: Some("Right".into()),
            children: vec![ImportNode::Mesh(half("Right", 1.0, 2.0, 5))],
        }],
    );
    doc.ingest(s, Vec::new()).unwrap();
    assert!(doc.health_report(None).families.is_empty());
}

#[test]
fn a_definition_member_is_one_row_however_often_it_is_placed() {
    let place = |x: f64| ImportNode::Instance {
        def: 0,
        pose: Transform::translation(Vec3::new(x, 0.0, 0.0)),
        name: None,
        tags: Vec::new(),
        hidden: false,
        key: None,
    };
    let s = scene(
        vec![place(0.0), place(5.0), place(10.0)],
        vec![DefRecipe {
            name: Some("Tray".into()),
            children: vec![ImportNode::Mesh(half("Tray", 0.0, 1.0, 1))],
        }],
    );
    let mut doc = Document::new();
    let (report, _) = doc.ingest(s, Vec::new()).unwrap();
    let health = doc.health_report(None);
    assert_eq!(health.rows.len(), 1, "one definition member, one row");
    let row = &health.rows[0];
    assert!(row.definition.is_some());
    assert_eq!(row.placements, 3);
    assert_eq!(row.diagnosis.verdict, Verdict::Cappable);
    assert_eq!(
        report.health.cappable, 1,
        "the import report counts it once too"
    );
    // A whole-document report sees members the world-object list skips.
    assert!(doc.visible_object_ids().is_empty());
}

#[test]
fn an_accepted_surface_is_flagged_and_still_open() {
    let mut doc = Document::new();
    doc.ingest(
        scene(vec![ImportNode::Mesh(partition())], Vec::new()),
        Vec::new(),
    )
    .unwrap();
    let id = doc.live_object_ids()[0];
    assert!(!doc.health_report(None).rows[0].accepted_surface);
    doc.attr_set(
        AttrTarget::Entity(EntityRef::Object(id)),
        SURFACE_NS,
        SURFACE_KEY,
        AttrValue::Bool(true),
    )
    .unwrap();
    let row = &doc.health_report(None).rows[0];
    assert!(row.accepted_surface);
    assert_eq!(row.diagnosis.verdict, Verdict::FlatSheet { holes: 0 });
    // Undo takes the flag back.
    doc.undo().unwrap();
    assert!(!doc.is_accepted_surface(id));
}

#[test]
fn a_scoped_report_covers_only_its_objects() {
    let mut doc = Document::new();
    doc.ingest(split_box(), Vec::new()).unwrap();
    let ids = doc.live_object_ids();
    let one = doc.health_report(Some(&ids[..1]));
    assert_eq!(one.rows.len(), 1);
    assert!(
        one.families.is_empty(),
        "a family needs its siblings in scope"
    );
    assert_eq!(doc.object_health(ids[0]).unwrap(), one.rows[0].diagnosis);
}

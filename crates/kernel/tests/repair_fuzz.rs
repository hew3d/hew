//! Fuzz harness for the Model Health repairs (DEVELOPMENT.md rule 3): random
//! caps, face deletions, surface marks and push/pulls, interleaved with
//! undo/redo, over a document seeded with the open shapes imports leave —
//! a box missing its lid (world, and inside out in a group), a pipe open at
//! both ends, a strip of faces, a solid with a stray sheet beside it, and
//! an open box shared by two instances — with torture mode on. Thickening
//! a flat sheet into a solid joins them.
//!
//! The document fuzzer (`document_fuzz.rs`) holds every object watertight,
//! which open imports are not by design, so the repairs get their own
//! harness. Invariants:
//! - after every op (applied or refused typed) every live object, world or
//!   definition member, validates; no repair refuses as `Invalid`;
//! - `save()` is deterministic and `load(save())` keeps the `state_hash`;
//! - every undo/redo succeeds, and unwinding the whole log and replaying it
//!   twice reproduces the same states (canonical fingerprints).

use kernel::repair::RepairError;
use kernel::{
    DefRecipe, Document, DocumentError, FaceId, ImportNode, ImportScene, KernelOp, MeshRecipe,
    ObjectId, Point3, Transform, Vec3,
};
use proptest::prelude::*;

#[derive(Debug, Clone)]
enum RepairOp {
    /// Cap the `obj_sel`-th live object: every rim that can take a face,
    /// or the rims in `mask` (bit i = rim i).
    Cap {
        obj_sel: usize,
        mask: Option<u8>,
    },
    /// Delete `count` faces starting at the `face_sel`-th.
    Delete {
        obj_sel: usize,
        face_sel: usize,
        count: usize,
    },
    Mark {
        obj_sel: usize,
        on: bool,
    },
    Thicken {
        obj_sel: usize,
        start: f64,
        end: f64,
    },
    PushPull {
        obj_sel: usize,
        face_sel: usize,
        distance: f64,
    },
    Undo,
    Redo,
}

fn arb_op() -> impl Strategy<Value = RepairOp> {
    prop_oneof![
        3 => (any::<usize>(), proptest::option::of(1u8..16))
            .prop_map(|(obj_sel, mask)| RepairOp::Cap { obj_sel, mask }),
        2 => (any::<usize>(), any::<usize>(), 1usize..4)
            .prop_map(|(obj_sel, face_sel, count)| RepairOp::Delete { obj_sel, face_sel, count }),
        1 => (any::<usize>(), any::<bool>())
            .prop_map(|(obj_sel, on)| RepairOp::Mark { obj_sel, on }),
        2 => (any::<usize>(), -0.3..0.3f64, -0.3..0.3f64)
            .prop_map(|(obj_sel, start, end)| RepairOp::Thicken { obj_sel, start, end }),
        2 => (any::<usize>(), any::<usize>(), prop_oneof![-0.4..-0.05f64, 0.05..0.6f64])
            .prop_map(|(obj_sel, face_sel, distance)| RepairOp::PushPull { obj_sel, face_sel, distance }),
        2 => Just(RepairOp::Undo),
        1 => Just(RepairOp::Redo),
    ]
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

fn box_corners(x0: f64, w: f64, h: f64) -> Vec<Point3> {
    let p = |x: f64, y: f64, z: f64| Point3::new(x0 + x, y, z);
    vec![
        p(0.0, 0.0, 0.0),
        p(w, 0.0, 0.0),
        p(w, w, 0.0),
        p(0.0, w, 0.0),
        p(0.0, 0.0, h),
        p(w, 0.0, h),
        p(w, w, h),
        p(0.0, w, h),
    ]
}

fn box_faces() -> Vec<Vec<usize>> {
    vec![
        vec![0, 3, 2, 1],
        vec![4, 5, 6, 7],
        vec![0, 1, 5, 4],
        vec![1, 2, 6, 5],
        vec![2, 3, 7, 6],
        vec![3, 0, 4, 7],
    ]
}

fn open_box(name: &str, x0: f64, inside_out: bool) -> MeshRecipe {
    // Every face but the lid (face 1), wound inward when `inside_out`.
    let faces = box_faces()
        .into_iter()
        .enumerate()
        .filter(|&(i, _)| i != 1)
        .map(|(_, mut f)| {
            if inside_out {
                f.reverse();
            }
            f
        })
        .collect();
    mesh(name, box_corners(x0, 1.0, 1.0), faces)
}

fn pipe(x0: f64) -> MeshRecipe {
    let mut positions = box_corners(x0, 2.0, 1.0);
    positions.extend(
        box_corners(x0, 1.0, 1.0)
            .into_iter()
            .map(|p| Point3::new(p.x + 0.5, p.y + 0.5, p.z)),
    );
    let walls = || box_faces().into_iter().skip(2);
    let mut faces: Vec<Vec<usize>> = walls().collect();
    faces.extend(walls().map(|f| f.into_iter().rev().map(|i| i + 8).collect()));
    mesh("Pipe", positions, faces)
}

fn strip(x0: f64) -> MeshRecipe {
    let p = |x: f64, y: f64| Point3::new(x0 + x, y, 0.0);
    mesh(
        "Strip",
        vec![
            p(0.0, 0.0),
            p(1.0, 0.0),
            p(2.0, 0.0),
            p(3.0, 0.0),
            p(0.0, 1.0),
            p(1.0, 1.0),
            p(2.0, 1.0),
            p(3.0, 1.0),
        ],
        vec![vec![0, 1, 5, 4], vec![1, 2, 6, 5], vec![2, 3, 7, 6]],
    )
}

fn crate_with_sheet(x0: f64) -> MeshRecipe {
    let mut positions = box_corners(x0, 1.0, 1.0);
    positions.extend(box_corners(x0 + 2.0, 1.0, 0.0)[..4].iter().copied());
    let mut faces = box_faces();
    faces.push(vec![8, 9, 10, 11]);
    mesh("Crate", positions, faces)
}

fn seed() -> Document {
    let place = |x: f64| ImportNode::Instance {
        def: 0,
        pose: Transform::translation(Vec3::new(x, 20.0, 0.0)),
        name: None,
        tags: Vec::new(),
        hidden: false,
        key: None,
    };
    let mut doc = Document::new();
    doc.set_torture_mode(true);
    doc.ingest(
        ImportScene {
            materials: Vec::new(),
            defs: vec![DefRecipe {
                name: Some("Bin".into()),
                children: vec![ImportNode::Mesh(open_box("Bin", 0.0, false))],
            }],
            roots: vec![
                ImportNode::Mesh(open_box("Tank", 0.0, false)),
                ImportNode::Group {
                    name: "Shed".into(),
                    children: vec![
                        ImportNode::Mesh(open_box("Flipped", 3.0, true)),
                        ImportNode::Mesh(strip(6.0)),
                    ],
                    tags: Vec::new(),
                    hidden: false,
                    key: None,
                },
                ImportNode::Mesh(pipe(11.0)),
                ImportNode::Mesh(crate_with_sheet(15.0)),
                place(0.0),
                place(4.0),
            ],
            guides: Vec::new(),
            tags: Vec::new(),
            camera: None,
            scenes: Vec::new(),
            section: None,
            view_style: None,
            extra: Default::default(),
            geo: None,
        },
        Vec::new(),
    )
    .unwrap();
    doc
}

fn pick<T: Copy>(items: &[T], sel: usize) -> Option<T> {
    (!items.is_empty()).then(|| items[sel % items.len()])
}

fn faces_of(doc: &Document, id: ObjectId) -> Vec<FaceId> {
    doc.object(id).unwrap().faces().keys().collect()
}

/// Apply one op. Typed refusals are fine; a repair refusing as `Invalid`
/// (its own result failed validation) and a failing undo/redo are not.
fn apply(doc: &mut Document, op: &RepairOp) -> Result<(), TestCaseError> {
    let ids = doc.live_object_ids();
    let result = match *op {
        RepairOp::Cap { obj_sel, mask } => {
            let Some(id) = pick(&ids, obj_sel) else {
                return Ok(());
            };
            let rims: Option<Vec<usize>> =
                mask.map(|m| (0..8).filter(|i| m & (1 << i) != 0).collect());
            doc.cap_rims(id, rims.as_deref()).map(|_| ())
        }
        RepairOp::Delete {
            obj_sel,
            face_sel,
            count,
        } => {
            let Some(id) = pick(&ids, obj_sel) else {
                return Ok(());
            };
            let faces = faces_of(doc, id);
            let chosen: Vec<FaceId> = (0..count)
                .map(|k| faces[(face_sel + k) % faces.len()])
                .collect();
            doc.delete_faces(id, &chosen).map(|_| ())
        }
        RepairOp::Mark { obj_sel, on } => {
            let Some(id) = pick(&ids, obj_sel) else {
                return Ok(());
            };
            doc.set_accepted_surface(id, on).map(|_| ())
        }
        RepairOp::Thicken {
            obj_sel,
            start,
            end,
        } => {
            let Some(id) = pick(&ids, obj_sel) else {
                return Ok(());
            };
            doc.thicken_sheet(id, start, end).map(|_| ())
        }
        RepairOp::PushPull {
            obj_sel,
            face_sel,
            distance,
        } => {
            let world = doc.visible_object_ids();
            let Some(id) = pick(&world, obj_sel) else {
                return Ok(());
            };
            let Some(face) = pick(&faces_of(doc, id), face_sel) else {
                return Ok(());
            };
            doc.apply_object_op(id, KernelOp::PushPull { face, distance })
                .map(|_| ())
        }
        RepairOp::Undo => {
            if doc.can_undo() {
                doc.undo()
                    .map_err(|e| TestCaseError::fail(format!("undo: {e:?}")))?;
            }
            return Ok(());
        }
        RepairOp::Redo => {
            if doc.can_redo() {
                doc.redo()
                    .map_err(|e| TestCaseError::fail(format!("redo: {e:?}")))?;
            }
            return Ok(());
        }
    };
    match result {
        Err(DocumentError::Repair(RepairError::Invalid)) => Err(TestCaseError::fail(format!(
            "{op:?}: the repair produced invalid topology"
        ))),
        _ => Ok(()),
    }
}

fn check_doc(doc: &Document, what: &str) -> Result<(), TestCaseError> {
    for id in doc.live_object_ids() {
        if let Err(e) = doc.object(id).unwrap().validate() {
            return Err(TestCaseError::fail(format!("{what}: {id:?} invalid: {e}")));
        }
    }
    let bytes = doc.save();
    prop_assert_eq!(&bytes, &doc.save(), "{}: save is not deterministic", what);
    let reloaded = Document::load(&bytes)
        .map_err(|e| TestCaseError::fail(format!("{what}: load(save) failed: {e}")))?;
    prop_assert_eq!(
        reloaded.state_hash(),
        doc.state_hash(),
        "{}: state hash changed across save/load",
        what
    );
    Ok(())
}

/// Per live object (world and member): its sorted vertex positions rounded
/// well below any tolerance, its face count, whether it is closed and
/// whether it is marked as a surface — the model up to handle renaming.
/// One object's fingerprint: quantized sorted vertices, face count,
/// closed, marked as a surface.
type ObjectPrint = (Vec<(i64, i64, i64)>, usize, bool, bool);

fn fingerprint(doc: &Document) -> Vec<ObjectPrint> {
    let q = |v: f64| (v * 1e6).round() as i64;
    let mut out: Vec<_> = doc
        .live_object_ids()
        .into_iter()
        .map(|id| {
            let obj = doc.object(id).unwrap();
            let mut pts: Vec<(i64, i64, i64)> = obj
                .vertices()
                .values()
                .map(|v| (q(v.position.x), q(v.position.y), q(v.position.z)))
                .collect();
            pts.sort_unstable();
            (
                pts,
                obj.faces().len(),
                obj.watertight() == kernel::WatertightState::Watertight,
                doc.is_accepted_surface(id),
            )
        })
        .collect();
    out.sort();
    out
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(96))]

    #[test]
    fn repair_sequences_stay_valid_and_unwind_exactly(
        ops in proptest::collection::vec(arb_op(), 1..16),
    ) {
        let mut doc = seed();
        check_doc(&doc, "seed")?;
        for (step, op) in ops.iter().enumerate() {
            apply(&mut doc, op)?;
            check_doc(&doc, &format!("step {step} {op:?}"))?;
        }

        let unwind = |doc: &mut Document, label: &str| -> Result<(), TestCaseError> {
            while doc.can_undo() {
                doc.undo().map_err(|e| TestCaseError::fail(format!("{label}: undo {e:?}")))?;
            }
            Ok(())
        };
        let replay = |doc: &mut Document, label: &str| -> Result<(), TestCaseError> {
            while doc.can_redo() {
                doc.redo().map_err(|e| TestCaseError::fail(format!("{label}: redo {e:?}")))?;
            }
            Ok(())
        };
        unwind(&mut doc, "first unwind")?;
        let empty = fingerprint(&doc);
        replay(&mut doc, "first replay")?;
        let maximal = fingerprint(&doc);
        check_doc(&doc, "maximal")?;
        unwind(&mut doc, "second unwind")?;
        prop_assert_eq!(fingerprint(&doc), empty);
        replay(&mut doc, "second replay")?;
        prop_assert_eq!(fingerprint(&doc), maximal);
    }
}

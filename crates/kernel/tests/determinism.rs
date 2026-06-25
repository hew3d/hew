//! Determinism guard (docs/DEVELOPMENT.md): a random document-op sequence
//! run **twice** must yield an identical [`Document::state_hash`] *and*
//! byte-identical [`Document::save`] output.
//!
//! This is the mechanical regression test for the whole determinism lane.
//! The two runs build independent documents in the same process — so their
//! allocator state and any (now-banned) `HashMap` seeds differ — meaning the
//! moment someone reintroduces iteration-order or RNG nondeterminism, this fails
//! at the operation that did it. It is the guard *for* the BTreeMap migration
//! and the FNV `state_hash` oracle, not just a user of them.

use kernel::{BooleanOp, Document, FaceId, NodeId, ObjectId, Plane, Point3, Transform, Vec3};
use proptest::prelude::*;

/// One scripted document operation. Targets are resolved by index **modulo the
/// live object count at apply time**, so the identical `Op` sequence drives two
/// fresh documents along the identical path; any state divergence is a bug.
#[derive(Debug, Clone)]
enum Op {
    /// Extrude an axis-aligned box (exercises the build / sketch-region path).
    AddBox {
        x: f64,
        y: f64,
        w: f64,
        d: f64,
        z: f64,
        h: f64,
    },
    /// Translate an object (baked affine transform).
    Translate {
        obj: usize,
        dx: f64,
        dy: f64,
        dz: f64,
    },
    /// Union / Subtract / Intersect two distinct objects (the heaviest
    /// determinism risk — the boolean arrangement code).
    Boolean { which: u8, a: usize, b: usize },
    /// Slice an object with an axis-aligned plane (reuses the booleans).
    Slice { obj: usize, axis: u8, offset: f64 },
    /// Push/pull (incl. through-cut) a face of an object.
    PushPull { obj: usize, face: usize, dist: f64 },
    /// Group two distinct objects (document tree mutation).
    Group { a: usize, b: usize },
    /// Deep-clone an object at a translation (fresh ids).
    Duplicate {
        obj: usize,
        dx: f64,
        dy: f64,
        dz: f64,
    },
    /// Delete an object (tombstone).
    Delete { obj: usize },
}

/// Coordinates are kept in a small range so boxes frequently overlap and the
/// boolean / slice / through-cut paths actually do arrangement work.
fn arb_op() -> impl Strategy<Value = Op> {
    let coord = -2.0f64..4.0;
    let size = 0.5f64..4.0;
    let delta = -3.0f64..3.0;
    let idx = 0usize..6;
    prop_oneof![
        (
            coord.clone(),
            coord.clone(),
            size.clone(),
            size.clone(),
            coord.clone(),
            size
        )
            .prop_map(|(x, y, w, d, z, h)| Op::AddBox { x, y, w, d, z, h }),
        (idx.clone(), delta.clone(), delta.clone(), delta.clone())
            .prop_map(|(obj, dx, dy, dz)| Op::Translate { obj, dx, dy, dz }),
        (0u8..3, idx.clone(), idx.clone()).prop_map(|(which, a, b)| Op::Boolean { which, a, b }),
        (idx.clone(), 0u8..3, -2.0f64..5.0).prop_map(|(obj, axis, offset)| Op::Slice {
            obj,
            axis,
            offset
        }),
        (idx.clone(), 0usize..8, delta.clone()).prop_map(|(obj, face, dist)| Op::PushPull {
            obj,
            face,
            dist
        }),
        (idx.clone(), idx.clone()).prop_map(|(a, b)| Op::Group { a, b }),
        (idx.clone(), delta.clone(), delta.clone(), delta.clone())
            .prop_map(|(obj, dx, dy, dz)| Op::Duplicate { obj, dx, dy, dz }),
        idx.prop_map(|obj| Op::Delete { obj }),
    ]
}

/// Extrude an axis-aligned box. Tolerant (returns `None` instead of panicking) so
/// a degenerate request is a clean no-op — identical in both runs.
fn add_box(doc: &mut Document, x: f64, y: f64, w: f64, d: f64, z: f64, h: f64) -> Option<ObjectId> {
    let plane =
        Plane::from_point_normal(Point3::new(0.0, 0.0, z), Vec3::new(0.0, 0.0, 1.0)).ok()?;
    let s = doc.add_sketch(plane);
    let (x1, y1) = (x + w, y + d);
    let corners = [
        (Point3::new(x, y, z), Point3::new(x1, y, z)),
        (Point3::new(x1, y, z), Point3::new(x1, y1, z)),
        (Point3::new(x1, y1, z), Point3::new(x, y1, z)),
        (Point3::new(x, y1, z), Point3::new(x, y, z)),
    ];
    {
        let sk = doc.sketch_mut(s)?;
        for (a, b) in corners {
            sk.add_segment(a, b).ok()?;
        }
    }
    let r = *doc.extrudable_regions(s).ok()?.first()?;
    Some(doc.extrude_region(s, r, h).ok()?.0)
}

/// Apply one op to `doc`, resolving targets against the current live-object list.
/// Every kernel error is intentionally swallowed: an op that cannot apply is a
/// no-op, and because the inputs are identical it is a no-op in *both* runs.
fn apply(doc: &mut Document, op: &Op) {
    // Target only **top-level** objects — mirroring the UI invariant that ops act
    // on world-context selections, never on a grouped leaf directly. (Driving a
    // replacing op like boolean/slice/through-cut at a grouped member would leave
    // its container pointing at a consumed id — a tree-consistency concern
    // distinct from determinism, and one the UI never reaches.)
    let ids: Vec<ObjectId> = doc
        .visible_object_ids()
        .into_iter()
        .filter(|&id| doc.node_parent(NodeId::Object(id)).is_none())
        .collect();
    let n = ids.len();
    let pick = |i: usize| -> Option<ObjectId> { if n == 0 { None } else { Some(ids[i % n]) } };
    match *op {
        Op::AddBox { x, y, w, d, z, h } => {
            let _ = add_box(doc, x, y, w, d, z, h);
        }
        Op::Translate { obj, dx, dy, dz } => {
            if let Some(id) = pick(obj) {
                let _ = doc.transform_object(id, &Transform::translation(Vec3::new(dx, dy, dz)));
            }
        }
        Op::Boolean { which, a, b } => {
            if n >= 2 {
                let (ai, bi) = (a % n, b % n);
                if ai != bi {
                    let bop = match which % 3 {
                        0 => BooleanOp::Union,
                        1 => BooleanOp::Subtract,
                        _ => BooleanOp::Intersect,
                    };
                    let _ = doc.boolean(bop, ids[ai], ids[bi]);
                }
            }
        }
        Op::Slice { obj, axis, offset } => {
            if let Some(id) = pick(obj) {
                let normal = match axis % 3 {
                    0 => Vec3::new(1.0, 0.0, 0.0),
                    1 => Vec3::new(0.0, 1.0, 0.0),
                    _ => Vec3::new(0.0, 0.0, 1.0),
                };
                if let Ok(plane) =
                    Plane::from_point_normal(Point3::new(offset, offset, offset), normal)
                {
                    let _ = doc.slice_node(id, &plane);
                }
            }
        }
        Op::PushPull { obj, face, dist } => {
            if let Some(id) = pick(obj) {
                let faces: Vec<FaceId> = doc
                    .object(id)
                    .map(|o| o.faces().keys().collect())
                    .unwrap_or_default();
                if !faces.is_empty() {
                    let f = faces[face % faces.len()];
                    let _ = doc.push_pull_through(id, f, dist);
                }
            }
        }
        Op::Group { a, b } => {
            if n >= 2 {
                let (ai, bi) = (a % n, b % n);
                if ai != bi {
                    let _ = doc.group_nodes(&[NodeId::Object(ids[ai]), NodeId::Object(ids[bi])]);
                }
            }
        }
        Op::Duplicate { obj, dx, dy, dz } => {
            if let Some(id) = pick(obj) {
                let _ = doc.duplicate_node(
                    NodeId::Object(id),
                    &Transform::translation(Vec3::new(dx, dy, dz)),
                );
            }
        }
        Op::Delete { obj } => {
            if let Some(id) = pick(obj) {
                let _ = doc.delete_node(NodeId::Object(id));
            }
        }
    }
}

/// Build a document by applying `program` to a fresh, empty document.
fn run(program: &[Op]) -> Document {
    let mut doc = Document::new();
    for op in program {
        apply(&mut doc, op);
    }
    doc
}

proptest! {
    // Booleans are the heavy operation; a few dozen randomized programs of up to
    // ~10 ops is plenty to exercise every path while staying fast in CI.
    #![proptest_config(ProptestConfig::with_cases(48))]

    /// The core guard: the same program is bit-for-bit reproducible.
    #[test]
    fn random_program_is_byte_and_hash_deterministic(program in prop::collection::vec(arb_op(), 1..11)) {
        let a = run(&program);
        let b = run(&program);
        prop_assert_eq!(
            a.state_hash(),
            b.state_hash(),
            "state_hash diverged across two runs of an identical program"
        );
        prop_assert_eq!(
            a.save(),
            b.save(),
            "save() bytes diverged across two runs of an identical program"
        );
    }
}

/// A fast, always-run smoke check (no proptest harness): a fixed sequence that
/// exercises build → boolean → push/pull → slice → duplicate → delete is
/// reproducible. Guards the common path even when proptest cases are filtered.
#[test]
fn scripted_sequence_is_deterministic() {
    let program = [
        Op::AddBox {
            x: 0.0,
            y: 0.0,
            w: 2.0,
            d: 2.0,
            z: 0.0,
            h: 2.0,
        },
        Op::AddBox {
            x: 1.0,
            y: 1.0,
            w: 2.0,
            d: 2.0,
            z: 0.0,
            h: 3.0,
        },
        Op::Boolean {
            which: 0,
            a: 0,
            b: 1,
        }, // union the two overlapping boxes
        Op::PushPull {
            obj: 0,
            face: 0,
            dist: 1.5,
        },
        Op::AddBox {
            x: -1.0,
            y: -1.0,
            w: 1.0,
            d: 1.0,
            z: 0.0,
            h: 1.0,
        },
        Op::Slice {
            obj: 0,
            axis: 2,
            offset: 1.0,
        },
        Op::Duplicate {
            obj: 0,
            dx: 5.0,
            dy: 0.0,
            dz: 0.0,
        },
        Op::Translate {
            obj: 1,
            dx: 0.0,
            dy: 4.0,
            dz: 0.0,
        },
        Op::Delete { obj: 0 },
    ];
    let a = run(&program);
    let b = run(&program);
    assert_eq!(
        a.state_hash(),
        b.state_hash(),
        "state_hash is deterministic"
    );
    assert_eq!(a.save(), b.save(), "save() bytes are deterministic");
    // Sanity: the program actually built something (the guard would be vacuous
    // if every op no-op'd).
    assert!(
        !a.visible_object_ids().is_empty(),
        "program produced objects"
    );
}

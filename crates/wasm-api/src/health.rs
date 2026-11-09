//! Model health for the app (`kernel::health`): the open objects of a
//! scope as JSON for the Model Health panel, and their rims as world-space
//! line segments for the viewport overlay. Pure reads of the document.

use std::collections::BTreeMap;

use kernel::health::{HealthCounts, Verdict};
use kernel::{Document, EntityRef, InstanceId, ObjectId, Point3, Transform};
use slotmap::Key;

/// An entity as the app names it: its FFI handle, in decimal.
fn ffi_id(e: EntityRef) -> String {
    match e {
        EntityRef::Object(id) => id.data().as_ffi(),
        EntityRef::Instance(id) => id.data().as_ffi(),
        EntityRef::Component(id) => id.data().as_ffi(),
        other => unreachable!("the health report names no {other:?}"),
    }
    .to_string()
}

/// Health counts as the app and the import report carry them.
pub(crate) fn counts_json(c: &HealthCounts) -> serde_json::Value {
    api::health_counts_json(c)
}

/// Every world placement of each object: identity for a world object, the
/// composed pose of each placement for a definition member (outermost
/// placing instance alongside).
fn placements(doc: &Document) -> BTreeMap<ObjectId, Vec<(Transform, InstanceId)>> {
    let mut out: BTreeMap<ObjectId, Vec<(Transform, InstanceId)>> = BTreeMap::new();
    for (o, pose, iid) in doc.expanded_placements() {
        out.entry(o).or_default().push((pose, iid));
    }
    out
}

/// See [`crate::Scene::objects_world_bounds`].
pub(crate) fn world_bounds(doc: &Document, targets: &[(ObjectId, Option<InstanceId>)]) -> Vec<f64> {
    let members = targets.iter().any(|&(o, _)| !doc.is_world_object(o));
    let all = if members {
        placements(doc)
    } else {
        BTreeMap::new()
    };
    let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
    for &(id, instance) in targets {
        let Some(obj) = doc.object(id) else { continue };
        let poses: Vec<Transform> = if doc.is_world_object(id) {
            vec![Transform::IDENTITY]
        } else {
            all.get(&id)
                .map(|v| {
                    v.iter()
                        .filter(|(_, iid)| instance.is_none_or(|i| i == *iid))
                        .map(|(t, _)| *t)
                        .collect()
                })
                .unwrap_or_default()
        };
        for pose in &poses {
            for v in obj.vertices().values() {
                let p = pose.apply_point(v.position);
                for (k, c) in [p.x, p.y, p.z].into_iter().enumerate() {
                    lo[k] = lo[k].min(c);
                    hi[k] = hi[k].max(c);
                }
            }
        }
    }
    if lo[0] > hi[0] {
        return Vec::new();
    }
    vec![lo[0], lo[1], lo[2], hi[0], hi[1], hi[2]]
}

fn poses(
    doc: &Document,
    all: &BTreeMap<ObjectId, Vec<(Transform, InstanceId)>>,
    id: ObjectId,
) -> Vec<Transform> {
    if doc.is_world_object(id) {
        vec![Transform::IDENTITY]
    } else {
        all.get(&id)
            .map(|v| v.iter().map(|(t, _)| *t).collect())
            .unwrap_or_default()
    }
}

/// The open objects of `scope` (every live object when `None`), as the
/// Model Health panel lists them ([`api::health::report_json`]).
pub(crate) fn report_json(doc: &Document, scope: Option<&[ObjectId]>) -> String {
    api::health::report_json(doc, scope, &ffi_id).to_string()
}

/// Every live object's verdict in short — counts by kind (without seam
/// families, which need every rim) and `[handle, kind, accepted]` for each
/// open object — for the status badge and the Outliner icons, which must
/// stay cheap on every edit of a big import. `cache` holds verdicts the
/// Scene drops for every object a change touches, so an edit re-diagnoses
/// only what it changed.
pub(crate) fn summary_json(doc: &Document, cache: &mut BTreeMap<ObjectId, Verdict>) -> String {
    let mut counts = HealthCounts::default();
    let mut open = Vec::new();
    for id in doc.live_object_ids() {
        let Some(obj) = doc.object(id) else { continue };
        let verdict = *cache.entry(id).or_insert_with(|| {
            if obj.watertight() == kernel::WatertightState::Watertight {
                Verdict::Solid
            } else {
                kernel::health::diagnose(obj).verdict
            }
        });
        counts.objects += 1;
        match verdict {
            Verdict::Solid => counts.solids += 1,
            Verdict::FlatSheet { .. } => counts.flat_sheets += 1,
            Verdict::BentSheet => counts.bent_sheets += 1,
            Verdict::Cappable => counts.cappable += 1,
            Verdict::Holes { .. } => counts.holes += 1,
            Verdict::Mixed => counts.mixed += 1,
            Verdict::NonManifold => counts.non_manifold += 1,
        }
        if verdict != Verdict::Solid {
            open.push(serde_json::json!([
                id.data().as_ffi().to_string(),
                api::health::verdict_kind(verdict),
                doc.is_accepted_surface(id),
            ]));
        }
    }
    serde_json::json!({ "counts": counts_json(&counts), "open": open }).to_string()
}

/// The rims of `ids` as world-space line segments (`[x0,y0,z0, x1,y1,z1]`
/// per boundary edge), at every placement of each object — a shared
/// definition's rim is drawn wherever the definition is. `only` narrows to
/// one rim of one object (the panel's current finding).
pub(crate) fn rim_segments(
    doc: &Document,
    ids: &[ObjectId],
    only: Option<(ObjectId, usize)>,
) -> Vec<f32> {
    let placed = placements(doc);
    let mut out = Vec::new();
    for &id in ids {
        if only.is_some_and(|(o, _)| o != id) {
            continue;
        }
        let Some(obj) = doc.object(id) else { continue };
        let d = kernel::health::diagnose(obj);
        let he = obj.half_edges();
        let pos = |h: kernel::HalfEdgeId| obj.vertices()[he[h].origin].position;
        let poses = poses(doc, &placed, id);
        for (ri, rim) in d.rims.iter().enumerate() {
            if only.is_some_and(|(_, r)| r != ri) {
                continue;
            }
            for &h in &rim.half_edges {
                let (a, b) = (pos(h), pos(he[h].next));
                for t in &poses {
                    push(&mut out, t.apply_point(a));
                    push(&mut out, t.apply_point(b));
                }
            }
        }
    }
    out
}

fn push(out: &mut Vec<f32>, p: Point3) {
    out.extend([p.x as f32, p.y as f32, p.z as f32]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use kernel::{DefRecipe, ImportNode, ImportScene, MeshRecipe, Vec3};

    fn open_tray(x0: f64) -> MeshRecipe {
        let p = |x: f64, y: f64, z: f64| Point3::new(x0 + x, y, z);
        let positions = vec![
            p(0.0, 0.0, 0.0),
            p(1.0, 0.0, 0.0),
            p(1.0, 1.0, 0.0),
            p(0.0, 1.0, 0.0),
            p(0.0, 0.0, 1.0),
            p(1.0, 0.0, 1.0),
            p(1.0, 1.0, 1.0),
            p(0.0, 1.0, 1.0),
        ];
        // No top: the rim is the square at z = 1.
        let faces = vec![
            vec![0, 3, 2, 1],
            vec![0, 1, 5, 4],
            vec![1, 2, 6, 5],
            vec![2, 3, 7, 6],
            vec![3, 0, 4, 7],
        ];
        let n = faces.len();
        MeshRecipe {
            name: "Tray".into(),
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

    fn doc_with(roots: Vec<ImportNode>, defs: Vec<DefRecipe>) -> Document {
        let mut doc = Document::new();
        doc.ingest(
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
            },
            Vec::new(),
        )
        .unwrap();
        doc
    }

    #[test]
    fn the_report_names_each_open_object_its_verdict_and_rims() {
        let doc = doc_with(vec![ImportNode::Mesh(open_tray(0.0))], Vec::new());
        let v: serde_json::Value = serde_json::from_str(&report_json(&doc, None)).unwrap();
        let row = &v["rows"][0];
        assert_eq!(row["verdict"]["kind"], "cappable");
        assert_eq!(row["name"], "Tray");
        assert_eq!(row["placements"], 1);
        assert_eq!(row["rims"][0]["role"], "outer");
        assert_eq!(row["rims"][0]["planar"], true);
        assert_eq!(row["rims"][0]["edges"], 4);
        assert_eq!(v["counts"]["cappable"], 1);
        // The id round-trips as the handle the app uses.
        let id: u64 = row["object"].as_str().unwrap().parse().unwrap();
        assert_eq!(id, doc.live_object_ids()[0].data().as_ffi());
    }

    #[test]
    fn the_summary_counts_what_the_report_counts_and_reuses_its_cache() {
        let doc = doc_with(
            vec![
                ImportNode::Mesh(open_tray(0.0)),
                ImportNode::Mesh(open_tray(3.0)),
            ],
            Vec::new(),
        );
        let mut cache = BTreeMap::new();
        let v: serde_json::Value = serde_json::from_str(&summary_json(&doc, &mut cache)).unwrap();
        let full: serde_json::Value = serde_json::from_str(&report_json(&doc, None)).unwrap();
        assert_eq!(v["counts"]["cappable"], full["counts"]["cappable"]);
        assert_eq!(v["counts"]["objects"], 2);
        assert_eq!(v["open"].as_array().unwrap().len(), 2);
        assert_eq!(v["open"][0][1], "cappable");
        assert_eq!(v["open"][0][2], false);
        assert_eq!(cache.len(), 2);
        // A cached verdict is what the summary reports until it is dropped.
        let first = doc.live_object_ids()[0];
        cache.insert(first, Verdict::Solid);
        let v: serde_json::Value = serde_json::from_str(&summary_json(&doc, &mut cache)).unwrap();
        assert_eq!(v["counts"]["solids"], 1);
    }

    #[test]
    fn a_members_bounds_are_its_placement_under_one_instance_not_the_whole_set() {
        let place = |x: f64| ImportNode::Instance {
            def: 0,
            pose: Transform::translation(Vec3::new(x, 0.0, 0.0)),
            name: None,
            tags: Vec::new(),
            hidden: false,
            key: None,
        };
        let doc = doc_with(
            vec![place(0.0), place(10.0)],
            vec![DefRecipe {
                name: Some("Tray".into()),
                children: vec![ImportNode::Mesh(open_tray(0.0))],
            }],
        );
        let member = doc.live_object_ids()[0];
        let second = doc.instance_ids()[1];
        let one = world_bounds(&doc, &[(member, Some(second))]);
        let all = world_bounds(&doc, &[(member, None)]);
        assert_eq!(one.len(), 6);
        assert!(one[0] >= 10.0 - 1e-9, "the second placement only: {one:?}");
        assert!(all[0] <= 1e-9 && all[3] >= 10.0, "every placement: {all:?}");
        let first = doc.instance_ids()[0];
        let both = world_bounds(&doc, &[(member, Some(first)), (member, Some(second))]);
        assert_eq!(both, all, "a batch is the union of its targets");
        assert!(world_bounds(&doc, &[]).is_empty());
    }

    #[test]
    fn a_definitions_rim_is_drawn_at_every_placement() {
        let place = |x: f64| ImportNode::Instance {
            def: 0,
            pose: Transform::translation(Vec3::new(x, 0.0, 0.0)),
            name: None,
            tags: Vec::new(),
            hidden: false,
            key: None,
        };
        let doc = doc_with(
            vec![place(0.0), place(10.0)],
            vec![DefRecipe {
                name: Some("Tray".into()),
                children: vec![ImportNode::Mesh(open_tray(0.0))],
            }],
        );
        let ids = doc.live_object_ids();
        let segs = rim_segments(&doc, &ids, None);
        // 4 rim edges × 2 placements × 2 points × 3 floats.
        assert_eq!(segs.len(), 4 * 2 * 2 * 3);
        let xs: Vec<f32> = segs.chunks(3).map(|p| p[0]).collect();
        assert!(
            xs.iter().any(|&x| x >= 10.0),
            "the second placement is moved"
        );
        assert!(
            segs.chunks(3).all(|p| (p[2] - 1.0).abs() < 1e-6),
            "the rim is the open top"
        );
        // The report lists the definition once, with both placements and
        // the instances that put it in the model.
        let v: serde_json::Value = serde_json::from_str(&report_json(&doc, None)).unwrap();
        assert_eq!(v["rows"].as_array().unwrap().len(), 1);
        assert_eq!(v["rows"][0]["placements"], 2);
        assert_eq!(v["rows"][0]["instances"].as_array().unwrap().len(), 2);
        assert_eq!(v["rows"][0]["definition_name"], "Tray");
        // One rim of one object narrows to that rim.
        let one = rim_segments(&doc, &ids, Some((ids[0], 0)));
        assert_eq!(one.len(), segs.len());
        assert!(rim_segments(&doc, &ids, Some((ids[0], 1))).is_empty());
    }
}

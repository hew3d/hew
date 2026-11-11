//! Model health as JSON (`kernel::health`; the import-health design §7):
//! one serializer for the app's Model Health panel (wasm handles), the
//! `hew.doc.health` command (public ids), and every import report's counts,
//! so they can never disagree about what an object is.

use std::collections::BTreeMap;

use kernel::health::{Finding, HealthCounts, RimRole, Verdict};
use kernel::{Document, EntityRef, InstanceId, ObjectId};
use serde_json::{Value, json};

/// A verdict's wire name.
pub fn verdict_kind(v: Verdict) -> &'static str {
    match v {
        Verdict::Solid => "solid",
        Verdict::FlatSheet { .. } => "flat_sheet",
        Verdict::BentSheet => "bent_sheet",
        Verdict::Cappable => "cappable",
        Verdict::Holes { .. } => "holes",
        Verdict::Mixed => "mixed",
        Verdict::NonManifold => "non_manifold",
    }
}

/// How much a verdict asks of a person, for summarizing several objects
/// (an instance's members) as the most serious one: solid, then the two
/// surfaces, then the open kinds by how hard they are to close.
pub fn verdict_rank(v: Verdict) -> u8 {
    match v {
        Verdict::Solid => 0,
        Verdict::FlatSheet { .. } => 1,
        Verdict::BentSheet => 2,
        Verdict::Cappable => 3,
        Verdict::Holes { .. } => 4,
        Verdict::Mixed => 5,
        Verdict::NonManifold => 6,
    }
}

/// A verdict's wire name and its numbers.
pub fn verdict_json(v: Verdict) -> Value {
    match v {
        Verdict::FlatSheet { holes } => json!({ "kind": "flat_sheet", "holes": holes }),
        Verdict::Holes { planar, warped } => {
            json!({ "kind": "holes", "planar": planar, "warped": warped })
        }
        other => json!({ "kind": verdict_kind(other) }),
    }
}

/// An import's objects by kind ([`HealthCounts`]) as every host's import
/// report carries them: solids, the two kinds of surface, the open kinds
/// that need attention, and the importer's seam families.
pub fn counts_json(c: &HealthCounts) -> Value {
    json!({
        "objects": c.objects,
        "solids": c.solids,
        "flat_sheets": c.flat_sheets,
        "bent_sheets": c.bent_sheets,
        "cappable": c.cappable,
        "holes": c.holes,
        "mixed": c.mixed,
        "non_manifold": c.non_manifold,
        "seam_families": c.seam_families,
    })
}

/// The open objects of `scope` (every live object when `None`) with their
/// verdicts, rims and seam families. `id` names an object, instance or
/// component on the wire — the app passes FFI handles, the API public ids.
/// A definition member is one row however often it is placed; its
/// `instances` are the outermost instances that put it in the model.
pub fn report_json(
    doc: &Document,
    scope: Option<&[ObjectId]>,
    id: &dyn Fn(EntityRef) -> String,
) -> Value {
    let report = doc.health_report(scope);
    let mut placed: BTreeMap<ObjectId, Vec<InstanceId>> = BTreeMap::new();
    for (o, _, iid) in doc.expanded_placements() {
        placed.entry(o).or_default().push(iid);
    }
    let rows: Vec<Value> = report
        .rows
        .iter()
        .map(|row| {
            let d = &row.diagnosis;
            let mut instances: Vec<InstanceId> = placed.get(&row.object).cloned().unwrap_or_default();
            instances.dedup();
            let branching = d
                .findings
                .iter()
                .filter(|f| matches!(f, Finding::BranchingBoundaryVertex { .. }))
                .count();
            let islands = d
                .findings
                .iter()
                .find_map(|f| match f {
                    Finding::DisconnectedIslands { count } => Some(*count),
                    _ => None,
                })
                .unwrap_or(1);
            json!({
                "object": id(EntityRef::Object(row.object)),
                "name": doc.object_name(row.object),
                "definition": row.definition.map(|c| id(EntityRef::Component(c))),
                "definition_name": row.definition.and_then(|c| doc.component_name(c)),
                "placements": row.placements,
                "instances": instances.into_iter().map(|i| id(EntityRef::Instance(i))).collect::<Vec<_>>(),
                "accepted": row.accepted_surface,
                "family": row.seam_family,
                "verdict": verdict_json(d.verdict),
                "rims": d.rims.iter().enumerate().map(|(i, r)| json!({
                    "role": match r.role { RimRole::Outer => "outer", RimRole::Hole => "hole" },
                    "planar": r.plane.is_some(),
                    "cappable": doc
                        .object(row.object)
                        .is_some_and(|o| kernel::repair::rim_is_cappable(o, d, i)),
                    "closed": r.closed,
                    "sheet_edge": r.sheet_edge,
                    "shared": row.shared_rims.contains(&i),
                    "perimeter": r.perimeter,
                    "extent": r.extent,
                    "edges": r.half_edges.len(),
                })).collect::<Vec<_>>(),
                "branching_vertices": branching,
                "islands": islands,
                "faces": doc.object(row.object).map_or(0, |o| o.faces().len()),
            })
        })
        .collect();
    let families: Vec<Value> = report
        .families
        .iter()
        .map(|f| {
            json!({
                "members": f.members.iter().map(|&o| id(EntityRef::Object(o))).collect::<Vec<_>>(),
                "shared_rims": f.shared_rims,
            })
        })
        .collect();
    json!({
        "rows": rows,
        "families": families,
        "counts": counts_json(&report.counts()),
    })
}

// ───────────────────────────────────────────────────────────── Mend

/// A Mend plan from its wire form (`hew.solid.mend`'s params and the
/// app's `Scene::mend` JSON share it): `weld` a distance in meters, or
/// `false`/`null` for none, omitted for the object's default distance;
/// `folds` and `split` booleans (omitted: on); `fill` `"holes"`, an array
/// of rim indices, or `false`/`null` (omitted: `"holes"`); `require_solid`
/// (omitted: off). Omitted fields give the one-click Mend.
pub fn mend_plan_from_json(
    v: &Value,
    obj: &kernel::Object,
) -> Result<kernel::repair::MendPlan, String> {
    use kernel::repair::{FillScope, MendPlan, default_weld_distance};
    let o = v
        .as_object()
        .ok_or_else(|| "a Mend plan is a JSON object".to_string())?;
    for k in o.keys() {
        if !matches!(
            k.as_str(),
            "object" | "weld" | "folds" | "split" | "fill" | "require_solid"
        ) {
            return Err(format!("unknown Mend field '{k}'"));
        }
    }
    let flag = |k: &str, default: bool| -> Result<bool, String> {
        match o.get(k) {
            None => Ok(default),
            Some(Value::Bool(b)) => Ok(*b),
            Some(_) => Err(format!("'{k}' must be true or false")),
        }
    };
    let weld = match o.get("weld") {
        None => Some(default_weld_distance(obj)),
        Some(Value::Null) | Some(Value::Bool(false)) => None,
        Some(Value::Bool(true)) => Some(default_weld_distance(obj)),
        Some(Value::Number(n)) => Some(n.as_f64().ok_or("'weld' must be a number")?),
        Some(_) => return Err("'weld' must be a distance, true or false".into()),
    };
    let fill = match o.get("fill") {
        None => Some(FillScope::Holes),
        Some(Value::Null) | Some(Value::Bool(false)) => None,
        Some(Value::String(s)) if s == "holes" => Some(FillScope::Holes),
        Some(Value::Array(a)) => Some(FillScope::Rims(
            a.iter()
                .map(|x| {
                    // Past usize (wasm's is 32 bits) is past every rim:
                    // refused here, never wrapped onto another rim.
                    x.as_u64()
                        .and_then(|i| usize::try_from(i).ok())
                        .ok_or_else(|| {
                            "'fill' rims must be indices of the object's rims".to_string()
                        })
                })
                .collect::<Result<_, _>>()?,
        )),
        Some(_) => return Err("'fill' must be \"holes\", a list of rims, or false".into()),
    };
    Ok(MendPlan {
        weld,
        folds: flag("folds", true)?,
        split: flag("split", true)?,
        fill,
        require_solid: flag("require_solid", false)?,
    })
}

/// What a Mend did, on the wire; `pieces` are the split-off objects' ids
/// as the caller names them.
pub fn mend_report_json(r: &kernel::repair::MendReport, pieces: Vec<String>) -> Value {
    json!({
        "welded_vertices": r.welded_vertices,
        "weld_max_move": r.weld_max_move,
        "collapsed_faces": r.collapsed_faces,
        "folds_removed": r.folds_removed,
        "seam_folds": r.seam_folds,
        "soft_seams": r.soft_seams,
        "split_vertices": r.split_vertices,
        "split_edges": r.split_edges,
        "pieces": pieces,
        "filled_rims": r.filled_rims,
        "stitched_rims": r.stitched_rims,
        "fill_triangles": r.fill_triangles,
        "skipped_rims": r.skipped_rims.iter().map(|s| json!({
            "object": s.object,
            "rim": s.rim,
            "reason": crate::refusal::snake_case(&format!("{:?}", s.reason)),
        })).collect::<Vec<_>>(),
        "reversed": r.reversed,
        "closed": r.closed,
        "pieces_closed": r.pieces_closed,
    })
}

/// A Mend preview's counts on the wire (the highlight points travel
/// separately).
pub fn mend_census_json(c: &kernel::repair::MendCensus, weld: Option<f64>) -> Value {
    json!({
        "weld": weld,
        "weld_pairs": c.weld_pairs,
        "weld_vertices": c.weld_vertices,
        "weld_max_gap": c.weld_max_gap,
        "folds": c.folds,
        "collapsed_faces": c.collapsed_faces,
        "split_edges": c.split_edges,
        "branching_vertices": c.branching_vertices,
        "islands": c.islands,
        "pieces": c.pieces,
        "hole_rims": c.hole_rims,
        "hole_rims_planar": c.hole_rims_planar,
        "hole_rims_warped": c.hole_rims_warped,
        "outer_rims": c.outer_rims,
        "fill_rims": c.fill_rims,
        "filled_rims": c.filled_rims,
        "fill_curved": c.fill_curved,
        "skipped_rims": c.skipped_rims,
    })
}

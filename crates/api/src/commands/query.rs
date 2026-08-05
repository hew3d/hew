//! The read surface: hew.query.*, hew.meta.documents (docs/HEW_API.md §7). Implemented by its wave; see
//! docs/design/api-implementation-conventions.md.

use super::{CmdError, Ctx, Handler};
use crate::geom;
use crate::ids;
use crate::locate;
use crate::refusal::Refusal;
use kernel::{
    EntityRef, FaceId, Guide, NodeId, Point3, SketchCurveKind, SurfaceRef, Vec3, WatertightState,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeSet;

/// This namespace's slice of the handler table.
pub fn handler(name: &str) -> Option<Handler> {
    match name {
        "hew.meta.documents" => Some(documents),
        "hew.query.scene" => Some(scene),
        "hew.query.entity" => Some(entity),
        "hew.query.faces" => Some(faces),
        "hew.query.raycast" => Some(raycast),
        "hew.query.measure" => Some(measure),
        "hew.query.resolve" => Some(resolve),
        "hew.query.context" => Some(context),
        _ => None,
    }
}

// ------------------------------------------------------------------ params

fn parse<T: serde::de::DeserializeOwned>(params: &Value) -> Result<T, CmdError> {
    serde_json::from_value(params.clone()).map_err(|e| CmdError::Params(e.to_string()))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EmptyParams {}

// -------------------------------------------------------------------- utils

fn unknown_entity(id: &str) -> CmdError {
    CmdError::Refusal(
        Refusal::api(
            "unknown_entity",
            &format!("'{id}' does not name a live entity of the required kind in this document."),
        )
        .with_detail(json!({ "id": id })),
    )
}

fn point_json(p: Point3) -> Value {
    json!([p.x, p.y, p.z])
}

fn vec3_json(v: Vec3) -> Value {
    json!([v.x, v.y, v.z])
}

fn plane_json(plane: &kernel::Plane) -> Value {
    json!({ "normal": vec3_json(plane.normal()), "point": point_json(plane.point()) })
}

fn bbox_json((min, max): (Point3, Point3)) -> Value {
    json!({ "min": point_json(min), "max": point_json(max) })
}

/// This entity's public id, from the document's own stable-id table
/// (docs/HEW_API.md §5.1). `None` only for an entity the document itself
/// never assigned one to, which does not happen for anything this module
/// enumerates from the document.
fn public_of(ctx: &Ctx, entity: &EntityRef) -> Option<String> {
    ctx.doc
        .sid_of(entity)
        .map(|sid| ids::public_id(entity, sid))
}

fn public_of_or_internal(ctx: &Ctx, entity: &EntityRef) -> Result<String, CmdError> {
    public_of(ctx, entity).ok_or_else(|| CmdError::Internal("entity carries no stable id".into()))
}

/// An instance's watertightness, resolved through its definition
/// (docs/HEW_API.md §7): watertight only when every one of the
/// definition's VISIBLE member objects is watertight, `false` if any is
/// leaky, `None` when the definition itself cannot be resolved (a
/// dangling instance, which the scene walk and `hew.query.entity` never
/// surface in practice — both already require `def` to resolve before
/// reaching this) or when no member is visible. Hidden members are
/// excluded, not counted as leaky: an undone in-definition creation
/// tombstones its member as hidden while leaving it listed in
/// `ComponentDef.members` (hide-not-delete), and such a member
/// contributes nothing to what the instance renders.
fn instance_watertight(ctx: &Ctx, def: Option<kernel::ComponentId>) -> Option<bool> {
    let members = def.and_then(|d| ctx.doc.def_members(d))?;
    let visible: Vec<_> = members
        .iter()
        .copied()
        .filter(|&m| ctx.doc.object(m).is_some())
        .collect();
    if visible.is_empty() {
        return None;
    }
    Some(visible.iter().all(|&m| ctx.doc.object_solid(m)))
}

/// A tree node as `{id, kind}` — used for shallow member lists (group /
/// component summaries) that do not need a full recursive `NodeSummary`.
fn node_ref_json(ctx: &Ctx, node: NodeId) -> Result<Value, CmdError> {
    let (entity, kind) = match node {
        NodeId::Object(o) => (EntityRef::Object(o), "object"),
        NodeId::Group(g) => (EntityRef::Group(g), "group"),
        NodeId::Instance(i) => (EntityRef::Instance(i), "instance"),
    };
    let id = public_of_or_internal(ctx, &entity)?;
    Ok(json!({ "id": id, "kind": kind }))
}

// ------------------------------------------------------------ hew.meta.documents

fn documents(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let _: EmptyParams = parse(params)?;
    Ok(json!({ "documents": ctx.host.documents() }))
}

// --------------------------------------------------------------- hew.query.scene

/// One tree node — recursive for groups (docs/HEW_API.md §7).
fn node_summary(ctx: &Ctx, node: NodeId) -> Result<Value, CmdError> {
    match node {
        NodeId::Object(id) => {
            let entity = EntityRef::Object(id);
            let obj = ctx
                .doc
                .object(id)
                .ok_or_else(|| CmdError::Internal("dangling object in tree walk".into()))?;
            Ok(json!({
                "id": public_of_or_internal(ctx, &entity)?,
                "kind": "object",
                "name": ctx.doc.object_name(id),
                "watertight": matches!(obj.watertight(), WatertightState::Watertight),
                "bbox": locate::entity_bbox(ctx, &entity).map(bbox_json),
                "tags": ctx.doc.node_tags(node),
            }))
        }
        NodeId::Group(id) => {
            let entity = EntityRef::Group(id);
            let members = ctx.doc.group_members(id).unwrap_or_default();
            let mut kids = Vec::with_capacity(members.len());
            for m in members {
                kids.push(node_summary(ctx, m)?);
            }
            Ok(json!({
                "id": public_of_or_internal(ctx, &entity)?,
                "kind": "group",
                "name": ctx.doc.group_name(id),
                "watertight": Value::Null,
                "bbox": locate::entity_bbox(ctx, &entity).map(bbox_json),
                "tags": ctx.doc.node_tags(node),
                "members": kids,
            }))
        }
        NodeId::Instance(id) => {
            let entity = EntityRef::Instance(id);
            let def = ctx.doc.instance_def(id);
            let name = ctx.doc.instance_name(id).map(str::to_string).or_else(|| {
                def.and_then(|d| ctx.doc.component_name(d))
                    .map(str::to_string)
            });
            let def_id = match def {
                Some(d) => Some(public_of_or_internal(ctx, &EntityRef::Component(d))?),
                None => None,
            };
            Ok(json!({
                "id": public_of_or_internal(ctx, &entity)?,
                "kind": "instance",
                "name": name,
                "watertight": instance_watertight(ctx, def),
                "bbox": locate::entity_bbox(ctx, &entity).map(bbox_json),
                "tags": ctx.doc.node_tags(node),
                "def": def_id,
            }))
        }
    }
}

fn sketch_summary(ctx: &Ctx, sketch_id: kernel::SketchId) -> Result<Value, CmdError> {
    let sk = ctx
        .doc
        .sketch(sketch_id)
        .ok_or_else(|| CmdError::Internal("dangling sketch in scene walk".into()))?;
    let sketch_sid = ctx
        .doc
        .sid_of(&EntityRef::Sketch(sketch_id))
        .ok_or_else(|| CmdError::Internal("sketch carries no stable id".into()))?;

    let regions: Vec<Value> = sk
        .regions()
        .keys()
        .map(|rid| {
            json!({
                "id": ids::region_id(sketch_sid, rid),
                "area": sk.region_area(rid).ok(),
            })
        })
        .collect();

    let mut curve_ids: BTreeSet<kernel::SketchCurveId> = BTreeSet::new();
    for (_, edge) in sk.edges() {
        if let Some(c) = edge.curve {
            curve_ids.insert(c);
        }
    }
    let curves: Vec<Value> = curve_ids
        .into_iter()
        .map(|cid| {
            let analytic = sk.curve_analytic(cid);
            json!({
                "id": ids::curve_id(sketch_sid, cid),
                "kind": analytic.map(|a| match a.kind {
                    SketchCurveKind::Circle => "circle",
                    SketchCurveKind::Polygon => "polygon",
                }),
                "center": analytic.map(|a| point_json(a.geom.center)),
                "radius": analytic.map(|a| a.geom.radius),
            })
        })
        .collect();

    // Edges — the scaffolding a client needs to trim a sketch after
    // drawing it (docs/HEW_API.md §5.2): enough to tell two candidates
    // apart and pick one (id + endpoints), plus the owning curve id when
    // the edge is one curve's facet (a circle's or arc's, so erasing "the
    // circle" versus "one facet" is the client's own informed choice).
    let edges: Vec<Value> = sketch_edges_json(sk, sketch_sid);

    Ok(json!({
        "id": public_of_or_internal(ctx, &EntityRef::Sketch(sketch_id))?,
        "plane": plane_json(&sk.plane()),
        "regions": regions,
        "curves": curves,
        "edges": edges,
    }))
}

/// One sketch's edges as `{id, from, to, curve}` — the shape
/// [`sketch_summary`] lists every edge in, and [`entity`]'s own edge
/// branch reuses (filtered to the one addressed edge) so both surfaces
/// agree byte-for-byte on what an edge looks like.
fn sketch_edges_json(sk: &kernel::Sketch, sketch_sid: u64) -> Vec<Value> {
    sk.edges()
        .iter()
        .map(|(eid, e)| {
            let a = sk.vertices()[e.from].position;
            let b = sk.vertices()[e.to].position;
            json!({
                "id": ids::edge_id(sketch_sid, eid),
                "from": point_json(a),
                "to": point_json(b),
                "curve": e.curve.map(|c| ids::curve_id(sketch_sid, c)),
            })
        })
        .collect()
}

fn guide_summary(ctx: &Ctx, id: kernel::GuideId) -> Result<Value, CmdError> {
    let g = *ctx
        .doc
        .guide(id)
        .ok_or_else(|| CmdError::Internal("dangling guide in scene walk".into()))?;
    let pub_id = public_of_or_internal(ctx, &EntityRef::Guide(id))?;
    Ok(match g {
        Guide::Line { origin, direction } => json!({
            "id": pub_id, "kind": "line",
            "origin": point_json(origin), "direction": vec3_json(direction),
        }),
        Guide::Point { position } => json!({
            "id": pub_id, "kind": "point",
            "position": point_json(position),
        }),
    })
}

fn material_summary(ctx: &Ctx, id: kernel::MaterialId) -> Result<Value, CmdError> {
    let m = ctx
        .doc
        .material(id)
        .ok_or_else(|| CmdError::Internal("dangling material in scene walk".into()))?;
    Ok(json!({
        "id": public_of_or_internal(ctx, &EntityRef::Material(id))?,
        "name": m.name,
        "color": { "r": m.color.r, "g": m.color.g, "b": m.color.b, "a": m.color.a },
        "has_texture": m.texture.is_some(),
    }))
}

fn tag_summary(ctx: &Ctx, path: &[String]) -> Result<Value, CmdError> {
    Ok(json!({
        "id": public_of_or_internal(ctx, &EntityRef::Tag(path.to_vec()))?,
        "path": path,
        "hidden": ctx.doc.tag_hidden(path),
    }))
}

fn component_summary(ctx: &Ctx, id: kernel::ComponentId) -> Result<Value, CmdError> {
    let instance_count = ctx.doc.instances_of(id).len();
    Ok(json!({
        "id": public_of_or_internal(ctx, &EntityRef::Component(id))?,
        "name": ctx.doc.component_name(id),
        "instance_count": instance_count,
    }))
}

fn scene(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let _: EmptyParams = parse(params)?;

    let mut tree = Vec::new();
    for node in ctx.doc.top_level_nodes() {
        tree.push(node_summary(ctx, node)?);
    }

    let mut sketches = Vec::new();
    for id in ctx.doc.sketch_ids() {
        sketches.push(sketch_summary(ctx, id)?);
    }

    let mut guides = Vec::new();
    for id in ctx.doc.guide_ids() {
        guides.push(guide_summary(ctx, id)?);
    }

    let mut materials = Vec::new();
    for id in ctx.doc.material_ids() {
        materials.push(material_summary(ctx, id)?);
    }

    let tag_paths: Vec<Vec<String>> = ctx.doc.tag_meta().map(|(p, _)| p.to_vec()).collect();
    let mut tags = Vec::new();
    for path in &tag_paths {
        tags.push(tag_summary(ctx, path)?);
    }

    let mut components = Vec::new();
    for id in ctx.doc.component_ids() {
        components.push(component_summary(ctx, id)?);
    }

    Ok(json!({
        "document": {
            "objects": ctx.doc.visible_object_ids().len(),
            "groups": ctx.doc.group_ids().len(),
            "instances": ctx.doc.instance_ids().len(),
            "components": ctx.doc.component_ids().len(),
            "sketches": ctx.doc.sketch_ids().len(),
            "guides": ctx.doc.guide_ids().len(),
            "materials": ctx.doc.material_ids().len(),
        },
        "tree": tree,
        "sketches": sketches,
        "guides": guides,
        "materials": materials,
        "tags": tags,
        "components": components,
    }))
}

// -------------------------------------------------------------- hew.query.entity

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EntityParams {
    id: String,
}

fn entity(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let p: EntityParams = parse(params)?;
    let resolver = ctx.resolver();
    let entity = resolver.resolve(&p.id);
    // A sketch edge is not a top-level `EntityRef` kind — it is a
    // compound id, resolved the same way `hew.sketch.offset`/
    // `hew.solid.extrude` resolve a region id (`IdResolver::resolve_edge`)
    // rather than through `IdResolver::resolve`. A client that just
    // queried or minted an edge id will reasonably try querying it right
    // back (docs/HEW_API.md §5.2), so this is handled here rather than
    // left to fall through as `unknown_entity`.
    let edge = entity
        .is_none()
        .then(|| resolver.resolve_edge(&p.id))
        .flatten();
    let entity = match (entity, edge) {
        (Some(e), _) => e,
        (None, Some((sketch, edge))) => return edge_entity(ctx, &p.id, sketch, edge),
        (None, None) => return Err(unknown_entity(&p.id)),
    };

    match entity {
        EntityRef::Object(oid) => {
            let obj = ctx.doc.object(oid).ok_or_else(|| unknown_entity(&p.id))?;
            let bbox = locate::entity_bbox(ctx, &EntityRef::Object(oid)).map(bbox_json);
            let material_default = obj
                .default_material()
                .and_then(|m| public_of(ctx, &EntityRef::Material(m)));
            Ok(json!({
                "kind": "object",
                "id": p.id,
                "name": ctx.doc.object_name(oid),
                "watertight": matches!(obj.watertight(), WatertightState::Watertight),
                "bbox": bbox,
                "face_count": obj.faces().len(),
                "tags": ctx.doc.node_tags(NodeId::Object(oid)),
                "material_default": material_default,
            }))
        }
        EntityRef::Sketch(sid) => {
            ctx.doc.sketch(sid).ok_or_else(|| unknown_entity(&p.id))?;
            let mut summary = sketch_summary(ctx, sid)?;
            summary["kind"] = json!("sketch");
            Ok(summary)
        }
        EntityRef::Group(gid) => {
            let members = ctx
                .doc
                .group_members(gid)
                .ok_or_else(|| unknown_entity(&p.id))?;
            let mut member_json = Vec::with_capacity(members.len());
            for m in members {
                member_json.push(node_ref_json(ctx, m)?);
            }
            Ok(json!({
                "kind": "group",
                "id": p.id,
                "name": ctx.doc.group_name(gid),
                "members": member_json,
            }))
        }
        EntityRef::Component(cid) => {
            let members = ctx
                .doc
                .def_members(cid)
                .ok_or_else(|| unknown_entity(&p.id))?;
            let mut member_ids = Vec::with_capacity(members.len());
            // Tombstoned (hidden) members stay listed in
            // `ComponentDef.members` (hide-not-delete) but are not live
            // entities — publishing their ids would hand out handles this
            // same command immediately refuses as unknown_entity. Every
            // sibling surface (watertightness, bboxes, raycast, save)
            // already filters them.
            for o in members {
                if ctx.doc.object(o).is_none() {
                    continue;
                }
                member_ids.push(public_of_or_internal(ctx, &EntityRef::Object(o))?);
            }
            let mut instance_ids = Vec::new();
            for i in ctx.doc.instances_of(cid) {
                instance_ids.push(public_of_or_internal(ctx, &EntityRef::Instance(i))?);
            }
            Ok(json!({
                "kind": "component",
                "id": p.id,
                "name": ctx.doc.component_name(cid),
                "members": member_ids,
                "instances": instance_ids,
            }))
        }
        EntityRef::Instance(iid) => {
            let def = ctx
                .doc
                .instance_def(iid)
                .ok_or_else(|| unknown_entity(&p.id))?;
            let pose = ctx
                .doc
                .instance_pose(iid)
                .ok_or_else(|| unknown_entity(&p.id))?;
            let name = ctx
                .doc
                .instance_name(iid)
                .map(str::to_string)
                .or_else(|| ctx.doc.component_name(def).map(str::to_string));
            Ok(json!({
                "kind": "instance",
                "id": p.id,
                "def": public_of_or_internal(ctx, &EntityRef::Component(def))?,
                "pose": pose.to_affine().to_vec(),
                "name": name,
                "watertight": instance_watertight(ctx, Some(def)),
            }))
        }
        EntityRef::Guide(gid) => {
            let g = *ctx.doc.guide(gid).ok_or_else(|| unknown_entity(&p.id))?;
            Ok(match g {
                Guide::Line { origin, direction } => json!({
                    "kind": "guide", "id": p.id, "guide_kind": "line",
                    "origin": point_json(origin), "direction": vec3_json(direction),
                }),
                Guide::Point { position } => json!({
                    "kind": "guide", "id": p.id, "guide_kind": "point",
                    "position": point_json(position),
                }),
            })
        }
        EntityRef::Material(mid) => {
            let m = ctx.doc.material(mid).ok_or_else(|| unknown_entity(&p.id))?;
            Ok(json!({
                "kind": "material",
                "id": p.id,
                "name": m.name,
                "color": { "r": m.color.r, "g": m.color.g, "b": m.color.b, "a": m.color.a },
                "has_texture": m.texture.is_some(),
            }))
        }
        EntityRef::Tag(path) => {
            let live = ctx.doc.tag_meta().any(|(pp, _)| pp == path.as_slice());
            if !live {
                return Err(unknown_entity(&p.id));
            }
            Ok(json!({
                "kind": "tag",
                "id": p.id,
                "path": path,
                "hidden": ctx.doc.tag_hidden(&path),
            }))
        }
    }
}

/// `hew.query.entity` on a sketch edge's own id — a client that just
/// queried or minted an edge id (drawing a sketch, or `hew.query.resolve`
/// against a geometric locator) will reasonably try querying it right
/// back, so this is wired up rather than left to refuse
/// (docs/HEW_API.md §5.2). `edge`'s presence in `sk.edges()` is re-checked
/// here rather than assumed from the caller's successful id-parse: the id
/// resolver only validates the SKETCH half of a compound id (generational
/// keys — `IdResolver::resolve_edge`'s doc comment), so a stale edge key
/// surfaces here as the same typed `unknown_entity` any other dangling id
/// gets, never a lookup panic.
fn edge_entity(
    ctx: &Ctx,
    public_id: &str,
    sketch_id: kernel::SketchId,
    edge: kernel::SketchEdgeId,
) -> Result<Value, CmdError> {
    let sk = ctx
        .doc
        .sketch(sketch_id)
        .ok_or_else(|| unknown_entity(public_id))?;
    let e = sk
        .edges()
        .get(edge)
        .ok_or_else(|| unknown_entity(public_id))?;
    let a = sk.vertices()[e.from].position;
    let b = sk.vertices()[e.to].position;
    let sketch_sid = ctx
        .doc
        .sid_of(&EntityRef::Sketch(sketch_id))
        .ok_or_else(|| CmdError::Internal("sketch carries no stable id".into()))?;
    Ok(json!({
        "kind": "edge",
        "id": public_id,
        "sketch": public_of_or_internal(ctx, &EntityRef::Sketch(sketch_id))?,
        "from": point_json(a),
        "to": point_json(b),
        "length": (b - a).length(),
        "curve": e.curve.map(|c| ids::curve_id(sketch_sid, c)),
    }))
}

// --------------------------------------------------------------- hew.query.faces

/// A single face's description — no index/token, just its geometry
/// (docs/HEW_API.md §5.2's locators are for mutation; a query answers with
/// values). Shared by `hew.query.faces` (every face) and `hew.query.resolve`
/// (one face locator resolved to the same shape).
fn face_description(ctx: &Ctx, face: locate::FaceRef) -> Result<Value, CmdError> {
    let object = ctx
        .doc
        .object(face.object)
        .ok_or_else(|| CmdError::Internal("dangling object behind a resolved face".into()))?;
    let f = object
        .faces()
        .get(face.face)
        .ok_or_else(|| CmdError::Internal("dangling face behind a resolved locator".into()))?;
    let outer: Vec<Point3> = object.loop_positions(f.outer_loop).collect();
    let holes: Vec<Vec<Point3>> = f
        .inner_loops
        .iter()
        .map(|&l| object.loop_positions(l).collect())
        .collect();
    let material = f
        .material
        .and_then(|m| public_of(ctx, &EntityRef::Material(m)));
    let surface = f.surface.map(|s| {
        let SurfaceRef::Cylinder {
            axis_point,
            axis,
            radius,
        } = s;
        json!({ "axis_point": point_json(axis_point), "axis": vec3_json(axis), "radius": radius })
    });
    Ok(json!({
        "object": public_of_or_internal(ctx, &EntityRef::Object(face.object))?,
        "plane": plane_json(&f.plane),
        "area": geom::face_area(&outer, &holes),
        "centroid": point_json(geom::face_centroid(&outer, &holes)),
        "outer": outer.iter().copied().map(point_json).collect::<Vec<_>>(),
        "holes": holes
            .iter()
            .map(|h| h.iter().copied().map(point_json).collect::<Vec<_>>())
            .collect::<Vec<_>>(),
        "material": material,
        "surface": surface,
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FacesParams {
    object: String,
}

fn faces(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let p: FacesParams = parse(params)?;
    let Some(EntityRef::Object(oid)) = ctx.resolver().resolve(&p.object) else {
        return Err(unknown_entity(&p.object));
    };
    let face_ids: Vec<FaceId> = ctx
        .doc
        .object(oid)
        .ok_or_else(|| unknown_entity(&p.object))?
        .faces()
        .keys()
        .collect();
    let mut out = Vec::with_capacity(face_ids.len());
    for fid in face_ids {
        out.push(face_description(
            ctx,
            locate::FaceRef {
                object: oid,
                face: fid,
            },
        )?);
    }
    Ok(json!({ "object": p.object, "faces": out }))
}

// ------------------------------------------------------------- hew.query.raycast

fn locator_missed() -> CmdError {
    CmdError::Refusal(Refusal::api(
        "locator_missed",
        "The ray hit no visible face. Check the origin and direction against the scene's geometry.",
    ))
}

fn locator_ambiguous() -> CmdError {
    CmdError::Refusal(Refusal::api(
        "ambiguous_locator",
        "The ray hit more than one face at the same distance. Nudge the ray so exactly one face is nearest.",
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RaycastParams {
    origin: Value,
    dir: Value,
}

/// What a raycast candidate hit belongs to.
#[derive(Debug, Clone, Copy)]
enum RaycastHit {
    World(kernel::ObjectId),
    Instance(kernel::InstanceId),
}

/// `(world-space distance along the ray, what was hit, world hit point,
/// world-space face normal)`. Every candidate — world object or instance
/// member — is reduced to this shape before comparison so ties and
/// "nearest wins" are decided in one common, physically meaningful unit.
type RayCandidate = (f64, RaycastHit, Point3, Vec3);

/// Folds one more candidate into the running nearest/tie state. A closer
/// candidate replaces; a candidate within [`geom::API_AMBIGUITY_TOL`] of
/// the current best is a tie (refused, never guessed — §5.2); anything
/// farther is dropped.
fn consider_hit(best: &mut Option<RayCandidate>, tie: &mut bool, candidate: RayCandidate) {
    match best {
        None => {
            *best = Some(candidate);
            *tie = false;
        }
        Some((bt, ..)) if candidate.0 < *bt - geom::API_AMBIGUITY_TOL => {
            *best = Some(candidate);
            *tie = false;
        }
        Some((bt, ..)) if (candidate.0 - *bt).abs() <= geom::API_AMBIGUITY_TOL => {
            *tie = true;
        }
        _ => {}
    }
}

/// Whether `node`, or any group above it, is user-hidden (SketchUp
/// "Hide" — view state a raycast must respect just as rendering does).
/// `crates/api` cannot depend on `hew-cli`, so this is a local copy of
/// the same walk `hew-cli/src/host.rs`'s headless snapshot uses.
fn node_or_ancestor_hidden(doc: &kernel::Document, node: NodeId) -> bool {
    if doc.node_user_hidden(node) {
        return true;
    }
    let mut cursor = doc.node_parent(node);
    while let Some(group) = cursor {
        if doc.node_user_hidden(NodeId::Group(group)) {
            return true;
        }
        cursor = doc.node_parent(NodeId::Group(group));
    }
    false
}

fn raycast(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let p: RaycastParams = parse(params)?;
    let origin = locate::parse_xyz(&p.origin)?;
    let dir = locate::parse_dir(&p.dir)?;
    // A world-space unit direction to measure every candidate's distance
    // along, regardless of which frame (world, or a definition mapped
    // back through an instance's pose) it was actually intersected in —
    // a local-space parameter is meaningless to compare against a
    // world-space one when the instance's pose scales (docs/design/
    // api-kernel-map.md §1.5's Transform).
    let dir_n = dir
        .normalized()
        .map_err(|_| CmdError::Params("dir must be non-zero".into()))?;

    let mut best: Option<RayCandidate> = None;
    let mut tie = false;

    // Every visible WORLD object.
    for oid in ctx.doc.visible_object_ids() {
        // User-hidden objects (or ones inside a user-hidden group) fall
        // through to what is beneath — matching the instance loop below
        // and the headless snapshot's treatment.
        if node_or_ancestor_hidden(ctx.doc, kernel::NodeId::Object(oid)) {
            continue;
        }
        let Some(obj) = ctx.doc.object(oid) else {
            continue;
        };
        for (_fid, f) in obj.faces() {
            let Some(t) = geom::ray_plane_t(origin, dir, &f.plane) else {
                continue;
            };
            let hit = origin + dir * t;
            let outer: Vec<Point3> = obj.loop_positions(f.outer_loop).collect();
            let holes: Vec<Vec<Point3>> = f
                .inner_loops
                .iter()
                .map(|&l| obj.loop_positions(l).collect())
                .collect();
            if !geom::face_contains(&outer, &holes, f.plane.normal(), hit) {
                continue;
            }
            let distance = (hit - origin).dot(dir_n);
            consider_hit(
                &mut best,
                &mut tie,
                (distance, RaycastHit::World(oid), hit, f.plane.normal()),
            );
        }
    }

    // Every visible instance's definition members: the ray is mapped into
    // definition space by the instance's inverse pose, and any hit is
    // mapped back. Poses are invertible by construction (every
    // `Document` constructor either checks this or composes only
    // invertible poses); a non-invertible one skips that one instance
    // rather than refusing the whole query.
    for instance in ctx.doc.instance_ids() {
        if node_or_ancestor_hidden(ctx.doc, NodeId::Instance(instance)) {
            continue;
        }
        let Some(def) = ctx.doc.instance_def(instance) else {
            continue;
        };
        let Some(pose) = ctx.doc.instance_pose(instance) else {
            continue;
        };
        let Some(members) = ctx.doc.def_members(def) else {
            continue;
        };
        let Ok(inv) = pose.inverse() else {
            continue;
        };
        let local_origin = inv.apply_point(origin);
        let local_dir = inv.apply_vector(dir);
        for member in members {
            let Some(obj) = ctx.doc.object(member) else {
                continue;
            };
            for (_fid, f) in obj.faces() {
                let Some(t) = geom::ray_plane_t(local_origin, local_dir, &f.plane) else {
                    continue;
                };
                let hit_local = local_origin + local_dir * t;
                let outer: Vec<Point3> = obj.loop_positions(f.outer_loop).collect();
                let holes: Vec<Vec<Point3>> = f
                    .inner_loops
                    .iter()
                    .map(|&l| obj.loop_positions(l).collect())
                    .collect();
                if !geom::face_contains(&outer, &holes, f.plane.normal(), hit_local) {
                    continue;
                }
                let hit_world = pose.apply_point(hit_local);
                // World-space distance, NOT the definition-space `t`
                // above (§ comment on `dir_n`): (hit - origin)·dir_n.
                let distance = (hit_world - origin).dot(dir_n);
                // `pose` is already known invertible (checked above), and
                // `apply_plane` is singular under exactly the same
                // determinant gate `inverse` is — so this cannot fail.
                let normal_world = pose
                    .apply_plane(&f.plane)
                    .expect("pose invertible implies its plane map is too")
                    .normal();
                consider_hit(
                    &mut best,
                    &mut tie,
                    (
                        distance,
                        RaycastHit::Instance(instance),
                        hit_world,
                        normal_world,
                    ),
                );
            }
        }
    }

    match best {
        Some(_) if tie => Err(locator_ambiguous()),
        Some((distance, source, hit, normal)) => {
            let (object, kind) = match source {
                RaycastHit::World(oid) => (
                    public_of_or_internal(ctx, &EntityRef::Object(oid))?,
                    "object",
                ),
                RaycastHit::Instance(iid) => (
                    public_of_or_internal(ctx, &EntityRef::Instance(iid))?,
                    "instance",
                ),
            };
            Ok(json!({
                "object": object,
                "kind": kind,
                "point": point_json(hit),
                "distance": distance,
                "normal": vec3_json(normal),
            }))
        }
        None => Err(locator_missed()),
    }
}

// ------------------------------------------------------------- hew.query.measure

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MeasureParams {
    from: Value,
    to: Value,
}

fn measure(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let p: MeasureParams = parse(params)?;
    let from = locate::resolve_point(ctx, &p.from)?;
    let to = locate::resolve_point(ctx, &p.to)?;
    let delta = to - from;
    Ok(json!({
        "distance": delta.length(),
        "delta": vec3_json(delta),
    }))
}

// ------------------------------------------------------------- hew.query.resolve

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResolveParams {
    #[serde(default)]
    point: Option<Value>,
    #[serde(default)]
    face: Option<Value>,
    #[serde(default)]
    edge: Option<Value>,
}

fn resolve(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let p: ResolveParams = parse(params)?;
    let present = [p.point.is_some(), p.face.is_some(), p.edge.is_some()]
        .into_iter()
        .filter(|b| *b)
        .count();
    if present != 1 {
        return Err(CmdError::Params(
            "hew.query.resolve needs exactly one of \"point\", \"face\", \"edge\"".into(),
        ));
    }
    if let Some(v) = p.point {
        let point = locate::resolve_point(ctx, &v)?;
        return Ok(json!({ "point": point_json(point) }));
    }
    if let Some(v) = p.face {
        let face = locate::resolve_face(ctx, &v)?;
        return Ok(json!({ "face": face_description(ctx, face)? }));
    }
    let v = p.edge.expect("exactly one branch present");
    match locate::resolve_edge_locator(ctx, &v)? {
        locate::AnyEdge::Solid(edge) => Ok(json!({
            "edge": {
                "kind": "solid",
                "object": public_of_or_internal(ctx, &EntityRef::Object(edge.object))?,
                "from": point_json(edge.endpoints.0),
                "to": point_json(edge.endpoints.1),
            }
        })),
        locate::AnyEdge::Sketch(edge) => {
            let sketch_sid = ctx
                .doc
                .sid_of(&EntityRef::Sketch(edge.sketch))
                .ok_or_else(|| CmdError::Internal("sketch carries no stable id".into()))?;
            Ok(json!({
                "edge": {
                    "kind": "sketch",
                    "id": ids::edge_id(sketch_sid, edge.edge),
                    "sketch": public_of_or_internal(ctx, &EntityRef::Sketch(edge.sketch))?,
                    "from": point_json(edge.endpoints.0),
                    "to": point_json(edge.endpoints.1),
                    "curve": edge.curve.map(|c| ids::curve_id(sketch_sid, c)),
                }
            }))
        }
    }
}

// ------------------------------------------------------------- hew.query.context

fn context(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let _: EmptyParams = parse(params)?;

    let mut stack = Vec::new();
    for frame in ctx.doc.session_stack() {
        let (kind, entity) = match frame {
            NodeId::Group(g) => ("group", EntityRef::Group(g)),
            NodeId::Instance(i) => ("component", EntityRef::Instance(i)),
            NodeId::Object(_) => {
                return Err(CmdError::Internal(
                    "session frame collapsed to an object, which never opens a session".into(),
                ));
            }
        };
        stack.push(json!({ "kind": kind, "id": public_of_or_internal(ctx, &entity)? }));
    }

    let direct_members = match ctx.doc.session_direct_members() {
        Some(members) => {
            let mut out = Vec::with_capacity(members.len());
            for m in members {
                out.push(node_ref_json(ctx, m)?);
            }
            Some(out)
        }
        None => None,
    };

    Ok(json!({ "stack": stack, "direct_members": direct_members }))
}

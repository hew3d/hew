//! The read surface: hew.query.*, hew.meta.documents (docs/agents/HEW_API.md §7). Implemented by its wave; see
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

/// A query handler whose result also carries the document's clock
/// ([`super::with_clock`]): `generation` and `epoch` on every
/// `hew.query.*` result (docs/agents/HEW_API.md §7).
macro_rules! clocked {
    ($handler:ident) => {{
        fn clocked(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
            let result = $handler(ctx, params)?;
            Ok(super::with_clock(ctx.doc, result))
        }
        clocked as Handler
    }};
}

/// This namespace's slice of the handler table.
pub fn handler(name: &str) -> Option<Handler> {
    match name {
        "hew.meta.documents" => Some(documents),
        "hew.query.scene" => Some(clocked!(scene)),
        "hew.query.entity" => Some(clocked!(entity)),
        "hew.query.faces" => Some(clocked!(faces)),
        "hew.query.raycast" => Some(clocked!(raycast)),
        "hew.query.measure" => Some(clocked!(measure)),
        "hew.query.resolve" => Some(clocked!(resolve)),
        "hew.query.context" => Some(clocked!(context)),
        "hew.query.changes_since" => Some(clocked!(changes_since)),
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
/// (docs/agents/HEW_API.md §5.1). `None` only for an entity the document itself
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
/// (docs/agents/HEW_API.md §7): watertight only when every one of the
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
    let def = def?;
    ctx.doc.def_member_nodes(def)?;
    // Nested members included: every leaf placement's object counts.
    let visible: Vec<_> = ctx
        .doc
        .expanded_def_placements(def)
        .into_iter()
        .map(|(m, _)| m)
        .filter(|&m| ctx.doc.object(m).is_some())
        .collect();
    if visible.is_empty() {
        return None;
    }
    Some(visible.iter().all(|&m| ctx.doc.object_solid(m)))
}

/// An object's model-health verdict name (`solid`, `flat_sheet`,
/// `bent_sheet`, `cappable`, `holes`, `mixed`, `non_manifold` —
/// `hew.doc.health` has the detail). `watertight` keeps its meaning beside
/// it: a surface is not watertight, but it is not broken either.
fn object_health(obj: &kernel::Object) -> &'static str {
    crate::health::verdict_kind(kernel::health::diagnose(obj).verdict)
}

/// An instance's health: the most serious verdict among the definition's
/// visible members (nested ones included), `None` exactly when
/// [`instance_watertight`] is.
fn instance_health(ctx: &Ctx, def: Option<kernel::ComponentId>) -> Option<&'static str> {
    let def = def?;
    ctx.doc.def_member_nodes(def)?;
    ctx.doc
        .expanded_def_placements(def)
        .into_iter()
        .filter_map(|(m, _)| ctx.doc.object(m))
        .map(|o| kernel::health::diagnose(o).verdict)
        .max_by_key(|&v| crate::health::verdict_rank(v))
        .map(crate::health::verdict_kind)
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
    Ok(json!({ "documents": ctx.grant.filter_documents(ctx.host.documents(ctx.doc)) }))
}

// --------------------------------------------------------------- hew.query.scene

/// The members every tree node carries (docs/agents/HEW_API.md §7): view
/// state, its material (an object's base, an instance's own), and its
/// containing group.
fn node_common(ctx: &Ctx, node: NodeId) -> Result<serde_json::Map<String, Value>, CmdError> {
    let material = match node {
        NodeId::Object(id) => ctx.doc.object(id).and_then(|o| o.default_material()),
        NodeId::Instance(id) => ctx.doc.instance_material(id).flatten(),
        NodeId::Group(_) => None,
    }
    .and_then(|m| public_of(ctx, &EntityRef::Material(m)));
    let parent = match ctx.doc.node_parent(node) {
        Some(g) => Some(public_of_or_internal(ctx, &EntityRef::Group(g))?),
        None => None,
    };
    let mut m = serde_json::Map::new();
    m.insert("user_hidden".into(), json!(ctx.doc.node_user_hidden(node)));
    m.insert(
        "hidden".into(),
        json!(ctx.doc.node_effectively_hidden(node)),
    );
    m.insert("material".into(), json!(material));
    m.insert("parent".into(), json!(parent));
    Ok(m)
}

fn with_common(ctx: &Ctx, node: NodeId, mut v: Value) -> Result<Value, CmdError> {
    if let Some(map) = v.as_object_mut() {
        map.extend(node_common(ctx, node)?);
    }
    Ok(v)
}

/// One tree node — recursive for groups (docs/agents/HEW_API.md §7).
fn node_summary(ctx: &Ctx, node: NodeId) -> Result<Value, CmdError> {
    let v = match node {
        NodeId::Object(id) => {
            let entity = EntityRef::Object(id);
            let obj = ctx
                .doc
                .object(id)
                .ok_or_else(|| CmdError::Internal("dangling object in tree walk".into()))?;
            json!({
                "id": public_of_or_internal(ctx, &entity)?,
                "kind": "object",
                "name": ctx.doc.object_name(id),
                "watertight": matches!(obj.watertight(), WatertightState::Watertight),
                "health": object_health(obj),
                "bbox": locate::entity_bbox(ctx, &entity).map(bbox_json),
                "tags": ctx.doc.node_tags(node),
            })
        }
        NodeId::Group(id) => {
            let entity = EntityRef::Group(id);
            let members = ctx.doc.group_members(id).unwrap_or_default();
            let mut kids = Vec::with_capacity(members.len());
            for m in members {
                kids.push(node_summary(ctx, m)?);
            }
            json!({
                "id": public_of_or_internal(ctx, &entity)?,
                "kind": "group",
                "name": ctx.doc.group_name(id),
                "watertight": Value::Null,
                "health": Value::Null,
                "bbox": locate::entity_bbox(ctx, &entity).map(bbox_json),
                "tags": ctx.doc.node_tags(node),
                "members": kids,
            })
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
            let pose = ctx.doc.instance_pose(id);
            json!({
                "id": public_of_or_internal(ctx, &entity)?,
                "kind": "instance",
                "name": name,
                "watertight": instance_watertight(ctx, def),
                "health": instance_health(ctx, def),
                "bbox": locate::entity_bbox(ctx, &entity).map(bbox_json),
                "tags": ctx.doc.node_tags(node),
                "def": def_id,
                "pose": pose.map(|p| p.to_affine().to_vec()),
                "mirrored": pose.is_some_and(|p| p.determinant() < 0.0),
            })
        }
    };
    with_common(ctx, node, v)
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
        .iter()
        .map(|(rid, r)| {
            json!({
                "id": ids::region_id(sketch_sid, rid),
                "area": sk.region_area(rid).ok(),
                // A loop of edges with no face (`sketch.rs` rule 6) has
                // nothing to extrude: `hew.solid.extrude` refuses it.
                "face": !r.faceless,
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
    // drawing it (docs/agents/HEW_API.md §5.2): enough to tell two candidates
    // apart and pick one (id + endpoints), plus the owning curve id when
    // the edge is one curve's facet (a circle's or arc's, so erasing "the
    // circle" versus "one facet" is the client's own informed choice).
    let edges: Vec<Value> = sketch_edges_json(sk, sketch_sid);

    let mut out = json!({
        "id": public_of_or_internal(ctx, &EntityRef::Sketch(sketch_id))?,
        "plane": plane_json(&sk.plane()),
        "regions": regions,
        "curves": curves,
        "edges": edges,
    });
    // A detached shape (in no plane's arrangement; detached shapes D10)
    // says so; a plain sketch's summary is unchanged.
    if sk.detached() {
        out["detached"] = json!(true);
    }
    Ok(out)
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
    // Tag paths, the way a node's read back; a guide with a hidden tag is
    // in the document but not drawn or snapped to.
    let tags = ctx.doc.guide_tags(id);
    Ok(match g {
        Guide::Line {
            origin,
            direction,
            span,
        } => json!({
            "id": pub_id, "kind": "line",
            "origin": point_json(origin), "direction": vec3_json(direction),
            "from": span.from, "to": span.to,
            "tags": tags,
        }),
        Guide::Point { position } => json!({
            "id": pub_id, "kind": "point",
            "position": point_json(position),
            "tags": tags,
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
        "texture": texture_json(m),
        "usage": ctx.doc.material_usage(id),
    }))
}

/// A material's image texture as the palette shows it — its codec and the
/// meters one tile covers — or `null` for a flat color. The image bytes
/// themselves stay out of the scene dump.
fn texture_json(m: &kernel::Material) -> Value {
    match &m.texture {
        Some(t) => json!({
            "format": match t.format {
                kernel::ImageFormat::Png => "png",
                kernel::ImageFormat::Jpeg => "jpeg",
            },
            "world_size": [t.world_size[0], t.world_size[1]],
        }),
        None => Value::Null,
    }
}

/// A face's positioned texture as `hew.material.position_texture` takes it:
/// the tile corners (0,0), (1,0) and (0,1) of its UV frame on the face's
/// plane, or `null` while the face uses the default projection.
fn texture_pins_json(uv: Option<kernel::UvFrame>, plane: &kernel::Plane) -> Value {
    let Some(f) = uv else {
        return Value::Null;
    };
    // Solve s·p + u0 = u, t·p + v0 = v, n·p = n·q for the three corners.
    let n = plane.normal();
    let q = plane.point();
    let d = n.dot(Vec3::new(q.x, q.y, q.z));
    let det = f.s.dot(f.t.cross(n));
    // Relative, like UvFrame::is_valid's own test, so a valid frame of any
    // tile size reads back.
    if !det.is_finite() || det.abs() <= 1e-9 * f.s.length() * f.t.length() {
        return Value::Null;
    }
    let corner = |u: f64, v: f64| {
        let (a, b, c) = (u - f.u0, v - f.v0, d);
        // p = (a (t×n) + b (n×s) + c (s×t)) / det
        let p = (f.t.cross(n) * a + n.cross(f.s) * b + f.s.cross(f.t) * c) * (1.0 / det);
        point_json(Point3::new(p.x, p.y, p.z))
    };
    json!({ "origin": corner(0.0, 0.0), "u": corner(1.0, 0.0), "v": corner(0.0, 1.0) })
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
        // World instances plus member instances of a LIVE owning
        // definition — the count a Components panel and a Purge Unused
        // preview act on, per `kernel::Document::definition_usage`'s doc
        // comment. In every reachable document state this equals
        // `instance_count` above (a member instance's row is only ever
        // live while its owning definition is too — deleting a definition
        // hides its own member subtree in the same step), so the two
        // fields agree in practice; `usage` is kept as its own field
        // because it is the kernel's own documented "what a Components
        // panel shows" accessor, not derived from `instance_count` here.
        "usage": ctx.doc.definition_usage(id),
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
    // back (docs/agents/HEW_API.md §5.2), so this is handled here rather than
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
                "health": object_health(obj),
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
            // Every member node (objects, groups, nested instances), live
            // ones only, in member order.
            let mut member_nodes = Vec::new();
            for m in ctx.doc.def_member_nodes(cid).unwrap_or_default() {
                let live = match m {
                    NodeId::Object(o) => ctx.doc.object(o).is_some(),
                    NodeId::Group(g) => ctx.doc.group_members(g).is_some(),
                    NodeId::Instance(i) => ctx.doc.instance_def(i).is_some(),
                };
                if live {
                    member_nodes.push(node_ref_json(ctx, m)?);
                }
            }
            Ok(json!({
                "kind": "component",
                "id": p.id,
                "name": ctx.doc.component_name(cid),
                "member_objects": member_ids,
                "members": member_nodes,
                "bounds_local": ctx.doc.definition_bounds(cid).map(bbox_json),
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
            let material = ctx
                .doc
                .instance_material(iid)
                .flatten()
                .and_then(|m| public_of(ctx, &EntityRef::Material(m)));
            Ok(json!({
                "kind": "instance",
                "id": p.id,
                "def": public_of_or_internal(ctx, &EntityRef::Component(def))?,
                "pose": pose.to_affine().to_vec(),
                "mirrored": pose.determinant() < 0.0,
                "name": name,
                "watertight": instance_watertight(ctx, Some(def)),
                "health": instance_health(ctx, Some(def)),
                "bbox": locate::entity_bbox(ctx, &EntityRef::Instance(iid)).map(bbox_json),
                "tags": ctx.doc.node_tags(NodeId::Instance(iid)),
                "bounds_local": ctx.doc.definition_bounds(def).map(bbox_json),
                "material": material,
                // An id names no path: a world instance's effective material
                // is its own (nothing encloses it), and a nested placement's
                // depends on the path it is reached by, which
                // `hew.query.occurrences` reports per row.
                "material_effective": material,
            }))
        }
        EntityRef::Guide(gid) => {
            let g = *ctx.doc.guide(gid).ok_or_else(|| unknown_entity(&p.id))?;
            Ok(match g {
                Guide::Line {
                    origin,
                    direction,
                    span,
                } => json!({
                    "kind": "guide", "id": p.id, "guide_kind": "line",
                    "origin": point_json(origin), "direction": vec3_json(direction),
                    "from": span.from, "to": span.to,
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
                "texture": texture_json(m),
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
        EntityRef::Annotation(aid) => {
            let a = ctx
                .doc
                .annotation(aid)
                .ok_or_else(|| unknown_entity(&p.id))?;
            let anchor = |x: &kernel::Anchor| point_json(x.point);
            let mut out = match a {
                kernel::Annotation::LinearDimension {
                    a,
                    b,
                    offset,
                    text_override,
                    ..
                } => json!({
                    "annotation_kind": "linear",
                    "a": anchor(a), "b": anchor(b), "offset": vec3_json(*offset),
                    "text_override": text_override,
                }),
                kernel::Annotation::RadialDimension {
                    anchor: at,
                    kind,
                    curve,
                    text_override,
                    ..
                } => json!({
                    "annotation_kind": "radial",
                    "a": anchor(at),
                    "radial_kind": match kind { kernel::RadialKind::Radius => "radius", kernel::RadialKind::Diameter => "diameter" },
                    "center": point_json(curve.center), "radius": curve.radius,
                    "text_override": text_override,
                }),
                kernel::Annotation::LeaderText {
                    anchor: at,
                    offset,
                    text,
                } => json!({
                    "annotation_kind": "leader",
                    "a": anchor(at), "offset": vec3_json(*offset), "text": text,
                }),
            };
            out["kind"] = json!("annotation");
            out["id"] = json!(p.id);
            out["detached"] = json!(ctx.doc.annotation_detached(aid).unwrap_or(false));
            out["tags"] = json!(ctx.doc.annotation_tags(aid));
            Ok(out)
        }
    }
}

/// `hew.query.entity` on a sketch edge's own id — a client that just
/// queried or minted an edge id (drawing a sketch, or `hew.query.resolve`
/// against a geometric locator) will reasonably try querying it right
/// back, so this is wired up rather than left to refuse
/// (docs/agents/HEW_API.md §5.2). `edge`'s presence in `sk.edges()` is re-checked
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
/// (docs/agents/HEW_API.md §5.2's locators are for mutation; a query answers with
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
        "texture_pins": texture_pins_json(f.uv_frame, &f.plane),
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FacesParams {
    #[serde(default)]
    object: Option<String>,
    /// An occurrence path ending at the object (docs/agents/HEW_API.md
    /// §5.2): the faces then also report `material_effective` through it.
    #[serde(default)]
    path: Option<Vec<String>>,
}

fn faces(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let p: FacesParams = parse(params)?;
    let (oid, object_id, inherited) = match (p.object, p.path) {
        (Some(id), None) => {
            let Some(EntityRef::Object(oid)) = ctx.resolver().resolve(&id) else {
                return Err(unknown_entity(&id));
            };
            (oid, id, None)
        }
        (None, Some(path)) => {
            let occ = locate::resolve_occurrence_path(ctx, &path)?;
            let NodeId::Object(oid) = occ.node else {
                let leaf = path.last().cloned().unwrap_or_default();
                return Err(unknown_entity(&leaf));
            };
            let id = public_of_or_internal(ctx, &EntityRef::Object(oid))?;
            (oid, id, Some(ctx.doc.inherited_material(&occ.path)))
        }
        _ => {
            return Err(CmdError::Params(
                "give exactly one of \"object\" or \"path\"".into(),
            ));
        }
    };
    let obj = ctx
        .doc
        .object(oid)
        .ok_or_else(|| unknown_entity(&object_id))?;
    let base = obj.default_material();
    let faces: Vec<(FaceId, Option<kernel::MaterialId>)> = obj
        .faces()
        .iter()
        .map(|(fid, f)| (fid, f.material))
        .collect();
    let mut out = Vec::with_capacity(faces.len());
    for (fid, own) in faces {
        let mut v = face_description(
            ctx,
            locate::FaceRef {
                object: oid,
                face: fid,
            },
        )?;
        // Through a path, the face's effective material: its own, else
        // the object's base, else the nearest painted instance above it.
        if let Some(inherited) = inherited {
            let effective = own
                .or(base)
                .or(inherited)
                .and_then(|m| public_of(ctx, &EntityRef::Material(m)));
            v["material_effective"] = json!(effective);
        }
        // The face's attribute dictionary (docs/agents/HEW_API.md §8), `{}`
        // for none.
        v["attrs"] =
            super::attrs::element_attrs_json(ctx.doc.object(oid).and_then(|o| o.face_attrs(fid)));
        out.push(v);
    }
    Ok(json!({ "object": object_id, "faces": out }))
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
    #[serde(default)]
    all: bool,
    #[serde(default)]
    max_distance: Option<f64>,
    #[serde(default)]
    exclude: Option<Vec<Value>>,
    #[serde(default)]
    include_hidden: bool,
}

/// The most hits an `all: true` raycast lists.
const MAX_RAY_HITS: usize = 1_000;

/// The most `exclude` entries one raycast takes.
const MAX_RAY_EXCLUDES: usize = 64;

/// One face crossing of the ray, reduced to world-space numbers so ties
/// and "nearest wins" are decided in one common, physically meaningful
/// unit whatever frame the face was intersected in.
struct RayCandidate {
    /// World-space distance along the ray.
    distance: f64,
    point: Point3,
    /// World-space face normal.
    normal: Vec3,
    /// The leaf placement's path (instances, world down), without the leaf.
    path: Vec<kernel::InstanceId>,
    leaf: kernel::ObjectId,
    /// The hit in the leaf Object's own frame (a face locator's `at`).
    at: Point3,
    hidden: bool,
}

/// Folds one more candidate distance into the running nearest/tie state.
/// A closer candidate replaces; a candidate within
/// [`geom::API_AMBIGUITY_TOL`] of the current best is a tie (refused, never
/// guessed — §5.2); anything farther is dropped.
fn consider_hit(best: &mut Option<f64>, tie: &mut bool, distance: f64) {
    match best {
        None => {
            *best = Some(distance);
            *tie = false;
        }
        Some(bt) if distance < *bt - geom::API_AMBIGUITY_TOL => {
            *best = Some(distance);
            *tie = false;
        }
        Some(bt) if (distance - *bt).abs() <= geom::API_AMBIGUITY_TOL => {
            *tie = true;
        }
        _ => {}
    }
}

/// The occurrence JSON members of a raycast hit.
fn ray_hit_json(ctx: &Ctx, hit: &RayCandidate) -> Result<Value, CmdError> {
    let leaf = public_of_or_internal(ctx, &EntityRef::Object(hit.leaf))?;
    let path = hit
        .path
        .iter()
        .map(|&i| public_of_or_internal(ctx, &EntityRef::Instance(i)))
        .collect::<Result<Vec<_>, _>>()?;
    // The legacy members: the top-level thing the ray hit.
    let (object, kind) = match path.first() {
        Some(first) => (first.clone(), "instance"),
        None => (leaf.clone(), "object"),
    };
    Ok(json!({
        "object": object,
        "kind": kind,
        "point": point_json(hit.point),
        "distance": hit.distance,
        "normal": vec3_json(hit.normal),
        "path": path,
        "leaf": leaf,
        "face": { "object": leaf, "at": point_json(hit.at) },
        "hidden": hit.hidden,
    }))
}

fn raycast(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let p: RaycastParams = parse(params)?;
    let origin = locate::parse_xyz(&p.origin)?;
    let dir = locate::parse_dir(&p.dir)?;
    // A world-space unit direction to measure every candidate's distance
    // along, regardless of which frame (world, or a leaf mapped back
    // through its placement's pose) it was actually intersected in — a
    // local-space parameter is meaningless to compare against a
    // world-space one when the pose scales (docs/design/api-kernel-map.md
    // §1.5's Transform).
    let dir_n = dir
        .normalized()
        .map_err(|_| CmdError::Params("dir must be non-zero".into()))?;
    if let Some(max) = p.max_distance
        && !(max.is_finite() && max > 0.0)
    {
        return Err(CmdError::Params(
            "max_distance must be a finite number above zero".into(),
        ));
    }

    // Excluded placements: each entry and everything under it.
    let mut excluded: BTreeSet<(Vec<kernel::InstanceId>, NodeId)> = BTreeSet::new();
    if let Some(entries) = &p.exclude {
        if entries.len() > MAX_RAY_EXCLUDES {
            return Err(CmdError::Params(format!(
                "exclude takes at most {MAX_RAY_EXCLUDES} entries"
            )));
        }
        let mut roots = Vec::with_capacity(entries.len());
        for v in entries {
            let occ = locate::resolve_occurrence_locator(ctx, v)?;
            roots.push((occ.path, occ.node));
        }
        if !roots.is_empty() {
            for row in ctx.doc.occurrences(Some(&roots), true)? {
                excluded.insert((row.path, row.node));
            }
        }
    }

    // Every leaf placement, once, under its composed world pose. A pose
    // that cannot be inverted (poses are invertible by construction) skips
    // that one placement rather than refusing the whole query.
    let mut candidates: Vec<RayCandidate> = Vec::new();
    for row in ctx.doc.occurrences(None, p.include_hidden)? {
        let NodeId::Object(leaf) = row.node else {
            continue;
        };
        if excluded.contains(&(row.path.clone(), row.node)) {
            continue;
        }
        let Some(obj) = ctx.doc.object(leaf) else {
            continue;
        };
        let Ok(inv) = row.pose.inverse() else {
            continue;
        };
        let local_origin = inv.apply_point(origin);
        let local_dir = inv.apply_vector(dir);
        for (_fid, f) in obj.faces() {
            let Some(t) = geom::ray_plane_t(local_origin, local_dir, &f.plane) else {
                continue;
            };
            let at = local_origin + local_dir * t;
            let outer: Vec<Point3> = obj.loop_positions(f.outer_loop).collect();
            let holes: Vec<Vec<Point3>> = f
                .inner_loops
                .iter()
                .map(|&l| obj.loop_positions(l).collect())
                .collect();
            if !geom::face_contains(&outer, &holes, f.plane.normal(), at) {
                continue;
            }
            let point = row.pose.apply_point(at);
            // World-space distance, NOT the local `t` above (§ comment on
            // `dir_n`): (hit - origin)·dir_n.
            let distance = (point - origin).dot(dir_n);
            if distance < 0.0 || p.max_distance.is_some_and(|max| distance > max) {
                continue;
            }
            // The face's outward normal in world space: the inverse
            // transpose of the pose, which keeps pointing the same side
            // of the surface under a reflection (a plane refit through
            // mapped points would flip it, since the winding reverses).
            let Ok(normal) = row.pose.apply_normal(f.plane.normal()).normalized() else {
                continue;
            };
            // `+ 0.0` turns a negative zero into a plain one.
            let normal = Vec3::new(normal.x + 0.0, normal.y + 0.0, normal.z + 0.0);
            candidates.push(RayCandidate {
                distance,
                point,
                normal,
                path: row.path.clone(),
                leaf,
                at,
                hidden: row.hidden,
            });
        }
    }

    if p.all {
        // Stable: ties keep occurrence order.
        candidates.sort_by(|a, b| a.distance.total_cmp(&b.distance));
        let capped = candidates.len() > MAX_RAY_HITS;
        candidates.truncate(MAX_RAY_HITS);
        let hits = candidates
            .iter()
            .map(|c| ray_hit_json(ctx, c))
            .collect::<Result<Vec<_>, _>>()?;
        let mut out = json!({ "hits": hits });
        if capped {
            out["raycast_hits_capped"] = json!(true);
        }
        return Ok(out);
    }

    let mut best: Option<f64> = None;
    let mut tie = false;
    for c in &candidates {
        consider_hit(&mut best, &mut tie, c.distance);
    }
    if best.is_some() && tie {
        return Err(locator_ambiguous());
    }
    let nearest = candidates
        .iter()
        .min_by(|a, b| a.distance.total_cmp(&b.distance))
        .ok_or_else(locator_missed)?;
    ray_hit_json(ctx, nearest)
}

// ------------------------------------------------------------- hew.query.measure

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MeasureParams {
    #[serde(default)]
    from: Option<Value>,
    #[serde(default)]
    to: Option<Value>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    path: Option<Vec<String>>,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    exclude: Option<Vec<String>>,
}

fn measure(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let p: MeasureParams = parse(params)?;
    let pair = p.from.is_some() || p.to.is_some();
    if usize::from(pair) + usize::from(p.id.is_some()) + usize::from(p.path.is_some()) != 1 {
        return Err(CmdError::Params(
            "hew.query.measure needs exactly one of {from, to}, {id}, or {path}".into(),
        ));
    }
    let scope = crate::scope::Scope::from_name(p.scope.as_deref())?;
    if pair && (p.scope.is_some() || p.exclude.is_some()) {
        return Err(CmdError::Params(
            "scope and exclude apply to measuring a part ({id} or {path}), not a distance".into(),
        ));
    }
    if pair {
        let (Some(from), Some(to)) = (&p.from, &p.to) else {
            return Err(CmdError::Params(
                "\"from\" and \"to\" are given together".into(),
            ));
        };
        let from = locate::resolve_point(ctx, from)?;
        let to = locate::resolve_point(ctx, to)?;
        let delta = to - from;
        return Ok(json!({
            "distance": delta.length(),
            "delta": vec3_json(delta),
        }));
    }
    let occ = match (&p.id, &p.path) {
        (Some(id), None) => locate::resolve_world_node(ctx, id)?,
        (None, Some(path)) => locate::resolve_occurrence_path(ctx, path)?,
        _ => unreachable!("exactly one subject form"),
    };
    let exclude = locate::resolve_excluded(ctx, &occ, scope, p.exclude.as_deref())?;
    measure_result_excluding(ctx.doc, occ, scope, &exclude)
}

/// Volume, area, and world-axis bounding box of one placement and
/// everything under it. A hidden part still measures (a measure is not a
/// view).
pub fn measure_result(doc: &kernel::Document, occ: kernel::Occurrence) -> Result<Value, CmdError> {
    measure_result_scoped(doc, occ, crate::scope::Scope::All)
}

/// [`measure_result`] over the leaves `scope` selects: under
/// [`crate::scope::Scope::Own`] only the container's own Objects count; a
/// container with none measures as an empty one does.
pub fn measure_result_scoped(
    doc: &kernel::Document,
    occ: kernel::Occurrence,
    scope: crate::scope::Scope,
) -> Result<Value, CmdError> {
    measure_result_excluding(doc, occ, scope, &[])
}

/// [`measure_result_scoped`] with the Objects in `exclude` left out of the
/// volume, area, box and counts. Excluding every own Object measures as a
/// container with none.
pub fn measure_result_excluding(
    doc: &kernel::Document,
    occ: kernel::Occurrence,
    scope: crate::scope::Scope,
    exclude: &[kernel::ObjectId],
) -> Result<Value, CmdError> {
    let mut leaves = crate::scope::scoped_leaves(doc, &locate::RenderRoot::Occurrence(occ), scope)?;
    leaves.retain(|(o, _, _)| !exclude.contains(o));
    let mut volume: Option<f64> = None;
    let mut area = 0.0;
    let mut open_leaves = 0usize;
    let mut points: Vec<Point3> = Vec::new();
    for (oid, pose, _path) in &leaves {
        let Some(obj) = doc.object(*oid) else {
            continue;
        };
        match obj.volume() {
            Some(v) => *volume.get_or_insert(0.0) += v * pose.determinant().abs(),
            None => open_leaves += 1,
        }
        // Per face, from the posed rings: exact for planar faces under any
        // affine pose, mirrored included (`apply_transform` refuses a
        // reflection).
        for (_fid, f) in obj.faces() {
            let outer: Vec<Point3> = obj
                .loop_positions(f.outer_loop)
                .map(|pt| pose.apply_point(pt))
                .collect();
            let holes: Vec<Vec<Point3>> = f
                .inner_loops
                .iter()
                .map(|&l| {
                    obj.loop_positions(l)
                        .map(|pt| pose.apply_point(pt))
                        .collect()
                })
                .collect();
            area += geom::face_area(&outer, &holes);
        }
        points.extend(
            obj.vertices()
                .values()
                .map(|v| pose.apply_point(v.position)),
        );
    }
    let Some((min, max)) = geom::bbox(points.into_iter()) else {
        return Err(CmdError::Params(
            "that entity has no geometry to measure".into(),
        ));
    };
    Ok(json!({
        "volume": volume,
        "area": area,
        "bbox": { "min": point_json(min), "max": point_json(max) },
        "extents": vec3_json(max - min),
        "leaf_count": leaves.len(),
        "open_leaves": open_leaves,
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
    #[serde(default)]
    occurrence: Option<Value>,
}

fn resolve(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let p: ResolveParams = parse(params)?;
    let present = [
        p.point.is_some(),
        p.face.is_some(),
        p.edge.is_some(),
        p.occurrence.is_some(),
    ]
    .into_iter()
    .filter(|b| *b)
    .count();
    if present != 1 {
        return Err(CmdError::Params(
            "hew.query.resolve needs exactly one of \"point\", \"face\", \"edge\", \"occurrence\""
                .into(),
        ));
    }
    if let Some(v) = p.occurrence {
        if v.is_string() {
            return Err(CmdError::Params(
                "an occurrence locator is {\"path\": [...]}".into(),
            ));
        }
        let occ = locate::resolve_occurrence_locator(ctx, &v)?;
        return Ok(json!({ "occurrence": super::parts::occurrence_json(ctx, &occ, None)? }));
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

// -------------------------------------------------------- hew.query.changes_since

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ChangesSinceParams {
    generation: u64,
}

/// What changed since a client saw `generation` (docs/agents/HEW_API.md
/// §4.5): the union of the change sets since then, or `full` when the
/// history no longer covers it.
fn changes_since(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let p: ChangesSinceParams = parse(params)?;
    Ok(json!({
        "from": p.generation,
        "changes": crate::events::changes_since_json(ctx.doc, p.generation),
    }))
}

// ------------------------------------------------------------- hew.query.context

fn context(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let _: EmptyParams = parse(params)?;

    let stack = context_stack_json(ctx.doc).map_err(CmdError::Internal)?;

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

    let fallback = context_fallback_json(ctx.doc, &ctx.host.fallback_context());

    Ok(json!({
        "stack": stack,
        "direct_members": direct_members,
        "fallback": fallback,
        "armed": ctx.host.gesture_armed(),
    }))
}

/// `hew.query.context`'s `fallback`: the app's in-context entries
/// sitting inside the innermost open frame, outermost first, each
/// `{kind, id}`. An entry whose node no longer carries a stable id (deleted
/// since the app last reported) is left out. Shared with the
/// `context_changed` event.
pub(crate) fn context_fallback_json(
    doc: &kernel::Document,
    entries: &[crate::host::ContextEntry],
) -> Value {
    let mut out = Vec::new();
    for e in entries {
        let (kind, entity) = match (e.kind, e.node) {
            (crate::host::ContextKind::Object, NodeId::Object(o)) => {
                ("object", EntityRef::Object(o))
            }
            (crate::host::ContextKind::Instance, NodeId::Instance(i)) => {
                ("instance", EntityRef::Instance(i))
            }
            _ => continue,
        };
        if let Some(sid) = doc.sid_of(&entity) {
            out.push(json!({ "kind": kind, "id": ids::public_id(&entity, sid) }));
        }
    }
    Value::Array(out)
}

/// `hew.query.context`'s `stack`: the open editing frames, outermost
/// first, each `{kind, id}`. Shared with the `context_changed` event so
/// the two shapes cannot drift.
pub(crate) fn context_stack_json(doc: &kernel::Document) -> Result<Value, String> {
    let mut stack = Vec::new();
    for frame in doc.session_stack() {
        let (kind, entity) = match frame {
            NodeId::Group(g) => ("group", EntityRef::Group(g)),
            NodeId::Instance(i) => ("component", EntityRef::Instance(i)),
            NodeId::Object(_) => {
                return Err(
                    "session frame collapsed to an object, which never opens a session".into(),
                );
            }
        };
        let sid = doc
            .sid_of(&entity)
            .ok_or_else(|| "entity carries no stable id".to_string())?;
        stack.push(json!({ "kind": kind, "id": ids::public_id(&entity, sid) }));
    }
    Ok(Value::Array(stack))
}

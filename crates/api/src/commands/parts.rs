//! Model reads for parts (docs/agents/HEW_API.md §7; docs/agents/PLUGINS.md
//! "Reading parts"): `hew.query.occurrences`, `hew.query.part_frame`,
//! `hew.query.signature`, `hew.query.mesh`, and `hew.query.topology`.

use super::{CmdError, Ctx, Handler};
use crate::locate;
use crate::part_frame;
use crate::refusal::Refusal;
use crate::scope::Scope;
use kernel::{EntityRef, NodeId, ObjectId, Occurrence, Point3, RenderRoot, Transform, Vec3};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// The most buffer bytes one `hew.query.mesh` result carries, before
/// base64.
pub const MAX_MESH_BYTES: usize = 64 * 1024 * 1024;

/// This namespace's slice of the handler table.
pub fn handler(name: &str) -> Option<Handler> {
    fn clocked(
        f: fn(&mut Ctx, &Value) -> Result<Value, CmdError>,
        ctx: &mut Ctx,
        params: &Value,
    ) -> Result<Value, CmdError> {
        let result = f(ctx, params)?;
        Ok(super::with_clock(ctx.doc, result))
    }
    fn occ(ctx: &mut Ctx, p: &Value) -> Result<Value, CmdError> {
        clocked(occurrences, ctx, p)
    }
    fn frame(ctx: &mut Ctx, p: &Value) -> Result<Value, CmdError> {
        clocked(part_frame_cmd, ctx, p)
    }
    fn mesh_c(ctx: &mut Ctx, p: &Value) -> Result<Value, CmdError> {
        clocked(mesh, ctx, p)
    }
    fn topo(ctx: &mut Ctx, p: &Value) -> Result<Value, CmdError> {
        clocked(topology, ctx, p)
    }
    fn sig(ctx: &mut Ctx, p: &Value) -> Result<Value, CmdError> {
        clocked(signature, ctx, p)
    }
    Some(match name {
        "hew.query.occurrences" => occ,
        "hew.query.part_frame" => frame,
        "hew.query.mesh" => mesh_c,
        "hew.query.topology" => topo,
        "hew.query.signature" => sig,
        _ => return None,
    })
}

fn parse<T: serde::de::DeserializeOwned>(params: &Value) -> Result<T, CmdError> {
    serde_json::from_value(params.clone()).map_err(|e| CmdError::Params(e.to_string()))
}

fn public(ctx: &Ctx, entity: &EntityRef) -> Result<String, CmdError> {
    ctx.doc
        .sid_of(entity)
        .map(|sid| crate::ids::public_id(entity, sid))
        .ok_or_else(|| CmdError::Internal("entity carries no stable id".into()))
}

fn node_entity(node: NodeId) -> EntityRef {
    match node {
        NodeId::Object(o) => EntityRef::Object(o),
        NodeId::Group(g) => EntityRef::Group(g),
        NodeId::Instance(i) => EntityRef::Instance(i),
    }
}

fn kind_name(node: NodeId) -> &'static str {
    match node {
        NodeId::Object(_) => "object",
        NodeId::Group(_) => "group",
        NodeId::Instance(_) => "instance",
    }
}

fn point_json(p: Point3) -> Value {
    json!([p.x, p.y, p.z])
}

fn vec_json(v: Vec3) -> Value {
    json!([v.x, v.y, v.z])
}

/// The part-model facts `hew.query.occurrences` adds to its rows
/// (docs/agents/PLUGINS.md "What a row says about a part"): the health
/// report, computed once per call and only when an Object row is listed,
/// and whether hidden members are listed.
pub(crate) struct RowFacts {
    include_hidden: bool,
    /// Object → (solid, family, family_closed), from one health report of
    /// the whole document, so `family` indexes `hew.doc.health`'s list.
    health: Option<BTreeMap<ObjectId, Option<usize>>>,
    /// Whether each family's shared rims are all cappable.
    family_closed: Vec<bool>,
}

impl RowFacts {
    fn new(doc: &kernel::Document, include_hidden: bool, any_object: bool) -> RowFacts {
        if !any_object {
            return RowFacts {
                include_hidden,
                health: None,
                family_closed: Vec::new(),
            };
        }
        let report = doc.health_report(None);
        let mut closed = vec![true; report.families.len()];
        let mut rows = BTreeMap::new();
        for row in &report.rows {
            if let Some(f) = row.seam_family {
                let cappable = doc.object(row.object).is_some_and(|o| {
                    row.shared_rims
                        .iter()
                        .all(|&i| kernel::repair::rim_is_cappable(o, &row.diagnosis, i))
                });
                if let Some(c) = closed.get_mut(f) {
                    *c &= cappable;
                }
            }
            rows.insert(row.object, row.seam_family);
        }
        RowFacts {
            include_hidden,
            health: Some(rows),
            family_closed: closed,
        }
    }

    /// `(solid, family, family_closed)` for an Object: solid when the
    /// health report has no row for it (its verdict is solid).
    fn object(&self, o: ObjectId) -> (bool, Option<usize>, Option<bool>) {
        match self.health.as_ref().and_then(|h| h.get(&o)) {
            None => (true, None, None),
            Some(family) => (
                false,
                *family,
                family.and_then(|f| self.family_closed.get(f).copied()),
            ),
        }
    }
}

/// Whether a member node is listed under `members`: live, and shown unless
/// hidden ones were asked for (user-hidden or on a hidden tag).
fn member_listed(doc: &kernel::Document, node: NodeId, include_hidden: bool) -> bool {
    let live = match node {
        NodeId::Object(o) => doc.object(o).is_some(),
        NodeId::Group(g) => doc.group_members(g).is_some(),
        NodeId::Instance(i) => doc.instance_def(i).is_some(),
    };
    live && (include_hidden || !doc.node_effectively_hidden(node))
}

/// `members` for a row: the node's direct members by kind, in member order.
fn members_json(ctx: &Ctx, o: &Occurrence, include_hidden: bool) -> Result<Value, CmdError> {
    let nodes = match o.node {
        NodeId::Object(_) => Vec::new(),
        NodeId::Group(g) => ctx.doc.group_members(g).unwrap_or_default(),
        NodeId::Instance(_) => o
            .definition
            .and_then(|d| ctx.doc.def_member_nodes(d))
            .unwrap_or_default(),
    };
    let (mut objects, mut groups, mut instances) = (Vec::new(), Vec::new(), Vec::new());
    for n in nodes {
        if !member_listed(ctx.doc, n, include_hidden) {
            continue;
        }
        let id = public(ctx, &node_entity(n))?;
        match n {
            NodeId::Object(_) => objects.push(id),
            NodeId::Group(_) => groups.push(id),
            NodeId::Instance(_) => instances.push(id),
        }
    }
    Ok(json!({ "objects": objects, "groups": groups, "instances": instances }))
}

/// One row as `OccurrenceJson` (docs/agents/HEW_API.md §7): `path` is the
/// instances above the node; `{ "path": [...path, id] }` locates it. With
/// `facts` (the `hew.query.occurrences` rows) it also carries `members`,
/// an Object's `solid` / `family` / `family_closed`, and an Instance's
/// `linked`; without (`hew.query.resolve`'s occurrence) it does not.
pub(crate) fn occurrence_json(
    ctx: &Ctx,
    o: &Occurrence,
    facts: Option<&RowFacts>,
) -> Result<Value, CmdError> {
    let mut path = Vec::with_capacity(o.path.len());
    for &i in &o.path {
        path.push(public(ctx, &EntityRef::Instance(i))?);
    }
    let definition = match o.definition {
        Some(d) => Some(public(ctx, &EntityRef::Component(d))?),
        None => None,
    };
    let parent = match o.parent_group {
        Some(g) => Some(public(ctx, &EntityRef::Group(g))?),
        None => None,
    };
    let material_id = |m: Option<kernel::MaterialId>| -> Option<String> {
        let m = m?;
        let e = EntityRef::Material(m);
        ctx.doc.sid_of(&e).map(|s| crate::ids::public_id(&e, s))
    };
    let (name, material) = match o.node {
        NodeId::Object(id) => (
            ctx.doc.object_name(id).map(str::to_string),
            material_id(ctx.doc.object(id).and_then(|obj| obj.default_material())),
        ),
        NodeId::Group(id) => (ctx.doc.group_name(id).map(str::to_string), None),
        NodeId::Instance(id) => (
            ctx.doc.instance_name(id).map(str::to_string),
            material_id(o.material),
        ),
    };
    let material_effective = material_id(o.material_effective);
    let definition_name = o
        .definition
        .and_then(|d| ctx.doc.component_name(d))
        .map(str::to_string);
    let mut row = json!({
        "path": path,
        "id": public(ctx, &node_entity(o.node))?,
        "kind": kind_name(o.node),
        "definition": definition,
        "parent": parent,
        "world_pose": o.pose.to_affine().to_vec(),
        "mirrored": o.pose.determinant() < 0.0,
        "hidden": o.hidden,
        "user_hidden": ctx.doc.node_user_hidden(o.node),
        "material": material,
        "material_effective": material_effective,
        "tags": ctx.doc.node_tags(o.node),
        "name": name,
        "definition_name": definition_name,
        "depth": o.depth,
    });
    if let Some(facts) = facts {
        row["members"] = members_json(ctx, o, facts.include_hidden)?;
        match o.node {
            NodeId::Object(id) => {
                let (solid, family, closed) = facts.object(id);
                row["solid"] = json!(solid);
                row["family"] = json!(family);
                row["family_closed"] = json!(closed);
            }
            NodeId::Instance(_) => {
                row["linked"] = json!(o.definition.is_some_and(|d| ctx.doc.is_linked_set(d)));
            }
            NodeId::Group(_) => {}
        }
    }
    Ok(row)
}

// ------------------------------------------------------ hew.query.occurrences

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OccurrencesParams {
    #[serde(default)]
    roots: Option<Vec<Value>>,
    #[serde(default)]
    kinds: Option<Vec<String>>,
    #[serde(default)]
    include_hidden: bool,
}

fn occurrences(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let p: OccurrencesParams = parse(params)?;
    let kinds = match &p.kinds {
        None => vec!["object", "group", "instance"],
        Some(list) => {
            let mut out = Vec::with_capacity(list.len());
            for k in list {
                match k.as_str() {
                    "object" | "group" | "instance" => out.push(k.as_str()),
                    other => {
                        return Err(CmdError::Params(format!(
                            "unknown kind \"{other}\"; expected object, group, or instance"
                        )));
                    }
                }
            }
            out
        }
    };
    let roots = match &p.roots {
        None => None,
        Some(list) if list.is_empty() => {
            return Err(CmdError::Params(
                "roots must name at least one entity".into(),
            ));
        }
        Some(list) => {
            let mut out = Vec::with_capacity(list.len());
            for v in list {
                let o = locate::resolve_occurrence_locator(ctx, v)?;
                out.push((o.path, o.node));
            }
            Some(out)
        }
    };
    let rows = ctx.doc.occurrences(roots.as_deref(), p.include_hidden)?;
    let listed: Vec<&Occurrence> = rows
        .iter()
        .filter(|r| kinds.contains(&kind_name(r.node)))
        .collect();
    let any_object = listed.iter().any(|r| matches!(r.node, NodeId::Object(_)));
    let facts = RowFacts::new(ctx.doc, p.include_hidden, any_object);
    let mut out = Vec::with_capacity(listed.len());
    for r in listed {
        out.push(occurrence_json(ctx, r, Some(&facts))?);
    }
    Ok(json!({ "occurrences": out }))
}

// ------------------------------------------------------ id-or-path targets

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TargetParams {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    path: Option<Vec<String>>,
    #[serde(default)]
    space: Option<String>,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    exclude: Option<Vec<String>>,
}

/// Coordinate space of a mesh or topology result.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Space {
    World,
    Local,
}

fn target(ctx: &Ctx, p: &TargetParams) -> Result<Occurrence, CmdError> {
    match (&p.id, &p.path) {
        (Some(id), None) => locate::resolve_world_node(ctx, id),
        (None, Some(path)) => locate::resolve_occurrence_path(ctx, path),
        _ => Err(CmdError::Params(
            "give exactly one of \"id\" or \"path\"".into(),
        )),
    }
}

/// `scope` belongs to `hew.query.part_frame` and `hew.query.signature`'s
/// siblings only; the buffer reads keep their M1 shape.
fn no_scope(p: &TargetParams, command: &str) -> Result<(), CmdError> {
    if p.scope.is_some() || p.exclude.is_some() {
        return Err(CmdError::Params(format!(
            "{command} takes no \"scope\" or \"exclude\""
        )));
    }
    Ok(())
}

fn space(p: &TargetParams) -> Result<Space, CmdError> {
    match p.space.as_deref() {
        None | Some("world") => Ok(Space::World),
        Some("local") => Ok(Space::Local),
        Some(other) => Err(CmdError::Params(format!(
            "unknown space \"{other}\"; expected world or local"
        ))),
    }
}

// ------------------------------------------------------ hew.query.part_frame

fn part_frame_cmd(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let p: TargetParams = parse(params)?;
    if p.space.is_some() {
        return Err(CmdError::Params(
            "hew.query.part_frame takes no \"space\"".into(),
        ));
    }
    let scope = Scope::from_name(p.scope.as_deref())?;
    let occ = target(ctx, &p)?;
    let exclude = locate::resolve_excluded(ctx, &occ, scope, p.exclude.as_deref())?;
    part_frame_result_excluding(ctx.doc, &occ, scope, &exclude)?.ok_or_else(|| {
        CmdError::Refusal(Refusal::api(
            "no_part_frame",
            "That entity has nothing to measure: it holds no faces.",
        ))
    })
}

/// `hew.query.part_frame`'s result for one placement, without the clock,
/// or `None` when it has nothing to measure. Shared with the app's own
/// measurement (`Scene::part_frame_json`) so the cut list and the bus
/// cannot disagree.
///
/// # Errors
/// [`kernel::DocumentError::TooManyOccurrences`] — the placement holds more
/// leaf placements than one walk lists.
pub fn part_frame_result(
    doc: &kernel::Document,
    occ: &Occurrence,
) -> Result<Option<Value>, kernel::DocumentError> {
    part_frame_result_scoped(doc, occ, Scope::All)
}

/// [`part_frame_result`] over the leaves `scope` selects.
///
/// # Errors
/// As [`part_frame_result`].
pub fn part_frame_result_scoped(
    doc: &kernel::Document,
    occ: &Occurrence,
    scope: Scope,
) -> Result<Option<Value>, kernel::DocumentError> {
    part_frame_result_excluding(doc, occ, scope, &[])
}

/// [`part_frame_result_scoped`] with the Objects in `exclude` left out.
///
/// # Errors
/// As [`part_frame_result`].
pub fn part_frame_result_excluding(
    doc: &kernel::Document,
    occ: &Occurrence,
    scope: Scope,
    exclude: &[kernel::ObjectId],
) -> Result<Option<Value>, kernel::DocumentError> {
    let Some(f) = part_frame::part_frame_excluding(doc, occ, scope, exclude)? else {
        return Ok(None);
    };
    Ok(Some(json!({
        "origin": point_json(f.origin),
        "x": vec_json(f.x),
        "y": vec_json(f.y),
        "z": vec_json(f.z),
        "extents": f.extents.to_vec(),
        "source": f.source.name(),
        "mirrored": occ.pose.determinant() < 0.0,
    })))
}

// ------------------------------------------------------ hew.query.signature

/// `hew.query.signature { id | path }` (docs/agents/PLUGINS.md "Same part,
/// mirrored part"): one Object's shape and mirror hashes and its frame. A
/// group or instance refuses `not_an_object`.
fn signature(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let p: TargetParams = parse(params)?;
    if p.space.is_some() || p.scope.is_some() || p.exclude.is_some() {
        return Err(CmdError::Params(
            "hew.query.signature takes only \"id\" or \"path\"".into(),
        ));
    }
    let occ = target(ctx, &p)?;
    if !matches!(occ.node, NodeId::Object(_)) {
        return Err(CmdError::Refusal(Refusal::api(
            "not_an_object",
            "hew.query.signature reads one object. List a group's or instance's objects with hew.query.occurrences, then ask for each.",
        )));
    }
    let s = crate::signature::signature(ctx.doc, &occ)?.ok_or_else(|| {
        CmdError::Refusal(Refusal::api(
            "no_part_frame",
            "That entity has nothing to measure: it holds no faces.",
        ))
    })?;
    Ok(json!({
        "shape": s.shape,
        "mirror": s.mirror,
        "symmetric": s.shape == s.mirror,
        "frame": {
            "origin": point_json(s.frame.origin),
            "x": vec_json(s.frame.x),
            "y": vec_json(s.frame.y),
            "z": vec_json(s.frame.z),
        },
    }))
}

// ------------------------------------------------------------ leaves

/// The leaf objects of a target with the pose that maps each into the
/// requested space: world poses, or poses in the node's own frame (an
/// object untransformed, a group's leaves untransformed, an instance's
/// leaves in definition-local coordinates).
fn leaves_in(
    ctx: &Ctx,
    occ: &Occurrence,
    space: Space,
) -> Result<Vec<(ObjectId, Transform)>, CmdError> {
    let world = |root: &RenderRoot| -> Result<Vec<(ObjectId, Transform)>, CmdError> {
        Ok(ctx
            .doc
            .render_root_leaves(root, true)?
            .into_iter()
            .map(|(o, t, _)| (o, t))
            .collect())
    };
    match (space, occ.node) {
        (Space::World, _) => world(&RenderRoot::Occurrence(occ.clone())),
        (Space::Local, NodeId::Object(o)) => Ok(vec![(o, Transform::IDENTITY)]),
        (Space::Local, NodeId::Instance(_)) => {
            let def = occ
                .definition
                .ok_or_else(|| CmdError::Internal("instance row without a definition".into()))?;
            world(&RenderRoot::Definition(def))
        }
        (Space::Local, NodeId::Group(_)) => {
            let leaves = world(&RenderRoot::Occurrence(occ.clone()))?;
            if occ.pose == Transform::IDENTITY {
                return Ok(leaves);
            }
            let inv = occ
                .pose
                .inverse()
                .map_err(|_| CmdError::Internal("singular occurrence pose".into()))?;
            Ok(leaves.into_iter().map(|(o, t)| (o, t.then(&inv))).collect())
        }
    }
}

// ----------------------------------------------------------- hew.query.mesh

/// Standard base64 with padding.
pub(crate) fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(ALPHABET[(n >> 18) as usize & 63] as char);
        out.push(ALPHABET[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

fn mesh_too_large(bytes: usize, limit: usize) -> CmdError {
    CmdError::Refusal(
        Refusal::api(
            "mesh_too_large",
            "That mesh is larger than one result can carry (64 MiB of buffers). Ask for its parts one at a time.",
        )
        .with_detail(json!({ "bytes": bytes, "limit": limit })),
    )
}

fn mesh(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    mesh_limited(ctx, params, MAX_MESH_BYTES)
}

/// `hew.query.mesh` with the buffer limit as a parameter, so the refusal
/// is testable without a 64 MiB model.
fn mesh_limited(ctx: &mut Ctx, params: &Value, limit: usize) -> Result<Value, CmdError> {
    let p: TargetParams = parse(params)?;
    no_scope(&p, "hew.query.mesh")?;
    let space = space(&p)?;
    let occ = target(ctx, &p)?;
    let leaves = leaves_in(ctx, &occ, space)?;
    let mut parts = Vec::with_capacity(leaves.len());
    for (o, pose) in leaves {
        let Some(obj) = ctx.doc.object(o) else {
            continue;
        };
        parts.push((
            obj,
            pose,
            Value::String(public(ctx, &EntityRef::Object(o))?),
        ));
    }
    mesh_block(ctx.doc, &parts, limit)
}

/// The `hew.query.mesh` result shape (without the clock) for `parts`, each
/// an Object, the pose that maps it into the result's space, and its
/// `parts[].id` (`null` for a transient compute result). Past `limit`
/// buffer bytes it refuses `mesh_too_large` rather than truncating.
pub(crate) fn mesh_block(
    doc: &kernel::Document,
    parts_in: &[(&kernel::Object, Transform, Value)],
    limit: usize,
) -> Result<Value, CmdError> {
    let mut positions: Vec<f32> = Vec::new();
    let mut normals: Vec<f32> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    let mut face_indices: Vec<u32> = Vec::new();
    let mut parts = Vec::with_capacity(parts_in.len());
    let mut face_base: u32 = 0;
    for (obj, pose, id) in parts_in {
        let (obj, pose) = (*obj, *pose);
        let rm = tessellate::tessellate(obj, doc.materials())
            .map_err(|e| CmdError::Internal(format!("tessellation failed: {e}")))?;
        let face_order: std::collections::BTreeMap<kernel::FaceId, u32> = obj
            .faces()
            .keys()
            .enumerate()
            .map(|(k, f)| (f, k as u32))
            .collect();
        // Front faces only (a painted back lies on its face): one compact
        // vertex run per face, each vertex tagged with its face.
        let mut remap: std::collections::BTreeMap<u32, u32> = std::collections::BTreeMap::new();
        let mut vertex_face: Vec<u32> = Vec::new();
        let vertex_base = (positions.len() / 3) as u32;
        let flip = pose.determinant() < 0.0;
        for &(fid, base, count) in &rm.face_ranges {
            let fi = face_order.get(&fid).copied().unwrap_or(0);
            for v in base..base + count {
                let k = v as usize;
                let pnt = Point3::new(
                    f64::from(rm.positions[k * 3]),
                    f64::from(rm.positions[k * 3 + 1]),
                    f64::from(rm.positions[k * 3 + 2]),
                );
                let nrm = Vec3::new(
                    f64::from(rm.normals[k * 3]),
                    f64::from(rm.normals[k * 3 + 1]),
                    f64::from(rm.normals[k * 3 + 2]),
                );
                let (pnt, nrm) = if pose == Transform::IDENTITY {
                    (pnt, nrm)
                } else {
                    (
                        pose.apply_point(pnt),
                        pose.apply_normal(nrm).normalized().unwrap_or(nrm),
                    )
                };
                remap.insert(v, vertex_base + vertex_face.len() as u32);
                vertex_face.push(fi);
                positions.extend([pnt.x as f32, pnt.y as f32, pnt.z as f32]);
                normals.extend([nrm.x as f32, nrm.y as f32, nrm.z as f32]);
            }
        }
        let index_base = indices.len();
        for tri in rm.front_triangles() {
            let (Some(&a), Some(&b), Some(&c)) =
                (remap.get(&tri[0]), remap.get(&tri[1]), remap.get(&tri[2]))
            else {
                continue;
            };
            if flip {
                indices.extend([a, c, b]);
            } else {
                indices.extend([a, b, c]);
            }
            face_indices.push(face_base + vertex_face[(a - vertex_base) as usize]);
        }
        let bytes =
            positions.len() * 4 + normals.len() * 4 + indices.len() * 4 + face_indices.len() * 4;
        if bytes > limit {
            return Err(mesh_too_large(bytes, limit));
        }
        parts.push(json!({
            "id": id,
            "face_index_base": face_base,
            "index_base": index_base,
            "index_count": indices.len() - index_base,
        }));
        face_base += obj.faces().len() as u32;
    }
    let le = |v: &[f32]| v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>();
    let le32 = |v: &[u32]| v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>();
    Ok(json!({
        "vertex_count": positions.len() / 3,
        "triangle_count": indices.len() / 3,
        "positions_base64": base64(&le(&positions)),
        "normals_base64": base64(&le(&normals)),
        "indices_base64": base64(&le32(&indices)),
        "face_indices_base64": base64(&le32(&face_indices)),
        "parts": parts,
    }))
}

// ------------------------------------------------------- hew.query.topology

fn topology(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let p: TargetParams = parse(params)?;
    no_scope(&p, "hew.query.topology")?;
    let space = space(&p)?;
    let occ = target(ctx, &p)?;
    let NodeId::Object(oid) = occ.node else {
        return Err(CmdError::Refusal(Refusal::api(
            "not_an_object",
            "hew.query.topology reads one object. List a group's or instance's objects with hew.query.occurrences, then ask for each.",
        )));
    };
    let obj = ctx
        .doc
        .object(oid)
        .ok_or_else(|| CmdError::Internal("dangling object behind an occurrence".into()))?;
    let pose = match space {
        Space::World => occ.pose,
        Space::Local => Transform::IDENTITY,
    };
    let id = Value::String(public(ctx, &EntityRef::Object(oid))?);
    Ok(topology_block(ctx.doc, obj, pose, id))
}

/// The `hew.query.topology` result shape (without the clock) for `obj`
/// under `pose`; `object` is its public id, or `null` for a transient
/// compute result.
pub(crate) fn topology_block(
    doc: &kernel::Document,
    obj: &kernel::Object,
    pose: Transform,
    object: Value,
) -> Value {
    let map_p = |q: Point3| pose.apply_point(q);
    let map_n = |n: Vec3| pose.apply_normal(n).normalized().unwrap_or(n);
    let scale = pose.similarity_scale();

    let vertex_index: std::collections::BTreeMap<kernel::VertexId, usize> = obj
        .vertices()
        .keys()
        .enumerate()
        .map(|(k, v)| (v, k))
        .collect();
    let vertices: Vec<Value> = obj
        .vertices()
        .values()
        .map(|v| point_json(map_p(v.position)))
        .collect();
    let face_index: std::collections::BTreeMap<kernel::FaceId, usize> = obj
        .faces()
        .keys()
        .enumerate()
        .map(|(k, f)| (f, k))
        .collect();
    // Loops read counter-clockwise about the returned normal, as the
    // kernel stores them; a mirrored pose reverses their sense, so they are
    // read back reversed (the mesh flips its winding the same way).
    let mirrored = pose.determinant() < 0.0;
    let loop_vertices = |l: kernel::LoopId| -> Vec<usize> {
        let mut v: Vec<usize> = obj
            .loop_half_edges(l)
            .map(|h| vertex_index[&obj.half_edges()[h].origin])
            .collect();
        if mirrored {
            v.reverse();
        }
        v
    };
    let material_id = |m: Option<kernel::MaterialId>| -> Option<String> {
        let m = m?;
        let e = EntityRef::Material(m);
        doc.sid_of(&e).map(|s| crate::ids::public_id(&e, s))
    };
    let mut faces = Vec::with_capacity(obj.faces().len());
    for (k, (fid, f)) in obj.faces().iter().enumerate() {
        let outer: Vec<Point3> = obj.loop_positions(f.outer_loop).map(map_p).collect();
        let holes: Vec<Vec<Point3>> = f
            .inner_loops
            .iter()
            .map(|&l| obj.loop_positions(l).map(map_p).collect())
            .collect();
        let surface = match (f.surface, scale) {
            (
                Some(kernel::SurfaceRef::Cylinder {
                    axis_point,
                    axis,
                    radius,
                }),
                Some(s),
            ) => json!({
                "kind": "cylinder",
                "axis_point": point_json(map_p(axis_point)),
                "axis": vec_json(pose.apply_vector(axis).normalized().unwrap_or(axis)),
                "radius": radius * s,
            }),
            _ => Value::Null,
        };
        faces.push(json!({
            "index": k,
            "plane": {
                "point": point_json(map_p(f.plane.point())),
                "normal": vec_json(map_n(f.plane.normal())),
            },
            "outer": loop_vertices(f.outer_loop),
            "holes": f.inner_loops.iter().map(|&l| loop_vertices(l)).collect::<Vec<_>>(),
            "material": material_id(f.material),
            "back_material": material_id(f.back_material),
            "surface": surface,
            "area": crate::geom::face_area(&outer, &holes),
            "attrs": super::attrs::element_attrs_json(obj.face_attrs(fid)),
        }));
    }
    let mut edges = Vec::with_capacity(obj.edges().len());
    for (k, (eid, e)) in obj.edges().iter().enumerate() {
        let he = &obj.half_edges()[e.half_edge];
        let a = vertex_index[&he.origin];
        let b = vertex_index[&obj.half_edges()[he.next].origin];
        let mut adjacent: Vec<kernel::FaceId> = vec![obj.loops()[he.loop_id].face];
        if let Some(t) = e.twin_half_edge {
            adjacent.push(obj.loops()[obj.half_edges()[t].loop_id].face);
        }
        // A circle's axis: the cylinder it rims, else the face it lies on.
        let curve = match (e.curve, scale) {
            (Some(c), Some(s)) => {
                let axis = adjacent
                    .iter()
                    .find_map(|f| {
                        obj.faces()[*f].surface.map(
                            |kernel::SurfaceRef::Cylinder { axis, .. }| pose.apply_vector(axis),
                        )
                    })
                    .unwrap_or_else(|| pose.apply_normal(obj.faces()[adjacent[0]].plane.normal()));
                json!({
                    "kind": "circle",
                    "center": point_json(map_p(c.center)),
                    "axis": vec_json(axis.normalized().unwrap_or(axis)),
                    "radius": c.radius * s,
                })
            }
            _ => Value::Null,
        };
        edges.push(json!({
            "index": k,
            "a": a,
            "b": b,
            "faces": adjacent.iter().map(|f| face_index[f]).collect::<Vec<_>>(),
            "curve": curve,
            "attrs": super::attrs::element_attrs_json(obj.edge_attrs(eid)),
        }));
    }
    json!({
        "object": object,
        "vertices": vertices,
        "faces": faces,
        "edges": edges,
    })
}

#[cfg(test)]
mod tests {
    use super::{CmdError, Ctx, base64, mesh_limited};
    use kernel::{Document, EntityRef, Plane, Point3};

    /// Past its buffer limit `hew.query.mesh` refuses `mesh_too_large` with
    /// the size it would have been, rather than truncating.
    #[test]
    fn a_mesh_past_the_buffer_limit_refuses_mesh_too_large() {
        let mut doc = Document::new();
        let ground = Plane::from_polygon(&[
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(0.0, 1.0, 0.0),
        ])
        .unwrap();
        let s = doc.add_sketch(ground);
        let sk = doc.sketch_mut(s).unwrap();
        let c = [
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(1.0, 1.0, 0.0),
            Point3::new(0.0, 1.0, 0.0),
        ];
        for k in 0..4 {
            sk.add_segment(c[k], c[(k + 1) % 4]).unwrap();
        }
        let regions = doc.extrudable_regions(s).unwrap();
        let o = doc.extrude_region(s, regions[0], 1.0).unwrap().0;
        let e = EntityRef::Object(o);
        let id = crate::ids::public_id(&e, doc.sid_of(&e).unwrap());
        let mut host = crate::host::NoHost;
        let mut tokens = super::super::FaceTokens::new();
        let grant = crate::grant::Grant::core();
        let mut ctx = Ctx {
            doc: &mut doc,
            host: &mut host,
            face_tokens: &mut tokens,
            current_label: None,
            grant: &grant,
        };
        let params = serde_json::json!({ "id": id });
        // A box: 24 vertices × 24 bytes + 12 triangles × 16 bytes = 768.
        assert!(mesh_limited(&mut ctx, &params, 768).is_ok());
        match mesh_limited(&mut ctx, &params, 767) {
            Err(CmdError::Refusal(r)) => {
                assert_eq!(r.name, "mesh_too_large");
                assert_eq!(r.detail["bytes"], 768);
            }
            other => panic!("expected mesh_too_large, got {other:?}"),
        }
    }

    #[test]
    fn base64_is_standard_with_padding() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64(&[0xff, 0xfe]), "//4=");
    }
}

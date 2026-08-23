//! Appearance + organization: hew.material.*, hew.tag.*, hew.guide.*
//! (docs/agents/HEW_API.md §7).
//!
//! `hew.material.create`/`hew.tag.create`/`hew.tag.set_visible` call
//! kernel entry points that are deliberately **not undoable**
//! (`Document::add_material`, `Document::set_tag_hidden` —
//! docs/design/api-kernel-map.md §1.9/§1.10): a palette addition or a
//! tag's hidden-by-default flag is view/registry state, not a modeled
//! edit. These three commands are the conformance suite's documented
//! allowlist entry for the one-envelope-one-undo property.

use super::entity::{resolve_node, unknown_entity};
use super::{CmdError, Ctx, Handler};
use crate::locate;
use crate::refusal::Refusal;
use kernel::{EntityRef, Material, Rgba8, Transform};
use serde_json::Value;

/// This namespace's slice of the handler table.
pub fn handler(name: &str) -> Option<Handler> {
    Some(match name {
        "hew.material.create" => material_create,
        "hew.material.paint" => material_paint,
        "hew.material.set_default" => material_set_default,
        "hew.material.set_opacity" => material_set_opacity,
        "hew.tag.create" => tag_create,
        "hew.tag.assign" => tag_assign,
        "hew.tag.set_visible" => tag_set_visible,
        "hew.tag.delete" => tag_delete,
        "hew.tag.rename" => tag_rename,
        "hew.guide.line" => guide_line,
        "hew.guide.point" => guide_point,
        "hew.guide.angular" => guide_angular,
        "hew.guide.clear" => guide_clear,
        _ => return None,
    })
}

/// Resolves a material's public id, refusing typed on the wrong kind or a
/// dangling id.
fn resolve_material(ctx: &Ctx, id: &str) -> Result<kernel::MaterialId, CmdError> {
    match ctx.resolver().resolve(id) {
        Some(EntityRef::Material(m)) => Ok(m),
        _ => Err(unknown_entity(id)),
    }
}

fn resolve_object(ctx: &Ctx, id: &str) -> Result<kernel::ObjectId, CmdError> {
    match ctx.resolver().resolve(id) {
        Some(EntityRef::Object(o)) => Ok(o),
        _ => Err(unknown_entity(id)),
    }
}

// -------------------------------------------------------------- material

fn parse_rgba(color: &[Value]) -> Result<Rgba8, CmdError> {
    if color.len() != 3 && color.len() != 4 {
        return Err(CmdError::Params(
            "color must be [r,g,b] or [r,g,b,a], each 0-255".into(),
        ));
    }
    let mut c = [255_u8; 4];
    for (i, v) in color.iter().enumerate() {
        let n = v
            .as_u64()
            .filter(|n| *n <= 255)
            .ok_or_else(|| CmdError::Params("color channels must be integers 0-255".into()))?;
        c[i] = n as u8;
    }
    Ok(Rgba8::rgba(c[0], c[1], c[2], c[3]))
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct MaterialCreateParams {
    name: String,
    color: Vec<Value>,
}

fn material_create(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let p: MaterialCreateParams =
        serde_json::from_value(params.clone()).map_err(|e| CmdError::Params(e.to_string()))?;
    let color = parse_rgba(&p.color)?;
    let id = ctx.doc.add_material(Material::solid(p.name, color));
    let resolver = ctx.resolver();
    let public = resolver
        .public_of(ctx.doc, &EntityRef::Material(id))
        .expect("just created");
    Ok(serde_json::json!({ "material": public }))
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct PaintParams {
    #[serde(default)]
    face: Option<Value>,
    #[serde(default)]
    id: Option<String>,
    material: Option<String>,
}

fn resolve_optional_material(
    ctx: &Ctx,
    material: &Option<String>,
) -> Result<Option<kernel::MaterialId>, CmdError> {
    material
        .as_deref()
        .map(|id| resolve_material(ctx, id))
        .transpose()
}

fn material_paint(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let p: PaintParams =
        serde_json::from_value(params.clone()).map_err(|e| CmdError::Params(e.to_string()))?;
    let material = resolve_optional_material(ctx, &p.material)?;
    match (&p.face, &p.id) {
        (Some(face), None) => {
            let face = locate::resolve_face(ctx, face)?;
            ctx.doc.paint_face(face.object, face.face, material)?;
        }
        (None, Some(id)) => {
            let object = resolve_object(ctx, id)?;
            ctx.doc.set_object_material(object, material)?;
        }
        _ => {
            return Err(CmdError::Params(
                "material.paint needs exactly one of \"face\" or \"id\"".into(),
            ));
        }
    }
    Ok(serde_json::json!({}))
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct SetDefaultParams {
    id: String,
    material: Option<String>,
}

fn material_set_default(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let p: SetDefaultParams =
        serde_json::from_value(params.clone()).map_err(|e| CmdError::Params(e.to_string()))?;
    let object = resolve_object(ctx, &p.id)?;
    let material = resolve_optional_material(ctx, &p.material)?;
    ctx.doc.set_object_material(object, material)?;
    Ok(serde_json::json!({}))
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct SetOpacityParams {
    material: String,
    alpha: u16,
}

fn material_set_opacity(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let p: SetOpacityParams =
        serde_json::from_value(params.clone()).map_err(|e| CmdError::Params(e.to_string()))?;
    if p.alpha > 255 {
        return Err(CmdError::Params("alpha must be 0-255".into()));
    }
    let material = resolve_material(ctx, &p.material)?;
    ctx.doc.set_material_alpha(material, p.alpha as u8)?;
    Ok(serde_json::json!({}))
}

// -------------------------------------------------------------------- tag

fn validate_tag_path(path: &[String]) -> Result<(), CmdError> {
    if path.is_empty() || path.iter().any(String::is_empty) {
        return Err(CmdError::Params(
            "tag path must be a non-empty list of non-empty segments".into(),
        ));
    }
    Ok(())
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct TagCreateParams {
    path: Vec<String>,
    #[serde(default)]
    hidden: Option<bool>,
}

fn tag_create(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let p: TagCreateParams =
        serde_json::from_value(params.clone()).map_err(|e| CmdError::Params(e.to_string()))?;
    validate_tag_path(&p.path)?;
    ctx.doc
        .set_tag_hidden(p.path.clone(), p.hidden.unwrap_or(false));
    let resolver = ctx.resolver();
    let public = resolver
        .public_of(ctx.doc, &EntityRef::Tag(p.path))
        .expect("just registered");
    Ok(serde_json::json!({ "tag": public }))
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct TagAssignParams {
    id: String,
    path: Vec<String>,
    #[serde(default)]
    remove: bool,
}

fn tag_assign(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let p: TagAssignParams =
        serde_json::from_value(params.clone()).map_err(|e| CmdError::Params(e.to_string()))?;
    validate_tag_path(&p.path)?;
    let node = resolve_node(ctx, &p.id)?;
    if p.remove {
        ctx.doc.remove_node_tag(node, &p.path)?;
    } else {
        ctx.doc.add_node_tag(node, p.path)?;
    }
    Ok(serde_json::json!({}))
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct TagSetVisibleParams {
    path: Vec<String>,
    visible: bool,
}

fn tag_set_visible(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let p: TagSetVisibleParams =
        serde_json::from_value(params.clone()).map_err(|e| CmdError::Params(e.to_string()))?;
    validate_tag_path(&p.path)?;
    ctx.doc.set_tag_hidden(p.path, !p.visible);
    Ok(serde_json::json!({}))
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct TagPathParams {
    path: Vec<String>,
}

fn tag_delete(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let p: TagPathParams =
        serde_json::from_value(params.clone()).map_err(|e| CmdError::Params(e.to_string()))?;
    validate_tag_path(&p.path)?;
    // `Document::delete_tag` on an unregistered path is a silent `Ok`
    // no-op (docs/design/api-kernel-map.md §1.10) — the API refuses
    // instead of pretending nothing was asked for.
    let covers = |tag: &[String]| tag.len() >= p.path.len() && tag[..p.path.len()] == p.path[..];
    let known = ctx.doc.tag_meta().any(|(tag, _)| covers(tag));
    if !known {
        return Err(CmdError::Refusal(Refusal::api(
            "unknown_tag",
            &format!("No tag is registered at or under '{}'.", p.path.join("/")),
        )));
    }
    ctx.doc.delete_tag(&p.path)?;
    Ok(serde_json::json!({}))
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct TagRenameParams {
    path: Vec<String>,
    new_path: Vec<String>,
}

/// `hew.tag.rename` — the Tags panel's in-place rename: move `path` (and
/// every tag nested under it) to `new_path`, keeping the tag's identity
/// (stable id, hidden flag, attributes; a Scene's captured hidden tags
/// follow). Undoable, one entry. Refuses an unknown source (mirroring
/// `hew.tag.delete`), a collision (`duplicate_tag`), and an empty or
/// self-nested target (`invalid_tag_path`) — never a silent no-op.
fn tag_rename(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let p: TagRenameParams =
        serde_json::from_value(params.clone()).map_err(|e| CmdError::Params(e.to_string()))?;
    validate_tag_path(&p.path)?;
    validate_tag_path(&p.new_path)?;
    let covers = |tag: &[String]| tag.len() >= p.path.len() && tag[..p.path.len()] == p.path[..];
    let registered = ctx.doc.tag_meta().any(|(tag, _)| covers(tag));
    let carried = !registered && {
        let doc = &ctx.doc;
        let mut nodes: Vec<kernel::NodeId> = Vec::new();
        nodes.extend(
            doc.visible_object_ids()
                .into_iter()
                .map(kernel::NodeId::Object),
        );
        nodes.extend(doc.group_ids().into_iter().map(kernel::NodeId::Group));
        nodes.extend(doc.instance_ids().into_iter().map(kernel::NodeId::Instance));
        nodes
            .into_iter()
            .any(|n| doc.node_tags(n).iter().any(|t| covers(t)))
    };
    if !registered && !carried {
        return Err(CmdError::Refusal(Refusal::api(
            "unknown_tag",
            &format!("No tag is registered at or under '{}'.", p.path.join("/")),
        )));
    }
    ctx.doc.rename_tag(&p.path, p.new_path)?;
    Ok(serde_json::json!({}))
}

// ----------------------------------------------------------------- guide

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct GuideLineParams {
    origin: Value,
    direction: Value,
}

fn guide_line(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let p: GuideLineParams =
        serde_json::from_value(params.clone()).map_err(|e| CmdError::Params(e.to_string()))?;
    let origin = locate::resolve_point(ctx, &p.origin)?;
    let direction = locate::parse_dir(&p.direction)?;
    let id = ctx.doc.add_guide_line(origin, direction)?;
    let resolver = ctx.resolver();
    let public = resolver
        .public_of(ctx.doc, &EntityRef::Guide(id))
        .expect("just created");
    Ok(serde_json::json!({ "guide": public }))
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct GuidePointParams {
    position: Value,
}

fn guide_point(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let p: GuidePointParams =
        serde_json::from_value(params.clone()).map_err(|e| CmdError::Params(e.to_string()))?;
    let position = locate::resolve_point(ctx, &p.position)?;
    let id = ctx.doc.add_guide_point(position)?;
    let resolver = ctx.resolver();
    let public = resolver
        .public_of(ctx.doc, &EntityRef::Guide(id))
        .expect("just created");
    Ok(serde_json::json!({ "guide": public }))
}

/// The kernel's `Guide` has exactly two variants, Line and Point
/// (docs/design/api-kernel-map.md §1.11) — there is no angular guide.
/// `hew.guide.angular` needs no kernel change: it is protractor
/// semantics composed entirely in the api layer — `base_dir` projected
/// into the plane named by `plane_normal`, rotated by `angle` about
/// `plane_normal` (Rodrigues, via `kernel::Transform::rotation`), and
/// the resulting direction handed to `Document::add_guide_line` exactly
/// as `hew.guide.line` does.
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct GuideAngularParams {
    origin: Value,
    plane_normal: Value,
    base_dir: Value,
    angle: f64,
}

fn guide_angular(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let p: GuideAngularParams =
        serde_json::from_value(params.clone()).map_err(|e| CmdError::Params(e.to_string()))?;
    if !p.angle.is_finite() {
        return Err(CmdError::Params("angle must be a finite number".into()));
    }
    let origin = locate::resolve_point(ctx, &p.origin)?;
    let plane_normal = locate::parse_dir(&p.plane_normal)?;
    let base_dir = locate::parse_dir(&p.base_dir)?;

    // Project base_dir into the plane perpendicular to plane_normal.
    // `parse_dir` already refused a plane_normal shorter than
    // `kernel::tol::NORMALIZE_MIN_LENGTH`, the exact threshold
    // `Vec3::normalized`/`Transform::rotation` gate on, so both are
    // unreachable here — kept as an internal assertion, not a params
    // refusal, so a drifting tolerance fails loudly instead of silently
    // misreporting the params as bad.
    let normal_unit = plane_normal
        .normalized()
        .map_err(|_| CmdError::Internal("plane_normal normalize should not fail here".into()))?;
    let in_plane = base_dir - normal_unit * base_dir.dot(normal_unit);
    if in_plane.length_squared() < 1e-24 {
        return Err(CmdError::Params(
            "base_dir is parallel to plane_normal — it has no projection into that plane".into(),
        ));
    }

    // Rotate the projected direction about plane_normal by angle
    // (Rodrigues); `Transform::rotation` normalizes the axis itself.
    let rotation = Transform::rotation(plane_normal, p.angle)
        .map_err(|_| CmdError::Internal("plane_normal rotation should not fail here".into()))?;
    let direction = rotation.apply_vector(in_plane);

    let id = ctx.doc.add_guide_line(origin, direction)?;
    let resolver = ctx.resolver();
    let public = resolver
        .public_of(ctx.doc, &EntityRef::Guide(id))
        .expect("just created");
    Ok(serde_json::json!({ "guide": public }))
}

fn guide_clear(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    #[derive(Debug, serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct ClearParams {}
    let _: ClearParams =
        serde_json::from_value(params.clone()).map_err(|e| CmdError::Params(e.to_string()))?;
    ctx.doc.delete_all_guides()?;
    Ok(serde_json::json!({}))
}

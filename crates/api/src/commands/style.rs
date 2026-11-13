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
use kernel::{EntityRef, ImageFormat, Material, Point3, Rgba8, Texture, Transform, UvFrame, Vec3};
use serde_json::Value;

/// This namespace's slice of the handler table.
pub fn handler(name: &str) -> Option<Handler> {
    Some(match name {
        "hew.material.create" => material_create,
        "hew.material.paint" => material_paint,
        "hew.material.set_default" => material_set_default,
        "hew.material.set_opacity" => material_set_opacity,
        "hew.material.set_pbr" => material_set_pbr,
        "hew.material.position_texture" => material_position_texture,
        "hew.material.replace" => material_replace,
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
    #[serde(default)]
    color: Option<Vec<Value>>,
    #[serde(default)]
    texture: Option<TextureParams>,
}

/// An image texture, the API's form of the palette's "Add texture": the
/// image (inline, or a file the host reads) and the real-world size one
/// tile covers.
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct TextureParams {
    #[serde(default)]
    bytes_base64: Option<String>,
    #[serde(default)]
    path: Option<String>,
    world_size: [f64; 2],
}

/// The codec of encoded image bytes, from their signature. The kernel
/// never decodes an image (material.rs), so this is the whole check: PNG
/// and JPEG are what the renderer and file format carry.
fn sniff_image_format(bytes: &[u8]) -> Option<ImageFormat> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(ImageFormat::Png)
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some(ImageFormat::Jpeg)
    } else {
        None
    }
}

fn parse_texture(ctx: &mut Ctx, t: TextureParams) -> Result<Texture, CmdError> {
    let [w, h] = t.world_size;
    if !(w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0) {
        return Err(CmdError::Params(
            "texture.world_size must be two positive lengths in meters".into(),
        ));
    }
    let image = match (t.bytes_base64, t.path) {
        (Some(b64), None) => super::doc::decode_base64(&b64)
            .ok_or_else(|| CmdError::Params("texture.bytes_base64 is not valid base64".into()))?,
        (None, Some(path)) => ctx.host.read_texture(&path).map_err(CmdError::Refusal)?,
        _ => {
            return Err(CmdError::Params(
                "texture needs exactly one of \"bytes_base64\" or \"path\"".into(),
            ));
        }
    };
    let format = sniff_image_format(&image)
        .ok_or_else(|| CmdError::Params("texture image must be a PNG or a JPEG".into()))?;
    Ok(Texture {
        image,
        format,
        world_size: [w, h],
    })
}

fn material_create(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let p: MaterialCreateParams =
        serde_json::from_value(params.clone()).map_err(|e| CmdError::Params(e.to_string()))?;
    // A texture is shown as-is under white, as the palette's "Add texture"
    // makes it; a color given with one tints it (the kernel modulates).
    let color = match (&p.color, &p.texture) {
        (Some(c), _) => parse_rgba(c)?,
        (None, Some(_)) => Rgba8::rgba(255, 255, 255, 255),
        (None, None) => {
            return Err(CmdError::Params(
                "material.create needs \"color\", \"texture\", or both".into(),
            ));
        }
    };
    let material = match p.texture {
        Some(t) => Material::textured(p.name, color, parse_texture(ctx, t)?),
        None => Material::solid(p.name, color),
    };
    let id = ctx.doc.add_material(material);
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
    /// For an instance id: paint the placement (`instance`, the default)
    /// or its definition's geometry (`definition`). A definition id takes
    /// only `definition`; nothing else takes a scope.
    #[serde(default)]
    scope: Option<String>,
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
        (Some(_), None) if p.scope.is_some() => {
            return Err(CmdError::Params(
                "scope applies to an instance or definition id, not a face".into(),
            ));
        }
        (Some(face), None) => {
            let face = locate::resolve_face(ctx, face)?;
            ctx.doc.paint_face(face.object, face.face, material)?;
        }
        (None, Some(id)) => match (ctx.resolver().resolve(id), p.scope.as_deref()) {
            (Some(EntityRef::Object(object)), None) => {
                ctx.doc.set_object_material(object, material)?;
            }
            (Some(EntityRef::Instance(instance)), None | Some("instance")) => {
                ctx.doc.set_instance_material(instance, material)?;
            }
            (Some(EntityRef::Instance(instance)), Some("definition")) => {
                let def = ctx
                    .doc
                    .instance_def(instance)
                    .ok_or_else(|| unknown_entity(id))?;
                ctx.doc.set_definition_material(def, material)?;
            }
            (Some(EntityRef::Component(def)), None | Some("definition")) => {
                ctx.doc.set_definition_material(def, material)?;
            }
            (
                Some(EntityRef::Object(_) | EntityRef::Instance(_) | EntityRef::Component(_)),
                Some(scope),
            ) => {
                return Err(CmdError::Params(format!(
                    "scope \"{scope}\" does not apply to {id}: an instance takes \"instance\" or \"definition\", a definition only \"definition\", an object none"
                )));
            }
            _ => return Err(unknown_entity(id)),
        },
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

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct SetPbrParams {
    material: String,
    metallic: f64,
    roughness: f64,
}

fn material_set_pbr(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let p: SetPbrParams =
        serde_json::from_value(params.clone()).map_err(|e| CmdError::Params(e.to_string()))?;
    let unit = |v: f64| v.is_finite() && (0.0..=1.0).contains(&v);
    if !unit(p.metallic) || !unit(p.roughness) {
        return Err(CmdError::Params(
            "metallic and roughness must each be from 0 to 1".into(),
        ));
    }
    let material = resolve_material(ctx, &p.material)?;
    ctx.doc
        .set_material_pbr(material, p.metallic as f32, p.roughness as f32)?;
    Ok(serde_json::json!({}))
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct PositionTextureParams {
    face: Value,
    #[serde(default)]
    pins: Option<Pins>,
    #[serde(default)]
    reset: Option<bool>,
}

/// The Position Texture tool's three pins: corners of one texture tile laid
/// on the face, in the face's object frame (the frame a face locator's
/// `at` uses). `u` is one tile along the texture's width from `origin`,
/// `v` one tile along its height.
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Pins {
    origin: [f64; 3],
    u: [f64; 3],
    v: [f64; 3],
}

fn degenerate_pins() -> CmdError {
    CmdError::Refusal(Refusal::api(
        "degenerate_pins",
        "The three pins don't span a tile on the face. Place origin, u and v at three corners of one tile, not in a line.",
    ))
}

/// The UV frame that puts tile corner (0, 0) at `origin`, (1, 0) at `u` and
/// (0, 1) at `v`, after projecting the pins onto the face's plane (as the
/// tool's pins are constrained to it). Solves `s·(u−o) = 1, s·(v−o) = 0,
/// s·n = 0` and the same for `t`, so both gradients lie in the plane.
fn frame_from_pins(pins: &Pins, normal: Vec3, plane_point: Point3) -> Result<UvFrame, CmdError> {
    let p = |a: [f64; 3]| Point3::new(a[0], a[1], a[2]);
    let project = |q: Point3| q - normal * (q - plane_point).dot(normal);
    let (o, pu, pv) = (
        project(p(pins.origin)),
        project(p(pins.u)),
        project(p(pins.v)),
    );
    let (a, b, n) = (pu - o, pv - o, normal);
    // Rows a, b, n; its inverse's columns are (b×n, n×a, a×b) / det.
    let det = a.dot(b.cross(n));
    if a.length() < 1e-9
        || b.length() < 1e-9
        || !det.is_finite()
        || det.abs() <= 1e-9 * a.length() * b.length()
    {
        return Err(degenerate_pins());
    }
    let s = b.cross(n) * (1.0 / det);
    let t = n.cross(a) * (1.0 / det);
    let ov = Vec3::new(o.x, o.y, o.z);
    let frame = UvFrame::new(s, t, -s.dot(ov), -t.dot(ov));
    if !frame.is_valid() {
        return Err(degenerate_pins());
    }
    Ok(frame)
}

fn material_position_texture(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    let p: PositionTextureParams =
        serde_json::from_value(params.clone()).map_err(|e| CmdError::Params(e.to_string()))?;
    let face = locate::resolve_face(ctx, &p.face)?;
    let frame = match (p.pins, p.reset) {
        (Some(pins), None | Some(false)) => {
            let f = &ctx
                .doc
                .object(face.object)
                .ok_or_else(|| CmdError::Internal("dangling object behind a resolved face".into()))?
                .faces()[face.face];
            Some(frame_from_pins(&pins, f.plane.normal(), f.plane.point())?)
        }
        (None, Some(true)) => None,
        _ => {
            return Err(CmdError::Params(
                "material.position_texture needs exactly one of \"pins\" or \"reset\": true".into(),
            ));
        }
    };
    // Already so (a reset of an unpositioned face, the same pins again):
    // nothing changes, so nothing is recorded, as hew.doc.purge_unused does.
    if ctx.doc.face_uv_frame(face.object, face.face) == Some(frame) {
        return Ok(serde_json::json!({}));
    }
    ctx.doc.set_face_uv_frame(face.object, face.face, frame)?;
    Ok(serde_json::json!({}))
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ReplaceParams {
    from: Option<String>,
    to: Option<String>,
    #[serde(default)]
    object: Option<String>,
}

/// The Paint tool's Shift-click: every face and object base painted `from`
/// becomes `to` (`null` is unpainted), across the document, or within one
/// object as Ctrl/Cmd+Shift-click does.
fn material_replace(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    // `null` is a value here (unpainted), so a missing key is not one.
    for key in ["from", "to"] {
        if params.get(key).is_none() {
            return Err(CmdError::Params(format!(
                "material.replace needs \"{key}\" (a material id, or null for unpainted)"
            )));
        }
    }
    let p: ReplaceParams =
        serde_json::from_value(params.clone()).map_err(|e| CmdError::Params(e.to_string()))?;
    let from = resolve_optional_material(ctx, &p.from)?;
    let to = resolve_optional_material(ctx, &p.to)?;
    let scope = match &p.object {
        Some(id) => kernel::MaterialScope::Object(resolve_object(ctx, id)?),
        None => kernel::MaterialScope::Document,
    };
    ctx.doc.replace_material(scope, from, to)?;
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

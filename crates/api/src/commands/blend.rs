//! `hew.*` commands of the true-curves effort (blend): `hew.solid.fillet`,
//! `hew.solid.chamfer`, `hew.solid.resize_blend` and `hew.solid.remove_blend`.
//! Declarations live in `registry.rs`; this module owns the handlers.
//!
//! Edges arrive as locators (01-conventions §7.1, `locate::resolve_edge_set`)
//! because solid edges have no public ids. Each located edge is expanded
//! with `Object::logical_edge` (tangent continuations unless
//! `"propagate": "none"`) before the kernel call, exactly as the Fillet tool
//! expands a click. Every length is meters and every angle radians.

use super::{CmdError, Ctx, Handler};
use crate::geom;
use crate::locate;
use crate::refusal::Refusal;
use kernel::{
    BlendProfile, BlendSpec, BlendStops, CornerProfile, CornerStyle, EdgeId, EntityRef, ObjectId,
    ProfileShape, RadiusHandle,
};
use serde_json::{Value, json};
use std::f64::consts::{FRAC_PI_2, PI};

/// This namespace's slice of the handler table.
pub fn handler(name: &str) -> Option<Handler> {
    Some(match name {
        "hew.solid.fillet" => fillet,
        "hew.solid.chamfer" => chamfer,
        "hew.solid.resize_blend" => resize_blend,
        "hew.solid.remove_blend" => remove_blend,
        _ => return None,
    })
}

// ------------------------------------------------------------- plumbing

fn parse_params<T: serde::de::DeserializeOwned>(params: &Value) -> Result<T, CmdError> {
    serde_json::from_value(params.clone()).map_err(|e| CmdError::Params(e.to_string()))
}

fn public_id_of(ctx: &Ctx, entity: EntityRef) -> String {
    let sid = ctx
        .doc
        .sid_of(&entity)
        .expect("an entity this command just touched always carries a stable id");
    crate::ids::public_id(&entity, sid)
}

fn unknown_entity(id: &str) -> CmdError {
    CmdError::Refusal(
        Refusal::api(
            "unknown_entity",
            &format!("'{id}' does not name a live entity of the required kind in this document."),
        )
        .with_detail(json!({ "id": id })),
    )
}

fn unknown_profile(name: &str) -> CmdError {
    CmdError::Refusal(
        Refusal::api(
            "unknown_profile",
            &format!(
                "'{name}' is not a profile. Use \"round\", \"cove\", \"bullnose\", a built-in id \
                 such as \"builtin:ogee\", or the id of a profile in the Library."
            ),
        )
        .with_detail(json!({ "profile": name })),
    )
}

fn finite(name: &str, v: f64) -> Result<f64, CmdError> {
    if v.is_finite() {
        Ok(v)
    } else {
        Err(CmdError::Params(format!("{name} must be a finite number")))
    }
}

fn need(name: &str, v: Option<f64>) -> Result<f64, CmdError> {
    finite(
        name,
        v.ok_or_else(|| CmdError::Params(format!("{name} is required")))?,
    )
}

fn propagate_flag(p: Option<&str>) -> Result<bool, CmdError> {
    match p {
        None | Some("tangent") => Ok(true),
        Some("none") => Ok(false),
        Some(other) => Err(CmdError::Params(format!(
            "propagate must be \"tangent\" or \"none\", not {other:?}"
        ))),
    }
}

/// The located edges expanded to logical edges (the chain a click on each
/// would blend), deduplicated in order. An edge that is not sharp stays as
/// given, so the kernel answers `edge_is_smooth` for it.
fn expand_edges(
    ctx: &Ctx,
    object: ObjectId,
    edges: &[EdgeId],
    propagate: bool,
) -> Result<Vec<EdgeId>, CmdError> {
    let obj = ctx
        .doc
        .object(object)
        .ok_or_else(|| unknown_entity("object"))?;
    let mut out: Vec<EdgeId> = Vec::new();
    for &e in edges {
        let chain = obj.picked_edge(e, propagate);
        let chain = if chain.is_empty() { vec![e] } else { chain };
        for x in chain {
            if !out.contains(&x) {
                out.push(x);
            }
        }
    }
    Ok(out)
}

fn stops_of(v: &Option<Value>) -> Result<Option<BlendStops>, CmdError> {
    let Some(v) = v else { return Ok(None) };
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct S {
        #[serde(default)]
        start: f64,
        #[serde(default)]
        end: f64,
    }
    let s: S = parse_params(v)?;
    Ok(Some(BlendStops {
        start: finite("stops.start", s.start)?,
        end: finite("stops.end", s.end)?,
    }))
}

fn handles_of(v: &Option<Vec<Value>>) -> Result<Vec<RadiusHandle>, CmdError> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct H {
        at: f64,
        radius: f64,
    }
    let mut out = Vec::new();
    for h in v.iter().flatten() {
        let h: H = parse_params(h)?;
        out.push(RadiusHandle {
            at: finite("handles.at", h.at)?,
            radius: finite("handles.radius", h.radius)?,
        });
    }
    Ok(out)
}

/// The largest normalized arc radius in a profile shape (1 when it has no
/// arcs), which scales the default facet count of a custom profile.
fn shape_arc_radius(shape: &ProfileShape) -> f64 {
    let mut prev = [1.0, 0.0];
    let mut best = 0.0_f64;
    for seg in &shape.segs {
        match *seg {
            kernel::ProfileSeg::Line { to } => prev = to,
            kernel::ProfileSeg::Arc { to, center, .. } => {
                best = best
                    .max(((prev[0] - center[0]).powi(2) + (prev[1] - center[1]).powi(2)).sqrt());
                prev = to;
            }
        }
    }
    if best > 0.0 { best } else { 1.0 }
}

/// Facets per quarter turn of a custom profile's arcs at drawn width
/// `width`: `max(2, draw_segments_per_turn(width · r_arc_max) / 4)`.
pub fn custom_arc_segments(shape: &ProfileShape, width: f64) -> u32 {
    (geom::draw_segments_per_turn(width * shape_arc_radius(shape)) / 4).max(2)
}

/// A named profile's shape: a built-in id (`"builtin:ogee"`, or the bare
/// `"ogee"`). Library items are resolved by the caller's host in a later
/// step; any other name is `unknown_profile`.
fn named_shape(name: &str) -> Option<ProfileShape> {
    let want = if name.starts_with("builtin:") {
        name.to_string()
    } else {
        format!("builtin:{name}")
    };
    kernel::bundled_profiles()
        .iter()
        .find(|(id, _, _)| *id == want)
        .map(|(_, _, make)| make())
}

/// The profile named by a fillet's `profile` parameter, sized.
#[allow(clippy::too_many_arguments)]
fn fillet_profile(
    ctx: &Ctx,
    name: Option<&str>,
    radius: Option<f64>,
    width: Option<f64>,
    segments: Option<u32>,
    theta: f64,
    bullnose_limit: impl FnOnce(u32) -> Option<f64>,
) -> Result<BlendProfile, CmdError> {
    match name.unwrap_or("round") {
        "round" => {
            let radius = need("radius", radius)?;
            Ok(BlendProfile::Round {
                radius,
                segments: segments.unwrap_or_else(|| geom::arc_segments(radius, PI - theta)),
            })
        }
        "cove" => {
            let radius = need("radius", radius)?;
            Ok(BlendProfile::Cove {
                radius,
                segments: segments.unwrap_or_else(|| geom::arc_segments(radius, PI - theta)),
            })
        }
        "bullnose" => Ok(BlendProfile::Bullnose {
            segments: segments.unwrap_or_else(|| {
                // A bullnose's size is fixed by the face width: its limit is
                // half of it, which is the half circle's radius.
                bullnose_limit(12).map_or(12, |r| geom::arc_segments(r, PI))
            }),
        }),
        other => {
            let (shape, drawn) = profile_shape(ctx, other)?;
            let width = need("width", width.or(radius).or(drawn))?;
            Ok(BlendProfile::Custom {
                arc_segments: segments.unwrap_or_else(|| custom_arc_segments(&shape, width)),
                shape,
                width,
            })
        }
    }
}

/// Resolves a profile name to a shape and, for a Library item, its drawn
/// width: a built-in id or a Library profile (by item id or path).
fn profile_shape(ctx: &Ctx, name: &str) -> Result<(ProfileShape, Option<f64>), CmdError> {
    if let Some(shape) = named_shape(name) {
        return Ok((shape, None));
    }
    if name.starts_with("builtin:") {
        return Err(unknown_profile(name));
    }
    let (shape, width) = super::library::library_profile(ctx, name)?;
    Ok((shape, Some(width)))
}

fn corner_style(kind: &str, given: Option<&str>) -> Result<CornerStyle, CmdError> {
    match (kind, given) {
        (_, None) => Ok(CornerStyle::Default),
        ("fillet", Some("rolled")) | ("chamfer", Some("mitered")) => Ok(CornerStyle::Default),
        ("fillet", Some("mitered")) => Ok(CornerStyle::Mitered),
        ("chamfer", Some("flat")) => Ok(CornerStyle::Flat),
        (kind, Some(other)) => Err(CmdError::Params(format!(
            "corners {other:?} is not valid for a {kind}"
        ))),
    }
}

fn blend_result(ctx: &Ctx, object: ObjectId, report: &kernel::BlendReport) -> Value {
    json!({
        "object_id": public_id_of(ctx, EntityRef::Object(object)),
        "blend_ids": report.blends.iter().map(|b| b.0).collect::<Vec<_>>(),
        "faces": report.new_faces.len(),
        "size": report.profile.size(),
    })
}

/// The dihedral of the first edge (π/2 when unreadable).
fn first_dihedral(ctx: &Ctx, object: ObjectId, edges: &[EdgeId]) -> f64 {
    ctx.doc
        .object(object)
        .and_then(|o| edges.first().and_then(|&e| o.edge_dihedral(e)))
        .unwrap_or(FRAC_PI_2)
}

// ------------------------------------------------------------- 2D corners

/// Blends one 2D corner: a world sketch's (`{"sketch": id, "at": p}`) or an
/// imprint's on a face (`{"face": <locator>, "at": p}`).
fn blend_corner(ctx: &mut Ctx, corner: &Value, profile: CornerProfile) -> Result<Value, CmdError> {
    let obj = corner
        .as_object()
        .ok_or_else(|| CmdError::Params("corner needs an object".into()))?;
    let at = locate::parse_xyz(
        obj.get("at")
            .ok_or_else(|| CmdError::Params("corner needs \"at\"".into()))?,
    )?;
    if let Some(sketch_pub) = obj.get("sketch").and_then(Value::as_str) {
        let Some(EntityRef::Sketch(sketch)) = ctx.resolver().resolve(sketch_pub) else {
            return Err(unknown_entity(sketch_pub));
        };
        let s = ctx
            .doc
            .sketch(sketch)
            .ok_or_else(|| unknown_entity(sketch_pub))?;
        let vertex = s
            .vertices()
            .iter()
            .map(|(id, v)| (id, (v.position - at).length()))
            .filter(|&(_, d)| d <= geom::API_SURFACE_TOL)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(id, _)| id)
            .ok_or_else(|| {
                CmdError::Refusal(Refusal::api(
                    "locator_missed",
                    "No sketch corner lies at that point.",
                ))
            })?;
        ctx.doc.sketch_blend_corner(sketch, vertex, profile)?;
        return Ok(json!({ "sketch_id": sketch_pub }));
    }
    if let Some(face_loc) = obj.get("face") {
        let f = locate::resolve_face(ctx, face_loc)?;
        let dc = super::draft::resolve_context(ctx, &json!({ "face": face_loc }))?;
        ctx.doc
            .begin_draft(dc.plane, dc.frame, &dc.objects, "Round corner")?;
        let outcome = ctx.doc.draft_check_corner(at, &profile).and_then(|v| {
            ctx.doc
                .draft_sketch_mut()
                .ok_or(kernel::SketchError::UnknownVertex)?
                .blend_corner(v, profile)
                .map(|_| ())
        });
        if let Err(e) = outcome {
            ctx.doc.cancel_draft();
            return Err(CmdError::from(kernel::DocumentError::Sketch(e)));
        }
        if let Err(e) = ctx.doc.end_draft() {
            ctx.doc.cancel_draft();
            return Err(CmdError::from(e));
        }
        return Ok(json!({ "object_id": public_id_of(ctx, EntityRef::Object(f.object)) }));
    }
    Err(CmdError::Params(
        "corner needs \"sketch\" or \"face\" with \"at\"".into(),
    ))
}

/// The segment count of a rounded 2D corner: the drawing tools' count for a
/// quarter turn at `radius` unless given, kept in the corner's 1..=1024.
fn corner_segments(radius: f64, given: Option<u32>) -> u32 {
    given
        .unwrap_or_else(|| geom::arc_segments(radius, FRAC_PI_2))
        .clamp(1, 1024)
}

// ------------------------------------------------------------- fillet

/// `hew.solid.fillet`: rounds, coves, bullnoses or profiles edges (or
/// rounds a 2D corner).
fn fillet(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct P {
        #[serde(default)]
        edges: Option<Vec<Value>>,
        #[serde(default)]
        corner: Option<Value>,
        #[serde(default)]
        radius: Option<f64>,
        #[serde(default)]
        segments: Option<u32>,
        #[serde(default)]
        profile: Option<String>,
        #[serde(default)]
        width: Option<f64>,
        #[serde(default)]
        propagate: Option<String>,
        #[serde(default)]
        stops: Option<Value>,
        #[serde(default)]
        handles: Option<Vec<Value>>,
        #[serde(default)]
        corners: Option<String>,
    }
    let p: P = parse_params(params)?;
    if let Some(corner) = &p.corner {
        if p.edges.is_some() {
            return Err(CmdError::Params(
                "give either edges or corner, not both".into(),
            ));
        }
        let radius = need("radius", p.radius)?;
        let profile = CornerProfile::Round {
            radius,
            segments: corner_segments(radius, p.segments),
        };
        return blend_corner(ctx, corner, profile);
    }
    let locators = p
        .edges
        .as_ref()
        .ok_or_else(|| CmdError::Params("fillet needs edges or corner".into()))?;
    let propagate = propagate_flag(p.propagate.as_deref())?;
    let (object, located) = locate::resolve_edge_set(ctx, locators)?;
    let edges = expand_edges(ctx, object, &located, propagate)?;
    let theta = first_dihedral(ctx, object, &edges);
    let corners = corner_style("fillet", p.corners.as_deref())?;
    let stops = stops_of(&p.stops)?;
    let handles = handles_of(&p.handles)?;

    let mut spec = BlendSpec {
        edges,
        profile: BlendProfile::Bullnose { segments: 12 },
        primary: None,
        corners,
        stops,
        handles,
    };
    let probe = |ctx: &Ctx, spec: &BlendSpec| -> Option<f64> {
        ctx.doc
            .object(object)?
            .max_blend_size(spec)
            .ok()
            .map(|l| l.max)
    };
    let bullnose_spec = spec.clone();
    spec.profile = fillet_profile(
        ctx,
        p.profile.as_deref(),
        p.radius,
        p.width,
        p.segments,
        theta,
        |n| probe(ctx, &bullnose_spec).filter(|_| n > 0),
    )?;
    let (report, _change) = ctx.doc.blend_edges(object, &spec)?;
    Ok(blend_result(ctx, object, &report))
}

// ------------------------------------------------------------- chamfer

/// `hew.solid.chamfer`: flat bevels with `distance`, `distances` plus
/// `primary`, or `distance` plus `angle`; or a 2D corner chamfer.
fn chamfer(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct P {
        #[serde(default)]
        edges: Option<Vec<Value>>,
        #[serde(default)]
        corner: Option<Value>,
        #[serde(default)]
        distance: Option<f64>,
        #[serde(default)]
        distances: Option<[f64; 2]>,
        #[serde(default)]
        primary: Option<Value>,
        #[serde(default)]
        angle: Option<f64>,
        #[serde(default)]
        propagate: Option<String>,
        #[serde(default)]
        stops: Option<Value>,
        #[serde(default)]
        corners: Option<String>,
    }
    let p: P = parse_params(params)?;
    if p.distance.is_some() && p.distances.is_some() {
        return Err(CmdError::Params(
            "give distance or distances, not both".into(),
        ));
    }
    if let Some(corner) = &p.corner {
        if p.edges.is_some() {
            return Err(CmdError::Params(
                "give either edges or corner, not both".into(),
            ));
        }
        let (a, b) = match (p.distance, p.distances) {
            (Some(d), None) => (finite("distance", d)?, finite("distance", d)?),
            (None, Some([a, b])) => (finite("distances", a)?, finite("distances", b)?),
            _ => {
                return Err(CmdError::Params(
                    "a corner chamfer needs distance or distances".into(),
                ));
            }
        };
        return blend_corner(
            ctx,
            corner,
            CornerProfile::Chamfer {
                setback_a: a,
                setback_b: b,
            },
        );
    }
    let locators = p
        .edges
        .as_ref()
        .ok_or_else(|| CmdError::Params("chamfer needs edges or corner".into()))?;
    let propagate = propagate_flag(p.propagate.as_deref())?;
    let (object, located) = locate::resolve_edge_set(ctx, locators)?;
    let edges = expand_edges(ctx, object, &located, propagate)?;
    let theta = first_dihedral(ctx, object, &edges);
    let (setback_a, setback_b) = match (p.distance, p.distances, p.angle) {
        (Some(d), None, None) => (finite("distance", d)?, finite("distance", d)?),
        (None, Some([a, b]), None) => (finite("distances", a)?, finite("distances", b)?),
        (Some(d), None, Some(alpha)) => {
            let (d, alpha) = (finite("distance", d)?, finite("angle", alpha)?);
            if alpha <= 0.0 || alpha >= PI - theta {
                return Err(CmdError::Refusal(
                    Refusal::api("angle_does_not_fit", "That angle doesn't fit this edge.")
                        .with_detail(json!({ "max": PI - theta })),
                ));
            }
            (d, d * alpha.sin() / (PI - theta - alpha).sin())
        }
        _ => {
            return Err(CmdError::Params(
                "chamfer needs distance, distances, or distance with angle".into(),
            ));
        }
    };
    let primary = match &p.primary {
        None => None,
        Some(loc) => {
            let f = locate::resolve_face(ctx, loc)?;
            if f.object != object {
                return Err(CmdError::Params(
                    "primary must be a face of the edges' object".into(),
                ));
            }
            Some(f.face)
        }
    };
    let spec = BlendSpec {
        edges,
        profile: BlendProfile::Chamfer {
            setback_a,
            setback_b,
        },
        primary,
        corners: corner_style("chamfer", p.corners.as_deref())?,
        stops: stops_of(&p.stops)?,
        handles: Vec::new(),
    };
    let (report, _change) = ctx.doc.blend_edges(object, &spec)?;
    Ok(blend_result(ctx, object, &report))
}

// ------------------------------------------------------------- edit

/// The blend a face belongs to: `(object, blend id, kind)`, or the
/// `unknown_blend` refusal.
fn blend_of(
    ctx: &Ctx,
    locator: &Value,
) -> Result<(ObjectId, kernel::BlendId, kernel::BlendKind), CmdError> {
    let f = locate::resolve_face(ctx, locator)?;
    let obj = ctx
        .doc
        .object(f.object)
        .ok_or_else(|| unknown_entity("object"))?;
    let face = obj
        .faces()
        .get(f.face)
        .ok_or_else(|| unknown_entity("face"))?;
    match face.blend {
        Some(tag) => Ok((f.object, tag.id, tag.kind)),
        None => Err(CmdError::from(kernel::DocumentError::Blend(
            kernel::BlendError::UnknownBlend,
        ))),
    }
}

/// `hew.solid.resize_blend`: re-blends a blend's restored edges at a new
/// size or kind.
fn resize_blend(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct P {
        face: Value,
        #[serde(default)]
        radius: Option<f64>,
        #[serde(default)]
        distance: Option<f64>,
        #[serde(default)]
        distances: Option<[f64; 2]>,
        #[serde(default)]
        width: Option<f64>,
        #[serde(default)]
        profile: Option<String>,
        #[serde(default)]
        segments: Option<u32>,
    }
    let p: P = parse_params(params)?;
    let (object, blend, kind) = blend_of(ctx, &p.face)?;
    let profile = match (p.distance, p.distances) {
        (Some(d), None) => BlendProfile::Chamfer {
            setback_a: finite("distance", d)?,
            setback_b: d,
        },
        (None, Some([a, b])) => BlendProfile::Chamfer {
            setback_a: finite("distances", a)?,
            setback_b: finite("distances", b)?,
        },
        (Some(_), Some(_)) => {
            return Err(CmdError::Params(
                "give distance or distances, not both".into(),
            ));
        }
        (None, None) => {
            // A fillet resizes as a round unless another kind is named; a
            // profile blend takes `width` with its (named) profile.
            let name = p.profile.as_deref().or(match kind {
                kernel::BlendKind::Profile => None,
                _ => Some("round"),
            });
            let Some(name) = name else {
                return Err(CmdError::Params(
                    "resizing a profile blend needs its profile name and width".into(),
                ));
            };
            fillet_profile(
                ctx,
                Some(name),
                p.radius,
                p.width,
                p.segments,
                FRAC_PI_2,
                |_| None,
            )?
        }
    };
    let (report, _change) = ctx.doc.resize_blend(object, blend, &profile)?;
    Ok(blend_result(ctx, object, &report))
}

/// `hew.solid.remove_blend`: restores the sharp edges a blend replaced.
fn remove_blend(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct P {
        face: Value,
    }
    let p: P = parse_params(params)?;
    let (object, blend, _kind) = blend_of(ctx, &p.face)?;
    ctx.doc.remove_blend(object, blend)?;
    Ok(json!({ "object_id": public_id_of(ctx, EntityRef::Object(object)) }))
}

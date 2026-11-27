//! `hew.*` commands of the true-curves effort (shape). Declarations live in
//! `registry.rs`; this module owns the handlers.

use super::{CmdError, Ctx, Handler};
use crate::geom;
use crate::locate;
use crate::refusal::Refusal;
use kernel::deform::{DeformFrame, DeformSpec};
use kernel::document::{LoftSource, RevolveProfile, RevolveSpec};
use kernel::{EntityRef, ObjectId, Point3, Vec3};
use serde_json::{Value, json};

/// This namespace's slice of the handler table.
pub(crate) fn handler(name: &str) -> Option<Handler> {
    Some(match name {
        "hew.solid.revolve" => revolve,
        "hew.solid.deform" => deform,
        "hew.solid.loft" => loft,
        // ==== true-curves: offset ====
        "hew.solid.shell" => shell,
        "hew.solid.project" => project,
        // ==== end true-curves: offset ====
        _ => return None,
    })
}

// ------------------------------------------------------------- plumbing

fn parse_params<T: serde::de::DeserializeOwned>(params: &Value) -> Result<T, CmdError> {
    serde_json::from_value(params.clone()).map_err(|e| CmdError::Params(e.to_string()))
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

fn public_id_of(ctx: &Ctx, entity: EntityRef) -> String {
    let sid = ctx
        .doc
        .sid_of(&entity)
        .expect("an entity this command just touched always carries a stable id");
    crate::ids::public_id(&entity, sid)
}

// ------------------------------------------------------------- revolve

/// `hew.solid.revolve`: turns a sketch region or a world face about an
/// axis lying in its plane (true-curves.md §5.1). The axis is two points
/// (`{from, to}`, each coordinates or a derived-point locator) or an edge's
/// line (`{edge}`, a solid or sketch edge locator). `segments` defaults to
/// the drawing tools' count for the profile's largest distance from the
/// axis, so the result facets like a circle drawn at that radius.
fn revolve(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct P {
        profile: Value,
        axis: Value,
        #[serde(default)]
        angle: Option<f64>,
        #[serde(default)]
        segments: Option<u32>,
    }
    let p: P = parse_params(params)?;

    let axis_obj = p
        .axis
        .as_object()
        .ok_or_else(|| CmdError::Params("axis needs an object".into()))?;
    let (from, to) = if let Some(edge) = axis_obj.get("edge") {
        if axis_obj.len() != 1 {
            return Err(CmdError::Params("axis.edge takes no sibling keys".into()));
        }
        locate::resolve_edge_locator(ctx, edge)?.endpoints()
    } else {
        let (Some(from), Some(to)) = (axis_obj.get("from"), axis_obj.get("to")) else {
            return Err(CmdError::Params(
                "axis needs {\"from\", \"to\"} or {\"edge\"}".into(),
            ));
        };
        if axis_obj.len() != 2 {
            return Err(CmdError::Params(
                "axis takes only \"from\" and \"to\"".into(),
            ));
        }
        (
            locate::resolve_point(ctx, from)?,
            locate::resolve_point(ctx, to)?,
        )
    };
    let axis = to - from;
    if axis.length_squared() < 1e-24 {
        return Err(CmdError::Params("the axis points must differ".into()));
    }
    let angle = p.angle.unwrap_or(std::f64::consts::TAU);
    if !angle.is_finite() {
        return Err(CmdError::Params("angle must be a finite number".into()));
    }

    let (profile, outer): (RevolveProfile, Vec<Point3>) =
        if let Some(profile_pub) = p.profile.as_str() {
            let (sketch, region) = ctx
                .resolver()
                .resolve_region(profile_pub)
                .ok_or_else(|| unknown_entity(profile_pub))?;
            let outer = ctx
                .doc
                .sketch(sketch)
                .and_then(|s| s.profile(region).ok())
                .map(|prof| prof.outer().to_vec())
                .unwrap_or_default();
            (RevolveProfile::Region { sketch, region }, outer)
        } else if let Some(profile_obj) = p.profile.as_object() {
            let face_val = profile_obj
                .get("face")
                .ok_or_else(|| CmdError::Params("profile object needs \"face\"".into()))?;
            if profile_obj.len() != 1 {
                return Err(CmdError::Params(
                    "profile.face takes no sibling keys".into(),
                ));
            }
            let f = locate::resolve_face(ctx, face_val)?;
            if ctx.doc.object_owner_component(f.object).is_some() {
                // Same posture (and code) as Follow Me: Revolve runs on its
                // engine, and a face locator names no instance to edit
                // through.
                return Err(CmdError::Refusal(Refusal::api(
                    "follow_me_in_component_unsupported",
                    "Revolve on a face inside a component definition isn't supported here — \
                     revolve a world object's face instead, or explode the instance first.",
                )));
            }
            let (outer, _holes) = locate::face_rings(ctx, f)?;
            (
                RevolveProfile::Face {
                    object: f.object,
                    face: f.face,
                },
                outer,
            )
        } else {
            return Err(CmdError::Params(
                "profile needs a region id or {\"face\": <locator>}".into(),
            ));
        };

    let segments = p.segments.unwrap_or_else(|| {
        let dir = axis * (1.0 / axis.length());
        let rho = outer
            .iter()
            .map(|&q| {
                let d = q - from;
                (d - dir * d.dot(dir)).length()
            })
            .fold(0.0_f64, f64::max);
        geom::draw_segments_per_turn(rho)
    });
    let spec = RevolveSpec {
        axis_point: from,
        axis,
        angle,
        segments,
    };
    let (object, _change) = ctx.doc.revolve(profile, &spec)?;
    Ok(json!({ "object_id": public_id_of(ctx, EntityRef::Object(object)) }))
}

// ==== true-curves: offset ====

/// A public id naming a live object (world object or definition member).
fn resolve_object(ctx: &Ctx, public: &str) -> Result<ObjectId, CmdError> {
    match ctx.resolver().resolve(public) {
        Some(EntityRef::Object(id)) if ctx.doc.object(id).is_some() => Ok(id),
        Some(other) if !matches!(other, EntityRef::Object(_)) => Err(CmdError::Params(format!(
            "'{public}' does not name an object"
        ))),
        _ => Err(unknown_entity(public)),
    }
}

/// `hew.solid.shell`: hollows a solid, leaving the located faces open.
fn shell(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct P {
        object: String,
        #[serde(default)]
        open: Vec<Value>,
        thickness: f64,
    }
    let p: P = parse_params(params)?;
    let object = resolve_object(ctx, &p.object)?;
    let mut open = Vec::with_capacity(p.open.len());
    for loc in &p.open {
        let f = locate::resolve_face(ctx, loc)?;
        if f.object != object {
            return Err(CmdError::Params(
                "every open face must be a face of `object`".into(),
            ));
        }
        open.push(f.face);
    }
    let spec = kernel::shell::ShellSpec {
        open,
        thickness: p.thickness,
    };
    ctx.doc.shell(object, &spec)?;
    Ok(json!({ "object_id": public_id_of(ctx, EntityRef::Object(object)) }))
}

/// `hew.solid.project`: imprints a sketch's regions on an object along the
/// sketch's normal (toward the object), then, when `depth` is given,
/// engraves (`> 0`) or embosses (`< 0`) the region, straight (default) or
/// following the surface. Two undo entries (imprint, depth), one frame.
fn project(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct P {
        object: String,
        sketch: String,
        #[serde(default)]
        regions: Vec<String>,
        #[serde(default)]
        depth: Option<f64>,
        #[serde(default)]
        mode: Option<String>,
    }
    let p: P = parse_params(params)?;
    let object = resolve_object(ctx, &p.object)?;
    let sketch = match ctx.resolver().resolve(&p.sketch) {
        Some(EntityRef::Sketch(s)) if ctx.doc.sketch(s).is_some() => s,
        _ => return Err(unknown_entity(&p.sketch)),
    };
    let mut regions = Vec::with_capacity(p.regions.len());
    for r in &p.regions {
        let (sk, region) = ctx
            .resolver()
            .resolve_region(r)
            .ok_or_else(|| unknown_entity(r))?;
        if sk != sketch {
            return Err(CmdError::Params(format!(
                "region '{r}' is not a region of the sketch"
            )));
        }
        regions.push(region);
    }
    let mode = match p.mode.as_deref() {
        None | Some("straight") => kernel::project::DepthMode::Straight,
        Some("follow") => kernel::project::DepthMode::Follow,
        Some(other) => {
            return Err(CmdError::Params(format!(
                "mode must be \"straight\" or \"follow\", not {other:?}"
            )));
        }
    };
    let spec = ctx.doc.project_spec_from_sketch(object, sketch, &regions)?;
    let (faces, _) = ctx.doc.project_imprint(object, &spec)?;
    let n = faces.len();
    if let Some(depth) = p.depth {
        let d = kernel::project::DepthSpec {
            faces,
            dir: spec.dir,
            depth,
            mode,
            regions: spec.regions.clone(),
        };
        ctx.doc.project_depth(object, &d)?;
    }
    Ok(json!({
        "object_id": public_id_of(ctx, EntityRef::Object(object)),
        "faces": n,
    }))
}

// ==== end true-curves: offset ====

// ------------------------------------------------------------- deform

/// A world axis named `"+x"` … `"-z"`.
fn named_axis(name: &str) -> Result<Vec3, CmdError> {
    Ok(match name {
        "+x" => Vec3::new(1.0, 0.0, 0.0),
        "-x" => Vec3::new(-1.0, 0.0, 0.0),
        "+y" => Vec3::new(0.0, 1.0, 0.0),
        "-y" => Vec3::new(0.0, -1.0, 0.0),
        "+z" => Vec3::new(0.0, 0.0, 1.0),
        "-z" => Vec3::new(0.0, 0.0, -1.0),
        other => {
            return Err(CmdError::Params(format!(
                "axis must be one of +x -x +y -y +z -z, not {other:?}"
            )));
        }
    })
}

/// `hew.solid.deform` (true-curves.md §5.3): Twist, Bend or Taper about the
/// object's world bounding box (the face opposite `end` is fixed), or Bend
/// to Path from a base line onto a path.
fn deform(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct P {
        object: String,
        mode: String,
        #[serde(default)]
        end: Option<String>,
        #[serde(default)]
        toward: Option<String>,
        #[serde(default)]
        angle: Option<f64>,
        #[serde(default)]
        ratio: Option<f64>,
        #[serde(default)]
        stations: Option<u32>,
        #[serde(default)]
        base: Option<Value>,
        #[serde(default)]
        path: Option<Value>,
    }
    let p: P = parse_params(params)?;
    let object = match ctx.resolver().resolve(&p.object) {
        Some(EntityRef::Object(id)) if ctx.doc.object_owner_component(id).is_none() => id,
        _ => {
            return Err(CmdError::Refusal(Refusal::api(
                "unknown_object",
                &format!(
                    "'{}' does not name a world object in this document.",
                    p.object
                ),
            )));
        }
    };
    let spec = if p.mode == "bend_to_path" {
        let base = p
            .base
            .as_ref()
            .and_then(Value::as_object)
            .ok_or_else(|| CmdError::Params("bend_to_path needs base {from, to}".into()))?;
        let (Some(from), Some(to)) = (base.get("from"), base.get("to")) else {
            return Err(CmdError::Params("base needs \"from\" and \"to\"".into()));
        };
        let (base_from, base_to) = (
            locate::resolve_point(ctx, from)?,
            locate::resolve_point(ctx, to)?,
        );
        let path_val = p
            .path
            .as_ref()
            .ok_or_else(|| CmdError::Params("bend_to_path needs a path".into()))?;
        let path = super::solid::follow_me_path(ctx, path_val)?;
        let (points, closed) = ctx.doc.follow_me_path_points(&path)?;
        DeformSpec::BendToPath {
            base_from,
            base_to,
            path: points,
            closed,
            stations_per_segment: p.stations,
        }
    } else {
        let end = named_axis(
            p.end
                .as_deref()
                .ok_or_else(|| CmdError::Params(format!("{} needs \"end\"", p.mode)))?,
        )?;
        let obj = ctx
            .doc
            .object(object)
            .ok_or_else(|| unknown_entity(&p.object))?;
        let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
        for v in obj.vertices().values() {
            for (i, c) in [v.position.x, v.position.y, v.position.z]
                .into_iter()
                .enumerate()
            {
                lo[i] = lo[i].min(c);
                hi[i] = hi[i].max(c);
            }
        }
        let axis_i = [end.x, end.y, end.z]
            .iter()
            .position(|c| c.abs() > 0.5)
            .expect("a named axis");
        let mut center = [0.0; 3];
        for i in 0..3 {
            center[i] = 0.5 * (lo[i] + hi[i]);
        }
        // The fixed end: the face opposite `end`.
        let sign = [end.x, end.y, end.z][axis_i];
        center[axis_i] = if sign > 0.0 { lo[axis_i] } else { hi[axis_i] };
        let length = hi[axis_i] - lo[axis_i];
        let u = match p.toward.as_deref() {
            Some(t) => named_axis(t)?,
            None => {
                // The cross axis closest to −Z, or +X for a vertical axis.
                if axis_i == 2 {
                    Vec3::new(1.0, 0.0, 0.0)
                } else {
                    Vec3::new(0.0, 0.0, -1.0)
                }
            }
        };
        let frame = DeformFrame {
            origin: Point3::new(center[0], center[1], center[2]),
            axis: end,
            u,
            length,
        };
        match p.mode.as_str() {
            "twist" => DeformSpec::Twist {
                frame,
                angle: p
                    .angle
                    .ok_or_else(|| CmdError::Params("twist needs \"angle\"".into()))?,
                stations: p.stations,
            },
            "bend" => DeformSpec::Bend {
                frame,
                angle: p
                    .angle
                    .ok_or_else(|| CmdError::Params("bend needs \"angle\"".into()))?,
                stations: p.stations,
            },
            "taper" => DeformSpec::Taper {
                frame,
                ratio: p
                    .ratio
                    .ok_or_else(|| CmdError::Params("taper needs \"ratio\"".into()))?,
                stations: p.stations,
            },
            other => {
                return Err(CmdError::Params(format!(
                    "mode must be twist, bend, taper or bend_to_path, not {other:?}"
                )));
            }
        }
    };
    ctx.doc.deform(object, &spec)?;
    Ok(json!({ "object_id": public_id_of(ctx, EntityRef::Object(object)) }))
}

// ------------------------------------------------------------- loft

/// `hew.solid.loft` (true-curves.md §5.4): a new solid skinned through
/// closed profiles in order, optionally ending in an apex and guided by up
/// to two rails. The profiles are not consumed.
fn loft(ctx: &mut Ctx, params: &Value) -> Result<Value, CmdError> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct P {
        profiles: Vec<Value>,
        #[serde(default)]
        apex: Option<Value>,
        #[serde(default)]
        rails: Vec<Value>,
        #[serde(default)]
        start_offsets: Vec<i32>,
        #[serde(default)]
        stations: Option<u32>,
    }
    let p: P = parse_params(params)?;
    let mut sources = Vec::with_capacity(p.profiles.len() + 1);
    for v in &p.profiles {
        if let Some(public) = v.as_str() {
            let (sketch, region) = ctx
                .resolver()
                .resolve_region(public)
                .ok_or_else(|| unknown_entity(public))?;
            sources.push(LoftSource::Region { sketch, region });
        } else if let Some(obj) = v.as_object() {
            if obj.contains_key("curve") || obj.contains_key("edges") {
                return Err(CmdError::Refusal(Refusal::api(
                    "open_profile",
                    "Loft needs closed shapes: name a region or a face, not a line or curve.",
                )));
            }
            let face_val = obj
                .get("face")
                .ok_or_else(|| CmdError::Params("a profile object needs \"face\"".into()))?;
            if obj.len() != 1 {
                return Err(CmdError::Params(
                    "profile.face takes no sibling keys".into(),
                ));
            }
            let f = locate::resolve_face(ctx, face_val)?;
            if ctx.doc.object_owner_component(f.object).is_some() {
                return Err(CmdError::Refusal(Refusal::api(
                    "unknown_object",
                    "Loft takes faces of world objects; explode the instance first.",
                )));
            }
            sources.push(LoftSource::Face {
                object: f.object,
                face: f.face,
            });
        } else {
            return Err(CmdError::Params(
                "a profile is a region id or {\"face\": <locator>}".into(),
            ));
        }
    }
    if let Some(apex) = &p.apex {
        sources.push(LoftSource::Vertex {
            point: locate::resolve_point(ctx, apex)?,
        });
    }
    let mut rails = Vec::with_capacity(p.rails.len());
    for r in &p.rails {
        rails.push(super::solid::follow_me_path(ctx, r)?);
    }
    let (object, _change) = ctx
        .doc
        .loft(&sources, &rails, &p.start_offsets, p.stations)?;
    Ok(json!({ "object_id": public_id_of(ctx, EntityRef::Object(object)) }))
}

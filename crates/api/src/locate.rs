//! Locator and derived-point resolution (docs/HEW_API.md §5.2, §5.3) —
//! the API's equivalent of pointing. A solid's faces and edges have no
//! persistent public identifiers; commands take locators, resolved at
//! dispatch time against the document state the command actually runs in.
//! Ambiguity refuses typed, never guesses. Sketch edges are the one
//! exception — they carry a durable public id (`ids::edge_id`) precisely
//! because they are stable, user-authored scaffolding rather than
//! sticky-geometry byproduct (§5.2's contrast) — but this module still
//! offers a geometric locator for them ([`resolve_sketch_edge`]), for a
//! client that has not queried one yet.

use crate::commands::{CmdError, Ctx};
use crate::geom;
use crate::refusal::Refusal;
use kernel::{EntityRef, FaceId, ObjectId, Point3, SketchId, Vec3};

/// A resolved face reference.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FaceRef {
    pub object: ObjectId,
    pub face: FaceId,
}

/// A resolved edge reference (a kernel undirected edge of a solid).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EdgeRef {
    pub object: ObjectId,
    pub edge: kernel::EdgeId,
    pub endpoints: (Point3, Point3),
}

/// A resolved sketch edge reference. Unlike a solid edge, a sketch edge
/// carries a durable public id (`ids::edge_id`, docs/HEW_API.md §5.2) — a
/// client that already queried one just names it directly — but this is
/// also reachable by the same kind of geometric locator a solid edge
/// needs, for a client that has not queried (`resolve_sketch_edge`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SketchEdgeRef {
    pub sketch: SketchId,
    pub edge: kernel::SketchEdgeId,
    pub endpoints: (Point3, Point3),
    /// The curve chain this edge belongs to, when it was drawn as part of
    /// one analytic circle/arc (`None` for a plain line).
    pub curve: Option<kernel::SketchCurveId>,
}

/// Any edge a caller may locate — a solid edge (§5.2, always by locator,
/// never by id) or a sketch edge (by its public id, or by the same
/// point/two-endpoint locator shapes). [`resolve_edge_locator`] is the one
/// entry point that accepts every form the grammar allows anywhere an
/// "edge locator" is documented (§5.2, §5.3's derived points,
/// `hew.query.resolve`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AnyEdge {
    Solid(EdgeRef),
    Sketch(SketchEdgeRef),
}

impl AnyEdge {
    /// The edge's two endpoints, whichever kind it is — the one thing
    /// every consumer (derived points, `hew.query.resolve`) actually
    /// needs.
    pub fn endpoints(&self) -> (Point3, Point3) {
        match self {
            AnyEdge::Solid(e) => e.endpoints,
            AnyEdge::Sketch(e) => e.endpoints,
        }
    }
}

fn missed(what: &str) -> CmdError {
    CmdError::Refusal(Refusal::api(
        "locator_missed",
        &format!(
            "The locator matched no {what}. Check the point or ray against the object's geometry."
        ),
    ))
}

fn ambiguous(what: &str) -> CmdError {
    CmdError::Refusal(Refusal::api(
        "ambiguous_locator",
        &format!(
            "The locator matched more than one {what} equally well. Give a point clearly inside one of them."
        ),
    ))
}

fn unknown_entity(id: &str) -> CmdError {
    CmdError::Refusal(
        Refusal::api(
            "unknown_entity",
            &format!("'{id}' does not name a live entity of the required kind in this document."),
        )
        .with_detail(serde_json::json!({ "id": id })),
    )
}

/// Parses `[x, y, z]` with finite components.
pub fn parse_xyz(v: &serde_json::Value) -> Result<Point3, CmdError> {
    let arr = v
        .as_array()
        .filter(|a| a.len() == 3)
        .ok_or_else(|| CmdError::Params("expected a point as [x, y, z]".into()))?;
    let mut c = [0.0_f64; 3];
    for (i, item) in arr.iter().enumerate() {
        let x = item
            .as_f64()
            .filter(|x| x.is_finite())
            .ok_or_else(|| CmdError::Params("point components must be finite numbers".into()))?;
        c[i] = x;
    }
    Ok(Point3::new(c[0], c[1], c[2]))
}

/// Parses a direction vector `[x, y, z]`, non-degenerate.
pub fn parse_dir(v: &serde_json::Value) -> Result<Vec3, CmdError> {
    let p = parse_xyz(v)?;
    let d = Vec3::new(p.x, p.y, p.z);
    if d.length_squared() < 1e-24 {
        return Err(CmdError::Params("direction must be non-zero".into()));
    }
    Ok(d)
}

/// Any point parameter: coordinates or a derived-point locator (§5.3).
pub fn resolve_point(ctx: &Ctx, v: &serde_json::Value) -> Result<Point3, CmdError> {
    if v.is_array() {
        return parse_xyz(v);
    }
    let obj = v
        .as_object()
        .ok_or_else(|| CmdError::Params("expected [x,y,z] or a derived-point object".into()))?;
    let kind = obj
        .get("point")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| CmdError::Params("derived point needs a \"point\" kind".into()))?;
    match kind {
        "midpoint" | "endpoint" => {
            let of = obj
                .get("of")
                .and_then(|o| o.get("edge"))
                .ok_or_else(|| CmdError::Params(format!("{kind} needs of.edge")))?;
            let edge = resolve_edge_locator(ctx, of)?;
            let (a, b) = edge.endpoints();
            if kind == "midpoint" {
                Ok(Point3::new(
                    (a.x + b.x) * 0.5,
                    (a.y + b.y) * 0.5,
                    (a.z + b.z) * 0.5,
                ))
            } else {
                let near = obj
                    .get("nearest")
                    .map(parse_xyz)
                    .transpose()?
                    .ok_or_else(|| {
                        CmdError::Params("endpoint needs \"nearest\" to pick one of the two".into())
                    })?;
                Ok(if (near - a).length() <= (near - b).length() {
                    a
                } else {
                    b
                })
            }
        }
        "center" => {
            let of = obj
                .get("of")
                .ok_or_else(|| CmdError::Params("center needs \"of\"".into()))?;
            resolve_center(ctx, of)
        }
        "centroid" => {
            let of = obj
                .get("of")
                .ok_or_else(|| CmdError::Params("centroid needs \"of\" (a face locator)".into()))?;
            let face = resolve_face(ctx, of)?;
            let (outer, holes) = face_rings(ctx, face)?;
            Ok(geom::face_centroid(&outer, &holes))
        }
        "quadrant" => {
            let of = obj
                .get("of")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| CmdError::Params("quadrant needs \"of\" (a curve id)".into()))?;
            let toward = obj
                .get("toward")
                .map(parse_xyz)
                .transpose()?
                .ok_or_else(|| CmdError::Params("quadrant needs \"toward\"".into()))?;
            let (sketch, curve) = ctx
                .resolver()
                .resolve_curve(of)
                .ok_or_else(|| unknown_entity(of))?;
            let sk = ctx.doc.sketch(sketch).ok_or_else(|| unknown_entity(of))?;
            let geomc = sk.curve_geom(curve).ok_or_else(|| {
                CmdError::Refusal(Refusal::api(
                    "no_such_point",
                    "That curve has no analytic circle behind it, so it has no quadrant points.",
                ))
            })?;
            // The quadrant point in the given direction, on the sketch plane.
            let plane = sk.plane();
            let n = plane.normal();
            let toward_v = toward - geomc.center;
            let in_plane = toward_v - n * toward_v.dot(n);
            let dir = in_plane
                .normalized()
                .map_err(|_| CmdError::Params("\"toward\" coincides with the center".into()))?;
            Ok(geomc.center + dir * geomc.radius)
        }
        "bbox" => {
            let of = obj
                .get("of")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| CmdError::Params("bbox needs \"of\" (an entity id)".into()))?;
            let anchor = obj
                .get("anchor")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("center");
            let entity = ctx
                .resolver()
                .resolve(of)
                .ok_or_else(|| unknown_entity(of))?;
            let (min, max) = entity_bbox(ctx, &entity).ok_or_else(|| {
                CmdError::Refusal(Refusal::api(
                    "no_such_point",
                    "That entity has no bounding box — a guide line is infinite (an origin and a direction), so a box anchor on it would be meaningless.",
                ))
            })?;
            Ok(match anchor {
                "min" => min,
                "max" => max,
                "center" => Point3::new(
                    (min.x + max.x) * 0.5,
                    (min.y + max.y) * 0.5,
                    (min.z + max.z) * 0.5,
                ),
                other => {
                    return Err(CmdError::Params(format!(
                        "unknown bbox anchor '{other}' (center | min | max)"
                    )));
                }
            })
        }
        "position" => {
            let of = obj
                .get("of")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| CmdError::Params("position needs \"of\" (a guide id)".into()))?;
            let entity = ctx
                .resolver()
                .resolve(of)
                .ok_or_else(|| unknown_entity(of))?;
            let EntityRef::Guide(gid) = entity else {
                return Err(CmdError::Params("position works on guide points".into()));
            };
            match ctx.doc.guide(gid) {
                Some(kernel::Guide::Point { position }) => Ok(*position),
                Some(kernel::Guide::Line { .. }) => Err(CmdError::Refusal(Refusal::api(
                    "no_such_point",
                    "That guide is an infinite line, not a point; it has no single position.",
                ))),
                None => Err(unknown_entity(of)),
            }
        }
        other => Err(CmdError::Params(format!(
            "unknown derived point kind '{other}'"
        ))),
    }
}

/// `center` targets: a curve id (circle/arc center) or a face locator on
/// a stamped curved wall (its cylinder's axis center at the face).
fn resolve_center(ctx: &Ctx, of: &serde_json::Value) -> Result<Point3, CmdError> {
    if let Some(curve_pub) = of.as_str() {
        let (sketch, curve) = ctx
            .resolver()
            .resolve_curve(curve_pub)
            .ok_or_else(|| unknown_entity(curve_pub))?;
        let sk = ctx
            .doc
            .sketch(sketch)
            .ok_or_else(|| unknown_entity(curve_pub))?;
        return sk.curve_center(curve).ok_or_else(|| {
            CmdError::Refusal(Refusal::api(
                "no_such_point",
                "That curve carries no analytic center.",
            ))
        });
    }
    // Face locator: a curved wall's cylinder axis.
    let face = resolve_face(ctx, of)?;
    let object = ctx.doc.object(face.object).ok_or_else(|| missed("face"))?;
    let f = object
        .faces()
        .get(face.face)
        .ok_or_else(|| missed("face"))?;
    let Some(surface) = f.surface else {
        return Err(CmdError::Refusal(Refusal::api(
            "no_such_point",
            "That face is not a stamped curved wall; it has no analytic center.",
        )));
    };
    let kernel::SurfaceRef::Cylinder {
        axis_point, axis, ..
    } = surface;
    // Project the face's ring centroid onto the cylinder axis.
    let ring: Vec<Point3> = object.loop_positions(f.outer_loop).collect();
    let c = geom::face_centroid(&ring, &[]);
    let axis_n = axis
        .normalized()
        .map_err(|_| CmdError::Internal("degenerate cylinder axis".into()))?;
    Ok(axis_point + axis_n * (c - axis_point).dot(axis_n))
}

/// The outer + hole rings of a face, as positions.
pub fn face_rings(ctx: &Ctx, face: FaceRef) -> Result<(Vec<Point3>, Vec<Vec<Point3>>), CmdError> {
    let object = ctx.doc.object(face.object).ok_or_else(|| missed("face"))?;
    let f = object
        .faces()
        .get(face.face)
        .ok_or_else(|| missed("face"))?;
    let outer: Vec<Point3> = object.loop_positions(f.outer_loop).collect();
    let holes: Vec<Vec<Point3>> = f
        .inner_loops
        .iter()
        .map(|&l| object.loop_positions(l).collect())
        .collect();
    Ok((outer, holes))
}

/// Resolves a face locator (§5.2): `{object, at}`, `{object, ray}`, or
/// `{"$face": "label#key"}`.
pub fn resolve_face(ctx: &Ctx, v: &serde_json::Value) -> Result<FaceRef, CmdError> {
    let obj = v
        .as_object()
        .ok_or_else(|| CmdError::Params("expected a face locator object".into()))?;

    if let Some(token) = obj.get("$face") {
        let text = token
            .as_str()
            .ok_or_else(|| CmdError::Params("$face must be a string".into()))?;
        let (label, key) = text
            .split_once('#')
            .ok_or_else(|| CmdError::Params("$face must be 'label#key'".into()))?;
        let (object, face) = ctx
            .face_tokens
            .get(label)
            .and_then(|keys| keys.get(key))
            .copied()
            .ok_or_else(|| {
                CmdError::Refusal(
                    Refusal::api(
                        "face_token_unknown",
                        &format!("No face token '{text}' was minted earlier in this transaction."),
                    )
                    .with_detail(serde_json::json!({ "token": text })),
                )
            })?;
        // A later command may have consumed the face; a stale token is a
        // typed refusal, never an aliased face (generational keys).
        let live = ctx
            .doc
            .object(object)
            .is_some_and(|o| o.faces().contains_key(face));
        if !live {
            return Err(CmdError::Refusal(Refusal::api(
                "face_token_stale",
                &format!(
                    "Face token '{text}' no longer names a live face — a later command in this transaction reshaped it."
                ),
            )));
        }
        return Ok(FaceRef { object, face });
    }

    let public = obj
        .get("object")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| CmdError::Params("face locator needs \"object\"".into()))?;
    let Some(EntityRef::Object(object_id)) = ctx.resolver().resolve(public) else {
        return Err(unknown_entity(public));
    };
    let object = ctx
        .doc
        .object(object_id)
        .ok_or_else(|| unknown_entity(public))?;

    if let Some(at) = obj.get("at") {
        let p = parse_xyz(at)?;
        // Candidate faces: containing the projected point, ranked by
        // distance to their plane.
        let mut hits: Vec<(f64, FaceId)> = Vec::new();
        for (fid, f) in object.faces() {
            let d = f.plane.signed_distance(p).abs();
            if d > geom::API_SURFACE_TOL {
                continue;
            }
            let outer: Vec<Point3> = object.loop_positions(f.outer_loop).collect();
            let holes: Vec<Vec<Point3>> = f
                .inner_loops
                .iter()
                .map(|&l| object.loop_positions(l).collect())
                .collect();
            if geom::face_contains(&outer, &holes, f.plane.normal(), p) {
                hits.push((d, fid));
            }
        }
        hits.sort_by(|a, b| a.0.total_cmp(&b.0));
        return match hits.as_slice() {
            [] => Err(missed("face")),
            [one] => Ok(FaceRef {
                object: object_id,
                face: one.1,
            }),
            [first, second, ..] => {
                if (second.0 - first.0).abs() <= geom::API_AMBIGUITY_TOL {
                    Err(ambiguous("face"))
                } else {
                    Ok(FaceRef {
                        object: object_id,
                        face: first.1,
                    })
                }
            }
        };
    }

    if let Some(ray) = obj.get("ray") {
        let origin = ray
            .get("origin")
            .map(parse_xyz)
            .transpose()?
            .ok_or_else(|| CmdError::Params("ray needs origin".into()))?;
        let dir = ray
            .get("dir")
            .map(parse_dir)
            .transpose()?
            .ok_or_else(|| CmdError::Params("ray needs dir".into()))?;
        let mut best: Option<(f64, FaceId)> = None;
        let mut tie = false;
        for (fid, f) in object.faces() {
            let Some(t) = geom::ray_plane_t(origin, dir, &f.plane) else {
                continue;
            };
            let hit = origin + dir * t;
            let outer: Vec<Point3> = object.loop_positions(f.outer_loop).collect();
            let holes: Vec<Vec<Point3>> = f
                .inner_loops
                .iter()
                .map(|&l| object.loop_positions(l).collect())
                .collect();
            if !geom::face_contains(&outer, &holes, f.plane.normal(), hit) {
                continue;
            }
            match best {
                None => best = Some((t, fid)),
                Some((bt, _)) if t < bt - geom::API_AMBIGUITY_TOL => {
                    best = Some((t, fid));
                    tie = false;
                }
                Some((bt, _)) if (t - bt).abs() <= geom::API_AMBIGUITY_TOL => tie = true,
                _ => {}
            }
        }
        return match best {
            Some(_) if tie => Err(ambiguous("face")),
            Some((_, fid)) => Ok(FaceRef {
                object: object_id,
                face: fid,
            }),
            None => Err(missed("face")),
        };
    }

    Err(CmdError::Params(
        "face locator needs \"at\", \"ray\", or \"$face\"".into(),
    ))
}

/// Resolves an edge locator: `{object, at}` — the object's edge nearest
/// the point within tolerance. Ambiguity (a shared vertex between two
/// equally near edges) refuses typed.
pub fn resolve_edge(ctx: &Ctx, v: &serde_json::Value) -> Result<EdgeRef, CmdError> {
    let obj = v
        .as_object()
        .ok_or_else(|| CmdError::Params("expected an edge locator object".into()))?;
    let public = obj
        .get("object")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| CmdError::Params("edge locator needs \"object\"".into()))?;
    let Some(EntityRef::Object(object_id)) = ctx.resolver().resolve(public) else {
        return Err(unknown_entity(public));
    };
    let object = ctx
        .doc
        .object(object_id)
        .ok_or_else(|| unknown_entity(public))?;
    let at = obj
        .get("at")
        .ok_or_else(|| CmdError::Params("edge locator needs \"at\"".into()))?;
    let p = parse_xyz(at)?;

    let mut best: Option<(f64, kernel::EdgeId, (Point3, Point3))> = None;
    let mut tie = false;
    for (eid, _) in object.edges() {
        let Some((a, b)) = object.edge_endpoints(eid) else {
            continue;
        };
        let (d, _) = geom::point_segment_distance(p, a, b);
        if d > geom::API_SURFACE_TOL {
            continue;
        }
        match best {
            None => best = Some((d, eid, (a, b))),
            Some((bd, ..)) if d < bd - geom::API_AMBIGUITY_TOL => {
                best = Some((d, eid, (a, b)));
                tie = false;
            }
            Some((bd, ..)) if (d - bd).abs() <= geom::API_AMBIGUITY_TOL => tie = true,
            _ => {}
        }
    }
    match best {
        Some(_) if tie => Err(ambiguous("edge")),
        Some((_, edge, endpoints)) => Ok(EdgeRef {
            object: object_id,
            edge,
            endpoints,
        }),
        None => Err(missed("edge")),
    }
}

/// The endpoints of a live sketch edge, from the sketch's own storage
/// (`Sketch::vertices`/`Sketch::edges`, both already public — no new
/// kernel accessor needed for this half).
fn sketch_edge_endpoints(
    sk: &kernel::Sketch,
    edge: kernel::SketchEdgeId,
) -> Option<(Point3, Point3)> {
    let e = sk.edges().get(edge)?;
    Some((sk.vertices()[e.from].position, sk.vertices()[e.to].position))
}

/// Resolves a sketch edge locator: `{"sketch": id, "at": [x, y, z]}` — the
/// sketch's edge nearest the point within tolerance (mirrors
/// [`resolve_edge`]'s object-edge-by-point form) — or `{"sketch": id,
/// "from": [x, y, z], "to": [x, y, z]}` — the edge whose endpoints
/// coincide with the two given points, via
/// [`kernel::Sketch::edge_at_positions`] (sticky rules forbid coincident
/// duplicate edges, so that match is unique by construction — no
/// ambiguity check needed for this form, unlike `at`).
pub fn resolve_sketch_edge(ctx: &Ctx, v: &serde_json::Value) -> Result<SketchEdgeRef, CmdError> {
    let obj = v
        .as_object()
        .ok_or_else(|| CmdError::Params("expected a sketch edge locator object".into()))?;
    let public = obj
        .get("sketch")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| CmdError::Params("sketch edge locator needs \"sketch\"".into()))?;
    let Some(EntityRef::Sketch(sketch_id)) = ctx.resolver().resolve(public) else {
        return Err(unknown_entity(public));
    };
    let sk = ctx
        .doc
        .sketch(sketch_id)
        .ok_or_else(|| unknown_entity(public))?;

    if let Some(at) = obj.get("at") {
        let p = parse_xyz(at)?;
        let mut best: Option<(f64, kernel::SketchEdgeId, (Point3, Point3))> = None;
        let mut tie = false;
        for (eid, _) in sk.edges() {
            let Some((a, b)) = sketch_edge_endpoints(sk, eid) else {
                continue;
            };
            let (d, _) = geom::point_segment_distance(p, a, b);
            if d > geom::API_SURFACE_TOL {
                continue;
            }
            match best {
                None => best = Some((d, eid, (a, b))),
                Some((bd, ..)) if d < bd - geom::API_AMBIGUITY_TOL => {
                    best = Some((d, eid, (a, b)));
                    tie = false;
                }
                Some((bd, ..)) if (d - bd).abs() <= geom::API_AMBIGUITY_TOL => tie = true,
                _ => {}
            }
        }
        return match best {
            Some(_) if tie => Err(ambiguous("sketch edge")),
            Some((_, edge, endpoints)) => Ok(SketchEdgeRef {
                sketch: sketch_id,
                edge,
                endpoints,
                curve: sk.edge_curve(edge),
            }),
            None => Err(missed("sketch edge")),
        };
    }

    if let (Some(from), Some(to)) = (obj.get("from"), obj.get("to")) {
        let a = parse_xyz(from)?;
        let b = parse_xyz(to)?;
        let edge = sk
            .edge_at_positions(a, b)
            .ok_or_else(|| missed("sketch edge"))?;
        let endpoints = sketch_edge_endpoints(sk, edge).ok_or_else(|| {
            CmdError::Internal("edge_at_positions returned a key its own sketch lacks".into())
        })?;
        return Ok(SketchEdgeRef {
            sketch: sketch_id,
            edge,
            endpoints,
            curve: sk.edge_curve(edge),
        });
    }

    Err(CmdError::Params(
        "sketch edge locator needs \"at\", or \"from\" and \"to\"".into(),
    ))
}

/// The one entry point for every "edge locator" the API's grammar
/// documents (§5.2, and §5.3's `{"point": "midpoint"/"endpoint", "of":
/// {"edge": …}}`): a bare string is a sketch edge's public id (§5.2's
/// addendum — sketch edges, unlike solid edges, carry a durable id); an
/// object carrying `"sketch"` is the sketch-edge geometric locator
/// ([`resolve_sketch_edge`]); anything else is the solid object-edge
/// locator ([`resolve_edge`]), unchanged.
pub fn resolve_edge_locator(ctx: &Ctx, v: &serde_json::Value) -> Result<AnyEdge, CmdError> {
    if let Some(s) = v.as_str() {
        let (sketch, edge) = ctx
            .resolver()
            .resolve_edge(s)
            .ok_or_else(|| unknown_entity(s))?;
        let sk = ctx.doc.sketch(sketch).ok_or_else(|| unknown_entity(s))?;
        let endpoints = sketch_edge_endpoints(sk, edge).ok_or_else(|| unknown_entity(s))?;
        return Ok(AnyEdge::Sketch(SketchEdgeRef {
            sketch,
            edge,
            endpoints,
            curve: sk.edge_curve(edge),
        }));
    }
    if v.get("sketch").is_some() {
        return resolve_sketch_edge(ctx, v).map(AnyEdge::Sketch);
    }
    resolve_edge(ctx, v).map(AnyEdge::Solid)
}

/// A plane spec (§7 semantics notes): where a drawing command draws.
pub enum PlaneTarget {
    /// A fresh sketch on this plane.
    NewPlane(kernel::Plane),
    /// Extend an existing sketch.
    Sketch(SketchId),
    /// Draw on a solid's face (the face-imprint path).
    Face(FaceRef),
}

/// Parses a plane spec: `{"ground": true}`, `{origin, normal}`,
/// `{"face": <locator>}`, or `{"sketch": "<id>"}`.
pub fn resolve_plane(ctx: &Ctx, v: &serde_json::Value) -> Result<PlaneTarget, CmdError> {
    let obj = v
        .as_object()
        .ok_or_else(|| CmdError::Params("expected a plane spec object".into()))?;
    if obj.get("ground").and_then(serde_json::Value::as_bool) == Some(true) {
        let plane =
            kernel::Plane::from_point_normal(Point3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 1.0))
                .expect("ground plane is well-formed");
        return Ok(PlaneTarget::NewPlane(plane));
    }
    if let Some(sketch) = obj.get("sketch").and_then(serde_json::Value::as_str) {
        let Some(EntityRef::Sketch(id)) = ctx.resolver().resolve(sketch) else {
            return Err(unknown_entity(sketch));
        };
        return Ok(PlaneTarget::Sketch(id));
    }
    if let Some(face) = obj.get("face") {
        return Ok(PlaneTarget::Face(resolve_face(ctx, face)?));
    }
    if let (Some(origin), Some(normal)) = (obj.get("origin"), obj.get("normal")) {
        let origin = parse_xyz(origin)?;
        let normal = parse_dir(normal)?;
        let plane = kernel::Plane::from_point_normal(origin, normal)
            .map_err(|_| CmdError::Params("degenerate plane normal".into()))?;
        return Ok(PlaneTarget::NewPlane(plane));
    }
    Err(CmdError::Params(
        "plane spec needs ground / origin+normal / face / sketch".into(),
    ))
}

/// Entity bbox for the `bbox` derived point and query summaries. `None`
/// for entities without a finite box (guide LINES — §5.3 refuses them).
pub fn entity_bbox(ctx: &Ctx, entity: &EntityRef) -> Option<(Point3, Point3)> {
    match entity {
        EntityRef::Object(id) => {
            let o = ctx.doc.object(*id)?;
            geom::bbox(o.vertices().values().map(|v| v.position))
        }
        EntityRef::Sketch(id) => {
            let s = ctx.doc.sketch(*id)?;
            geom::bbox(s.vertices().values().map(|v| v.position))
        }
        EntityRef::Guide(id) => match ctx.doc.guide(*id)? {
            kernel::Guide::Point { position } => Some((*position, *position)),
            kernel::Guide::Line { .. } => None,
        },
        EntityRef::Group(id) => {
            let members = ctx.doc.group_members(*id)?;
            let boxes: Vec<(Point3, Point3)> =
                members.iter().flat_map(|m| node_bbox(ctx, *m)).collect();
            merge_boxes(boxes)
        }
        EntityRef::Instance(id) => instance_bbox(ctx, *id),
        EntityRef::Component(id) => {
            let members = ctx.doc.def_members(*id)?;
            let boxes: Vec<(Point3, Point3)> = members
                .iter()
                .filter_map(|&o| {
                    let obj = ctx.doc.object(o)?;
                    geom::bbox(obj.vertices().values().map(|v| v.position))
                })
                .collect();
            merge_boxes(boxes)
        }
        EntityRef::Material(_) | EntityRef::Tag(_) => None,
    }
}

/// A tree node's bbox (world space).
pub fn node_bbox(ctx: &Ctx, node: kernel::NodeId) -> Option<(Point3, Point3)> {
    match node {
        kernel::NodeId::Object(id) => entity_bbox(ctx, &EntityRef::Object(id)),
        kernel::NodeId::Group(id) => entity_bbox(ctx, &EntityRef::Group(id)),
        kernel::NodeId::Instance(id) => instance_bbox(ctx, id),
    }
}

fn instance_bbox(ctx: &Ctx, id: kernel::InstanceId) -> Option<(Point3, Point3)> {
    let def = ctx.doc.instance_def(id)?;
    let pose = ctx.doc.instance_pose(id)?;
    let members = ctx.doc.def_members(def)?;
    let points = members.iter().flat_map(|&o| {
        ctx.doc
            .object(o)
            .into_iter()
            .flat_map(|obj| {
                obj.vertices()
                    .values()
                    .map(|v| v.position)
                    .collect::<Vec<_>>()
            })
            .map(move |p| pose.apply_point(p))
    });
    geom::bbox(points)
}

fn merge_boxes(boxes: Vec<(Point3, Point3)>) -> Option<(Point3, Point3)> {
    let mut it = boxes.into_iter();
    let (mut min, mut max) = it.next()?;
    for (lo, hi) in it {
        min = Point3::new(min.x.min(lo.x), min.y.min(lo.y), min.z.min(lo.z));
        max = Point3::new(max.x.max(hi.x), max.y.max(hi.y), max.z.max(hi.z));
    }
    Some((min, max))
}

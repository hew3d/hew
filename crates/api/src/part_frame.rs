//! Part frames: a part measured along its own axes (docs/agents/PLUGINS.md
//! "Reading parts"; `hew.query.part_frame`). Pure over a [`Document`].
//!
//! Every Hew face is planar, so an object's frame comes from its largest
//! face; there is no oriented-bounding-box source. A `hew.part` / `frame`
//! attribute overrides the frame, expressed in the face frame the part
//! reports without it, so a plugin can echo reported axes back and the
//! override turns and moves with the part (up to the part's own
//! symmetry; see [`canonical_frame`]).

use kernel::{
    AttrTarget, AttrValue, Document, DocumentError, EntityRef, NodeId, ObjectId, Occurrence,
    Point3, RenderRoot, Transform, Vec3,
};

/// A part's frame in world space: orthonormal, right-handed axes, an
/// origin, and the part's extents along each axis.
#[derive(Debug, Clone, PartialEq)]
pub struct PartFrame {
    pub origin: Point3,
    pub x: Vec3,
    pub y: Vec3,
    pub z: Vec3,
    /// Along x, y, z.
    pub extents: [f64; 3],
    pub source: FrameSource,
}

/// Where a frame came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameSource {
    /// A `hew.part` / `frame` attribute.
    Attribute,
    /// An instance's definition axes.
    Definition,
    /// The largest planar face.
    Face,
}

impl FrameSource {
    pub fn name(self) -> &'static str {
        match self {
            FrameSource::Attribute => "attribute",
            FrameSource::Definition => "definition",
            FrameSource::Face => "face",
        }
    }
}

/// Relative tolerance under which two face areas, two edge lengths, or two
/// coordinates of the tie-break count as equal.
const FRAME_TIE_TOL: f64 = 1e-9;

/// How far an override's axes may stray from unit length and from
/// perpendicular before the override is ignored.
const OVERRIDE_ORTHO_TOL: f64 = 1e-6;

/// The attribute namespace and key of a frame override.
pub const FRAME_NAMESPACE: &str = "hew.part";
pub const FRAME_KEY: &str = "frame";

/// Whether `a` and `b` agree within [`FRAME_TIE_TOL`] of `unit`, the
/// part's own size raised to the quantity's dimension (a length, an area),
/// so a tie means the same thing for a 10 mm part and a 10 m one.
fn tie(a: f64, b: f64, unit: f64) -> bool {
    (a - b).abs() <= FRAME_TIE_TOL * unit
}

/// (z, y, x) lexicographic order, ties within tolerance — the last resort
/// for faces and edges no shape-intrinsic measure tells apart.
fn cmp_zyx(a: Point3, b: Point3, unit: f64) -> std::cmp::Ordering {
    for (p, q) in [(a.z, b.z), (a.y, b.y), (a.x, b.x)] {
        if !tie(p, q, unit) {
            return p.total_cmp(&q);
        }
    }
    std::cmp::Ordering::Equal
}

/// `a`'s vertices before `b`'s, each list sorted (z, y, x) and compared
/// element by element: the lowest vertex decides, and a face sharing its
/// lowest corner with another is told apart by its next one.
fn lower_face(a: &[Point3], b: &[Point3], unit: f64) -> bool {
    let sorted = |v: &[Point3]| {
        let mut s = v.to_vec();
        s.sort_by(|p, q| cmp_zyx(*p, *q, unit));
        s
    };
    let (sa, sb) = (sorted(a), sorted(b));
    for (p, q) in sa.iter().zip(&sb) {
        match cmp_zyx(*p, *q, unit) {
            std::cmp::Ordering::Equal => continue,
            o => return o == std::cmp::Ordering::Less,
        }
    }
    sa.len() < sb.len()
}

/// The loop rotated to start at its lowest vertex in (z, y, x) order, so
/// "loop order" does not depend on where the kernel happens to start it.
fn canonical_start(mut ring: Vec<Point3>, unit: f64) -> Vec<Point3> {
    if let Some((k, _)) = ring
        .iter()
        .enumerate()
        .min_by(|(_, p), (_, q)| cmp_zyx(**p, **q, unit))
    {
        ring.rotate_left(k);
    }
    ring
}

/// A face's handedness-aware shape signature about the part's centroid
/// `c`: each outer-loop edge's triangle with the face centroid, weighted by
/// its start vertex's squared distance from `c`. Rotations and moves keep
/// it; a face and its mirror image (the top and bottom of an extrusion
/// whose outline has no mirror symmetry) differ in it, because their loops
/// run in opposite senses.
fn face_signature(outer: &[Point3], normal: Vec3, c: Point3) -> f64 {
    let n = outer.len() as f64;
    let fc = outer.iter().fold(Vec3::new(0.0, 0.0, 0.0), |acc, p| {
        acc + (*p - Point3::ORIGIN)
    }) * (1.0 / n);
    let fc = Point3::ORIGIN + fc;
    let mut sum = 0.0;
    for k in 0..outer.len() {
        let (p, q) = (outer[k], outer[(k + 1) % outer.len()]);
        let area = (p - fc).cross(q - fc).dot(normal);
        sum += (p - c).length_squared() * area;
    }
    sum
}

fn dot(p: Point3, v: Vec3) -> f64 {
    p.x * v.x + p.y * v.y + p.z * v.z
}

/// The AABB of `points` along the basis: `(min, max)` coordinates.
fn extents_in(points: &[Point3], basis: [Vec3; 3]) -> Option<([f64; 3], [f64; 3])> {
    let first = points.first()?;
    let mut lo = [0.0; 3];
    let mut hi = [0.0; 3];
    for k in 0..3 {
        lo[k] = dot(*first, basis[k]);
        hi[k] = lo[k];
    }
    for p in &points[1..] {
        for k in 0..3 {
            let c = dot(*p, basis[k]);
            lo[k] = lo[k].min(c);
            hi[k] = hi[k].max(c);
        }
    }
    Some((lo, hi))
}

fn origin_at(basis: [Vec3; 3], c: [f64; 3]) -> Point3 {
    let v = basis[0] * c[0] + basis[1] * c[1] + basis[2] * c[2];
    Point3::new(v.x, v.y, v.z)
}

/// Every vertex of the leaves, in world space.
fn world_points(leaves: &[(ObjectId, Transform)], doc: &Document) -> Vec<Point3> {
    let mut out = Vec::new();
    for (o, pose) in leaves {
        if let Some(obj) = doc.object(*o) {
            out.extend(
                obj.vertices()
                    .values()
                    .map(|v| pose.apply_point(v.position)),
            );
        }
    }
    out
}

/// The canonical frame of a set of world-space leaf placements: z is the
/// outward normal of the largest planar face by area, x the direction of
/// that face's longest outer-loop edge (the loop read counter-clockwise
/// about z), perpendicularized against z, y = z × x, origin = the AABB
/// minimum corner in that basis. `extents` are the AABB extents in the
/// basis.
///
/// Ties are broken first by the part's own shape, so turning or moving a
/// part turns or moves its frame with it: between equal-area faces, the
/// larger [`face_signature`]; between equal-length edges, the one whose
/// start, then end, lies nearer the part's centroid. Only faces or edges
/// the part's own symmetry makes interchangeable (the top and bottom of a
/// plain box) fall through to a world-position rule: the face whose
/// vertices are lowest in (z, y, x) order, the earliest edge reading the
/// loop from its lowest vertex. So the frame is deterministic for a given
/// document and across a save/load round trip, and moves rigidly with the
/// part up to that symmetry. `None` when the leaves have no face.
pub fn canonical_frame(leaves: &[(ObjectId, Transform)], doc: &Document) -> Option<PartFrame> {
    let points = world_points(leaves, doc);
    let first = *points.first()?;
    let (mut lo, mut hi) = (first, first);
    let mut sum = Vec3::new(0.0, 0.0, 0.0);
    for p in &points {
        lo = Point3::new(lo.x.min(p.x), lo.y.min(p.y), lo.z.min(p.z));
        hi = Point3::new(hi.x.max(p.x), hi.y.max(p.y), hi.z.max(p.z));
        sum = sum + (*p - Point3::ORIGIN);
    }
    let centroid = Point3::ORIGIN + sum * (1.0 / points.len() as f64);
    // The part's size: every tie is judged relative to it.
    let size = (hi - lo).length().max(f64::MIN_POSITIVE);

    // (area, signature, normal, outer loop CCW about the normal).
    let mut best: Option<(f64, f64, Vec3, Vec<Point3>)> = None;
    for (o, pose) in leaves {
        let Some(obj) = doc.object(*o) else {
            continue;
        };
        let mirrored = pose.determinant() < 0.0;
        for f in obj.faces().values() {
            let mut outer: Vec<Point3> = obj
                .loop_positions(f.outer_loop)
                .map(|p| pose.apply_point(p))
                .collect();
            if outer.is_empty() {
                continue;
            }
            // A mirrored pose reverses a loop's sense about its (outward)
            // normal; read it back counter-clockwise.
            if mirrored {
                outer.reverse();
            }
            let holes: Vec<Vec<Point3>> = f
                .inner_loops
                .iter()
                .map(|&l| obj.loop_positions(l).map(|p| pose.apply_point(p)).collect())
                .collect();
            let Ok(normal) = pose.apply_normal(f.plane.normal()).normalized() else {
                continue;
            };
            let area = crate::geom::face_area(&outer, &holes);
            let signature = face_signature(&outer, normal, centroid);
            let better = match &best {
                None => true,
                Some((ba, bs, _, bo)) => {
                    if !tie(area, *ba, size * size) {
                        area > *ba
                    } else if !tie(signature, *bs, size.powi(4)) {
                        signature > *bs
                    } else {
                        lower_face(&outer, bo, size)
                    }
                }
            };
            if better {
                best = Some((area, signature, normal, outer));
            }
        }
    }
    let (_, _, z, outer) = best?;
    let outer = canonical_start(outer, size);
    // The longest outer-loop edge; ties: the one starting, then ending,
    // nearer the centroid; then the earliest from the loop's lowest vertex.
    let mut edge: Option<(f64, f64, f64, Vec3)> = None;
    for k in 0..outer.len() {
        let (p, q) = (outer[k], outer[(k + 1) % outer.len()]);
        let d = q - p;
        let len = d.length();
        let (ds, de) = ((p - centroid).length(), (q - centroid).length());
        let better = match &edge {
            None => true,
            Some((l, bs, be, _)) => {
                if !tie(len, *l, size) {
                    len > *l
                } else if !tie(ds, *bs, size) {
                    ds < *bs
                } else if !tie(de, *be, size) {
                    de < *be
                } else {
                    false
                }
            }
        };
        if better {
            edge = Some((len, ds, de, d));
        }
    }
    let (_, _, _, d) = edge?;
    let x = (d - z * d.dot(z)).normalized().ok()?;
    let y = z.cross(x);
    let basis = [x, y, z];
    let (lo, hi) = extents_in(&points, basis)?;
    Some(PartFrame {
        origin: origin_at(basis, lo),
        x,
        y,
        z,
        extents: [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]],
        source: FrameSource::Face,
    })
}

/// Three finite numbers.
fn vec_attr(v: &AttrValue) -> Option<Vec3> {
    let AttrValue::List(items) = v else {
        return None;
    };
    if items.len() != 3 {
        return None;
    }
    let mut c = [0.0; 3];
    for (k, it) in items.iter().enumerate() {
        c[k] = match it {
            AttrValue::Int(i) => *i as f64,
            AttrValue::Float(f) => *f,
            _ => return None,
        };
        if !c[k].is_finite() {
            return None;
        }
    }
    Some(Vec3::new(c[0], c[1], c[2]))
}

/// The `hew.part` / `frame` override on `node`: `(origin, x, y)` in the
/// face frame's coordinates, when present, well-formed, finite, and
/// orthonormal within tolerance.
fn frame_override(doc: &Document, node: NodeId) -> Option<(Vec3, Vec3, Vec3)> {
    let entity = match node {
        NodeId::Object(o) => EntityRef::Object(o),
        NodeId::Group(g) => EntityRef::Group(g),
        NodeId::Instance(i) => EntityRef::Instance(i),
    };
    let dict = doc.attr_get(&AttrTarget::Entity(entity)).ok()??;
    let AttrValue::Map(m) = dict.get(FRAME_NAMESPACE)?.get(FRAME_KEY)? else {
        return None;
    };
    let o = vec_attr(m.get("origin")?)?;
    let x = vec_attr(m.get("x")?)?;
    let y = vec_attr(m.get("y")?)?;
    let unit = |v: Vec3| (v.length() - 1.0).abs() <= OVERRIDE_ORTHO_TOL;
    if !unit(x) || !unit(y) || x.dot(y).abs() > OVERRIDE_ORTHO_TOL {
        return None;
    }
    Some((o, x, y))
}

/// The leaf placements under an occurrence, at world poses.
fn leaves_of(
    doc: &Document,
    occ: &Occurrence,
) -> Result<Vec<(ObjectId, Transform)>, DocumentError> {
    Ok(doc
        .render_root_leaves(&RenderRoot::Occurrence(occ.clone()), true)?
        .into_iter()
        .map(|(o, p, _)| (o, p))
        .collect())
}

/// The frame of one placement (rules in docs/agents/PLUGINS.md "Reading
/// parts"): an instance's definition axes; else an object's or group's
/// largest-face frame, extents ordered `[length, width, thickness]`, or
/// its `hew.part` / `frame` override read in that frame. `Ok(None)` when
/// the placement has nothing to measure.
///
/// # Errors
/// [`DocumentError::TooManyOccurrences`] — the placement holds more leaf
/// placements than one walk lists.
pub fn part_frame(doc: &Document, occ: &Occurrence) -> Result<Option<PartFrame>, DocumentError> {
    match occ.node {
        NodeId::Instance(_) => {
            let Some(def) = occ.definition else {
                return Ok(None);
            };
            let Some((lo, hi)) = doc.definition_bounds_checked(def)? else {
                return Ok(None);
            };
            let a = occ.pose.to_affine();
            let col = |j: usize| Vec3::new(a[j], a[4 + j], a[8 + j]);
            let (cx, cy, cz) = (col(0), col(1), col(2));
            let (Ok(x), Ok(y), Ok(z)) = (cx.normalized(), cy.normalized(), cz.normalized()) else {
                return Ok(None);
            };
            let y = if occ.pose.determinant() < 0.0 { -y } else { y };
            Ok(Some(PartFrame {
                origin: Point3::new(a[3], a[7], a[11]),
                x,
                y,
                z,
                extents: [
                    (hi.x - lo.x) * cx.length(),
                    (hi.y - lo.y) * cy.length(),
                    (hi.z - lo.z) * cz.length(),
                ],
                source: FrameSource::Definition,
            }))
        }
        NodeId::Object(_) | NodeId::Group(_) => {
            let leaves = leaves_of(doc, occ)?;
            let Some(c) = canonical_frame(&leaves, doc) else {
                return Ok(None);
            };
            let face = ordered(c, &leaves, doc);
            if let Some(f) = override_frame(doc, occ.node, &face, &leaves) {
                return Ok(Some(f));
            }
            Ok(Some(face))
        }
    }
}

/// Rule 2: the override read in the face frame's coordinates — the frame
/// `part_frame` reports for the part without the attribute, so a plugin
/// can echo reported axes back.
fn override_frame(
    doc: &Document,
    node: NodeId,
    face: &PartFrame,
    leaves: &[(ObjectId, Transform)],
) -> Option<PartFrame> {
    let (o, ox, oy) = frame_override(doc, node)?;
    let map = |v: Vec3| face.x * v.x + face.y * v.y + face.z * v.z;
    let origin = face.origin + map(o);
    let x = map(ox).normalized().ok()?;
    let yy = map(oy);
    let y = (yy - x * yy.dot(x)).normalized().ok()?;
    let z = x.cross(y);
    let points = world_points(leaves, doc);
    let (lo, hi) = extents_in(&points, [x, y, z])?;
    Some(PartFrame {
        origin,
        x,
        y,
        z,
        extents: [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]],
        source: FrameSource::Attribute,
    })
}

/// Rule 3: `[l, w, t]` — x swapped with y when the y extent is larger by
/// more than float noise (a square part keeps its face frame), z recomputed
/// to stay right-handed, origin recomputed in the new basis.
fn ordered(c: PartFrame, leaves: &[(ObjectId, Transform)], doc: &Document) -> PartFrame {
    let size = c.extents.iter().fold(0.0_f64, |a, &e| a.max(e));
    if c.extents[1] <= c.extents[0] || tie(c.extents[1], c.extents[0], size) {
        return c;
    }
    let (x, y) = (c.y, c.x);
    let z = x.cross(y);
    let points = world_points(leaves, doc);
    match extents_in(&points, [x, y, z]) {
        Some((lo, hi)) => PartFrame {
            origin: origin_at([x, y, z], lo),
            x,
            y,
            z,
            extents: [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]],
            source: FrameSource::Face,
        },
        None => c,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kernel::{AttrValue, Plane};
    use std::collections::BTreeMap;

    fn ground() -> Plane {
        Plane::from_polygon(&[
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(1.0, 0.0, 0.0),
            Point3::new(0.0, 1.0, 0.0),
        ])
        .unwrap()
    }

    fn extrude_box(doc: &mut Document, x1: f64, y1: f64, h: f64) -> ObjectId {
        let s = doc.add_sketch(ground());
        let c = [
            Point3::new(0.0, 0.0, 0.0),
            Point3::new(x1, 0.0, 0.0),
            Point3::new(x1, y1, 0.0),
            Point3::new(0.0, y1, 0.0),
        ];
        let sk = doc.sketch_mut(s).unwrap();
        for k in 0..4 {
            sk.add_segment(c[k], c[(k + 1) % 4]).unwrap();
        }
        let regions = doc.extrudable_regions(s).unwrap();
        doc.extrude_region(s, regions[0], h).unwrap().0
    }

    fn rotation() -> Transform {
        Transform::rotation(Vec3::new(0.0, 0.0, 1.0), 30f64.to_radians())
            .unwrap()
            .then(&Transform::rotation(Vec3::new(1.0, 0.0, 0.0), 15f64.to_radians()).unwrap())
    }

    /// A 400 × 200 × 18 mm board turned 30° about z then 15° about x.
    fn rotated_board() -> (Document, ObjectId) {
        let mut doc = Document::new();
        let b = extrude_box(&mut doc, 0.4, 0.2, 0.018);
        doc.transform_object(b, &rotation()).unwrap();
        (doc, b)
    }

    fn pf(doc: &Document, o: &Occurrence) -> PartFrame {
        part_frame(doc, o).unwrap().unwrap()
    }

    fn occ(doc: &Document, node: NodeId) -> Occurrence {
        doc.occurrence_at(&[], node).unwrap()
    }

    fn assert_extents(got: [f64; 3], want: [f64; 3]) {
        for k in 0..3 {
            assert!((got[k] - want[k]).abs() < 1e-9, "{got:?} vs {want:?}");
        }
    }

    #[test]
    fn a_rotated_board_measures_its_true_size() {
        let (doc, b) = rotated_board();
        let f = pf(&doc, &occ(&doc, NodeId::Object(b)));
        assert_eq!(f.source, FrameSource::Face);
        assert_extents(f.extents, [0.4, 0.2, 0.018]);
        let n = rotation()
            .apply_normal(Vec3::new(0.0, 0.0, 1.0))
            .normalized()
            .unwrap();
        assert!(f.z.cross(n).length() < 1e-9, "z ∥ the rotated face normal");
        // Right-handed and orthonormal.
        assert!((f.x.cross(f.y) - f.z).length() < 1e-9);
        assert!(f.x.dot(f.y).abs() < 1e-12);
    }

    #[test]
    fn a_cube_is_deterministic_across_calls_and_a_save_load_round_trip() {
        let mut doc = Document::new();
        let c = extrude_box(&mut doc, 0.3, 0.3, 0.3);
        let a = pf(&doc, &occ(&doc, NodeId::Object(c)));
        let b = pf(&doc, &occ(&doc, NodeId::Object(c)));
        assert_eq!(a, b);
        let re = Document::load(&doc.save()).unwrap();
        let id = re.visible_object_ids()[0];
        let r = pf(&re, &occ(&re, NodeId::Object(id)));
        assert_eq!(a, r);
    }

    #[test]
    fn an_instance_reports_definition_extents_times_its_scale() {
        let (mut doc, b) = rotated_board();
        let (cid, inst, _) = doc.make_component(&[NodeId::Object(b)]).unwrap();
        doc.transform_instance(inst, &Transform::scale(Vec3::new(2.0, 1.0, 0.5)))
            .unwrap();
        let o = occ(&doc, NodeId::Instance(inst));
        let f = pf(&doc, &o);
        assert_eq!(f.source, FrameSource::Definition);
        let (lo, hi) = doc.definition_bounds(cid).unwrap();
        assert_extents(
            f.extents,
            [(hi.x - lo.x) * 2.0, hi.y - lo.y, (hi.z - lo.z) * 0.5],
        );
        // A mirrored placement stays right-handed.
        doc.transform_instance(inst, &Transform::scale(Vec3::new(-1.0, 1.0, 1.0)))
            .unwrap();
        let f = pf(&doc, &occ(&doc, NodeId::Instance(inst)));
        assert!((f.x.cross(f.y) - f.z).length() < 1e-9);
    }

    fn set_override(doc: &mut Document, node: NodeId, x: [f64; 3], y: [f64; 3]) {
        set_override_at(doc, node, [0.0, 0.0, 0.0], x, y);
    }

    fn set_override_at(
        doc: &mut Document,
        node: NodeId,
        origin: [f64; 3],
        x: [f64; 3],
        y: [f64; 3],
    ) {
        let list = |v: [f64; 3]| AttrValue::List(v.iter().map(|&c| AttrValue::Float(c)).collect());
        let mut m = BTreeMap::new();
        m.insert("origin".to_string(), list(origin));
        m.insert("x".to_string(), list(x));
        m.insert("y".to_string(), list(y));
        let entity = match node {
            NodeId::Object(o) => EntityRef::Object(o),
            NodeId::Group(g) => EntityRef::Group(g),
            NodeId::Instance(i) => EntityRef::Instance(i),
        };
        doc.attr_set(
            AttrTarget::Entity(entity),
            FRAME_NAMESPACE,
            FRAME_KEY,
            AttrValue::Map(m),
        )
        .unwrap();
    }

    #[test]
    fn an_override_swaps_axes() {
        let (mut doc, b) = rotated_board();
        let node = NodeId::Object(b);
        let face = pf(&doc, &occ(&doc, node));
        set_override(&mut doc, node, [0.0, 1.0, 0.0], [1.0, 0.0, 0.0]);
        let f = pf(&doc, &occ(&doc, node));
        assert_eq!(f.source, FrameSource::Attribute);
        assert!((f.x - face.y).length() < 1e-9);
        assert!((f.y - face.x).length() < 1e-9);
        assert!((f.z + face.z).length() < 1e-9);
        assert_extents(f.extents, [0.2, 0.4, 0.018]);
    }

    /// The override is read in the face frame the part reports, so echoing
    /// that frame back changes nothing — even where rule 3 swapped it.
    #[test]
    fn echoing_the_reported_face_frame_changes_nothing() {
        let (mut doc, g) = plate_and_stick();
        let node = NodeId::Group(g);
        let face = pf(&doc, &occ(&doc, node));
        set_override(&mut doc, node, [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        let f = pf(&doc, &occ(&doc, node));
        assert_eq!(f.source, FrameSource::Attribute);
        for (a, b) in [(f.x, face.x), (f.y, face.y), (f.z, face.z)] {
            assert!((a - b).length() < 1e-9, "{a:?} vs {b:?}");
        }
        assert!((f.origin - face.origin).length() < 1e-9);
        assert_extents(f.extents, face.extents);
    }

    /// A plate 0.4 × 0.2 × 0.018 plus a stick lying along y past it, grouped:
    /// the plate's face gives the frame, but the group is longer along that
    /// frame's y, so rule 3 swaps.
    fn plate_and_stick() -> (Document, kernel::GroupId) {
        let mut doc = Document::new();
        let plate = extrude_box(&mut doc, 0.4, 0.2, 0.018);
        let stick = extrude_poly(
            &mut doc,
            &[(0.0, 0.2), (0.05, 0.2), (0.05, 0.8), (0.0, 0.8)],
            0.05,
        );
        let (g, _) = doc
            .group_nodes(&[NodeId::Object(plate), NodeId::Object(stick)])
            .unwrap();
        (doc, g)
    }

    fn extrude_poly(doc: &mut Document, pts: &[(f64, f64)], h: f64) -> ObjectId {
        let s = doc.add_sketch(ground());
        let sk = doc.sketch_mut(s).unwrap();
        for k in 0..pts.len() {
            let (a, b) = (pts[k], pts[(k + 1) % pts.len()]);
            sk.add_segment(Point3::new(a.0, a.1, 0.0), Point3::new(b.0, b.1, 0.0))
                .unwrap();
        }
        let regions = doc.extrudable_regions(s).unwrap();
        doc.extrude_region(s, regions[0], h).unwrap().0
    }

    fn right_handed(f: &PartFrame) {
        assert!((f.x.cross(f.y) - f.z).length() < 1e-9, "{f:?}");
        assert!(f.x.dot(f.y).abs() < 1e-9 && f.x.dot(f.z).abs() < 1e-9);
    }

    #[test]
    fn rule_three_swaps_to_length_width_thickness_and_stays_right_handed() {
        let (doc, g) = plate_and_stick();
        let f = pf(&doc, &occ(&doc, NodeId::Group(g)));
        assert_eq!(f.source, FrameSource::Face);
        assert_extents(f.extents, [0.8, 0.4, 0.05]);
        right_handed(&f);
        assert!(f.x.cross(Vec3::new(0.0, 1.0, 0.0)).length() < 1e-9, "x ∥ y");
        // The origin is the box's minimum corner in the frame: every vertex
        // sits within [0, extent] along each axis from it.
        let o = occ(&doc, NodeId::Group(g));
        for p in world_points(&leaves_of(&doc, &o).unwrap(), &doc) {
            let d = p - f.origin;
            for (k, axis) in [f.x, f.y, f.z].into_iter().enumerate() {
                let c = d.dot(axis);
                assert!(c > -1e-9 && c < f.extents[k] + 1e-9, "{c} on axis {k}");
            }
        }
    }

    /// A square board turned by any angle keeps its face frame: the swap
    /// does not fire on the float noise between its two in-plane extents.
    #[test]
    fn a_square_board_never_swaps_on_float_noise() {
        for deg_z in (1..90).step_by(4) {
            for deg_x in [7.0_f64, 21.0, 35.0] {
                let mut doc = Document::new();
                let b = extrude_box(&mut doc, 0.3, 0.3, 0.018);
                let r =
                    Transform::rotation(Vec3::new(0.0, 0.0, 1.0), f64::from(deg_z).to_radians())
                        .unwrap()
                        .then(
                            &Transform::rotation(Vec3::new(1.0, 0.0, 0.0), deg_x.to_radians())
                                .unwrap(),
                        );
                doc.transform_object(b, &r).unwrap();
                let o = occ(&doc, NodeId::Object(b));
                let c = canonical_frame(&leaves_of(&doc, &o).unwrap(), &doc).unwrap();
                let f = pf(&doc, &o);
                assert_eq!(f, c, "{deg_z}° / {deg_x}°");
            }
        }
    }

    /// An object inside a mirrored placement still gets a right-handed
    /// frame with its true size.
    #[test]
    fn an_object_under_a_mirrored_placement_is_right_handed() {
        let mut doc = Document::new();
        let b = extrude_box(&mut doc, 0.4, 0.2, 0.018);
        let (_, inst, _) = doc.make_component(&[NodeId::Object(b)]).unwrap();
        doc.transform_instance(inst, &Transform::scale(Vec3::new(-1.0, 1.0, 1.0)))
            .unwrap();
        let o = doc.occurrence_at(&[inst], NodeId::Object(b)).unwrap();
        assert!(o.pose.determinant() < 0.0);
        let f = pf(&doc, &o);
        assert_eq!(f.source, FrameSource::Face);
        assert_extents(f.extents, [0.4, 0.2, 0.018]);
        right_handed(&f);
        // z is the mirrored outward normal of the largest face: ±z.
        assert!(f.z.cross(Vec3::new(0.0, 0.0, 1.0)).length() < 1e-9);
    }

    /// An L whose legs differ has no symmetry, so turning it turns its face
    /// frame and its override with it exactly.
    #[test]
    fn the_face_frame_and_an_override_turn_with_an_asymmetric_part() {
        let l = [
            (0.0, 0.0),
            (0.5, 0.0),
            (0.5, 0.1),
            (0.1, 0.1),
            (0.1, 0.3),
            (0.0, 0.3),
        ];
        let mut doc = Document::new();
        let o = extrude_poly(&mut doc, &l, 0.02);
        let node = NodeId::Object(o);
        set_override_at(
            &mut doc,
            node,
            [0.01, 0.02, 0.003],
            [0.0, 1.0, 0.0],
            [1.0, 0.0, 0.0],
        );
        let before = pf(&doc, &occ(&doc, node));
        let mut face_doc = Document::new();
        let fo = extrude_poly(&mut face_doc, &l, 0.02);
        let face_before = pf(&face_doc, &occ(&face_doc, NodeId::Object(fo)));
        for (k, axis) in [
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(1.0, 1.0, 0.3),
            Vec3::new(-0.2, 0.7, 1.0),
        ]
        .into_iter()
        .enumerate()
        {
            let r = Transform::rotation(axis, 0.4 + 0.9 * k as f64)
                .unwrap()
                .then(&Transform::translation(Vec3::new(1.0, -2.0, 0.5)));
            for (d, n, b) in [
                (&mut doc, node, &before),
                (&mut face_doc, NodeId::Object(fo), &face_before),
            ] {
                let NodeId::Object(id) = n else {
                    unreachable!()
                };
                let mut turned = d.clone();
                turned.transform_object(id, &r).unwrap();
                let f = pf(&turned, &occ(&turned, n));
                for (a, w) in [(f.x, b.x), (f.y, b.y), (f.z, b.z)] {
                    assert!((a - r.apply_vector(w)).length() < 1e-9, "{k}: {a:?}");
                }
                assert!((f.origin - r.apply_point(b.origin)).length() < 1e-9);
                assert_extents(f.extents, b.extents);
            }
        }
    }

    #[test]
    fn a_non_orthonormal_override_falls_through_to_face() {
        let (mut doc, b) = rotated_board();
        let node = NodeId::Object(b);
        set_override(&mut doc, node, [1.0, 0.0, 0.0], [0.5, 0.5, 0.0]);
        let f = pf(&doc, &occ(&doc, node));
        assert_eq!(f.source, FrameSource::Face);
        assert_extents(f.extents, [0.4, 0.2, 0.018]);
    }
}

//! The skin builder (true-curves.md §5.2, §10): a solid from a sequence of
//! equal-length rings plus caps. Follow Me's twist and end scale build on
//! it, and so do Deform and Loft, so a generated sweep has one assembly
//! path: planar bands where the four corners are coplanar, two triangles
//! with a soft diagonal where they are not, soft seams where the profile
//! or the path is smooth, and planar caps.

use std::collections::BTreeMap;

use crate::material::FaceMaterial;
use crate::math::{Plane, Point3, Vec3};
use crate::patch::{EdgeMeta, PVert, PatchFace, PolyPatch, self_intersection};
use crate::shape_error::ShapeError;
use crate::surface;
use crate::tol;
use crate::topo::{FaceAttrs, Object, SurfaceRef};

/// What [`build_skin`] builds.
#[derive(Debug, Clone)]
pub(crate) struct SkinSpec {
    /// `stations[s][l]` = loop `l` (0 = outer, 1.. = holes) at station `s`.
    /// Every station has the same loop count and each loop the same point
    /// count across stations. The LAST station may instead be a single
    /// point (an apex): `stations[last] = vec![vec![apex]]`, allowed only
    /// when the other stations have exactly one loop.
    pub stations: Vec<Vec<Vec<Point3>>>,
    /// `column_smooth[l][j]`: the longitudinal line through point `j` of
    /// loop `l` is a smooth seam (soft): true inside a drawn curve, false at
    /// a corner.
    pub column_smooth: Vec<Vec<bool>>,
    /// `station_smooth[s]` for every station (only interior entries are
    /// read): the transverse seam at station `s` is soft.
    pub station_smooth: Vec<bool>,
    /// Cap the first / last station with planar faces (outer + holes).
    pub cap_start: bool,
    pub cap_end: bool,
    /// `claims[s][l][j]`: candidate claim for the band face between
    /// stations `s` and `s+1`, columns `j` and `j+1` of loop `l`. Stamped
    /// only where it holds on the face's own vertices.
    pub claims: Option<Vec<Vec<Vec<Option<SurfaceRef>>>>>,
    /// Material for every face.
    pub material: FaceMaterial,
    /// Where hard columns fade (`None`: hard all the way, the column's own
    /// rule): see [`ColumnFade`]. Loft's columns fade out between the
    /// profiles they are a corner of.
    pub column_fade: Option<ColumnFade>,
}

/// How a hard column of loop 0 creases along each band (`rule[s][j]` for
/// band `s`, column `j`; see [`Fade`]; D-126 R4).
#[derive(Debug, Clone)]
pub(crate) struct ColumnFade {
    /// The least fold (a difference of the two band faces' unit normals)
    /// by which a [`Fade::Stand`] column must exceed its reference to
    /// crease (the reference doubled, if more).
    pub fold: f64,
    pub rule: Vec<Vec<Fade>>,
    /// Per band, the facet floor of its span (D-128), as a fold (`2 sin(θ/2)`
    /// of the angle): a one-sided column's reference is at least this, so
    /// it creases only against more than the turn of a curve's own facets
    /// in the span's profiles. Empty: no floor.
    pub floor: Vec<f64>,
    /// Per band per column, the turn (an angle) of the corner a one-sided
    /// column is at its own profile (D-129 A): it also creases where it
    /// folds more than two thirds of that turn. Empty: no guard.
    pub turn: Vec<Vec<f64>>,
}

/// One hard column's seam along one band.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Fade {
    /// A crease (the column is a corner in both profiles of the span).
    Crease,
    /// A crease only where it stands out: where its fold on this band
    /// exceeds `max(reference + fold, 2 · reference)`, the reference being
    /// the largest soft fold of the nearest plain quad (neither column
    /// one-sided) along the band each way, or the span's facet floor (the
    /// largest turn at a vertex of either profile that is no corner: a
    /// curve's own facets) where that is more (see `stands_out`, D-128). The
    /// column is a corner in one of the span's profiles: the one at the
    /// band's start (its lower station) or end. Its band quads fan from
    /// the corner (D-126 faceting).
    Stand(Side),
    /// Smooth (the column is a corner in neither profile of the span).
    Soft,
}

/// Which profile of a span a one-sided column is a corner of: the one at
/// the span's start (nearer the band's lower station) or its end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Side {
    Start,
    End,
}

/// A diagonal the fan rule placed: its two points, the band and loop, the
/// one-sided columns it serves, and the normals of its two triangles.
struct FanDiagonal {
    ends: (usize, usize),
    band: usize,
    lp: usize,
    serves: Vec<usize>,
    normals: (Option<Vec3>, Option<Vec3>),
}

/// A band's piece normals at its left and right columns.
type BandSides = (Option<Vec3>, Option<Vec3>);

/// Whether a fold stands out from its reference (D-126), compared as
/// angles: more than `max(reference + threshold, 2 · reference)`, so a seam
/// folding twice the angle of the surface beside it creases and one a
/// little sharper does not. The arguments are differences of unit normals
/// (`2 sin(θ/2)` for an angle θ), as the folds are measured.
fn stands_out(fold: f64, reference: f64, threshold: f64) -> bool {
    let angle = |chord: f64| 2.0 * (chord / 2.0).clamp(0.0, 1.0).asin();
    let (f, r, t) = (angle(fold), angle(reference), angle(threshold));
    f > (r + t).max(2.0 * r)
}

/// Whether a one-sided column (or a fan diagonal serving one) creases
/// (D-126 R4, D-128, D-129 A): where its fold stands out from its
/// reference (which includes the facet floor), or, as a guard, where it
/// folds more than two thirds of its corner's own turn `turn` and twice the
/// floor, and more than the threshold. Folds, the reference, the floor
/// and the threshold are normal differences (`2 sin(θ/2)`); `turn` is an
/// angle. The guard keeps a real corner beside a steeply folded skirt (a
/// spike, a square's corner beside a coarse round one); a corner whose
/// fold is between half and two thirds of its turn there still softens.
fn creases(fold: f64, reference: f64, turn: f64, floor: f64, threshold: f64) -> bool {
    let angle = |chord: f64| 2.0 * (chord / 2.0).clamp(0.0, 1.0).asin();
    let (f, fl, t) = (angle(fold), angle(floor), angle(threshold));
    stands_out(fold, reference, threshold) || (f > (2.0 * turn / 3.0).max(2.0 * fl) && f > t)
}

/// The fan rule (D-126 faceting) for a non-planar quad between columns j
/// and j + 1 under their rules: `Some((true, ..))` for the q1–q3 diagonal,
/// `Some((false, ..))` for q0–q2, with which columns it serves (`false` j,
/// `true` j + 1); `None` where the shorter diagonal decides (no one-sided
/// column, or two asking different diagonals).
fn fan_choice(left: Option<Fade>, right: Option<Fade>) -> Option<(bool, Vec<bool>)> {
    // Column j toward the start: q0–q2; toward the end: q3–q1. Column
    // j + 1 toward the start: q1–q3; toward the end: q2–q0.
    let pj = match left {
        Some(Fade::Stand(side)) => Some(side == Side::End),
        _ => None,
    };
    let pk = match right {
        Some(Fade::Stand(side)) => Some(side == Side::Start),
        _ => None,
    };
    match (pj, pk) {
        (Some(a), None) => Some((a, vec![false])),
        (None, Some(b)) => Some((b, vec![true])),
        (Some(a), Some(b)) if a == b => Some((a, vec![false, true])),
        _ => None,
    }
}

fn newell(pts: &[Point3]) -> Option<Plane> {
    Plane::from_polygon(pts).ok()
}

fn planar(pts: &[Point3]) -> bool {
    newell(pts).is_some_and(|pl| {
        pts.iter()
            .all(|&p| pl.signed_distance(p).abs() <= tol::PLANE_DIST)
    })
}

/// Six times the signed volume a closed polygon set encloses (fan
/// triangles; exact for planar polygons of any convexity).
fn six_volume(verts: &[Point3], faces: &[PatchFace]) -> f64 {
    let mut v = 0.0;
    for f in faces {
        for lp in std::iter::once(&f.outer).chain(f.holes.iter()) {
            let p0 = verts[lp[0]].to_vec();
            for k in 1..lp.len() - 1 {
                let (p1, p2) = (verts[lp[k]].to_vec(), verts[lp[k + 1]].to_vec());
                v += p0.dot(p1.cross(p2));
            }
        }
    }
    v
}

/// Builds the solid `spec` describes: validated and watertight, oriented
/// outward, checked for self-intersection.
///
/// # Errors
/// [`ShapeError::ShapeDegenerate`] for a malformed spec or invalid
/// topology; [`ShapeError::SelfIntersecting`] (no limit) when two faces
/// contact away from their shared edges.
pub(crate) fn build_skin(spec: &SkinSpec) -> Result<Object, ShapeError> {
    let bad = ShapeError::ShapeDegenerate;
    let st = &spec.stations;
    if st.len() < 2 {
        return Err(bad);
    }
    let loops = st[0].len();
    let last = st.len() - 1;
    let apex = st[last].len() == 1 && st[last][0].len() == 1 && st[0][0].len() > 1;
    if loops == 0 || (apex && (loops != 1 || spec.cap_end)) {
        return Err(bad);
    }
    let ring_stations = if apex { last } else { st.len() };
    for ring in &st[..ring_stations] {
        if ring.len() != loops {
            return Err(bad);
        }
        for (l, lp) in ring.iter().enumerate() {
            if lp.len() < 3 || lp.len() != st[0][l].len() {
                return Err(bad);
            }
        }
    }
    if spec.column_smooth.len() != loops
        || spec
            .column_smooth
            .iter()
            .zip(&st[0])
            .any(|(c, lp)| c.len() != lp.len())
    {
        return Err(bad);
    }

    // Points: one patch vertex per station point (the apex is one point).
    let mut verts: Vec<PVert> = Vec::new();
    let mut pos: Vec<Point3> = Vec::new();
    let mut idx: Vec<Vec<Vec<usize>>> = Vec::with_capacity(st.len());
    for ring in &st[..ring_stations] {
        let mut r = Vec::with_capacity(loops);
        for lp in ring {
            let mut v = Vec::with_capacity(lp.len());
            for &p in lp {
                v.push(verts.len());
                verts.push(PVert::New(p));
                pos.push(p);
            }
            r.push(v);
        }
        idx.push(r);
    }
    let apex_idx = apex.then(|| {
        let p = st[last][0][0];
        verts.push(PVert::New(p));
        pos.push(p);
        pos.len() - 1
    });
    let at = |s: usize, l: usize, j: usize| -> usize {
        match apex_idx {
            Some(a) if s == last => a,
            _ => idx[s][l][j],
        }
    };

    let attrs = |surface: Option<SurfaceRef>| FaceAttrs {
        material: spec.material,
        back_material: None,
        uv_frame: None,
        surface,
        blend: None,
        attrs: crate::topo::AttrSlot::NONE,
    };
    let mut faces: Vec<PatchFace> = Vec::new();
    let mut claims_of: Vec<Option<SurfaceRef>> = Vec::new();
    let mut meta: BTreeMap<(usize, usize), EdgeMeta> = BTreeMap::new();
    let key = |a: usize, b: usize| if a < b { (a, b) } else { (b, a) };
    let soft = |meta: &mut BTreeMap<(usize, usize), EdgeMeta>, a: usize, b: usize| {
        meta.insert(
            key(a, b),
            EdgeMeta {
                soft: true,
                ..EdgeMeta::default()
            },
        );
    };

    // Bands. `sides[(s, l, j)]`: the normals of band `j`'s pieces touching
    // its left column (`j`) and its right one (`j + 1`), for the fold of a
    // hard column.
    let lens: Vec<usize> = st[0].iter().map(Vec::len).collect();
    let mut fans: Vec<FanDiagonal> = Vec::new();
    // The fold across each split quad's diagonal, by (band, loop, quad).
    let mut diagonal_fold: BTreeMap<(usize, usize, usize), f64> = BTreeMap::new();
    let unit = |ids: &[usize]| -> Option<Vec3> {
        let pts: Vec<Point3> = ids.iter().map(|&i| pos[i]).collect();
        newell(&pts).map(|pl| pl.normal())
    };
    let mut sides: BTreeMap<(usize, usize, usize), BandSides> = BTreeMap::new();
    for s in 0..last {
        for (l, &n) in lens.iter().enumerate() {
            for j in 0..n {
                let k = (j + 1) % n;
                let q = [at(s, l, j), at(s, l, k), at(s + 1, l, k), at(s + 1, l, j)];
                let claim = spec
                    .claims
                    .as_ref()
                    .and_then(|c| c.get(s)?.get(l)?.get(j).copied().flatten());
                if q[2] == q[3] {
                    let nt = unit(&[q[0], q[1], q[2]]);
                    sides.insert((s, l, j), (nt, nt));
                    faces.push(PatchFace {
                        outer: vec![q[0], q[1], q[2]],
                        holes: vec![],
                        attrs: attrs(None),
                        plane: None,
                    });
                    claims_of.push(claim);
                    continue;
                }
                let corners: Vec<Point3> = q.iter().map(|&i| pos[i]).collect();
                if planar(&corners) {
                    let nq = unit(&q);
                    sides.insert((s, l, j), (nq, nq));
                    faces.push(PatchFace {
                        outer: q.to_vec(),
                        holes: vec![],
                        attrs: attrs(None),
                        plane: None,
                    });
                    claims_of.push(claim);
                } else {
                    // A column that is a corner in one profile of the span
                    // fans from that corner (D-126 faceting): column j
                    // toward the start takes q0–q2, toward the end q3–q1;
                    // column j + 1 toward the start q1–q3, toward the end
                    // q2–q0. One such column, or two asking the same
                    // diagonal, decide; otherwise the shorter diagonal: the
                    // one that stays nearest the ruled surface between the
                    // two rings (a long one can cut across the solid on a
                    // strong twist).
                    let d02 = (pos[q[2]] - pos[q[0]]).length();
                    let d13 = (pos[q[3]] - pos[q[1]]).length();
                    let fan = (l == 0)
                        .then_some(spec.column_fade.as_ref())
                        .flatten()
                        .and_then(|fade| {
                            fan_choice(
                                fade.rule.get(s).and_then(|r| r.get(j)).copied(),
                                fade.rule.get(s).and_then(|r| r.get(k)).copied(),
                            )
                        });
                    let across13 = match fan {
                        Some((d, _)) => d,
                        None => d13 < d02 - tol::POINT_MERGE,
                    };
                    let (tris, diag) = if across13 {
                        ([[q[0], q[1], q[3]], [q[1], q[2], q[3]]], (q[1], q[3]))
                    } else {
                        ([[q[0], q[1], q[2]], [q[0], q[2], q[3]]], (q[0], q[2]))
                    };
                    // The left column is q0–q3, the right one q1–q2.
                    let touching = |a: usize, b: usize| {
                        tris.iter()
                            .find(|t| t.contains(&a) && t.contains(&b))
                            .and_then(|t| unit(t))
                    };
                    sides.insert((s, l, j), (touching(q[0], q[3]), touching(q[1], q[2])));
                    for tri in tris {
                        faces.push(PatchFace {
                            outer: tri.to_vec(),
                            holes: vec![],
                            attrs: attrs(None),
                            plane: None,
                        });
                        claims_of.push(claim);
                    }
                    if let (Some(a), Some(b)) = (unit(&tris[0]), unit(&tris[1])) {
                        diagonal_fold.insert((s, l, j), (a - b).length());
                    }
                    match fan {
                        // Only a fan diagonal may crease: decided below,
                        // once every band's folds are known.
                        Some((_, serves)) => fans.push(FanDiagonal {
                            ends: diag,
                            band: s,
                            lp: l,
                            serves: serves.into_iter().map(|c| if c { k } else { j }).collect(),
                            normals: (unit(&tris[0]), unit(&tris[1])),
                        }),
                        None => soft(&mut meta, diag.0, diag.1),
                    }
                }
            }
        }
    }

    // Seams: longitudinal per column, transverse per interior station.
    // The fold at column `x` of band `s` (loop `l`, `n` columns), and the
    // reference a one-sided column there is measured against (D-126 R4):
    // the larger fold of the nearest column each way that is a corner in
    // neither profile of the span (none: 0).
    let rule_at = |s: usize, x: usize| {
        spec.column_fade
            .as_ref()
            .and_then(|fade| fade.rule.get(s).and_then(|r| r.get(x)).copied())
    };
    let fold_at = |s: usize, l: usize, n: usize, x: usize| -> Option<f64> {
        let left = sides.get(&(s, l, x)).and_then(|p| p.0);
        let right = sides.get(&(s, l, (x + n - 1) % n)).and_then(|p| p.1);
        left.zip(right).map(|(a, b)| (a - b).length())
    };
    let one_sided = |s: usize, x: usize| matches!(rule_at(s, x), Some(Fade::Stand(_)));
    let turn_at = |s: usize, x: usize| {
        spec.column_fade
            .as_ref()
            .and_then(|f| f.turn.get(s).and_then(|r| r.get(x)).copied())
            .unwrap_or(0.0)
    };
    let floor_at = |s: usize| {
        spec.column_fade
            .as_ref()
            .and_then(|f| f.floor.get(s).copied())
            .unwrap_or(0.0)
    };
    // Walking from column `j` along the band to the nearest plain quad
    // each way (neither of its columns one-sided): the largest fold among
    // its soft edges (its smooth column seams and its diagonal if split);
    // the larger of the two sides, a side without one counting for
    // nothing (none: 0).
    let reference = |s: usize, l: usize, n: usize, j: usize| -> f64 {
        let plain = |q: usize| !one_sided(s, q) && !one_sided(s, (q + 1) % n);
        let soft_folds = |q: usize| -> f64 {
            let mut m = diagonal_fold.get(&(s, l, q)).copied().unwrap_or(0.0);
            for c in [q, (q + 1) % n] {
                if rule_at(s, c) == Some(Fade::Soft)
                    && let Some(f) = fold_at(s, l, n, c)
                {
                    m = m.max(f);
                }
            }
            m
        };
        // Quad q lies between columns q and q + 1: to the left of j the
        // first is quad j − 1, to the right quad j.
        let left = (1..=n).map(|d| (j + n - d) % n).find(|&q| plain(q));
        let right = (0..n).map(|d| (j + d) % n).find(|&q| plain(q));
        let floor = spec
            .column_fade
            .as_ref()
            .and_then(|f| f.floor.get(s).copied())
            .unwrap_or(0.0);
        [left, right]
            .into_iter()
            .flatten()
            .map(soft_folds)
            .fold(floor, f64::max)
    };
    for s in 0..last {
        for (l, &n) in lens.iter().enumerate() {
            for j in 0..n {
                let faded =
                    spec.column_fade
                        .as_ref()
                        .is_some_and(|fade| match (l, rule_at(s, j)) {
                            (0, Some(Fade::Soft)) => true,
                            (0, Some(Fade::Stand(_))) => {
                                let r = reference(s, l, n, j);
                                fold_at(s, l, n, j).is_some_and(|f| {
                                    !creases(f, r, turn_at(s, j), floor_at(s), fade.fold)
                                })
                            }
                            _ => false,
                        });
                if spec.column_smooth[l][j] || faded {
                    soft(&mut meta, at(s, l, j), at(s + 1, l, j));
                }
            }
        }
    }
    // A fan diagonal creases where it stands out by the same measure,
    // against the reference of the column it serves (the larger of two).
    for fd in &fans {
        let n = lens[fd.lp];
        let threshold = spec.column_fade.as_ref().map_or(0.0, |f| f.fold);
        let r = fd
            .serves
            .iter()
            .map(|&c| reference(fd.band, fd.lp, n, c))
            .fold(0.0_f64, f64::max);
        let fold = match fd.normals {
            (Some(a), Some(b)) => (a - b).length(),
            _ => 0.0,
        };
        let turn = fd
            .serves
            .iter()
            .map(|&c| turn_at(fd.band, c))
            .fold(0.0_f64, f64::max);
        if !creases(fold, r, turn, floor_at(fd.band), threshold) {
            soft(&mut meta, fd.ends.0, fd.ends.1);
        }
    }
    for s in 1..last {
        if !spec.station_smooth.get(s).copied().unwrap_or(false) {
            continue;
        }
        for (l, &n) in lens.iter().enumerate() {
            for j in 0..n {
                soft(&mut meta, at(s, l, j), at(s, l, (j + 1) % n));
            }
        }
    }

    // Caps: the start reversed, the end as given.
    let cap = |s: usize, reverse: bool| -> PatchFace {
        let mut loops_idx: Vec<Vec<usize>> = idx[s].clone();
        if reverse {
            for lp in &mut loops_idx {
                lp.reverse();
            }
        }
        let outer = loops_idx.remove(0);
        PatchFace {
            outer,
            holes: loops_idx,
            attrs: attrs(None),
            plane: None,
        }
    };
    if spec.cap_start {
        faces.push(cap(0, true));
        claims_of.push(None);
    }
    if spec.cap_end && !apex {
        faces.push(cap(last, false));
        claims_of.push(None);
    }

    // Orientation: outward (positive volume).
    if six_volume(&pos, &faces) < 0.0 {
        for f in &mut faces {
            f.outer.reverse();
            for h in &mut f.holes {
                h.reverse();
            }
        }
    }
    for f in &faces {
        let ring: Vec<Point3> = f.outer.iter().map(|&i| pos[i]).collect();
        if newell(&ring).is_none() {
            return Err(bad);
        }
    }

    let patch = PolyPatch {
        verts,
        remove: vec![],
        add: faces,
        edge_meta: meta,
        expect_watertight: spec.cap_start && (spec.cap_end || apex),
    };
    let result = Object::empty().apply_patch(&patch).map_err(|_| bad)?;
    let mut obj = result.object;

    // Claims, map-or-drop.
    for (i, &fid) in result.faces.iter().enumerate() {
        let Some(claim) = claims_of[i] else { continue };
        let ring: Vec<Point3> = obj.loop_positions(obj.faces[fid].outer_loop).collect();
        let plane = obj.faces[fid].plane;
        if claim.holds_on(&ring, &plane, tol::PLANE_DIST)
            && surface::validate_face_claim(&claim, &plane, &ring, tol::PLANE_DIST)
        {
            obj.faces[fid].surface = Some(claim);
        }
    }
    obj.validate().map_err(|_| bad)?;
    if self_intersection(&obj).is_some() {
        return Err(ShapeError::SelfIntersecting { limit: None });
    }
    Ok(obj)
}

/// The area centroid of a planar polygon (fan triangulation about its first
/// vertex), or `None` for a zero-area ring.
pub(crate) fn area_centroid(ring: &[Point3], normal: Vec3) -> Option<Point3> {
    let o = ring.first()?.to_vec();
    let mut area = 0.0;
    let mut acc = Vec3::new(0.0, 0.0, 0.0);
    for i in 1..ring.len().saturating_sub(1) {
        let (b, c) = (ring[i].to_vec(), ring[i + 1].to_vec());
        let ar = (b - o).cross(c - o).dot(normal) * 0.5;
        area += ar;
        acc = acc + (o + b + c) * (ar / 3.0);
    }
    if area.abs() <= tol::NORMALIZE_MIN_LENGTH {
        return None;
    }
    let v = acc / area;
    Some(Point3::new(v.x, v.y, v.z))
}

/// Rotates `p` about the line through `c` along unit `axis` by `angle`
/// (right-hand rule), Rodrigues' formula.
pub(crate) fn rotate_about(p: Point3, c: Point3, axis: Vec3, angle: f64) -> Point3 {
    let v = p - c;
    let (s, co) = angle.sin_cos();
    let r = v * co + axis.cross(v) * s + axis * (axis.dot(v) * (1.0 - co));
    c + r
}

// ═══════════════════════════════════════════════════ test support

/// The spec shape [`build_skin_for_test`] takes (mirrors `SkinSpec`).
#[doc(hidden)]
#[derive(Debug, Clone)]
pub struct SkinSpecForTest {
    pub stations: Vec<Vec<Vec<Point3>>>,
    pub column_smooth: Vec<Vec<bool>>,
    pub station_smooth: Vec<bool>,
    pub cap_start: bool,
    pub cap_end: bool,
}

/// [`build_skin`] for the executable specs. Test support only.
#[doc(hidden)]
pub fn build_skin_for_test(spec: &SkinSpecForTest) -> Result<Object, ShapeError> {
    build_skin(&SkinSpec {
        stations: spec.stations.clone(),
        column_smooth: spec.column_smooth.clone(),
        station_smooth: spec.station_smooth.clone(),
        cap_start: spec.cap_start,
        cap_end: spec.cap_end,
        claims: None,
        material: None,
        column_fade: None,
    })
}

#[cfg(test)]
mod tests {
    use super::{ColumnFade, Fade, Side, SkinSpec, build_skin, creases, fan_choice, stands_out};
    use crate::math::Point3;
    use crate::surface::EdgeSmoothness;
    use crate::tol;

    /// Whether column 0's wall edge on the upper band of a prism two units
    /// tall over `ring` is hard, under `rule` there and `others` at every
    /// other column of that band (the lower band all creases).
    /// `turn` is column 0's own corner turn (the guard's), `floor` the
    /// span's facet floor as an angle.
    fn upper_column_zero_hard(
        ring: &dyn Fn(f64) -> Vec<Point3>,
        rule: Fade,
        others: Fade,
        turn: f64,
        floor: f64,
    ) -> bool {
        let k = ring(0.0).len();
        let mut upper = vec![others; k];
        upper[0] = rule;
        let mut turns = vec![0.0; k];
        turns[0] = turn;
        let obj = build_skin(&SkinSpec {
            stations: vec![vec![ring(0.0)], vec![ring(1.0)], vec![ring(2.0)]],
            column_smooth: vec![vec![false; k]],
            station_smooth: vec![true; 3],
            cap_start: true,
            cap_end: true,
            claims: None,
            material: None,
            column_fade: Some(ColumnFade {
                fold: tol::LOFT_CREASE_FOLD,
                rule: vec![vec![Fade::Crease; k], upper],
                floor: vec![0.0, 2.0 * (floor / 2.0).sin()],
                turn: vec![vec![0.0; k], turns],
            }),
        })
        .unwrap();
        let p0 = ring(1.5)[0];
        let edge = obj
            .edges()
            .keys()
            .find(|&x| {
                let (p, q) = obj.edge_endpoints(x).unwrap();
                let on = |a: Point3| (a.x - p0.x).abs() < 1e-12 && (a.y - p0.y).abs() < 1e-12;
                on(p) && on(q) && p.z.min(q.z) > 0.5
            })
            .expect("column 0's upper edge");
        obj.edge_smoothness(edge) == EdgeSmoothness::Hard
    }

    /// A square with the middle of one side pushed out so the boundary
    /// turns by `turn` there (column 0); its corners crease, so no column
    /// is a reference and the reference is 0.
    fn pushed_square(turn: f64, rule: Fade) -> bool {
        let e = (turn / 2.0).tan();
        let ring = move |z: f64| {
            vec![
                Point3::new(1.0 + e, 0.0, z),
                Point3::new(1.0, 1.0, z),
                Point3::new(-1.0, 1.0, z),
                Point3::new(-1.0, -1.0, z),
                Point3::new(1.0, -1.0, z),
            ]
        };
        upper_column_zero_hard(&ring, rule, Fade::Crease, turn, 0.0)
    }

    /// A regular 48-gon (every seam folds by 7.5°, all smooth) with vertex
    /// 0 pushed out radially so the boundary turns by `turn` there.
    fn pushed_gon(turn: f64, rule: Fade) -> bool {
        let k = 48;
        let step = std::f64::consts::TAU / k as f64;
        // The sides from vertex 0 to its neighbors at ±step meet at an
        // interior angle π − turn: tan(turn / 2) = (r0 − cos step) / sin step.
        let r0 = step.cos() + step.sin() * (turn / 2.0).tan();
        let ring = move |z: f64| -> Vec<Point3> {
            (0..k)
                .map(|v| {
                    let a = step * v as f64;
                    let r = if v == 0 { r0 } else { 1.0 };
                    Point3::new(r * a.cos(), r * a.sin(), z)
                })
                .collect()
        };
        // The 48-gon's seams are a curve's facets: a 7.5° floor.
        upper_column_zero_hard(&ring, rule, Fade::Soft, turn, step)
    }

    #[test]
    fn a_fading_column_creases_only_where_it_stands_out_from_its_neighbors() {
        let deg = |d: f64| d.to_radians();
        // No reference column (its neighbors are corners): a degree is the
        // line.
        assert!(
            !pushed_square(deg(0.99), Fade::Stand(Side::Start)),
            "just under a degree, reference 0: soft"
        );
        assert!(
            pushed_square(deg(1.01), Fade::Stand(Side::Start)),
            "just over: a crease"
        );
        // Beside seams that fold by about 7.5°, 6° does not stand out and
        // 30° does.
        assert!(
            !pushed_gon(deg(6.0), Fade::Stand(Side::Start)),
            "6° beside 7.5° seams: soft"
        );
        assert!(
            pushed_gon(deg(30.0), Fade::Stand(Side::Start)),
            "30° beside 7.5° seams: a crease"
        );
        assert!(
            pushed_square(deg(0.5), Fade::Crease),
            "a corner in both profiles creases"
        );
        assert!(
            !pushed_gon(deg(30.0), Fade::Soft),
            "a corner in neither is smooth"
        );
    }

    /// The fan rule's diagonal (D-126 faceting): `true` is q1–q3, `false`
    /// q0–q2, for a quad between columns j (left) and j + 1 (right).
    #[test]
    fn a_one_sided_column_fans_its_quads_from_its_corner() {
        let start = Some(Fade::Stand(Side::Start));
        let end = Some(Fade::Stand(Side::End));
        let soft = Some(Fade::Soft);
        let crease = Some(Fade::Crease);
        // One-sided left column: toward the start q0–q2, toward the end q3–q1.
        assert_eq!(fan_choice(start, soft), Some((false, vec![false])));
        assert_eq!(fan_choice(end, crease), Some((true, vec![false])));
        // One-sided right column: toward the start q1–q3, toward the end q2–q0.
        assert_eq!(fan_choice(soft, start), Some((true, vec![true])));
        assert_eq!(fan_choice(None, end), Some((false, vec![true])));
        // Toward opposite profiles both ask one diagonal: it serves both.
        assert_eq!(fan_choice(start, end), Some((false, vec![false, true])));
        assert_eq!(fan_choice(end, start), Some((true, vec![false, true])));
        // Toward the same profile, or neither: the shorter diagonal.
        assert_eq!(fan_choice(start, start), None);
        assert_eq!(fan_choice(end, end), None);
        assert_eq!(fan_choice(soft, crease), None);
        assert_eq!(fan_choice(None, None), None);
    }

    /// A fold creases where its angle is more than `max(ref + 1°, 2 · ref)`
    /// (the arguments are normal differences, `2 sin(θ/2)`, as the kernel
    /// measures folds; the comparison is of the angles).
    #[test]
    fn a_fold_stands_out_over_its_reference_plus_a_degree_and_twice_it() {
        let f = |deg: f64| 2.0 * (deg.to_radians() / 2.0).sin();
        let t = tol::LOFT_CREASE_FOLD;
        assert!(!stands_out(f(41.0), f(33.0), t), "41 beside 33: soft");
        assert!(
            stands_out(f(90.0), 0.0, t),
            "90 with no reference: a crease"
        );
        assert!(stands_out(f(10.0), f(4.0), t), "10 beside 4: a crease");
        assert!(!stands_out(f(7.0), f(4.0), t), "7 beside 4: soft");
        assert!(!stands_out(f(0.99), 0.0, t), "under a degree: soft");
        assert!(stands_out(f(1.01), 0.0, t), "over a degree: a crease");
        // Twice the reference is twice the angle: 67° beside 33.68° is
        // under 67.36°, 67.7° over it (a doubled chord would put the line
        // near 70.8°).
        assert!(!stands_out(f(67.0), f(33.68), t), "67 beside 33.68: soft");
        assert!(
            stands_out(f(67.7), f(33.68), t),
            "67.7 beside 33.68: a crease"
        );
        // Against a 48-gon circle's 7.5° facet floor (D-128): a 64-gon's
        // 5.6° corner and an 8.4° one do not stand out, 15.1° does.
        assert!(!stands_out(f(5.6), f(7.5), t), "5.6 against 7.5: soft");
        assert!(stands_out(f(15.1), f(7.5), t), "15.1 against 7.5: a crease");
        assert!(!stands_out(f(8.4), f(7.5), t), "8.4 against 7.5: soft");
    }

    /// The guard (D-129 A): a corner creases where it folds more than two
    /// thirds of its own turn and twice the floor, and more than a degree,
    /// even beside a steeper reference.
    #[test]
    fn a_corner_folding_most_of_its_turn_creases_beside_a_steep_skirt() {
        let f = |deg: f64| 2.0 * (deg.to_radians() / 2.0).sin();
        let t = tol::LOFT_CREASE_FOLD;
        let deg = |d: f64| d.to_radians();
        assert!(
            creases(f(61.0), f(40.0), deg(90.0), 0.0, t),
            "61 of a 90 turn: a crease"
        );
        assert!(
            !creases(f(59.0), f(40.0), deg(90.0), 0.0, t),
            "59 of a 90 turn: soft"
        );
        assert!(
            !creases(f(10.0), f(7.5), deg(12.0), f(7.5), t),
            "10 of a 12 turn against a 7.5 floor: soft"
        );
        assert!(
            !creases(f(0.9), 0.0, deg(1.0), 0.0, t),
            "0.9 of a 1 turn: under a degree"
        );
    }
}

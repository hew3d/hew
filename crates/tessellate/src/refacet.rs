//! Export re-faceting: true curves for STL (the true-curves design §4.2,
//! stage 6).
//!
//! The viewport renders the stored facets; export may do better. A stamped
//! surface of revolution ([`kernel::SurfaceRef`]: a cylinder wall, a cone
//! frustum, one row of a torus band) whose **entire boundary is still
//! analytic** — pristine chord-quad facets between two circles about the
//! axis, bordered by caps perpendicular to it or by other such bands on the
//! same circles, side seams intact — can be re-sampled from its analytic
//! definition at any chosen resolution. Bands sharing a circle refine
//! together or not at all. Sphere fans export at their stored facets. A wall that fails that legitimacy
//! condition (boolean seams across facets, bossed facets, slanted caps) is
//! exported at its stored resolution, honestly, never approximated.
//!
//! Watertightness is the load-bearing obligation: a re-faceted wall's cap
//! edges must match the re-faceted rims exactly at ANY resolution. The
//! mechanism is shared points, not tolerance welding — each new rim point is
//! computed once per (band, rim, angle) *station* and the identical f64
//! value is used by the wall quads and by every cap polygon that borders
//! them, so matching is bitwise by construction. Two bands on one circle
//! compute its points in one canonical form (one frame per axis line, swept
//! positively from the same anchor vertex), so they agree bitwise too. Original vertices that
//! outside geometry depends on (band ends, cap vertices with non-band
//! edges) become *anchors*: they are preserved bit-exact and the resampling
//! subdivides between them.
//!
//! Nothing here mutates the object or repairs geometry: a band either
//! qualifies and re-facets exactly, or every one of its faces falls back to
//! stored facets (rule 4 in spirit — the fallback is the honest carrier).

use std::collections::{BTreeMap, BTreeSet};

use kernel::{EdgeId, FaceId, Object, Point3, SurfaceRef, Vec3, VertexId, WatertightState};

use crate::TessellateError;

/// Hard bounds on the export resolution (segments per full turn). Below 8
/// the sampling stops resembling the claimed circle; above 512 file size
/// grows with no visible or printable benefit.
pub const MIN_SEGMENTS_PER_TURN: u32 = 8;
/// See [`MIN_SEGMENTS_PER_TURN`].
pub const MAX_SEGMENTS_PER_TURN: u32 = 512;

/// Export tessellation of one object as a flat triangle soup (9 `f64`s per
/// triangle: three CCW-from-outside vertices), at a chosen curve
/// resolution.
///
/// `segments_per_turn == 0` disables re-faceting entirely (stored facets,
/// still a valid export path); any other value is clamped to
/// [`MIN_SEGMENTS_PER_TURN`]`..=`[`MAX_SEGMENTS_PER_TURN`] and re-facets
/// every qualifying band at that angular resolution, subdividing
/// between preserved anchor vertices. Non-watertight objects are exported
/// at stored facets (re-faceting reasons over twins).
///
/// Deterministic: face slot order drives band discovery, station
/// generation, and emission order.
pub fn export_triangles(
    object: &Object,
    segments_per_turn: u32,
) -> Result<Vec<f64>, TessellateError> {
    export_triangles_with_faces(object, segments_per_turn).map(|(soup, _)| soup)
}

/// Same export as [`export_triangles`], but alongside the triangle soup
/// returns one [`FaceId`] per emitted triangle — the *originating* face for
/// a cap/polygon triangle, or a band's chosen *representative* face
/// (`Band::faces[0]`, the first face slot-order contributed to the band) for
/// every triangle a re-faceted band emits.
///
/// That representative choice is the honest one available: a re-faceted
/// band's stations are resampled on the analytic circle at the caller's
/// chosen resolution, which deliberately abandons the original per-facet
/// boundaries (the whole point of true-curve export) — there is no single
/// "originating face" for a station triangle in general. Callers that need
/// this for per-triangle material (`crates/mesh-export`) accept that a band
/// exports under one representative face's material/UV; a band whose facets
/// were painted uniformly (the common case) loses nothing, and mixed-paint
/// bands are a narrow, documented approximation rather than a fabricated
/// per-triangle value.
pub fn export_triangles_with_faces(
    object: &Object,
    segments_per_turn: u32,
) -> Result<(Vec<f64>, Vec<FaceId>), TessellateError> {
    let mut soup: Vec<f64> = Vec::new();
    let mut face_ids: Vec<FaceId> = Vec::new();

    let refacet_enabled =
        segments_per_turn != 0 && object.watertight() == WatertightState::Watertight;
    let step = if refacet_enabled {
        let n = segments_per_turn.clamp(MIN_SEGMENTS_PER_TURN, MAX_SEGMENTS_PER_TURN);
        2.0 * std::f64::consts::PI / f64::from(n)
    } else {
        0.0
    };

    // ---- Phase 1: discover qualifying bands -------------------------------
    let mut bands: Vec<Band> = if refacet_enabled {
        discover_bands(object)
    } else {
        Vec::new()
    };

    // ---- Phase 2 + 3: stations, cap rewrites, demotion fixpoint -----------
    // Demotion only ever grows, so the loop terminates. A demoted band's
    // faces are exported at stored facets and its cap chains revert to the
    // original vertices — which were valid before, so reverting cannot
    // introduce a new failure by itself; the re-check exists because OTHER
    // bands' refined chains still touch the same caps.
    let mut demoted: BTreeSet<usize> = BTreeSet::new();
    let cap_rewrites: BTreeMap<FaceId, Vec<Vec<Point3>>>;
    loop {
        for (i, band) in bands.iter_mut().enumerate() {
            if !demoted.contains(&i) && band.stations.is_empty() {
                match build_stations(object, band, step) {
                    Some(stations) => band.stations = stations,
                    None => {
                        demoted.insert(i);
                    }
                }
            }
        }
        // A band whose rim borders another band that stays at stored
        // facets cannot refine either: the shared circle must match.
        let refined: BTreeSet<FaceId> = bands
            .iter()
            .enumerate()
            .filter(|(i, _)| !demoted.contains(i))
            .flat_map(|(_, b)| b.faces.iter().copied())
            .collect();
        let stale: BTreeSet<usize> = bands
            .iter()
            .enumerate()
            .filter(|(i, b)| {
                !demoted.contains(i) && b.claimed_neighbors.iter().any(|f| !refined.contains(f))
            })
            .map(|(i, _)| i)
            .collect();
        if !stale.is_empty() {
            demoted.extend(stale);
            continue;
        }
        match rewrite_caps(object, &bands, &demoted) {
            Ok(rewrites) => {
                cap_rewrites = rewrites;
                break;
            }
            Err(newly_demoted) => {
                demoted.extend(newly_demoted);
            }
        }
    }

    // ---- Phase 4: emit ------------------------------------------------------
    // Refined band faces are replaced wholesale by the station grid; every
    // other face triangulates its (possibly rewritten) polygon.
    let mut band_face_set: BTreeSet<FaceId> = BTreeSet::new();
    for (i, band) in bands.iter().enumerate() {
        if demoted.contains(&i) {
            continue;
        }
        band_face_set.extend(band.faces.iter().copied());
        let before = soup.len();
        emit_band(band, &mut soup);
        let added = (soup.len() - before) / 9;
        let representative = *band.faces.first().expect("a band always has faces");
        face_ids.extend(std::iter::repeat_n(representative, added));
    }

    for (fid, face) in object.faces() {
        if band_face_set.contains(&fid) {
            continue;
        }
        let loops: Vec<Vec<Point3>> = match cap_rewrites.get(&fid) {
            Some(rewritten) => rewritten.clone(),
            None => std::iter::once(face.outer_loop)
                .chain(face.inner_loops.iter().copied())
                .map(|l| object.loop_positions(l).collect())
                .collect(),
        };
        let before = soup.len();
        emit_polygon(fid, face.plane.normal(), &loops, &mut soup)?;
        let added = (soup.len() - before) / 9;
        face_ids.extend(std::iter::repeat_n(fid, added));
    }

    debug_assert_eq!(soup.len() / 9, face_ids.len());
    Ok((soup, face_ids))
}

// ─────────────────────────────────────────────────────────────── bands

/// The point a station contributes at one rim level: an original vertex
/// (preserved bit-exact) or a point computed on the analytic circle.
#[derive(Debug, Clone, Copy)]
struct Station {
    lo: Point3,
    hi: Point3,
    /// Anchor rim vertices this station preserves, if any (lo, hi).
    lo_vertex: Option<VertexId>,
    hi_vertex: Option<VertexId>,
}

/// One connected chain of pristine chord-quad facets of a single surface
/// between two axial levels — the unit of re-faceting.
#[derive(Debug, Clone)]
struct Band {
    faces: Vec<FaceId>,
    axis_point: Point3,
    axis: Vec3,
    /// The larger ring radius, for angular tolerances.
    radius: f64,
    /// Orthonormal basis perpendicular to `axis` (u × v = axis).
    u: Vec3,
    v: Vec3,
    /// Rim vertices in chain order per level; length = faces + 1 for an
    /// open chain, faces for a closed one.
    rim_lo: Vec<VertexId>,
    rim_hi: Vec<VertexId>,
    /// Unwrapped angle of each rim vertex (indexes match `rim_lo`).
    rim_angle: Vec<f64>,
    closed: bool,
    /// The analytic surface, and the sign taking its normal to the stored
    /// facets' outward side — what orients the emitted quads.
    surface: SurfaceRef,
    sense: f64,
    /// Claimed faces outside the band bordering its rims: other bands on
    /// the same circles, which must refine too or this band demotes.
    claimed_neighbors: BTreeSet<FaceId>,
    /// Every edge belonging to a band facet (rim chords + verticals).
    band_edges: BTreeSet<EdgeId>,
    /// Rim chord edge -> (level, chain position i: the chord connects rim
    /// vertex i and i+1 — cyclic for closed bands).
    chords: BTreeMap<EdgeId, (RimLevel, usize)>,
    /// Filled by `build_stations`; empty until then.
    stations: Vec<Station>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum RimLevel {
    Lo,
    Hi,
}

/// The tolerance used to accept a vertex as lying on its claimed surface
/// and to bucket axial levels. Same constant the kernel validator uses for
/// the claim itself.
fn tol_pm() -> f64 {
    kernel::tol::POINT_MERGE
}

/// Group every attributed face by tolerance-equal surface, then build the
/// qualifying bands. Groups where ANY structural check fails contribute no
/// bands at all (all their faces export at stored facets) — a group is one
/// logical wall and must not be half-refined.
fn discover_bands(object: &Object) -> Vec<Band> {
    // First-appearance grouping in face slot order (deterministic).
    let mut groups: Vec<(SurfaceRef, Vec<FaceId>)> = Vec::new();
    for (fid, face) in object.faces() {
        let Some(sr) = face.surface else { continue };
        match groups.iter_mut().find(|(g, _)| g.same_surface(&sr)) {
            Some((_, list)) => list.push(fid),
            None => groups.push((sr, vec![fid])),
        }
    }

    // One frame per axis line, the first group's: coaxial surfaces whose
    // stored axes differ by rounding still share every computed point.
    let mut frames: Vec<(Point3, Vec3)> = Vec::new();
    let mut bands = Vec::new();
    for (sr, faces) in groups {
        let Some((p, a)) = revolution_axis(&sr) else {
            continue;
        };
        let frame = match frames.iter().find(|&&(fp, fa)| coaxial(&sr, fp, fa)) {
            Some(&f) => f,
            None => {
                let f = canonical_axis(p, a);
                frames.push(f);
                f
            }
        };
        if let Some(mut group_bands) = build_group_bands(object, sr, frame, &faces) {
            bands.append(&mut group_bands);
        }
    }
    bands
}

/// Structural qualification of one surface group; `None` demotes the whole
/// group to stored facets.
fn build_group_bands(
    object: &Object,
    sr: SurfaceRef,
    (axis_point, axis): (Point3, Vec3),
    faces: &[FaceId],
) -> Option<Vec<Band>> {
    // Surfaces of revolution re-facet here — cylinder walls, cone frusta
    // and the rows of torus bands — each band a strip between two circles
    // about the axis, resampled around it. Spheres export at stored facets.
    let is_cylinder = matches!(sr, SurfaceRef::Cylinder { .. });
    if matches!(sr, SurfaceRef::Sphere { .. }) {
        return None;
    }
    let (u, v) = kernel::triangulate::plane_basis(axis);
    let group_set: BTreeSet<FaceId> = faces.iter().copied().collect();

    // Per-face structure: a chord quad with two vertices on each of two
    // axial levels, all on the cylinder. After the stacking pass below, one
    // `Quad` may be a merged *column* of several vertically stacked faces
    // (a Follow Me sweep along collinear path segments): `faces` lists its
    // rows bottom to top, `lo`/`hi`/chords are the column extremes, and
    // `extra_edges` carries the swallowed interior rims and upper-row
    // verticals so anchor detection still counts them as band-owned.
    struct Quad {
        faces: Vec<FaceId>,
        t_lo: f64,
        t_hi: f64,
        r_lo: f64,
        r_hi: f64,
        lo: [VertexId; 2],
        hi: [VertexId; 2],
        /// The two vertical (lo–hi) edges of the BASE row — what chain
        /// adjacency between angularly neighboring columns is keyed on.
        verticals: [EdgeId; 2],
        chord_lo: EdgeId,
        chord_hi: EdgeId,
        /// Interior edges of a merged column (upper rows' verticals plus
        /// the swallowed interior rim chords). Band-owned, never rim chords.
        extra_edges: Vec<EdgeId>,
    }

    let mut quads: Vec<Quad> = Vec::new();
    for &fid in faces {
        let face = &object.faces()[fid];
        if !face.inner_loops.is_empty() {
            return None;
        }
        let hes: Vec<_> = object.loop_half_edges(face.outer_loop).collect();
        if hes.len() != 4 {
            return None;
        }
        let verts: Vec<VertexId> = hes.iter().map(|&h| object.half_edges()[h].origin).collect();
        let pos = |vid: VertexId| object.vertices()[vid].position;
        let t_of = |vid: VertexId| (pos(vid) - axis_point).dot(axis);
        let radial = |vid: VertexId| {
            let d = pos(vid) - axis_point;
            (d - axis * d.dot(axis)).length()
        };
        if verts.iter().any(|&w| sr.distance(pos(w)).abs() > tol_pm()) {
            return None;
        }
        // Two rings, two vertices each: a ring is a (t, ρ) circle.
        let keys: Vec<(f64, f64)> = verts.iter().map(|&w| (t_of(w), radial(w))).collect();
        let same = |a: (f64, f64), b: (f64, f64)| {
            (a.0 - b.0).abs() <= tol_pm() && (a.1 - b.1).abs() <= tol_pm()
        };
        let lo_key = keys
            .iter()
            .copied()
            .min_by(|a, b| {
                if (a.0 - b.0).abs() > tol_pm() {
                    a.0.total_cmp(&b.0)
                } else {
                    a.1.total_cmp(&b.1)
                }
            })
            .expect("four vertices");
        let is_lo: Vec<bool> = keys.iter().map(|&k| same(k, lo_key)).collect();
        let hi_key = keys[is_lo.iter().position(|&b| !b)?];
        if is_lo.iter().filter(|&&b| b).count() != 2
            || keys
                .iter()
                .zip(&is_lo)
                .any(|(&k, &lo)| !lo && !same(k, hi_key))
            || same(lo_key, hi_key)
        {
            return None; // not exactly two rings, 2 + 2
        }
        let (t_min, t_max) = (lo_key.0, hi_key.0);
        if is_cylinder && t_max - t_min <= tol_pm() {
            return None; // degenerate band height
        }
        // The two lo vertices must be cyclically adjacent (a chord quad,
        // not a bowtie).
        let lo_adjacent = (0..4).any(|i| is_lo[i] && is_lo[(i + 1) % 4]);
        if !lo_adjacent {
            return None;
        }
        // Classify the four edges: the half-edge from verts[i] to
        // verts[i+1] is a chord if both ends are on the same level, a
        // vertical otherwise.
        let mut verticals = Vec::new();
        let mut chord_lo = None;
        let mut chord_hi = None;
        for i in 0..4 {
            let e = object.half_edges()[hes[i]].edge;
            let a_lo = is_lo[i];
            let b_lo = is_lo[(i + 1) % 4];
            match (a_lo, b_lo) {
                (true, true) => chord_lo = Some(e),
                (false, false) => chord_hi = Some(e),
                _ => verticals.push(e),
            }
        }
        if verticals.len() != 2 {
            return None;
        }
        let (chord_lo, chord_hi) = (chord_lo?, chord_hi?);
        let lo: Vec<VertexId> = (0..4).filter(|&i| is_lo[i]).map(|i| verts[i]).collect();
        let hi: Vec<VertexId> = (0..4).filter(|&i| !is_lo[i]).map(|i| verts[i]).collect();
        quads.push(Quad {
            faces: vec![fid],
            t_lo: t_min,
            t_hi: t_max,
            r_lo: lo_key.1,
            r_hi: hi_key.1,
            lo: [lo[0], lo[1]],
            hi: [hi[0], hi[1]],
            verticals: [verticals[0], verticals[1]],
            chord_lo,
            chord_hi,
            extra_edges: Vec::new(),
        });
    }

    // ---- vertical stacking ------------------------------------------------
    // One cylinder built in several axial rows — a Follow Me sweep along
    // collinear path segments — is ONE logical wall: its rows meet
    // rim-to-rim on chords shared between two group quads. Merge clean 1:1
    // columns into taller quads so the wall re-facets as a single band.
    // The swallowed interior rims vanish from a refined export, so they
    // must be fully interior to the group: any outside edge referencing an
    // interior rim vertex (an open-arc band's flat neighbors, an imprint, a
    // boolean seam) demotes the whole group to stored facets instead.
    let mut up_link: Vec<Option<usize>> = vec![None; quads.len()];
    let mut down_link: Vec<Option<usize>> = vec![None; quads.len()];
    {
        let mut chord_owner: BTreeMap<EdgeId, Vec<(usize, RimLevel)>> = BTreeMap::new();
        for (qi, q) in quads.iter().enumerate() {
            chord_owner
                .entry(q.chord_lo)
                .or_default()
                .push((qi, RimLevel::Lo));
            chord_owner
                .entry(q.chord_hi)
                .or_default()
                .push((qi, RimLevel::Hi));
        }
        for owners in chord_owner.values() {
            if !is_cylinder {
                // Rows of a torus or cone band share their rings; each row
                // re-facets as its own band (the meridian is not straight).
                continue;
            }
            match owners.as_slice() {
                [_] => {} // rim against a non-group face: cap checks below
                [(a, la), (b, lb)] => {
                    // Two group quads share this rim: legal only as a clean
                    // stack — one's hi rim is the other's lo rim, same
                    // vertex pair, coincident levels, one link each way.
                    let (lower, upper) = match (la, lb) {
                        (RimLevel::Hi, RimLevel::Lo) => (*a, *b),
                        (RimLevel::Lo, RimLevel::Hi) => (*b, *a),
                        _ => return None,
                    };
                    let l = &quads[lower];
                    let u = &quads[upper];
                    if !(u.lo.contains(&l.hi[0]) && u.lo.contains(&l.hi[1]))
                        || (l.t_hi - u.t_lo).abs() > tol_pm()
                        || up_link[lower].is_some()
                        || down_link[upper].is_some()
                    {
                        return None;
                    }
                    up_link[lower] = Some(upper);
                    down_link[upper] = Some(lower);
                }
                _ => return None, // 3+ quads on one rim chord
            }
        }
    }
    if up_link.iter().any(Option::is_some) {
        let group_edges: BTreeSet<EdgeId> = quads
            .iter()
            .flat_map(|q| q.verticals.iter().copied().chain([q.chord_lo, q.chord_hi]))
            .collect();
        let interior: BTreeSet<VertexId> = up_link
            .iter()
            .enumerate()
            .filter(|(_, up)| up.is_some())
            .flat_map(|(qi, _)| quads[qi].hi)
            .collect();
        for (eid, edge) in object.edges() {
            if group_edges.contains(&eid) {
                continue;
            }
            let he = &object.half_edges()[edge.half_edge];
            let a = he.origin;
            let b = object.half_edges()[he.next].origin;
            if interior.contains(&a) || interior.contains(&b) {
                return None; // outside geometry references a swallowed rim
            }
        }

        let mut merged: Vec<Quad> = Vec::new();
        for base in 0..quads.len() {
            if down_link[base].is_some() {
                continue; // reached via its column's base
            }
            let mut column = vec![base];
            let mut top = base;
            while let Some(next) = up_link[top] {
                column.push(next);
                top = next;
            }
            let mut extra: Vec<EdgeId> = Vec::new();
            for (k, &qi) in column.iter().enumerate() {
                if k > 0 {
                    extra.extend(quads[qi].verticals);
                    extra.push(quads[qi].chord_lo);
                }
            }
            merged.push(Quad {
                faces: column
                    .iter()
                    .flat_map(|&qi| quads[qi].faces.iter().copied())
                    .collect(),
                t_lo: quads[base].t_lo,
                t_hi: quads[top].t_hi,
                r_lo: quads[base].r_lo,
                r_hi: quads[top].r_hi,
                lo: quads[base].lo,
                hi: quads[top].hi,
                verticals: quads[base].verticals,
                chord_lo: quads[base].chord_lo,
                chord_hi: quads[top].chord_hi,
                extra_edges: extra,
            });
        }
        quads = merged;
    }

    // Bucket by the ring pair within tolerance.
    let mut buckets: Vec<(f64, f64, f64, f64, Vec<usize>)> = Vec::new();
    for (qi, q) in quads.iter().enumerate() {
        match buckets.iter_mut().find(|(lo, hi, rl, rh, _)| {
            (q.t_lo - *lo).abs() <= tol_pm()
                && (q.t_hi - *hi).abs() <= tol_pm()
                && (q.r_lo - *rl).abs() <= tol_pm()
                && (q.r_hi - *rh).abs() <= tol_pm()
        }) {
            Some((.., list)) => list.push(qi),
            None => buckets.push((q.t_lo, q.t_hi, q.r_lo, q.r_hi, vec![qi])),
        }
    }

    // Rim chords must border a NON-group, surface-free face whose plane is
    // perpendicular to the axis — the analytic-cap legitimacy condition.
    // Checked over every (possibly merged) quad's EXTREME chords, since the
    // whole group stands or falls together; interior rims were resolved by
    // the stacking pass above.
    // A rim may also border another band on the same circle (a wall below
    // a torus band, a torus band's rows): coaxial claimed neighbours are
    // recorded, and demote this band if they do not refine with it.
    let mut claimed_by_quad: Vec<BTreeSet<FaceId>> = vec![BTreeSet::new(); quads.len()];
    for (qi, q) in quads.iter().enumerate() {
        for &chord in &[q.chord_lo, q.chord_hi] {
            let edge = &object.edges()[chord];
            let (h_a, h_b) = (edge.half_edge, edge.twin_half_edge?);
            for h in [h_a, h_b] {
                let f = object.loops()[object.half_edges()[h].loop_id].face;
                if q.faces.contains(&f) {
                    continue;
                }
                let neighbor = &object.faces()[f];
                if group_set.contains(&f) {
                    if is_cylinder {
                        return None; // non-stack group contact: unsupported
                    }
                    claimed_by_quad[qi].insert(f);
                    continue;
                }
                if let Some(other) = neighbor.surface {
                    if !coaxial(&other, axis_point, axis) {
                        return None; // rim against an unrelated wall
                    }
                    claimed_by_quad[qi].insert(f);
                    continue;
                }
                if neighbor.plane.normal().dot(axis).abs() < 1.0 - kernel::tol::NORMAL_DIRECTION {
                    return None; // slanted cap: resampled rim would leave it
                }
            }
        }
    }

    // Assemble chains per bucket via vertical-edge adjacency.
    let mut bands = Vec::new();
    for (_, _, r_lo, r_hi, members) in buckets {
        // vertical edge -> quads using it (within this bucket).
        let mut by_vertical: BTreeMap<EdgeId, Vec<usize>> = BTreeMap::new();
        for &qi in &members {
            for &e in &quads[qi].verticals {
                by_vertical.entry(e).or_default().push(qi);
            }
        }
        if by_vertical.values().any(|list| list.len() > 2) {
            return None; // non-manifold in band terms
        }
        let neighbor_of = |qi: usize, e: EdgeId| -> Option<usize> {
            by_vertical[&e].iter().copied().find(|&other| other != qi)
        };

        let mut visited: BTreeSet<usize> = BTreeSet::new();
        for &start_candidate in &members {
            if visited.contains(&start_candidate) {
                continue;
            }
            // Walk to one end of this connected chain (or detect a cycle).
            let (start, closed) = {
                let mut current = start_candidate;
                let mut entry_edge: Option<EdgeId> = None;
                loop {
                    let q = &quads[current];
                    let back = q
                        .verticals
                        .iter()
                        .copied()
                        .find(|&e| Some(e) != entry_edge && neighbor_of(current, e).is_some());
                    match back {
                        Some(e) => {
                            let next = neighbor_of(current, e).expect("checked");
                            if next == start_candidate {
                                break (start_candidate, true);
                            }
                            entry_edge = Some(e);
                            current = next;
                        }
                        None => break (current, false),
                    }
                }
            };

            // Walk the chain from `start`, collecting quads and rims.
            let mut chain: Vec<usize> = vec![start];
            visited.insert(start);
            let mut prev_edge: Option<EdgeId> = None;
            loop {
                let current = *chain.last().expect("nonempty");
                let q = &quads[current];
                let forward = q
                    .verticals
                    .iter()
                    .copied()
                    .find(|&e| Some(e) != prev_edge && neighbor_of(current, e).is_some());
                let Some(e) = forward else { break };
                let next = neighbor_of(current, e).expect("checked");
                if visited.contains(&next) {
                    break; // closed the cycle
                }
                visited.insert(next);
                chain.push(next);
                prev_edge = Some(e);
            }

            // Rim vertex order along the chain. For each consecutive quad
            // pair, the shared vertical edge identifies the shared (lo, hi)
            // vertices; the chain's rims start at the non-shared vertices
            // of the first quad.
            let shared_lo = |a: usize, b: usize| -> Option<VertexId> {
                quads[a]
                    .lo
                    .iter()
                    .copied()
                    .find(|w| quads[b].lo.contains(w))
            };
            let shared_hi = |a: usize, b: usize| -> Option<VertexId> {
                quads[a]
                    .hi
                    .iter()
                    .copied()
                    .find(|w| quads[b].hi.contains(w))
            };
            let mut rim_lo: Vec<VertexId> = Vec::with_capacity(chain.len() + 1);
            let mut rim_hi: Vec<VertexId> = Vec::with_capacity(chain.len() + 1);
            if chain.len() == 1 {
                let q = &quads[chain[0]];
                // Orient the single quad's rims consistently: lo[0]–lo[1]
                // and the hi pair matched by the vertical edges. The two
                // verticals connect lo/hi endpoints; pick hi order so that
                // rim_hi[k] is vertically connected to rim_lo[k].
                let connected = |e: EdgeId, lo: VertexId, hi: VertexId| -> bool {
                    let edge = &object.edges()[e];
                    let he = &object.half_edges()[edge.half_edge];
                    let a = he.origin;
                    let b = object.half_edges()[he.next].origin;
                    (a == lo && b == hi) || (a == hi && b == lo)
                };
                rim_lo.extend(q.lo);
                let hi_first =
                    q.hi.iter()
                        .copied()
                        .find(|&w| q.verticals.iter().any(|&e| connected(e, q.lo[0], w)))
                        .unwrap_or(q.hi[0]);
                let hi_second = q.hi.iter().copied().find(|&w| w != hi_first).expect("two");
                rim_hi.extend([hi_first, hi_second]);
            } else {
                for k in 0..chain.len() {
                    let q = &quads[chain[k]];
                    if k == 0 {
                        let s_lo = shared_lo(chain[0], chain[1]).or(None);
                        let s_hi = shared_hi(chain[0], chain[1]).or(None);
                        let (Some(s_lo), Some(s_hi)) = (s_lo, s_hi) else {
                            return None;
                        };
                        let first_lo = q.lo.iter().copied().find(|&w| w != s_lo).expect("two");
                        let first_hi = q.hi.iter().copied().find(|&w| w != s_hi).expect("two");
                        rim_lo.extend([first_lo, s_lo]);
                        rim_hi.extend([first_hi, s_hi]);
                    } else {
                        let prev_lo = *rim_lo.last().expect("nonempty");
                        let prev_hi = *rim_hi.last().expect("nonempty");
                        if !q.lo.contains(&prev_lo) || !q.hi.contains(&prev_hi) {
                            return None; // chain does not share the rim pair
                        }
                        let next_lo = q.lo.iter().copied().find(|&w| w != prev_lo).expect("two");
                        let next_hi = q.hi.iter().copied().find(|&w| w != prev_hi).expect("two");
                        if closed && k == chain.len() - 1 {
                            if next_lo != rim_lo[0] || next_hi != rim_hi[0] {
                                return None;
                            }
                        } else {
                            rim_lo.push(next_lo);
                            rim_hi.push(next_hi);
                        }
                    }
                }
            }

            // Unwrapped rim angles, monotone along the chain, each facet
            // sweeping less than a half turn, total at most one full turn.
            let angle_of = |w: VertexId| -> f64 {
                let d = object.vertices()[w].position - axis_point;
                let x = d.dot(u);
                let y = d.dot(v);
                y.atan2(x)
            };
            let mut rim_angle: Vec<f64> = Vec::with_capacity(rim_lo.len());
            for (k, &w) in rim_lo.iter().enumerate() {
                let raw = angle_of(w);
                if k == 0 {
                    rim_angle.push(raw);
                } else {
                    let prev = rim_angle[k - 1];
                    let mut d = raw - prev;
                    while d > std::f64::consts::PI {
                        d -= 2.0 * std::f64::consts::PI;
                    }
                    while d < -std::f64::consts::PI {
                        d += 2.0 * std::f64::consts::PI;
                    }
                    rim_angle.push(prev + d);
                }
            }
            let sweeps: Vec<f64> = rim_angle.windows(2).map(|w| w[1] - w[0]).collect();
            if sweeps.is_empty() && !closed {
                return None;
            }
            let dir = if closed && chain.len() == 1 {
                1.0 // single closed facet cannot happen (checked below)
            } else if sweeps.is_empty() {
                return None;
            } else {
                sweeps[0].signum()
            };
            if sweeps
                .iter()
                .any(|&s| s.signum() != dir || s.abs() >= std::f64::consts::PI || s.abs() == 0.0)
            {
                return None;
            }
            let total: f64 = sweeps.iter().map(|s| s.abs()).sum();
            let full_turn = 2.0 * std::f64::consts::PI;
            if closed {
                // The wrap-around facet closes the remaining sweep.
                if chain.len() < 3 || total >= full_turn {
                    return None;
                }
            } else if total > full_turn + 1e-9 {
                return None;
            }

            // The base row's first facet sets the band's outward side.
            let q0 = &quads[chain[0]];
            let face0 = &object.faces()[q0.faces[0]];
            let mid = {
                let a = object.vertices()[q0.lo[0]].position;
                let b = object.vertices()[q0.lo[1]].position;
                Point3::new((a.x + b.x) / 2.0, (a.y + b.y) / 2.0, (a.z + b.z) / 2.0)
            };

            let mut band_edges: BTreeSet<EdgeId> = BTreeSet::new();
            let mut chords: BTreeMap<EdgeId, (RimLevel, usize)> = BTreeMap::new();
            for (k, &qi) in chain.iter().enumerate() {
                let q = &quads[qi];
                band_edges.extend(q.verticals);
                band_edges.extend(q.extra_edges.iter().copied());
                band_edges.insert(q.chord_lo);
                band_edges.insert(q.chord_hi);
                chords.insert(q.chord_lo, (RimLevel::Lo, k));
                chords.insert(q.chord_hi, (RimLevel::Hi, k));
            }

            let sense = {
                let n = face0.plane.normal();
                match sr.normal_at(mid) {
                    Some(m) if m.dot(n) < 0.0 => -1.0,
                    Some(_) => 1.0,
                    None => return None,
                }
            };
            let claimed_neighbors: BTreeSet<FaceId> = chain
                .iter()
                .flat_map(|&qi| claimed_by_quad[qi].iter().copied())
                .collect();
            bands.push(Band {
                faces: chain
                    .iter()
                    .flat_map(|&qi| quads[qi].faces.iter().copied())
                    .collect(),
                axis_point,
                axis,
                radius: r_lo.max(r_hi),
                u,
                v,
                rim_lo,
                rim_hi,
                rim_angle,
                closed,
                surface: sr,
                sense,
                claimed_neighbors,
                band_edges,
                chords,
                stations: Vec::new(),
            });
        }
    }
    Some(bands)
}

// ───────────────────────────────────────────────────────────── stations

/// Build the band's station list: anchors (rim vertices with any incidence
/// outside the band, preserved bit-exact) plus computed subdivisions on the
/// analytic circle. `None` demotes the band (ambiguous anchor pairing).
fn build_stations(object: &Object, band: &Band, step: f64) -> Option<Vec<Station>> {
    // Angular coincidence tolerance, derived from the kernel's point-merge
    // tolerance at this band's radius (arc length ≈ radius · angle).
    let angular_eps = tol_pm() / band.radius.max(tol_pm());
    // Vertex -> incident edges, restricted to what we need: any edge NOT in
    // the band's own edge set makes a rim vertex an anchor.
    let mut has_outside_edge: BTreeSet<VertexId> = BTreeSet::new();
    for (eid, edge) in object.edges() {
        if band.band_edges.contains(&eid) {
            continue;
        }
        let he = &object.half_edges()[edge.half_edge];
        has_outside_edge.insert(he.origin);
        has_outside_edge.insert(object.half_edges()[he.next].origin);
    }

    let n = band.rim_lo.len();
    let pos = |w: VertexId| object.vertices()[w].position;
    let angle_of = |p: Point3| -> f64 {
        let d = p - band.axis_point;
        d.dot(band.v).atan2(d.dot(band.u))
    };

    // Anchor positions in chain order: chain ends, and every rim index
    // where either ring's vertex has an edge outside the band.
    struct Anchor {
        k: usize,
        lo_vertex: Option<VertexId>,
        hi_vertex: Option<VertexId>,
    }
    let mut anchors: Vec<Anchor> = Vec::new();
    for k in 0..n {
        let lo_anchor = has_outside_edge.contains(&band.rim_lo[k]);
        let hi_anchor = has_outside_edge.contains(&band.rim_hi[k]);
        if !band.closed && (k == 0 || k == n - 1) {
            // Chain ends are always stations; their vertical edge borders a
            // neighbor face, so both rim vertices are necessarily anchors.
            anchors.push(Anchor {
                k,
                lo_vertex: Some(band.rim_lo[k]),
                hi_vertex: Some(band.rim_hi[k]),
            });
        } else if lo_anchor || hi_anchor {
            anchors.push(Anchor {
                k,
                lo_vertex: lo_anchor.then_some(band.rim_lo[k]),
                hi_vertex: hi_anchor.then_some(band.rim_hi[k]),
            });
        }
    }
    if band.closed && anchors.is_empty() {
        // Free choice of phase: the first rim position.
        anchors.push(Anchor {
            k: 0,
            lo_vertex: None,
            hi_vertex: None,
        });
    }
    if !band.closed && anchors.len() < 2 {
        return None;
    }

    // The subdivision points of one ring between two of its vertices, in
    // a canonical form shared by every band on that circle: swept in the
    // frame's positive sense from the start vertex, its (t, ρ) taken from
    // that vertex, and reversed afterwards for a band running the other
    // way. Same anchor vertices therefore give bitwise the same points.
    let dir = (band.rim_angle[1 % n] - band.rim_angle[0]).signum();
    let full_turn = 2.0 * std::f64::consts::PI;
    let ring_points = |from: VertexId, to: VertexId| -> (usize, Vec<Point3>) {
        let (s, e) = if dir > 0.0 { (from, to) } else { (to, from) };
        let (ps, pe) = (pos(s), pos(e));
        let a0 = angle_of(ps);
        let mut sweep = angle_of(pe) - a0;
        while sweep <= angular_eps {
            sweep += full_turn;
        }
        while sweep > full_turn + angular_eps {
            sweep -= full_turn;
        }
        let d = ps - band.axis_point;
        let t = d.dot(band.axis);
        let rho = (d - band.axis * t).length();
        let foot = band.axis_point + band.axis * t;
        let count = ((sweep / step) - 1e-9).ceil().max(1.0) as usize;
        let mut pts: Vec<Point3> = (1..count)
            .map(|j| {
                let angle = a0 + sweep * (j as f64) / (count as f64);
                foot + band.u * (rho * angle.cos()) + band.v * (rho * angle.sin())
            })
            .collect();
        if dir < 0.0 {
            pts.reverse();
        }
        (count, pts)
    };

    let mut stations: Vec<Station> = Vec::new();
    let segments = if band.closed {
        anchors.len()
    } else {
        anchors.len() - 1
    };
    for i in 0..anchors.len() {
        let a = &anchors[i];
        let (lo, hi) = (pos(band.rim_lo[a.k]), pos(band.rim_hi[a.k]));
        stations.push(Station {
            lo,
            hi,
            lo_vertex: a.lo_vertex,
            hi_vertex: a.hi_vertex,
        });
        if i >= segments {
            break;
        }
        let b = &anchors[(i + 1) % anchors.len()];
        if b.k != a.k {
            let sweep = (band.rim_angle[b.k] - band.rim_angle[a.k]).abs();
            let wrapped = if band.closed && b.k < a.k {
                full_turn - sweep
            } else {
                sweep
            };
            if wrapped <= angular_eps {
                return None; // two unpaired anchors at one angle
            }
        }
        let (n_lo, lo_pts) = ring_points(band.rim_lo[a.k], band.rim_lo[b.k]);
        let (n_hi, hi_pts) = ring_points(band.rim_hi[a.k], band.rim_hi[b.k]);
        if n_lo != n_hi {
            return None; // the two rings would not subdivide alike
        }
        for (lo, hi) in lo_pts.into_iter().zip(hi_pts) {
            stations.push(Station {
                lo,
                hi,
                lo_vertex: None,
                hi_vertex: None,
            });
        }
    }
    if stations.len() < 2 || (band.closed && stations.len() < 3) {
        return None;
    }
    Some(stations)
}

// ───────────────────────────────────────────────────────────── emission

/// Emit the re-faceted wall quads for one band.
fn emit_band(band: &Band, soup: &mut Vec<f64>) {
    let s = &band.stations;
    let count = if band.closed { s.len() } else { s.len() - 1 };
    // Winding: the first quad against the analytic normal at its middle,
    // turned to the stored facets' outward side.
    let flip = {
        let j = 1 % s.len();
        let (a, b, c) = (s[0].lo, s[j].lo, s[j].hi);
        let mid = Point3::new(
            (s[0].lo.x + s[j].hi.x) / 2.0,
            (s[0].lo.y + s[j].hi.y) / 2.0,
            (s[0].lo.z + s[j].hi.z) / 2.0,
        );
        let n = band.surface.normal_at(mid).unwrap_or(band.axis) * band.sense;
        (b - a).cross(c - a).dot(n) < 0.0
    };
    for i in 0..count {
        let j = (i + 1) % s.len();
        let quad = [s[i].lo, s[j].lo, s[j].hi, s[i].hi];
        push_triangle(soup, quad[0], quad[1], quad[2], flip);
        push_triangle(soup, quad[0], quad[2], quad[3], flip);
    }
}

fn push_triangle(soup: &mut Vec<f64>, a: Point3, b: Point3, c: Point3, flip: bool) {
    let (b, c) = if flip { (c, b) } else { (b, c) };
    soup.extend([a.x, a.y, a.z, b.x, b.y, b.z, c.x, c.y, c.z]);
}

/// Triangulate one polygon-with-holes face into the soup, reusing the
/// renderer's bridging ear clipper.
fn emit_polygon(
    fid: FaceId,
    normal: Vec3,
    loops: &[Vec<Point3>],
    soup: &mut Vec<f64>,
) -> Result<(), TessellateError> {
    let (u, v) = kernel::triangulate::plane_basis(normal);
    let to_arr = |l: &Vec<Point3>| -> Vec<[f64; 3]> { l.iter().map(|p| [p.x, p.y, p.z]).collect() };
    let outer = to_arr(&loops[0]);
    let holes: Vec<Vec<[f64; 3]>> = loops[1..].iter().map(to_arr).collect();
    if outer.len() < 3 {
        return Err(TessellateError::DegenerateFace { face: fid });
    }
    let poly = kernel::triangulate::bridge_holes(&outer, &holes, u, v).map_err(
        |kernel::triangulate::UnbridgeableHole| TessellateError::UnbridgeableHole { face: fid },
    )?;
    for [i, j, k] in kernel::triangulate::ear_clip(&poly) {
        soup.extend([
            poly[i][0], poly[i][1], poly[i][2], poly[j][0], poly[j][1], poly[j][2], poly[k][0],
            poly[k][1], poly[k][2],
        ]);
    }
    Ok(())
}

// ─────────────────────────────────────────────────────────── cap rewrite

/// Rewrite every cap loop that borders a refined band, substituting the
/// band's station points for the original rim chords. Returns the rewritten
/// loops per face on success, or the set of band indices to demote when a
/// rewritten face stops being a valid simple polygon-with-holes.
#[allow(clippy::type_complexity)]
fn rewrite_caps(
    object: &Object,
    bands: &[Band],
    demoted: &BTreeSet<usize>,
) -> Result<BTreeMap<FaceId, Vec<Vec<Point3>>>, BTreeSet<usize>> {
    // Chord edge -> (band index, level, chain position).
    let mut chord_lookup: BTreeMap<EdgeId, (usize, RimLevel, usize)> = BTreeMap::new();
    for (bi, band) in bands.iter().enumerate() {
        if demoted.contains(&bi) {
            continue;
        }
        for (&e, &(level, k)) in &band.chords {
            chord_lookup.insert(e, (bi, level, k));
        }
    }
    if chord_lookup.is_empty() {
        return Ok(BTreeMap::new());
    }

    // Rim vertex -> station index per band/level (anchors only; run
    // boundaries are always anchors).
    let mut station_of: BTreeMap<(usize, RimLevel, VertexId), usize> = BTreeMap::new();
    for (bi, band) in bands.iter().enumerate() {
        if demoted.contains(&bi) {
            continue;
        }
        for (si, st) in band.stations.iter().enumerate() {
            if let Some(w) = st.lo_vertex {
                station_of.insert((bi, RimLevel::Lo, w), si);
            }
            if let Some(w) = st.hi_vertex {
                station_of.insert((bi, RimLevel::Hi, w), si);
            }
        }
    }

    let band_face_set: BTreeSet<FaceId> = bands
        .iter()
        .enumerate()
        .filter(|(bi, _)| !demoted.contains(bi))
        .flat_map(|(_, b)| b.faces.iter().copied())
        .collect();

    let mut rewrites: BTreeMap<FaceId, Vec<Vec<Point3>>> = BTreeMap::new();
    let mut failures: BTreeSet<usize> = BTreeSet::new();

    for (fid, face) in object.faces() {
        if band_face_set.contains(&fid) {
            continue;
        }
        let loop_ids: Vec<_> = std::iter::once(face.outer_loop)
            .chain(face.inner_loops.iter().copied())
            .collect();
        let mut new_loops: Vec<Vec<Point3>> = Vec::with_capacity(loop_ids.len());
        let mut touched: BTreeSet<usize> = BTreeSet::new();

        for &lid in &loop_ids {
            let hes: Vec<_> = object.loop_half_edges(lid).collect();
            // Mark each half-edge with the refined chord it traverses.
            let marks: Vec<Option<(usize, RimLevel, usize)>> = hes
                .iter()
                .map(|&h| chord_lookup.get(&object.half_edges()[h].edge).copied())
                .collect();
            if marks.iter().all(|m| m.is_none()) {
                new_loops.push(object.loop_positions(lid).collect());
                continue;
            }
            for m in marks.iter().flatten() {
                touched.insert(m.0);
            }

            let m = hes.len();
            let origin = |i: usize| object.half_edges()[hes[i]].origin;
            let pos = |w: VertexId| object.vertices()[w].position;

            if marks.iter().all(|mk| {
                mk.is_some() && mk.map(|(b, l, _)| (b, l)) == marks[0].map(|(b, l, _)| (b, l))
            }) {
                // The whole loop is one band rim (a full-circle cap edge):
                // replace it with all stations, in the loop's own direction.
                let (bi, level, _) = marks[0].expect("all marked");
                let band = &bands[bi];
                // Determine direction: consecutive origins are consecutive
                // rim vertices; find their chain positions via the chord's
                // stored position.
                let k0 = marks[0].expect("marked").2;
                let k1 = marks[1 % m].expect("marked").2;
                // Chain positions cycle over the band's COLUMNS — one chord
                // per level per column — not its faces (a merged column
                // carries several stacked faces).
                let len = band.chords.len() / 2;
                let forward = (k0 + 1) % len == k1 || (m == 1);
                let pts: Vec<Point3> = band
                    .stations
                    .iter()
                    .map(|st| match level {
                        RimLevel::Lo => st.lo,
                        RimLevel::Hi => st.hi,
                    })
                    .collect();
                let new_loop: Vec<Point3> = if forward {
                    pts
                } else {
                    pts.into_iter().rev().collect()
                };
                new_loops.push(new_loop);
                continue;
            }

            // Mixed loop: substitute runs between anchors.
            let mut out: Vec<Point3> = Vec::new();
            let mut i = 0;
            while i < m {
                match marks[i] {
                    None => {
                        out.push(pos(origin(i)));
                        i += 1;
                    }
                    Some((bi, level, _)) => {
                        // Extend the run over consecutive same-(band,level)
                        // marks.
                        let mut j = i;
                        while j < m && marks[j].map(|(b, l, _)| (b, l)) == Some((bi, level)) {
                            j += 1;
                        }
                        let band = &bands[bi];
                        let a = origin(i);
                        let b_vertex = origin(j % m);
                        let sa = station_of.get(&(bi, level, a)).copied();
                        let sb = station_of.get(&(bi, level, b_vertex)).copied();
                        let (Some(sa), Some(sb)) = (sa, sb) else {
                            // Run boundary is not an anchor — should be
                            // impossible; demote defensively.
                            failures.insert(bi);
                            out.push(pos(a));
                            i = j;
                            continue;
                        };
                        out.push(pos(a));
                        // Direction: the first chord's chain position tells
                        // which rim neighbor follows the run start.
                        let (_, _, k_first) = marks[i].expect("marked");
                        let rim = match level {
                            RimLevel::Lo => &band.rim_lo,
                            RimLevel::Hi => &band.rim_hi,
                        };
                        let len = rim.len();
                        // chord k connects rim[k] and rim[k+1] (cyclic when
                        // closed): the run leaves `a` toward the other end.
                        let forward = rim[k_first] == a;
                        debug_assert!(forward || rim[(k_first + 1) % len] == a);
                        let scount = band.stations.len();
                        let step_dir: isize = if forward { 1 } else { -1 };
                        let mut si = sa as isize;
                        loop {
                            si += step_dir;
                            let idx = if band.closed {
                                si.rem_euclid(scount as isize) as usize
                            } else if si < 0 || si >= scount as isize {
                                // Fell off an open band without reaching
                                // sb: inconsistent; demote.
                                failures.insert(bi);
                                break;
                            } else {
                                si as usize
                            };
                            if idx == sb {
                                break;
                            }
                            let st = &band.stations[idx];
                            out.push(match level {
                                RimLevel::Lo => st.lo,
                                RimLevel::Hi => st.hi,
                            });
                        }
                        i = j;
                    }
                }
            }
            new_loops.push(out);
        }

        if touched.is_empty() {
            continue;
        }
        // A rewritten face must still be a simple polygon with disjoint
        // holes strictly inside; otherwise the refinement collided with cap
        // geometry the original faceting cleared — demote every band that
        // touched this face.
        if !loops_valid(face.plane.normal(), &new_loops) {
            failures.extend(touched);
            continue;
        }
        rewrites.insert(fid, new_loops);
    }

    if failures.is_empty() {
        Ok(rewrites)
    } else {
        Err(failures)
    }
}

// ──────────────────────────────────────────────────── 2-D validity checks

/// Simple + disjoint check for a rewritten cap: every loop simple, holes
/// inside the outer loop, holes pairwise non-overlapping. Loops of one face
/// may touch at isolated points (a circle tangent to the face's edge, two
/// circles touching) but never along an edge, so a hole vertex lying on
/// another ring is no evidence either way: the even-odd test answers such a
/// point by which edge it sits on, which would demote a tangent rim on some
/// edges and not others. An edge running along another ring is invalid.
fn loops_valid(normal: Vec3, loops: &[Vec<Point3>]) -> bool {
    let (u, v) = kernel::triangulate::plane_basis(normal);
    let project = |l: &Vec<Point3>| -> Vec<[f64; 2]> {
        l.iter()
            .map(|p| [p.to_vec().dot(u), p.to_vec().dot(v)])
            .collect()
    };
    let loops2d: Vec<Vec<[f64; 2]>> = loops.iter().map(project).collect();

    for l in &loops2d {
        if l.len() < 3 || !polygon_simple_2d(l) {
            return false;
        }
    }
    let outer = &loops2d[0];
    for hole in &loops2d[1..] {
        if runs_along(hole, outer) {
            return false;
        }
        let mut probes = ring_probes_off(hole, outer).peekable();
        if probes.peek().is_none() || probes.any(|p| !point_in_polygon_2d(p, outer)) {
            return false;
        }
        if rings_cross(hole, outer) {
            return false;
        }
    }
    for i in 1..loops2d.len() {
        for j in (i + 1)..loops2d.len() {
            let (a, b) = (&loops2d[i], &loops2d[j]);
            if rings_cross(a, b)
                || runs_along(a, b)
                || runs_along(b, a)
                || ring_probes_off(b, a).any(|p| point_in_polygon_2d(p, a))
                || ring_probes_off(a, b).any(|p| point_in_polygon_2d(p, b))
            {
                return false;
            }
        }
    }
    true
}

/// The vertices and edge midpoints of `ring` that are not on `other`
/// (within `POINT_MERGE`): the points whose inside/outside answer against
/// `other` means something.
fn ring_probes_off<'a>(
    ring: &'a [[f64; 2]],
    other: &'a [[f64; 2]],
) -> impl Iterator<Item = [f64; 2]> + 'a {
    let n = ring.len();
    (0..n)
        .flat_map(move |i| {
            let (a, b) = (ring[i], ring[(i + 1) % n]);
            [a, [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0]]
        })
        .filter(move |&p| !on_ring_2d(p, other))
}

/// Whether some edge of `ring` runs along `other`: its midpoint lies on
/// `other`.
fn runs_along(ring: &[[f64; 2]], other: &[[f64; 2]]) -> bool {
    let n = ring.len();
    (0..n).any(|i| {
        let (a, b) = (ring[i], ring[(i + 1) % n]);
        on_ring_2d([(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0], other)
    })
}

/// Whether `p` lies within `POINT_MERGE` of some edge of `ring`.
fn on_ring_2d(p: [f64; 2], ring: &[[f64; 2]]) -> bool {
    let n = ring.len();
    (0..n).any(|i| {
        let (a, b) = (ring[i], ring[(i + 1) % n]);
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let len2 = dx * dx + dy * dy;
        let t = if len2 > 0.0 {
            (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / len2).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let (ex, ey) = (a[0] + t * dx - p[0], a[1] + t * dy - p[1]);
        (ex * ex + ey * ey).sqrt() <= kernel::tol::POINT_MERGE
    })
}

fn seg_cross(p: [f64; 2], q: [f64; 2], r: [f64; 2], s: [f64; 2]) -> bool {
    let orient = |a: [f64; 2], b: [f64; 2], c: [f64; 2]| -> f64 {
        (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
    };
    let d1 = orient(p, q, r);
    let d2 = orient(p, q, s);
    let d3 = orient(r, s, p);
    let d4 = orient(r, s, q);
    ((d1 > 0.0 && d2 < 0.0) || (d1 < 0.0 && d2 > 0.0))
        && ((d3 > 0.0 && d4 < 0.0) || (d3 < 0.0 && d4 > 0.0))
}

/// Proper self-intersection test over one ring (adjacent segments share an
/// endpoint and are exempt).
fn polygon_simple_2d(ring: &[[f64; 2]]) -> bool {
    let n = ring.len();
    for i in 0..n {
        let (a1, a2) = (ring[i], ring[(i + 1) % n]);
        for j in (i + 1)..n {
            if j == i || (j + 1) % n == i || (i + 1) % n == j {
                continue;
            }
            let (b1, b2) = (ring[j], ring[(j + 1) % n]);
            if seg_cross(a1, a2, b1, b2) {
                return false;
            }
        }
    }
    true
}

/// Proper crossing between two rings.
fn rings_cross(a: &[[f64; 2]], b: &[[f64; 2]]) -> bool {
    let n = a.len();
    let m = b.len();
    for i in 0..n {
        for j in 0..m {
            if seg_cross(a[i], a[(i + 1) % n], b[j], b[(j + 1) % m]) {
                return true;
            }
        }
    }
    false
}

/// Even-odd point-in-polygon. A point on the ring gets either answer,
/// depending on which edge it sits on; callers exclude such points.
fn point_in_polygon_2d(p: [f64; 2], ring: &[[f64; 2]]) -> bool {
    let n = ring.len();
    let mut inside = false;
    let mut j = n - 1;
    for i in 0..n {
        let (a, b) = (ring[i], ring[j]);
        if ((a[1] > p[1]) != (b[1] > p[1]))
            && (p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0])
        {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// The axis line in a canonical form: the direction with its first nonzero
/// component positive, the point the line's foot from the origin.
fn canonical_axis(point: Point3, axis: Vec3) -> (Point3, Vec3) {
    let flip = if axis.x.abs() > 1e-12 {
        axis.x < 0.0
    } else if axis.y.abs() > 1e-12 {
        axis.y < 0.0
    } else {
        axis.z < 0.0
    };
    let axis = if flip { -axis } else { axis };
    let p = point.to_vec();
    let foot = p - axis * p.dot(axis);
    (Point3::new(foot.x, foot.y, foot.z), axis)
}

/// Whether surface `s` revolves about the same axis line.
fn coaxial(s: &SurfaceRef, axis_point: Point3, axis: Vec3) -> bool {
    let Some((p, a)) = revolution_axis(s) else {
        return false;
    };
    let d = p - axis_point;
    a.dot(axis).abs() >= 1.0 - kernel::tol::NORMAL_DIRECTION
        && (d - axis * d.dot(axis)).length() <= tol_pm()
}

/// A point on the axis and the axis direction of a surface of revolution
/// that re-facets here; `None` for a sphere.
fn revolution_axis(s: &SurfaceRef) -> Option<(Point3, Vec3)> {
    match *s {
        SurfaceRef::Cylinder {
            axis_point, axis, ..
        } => Some((axis_point, axis)),
        SurfaceRef::Cone { apex, axis, .. } => Some((apex, axis)),
        SurfaceRef::Torus { center, axis, .. } => Some((center, axis)),
        SurfaceRef::Sphere { .. } => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ring(pts: &[(f64, f64)]) -> Vec<Point3> {
        pts.iter().map(|&(x, y)| Point3::new(x, y, 0.0)).collect()
    }

    const UP: Vec3 = Vec3 {
        x: 0.0,
        y: 0.0,
        z: 1.0,
    };

    fn square(x0: f64, y0: f64, s: f64) -> Vec<Point3> {
        ring(&[(x0, y0), (x0 + s, y0), (x0 + s, y0 + s), (x0, y0 + s)])
    }

    /// A diamond whose corner `(cx - r, cy)` is its leftmost point.
    fn diamond(cx: f64, cy: f64, r: f64) -> Vec<Point3> {
        ring(&[(cx - r, cy), (cx, cy - r), (cx + r, cy), (cx, cy + r)])
    }

    #[test]
    fn a_hole_touching_the_outer_at_a_vertex_is_valid_on_every_side() {
        let outer = square(0.0, 0.0, 4.0);
        for hole in [
            diamond(1.0, 2.0, 1.0),
            diamond(3.0, 2.0, 1.0),
            diamond(2.0, 1.0, 1.0),
            diamond(2.0, 3.0, 1.0),
        ] {
            assert!(loops_valid(UP, &[outer.clone(), hole]));
        }
    }

    #[test]
    fn holes_touching_at_a_vertex_are_valid_either_way_round() {
        let outer = square(0.0, 0.0, 6.0);
        let (a, b) = (diamond(2.0, 3.0, 1.0), diamond(4.0, 3.0, 1.0));
        assert!(loops_valid(UP, &[outer.clone(), a.clone(), b.clone()]));
        assert!(loops_valid(UP, &[outer, b, a]));
    }

    #[test]
    fn a_hole_running_along_the_outer_is_invalid() {
        let outer = square(0.0, 0.0, 4.0);
        assert!(!loops_valid(UP, &[outer, square(0.0, 0.0, 2.0)]));
    }

    #[test]
    fn holes_sharing_an_edge_or_coinciding_are_invalid() {
        let outer = square(0.0, 0.0, 6.0);
        let a = square(1.0, 1.0, 2.0);
        assert!(!loops_valid(
            UP,
            &[outer.clone(), a.clone(), square(3.0, 1.0, 2.0)]
        ));
        assert!(!loops_valid(UP, &[outer, a.clone(), a]));
    }

    #[test]
    fn a_hole_outside_the_outer_is_invalid() {
        let outer = square(0.0, 0.0, 4.0);
        assert!(!loops_valid(UP, &[outer, square(5.0, 1.0, 1.0)]));
    }
}

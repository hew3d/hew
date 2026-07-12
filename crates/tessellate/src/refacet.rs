//! Export re-faceting: true curves for STL (docs/design/true-curves.md §4.2,
//! stage 6).
//!
//! The viewport renders the stored facets; export may do better. A stamped
//! cylinder wall ([`kernel::SurfaceRef`]) whose **entire boundary is still
//! analytic** — pristine chord-quad facets between two caps perpendicular to
//! the axis, side seams intact — can be re-sampled from its analytic
//! definition at any chosen resolution. A wall that fails that legitimacy
//! condition (boolean seams across facets, bossed facets, slanted caps) is
//! exported at its stored resolution, honestly, never approximated.
//!
//! Watertightness is the load-bearing obligation: a re-faceted wall's cap
//! edges must match the re-faceted rims exactly at ANY resolution. The
//! mechanism is shared points, not tolerance welding — each new rim point is
//! computed once per (band, rim, angle) *station* and the identical f64
//! value is used by the wall quads and by every cap polygon that borders
//! them, so matching is bitwise by construction. Original vertices that
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
/// every qualifying cylinder band at that angular resolution, subdividing
/// between preserved anchor vertices. Non-watertight objects are exported
/// at stored facets (re-faceting reasons over twins).
///
/// Deterministic: face slot order drives band discovery, station
/// generation, and emission order.
pub fn export_triangles(
    object: &Object,
    segments_per_turn: u32,
) -> Result<Vec<f64>, TessellateError> {
    let mut soup: Vec<f64> = Vec::new();

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
        emit_band(band, &mut soup);
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
        emit_polygon(fid, face.plane.normal(), &loops, &mut soup)?;
    }

    Ok(soup)
}

// ─────────────────────────────────────────────────────────────── bands

/// The point a station contributes at one rim level: an original vertex
/// (preserved bit-exact) or a point computed on the analytic circle.
#[derive(Debug, Clone, Copy)]
struct Station {
    /// Unwrapped angle along the chain (monotone with station index).
    angle: f64,
    lo: Point3,
    hi: Point3,
    /// Anchor rim vertices this station preserves, if any (lo, hi).
    lo_vertex: Option<VertexId>,
    hi_vertex: Option<VertexId>,
}

/// One connected chain of pristine chord-quad facets of a single cylinder
/// between two axial levels — the unit of re-faceting.
#[derive(Debug, Clone)]
struct Band {
    faces: Vec<FaceId>,
    axis_point: Point3,
    axis: Vec3,
    radius: f64,
    /// Orthonormal basis perpendicular to `axis` (u × v = axis).
    u: Vec3,
    v: Vec3,
    t_lo: f64,
    t_hi: f64,
    /// Rim vertices in chain order per level; length = faces + 1 for an
    /// open chain, faces for a closed one.
    rim_lo: Vec<VertexId>,
    rim_hi: Vec<VertexId>,
    /// Unwrapped angle of each rim vertex (indexes match `rim_lo`).
    rim_angle: Vec<f64>,
    closed: bool,
    /// Facet normals point away from the axis (an outer wall) or toward it
    /// (a hole wall).
    outward: bool,
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

/// The tolerance used to accept a vertex as lying on its claimed cylinder
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

    let mut bands = Vec::new();
    for (sr, faces) in groups {
        if let Some(mut group_bands) = build_group_bands(object, sr, &faces) {
            bands.append(&mut group_bands);
        }
    }
    bands
}

/// Structural qualification of one surface group; `None` demotes the whole
/// group to stored facets.
fn build_group_bands(object: &Object, sr: SurfaceRef, faces: &[FaceId]) -> Option<Vec<Band>> {
    let SurfaceRef::Cylinder {
        axis_point,
        axis,
        radius,
    } = sr;
    let (u, v) = crate::plane_basis(axis);
    let group_set: BTreeSet<FaceId> = faces.iter().copied().collect();

    // Per-face structure: a chord quad with two vertices on each of two
    // axial levels, all on the cylinder.
    struct Quad {
        face: FaceId,
        t_lo: f64,
        t_hi: f64,
        lo: [VertexId; 2],
        hi: [VertexId; 2],
        /// The two vertical (lo–hi) edges, and the two rim chords (lo, hi).
        verticals: [EdgeId; 2],
        chord_lo: EdgeId,
        chord_hi: EdgeId,
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
        if verts.iter().any(|&w| (radial(w) - radius).abs() > tol_pm()) {
            return None;
        }
        let ts: Vec<f64> = verts.iter().map(|&w| t_of(w)).collect();
        let t_min = ts.iter().cloned().fold(f64::INFINITY, f64::min);
        let t_max = ts.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        if t_max - t_min <= tol_pm() {
            return None; // degenerate band height
        }
        let is_lo: Vec<bool> = ts.iter().map(|&t| (t - t_min).abs() <= tol_pm()).collect();
        if is_lo.iter().filter(|&&b| b).count() != 2
            || ts
                .iter()
                .zip(&is_lo)
                .any(|(&t, &lo)| !lo && (t - t_max).abs() > tol_pm())
        {
            return None; // not exactly two levels, 2 + 2
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
            face: fid,
            t_lo: t_min,
            t_hi: t_max,
            lo: [lo[0], lo[1]],
            hi: [hi[0], hi[1]],
            verticals: [verticals[0], verticals[1]],
            chord_lo,
            chord_hi,
        });
    }

    // Bucket by (t_lo, t_hi) within tolerance.
    let mut buckets: Vec<(f64, f64, Vec<usize>)> = Vec::new();
    for (qi, q) in quads.iter().enumerate() {
        match buckets.iter_mut().find(|(lo, hi, _)| {
            (q.t_lo - *lo).abs() <= tol_pm() && (q.t_hi - *hi).abs() <= tol_pm()
        }) {
            Some((_, _, list)) => list.push(qi),
            None => buckets.push((q.t_lo, q.t_hi, vec![qi])),
        }
    }

    // Rim chords must border a NON-group, surface-free face whose plane is
    // perpendicular to the axis — the analytic-cap legitimacy condition.
    // Checked over every quad regardless of bucket, since the whole group
    // stands or falls together.
    for q in &quads {
        for &chord in &[q.chord_lo, q.chord_hi] {
            let edge = &object.edges()[chord];
            let (h_a, h_b) = (edge.half_edge, edge.twin_half_edge?);
            for h in [h_a, h_b] {
                let f = object.loops()[object.half_edges()[h].loop_id].face;
                if f == q.face {
                    continue;
                }
                if group_set.contains(&f) {
                    return None; // stacked bands sharing a rim: unsupported
                }
                let neighbor = &object.faces()[f];
                if neighbor.surface.is_some() {
                    return None; // rim against another wall: unsupported
                }
                if neighbor.plane.normal().dot(axis).abs() < 1.0 - kernel::tol::NORMAL_DIRECTION {
                    return None; // slanted cap: resampled rim would leave it
                }
            }
        }
    }

    // Assemble chains per bucket via vertical-edge adjacency.
    let mut bands = Vec::new();
    for (t_lo, t_hi, members) in buckets {
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

            // Outward or inward, from the first facet's plane.
            let q0 = &quads[chain[0]];
            let face0 = &object.faces()[q0.face];
            let mid = {
                let a = object.vertices()[q0.lo[0]].position;
                let b = object.vertices()[q0.lo[1]].position;
                Point3::new((a.x + b.x) / 2.0, (a.y + b.y) / 2.0, (a.z + b.z) / 2.0)
            };
            let radial_dir = {
                let d = mid - axis_point;
                d - axis * d.dot(axis)
            };
            let outward = face0.plane.normal().dot(radial_dir) > 0.0;

            let mut band_edges: BTreeSet<EdgeId> = BTreeSet::new();
            let mut chords: BTreeMap<EdgeId, (RimLevel, usize)> = BTreeMap::new();
            for (k, &qi) in chain.iter().enumerate() {
                let q = &quads[qi];
                band_edges.extend(q.verticals);
                band_edges.insert(q.chord_lo);
                band_edges.insert(q.chord_hi);
                chords.insert(q.chord_lo, (RimLevel::Lo, k));
                chords.insert(q.chord_hi, (RimLevel::Hi, k));
            }

            bands.push(Band {
                faces: chain.iter().map(|&qi| quads[qi].face).collect(),
                axis_point,
                axis,
                radius,
                u,
                v,
                t_lo,
                t_hi,
                rim_lo,
                rim_hi,
                rim_angle,
                closed,
                outward,
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
    let angular_eps = tol_pm() / band.radius;
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
    let foot_lo = band.axis_point + band.axis * band.t_lo;
    let foot_hi = band.axis_point + band.axis * band.t_hi;
    let point_at = |foot: Point3, angle: f64| -> Point3 {
        foot + band.u * (band.radius * angle.cos()) + band.v * (band.radius * angle.sin())
    };

    // Anchor stations, in chain order. A lo anchor and hi anchor at the
    // same chain position merge into one station (they are the two ends of
    // one vertical edge — the original grid guarantees the pairing).
    #[derive(Clone, Copy)]
    struct AnchorStation {
        angle: f64,
        lo_vertex: Option<VertexId>,
        hi_vertex: Option<VertexId>,
    }
    let mut anchors: Vec<AnchorStation> = Vec::new();
    for k in 0..n {
        let lo_anchor = has_outside_edge.contains(&band.rim_lo[k]);
        let hi_anchor = has_outside_edge.contains(&band.rim_hi[k]);
        if !band.closed && (k == 0 || k == n - 1) {
            // Chain ends are always stations; their vertical edge borders a
            // neighbor face, so both rim vertices are necessarily anchors.
            anchors.push(AnchorStation {
                angle: band.rim_angle[k],
                lo_vertex: Some(band.rim_lo[k]),
                hi_vertex: Some(band.rim_hi[k]),
            });
            continue;
        }
        if lo_anchor || hi_anchor {
            anchors.push(AnchorStation {
                angle: band.rim_angle[k],
                lo_vertex: lo_anchor.then_some(band.rim_lo[k]),
                hi_vertex: hi_anchor.then_some(band.rim_hi[k]),
            });
        }
    }

    let make_station = |a: &AnchorStation| -> Station {
        Station {
            angle: a.angle,
            lo: a
                .lo_vertex
                .map(|w| object.vertices()[w].position)
                .unwrap_or_else(|| point_at(foot_lo, a.angle)),
            hi: a
                .hi_vertex
                .map(|w| object.vertices()[w].position)
                .unwrap_or_else(|| point_at(foot_hi, a.angle)),
            lo_vertex: a.lo_vertex,
            hi_vertex: a.hi_vertex,
        }
    };
    let subdivide = |from: f64, to: f64, out: &mut Vec<Station>| {
        let sweep = to - from;
        let count = ((sweep.abs() / step) - 1e-9).ceil().max(1.0) as usize;
        for j in 1..count {
            let angle = from + sweep * (j as f64) / (count as f64);
            out.push(Station {
                angle,
                lo: point_at(foot_lo, angle),
                hi: point_at(foot_hi, angle),
                lo_vertex: None,
                hi_vertex: None,
            });
        }
    };

    let mut stations: Vec<Station> = Vec::new();
    if band.closed && anchors.is_empty() {
        // Free choice of phase: start at the first rim vertex's angle.
        let start = band.rim_angle[0];
        let dir = (band.rim_angle[1] - band.rim_angle[0]).signum();
        let full = 2.0 * std::f64::consts::PI * dir;
        let count = ((full.abs() / step) - 1e-9).ceil().max(3.0) as usize;
        for j in 0..count {
            let angle = start + full * (j as f64) / (count as f64);
            stations.push(Station {
                angle,
                lo: point_at(foot_lo, angle),
                hi: point_at(foot_hi, angle),
                lo_vertex: None,
                hi_vertex: None,
            });
        }
    } else if band.closed {
        // Anchored full circle: subdivide between consecutive anchors,
        // wrapping from the last back to the first.
        let dir = (band.rim_angle[1] - band.rim_angle[0]).signum();
        for i in 0..anchors.len() {
            let a = anchors[i];
            stations.push(make_station(&a));
            let next_angle = if i + 1 < anchors.len() {
                anchors[i + 1].angle
            } else {
                anchors[0].angle + 2.0 * std::f64::consts::PI * dir
            };
            if (next_angle - a.angle).abs() <= angular_eps {
                return None; // two unpaired anchors at one angle
            }
            subdivide(a.angle, next_angle, &mut stations);
        }
    } else {
        for i in 0..anchors.len() {
            let a = anchors[i];
            stations.push(make_station(&a));
            if i + 1 < anchors.len() {
                if (anchors[i + 1].angle - a.angle).abs() <= angular_eps {
                    return None;
                }
                subdivide(a.angle, anchors[i + 1].angle, &mut stations);
            }
        }
        if stations.len() < 2 {
            return None;
        }
    }
    Some(stations)
}

// ───────────────────────────────────────────────────────────── emission

/// Emit the re-faceted wall quads for one band.
fn emit_band(band: &Band, soup: &mut Vec<f64>) {
    let s = &band.stations;
    let count = if band.closed { s.len() } else { s.len() - 1 };
    // Winding: increasing angle is CCW seen from +axis; an outward wall
    // swept CCW winds [lo_i, lo_j, hi_j, hi_i] to face outward. A clockwise
    // chain or an inward (hole) wall flips — and both together cancel.
    let increasing = if band.closed && s.len() >= 2 {
        s[1].angle > s[0].angle
    } else {
        s[s.len() - 1].angle > s[0].angle
    };
    let flip = band.outward != increasing;
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
    let (u, v) = crate::plane_basis(normal);
    let to_arr = |l: &Vec<Point3>| -> Vec<[f64; 3]> { l.iter().map(|p| [p.x, p.y, p.z]).collect() };
    let outer = to_arr(&loops[0]);
    let holes: Vec<Vec<[f64; 3]>> = loops[1..].iter().map(to_arr).collect();
    if outer.len() < 3 {
        return Err(TessellateError::DegenerateFace { face: fid });
    }
    let poly = crate::build_polygon_with_holes(&outer, &holes, u, v)
        .map_err(|crate::UnbridgeableHole| TessellateError::UnbridgeableHole { face: fid })?;
    for [i, j, k] in crate::ear_clip(&poly) {
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
                let len = band.faces.len();
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
/// strictly inside the outer loop, holes pairwise non-overlapping.
fn loops_valid(normal: Vec3, loops: &[Vec<Point3>]) -> bool {
    let (u, v) = crate::plane_basis(normal);
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
        if hole.iter().any(|&p| !point_in_polygon_2d(p, outer)) {
            return false;
        }
        if rings_cross(hole, outer) {
            return false;
        }
    }
    for i in 1..loops2d.len() {
        for j in (i + 1)..loops2d.len() {
            if rings_cross(&loops2d[i], &loops2d[j])
                || loops2d[j]
                    .iter()
                    .any(|&p| point_in_polygon_2d(p, &loops2d[i]))
                || loops2d[i]
                    .iter()
                    .any(|&p| point_in_polygon_2d(p, &loops2d[j]))
            {
                return false;
            }
        }
    }
    true
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

/// Even-odd point-in-polygon (strict interior not required here — rim
/// points touching a boundary exactly are pathological enough to demote).
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

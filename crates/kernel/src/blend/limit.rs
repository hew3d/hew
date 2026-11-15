//! The size limit (WP05 §9, decision I-8): the largest size at which the
//! blend succeeds, by bisection on "plan, apply and guard all pass".

use super::plan::PlanFail;
use super::{BlendError, BlendLimit, BlendProfile, BlendSpec, LimitFeature};
use crate::tol;
use crate::topo::Object;

thread_local! {
    /// How many limit searches this thread has run (test support).
    static SEARCHES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// How many sizes the hard-corner search has tried (test support).
    static PROBES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// How many sizes [`Object::hard_corner_limit`] has tried on the calling
/// thread. Test support only.
#[doc(hidden)]
pub fn limit_probes_for_test() -> usize {
    PROBES.with(std::cell::Cell::get)
}

/// How many limit searches ([`Object::max_blend_size`] and its plain form)
/// the calling thread has run. Test support only.
#[doc(hidden)]
pub fn limit_searches_for_test() -> usize {
    SEARCHES.with(std::cell::Cell::get)
}

/// The feature a failed attempt names.
fn feature(f: PlanFail) -> Result<LimitFeature, BlendError> {
    match f {
        PlanFail::Size(l) => Ok(l),
        PlanFail::Obstructed(_) => Ok(LimitFeature::OtherGeometry),
        // Topology the checks let through counts as "too large" inside the
        // bisection; every other hard refusal holds at any size.
        PlanFail::Hard(BlendError::BlendDegenerate) => Ok(LimitFeature::Unknown),
        PlanFail::Hard(e) => Err(e),
    }
}

impl Object {
    /// The largest size (the profile's primary length, at most the spec's)
    /// at which `spec` blends as [`Object::blend_edges`] makes it, and the
    /// feature that stops it growing. Where the spec's edges run into
    /// existing blends, each size tried re-solves the corner as the commit
    /// does (falling back to the plain blend where the re-solve fails, as
    /// the commit does), so the limit is the commit's.
    ///
    /// # Errors
    /// Any hard refusal of the spec (`UnknownEdge`, `UnsupportedVertex`, …);
    /// `RadiusTooLarge { max: 0, … }` when even a vanishing size fails (at a
    /// re-solved corner where no size up to the asked one builds, the
    /// refusal at the smallest size tried, a hard one as itself).
    pub fn max_blend_size(&self, spec: &BlendSpec) -> Result<BlendLimit, BlendError> {
        let limit = self.max_blend_size_facets(spec);
        self.with_rolling(spec, limit, false).0
    }

    /// `limit` (the facet planners' answer for `spec`) widened to what the
    /// commit builds where a round rolls across the facets as a swept blend
    /// (an imported mesh or a generated wall, D-74, D-119): the largest
    /// size that rolls, found by [`Object::largest_that_rolls`], where it
    /// is larger, named as limited by the shape of the model (`Unknown`).
    /// A preview on an imported mesh does not search (minutes on a scan;
    /// it is asked again on every pointer move); on a generated wall it
    /// does, so it names what the commit names.
    /// `asked_fails` skips trying the asked size itself, which the caller
    /// saw fail. Also returns whether the swept search widened it.
    pub(crate) fn with_rolling(
        &self,
        spec: &BlendSpec,
        limit: Result<BlendLimit, BlendError>,
        asked_fails: bool,
    ) -> (Result<BlendLimit, BlendError>, bool) {
        let (Some(s0), BlendProfile::Round { .. }) = (spec.profile.size(), &spec.profile) else {
            return (limit, false);
        };
        // An import's search takes minutes on a scan, so its preview skips
        // it; a generated wall's is bounded and a preview names what the
        // commit names.
        let import = self.planarity_tol > crate::tol::PLANE_DIST;
        if (super::is_preview() && import)
            || super::is_catalog_only()
            || !self.facets_roll(spec)
            || limit.as_ref().is_ok_and(|l| l.max >= s0)
        {
            return (limit, false);
        }
        match self.largest_that_rolls(spec, asked_fails) {
            Some(m) if limit.as_ref().map_or(true, |l| m > l.max) => (
                Ok(BlendLimit {
                    max: m,
                    limited_by: LimitFeature::Unknown,
                }),
                true,
            ),
            _ => (limit, false),
        }
    }

    /// The largest size up to `spec`'s that rolls as the commit makes it
    /// (the facet planners first, the swept blend where they fall back to
    /// it; one round of the swept blend's station removal per attempt), or
    /// `None`. Which sizes roll is not monotone on facets (on the S2 deck
    /// 40 mm rolls, 50 mm does not, 0.1 m does), so the search scans down
    /// from the asked size, [`ROLL_SCAN_PER_OCTAVE`] sizes in each of the
    /// [`ROLL_SCAN_OCTAVES`] octaves below it, stops at the first that rolls
    /// (the largest found), and bisects the gap to the failing size above it
    /// to within [`super::SWEPT_LIMIT_PRECISION`] of the size: it never
    /// names a size below one it found to roll. At most 16 sizes scanned
    /// and two bisections, each a facet attempt and at most one swept one;
    /// on an imported mesh, whose attempts take seconds, one size per
    /// octave (the halving of D-74).
    pub(crate) fn largest_that_rolls(&self, spec: &BlendSpec, asked_fails: bool) -> Option<f64> {
        let s0 = spec.profile.size()?;
        let rolls = |s: f64| {
            let probe = spec.at_size(s);
            super::swept::with_avoid_rounds(1, || match self.blend_once(&probe) {
                Ok(_) => true,
                Err(
                    PlanFail::Hard(
                        BlendError::UnsupportedVertex { .. } | BlendError::UnsupportedEdge,
                    )
                    | PlanFail::Size(_),
                ) => super::classify::with_forced_swept(|| self.blend_once(&probe)).is_ok(),
                Err(_) => false,
            })
        };
        if !asked_fails && rolls(s0) {
            return Some(s0);
        }
        // An import's attempts take seconds on a scan (D-74): one size per
        // octave there, the halving that search always made.
        let per_octave = if self.planarity_tol > crate::tol::PLANE_DIST {
            1
        } else {
            ROLL_SCAN_PER_OCTAVE
        };
        let mut above = s0;
        let mut lo = None;
        let mut top = s0;
        'scan: for _ in 0..ROLL_SCAN_OCTAVES {
            for k in 1..=per_octave {
                let s = top * (1.0 - 0.5 * k as f64 / per_octave as f64);
                if rolls(s) {
                    lo = Some(s);
                    break 'scan;
                }
                above = s;
            }
            top *= 0.5;
        }
        let mut lo = lo?;
        let mut hi = above;
        while hi - lo > super::SWEPT_LIMIT_PRECISION * hi {
            let mid = 0.5 * (lo + hi);
            if rolls(mid) {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        Some(lo)
    }

    /// [`Object::max_blend_size`] for the facet planners (and the corner
    /// re-solve) alone, without the swept blend's sizes.
    pub(crate) fn max_blend_size_facets(&self, spec: &BlendSpec) -> Result<BlendLimit, BlendError> {
        let corners = match spec.profile.size() {
            Some(_) => super::edit::CornerResolve::prepare(self, spec),
            None => None,
        };
        let Some(corners) = corners else {
            return self.max_blend_size_plain(spec);
        };
        let soft = |f: &PlanFail| matches!(f, PlanFail::Size(_) | PlanFail::Obstructed(_));
        // A hard refusal at the asked size stands (it is the commit's), but
        // below it one is a size failure too: which corner pass comes first
        // depends on the size, so whether the planner can build a corner
        // does as well.
        let window = corners.neighbour_sizes();
        self.max_size_by(spec, Some(&window), true, |probe| {
            let resolved = match corners.run(probe, |o, s| {
                o.blend_attempt(s)
                    .map(|(after, report, _)| (after, report))
                    .map_err(|(f, _)| f)
            }) {
                Ok(_) => return Ok(()),
                Err(f) => f,
            };
            // The commit's fallback; where both fail, the re-solve's
            // failure when it ran out of room and the plain one is a size
            // failure too or the planner's refusal of the corner (as the
            // commit's refusal picks), else the plain one's.
            match self.blend_attempt(probe) {
                Ok(_) => Ok(()),
                Err((plain, _)) => Err(match plain {
                    PlanFail::Hard(BlendError::UnsupportedVertex { .. })
                    | PlanFail::Size(_)
                    | PlanFail::Obstructed(_)
                        if soft(&resolved) =>
                    {
                        resolved
                    }
                    _ => plain,
                }),
            }
        })
    }

    /// The largest size below `spec`'s that the facet planners build, where
    /// they refuse a corner outright at `spec`'s size (D-119): sizes are
    /// tried from the top down, [`HARD_SCAN_PER_OCTAVE`] evenly in each of
    /// the [`HARD_SCAN_OCTAVES`] octaves below the asked size, stopping at
    /// the first that builds, then at most [`HARD_BISECTIONS`] halvings of
    /// the gap to the failing size above it; a hard refusal counts as too
    /// large throughout. Facet planners only, no swept attempt (a spec
    /// with an edge outside their catalog is not searched), so the search
    /// costs the same in a preview and a commit (58 attempts at most). `None` when no size tried builds; the feature named is the
    /// last size failure the bisection saw, else the scan's, else
    /// `Unknown`.
    pub(crate) fn hard_corner_limit(&self, spec: &BlendSpec) -> Option<BlendLimit> {
        let s0 = spec.profile.size()?;
        // Facet planners only: an edge outside their catalog plans as a
        // swept blend, whose attempts this search does not make.
        let infos = super::classify::classify(self, spec).ok()?;
        if infos
            .iter()
            .any(|i| i.kind == super::classify::EdgeKind::Swept)
        {
            return None;
        }
        let at = |s: f64| {
            PROBES.with(|c| c.set(c.get() + 1));
            self.blend_once(&spec.at_size(s)).map(|_| ())
        };
        let size_feature =
            |f: PlanFail| feature(f).ok().filter(|_| !matches!(f, PlanFail::Hard(_)));
        let mut sizes: Vec<f64> = Vec::new();
        let mut top = s0;
        for _ in 0..HARD_SCAN_OCTAVES {
            let step = 0.5 * top / HARD_SCAN_PER_OCTAVE as f64;
            sizes.extend((1..=HARD_SCAN_PER_OCTAVE).map(|k| top - step * k as f64));
            top *= 0.5;
        }
        sizes.retain(|&t| t >= tol::BLEND_LIMIT_FLOOR_REL * s0);
        let mut above = s0;
        let mut limited_by = None;
        let mut lo = None;
        for s in sizes {
            match at(s) {
                Ok(()) => {
                    lo = Some(s);
                    break;
                }
                Err(f) => {
                    above = s;
                    limited_by = size_feature(f).or(limited_by);
                }
            }
        }
        let mut lo = lo?;
        let mut hi = above;
        let mut bisected = None;
        for _ in 0..HARD_BISECTIONS {
            if hi - lo <= tol::POINT_MERGE {
                break;
            }
            let mid = 0.5 * (lo + hi);
            match at(mid) {
                Ok(()) => lo = mid,
                Err(f) => {
                    hi = mid;
                    bisected = size_feature(f).or(bisected);
                }
            }
        }
        Some(BlendLimit {
            max: lo,
            limited_by: bisected.or(limited_by).unwrap_or(LimitFeature::Unknown),
        })
    }

    /// [`Object::max_blend_size`] for the plain blend (no corner re-solve):
    /// the limit [`Object::blend_edges_plain`] has, which a refusal from
    /// that path names (a blend put back after an edit, say). It tries the
    /// facet planners alone, as the limit always has: an imported mesh's
    /// swept limit is [`Object::swept_refusal`]'s own search (D-74).
    pub(crate) fn max_blend_size_plain(&self, spec: &BlendSpec) -> Result<BlendLimit, BlendError> {
        // A hard refusal below the asked size counts as a size failure
        // where every edge is the facet planners' (D-119); a swept edge's
        // attempts cost too much to search past one.
        let facets_only = super::classify::classify(self, spec).is_ok_and(|infos| {
            infos
                .iter()
                .all(|i| i.kind != super::classify::EdgeKind::Swept)
        });
        self.max_size_by(spec, None, facets_only, |probe| {
            self.blend_once(probe).map(|_| ())
        })
    }

    /// The limit search over `at` (one attempt at a scaled spec).
    ///
    /// With `window` (a corner re-solve: the sizes of the blends it puts
    /// back), the sizes that build need not be one interval from zero: they
    /// can start well above a neighbour's size, and an equal-size merge can
    /// build at a single size below them. So the search scans a fixed set
    /// of sizes from the asked one down (see [`scan_limit`]) and
    /// bisects up from the largest that builds; a hard refusal below the
    /// asked size counts as a size failure there, as it does with
    /// `hard_below` (D-119; else a hard refusal ends the search).
    fn max_size_by(
        &self,
        spec: &BlendSpec,
        window: Option<&[f64]>,
        hard_below: bool,
        at: impl Fn(&BlendSpec) -> Result<(), PlanFail>,
    ) -> Result<BlendLimit, BlendError> {
        let Some(s0) = spec.profile.size() else {
            // A bullnose has no size: its radius is half the face's width.
            let infos = super::classify::classify(self, spec)?;
            let bn = super::plan::bullnose_frame(self, &infos).map_err(|f| match f {
                PlanFail::Hard(e) => e,
                _ => BlendError::BullnoseNeedsParallelSides,
            })?;
            return Ok(BlendLimit {
                max: bn.r,
                limited_by: LimitFeature::Unknown,
            });
        };
        // Below the asked size (whose own refusal stands): the feature, or
        // `None` for a hard refusal, which counts as too large (it names no
        // feature, so it does not replace the one found). Below the asked
        // size the planners can refuse a corner outright at sizes where a
        // smaller one builds (its run-out reaches the next vertex), so with
        // `hard_below` such a refusal does not end the search (D-119); where
        // nothing builds, the hard refusal at the floor stands.
        let below = |f: PlanFail| match feature(f) {
            Ok(l) => Ok(Some(l)),
            Err(_) if hard_below => Ok(None),
            Err(e) => Err(e),
        };
        SEARCHES.with(|c| c.set(c.get() + 1));
        let at = |s: f64| at(&spec.at_size(s));
        let fail_hi = match at(s0) {
            Ok(()) => {
                return Ok(BlendLimit {
                    max: s0,
                    limited_by: LimitFeature::Unknown,
                });
            }
            Err(f) => feature(f)?,
        };
        let mut hi = s0;
        let mut limited_by = fail_hi;
        let mut lo = tol::BLEND_LIMIT_FLOOR_REL * s0;
        if let Some(neighbours) = window {
            let (found, above, by) = scan_limit(s0, neighbours, &at)?;
            lo = found;
            hi = above;
            if let Some(l) = by {
                limited_by = l;
            }
        } else if let Err(f) = at(lo) {
            // A vanishing size can fail on its own (a facet narrower than the
            // merge tolerance), so the floor is no lower bound there: try
            // sizes down from the asked one for one that works.
            let floor_hard = match f {
                PlanFail::Hard(e) if e != BlendError::BlendDegenerate => Some(e),
                _ => None,
            };
            let floor = below(f)?.unwrap_or(LimitFeature::Unknown);
            let mut tries: Vec<f64> = Vec::new();
            let mut s = s0;
            for _ in 0..20 {
                s *= 0.5;
                tries.push(s);
            }
            tries.retain(|&t| t < s0 && t > lo);
            tries.sort_by(|x, y| y.total_cmp(x));
            tries.dedup_by(|x, y| (*x - *y).abs() <= tol::POINT_MERGE);
            let mut found = None;
            for s in tries {
                match at(s) {
                    Ok(()) => {
                        found = Some(s);
                        break;
                    }
                    Err(f) => {
                        hi = s;
                        if let Some(l) = below(f)? {
                            limited_by = l;
                        }
                    }
                }
            }
            let Some(s) = found else {
                // Nothing builds: a corner refused at every size is that
                // refusal, else too large at any size.
                return Err(floor_hard.unwrap_or(BlendError::RadiusTooLarge {
                    max: 0.0,
                    limited_by: floor,
                }));
            };
            lo = s;
        }
        for _ in 0..60 {
            if hi - lo <= tol::POINT_MERGE {
                break;
            }
            let mid = 0.5 * (lo + hi);
            match at(mid) {
                Ok(()) => lo = mid,
                Err(f) => {
                    hi = mid;
                    if let Some(l) = below(f)? {
                        limited_by = l;
                    }
                }
            }
        }
        Ok(BlendLimit {
            max: lo,
            limited_by,
        })
    }
}

/// The scan of [`Object::max_blend_size`] at a corner re-solve: the sizes
/// below the asked `s0` that it tries, from the top down, stopping at the
/// first that builds. Each of the top [`SCAN_OCTAVES`] octaves below `s0`
/// is sampled evenly at [`SCAN_PER_OCTAVE`] sizes (a window of sizes that
/// build is found wherever it is wider than about 1/32 of its top), then
/// one size per octave further down to a millionth of `s0`, plus each
/// neighbour's size and a little above it (an equal-size merge builds at
/// that one size). Returns the size found, the failing size just above it
/// (where the bisection starts) and that failure's feature. Where nothing
/// builds, the refusal at the smallest size tried: a hard refusal (the
/// corner cannot be built at any size) as itself, else `RadiusTooLarge {
/// max: 0 }`.
fn scan_limit(
    s0: f64,
    neighbours: &[f64],
    at: &impl Fn(f64) -> Result<(), PlanFail>,
) -> Result<(f64, f64, Option<LimitFeature>), BlendError> {
    let mut sizes: Vec<f64> = Vec::new();
    let mut top = s0;
    for _ in 0..SCAN_OCTAVES {
        let step = 0.5 * top / SCAN_PER_OCTAVE as f64;
        sizes.extend((0..SCAN_PER_OCTAVE).map(|k| top - step * k as f64));
        top *= 0.5;
    }
    while top > tol::BLEND_LIMIT_FLOOR_REL * s0 {
        sizes.push(top);
        top *= 0.5;
    }
    sizes.push(tol::BLEND_LIMIT_FLOOR_REL * s0);
    for &n in neighbours {
        sizes.extend(NEIGHBOUR_MULTIPLES.iter().map(|k| n * k));
    }
    sizes.retain(|&t| t < s0 && t >= tol::BLEND_LIMIT_FLOOR_REL * s0);
    sizes.sort_by(|x, y| y.total_cmp(x));
    sizes.dedup_by(|x, y| (*x - *y).abs() <= tol::POINT_MERGE);
    let mut above = s0;
    let mut by = None;
    let mut last = None;
    for s in sizes {
        match at(s) {
            Ok(()) => return Ok((s, above, by)),
            Err(f) => {
                above = s;
                if let Ok(l) = feature(f) {
                    by = Some(l);
                }
                last = Some(f);
            }
        }
    }
    Err(match last {
        Some(PlanFail::Hard(e)) if e != BlendError::BlendDegenerate => e,
        Some(f) => BlendError::RadiusTooLarge {
            max: 0.0,
            limited_by: feature(f).unwrap_or(LimitFeature::Unknown),
        },
        None => BlendError::RadiusTooLarge {
            max: 0.0,
            limited_by: LimitFeature::Unknown,
        },
    })
}

/// The octaves below the asked size a re-solve's limit scan samples evenly.
const SCAN_OCTAVES: usize = 6;
/// Sizes per octave in that scan.
const SCAN_PER_OCTAVE: usize = 16;
/// The sizes the scan tries at and just above each neighbour's (as
/// multiples of it): equal sizes merge into one blend, and one a hair above
/// may not build.
const NEIGHBOUR_MULTIPLES: [f64; 6] = [1.0, 1.001, 1.01, 1.05, 1.1, 1.25];

/// The octaves below the asked size the hard-corner search samples.
const HARD_SCAN_OCTAVES: usize = 3;
/// Sizes per octave in that search.
const HARD_SCAN_PER_OCTAVE: usize = 16;
/// Halvings of the gap after the first size that builds.
const HARD_BISECTIONS: usize = 10;

/// The octaves below the asked size [`Object::largest_that_rolls`] scans.
const ROLL_SCAN_OCTAVES: usize = 4;
/// Sizes per octave in that scan (each octave's top times 7/8, 3/4, 5/8,
/// 1/2).
const ROLL_SCAN_PER_OCTAVE: usize = 4;

#[cfg(test)]
mod tests {
    use crate::blend::{BlendProfile, BlendSpec, LimitFeature};
    use crate::loft::{LoftProfile, LoftSpec};
    use crate::math::Point3;
    use crate::topo::Object;

    #[test]
    fn the_hard_corner_search_stops_at_the_first_size_that_builds() {
        // The real-app check's S2 deck (perimeter-matched, see below): the
        // facet planners build its rim up to 14.3 mm; asked at 50 mm, the
        // search finds that within 48 sizes and ten bisections.
        let rect = |w: f64, h: f64, z: f64| LoftProfile {
            points: vec![
                Point3::new(w, -h, z),
                Point3::new(w, h, z),
                Point3::new(-w, h, z),
                Point3::new(-w, -h, z),
            ],
            curves: Vec::new(),
            smooth: Vec::new(),
            through: Vec::new(),
            start_offset: 0,
        };
        // The middle rectangle's (−x, +y) corner chamfered 2 cm: five
        // corners against four, so the Loft matches them by perimeter and
        // the deck keeps its short pieces by the corners (D-126).
        let mut middle = rect(0.6, 0.4, 0.5);
        middle.points.splice(
            2..3,
            [Point3::new(-0.58, 0.4, 0.5), Point3::new(-0.6, 0.38, 0.5)],
        );
        let hull = Object::from_loft(&LoftSpec {
            profiles: vec![rect(0.5, 0.3, 0.0), middle, rect(0.55, 0.35, 1.0)],
            apex: None,
            rails: Vec::new(),
            stations: None,
        })
        .unwrap();
        let deck: Vec<_> = hull
            .sharp_edges()
            .into_iter()
            .filter(|&e| {
                let (p, q) = hull.edge_endpoints(e).unwrap();
                (p.z - 1.0).abs() < 1e-9 && (q.z - 1.0).abs() < 1e-9
            })
            .collect();
        let spec = BlendSpec {
            edges: deck,
            profile: BlendProfile::Round {
                radius: 0.05,
                segments: 6,
            },
            primary: None,
            corners: Default::default(),
            stops: None,
            handles: Vec::new(),
        };
        let before = super::limit_probes_for_test();
        let limit = hull.hard_corner_limit(&spec).expect("a size builds");
        let used = super::limit_probes_for_test() - before;
        assert!(used <= 58, "{used}");
        assert!((limit.max - 0.01426).abs() < 3e-4, "{}", limit.max);
        assert_eq!(limit.limited_by, LimitFeature::ShortEdge);
    }
}

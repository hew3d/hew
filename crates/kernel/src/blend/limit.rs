//! The size limit (WP05 §9, decision I-8): the largest size at which the
//! blend succeeds, by bisection on "plan, apply and guard all pass".

use super::plan::PlanFail;
use super::{BlendError, BlendLimit, BlendProfile, BlendSpec, LimitFeature};
use crate::tol;
use crate::topo::Object;

thread_local! {
    /// How many limit searches this thread has run (test support).
    static SEARCHES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// How many sizes the limit searches have tried (test support).
    static ATTEMPTS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// How many sizes the hard-corner search has tried (test support).
    static PROBES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// How many sizes [`Object::hard_corner_limit`] has tried on the calling
/// thread. Test support only.
#[doc(hidden)]
pub fn limit_probes_for_test() -> usize {
    PROBES.with(std::cell::Cell::get)
}

/// How many sizes the limit searches ([`Object::max_blend_size`] and its
/// plain form) have tried on the calling thread, the asked size included.
/// Test support only.
#[doc(hidden)]
pub fn limit_attempts_for_test() -> usize {
    ATTEMPTS.with(std::cell::Cell::get)
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
    /// On a spec with a swept chain (and no corner re-solve), where the
    /// asked size fails on size or obstruction, a hard refusal at a smaller
    /// size is not the answer where any size builds: it counts as too large,
    /// naming no feature, and the search goes on (where none builds, the
    /// floor's hard refusal stands). Which sizes build need not be one
    /// interval there (a narrow window of refused sizes can lie below the
    /// limit), and the size named is the one [`swept_limit`] defines from
    /// the shape alone, the same whatever size is asked. A hard refusal of
    /// the asked size itself stands.
    ///
    /// # Errors
    /// Any hard refusal of the spec at the asked size (`UnknownEdge`,
    /// `UnsupportedVertex`, …); `RadiusTooLarge { max: 0, … }` when even a
    /// vanishing size fails (at a re-solved corner where no size up to the
    /// asked one builds, the refusal at the smallest size tried, a hard one
    /// as itself).
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
        self.max_size_by(spec, Some(&window), false, |probe| {
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
    ///
    /// Where any edge is a swept chain, the search is [`swept_limit`]'s:
    /// below an asked size that failed on size or obstruction, a hard
    /// refusal at a smaller size is not the answer where any size builds
    /// (it counts as too large, naming no feature, and the search goes on),
    /// and the size named does not depend on the size asked. A hard refusal
    /// of the asked size itself stands.
    pub(crate) fn max_blend_size_plain(&self, spec: &BlendSpec) -> Result<BlendLimit, BlendError> {
        let swept = super::classify::classify(self, spec).is_ok_and(|infos| {
            infos
                .iter()
                .any(|i| i.kind == super::classify::EdgeKind::Swept)
        });
        self.max_size_by(spec, None, swept, |probe| {
            self.blend_once(probe).map(|_| ())
        })
    }

    /// The limit search over `at` (one attempt at a scaled spec). A hard
    /// refusal of the asked size stands; below it one counts as a size
    /// failure (D-119: the planners can refuse a corner outright at sizes
    /// where a smaller one builds, its run-out reaching the next vertex).
    ///
    /// With `window` (a corner re-solve: the sizes of the blends it puts
    /// back), the sizes that build need not be one interval from zero: they
    /// can start well above a neighbour's size, and an equal-size merge can
    /// build at a single size below them. So the search scans a fixed set
    /// of sizes from the asked one down (see [`scan_limit`]) and
    /// bisects up from the largest that builds. With `swept` (a spec with a
    /// swept chain, no window) the search is [`swept_limit`]'s.
    fn max_size_by(
        &self,
        spec: &BlendSpec,
        window: Option<&[f64]>,
        swept: bool,
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
        SEARCHES.with(|c| c.set(c.get() + 1));
        let at = |s: f64| {
            ATTEMPTS.with(|c| c.set(c.get() + 1));
            at(&spec.at_size(s))
        };
        if swept && window.is_none() {
            let import = self.planarity_tol > tol::PLANE_DIST;
            return swept_limit(s0, import, &at);
        }
        // Below the asked size (whose own refusal stands): the feature, or
        // `None` for a hard refusal, which counts as too large (it names no
        // feature, so it does not replace the one found); where nothing
        // builds, the hard refusal at the floor stands.
        let below = |f: PlanFail| feature(f).ok();
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
            let floor = below(f).unwrap_or(LimitFeature::Unknown);
            let mut found = None;
            for s in halvings(s0) {
                match at(s) {
                    Ok(()) => {
                        found = Some(s);
                        break;
                    }
                    Err(f) => {
                        hi = s;
                        if let Some(l) = below(f) {
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
        for _ in 0..BISECTIONS {
            if hi - lo <= tol::POINT_MERGE {
                break;
            }
            let mid = 0.5 * (lo + hi);
            match at(mid) {
                Ok(()) => lo = mid,
                Err(f) => {
                    hi = mid;
                    if let Some(l) = below(f) {
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

/// The sizes the limit search tries down from the asked `s0` where the
/// floor fails: [`HALVINGS`] halvings of it, largest first, above the
/// floor.
fn halvings(s0: f64) -> Vec<f64> {
    let floor = tol::BLEND_LIMIT_FLOOR_REL * s0;
    let mut tries: Vec<f64> = Vec::new();
    let mut s = s0;
    for _ in 0..HALVINGS {
        s *= 0.5;
        tries.push(s);
    }
    tries.retain(|&t| t < s0 && t > floor);
    tries.sort_by(|x, y| y.total_cmp(x));
    tries.dedup_by(|x, y| (*x - *y).abs() <= tol::POINT_MERGE);
    tries
}

/// What one attempt below the asked size says to [`swept_limit`].
#[derive(Clone, Copy)]
enum Probe {
    /// The size builds.
    Builds,
    /// Too large: a size failure, or an obstruction, with the feature in
    /// the way.
    TooLarge(LimitFeature),
    /// A hard refusal (any, `BlendDegenerate` included): too large, with
    /// no feature named.
    Refused,
}

impl Probe {
    fn of(r: Result<(), PlanFail>) -> Probe {
        match r {
            Ok(()) => Probe::Builds,
            Err(PlanFail::Size(l)) => Probe::TooLarge(l),
            Err(PlanFail::Obstructed(_)) => Probe::TooLarge(LimitFeature::OtherGeometry),
            Err(PlanFail::Hard(_)) => Probe::Refused,
        }
    }
}

/// The limit search on a spec with a swept chain, over `at` (one attempt at
/// a size), from the asked size `s0`; `import` on an imported mesh.
///
/// Which sizes build is not one interval from zero there. On a rim whose
/// chamfer meets the corner of a facet at its end, sizes build, then a
/// narrow window is refused (hard, or `BlendDegenerate`), then sizes build
/// again, then everything is too large: the sizes that fail on size or
/// obstruction are one run from the size limit up, but the refusals below
/// it come and go. So the size named is defined as follows, and depends on
/// the shape alone, never on the size asked:
///
/// 1. A hard refusal of `s0` itself stands (it is the commit's).
/// 2. The size limit is the boundary between the sizes that fail on size
///    and the rest: the floor, or where it fails a halving scan down from
///    `s0` to the first size that builds, then a bisection whose upper end
///    only a size failure moves and whose lower end any other answer
///    moves, a refusal included, to within [`tol::POINT_MERGE`]. Where the
///    largest size below that boundary the bisection tried builds, it is
///    the answer.
/// 3. Where a refusal reaches up to the boundary, the answer is the largest
///    size below it that builds, as found by sizes laid out from the
///    boundary down (as [`Object::hard_corner_limit`] lays them out from
///    the asked size, one per octave on an imported mesh), the first that
///    builds then bisected up against the failing size above it; where
///    none builds above the largest size the search saw build, that size
///    bisected up likewise. The answer is never below a size the search
///    saw build.
///
/// A refusal below `s0` is never the answer where some size builds. Where
/// none does, the floor's hard refusal stands (the corner cannot be built
/// at any size, as the facet planners' search answers), else too large at
/// any size (`RadiusTooLarge { max: 0 }`, the floor's feature where it
/// failed on size).
///
/// Natively both bisections stop within [`tol::POINT_MERGE`], so from any
/// asked size the answer is the same to within it. Steps 1 and 2 make at
/// most 2 + [`HALVINGS`] + [`BISECTIONS`] attempts (the asked size, the
/// floor, the scan, the bisection), as the search always could; step 3 at
/// most [`HARD_SCAN_OCTAVES`] times [`HARD_SCAN_PER_OCTAVE`] sizes and
/// [`BISECTIONS`] more. On an imported mesh, whose attempts take seconds,
/// step 3 tries one size per octave and bisects to within
/// [`super::SWEPT_LIMIT_PRECISION`] of the size, as
/// [`Object::largest_that_rolls`] rations its own.
fn swept_limit(
    s0: f64,
    import: bool,
    at: &impl Fn(f64) -> Result<(), PlanFail>,
) -> Result<BlendLimit, BlendError> {
    let mut limited_by = match at(s0) {
        Ok(()) => {
            return Ok(BlendLimit {
                max: s0,
                limited_by: LimitFeature::Unknown,
            });
        }
        Err(f) => feature(f)?,
    };
    let floor = tol::BLEND_LIMIT_FLOOR_REL * s0;
    // `hi` the smallest size seen to fail on size (the asked one to start
    // with), `below` the largest size under it seen not to, `built` the
    // largest seen to build and whether `below` is that one.
    let mut hi = s0;
    let (mut below, mut built, mut top_builds);
    let at_floor = at(floor);
    let floor_hard = match at_floor {
        Err(PlanFail::Hard(e)) if e != BlendError::BlendDegenerate => Some(e),
        _ => None,
    };
    match Probe::of(at_floor) {
        Probe::Builds => (below, built, top_builds) = (floor, floor, true),
        first => {
            // A vanishing size can fail on its own (a facet narrower than
            // the merge tolerance), so the floor is no lower bound there:
            // sizes down from the asked one, to the first that builds. A
            // size failure under a refusal already seen counts as the
            // refusal does (not past the size limit).
            let mut under: Option<f64> = None;
            let mut found = None;
            for s in halvings(s0) {
                match Probe::of(at(s)) {
                    Probe::Builds => {
                        found = Some(s);
                        break;
                    }
                    Probe::TooLarge(l) if under.is_none() => {
                        hi = s;
                        limited_by = l;
                    }
                    Probe::TooLarge(_) | Probe::Refused => {
                        under.get_or_insert(s);
                    }
                }
            }
            let Some(s) = found else {
                return Err(floor_hard.unwrap_or(BlendError::RadiusTooLarge {
                    max: 0.0,
                    limited_by: match first {
                        Probe::TooLarge(l) => l,
                        _ => LimitFeature::Unknown,
                    },
                }));
            };
            (below, built) = (under.unwrap_or(s), s);
            top_builds = under.is_none();
        }
    }
    for _ in 0..BISECTIONS {
        if hi - below <= tol::POINT_MERGE {
            break;
        }
        let mid = 0.5 * (below + hi);
        match Probe::of(at(mid)) {
            Probe::Builds => (below, built, top_builds) = (mid, mid, true),
            Probe::TooLarge(l) => (hi, limited_by) = (mid, l),
            Probe::Refused => (below, top_builds) = (mid, false),
        }
    }
    if top_builds {
        return Ok(BlendLimit {
            max: built,
            limited_by,
        });
    }
    // A refusal reaches up to the size limit: sizes laid out from it down,
    // the first that builds bisected up against the failing one above it.
    let per_octave = if import { 1 } else { HARD_SCAN_PER_OCTAVE };
    let mut sizes: Vec<f64> = Vec::new();
    let mut top = hi;
    for _ in 0..HARD_SCAN_OCTAVES {
        let step = 0.5 * top / per_octave as f64;
        sizes.extend((1..=per_octave).map(|k| top - step * k as f64));
        top *= 0.5;
    }
    sizes.retain(|&t| t < below && t > floor);
    let mut above = below;
    let found = sizes.into_iter().find(|&t| {
        let b = matches!(Probe::of(at(t)), Probe::Builds);
        if !b {
            above = t;
        }
        b
    });
    // Never below a size seen to build: where the scan found none above
    // the largest one (none at all, or a window the scan stepped over),
    // that one is bisected up instead, against the lowest size above it
    // seen not to build.
    let (mut lo, mut up) = match found {
        Some(t) if t > built => (t, above),
        _ if above > built => (built, above),
        _ => (built, below),
    };
    for _ in 0..BISECTIONS {
        let close = if import {
            super::SWEPT_LIMIT_PRECISION * up
        } else {
            tol::POINT_MERGE
        };
        if up - lo <= close {
            break;
        }
        let mid = 0.5 * (lo + up);
        if matches!(Probe::of(at(mid)), Probe::Builds) {
            lo = mid;
        } else {
            up = mid;
        }
    }
    Ok(BlendLimit {
        max: lo,
        limited_by,
    })
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

/// Halvings of the asked size the limit search tries where the floor fails.
const HALVINGS: usize = 20;
/// Bisection steps of the limit search, at most (it stops within
/// [`tol::POINT_MERGE`]).
const BISECTIONS: usize = 60;

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
    use crate::blend::BlendError;
    use crate::blend::plan::PlanFail;
    use crate::blend::{BlendProfile, BlendSpec, LimitFeature};
    use crate::loft::{LoftProfile, LoftSpec};
    use crate::math::Point3;
    use crate::tol;
    use crate::topo::Object;
    use crate::trig;
    use std::cell::Cell;

    /// Sizes from 0.3 mm to 30 mm, 40 to the decade, as asked sizes.
    fn asked_sizes() -> Vec<f64> {
        (0..=80)
            .map(|k| 0.3e-3 * trig::powf(10f64, k as f64 / 40.0))
            .collect()
    }

    /// Which sizes build on the grooved knob's outer rim chamfer, as
    /// measured: up to 0.21365 mm, then refused (`BlendDegenerate`) to
    /// 0.2150 mm, then again up to 0.2213 mm, too large (facet width)
    /// above; and a vanishing size refused as too small.
    fn knob_rim(s: f64) -> Result<(), PlanFail> {
        if s <= tol::POINT_MERGE {
            return Err(PlanFail::Hard(BlendError::RadiusTooLarge {
                max: 0.0,
                limited_by: LimitFeature::Unknown,
            }));
        }
        if s < 2e-9 + tol::POINT_MERGE {
            return Err(PlanFail::Hard(BlendError::BlendDegenerate));
        }
        match s {
            s if s > 0.2213e-3 => Err(PlanFail::Size(LimitFeature::FacetWidth)),
            s if s > 0.21365e-3 && s < 0.2150e-3 => {
                Err(PlanFail::Hard(BlendError::BlendDegenerate))
            }
            _ => Ok(()),
        }
    }

    /// The search from each asked size, with the attempts each took.
    fn search_all(
        import: bool,
        f: impl Fn(f64) -> Result<(), PlanFail>,
    ) -> Vec<(f64, Result<super::BlendLimit, BlendError>, usize)> {
        asked_sizes()
            .into_iter()
            .map(|s0| {
                let n = Cell::new(0);
                let at = |s: f64| {
                    n.set(n.get() + 1);
                    f(s)
                };
                let r = super::swept_limit(s0, import, &at);
                (s0, r, n.get())
            })
            .collect()
    }

    /// The most attempts [`super::swept_limit`] makes.
    const MOST: usize = 2
        + super::HALVINGS
        + super::BISECTIONS
        + super::HARD_SCAN_OCTAVES * super::HARD_SCAN_PER_OCTAVE
        + super::BISECTIONS;

    /// The most attempts an import's scan and its bisection add: a size
    /// per octave, then halvings of at most an octave down to a tenth.
    const IMPORT_MORE: usize = super::HARD_SCAN_OCTAVES + 8;

    #[test]
    fn a_swept_limit_past_a_refused_window_is_the_top_of_the_sizes_that_build() {
        for import in [false, true] {
            for (s0, r, n) in search_all(import, knob_rim) {
                let l = r.unwrap_or_else(|e| panic!("{s0}: {e:?}"));
                assert!(
                    l.max <= 0.2213e-3 && l.max >= 0.2213e-3 - tol::POINT_MERGE,
                    "asked {s0}: {}",
                    l.max
                );
                assert_eq!(l.limited_by, LimitFeature::FacetWidth, "asked {s0}");
                assert!(n <= MOST, "asked {s0}: {n}");
                // No refusal reaches the limit, so the search ends with its
                // bisection.
                assert!(
                    n <= 2 + super::HALVINGS + super::BISECTIONS,
                    "asked {s0}: {n}"
                );
            }
        }
    }

    #[test]
    fn a_swept_limit_where_a_refusal_reaches_it_is_one_size_from_any_asked_size() {
        // Builds up to 0.2 mm, refused from there to 0.2213 mm, too large
        // above.
        let f = |s: f64| match s {
            s if s > 0.2213e-3 => Err(PlanFail::Size(LimitFeature::AdjacentFace)),
            s if s > 0.2e-3 => Err(PlanFail::Hard(BlendError::UnsupportedVertex {
                at: Point3::new(0.0, 0.0, 0.0),
            })),
            _ => Ok(()),
        };
        for import in [false, true] {
            let all = search_all(import, f);
            let first = all[0].1.as_ref().unwrap().max;
            // At the edge natively; within the swept search's precision on
            // an import (where a size the search saw build can stand in
            // for the scan's own).
            let close = if import {
                super::super::SWEPT_LIMIT_PRECISION * 0.2213e-3
            } else {
                tol::POINT_MERGE
            };
            for (s0, r, n) in &all {
                let l = r.as_ref().unwrap_or_else(|e| panic!("{s0}: {e:?}"));
                assert!(f(l.max).is_ok(), "asked {s0}: {} does not build", l.max);
                assert_eq!(l.limited_by, LimitFeature::AdjacentFace, "asked {s0}");
                assert!(*n <= MOST, "asked {s0}: {n}");
                // An import's scan tries one size per octave and its
                // bisection stops at the swept search's precision.
                assert!(
                    !import || *n <= 2 + super::HALVINGS + super::BISECTIONS + IMPORT_MORE,
                    "asked {s0}: {n} on an import"
                );
                assert!(
                    (l.max - first).abs() <= close,
                    "asked {s0}: {} against {first}",
                    l.max
                );
                assert!(l.max >= 0.2e-3 - close, "asked {s0}: {}", l.max);
            }
        }
    }

    #[test]
    fn a_swept_limit_is_never_below_a_size_the_search_saw_build() {
        // Builds below 0.4 and in a sliver at 0.5 that the first bisection
        // lands in and the scan from the size limit steps over; refused
        // between, too large above 0.52.
        let f = |s: f64| {
            if s > 0.52 {
                Err(PlanFail::Size(LimitFeature::FacetWidth))
            } else if s < 0.4 || (s > 0.4999 && s < 0.5001) {
                Ok(())
            } else {
                Err(PlanFail::Hard(BlendError::BlendDegenerate))
            }
        };
        let l = super::swept_limit(1.0, false, &f).unwrap();
        assert!(f(l.max).is_ok(), "{}", l.max);
        assert!(l.max >= 0.5, "{}", l.max);
        assert_eq!(l.limited_by, LimitFeature::FacetWidth);
    }

    #[test]
    fn a_swept_limit_under_a_wide_refusal_is_the_top_of_the_sizes_that_build() {
        // Builds up to 0.01 mm, refused from there to 0.5 mm (further below
        // the size limit than the scan reaches), too large above.
        let f = |s: f64| match s {
            s if s > 0.5e-3 => Err(PlanFail::Size(LimitFeature::AdjacentFace)),
            s if s > 0.01e-3 => Err(PlanFail::Hard(BlendError::UnsupportedVertex {
                at: Point3::new(0.0, 0.0, 0.0),
            })),
            _ => Ok(()),
        };
        for (s0, r, n) in search_all(false, f) {
            if s0 <= 0.5e-3 {
                continue;
            }
            let l = r.unwrap_or_else(|e| panic!("{s0}: {e:?}"));
            assert!(
                l.max <= 0.01e-3 && l.max >= 0.01e-3 - tol::POINT_MERGE,
                "asked {s0}: {}",
                l.max
            );
            assert!(n <= MOST, "asked {s0}: {n}");
        }
    }

    #[test]
    fn a_swept_limit_keeps_the_asked_sizes_own_hard_refusal() {
        let at = |_: f64| {
            Err(PlanFail::Hard(BlendError::UnsupportedVertex {
                at: Point3::new(1.0, 2.0, 3.0),
            }))
        };
        assert_eq!(
            super::swept_limit(0.01, false, &at),
            Err(BlendError::UnsupportedVertex {
                at: Point3::new(1.0, 2.0, 3.0)
            })
        );
    }

    #[test]
    fn a_swept_limit_where_no_smaller_size_builds_is_the_floors_refusal() {
        // Too large as asked, refused hard at every smaller size: the
        // corner cannot be built at any size, and says so.
        let at = |s: f64| {
            if s >= 0.01 {
                Err(PlanFail::Size(LimitFeature::FacetWidth))
            } else {
                Err(PlanFail::Hard(BlendError::UnsupportedVertex {
                    at: Point3::new(0.0, 0.0, 0.0),
                }))
            }
        };
        assert_eq!(
            super::swept_limit(0.01, false, &at),
            Err(BlendError::UnsupportedVertex {
                at: Point3::new(0.0, 0.0, 0.0)
            })
        );
    }

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

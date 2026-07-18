# Follow Me — design

Follow Me sweeps a closed sketch profile along a path — a chain of sketch
edges, or a solid face's boundary loop — into a new watertight Object. It is
the workhorse for pipes, moldings, bottles, and chamfered or rounded edge
runs. This document fixes the operation's semantics before its
implementation (spec-first, DEVELOPMENT.md §4): eligibility, the join
strategy, the error taxonomy, curve-identity handling, and the undo
contract. The executable specs live in `crates/kernel/tests/op_specs.rs`
(`follow_me_*`) and `document_specs.rs`.

## 1. Semantics

`Object::from_follow_me(profile, path, closed)` sweeps a validated
[`Profile`] (the same type extrusion consumes: planar outer ring, optional
holes, per-edge analytic curve attribution) along a polyline path.

The construction is classic translate-and-miter:

- **Stations.** The profile ring starts on its own plane (station 0). At
  each interior path joint the ring is carried onto the joint's **miter
  plane** — the plane through the joint whose normal is the normalized sum
  of the adjacent segment directions (the angle bisector). Each ring vertex
  travels along the incoming segment direction to its intersection with the
  miter plane; ring vertices at one station therefore differ from the
  previous station by per-vertex multiples of one shared direction, which
  makes every wall quad planar **by construction** (all four corners lie in
  the plane spanned by the profile edge and the segment direction).
- **Walls.** One quad wall per profile boundary edge per path segment.
  Outer-ring walls face outward, hole-ring walls inward (holes sweep into
  tunnels), mirroring `from_extrusion`.
- **Caps.** An open path gets two caps: the profile itself at station 0
  (wound to face against the sweep) and the projected ring on the plane
  perpendicular to the last segment through the path's end point. A closed
  path gets no caps — the final segment's walls close onto the station-0
  ring's own vertices, welding the seam exactly (shared vertices, not
  coincident copies).
- **Result.** A discrete, watertight Object — a first-class solid like any
  extrusion: badge, booleans, push/pull on its flat faces, save/load, and
  solid-gated export all apply to it with no special cases. Freshly swept
  faces carry no material (they resolve to the object default, like
  extruded walls).

The seam weld of a closed path is exact because the path is planar (both
path sources are planar by construction — a sketch is a plane, a face is a
plane) and every joint is mitered: the ring-to-ring transport between
consecutive *perpendicular* cross-sections is an isometry (the miter plane
makes equal angles with both), so the composition around a closed loop is
the identity and the returning ring lands on the station-0 ring up to
floating-point residue absorbed by the planarity tolerance. A seam
anywhere else (e.g. at a path corner, on a non-miter plane) is *not*
identity — which is why the anchoring rules below refuse those
configurations instead of nudging geometry to fit (rule 4).

Two generalizations of that identity argument carry the curve-path
anchoring in §2 (both used only for curve-attributed segments; the proofs
are tolerance-free, the final transported-ring check below still verifies
the landing):

- **Tilted station 0 over a split segment.** When the seam splits one
  segment in two collinear halves, station 0 need not be perpendicular to
  that segment — ANY station-0 plane not parallel to it closes exactly.
  Writing the loop as `C = T_back ∘ K ∘ T_fwd` (K the mitered chain
  between the first and last interior stations, `T_fwd`/`T_back` the
  transports leaving/re-entering station 0, both along the SAME split
  segment direction d), the classic perpendicular-station identity gives
  `K = (T'_back)⁻¹ ∘ (T'_fwd)⁻¹` for the perpendicular reference station;
  substituting, `C` becomes a composition of three projections all along
  d, from station 0 back to itself — the identity. This is what lets a
  radial profile plane cross a circle's facet mid-run at up to half a
  facet angle off the chord.
- **Seam at a joint whose miter plane IS the profile plane.** A seam
  placed exactly at a path vertex closes iff station 0 is that joint's
  miter (bisector) plane — for two facets of the same drawn circle the
  bisector is the analytic tangent at the vertex, so a radial profile
  plane through a facet vertex is exactly the miter plane. This is the
  natural lathe seam (the profile snapped to a drawn rim vertex). At a
  *polyline* corner the bisector is perpendicular to neither segment, so
  the profile-perpendicularity rule still refuses corners (unchanged).

## 2. Eligibility

**Profile.** A closed region of a visible sketch (`SketchRegionId`),
exactly as `extrude_region` takes one. Holes are allowed. Imprinted
sub-faces of a solid are not accepted as profiles in this release (a sweep
growing out of a solid's own face is a union the user performs explicitly,
matching Hew's explicit-combination model).

**Path.** Either:

- a **sketch edge chain**: a set of edges of one visible sketch that forms
  a single connected chain — every vertex incident to at most two selected
  edges, one connected component, no branches. The chain may be open or
  closed. Arc/circle edges participate as their stored draw-time facets
  (the adaptive facet density chosen when the curve was drawn — the sweep
  reuses those facets rather than re-sampling), or
- a **solid face boundary**: the outer loop of one face of a visible world
  object (crown molding around a tabletop). Always a closed path. The
  solid itself is untouched — the sweep produces a separate Object the
  user may union or subtract explicitly. Unlike a sketch path, a face path
  is **clicked directly** and cannot be preselected — a face is not a
  selectable entity a Select click leaves behind, and selecting the whole
  solid contributes no path — so the flat face whose rim the molding follows
  is picked in the tool by pointing at it (§8).

The path's sketch (or the source solid) is never consumed or modified.

**Anchoring.** The profile plane must be *perpendicular to the path where
the sweep starts* — Hew does not re-orient the profile (SketchUp rotates
it silently; we refuse instead, so the committed solid is always exactly
where the drawn profile is). What "perpendicular" is measured against
depends on the segment: a plain segment is its own direction, but a
segment carrying `CurveGeom` (a drawn circle/arc facet — path resolution
carries the attribution per segment) is measured against the DRAWN curve,
because a facet chord deviates from the true tangent by half a facet
angle (7.5° at the 24-facet floor — orders beyond any tolerance), which
made radially-placed profiles on drawn circles, i.e. lathes, structurally
impossible under the chord rule:

- **Open path:** the profile plane's normal must be parallel to the first
  segment's direction (within `tol::NORMAL_DIRECTION`) and the first path
  vertex must lie on the profile plane (within `tol::PLANE_DIST`). If the
  *last* vertex satisfies this instead, the path is traversed in reverse.
  For a curve-attributed end segment the normal is instead measured
  against the analytic tangent at the end vertex (the chord's component
  perpendicular to the radial there).
- **Closed path, plain segment:** the profile plane must be perpendicular
  to some segment's direction and cross that segment strictly between its
  endpoints (by more than `tol::POINT_MERGE` at each end). The path is
  rotated to start at that crossing, splitting the segment in two
  collinear halves whose shared station is the profile plane itself. A
  profile plane passing exactly through a path *corner* is refused: the
  seam would have to sit on a non-miter plane, where the returning ring
  provably does not match the profile (see §1) — welding it would be
  silent repair.
- **Closed path, curve-attributed segment:** the profile plane must be
  *radial* to the segment's circle — its normal in the curve's plane
  (within `tol::NORMAL_DIRECTION`) and the plane passing through the
  circle's center (within `tol::PLANE_DIST`); that is precisely
  "perpendicular to the drawn curve where it crosses it". The seam sits
  either at a strict-interior chord crossing (the split-segment arm — the
  station-0 plane tilts from the chord by up to half a facet angle, sound
  by the tilted-station identity in §1) or exactly at a facet VERTEX
  shared by two facets of the same curve (same center and radius within
  `tol::POINT_MERGE`), where the tangent is the chord bisector and the
  profile plane is the joint's own miter plane (§1) — the natural seam
  for a profile snapped onto a drawn rim vertex. Snap-exact placement is
  what makes these tolerances reachable by hand: the drawn circle's own
  vertices, axis ("on axis") points, and axis-locked rotations produce
  exactly radial planes; free-hand placement does not, and refuses.

**Nearest-anchor selection.** A profile plane is generally perpendicular to
the path at *more than one* place: a radial plane through a full circle
crosses it at TWO antipodal points, a rectangle's profile plane crosses both
of a parallel pair, and so on. All such crossings are gathered, then the seam
is anchored at the one **nearest the profile** — the crossing point closest
to the profile's outer-ring centroid — because the profile physically sits at
its own seam. (Path vertices are numbered by sketch-vertex id, unrelated to
where the profile was placed, so "the first perpendicular segment in path
order" is the wrong side about half the time: the mitered transport then
carries the ring from the far side and the advance check refuses a sound
lathe as `PathTooTight`.) Equidistant candidates — a profile centered on the
axis, which the advance check refuses anyway — break to the lowest path index,
keeping the choice deterministic (§7). This is what makes "anywhere on the
rim works" hold for the whole rim, both halves, not just the half the path
happens to number first.

Face-boundary paths carry no curve attribution (cap rims carry no
`CurveGeom` claims — §4), so a face loop always uses the plain-segment
rule; molding around a cylinder's lid keeps chord semantics (a scoped
gap, §8).

The profile may sit anywhere on its plane relative to the path (the ring
is carried rigidly; a molding offset from its spine is legitimate).

## 3. Join strategy and validity bounds

- **Miter joins only.** Every interior joint is a miter (bisector plane).
  There are no round or butt joins in this release.
- **Reversal refusal, with a miter limit.** Adjacent path segments that
  double back exactly (directions summing below
  `tol::NORMALIZE_MIN_LENGTH`) have no miter plane; refused. Joints that
  *nearly* double back refuse via the same variant through a **miter
  limit** (`tol::FOLLOW_ME_MITER_LIMIT`): projecting the ring onto a
  joint's bisector plane stretches every displacement by the classic
  stroke miter ratio `1 / cos(θ/2)`, which diverges as the turn angle θ
  approaches 180° — a joint 1e-3 rad short of a reversal stretches
  ~2000×, committing a watertight-but-absurd spike thousands of times
  the model's own scale that nothing structural can catch (the advance
  check bounds only the *compressed* inner side of a bend, never the
  stretched outer side). The limit of 8 admits every ordinary bend with
  wide margin (90° → 1.41×, 135° → 2.61×, 150° → 3.86×) and refuses
  joints within ~14° of a full reversal, where no usefully bounded miter
  exists. A policy bound on result quality — named once, not a tolerance
  comparison.
- **Advance check (local self-intersection).** After stationing, every
  ring vertex must advance along its segment: for each segment `k` and
  vertex `j`, `(v[k+1][j] − v[k][j]) · d[k] > tol::POINT_MERGE`. A path
  that bends tighter than the profile is wide fails this exactly at the
  vertex that would be dragged backward — refused, nothing built. Its one
  exemption is a **pole** on a circular path (§9): an on-axis vertex is
  fixed by the revolution and legitimately does not advance, so it is
  skipped here rather than read as the self-touch it once was. On any
  non-circular path an on-axis-like vertex is NOT fixed and still refuses.
- **Global self-intersection.** After construction, every face pair of the
  result is checked for improper contact with the same primitive flat-face
  push/pull uses (`faces_improperly_contact`): contact anywhere other than
  legitimately shared vertices/edges — a U-shaped sweep whose legs
  interpenetrate, an end cap grazing a wall — refuses the whole operation.
  This check classifies contacts at shared elements **exactly**, which the
  closed-lathe seam needs: when the radial profile plane crosses a facet a
  hair before a rim vertex (a few microradians — where a vertex seam would
  *not* close, since its station-0 plane is that many microradians off the
  miter plane, so the split-segment arm is the only sound seam), the split
  leaves a full wrap facet and a sub-facet **sliver**. The sliver is
  legitimate geometry, but its two seam walls over one profile edge are
  near-collinear and meet at their shared rim vertex, so the guard's
  segment/segment and segment/plane solves are ill-conditioned there and
  reconstruct that shared vertex displaced by ~1e-7 — orders past
  `tol::POINT_MERGE`. Left unhandled, that displaced point reads as a
  non-shared contact and refuses a sound lathe as `SweepSelfIntersects` at a
  thin band of placement angles either side of every rim vertex. The guard
  therefore takes the contact from the **exact** shared element rather than
  the ill-conditioned reconstruction — but strictly gated on **genuine
  topological sharing**, never mere geometric proximity, because this is a
  safety-critical guard that must not mask a real interpenetration: two
  coplanar segments short-circuit to their meeting point only when it is a
  confirmed shared vertex of the two faces, and a transversal edge snaps to an
  on-plane endpoint only when that endpoint is a confirmed shared vertex.
  Endpoints that merely fall within tolerance of each other, or of the plane,
  but are **not** shared, fall through to the honest crossing solve — so a
  real crossing whose carrier lines meet up to `POINT_MERGE / NORMAL_DIRECTION`
  away, or a piercing whose near-plane endpoint sits outside the other face
  with the crossing in its interior, is still caught. The sliver's rim vertex
  genuinely *is* a shared vertex of its two seam walls, so the gated bypass
  still fires for it. This is robustness in classifying a contact that is
  already there, not geometry repair (rule 4): no vertex is welded, moved, or
  dropped, and the sliver is committed exactly as built.
- **Structural backstop.** The built object must pass the full topology
  validator and be watertight; any failure refuses with the document
  untouched. The operation constructs into a fresh Object and commits only
  after every check passes (strong exception guarantee is trivial: nothing
  existing is mutated before the single commit point).

## 4. Curve identity (true-curves overlay)

- **Profile curve edges → cylinder walls, per segment.** A profile edge
  carrying `CurveGeom` (a drawn arc/circle facet) sweeps, along each path
  segment, a wall that lies exactly on a right circular cylinder: axis =
  that segment's direction through the transported curve center, radius =
  the curve's radius (transport preserves radial distance about the
  segment axis). Each such wall is stamped `SurfaceRef::Cylinder`,
  exactly as extrusion stamps its side walls — so a straight-line path
  sweeping a circular profile *is* a stamped cylinder (smooth shading,
  analytic rims, whole-wall push/pull all apply). Every stamp is verified
  against the validator's own cylinder-claim predicates before it is
  applied and dropped if floating-point transport has drifted past
  tolerance — stamp-wrong is worse than don't-stamp (map-or-drop).
- **Path arcs → faceted, unstamped along the turn.** A profile swept
  around a path arc traces a toroidal surface. There is no `SurfaceRef`
  variant for tori, and inventing one is a file-format change this feature
  does not need; those walls keep their honest facets. Per-segment
  cylinder stamps for curved *profile* edges still apply (each facet of a
  path arc is a straight segment), so a torus shades smooth around its
  tube and faceted along its ring. Recorded as a scoped gap.
- **Export re-faceting.** STL/3MF curve-resolution export treats a
  circle profile swept along *collinear* path runs as the one cylinder it
  is: the stacked per-segment wall rows merge into a single re-facet band
  (the swallowed interior rims must be fully interior to the wall — a
  band whose interior rim is referenced by outside geometry, e.g. an
  open-arc profile whose flat neighbor walls share the seam, demotes to
  stored facets rather than orphan a shared vertex into a T-junction).
  Walls around a path *turn* have elliptical miter rims, outside the band
  machinery's analytic-cap model: they deliberately export at drawn
  resolution. Both behaviors are pinned by specs in
  `crates/tessellate/tests/refacet_specs.rs`.
- **No cap-edge stamping.** Matches extrusion (true-curves C3): cap rim
  edges carry no `CurveGeom` claims.

## 5. Error taxonomy

`FollowMeError` (kernel, `ops.rs`), surfaced as
`DocumentError::FollowMe(_)`; every variant leaves the document untouched:

| Variant | Meaning |
|---|---|
| `EmptyPath` | No path edges given / path has no segments. |
| `UnknownPathEdge` | A path edge handle is stale or from another sketch. |
| `PathBranches` | A path vertex is incident to more than two selected edges. |
| `PathDisconnected` | The selected edges form more than one chain. |
| `PathSegmentTooShort` | Consecutive path points within `tol::POINT_MERGE`. |
| `ProfileNotPerpendicular` | No path end (open) or segment (closed) is perpendicular to the profile plane — for curve-attributed segments, perpendicular to the DRAWN curve (a radial plane); includes a closed sweep whose transported ring fails to land back on the profile (the seam does not close). |
| `PathDetachedFromProfile` | Perpendicularity holds but the path does not start on / cross the profile plane (incl. the closed-path polyline-corner case). |
| `PathReverses` | Adjacent segments double back — exactly (no miter plane exists) or beyond `tol::FOLLOW_ME_MITER_LIMIT` (no usefully bounded miter exists). |
| `PathTooTight` | A ring vertex fails the advance check (bend tighter than the profile). Also a lathe profile touching the axis on a NON-circular path — the on-axis vertex never advances. On a recognized circle path the axis-touching case instead closes a pole (§9), not this error. |
| `ProfileCrossesAxis` | The profile crosses the revolution axis of a drawn-circle path in a way that cannot be revolved into one faithful solid (§9.2): only an axis-centered analytic circle (the sphere) splits losslessly. An asymmetric or multi-lobe crossing is refused — revolve a one-sided profile instead. |
| `SweepSelfIntersects` | The built solid's faces improperly contact (global check). |
| `SweepDegenerate` | Construction or validation failed (backstop, mirrors `ExtrudeError::DegenerateGeometry`); also a pole solid that assembled disconnected (§9.4). |

Document-level handle errors (`UnknownSketch`, `UnknownObject`,
`UnknownFace`, `Sketch(UnknownRegion)`) reuse the existing variants.

## 6. Undo

Follow Me commits exactly like `extrude_region`: it creates one new world
Object and consumes the profile region's exclusive scaffolding from its
sketch (Model D — the outline became the solid's cross-section; for an
open path it is literally the start cap). It therefore records the
existing `DocAction::CreatedObject` — undo hides the solid and re-inserts
the scaffolding atomically (failing typed on `RestoreConflicts`), redo
re-deletes by geometry — machinery already proven by extrusion, gaining
its guarantees without a new action variant. The path sketch (or source
solid) is untouched by the operation, so undo has nothing to restore
there. No per-Object `History` entry is involved (the op is object
*birth*, not object surgery), so rule 9 replay proofs do not apply.

## 7. Persistence and API surface

- **File format: no change.** The swept solid serializes as an ordinary
  Object; its cylinder stamps use the existing `SurfaceRef::Cylinder`
  rows (geometry buffer v4). Nothing new is written.
- **wasm-api:** two thin `Scene` methods (rule 8 —
  smallest surface that solves the problem): `follow_me_along_edges(
  profile_sketch, region, path_sketch, edge_ids[])` and
  `follow_me_around_face(profile_sketch, region, object, face)`, both
  returning the new object id and reconciling caches from the `DocChange`,
  errors thrown as `CODE: message` like every other mutation.

## 8. Scoped out (deliberately)

- Profile re-orientation to a non-perpendicular path (SketchUp rotates the
  profile; Hew refuses typed — revisit only with real demand).
- Profiles that are imprinted sub-faces of a solid; sweeps that consume or
  cut the path's solid (SketchUp's subtractive Follow Me on a face) — use
  explicit boolean subtract with the swept solid instead.
- Open paths with the profile mid-path (sweep both ways from the profile).
- Non-planar (3D) paths: both path sources are planar today; the miter
  construction generalizes, but the closed-seam identity argument and the
  torsion behavior would need their own treatment.
- Toroidal surface identity (no `SurfaceRef::Torus`); path-arc walls stay
  faceted.
- Analytic-tangent anchoring on FACE-boundary paths: face rims carry no
  `CurveGeom` (§4), so molding around a cylinder's lid measures
  perpendicularity against the rim's chords. Deriving rim curve identity
  from the adjacent wall's `SurfaceRef::Cylinder` would close this;
  deferred until it bites.
- Round/butt join styles.
- Pole closure (§9) on a NON-circular closed path: an on-axis vertex there
  is not fixed by any revolution and still refuses `PathTooTight` (no
  silent weld). A profile that crosses the axis but is not an axis-centered
  analytic circle (asymmetric, multi-lobe, off-center, holed) is refused
  `ProfileCrossesAxis` rather than split — only the drawn-circle sphere
  revolves losslessly (§9.2).
- **Face paths on non-plain objects.** `follow_me_around_face` takes only
  `(object, face)` and is coordinate-correct only for a plain,
  identity-placed, top-level world object. A face on a component INSTANCE is
  stored in definition-local space (its world pose lives on the placement),
  and there is no `follow_me_in_component` surface (cf.
  `push_pull_in_component`), so molding a face of a component/group — or of
  anything reached inside an editing context — is out of scope for now. The
  tool gates the face branch to that plain-top-level set (the same
  `defaultFaceEligible` policy every face tool uses) and refuses an
  instanced/grouped/in-context face with copy rather than sweeping it in the
  wrong frame. Revisit if an in-component molding kernel/wasm surface lands.
- Live drag-along-path preview (idiom b): the shipped interaction is
  preselect path → activate Follow Me → click the profile region. In the
  tool, one click on a sketch edge (or a single-edge preselection) picks
  up the edge's whole connected island; at the profile stage a solid-face
  click re-picks the PATH instead of committing (faces are never profiles
  in this release, so the intent is unambiguous), but **only while the path
  is a leftover preselection** — the stale-selection-from-placing-the-
  profile case that would otherwise swallow the click silently. Once the
  path was picked deliberately in the tool, a stray face graze is ignored
  rather than silently swapping the swept face out from under the next
  profile click; Esc steps back to re-pick. The tool tracks this provenance
  (preselection vs in-tool) per path, and a recovered face becomes an
  in-tool pick so a further graze cannot re-target it.

### Picking a solid-face path directly (tool)

A face cannot be preselected (§2), so the path stage of the tool makes the
flat face **directly and discoverably** pickable, and never fails silently:

- **Hover preview.** While choosing the path, the loop under the cursor —
  the face's boundary (or a sketch edge's whole island) — is highlighted
  *before* the click, so the face that will be swept is visible up front.
  This is what makes "which face becomes the path" a deliberate choice
  rather than "whatever the ray happened to hit behind the standing
  profile," the failure the maintainer saw. (A drawn sketch region occludes
  the cursor for the *snap* engine, but the tool's face pick is a direct
  ray, so aim the click at a clear part of the flat face, not through the
  profile that overlaps it.)
- **No silent miss.** A path-stage click that resolves to no followable
  geometry surfaces guidance ("Click the flat face to run the profile
  around it…") instead of doing nothing — repeated empty clicks collapse to
  one message until a real target appears under the cursor again.
- **Face-worded refusals.** The kernel's eligibility errors are the same
  whatever the path source, but two of them read differently when the path
  is a *face the user just clicked*: the profile is already placed and it is
  the FACE that is wrong, so `ProfileNotPerpendicular` is re-worded to "that
  face is parallel to the profile" and `PathTooTight` to "that face is
  thinner than the profile is deep." The profile is **never** silently
  reoriented to fit the face (rule 4; profile re-orientation is scoped out
  above) — a wrong face is named, not worked around. Refusals from a drawn
  (sketch) path keep the generic perpendicularity copy.

## 9. Pole closure — spheres, goblets, cones

A radial profile swept around a **drawn circle** path is a lathe (§2): the
mitered transport is a revolution about the circle's axis (proof below). The
base case in §1–§3 keeps the profile clear of that axis and makes a solid of
revolution with a hole down the middle — a torus, a wheel, a ring (genus 1).
This section covers what happens when the profile *reaches* the axis: it
closes a **pole**, and the result is a solid with no central hole — a sphere,
a goblet, a cone (genus 0). It is how SketchUp makes a sphere: draw a circle
(the path), draw a perpendicular circle through the axis (the profile),
Follow Me the profile around the path.

### 9.1 The axis, and why the transport is a revolution

Pole closure is only well-defined when the path is a **circle** — every
segment carrying the same analytic `CurveGeom` (one center `C`, one radius,
within `tol::POINT_MERGE`), closed and planar. The **axis** is then the line
through `C` along the path plane's normal.

The key fact the whole feature rests on: for a circle path, the miter plane
at every path vertex **contains the axis**. The bisector of two chords
meeting at a circle vertex is the tangent there (the chords make equal angles
with it), so the miter normal is the tangent — perpendicular to the radial —
and the plane through the vertex with that normal contains both the axis
direction and the center. Consequently:

- The station-to-station transport (project along the incoming chord onto the
  next miter plane) carries a radial profile as a **pure rotation about the
  axis**: every point at radial distance `ρ` traces the path's regular
  N-gon of circumradius `ρ`. (This is what the faceted-Pappus lathe volume
  specs already pin.)
- A point **on the axis** (`ρ = 0`) lies on every miter plane, so the
  projection leaves it fixed. On-axis vertices are therefore **fixed across
  every station** to floating-point residue (~1e-16, far inside
  `tol::POINT_MERGE`) — they are poles.

On a **non-circular** closed path the miter planes do not contain any single
line, an "on-axis" vertex is dragged station to station, and the advance
check refuses it exactly as before (§3). Pole closure never engages there.

### 9.2 Touch vs. cross, and the double cover

A profile relates to the axis (a line in the radial profile plane) in three
ways:

- **Clear** — entirely on one side, not reaching the axis. Unchanged: a
  torus/tube (§1).
- **Touching** — on one side, with part of its boundary *on* the axis. That
  on-axis boundary is a segment (or a single tangent vertex); its endpoints
  are poles. A goblet, a cone, a bead: revolve the one-sided region and the
  axis boundary collapses to the pole line. No split needed.
- **Crossing** — area on **both** sides of the axis. This is the sphere's
  profile: a full circle centered on the axis crosses it at the two poles.

The crossing case has a trap. A solid of revolution is generated by revolving
the profile region **on one side** of the axis, 360°. A full circle revolved
naively traces the sphere **twice** — the near half and the far half each
generate the whole surface — yielding two coincident shells, which is not a
watertight solid. The resolution is geometric: **split the crossing profile at
the axis and revolve one half.** The half's on-axis boundary is the diameter
segment (its two ends the poles); its outer boundary traces the surface once.
Splitting a crossing profile reduces it to the *touching* case, so there is
exactly one pole mechanism.

But splitting is only sound when it is **faithful** — when discarding one half
loses nothing and the revolved half is a single shell. Two ways it is not:

- **Asymmetric crossing.** If the two halves differ (a lopsided, egg-shaped
  silhouette), clipping to one side silently *drops the geometry the user drew
  on the other side*. The result is a valid watertight solid built from the
  wrong profile — no error, no signal.
- **Multi-lobe crossing.** If the boundary crosses the axis more than twice (a
  dominant lobe plus a notch dipping across — a plausible baluster
  silhouette), the clipped half has several disjoint on-axis contact segments.
  Suppressing them (§9.3) *severs the revolve into two disjoint watertight
  shells packed into one Object* — `validate()` and `watertight()` check only
  local half-edge consistency, so they certify it as sound. This is precisely
  the silent-invalid-solid outcome this feature exists to avoid (rule 4).

So a crossing profile is split **only** when it is an **analytic circle
centered on the axis** — the exact, deterministic criterion for a lossless,
single-crossing, mirror-symmetric split (the sphere). A circle centered on the
axis crosses at exactly two points and its two halves are mirror images, so
discarding one is lossless and the revolve is one shell. **Every other
crossing** — asymmetric, multi-lobe, an off-center circle, a holed profile —
is refused typed (`ProfileCrossesAxis`), document untouched: revolve a
one-sided profile instead.

The circle test (`outer_is_axis_centered_circle`) does **not trust the
`CurveGeom` attribution**. The public Sketch API stamps every segment added
under an open curve bracket with that bracket's circle, on the circle or not
(no positional check), so a mislabeled profile — real facets plus a detour out
to an off-circle lump — reports one center and radius for every edge while not
being a circle. The test therefore re-verifies the geometry against the claim,
exactly as `cylinder_claim_holds` re-verifies a `SurfaceRef` (map-or-drop): the
attribution must be consistent, its center on the axis, AND every boundary
vertex must sit at the radius from that center (within `tol::PLANE_DIST`).
Geometry that does not match the claimed circle is not a faithful circle and is
refused. This is phase-independent — an off-phase circle's facet vertices
genuinely lie on the circle, which is why vertex-**on**-circle is the right
test where vertex-mirror-symmetry (which the off-phase fixture fails) was not.

`split_profile_for_pole` returns `Ok(None)` for a non-crossing profile,
`Ok(Some(half))` for the faithful circle (a Sutherland–Hodgman clip to the
`+radial` half inserting the two poles; attribution dropped so the split walls
export as honest facets), and `Err(ProfileCrossesAxis)` otherwise. The split
runs **before anchoring** on purpose: it moves the profile centroid off the
axis, which the nearest-anchor rule (§2) needs to seam on the near path
crossing rather than the antipode.

### 9.3 The pole fan — topology

With the profile one-sided (touching), the sweep assembles as in §1 with
three local changes at on-axis ring vertices:

- **Collapse.** Each on-axis vertex is one **shared** position (station 0's
  fixed pole), reused at every station, instead of one copy per station.
  Off-axis vertices keep their per-station copies and weld the closed seam as
  usual.
- **Fan.** A wall quad `[s·a, s·b, s′·b, s′·a]` with **one** on-axis corner
  degenerates to a **triangle** fanning to that pole (the duplicated pole
  corner drops out). Around a pole the N such triangles form a closed
  triangle fan — a disk in the surface — so the pole is a manifold vertex.
- **Suppress.** A wall **both** of whose profile-edge endpoints are on the
  axis (the diameter/axis segment) collapses to fewer than three vertices and
  is dropped: it is interior to the solid (the axis runs through the
  material), not a boundary face.

Manifoldness holds because no directed edge repeats: a pole-fan edge
`pole → s·v` twins the neighbouring fan's `s·v → pole`, and the fan's far
edge `s·v → s′·v` twins the adjacent latitude quad. The result passes the
same validator, watertight check, and global self-intersection test every
sweep does; the sphere's two pole vertices give it the Euler characteristic
`V − E + F = 2` of a genus-0 shell.

**Backstop — single connected component.** The faithful-split gate (§9.2)
refuses every crossing that is not a lossless sphere up front, so it never
reaches assembly. As defense in depth, independent of that gate, a pole solid
is verified to be a **single connected component** (`face_components().len()
== 1`) after it is built; a disconnected result is refused typed
(`SweepDegenerate`), never emitted. This matters because `validate()` and
`watertight()` check only local half-edge consistency — they certify two
disjoint shells as a watertight Object (see the validate() note in §9.5) — so
connectivity is the check that actually enforces "one pole solid, not two."

### 9.4 Determinism and refusals

Determinism (rule 7) is preserved end to end: axis recovery, the faithful-
circle test, the split's clip, pole indexing, and fan assembly are all
order-fixed with no hash-order dependence, so the same input rebuilds
bit-for-bit. What refuses, typed and with the document untouched:

- A profile that **crosses** the axis but is not an axis-centered analytic
  circle — asymmetric, multi-lobe, off-center, or holed (§9.2) —
  `ProfileCrossesAxis`. Revolve a one-sided profile instead.
- An on-axis profile on a **non-circular** closed path (§9.1) —
  `PathTooTight` (the would-be pole is not fixed by any revolution).
- A **genuinely too-tight** non-pole bend (profile wider than the path's
  inner radius of curvature) — `PathTooTight`, unchanged.
- A profile **not radial** to the drawn circle — `ProfileNotPerpendicular`,
  unchanged (an unattributed facet ring is a polyline; a radial plane is
  perpendicular to no chord, so a sphere attempt on an unrecognized circle
  refuses rather than guessing).
- A pole solid that would be **disconnected** — `SweepDegenerate` (the
  connectivity backstop; unreachable through the faithful-split gate today,
  kept as insurance).

### 9.5 Persistence and API

The pole solid serializes as an ordinary Object (its split walls carry no
surface stamps), and the kernel entry point is the existing
`Object::from_follow_me` / `Document::follow_me` — pole closure is handled
entirely inside the sweep from the profile, path, and per-segment curve
attribution it already receives. **No file-format or wasm-api signature
change.** One additive `FollowMeError` variant (`ProfileCrossesAxis`) is
introduced for the faithful-split refusal; it surfaces at the boundary as the
usual `CODE: message` (the code derived from the variant name), needing no
wasm-api code change.

**A validate() gap, flagged.** An Object is definitionally one watertight
solid, yet `validate()` certifies two disjoint shells as `Watertight` — it
never checks single-connectedness. Broadening `validate()` to require one
component was assessed against the full suite and **rejected**: legitimate
operations produce multi-shell Objects (a boolean union of two disjoint
solids keeps both volumes in one Object; contact-test fixtures are
multi-component by construction). The pole path therefore carries its own
connectivity backstop (above) rather than force a general check that would
reject those. Whether Objects *should* be single-component in general — and
where the multi-shell cases ought to split — is a broader kernel question for
the maintainer, noted here rather than decided by this feature.

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
  user may union or subtract explicitly.

The path's sketch (or the source solid) is never consumed or modified.

**Anchoring.** The profile plane must be *perpendicular to the path where
the sweep starts* — Hew does not re-orient the profile (SketchUp rotates
it silently; we refuse instead, so the committed solid is always exactly
where the drawn profile is):

- **Open path:** the profile plane's normal must be parallel to the first
  segment's direction (within `tol::NORMAL_DIRECTION`) and the first path
  vertex must lie on the profile plane (within `tol::PLANE_DIST`). If the
  *last* vertex satisfies this instead, the path is traversed in reverse.
- **Closed path:** the profile plane must be perpendicular to some
  segment's direction and cross that segment strictly between its
  endpoints (by more than `tol::POINT_MERGE` at each end). The path is
  rotated to start at that crossing, splitting the segment in two
  collinear halves whose shared station is the profile plane itself. A
  profile plane passing exactly through a path *corner* is refused: the
  seam would have to sit on a non-miter plane, where the returning ring
  provably does not match the profile (see §1) — welding it would be
  silent repair.

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
  vertex that would be dragged backward — refused, nothing built. (This is
  also what refuses a lathe profile touching its own axis of revolution:
  the on-axis vertex never advances.)
- **Global self-intersection.** After construction, every face pair of the
  result is checked for improper contact with the same primitive flat-face
  push/pull uses (`faces_improperly_contact`): contact anywhere other than
  legitimately shared vertices/edges — a U-shaped sweep whose legs
  interpenetrate, an end cap grazing a wall — refuses the whole operation.
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
| `ProfileNotPerpendicular` | No path end (open) or segment (closed) is perpendicular to the profile plane — including a closed sweep whose transported ring fails to land back on the profile (the seam does not close). |
| `PathDetachedFromProfile` | Perpendicularity holds but the path does not start on / cross the profile plane (incl. the closed-path corner case). |
| `PathReverses` | Adjacent segments double back — exactly (no miter plane exists) or beyond `tol::FOLLOW_ME_MITER_LIMIT` (no usefully bounded miter exists). |
| `PathTooTight` | A ring vertex fails the advance check (bend tighter than the profile; lathe profile touching the axis). |
| `SweepSelfIntersects` | The built solid's faces improperly contact (global check). |
| `SweepDegenerate` | Construction or validation failed (backstop, mirrors `ExtrudeError::DegenerateGeometry`). |

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
- Round/butt join styles; automatic weld of a lathe profile touching its
  axis (refused via `PathTooTight`).
- Live drag-along-path preview (idiom b): the shipped interaction is
  preselect path → activate Follow Me → click the profile region.

# True curved geometry — design decision

Hew's circles and arcs are faceted polygons from the moment they are drawn.
This document settles how "real arcs and circles" (ROADMAP.md, longer-term)
enter the architecture: as first-class analytic B-rep geometry inside the
kernel, or as durable analytic metadata layered over the existing polyhedral
carrier. The two paths diverge at the boolean engine and never reconverge,
so the choice has to be made once, deliberately, before any curve-adjacent
work lands.

## 1. Where curves stand today

The facts below are what any design must build on; each is load-bearing
later in the document.

**The analytic definition exists only in the tool layer, for milliseconds.**
`CircleTool` computes a center and radius and tessellates a fixed 24-gon
(`CIRCLE_SEGMENTS = 24`, `app/src/tools/CircleTool.ts`); `ArcTool` computes
`{center, radius, startAngle, sweep}` from the chord and sagitta and
tessellates adaptively at 12 segments per quarter turn
(`app/src/tools/arcMath.ts`). Both then feed plain segments through
`sketch_add_segment` inside a `sketch_begin_curve`/`sketch_end_curve`
bracket and discard the analytic parameters. Nothing downstream can ever
recover them exactly.

**The kernel's curve slot carries identity and nothing else.** A sketch
curve chain is a `SlotMap<SketchCurveId, ()>` — the id *is* the entire
payload (`crates/kernel/src/sketch.rs`). Edges carry `curve:
Option<SketchCurveId>`, split fragments inherit it, and manifest v7
persists it as a dense integer. This buys selection/deletion of a drawn
arc as one unit, and it is the designed on-ramp: adding geometry to the
chain is an additive change, not a redesign.

**Extrusion is where identity dies.** `Sketch::profile` flattens a region
to bare positions (`Profile { plane, outer, holes }`), and
`Object::from_extrusion` emits one quad wall per profile edge with no
provenance — walls carry `(material: None, uv_frame: None)` and nothing
else (`crates/kernel/src/ops.rs:629-633`). A solid extruded from a circle
is indistinguishable from one extruded from a hand-drawn 24-gon.

**Faces are planar by type, not by tolerance.** `Face` stores exactly
`{outer_loop, inner_loops, plane, material, uv_frame}`
(`crates/kernel/src/topo.rs:71-91`); `Plane` is the only analytic surface
in the kernel, and it is *recomputed from boundary vertices* at every load
(HEW_FILE_FORMAT.md §3.3), transform (`Transform::apply_plane` refits from
mapped sample points), and merge. `Edge`, `HalfEdge`, `Vertex`, and `Loop`
carry no metadata slot at all.

**The boolean engine is a straight-segment planar arrangement.** Every
face is imprinted with seam *segments* (`seam_segments` clips the
plane/plane intersection *line* to both regions), then partitioned by
literally instantiating a `Sketch` on the face plane and calling
`add_segment` (`arrange_face`, `crates/kernel/src/boolean.rs:486-516`).
Watertightness of the result rests on one argument, stated in that
module's header: a seam is computed once and imprinted identically on both
operands, so T-junctions are consistent by construction and `POINT_MERGE`
vertex welding pairs the seam half-edges into twins. Classification is
combinatorial — an interior point per sub-face, membership decided exactly
from a covering coplanar partner or a parity ray-cast — which is what lets
coplanar contact resolve without epsilon area comparisons (ARCHITECTURE.md
§2.3).

**Rendering and export never see the kernel's faces as anything but
facets.** `tessellate` emits flat-shaded triangles (per-face normals,
vertices duplicated per face) and one line segment per unique kernel edge
— every facet seam of a cylinder draws as a hard edge. STL and glTF export
live in the app (`app/src/io/exporters/`) and serialize the *live render
buffers* (`SceneRenderer.buildExportScene()` shares geometry by reference
with the viewport); there is no export-time visit to kernel topology at
all.

**Inference has no notion of a circle.** `InferenceScene` registers
vertices, edge segments, and planar face regions per object; `SnapKind`
has no Center/Tangent/Quadrant, and the app never snaps to an arc's
center after the gesture that drew it ends.

**Push/pull already trips over facets.** The neighbor classifier accepts
only transverse or coplanar neighbors and refuses anything slanted with
`NonManifoldResult` (`ops.rs:753-760`) — the doc comment names "adjacent
facets of an N-gon prism" as the canonical refused case. The roadmap's
near-term "push/pull on any planar face" item is this limitation.

## 2. The fork

**(A) Full analytic B-rep.** Circular arcs as first-class edge geometry,
cylindrical/conical patches as first-class face geometry, exact
curve/plane and curve/curve intersection inside booleans. The kernel's
answer to "what is this face" becomes a surface, not a plane.

**(B) Analytic overlay on the faceted carrier.** The half-edge polyhedron
remains the substrate for booleans, watertightness, and the validator.
Analytic definitions — a circle's center/radius on the sketch, a
cylinder's axis/radius on extruded wall faces — are durable, propagated,
*validated* metadata. Everything user-visible that "true curves" promises
is derived from the metadata: smooth rendering and suppressed facet-seam
edges, center/axis/tangent inference, whole-wall selection and paint,
resolution as a draw-time (and, narrowly, export-time) choice, and true
analytic surfaces for a future STEP exporter. This is SketchUp's
ArcCurve/soft-edge model made rigorous: propagation rules and validator
invariants instead of conventions.

## 3. Path A: full analytic B-rep

### 3.1 What changes, per crate

`kernel` — every module. `Face.plane` becomes `Face.surface` (plane |
cylinder | cone); `Edge` gains curve geometry (line | circular arc); the
validator must check that twinned half-edges agree on *curve* geometry,
not just endpoints, and that a face's boundary curves lie on its surface
(the planarity check generalizes per surface type). Every consumer of
`face.plane` — the push/pull classifier, `split_face`, coplanar merge,
`Plane::from_polygon` refits after transforms, the sketch region tracer,
`geom2d` — is rewritten or specialized. The sketch itself needs arc-aware
sticky rules: arc/segment and arc/arc intersection in `add_segment`,
angle-sorted adjacency ordered by tangent direction with curvature
tie-breaks in the region tracer, circular-segment area terms in winding
tests.

`tessellate` — planar ear clipping no longer suffices; curved faces need
parameter-domain triangulation of trimmed surfaces, and the edge buffer
needs real silhouette/seam classification.

`inference` — arc edges need ray-to-circle nearest-point tests plus the
new snap kinds; this work is identical under path B, so it is not a
differentiator.

`serialize` + format — the geometry buffer's core premise ("face
generators: positions plus index loops; planes recomputed on load") is
gone. Curved faces need stored surface definitions, curved edges stored
curve definitions, and the load-time validator needs surface-aware
planarity/incidence checks. This is a new geometry buffer major version
with a permanent parallel read path for v≤3.

`wasm-api`/app — new tessellation controls, new picking (ray vs curved
face), new tool behaviors (push/pull of a cylindrical wall is a radius
offset, not a translation).

### 3.2 What it does to the boolean engine

This is where path A stops being an engineering project and becomes a
research one.

- **The curve algebra is not closed under intersection.** Plane∩cylinder
  is an ellipse — already outside an arcs-and-lines vocabulary.
  Cylinder∩cylinder is a degree-4 space curve with, in general, no
  rational parameterization. Every boolean between two curved solids
  produces edges the kernel cannot represent unless the curve types grow
  to NURBS/algebraic curves (an OpenCASCADE-scale undertaking, in a
  kernel that is forbidden dependencies) or the result is *approximated
  with polylines* — which reintroduces facets precisely where the user
  asked for exactness, and means any booleaned model degrades to path B's
  representation anyway, but now with two representations to keep
  coherent everywhere.
- **The watertightness argument breaks.** Intersection points of two
  conics are algebraic numbers of degree 4; the current guarantee — one
  seam, computed once, imprinted identically, welded at `POINT_MERGE` —
  must be re-established over curve *parameters* on two different
  surfaces' domains, where the same physical point computes through
  different conditioning on each side. Twin pairing then needs shared
  exact curve objects plus tolerance-welded endpoints; the places that
  are today exact-by-construction become tolerance negotiations.
- **The refuse-tangency position becomes untenable.** ARCHITECTURE.md §4
  deliberately refuses booleans that would resolve a pure tangency. With
  analytic cylinders, tangency is not an edge case: a cylinder lying on a
  plane touches along an exact line; filleted and concentric geometry is
  tangent by *design*. Path A must either resolve tangencies (the
  numerically fragile territory the engine was designed to stay out of)
  or refuse a large class of everyday curved-model booleans. The faceted
  carrier dodges this structurally: faceting converts tangencies into
  either clean transversal crossings or coplanar facet contact, which the
  engine already resolves exactly — verified by spike
  (`crates/kernel/tests/faceted_carrier_booleans.rs`): union, subtract,
  and intersect of overlapping 24-gon prisms, of prisms with mismatched
  facet counts (24 vs 20), coaxial coplanar stacking, and a through-hole
  drill all produce validated watertight results today.
- The arrangement substrate (`arrange_face` → `Sketch::add_segment`) and
  the interior-point/ray-cast classifiers all assume straight edges and
  planar regions; each generalizes only after the sketch tracer itself
  handles arcs.

### 3.3 Position audit, robustness, interchange, migration

Broken or reopened: booleans-as-polygon-arrangement, refuse-tangency, the
tolerance policy (new named tolerances for curve incidence, curve
parameter equality, surface distance — each a new fragility surface), the
geometry buffer design, "no dangling edges" (unchanged in spirit,
redefined in mechanism), the validator's entire planarity story.
Preserved: solids-first data model, explicit combination, kernel purity
(only if no geometry dependency is added — which is exactly what makes
the boolean problem intractable in-house).

Determinism survives in principle (f64 is deterministic) but every
iteration-order and conditioning choice in new curve code is a fresh
determinism obligation. STL/3MF/glTF export finally gets true
resolution-at-export — the one clean win A has over B — and STEP
interchange becomes natural. Existing `.hew` files load as all-planar
models; they gain nothing retroactively.

### 3.4 Staging and risk

There is no honest thin slice. Arc-aware sketches are landable alone, but
the first curved *solid* requires surface faces, curved edges, the
validator, serialization, tessellation, and at minimum plane/cylinder
booleans, simultaneously. Estimated as a rewrite of the majority of
`kernel` with a long period of a second, semi-working geometry class in
the product.

**Scariest unknown:** closure of the boolean result — representing and
welding cylinder/cylinder intersection curves watertight and
deterministically without importing a full B-rep library. No credible
in-house answer exists at this project's size, and the licensing/purity
rules preclude buying one.

## 4. Path B: analytic overlay on the faceted carrier

### 4.1 The data

Two small types, both dumb data:

- **Sketch:** the curve-chain slot gains geometry — `SketchCurve {
  center: Point3, radius: f64 }` (the sketch plane supplies the rest; arc
  extent is derivable from the member edges; a chain is a full circle iff
  it closes). Captured from the tool at `begin_curve` time, which is the
  one moment the analytic truth exists today.
- **Solid:** `Face` gains `surface: Option<SurfaceRef>` alongside
  `material`/`uv_frame`, where `SurfaceRef::Cylinder { axis_point: Point3,
  axis: Vec3 (unit), radius: f64 }`. A facet carrying it asserts "I am a
  chord facet of this infinite cylinder"; angular and axial extent are
  derived from the facet's own vertices, never stored. Cones and spheres
  are future variants; nothing in the plumbing is cylinder-specific.

Everything else derives: cap-circle centers (project attributed faces'
vertices onto the axis; the parameter extremes are the two cap centers),
soft edges (an edge whose two faces carry equal `SurfaceRef`s is an
interior seam of one logical wall), whole-wall selection/paint (flood
fill over equal attributes), smooth normals (the true cylinder normal at
each vertex), tangent/quadrant snaps, and STEP surfaces. Deriving rather
than storing keeps exactly one thing that can go stale.

### 4.2 Propagation, validation, honesty

The precedent is `uv_frame`, which already solved this problem shape:
optional per-face payload, inherited by split children
(`ops.rs:3571-3579`), carried through the boolean's `FacePoly` →
`OrientedFace` → assembly pipeline, equality-gated at coplanar-seam
dissolve, version-gated in the geometry buffer (v2). `SurfaceRef` rides
the identical rails:

- **Split/boolean sub-faces inherit.** A sub-face of a chord facet lies
  on the same chord plane and asserts the same cylinder — inheritance is
  geometrically sound, and it is what makes soft edges and center snaps
  survive a subtract (a drilled cylinder still shades smooth and still
  knows its axis).
- **Seam dissolve is unaffected.** `merge_coplanar_faces` only merges
  coplanar faces; two facets of one cylinder are never coplanar, and the
  equality gate (extended to `surface`) keeps an attributed facet from
  merging with an unattributed coplanar neighbor.
- **Transforms map or drop.** `Object::apply_transform` maps `axis_point`
  as a point, `axis` through the linear part (renormalized), `radius` by
  the uniform scale factor when the linear part is a similarity; a
  non-similarity map (non-uniform scale would make the section an
  ellipse) *drops* the attribute. Dropping metadata is not silent
  geometry repair — the carrier is untouched and correct; the model
  merely stops claiming an analytic ancestry it no longer has. This
  drop-when-not-understood rule is the overlay's single most important
  contract: **every operation either maps the attribute or removes it;
  none may leave it unexamined.**
- **The validator gets teeth (rule 2).** An attributed face must satisfy:
  `axis` unit within `NORMAL_DIRECTION`, `axis · face_normal ≈ 0` (a
  chord plane is parallel to the axis), distance(axis line, face plane) ≤
  radius, and every face vertex within `radius + POINT_MERGE` of the
  axis. Cheap, exact, and it converts "stale metadata" from a silent lie
  into a debug-build failure.

What the overlay does *not* promise, stated plainly: the material truth
is the facets. A 24-gon "Ø20mm" hole's inscribed diameter is
20·cos(π/24) ≈ 19.83 mm; booleans, volume, and STL are chord-accurate,
not circle-accurate. The overlay makes the model *know* the intended
circle (for snapping, readouts, export of analytic surfaces) while the
solid remains what the facets say. Two consequences follow. First,
draw-time resolution matters more than export-time resolution: the
segment constants become user-visible quality settings with
radius-adaptive defaults (chord-sagitta tolerance), which needs no kernel
change at all. Second, export-time re-faceting is legitimate only for a
wall whose *entire boundary* is still analytic (its caps and, for arcs,
straight side seams); a wall crossed by a boolean seam is frozen at
authored resolution, because refining it would tear the polyline seam its
neighbor was welded to. Re-faceting is therefore a narrow, late,
optional stage — not the load-bearing story.

### 4.3 What changes, per crate

- `kernel`: `SketchCurve` payload + capture API; `Profile` carries
  per-boundary-edge curve attribution from `Sketch::profile`;
  `from_extrusion` stamps wall `face_specs` with `SurfaceRef`;
  propagation in split/boolean/transform per §4.2; validator extension;
  serialization (below). No new dependencies, no new tolerance constants
  beyond reuse of existing ones.
- `serialize` + HEW_FILE_FORMAT.md: manifest v10 adds
  `sketches[].curves: [{id, center, radius}]` (the id space the v7
  `edges[].curve` field already references — currently identity-only);
  geometry buffer v4 adds a per-face optional surface payload,
  version-gated exactly as `uv_frame` was at v2. Both additive: a v10/v4
  reader loads every older file with the documented absent-field
  defaults (no metadata); older readers refuse newer files per the
  standing versioning policy. No migration step exists or is needed —
  pre-existing models simply keep faceted behavior forever, which is
  what they actually contain.
- `inference`: new `SnapKind::Center` (and later Tangent/Quadrant)
  slotted into the priority order; center candidates derived at
  registration time from attributed faces. Mechanically identical to how
  guides snap today.
- `tessellate`: unchanged for correctness; later stage splits
  `edge_positions` into hard and soft buffers and emits true-normal
  smooth shading for attributed faces. Render-only, behind the existing
  `RenderMesh` contract (plus regenerated visual goldens).
- `wasm-api`: `sketch_begin_curve` grows an analytic variant; snap kinds
  and (later) soft-edge buffers cross the boundary; recording/replay
  records the new bracket parameters.
- app: tools pass the center/radius they already compute; snap cue for
  Center; later, edge-layer split and resolution settings. Exporters are
  untouched until the optional re-faceting stage.

### 4.4 Position audit, robustness, interchange

Every ARCHITECTURE.md §4 position survives verbatim: booleans stay a
polygon arrangement with exact coplanar coverage and tangency refusal
(the spike shows the carrier already covers the curved-solid cases);
watertightness semantics untouched; no silent repair (metadata drops are
explicit contract, geometry is never adjusted); tolerance policy
untouched (the validator checks reuse `POINT_MERGE`/`NORMAL_DIRECTION`);
determinism holds because attributes are computed once from profile data
and serialized in face order. Undo needs one mechanism beyond riding the
existing clone-mutate-validate path: an op whose inverse *re-creates* a
face (undoing a merge) must not re-derive that face's attributes from
whatever survives — that would resurrect a claim the dissolved face had
legitimately dropped, or lose paint it carried. The merge reports
therefore snapshot each dissolved face's attribute state (pinned to an
interior point for the outer pair, so restoration matches fragments
geometrically), and the inverse split ops re-apply the snapshot instead
of inheriting. The snapshot is best-effort per face: a face too thin to
pin an interior point in (sub-nanometer slivers, constructible only from
imported polygon soup) merges exactly as it always has and falls back to
inheriting on undo — attribute fidelity degrades before forward behavior
ever does. STL/3MF/glTF: unchanged by default,
optionally better (re-facet pristine walls at export; emit true smooth
normals to glTF). STEP (future sidecar): pristine attributed walls export
as genuine cylindrical faces instead of prisms — the overlay is exactly
the data the converter needs.

### 4.5 Staging (each stage independently landable)

1. **Sketch curve geometry** — `SketchCurve {center, radius}`, capture
   API through wasm-api + tools + recording, manifest v10. Value shipped:
   exact radius/center readouts, groundwork for everything else.
2. **Carry-through extrusion** — `Profile` attribution,
   `from_extrusion` stamping, propagation rules, validator extension,
   geometry buffer v4. Value: solids know their cylinders; persists.
3. **Center/axis inference** — `SnapKind::Center` derived from the
   metadata, wasm + app cue. First user-facing "true curve" behavior.
4. **Soft edges + smooth shading** — tessellate emits hard/soft edge
   buffers and true normals for attributed faces; goldens regenerate.
5. **Draw-time resolution** — user-settable, radius-adaptive segment
   counts (app-only).
6. **Whole-wall paint/select; tangent/quadrant snaps; optional export
   re-faceting of pristine walls; STEP surface export via the sidecar.**

Stages 1–3 are required by *both* paths (path A also needs the analytic
definition captured at the sketch and carried to the solid, and the
inference work is identical), which makes them no-regret regardless of
any future revisiting of this decision.

### 4.6 Risks, the push/pull contract, and the scariest unknown

- **Stale metadata** is the structural risk; the validator invariant and
  the map-or-drop contract are the mitigations, and both land in stage 2
  with the data itself.

**The `SurfaceRef` contract for every push/pull mode** (normative — this
text is what any concurrent or future push/pull generalization reconciles
against at merge time; a branch that generalized push/pull without
`SurfaceRef` knowledge adopts these clauses verbatim, they are exactly the
map-or-drop rule specialized per mode):

1. **Translate mode** (unattributed face, transverse neighbors): the moved
   face's own claim — it has none by hypothesis — is moot; **stretched
   transverse walls keep their references** (their planes are unchanged
   and they still lie on the same infinite cylinder; axial extent is
   derived, so nothing goes stale). Implemented, spec-covered.
2. **Coplanar-sibling and slanted-neighbor walling** (a neighbor stays put
   and a bridge wall is built along the shared edge): the unmoved neighbor
   keeps its reference untouched (nothing about it moved); **freshly built
   bridge walls carry `None`** (they are new planes with no analytic
   ancestry); the moved face, if it somehow carries a claim, drops it (it
   leaves its chord plane). A generalized-push/pull implementation needs
   no `SurfaceRef` reasoning beyond these three clauses.
3. **Whole-wall radial offset** (attributed face — this mode *preempts*
   all others; implemented on this branch): `push_pull` on a face carrying
   `SurfaceRef::Cylinder` acts on the **logical wall**, never the facet.
   Every face of the object claiming the same cylinder (tolerance-equal
   axis line + radius — `SurfaceRef::same_surface`, the one shared
   grouping predicate) moves under the radial affine map `p ↦ foot(p) +
   (p − foot(p))·r′/r`; their references **map** to radius `r′` (never
   dropped — the wall still is exactly that cylinder). Neighbors sharing
   vertices follow: they translate (a D-profile's chord wall), stretch
   in-plane (caps ⊥ axis, radial pie-slice walls), or pivot (a prism wall
   tangent to a rounded corner — two parallel vertical edges always span
   a plane). A neighbor that would *bend* off any single plane (stepped or
   bossed walls pinned elsewhere) refuses typed
   (`WallNeighborNonPlanar`); loops that would cross or orphan a hole
   refuse (`NonManifoldResult`); `r′ ≤ POINT_MERGE` refuses
   (`RadiusVanishes`). The offset additionally carries interpenetration
   and engulfment guards (`NonManifoldResult`): every reshaped face is
   tested against every other face of the object for contact away from
   their legitimately shared elements, and any vertex of a non-moving
   shell whose point-in-solid classification against the moving shells
   changes between the old and new positions refuses (a wall must not
   grow into — or cleanly past — geometry it shares nothing with). These
   guards deliberately mirror the stretch-mode guards on the
   generalized-push/pull branch (`faces_improperly_contact`,
   boundary-poke probing with per-interval coplanar cuts, a swept-region
   test); at integration the two families unify into one shared set.
   Translate-mode push/pull's inherited lack of the same guards is a
   known pre-existing gap on main — tracked with the generalized
   push/pull effort, NOT covered by this branch (see ROADMAP).
   Attributed faces never route to push-through
   (`push_pull_overshoots` is defined `false` for them). The picked
   facet's outward normal fixes the sign: pulling an outer wall outward
   grows the radius, pulling a hole wall toward its axis shrinks the hole
   — both are "distance along the normal", so the tool needs no
   special-casing.
4. **`extrude_sub_face` / `collapse_sub_face`** (boss/recess on a face):
   pulling a flat imprinted sub-face UP (boss) or DOWN (recess) sweeps the
   walls of a cylinder, so `extrude_sub_face` **stamps** each raised wall
   `SurfaceRef::Cylinder` — the pull-up mirror of `from_extrusion`'s
   `wall_surface` and of the push-through tunnel stamping (§C3). Both signs
   stamp: a recess is a resizable cylindrical pocket exactly as a boss is a
   resizable cylinder. The stamp is gated on the sub-face boundary being a
   genuine **full circle** — every boundary edge a chord of one imprinted
   circle (`Edge::curve`, consistent center/radius) **and** the ring passing
   a geometric test in two independent parts: the vertices wind once around
   the center in near-uniform angular steps (heterogeneity gate), AND the
   ring has at least `MIN_CIRCLE_SEGMENTS` (24, the draw tools'
   segments-per-turn floor, §6) facets (absolute density gate). Both are
   load-bearing. The heterogeneity gate rejects an arc closed by a straight
   chord (every edge can carry the same claim, but the long closing chord is
   a flat wall). The density gate rejects a homogeneous COARSE ring — an
   equilateral triangle or a skip-connected 12-gon of concyclic points whose
   steps are uniform yet far too large to be facets — which the relative
   uniformity test cannot see, since a regular n-gon's steps are all exactly
   2π/n for any n. Stamping either sweeps a secant into a "cylinder wall": a
   map-or-drop soundness break (stamp-wrong is worse than don't-stamp). A
   rectangle, a mixed loop, or a split fragment stamps nothing. The raised sub-face itself **drops** its
   inherited claim (it leaves the chord plane); the parent keeps its own;
   collapse does **not** restore the claim (only undo's snapshot path may,
   see §4.4). Because the boss/recess wall then carries a surface, a later
   push of it routes to whole-wall offset (clause 3), whose obstruction
   guards are `GuardMode`-gated and skipped on replay (rule 9) — the boss
   put those guards on a History op's exact-inverse path for the first time.
   Implemented, spec-covered.
5. **Push-through** (inward overshoot → boolean subtract): boolean
   inheritance rules apply — arrangement fragments of attributed faces
   inherit, cutter-derived faces carry `None`, and the coplanar-dissolve
   equality gate keeps claims from bleeding across seams. Implemented,
   spec-covered.

- **The generalized push/pull** (roadmap near-term: slanted neighbors)
  lands against clause 2 above; whole-wall offset (clause 3) must keep
  routing first — the two features compose by precedence, not by merging
  their mechanisms.
- **Scariest unknown:** whether facet-level accuracy is *acceptable* to
  the target audience once the app renders curves smoothly — smooth
  shading hides the facets that STL then faithfully prints. Path B's
  answer is draw-time adaptive resolution plus honest readouts, but if
  print-accuracy expectations harden into "the mesh must be exact", no
  overlay can satisfy them — and neither can path A short of NURBS-complete
  booleans. This is a product-expectation risk, not an architectural one,
  and it is the same risk SketchUp has lived with for twenty years.

## 5. Recommendation

**Path B.** The reasoning, compressed:

1. Path A's boolean problem has no in-house-sized solution. The curve
   algebra is not closed under intersection (cylinder/cylinder quartics),
   the exact-seam watertightness argument does not transfer, and the
   refuse-tangency position becomes untenable — while the kernel's
   dependency and licensing walls rule out adopting OpenCASCADE-class
   machinery in-process. Any partial path A degrades booleaned curves to
   facets anyway, converging on B's representation with double the
   representational surface area.
2. Path B preserves every design position the codebase is built on, and
   the carrier is already proven on curved-solid workloads (the spike
   suite passes today, including mismatched facet counts and coplanar
   stacking).
3. Path B's cost concentrates in small, staged, individually shippable
   changes riding an existing precedent (`uv_frame`), each delivering
   user value (center snapping, smooth rendering, honest radius
   readouts) within one stage of landing.
4. Path B is a prefix of path A, not an alternative to it: the overlay
   records precisely the analytic truth a real B-rep would need, in the
   file format, from day one. If the project ever grows the resources for
   analytic booleans (or an out-of-process OpenCASCADE evaluator makes
   them cheap), the migration starts from models that already know their
   cylinders — nothing done here is thrown away.

The immediate groundwork is stages 1–3 of §4.5, which are also the
no-regret set under path A.

## 6. Resolution policy — the facet-count decision (ratified)

With the overlay complete (smooth shading, export re-faceting, whole-wall
push/pull), two open questions from §4.2 are settled here.

**Inscribed stays.** The circle tool keeps generating polygons whose
vertices lie ON the analytic circle. Circumscribed polygons (flats at the
drawn radius, vertices at `r·sec(π/N)`) were considered for material
truth — a circumscribed "Ø20" hole's flats measure 20 mm — and rejected:
vertices-on-the-cylinder is the overlay's load-bearing invariant. The
validator holds every attributed vertex within `radius + POINT_MERGE`;
band detection for export re-faceting requires it; the whole-wall radial
offset maps it exactly; endpoint and intersection snaps land on the true
circle. Circumscribing would force a parallel `r_outer` notion through
all of them for a benefit export re-faceting already delivers exactly
(the re-sampled mesh converges to the true circle; a re-faceted Ø20 hole
IS Ø20 at any resolution the printer can see). What "Ø20" means: the
analytic claim is exactly Ø20; the stored carrier is chord-accurate from
inside; export restores material truth wherever the wall is still
analytic. For a print-critical bore that a boolean has frozen at stored
facets, the honest answers are draw finer or re-cut — not a global
circumscribe that breaks the kernel's invariants.

**24 becomes the floor, adaptive by radius up to 96.** The default
segment count stays 24 for small curves and now grows with radius —
draw-time resolution is the floor booleans freeze walls at, so it is the
one number that still matters after the overlay. The rule: segments per
full turn `N(r) = clamp(24, 96, ceil4(π / acos(1 − s/r)))` with a fixed
chord-sagitta budget `s = 0.5 mm` — the coarsest faceting whose maximum
chord deviation stays at half a millimeter, about an FDM printer's
practical tolerance and below silhouette visibility at arm's length.
Consequences: a Ø40 mm knob keeps 24 facets (sagitta 0.17 mm — raising
it would only bloat booleans and files); a 0.5 m radius arch gets 72; a
1 m radius wall gets 96 (sagitta 0.54 mm) instead of 24 (8.6 mm — a
visibly polygonal silhouette smooth shading cannot hide). `ceil4` rounds
up to a multiple of 4 so drawn circles always carry vertices at the
quadrant angles (see `SnapKind::Quadrant`). The cap at 96 is a
perf/file-size judgment: beyond it the silhouette argument fades and
export re-faceting supplies accuracy on demand. Arcs use the same
per-turn density scaled by their sweep (minimum 2 segments), replacing
the old fixed 12-per-quarter-turn, so an arc and a circle of equal
radius facet at equal density. Raising the internal default globally
(e.g. to 96 everywhere) was rejected: it costs every model on boolean
arrangement time, undo clones, tessellation, and file size, while
buying accuracy only in the boolean-frozen cases the sagitta budget
already bounds to half a millimeter.

## Appendix: Implementation status

Maintained in every commit that changes it. If you are picking this work
up cold: read §4 first, then this appendix, then the cited commits.

### Done

- **Stage 1 — sketch curve geometry** (`dcf6309`, load-hardening
  `044dc70`): `CurveGeom {center, radius}` on curve chains, captured via
  `Sketch::begin_curve_with`, manifest v10, map-or-drop under sketch
  edits, wasm `sketch_begin_curve_with`/`sketch_curve_geom`, recorded as
  an additive replay call.
- **Stage 2 — carry-through extrusion** (`85ad326`, gate/inheritance
  tests `9e9461e`): `SurfaceRef::Cylinder` stamped on extruded walls,
  geometry buffer v4, propagation through split/boolean/transform,
  validator teeth (`FaceSurfaceMismatch`).
- **Stage 3 — center inference** (`cef9e12`): `SnapKind::Center` from
  `analytic_cap_centers()`, occlusion/instance rules, app cue.
- **Undo attribute snapshots** (`5edbf41`, best-effort fix `7bc72fa`):
  merge reports snapshot dissolved faces' attrs; undo restores instead of
  re-deriving.
- **Whole-wall push/pull** (`f461c84`): §4.6 clause 3. `push_pull` on
  an attributed facet radially offsets the whole logical wall;
  `SurfaceRef::same_surface` is the shared grouping predicate (in
  `topo.rs`, also used by cap centers); typed refusals
  `RadiusVanishes`/`WallNeighborNonPlanar`; `push_pull_overshoots` is
  false for attributed faces, so the wasm layer never routes them to
  push-through. Specs: `surface_ref_specs.rs` §whole-wall (incl. proptest
  round-trip, history undo/redo, hole-wall sign, pivot semantics, bend
  refusal). No wasm/app changes were needed — the existing `push_pull`
  path routes.
- **Export re-faceting** (this commit; stage 6, the headline):
  `tessellate::export_triangles(object, segments_per_turn)`
  (`crates/tessellate/src/refacet.rs`) re-samples every pristine cylinder
  *band* (connected chain of chord quads between two caps ⊥ axis, every
  vertex on the claimed cylinder) at the requested resolution, 0 =
  stored facets. Watertight at ANY resolution by construction: each new
  rim point is computed once per station and shared bitwise between the
  wall quads and every cap polygon bordering them; rim vertices with any
  incidence outside the band ("anchors" — band ends, drawn cap edges)
  are preserved bit-exact and resampling subdivides between them. Bands
  that fail the legitimacy condition (boolean seams, bosses, slanted
  caps, stacked shared rims) demote honestly to stored facets, as does a
  band whose refined rim would cross other cap geometry (2-D
  simplicity/disjointness re-check with a demotion fixpoint). wasm:
  `Scene::object_export_triangles(id, segments)` (f32 soup). App: the
  STL exporter is now kernel-sourced (`collectKernelTriangles` walks
  objects + posed instances; three.js is out of the STL path), and the
  Export dialog gained a per-STL "Curve resolution" select (As modeled /
  24 / 48 / 96 / 192; default 48). Specs:
  `crates/tessellate/tests/refacet_specs.rs` (manifold-at-any-resolution
  oracle over bitwise directed edges + exact inscribed-prism volume
  assertions + honest-fallback equality + anchor preservation +
  demotion-on-collision), app unit tests for the collector/dialog, and a
  real-browser end-to-end run (draw circle → extrude → wall push/pull →
  File ▸ Export ▸ STL ▸ Fine → downloaded bytes contain exactly the
  re-faceted triangle count).

- **Smooth rendering** (this commit; stage 4): attributed faces shade
  with true per-vertex cylinder normals (radial off the axis, sign from
  the chord plane's side — outer walls outward, hole walls inward;
  adjacent facets share bitwise-identical seam normals). `RenderMesh`
  splits edges into `edge_positions` (hard) and `soft_edge_positions`
  (interior seams between faces claiming `same_surface`); the viewport
  renders hard edges only, so cylinders read as one smooth wall with
  crisp cap rims. Softness is derived at tessellation time — NO format
  bump. wasm `MeshJs::soft_edge_positions` exposes the suppressed buffer.
  Specs in `refacet_specs.rs` (normals radial/inward/unit, seam-normal
  bitwise sharing, soft/hard counts for cylinder and D-profile, the
  seam-dissolve-gate profile staying hard, plain boxes byte-unchanged —
  originally asserted via derived properties only; strengthened to a
  literal byte-for-byte fixture comparison in review follow-up F4
  below).
  Visual goldens contain no curved solids (box/materials/guides only)
  and box output is asserted unchanged, so no golden regen is owed; a
  curved-solid golden can only be authored on the pinned Linux runner
  (`.github/workflows/regen-visual-goldens.yml`) — worth adding
  post-merge. Deliberate scope cut: no view-dependent silhouette
  ("profile") strokes for the smooth wall — the shading contour and hard
  rims carry the silhouette. NOTE: as originally landed this stage
  computed the normals but the viewport threw them away (`flatShading:
  true` on every mesh material makes three.js ignore the `normal`
  attribute); the eyeball check above passed against a flat-shaded
  cylinder. Fixed in the review follow-ups below, with a programmatic
  luminance-gradient verification replacing the eyeball.

- **Facet-count decision + draw-time adaptive resolution** (this commit;
  stage 5): decision ratified and reasoned in §6 — inscribed polygons
  stay (vertices-on-cylinder is the overlay's invariant), 24 becomes the
  floor of an adaptive rule `clamp(24, 96, ceil4(π/acos(1 − s/r)))` with
  a 0.5 mm sagitta budget (`segmentsPerTurn` in `app/src/tools/arcMath.ts`),
  rounded to multiples of 4 for quadrant vertices. `CircleTool` (ground +
  face modes, previews and commits) and `ArcTool` (via `arcSegmentCount`,
  now sweep × per-turn density, replacing 12-per-quarter) adapt; the test
  harness keeps a fixed default 24 for determinism. App-only — no kernel
  change.

- **Inference completion** (this commit): `Object::analytic_rims()`
  (topo.rs) generalizes the cap-centers query — per claimed cylinder, the
  two rim circles (exact center/axis/radius at the axial extremes, a
  deterministic `geom2d::plane_axes` angular basis) plus merged angular
  coverage from the claiming facets' vertex angles; `analytic_cap_centers`
  now derives from it. `SnapKind::Quadrant` (ranked just under Center):
  the covered cardinal points of each rim, registered like centers
  (transform as points under any pose). `SnapKind::Tangent` (between
  Intersection and OnEdge): rims register in the inference scene (mapped
  under similarity placements, DROPPED under non-similarity poses — an
  ellipse is not representable, map-or-drop at the query layer) and
  tangent candidates are computed per query from `SnapQuery::anchor`
  (the two rim points where the anchor segment touches the exact circle,
  coverage-gated, anchor strictly outside the circle). Same occlusion and
  removal rules as Center; linear walk off the spatial index so indexed
  and reference resolves agree. App: kind strings/colors/tooltips/
  hysteresis plumbed ("quadrant" shares Center's teal; "tangent" gets its
  own hue); **gap found and fixed**: `LineTool.snapConstraint` only
  passed its anchor when an axis lock was active, so anchor-dependent
  candidates could never fire — it now always passes the anchor once a
  chain is anchored. Arcs end-to-end verified: partial-arc coverage specs
  at kernel (rim coverage), tessellate (band re-facet + soft edges), and
  inference (uncovered quadrant/tangent never offered) layers, plus a
  real-mouse browser run resolving Quadrant and Tangent tooltips through
  the actual pointer pipeline.

- **Recording format posture** (this commit): RATIFIED as keep-additive
  at v2. New `method` variants (like `sketch_begin_curve_with`) do not
  bump the version — old recordings replay unchanged on new builds, and
  a recording using a new method fails loudly (typed parse error) on old
  builds, never silently divergent. v3 is reserved for changes an old
  reader would MISinterpret (renamed/re-typed fields); none is planned
  and no fixtures needed regeneration. Documented normatively in
  docs/DIAGNOSTICS.md, whose method table now also lists the
  gesture/curve bracket calls it had fallen behind on.
- **Docs** (this commit): ROADMAP moves the true-curves capabilities
  into Shipped and marks Follow Me / Offset as unblocked; user guide
  updates in site/src/content/learn (core-concepts snap table + drawing
  resolution/snaps + push/pull curved-wall section + import-export STL
  curve resolution) with the export-dialog screenshot regenerated to
  show the STL curve-resolution select (only that screenshot changed).

### Review follow-ups (adversarial review of f461c84..66e4986)

Four findings, all empirically confirmed; fixed in order of severity.

- **F3 [critical, FIXED] — smooth shading never rendered.** The
  tessellator emitted true per-vertex cylinder normals but every
  viewport mesh material set `flatShading: true`, which makes three.js
  ignore the `normal` attribute and derive flat normals in the shader.
  Fix: the flag is dropped globally in `SceneRenderer._buildMaterialArray`
  (all three material sites — default, textured/palette, fallback; the
  object, materialized-instance, and instanced-batch paths all build
  through it). Global removal is correct because the tessellator
  duplicates vertices per face and writes the exact face-plane normal at
  every corner of an unattributed face — planar faces shade flat from
  the attribute alone, byte-identically to before (verified: box face
  luminance levels unchanged pre/post fix). Verified programmatically
  with a throwaway Playwright driver (dev server, draw circle → extrude
  + control box, screenshot, luminance samples at 1° steps across the
  curved wall): pre-fix showed 15°-wide plateaus with adjacent-sample
  steps of 9 luminance units (hard facet seams); post-fix max adjacent
  step 1.0 over a 30-unit gradient, and the box's two visible faces
  stayed flat (per-face range 0.0) at their exact pre-fix levels.

- **F1 [major, FIXED] — rim coverage merged across both rims.**
  `Object::analytic_rims()` pooled every claiming facet's angular
  interval into one per-surface list, so a notch cut into the TOP rim
  kept claiming coverage there (the intact bottom rim masked it) and
  Quadrant/Tangent/Center-adjacent snaps were offered on rim arcs that
  no longer exist. Fix: coverage is now computed per rim — only
  outer-loop boundary edges whose both endpoints lie at that rim's
  axial station (within `POINT_MERGE` along the axis) contribute their
  angular span; a straight edge with both ends at the station lies
  wholly at it, so the station filter is exact for the polyhedral
  carrier. Spec: `notch_cut_into_one_rim_uncovers_only_that_rim`
  (red-checked against the merged behavior — it failed exactly at the
  masked +X quadrant on the notched top rim): a box notch into the top
  rim refuses the notched arc's quadrant on top while the bottom rim
  keeps full coverage and all four quadrants.

- **F2 [major, FIXED] — whole-wall radial grow had no interpenetration
  check.** `offset_cylinder_wall` re-validated only faces sharing a
  moved vertex; the confirmed repro — a two-shell Object (24-gon
  cylinder r=1 unioned with a disjoint unit box at x∈[1.5,2.5]),
  `push_pull(wall, 2.0)` — returned Ok, validated, and reported
  Watertight with the box entombed inside the grown cylinder. Fix, per
  §4.6 clause 3's amended text: (1) an all-pairs improper-contact sweep
  — every reshaped face against every face of the object, contact away
  from shared vertices/twin edges refuses, with transversal crossings
  tested at the exact plane-crossing point and coplanar overlap probed
  per boundary-cut interval; (2) an engulfment test — any vertex of a
  non-moving shell whose parity-ray-cast classification against the
  moving shells' face set differs between the pre- and post-offset
  geometry refuses. The completeness argument for vertex-based
  engulfment (stated in full at the guard): the swept volume is bounded
  by the moving shells' old and new boundaries; crossing the old one
  implies improper contact in the *input* (excluded by validity),
  crossing the new one is face/face surface intersection, whose
  transversal segments always cross a boundary edge (caught by the
  sweep), so a shell the guards don't touch is entirely inside and every
  vertex flips classification. The parity formulation (not a radial
  annulus test) is what makes this exact for translating and pivoting
  neighbor walls too, whose swept slabs/wedges lie partly below the
  claimed radius. Guards mirror the stretch-mode guards on the
  pushpull-any-face branch for clean unification. Specs:
  `wall_grow_that_would_engulf_a_disjoint_shell_refuses` and
  `wall_grow_into_another_shell_refuses` (both red-checked — they
  returned Ok before the guards) plus the
  `wall_grow_with_clearance_still_succeeds` control; every pre-existing
  whole-wall spec passes unchanged. Scope note: translate-mode
  push/pull's identical gap predates this branch and is tracked with
  the generalized-push/pull effort (ROADMAP), not here.

- **F4 [minor, FIXED] — "boxes byte-unchanged" overclaimed what the spec
  asserted.** The cited spec
  (`unattributed_objects_are_unchanged_by_the_edge_split`) checked
  derived properties (soft buffer empty, edge count, flat normals), not
  bytes. Strengthened to the literal claim: the spec now serializes
  every tessellation buffer (positions, normals, colors, uvs, edge
  positions, indices, group ranges) to exact bit patterns and compares
  byte-for-byte against
  `crates/tessellate/tests/golden/unattributed_box_mesh.golden`, a
  fixture generated by running the SAME box through the tessellator at
  commit 0d213cd — the last commit before smooth shading landed. The
  comparison passes: box output really is byte-identical across the
  smooth-shading change. The box uses only exact dyadic coordinates and
  axis-aligned planes, so the fixture is machine-stable; a deliberate
  future tessellation change regenerates it (dump the new output with
  the spec's own serializer) and updates this entry.

### Review follow-ups, round 2 (adversarial review of the fixes above)

- **R2-F1 [major, FIXED] — smooth shading rule for instanced batches.**
  Dropping `flatShading` globally (F3) exposed the instanced-batch
  path's normal transforms: the InstancedMesh shader multiplies normals
  by `mat3(instanceMatrix)` with a squared-column-norm compensation
  that is only a true inverse-transpose for orthogonal-column matrices
  (three.js's chunk says outright "shear transforms in the instance
  matrix are not supported"), so non-similarity poses shaded
  smooth-but-WRONG. Worse, the review's mirror carve-out ("mirrors are
  their own inverse-transpose") is itself unsafe in the batch path:
  BackSide materials define FLIP_SIDED, which negates the transformed
  normal in the vertex shader — a mirrored batch with attribute normals
  lights inside-out (verified empirically: uniform ambient-only walls,
  zero diffuse variation). The 'D' (DoubleSide) bucket mixes mirrored
  and unmirrored slots under DOUBLE_SIDED's camera-facing flip, which
  equally disagrees with attribute normals on the mirrored ones.
  **The rule (normative):** in the batch path, only the 'F' bucket —
  watertight, non-mirrored, similarity pose — renders the tessellator's
  per-vertex analytic normals; a new 'G' bucket (watertight,
  non-mirrored, NON-similarity) plus 'B' and 'D' use `flatShading`
  (screen-space derivative normals, geometrically exact for any pose):
  honest degradation, faceted-but-right over smooth-but-wrong, the
  map-or-drop posture applied to rendering (tangent snaps already drop
  under non-similarity poses for the same reason). Similarity is
  detected per pose (orthogonal columns, equal norms, relative 1e-6)
  and re-bucketing rides the existing reflected-flip rebuild path.
  Standalone objects and materialized instances keep smooth shading
  under EVERY pose — their normals go through the CPU-side
  `normalMatrix`, a true inverse-transpose. Verified programmatically
  (throwaway Playwright driver, deleted after): a cylinder component
  with an identity placement, a mirrored placement, and a
  scale-after-rotation (shear) placement, all batched; luminance
  sampled at 1° of surface parameter against the analytic light rig
  (one directional light + ambient). Post-fix: identity wall smooth
  (max adjacent step 1.0), mirror and shear walls flat (steps 6.0/7.0)
  with measured luminance correlating 0.99/1.00 against the predicted
  diffuse term from TRUE inverse-transpose facet normals, and the
  mirror lit on the same world side as the control. Pre-fix (red): the
  mirror rendered ambient-only (correlation 0.00 — the FLIP_SIDED
  inversion) and the shear rendered smooth (no facet steps) with its
  bright lobe displaced off the true-normal prediction.

- **R2-F3 [major, FIXED] — Center snaps ignored per-rim coverage.** The
  F1 fix gated quadrants (and tangents were coverage-gated already),
  but `analytic_cap_centers()` and the inference registration pushed
  every rim's center unconditionally: slant-cut the whole top off a
  cylinder and the top rim correctly computes `Some(vec![])` coverage
  — zero surviving arc — yet a live Center floated at
  `axis_point + axis·t_max` (the station of the single highest
  surviving vertex, the center of no circle). Fix:
  `AnalyticRim::has_coverage()` (false exactly for empty coverage)
  gates the kernel's `analytic_cap_centers` — the source of truth —
  and the inference registration skips a vacant rim entirely (center,
  quadrants, and tangent rims alike). Decision, stated in
  `analytic_rims`' contract: vacant rims are still REPORTED by the raw
  query — the two-rims-per-group shape holds for indexing consumers
  and the station is the wall's true axial extreme, which is real
  information about the surviving geometry — but every candidate
  derived from the rim *circle* gates on `has_coverage`. Specs
  (red-checked; both failed at the phantom center before the gate):
  kernel `slant_cut_rim_offers_no_center` (exactly one cap center, the
  bottom's) and inference `slant_cut_rim_offers_no_phantom_center`
  (aiming straight at the fabricated point resolves no Center there;
  the intact bottom rim's center still snaps from below).

- **R2-F2 [minor, FIXED] — byte-golden failure was an unlocalized byte
  dump.** The F4 spec's `assert_eq!` on the whole byte vector printed
  two kilobyte arrays on divergence. The comparison now walks the
  fixture's self-describing sections and, on mismatch, panics naming
  the diverging buffer, the element index, and both 4-byte bit
  patterns (raw and as f32), with the whole-stream equality kept as a
  backstop. Red-checked by flipping one byte inside the fixture's
  normals section: the failure reads "section 'N' (normals) diverges
  … at element 10: got … want …" (fixture restored after).

### Playtest fixes

Maintainer playtest findings resolved after the branch's original scope.
Each status-changing commit updates this section; a successor resuming cold
reads it after §4.

- **Boss/recess wall stamping — the pull-UP mirror of C3 (§4.6 clause 4).**
  Drawing a circle on a solid cap and pulling that disk UP into a boss (or
  DOWN into a recess) rendered FACETED and a push of a boss wall hit the
  flat translate-and-build path (one facet) instead of the whole-wall
  radial offset — `Object::extrude_sub_face` never stamped its raised walls,
  the exact mirror of the fixed push-through hole case. Fix:
  `extrude_sub_face` now stamps each raised wall `SurfaceRef::Cylinder`
  (both signs — a recess is a resizable cylindrical pocket) when the
  sub-face boundary is a genuine **full circle**: every boundary edge a
  chord of one imprinted `Edge::curve` (consistent center/radius) AND the
  ring passing two independent geometric gates — the vertices wind once
  around the center in near-uniform angular steps (heterogeneity, rejecting
  an arc closed by a straight chord) AND the ring has at least
  `MIN_CIRCLE_SEGMENTS` (24) facets (absolute density, rejecting a coarse
  uniform ring — a triangle or a skip-connected 12-gon — that the relative
  uniformity test alone lets through, since a regular n-gon's steps are all
  exactly 2π/n). Both gates matter — stamping a secant into a "cylinder
  wall" is a map-or-drop soundness break. `collapse_sub_face` leaves no
  stale surface (the walls it removes carry it). Specs
  (`surface_ref_specs.rs`, red-checked): a 24-gon and a finer 48-gon boss
  stamp every wall; a wall push offsets the whole radius and round-trips;
  shrinking past the radius refuses `RadiusVanishes`; rectangle,
  arc-plus-chord, equilateral-triangle, and skip-12 bosses stay flat; a
  bossed-wall offset survives a full History unwind/replay.

  Two adversarial-review follow-ups landed with it. **Rule-9 replay
  soundness (two parts):** (a) `Face::surface` decides push/pull routing but
  was not restored by `StateProof::verify_and_align`, so a re-created boss
  came back a bare facet and the next offset rerouted and diverged — the
  proof now carries and restores `surface`. (b) `offset_cylinder_wall` ran
  its interpenetration + engulfment obstruction sweeps unconditionally, so
  `push_pull_replay` refused the exact inverse of a push the forward path
  accepted ("push/pull sweep has no manifold result"); those sweeps are now
  `GuardMode`-gated and skipped on replay, exactly like the translate-and-
  build guards and `extrude_sub_face`'s obstruction ray. This is a
  pre-existing whole-wall-code gap the boss work merely made reachable in
  op_fuzz (a `from_extrusion` cylinder wall pushed and undone hits it too);
  the fix improves main. Distilled regression:
  `op_fuzz::recess_wall_offset_undo_is_guard_exempt_on_replay` (the harness's
  own minimization, red-checked). op_fuzz + document_fuzz green at 16k.

- **C3 — imprinted circles kept their identity onto the solid; tunnel walls
  from a through-cut now stamp `SurfaceRef::Cylinder`.** Drawing a circle on
  a solid face (imprint) and pushing that face through the solid used to
  drop the circle at TWO points: `split_face_inner` imprinted a raw 24-gon
  (solid `Edge`/`Loop` carried no curve slot), and `push_through` rebuilt its
  cutting tool via `Profile::new` from bare point loops. `from_extrusion`
  therefore had nothing to stamp — the tunnel walls were flat facets and a
  whole-wall push refused with `NonManifoldResult`.

  Fix (the on-ramp §4.1/§4.3 named): `Edge` gains `curve: Option<CurveGeom>`,
  the solid-side mirror of `SketchEdge::curve` + `CurveGeom` — set when a
  circle/arc is imprinted (`Object::split_face_inner_with_curve`, plumbed
  through `KernelOp::SplitFaceInner.curve` so undo/redo carry it), read by
  `push_through` to re-attribute the tool `Profile` per boundary edge so
  `from_extrusion` stamps the cut's tunnel walls. Hole-wall stamping was
  already correct (the tool's outer walls become the inward-facing tunnel
  walls under the subtract; `SurfaceRef` carries no inside/outside notion and
  the validator/`same_surface` checks are orientation-agnostic).

  Map-or-drop, per the overlay contract: `Edge::curve` maps under a
  similarity (center as a point, radius by the uniform scale) and DROPS under
  any non-similarity transform or across a boolean (result edges rebuild
  fresh with no claim) or an interior edge split (a chord split lands its new
  vertex inside the circle, so the fragments are no longer chords). The
  validator gets teeth (`EdgeCurveMismatch`): a present claim's endpoints lie
  within tolerance of `radius` from `center`. The caller owns the analytic
  truth — `split_face_inner_with_curve` verifies the claim against the loop
  up front and refuses a mismatch typed (`CurveClaimOffLoop`); the kernel
  never fits a circle to the facet points (rule 4).

  Deliberately NOT done — `from_extrusion` does not stamp cap EDGES with the
  profile circle. A cap whose outer loop is a circle can only be pushed
  *through* into the opposing cap of the same prism (which vanishes it); any
  reachable "push a circular face through" is the imprinted disk sub-face,
  whose edges the imprint stamps directly. Cap-edge stamping would serve no
  reachable case and carries a far-cap-center trap, so it is omitted. Undo of
  a *merge* that dissolved an imprinted circle re-imprints with `curve: None`
  (the dissolved edge's claim is not snapshotted — attribute fidelity
  degrades before forward behavior, §4.4).

  Specs (`surface_ref_specs.rs` §imprint → push-through, red-checked by
  disabling the imprint stamp AND the push_through attribution — both drop
  the tunnel to 0 attributed walls): draw-circle-on-box-top + push_through
  stamps 24 tunnel walls with the exact center/axis/radius; a subsequent
  whole-wall push offsets the hole radius and undoes exactly; shrinking past
  the radius refuses `RadiusVanishes`; similarity maps / shear drops the
  claim; a wrong claim refuses `CurveClaimOffLoop`. Validator unit test
  `validator_catches_lying_edge_curve`.

  Persistence — geometry buffer **v5** appends a per-face edge-curve block
  after each face's hole loops: a one-byte "any" flag (0 for the common face
  with no claim, so plain solids grow by one byte per face) and, when set,
  a `flag + center xyz + radius` entry per loop edge in loop order. On decode
  the claims re-attach positionally after topology rebuilds, before
  validation (a stale stored claim fails as `EdgeCurveMismatch`). So an
  imprinted circle saved BEFORE its push-through reloads with its identity
  intact. Additive and version-gated exactly as `surface` was at v4; manifest
  format version is untouched (v10). Spec: `serialize_specs.rs`
  `imprinted_circle_edge_claim_round_trips_through_geometry_buffer_v5` checks
  the claim directly AND end-to-end (the reloaded disk still drills a smooth
  24-wall tunnel); `representative.hew` golden regenerated; HEW_FILE_FORMAT.md
  updated in the same commit.

  App wiring: `Scene::split_face_inner_with_curve` (wasm) carries the drawn
  center/radius; CircleTool face mode calls it, measuring the radius to the
  loop's own first vertex so the claim matches the imprinted points. ArcTool
  and RectangleTool keep the plain imprint on purpose — their loops are mixed
  (a pie's radial edges, an arc's chord, a rectangle's sides are not on any
  one circle), and `split_face_inner_with_curve` stamps EVERY loop edge with
  the one circle, so its up-front guard would reject a center or chord vertex.
  A full circle is the only loop it accepts; anything else stays flat
  (map-or-drop, general per-edge attribution deferred). Real-browser verified
  on port 5184 (throwaway Playwright driver, deleted after): box → imprint a
  circle on the top carrying identity → push the disk through drills a
  through-hole that renders as one SMOOTH cylinder wall (screenshot: smooth
  luminance gradient across the tunnel, crisp cap rim, no hard facet steps;
  status "solid"); picking a tunnel wall and pushing it OFFSETS the hole
  radius with no error — the `NonManifoldResult` symptom is gone. The
  `__hew_test` harness gained `imprintCircleOnFace` (draw-on-face was
  previously unreachable through the harness).

- **C3 review follow-ups (adversarial review of the C3 fix).** Four confirmed
  findings, all empirically reproduced, fixed on the branch.

  - **[critical] `Edge::curve` was not map-or-dropped when an op moves a
    SUBSET of vertices.** The map-or-drop discipline applied to `Face::surface`
    was not extended to the new field. Repro (debug + release): imprint a
    circle, boss it up (`extrude_sub_face`), then `push_pull` the holed top to
    thicken the box — the translate moves the hole-ring vertices off the stored
    circle, leaving a stale claim that panicked `check_invariants` (debug) /
    false-refused `NonManifoldResult` (release) for an op unrelated to the
    circle. Audited EVERY vertex-moving kernel path: `push_pull` (its translate
    fast path, coplanar-aware path, and collapse path all share one finalize),
    `offset_cylinder_wall`, `extrude_sub_face`, `collapse_sub_face` each now
    call `Object::drop_stale_edge_curves()` before validation (the SAFE
    minimum — drop any claim its moved endpoints no longer satisfy, the exact
    inverse of the validator's edge-curve check, so anything kept passes
    validation). `apply_transform` already maps/drops per-edge for the
    whole-object case; boolean and slice REBUILD edges fresh (`curve = None`),
    so they can never carry a stale claim. Optional rigid-translate MAPPING is
    deferred — drop degrades gracefully to flat facets and is correct+safe.
    Spec `thickening_a_box_after_bossing_an_imprinted_circle_map_or_drops_the_claim`
    (red-checked: disabling the push_pull drop panics with "analytic circle
    disagrees with its geometry"). Fuzz: op_fuzz gained an `ImprintCircle` op
    (a 24-gon carrying its `CurveGeom` inside a random face), so random
    push_pull/extrude sequences now exercise the drop paths; op_fuzz +
    document_fuzz both green at 16k cases.

  - **[major] v5 decoder did not verify shared-edge claim agreement.** Each
    edge claim is stored twice (once per incident face); the decoder kept
    whichever face loaded last (order-dependent, accepted a tampered file).
    Now a VALIDATING loader: the two incident faces' claims for a shared edge
    must agree within `POINT_MERGE`, else a typed `DecodeError::Corrupt`
    ("shared edge has disagreeing analytic circle claims"). Spec
    `disagreeing_shared_edge_claims_are_rejected_on_load` (tampers one stored
    radius; red-checked: without the gate the file is accepted or caught only
    incidentally by the validator, never as the precise disagreement error).

  - **[major] draw-on-face imprint was unrecordable/unreplayable.** Neither
    `split_face_inner_with_curve` nor `push_pull` was in the diagnostics
    recording set (the whole modify family was absent), so a C3 session
    diverged on replay. Added `RecordedCall::SplitFaceInner` (carrying the
    `[cx,cy,cz,radius]` curve) and `RecordedCall::PushPull`, both recorded and
    given replay arms — additive variants at recording v2, per the ratified
    grow-additively posture (DIAGNOSTICS.md method table updated). Spec
    `draw_on_face_circle_then_through_cut_records_and_replays`: box → imprint
    circle carrying identity → push through → replay reproduces the exact
    `state_hash` and byte-identical save (red-checked: without the imprint
    recording, the recorded call stream lacks it). The remaining modify ops
    (`extrude_sub_face`, `collapse_sub_face`, `split_face`, `merge_faces`,
    `merge_inner_face`) stay unrecorded — a pre-existing broader gap, not owed
    by C3, to fill additively later.

  - **[minor] doc tolerance mismatch.** `Edge::curve`'s doc said the validator
    holds endpoints within `POINT_MERGE`, but the check (validator + imprint
    guard) uses the object's `planarity_tol` (the same gate `Face::surface`
    uses — `PLANE_DIST` native, `IMPORT_PLANE_DIST` imported). `planarity_tol`
    is correct; the doc now states it.

### Remaining, in priority order

Branch scope is COMPLETE. Deferred, explicitly not owed by this effort:
ArcTool passing its chord anchor (tangent-while-drawing-arcs), a
curved-solid visual golden authored on the pinned Linux runner, 3MF
export riding the kernel-sourced triangle path, whole-wall paint/select
(flood fill over `same_surface`), and STEP surface export via the
planned OpenCASCADE sidecar — each starts from the data and queries this
branch landed (`SurfaceRef`, `analytic_rims`, `export_triangles`).

### Gotchas for a successor (export re-faceting)

- The watertightness proof is BITWISE point sharing, not tolerance
  welding — never recompute a station point twice from its angle; store
  it once and index it. The manifold oracle in `refacet_specs.rs`
  compares `f64::to_bits`.
- Anchors keep their ORIGINAL positions (drawn trigonometry never lands
  on round coordinates — `sin(pi)` is 1.2e-16); tests must read expected
  positions from the object, not from ideal math.
- The demotion fixpoint only ever grows the demoted set, so it
  terminates; demote at BAND granularity, globally (a band is refined
  everywhere or nowhere — wall and both caps must agree).
- Down-sampling (export coarser than stored) rides the same machinery
  and is deliberately allowed.
- `emit_band` winding: outward XOR chain-angle-direction decides the
  flip; the volume assertions in the specs pin it.

### Gotchas for a successor

- The radial offset map is affine per cross-section; that is why
  fully-moved faces stay planar. Do NOT try to re-facet or re-trim
  neighbors inside push/pull — refusal is the contract for anything that
  would bend.
- Prism walls never bend (two parallel vertical edges span a plane); they
  pivot. The genuine bend case needs a face pinned at three or more
  non-collinear stations (e.g. bossed wall) — that's what the
  `bossed_neighbor_wall_refuses_the_offset_typed` spec constructs.
- `refit_face_plane` checks simplicity/hole-containment/orientation but
  NOT hole-vs-hole disjointness; the wall offset adds that check itself.
- Undo of the offset is `PushPull{face, -d}` via the existing history
  inverse — the report's `face` survives, so nothing new was needed. The
  round-trip is tolerance-exact, not bit-exact (like translate mode).
- Red-check trick: disable the `surface.is_some()` routing branch in
  `push_pull` and the whole-wall spec section must fail.

### Next action

None on this branch — the maintainer playtests. If something breaks,
start from the stage's spec file (each commit names its specs) and the
per-stage gotchas above; every stage red-checks by disabling its
feature branch/registration and watching its positive specs fail.

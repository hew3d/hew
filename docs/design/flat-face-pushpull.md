# Flat-face push/pull (translate-and-build)

Status: implemented on branch `pushpull-any-face`. This note is the design of
record and the successor handoff for that work; keep it current with every
status-changing commit.

## What this is

Push/pull on **any planar face** of a solid, whatever the angle of its
neighbours — a Slice-produced wedge's cut face, a prism side facet (the Circle
tool's own output), a face from a dissolved boolean seam. Every such face is
flat, so it follows classic SketchUp push/pull:

- The moved face translates **rigidly** by `distance` along its normal.
- Every **transverse** neighbour (perpendicular to the sweep, e.g. a box's
  side wall) simply extends — the shared ring translates in place, no wall is
  minted. A face whose boundary is entirely transverse (a box face) keeps the
  bit-identical pure-translate fast path.
- Every **non-transverse** neighbour — a coplanar `split_face` sibling **or** a
  slanted wedge/facet neighbour — has its shared edge **unwelded** and a fresh
  quad side wall erected between the old edge and the raised one. The neighbour
  keeps its shape; the solid gains a facet. Junctions where such an edge meets
  a transverse one reshape the transverse neighbour into a (still planar)
  stepped polygon.

This **replaces** the earlier in-plane "stretch" mechanism (neighbours slid
their shared vertices within their own planes to keep the moved face's boundary
on them). Stretch was playtested and rejected for flat faces: pulling a
prism's flat wall should erect a prism of material, exactly like pulling a
rectangle, not bulge the whole solid. The stretch solve (`push_pull_stretch`)
and its `STRETCH_MIN_ALIGNMENT` tolerance are gone; its **validity checks**
(interpenetration / boundary / engulfment) are kept and reused to validate the
built result (`validate_sweep_result`).

## The pull/push asymmetry

- **PULL (outward) is unbounded by neighbour angle.** Erecting a prism of
  material on a flat face is valid however oblique the neighbours — the moved
  face travels away from them and the new walls are non-degenerate — so a pull
  never refuses on account of neighbour angle, which is the point of the
  rework. It is not *unconditionally* unbounded: the kept interpenetration and
  engulfment checks still fire if the new walls would ram a distant part of the
  same non-convex solid (a genuine self-intersection), refusing typed. On a
  convex-enough solid a pull of any distance validates.
- **PUSH (inward) is bounded by validity.** The built result is checked for
  self-intersection; the push refuses typed (`NonManifoldResult`, object
  byte-identical) the moment the moved face would cross the fixed structure it
  is pushed into. For a wedge's slant face that limit is **zero** — the moved
  face immediately drives across the fixed bottom/back, so it cannot be pushed
  in at all ("in a prism case, I shouldn't be able to push at all, just pull").
  A fatter prism's facet can be pushed in until it would. Pushing far enough to
  consume the solid refuses as `WouldVanish`. The limit is **derived** from the
  guards, never hardcoded.

## Code shape (crates/kernel/src/ops.rs)

- `Object::push_pull` classifies each boundary edge `Transverse` /
  `Coplanar` / `Slanted`. `has_wall = has_coplanar || has_slanted`.
  - `is_collapse` (a direct `push_pull(-d)` that exactly closes a coplanar
    step) → `try_collapse_coplanar_step` (unchanged).
  - `has_wall` → `push_pull_build_walls` (the former `push_pull_coplanar_aware`,
    generalized so a `Slanted` edge builds a wall exactly like a `Coplanar`
    one) then `validate_sweep_result` to bound the sweep.
  - else → the pure-translate fast path (box), bit-identical to before.
- The interior-obstruction column guard and `WouldVanish` run only for the
  non-wall paths; the wall path is bounded by `validate_sweep_result` instead
  (so an outward pull is unbounded).
- `validate_sweep_result` reuses `check_face_boundary`,
  `faces_improperly_contact`, and the engulfment band test.

## Undo contract

A pure translate (box) and a pure coplanar step both still invert the classic
way: the recorded inverse is `PushPull { face, -distance }`, and the coplanar
step-close is re-detected by `find_collapse_plans` at undo time. This path is
untouched by this branch.

A push that erects a wall along a **slanted** neighbour cannot be inverted that
way — the wall is perpendicular to the moved face and bridges a **non-coplanar**
neighbour, which `find_collapse_plans` deliberately will not match. So the
exact inverse is recorded as **data**:

- `PushPullReport::requires_unbuild_inverse` is set true iff the push built a
  wall along a slanted neighbour. (It is not exposed across the WASM boundary.)
- `History::derive_inverse` then records `KernelOp::UnbuildPushPull { face,
  walls: report.created_faces, distance }` instead of `PushPull { -distance }`.
- `Object::unbuild_push_pull` removes exactly those recorded walls
  (`find_unbuild_plans`, which matches the recorded pristine-quad walls with no
  coplanar-far-face restriction) via the **shared** `collapse_plans_surgery`
  (extracted from `try_collapse_coplanar_step`, so the two share the weld /
  un-splice / remove surgery and cannot drift). It restores the moved face and
  is clone-validate-commit, so any failure leaves the object byte-identical.

In the common case undo is **LIFO**, so the walls a push recorded are still
pristine quads when that push is the one being undone. But that is not
guaranteed, so `find_unbuild_plans` performs a **complete** up-front validity
check before it will build a plan for a recorded wall: the wall must still be
in the recorded set, its outer loop a clean 4-cycle, it must carry **no inner
loops**, and this inverse sweep must close it exactly. Any deviation skips the
wall; the caller requires every recorded wall to yield a plan, so one skip is a
typed refusal (object untouched), never a partial or corrupting un-build. The
inner-loop check is load-bearing: an intervening `split_face_inner` can append
a hole loop and a sub-face to a built wall (its outer 4-cycle untouched), and
removing such a wall would orphan its hole — a debug `check_invariants` panic
and a release corruption. The check refuses that in **both** debug and release.
Spec: `op_specs.rs::unbuild_refuses_when_a_recorded_wall_gained_a_hole` (runs
under plain `cargo test`, i.e. debug, so a pass proves no panic). It does
**not** rewrite `find_collapse_plans` or the guard machinery, keeping the
inverse self-contained and mergeable with the sibling history-soundness branch.

### Documented gap (fail-typed, never corrupt)

If an intervening op subdivides or consumes a recorded wall and a later undo
does not restore it as a pristine quad, `unbuild_push_pull` cannot faithfully
reverse — the completeness check above skips it and the op refuses typed
(`NonManifoldResult`) with the object untouched, in both debug and release.
This is the same quad-wall / intervening-topology limitation as the deferred
"generalized step-wall recognition for collapse" (ROADMAP), reached through the
new inverse op. Both fuzz harnesses tolerate it via
`is_known_inverse_guard_gap` with pending op `UnbuildPushPull` and the same
`NonManifoldResult` signature. The general fix is
the same as for the `PushPull` inverse: generalized (non-quad) step-wall
recognition, deferred until after initial release.

## Merge-time dispatch seam (curves branch)

Curved surfaces are out of scope on this branch: it has **no `SurfaceRef`**
(that lives on the sibling curves branch). A faceted cylinder here is just flat
facets, so a facet push does the flat translate-and-build (a bump) — accepted
on this branch. The whole-wall "entire curved part expands" behaviour is
delivered by the curves branch.

**At merge, the dispatch is disjoint by face:**

```
if face carries an analytic SurfaceRef  → whole-wall expand (curves branch)
else                                    → translate-and-build (this branch)
```

Add the `face.surface` check at the **top** of `Object::push_pull`, routing
surface-carrying faces to the curves path before the flat classification below.
The flat path here is already structured so this check drops in without
reworking it: the classification, the wall build, the result-validation, and
the undo are all keyed off boundary-edge geometry, not off any assumption that
faces are surface-free. No shared state to reconcile beyond that top-level
branch.

## Known deferred case: P4 hole-edge push

Pushing an **outer edge of a face into or past one of its own holes**
(rectangle-with-hole) is a subtle case the maintainer deferred until true
circles merge. The tapered-hole *pull* (pulling the whole holed face out) falls
out cleanly here — the hole ring rides up rigidly and its slanted tunnel walls
each grow a wall — but the edge-into-hole *push* is not specifically handled;
translate-and-build either builds a valid result or refuses typed. Do not
over-invest; revisit with the curves work.

## Test coverage

`crates/kernel/tests/op_specs.rs` — the "flat-face push/pull
(translate-and-build)" section: prism wall, unbounded pull, hex/cylinder facet
pads, inward recess, wedge (builds walls out / cannot push in), tetra/octa
faces, mixed coplanar+slanted, dissolved-seam face, near-flat chamfer (no
slide), holed face, tapered hole ring, the interpenetration/engulfment guards,
and the star-prism property test (accepted → watertight + genus + invertible
via History; refused → byte-identical). `crates/kernel/src/history.rs` unit
tests and the op/document fuzz harnesses cover the undo/redo cycles.

## Appendix: adversarial-review round

An adversarial review after the initial landing surfaced four confirmed
findings, all addressed on this branch:

- **[major] `find_unbuild_plans` validity check was incomplete.** It checked
  only the wall's outer 4-cycle, not `inner_loops.is_empty()`, so undoing a
  push after an intervening `split_face_inner` imprinted a hole on a built wall
  removed that wall and orphaned its hole loop + sub-face — a debug
  `check_invariants` panic (release corruption behind the backstop). The check
  is now complete (recorded set membership, clean 4-cycle outer loop, no inner
  loops, exact closure), so the case refuses typed byte-identical in both debug
  and release. Regression:
  `op_specs.rs::unbuild_refuses_when_a_recorded_wall_gained_a_hole`. Fuzz re-run
  green at 16k (debug + release).
- **[minor] "pull is unconditionally unbounded" overclaim** corrected to
  "unbounded by neighbour angle" across the `push_pull` contract, the
  `NonManifoldResult` doc, the two inline sweep comments, `validate_sweep_result`,
  the ROADMAP, and this note: a pull whose walls would ram a distant part of a
  non-convex solid still refuses typed (a real self-intersection).
- **[minor] stale `push_pull_coplanar_aware` references** (renamed to
  `push_pull_build_walls`) fixed, including two broken rustdoc intra-doc links.
- **[refuted]** the merge-dispatch-note insertion-point concern — no change.

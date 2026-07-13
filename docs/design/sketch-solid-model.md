# Sketch–Solid Consumption: A Redesign

Status: **ratified — Model D (§4D/§6), shipped; the re-extrusion gate was
subsequently DROPPED after playtest** (see the Implementation status
appendix at the end of this document). The maintainer's rulings on the open
questions of §8: delete-to-recover is not load-bearing (undo is the only
way back); the re-extrusion gate — initially kept in its global form — was
ultimately dropped as inconsistent with Hew's freely-interpenetrating-solids
model, confusing in practice, and a blocker of legitimate work (§8 Q2,
resolved below), so re-extruding occupied ground is now simply **allowed**,
producing a coincident second solid like any other overlap; files older than
manifest v10 are treated as nonexistent — loading simply ignores their
stored `footprints`/`consumed` fields.

> **Gate dropped (playtest ruling).** Everything below describing the
> "derived standing-solid gate" (§4D stage 2, §6 stage 1, §8 Q2) documents
> the model *as first shipped*. The gate is gone: `extrude_region` never
> refuses on overlap, `Document::region_blocker` and its helpers are
> deleted, and `DocumentError::RegionBlocked` is removed. The "consumption
> is becoming" half of Model D — extrusion deletes the region's scaffolding,
> nothing hidden resurrects — is unchanged and remains the model of record.
> The launder case (Z10) is now *allowed*, not refused: redrawing a standing
> solid's base and extruding produces a coincident second solid.

This document catalogs the user-visible failure modes of
the shipped Sketch-vs-Solid relationship, develops four candidate
replacements, evaluates them against the constraints that are not up for
discussion, and recommends one. It exists because the shipped model is
correct and launder-proof but produces sketches that appear and disappear by
side effect — behavior at odds with the project's own "what you see is what
you have" position.

## 1. The shipped model

Extruding a sketch region consumes the geometry that bounded it. Consumption
is implemented as three cooperating mechanisms:

- **Footprints.** Each extruded Object freezes its profile's loops in world
  coordinates (`Footprint` on the `ObjectRecord`; persisted since manifest
  v9). Boolean, slice, and push-through results inherit their operands'
  footprints — both operands' for union and intersect, only the kept
  operand's for subtract, the whole set for every slice or push-through
  piece.
- **A derived consumed set.** A sketch region is consumed iff its material
  overlaps a live (non-hidden) Object's footprint on that sketch
  (`Document::recompute_consumed`, the only writer). Because the rule is
  geometric rather than handle-based, no sequence of splits, merges, or
  redraws can make area under a standing solid extrudable again.
- **Tombstones.** Edges and vertices needed only by consumed regions are
  hidden from rendering, picking, and snapping
  (`Sketch::consumed_tombstones`) — not deleted. A sketch whose every edge
  is tombstoned is dropped from `Document::sketch_ids` and vanishes from
  the UI entirely.

Deleting the last Object whose footprint covers an area re-derives the set,
lifts the tombstones, and the outline returns. Files older than manifest v9
carry consumed claims with no attributable owner; these load as ownerless
footprints frozen for the document's lifetime.

The derivation is sound: every mutation path converges on the same state by
recomputing from (sketch geometry, live footprints), undo/redo included.
The failures below are not bugs in that machinery. They are consequences of
its two structural choices: consumed geometry *continues to exist
invisibly*, and the claim that suppresses it is *frozen in world space,
attached to neither the solid's position nor the sketch's*.

## 2. Observed failure modes

Each sequence below is traceable to the current kernel. None involves a
crash or corruption; each is a moment where the document does something the
user has no way to predict.

**Z1 — Outlines resurrect on delete.** Draw a rectangle, extrude it, model
for an hour, delete the solid. The rectangle outline reappears on the
ground plane. If the rectangle was the sketch's only content, the sketch
itself had vanished from the outliner at extrusion time
(`is_sketch_fully_consumed`) — deletion resurrects an entity the user may
never have known existed. The affordance is deliberate (recover the profile,
re-extrude differently) but arrives without any cue connecting the deleted
solid to the outline, and possibly long after the sketch context is
forgotten.

**Z2 — Booleans resurrect outlines as a side effect.** Extrude A and B from
the same ground sketch; subtract B from A. The result inherits only A's
footprints, so B's outline pops back into the sketch the moment the
subtract commits — the cutter's scaffolding "frees" while the user is
thinking about the cut, not about sketches. Undoing the subtract makes the
outline vanish again.

**Z3 — Moving a solid strands its claim.** `Document::transform_object`
bakes the transform into the mesh and touches nothing else; footprints stay
where the extrusion happened. Three consequences compound:

1. The vacated ground remains unextrudable — drawing there produces regions
   that refuse to extrude, with nothing visible to explain why.
2. The solid's new location claims nothing — regions drawn beneath it in
   the same sketch extrude freely, producing interpenetrating solids, the
   exact outcome `RegionConsumed` exists to refuse.
3. Deleting the moved solid resurrects the outline at the *original*
   location, arbitrarily far from where the user deleted something.

**Z4 — Moving a sketch frees regions under a standing solid.**
`Document::transform_sketch` re-derives after the move, so tombstoned
outlines carried out from under a solid reappear at the offset position
while the solid keeps standing; sliding the sketch back makes them vanish
again. The claim stays with the ground, not with either entity the user
manipulated.

**Z5 — Island moves drag invisible geometry.** Islands are connected
components of the *full* edge graph; tombstoned edges still belong to them
(`Sketch::recompute_islands`, `apply_transform_island`). Draw two
rectangles sharing an edge, extrude the left one, then select and move the
right shape: the three tombstoned left-rectangle edges travel with it,
leave the footprint, and materialize attached to the moved shape — a
phantom second rectangle the user never selected.

**Z6 — Region merges eat visibly free area.** Same two rectangles, left one
extruded. The shared edge is live (the right region needs it) and deletable
(`sketch_edge_borders_consumed` permits it). Deleting it merges the two
regions; the merged region overlaps the footprint, so the whole of it is
consumed — the right rectangle's three remaining edges tombstone on the
spot. Deleting one line makes three others vanish and renders area the user
believes free unextrudable. Redrawing the boundary splits the region and
recovers the open half, but nothing suggests that remedy.

**Z7 — Inheritance over-claims.** An intersect result inherits both
operands' full footprints although its material lies only in their overlap;
every slice piece and every push-through piece inherits the whole source
set. Area no surviving piece stands on stays consumed until the last
descendant dies. Conservative and never wrong in the launder direction, but
the claim diverges further from visible reality with each operation.

**Z8 — Component capture makes claims immortal.** `make_component` re-owns
the selected solids as definition members without hiding them, and
`recompute_consumed` filters only on `hidden` — so a definition member's
footprint stays live. Moving the instance moves nothing in the claim;
deleting the instance hides only the instance; exploding it clones the
geometry (with empty footprints) and still leaves the definition member
un-hidden. There is no user-facing way to delete a definition. Net: turning
an extruded solid into a component permanently consumes its birth area, and
the outline can never resurrect — the inverse zombie of Z1.

**Z9 — Copies never claim.** `duplicate_node`, `explode_instance`, and
`make_unique` all create objects with empty footprint lists. A copy placed
on live sketch regions leaves them extrudable; extruding beneath it
interpenetrates. Whether ground is claimed depends on the *provenance* of
the solid standing on it, which the user cannot see.

**Z10 — A fresh sketch launders everything.** Footprints bind to a
`SketchId`; the consumed derivation filters on it. A later sketch on the
same plane — routine after save/load, since the app's cached ground-sketch
handle resets and `begin_ground_sketch` is additive — starts with no
footprints shadowing it. Redraw a standing solid's base outline there
(snapping to the solid's own edges makes this easy) and it extrudes into a
coincident duplicate. The launder-resistance the geometric derivation was
built for holds only *within one Sketch entity*; the document-level
invariant it appears to enforce does not exist.

**Z11 — Pre-v9 claims are frozen forever.** Legacy consumed regions load as
ownerless document-lifetime footprints and are written back into `consumed`
on save, so a reload re-freezes them. Deleting every solid in the file
never frees the area, and no entity in the document explains the refusal.

Z3(ii), Z9, and Z10 are one finding seen from three sides: consumption
never was a "no two solids on the same area" invariant. Overlapping solids
are freely creatable by move, copy, and fresh-sketch redraw. What the
machinery actually delivers is narrower — *refuse re-extruding a region
that already became a solid's base, in place* — and the design space opens
up once the mechanism is sized to that job rather than to an invariant it
cannot enforce.

## 3. Fixed constraints

Candidates are evaluated against these; none is negotiable here.

1. **Watertightness stays meaningful.** Every entity claiming to be a solid
   honors the contract; no suppressed "leaky" category.
2. **No zero-thickness solids.** ARCHITECTURE.md §2.6 gives the two
   standing reasons; a candidate that models flat shapes as degenerate
   Objects must answer both, not restate them.
3. **No silent geometry repair** (DEVELOPMENT.md rule 4).
4. **Combination is always explicit.**
5. **Determinism** — same operations, bit-identical results.
6. **File-format migration** — v9 footprints and pre-v9 frozen claims must
   load into something with defined, documented semantics; loading never
   silently repairs, and old files never fail to load.
7. **Undo/redo coherence** — every step reverses exactly, including the
   sketch-visibility consequences of solid operations.
8. **Launder resistance** — while a solid stands on an area, no sequence of
   sketch splits, merges, or redraws may make that area extrudable again.

## 4. Candidate models

### A. Permanent consumption, frozen refusal

Extrusion deletes the consumed scaffolding outright — the tombstone rule's
edge/vertex set, removed rather than hidden, restored by undoing the
extrusion. Footprints remain exactly as today but serve one purpose:
refusing re-extrusion over occupied area. Nothing ever resurrects; deleting
a solid deletes a solid.

Fixes Z1, Z2, Z4, Z5 (nothing hidden exists to reappear or be dragged) and
Z6 (deleted geometry cannot merge into a live region; a redrawn region
overlapping a footprint simply refuses to extrude, with all its edges
visible). Leaves Z3(i) and (ii), Z7, Z8, Z9, Z10 untouched — the claim is
still frozen world-space data with inheritance rules — and makes Z11
uniform rather than special (every claim is now frozen-forever, so legacy
files stop being a category).

What breaks: the delete-solid-recover-outline affordance, entirely. Undo
covers the mistake case; deliberate recover-and-re-extrude becomes redraw.
Whether that affordance is load-bearing is an open question (§8), but it is
the named source of the zombie complaint.

### B. Explicit lifecycle — visible ghosts

Consumed outlines stay in the document as a third sketch-entity state:
rendered dimmed, selectable, listed in the outliner, releasable or
deletable explicitly. Deleting the last covering solid changes nothing
automatically; the ghost persists until the user acts on it. All current
bookkeeping (footprints, derivation, tombstones-as-state) remains, plus a
persisted ghost state, ghost picking, ghost rendering, and outliner
presence.

This is the honest version of what Fusion-style tools do: hiding is
tolerable there because the hidden sketch has a permanent home (browser
entry, visibility toggle) and a timeline explaining provenance. Hew has an
outliner that could host the same. But the fix is cosmetic where the
disease is structural: Z3, Z7, Z8, Z9, Z10 persist unchanged, the ghost
under a standing solid is drawn exactly where the solid occludes it, and
the model *adds* states and rules to the entity whose lifecycle is already
the confusing part. Most machinery retained, most zombies retained, most
new code of any candidate.

### C. Kinematic binding — the solid carries its base

Footprints become owned, transformable data: `transform_object` (and group
transforms, and instance poses via composition) move the claim with the
solid. Moving a solid frees its birth ground and claims its landing (when
the carried footprint still lies in a sketch's plane; a lift or out-of-plane
rotation suspends the claim). Deleting a solid restores its outline *at the
solid's current location*, minting a sketch on the current base plane if
none exists.

This makes the mental model coherent — "the solid carries its base sketch"
— and directly fixes Z3 and Z8 (claims follow placement), while Z4 becomes
defensible (the sketch left; the solid kept its base). But it keeps
resurrection as a side effect (Z1/Z2 relocate rather than disappear), keeps
inheritance rules (Z7) and the per-sketch binding (Z10), and adds the most
intricate machinery of any candidate: footprint transform composition
through every op that moves geometry, delete-time synthesis of sketch
geometry on arbitrary planes, and undo entries for that synthesis. The
cost is spent making a side-effect behavior more elaborate instead of
removing it.

### D. Consumption is becoming — deletion plus a derived standing-solid gate

Two moves, each removing a mechanism rather than adding one:

1. **Extrusion deletes the scaffolding**, as in A. The outline was the
   profile; the profile is now the solid's bottom face. The sketch's
   remaining geometry (shared edges still bounding live regions, open
   chains) stays, exactly per the current tombstone rule but as real
   deletion, recorded in the extrusion's undo step.
2. **The re-extrusion gate derives from live geometry, not stored claims.**
   A sketch region refuses to extrude iff its material overlaps the
   coplanar contact of a visible solid — any face of any visible Object
   (instances via posed definition faces) lying in the sketch's plane. Same
   `loops_overlap` test as today; the operand set is computed from the
   scene instead of loaded from footprint records.

No footprints are stored, so there is nothing to freeze, inherit, strand,
or leak:

- Z1/Z2/Z4/Z5: nothing hidden exists; nothing resurrects, ever.
- Z3: the claim is the solid's own base face — it moves when the solid
  moves, in both directions, with no bookkeeping.
- Z6: deleted geometry cannot merge. A redrawn region spanning occupied and
  free ground refuses to extrude until split — with every edge visible and
  the blocking solid physically present on it.
- Z7: a boolean, slice, or push-through result claims exactly the area its
  actual geometry stands on.
- Z8/Z9: components and copies claim by standing, like everything else.
- Z10: the gate goes *global* — any sketch on any plane is blocked where a
  visible solid's face sits on it. The launder-resistance property is
  strengthened from per-sketch to per-document.
- Z11: migration is uniform (§6).

This candidate is SketchUp's insight — the drawn profile *becomes* the
face; there is no separate consumed state — imported without SketchUp's
data model. The §2.6 reasons against zero-thickness solids are about
representing *un-extruded* shapes as degenerate Objects: they would
permanently violate the watertightness contract, and they would force a
face-versus-solid case into booleans and push/pull. Neither applies here.
Un-extruded shapes remain first-class Sketches; no flat Object is ever
created; the "imprint" is the bottom face of a genuinely watertight solid
that already exists. What §2.6 argues against is unaffected by ceasing to
pretend the extruded remainder still exists as hidden 2D geometry.

The honest weaknesses of D:

- The recover-outline affordance dies, as in A.
- A solid lifted off the plane no longer blocks extrusion beneath it. This
  is a real change, but the blocked case it abandons is one the current
  model never handled either (Z3(ii)) — and physical overlap was never
  prevented in general.
- Deriving the gate scans visible solids' faces per consumed-set refresh
  instead of a footprint list. Bounded by a plane-key filter over faces
  whose plane data already exists; ordered iteration preserves
  determinism; the model sizes Hew targets make this negligible.
- The extrusion undo step grows: it must restore the deleted sketch
  geometry (the existing gesture-snapshot machinery already does exactly
  this shape of work).

## 5. Evaluation against the constraints

| Constraint | A | B | C | D |
|---|---|---|---|---|
| Watertightness meaningful | yes | yes | yes | yes |
| No zero-thickness solids | yes | yes | yes | yes (§4D) |
| No silent repair | yes | yes | yes | yes |
| Explicit combination | yes | yes | yes | yes |
| Determinism | yes | yes | yes | yes (ordered scan) |
| Migration defined | uniform freeze | state added | synthesis rules | uniform deletion (§6) |
| Undo/redo coherent | yes | yes + ghost states | yes + synthesis undo | yes; coupling removed |
| Launder resistance | per-sketch (Z10 remains) | per-sketch | per-sketch unless extended | per-document |
| Zombies removed | Z1/2/4/5/6 | none (made visible) | Z3/Z8; others relocate | Z1–Z10 |
| Net mechanism | footprints kept | everything kept + ghosts | footprints made kinematic | footprints removed |

Constraint 8 deserves one note: no candidate weakens it. A, B, C keep the
geometric derivation; D re-derives the same refusal from strictly fresher
data (the solid's actual current base rather than a snapshot of it), and
extends it across sketches.

## 6. Recommendation: D, staged

D is recommended because it is the only candidate that removes the causes
rather than managing the symptoms. Every zombie in §2 traces to one of two
things — invisible geometry that still exists, or a frozen claim detached
from both entities it relates — and D deletes both, replacing them with
"the sketch became the solid; the solid blocks where it stands," a rule a
user can verify by looking at the screen.

Stages, each independently landable:

1. **Derived standing-solid gate, additive.** `extrude_region` additionally
   refuses when the region overlaps a visible solid's coplanar face
   contact, on any sketch. Pure addition beside the existing footprint
   check; closes Z3(ii), Z9, Z10 immediately. New typed error naming the
   blocking object (the roadmap's plain-language-errors work wants exactly
   this payload).
2. **Consumption becomes deletion.** Extrusion removes the tombstone
   edge/vertex set instead of hiding it; `DocAction::CreatedObject` carries
   what it removed (or a sketch snapshot, as gestures do) and undo restores
   it. The consumed index stops growing; delete/boolean/undo paths stop
   touching sketch visibility. Closes Z1, Z2, Z4, Z5, Z6.
3. **Retire the stored model.** Remove `Footprint` storage, inheritance at
   boolean/slice/push-through, `legacy_footprints`, the consumed indices,
   and `consumed_tombstones`. Manifest v10 stops writing `footprints` and
   `consumed`. Loading v8/v9: for each consumed region, delete its
   tombstoned edges/vertices (the exact set the current loader hides);
   footprint data is dropped after use. Loading pre-v8: the same, for
   ownerless claims — the file's consumed set is honored by deletion, not
   by freezing. Documented in HEW_FILE_FORMAT.md in the same commit; the
   migration is deterministic and total, and the visible document is
   pixel-identical before and after (the geometry deleted was already
   invisible).
4. **Surface cleanup.** wasm-api sheds the live-edge filters and consumed
   queries; the UI sheds nothing visible, because nothing visible ever
   depended on tombstones.

What gets worse, stated plainly:

- Deleting a solid recovers no outline. Redraw or undo are the remedies.
- A floating solid does not reserve the ground beneath it.
- Old files' frozen claims dissolve where no solid still stands on them —
  area a pre-v10 build would have refused becomes drawable. The claim is
  honored (the geometry it hid is gone), but the *refusal* it implied is
  not preserved.
- The extrusion undo record is larger.
- Readers of v10 files that relied on the redundant `consumed` list find
  it gone (with nothing consumed, there is nothing to list).

## 7. Prior art, where it changes a conclusion

**SketchUp.** There is no sketch entity: drawing produces model edges and
faces directly, push/pull moves a face, and the question "what happens to
the outline" cannot arise because the outline *is* the bottom face's
boundary. This is the strongest evidence that "the profile becomes the
solid" matches the direct-modeling intuition Hew borrows — and SketchUp's
costs (accidental welding, hollow face-soup) are precisely the two failure
modes Hew's Object model already fixes by other means, so the becoming can
be adopted without the stickiness.

**Fusion 360 (and Shapr3D similarly).** Sketches persist as first-class
inputs; a feature that uses a profile auto-hides the sketch, which remains
in the browser with a visibility toggle. Hiding works there because the
hidden thing has a permanent, inspectable home and a history tree
explaining provenance. Hew's tombstones hide without a home — candidate B
is what adopting the missing home would look like, and §4B is why that
spends effort making the current model presentable instead of simple.

**Plasticity (direct modeling, no history).** Source curves survive solid
creation as ordinary independent objects; deleting a solid never revives or
removes a curve. Across the field, no direct modeler resurrects 2D input
when a body dies — the affordance D removes is one no incoming user's
intuition expects to exist.

## 8. Open questions

1. **Is delete-to-recover load-bearing?** Any workflow that relies on
   deleting a solid to get its profile back (rather than undo or push/pull
   to re-dimension) dies with D and A alike. If it must survive, C is the
   only candidate that keeps it coherent, at its stated cost.
2. **How wide should the gate be? — RESOLVED: drop it entirely.** D first
   shipped the standing-solid refusal global across sketches. Playtest
   settled the alternative named here: drop the gate entirely,
   SketchUp-style, and let a deliberate redraw over an occupied base extrude
   an overlapping solid. The gate proved inconsistent with Hew's
   freely-interpenetrating-solids model (overlap is creatable by move, copy,
   and boolean anyway), confusing (a refusal with no physical obstruction
   visible), and a blocker of legitimate work (a larger coaxial solid over a
   smaller one). The one accident it prevented — double-extruding the same
   spot — is cheaply undone and rare enough not to justify the mechanism.
   `extrude_region` now never refuses on overlap.
3. **Migration posture for ownerless legacy claims.** Honoring pre-v10
   consumed sets by deletion (D) changes what an old file *refuses*
   compared to the freeze-forever behavior. Is that acceptable, or must
   ownerless claims persist as refusal zones for the document's lifetime
   even under D?
4. **Should stage 1 land regardless of the rest?** The derived gate closes
   the interpenetration holes (Z3(ii), Z9, Z10) under any candidate,
   including keeping the current model; it is compatible with all of A–D.

## Appendix: Implementation status

Maintained in every status-changing commit (successor-handoff contract).
If you are resuming this work cold: read §4D and §6 above first, then this.

### Done

- **The standing-solid gate was DROPPED (playtest ruling).** After the model
  shipped, the maintainer's playtest resolved §8 Q2: drop the re-extrusion
  gate entirely rather than keep it global. Removed: `extrude_region`'s gate
  check, `Document::region_blocker` and its gate-only helpers
  (`ancestor_group_hidden`, `tags_hidden`, the free `object_contact_overlaps`,
  and the geom2d `loops_overlap`/`bboxes_touch`/`segments_cross_properly`
  chain that only it used), and `DocumentError::RegionBlocked`.
  `extrudable_regions` is now simply all closed regions. The wasm
  `sketch_regions`/`pick_sketch_region` split (which surfaced blocked regions
  so a push/pull attempt could show the refusal) collapses — every region is
  extrudable — and the app's `RegionBlocked` toast mapping is gone. No file
  format change: the gate was always derived, never stored; manifest stays
  v11 and goldens are unaffected. The Z-spec suite's gate-refusal specs
  (Z3/Z4/Z6/Z7/Z8/Z9/Z10 and the tag-visibility/adjacency/top-face/
  two-solids extras) were flipped to the no-gate behavior or deleted; the
  consumption-by-deletion specs (Z1/Z2/Z5/Z11 and the extrusion-undo specs)
  are unchanged. The document-fuzz repros' gate-clearance workarounds
  (`GATE_CLEARANCE_Y` + extrude-far-then-transform-back) were reverted to
  plain overlapping extrudes. **The "consumption is becoming" half of Model
  D is untouched.**

- **Round-2 F2 — undo/redo are recordable** (this commit):
  `RecordedCall::SceneUndo`/`SceneRedo` (additive variants, the
  `SketchBeginCurveWith` posture), recorded in
  `scene_undo`/`scene_redo` only on success (a refused redo commits
  nothing and is not captured), replayed through the same entry points.
  Red-checked golden-hash spec `record_then_replay_captures_undo_redo`:
  draw → extrude → undo (scaffolding re-inserted) → redo (re-deleted by
  geometry) → undo → gesture-bracketed draw (clears redo) → failed redo
  attempt (unrecorded) → eraser delete → extrude; replay reproduces the
  exact hash and byte-identical save. DIAGNOSTICS.md's method table now
  lists every recorded call, the new variants included. Incidental note
  the spec surfaced: UNBRACKETED `sketch_add_segment` bypasses the
  document log and does not clear the redo stack — tools always bracket,
  so the spec drives the bracketed path.

- **Round-2 F1 — retroactive consumption is version-gated** (commit
  add7db8): `DocLoadRaw` carries the manifest's declared `format_version`;
  the retroactive-consumption block runs only when it is older than
  `MANIFEST_CLAIMS_RETIRED_VERSION` (11, a named constant in
  serialize.rs). Decode rejects a `consumed` list in a manifest declaring
  v11+ as typed `MalformedManifest` — reject-not-repair: acting on a
  field the declared version retired would silently delete sketch
  geometry no standing solid claims. Spec
  `consumed_field_smuggled_into_a_v11_file_is_rejected` (red-checked:
  previously loaded fine and deleted the scaffolding); the genuine
  v10-shaped fixture spec still passes. HEW_FILE_FORMAT.md states the
  exact gating.

- **Review minors** (commit 22741de): the eraser's commit
  (`sketch_remove_edge`) is captured by the session recorder and replayed
  (`RecordedCall::SketchRemoveEdge`, additive variant like
  `SketchBeginCurveWith`; replay spec red-checked). Z7 now covers all
  three combinators the doc names: boolean (intersect), slice
  (`z7_slice_pieces_claim_exactly_where_they_stand`), and push-through
  (`z7_push_through_results_claim_their_holed_base` — a through-hole
  opens the ground beneath it). Byte-stable clean-v11 resave from an
  old-shaped manifest was already pinned inside the F7 fixture spec.
  Stale docs fixed: SelectTool.ts pick description, wasm
  `pick_sketch_region` doc (blocked regions DO pick; the refusal comes
  from the extrude attempt), serialize.rs v8/v9 version notes (marked
  retired-at-v11, no more live references to the consumed derivation).
  The golden_file.rs "consumed region" comment stays — refuters
  correctly note "consumed" is Model D's own vocabulary.

- **Review F10 — extrusion undo merges, never clobbers** (commit ed4586e):
  `DocAction::CreatedObject` stores the deleted scaffolding as
  re-insertable rows (endpoints + curve-chain id) instead of whole-sketch
  snapshots. Undo re-inserts via `Sketch::restore_edges` (endpoint welds
  OK; any split/cross/overlap with geometry drawn since →
  `SketchError::RestoreConflicts`, document untouched, action back on the
  undo stack). Redo re-deletes BY GEOMETRY (`Sketch::edge_at_positions`
  over the rows) — the document fuzzer proved an id set goes stale when a
  gesture undo/redo on the same sketch snapshot-restores the outline's
  original edge ids. Consequence, documented on the action: restored
  edges/regions carry fresh handles (slotmaps cannot re-mint keys);
  object/sketch handles stay stable; callers re-query. Specs
  (red-checked, 2/2 failed pre-fix):
  `undoing_an_extrusion_preserves_interleaved_sketch_edits`,
  `undoing_an_extrusion_refuses_typed_on_conflicting_edits`; the three
  handle-stability specs adapted to the re-query contract.

- **Review F7 — pre-v11 files consume on load, not resurrect**
  (this commit): the loader honors an old file's stored `consumed` index
  ONE final time by applying becoming retroactively —
  `Sketch::regions_scaffolding` (set variant of `region_scaffolding`:
  edge dies iff on a consumed boundary and no survivor's) + one
  `remove_edges` per sketch, emptied sketches removed, index discarded.
  The Manifest DTO reads `consumed` again (`#[serde(default,
  skip_serializing_if)]` — never written at v11); dangling pairs are
  typed `DanglingReference` errors (range-validated at decode). Specs:
  `older_files_consumed_claims_become_deletion_on_load` (shared-wall
  survival, emptied-sketch removal, clean byte-stable v11 resave) and
  `older_files_dangling_consumed_pairs_are_rejected`, both red-checked
  (written before the loader change; 2/2 failed). HEW_FILE_FORMAT.md's
  retired-fields section now describes the one-time consumption.

- **Review F5 — the gate honors tag visibility** (commit d82148a):
  `region_blocker` excludes tag-hidden solids: `Document::tags_hidden`
  implements the Tags panel's union rule kernel-side (a node is
  tag-hidden iff any of its tag paths is at or under a hidden `tag_meta`
  path — anchor-prefix, the app's `isPathUnder`), checked on the node and
  every ancestor group (`ancestor_group_hidden`, which also carries the
  user-hidden check). Audit answer, recorded in `region_blocker`'s doc:
  the wasm `Scene::set_hidden` session state is the app-computed cache of
  exactly the two kernel-persisted signals (per-node user hides +
  tag_meta hides), so the gate needs no separate consultation of it.
  Specs: `tag_hidden_solids_claim_nothing` (incl. parent-path anchor and
  ancestor-group tag) and `tag_hidden_instances_claim_nothing`,
  red-checked (tags_hidden neutralized → both fail).

- **Blocked regions render and pick; real-browser playtest complete**
  (this commit): the wasm `sketch_regions` and `pick_sketch_region` now
  serve ALL closed regions — a gate-blocked region still fills (lifted
  fills avoid z-fighting) and picks, so a push/pull attempt on it
  surfaces the RegionBlocked toast instead of the region being silently
  unselectable (the ergonomics §4D asks for; kernel
  `extrudable_regions` stays gate-filtered). Verified end to end in a
  real Chromium against the dev server on 5186 with a throwaway
  Playwright driver (real mouse + VCB where the flow is user-visible,
  harness where it is semantic): draw → extrude (saved file has no
  sketches and no consumed/footprints keys) → undo (outline back, solid
  gone, one step) → redo → move solid (landing refused naming
  RegionBlocked, birth ground extrudes) → delete (sketch count
  unchanged — nothing resurrects; the previously refused handles
  extrude) → save/reload → redraw over the base refused → Z6 spanning
  region drawn with real mouse clicks, push/pull on its free part shows
  "A standing solid already occupies this area". Driver deleted after
  the run, per plan.

- **Docs (commit 50018f7):** ARCHITECTURE.md §2.6 rewritten for Model D
  (becoming + derived gate, reasons not history; §2.2 cross-reference
  updated), ROADMAP's push/pull entry names the gate, user guide
  (site/src/content/learn): drawing.md loses the footprint-era "solid
  remembers the area" / "outline returns" prose, push-pull.md explains
  becoming + the refusal message + undo as the way back. No screenshots
  change: tombstoned edges were already invisible, so deletion renders
  identically.

- **Stages 2+3 — consumption becomes deletion; stored model retired**
  (commit e83480d):
  - `extrude_region` deletes the region's scaffolding
    (`Sketch::region_scaffolding` — the edges bounding ONLY that region —
    then `Sketch::remove_edges`, one recompute of regions/islands, orphan
    vertices dropped). Shared edges survive with their neighbor; open
    chains survive; a partially consumed curve chain keeps its (still
    valid) `CurveGeom`. An extrusion that empties the sketch removes the
    sketch itself (`hidden_sketches`). `DocAction::CreatedObject` carries
    before/after sketch snapshots + `emptied`; undo/redo restore them
    atomically with the object's visibility.
  - Retired: `Footprint`, `ObjectRecord::footprints`,
    `Document::{consumed, legacy_footprints, consumed_sketch_edges,
    consumed_sketch_verts}`, `recompute_consumed`, `refresh_consumed`,
    `is_region_consumed`, `is_sketch_edge_consumed`, `consumed_edges_of`,
    `sketch_edge_borders_consumed`, `is_sketch_fully_consumed`,
    `footprint_sketches_of`, boolean/slice/push-through inheritance,
    `Deleted::footprint_sketches`, `DocumentError::RegionConsumed`,
    `Sketch::consumed_tombstones` (replaced by `region_scaffolding`).
    `extrudable_regions` filters by `region_blocker`; `curve_chain_at`
    lost its `hidden` parameter.
  - Manifest v11: `consumed` / `objects[].footprints` / `objects[].source`
    no longer written; loaders ignore them in older files (maintainer
    ruling). HEW_FILE_FORMAT.md updated; golden regenerated.
  - wasm-api: consumed-edge filters gone, `island_borders_solid` +
    `IslandBordersSolid` refusal gone (islands hold only real edges),
    `sketch_edge_endpoints`/curve/island queries serve all edges;
    `RegionBlocked` surfaces as `"RegionBlocked: a standing solid already
    occupies this area"`; app maps it to a friendly toast
    (app/src/viewport/geoHelpers.ts `kernelErrorMessage`).
  - Z catalog complete in `sketch_solid_model_specs.rs`: Z1–Z11 plus
    multi-solid coverage, hidden-solid, adjacency, and top-face specs.
    Footprint-era specs adapted or deleted deliberately (see the commit
    body); `document_specs.rs` extrusion/scaffolding specs rewritten for
    deletion semantics; `serialize_specs.rs` gained
    `older_files_stored_claim_fields_are_ignored` and
    `z11_saved_files_store_no_claims` (in the Z file) pins that v11 writes
    no claim fields.

- **Stage 1 — derived standing-solid gate, additive** (commit 729581b):
  - `Document::region_blocker(sketch, region) -> Option<NodeId>` in
    `crates/kernel/src/document.rs`: scans visible world objects (slotmap
    order), then visible instances (definition member faces mapped through
    the pose), for a face coplanar with the sketch plane (either normal
    orientation, `tol::NORMAL_DIRECTION` + `tol::PLANE_DIST`) whose
    material overlaps the region's (`geom2d::loops_overlap`; boundary
    grazing is not overlap). "Visible" = not tombstoned, not user-hidden
    (node or ancestor group). Free helper `object_contact_overlaps` at the
    bottom of document.rs.
  - `extrude_region` refuses with new `DocumentError::RegionBlocked { by }`
    ("a standing solid already occupies this area"), checked after the
    (still-present) legacy `RegionConsumed` check.
  - Specs: `crates/kernel/tests/sketch_solid_model_specs.rs` — Z3(ii), Z8,
    Z9, Z10 (incl. save/load), hidden-solids-claim-nothing, adjacency,
    top-face blocking. Red-checked (gate disabled → 7/8 fail).
  - Adapted tests that seeded coincident/overlapping same-plane extrusions
    as scaffolding: `doc_save_load_hash_repro.rs`, `document_fuzz.rs`
    (`add_box` → `Option`, blocked seeds skipped), `document_specs.rs`
    (torture test moves b into overlap), `golden_file.rs` (rect 1 moved to
    free ground; golden regenerated), wasm-api tests (second square drawn
    at an offset and moved into place).

### Remaining (priority order)

Nothing. The gate was dropped after playtest (see the top of this appendix
and the Status banner); the "consumption is becoming" model, its docs, and
its verification are complete.

### Gotchas

- `pnpm install` needed in this fresh worktree before app builds; wasm:
  `wasm-pack build crates/wasm-api --target web --out-dir ../../app/src/wasm/pkg`.
- `scripts/verify.sh` exit code must be checked DIRECTLY (tail masks it).
- Goldens regenerate with `REGENERATE_GOLDEN=1 cargo test -p kernel
  --test golden_file`.
- The gate is gone: tests, fuzz repros, and the app may extrude overlapping
  or coincident bases directly (interpenetration is allowed everywhere).
  The earlier "build beside + move into place" workaround is no longer
  needed and was reverted where it existed.

### Next action

None. The gate is dropped; the consumption-by-deletion model stands. Any
future revisit of re-extrusion ergonomics starts from `extrude_region`
(kernel — no longer gated) and the sketch/region wasm surface.

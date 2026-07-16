# Hew — Roadmap

## Where Hew is

Hew is publicly released: the core modeling experience — draw, push/pull,
combine, organize, and save or export a model start to finish — ships as
native desktop apps for macOS, Windows, and Linux (with automatic updates)
and as a browser app at app.hew3d.com. The release is aimed squarely at the
3D-printing and maker audience: model something in a familiar,
SketchUp-like interaction style and export a file a slicer accepts as
watertight, with no repair step. Development continues on the planned work
below.

## Shipped

### Modeling

- Rectangle, line, circle, and arc drawing on the ground plane or on any
  face, all sharing one ground sketch so mixed-tool profiles close into
  regions; arcs optionally close as a pie or a chord segment (Alt cycles)
- Drawing-like sketch editing: lines are selectable and deletable
  (merging the regions they separated), a drawn arc or circle selects
  and deletes as one curve, and each connected shape is an independent
  island — selected, deleted, and moved without touching anything else
- Push/pull to extrude a closed profile into a solid, with a live preview;
  the outline becomes the solid's base face and leaves the sketch.
  Re-extruding occupied ground is allowed — Hew's solids interpenetrate
  freely, so a region over a standing solid extrudes into a coincident
  second solid
- Push/pull through-cuts that punch holes or remove material, including
  splitting a solid into two when a cut fully severs it
- Explicit boolean union, subtract, and intersect, with coplanar seams
  dissolved on the result (a union of two flush boxes reads as one box)
- Non-destructive, nestable grouping (Group/Ungroup)
- Group-level booleans: Union/Subtract/Intersect accept a whole group on
  either side (or mixed with a plain solid). The group's solids are fused
  first, then the operation applies; one connected result stays a single
  solid, and disjoint pieces arrive together in a result group named from
  the operands. Instances inside an operand refuse with a typed error
  (explode first) rather than being made unique implicitly
- Group duplication: Move+Alt-copy deep-copies a whole group — nested
  groups recursively, contained component instances as new instances of
  the same definition, names/tags/materials preserved — fully independent
  afterward, removed by a single undo
- Components: shared geometry with independent per-instance position,
  rotation, scale, and mirroring; edit the definition once, every instance
  updates. A component takes its name and tags from what it was made of;
  every instance shows the definition's name (a renamed instance reads
  "Instance Name (Definition Name)"), and Object Info counts a selected
  instance's siblings — click the count to select them all
- Make Unique (detach an instance into its own copy, named "<definition>
  Copy" — or after the instance's own name if it has one) and Explode
  (bake an instance back into ordinary geometry)
- Slice: cut a solid along a plane into two independent watertight solids
- Push/pull on any planar face of a solid, not just faces with
  perpendicular neighbors: it follows classic SketchUp translate-and-build —
  the flat face moves rigidly and each oblique or coplanar neighbor grows a
  fresh side wall, so pulling a sliced wedge's cut face erects a prism of
  material along its slope and a faceted circle's side facet grows a pad.
  Pulling outward succeeds however oblique the neighbors (unbounded by
  neighbor angle — it just erects more material); pushing inward succeeds only
  as far as the result stays valid and otherwise refuses with a typed error,
  object unchanged — a wedge's slant face cannot be pushed in at all. (A pull
  whose walls would ram a distant part of a non-convex solid still refuses,
  since that is a real self-intersection.) Undo of a wall-building push is
  exact, recorded as data
- Follow Me: sweep a closed sketch profile along a path — a connected
  chain of sketch edges (lines and arcs, open or closed) or a solid
  face's outer boundary — into a new watertight solid, mitered at every
  path corner. The profile must sit perpendicular across the path (Hew
  never re-orients it); closed loops weld their seam exactly (frames,
  faceted-lathe shapes); a drawn circle swept along a straight run stays
  a true stamped cylinder, while walls around a path's turns keep honest
  facets (no toroidal surface identity yet). Ineligible sweeps —
  non-perpendicular or detached profiles, branching or disconnected
  selections, bends tighter than the profile, self-intersecting results
  — refuse typed with the document untouched (docs/design/follow-me.md)
- Move and rotate with axis-locked inference snapping; tapping Option/Alt
  durably toggles Move between moving and copying (so an exact copy
  distance can be typed), and a committed copy refines into an array:
  typing `x3` places 3 copies at the same spacing along the vector, `/3`
  places 3 copies dividing the distance — re-enter a different count while
  the gesture is hot, one undo removes the whole array
- Delete for objects, groups, instances, and guides
- Drawing directly on a solid's face splits it and supports bosses/recesses,
  following the same "sticky geometry" rules SketchUp users already know
- Offset: copy a sketch region's or solid face's boundary inward or outward
  at a uniform distance, with a live preview and typed exact distances.
  Straight edges offset to mitered parallels; drawn circles and arcs offset
  to true concentric curves (exact center, radius ± the distance). A region
  offsets all its loops (holes included) and both results stay extrudable; a
  face offset imprints the inset loop for boss/recess work. A distance the
  shape cannot absorb is refused typed — never a repaired result
- True curves over the faceted carrier: a drawn circle or arc keeps its
  exact center and radius, extruded curved walls know the cylinder they
  approximate, render smoothly with the facet seams suppressed, and
  facet counts adapt to the curve's size at draw time
- Whole-wall push/pull on curved walls: pushing any facet of a drawn
  cylinder offsets the whole wall's radius exactly, with typed refusals
  where the result would be invalid — including growing a wall into, or
  cleanly past, geometry it shares nothing with (interpenetration and
  engulfment guards). Known gap, tracked separately: plain
  translate-mode push/pull has always lacked the equivalent guards (a
  face translated far enough can pass through a disjoint shell of the
  same Object); the generalized-push/pull effort builds them for its
  stretch path, and translate mode follows at that integration
- Standard camera views (Top, Front, Iso, etc.) plus orbit/pan/zoom navigation
- Full undo/redo across the whole document

### Inference & precision

- Snapping to endpoints, midpoints, edges, faces, and locked axes, with
  on-screen cues for every snap
- Analytic curve snaps: the exact center, quadrant points, and
  tangent-from-anchor points of drawn circles and arcs, honoring each
  arc's actual angular range
- Construction guides — guide lines and points that participate in
  snapping and are saved with the model
- Tape Measure (point-to-point distance, or drop a parallel guide at an
  offset) and Protractor (measure an angle, or drop an angular guide)
- Metric and imperial units, including SketchUp-style architectural
  (feet-inches-fractions) input, with typed numeric entry on every
  length-driven tool
- Live watertightness status for every solid
- Snapping that stays fast on instance-heavy models: candidates are
  stored once per component definition and every placement resolves
  through a two-level spatial index, so load, undo, and visibility
  changes no longer pay per-instance registration cost
- Targeted refresh for history and palette mutations: undo/redo rebuilds
  only the scene nodes its document change names (falling back to a full
  rebuild only for group-structural steps), and the Materials panel's
  opacity slider updates the renderer's already-built material opacities
  in place — the same mechanism isolation dimming uses — with no
  re-tessellation at all

### Objects & organization

- Document outliner with click-to-select and double-click to enter an
  object's editing context, with the rest of the scene dimmed for focus
- Tags for organizing and toggling visibility of groups of objects
- An Object Info panel for renaming, tagging, and checking an object's
  solid status

### Materials

- A document-wide material palette: flat colors and image textures, painted
  per face
- An object-level default material so newly extruded or grown faces inherit
  a sensible color automatically
- Materials survive splitting, imprinting, and boolean operations
- Per-material opacity (glass, scrim, etc.), adjustable from the Materials
  panel and applying uniformly to flat colors and textures alike

### File format & persistence

- An open native file format (zip container, JSON manifest, binary geometry
  buffers) with deterministic, byte-stable output
- Save/load, autosave, and crash recovery — unsaved work survives an
  unexpected quit and is offered back on the next launch

### Import & export

- glTF/GLB export and import, round-trip tested for fidelity through Blender
- COLLADA (`.dae`) import from SketchUp's own export path, with a healing
  pass that repairs common non-manifold export artifacts
- Direct SketchUp (`.skp`) import for the 2017 file format, built on
  [OpenSKP](https://github.com/hew3d/openskp), a clean-room reader with no
  Trimble SDK code anywhere in its lineage — names, materials, components,
  tags, and guides all come across natively
- Binary STL export, scaled for 3D printing; export is gated on every
  object being a solid, so an exported file is guaranteed manifold
- STL curve resolution: cylinder walls re-facet from their analytic
  definitions at a chosen smoothness (the stored facets are the floor,
  not the ceiling), staying manifold at any setting
- 3MF export — a modern print format alongside STL: explicit millimeter
  units, one mesh per object carrying its name and per-face colors,
  better suited to multi-part prints; gated on solids the same way STL is

| Format | Import | Export |
|---|---|---|
| `.hew` (native) | yes | yes |
| glTF / GLB | yes | yes |
| COLLADA (`.dae`) | yes | — |
| SketchUp (`.skp`, 2017 format) | yes | — |
| STL | — | yes |
| 3MF | — | yes |

### Application shell & UX

- Native desktop app (macOS, Windows, Linux) with native file dialogs,
  recent files, and file-type association — double-clicking a file while
  Hew is already running opens it in the running instance instead of
  spawning a second app
- Browser build with offline support and PWA install
- A labeled tool rail, a command palette (Ctrl/Cmd-K) for finding any tool
  or action by name, a contextual action dock that follows the current
  selection, and a docked properties/outliner/materials tray
- Stage-aware status-bar guidance: every tool tells you what to do next
  ("Click the opposite corner — or type exact dimensions"), updating live
  as the gesture advances
- A welcome screen on bare launches: recent files, the bundled samples,
  and a getting-started link, with a persisted "show on startup" toggle
- Bundled sample models, generated through the kernel's own API: a wall
  clock (a true-curve oak rim, twelve brass hour markers as component
  instances, grouped hands, a translucent glass cover on a tag) and a
  café table (a textured oak top on walnut cylinder legs, crossed
  stretchers, and a resting pen cup) — showing true curves, image
  textures, components, groups, per-material opacity, tags, and guides
- Light and dark themes throughout
- Native window chrome per platform — the system menu bar on macOS,
  native decorations on Windows, a custom title bar on Linux (WebKitGTK
  cannot repaint the native caption)

### Reliability & diagnostics

- A deterministic kernel: the same sequence of operations always produces
  bit-identical results, which makes bugs reproducible instead of "worked
  twice, failed the third time"
- Sound undo/redo by contract: a replayed inverse is never refused by a
  best-effort heuristic and is verified against — then aligned to — the
  recorded state before committing, so undo either restores exactly what
  was there or fails with a typed error; it never corrupts
  (ARCHITECTURE.md §5.7)
- Canonical geometry serialization: saved bytes no longer depend on
  internal storage order, so undo/redo slot reallocation cannot drift a
  saved file. Scope note: this removes ORDER drift; value-level
  floating-point drift from baked move/rotate/scale round-trips (the
  documented `(p + d) − d ≠ p` trap, outside the history-replay proof
  mechanism) remains a known, tolerance-absorbed limitation
- Structured diagnostic logging, with an optional Debug Mode for deeper
  logging and extra internal validation
- Session recording and replay, so a captured session becomes both a bug
  reproducer and a permanent regression test
- An in-app "Report Bug" action that bundles logs, the current file, and a
  session recording for troubleshooting
- Automated testing at every level — unit, component, and end-to-end — plus
  visual-regression checks against reference renders, run on every change
- Plain-language error messages: every operation Hew refuses explains what
  happened and suggests a next step, in the user's vocabulary — the full
  kernel error inventory is covered, enforced by an exhaustiveness test

## Planned

### Near-term

- **Flat-path push/pull into a face's own holes.** Both push/pull paths
  are shipped (translate-and-build for any planar face, whole-wall radial
  offset for analytic curved walls — see Shipped). The one flat-path case
  still refused is pushing an outer face edge into or past one of its own
  holes: the deferred P4 hole-edge case, revisited with true circles
- **A short first-model guide** — rectangle, push/pull, a circle or arc
  detail, export to STL, slice

### Longer-term

- **STEP/IGES import and export**, for precise CAD interchange with
  engineering tools (via OpenCASCADE)
- **Non-uniform scale**, and moving/rotating/scaling multiple selected
  objects together
- **Nested component definitions** (a component containing other
  components)
- **Layers and saved Scenes** (named camera bookmarks)
- **Section planes** — non-destructive visual clipping, distinct from the
  existing destructive Slice tool
- **A WebGPU rendering path**, as a progressive enhancement over the
  current WebGL2 baseline
- **An out-of-process kernel option** for very large models
- **Multi-user collaboration**
- **A plugin/extension API** — sandboxed by design: plugins are separate
  programs reaching Hew only through a documented API (see "Plugins run
  sandboxed" in ARCHITECTURE.md §4), never linked in-process
- **SketchUp (`.skp`) export** — import is supported today; writing `.skp`
  is not yet
- **Signed, notarized installers, auto-update, and a hosted web build**
  with no install step

### Deferred until after initial public release

- **Generalized step-wall recognition for the recorded push/pull inverse.**
  `find_unbuild_plans`, behind the recorded `UnbuildPushPull` inverse of a
  slanted-neighbor translate-and-build push, matches only pristine quad
  walls. When an intervening op subdivides or consumes one of those walls —
  or a redo rebuilds them with fresh handles — the exact un-build is
  impossible, so undo/redo refuses typed with the object untouched rather
  than closing the step. Both fuzz harnesses tolerate exactly this
  `UnbuildPushPull` signature via `is_known_inverse_guard_gap`. The fix
  extends wall matching (and the shared collapse surgery) to subdivided
  and L-shaped prismatic walls, which also lets a plain `push_pull(-d)`
  re-close the built step directly.

## Non-goals

- **Silent geometry repair.** An operation that would produce invalid
  geometry fails with a clear error rather than being patched up invisibly.
  Objects are never left in a broken state behind the scenes.
- **Implicit merging of geometry.** Objects never weld together on their
  own; combining them (union, group, etc.) is always an explicit action.
- **General sub-element (face/edge) topology editing.** Hew's data model
  works in terms of whole, watertight Objects. Direct-pick tools like
  Push/Pull and Paint cover the common editing cases without exposing raw
  mesh topology; an operation that would tear open a solid is refused
  rather than allowed.

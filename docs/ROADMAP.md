# Hew — Roadmap

## Where Hew is

Hew has reached a feature-complete MVP of its core modeling experience: draw,
push/pull, combine, organize, and save or export a model start to finish, on
desktop (macOS, Windows, Linux) and in the browser. Development is now in a
hardening and polish phase — closing the sharp edges a newcomer hits in their
first hour, rounding out import/export coverage, and building the reliability
infrastructure needed to trust the app for real work. The near-term goal is a
public early-access release aimed squarely at the 3D-printing and maker
audience: model something in a familiar, SketchUp-like interaction style and
export a file a slicer accepts as watertight, with no repair step.

## Shipped

### Modeling

- Rectangle, line, circle, and arc drawing on the ground plane or on any face
- Push/pull to extrude a closed profile into a solid, with a live preview
- Push/pull through-cuts that punch holes or remove material, including
  splitting a solid into two when a cut fully severs it
- Explicit boolean union, subtract, and intersect
- Non-destructive, nestable grouping (Group/Ungroup)
- Components: shared geometry with independent per-instance position,
  rotation, scale, and mirroring; edit the definition once, every instance
  updates
- Make Unique (detach an instance into its own copy) and Explode (bake an
  instance back into ordinary geometry)
- Slice: cut a solid along a plane into two independent watertight solids
- Move and rotate with axis-locked inference snapping; Option/Alt-drag to
  copy while moving
- Delete for objects, groups, instances, and guides
- Drawing directly on a solid's face splits it and supports bosses/recesses,
  following the same "sticky geometry" rules SketchUp users already know
- Standard camera views (Top, Front, Iso, etc.) plus orbit/pan/zoom navigation
- Full undo/redo across the whole document

### Inference & precision

- Snapping to endpoints, midpoints, edges, faces, and locked axes, with
  on-screen cues for every snap
- Construction guides — guide lines and points that participate in
  snapping and are saved with the model
- Tape Measure (point-to-point distance, or drop a parallel guide at an
  offset) and Protractor (measure an angle, or drop an angular guide)
- Metric and imperial units, including SketchUp-style architectural
  (feet-inches-fractions) input, with typed numeric entry on every
  length-driven tool
- Live watertightness status for every solid

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

| Format | Import | Export |
|---|---|---|
| `.hew` (native) | yes | yes |
| glTF / GLB | yes | yes |
| COLLADA (`.dae`) | yes | — |
| SketchUp (`.skp`, 2017 format) | yes | — |
| STL | — | yes |

### Application shell & UX

- Native desktop app (macOS, Windows, Linux) with native file dialogs,
  recent files, and file-type association
- Browser build with offline support and PWA install
- A labeled tool rail, a command palette (Ctrl/Cmd-K) for finding any tool
  or action by name, a contextual action dock that follows the current
  selection, and a docked properties/outliner/materials tray
- Light and dark themes throughout
- Native window chrome per platform — the system menu bar on macOS, a
  custom title bar on Windows and Linux

### Reliability & diagnostics

- A deterministic kernel: the same sequence of operations always produces
  bit-identical results, which makes bugs reproducible instead of "worked
  twice, failed the third time"
- Structured diagnostic logging, with an optional Debug Mode for deeper
  logging and extra internal validation
- Session recording and replay, so a captured session becomes both a bug
  reproducer and a permanent regression test
- An in-app "Report Bug" action that bundles logs, the current file, and a
  session recording for troubleshooting
- Automated testing at every level — unit, component, and end-to-end — plus
  visual-regression checks against reference renders, run on every change

## Planned

### Near-term

- **3MF export** — a modern print format alongside STL: explicit units,
  one mesh per object with its name and color, better suited to multi-part
  prints
- **Push/pull on any planar face of a solid**, not just faces with
  perpendicular neighbors — currently the sharpest modeling dead-end a new
  user can hit (it affects, among other things, faceted circles and sliced
  solids)
- **Plain-language error messages** for every operation Hew refuses, each
  with a suggested next step, instead of a raw technical error
- **A welcome screen** on launch, with recent files, bundled sample models,
  and a link to a getting-started guide
- **Bundled sample models** — a finished, printable object and a
  mid-construction scene showing groups, components, materials, and guides
- **A short first-model guide** — rectangle, push/pull, a circle or arc
  detail, export to STL, slice
- **Consistent status-bar guidance** for every tool, so the app always
  tells you what to do next
- **Reliable file-open on Windows and Linux** when double-clicking a file
  while Hew is already running

### Longer-term

- **STEP/IGES import and export**, for precise CAD interchange with
  engineering tools (via OpenCASCADE)
- **True curved geometry** — real arcs and circles instead of today's
  faceted approximations — plus Follow Me and Offset tools
- **Non-uniform scale**, and moving/rotating/scaling multiple selected
  objects together
- **Nested component definitions** (a component containing other
  components)
- **Array copy** — duplicate an object along a line a set number of times
  in one step
- **Group-level booleans and group duplication**
- **Layers and saved Scenes** (named camera bookmarks)
- **Section planes** — non-destructive visual clipping, distinct from the
  existing destructive Slice tool
- **A WebGPU rendering path**, as a progressive enhancement over the
  current WebGL2 baseline
- **Shared inference geometry across component instances.** The snapping
  engine keeps its own world-space copy of every placement's geometry, so
  a model with thousands of component instances pays registration time
  and memory for each placement at load, import, and undo. Storing
  candidates once per definition member and resolving each placement's
  transform at query time — the same idea GPU instancing applies to draw
  calls — collapses that cost to one copy per definition. It requires a
  two-level spatial index (per-definition trees under a tree of placement
  bounds), which is why it is staged after the flat index has proven out.
- **Targeted refresh for palette-wide and history mutations.** Undo/redo,
  and any mutation that can't cheaply name which objects it touched (e.g.
  the Materials panel's opacity slider), fall back to a full-scene
  rebuild — re-tessellating and re-uploading every object's GPU buffers,
  not just the ones actually affected. On a large model this pays the same
  cost class the large-model rendering work already targeted, just through
  a different door: undo/redo has always paid it, and the opacity slider
  now pays it too, once per commit. A real fix needs either a reverse
  index (which objects/instances reference a given material) or a
  live-mutation path that twiddles the renderer's already-built THREE.js
  material opacities in place — the same trick isolation-dimming already
  uses — instead of re-tessellating. Scope note: this is a renderer-wide
  gap, not opacity-specific; fixing it should cover undo/redo's fallback
  too, since they share the same root cause.
- **An out-of-process kernel option** for very large models
- **Multi-user collaboration**
- **A plugin/extension API**
- **SketchUp (`.skp`) export** — import is supported today; writing `.skp`
  is not yet
- **Signed, notarized installers, auto-update, and a hosted web build**
  with no install step

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

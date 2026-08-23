# Hew — Detailed status & plan (agents and contributors)

> This is the exhaustive, engineering-level inventory of what is
> shipped, planned, deferred, and out of scope. It exists for AI agents
> and contributors working in this codebase — code comments and
> `docs/agents/HEW_API.md` cite it by section, so section names are load-bearing.
> For the human-readable summary, read [ROADMAP.md](../ROADMAP.md).


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

- Rectangle, line, circle, arc, and regular polygon drawing on any plane,
  SketchUp-style: the ground plane, any face, the plane of a hovered sketch
  (drawing onto it extends it directly), or — with a draw tool idle — an
  arrow-key lock to a world-axis plane through the next click (Right/Left/Up
  = red/green/blue, same key mapping and color semantics as the transform
  tools' axis locks). Mixed-tool profiles close into regions in one shared
  sketch PER plane (ground drawing keeps its own single shared ground
  sketch, generalized rather than replaced); arcs optionally close as a pie
  or a chord segment (Alt cycles); polygon side count is typed (`Ns`) and
  persists for the session. Editing (select, delete, transform) is
  plane-blind and already worked on a rotated sketch. TapeMeasure and
  Protractor aren't limited to the ground plane either — a guide or
  measurement follows a hovered sketch's plane, an idle arrow-key plane
  lock, or (Tape Measure's parallel guides) the plane a picked face or
  edge actually lies in
- Drawing-like sketch editing: lines are selectable and deletable
  (merging the regions they separated), a drawn arc or circle selects
  and deletes as one curve, and each connected shape is an independent
  island — selected, deleted, and transformed (moved, rotated, scaled;
  out-of-plane rotations included, detaching the shape into its own
  sketch when it shares one) without touching anything else
- Push/pull to extrude a closed profile into a solid, with a live preview;
  the outline becomes the solid's base face and leaves the sketch.
  Re-extruding occupied ground is allowed — Hew's solids interpenetrate
  freely, so a region over a standing solid extrudes into a coincident
  second solid
- Push/pull through-cuts that punch holes or remove material, including
  splitting a solid into two when a cut fully severs it
- Push/pull SketchUp-parity modifiers: double-click a face (or sketch
  region) to repeat the last committed distance there; a durable Ctrl/Cmd
  toggle (tap to flip, cursor badge + status-bar cue) extrudes a NEW
  coincident solid from the clicked face instead — the source is left
  untouched, sharing a footprint with the new solid exactly like
  re-extruding occupied ground
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
- Nested components: a definition contains other components and groups,
  not just objects. Make Component folds a selected instance in as a
  nested member and keeps a selected group whole; editing drills down
  (double-click a nested instance inside an open component to edit it in
  place, Escape steps back one level at a time), and placing a component
  while editing another folds it in when you step out. The definition
  graph stays acyclic and bounded — a component cannot contain itself,
  and nesting past 64 levels or a million rendered parts is refused
- Make Unique (detach an instance into its own copy, named "<definition>
  Copy" — or after the instance's own name if it has one) and Explode
  (bake an instance back into ordinary geometry; a nested member
  surfaces as an ordinary instance, its geometry still shared)
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
- Follow Me: sweep a closed sketch profile — or any face of a solid —
  along a path (a connected chain of sketch edges, a solid face's
  boundary, or a face reached through a component instance) into a new
  watertight solid, mitered at every path corner, with corner seams and
  detached paths handled. A profile that is not perpendicular is stood
  upright automatically (the swept copy folds into place, hinged where it
  meets the path; the source is untouched), so flat-drawn profiles lathe
  and frame directly. Partial sweeps drag to a stop in either direction
  around a closed loop; a Ctrl/Cmd-click merges the molding with the
  path's own solid in one gesture and one undo step (carving where it
  overlaps, adding where it rides the surface). Sweeps committed inside a
  group land in that group. Facet seams of drawn curves — along the path
  and around the profile — render soft (smoothed shading, no seam lines);
  a drawn circle swept along a straight run stays a true stamped
  cylinder, while walls around a path's turns keep honest facets (no
  toroidal surface identity yet — see Planned). Ineligible sweeps —
  branching or disconnected selections, bends tighter than the profile,
  straddled corner starts, an on-axis profile on a non-circular path,
  self-intersecting results — refuse typed with the document untouched
- Move and rotate with axis-locked inference snapping; tapping Option/Alt
  durably toggles Move between moving and copying (so an exact copy
  distance can be typed), and a committed copy refines into an array:
  typing `3x` (or `x3`) places 3 copies at the same spacing along the
  vector, `3/` (or `/3`) places 3 copies dividing the distance —
  re-enter a different count while the gesture is hot, one undo removes
  the whole array. Sketch selections copy too, keeping curve identity (a
  copied circle is a true circle): an in-plane offset replays through the
  sticky rules into the same sketch, while an out-of-plane offset (copying
  a shape up off its plane) lands the copy on a new sketch on the
  translated plane, source untouched — so a profile can be copied up rather
  than moving the only one you have. Sketch copies stay single (no ×N
  array) until a kernel-side sketch duplicate op exists
- Scale: a bounding-box grip gizmo — 6 face-center grips stretch one axis,
  8 corner grips scale uniformly, and 12 edge-midpoint grips scale two axes
  — about a chosen anchor (the grabbed grip's opposite by default, the box
  center with a durable Ctrl toggle), with typed exact factors or target
  dimensions. Dragging a grip past its anchor clamps rather than reflects
  (mirroring is a separate future tool). Move, Rotate, and Scale all extend
  to a whole mixed selection — one Object, a Group, a component instance,
  or any combination — as one shared transform and one undo step; a scaled
  component instance carries the (possibly non-uniform) scale in its own
  pose, leaving the shared definition and every sibling instance untouched
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
- Standard camera views (Top, Front, Iso, etc.), orbit/pan/zoom navigation,
  and a Parallel Projection toggle (true orthographic, not a narrowed
  field of view) that keeps every screen-constant widget (transform grips,
  protractor/rotate disks, section-plane and slice previews) correctly
  sized in both projections; a typed Field of View control; Zoom Window
  (drag a rectangle to frame it)
- Position Camera, Look Around, and Walk: first-person camera placement
  and walkthrough navigation (click or drag to stand and look, mouse-look,
  and forward/turn/strafe movement), sharing one session eye height that's
  typed like any other measurement
- The working camera view (projection, field of view, eye, target, and up)
  is saved with the document and restored on load — optional and additive,
  so older files are unaffected and simply open to today's home framing
- Full undo/redo across the whole document

- 3D Text: extruded, watertight solid text placed on any face, the ground,
  or an axis-locked plane, from bundled fonts or any font loaded from disk —
  letter counters become genuine holes, and each placement folds into one
  component so repeated words stay editable in one place
- Container editing via the session stack: double-click a group or a
  component instance and its contents become ordinary top-level world
  geometry for the edit — every tool (sketching, extrusion, Follow Me,
  booleans, slice, push/pull-through, transforms) works on them by
  construction, no container-aware code path needed. A group session is a
  transient ungroup (nothing bakes; grouping, component/instance creation,
  import, and 3D text stay available inside one, their products folding
  into the group); a component session is a transient explode of the
  instance's pose, always the innermost frame, with sibling instances
  hidden for the session's duration instead of updating live. Frames nest
  per container level and close LIFO (Escape, double-click outside),
  folding edits and mid-session creations back into the container; both
  entry paths — viewport drill-down and Outliner double-click — produce
  the same breadcrumb, with every level on it marked as editing. A
  component session requires the instance's pose to be a similarity
  (uniform scale, no mirror); a group-nested placement now opens through
  its group's session, and a non-uniformly-scaled, mirrored, or
  elsewhere-grouped-sibling placement falls back to the previous
  in-context editing model (the component-edit-parity machinery, still
  fully supported for those cases). Session open/close are ordinary undo
  steps, and saving — autosave included — works transparently mid-session
- In-context resize: the Tape Measure's measure-then-type gesture inside
  an open group or component resizes just that container's contents about
  the measured point — a component resize reaches every placement — while
  the same gesture at the top level still resizes the whole model
- Movable drawing axes: place and orient a document-level frame that axis
  locks, draw planes, inference snapping, axis colors, and the rendered
  triad all follow, with Reset Drawing Axes to return to the world frame

### Inference & precision

- Snapping to endpoints, midpoints, edges, faces, and locked axes, with
  on-screen cues for every snap
- Analytic curve snaps: the exact center, quadrant points, and
  tangent-from-anchor points of drawn circles and arcs, honoring each
  arc's actual angular range
- Construction guides — guide lines and points that participate in
  snapping and are saved with the model
- Tape Measure (point-to-point distance, or drop a parallel guide at an
  offset — sourced from an edge (a plain object's, a group member's, or a
  component instance's), a sketch line, a world axis, or another guide
  line) and Protractor (measure an angle, or drop an angular guide). A
  guide or measurement isn't confined to the ground plane: it follows a
  hovered sketch's plane, an idle arrow-key plane lock, or — for a
  parallel guide whose source edge doesn't lie flat on the ground — the
  plane a picked face or the frozen plane lock actually implies (a
  face picked under the first click engages only while the cursor stays
  over that same face, and releases to the plain sideways offset the
  instant it moves off — it never locks onto whatever face happens to be
  under the very first click for the rest of the drag). A guide or
  measurement started from a frozen plane keeps an off-plane snap
  (an endpoint or midpoint that doesn't sit exactly in it) reachable —
  a measurement projects it onto the plane, a parallel guide instead
  keeps only its component along the guide's own offset direction —
  flagged as projected in the inference chip either way, rather than
  dropping the candidate or landing off-plane. Arrow
  keys lock the parallel guide's offset direction, or the measured
  distance from a point, to a drawing axis mid-gesture — an axis that
  runs along the source edge is rejected with a status hint instead of a
  silent no-op — and Shift momentarily latches the nearest axis for as
  long as it's held; the measurement preview, and a parallel guide's
  offset connector, color by whichever drawing axis is locked or
  inferred. Holding Ctrl/Cmd
  measures without dropping a guide (SketchUp's measure-only mode);
  measuring between two empty points drops both a guide point at the
  second click and an infinite guide line through both points, the same
  parity SketchUp itself has. The distance readout stays in the corner
  after a measurement commits, so it's there to refer back to, until a
  different tool is selected. Typing a length right after measuring
  between two real points — whether before the second click or, just as
  well, afterward once the readout is already showing the measured
  distance — offers to resize the WHOLE model so that distance becomes
  the typed one (a confirmation dialog shows the scale factor);
  confirming uniformly rescales every object, sketch, and guide about the
  world origin in one undo step — component definitions stay at their
  authored size, only instance poses scale
- Section Plane: a non-destructive clipping plane for looking inside a
  model (wall thickness, clearance, voids) — click a face to section it
  there, drag its widget to sweep the cut, toggle it on/off, delete it.
  Distinct from the destructive Slice tool: it changes no geometry, only
  what the viewport draws. Saved with the document as view state (outside
  undo, like the camera) and captured by value into Scenes; uncapped (the
  cut reads via exposed walls, not a filled stencil) — section fill is a
  later rendering feature
- Scenes: named, saved views of a document — the camera, which objects
  and tags are hidden, the section plane, and the grid/axes/guides
  toggles, each independently capturable — restored in one step with an
  animated camera move (View ▸ Scenes ▸ Scene Transitions to turn it off).
  Authored in the editor's Scenes tray section (add, update, rename,
  describe, reorder, delete; the active Scene shows a drift marker once
  the view no longer matches what it captured); consumed in Shop Mode,
  where a pill under the top strip and the Views sheet switch between
  them; addressable from the Hew API (`hew.scenes.*`, and
  `hew.view.snapshot`/`hew.view.line_drawing`/`hew.print.pdf` all accept a
  Scene id and render it headlessly)
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
- Tags for organizing and toggling visibility of groups of objects,
  deletable without touching the geometry that carries them; clicking a
  tag selects everything it carries, and a tag is renamed in place
  (Finder-style click-twice or double-click, and `hew.tag.rename` from the
  API) with every object and nested tag path following the new name
- An Object Info panel for renaming, tagging, and checking an object's
  solid status; with several objects selected it tags them all at once,
  showing the tags they share

### Materials

- A document-wide material palette: flat colors and image textures, painted
  per face
- An object-level default material so newly extruded or grown faces inherit
  a sensible color automatically
- Materials survive splitting, imprinting, and boolean operations
- Per-material opacity (glass, scrim, etc.), adjustable from the Materials
  panel and applying uniformly to flat colors and textures alike
- Alt-click to sample a face's material into the current swatch, and
  Shift-click to replace that material everywhere in the document (or, with
  Ctrl/Cmd, only within the clicked object) in one undoable step
- Position Texture: drag a texture's corner pins on a face to move, rotate,
  scale, and shear it, or type an exact angle or scale factor
- A searchable material palette, with the add-color and add-texture panels
  collapsed until needed

### Library

- A personal library of reusable items — Components, Materials, and whole
  Models — where **every item is an ordinary `.hew` file** in a
  user-relocatable folder (Settings ▸ Folders; default `~/Hew Library`),
  so a library syncs and shares as plain files and any item opens as a
  document in its own right. Item metadata (name, keywords, collection,
  provenance) lives in the file's own v14 attribute dictionaries under the
  first-party `hew.library` namespace — no side database
- A modal browser (Window ▸ Library, the tool rail, ⇧L, or the command
  palette): category sidebar with counts, name/keyword search, scope
  filters (all / in this model / recently saved), user collections,
  thumbnail grid, and a detail pane that reads each file's manifest only —
  listing a large library never decodes geometry. Manage in place: rename,
  keywords, collections, re-render thumbnail, reveal on disk, delete
- Save to Library from the action dock (any solid, group, or component
  instance — asked for a name and nothing else), from File ▸ Save to
  Library for whole-document Model items, and from the Materials panel for
  single materials. A selection saves wrapped as a component definition
  plus identity instance; the item's insertion origin is the document's
  current drawing-axes origin. Thumbnails render in the background through
  the same deterministic headless rasterizer as `hew.view.snapshot`,
  cached by content hash
- Insert with cursor placement: the item's real geometry ghosts under the
  cursor with full inference snapping (Move parity), a marker shows the
  saved origin, one click commits, Esc cancels — one undo step. Inserts
  are **lossless kernel grafts** (analytic curve surfaces, per-edge circle
  claims, soft edges, materials, tags, guides, and definition-owned
  sketches all carry; a model item's loose sketches and annotations are
  reported as skipped, never silently dropped) and **copies, never
  links**. Re-inserting the same item version reuses the definition
  already in the document — never a "Chair (2)" — and palette materials
  deduplicate by content, at insert and at Paint-with-this/Add-to-palette
  alike
- Command-palette quick insert: typing an item's name offers *Insert
  "…"* straight into cursor placement, without opening the browser
- Browser storage: the web build keeps the library in the origin-private
  file system (same folder layout as the desktop store, `.hew` files plus
  a content-hash thumbnail cache), requests durable storage on first
  write, and offers per-item Download as the escape hatch; Chromium-family
  browsers can instead bind a real folder via `showDirectoryPicker` — the
  handle persists in IndexedDB, permission re-grants surface as a
  Reconnect button in the dialog, and binding migrates existing
  browser-storage items into the folder (existing files win, nothing
  overwritten). A bound folder uses the same layout as the desktop
  library folder, so the two are interchangeable — including a
  cloud-synced folder. A browser without origin-private storage still
  reports the library honestly unavailable

### Annotations

- Linear and radial dimensions and leader text, anchored to the geometry
  they describe so they follow it through moves, rotations, and scales — a
  transform that would distort a radius detaches the annotation visibly
  instead of reporting a wrong number
- Annotations save with the document and read at a constant on-screen size,
  staying legible at any zoom

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
  tags, and guides all come across natively. Groups arrive as groups and
  components as shared components, nested assemblies keeping both their
  hierarchy and their sharing; a group wrapping a single solid becomes
  just that solid, since Hew needs no wrapper to isolate geometry
- Binary STL export, scaled for 3D printing; export is gated on every
  object being a solid, so an exported file is guaranteed manifold
- STL curve resolution: cylinder walls re-facet from their analytic
  definitions at a chosen smoothness (the stored facets are the floor,
  not the ceiling), staying manifold at any setting
- 3MF export — a modern print format alongside STL: explicit millimeter
  units, one mesh per object carrying its name and per-face colors,
  better suited to multi-part prints; gated on solids the same way STL is
- Native STL import — the maker-download-and-remix path (Printables,
  Thingiverse): a hand-written binary/ASCII reader (no external STL crate)
  feeds the same weld → heal pipeline every importer shares, so a triangle
  soup with no shared vertices, no object grouping, and no units comes back
  as editable, correctly oriented, watertight-or-honestly-leaky Objects — a
  disjoint multi-part plate splits into one Object per part, and a hollow part
  (outer wall enclosing an inner wall) reconstructs into one Object with a
  cavity. Objects are named from the file (`bunny.stl` → "bunny", "bunny (2)",
  …). STL carries no unit information, so the UI prompts once per import
  (millimeters default, the maker convention)
- USDZ export for AR Quick Look — an uncompressed USD zip (`model.usda`),
  Y-up in meters, one mesh per part with `UsdPreviewSurface` materials,
  validated against Apple's ARKit profile so it opens straight into iOS AR
  Quick Look; gated on solids the same way STL and 3MF are, and reachable
  from `hew.doc.export` everywhere (wasm API, scripting registry, CLI)
- SVG line-drawing export — a true-size, millimeter-`viewBox` hidden-line
  drawing (`crates/hlr`) at a chosen view and scale, hidden lines removed
  or dashed, for laser/CNC and other vector workflows; the same emitter
  that draws vector Line art pages when printing to scale

| Format | Import | Export |
|---|---|---|
| `.hew` (native) | yes | yes |
| glTF / GLB | yes | yes |
| COLLADA (`.dae`) | yes | — |
| SketchUp (`.skp`, 2017 format) | yes | — |
| STL | yes | yes |
| 3MF | — | yes |
| USDZ | — | yes |
| SVG line drawing | — | yes |

### Application shell & UX

- Native desktop app (macOS, Windows, Linux) with native file dialogs,
  recent files, and file-type association — double-clicking a file while
  Hew is already running opens it in the running instance instead of
  spawning a second app
- Browser build with offline support and PWA install
- A labeled tool rail, a command palette (Ctrl/Cmd-K) for finding any tool
  or action by name, a contextual action dock that follows the current
  selection (including Group for a multi-selection and Make Component for
  objects and groups, shown only when the selection qualifies), and a
  docked properties/outliner/materials tray
- One-gesture object interaction: with an empty selection, Move / Rotate /
  Scale act on whatever the first click lands on; the Select tool drags
  objects directly (a threshold-gated one-shot Move — full snapping, axis
  locks, Alt-copy, typed distances — that springs back to Select on
  release); and every draw tool imprints a plain object's face at the top
  level, with push/pull holding to the same eligibility. Groups and
  components keep their explicit double-click editing step — their members
  are not directly editable from outside
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
- Multi-window document editing on desktop: File ▸ New and File ▸ Open both
  open into a fresh window whenever the current one isn't a blank,
  untouched "Untitled" document, so an in-progress model is never silently
  replaced; the Window menu lists every open document window, with a
  checkmark on the current one, to switch between them, and opening a file
  that is already open brings its window forward instead of opening a
  duplicate
- Shop Mode — a read-only, touch-first phone viewer (auto-selected on a
  phone, or via an explicit override) for referencing a model in the
  workshop: tap a part for its size, a live parts cutlist, a Tape Measure
  with a magnifier loupe, hold-a-face to isolate a part, standard camera
  views, offline recents with thumbnails, and View in AR on iOS. A model
  reaches the phone via the Files app or "Open on Phone" from the desktop —
  an end-to-end-encrypted QR handoff through a zero-knowledge relay (the
  desktop encrypts in the browser; the decryption key rides in the QR
  fragment and never reaches the server), picked up by an in-app camera
  scanner
- The relay is self-hostable: whatever origin serves a self-hosted Hew web
  app also serves the relay under `/relay/`, no separate config on the
  phone. The `hew-relay` binary (also a Docker image and a Proxmox LXC
  installer) runs the same contract as the public relay; the desktop's
  Settings ▸ Advanced ▸ Server pane points at it, with an optional upload
  key
- File ▸ Print… — one Print Layout window for paper and PDF, with a live
  page preview: **Standard** mode prints one page, the current view as
  framed or zoomed to fill the page; **Scaled** mode forces parallel
  projection and prints at an exact drawing scale (metric and
  architectural-imperial preset ladders, a free-form custom entry, and a
  Fit action that sets the exact ratio filling one page), with Model /
  Selection / Current view extents — Selection is what makes "print this
  one part at 1:1 as a template" a single step
- Multi-page scaled prints tile automatically into a lettered/numbered
  grid (A1, A2, B1…), with an overlap band for gluing reserved inside the
  printable area, corner crop marks, dashed trim lines, neighbor labels, a
  drag-the-preview nudge to move a seam off an awkward spot, and a graphic
  scale bar on every page so a print can be verified with a ruler before
  anything is cut
- Two print styles: As shown (a 300 dpi bitmap, always on white paper
  under fixed lighting) and Line art (white faces, black edges, and the
  silhouettes of curved walls); in Scaled mode, Line art is true vector
  hidden-line art (`crates/hlr`) —
  crisp at any zoom, a small PDF even across dozens of tiles — with an
  automatic raster fallback, flagged, for a model too complex to trace
- **Save PDF…** writes the composed pages as a PDF directly
  (`crates/pdfwrite`), vector pages staying vector, with no OS print
  dialog in the way; **Print…** hands the same pages to the operating
  system's own print dialog (the macOS sheet, the Windows system dialog
  including Microsoft Print to PDF, the Linux GTK dialog, or the
  browser's print dialog)
- Pages: Each Scene prints one page or tile set per Scene, from that
  Scene's own camera, hidden set, and section plane, with page numbers
  running across the whole job; a Cut list page appends every part once
  with its quantity and L × W × H, every placement of a component folded
  onto its definition
- Shop Mode gets its own Print Layout sheet from the document menu, with
  Save PDF… as the primary action (the iOS share sheet, or a download
  elsewhere) since AirPrint on iOS ignores the paper size Hew composed for

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

### Longer-term

- **True toroidal surfaces** (`SurfaceRef::Torus`): sweep walls around a
  turning path currently carry no analytic surface claim — a cylinder
  claim there would be false, so the verifier refuses it and push/pull
  and refacet on such walls stay facet-level (rendering is already
  smooth via soft seams). A torus surface class through the validator,
  tessellation, and serialization (a geometry-buffer bump) restores
  analytic behavior for pipes and lathe rings
- **STEP/IGES import and export**, for precise CAD interchange with
  engineering tools (via OpenCASCADE)
- **Section fill** (capping the cut) and multi-plane management, on top
  of the persisted section plane Scenes already capture
- **A WebGPU rendering path**, as a progressive enhancement over the
  current WebGL2 baseline
- **An out-of-process kernel option** for very large models
- **Multi-user collaboration**
- **A plugin/extension API** — sandboxed by design: plugins are separate
  programs reaching Hew only through a documented API (see "Plugins run
  sandboxed" in ARCHITECTURE.md §4), never linked in-process. Its
  foundation exists today: the Hew API (docs/agents/HEW_API.md) is implemented
  headless in `crates/api`, with `hew-cli` serving it to scripts and to
  AI agents over MCP; the plugin system adds sandboxed transports and
  scoped profiles on top of the same bus
- **SketchUp (`.skp`) export** — import is supported today; writing `.skp`
  is not yet
- **Signed Windows installers** — macOS builds are signed and notarized,
  auto-update and the hosted web build are shipped; Windows executables
  ship unsigned (SmartScreen warns, documented as accepted) until the
  project meets code-signing programs' visibility bar

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

- **Axis-snap ranking when a world axis is viewed edge-on.** All three world
  axes share one snap kind, and the pick breaks ties by screen-angle, so from
  a camera aligned with an axis the far half of that axis (e.g. the
  underground −Z) can win the pick cone over the near half and flip the sign
  of a typed, axis-locked Move. It is deterministic and surfaces only from
  specific axis-aligned viewpoints. The fix is a ranking refinement (prefer
  the axis half toward the camera / the anchored side); deferred as too niche
  to hold an early release. An investigation branch characterizing the
  degeneracies exists.

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

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
  tools' axis locks), or a Shift pin of the plane under the cursor (a
  hovered face's plane, a sketch's, or the ground) that holds the cursor
  to that plane wherever it goes until Shift or Escape releases it.
  Starting a shape on a solid's edge or corner draws on the face that
  edge belongs to — the one the cursor is leaning toward within the snap
  radius, the cursor's previous hover face at zero offset, else the most
  camera-facing one — never on the ground beneath. Over empty space the
  first click lands on the axis plane that faces the camera: the ground
  unless a vertical plane of the drawing axes faces the view clearly more
  (about 20 degrees above level, square-on to a wall), through the axes'
  origin, shown by a plane-colored ring on the snap dot, the plane's name on
  the inference chip, and a status-bar line (a small patch in empty space
  or while a plane is held, a face outline when leaning onto a corner or
  edge; the older 4 m grid patch is gone); Shift
  pins the inferred plane and the arrow keys still override it (Place
  Image takes the same inference). In face mode the shape's later points honour the
  snap the inference chip names (an endpoint IS the corner, not the ray's
  own plane hit a pixel off it). A Line chain on a face commits one cut per
  edge-to-edge stretch (a chain touching the boundary partway is not
  refused whole), a chain of interior points closing on its start imprints
  a sub-face, and placed-but-uncommitted face segments are drawn as the
  chain grows. Mixed-tool profiles close into regions in one shared
  sketch PER plane (ground drawing keeps its own single shared ground
  sketch, generalized rather than replaced); arcs optionally close as a pie
  or a chord segment (Alt cycles); polygon side count is typed (`Ns`) and
  persists for the session. Editing (select, delete, transform) is
  plane-blind and already worked on a rotated sketch. TapeMeasure and
  Protractor aren't limited to the ground plane either — a guide or
  measurement follows a hovered sketch's plane, an idle arrow-key plane
  lock, or (Tape Measure's parallel guides) the plane a picked face or
  edge actually lies in
- Exact-dimension typing works both mid-gesture (before the committing
  click) and right after the shape is already drawn, on every draw tool
  that takes a typed measurement: Rectangle's `W,D`, Circle's and
  Polygon's radius (Polygon's side count too, via `Ns`), Arc's bulge, and
  Line's last segment length. Typing a value and pressing Enter redraws
  the just-drawn shape in place — same anchor, same growth direction (and
  for Arc, same side of the chord and open/pie/segment closure) — as
  often as needed until the next click, Escape, tool switch, or other
  model change; undo of a resize removes the shape in one step. One
  shared engine (`app/src/tools/retypeWindow.ts`) owns the undo →
  recommit → restore cycle every tool's post-click window goes through;
  each tool supplies only its own geometry and typed-value grammar. The
  same engine backs the modify tools' own post-commit windows: Push/Pull
  and Offset redo their last distance (positive keeps the committed
  direction, negative flips it), Move redoes its last distance or copy
  (sharing its typed buffer with the ×N/`÷N` array window), Rotate redoes
  its last angle the same way, and Scale redoes its last factor or target
  dimension about the same pivot and driven axes
- Drawing right up to a face's edges: a profile drawn on a face whose
  sides run along part of the face boundary (a rectangle from one edge's
  midpoint to another's, or flush against an edge) imprints as one or more
  boundary-to-boundary chords — one undo entry — and the drawn region is
  a real face to push, pull, or paint. A profile that touches the edges
  of the face and its imprints at two or more isolated points (a circle
  inscribed in an imprinted rectangle, tangent to three of its sides; a
  circle whose rim was snapped onto an edge; a polygon with corners on
  edges; a shape crossing a chord), or that crosses an imprint's
  boundary or runs flush against it from outside, splits every face it
  crosses, exactly as the same shapes do in a ground sketch, with circle
  claims intact wherever a tangency falls on a facet vertex
  (the drafting engine, ARCHITECTURE.md 2.4, lowering the drawing into
  ordinary `split_face` cuts and re-traces; a piece outside an imprint
  cuts the face around it from the imprint's hole back to it). A circle overshooting a rectangle drawn a
  few microns short leaves a sliver, as on the ground; a sliver too small
  for a saved file to reopen (under about half a square micron) is
  refused, and the validator holds every operation to the same limit. A
  shape whose pieces join two different boundaries of one face (a circle
  overlapping two separate imprints, a rectangle from the face's edge
  across an imprint, a rectangle flush between two imprints) cuts that
  face with all of its pieces at once through a local re-trace
  (`Object::retrace_face`: several paths inserted or removed in one step,
  loops rebuilt by rotational order at each vertex, its own inverse with
  the arguments swapped), so the boundaries it joins become one, as on
  the ground; bridges exist only inside the re-trace, never in a
  committed face, so the file format is unchanged. A circle crossing
  the face's own edge or corner splits the face the same way: the lens
  is a face that keeps its circle on every whole facet, and the part past
  the edge is a free sketch closed by that edge. A loop touching at a
  single point, including a circle tangent to the face's own edge, is a
  face too: the face around it holds it as a hole sharing that one vertex
  with its outer loop (loops of a face may share isolated vertices, never
  an edge), and the disc and the face around it push and pull, through to
  the far side included
- Deleting lines on a face merges the regions on both sides, as on the
  ground, including a region and the face it was drawn in: selecting a
  corner leftover's rectangle edges (between a rectangle imprint and a
  circle inscribed in it) and pressing Delete dissolves the leftover into
  the face, rounding that corner of the rectangle; doing both corners and
  deleting the arc to the rest leaves one imprint with rounded corners
  that pushes and pulls with smooth (cylinder-claimed) walls. One Delete
  dissolves the whole run the two faces share — every run, when they meet
  along several (the merged face gains a hole where they enclosed
  another, or a hole splits in two; the re-trace again) — and the face
  holding the hole keeps its paint. Underneath, `merge_faces` accepts a run between an
  outer loop and a hole loop and `split_face` a cut from a hole loop back
  to the same hole — exact inverses, so undo and redo are exact — and
  every such run is a selectable chord imprint. Booleans keep every
  coplanar edge of a drawn shape, whichever loops it lies between, and a
  click selects a drawn line within 8 screen pixels whatever the model's
  scale, even where a guide lies along it. Cuts that close on one point,
  cross themselves, or repeat a point, and merges that would pinch a
  loop, refuse typed (the drafting engine instead re-traces, splitting a
  pinched walk into loops that share the vertex); undo and redo restore
  paint exactly
- A drawn circle or arc stays one shape after other shapes cut it apart
  on a face: its runs (chord runs whose edges are all facets of one
  circle; now the engine's curve units, keyed by geometry in
  `unit_key.rs`) list once in the outliner ("Circle",
  "Arc"), a double-click on any run (or the ground's triple-click)
  selects the whole curve, and Delete merges every run's faces in one
  undo entry (a draft delete of the curve's edge keys) — a circle drawn across a
  rectangle deletes back to the rectangle, and a delete that leaves two
  shapes touching at a point leaves them as two holes sharing it. Clicking one line
  whose two faces meet along several runs highlights every run Delete
  will remove. The Line tool on a face hands its whole chain to the
  kernel (now one `Document::draft` gesture, ARCHITECTURE.md 2.4):
  it is cut wherever it meets an edge, crossing imprints and running
  from the face's edge across them, as one undo entry, and a dangling
  end (refused at the time) is now kept as a loose owned edge. This completes the face-arrangement
  program: shapes and lines drawn on a face split, merge, and delete as
  the same lines do on the ground. Known limits: a
  facet a crossing cuts mid-way keeps no circle claim in the solid (the
  drafting engine still lists it with its circle), so pushing that
  region leaves one flat wall among the cylinder's; and a sliver under
  about half a square micron is refused. The Arc tool's open arc, and the API's
  face-mode `draw_line` and open `draw_arc`, go through the same drafting
  engine, so they cross shapes on a face too; `hew.solid.imprints`
  lists cut-apart curves in `curve_units`
- Push/pull past a co-facing wall: an inward push whose swept column would
  cross the plane of a face pointing the same way (the bowl of a P-shaped
  slab pushed past the stem's end) routes to the subtract path and carves
  the material, and an outward pull whose prism would grow past such a
  wall ahead of it (the stem pulled past the bowl's end, or a notched bowl
  pulled back out) or into a wall facing it across a gap (one arm of a U
  pulled into the other) routes to the union path, where the flat
  translate-and-build path could only refuse; a pull exactly flush with
  the wall merges the two faces into one. A face whose boundary vertex is
  shared with a face it has no edge in common with (the exposed end of a
  thick part where a thinner part joins it — its corner is also the thin
  part's wall corner) takes the boolean route at any distance, since a
  rigid translate would bend that wall off its plane. The boolean's
  assembly splits every edge at any result vertex lying on it, so a seam
  whose continuation onto a neighbouring face degenerates to a point (a
  prism resting on a step's top and reaching past the step's far edge)
  still welds watertight
- Editable shapes on faces (imprints): a shape drawn on a solid's face and
  not yet pushed or pulled is selectable (click inside it; the object is
  one click away anywhere else), fills and outlines in the attached orange (part of the solid's face; a free sketch is blue), lists in
  the Outliner under its object as a named child row (Circle, Rectangle,
  Shape, Arc shape, Line, Arc, Edge shape), and moves, rotates, scales, and deletes ON its
  face — Move constrains the drop to the face's plane. (Superseded by
  unified drafting, above: on a face every transform arranges as it does
  on the ground, a shape may slide off the edge and split, and copies are
  supported; what stays refused is tilting a shape off its plane.) Shapes
  drawn up to a face's edge (chords) select by their line and delete the
  same way. A shape drawn, offset, moved, or scaled all the way around
  other shapes, bosses, recesses, or holes adopts them — they move with it,
  and deleting it leaves them on the face; a shape holding a boss or
  recess cannot slide until that is undone. Circles and arcs drawn on a face keep their
  circle on the solid's edges (per-edge `Edge::curve` claims through
  `split_face_with_curves` / `split_face_inner_with_curves`, an arc's
  facets claimed and its closing lines plain), offer center and quadrant
  snaps like curves on the ground, and push or pull into smooth walls
  exactly as from a ground sketch: every wall-raising path (boss,
  wall-building push/pull, push-through) stamps a wall PER FACET from its
  base edge's claim, gated on the edge being a genuine facet rather than a
  secant (`chord_facet_ok`), so an arc closed by a line raises smooth arc
  walls and one flat wall; a push carries each claim with the rim it moves
  and hands it to the fresh rim of a wall it builds; the Offset tool's face
  commit imprints each offset arc facet with its concentric circle, so a
  recessed inset's inner curve is smooth too. Recovered structurally from the solid's topology on
  demand (no file-format change). The legacy imprint planner that once did
  this (`face_features`, `transform_sub_face`, `transform_chord` /
  `dissolve_imprint`) is gone: the drafting engine (ARCHITECTURE.md 2.4)
  lists and edits the same drawing as plane units, and the
  `hew.solid.imprints` / `*_imprint` API commands run over it. Instanced geometry gets the selection highlight but no orange
  fill overlay
- Drawing-like sketch editing: lines are selectable and deletable
  (merging the regions they separated), a drawn arc or circle selects
  and deletes as one curve, and each connected shape is an independent
  island — selected, deleted, and transformed (moved, rotated, scaled;
  out-of-plane rotations included, detaching the shape into its own
  sketch when it shares one) without touching anything else. A shape
  moved, rotated, or scaled onto other shapes, or a point dragged onto
  other lines, joins them exactly as drawing it there would — lines split
  and merge and regions re-form, on every plane, in one undo step — and
  line contact is measured in metres at every scale
- Unified drafting: the ground and every solid face are drafting planes
  driven by ONE engine (`kernel::draft`), so a gesture performed on a box
  top leaves the same drawing as on the ground (the app's parity E2E
  holds both to equal units, curve runs, edges, and region rings). After
  every edit each edge is stored where it can be: over a face and part of
  a region the solid can hold, as that face's topology (an imprint, as
  before); over a face but loose (a free end) or part of a region the
  solid cannot hold (a sliver), in a sketch OWNED by that object
  on that plane; over the void, in the plane's shared world sketch. A
  loose line on a face is kept; a shape straddling a face edge splits into
  a face part (Push/Pull grows or carves the solid) and a free part
  (Push/Pull extrudes a new object), in either order; moving, copying,
  arraying, scaling, rotating, offsetting, or dragging a vertex of a shape
  on a face arranges exactly as drawing it there would, and the ground's
  "would make lines cross or merge" refusal is gone too. Selection works on
  plane units (shape, curve run, line, region) on every plane, and a
  solid's own edges are never drawing: they cannot be moved or erased by
  drawing edits. Owned drawing follows its object (push/pull ride,
  booleans, slices, duplicates, explodes, and library copies copy it;
  deleting the object takes it, undo brings it back) and reconciles after
  every action. A region a face cannot hold says why on Push/Pull. A
  clicked value equals the displayed one for free picks (snapped picks
  keep their exact point). A regular polygon drawn on a face keeps its
  polygon mark on the face's edges, so it selects as one polygon and
  offers its centre snap there as on the ground, and a shape slid past a
  face's edge and back is whole again (the split points it gained where it
  crossed are healed once they sit off every solid edge). Two polygons
  inscribed in one circle stay two polygons (their curve keys hash their
  corners). A selection transform or copy that moves objects together
  with drawing (Select All, then Move) is one undo step and all-or-nothing
  (a wasm history bracket over the kernel's transaction): drawing that
  rides on a selected object's face moves with it, and the free half of a
  shape straddling that face's edge is redrawn where it lands. Native format
  v17 adds the optional `owner_object` on sketches, and geometry buffer v7
  the per-edge polygon mark (`docs/dev/HEW_FILE_FORMAT.md`); the wasm and
  Hew API surfaces address drawing by plane context and unit key
  (`plane_units`, `draft_*`)
- Detached shapes: Ctrl/Cmd on a draw tool keeps the shape apart from the
  plane's drawing (`Sketch::detached`, sketch-only draft scope,
  `draft_begin_detached`, API `detached`); Push/Pull on it makes a new
  Object; the Getting Started cutter is drawn this way. A detached shape
  takes part in no arrangement with ordinary drawing, and never closes a
  face against a solid's face edge, except that an Object edge in its
  plane whose two ends are corners of the detached shape closes it; a
  detached line closes a face with other detached lines and with such
  Objects' edges only. Detached lines that share corners close a face;
  lines that only cross do not, and a crossing line neither splits nor
  blocks a face. Every region the connected detached lines bound becomes
  a face, not only those the newest line bounds (a gable split by a post
  fills both halves); a region a loop of the connected set already holds
  is never made again, so a face is final to later detached gestures (a
  line drawn across it stays loose), while a separate shape never blocks
  one (nested shapes both fill). A detached loop on an Object's face is a
  loose face there. A curve taken into a face keeps its curve chain
- Faces from lines on any planes: a loop of straight edges on one plane
  is a face whichever sketches hold its lines. Edges that lie in a plane
  but belong to another sketch or to an Object are foreign edges to that
  plane's lift (fixed, never moved), and a close pass after every draft
  and sketch move/copy finds the planes the edit's edges close (the plane
  through the edit and each edge meeting it), writes the face into that
  plane's free sketch, and takes the edge out of the sketch it was drawn
  in if it bounded no region there. Three lines up to a corner's apex
  fill a triangle on an inclined plane; a wire cube fills as it
  closes. Exact planarity: lines that miss a plane make no face, and
  Line says so. Tag-hidden and closed-group sketches take no part. No
  file format change
- Automatic solids and growing a solid: when an edit leaves flat faces
  that enclose a space with no gap, they become one Object in the same
  undo step (`document/cells.rs`: refine, rims, prune, cell walk, build
  through `from_polygons_impl`, Watertight and `validate()` or nothing).
  A shape closed against one solid's faces (a roof on a box) grows that
  solid: new id, name, tags, and parent carried over, imprints kept.
  Detached faces give a separate Object (a detached gable closes against
  a box's top edge when the edge's two ends are corners of the shape);
  a cell inside a solid does nothing; two rooms sharing a wall are two
  solids; a union that refuses leaves a separate Object and a toast.
  API: `faces_closed` and `solid {object_id, grew}` on the nine drawing
  and unit-edit commands. The wasm sketch moves and copies
  (`transform_sketch_islands`, `copy_sketch_islands`) return the draw
  calls' report JSON, so a Move, Rotate, Scale or Copy that closes a
  face or a solid reports it (moving a whole sketch, as opposed to part
  of one, reports nothing yet)
- Push-through keeps the Object's name and tags on every result piece
  (an inward push that severs the solid gives each piece both)
- Push/pull to extrude a closed profile into a solid, with a live preview
  that borrows its depth only from references OFF the face's own plane (a
  face's own edges and the drawing axes are never a depth), and picks a
  drawn region standing in front of a solid's face over the face behind it.
  A sub-face whose corners are the solid's own corners (a face split corner
  to corner, a triangle drawn from a box's corners) pushes in like any
  other: the wall built along the cut lands its side edges along the
  solid's existing edges instead of spiking the neighbouring face, and the
  through-cut detection recognises such a face's footprint (its vertices
  all lie on the far wall's outline) so a full-depth push carves the prism
  out rather than refusing as "would remove the whole object";
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
  dissolved on the result (a union of two flush boxes reads as one box).
  A subtract whose cutter sits apart from, or only touches, the target
  refuses with a typed `NothingRemoved` and leaves both solids in place
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
- Make Unique (detach an instance into its own copy, keeping the
  definition's name — or taking the instance's own name if it has one; the copied
  members keep their names, tags, and attributes) and Explode
  (bake an instance back into ordinary geometry; a nested member
  surfaces as an ordinary instance, its geometry still shared)
- Copy/Cut/Paste and Paste In Place of component instances stay linked
  within a document: the clipboard bytes carry a `hew.clipboard` origin
  stamp per definition (the copying window's document token plus the
  source definition's stable id) and the paste links each stamped
  definition back to the live original (`Document::extract_clipboard_item`
  / `InsertOptions::origin`), so the pasted instance places the same
  definition as Move+copy and Place Copy do — several copied instances of
  one definition share it, nested definitions link per definition, a
  mixed selection pastes its plain objects as independent copies, undo
  removes only the pasted instances. A paste under another origin
  (another window, or after File ▸ Open/New in this one) copies the
  definition as before, with repeat pastes of the same copy reusing it by
  clipboard provenance; Make Unique breaks a link on purpose
- Linked copies in the app: Move, Rotate, array, and Paste copies are
  linked by default (Settings ▸ Editing ▸ "Copies are linked"); tapping
  the copy key cycles Move, Copy, the other kind of copy, Move, and the
  cursor badge, hint, and readout follow (`+`, `+ independent`, `+ linked`).
  An array repeats the kind of copy it follows. Paste into the same
  document links to the copied nodes while they are still live (the
  clipboard records the document and window it came from); Edit ▸ Paste
  independent and Paste linked force either form. A placement of a linked
  set reads in the Outliner by its own name, else the shared name, else its
  part's name, else "Copies", with a link glyph and a ×N count (none for a
  set of one) and a row "Copies" menu: Unlink, Unlink all, Make component
  (promote: asks for a name), Select all copies, Rename. The same actions
  sit in the Object menu, the contextual dock, and Object Info ("Linked
  copies: N", "Name (all copies)", "This copy"). Subtract on a placement
  cuts through the shared shape (a strip offers "Cut all N copies" or
  "Only this one"); Union and Intersect unlink the placement first, as one
  undo step labeled "(unlinked)", and say so. Slice and Push/Pull on a face
  reached through a linked placement edit the shared shape with no context
  to enter. Link identical copies (joining look-alike loose Objects into
  one set) is not built yet: it waits on the query that fingerprints a
  shape.
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
- Follow Me honors the selection exactly as the path: a single selected
  sketch segment sweeps only that segment, rather than expanding to its
  whole connected run; triple-clicking a segment with the Select tool
  selects the whole run first when that's what's wanted. Clicking a line
  from inside the tool (skipping preselection) still picks up its whole
  connected shape
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
  vector, `3/` (or `/3`) places 3 copies dividing the distance, and two
  counts (`4x,3x`, `x4,x3`, `*4,*3`) lay a grid of (4+1) by (3+1) slots
  with the original at a corner, one move length apart in both
  directions (the second direction is the horizontal perpendicular, or
  the arrow-key axis; an arrow key while the grid is hot re-lays it; the
  grid follows the copy kind, and a product over 1,000 copies is refused;
  guides copy along one direction only) —
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
  dimensions. Dragging a grip past its anchor mirrors the selection,
  as SketchUp's does; a mirrored solid stays right side out. The grip under the cursor lights
  together with the point it scales about (its opposite, or the center
  with Ctrl), joined by a dashed line, and the pair stays through the drag.
  Move (base point to destination) and Rotate (center to the first point)
  draw the same dashed line, in the construction guides' gray or in the
  color of the drawing axis it lies on. Move, Rotate, and Scale all extend
  to a whole mixed selection — one Object, a Group, a component instance,
  or any combination — as one shared transform and one undo step; a scaled
  component instance carries the (possibly non-uniform) scale in its own
  pose, leaving the shared definition and every sibling instance untouched
- Flip: three axis squares through the selection's bounding-box center
  (or a hovered model face) mirror the selection in one click and one undo
  step, with a hover ghost; objects and drawing bake the mirror, instances
  take a mirrored pose. The copy toggle makes a mirrored copy, linked by
  default (a placement with a reflected pose). Object ▸ Flip Along ▸ Red /
  Green / Blue flips in place without a tool change. Kernel
  `Transform::reflection`, `Document::world_bounds`, `flip_matrix`,
  `flip_nodes`, `flip_copy`; wasm `flip_matrix`, `flip_selection`
  (recorded). No shortcut, as in SketchUp
- Part Axes (C12): View ▸ Part Axes draws the frame of the selected or edited
  component (an origin mark and red/green/blue arms of constant screen
  length) and, for a selected Object or group with a `hew.part.frame`
  override, a triad at that frame; at most 20 at once, hidden from prints.
  Off by default; an app-wide setting kept in localStorage as
  `hew.settings.showPartAxes` and synced across windows; menu id
  `view-part-axes`. A component nested in another draws only when its owner
  has one placement; hidden nodes draw none. Limit: override frames of
  Objects inside an entered component are not drawn. App-only
  (`viewport/partAxes.ts`). No shortcut
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
- View die (view-die.md §2-§6): a corner-cut die in the editor viewport's
  top right that turns with the camera (click a face or corner for a
  standard view or corner view, drag to orbit), with four plane pips under
  it (red, green, blue, custom). A pip shows the inferred plane when
  filled and a locked or pinned plane with a padlock; a click is the mouse
  form of the arrow-key lock, and the custom pip pins the tracked hover
  plane like Shift. Each draw tool exposes a `DrawPlaneState` from
  `drawPlane.ts` that the die reads. The Orbit/Top/Iso/Front chips
  (`ViewportHUD`) are removed; the measurement box defaults to
  bottom-right. Not shown in Shop mode.
- Standard camera views (Top, Front, Iso, etc.), orbit/pan/zoom navigation,
  and a Parallel Projection toggle (true orthographic, not a narrowed
  field of view) that keeps every screen-constant widget (transform grips,
  protractor/rotate disks, section-plane and slice previews) correctly
  sized in both projections; a typed Field of View control; Zoom Window
  (drag a rectangle to frame it)
- Orbit and zoom pivot on the geometry under the cursor: a middle-drag
  orbits around the surface being pointed at and each scroll step covers
  a fixed fraction of the distance to it, with a near-absolute zoom floor
  (a millimetre at model scale) and clipping planes that follow the eye —
  so a small detail of a large model is reachable, stays on screen while
  orbiting, and renders without z-fighting
- Shift pressed or released mid-drag switches an orbit drag to a pan and
  back, for the Orbit tool's left-drag, a middle-button orbit under any
  tool, and the Pan tool's left-drag; holding Ctrl (Windows/Linux) or ⌘
  (macOS) during an orbit or pan puts the camera in a precise 1:1 mode —
  no inertia (the coasting tail and the slight lag while moving) for as
  long as the key is held, and pressing it while the camera is still
  coasting stops it dead
- Position Camera, Look Around, and Walk: first-person camera placement
  and walkthrough navigation (click or drag to stand and look, mouse-look,
  and forward/turn/strafe movement), sharing one session eye height that's
  typed like any other measurement
- Fly: hold the right mouse button under any tool to fly through the
  model in perspective — the mouse looks, W A S D move along the view
  direction and sideways, E and Q climb and descend along world Z, the
  wheel sets a session speed, Shift sprints — with a short glide on
  starting and stopping; the active tool keeps the left button and its
  gesture throughout, and its preview follows the moving view. The cursor
  is hidden and frozen for the flight (the Pointer Lock API where the
  browser or webview grants it; the macOS shell warps, freezes and hides
  it itself, since WKWebView denies the API) and comes back where it was.
  Camera ▸ Fly latches the same flight on with no button held: the mouse
  or a trackpad finger looks, the wheel, a two-finger scroll or a pinch
  sets the speed, Esc exits; without a lock a two-finger scroll looks and a
  click on the view locks. Right-drag no longer pans: pan is
  Shift+middle-drag or the Pan tool
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
  instance's pose, always the innermost frame, with every other placement
  of the definition staying visible and showing the edit live through its
  own pose (the live-sibling rule, ARCHITECTURE.md 2.11). Frames nest
  per container level and close LIFO (Escape, double-click outside),
  folding edits and mid-session creations back into the container; both
  entry paths — viewport drill-down and Outliner double-click — produce
  the same breadcrumb, with every level on it marked as editing. A
  component session requires the instance's pose to be a similarity
  (uniform scale; a mirror is allowed); a group-nested placement now opens through
  its group's session, and a non-uniformly-scaled placement, or one entered
  directly inside a group, falls back to the previous
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
- A face's texture UV frame moves with the face under an object transform
  and a session pose (instance pose, shear, direct session pose, two
  placements of one component), so an imported textured face no longer
  slides when moved, rotated, or opened and a resized picture stretches
  instead of tiling. A transform that would squash a frame past what it can
  hold is refused before anything moves; a frame that was already invalid
  does not block a move
- Loops of a face may share isolated vertices (never an edge, and each loop
  stays simple), so a circle tangent to a face's edge at exactly one point,
  two circles touching at a point, or a circle touching an imprint's corner
  commit as a disc plus a parent face whose hole touches its outer loop. The
  validator reports a non-simple loop and an edge with the same face on both
  sides as typed errors and checks holes edge by edge (each hole edge cut at
  every outer vertex lying on it, every piece inside the outer ring); an
  imported object may graze its ring within the import tolerance. Inside
  tests treat a point on any of a face's loops the same on every edge, so
  cut-side hole assignment, STL cylinder re-facing, and loose lines carried
  through a boolean, slice, or push no longer depend on the cut direction or
  the plane's projection axes
- Click-drag-release as well as click-click for every tool with a start
  and an end point (Line, Rectangle, Circle, Polygon, Arc, Push/Pull, Move,
  Rotate, Scale, Offset, Tape Measure, Dimension, Protractor): a press that
  takes a participating tool from idle to mid-gesture, released past the
  Select tool's drag threshold, is delivered as the tool's second click
  through the ordinary click path at the release snap, so commits, typed
  values, inference, and locks behave as after a click; a release inside the
  threshold stays a plain click. A typed Enter or Escape spends an armed
  release, other pointers are ignored while a drag is armed, and a drawing
  one-finger touch drag no longer also orbits. A dragged Line is one segment
  and ends the chain
- Clicking a line chain's last placed point again finishes the chain at any
  pace, with no toast, when the click lands within a few screen pixels of
  the point; a snap that collapses onto it while the cursor is elsewhere is
  still refused with a message that says what to do. A trailing browser
  double-click after the finish is consumed rather than entering the object
- Keys typed into a text field or a modal dialog never reach the active tool
  (one predicate gates tool capture, bare-key shortcuts, and the final key
  forward; a dialog counts as modal unless it says `aria-modal="false"`)
- Push/pull's preview is tinted per mode through two theme tokens, orange
  for a plain pull, blue for extrude-as-new, red for a cut, in light and
  dark; toggling Ctrl mid-gesture changes the hue, and the bare arrow drawn
  with no swept prism follows the mode. A free-sketch region always extrudes
  into a new object, so its ghost is the new-object blue in both directions
- Draw color language (`--draw-free`/`--draw-attached` and their fill tokens):
  drawing that belongs to no solid is blue and pushes into a new object;
  drawing that is part of a solid's face (an imprint, an object-owned sketch
  edge) is orange and pushes the solid itself. The viewport colors each edge
  by its store (`Scene::sketch_owner_object`); the rubber band is orange only
  when the shape would join a face, and Offset's ghost takes its source's color

### Inference & precision

- Snapping to endpoints, midpoints, edges, faces, and locked axes, with
  on-screen cues for every snap. Where a drawing axis — or the soft axis
  through a gesture's anchor — crosses an edge, the crossing itself snaps
  (an intersection carrying the edge and the axis), so dragging along an
  axis reaches the far edge of a face instead of sliding past it on the
  axis line. Where edges of two separate owners (Objects, component
  placements, sketches) truly cross in 3D, the crossing snaps too, as an
  intersection carrying both edges: separate Objects never merge, so
  nothing else puts a point there. Only the edges through the pick cone
  are paired, at most 2,000 pairs a query (past that the query offers no
  crossing and counts the skip), and screen-space crossings of edges that
  only overlap on screen are not offered. Where an edge of one owner passes
  through a face of another (a panel's edge entering the wall it is set
  into, with nothing imprinted on the wall), the piercing snaps as the same
  intersection, carrying the edge and the pierced face: the edge must cross
  the face's plane strictly (an edge ending on the face is its own
  endpoint; an edge lying in the plane has no single point) inside the
  face's outer loop and outside its holes, curved surfaces counting as
  their facets. Edges through the pick cone are paired with the faces the
  cone reaches, nearest pairs first, at most 2,000 pairs a query; the
  pairs a capped query leaves untested are counted and traced, like the
  edge pairs. Where a face of one owner meets a face of another (a slab
  passing through a wall draws four such lines on the wall's face), the
  stretch of the two planes' intersection line inside both faces — outer
  loops, outside their holes — is a virtual edge: the cursor slides along
  it as On Intersection Line (the same pick reach and rank as a real edge),
  its middle is a Midpoint, and its ends are the piercings and crossings
  the intersection snaps already name. The line runs through both faces'
  interiors: a face that only touches the other's plane along its own edge
  (a box flush against a wall) makes no line, only that real edge, decided
  with a named tolerance so a rotated pair behaves like an axis-aligned
  one. On a chord facet of a curved surface (a slab through a cylinder)
  each facet's piece is the line but offers no middle. Parallel and
  coplanar faces make no line, one owner's own faces never pair, and a
  selection pick is never offered one (two faces at once are nothing to
  select). Both faces travel with the snap — reported as a meeting line,
  never as a pick of either face, so no tool adopts one face's plane or
  part from it — and the cue draws the stretch itself, since nothing in
  the model does. The faces the cone reaches are paired across owners
  front to back along the ray, so faces hidden behind the one in view
  cannot crowd out the line in view; at most 2,000 pairs a query, each
  rejected cheaply when its line clipped to both faces' boxes misses the
  pick cone, counted and traced past the cap like the other two walks;
  the piercing and meeting walks share one cone-face pool, built only
  when faces of two owners have boxes in the cone (drawn regions carry
  boxes of their own). Candidates
  within a pixel of each other on screen rank by depth, nearest first, so
  a stack of corners seen from a nearly-vertical Top view — top plate,
  sole plate, slab — resolves to the one in front rather than to
  whichever the camera's slight tilt happens to lean toward.
  Named points (endpoints, midpoints, centers, quadrants) capture from
  1.75 times as far as lines, so a corner on an edge or axis wins near it,
  and the soft axis is capped on screen to an edge's pick radius rather
  than reaching by angle alone; Select's clicks and hover get no extra
  point reach, so a small face's middle still selects the face
- A snap to a point is that point: Line no longer drops a snapped
  corner, midpoint, or guide point onto the ground, and the first click of
  any draw tool on a point in the air starts there, on the camera-facing
  axis plane through the point (when it is nearer the eye than the ray's
  hit on the chosen plane; a hidden point still drops with "projected").
  Later clicks of Line honor a named point off the chain's plane and
  re-home; Rectangle, Circle, Polygon, and Arc drop it onto their frozen
  plane and say "projected". An arrow lock holds the plane for all of
  them; a Shift pin holds the plane's orientation, and the first click
  through a named point off it starts the shape at that point's height
  (later clicks are the "projected" ones). The up arrow forces the ground
- From Point reference lines on the drawing plane
  (`app/src/tools/referenceLines.ts`, shared by the five draw tools): a
  reference point offers two lines through its foot on the plane, along
  the two directions a Rectangle's sides take there, and the crossing of
  two such lines snaps. Reference points are the gesture's own points,
  the corners of the shape the gesture started on (up to the 64 nearest
  the anchor, read with the wasm call `snap_owner_points`), and up to
  four points the cursor rested on for 300 ms (cleared on a tool change
  and on Esc from idle). A reference line replaces only a weak engine
  answer (plane, ground, on-face, on-axis); a real point or edge wins.
  The cue is a dashed line from the foot, a dot at the foot, and a tie
  from an off-plane point to its foot; the chip reads "From Point"
- Analytic curve snaps: the exact center, quadrant points, and
  tangent-from-anchor points of drawn circles and arcs, honoring each
  arc's actual angular range
- Tray sections: a long section can be given more or less height by a drag
  handle or the Up/Down arrow keys; sections keep their automatic height
  until one is resized
- Construction guides — guide lines and points that participate in
  snapping and are saved with the model. Move and Rotate transform a
  guide like any selection (axis locks, typed and retyped values), and
  their copy toggle copies it, ×N / /N arrays (rows, fans) included,
  each transform or copy one undo step. With nothing else selected, a
  click on a guide starts transforming that guide; a guide taken that
  way is selected only for the gesture
- Tape Measure (point-to-point distance, or drop a parallel guide at an
  offset — sourced from an edge (a plain object's, a group member's, or a
  component instance's), a sketch line, the line where two Objects' faces
  meet (On Intersection Line), a world axis, or another guide
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
- Blue-axis lengths: with a line anchored on the ground and no arrow lock,
  a snap straight above or below the anchor keeps its height as an implicit
  blue lock and the segment re-homes onto a vertical plane through the
  anchor (the path the arrow lock uses), so hover, click, and typed length
  agree. The Arc tool's chord stage anchors at its first point when no
  arrow lock or Shift pin holds the plane, offering the same red, green, and
  blue inferences Line does; a chord straight up or down moves the gesture
  onto the vertical plane through the chord, and Esc from the bulge returns
  to the chord on the ground
- Arrow keys after a face-anchored first click: with Rectangle, Circle,
  Polygon, or Arc anchored on a face, an arrow moves the gesture onto the
  locked plane through the anchor (the same result as pressing it before the
  click), the same arrow again or Down returns it to the face it started on,
  and a held arrow no longer toggles the lock on every autorepeat
- A crossing outranks a midpoint and reaches as far as other named points;
  Tape Measure treats a crossing as a point (the first click measures from
  it, and the hover hint says so) while edges, sketch edges, axes, and
  guides, and the line where two Objects' faces meet (not its middle or
  ends, which are points) still start parallel guides, and a crossing never anchors a resize
  scoped to an open group or component, since the snap names only one of its
  two edges
- X-ray and Wireframe render modes (View menu on the web and native menus,
  and the command palette; one mode at a time, choosing the active one
  returns to Shaded) with the pick policy set once on the inference scene
  as a face policy, so every tool obeys it without a per-call flag. Shaded:
  faces occlude, only visible geometry snaps or picks. X-ray (faces at 0.4
  opacity, depth writes off): every edge, endpoint, midpoint, crossing,
  center, and quadrant is snappable and selectable, faces stay targets with
  the nearest along the ray winning off an edge or point. With an editing
  scope open (a component or group session, an object context, or a legacy
  in-context instance), Shaded and X-ray picks land on the nearest face,
  sketch region, sketch edge, sketch vertex or sketch inside the scope and
  skip what lies outside it (the editing view draws the outside
  translucent, so what is in scope can be seen and so picked); the scope is
  the renderer's lit set plus, for a session's own scope, the frame's own
  drawing (`Document::session_frame_sketches`: a component frame's
  definition drawing and lines drawn since the open, a group frame's held
  drawing and lines drawn since the open), pushed to the kernel by
  `Scene::set_pick_scope(objects, instances, frame_sketches)` /
  `clear_pick_scope` (`PickScope` in the inference crate); empty lists are
  still a scope (a component with only drawing left), only
  `clear_pick_scope` clears; an object context on top of a session passes
  `frame_sketches = false`. With nothing in scope under the cursor the pick
  is the nearest of any owner, so the tools' refusal hints still run;
  Wireframe, the top level, snapping and the geometry-only camera picks
  ignore the scope.
  Wireframe (no faces, every edge): no On Face snaps, face hover, selection,
  push/pull, or paint; drawing lands on the ground, a locked plane, or an edge
  or point.
  The orbit pivot and in-session sketch selection use geometry-only picks
  (`pick_face_geometry`, `pick_sketch_region_geometry`). The five draw tools
  that drop an off-plane snap onto their plane opt in to hidden points
  (`see_through_points` on the snap call): a hidden edge or corner adopts
  the face it lies on in X-ray, any other hidden point is dropped onto the
  clicked face's plane with a projected cue. The mode is per window, not
  saved in the file, reset by New and kept by Revert; `set_render_mode`
  refuses a bad string as `BadRenderMode`. A mode switch re-resolves the cue
  at the parked pointer, window guide/axis/render-mode switches survive a
  document load, and edges draw light gray on the dark theme in the
  see-through modes. The raster print follows the mode; the scaled line-art
  print falls back to a bitmap with a note while a mode is on
- Standard views (Top, Bottom, Front, Back, Left, Right) only turn the
  camera about the current orbit target: target, distance, and orthographic
  zoom stay, so the on-screen scale is unchanged; the view limits still
  resync from the model's extents. Iso still frames the model. The live
  API's standard-view directive follows, and a standard view cancels a
  running scene-transition glide
- A Precision choice under Units, per person and per display format: zero
  to six decimals for meters, centimeters, millimeters, and decimal inches;
  1', 1", 1/2" down to 1/64" for fractional and architectural inches; and
  Exact for every format (the fewest decimals within a micrometer for
  metric, the smallest power-of-two fraction up to 1/64 otherwise, decimal
  inches to five places when no fraction is exact). Defaults are each
  format's historical rounding. A readout rounded to the chosen grid carries
  a leading `~` only when rounding changed the value; typed input accepts and
  ignores a leading `~`. The Measurements box, Tape Measure, Protractor,
  Object Info sizes, printed pages, and placed Dimension text all follow it
  (an overridden dimension text stays as typed), editable fields seed at
  Exact so a rounded display never becomes a rounded value, and the SketchUp
  writer and reader drop the prefix from dimension text

### Objects & organization

- Document outliner with click-to-select and double-click to enter an
  object's editing context, with the rest of the scene dimmed for focus
- Move an object, group, or component instance into a group, or back out
  to the top level, without touching its geometry: drag its Outliner row
  onto a group row (or the root Model row), or Cut it, edit the group, and
  Paste In Place. One labeled undo entry either way (`reparent_nodes` /
  `hew.group.reparent`); refused typed for a group-into-itself-or-descendant
  cycle or
  while a group/component session is open
- Selection path: a strip at the top of the viewport reads the containment
  chain of the selection from Model down to the thing clicked (`Model ›
  Kitchen › Base Cabinets › Cabinet 2 › Door`), replacing the "Editing X"
  pill; open session frames appear in the same strip with an `editing`
  chip. A crumb click re-targets the selection to that level without
  opening a session; a double-click opens it (the Outliner row's
  double-click path). A member of an unopened component instance ends the
  path as a non-selectable crumb. The Select tool's inference tip names
  the hovered thing and its parent after a 400 ms dwell, and Object Info
  shows the same containment line under Type (docs/design/selection-path.md)
- Tags for organizing and toggling visibility of groups of objects,
  deletable without touching the geometry that carries them; clicking a
  tag selects everything it carries, and a tag is renamed in place
  (Finder-style click-twice or double-click, and `hew.tag.rename` from the
  API) with every object and nested tag path following the new name
- An Object Info panel for renaming, tagging, and checking an object's
  solid status; with several objects selected it tags them all at once,
  showing the tags they share. It also reads Volume, Surface area, and
  Size in the display units (summed over a selection, a group, or an
  instance; a dash with "Not closed" when any leaf is open), from the
  wasm `measure_json`, which shares `hew.query.measure`'s builder
- A Components panel listing every component definition with its live
  instance count; rename and delete (deleting a definition removes every
  instance that places it, confirmed when it has any); Purge Unused
  removes every material and component definition nothing references, as
  one undoable step, and reports what it removed
- `hew.entity.rename`/`hew.entity.delete` reach materials and component
  definitions the same way they reach any other entity; `hew.doc.purge_unused`
  is the API counterpart of Purge Unused
- Sorting and filters: the Materials palette and Components panel list by
  name (case-insensitive, numeric, so "paint 2" precedes "Paint 10"). The
  Outliner has a sort control (document order, the default, name, kind:
  objects, groups, components, sketches, then drawing) applied at every
  level including open sessions and free sketches; rows keep their
  document-order index, and the choice is per person, stored with the other
  View settings. The Tags pane gets the same filter box as the other panels
  (a parent stays, dimmed, when only a child matches), and one shared
  comparator serves every panel and the tag tree
- Object Info's tag chips jump to the Tags pane (tray restored, tag selected,
  parents expanded, scrolled into view), and a Materials row lists every
  distinct material on the selection (own paint, per-face paint front and
  back, and for a group or component the recursive union including hidden
  contents) as swatch-and-name links that select and reveal the material in
  the Materials pane without arming Paint; nothing painted reads "None" and
  past twelve entries "+N more" expands the rest. The read-only wasm
  `object_materials` query backs it without a tessellation per object

### Materials

- A document-wide material palette: flat colors and image textures, painted
  per face
- An object-level default material so newly extruded or grown faces inherit
  a sensible color automatically
- Materials survive splitting, imprinting, and boolean operations
- Physically based materials: metallic and roughness factors (Metallic and
  Roughness sliders, `hew.material.set_pbr`) and metallic, roughness, normal
  and occlusion maps carried from a `.skp`, approximated in the viewport with
  Phong shading and written back on a save to the 2021-and-later container
- Per-material opacity (glass, scrim, etc.), adjustable from the Materials
  panel and applying uniformly to flat colors and textures alike
- Alt-click to sample a face's material into the current swatch, and
  Shift-click to replace that material everywhere in the document (or, with
  Ctrl/Cmd, only within the clicked object) in one undoable step
- Position Texture: drag a texture's corner pins on a face to move, rotate,
  scale, and shear it, or type an exact angle or scale factor
- From the Hew API, `hew.material.create` makes a texture material from an
  image (inline bytes, or a file `hew-cli` reads) and its real-world tile
  size, and `hew.material.position_texture` pins a face's texture the way
  the tool does; the query commands read both back. `hew.material.replace`
  is the Paint tool's Shift-click, and `hew.library.save {material}` /
  `hew.library.insert` save a material to the Library and add it back to
  the palette
- A searchable material palette, with the add-color and add-texture panels
  collapsed until needed
- Rename and delete a palette material from the Materials panel; deleting
  a material used on faces or as an object's default unpaints them (with
  a confirmation naming how many), and Purge Unused clears out whatever
  nothing references

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
- The on-disk layout (folder resolution, file naming, item metadata
  parsing) is owned by one crate, `crates/library`, shared between the
  Tauri shell's `library_*` commands and `hew-cli`/MCP's `hew.library.*`
  namespace (docs/agents/HEW_API.md §8.1) — `list`, `describe`, `insert`,
  `save`, `remove`, `update_meta`, with the same idempotent-re-insert and
  provenance-stamping semantics as the UI's own Library browser. `--live`
  pre-resolves every library command on the CLI's own side (HEW_API.md
  §12.1), since the desktop app's live host has no filesystem of its own

### Annotations

- Linear and radial dimensions and leader text, anchored to the geometry
  they describe so they follow it through moves, rotations, and scales — a
  transform that would distort a radius detaches the annotation visibly
  instead of reporting a wrong number
- Annotations save with the document and read at a constant on-screen size,
  staying legible at any zoom
- Dimension arrowheads are a fixed size on screen tied to the label text
  height and capped so the two heads never cover more than about 60 percent
  of the line, built in the per-frame layout pass at each tip's own depth for
  linear and radial dimensions; the same pass keeps an outside label's tail
  out of the text, puts a radius label past its leader end, keeps a small
  circle's center cross inside it, and limits extension-line overshoot to a
  few pixels. The raster print uses the same layout; the vector print sizes
  arrows in paper millimeters and drops any head seen end-on. Label textures
  are tagged sRGB so the light theme's ink stays near-black

### File format & persistence

- An open native file format (zip container, JSON manifest, binary geometry
  buffers) with deterministic, byte-stable output
- Save/load, autosave, and crash recovery — unsaved work survives an
  unexpected quit and is offered back on the next launch

### Import & export

- glTF/GLB export and import, round-trip tested for fidelity through Blender
- COLLADA (`.dae`) import from SketchUp's own export path, with a healing
  pass that repairs common non-manifold export artifacts
- Direct SketchUp (`.skp`) import — the 2026 container every current
  SketchUp saves, and every older release, 3 to 8 and 2013 to 2020 — built on
  [Skpwright](https://github.com/hew3d/skpwright), a clean-room reader with no
  Trimble SDK code anywhere in its lineage — names, materials, components,
  tags, guides, scenes, and the saved view all come across natively, and
  identically from either container (every corpus file's 2026 save is
  held to its 2017 original by the importer's own equivalence suite).
  Groups arrive as groups and components as shared components, nested
  assemblies keeping both their hierarchy and their sharing; a group
  wrapping a single solid becomes just that solid, since Hew needs no
  wrapper to isolate geometry. Each SketchUp scene becomes a Hew Scene
  with its camera, its hidden objects and hidden tags, and its active
  section cut; opening a `.skp` into an empty document adopts the
  SketchUp camera (and the section cut of the scene it was saved on, or
  else the model's own active plane), so the model first appears as it
  last did in SketchUp. Production-scale
  files (hundreds of megabytes, millions of faces, renderer-plugin
  attribute dictionaries) walk exactly on the continuous archive path.
  A file cut short (a download or copy that didn't finish) is refused as
  incomplete or damaged when nothing of it can be read, rather than
  opening as an empty model, and opens with a plain notice that parts may
  be missing when something can: the 2017 whole-file grammar reads every
  complete file and refuses every cut one, and a 2026 file loses its ZIP
  end record first.
  Soft and smooth edges, welded arcs and circles, every placement's own
  name, guide points, and attribute dictionaries (dynamic components',
  plugins') come across too, and so do a material's transparency, the
  paint on each side of a face (`Face::back_material`, drawn where a
  back can be seen), the side of the model a section cut keeps, a
  parallel scene's view height, content a component hides by tag or by
  its own hidden flag (hidden in every placement), SketchUp's own
  dimensions and texts as Hew annotations carrying their layers' tags
  (inside groups and components too), placed images in either container
  wherever they sit, the file's backdrop (background, sky and ground,
  the axes) as the document's view style and each scene's saved style
  as the scene's own, and edges that bound no face
  as drafting lines — a group of lines alone becoming a group that holds
  its drawing (`Document::set_sketch_group`), shown and hidden with it.
  Each was checked against SketchUp itself on a production house model
  with 32 scenes
- SketchUp (`.skp`) as a working format (`crates/skp-export`, on
  Skpwright's writers): File ▸ Open makes a `.skp` the document's own file,
  Save writes it back in its own container, and Save As and Export write
  `.hew`, the 2021-and-later container (optionally at revision 17 for older apps), or any release from 3 to 8 and 2013 to 2020.
  Every Object becomes a SketchUp group of its own (solids stay
  discrete), merge groups and components keep their structure, sketches
  become loose geometry, curved walls are soft and smooth, circles are
  welded arcs, textures keep Hew's exact placement, and tags, scenes,
  guides, the section plane, camera, axes, attributes, dimensions and
  leader text come along. A `hew` attribute dictionary carries what the
  format has no field for (tag paths, sketches, curved-surface claims,
  scene display toggles), so a file Hew wrote reopens in Hew with
  nothing lost — the round-trip suite holds every corpus file and a
  native document to that. Saving back over an opened `.skp` builds on
  the file: everything unchanged (a group's or component's geometry with
  every soft edge, curve, texture placement and per-face layer, whole
  group subtrees, the loose geometry) and everything Hew does not model
  (styles, settings, fonts, texts, unused components, dictionaries) stay
  exactly as SketchUp wrote them, and only what changed is rewritten —
  an unchanged open-and-save of the 753-definition theater model rewrites
  none of them, a moved object exactly one. A one-time notice names what
  a `.skp` could not hold exactly, with a "Save a .hew copy" button. The
  desktop app registers as an alternate `.skp` handler on macOS (Open
  With), not the default
- **Make Solid** (`kernel::health`, `kernel::repair`): an import report
  states neutrally how many objects came in solid and how many are panels,
  sheets and open shapes (normal in SketchUp models, which work as they
  are), counting a component's members once. **Object ▸ Make Solid…** (the
  selection, or the whole model; the report's button: the import) walks
  through one kind per page in plain words: openings (left open by default,
  since ducts, pipes and lidless boxes are open on purpose; closed by
  default on a selection), split shapes (rejoined at their seams only),
  flat panels (left, or thickened), curved sheets, and shapes that can't be
  closed with flat faces, each with a one-sentence reason. Choose for each
  item overrides a page's choice per object; the whole run is one undo
  step. Clicking an item isolates it in the viewport (everything else
  hidden, the rest of its group or component faded) and draws its open
  edges. Push/Pull, the booleans, Slice and the STL/3MF warning point to
  Make Solid. **Object ▸ Model Health…** opens a floating window (closed
  at launch, moved and sized by the person, remembered) listing every
  object that isn't solid for one-at-a-time repairs; framing from it fits
  the object beside the window. Object Info has the same buttons, and
  Push/Pull on a flat panel makes it a slab. The same is in the API
  (`hew.doc.health` with per-rim `cappable` / `shared` / `sheet_edge`,
  `hew.solid.cap` / `thicken` / `delete_faces` / `mark_surface`, `health`
  in `hew.query`), so a script can repair a file. **Mend**
  (`kernel::repair::mend`, `hew.solid.mend`) is the repair for imported
  and scanned meshes Close can't finish: it merges points closer than a
  merge distance, removes paper-thin overlaps, separates parts that touch
  at a single point (each piece keeps the name and tags), stitches narrow
  seams and fills small gaps, as one undo step, behind a busy card. It
  does what it can and lists each gap that stayed open with the reason;
  it refuses, changing nothing, when the result would be worse than the
  start. The Model Health row and group (**Mend**, **Mend all**, "Choose
  what to mend" with an editable merge distance) and the Make Solid wizard
  ("Needs mending") preview it exactly, counts and "N gaps will stay open"
  included, by running the Mend on a copy; hovering shows the points
  merged (red) and split (blue) and the gaps filled (green). A row whose
  every opening is warped offers Mend, not Close.
- Make Solid also takes
  selected **sketches**: it runs the close pass over every line of the
  selection, then the closed-faces search (see Drawing) seeded by every
  face of the selection and every face the pass made, which covers files
  drawn before automatic solids and faces an older build never formed. It
  makes a solid or nothing at all: when none forms, the pass's faces are
  not kept and no undo step is recorded; when one forms, every face the
  pass made is kept, those on shapes that stayed open included. When the faces do not close, the viewport marks the
  open edges in red, edges of faces that lie over each other in red too,
  and loops of lines that do not lie on one plane in the warning color
  (`faces_open_edge_segments`, read-only), and the page says how many
  edges are open. `faces_not_solid` carries `detail.segments`. API: `hew.solid.from_faces {sketches}`
  returns `{object_id, grew}` or refuses `faces_not_closed`,
  `loop_not_planar`, `faces_not_solid`, or `unknown_entity`
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
| SketchUp (`.skp`, 2026 format and releases 3 to 8 and 2013 to 2020) | yes | yes |
| STL | yes | yes |
| 3MF | — | yes |
| USDZ | — | yes |
| SVG line drawing | — | yes |
- Reference pictures: File ▸ Import Image… (or a PNG or JPEG dropped on the
  viewport) arms `PlaceImageTool`: a ghost follows the cursor on the face
  under it or the ground (arrow keys lock the plane), the first click sets a
  corner, the cursor sizes it with the aspect locked, flipping to the
  cursor's side, and a typed width or a second click places it. The result
  is what a SketchUp image opens as, in one undo step: one textured material
  carrying the bytes verbatim, one definition holding a single quad whose UV
  frame stretches the picture over it once, and one placement with a rigid
  pose, so the Tape Measure resizes it like any part. The size comes from the
  PNG or JPEG header (EXIF orientation honored); over 64 megapixels, under a
  micrometer a side, or over 1,000,000 meters a side is refused with its own
  reason. The `image-import` crate builds the recipe for the browser and
  `hew-cli`; `hew.doc.import` of a `.png` or `.jpg` places it at an optional
  frame and width and the wasm API gains `place_image`, recorded and
  replayed. Every host checks the kernel can build the face before any
  mutation and refuses with a typed reason; a backstop failure after ingest
  says the document may hold a partial import

### Application shell & UX

- Native desktop app (macOS, Windows, Linux) with native file dialogs,
  recent files, and file-type association — double-clicking a file while
  Hew is already running opens it in the running instance instead of
  spawning a second app
- Browser build with offline support and PWA install
- The viewport renders on demand: no frame is requested while nothing
  changes or the window is hidden, and the periodic app timers (save-state
  label, autosave) arm only when they have work — an idle Hew costs the
  machine nothing
- A labeled tool rail, a command palette (Ctrl/Cmd-/) for finding any tool
  or action by name, a contextual action dock that follows the current
  selection (including Group for a multi-selection and Make Component for
  objects and groups, shown only when the selection qualifies), and a
  docked properties/outliner/materials tray
- Room to model on demand: the tool rail compacts to icons (View ▸ Compact
  Tool Rail, or its chevron), the tray hides entirely (View ▸ Tray, or its
  chevron), and the contextual dock can be turned off (View ▸ Contextual
  Dock). No keyboard shortcuts, by design. Each open window keeps its own
  layout; the last one set is what a new window or the next launch starts
  with. A section asked for while the tray is hidden brings the tray back with
  that section open, and a field mid-edit inside a closing section or tray
  commits first
- One-gesture object interaction: with an empty selection, Move / Rotate /
  Scale act on whatever the first click lands on, and hovering highlights
  it beforehand (SketchUp's pre-selection highlight — the object, group,
  instance, or drawn shape the click would take, and for Move and Rotate a
  guide, in the selection color, through the same resolver the click
  runs). A selection Move or
  Rotate took this way lasts only for the gesture: once the move or
  rotation lands — copies and ×N / /N arrays included — or is cancelled or
  refused, nothing is selected again; a selection made beforehand is kept.
  Scale's first click only selects and shows the grips; after that a click
  off the grips re-targets (better than SketchUp, whose Scale stays on its
  selection): on another object it selects that one, on nothing it clears
  the selection. The Select tool drags
  objects directly (a threshold-gated one-shot Move — full snapping, axis
  locks, Alt-copy, typed distances — that springs back to Select on
  release); and every draw tool imprints a plain object's face at the top
  level, with push/pull holding to the same eligibility. Groups and
  components keep their explicit double-click editing step — their members
  are not directly editable from outside
- The Select tool's full SketchUp modifier matrix, on both a click and a
  marquee drag: plain replaces the selection; Shift toggles each picked
  node; Ctrl/⌘/Option always adds; Shift plus one of those always
  subtracts. Triple-clicking a sketch line (or a drawn arc/circle) selects
  its whole connected shape, while a single click still selects just the
  one segment under the cursor. Edit ▸ Select None (⇧⌘A / Ctrl+Shift+A)
  and Edit ▸ Invert Selection round out Select All, mirrored on the native
  menu bar and the command palette
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
- Live API window management: `hew.meta.documents` lists every open window
  (stable `doc_N` ids), `hew.doc.attach`/`--document` and a per-request
  `document` member address any window — background ones included, without
  raising them — and `hew.doc.new`/`open` create documents in new
  background windows, with `hew.doc.close`/`focus` to finish
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
  onto its definition. Each part is measured along its own axes (its
  largest face for an object or group, its definition's axes for an
  instance), so a board turned at an angle lists its true size rather
  than its world-aligned box; Shop Mode's parts sheet reads the same
  numbers
- Shop Mode gets its own Print Layout sheet from the document menu, with
  Save PDF… as the primary action (the iOS share sheet, or a download
  elsewhere) since AirPrint on iOS ignores the paper size Hew composed for
- Undo-to-clean: the document's dirty/save-state indicator is derived from
  the kernel's own saved mark (`Document::at_saved_mark`), so undoing back
  to exactly the undo depth you last saved at clears the dirty dot with no
  re-save needed, matching every other undo-aware editor; a handful of
  changes the kernel's undo stack can't see (Scenes edits, Library
  provenance stamps, a recovered or freshly imported document) are tracked
  as separate non-undoable-dirty reasons instead. The Edit menu names the
  specific step you're about to undo/redo ("Undo Push/Pull," "Redo Move"),
  derived from the action itself rather than only from a labeled
  transaction. A **Changes** tray section (View ▸ Changes) lists the
  session's history with a marker at the saved depth, the undone tail
  dimmed, and an origin badge on API-authored entries. The native `ask`/
  browser `confirm` prompt for a discard-triggering gesture (File ▸ Close/
  New/Open/Import, the desktop window close) is replaced by an in-app
  dialog listing what would be lost — entries since the save, plus any
  non-undoable-dirty reasons — with Save/Don't Save/Cancel, Save running
  the normal save flow and continuing the original action once it lands
- File ▸ Revert to Saved (after Save As, in the web and native menus; also in
  the command palette) is enabled when the document is dirty and bound to a
  file. It confirms once with Cancel focused, then re-reads the saved bytes
  through the ordinary open path, keeping the window, camera, panels, and
  recents entry, with empty undo and redo stacks and a clean title; a read
  failure leaves the document as it was, and a change made during the read
  (judged by the kernel's undo depth) cancels the revert with a message.
  Reverting a SketchUp file keeps the camera and skips the import report.
  `hew.doc.revert` is host-side next to save and close and refuses
  `nothing_to_revert` without a file or at the saved mark (a headless revert
  judges by the history's saved mark, the live app by its dirty flag); the
  desktop router hands it to the window, which reverts without asking
- A View pane in Settings (between Units and Theme) holds four per-person
  preferences that persist in `localStorage`, sync across windows, and
  default to prior behavior: zoom wheel speed (five stops, Slow to Fast),
  smooth camera moves (off applies every camera change at once, including
  scene activation, the live bridge, and orbit and pan inertia), two
  switches under At the cursor that hide the inference hint and the
  tool-shaped pointer icon, and the Measurements box position (top right or
  bottom right, with the Fly speed readout following)
- Localization: Lingui macros and gettext catalogs under
  `app/src/locales/` (an English source, a Japanese catalog, and a
  compile-time pseudo-locale), an `I18nProvider` over every render root with
  latest-wins activation and an English fallback, and a Language setting
  (System default, each shipped language in its own name, the pseudo-locale
  while Debug Mode is on) persisted and synced like units and theme; a
  switch re-renders in place and re-derives the window title. Every
  user-facing string is wrapped (panels, dialogs, tooltips, tool prompts,
  Measurements box, kernel error copy, Model Health, palette, Shop Mode,
  Print, Report Bug, Settings, HTML menu bar), counts go through ICU plurals,
  numbers through `Intl`, and module-level copy resolves at read time.
  Each history entry carries a structured kind (a stable id plus arguments)
  that the kernel renders its English label from and the app builds its
  localized label from; the English label stays the API's `expected_label`.
  The desktop shell builds its native menu from ids and a per-locale JSON
  generated from one label table and the catalogs, so a language switch
  relabels the menu, updater dialogs, window titles, and library folder
  picker, and the locale is remembered for the next launch. Keyboard
  shortcuts match the ASCII letter a keystroke produced and fall back to the
  physical key (AZERTY, Dvorak, idle IMEs work; Option/Alt combinations
  never dispatch), text-field Enter, Escape, and arrows ignore IME
  composition, and palette search normalizes with NFKC and matches English
  tool names in every locale. A first launch on a profile with no state
  seeds millimeters in Japan, and bug reports record the UI language. The
  verify gate adds Oxlint with the Lingui plugin, a catalog extraction
  check, shipped-locale completeness, a native menu label check, and a
  pseudo-locale Playwright smoke. `SHIPPED_LOCALES`
  (`app/src/i18n/locales.ts`) is English only; the Japanese catalog is not
  offered until complete (`docs/LOCALIZATION.md`)
- User-configurable keyboard shortcuts (Settings ▸ Controls): every command
  in the `menuActionRef` id space (tools, File/Edit/Window commands, views,
  toggles, scenes) can take any number of keys, listed in the command
  palette's own look with search, key recording, a confirm before moving a
  key another command uses, and per-command and global reset. The defaults
  (`app/src/keymap/defaults.ts`) are SketchUp's bare letters plus, on
  the macOS desktop only, SketchUp for Mac's Cmd-combinations; only the person's changes are stored, so a new default
  reaches everyone who never touched that command. One keydown dispatcher
  (`keymap/dispatch.ts`) serves the editor and Shop Mode, and the tool
  rail, both menu bars, and the palette all show the effective keys. Esc,
  Enter, Tab, and the arrows stay with the tools. On macOS the native menu's
  accelerators follow the keymap through `set_menu_accelerators`; Cut, Copy,
  Paste, and Select All keep their standard keys there, because text fields
  rely on those items. Mouse bindings will join the same tab

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
- Help ▸ Sent Reports: a reporter who ticks "Let me know what happens with
  this report" can read the thread, reply, attach a file, and see status
  changes, through a per-report secret held only by the sending Hew (no
  account, no email); the desktop shell and the web build both check for
  replies after launch and badge the Help item with the unread count
  (`workers/bug-intake`'s ReportThread, shell `follow.rs`)
- Help ▸ Report Bug: a dialog that previews what a report contains (system
  details, recorded steps, imported files, a file that failed to open, the
  model, and a log tail, each one untickable) and sends it privately to Hew's bug-report intake service
  (`workers/bug-intake`) or saves it to a file
- Automated testing at every level — unit, component, and end-to-end — plus
  visual-regression checks against reference renders, run on every change
- Plain-language error messages: every operation Hew refuses explains what
  happened and suggests a next step, in the user's vocabulary — the full
  kernel error inventory is covered, enforced by an exhaustiveness test

## Planned

### Near-term

- **First translated language.** The localization plumbing is shipped (see
  Application shell & UX) with English the only offered language; the
  Japanese catalog awaits a translator and is offered once complete. The
  headless API, CLI, file format, user-typed names, and the website stay
  English. Right-to-left layout and comma-decimal measurement parsing are
  separate, later work
- **Flat-path push/pull into a face's own holes.** Both push/pull paths
  are shipped (translate-and-build for any planar face, whole-wall radial
  offset for analytic curved walls — see Shipped). The one flat-path case
  still refused is pushing an outer face edge into or past one of its own
  holes: the deferred P4 hole-edge case, revisited with true circles
- **Editable polygon side count after the fact.** A drawn circle's
  Segments can be changed in Object Info at any time (the chain keeps its
  exact circle and only its chords are rebuilt), but a regular polygon's
  side count is settable only while drawing or in its post-click `Ns`
  window. Once committed, a polygon's sides are its real geometry and
  Object Info offers no Sides field, so a 48-sided polygon cannot be
  changed back. Planned: a Sides field for a whole, untouched regular
  polygon (its centre is already kept, so the rebuild has an anchor),
  with the same refusals as circle re-faceting. It stays a polygon at any
  count: sides remain hard edges when extruded, unlike a circle's smooth
  wall

### Longer-term

- **More Model Health repairs** for what Close and Thicken… leave open:
  thickening bent surfaces (an offset shell that refuses folds it cannot
  resolve), patching non-planar holes, bridging two parallel openings
  (a window pane modeled as two faces), and welding openings that miss
  each other by a hair. Removing faces enclosed inside a shape was
  prototyped and measured: it made 2 of 88 hard cases solid across three
  real models (the rest have real open ends), so it waits for a model where
  inside faces are the main cause

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
  AI agents over MCP — including the Library (`hew.library.*`, §8.1),
  which round-trips a saved item's geometry through the same kernel
  entry points the UI's own Library browser uses; the plugin system adds
  sandboxed transports on top of the same bus. In place today: every
  connection carries a grant (a ceiling plus permissions) instead of a
  profile, every command declares the permission it needs, and the
  dispatcher enforces restricted grants (document binding, path
  parameters, attribute namespaces) at its existing check sites, inside
  transactions too (HEW_API.md §10); the manifest parser lives in
  `crates/plugin-manifest` (docs/agents/PLUGINS.md). A plugin's manifest
  commands register on the one shared registry as `<plugin-id>.<local>`;
  the dispatcher checks their params against a small JSON Schema subset
  and hands them back as forwards instead of executing them, then
  validates and shapes the plugin's reply (HEW_API.md §10.2). Until the
  realm lands the app answers every forward "unavailable", and MCP labels
  plugin results with their `provider`. A mutating plugin command runs in
  a kernel compound (one undo entry, the UI and every other client held
  off meanwhile, abort restoring the whole document, a busy notice with
  Cancel), recorded and replayed as one `plugin_compound` call; envelopes
  can coalesce into one undo step (`coalesce`) and refuse a stale model
  (`expect_generation`), and reads carry the history generation and the
  document epoch (HEW_API.md §6.1, §6.5, §7). A connection subscribes to
  events (`hew.event.subscribe`) and receives them as notification frames
  between its replies: `document_changed` with a change set by public id
  after every entry, undo, redo, abort, and replacement, plus context,
  Scene, units, and document lifecycle events; `hew.query.changes_since`
  catches a reader up, the desktop socket and the Windows named pipe
  write events between replies, `hew-cli events --live` prints them,
  and MCP follows `plugin_state`
  (HEW_API.md §4.5). A connection holding `view` reads and sets the
  app's selection (replace, add, subtract, toggle), hides and shows
  nodes, isolates, and frames entities, all view state that rides a
  transaction without an undo entry, and hears `selection_changed` and
  `tool_changed`; the desktop app mirrors its selection, isolation, and
  active tool into the kernel so the bus answers from there
  (HEW_API.md §7.3). A reader lists every placement with its path and
  world pose (`hew.query.occurrences`, nested components expanded, from
  roots such as the selection), measures a part along its own axes
  (`hew.query.part_frame`, honoring a `hew.part` frame override), reads
  a placement's mesh and an object's topology, and exports or draws only
  a selection (HEW_API.md §7.4). A ray reports the placed part it hit
  through any depth of nesting, with its occurrence path, leaf, and a
  face locator, and can list every crossing (`hew.query.raycast`'s `all`,
  `max_distance`, `exclude`, `include_hidden`); `hew.query.measure` by
  id or path gives a part's volume, area, and bounding box, summing the
  members of groups and instances (HEW_API.md §7.4). A file's bytes
  become one unplaced component definition
  (`hew.component.import_definition`: `.hew`, `.skp`, glTF/GLB, STL,
  COLLADA), and importing the same bytes again returns that definition
  instead of making another, so a hardware or cabinet-module library is
  one definition per model (HEW_API.md §7, document commands). The app itself
  renders `hew.view.snapshot` (of a selection, or a definition's `cmp_` id,
  on a transparent background) and `hew.view.line_drawing` through the
  shared `crates/headless-render`, byte for byte what `hew-cli` produces
  (HEW_API.md §7.2). A writer gives a definition new axes
  while every placement stays put (`hew.component.rebase`), mirrors one
  placement (`hew.component.flip_instance`), sets or clears a part's
  frame override (`hew.entity.set_frame`), and copies a selection into a
  grid of up to three axes in one undo step (`hew.entity.move`'s
  `copy.grid`). The kernel forks one placement at any depth by its
  occurrence path and reports every id the fork created
  (`Document::make_unique_by_path`: with the cascade, only that placement
  diverges; `deep` forks the nested definitions too; one undo entry),
  exported to the app as `make_unique_by_path_json`; its bus command
  (`hew.component.make_unique` by path) is still to come. The kernel's
  copies link when the caller asks for it (`LinkPolicy::Linked` on
  `duplicate_node`, `duplicate_nodes_array`, and
  `duplicate_nodes_placements`): each Object or Group copied is wrapped
  as a linked set (`Document::wrap_linked`, a definition carrying `hew` /
  `linked`) and every copy is a placement of it, in one undo entry; a set
  goes with its last placement, `promote_linked` turns one into a named
  component, change sets carry `linked` and `copied_from`, and the wasm
  surface exports `wrap_linked`, `is_linked`, `set_linked`,
  `owner_definition`, `duplicate_selection_array_linked`, and
  `duplicate_selection_placements` (with a `_linked` sibling that reports
  the wraps; the Move grid, recorded as one call). The bus still copies
  independently. `hew-cli --version` and `hew-relay --version` report the release
  tag's version on a tag build; the `core` ceiling grants `hew.view.hide`,
  `show`, `show_all`, and `hidden` without a window; `hew_query` admits every
  command that does not change the document; `hew.entity.rotate`'s angle is
  documented in radians; headless PNGs are deflate-compressed; and `hew-cli
  --live` writes a snapshot or line drawing client-side from one render. A boolean's
  result keeps its first operand's name, tags, and attributes. A plugin
  checks before it commits: `hew.doc.transact` with `dry_run: true`
  rehearses a whole envelope on a copy of the document and answers what
  the commit would, needing only read permissions (HEW_API.md §6.6), and
  `hew.compute.boolean`, `slice`, `shells`, `measure`, `outline`, and
  `offset` answer geometry questions on transient solids (an object, a
  re-posed object, or a prism or box described inline) without touching
  the document, one at a time per connection and capped at 2,000,000
  triangles (HEW_API.md §7.6). `hew.solid.subtract` cuts a part through
  its path with such a prism or box, no cutter object created.
  `hew.compute.project` and `hew.compute.stretch` are declared and wait
  for the flat-drawing projection and section stretch in core. The snap
  service can lock to any direction through an
  anchor, for plugin tools to use. A component placement carries its own
  material (manifest v27), shown on member faces nobody painted, the
  nearest painted placement winning; `hew.material.paint` sets it or a
  whole definition's, occurrence rows and path-located faces report
  `material_effective`, and the Paint tool paints the placement under
  the pointer at the top level (HEW_API.md §7.5). A solid's faces and
  edges carry attribute dictionaries of their own (geometry buffer v9),
  addressed by locator through `hew.attr.*`, reported by
  `hew.query.faces` and `hew.query.topology`, and carried through edits
  by per-site rules; edge dictionaries do not yet survive booleans,
  slices, thickening, or splitting a solid into pieces (HEW_API.md §8).
  A saved document carries a persistent id (manifest v29 `document_id`),
  minted on its first save and kept through Save As and copies, which
  `hew.meta.documents` and the open and save events report for
  per-document plugin state to key on (docs/agents/PLUGINS.md). The SketchUp
  writers do not yet write instance materials or face and edge
  dictionaries. Still to come: these effects in the browser and
  Shop Mode, the sandboxed realm, install and consent, and the workbench
  contribution points.
  Stage P0 (realm probe): in progress; the engine record is in
  docs/agents/PLUGINS.md and its macOS and Windows cells are still unfilled.
- **SketchUp (`.skp`) writing — the remaining gaps.** Dimensions and
  leader text are written with point anchors, so in SketchUp they do not
  follow the geometry they measure (a radius dimension hangs on an edge of
  the arc under its point; one of a circle nothing draws stays Hew's
  only); SketchUp's own dimensions and texts open as Hew annotations
  wherever their list is placed once — screen texts (pinned to the view,
  not the model), and dimensions and texts inside a component placed
  several times, stay in the file, and a save back keeps them as they
  are; hidden edges that bound no face stay in the file too (Hew has no
  hidden drawing); a document's and a scene's view style cover the
  backdrop and the axes only — edge and face styles wait for a styles
  feature;
  images open only from the 2026 format and only among the model's own
  entities; a model's style (background, sky and ground, edge and face
  styles) is not carried; a save back over a `.skp` keeps the definition
  of a group the document deleted (unplaced, as a purge would remove);
  SketchUp-native loose geometry opens as a healed mesh Object rather
  than as sketches; the `.skp` file association is macOS-only (the
  Windows installer would take the default from SketchUp)
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

- **Share relay uploads against the Workers Free CPU limit.** The hosted
  "Open on Phone" relay (`workers/share-relay`) accepts a drop of up to
  32 MiB in a single `PUT /drop`. The Workers Free plan allows 10 ms of CPU
  per request, and reading and batching a body that size through one request
  may exceed it; `workers/bug-intake` uploads in 1.9 MB pieces for exactly
  this reason. A large relay drop has never been exercised against the limit
  in production. Revisit if a large "Open on Phone" fails: chunk the upload
  the way `workers/bug-intake` does, or move the Worker to the Paid plan.

- **Report Bug's pause while gathering after a large import.** Opening
  Help ▸ Report Bug copies the whole session recording out of the kernel in
  one `peek_recording` call. After a 59 MiB SketchUp import that is a 211 MB
  string and a 1.6 s main-thread block, shown under "Gathering details…";
  the rest of the dialog's work yields to the page. Removing the pause needs
  a wasm-api change: a chunked or streamed recording export. Revisit on a
  complaint.

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
  rather than allowed. The one sub-element that IS editable is a drawn,
  unpushed shape on a face (drawing, see Shipped): every edit it
  allows lowers to a coplanar surgery that cannot open the shell, and it is
  still not a general face or edge editor — a boss's wall or a solid's
  own corner cannot be dragged. Model Health's face removal is the other
  exception, and it works only on open imported objects, never a solid.

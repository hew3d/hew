# Hew — Architecture

Hew is a cross-platform 3D modeler. This document describes the shape of the
system: the data model, the crate and module topology, the design positions
that shaped it, and the mechanisms that keep it robust. It is meant to orient
a reader who is about to work in the codebase, not to justify every choice —
the commit history and the code do that.

## 1. Overview

Hew combines two things that don't usually travel together: an interaction
model borrowed from SketchUp, and a data model borrowed from solid CAD.

The interaction model is direct modeling. Draw a closed shape on a surface,
push or pull a face to give it depth, and let a pervasive inference engine
snap the cursor to endpoints, midpoints, edges, faces, and axes as you work.
There is no history tree, no parametric feature list, no sketch-then-pad
dialog. What you see is what you have.

The data model is solids-first. Every enclosed volume in a Hew document is a
discrete Object — a watertight solid, tracked as such, that never merges with
another Object by accident. Geometry inside an Object behaves the way
SketchUp geometry behaves (edges split faces, coplanar loops become faces,
push/pull reshapes a face), but that stickiness stops at the Object's
boundary. Two Objects sitting flush against each other stay two Objects until
something explicitly joins them.

This pairing exists to fix the two failure modes that make raw SketchUp-style
modeling frustrating at scale:

- **Accidental welding.** In a pure "sticky geometry" model, ungrouped
  geometry fuses with anything it touches, and un-fusing it later is not
  really possible. Hew scopes stickiness to an explicit container (the
  Object) so touching geometry never welds unless a user asks for that.
- **Hollow non-solids.** A face-soup model has no concept of "solid" beyond
  visual closure — delete one face and a shape that looked whole is now an
  open shell, silently. Hew tracks watertightness as a real, queryable
  property of every Object and refuses operations that would open one
  without saying so.

Architecturally, this is implemented as a single Rust geometry kernel — pure,
UI-free, compiled to WebAssembly — driving a TypeScript/React interface built
once and shipped both as a desktop application (via Tauri) and as a static
web app, rendered with three.js on a WebGL2 baseline. The kernel is the
source of truth for everything that must be exact: topology, watertightness,
booleans, undo, and file I/O. The UI owns everything that is a matter of
taste or platform convention: camera behavior, tool state machines, panel
layout, and session state such as which Object is currently being edited.

## 2. The Data Model

This is the part of Hew that most determines how it feels to use, and it is
the part most worth understanding before touching the kernel.

### 2.1 Document and Object

A Hew document is a tree. Its nodes are:

- **Objects** — discrete, watertight solids. Each Object owns an island of
  half-edge mesh geometry: vertices, edges, and planar faces (including faces
  with holes), with a tracked watertight/leaky state.
- **Groups** — non-destructive structural containers with no geometry of
  their own; a unit of selection and transform over their children.
- **Component definitions and instances** — a definition owns a flat set of
  geometry in its own local coordinate space; an instance places that
  geometry in the tree at an invertible pose. Multiple instances share one
  definition's geometry.
- **Sketches** — first-class 2D drafting entities, not solids (see 2.6).
- **Guides** — construction lines and points, not solids (see 2.9).

Cross-node operations — union, subtract, merge, slice — are always explicit,
document-level commands. Nothing in this tree merges as a side effect of
proximity.

### 2.2 Extrusion automatically creates an Object

Extruding a closed 2D profile is the one operation that manufactures a new
Object without a separate "make solid" step. A drawn rectangle or polygon,
once pulled off its plane, becomes a self-contained watertight solid with no
further action required. This is the concrete expression of "solids by
default": a user never has to remember to group their geometry to keep it
from fusing with the rest of the scene, because the geometry that would fuse
doesn't exist yet — only its 2D sketch outline does, and that outline is
consumed once the extrusion happens: it becomes the solid's base face and
leaves the sketch (see 2.6).

### 2.3 Combination is always explicit

Two Objects only become one via a deliberate command: boolean union,
subtract, or a non-destructive Group. There is no implicit welding across
Object boundaries, no matter how precisely two Objects' faces coincide.
Booleans are implemented as a polygon arrangement operation that reuses the
same face-splitting machinery as ordinary editing, rather than a
triangle-soup CSG library — this lets coplanar contact (shared walls,
stacked solids, coincident faces) be resolved exactly instead of falling back
to numerically fragile epsilon comparisons. A boolean that would depend on
resolving a pure tangency (an edge lying exactly on a face, with no volume
overlap) refuses cleanly rather than guess.

### 2.4 Sticky geometry, scoped to an Object

Inside a single Object, Hew reproduces SketchUp's "sticky geometry" feel:

- Drawing an edge across a face splits that face.
- A closed loop of coplanar edges automatically becomes a face.
- Push/pull reshapes any face region, generating or merging the side walls
  needed to keep the result watertight.
- Coincident vertices and edges merge — but only with other geometry inside
  the same Object.

Inference snapping itself is not scoped this way: the cursor can snap to
anything visible in the whole scene (another Object's edge, a guide, an
axis), because snapping is a read-only query, not a mutation. Only the
merging of geometry into a single mesh is confined to the active Object.
What counts as visible follows the window's render mode, set once on the
inference scene as a face policy (`inference::FacePolicy`, pushed through
`Scene::set_render_mode`) rather than passed by each tool: in Shaded faces
hide what is behind them; in X-ray nothing is hidden but faces are still
snap and pick targets; in Wireframe nothing is hidden and faces are not
targets at all (no on-face snap, no face or sketch-region pick). In both
see-through modes a query that opts in (`SnapQuery::see_through_points`,
set only by the draw tools that drop an off-plane snap onto their plane
and say "projected") is not limited by its constraint plane (the face the
tool is held to) for named points and edges, so drawing reaches hidden
geometry through the face; every other query keeps the plain plane
filter. Camera questions (the orbit pivot) ask `Scene::pick_face_geometry`
and `Scene::pick_sketch_region_geometry`, which ignore the policy.

A shape drawn on a face and not yet pushed or pulled is **drawing**, and
drawing on a face goes through the same engine as drawing on the ground
(the drafting engine, below). Where the drawing separates regions of a
face it is real topology (a coplanar sub-face twinned with a hole in its
parent, or a run of edges between two coplanar faces), never a separate
overlay entity: that is what lets every operation (move, push/pull of the
parent, slice, booleans) carry it along with no special code. Drawing that
is not topology (a loose chain, a dangling end, a sliver the face cannot
hold) lives in the object's OWNED sketch (2.6), because a solid's
face loops hold no loose edges. Nothing about the drawing is an object of
its own: the app selects and edits it as **plane units** (below), derived
on demand from the lifted plane and keyed by geometry, never stored. The
moment a drawn region is pushed or pulled it is solid geometry again.

Three kernel ops carry the topology half of every lowered edit and stay
the vocabulary of history replay: `split_face` (a boundary-to-boundary cut,
or a cut with both ends on the same hole loop), `merge_faces` (its exact
inverse, across one run or, through the re-trace, several — a run passing
through a vertex other edges also use counts as the pieces it splits
into there, so its undo re-inserts each boundary to boundary), and
`retrace_face`, which inserts several paths at once and rebuilds the
face's loops by rotational order at each vertex, the half-edge form of the
ground's region tracer. Every loop is simple (no vertex twice in one
loop), and loops of one face may share vertices at isolated points; two
loops never share an edge. A closed walk the re-trace takes that visits a
vertex twice is split there into simple loops, so a circle tangent to a
face's edge at one point is a disc face plus a hole in the face around it
sharing that one vertex with the outer loop, and a figure-8 hole is two
holes sharing a vertex. Bridges between boundary loops are intermediate
only: a walk running out along an edge and back along its twin is
refused, so committed state never holds a bridge. The validator enforces
the rule, and the ground's region tracer splits its rings the same way. `merge_faces` also dissolves a run between a face's outer loop
and a **hole loop** of the face around it: the hole owner keeps its
handle, outer loop, plane, and paint, and the hole is rerouted around what
was dissolved. Each op records the other as its undo, so the split/merge
identity and the history proof cover both kinds of run. These ops are no
longer a UI entry point: the engine reaches them only through lowering.

#### The drafting engine

The plane is the drafting context. `Document::draft` (`crates/kernel/src/
document/draft.rs`; `begin_draft` / `end_draft` for gestures spanning
events) edits the whole drawing on one plane as one undo step:

- **Lift** (`lift_plane`). Everything drawn on the plane is replayed into
  ONE working `Sketch`: the face topology of every drawable object's faces
  on it, those objects' owned sketches, and the free sketches there. A
  solid's own edges on the plane (where its faces meet non-coplanar ones)
  are FIXED: drafting splits them where lines meet them but never moves or
  removes them.
- **Edit.** Any ordinary `Sketch` method runs on the working sketch
  (drawing, erasing, moving islands, dragging points, offsetting,
  re-faceting), and the sticky rules arrange the edit exactly as they do
  on the ground. This is why a shape on a face splits faces the way the
  same shape splits a ground sketch, including a circle inscribed in an
  imprinted rectangle or crossing shapes cutting a circle into arcs.
- **Lower.** The edit is diffed against the lift by exact positions, and
  each touched island is re-partitioned by where its edges lie. An edge
  over a face that separates two regions becomes that face's topology, by
  minimal delta (only removed edges are removed, only new ones inserted)
  through `retrace_face` rehearsed on a copy. A loose edge over a face goes
  to that object's OWNED sketch. An edge over no face goes to a FREE
  sketch. Stores whose content did not change are not rewritten. A region
  the face cannot hold (a sliver too small for a saved file to reopen) is
  kept as owned drawing instead of refused.
- **Materialization.** A free region may be closed partly by a solid's
  edge (a crescent closed by a face's edge). Before any operation moves or
  removes solid edges (transform, delete, boolean, slice, push/pull) the
  spans that close such a region are copied into the free sketch, so the
  void keeps what the user saw; spans backed by a face edge again are
  dropped afterwards. A free region is thereby extrudable like any ground
  region, and a covered region is pushed or pulled as its face.
- **Units and keys** (`crates/kernel/src/unit_key.rs`, `Document::
  plane_units`). Islands, curve runs, edges, and regions are keyed by
  geometry, not slot history: FNV-1a over the `(u, v)` bit patterns in the
  plane's frame, with axes taken from a quantized normal so one plane
  reached two ways keys alike. An island key is its minimum edge key, a
  curve key hashes the analytic circle (a polygon's also its corners, so
  two polygons inscribed in one circle key apart), and a region key hashes its whole
  outer ring, so a shape flush to a face's corner never collides with the
  face around it. Keys survive save/load and replay; they change only when
  the geometry they hash does. A curve run is a circle or arc whose edges
  are covered across the plane, whole when it closes. Deleting a run
  deletes it as one undo step; a circle drawn across a rectangle deletes
  back to the rectangle.
- **Surface.** `crates/wasm-api/src/draft_api.rs` exposes `draft_*` calls
  (edits address edge keys; curve and region operations take curve and
  region keys; `draft_begin_ctx` reopens a draft on a plane context
  string). The app commits each draw gesture as one draft
  (`app/src/tools/draftCommit.ts`) and lists and selects plane units
  through `app/src/tools/planeUnits.ts`; the Hew API's face-target drawing
  goes through the same engine.

An OPEN chain on a face is legal: a dangling end is simply a loose edge
and lives in the owned sketch.

### 2.5 Editing context, not selection, determines stickiness

Which Object a new stroke joins is determined by an editing context — the
Object (or nested path of Group/Object) a user has entered, double-click
style, the same gesture SketchUp uses. Drawing at the top level of the
document creates or extends a Sketch; drawing after entering an Object
applies that Object's sticky rules. Within either, the drawing plane is the
drafting context (2.4): a gesture edits everything drawn on that plane,
ground or face, as one draft. Selection was considered as the
triggering signal and rejected: it is a weaker, more modal proxy for "what am
I editing" than an explicit enter/exit context, and it doesn't generalize
cleanly to nested Groups and Components. Entering a plain Object is pure
application-layer state — the breadcrumb path back out, nothing in the
kernel. Entering a Group or a Component instance reaches the kernel: it
opens a **session frame** (2.11) — transient `Document` state holding the
container's contents surfaced as ordinary top-level world geometry for the
edit's duration, one stacked frame per nesting level. That state is never
serialized — a save mid-session transparently writes the document as if
every open frame were closed — so the persisted document model is
unaffected even though the live, in-memory one temporarily is.

### 2.6 Sketches are first-class, not zero-thickness solids

A shape drawn but not yet extruded — a rectangle, a polygon, an arc — is a
persistent Sketch entity in the document tree: movable, copyable, and
deletable before it becomes a solid. It is deliberately not modeled as a
degenerate ("zero-thickness") solid Object, for two reasons:

- It would permanently violate the watertightness contract every other
  Object honors, forcing a special case that suppresses the "leaky" state
  for an entire category of entity that was never meant to be watertight.
- Booleans and push/pull are defined over watertight operands. Treating flat
  shapes as solids would force every operation to handle a face-versus-solid
  case in addition to solid-versus-solid, for no benefit.

A Sketch has one of three owners: the world (a **free** sketch, drawn over
no face), a component definition (in definition-local coordinates, reaching
the scene through its instances; its plane is the drawing instance's
pose-inverse applied to the world plane keeping its side, so through that
instance, mirrored or not, the sketch faces the user's up and a positive
extrusion goes where they pointed), or an **Object** (an *owned* sketch, kept
in `object_sketches`). An owned sketch holds exactly the drawing over the
owner's faces that cannot be face topology (loose chains, sliver regions); it is expressed in the owner's frame and is in exactly one of the
three states, never two. Its liveness is derived from the owner, so every
hide, delete, and undo path carries it with no code of its own
(`Document::sketch_tombstoned`). It rides with its owner: transforms
apply to it, and push/pull moves the owned edges over the moved faces
(`plan_face_ride` / `install_face_ride`). Booleans, slices, push-through,
and merging sweeps copy each owned edge onto the result whose coplanar face
lies under it (`carry_owned_sketches`), and duplicates, explodes, and
library copies copy it to the new owner. Two invariants hold in debug
builds: I7, no owned edge duplicates an edge of its owner on that plane;
and I8, a coplanar same-facing owner face exists for every owned sketch. A
reconcile pass (`reconcile_owned_drawing`) after every recorded action
keeps them true, dropping an owned edge the owner now holds as topology,
freeing a sketch left with no coplanar face, and deleting an emptied one,
all inside the same undo step (`Document::complete_change` adds owned
sketches to a change). The file records an owned sketch's owner as
`owner_object` (HEW_FILE_FORMAT.md 4.6, v17).

A free sketch may be **detached**: it takes part in no plane arrangement
(`Sketch::detached`, format v23), is its own drafting context, and
extrudes only to a new Object. It is the one way to draw on a face
without joining it; nothing joins it later. The viewport draws a
detached sketch, like any free sketch, in blue, and the edges and regions
of a solid's face (imprints, object-owned sketches) in orange.

Imports are the one source of open Objects, and many of them are
**surfaces**: a single sheet SketchUp let someone draw on purpose (a pane
of glass, a sign, terrain). Model Health (`kernel::health`) reports that as
a *verdict* — `flat_sheet` or `bent_sheet` beside `cappable`, `holes`,
`mixed` and `non_manifold` — derived from the open boundary every time it
is asked, never stored as a kind of Object. A surface is still an open
Object to every operation; the verdict only keeps the UI from presenting it
as broken, and the one flag a person can set (`hew.health` / `surface` in
the Object's attributes) records that they accepted it as a surface. Native
drawing still never makes one: a flat shape stays a Sketch until it is
extruded.

A Sketch is the larval form of a solid: extruding a region makes the
drawn profile BECOME the solid's base face, so the geometry that bounded
it leaves the sketch — really deleted, not hidden. Exactly the edges no
surviving region needs go: an edge shared with a neighboring region stays
so the neighbor remains closed, open chains stay, and an extrusion that empties
a sketch removes the sketch entity itself. Nothing invisible persists, so
nothing can resurrect by side effect — deleting a solid deletes a solid,
and the one road back to an outline is undo, which restores the sketch
and removes the solid in the same atomic step. "What you see is what you
have" is the invariant this buys: every entity in the document is either
visible geometry or nothing.

Re-extruding occupied ground is allowed. Hew's solids interpenetrate
freely, so drawing a region over a visible solid's base and extruding it
simply produces a second, coincident solid — exactly as any other overlap
does. An earlier design refused this with a derived "standing-solid gate"
(a region overlapping a visible solid's coplanar face would not extrude);
it was dropped as inconsistent with the interpenetration model and
confusing in practice — it also blocked legitimate work, such as extruding
a larger coaxial solid over a smaller one. Nothing about re-extrusion is
stored or derived: the `.hew` format carries no claim data at v11, and the
stored claims of older versions are ignored on load (HEW_FILE_FORMAT.md).

Within a Sketch, the user-facing units are derived, identity-stable
sub-entities, mirroring how regions already work: **islands** (connected
components — each shape drawn apart from the others selects, deletes, and
moves independently, recomputed on every edit with stable ids) and
**curves** (the facet chain one arc or circle gesture committed, carried as
edge metadata and persisted in the file format, so a drawn curve selects
and deletes as a unit). A curve chain also carries the analytic circle it
was drawn from (center + radius), and extrusion stamps each side wall with
the cylinder it is a chord facet of — durable metadata over the faceted
carrier, propagated under a strict map-or-drop contract and held honest by
the validator. The same identity follows a curve onto a solid's face: a
circle or arc drawn there (lowered to topology) stamps its facets' circle onto the solid's
edges (`Edge::curve`, one claim per edge, so a pie or segment claims only
its arc), every wall-raising path — boss, wall-building push/pull,
push-through — stamps a wall per facet from its base edge's claim (gated
on the edge being a genuine facet, never a secant), and a push carries a
claim with the rim it moves. Push/pull on a stamped wall facet acts on the *logical
wall*: an exact radial offset of every facet claiming that cylinder,
derived from the stored axis and radius rather than from any facet's
plane, refusing typed where a neighbor would bend.

### 2.7 Groups and Components are the two ways to compose

Groups and Components answer different questions and stay separate concepts:

- A **Group** answers "which pieces move together?" It has no geometry, no
  pose, and welds nothing; transforming a group bakes the transform into
  every leaf Object beneath it.
- A **Component** (definition + instance) answers "where do copies of this
  thing appear?" A definition owns geometry once; every instance carries its
  own invertible affine pose, which — unlike a baked transform — may include
  mirroring or non-uniform scale, since a pose is applied at render and
  tessellation time rather than baked into stored geometry.

A move, rotate, or scale of a plain Object or a Group bakes the transform
directly into vertex and plane data. This keeps the kernel's core geometry
representation pose-free everywhere except Component instances, which is the
one place a persistent pose earns its complexity.

A placement can be forked by occurrence path (`make_unique_by_path`): with
the cascade on, every definition on the path is forked so only that
placement diverges; the call returns the ids it created (the remap). A
forked definition keeps its attributes and its name, and sheds its Library
provenance.

A **linked set** is an ordinary definition carrying the attribute `hew` /
`linked`; it is how a copy of an Object or a Group links to its source.
`Document::wrap_linked` folds the node into a new definition without
flattening it: the node keeps its id, name, tags, attributes, history, and
subtree, no geometry moves, and one identity placement takes its spot in
its parent (a group, or an outer definition for a node reached by path).
The duplicating ops take a `LinkPolicy`: with `Linked`, each Object or
Group root is wrapped as its own set and every copy is a further placement
of it, so a reflecting step makes a mirrored placement instead of baked
geometry; the wraps and the copies are one undo entry, and undoing it
returns the loose node exactly. A set reduced to one placement stays a
set (only undoing the wrap itself restores the loose node), and a linked
set goes with its last live placement — deleting, exploding, or unlinking
it hides the definition and its members, and undo brings them back. The
rule is transitive: a set placed only by instances among those hidden
members goes too, as does one placed only inside a deleted definition —
while
a named component stays available with no placements. Removing the
attribute (`promote_linked`) turns a set into a named component; the
reverse does not exist.

Editing a definition's members is covered separately in 2.11: because a
definition's members are already ordinary Objects (just owned by a
`ComponentId` rather than the world), editing one in place is a matter of
temporarily changing what owns them, not a distinct editing mode.

### 2.8 Materials and Tags

Materials are a document-level palette; each face carries an optional
material reference, and each Object carries an optional default material
that unmarked faces fall back to. Each component instance carries an
optional material of its own as well: a member face with neither its own
material nor an Object default shows the material of the nearest instance
above it that has one, so one definition can show a different color per
placement while painted faces keep theirs (SketchUp's rule). The kernel
resolves that chain (`Document::inherited_material`) and the tessellator
takes the inherited material as an input, so a member's render mesh is
keyed by member and inherited material. A material is a name, a color, and an
optional texture. Textures are stored as opaque, already-encoded image bytes
— the kernel never decodes an image, keeping image codecs out of a crate
that must stay dependency-light and pure — mapped onto a face by an affine
UV frame precise enough to reproduce an imported model's original texture
scale and orientation, not just a generic planar projection.

Attribute dictionaries (client data the kernel stores and never
interprets, docs/agents/HEW_API.md §8) live on the document and its
entities in one document-level table, and on a solid's faces and edges
inside the Object itself: the Object holds an append-only table of
interned dictionaries, and each face and edge carries a small `Copy` slot
into it (none by default). A write interns a new dictionary and repoints
the element, never editing a slot in place, so elements may share one and
an undo snapshot can record a slot by value; slots are never renumbered
while the Object lives, and a dictionary nothing references any more is
simply not written on save. Each operation assigns slots at the place it
builds an element (a face built from another takes its slot, a fresh wall
none, a merge keeps the survivor's); whole-Object rebuilds (booleans,
slices, thickening, component splits) carry face dictionaries by value
into the new Object and rebuild edges without any. History replay restores
dictionaries with the rest of the recorded state (§5.7).

Tags (Hew's analog of SketchUp Layers) are first-class document metadata used
to group and show/hide Objects, Groups, and Instances independently of the
tree structure they otherwise live in.

### 2.9 Guides

Guides are persisted construction geometry — offset lines, angular lines,
and points — distinct from both Sketches and solid Objects. They carry no
fill and no watertightness, never extrude, and exist purely as durable
reference geometry and inference snap targets, created by measuring and
angle tools and cleared explicitly. A guide carries tags like a node: one
with a hidden tag stays in the document but is neither drawn nor snapped
to, so a Scene's hidden tags hide guides as they hide drawing (a `.skp`
guide opens on its layer's tag). A guide line has a stretch: the whole
line for one Hew draws, or a ray or a segment where a `.skp` file holds
one (SketchUp's Tape Measure makes both), and only that stretch is drawn,
snapped to, or crossed for an intersection snap.

A Sketch's closed loop normally has a face: the region that is filled,
picked and extruded. A loop can also be edges alone (`faceless`), which is
what SketchUp has where a face was erased or edges alone were drawn: a
wire-frame component, a renderer's light rig. Such a loop is not filled,
picked or extruded, and it stays a loop with no face while it is moved,
copied, split by a line ending on one of its edges, or merged by erasing
an edge. Drawing over one of its edges gives it a face, as in SketchUp,
and a loop a drawing step closes always has one.

### 2.10 Operations that preserve watertightness by construction

A handful of higher-level operations are worth calling out because each one
is designed so that watertightness can never be broken as a side effect:

- **Push-through.** Pushing or pulling a face past the Object's opposite
  face performs a boolean subtraction of the swept volume rather than
  producing an open shell — this is the *only* way material is removed from
  a solid in Hew.
- **Slice.** Cutting an Object with a plane produces two independent,
  fully watertight Objects sharing a coincident cut face — a real geometric
  split (in the spirit of a CAD "split body" operation), not a virtual
  overlay bolted onto one solid.
- **Delete.** Deletion removes whole nodes (an Object, Group, Instance, or
  guide) at a time. Deleting a single face or edge is deliberately
  unsupported: allowing it would let a watertight solid become a leaky shell
  through an everyday action, reintroducing the exact failure mode the data
  model exists to prevent. Removing material or punching an opening always
  routes through push-through instead. The one sub-element a user can
  delete is drawing (2.4): erasing a drawn shape on a face merges two coplanar
  faces back into one, which leaves the shell closed by construction, so
  the kernel accepts exactly that and still refuses any merge across
  non-coplanar faces.
  Model Health's `delete_faces` removes faces from an **open** Object only
  (an import's stray sheet, an importer's partition sheet); a solid refuses
  it with `ObjectIsSolid`, the same stance.

### 2.11 The session model: editing containers with the unmodified tool set

Editing inside a container — a Group's members, a Component's shared
geometry — has to reconcile the container's semantics with the fact that
Hew's tool set — push/pull, Follow Me, booleans, slice, sketching — is
written entirely in terms of ordinary top-level world Objects, with no
notion of a parent, a definition, or a pose. Rather than teach every tool
a second, container-relative code path, Hew changes what the contents
temporarily ARE, and lets the unmodified tool set do the rest. Entering a
container opens a **session frame** on a kernel-held stack; frames nest
one per container level and close strictly LIFO (Escape, double-click
outside). Both frame kinds lean on the same structural insight: the
contents are already real Objects, merely attached to a container. While
a frame is open, picks favor its own geometry in Shaded and X-ray alike
(the editing view draws everything outside it translucent): the kernel
skips faces and drawing outside the frame (`PickScope`, plus the frame's
own sketches from `Document::session_frame_sketches`) so a tool reaches a
buried face of the component or group being edited through whatever
encloses it.

Entering a **Group** is a transient *ungroup*: the group's direct members
have their parents cleared to the top level and the group record hides
with its member list intact — precisely the document's existing ungroup
posture, so no invariant changes. There is no pose and nothing to bake;
the flip is exact. Every tool then sees genuinely top-level geometry, and
the replacing operations that refuse a grouped operand outside a session —
booleans, slice, push/pull-through, the merging Follow Me forms — simply
work. Closing the frame regroups: survivors re-home in their original
order, everything created during the session that is still live and
top-level folds in as a new member (grouping, component and instance
creation, import, and 3D text are all legal inside a group frame for
exactly this reason), members released by ungrouping a group made
mid-session from original members keep their original place and are
listed once, members deleted or consumed mid-session drop out, and a
group left with nothing is deleted outright.

Entering a **Component instance** opens an **explode session** — the
transient-*explode* analog, and always the innermost frame (definitions
are flat, so nothing inside one can host a further frame): the
kernel bakes the instance's pose into the definition's member Objects and
sketches — the SAME `ObjectId`/`SketchId`s, a move of ownership rather than
a copy — and retargets them from the definition to the world tree. From
that point every tool sees ordinary world geometry and behaves exactly as
it does anywhere else, because that is what it now is. Only the entered
instance hides (its geometry is the world geometry now; drawing it through
the instance path too would double it). Every other placement of the
definition stays live and shows the edit as it happens — **live sibling
placements**, SketchUp's behavior. There is still exactly one copy of the
geometry, on loan to the world tree, so nothing is kept in sync: a sibling
simply renders the session's world objects through the session's inverse
pose and then its own. That is `Document::expand_def_core`'s on-loan rule:
`expanded_def_placements` of an on-loan definition answers with the frame's
live world objects — the original members plus everything created since
the open and not claimed by a deeper frame — at the inverse session pose,
and with the frame's surfaced and mid-session-placed instances expanded
the same way one level down. A placement inside another definition, and
a sibling nested in a group, render through the same rule, and nested
frames compose, so a sibling of the outer frame shows an edit made two
frames deep. Because a sibling is a real live instance, inference snaps to
it, picks report it, and the Outliner lists it, with no sibling-specific
code anywhere; the renderer's instanced-batch path draws it from the same
cached member mesh it would use for a closed definition, so a session edit
costs one mesh re-pull per changed object, not one per placement. Every
mid-session `DocChange` lists the live placements in `instances_touched`
(`Document::complete_change`; `Scene::session_placements` reads the same
list), exactly as a definition edit lands every instance there. The
gesture BEFORE the commit shows on the siblings too, as it does in
SketchUp: a tool previews in the session's world coordinates (Push/Pull's
swept prism, a Move/Rotate/Scale ghost of the selection, Offset's loops —
each marked as session geometry by the preview builder that makes it,
`markSessionGhost`), and the viewport's sibling preview mirror
(`app/src/viewport/siblingPreview.ts`) draws every marked ghost again
through each sibling's frame — `Scene::session_sibling_frames`, one
world-to-world affine per shown placement image: the inverse session pose,
the nesting poses for a placement reached through another definition,
then the placement's own pose, with the entered instance and any hidden,
tag-hidden, or isolated-away placement left out. The mirrors are clones
sharing the ghost's geometry and materials, built when the ghost appears
and dropped when the preview empties (commit, cancel, Escape, tool
switch). A snapshot of the ghost tree (node identity, geometry, material,
transform, visibility) decides what a frame costs: nothing when nothing
changed, a retarget of the existing clones when the same shape carries
new geometry or transforms (Push/Pull's fresh prism per move, a driven
Move/Rotate/Scale ghost, a child swapped in place), a rebuild only when
the tree's shape changes — so a drag never re-tessellates per move. The
frames are read when a gesture's first ghost appears and again only when
the session fingerprint (entered instance, placement list) changes or the
viewport invalidates them on a document, visibility, or isolation change,
so a mid-gesture undo or hide never leaves a mirror at a stale pose. Two
budgets bound a big model: the frame cap (the first N placements in
kernel order get a mirror) and a total node budget (a big multi-select
ghost gets fewer full mirrors; one too big for even one mirror is drawn
on every frame as a wireframe box of its bounds). The out-of-band
capture render syncs the mirrors too, so a thumbnail never draws clones
of geometry a tool has already disposed. Cursor cues that are not
geometry — a protractor, a leader, a station marker — are not marked and
are not mirrored. What a
sibling cannot be is an operand of a definition-level op while the loan is
out: `make_unique`, `transform_instance`, `apply_def_op`,
`delete_def_member`, and the in-instance surfaces refuse
`ExplodeSessionOpen` on it, since the definition's members are world
objects right now and treating them as def-local content would corrupt
them; the session's own world-path tools are the way to edit what it
shows. The frame's drawing travels the same way: `def_rendered_sketches`
answers, for an on-loan definition, with the session's live world sketches
(the definition's own, baked at open; free sketches drawn mid-session; the
members' owned sketches) and the inverse session pose as their frame, so a
sibling's def-sketch linework follows the drawing as it happens (those
lines snap only through the entered copy, as a closed definition's sketches
snap only through the active instance). Undo inside the session reverts
the siblings with the edit, and undoing the open restores the plain
definition render. A close that empties the definition tombstones the
siblings with it (`emptied_instances`), splices a grouped sibling out of
its group's member list and detaches annotations anchored to one exactly
as `delete_node` would (`emptied_parent_members`, `reanchored`), and undo
revives and re-lists exactly those. Closing the session
(Escape, double-click outside) folds the members back: an Object or sketch
the session touched
unbakes through the inverse pose, one it never touched is restored from a
pristine snapshot taken at open (bit-exact, sidestepping round-trip
floating-point noise entirely for the common case of a session that only
touches some members), and anything drawn or created during the session —
new Objects, new sketches — folds in too, becoming part of the definition
(SketchUp's "what you draw while editing goes into the component"). A
member group ungrouped during the session releases its contents to the
definition's top level, and they are listed as direct members of the
definition at close. A linked copy made in the session wraps a session
member into a set of its own; the close leaves that member in the set and
folds the set's placements in, so they come back as a nested pair. Every
placement of the definition then shows the result.

This works only when the instance's pose is a **similarity** — a rotation,
uniform scale, and optionally a mirror (`Transform::similarity_scale`).
Similarities are exactly the transforms that keep a circle a circle, which
is what lets analytic curve and surface metadata (2.6) survive the
bake/unbake round trip intact. A mirrored pose bakes like any other: the
geometry is mirrored and every face is turned over, so the solid stays
right side out. Group nesting gates a session the same way for the
ENTERED instance: the bake produces top-level world objects, which a
group-nested placement cannot survive coherently (splicing baked members
into a containing group is not modeled). A sibling nested in some other
group does not trip it: nothing hides a sibling, so its group keeps
listing a live member. Session nesting dissolves the common case of the
gate: an instance inside a Group is genuinely top-level while its group's
frame is open, so drilling in through the group opens its explode session
normally. An instance that fails either gate — a non-uniform scale, or a
placement entered directly inside a group — falls back to the previous
in-context editing model instead: the tool set operates on the
definition's members directly, through the pose, without baking or
retargeting anything. That model remains fully supported for exactly
these cases; the session stack is the default path, not a replacement for
it.

The bus can drive the same entry convergence the Outliner uses
(`hew.view.enter`, `exit`, and `exit_all`, HEW_API.md §6.3): the app opens
the same session frames level by level, falls back to the in-context entry
for the same instances, and reports the result as `context_changed` with
`stack` and `fallback`.

The API's path forms use the same through-the-pose model with no session
at all: a move, turn, scale, copy, boolean, or slice addressed to a
member by an occurrence path (`Document::transform_def_members`,
`duplicate_def_members_array`, `boolean_through_placement`,
`boolean_members_through_placement`, `slice_through_placement`) is conjugated, or mapped, through the path's
composed pose (`Document::path_pose`) and applied to the definition in
its own frame, so every placement follows in one undo entry; a
definition on loan to an open session refuses them, because its members
are world nodes for the session's duration. The ones that make new
members (copy, boolean, slice) refuse while any session is open, since a
session close folds every node made since its open into the open
container. `boolean_through_placement` runs only a subtract: a union or
intersect with a world tool would move world material into every
placement, which unlinking is for (Addendum D §5).

The stack also gives in-context resize a natural target: the Tape
Measure's measure-then-type-a-length gesture, which resizes the whole
model at the top level, instead resizes exactly the innermost open
frame's contents about the measured point (`rescale_session`). Under a
group frame the members scale in place — leaf objects bake the transform,
member instances absorb it into their poses, definitions stay at their
authored size; under a component frame the scaled members fold back into
the definition at close, so every placement resizes about its own image
of the anchor. The world outside the frame never moves.

A session that deletes every member leaves the definition with nothing to
fold back into: closing it deletes the definition and every instance of it
outright, matching SketchUp's posture for an emptied component — unless a
live definition still nests it, in which case it stays, empty, because
deleting it would leave that nested placement pointing at nothing (the same
case `delete_definition` refuses). Opening and
closing a session are themselves ordinary, granular undo steps — undoing
past a close visually reopens the session, and every operation performed
inside one is individually undoable forever, exactly like any other
document edit. An open session is transient, in-memory `Document` state
only: it is never part of what `.hew` serializes, and saving (autosave
included) while a session is open transparently writes the document as if
the session had already been closed, without interrupting the edit.

**The change ring.** Every history change the `Document` makes leaves one
session-only record in a bounded ring (256 records, `crates/kernel/src/
changes.rs`): a pushed entry, an undo, a redo, an aborted transaction or
compound, a document replacement. Each record carries the history clock,
the entry's label and origin, and a change set classified from the entry's
`DocAction`s (`document/change_sets.rs`, an exhaustive match: created
entities are added, deleted or consumed ones removed, everything else
touched modified; an import or a rescale is `full`). A bracket that drains
its children into one entry (a transaction, a compound, a definition
delete, 3D Text) replaces their records with its own, so the ring holds
one record per visible entry; while such a bracket is open the ring does
not trim, so its children never push older history out. An abort drops
its children's records and adds one `full` record. The ring is never serialized and is not part of the file format.
The API builds `document_changed` events and `hew.query.changes_since`
from it (HEW_API.md §4.5).

**Rehearsals.** An API dry run (HEW_API.md §6.6) runs an envelope's
commands on one clone of the `Document` with no bracket open, against a
host that refuses every view or selection effect, and drops the clone.
The untouched original stands in for the bracket's snapshot:
`Document::rehearsal_disturbance` makes the same three refusals a commit
would (history disturbed, session or gesture unbalanced) by comparing the
clone with it, so a rehearsal answers exactly what the commit would at
the cost of one clone, and the live document's history, epoch, and change
ring never move. The pure computations (`hew.compute.*`, §7.6) copy only
their operands and run the kernel's pure functions on them
(`Object::boolean_product`, the same product a committed boolean
records). A cut through a placement can take such a transient solid as
its cutter (`Document::subtract_solid_through_placement`): its
`DocAction::Boolean` carries no `b`, so undo has only the host to bring
back.

## 3. Crate and Module Topology

| Crate / module | Responsibility | Depends on |
|---|---|---|
| `crates/kernel` | Half-edge mesh, Document tree (Sketch / Object / Group / Component / Guide), watertightness tracking, booleans, undo, native-format serialization | nothing UI-, I/O-, or network-facing |
| `crates/inference` | Snapping/inference engine: endpoint, midpoint, on-edge, on-face, axis, parallel, perpendicular queries over a spatial index | `kernel` |
| `crates/tessellate` | Kernel topology → render buffers | `kernel` |
| `crates/mesh-heal` | Shared foreign-mesh healing pipeline (weld, dedup, T-junction stitching, orientation, coplanar merge — skipped for `.skp`, whose faces are already the drawn polygons) used by every importer | `kernel` |
| `crates/mesh-export` | STL, 3MF, glTF/GLB, and USDZ writers for `hew.doc.export`, shared by every host that implements it | `kernel`, `tessellate` |
| `crates/dae-import` | COLLADA (`.dae`) → Objects/Instances | `kernel`, `mesh-heal` |
| `crates/gltf-import` | glTF/GLB → Objects/Instances | `kernel`, `mesh-heal` |
| `crates/skp-import` | Clean-room SketchUp (`.skp`) → Objects/Instances, built on the `openskp` reader | `kernel`, `mesh-heal`, `openskp` |
| `crates/skp-export` | Document → SketchUp `.skp` (2026 container or 2017 format) on OpenSKP's writers, and the save back over an opened `.skp` that keeps what did not change | `kernel`, `tessellate`, `skp-import`, `openskp` |
| `crates/stl-import` | STL (binary + ASCII, hand-written parser) → Objects | `kernel`, `mesh-heal` |
| `crates/image-import` | PNG/JPEG → a placed picture: one face painted with it in a component, placed once (the `.skp` image shape); reads only the pixel size from the header | `kernel` |
| `crates/softrender` | Headless render device: a pure software rasterizer over `tessellate`'s buffers, for `hew.view.snapshot` off-screen | `kernel`, `tessellate` |
| `crates/hlr` | Hidden-line removal: projects tessellated solids onto a view plane and returns visible line work — hard edges, curved-wall silhouettes, section-cut outlines — as 2D segments, plus an SVG writer, for vector Line art printing and SVG export | `kernel`, `tessellate` |
| `crates/headless-render` | The renderers behind `hew.view.snapshot` and `hew.view.line_drawing` (selection, background, fitted cameras, view helpers shared with `hew.print.pdf`), pure over `&Document` and shared by `hew-cli`'s `CliHost` and the app's `LiveHost` | `api`, `kernel`, `softrender`, `hlr` |
| `crates/pdfwrite` | Small hand-written PDF 1.4 writer (pages of images, stroked paths, text) for Save PDF… and headless `hew.print.pdf` | nothing in-tree (`miniz_oxide` for DEFLATE) |
| `crates/wasm-api` | `wasm-bindgen` surface exposing the kernel (plus inference, tessellate, hlr, pdfwrite, softrender, headless-render, the api, and the importers/exporters) to the UI | `kernel`, `inference`, `tessellate`, `hlr`, `pdfwrite`, `softrender`, `headless-render`, `api`, `mesh-export`, `dae-import`, `gltf-import`, `skp-import`, `skp-export`, `stl-import`, `image-import` |
| `crates/plugin-manifest` | The plugin manifest (`hew-plugin.json`) parser and validator, pure and shared by every host; `crates/api` reads permission names from it (docs/agents/PLUGINS.md) | `serde`, `semver` only |
| `crates/library` | The Hew Library's on-disk layout: folder resolution, item naming, atomic list/read/write/remove, content hashing — one implementation behind `hew-cli`'s `hew.library.*` and the desktop shell's `library_*` commands | `kernel` |
| `app/` | TypeScript/React UI: viewport, tools, panels | `wasm-api` (via the compiled WASM package) |
| `shells/tauri` | Desktop shell | `app/`, `library` |
| `shells/web` | Static web build | `app/` |
| `workers/share-relay` | The public "Open on Phone" relay Worker (Durable Objects); `crates/hew-relay` is the self-hostable binary that speaks the same contract | none in-tree |
| `workers/bug-intake` | The private Help ▸ Report Bug intake Worker: chunked upload into Durable Objects, notification email, Cloudflare-Access-protected admin page | none in-tree |

### 3.1 Purity boundaries

`kernel`, `inference`, and `tessellate` never depend on UI, rendering,
filesystem, or network crates, and the dependency arrow between the kernel
and its importers only ever points one way: importers depend on the kernel
to build Objects, the kernel never depends on an importer. The WASM boundary
— the only place kernel types cross into JavaScript — lives exclusively in
`wasm-api`. This means every geometric operation can be fuzzed, proptested,
and reasoned about as ordinary Rust, with no UI or platform state entangled
in it, and it means the kernel is trivially reusable from any host that can
run WebAssembly or link a native Rust crate.

Every public kernel mutation runs a topology validator in debug builds
(manifold edges, closed face loops, consistent watertightness flags, no
orphan half-edges), and geometric operations carry property-based tests for
invariants such as: extruding a closed profile is always watertight;
push/pull followed by its exact inverse restores the original topology;
splitting a face and re-merging the split is the identity operation.

### 3.2 Why the kernel runs as WebAssembly everywhere

The kernel compiles to WebAssembly via `wasm-bindgen` and runs inside the
webview on every platform — including the desktop build, where a
Tauri application could instead have linked the kernel natively into its
Rust host process. Running one identical code path everywhere gives a
synchronous, in-memory API with no serialization of geometry across an IPC
boundary, and it means desktop and web builds can share every bug fix and
every behavior without a native/WASM divergence to keep in sync.
WebAssembly runs close enough to native for this kind of geometry work,
and its 32-bit address space is more than sufficient for the model sizes
Hew targets. Kernel purity (3.1) is what keeps a future escape
hatch — a native-process kernel talking to the UI over a binary IPC channel
for very large models — available without having to pay for it today: a
kernel with no UI or I/O dependencies can be relinked into a different host
without being rewritten.

### 3.3 Rendering

`tessellate` converts kernel topology into flat vertex/index/normal buffers;
the UI hands those to three.js, rendered on a WebGL2 baseline. Faces
carrying an analytic surface reference shade with true per-vertex surface
normals, and the facet seams interior to one curved wall are emitted as a
separate soft-edge buffer the viewport suppresses — a drawn cylinder reads
as one smooth wall while its cap rims stay crisp. WebGL2 is the
lowest common denominator reliably available across the three
platform webviews Hew targets (WebView2 on Windows, WKWebView on macOS,
WebKitGTK on Linux), and a SketchUp-class viewport — flat-shaded faces plus
edge overlays — does not need more than that. WebGPU support in those same
webviews is uneven enough that it is treated as a later, strictly optional
enhancement, not a baseline requirement; `tessellate` exists in part to keep
the kernel decoupled from whichever rendering API is in use, so that
enhancement doesn't touch kernel code.

### 3.4 Interop

Interop with formats outside Hew's own format is handled by standalone
converters that sit at arm's length from the core kernel, rather than by
teaching the kernel about any external format:

- **`dae-import`** reads COLLADA 1.4 (`.dae`), the open interchange format
  SketchUp exports natively, then runs it through the shared healing
  pipeline in `mesh-heal` (vertex welding, degenerate-face dropping,
  duplicate-face removal, T-junction stitching, orientation repair,
  coplanar-triangle merge, and hole reconstruction) to reconstruct editable,
  correctly-oriented, watertight-or-honestly-leaky Objects rather than raw
  triangle soup.
- **`skp-import`** reads SketchUp's native `.skp` format directly, built on
  [OpenSKP](https://github.com/hew3d/openskp), a clean-room reader
  maintained as its own project and consumed as an ordinary versioned
  dependency. It shares the same `mesh-heal` pipeline as every other
  importer, and reports where each imported node came from in the file.
- **`skp-export`** writes `.skp` on OpenSKP's writers, in the mapping
  `skp-import` reads back as the same document. Saving back over an
  opened `.skp` starts from the file's own model and reuses its records
  for everything the document did not change, so a file can go Hew →
  SketchUp → Hew without losing what either side does not model.
- **`gltf-import`** reads glTF/GLB through the same importer shape and the
  same shared healing pipeline.
- **`stl-import`** reads STL (binary and ASCII, auto-detected; a
  hand-written parser on `std` — no external STL dependency). STL is the
  crudest source format: an unstructured triangle soup with no shared
  vertices, no object grouping, no names, no materials, and no units, and
  its stored facet normals are unreliable and ignored. The importer welds
  the soup, splits it into geometrically-disjoint shells (so a downloaded
  multi-part plate becomes one Object per part — `mesh_heal::split`'s
  non-manifold cutter alone does not do this, since two untouching shells
  already satisfy the kernel's directed-edge precondition), then runs the
  same shared healing pipeline per shell. STL carries no units, so the UI
  prompts once per import for a unit scale (millimeters default).
- **`image-import`** places a PNG or JPEG as a reference picture (Place
  Image, and `hew.doc.import` of a `.png`/`.jpg`): a component definition
  named after the file holding one rectangular face painted with the
  picture, placed by one instance whose pose is the placement frame — the
  shape `skp-import` gives a SketchUp image. The face's material keeps the
  bytes verbatim with a tile the size of the face, and its UV frame
  stretches that tile over the face once; a face's UV frame rides with
  the face through every transform (`Object::apply_transform`), so a
  picture stays stretched over its face when it is moved, opened or
  resized. Only the pixel size is read (PNG `IHDR`, JPEG frame header and
  EXIF orientation); pictures over 64 megapixels are refused.
- STEP/IGES interoperability (planned — see `docs/agents/ROADMAP.md`) is designed
  as a standalone converter wrapping OpenCASCADE, invoked as an external
  process rather than linked into the kernel — keeping a large C++
  dependency and its licensing entirely out of the Rust workspace.
- Export to interchange formats is one-way and needs no healing machinery
  (it never reconstructs editable kernel topology from someone else's
  mesh). glTF/GLB reads the already-tessellated, already-rendered scene
  data. STL is sourced from the kernel's **export tessellation** instead:
  each object re-facets its stamped cylinder walls from their analytic
  surface references at a user-chosen resolution — true curves for STL,
  manifold at any setting — falling back to stored facets for any wall
  whose boundary is no longer fully analytic.

Every importer depends on the kernel and never the reverse, so the kernel's
public surface has no knowledge of COLLADA, glTF, `.skp`, or STL at all — it
only ever sees the Objects, Instances, and Groups an importer constructs
from them.

### 3.5 Native file format

Hew's own format (`.hew`) is a ZIP container holding a JSON manifest (the
document tree, materials, and metadata) plus binary geometry buffers, one
per Object. It is deterministic — saving the same in-memory document twice
produces byte-identical output — and validating rather than repairing:
loading a file runs the same topology validator a live mutation would, and a
truncated or tampered file fails with a typed error rather than loading into
a silently broken document. The format is documented independently of any
one implementation, versioned, and is the single contract every converter
and the kernel's own serializer must agree with.

## 4. Key Design Positions

These are the positions that still shape the codebase today, stated as
reasons rather than as a history of how they were reached.

**A standalone application, not a plugin for an existing CAD host.**
SketchUp-style direct modeling — no history tree, sticky geometry, immediate
push/pull — conflicts structurally with parametric, history-based kernels
built around a recompute model. Hosting Hew's interaction model inside one
of those systems would mean fighting its document model at every turn rather
than building on it.

**Plugins run sandboxed, never in-process.** Hew will grow a SketchUp-style
plugin ecosystem, and some plugins will be proprietary — but a plugin is a
*separate program* that reaches Hew only through the documented Plugin API,
running in an isolated realm (a worker or separate WASM context in the web
build, a child process under Tauri) and communicating by message-passing. No
plugin loads into the kernel's address space or links against its libraries.
This is binding, for three reinforcing reasons. It makes the boundary the
[Plugin API Exception](../../LICENSE-EXCEPTION) is scoped to — interaction
*solely through the documented Plugin API* — true by construction rather than
by policy. It keeps proprietary plugins clear of the copyleft of any GPL
dependency embedded in the kernel: a sandboxed plugin is mere aggregation,
not a combined work, so OpenSKP's GPLv3 (or any future GPL dependency's) does
not reach it. And it contains an untrusted third party — a plugin can crash
neither the kernel nor the open document, and sees only what the API hands
it. On a WASM stack this isolation is nearly free, where a native host would
pay for it in IPC plumbing.

**A Rust core.** A half-edge mesh kernel is fundamentally pointer-graph
manipulation, and Rust's ownership model turns the failure mode of a broken
mutation from silent memory corruption into a compile error, which matters a
great deal in a domain (geometry) where a corrupted invariant can silently
propagate for a long time before it's noticed. A single toolchain
(`cargo build`/`test`/`clippy`/`fmt`) is also a complete verification loop
with no separate build-system layer to maintain.

**One kernel, two shells.** A single Rust kernel and a single
TypeScript/React UI ship as both a desktop application (Tauri) and a static
web application. Tauri was chosen over Electron for a materially smaller,
better desktop citizen, at the accepted cost of variance across
platform webviews (see 3.3) rather than shipping a bundled browser engine.

**The kernel runs as WebAssembly on every platform, not just the web
build.** Covered in depth in 3.2: one code path, no IPC serialization of
geometry, and a preserved (not prematurely built) escape hatch to a native
kernel process for very large models.

**three.js on a WebGL2 baseline.** The only 3D rendering API reliably
present across every platform webview Hew targets; sufficient for the kind
of viewport a SketchUp-like tool needs; isolated behind `tessellate` so a
later WebGPU path doesn't require kernel changes.

**Interop lives in standalone converters, never in the kernel.** Every
external format — COLLADA, glTF, `.skp`, STEP — is handled by a crate that
depends on the kernel, never the other way around, and that owns all
format-specific parsing and healing logic. This keeps the kernel's surface,
its dependency list, and its test matrix free of any one interchange
format's quirks.

**An open, documented native file format.** The `.hew` container (ZIP +
JSON manifest + binary geometry) is specified independently of any one
implementation and versioned from the start, so it can act as the single
contract between the application and every converter, rather than an
implementation detail reverse-engineered later.

**No dangling edges inside a solid.** A half-edge solid cannot represent a
half-finished cut without breaking its own manifold invariants, so the
kernel only ever accepts complete, boundary-to-boundary cuts. A partially
drawn stroke is tool-layer state in the UI, buffered until it closes; every
state the kernel actually commits is a valid solid.

**Sketches are a first-class entity, never a zero-thickness solid.**
Covered in 2.6. This keeps the watertightness contract meaningful for every
entity that claims to be a solid, and keeps boolean and push/pull operations
from needing a face-versus-solid case alongside solid-versus-solid.

**Stickiness follows editing context, not selection.** Covered in 2.5.
Context is an explicit enter/exit model, generalized to a path through
nested Groups and Components. The enter/exit gesture lives entirely as
application/session state; entering a Component instance additionally opens
a transient, kernel-held explode session (2.11) rather than persisted
document state.

**Booleans resolve coplanar contact exactly, and refuse rather than guess
at pure tangency.** Shared walls, stacked solids, and coincident faces are
common in practice, not edge cases, so the boolean engine classifies
coplanar sub-faces by an exact, tolerance-free coverage test instead of
falling back to a numerically fragile epsilon comparison. A genuinely
ambiguous case (an edge lying exactly on a face with no area overlap) is
refused outright rather than resolved arbitrarily. A Subtract whose cutter
is disjoint from, or only touches, the target is refused as `NothingRemoved`
(the arrangement dropped no face of the target and kept no face of the
cutter, so the target's skin is unchanged), since handing back the target
unchanged would read as a cut that silently did nothing.
Results are also cleaned up: coplanar seams a boolean (or a through-cut
push/pull) introduces are dissolved, so two joined tops read as one face —
while coplanar edges the operands already carried (drawing on faces, not
yet extruded) are preserved, and differing face materials are a hard stop.

**A baked transform for plain geometry; a real pose only for Component
instances.** Move, rotate, and scale write directly into an Object's or
Group's vertex and plane data, keeping the kernel's core representation
pose-free everywhere except the one place — instanced, shared geometry —
where a persistent, invertible pose (including reflection and non-uniform
scale) is the feature, not a complication.

**Groups and Components solve two different problems and stay two
different entities.** A Group is a non-destructive, geometry-free structural
container; a Component definition/instance pair is how geometry gets shared
and instanced. Collapsing the two would make either "share geometry" or
"group without merging" a second-class case of the other.

**Material and texture data are opaque to the kernel.** Textures are stored
as already-encoded image bytes; the kernel never links an image codec. An
affine per-face UV frame is precise enough to reproduce an imported model's
authored texture mapping without needing per-vertex UV storage, which a
flat-faced, solids-first model doesn't otherwise require.

**Push-through is the only way to remove material from a solid.** Covered
in 2.10. Centralizing material removal in one boolean-backed operation is
what makes the "delete a whole node only" restriction on plain deletion safe
rather than merely convenient.

**Units are a display concern layered losslessly over an f64-meter
kernel.** The kernel always works in meters; metric and imperial display
formats (including architectural feet-inches-fractions) are defined as exact
conversions of that one internal representation, so switching units is
never lossy and never touches stored geometry.

**A licensing wall around proprietary format knowledge, enforced as an
admissibility test rather than a blanket ban on a format.** Nothing derived
from a proprietary SDK — its headers, its constants, or format knowledge
obtained under its license — may enter the kernel's dependency chain,
including transitively through an interop crate. What that permits is not
"never support `.skp`," but "support it only through a reader whose entire
provenance is clean" — which is what makes a clean-room reader like OpenSKP,
maintained independently and consumed as an ordinary dependency, an
acceptable way to read a proprietary format without ever touching its SDK.

**Determinism is treated as a kernel invariant, not an incidental
property.** Kernel crates avoid hash-order-dependent collections in favor of
ordered ones, so that the exact same sequence of operations always produces
bit-identical output. This is what makes recording a sequence of operations
and replaying it later a reliable way to reproduce and permanently regress-
test a bug, instead of an approach that only works "most of the time."

## 5. Robustness Model

Robust behavior under floating-point geometry is treated as the project's
central engineering risk, and the mechanisms below exist specifically to
manage it.

### 5.1 Watertightness is a tracked, visible property

Every Object carries an explicit watertight/leaky state as part of its data,
not something inferred by inspection when needed. Operations that could
compromise it are the ones a user has to reach for deliberately (union,
subtract, slice, push-through); an Object that ends up leaky (most often
from import, where the source geometry may genuinely have gaps) is flagged
as such rather than presented as solid.

The UI does not show the bare flag. An open Object is shown by its Model
Health verdict (2.6): a surface reads as a surface, with its own Outliner
icon. The status badge and the import report state how many Objects are
not solid without treating it as a fault (open shapes are normal in
imported models); the Model Health window lists every open Object
(component members once) with its open rims, and Make Solid closes the
ones a person needs as solids.

### 5.2 The topology validator

A validator checks manifold edges, closed face loops, consistent
watertightness flags, and the absence of orphaned half-edges. It runs after
every mutation in debug builds. Because a release WebAssembly build compiles
out debug-only checks, user-reachable mutating operations additionally carry
an always-on validation pass: geometry is cloned, mutated, and validated
before the mutation is allowed to commit, so a failure leaves the original
Object untouched and surfaces as a typed error rather than a crash or, worse,
a silently corrupted document.

### 5.3 No silent geometry repair

If an operation would produce invalid topology, it fails loudly with a typed
error. The kernel never "fixes up" geometry a user's action would have
broken. The one place repair genuinely happens is at an import boundary,
where foreign mesh data is normalized — welded, de-duplicated, re-oriented,
and stitched — before it is ever handed to the kernel's own construction
path; that healing is explicitly bounded to the point where external data
enters the system, and native kernel operations are never subject to it.

Repairing an import after it lands is a separate, explicit thing a person
(or an API client) asks for, never something an import or a diagnosis does
on its own: `cap_rims` closes flat openings with faces, `thicken_sheet`
pulls a flat surface into a solid, `delete_faces` removes faces from an
open Object, and marking a surface sets one attribute. Each is an ordinary
undoable operation that commits only a result the validator accepts and
refuses typed otherwise (`kernel::repair`).

### 5.4 Tolerance policy

All kernel lengths are `f64` meters. Every tolerance is a named constant —
there is exactly one place in the kernel where an epsilon literal is allowed
to appear, and everything else refers to it by name. Native, kernel-
constructed geometry is held to a strict nanometer-scale tolerance for point
coincidence and planarity. Geometry arriving through an importer is held to
a separate, deliberately wider planarity tolerance, because coordinates from
a single-precision source format are quantized far more coarsely than that —
a face a user drew flat can arrive a fraction of a millimeter off its own
best-fit plane for reasons that have nothing to do with whether it's
actually warped. Using one strict tolerance everywhere would either reject
huge fractions of real-world imported models outright, or loosen native
geometry's own gate to the point that a native operation producing a
genuinely non-planar face would go undetected; keeping the two tolerances
separate avoids both failure modes.

### 5.5 Determinism and replay

Kernel crates deliberately avoid iteration-order-dependent collections, so
that the same sequence of operations always yields the same result down to
the byte — this is what makes the native file format's own
byte-for-byte determinism (3.5) possible, and what makes a recorded sequence
of operations a reliable regression test rather than one that only
reproduces a bug intermittently.

### 5.6 A layered test strategy

Geometric correctness is tested at the layer where it is cheapest to reason
about, from the bottom up:

- **Property-based tests** at the kernel layer assert invariants that must
  hold for *any* valid input, not just a handful of example cases: extruding
  a closed profile always yields a watertight solid; push/pull followed by
  its exact inverse restores the original topology; splitting a face and
  re-merging the split is the identity operation.
- **Deterministic replay** exercises recorded operation sequences headlessly
  against the same kernel a real session would drive, without a browser or
  GPU in the loop, asserting against a known-good result hash.
- **End-to-end tests** are kept comparatively thin, reserved for the flows
  that only exist once the real UI, rendering, and input pipeline are all in
  the loop together.

Loading the native file format itself runs the same validator a live
mutation does, so a corrupted or hand-tampered file is rejected outright
rather than loaded into a document that looks fine until something touches
the wrong triangle.

### 5.7 History replay is guard-exempt, and carries proof

Some operations run best-effort **obstruction guards**: heuristics that
refuse a sweep whose result would stay manifold — and therefore pass the
topology validator — while self-intersecting in space (a recess pushed
deeper than the material beneath it, a boss driven through geometry in
front of it). Guards read the geometry *surrounding* an operation, and they
are deliberately conservative: refusing a legal forward operation costs the
user a retry, while accepting an illegal one silently corrupts the model.

That conservatism must never break undo. Undo and redo are LIFO — a
recorded inverse dispatches against exactly the state its operation
produced, and a redo dispatches against exactly the state its operation was
originally accepted in — so the state a replayed operation re-enters is one
the kernel has *already accepted*. There is nothing left for a heuristic to
protect against, and a heuristic must not be able to refuse the replay. The
History therefore dispatches recorded inverses (undo) and recorded forward
ops (redo) in a distinct **replay mode**:

- **Guard-exempt.** The obstruction heuristics are skipped. Everything
  structural still runs: input checking, the surgery's own typed refusals,
  the debug invariant checker, and the always-on release validator. Replay
  mode never disables validation — only the heuristics whose job the proof
  below does better.
- **Proof-carrying.** The exemption is not taken on faith. When an entry is
  recorded, the History captures a geometric fingerprint of the state the
  replay must reproduce (for an inverse, the state before the op; for a
  redo, the state after it): every face's outer and hole rings plus its
  plane. The replayed op runs on a clone and commits only if its result
  matches that fingerprint, up to the same tolerance-aware equivalence the
  round-trip property tests use. A mismatch is a kernel bug by definition,
  surfaced as a typed error with the object untouched — undo can fail typed
  on a kernel bug, but it never corrupts, and it is never refused by a
  heuristic.
- **Exact, not merely close.** On a successful match the committed clone is
  aligned to the recorded coordinates (vertex positions and face planes).
  A replayed op recomputes geometry, and floating-point round-trips are not
  bitwise (`(p + d) − d ≠ p`); committing recomputed coordinates would let
  that noise accumulate across undo/redo cycles — refit normals amplify it
  by sweep-distance over face-extent — until a marginal tolerance decision
  inside a later replay flips and refuses an op its forward pass accepted.
  Alignment makes every replay re-enter the exact bits of the recorded
  accepted state, so replay is idempotent and noise cannot accumulate. This
  is not geometry repair (§5.3 is about masking invalid results); it is the
  definition of undo/redo, and the aligned state still passes the full
  validator before committing.

This is DEVELOPMENT.md rule 9. The alternative — holding every guard to one
shared fidelity and trusting that a replayed op re-derives the same
accept/refuse decision at replay time — was rejected because it cannot be
made exact: guards compare tolerances against recomputed positions,
floating-point round-trips are not bitwise (`(p + d) − d ≠ p`), and every
future guard would re-open the gap. Verifying the replayed *result* against
the recorded state discharges the guards' entire purpose without depending
on how any guard is implemented.

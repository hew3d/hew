---
title: "Drawing"
description: "The five profile tools, drawing on a face exactly as on the ground, offsetting a boundary, editing what you drew, and every way to type an exact dimension."
order: 5
---

Hew's five drawing tools create 2D profiles. On the empty ground plane, a closed profile becomes a **sketch region** ready to extrude. Drawn directly on a solid's face, the same tools work the same way: the face is a drawing plane just like the ground, and a closed profile on it becomes a region you can push in or pull out. The tool decides which plane you are on from what's under your cursor; there is no explicit switch.

<docs-video theme="dark" slug="drawing" label="drawing demonstration"></docs-video>

![Four filled sketch regions on the ground: a rectangle, a circle, a hexagon, and a closed arc segment](/docs/drawing-shapes.webp)

The five tools share one working sketch on the ground, so shapes drawn with different tools combine: draw an arc, switch to Line, and close it with a chord — the face forms the moment the circuit closes, exactly as if one tool had drawn it all. An arch profile, for instance, is three Line segments topped with an Arc, extruded once.

Layered shapes edit like a drawing, too. Where two closed shapes share a line, that line splits them into separate regions; select it with the Select tool and press `Delete`, and the regions merge into one larger face. Draw a rectangle, cap it with an arc, delete the line between them — one arch-shaped region, one pull.

A closed loop of lines can also have no face at all: that is how a wire frame or a light rig opened from a SketchUp file arrives, since SketchUp has no face there either. Such a loop draws as lines only and has nothing to push or pull. Draw over one of its lines with the Line tool, from one end to the other, and the loop gets its face, just as it does in SketchUp.

Selection matches what you drew. Clicking a straight line selects just that line. Clicking a drawn arc or circle selects the curve up to wherever it crosses or touches other geometry — an untouched circle selects whole; a circle welded into an outline selects one run at a time, so you can delete the part inside without losing the rounded corner. Clicking inside a filled region selects the connected shape it belongs to, and shapes drawn apart from each other are independent: selecting, deleting, or moving one never touches the other, even though they share the working sketch.

While any drawing tool is active, the status bar explains the next step, the top-right readout shows live dimensions, and everything you type goes into that readout (see "Typing exact values" below).

As in SketchUp, a tool's first two points also take a drag: press on the first point, drag, and let go on the second (a rectangle's two corners, a circle's or polygon's center and rim, an arc's chord, an offset's start and end), and a value typed right after you let go applies just as it would after a click; a short press is still a click, so click, move, click works exactly as before.

## Line (`L`)

Draws a chain of straight edges.

1. Click to anchor the first point.
2. Each further click commits one segment and continues the chain, with a rubber-band preview to the cursor.
3. Closing a loop (clicking back on your starting point, or completing any closed circuit) automatically forms a face and ends the chain.

To finish a chain *without* closing it: click the last point again (or double-click it), press `Enter` with nothing typed, or press `Esc` once. Pressing `Esc` again cancels the tool's gesture entirely.

Press on the first point, drag, and let go on the second instead, and you get a single segment: the chain ends there, ready for a new line, and a length typed right after you let go resizes that segment.

**Exact lengths:** once a chain is started, type a length (e.g. `750mm`) and press `Enter` — the next segment commits at exactly that length along the direction you're pointing.

Type a length right after a segment commits, before moving the cursor, and it resizes *that* segment instead, along the direction it was already drawn — the chain then continues from the new end point. Move the cursor first and a typed length goes back to drawing the next segment, as above (see "Typing exact values" below).

**Axis locking:** hold `Shift` to lock the segment to whichever axis it's already leaning toward, or press an arrow key for an explicit lock: `→` locks to X (red), `←` to Y (green), `↑` to Z (blue). Press `↓` (or the same arrow again) to unlock.

**Axis inference:** you usually don't need the lock at all. Move roughly along an axis and the segment snaps onto it by itself, the rubber band turning that axis's color and the tooltip naming it. Move away and it lets go — it's a suggestion, not a commitment. A held lock draws a heavier line, so you can always tell which one you have. A point you snap to always wins over an inferred axis, and so does a construction guide you placed yourself.

**Locking after the first click:** an arrow key works at any point up to the moment a segment commits, not just before you start. Locking to an axis that leaves the plane you're drawing on is allowed — the line keeps going into space, and Hew starts a fresh sketch on the plane that contains your last segment and the new direction, so an L-shape drawn this way stays on one plane and can still close a face. What it won't do is silently drag the point back onto the old plane. Starting on an edge two faces share — the top corner of a box, the midpoint of its top edge — and then locking to an axis that runs along the *other* face switches the line onto that face, so `→` from a box's top-west edge draws across the top, not into the side. While locked, every point you hover projects onto the locked line: the chip names the point ("Midpoint, on red axis") and a dotted tie runs from that point to where it lands on the line.

**Closing a shape from an earlier point:** draw two sides of a rectangle — say up the green axis, then over along red — and start the third heading back the way you came. Hew notices you're aiming at the line the *first* side would make if it kept going, shows a dashed guide through that earlier corner, and pulls your cursor onto exactly where the two lines cross. Click there and the last side closes the rectangle exactly, corner to corner, with no need to eyeball it. Like axis inference, this is only a suggestion: get close enough (a few pixels) and it takes over; a real snap point always wins over it, so it never gets in the way of something more precise underneath.

One consequence worth knowing: an outline that ends up spread across two planes can't become a single face, because a face needs one plane. If you close a shape like that back onto its own start point, Hew tells you so rather than leaving you wondering why no face appeared.

## Rectangle (`R`)

1. Click to set the first corner.
2. Click again to set the opposite corner.

**Exact dimensions:** after the first click, type both dimensions separated by a comma or an `x` (`2m,1m`, `50 x 30`, `3',18"`) and press `Enter`. A single value makes a square. The first number is the width across the surface you're drawing on and the second is the height up it; on the ground and other flat surfaces that means along the red axis, then the green. The Measurements box shows the pair in the same order while you drag. The rectangle grows in the direction your cursor was heading; if the cursor hasn't moved since the click (a click on a snapped guide intersection, say), it grows right and up as you see the face from outside — toward +X and +Y on a horizontal one.

A click works the same way: what the Measurements box shows when you click the second corner is exactly what you get, as if you had typed it, unless the click snapped onto existing geometry, which keeps its exact point.

You can also type dimensions *after* the second click. The rectangle you just drew resizes in place, keeping the same first corner and growth direction — retype and press `Enter` again as many times as you like, until you click, press `Esc`, switch tools, or make any other change to the model. Every drawing tool works this way; see "Typing exact values" below.

## Circle (`C`)

1. Click to set the center.
2. Move outward (the readout shows the radius) and click to set a point on the rim, or type an exact radius and press `Enter`.

**Standing a shape up:** the arrow keys choose which plane you're drawing on, and they work *after* you've placed the center as well as before. Press `→` with a center already down and the circle stands vertical, hinged through that center — the same for Rectangle, Polygon, and an Arc you haven't finished. Press the same arrow again to release it.

Note the difference from the line tool: there, an arrow picks the *direction* a segment runs, so `↑` draws straight up. Here an arrow picks the *plane's own axis* — the direction it faces — so `→` and `←` give you upright planes, and `↑` gives you a flat one at the height of the point you clicked.

Hew's circles are stored as regular polygons, but they remember the exact circle you drew — center and radius ride along with the shape, power the Center/Quadrant/Tangent snaps, keep extruded walls smooth on screen, and let STL export re-facet the wall at whatever resolution you pick. The stored facet count adapts to the size of the circle: small circles get 24 sides, larger ones up to 96, keeping the worst chord deviation at about half a millimeter.

Type a radius after the second click, too: the circle you just drew redraws around the same center, at the new radius (see "Typing exact values" below).

## Polygon

A regular N-sided polygon, for hex bolt heads and nuts, gear blanks, standoffs, and dice:

1. Click to set the center.
2. Move outward — a preview shows the polygon with one vertex under the cursor, rotating to follow it — and click to commit, or type an exact radius and press `Enter`.

**Sides:** six by default. Type `Ns` at any point during the gesture (`8s` for an octagon) to change the side count — it can be typed before or after the radius, and re-typing it updates the preview immediately. The count is clamped between 3 and 120; above that, Circle is the right tool. The side count you last used carries over to the next polygon you draw, so a run of hex nuts doesn't need retyping `6s` each time.

Unlike Circle, a polygon carries no hidden analytic shape — the straight facets you see are its exact geometry, stored as plain edges like any other drawn line. The typed radius is the **circumradius**, center to vertex, matching SketchUp's Polygon tool.

Both a radius and `Ns` also work after the second click: the polygon you just drew redraws at the new radius, the new side count, or both, around the same center (see "Typing exact values" below).

## Arc (`A`)

A two-point arc, like SketchUp's:

1. Click one endpoint.
2. Click the other endpoint to set the chord.
3. Move perpendicular to the chord to pull out the bulge, then click to commit.

`Esc` steps back one stage at a time. The readout shows the arc's radius. Typed values work at both stages: in the chord stage a typed length places the second endpoint at that distance, and in the bulge stage it sets the bulge depth. A flat, zero-bulge arc is refused ("Pull out the bulge"). Arcs facet at the same density as circles of the same radius, and like circles they carry their exact center and radius for snapping, smooth display, and export.

A typed length after the third click works too: the arc you just drew redraws with that bulge, on the same side of the same chord and with the same open/pie/segment closure (see "Typing exact values" below).

**Closing the arc:** press `Option`/`Alt` mid-gesture to cycle what the commit produces — the open arc, a **pie** closed to the center with two straight edges, or a **segment** closed with the chord. The preview draws the closing edges and the readout names the mode. Both closed forms are complete profiles: on the ground they become a region immediately, and on a face they split it, ready to push/pull. An arc keeps its curve on a face exactly as on the ground: push or pull the region and the arc's walls come out smooth, with center and quadrant snaps on their rims, while the closing lines make flat walls. The chosen mode sticks for further arcs until you switch tools.

## Drawing on a face

Point any drawing tool at a solid's face and draw. A face is a drawing plane exactly like the ground, so every gesture in this chapter does the same thing on a face that it does on the ground: the same shapes, the same snapping, the same selecting, moving, and deleting. What changes is only where the result lives.

Two colors tell you what a drawing belongs to. **Orange** drawing is part of a solid's face: a shape drawn on a face, and a loose line on it, show in orange, and pushing or pulling an orange region changes the solid itself. **Blue** drawing belongs to no solid: anything you draw on the ground or in empty space, a detached shape, and a shape inside a group show in blue, and pushing or pulling a blue region makes a new object. While you draw, the outline is orange only when the shape will land on a face; on the ground, or with Detached on, it is blue. A shape that hangs over a face's edge shows orange where it is part of the face and blue where it lies past the edge. Moving a blue shape onto a face turns it orange when it joins that face, and a detached shape stays blue wherever it lands.

- **A closed shape over the face** becomes part of the solid's face, cutting it into regions. Push or pull a region and you get a recess, a through-hole, or a raised boss. Shapes can touch the face's own edges, run flush against them, cross each other, or sit inside each other; the face splits into the same pieces the ground would.
- **A loose line stays.** A line that starts or ends inside the face, without closing a loop or reaching an edge, is kept as it is, like a construction line on the ground. It belongs to the solid: move the face by push/pull and the line goes with it, and deleting the solid deletes it.
- **A shape that hangs over the edge is split in two.** Draw a rectangle half on a box's top and half past its edge. The part over the face is a face region (push or pull it and the box grows or is cut), and the part past the edge is a region of the ground-side drawing (push or pull it and you get a new, separate object). The two parts stay one shape for selecting and moving, and you can push the pieces in either order.
- **Shapes that touch at a single point are faces too.** A circle whose rim sits on the face's edge, two circles touching, a shape meeting another's corner: each becomes its own region, and you can push or pull it, or the face around it.
- **A shape the face can't hold** — one that leaves a sliver too thin to keep — is still drawn and kept. It just can't be pushed: when you try, Push/Pull tells you which part is the problem so you can move it clear.

### Detaching a shape

A shape drawn on a face normally joins that face's drawing: it cuts the face into regions, and a shape that hangs over the edge splits in two. To draw a shape that stays whole instead, tap `Ctrl` (`⌘` on a Mac) with Line, Rectangle, Circle, Polygon, or Arc active. The cursor gains a small `+`, the readout gains a "Detached ·" prefix, the hint at the bottom says Detach is on, and the outline you are drawing turns from orange to blue. Orange means the shape will join the face under it; blue means it stays apart, as with Push/Pull's extrude-as-new.

A detached shape touches nothing until you push it. It sits on the plane as a sketch of its own, and the face under it keeps its edges and regions. Snapping still sees it, and it snaps to everything else. Later drawing never joins it, and Push/Pull on it makes a new object, never a change to the face. Detached stays on until you tap the key again or switch tools.

There is no command that joins a detached shape to a face afterward. To change your mind, delete it and draw it again without the Ctrl tap.

A solid's own edges are not part of the drawing. You can snap to them and draw up to them, but drawing never moves or erases them, and the Select, Move, and Delete tools treat the solid's corners and edges as the solid's.

At the top level, any plain object's face is drawable directly. Faces inside a **group or component** are the exception: double-click in first, and drawing is then scoped to what you entered. Starting a line on an edge or corner two faces share is fine; the face the line runs across is settled by where you put its second point (or by an arrow-key lock), so a line from the midpoint of a box's top edge to the midpoint of the opposite one cuts the top, not the side.

A line chain on a face may run from edge to edge, cross shapes you've drawn there, or wander across the middle; an arc arched over a rectangle splits it. Each segment is its own step, so `Ctrl+Z` takes the chain back one segment at a time. A circle or arc you drew stays one **Circle** or **Arc** when other shapes cut it apart, so you can select it whole and delete it in one step.

![A box with a circular boss pulled out and a circular recess pushed in, both drawn directly on its front face](/docs/drawing-on-face.webp)

## Drawing on other planes

Nothing ties the drawing tools to the ground. Every shape lies on *some* plane, and the ground is simply the one the tools fall back to when nothing else is under the cursor. There are three planes to think in — the ground and anything parallel to it is a **blue** plane (it faces the blue Z axis), a wall running north–south faces the **red** X axis and is a red plane, and a wall running east–west faces the green Y axis and is a **green** plane — and several ways to draw on one that isn't the ground.

<docs-video theme="dark" slug="drawing-planes" label="drawing on other planes demonstration"></docs-video>

**Face the plane.** Orbit until a wall faces you, and shapes you draw in empty space land on that wall's plane. Hover with the Rectangle, Circle, Polygon, Arc, or Line tool and a tinted grid patch shows the plane under the cursor, with a status-bar line naming it ("Facing the green plane"). Press a Front, Back, Left, or Right standard view and it happens on its own. From above, or from any ordinary three-quarter view, you stay on the ground; a wall takes over only once the camera drops to roughly 20 degrees above level when you face it squarely, a little lower when you are turned away from it. The plane runs through the drawing axes' origin, so everything drawn this way from one view is coplanar. Press `↑` for the ground, or `→` or `←` for a red or green plane, and the choice is yours instead; press the same arrow again and the view decides again. To draw on a wall at a specific depth, start on geometry there or hover a face and press `Shift`. The plane is settled at your first click and stays put if you orbit afterward. Hover a shape you drew earlier and the patch shows its plane, so you can see the next shape will join it.

**Use a face.** If the plane you want already exists as a face — the side of a box, the top of a slab — point the tool at it and draw. That's the "Drawing on a face" case above: the shape cuts the face, and the piece it encloses is ready to push or pull. This is the everyday way to draw a window in a wall or a hole through a plate. Starting on the face's edge or corner counts as starting on the face: click the midpoint of a box's vertical edge and the shape goes on the wall that edge belongs to, never on the ground under it. At a corner or an edge shared by two faces, it goes on the face the cursor is leaning toward: slide onto an upper corner from the front face and the patch stays on the front, nudge a few pixels up and it flips to the top, and a nudge off the box to one side gives that side's plane, extended past the solid. The patch shows which before you click.

**Pin the plane with `Shift`.** Hover the face whose plane you want — the side of a box, say — and press `Shift`. The status bar reads "Pinned to the hovered plane", and from then on the cursor stays on that plane wherever it goes: past the face's edge, over the ground, across another solid. Click twice and the shape lands on that plane as an ordinary sketch, even where there is no face beneath it — the way to continue a wall's plane out past the wall. Hovering empty space and pressing `Shift` pins the plane the patch is showing (the ground, or a wall you are facing), and hovering the ground pins the ground itself, so a shape drawn over a box still lands underneath it. The pin stays on for the next shape too; press `Shift` again, or `Esc`, to release it. The Line tool takes the same pin before its first click (mid-chain, `Shift` locks the segment's axis instead, as above).

**Lock the plane with an arrow key.** Rectangle, Circle, Polygon, and Arc all take a plane lock before the first click: press `→` and the status bar reads "Locked to the red plane"; `←` is the green plane, `↑` the blue one. The first click is free — snap to a corner, a midpoint, or a bare spot on the ground — and the shape is drawn on the locked plane *through that point*, standing in empty space with no face beneath it. The preview and its dashed helper take the plane's color so you can see which way it faces before you commit. Press the same arrow again, `↓`, or `Esc` to release the lock. The arrow works after the first click too, hinging a shape you've already started up through its anchor (see "Standing a shape up" under Circle). What you get is an ordinary sketch region, so Push/Pull extrudes it sideways exactly as it would extrude one on the ground — a vertical rectangle pulled along the ground is a wall.

**Point the Line tool into space.** Line needs no plane up front, because it works one segment at a time. Draw a segment along the ground, press `↑`, and the next one runs straight up from its end; `→` and `←` run segments along red and green. The moment a segment leaves the plane you were on, a fresh sketch begins on the plane that holds your last segment and the new direction, and everything you draw from there stays on it: up, across, and back down closes into a vertical face standing on that first ground line. The face's own corner is the point where the chain left the ground, so close on that one — the segment you drew before it belongs to the ground sketch and stays there.

Getting Started walks through all of this on a real part — [drawing a cutter on the side of a bin](/learn/getting-started/#7-work-on-a-different-plane).

## Offset (`F`)

Offset copies an existing boundary a set distance inward or outward — the quickest way to draw a border, a rim, or a wall of even thickness without measuring anything.

1. Click a filled region — its interior or right on its boundary edge, both work — on the ground, on a solid's face, or on a shape drawn there.
2. Move the cursor inward or outward — a preview of the offset outline follows at the distance shown in the readout.
3. Click to commit, or type an exact distance and press `Enter`.

Every edge of the boundary moves by the same distance: straight edges stay parallel, and a drawn circle or arc offsets to a true concentric curve — same center, radius changed by exactly the offset — so the result snaps, displays, and exports like any freshly drawn curve. A region with a hole offsets both boundaries at once, keeping the band between them even.

Offset works the same on the ground and on a face. On the ground, the offset outline joins the same drawing, so both the original and the new region are ready to extrude — offset a circle outward and you have a ring to pull into a pipe wall. On a solid's face, offsetting inward leaves an inset region to push or pull. Offsetting outward is fine too, even past the face's edge: the loop splits where it crosses the edge, with the part over the face ready to push or pull the solid and the part beyond ready to extrude into a new object. An arc in the outline offsets as a concentric arc and keeps its curve, so pushing the inset down hollows the shape with an inner curve as smooth as the outer one.

If the distance is more than the shape can absorb — an inward offset past the middle, or an arc squeezed to nothing — the preview disappears and committing is refused with a message; nothing is committed halfway.

Type a distance right after the click commits too: the offset you just made redoes at the new distance — positive keeps the same inward or outward direction, negative flips it — as many times as you like, until you click a face or profile, press `Esc`, or switch tools.

## Editing a sketch

Everything you draw is made of four kinds of unit, and the Select tool picks the one you click:

- A **line** (a single straight stretch), picked by clicking it.
- A **curve run**: a drawn circle, arc, or polygon, or the piece of one between the places other geometry cuts it. An untouched circle is one run; a circle crossed by a line is two, and clicking one selects just that run.
- A **shape**: everything connected, which you get by clicking inside a filled region, or by triple-clicking (or double-clicking) any line or curve of it.
- A **region**: the filled area itself, which is what Push/Pull and Offset pick.

A rubber-band drag selects the shapes it encloses (or touches, dragging right to left), on the ground or on a face. Start the drag off a solid; pressing on a solid and dragging moves it. Shapes drawn apart from each other are independent: selecting, deleting, or moving one never touches the other.

**Deleting a line or curve.** With the Select tool, click a line or curve run and press `Delete`. If it separated two regions, they merge; if it closed a region, the region opens back up. Deleting a shape's last line removes it entirely. Every line you can see is an ordinary line: pulling a region into a solid removes its outline from the drawing (the outline became the solid's base), so nothing invisible lingers behind to complicate later edits. A solid's own edges are the exception in the other direction: you can't select them as drawing, so `Delete` can't erase one.

**Deleting or moving a whole shape.** Click inside a shape's filled region (or its row in the outliner) to select the shape, then `Delete` removes it or Move slides it, without disturbing anything else you've drawn. On a face it works the same way: a shape drawn there selects when you click inside it, and clicking anywhere else on the solid selects the solid.

**Moving, copying, and transforming.** Move, Rotate, and Scale work on a shape on a face exactly as on the ground, and a copy-move makes a copy there too. Move a shape onto other shapes and it joins them exactly as if you had drawn it there: lines split where they cross, overlapping lines merge, and the regions re-form, all as one step that a single `Ctrl+Z` takes back. This is true on the ground and on a face alike; nothing is refused for crossing another shape. A shape can slide right off a face's edge: it splits into a part on the face and a part beyond, and slides back together if you bring it back. Rotating a shape out of its plane, standing a profile upright, splits it into a sketch of its own so the rest of the drawing stays put. A detached shape never joins what it lands on. Once you push or pull a region, it is solid geometry and no longer a shape you can move; undo the push to get the shape back.

**Moving a point.** The **Edit Vertex** tool (Tools ▸ Edit Vertex, or find it in the palette) adjusts a single point of a drawing, on the ground or on a face:

1. Click a vertex to grab it.
2. Click its new position — the connected edges stretch to follow.

Drag a vertex across or onto other lines and the drawing arranges itself the way a drawn line would: lines split where they cross and the regions re-form, as one undoable step. If a move would collapse a segment to nothing, Hew refuses with a message and leaves the vertex where it was. A solid's own corner isn't a drawing vertex, so it can't be grabbed this way.

**Circle segments.** Select a drawn circle and change **Segments** in Object Info to refacet it. This works on a face as it does on the ground.

## Inference while drawing

Every click snaps. The colored dot and label at the cursor tell you what you're about to snap to — Endpoint (green), Center and Quadrant of a drawn circle or arc (teal), Midpoint (cyan), Intersection (amber — where a construction guide crosses an edge, a sketch line, or another guide, where edges of two separate objects pass through each other, or where an edge of one object passes through a face of another, such as a panel's edge entering the wall it is set into), Tangent (violet — where an in-progress line just grazes a drawn circle), On Edge (red), On Intersection Line (orange-red — the line where a face of one object meets a face of another, such as the four lines a slab passing through a wall draws on the wall's face; objects never merge, so it is not a real edge, but you can slide along it, its middle is a Midpoint, and its ends are Intersections; where it runs along a curved face, such as a slab through a cylinder, it has no Midpoint), On Face (blue), On Guide (purple), On Axis (the axis color), or Ground (gray). Lines you draw across each other need no cue of their own: crossings become real endpoints the moment they're drawn. A dashed helper line appears through the snap point when the snap has a direction, such as an axis alignment, and the line where two faces meet is drawn in full while you hover it. Construction guides let you add snap targets of your own; [Precision, measurement, and guides](/learn/measurement-and-guides/) covers them.

## Typing exact values

There's no input box to click, and none appears. With a tool mid-gesture, start typing: what you type appears in the top-right readout, `Enter` commits, `Backspace` edits. Every length-driven tool accepts:

- A bare number, read in your current display unit (`1.5` = 1.5 m in Meters mode, 1.5" in an imperial mode).
- An explicit unit that overrides the display unit: `mm`, `cm`, `m`, `km`, `in`, `ft` — `250mm`, `3.5cm`, `6"`, `2'`.
- Feet-inches-fractions, SketchUp style: `5'3"`, `5' 3-1/2"`, `3 1/2"`, `5/8"`.

**Right after the click, too.** Any measurement you can type *before* a shape's committing click, you can also type immediately *after* it: the shape you just drew redraws at the new value, in place — same anchor point, same direction, same everything except the number you changed. Retype and press `Enter` again as many times as you like, until you click, press `Esc`, switch tools, or make any other change to the model (an undo included); a single undo afterward removes the shape entirely, no matter how many times you retyped it. Each tool's section above notes exactly what redraws.

The modify tools work the same way right after *their* commit: Push/Pull, Offset (above), Move, Rotate, and Scale each redo the operation you just made at a newly typed value rather than starting a fresh one. [Push/Pull](/learn/push-pull/) and [Move, Rotate, and Scale](/learn/moving-and-transforming/) cover exactly what redoes for each.

Angle tools (Rotate, Protractor) take plain degrees; Scale takes a plain factor, or a length in any of the forms above as a target dimension for the axis being dragged. Display units are set in **Settings ▸ Units** ([Settings](/learn/settings/)).

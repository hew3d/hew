---
title: "Move, Rotate, and Scale"
description: "Move, Rotate, or Scale with full snapping, axis locks, and typed values — or drag an object straight from Select. Copying is a Move with the copy modifier tapped — and ×N multiplies it into an array."
order: 9
---

Move, Rotate, and Scale all work on the current selection — but they don't demand one. With nothing selected, clicking an object with any of the three selects it and starts the gesture in the same click, so moving something is one fluid motion, not Select-then-Move. With a selection already made, the click places the first reference point as usual.

<docs-video theme="dark" slug="moving-and-transforming" label="moving and transforming demonstration"></docs-video>

The fastest move doesn't even need the Move tool: with **Select** active, press on an object and **drag** it. Past a few pixels the drag becomes a real move — full snapping, axis locks, the copy modifier, typed distances — and releasing the button commits it. The tool stays Select throughout, like dragging an icon on your desktop. Dragging a member of a multi-selection drags the whole selection.

All three transform tools show a live ghost preview of the result and a live readout, and all three accept typed exact values mid-gesture.

## Select (`Space`)

A quick refresher:

- **Click** an object, sketch, or guide to select it, replacing whatever was selected before. Clicking empty space clears the selection.
- **Triple-click** a line, or a drawn arc or circle, to select its whole connected shape instead of just the one line or curve run under the cursor. This works on the ground and on a solid's face alike.
- **Drag an object** to move it (see above). `Esc` mid-drag cancels the move and puts everything back.
- **Drag from empty space** to rubber-band a selection, SketchUp-style: dragging left→right draws a solid rectangle and selects what falls **entirely inside** it; dragging right→left draws a dashed rectangle and selects everything the rectangle **touches**.
- **Double-click** a group, component, or object to enter its editing context (the rest of the scene dims); press `Esc` to step back out.

### Selecting more than one thing

A click or a drag replaces the selection by default. Add a modifier — to a click or a drag alike — to build up a selection instead:

- **`Shift`** toggles: whatever it touches is added if it wasn't selected, removed if it was.
- **`⌘` or `Option`** (Windows/Linux: `Ctrl` or `Alt`) always adds, never removing anything. On a Mac, `Ctrl`-click is a right-click, so it isn't one of these.
- **`Shift` + `⌘`/`Option`** (Windows/Linux: `Shift` + `Ctrl`/`Alt`) always subtracts, never adding anything.

The same three combinations work on a rubber-band drag, window or crossing.

Beyond clicking and dragging, **Edit ▸ Select All** (`⌘A` / `Ctrl+A`) selects every visible object, group, component, and drawn shape (on the ground or on a solid's face) — the whole model (inside a group's editing context, that group's contents instead); **Edit ▸ Select None** (`⇧⌘A` / `Ctrl+Shift+A`) clears the selection; and **Edit ▸ Invert Selection** swaps the two, selecting everything that wasn't selected and deselecting everything that was.

## Move (`M`)

1. Click a **base point**. Pick a meaningful one, like a corner you want to land somewhere. (With nothing selected, this click also selects the object under the cursor.)
2. Click the **destination**. The base point lands exactly there, snapping to anything the inference engine finds.

Move, Rotate, and Scale also take a drag: press on the first point (Move's base point, Rotate's pivot, a Scale grip), drag, and let go where the second click would go; a short press is still a click, and a value typed right after you let go applies just as it would after a click.

**Exact distance:** after the base point, type a length (`1.5m`, `8"`) and press `Enter` — the selection moves exactly that far in the direction you were dragging (or along the locked axis).

**Axis locking:** hold `Shift` to lock to the dominant axis of your drag, or press `→` for X, `←` for Y, `↑` for Z (`↓` clears). The preview line takes the axis color.

**Copy instead of move:** tap `Option` (Mac) / `Ctrl` (Windows, Linux) — copy mode switches on and stays on, with the readout prefixed "Copy ·", a `+` badge on the cursor, and the status bar confirming it. Because it's a toggle rather than a held key, everything else works exactly as in a plain move: type an exact distance and press `Enter` to place a copy at a precise offset. The original stays put, the copy lands at the destination and becomes the new selection, so repeated moves chain copies. Sketch shapes copy too, keeping their curve identity — a copied circle is a true circle, center snap and all. Copy a shape within its plane and the duplicate is redrawn the way hand drawing would be, so if it lands on other lines they split each other exactly as drawn lines would. That holds on a solid's face too: copy a shape across a face, or past its edge, and the copy splits into a part on the face and a part beyond. Copy a shape *off* its plane — lifting a ground profile straight up, say — and the copy arrives on a new sketch on the plane it landed on, with the original untouched (handy for [Follow Me](/learn/follow-me/): copy a profile up instead of moving the only one you have). Tap the same key again to go back to moving.

**Retyping after the commit:** once the destination click lands, type a length and press `Enter` again — the move (or copy) you just made redoes at the new distance along the same direction, positive keeping it and negative flipping it — as many times as you like, until you click a new base point, press `Esc`, or switch tools. One `Undo` afterward removes the whole move, no matter how many times you retyped it.

**Array copy:** right after a copy commits, type a multiplier and press `Enter`:

- `3x` (or `x3`, or `*3` — either order works) makes **3 copies total** at that same spacing, continuing along the same line — copy something 2 m over, type `5x`, and five copies march off at 2 m intervals.
- `3/` (or `/3`) makes **3 copies dividing the distance** — place the last fence post first, then `3/` fills the run with evenly spaced posts between.

![A cedar fence with six evenly spaced posts and two rails, one post shown mid-copy down the line](/docs/transform-array.webp)

The gesture stays live until you start something else, so if `5x` turns out wrong, type `8x` or `4/` instead and the array re-resolves. However many copies it made, the whole array is **one undo step**. Copies of a component are new instances of the same component; copies of plain objects and groups are independent duplicates. Arrays work for drawn shapes too: a Move or Rotate copy of a shape takes `3x` and `3/`, on the ground or on a face, and the whole array is still one undo step.

The retyped-distance window and the array window share one typed buffer: right after a copy commits, a plain length retypes the distance and `3x`/`3/` makes an array — either one applies to that same commit. Once you've typed an array, the gesture stays hot for another array spec (`8x` after `5x`, say), but the plain-distance retype closes; make a fresh copy to retype a distance again.

## Rotate (`Q`)

Rotate puts a **protractor** — a round dial — under your cursor. The dial lies in the plane you'll rotate in, and its color is the axis you'll spin around: blue on the ground (vertical Z), red for X, green for Y, or purple for an off-axis face. As you move over the model the dial tilts to whatever **face or edge** is under the cursor, so you can see the axis before you commit to it.

1. Hover until the dial shows the axis you want, then click the **pivot** (the center of rotation).
2. Click a **reference point** to define the zero direction.
3. Sweep to the new angle and click to commit. A dim arm marks 0° and a colored arm tracks the live angle.

**Locking the axis.** Hold `Shift` to lock the dial to the axis it's currently showing — it renders solid, with a short stub along the axis, so the lock is obvious. Or force a world axis outright: `→` X, `←` Y, `↑` Z; `↓` clears the lock and goes back to following faces. Locking with an arrow is how you rotate something that offers no face to aim at — tipping a **cylinder** onto its side, say: hover it, press `←` or `→` to lock a horizontal axis, then pick your two points. Once locked, your reference click and the live sweep are held to that plane, so you can snap exactly onto a point in it — a corner, an edge — instead of the nearest point in full 3D, which could read differently than the flat angle you're actually setting.

The live angle snaps to 15° increments as you sweep. For any other angle, type degrees (e.g. `22.5`, negative allowed) and press `Enter`.

Type degrees again right after the commit and Hew redoes the rotation about that same pivot and axis — positive keeps the direction you just rotated, negative flips it — repeatable until you click to start a new rotation, press `Esc`, or switch tools; one `Undo` removes the whole rotation. Rotate's copy toggle and its ×N/`÷N` array window work exactly like Move's (above), sharing the same typed buffer.

## Scale (`S`)

Scale draws a **grip gizmo** on the selection's bounding box: a grip at the center of each of the 6 faces, one at each of the 8 corners, and one at each of the 12 edge midpoints.

![The Scale tool active on a box, showing the full grip gizmo; the status bar explains what each grip does](/docs/scale-grips.webp)

1. **Drag a grip.**
   - A **face grip** stretches one axis only — height, width, or depth — leaving the other two untouched.
   - An **edge grip** drives the two axes it sits between, each following your drag independently — pull mostly along one axis and that axis stretches more. (A single typed value scales both proportionally instead.)
   - A **corner grip** scales all three axes together — uniform scale, like SketchUp's own corner handles.
2. **Click again** to commit. The live readout shows the factor (`×1.50`) tagged with the axis for a face grip, both axes' factors for an edge grip.

By default the grip **opposite** the one you grabbed stays anchored — grab the top face and the bottom stays put, so the object grows upward from there. Tap `Control` to durably anchor at the box's **center** instead, so both sides move symmetrically; tap it again to go back to the opposite-grip anchor. Dragging a grip past its anchor never flips the object inside out — it clamps at a tiny minimum size instead (mirroring is a separate tool).

**Exact values:** type a plain number (`0.5`, `2.54`) and press `Enter` for a factor on the driven axis or axes — a corner grip's one typed value drives all three. Type a length instead (`50mm`, `8"`) and Hew reads it as a **target dimension**: the driven axis scales to land at exactly that size, computed against the box's current extent.

**Retyping after the commit:** once a grip-drag click commits, type another factor or dimension and press `Enter` — the scale redoes about the same pivot and the same driven axis or axes, measuring a typed dimension against the box's size *before* that first scale, so `50mm` means exactly what it would have meant mid-drag. Repeat as many times as you like, until you drag a different grip, press `Esc`, or switch tools; one `Undo` removes the whole scale.

Scaling a component instance scales that instance alone — the shared definition and every sibling instance are untouched, even for a non-uniform stretch.

## What transforms apply to

Move, Rotate, and Scale act on the whole selection: an object, a group (with everything inside it), a component instance (each instance transforms independently), a free-standing sketch — or any multi-selection of these at once. Select All followed by Move relocates an entire model in one gesture, and the whole act is a single undo step. Move also takes a construction guide: with no objects selected, a click on a guide moves (or copies) that guide — see [Working with guides](/learn/measurement-and-guides/#working-with-guides). A multi-selection's Scale gizmo is built from the union of every selected object's bounding box, so grabbing a grip spreads the whole selection apart (or together) correctly around the chosen anchor, rather than resizing each object in place. A copy-move duplicates everything in the selection — solids, groups, components, and sketch shapes alike. A sketch's in-plane copies arrive as one undo step per sketch. A copy that leaves its sketch plane lands on a new sketch on the plane it moved to — a whole sketch's shapes travel together onto that one new sketch, so a shape with a hole keeps its hole, one undo step per sketch, the original left where it was. Object copies of one commit — array copies included — undo as a single step.

Drawn geometry transforms at shape granularity, on the ground and on a solid's face alike. Whether you selected a filled shape, one of its lines, or a drawn arc or circle, the transform moves the whole connected shape as a rigid body — an open chain of lines included. Rotation isn't confined to the sketch plane: tipping a drawn profile upright (the [Follow Me](/learn/follow-me/) setup) rotates the shape out of the ground with the same gesture as any other rotation. If the shape is the only thing in its sketch, the whole sketch tips with it; if it shares the working sketch with other drawing, the shape splits off into its own sketch on the new plane and everything else stays where it was. An in-plane move that lands a shape on top of another joins them exactly as drawing it there would: lines split and merge and the regions re-form, as one undo step, and the moved shape stays selected. A face is no different from the ground here: a shape moved or dragged half past a face's edge splits into a part on the face and a part beyond it, each ready for Push/Pull, and moving it back joins them again. A solid's own edges and corners are not part of the drawing, so none of these tools can move them.

## Copy and paste

Move+Alt (above) is the fastest way to duplicate something in place, but `⌘C`/`⌘V` work too, and they're the way to carry a selection somewhere the model doesn't reach on its own — a different open document, say.

- **Copy** (`⌘C` / `Ctrl+C`) copies the selected objects, groups, and components. It needs at least one of those in the selection; a sketch-only selection refuses with a reminder to use Move+Alt instead, which does carry sketch geometry.
- **Paste** (`⌘V` / `Ctrl+V`) arms a placement ghost that follows the cursor with the same inference snapping as Move — click to drop it, `Esc` to cancel. Paste again for another copy at a new spot.
- **Paste In Place** (`⇧⌘V` / `Ctrl+Shift+V`) drops the copy back exactly where it was cut or copied from, with no click.
- **Cut** (`⌘X` / `Ctrl+X`) copies, then deletes the selection — one undo step for the delete.

A copy of a component instance reuses the original's definition (pasting the same clipboard twice doesn't mint two definitions); a copy of a plain object or group stays a plain object or group.

## Deleting

`Delete` or `Backspace` removes the current selection with any tool active. The contextual dock's **Erase** button and **Edit ▸ Delete** do the same. Deleting is undoable, like everything else — `⌘Z` / `Ctrl+Z` undoes, `⇧⌘Z` / `Ctrl+Shift+Z` redoes, across the entire document history.

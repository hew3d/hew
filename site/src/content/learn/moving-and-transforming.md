---
title: "Move, Rotate, and Scale"
description: "Move, Rotate, or Scale with full snapping, axis locks, and typed values — or just drag an object with Select. Copying is a Move with Alt tapped — and ×N multiplies it into an array."
order: 8
---

Move, Rotate, and Scale all work on the current selection — but they don't demand one. With nothing selected, clicking an object with any of the three selects it and starts the gesture in the same click, so moving something is one fluid motion, not Select-then-Move. With a selection already made, the click places the first reference point as usual.

The fastest move doesn't even need the Move tool: with **Select** active, press on an object and **drag** it. Past a few pixels the drag becomes a real move — full snapping, axis locks, Alt-copy, typed distances — and releasing the button commits it. The tool stays Select throughout, like dragging an icon on your desktop. Dragging a member of a multi-selection drags the whole selection.

All three transform tools show a live ghost preview of the result and a live readout, and all three accept typed exact values mid-gesture.

## Select (`Space`)

A quick refresher:

- **Click** an object, sketch, or guide to select it. Clicking empty space clears the selection.
- **Shift-click** adds to or removes from the selection.
- **Drag an object** to move it (see above). `Esc` mid-drag cancels the move and puts everything back.
- **Drag from empty space** to rubber-band a selection, SketchUp-style: dragging left→right draws a solid rectangle and selects what falls **entirely inside** it; dragging right→left draws a dashed rectangle and selects everything the rectangle **touches**. Hold `Shift` to add the result to the current selection (a `Shift`-drag always rubber-bands, even over an object); `Esc` cancels a drag in progress.
- **Select All** (`⌘A` / `Ctrl+A`, or Edit ▸ Select All) selects every visible object, group, component, and free-standing sketch — the whole model. Inside a group's editing context it selects that group's contents instead.
- **Double-click** a group, component, or object to enter its editing context (the rest of the scene dims); press `Esc` to step back out.

## Move (`M`)

1. Click a **base point**. Pick a meaningful one, like a corner you want to land somewhere. (With nothing selected, this click also selects the object under the cursor.)
2. Click the **destination**. The base point lands exactly there, snapping to anything the inference engine finds.

**Exact distance:** after the base point, type a length (`1.5m`, `8"`) and press `Enter` — the selection moves exactly that far in the direction you were dragging (or along the locked axis).

**Axis locking:** hold `Shift` to lock to the dominant axis of your drag, or press `→` for X, `←` for Y, `↑` for Z (`↓` clears). The preview line takes the axis color.

**Copy instead of move:** tap `Option`/`Alt` — copy mode switches on and stays on, with the readout prefixed "Copy ·", a `+` badge on the cursor, and the status bar confirming it. Because it's a toggle rather than a held key, everything else works exactly as in a plain move: type an exact distance and press `Enter` to place a copy at a precise offset. The original stays put, the copy lands at the destination and becomes the new selection, so repeated moves chain copies. Sketch shapes copy too, keeping their curve identity — a copied circle is a true circle, center snap and all. Copy a shape within its plane and the duplicate is redrawn through the same sticky rules as hand drawing, so if it lands on other lines they split each other exactly as drawn lines would. Copy a shape *off* its plane — lifting a ground profile straight up, say — and the copy arrives on a new sketch on the plane it landed on, with the original untouched (handy for [Follow Me](/learn/follow-me/): copy a profile up instead of moving the only one you have). Tap `Alt` again to go back to moving.

**Array copy:** right after a copy commits, type a multiplier and press `Enter`:

- `3x` (or `x3`, or `*3` — either order works) makes **3 copies total** at that same spacing, continuing along the same line — copy something 2 m over, type `5x`, and five copies march off at 2 m intervals.
- `3/` (or `/3`) makes **3 copies dividing the distance** — place the last fence post first, then `3/` fills the run with evenly spaced posts between.

The gesture stays live until you start something else, so if `5x` turns out wrong, just type `8x` or `4/` and the array re-resolves. However many copies it made, the whole array is **one undo step**. Copies of a component are new instances of the same component; copies of plain objects and groups are independent duplicates. Arrays apply to solids, groups, and components — a sketch copy stays a single copy, so the ×N window doesn't open after one.

## Rotate (`Q`)

Rotate puts a **protractor** — a round dial — under your cursor. The dial lies in the plane you'll rotate in, and its color is the axis you'll spin around: blue on the ground (vertical Z), red for X, green for Y, or purple for an off-axis face. As you move over the model the dial tilts to whatever **face or edge** is under the cursor, so you can see the axis before you commit to it.

1. Hover until the dial shows the axis you want, then click the **pivot** (the center of rotation).
2. Click a **reference point** to define the zero direction.
3. Sweep to the new angle and click to commit. A dim arm marks 0° and a colored arm tracks the live angle.

**Locking the axis.** Hold `Shift` to lock the dial to the axis it's currently showing — it renders solid, with a short stub along the axis, so the lock is obvious. Or force a world axis outright: `→` X, `←` Y, `↑` Z; `↓` clears the lock and goes back to following faces. Locking with an arrow is how you rotate something that offers no face to aim at — tipping a **cylinder** onto its side, say: hover it, press `←` or `→` to lock a horizontal axis, then pick your two points.

The live angle snaps to 15° increments as you sweep. For any other angle, type degrees (e.g. `22.5`, negative allowed) and press `Enter`.

## Scale (`S`)

Scale draws a **grip gizmo** on the selection's bounding box: a grip at the center of each of the 6 faces, one at each of the 8 corners, and one at each of the 12 edge midpoints.

1. **Drag a grip.**
   - A **face grip** stretches one axis only — height, width, or depth — leaving the other two untouched.
   - An **edge grip** drives the two axes it sits between, each following your drag independently — pull mostly along one axis and that axis stretches more. (A single typed value scales both proportionally instead.)
   - A **corner grip** scales all three axes together — uniform scale, like SketchUp's own corner handles.
2. **Click again** to commit. The live readout shows the factor (`×1.50`) tagged with the axis for a face grip, both axes' factors for an edge grip.

By default the grip **opposite** the one you grabbed stays anchored — grab the top face and the bottom stays put, so the object grows upward from there. Tap `Control` to durably anchor at the box's **center** instead, so both sides move symmetrically; tap it again to go back to the opposite-grip anchor. Dragging a grip past its anchor never flips the object inside out — it clamps at a tiny minimum size instead (mirroring is a separate tool).

**Exact values:** type a plain number (`0.5`, `2.54`) and press `Enter` for a factor on the driven axis or axes — a corner grip's one typed value drives all three. Type a length instead (`50mm`, `8"`) and Hew reads it as a **target dimension**: the driven axis scales to land at exactly that size, computed against the box's current extent.

Scaling a component instance scales that instance alone — the shared definition and every sibling instance are untouched, even for a non-uniform stretch.

## What transforms apply to

Move, Rotate, and Scale act on the whole selection: an object, a group (with everything inside it), a component instance (each instance transforms independently), a free-standing sketch — or any multi-selection of these at once. Select All followed by Move relocates an entire model in one gesture, and the whole act is a single undo step. A multi-selection's Scale gizmo is built from the union of every selected object's bounding box, so grabbing a grip spreads the whole selection apart (or together) correctly around the chosen anchor, rather than resizing each object in place. A copy-move duplicates everything in the selection — solids, groups, components, and sketch shapes alike; a sketch's in-plane copies arrive as one undo step per sketch, and a copy that leaves its sketch plane lands on a new sketch on the plane it moved to — a whole sketch's shapes travel together onto that one new sketch (so a shape with a hole keeps its hole), one undo step per sketch, the original left where it was. Object copies of one commit — array copies included — undo as a single step.

Sketch geometry transforms at shape granularity. Whether you selected a filled shape, one of its lines, or a drawn arc or circle, the transform moves the whole connected shape as a rigid body — an open chain of lines included. Rotation isn't confined to the sketch plane: tipping a drawn profile upright (the [Follow Me](/learn/follow-me/) setup) rotates the shape out of the ground with the same gesture as any other rotation. If the shape is the only thing in its sketch, the whole sketch tips with it; if it shares the working sketch with other drawing, the shape splits off into its own sketch on the new plane and everything else stays where it was. An in-plane move that would land one shape on top of another is still refused rather than welded.

## Deleting

`Delete` or `Backspace` removes the current selection with any tool active. The contextual dock's **Erase** button and **Edit ▸ Delete** do the same. Deleting is undoable, like everything else — `⌘Z` / `Ctrl+Z` undoes, `⇧⌘Z` / `Ctrl+Shift+Z` redoes, across the entire document history.

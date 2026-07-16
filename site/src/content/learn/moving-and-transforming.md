---
title: "Move, Rotate, and Scale"
description: "Select first, then Move, Rotate, or Scale with full snapping, axis locks, and typed values. Copying is a Move with Alt held."
order: 7
---

Move, Rotate, and Scale all follow the same pattern: **select something first** (with the Select tool, `Space`), then activate the tool and place two or three reference points. If nothing is selected, the tool reminds you in a toast.

All three show a live ghost preview of the result and a live readout, and all three accept typed exact values mid-gesture.

## Select (`Space`)

A quick refresher, since every transform starts here:

- **Click** an object, sketch, or guide to select it. Clicking empty space clears the selection.
- **Shift-click** adds to or removes from the selection.
- **Drag** to rubber-band a selection, SketchUp-style: dragging left→right draws a solid rectangle and selects what falls **entirely inside** it; dragging right→left draws a dashed rectangle and selects everything the rectangle **touches**. Hold `Shift` to add the result to the current selection; `Esc` cancels a drag in progress.
- **Select All** (`⌘A` / `Ctrl+A`, or Edit ▸ Select All) selects every visible object, group, component, and free-standing sketch — the whole model. Inside a group's editing context it selects that group's contents instead.
- **Double-click** a group, component, or object to enter its editing context (the rest of the scene dims); press `Esc` to step back out.

## Move (`M`)

1. Click a **base point**. Pick a meaningful one, like a corner you want to land somewhere.
2. Click the **destination**. The base point lands exactly there, snapping to anything the inference engine finds.

**Exact distance:** after the base point, type a length (`1.5m`, `8"`) and press `Enter` — the selection moves exactly that far in the direction you were dragging (or along the locked axis).

**Axis locking:** hold `Shift` to lock to the dominant axis of your drag, or press `→` for X, `←` for Y, `↑` for Z (`↓` clears). The preview line takes the axis color.

**Copy instead of move:** hold `Option`/`Alt` while committing — the original stays put and a copy lands at the destination, with the readout prefixed "Copy ·". The copy becomes the new selection, so repeated Alt-moves chain copies one after another. This works on whatever you have selected: an object copies its geometry, a **group** copies its entire contents — nested groups and all, names, tags, and materials included — and a component instance copies as another instance of the same definition (the copies still update together; use Make Unique to break that). A copied group is fully independent of the original, and one undo removes the whole copy.

## Rotate (`Q`)

Rotate puts a **protractor** — a round dial — under your cursor. The dial lies in the plane you'll rotate in, and its color is the axis you'll spin around: blue on the ground (vertical Z), red for X, green for Y, or purple for an off-axis face. As you move over the model the dial tilts to whatever **face or edge** is under the cursor, so you can see the axis before you commit to it.

1. Hover until the dial shows the axis you want, then click the **pivot** (the center of rotation).
2. Click a **reference point** to define the zero direction.
3. Sweep to the new angle and click to commit. A dim arm marks 0° and a colored arm tracks the live angle.

**Locking the axis.** Hold `Shift` to lock the dial to the axis it's currently showing — it renders solid, with a short stub along the axis, so the lock is obvious. Or force a world axis outright: `→` X, `←` Y, `↑` Z; `↓` clears the lock and goes back to following faces. Locking with an arrow is how you rotate something that offers no face to aim at — tipping a **cylinder** onto its side, say: hover it, press `←` or `→` to lock a horizontal axis, then pick your two points.

The live angle snaps to 15° increments as you sweep. For any other angle, type degrees (e.g. `22.5`, negative allowed) and press `Enter`.

## Scale (`S`)

1. Click a **base point**.
2. Move away from or toward the selection's center and click to commit. The readout shows the factor (`×1.50`).

Scaling is **uniform**, about the selection's bounding-box center. For an exact factor, type it (`0.5`, `2.54`) and press `Enter`; factors must be positive. Non-uniform (per-axis) scaling is on the roadmap.

## What transforms apply to

Move, Rotate, and Scale act on the whole selection: an object, a group (with everything inside it), a component instance (each instance transforms independently), a free-standing sketch — or any multi-selection of these at once. Select All followed by Move relocates an entire model in one gesture, and the whole act is a single undo step. Multi-selections scale about the selection's overall bounding-box center. The one multi-selection caveat: an `Option`/`Alt` copy-move duplicates each solid but plain-moves any sketches in the selection (sketches have no copy support yet), and undoing a multi-copy takes one undo per copied node.

## Deleting

`Delete` or `Backspace` removes the current selection with any tool active. The contextual dock's **Erase** button and **Edit ▸ Delete** do the same. Deleting is undoable, like everything else — `⌘Z` / `Ctrl+Z` undoes, `⇧⌘Z` / `Ctrl+Shift+Z` redoes, across the entire document history.

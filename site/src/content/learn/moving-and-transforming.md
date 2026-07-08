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
- **Double-click** a group, component, or object to enter its editing context (the rest of the scene dims); press `Esc` to step back out.

There is no rubber-band/marquee selection yet; use Shift-click or the Outliner for multiple selections.

## Move (`M`)

1. Click a **base point**. Pick a meaningful one, like a corner you want to land somewhere.
2. Click the **destination**. The base point lands exactly there, snapping to anything the inference engine finds.

**Exact distance:** after the base point, type a length (`1.5m`, `8"`) and press `Enter` — the selection moves exactly that far in the direction you were dragging (or along the locked axis).

**Axis locking:** hold `Shift` to lock to the dominant axis of your drag, or press `→` for X, `←` for Y, `↑` for Z (`↓` clears). The preview line takes the axis color.

**Copy instead of move:** hold `Option`/`Alt` while committing — the original stays put and a copy lands at the destination, with the readout prefixed "Copy ·". The copy becomes the new selection, so repeated Alt-moves chain copies one after another.

## Rotate (`Q`)

1. Click the **pivot**. The rotation axis is the normal of the face you clicked (rotating on the ground spins around vertical Z).
2. Click a **reference point** to define the zero direction.
3. Sweep to the new angle and click to commit.

The live angle snaps to 15° increments as you sweep. For any other angle, type degrees (e.g. `22.5`, negative allowed) and press `Enter`. Arrow keys force the rotation axis to a world axis: `→` X, `←` Y, `↑` Z; `↓` returns to the face-derived axis.

## Scale (`S`)

1. Click a **base point**.
2. Move away from or toward the selection's center and click to commit. The readout shows the factor (`×1.50`).

Scaling is **uniform**, about the selection's bounding-box center. For an exact factor, type it (`0.5`, `2.54`) and press `Enter`; factors must be positive. Non-uniform (per-axis) scaling is on the roadmap.

## What transforms apply to

Move, Rotate, and Scale act on one selected thing at a time: an object, a group (with everything inside it), a component instance (each instance transforms independently), or a free-standing sketch. To move several things as one, group them first ([Groups and components](/learn/groups-and-components/)); transforming a whole multi-selection in one gesture is planned but not available yet.

## Deleting

`Delete` or `Backspace` removes the current selection with any tool active. The contextual dock's **Erase** button and **Edit ▸ Delete** do the same. Deleting is undoable, like everything else — `⌘Z` / `Ctrl+Z` undoes, `⇧⌘Z` / `Ctrl+Shift+Z` redoes, across the entire document history.

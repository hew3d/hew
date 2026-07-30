---
title: "Dimensions and text"
description: "Annotate a model with dimensions that follow the geometry they measure, radius and diameter callouts, and leader text."
order: 9.5
---

Dimensions and leader text annotate a model without becoming part of it. They're document entities like guides: they don't affect watertightness, they don't export as geometry, and they follow the shapes they're attached to when those shapes move.

## Linear dimensions

1. Activate **Dimension**.
2. Click the two points you want to measure between — endpoints, midpoints, and every other snap works here.
3. Move away from the line and click again to place the dimension where you want it.

That third click matters more than it looks. It sets the plane the dimension is drawn in, and Hew picks that plane from where you drag: the sketch plane both points share if they share one, otherwise the axis-aligned plane through the measured line that best matches the drag — pull up from a horizontal edge and the dimension stands in the vertical plane above it, pull sideways and it lies down flat. The plane is anchored to the model, not the camera, so a dimension placed from one view reads the same from every other. A dimension drawn above the ground stays above the ground.

The arrow keys lock the plane instead of letting the drag choose, the same way they lock a drawing plane in the shape tools: `→` red, `←` green, `↑` blue, and the same arrow again (or `↓`) releases the lock. From a viewpoint square-on to the measured line, two candidate planes can overlap on screen and the drag can't tell them apart — Hew keeps the one facing you, and an arrow lock is how you pick the other.

Type a value before the final click and the dimension takes that text instead of the measured length; press `Tab` to go back to the real measurement.

## Radius and diameter

Circles and arcs get their own callouts, and which one you get depends on what you click:

- **Click the curve, then move off it** and click in space — you get a radius on an arc, a diameter on a full circle. Press `Tab` to switch between them.
- **Click the curve, then its centre** (or the centre first, then the curve) — a radius, drawn along the actual radius from the centre out to the rim.
- **Click two points across the curve, through the centre** — a diameter, drawn rim to rim.
- **Click two arbitrary points on the curve** — an ordinary linear dimension of that chord, because that's what you measured. It isn't a radius and won't claim to be one.

The drawn line always shows what's being measured: a radius runs from the centre mark to the rim, a diameter spans the full width through the centre.

## Leader text

**Text** places a note with a leader line pointing at whatever you clicked. Click the target, drag out to where the text should sit, click again, and type. The text sits where you put it in space rather than dropping to the ground.

## How annotations behave

**They're hidden by geometry.** A dimension behind a wall is behind the wall — annotations are ordinary depth-tested ink, not an always-on-top overlay. Orbit around and they come and go the way the model does.

**The line breaks around its own label**, so the text is never crossed out by the dimension it belongs to. If a dimension is too short for its text, the text moves outside the extension lines instead.

**Labels hold their size on screen** as you zoom, and turn to stay readable rather than reading backwards from the far side.

**They follow the geometry.** Move, rotate, or scale something a dimension is attached to and the dimension goes with it. If an edit destroys what a dimension was measuring — deleting the object, or consuming it in a boolean — the annotation is marked detached and drawn in a warning colour rather than quietly pointing at nothing. Re-pick its anchors to reattach it.

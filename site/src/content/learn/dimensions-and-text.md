---
title: "Dimensions and text"
description: "Annotate a model with dimensions that follow the geometry they measure, radius and diameter callouts, and leader text."
order: 11
---

Dimensions and leader text annotate a model without becoming part of it. They're document entities like guides: they don't affect watertightness, they don't export as geometry, and they follow the shapes they're attached to when those shapes move.

<docs-video theme="dark" slug="dimensions-and-text" label="dimensions and text demonstration"></docs-video>

![A box with two linear dimensions reading 1.8 m and 0.9 m, next to a cylinder with an R0.5 m radial dimension](/docs/dimensions.webp)

## Linear dimensions

1. Activate **Dimension**.
2. Click the two points you want to measure between — endpoints, midpoints, and every other snap works here.
3. Move away from the line and click again to place the dimension where you want it.

The two points also take a drag: press on the first, drag, and let go on the second, then click to place the dimension.

To dimension a whole line, click the line itself rather than a point on it: both of its ends are taken at once, and you go straight to placing the dimension. Press on the line, drag out, and let go to do it in one motion. An endpoint or a midpoint still starts a point-to-point dimension. A line lying along a guide or an axis snaps to the guide or axis instead, so dimension it by its two endpoints.

The plane is already decided by the time you place the second point, not just the third. Hew reads the first click — the face it landed on, or, lacking that, whichever axis-aligned plane most squarely faces the way you're looking — and holds the second point to it. A real snap elsewhere in space (an endpoint, a midpoint, a curve's center) lands exactly where it is as long as you could actually see it move there — sighting straight down on a framed wall, a stud top a few centimeters under the top plates reads as the same point on screen, so it's carried onto the wall's own plane instead of a measurement quietly picking up the wrong one. Anything looser gets carried onto that same plane too. Measuring a wall's top edge from straight above used to be able to slide the second point down onto the ground underneath it — now it stays at the wall's own height.

That third click still matters. It sets the plane the dimension is finally drawn in, and Hew picks that plane from where you drag: the sketch plane both points share if they share one, otherwise the axis-aligned plane through the measured line that best matches the drag — pull up from a horizontal edge and the dimension stands in the vertical plane above it, pull sideways and it lies down flat. The plane is anchored to the model, not the camera, so a dimension placed from one view reads the same from every other. A dimension drawn above the ground stays above the ground.

The arrow keys lock the plane instead of letting the drag choose, the same way they lock a drawing plane in the shape tools: `→` red, `←` green, `↑` blue, and the same arrow again (or `↓`) releases the lock. Press one before the first click, or between the two points, and it pins the plane right away instead of waiting for the drag. From a viewpoint square-on to the measured line, two candidate planes can overlap on screen and the drag can't tell them apart — Hew keeps the one facing you, and an arrow lock is how you pick the other. While a lock is on, the line you're dragging out is drawn in that axis's own color, so the lock is visible while you're still placing the dimension, not just after.

Dragging the third click near an already-placed dimension's own line — parallel to the one you're placing, on the same plane — snaps the new one to sit exactly on it, so a row of dimensions along the same wall lines up without you having to eyeball it. Move away and the snap lets go.

Type a value before the final click and the dimension takes that text instead of the measured length; press `Tab` to go back to the real measurement.

## Radius and diameter

Circles and arcs get their own callouts, and which one you get depends on what you click:

- **Click the curve, then move off it** and click in space — you get a radius on an arc, a diameter on a full circle. Press `Tab` to switch between them.
- **Click the curve, then its center** (or the center first, then the curve) — a radius, drawn along the actual radius from the center out to the rim.
- **Click two points across the curve, through the center** — a diameter, drawn rim to rim.
- **Click two arbitrary points on the curve** — an ordinary linear dimension of that chord, because that's what you measured. It isn't a radius and won't claim to be one.

Circles, arcs, and polygons drawn on a solid's face are measured and snapped the same way as those on the ground, including a polygon's center snap.

The drawn line always shows what's being measured: a radius runs from the center mark to the rim, a diameter spans the full width through the center.

## Leader text

**Text** places a note with a leader line pointing at whatever you clicked. Click the target, drag out to where the text should sit, click again, and type. The text sits where you put it in space rather than dropping to the ground.

## How annotations behave

**They're hidden by geometry.** A dimension behind a wall is behind the wall — annotations are ordinary depth-tested ink, not an always-on-top overlay. Orbit around and they come and go the way the model does.

**The line breaks around its own label**, so the text is never crossed out by the dimension it belongs to. If a dimension is too short for its text, the text moves outside the extension lines instead.

**Labels hold their size on screen** as you zoom, and turn to stay readable rather than reading backwards from the far side.

**They follow the geometry.** Move, rotate, or scale something a dimension is attached to and the dimension goes with it. If an edit destroys what a dimension was measuring — deleting the object, or consuming it in a boolean — the annotation is marked detached and drawn in a warning color rather than quietly pointing at nothing. Re-pick its anchors to reattach it.

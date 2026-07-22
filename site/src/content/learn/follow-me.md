---
title: "Follow Me"
description: "Sweep a profile along a path — pipes, moldings, frames, and lathe-like shapes from one profile and one line to follow."
order: 7
---

Follow Me sweeps a closed profile along a path and answers with a new watertight solid. It's the tool for everything push/pull can't reach in one straight stroke: a pipe that turns corners, a molding run around a tabletop, a picture frame, a rounded or chamfered edge strip.

## The gesture

1. Select the path with the Select tool — a drawn line, curve, or any connected run of sketch edges. One click on one line is enough: Follow Me takes the whole connected run it belongs to. Once picked, the path highlights boldly along its whole length, so it's obvious what you're about to sweep.
2. Activate Follow Me (tool rail, or search "sweep" in the palette).
3. Click the profile — a closed sketch region sitting square across the path's end. A plain click sweeps the whole path; press and drag along the path instead for a partial sweep (see [A partial sweep](#a-partial-sweep) below).

The sweep commits immediately: the profile is carried along the path, turning each corner with a clean miter, and the result is an ordinary solid — watertight badge, push/pull on its flat ends, booleans, export, all as usual.

You can also skip the preselection: activate Follow Me first, then click the path (clicking one line picks up the whole connected shape it belongs to), then click the profile. `Esc` steps back a stage.

Whatever is selected when you activate the tool becomes the path — including a selection left over from placing the profile. If the wrong thing is highlighted, click the face you meant to follow (a face click at the profile step re-targets the path) or press `Esc` and pick the path again.

## A partial sweep

Press on the profile and drag along the highlighted path instead of just clicking, and Follow Me stops the sweep partway through the run. A marker rides along the path as you drag, with a live length readout tracking how far you've gone; release to build the sweep, capped with a flat end where you stopped.

A plain click — no dragging — still sweeps the whole path, exactly as before. For a precise length without eyeballing it, type the number and press `Enter` instead of dragging; that works whether or not you've started dragging yet. `Esc` cancels the drag and drops back to the profile stage — the path stays picked, ready to try again.

## Setting up the profile

The profile must sit **perpendicular to the path where the sweep starts** — square across it, like a slice of the result. Hew doesn't rotate the profile into place for you; the solid lands exactly where the profile is.

Hover a profile before you click and Hew tells you up front whether that placement will work: an outline and a colored marker appear on the profile, and the status bar spells out the verdict — ready to sweep, refused (with the reason and what to change), or too close to call. You get that reading before you commit, not a refusal after the fact.

The usual setup: draw the profile flat on the ground, select it, and stand it upright with [Rotate](/learn/moving-and-transforming/), placing it across the path's first segment. If the profile shares the working sketch with other drawing — the path, say — standing it upright splits it into its own sketch and leaves the rest where it was. The path itself must start on the profile's plane (snapping makes this exact — start the path's first line on the standing profile, or Move the standing profile onto the path's first point).

The profile doesn't have to land exactly on an open path's end, either. Turn it the right way — square to the path — and set it down near the end but not quite touching, and Hew carries the path's shape over to wherever the profile actually stands and sweeps from there. So a profile that's a hair off the end still works; the sweep just starts where the profile is, not where the path's own end vertex sits.

The profile doesn't have to be centered on the path — a molding offset from its spine is fine. Profiles with holes work too: the hole tunnels the whole length of the sweep.

## Closed paths

A path that closes into a loop — a circle, a rectangle, any drawn ring — sweeps into a closed ring solid with no end caps: a frame, a gasket, a torus. Sweep a profile around a drawn circle and you have a lathe: bottle rims, wheels, round handles.

For a loop of straight lines — a rectangle, any drawn ring of lines — the profile can start in the *middle* of a run, or right at a **corner**. At a corner it seams into a clean miter, as long as it's stood past the corner rather than hanging back over the edge leading into it — picture-frame moldings, mitered corners included, are buildable starting right at the corner this way. A profile that does hang back over that incoming edge folds into its own material on the first stretch of the sweep, and Hew refuses; slide it fully past the corner and try again.

A drawn circle is measured as the true curve it is: stand the profile square across the rim, its face looking along the circle — in line with the circle's center. Let the snaps make that exact: draw the circle from its center out along a drawing axis, stand the profile upright with the rotation locked to that axis, and set it on the rim. Anywhere on the rim works, including the drawn points of the circle itself.

## Spheres and other poles

Let the profile reach the circle's axis — the center line the lathe turns about — and the sweep closes a **pole** instead of leaving a hole down the middle: a sphere, a goblet, a cone, each a single watertight solid.

The sphere is the headline case. Draw a circle for the path. Draw a second circle the same size standing upright *through* the axis — its own center on the path circle's center — so it crosses the axis at a top and a bottom point. Follow Me the upright circle around the path: Hew revolves one half of it and closes both crossings into poles, giving you a clean sphere. (Draw the path circle from its center, then draw the profile circle from that same center with the rotation locked upright, and the two points snap exact.)

A profile that only touches the axis — a goblet's outline resting on the center line, a cone's slant running up to a tip — closes the same way, a pole wherever the outline meets the axis.

Poles need a **drawn circle** path. On a straight-line loop, or any ring Hew doesn't read as a true circle, a profile reaching the center has no clean revolution to close on and refuses — keep it clear of the axis there.

## Running around a face

Instead of a drawn path, run the sweep around a **face of a solid**: click the flat face whose rim you want the molding to follow — a tabletop's top for crown molding around its edge, a lid's top for a lip. The sweep runs around *that* face's outer boundary, and the result is a separate new solid; the original is untouched. To make it one piece, union them; to carve it away (a chamfer or a groove), subtract it ([Combining solids](/learn/combining-solids/)).

Click the face itself, squarely. Unlike a drawn path, a solid's face can't be preselected — selecting the whole object gives Follow Me no path — so with the tool active you point at the face and click it directly. Hover it first: the rim that will be swept lights up before you click, so you can see which face you're about to pick. Aim at a clear part of the flat face, not *through* the standing profile — the profile sits in front of whatever's behind it, and a click that reaches past it picks the face beyond. If a click lands on nothing followable, the tool says so ("Click the flat face to run the profile around it") rather than sitting silent.

The profile stands square across that face's boundary — its plane cutting one of the face's edges partway along the run, not at a corner and not flat along the edge. Stand it up beside the solid and Move it onto the rim; the midpoint and edge snaps land it exactly, and it can hang off the edge for a molding that sits proud. Because the profile must be perpendicular to the rim it wraps, a face parallel to the profile is refused ("that face is parallel to the profile"), and so is one thinner than the profile is deep ("that face is thinner than the profile is deep") — Hew names the wrong face instead of quietly rotating the profile to fit it. Pick the face the profile stands across.

The face has to belong to a **plain object** — not a component instance or a grouped object, whose faces sit in their own frame. Follow Me says so if you click one; explode the instance or ungroup it first, then follow the face.

## What Follow Me refuses

Follow Me never commits a broken solid. It refuses, with the model untouched, when:

- the profile isn't perpendicular to the path anywhere near a legal start (an open path's end, or — on a closed path — the middle of a run or a corner stood past);
- on a closed path, the profile is perpendicular but doesn't actually touch the path anywhere;
- the path branches, or the selection is in disconnected pieces;
- the path doubles back on itself — or turns nearly all the way around — at a corner: there is no clean way to miter a hairpin;
- the path bends tighter than the profile is wide — the sweep would fold into itself;
- the swept shape would run into itself further along the path;
- a profile reaches the lathe axis on a path that *isn't* a drawn circle — only a true circle revolves cleanly into a pole (see [Spheres and other poles](#spheres-and-other-poles)); elsewhere, keep the profile clear of the center.

Each refusal says what to change. Adjust the profile or path and click again — the picked path stays selected.

## Curves stay curves

A drawn circle swept along a straight run is a true cylinder, exactly as if you'd extruded it: it shades smooth, remembers its exact radius, and exports at any resolution — even when the run was drawn in several strokes. Around a path's turns the walls keep their honest facets and export at the resolution they were drawn.

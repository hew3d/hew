---
title: "Follow Me"
description: "Sweep a profile along a path — pipes, moldings, frames, and lathe-like shapes from one profile and one line to follow."
order: 7
---

Follow Me sweeps a closed profile along a path and answers with a new watertight solid. It's the tool for everything push/pull can't reach in one straight stroke: a pipe that turns corners, a molding run around a tabletop, a picture frame, a rounded or chamfered edge strip.

## The gesture

1. Select the path with the Select tool — a drawn line, curve, or any connected run of sketch edges. One click on one line is enough: Follow Me takes the whole connected run it belongs to.
2. Activate Follow Me (tool rail, or search "sweep" in the palette).
3. Click the profile — a closed sketch region sitting square across the path's end.

The sweep commits immediately: the profile is carried along the path, turning each corner with a clean miter, and the result is an ordinary solid — watertight badge, push/pull on its flat ends, booleans, export, all as usual.

You can also skip the preselection: activate Follow Me first, then click the path (clicking one line picks up the whole connected shape it belongs to), then click the profile. `Esc` steps back a stage.

Whatever is selected when you activate the tool becomes the path — including a selection left over from placing the profile. If the wrong thing is highlighted, click the face you meant to follow (a face click at the profile step re-targets the path) or press `Esc` and pick the path again.

## Setting up the profile

The profile must sit **perpendicular to the path where the sweep starts** — square across it, like a slice of the result. Hew doesn't rotate the profile into place for you; the solid lands exactly where the profile is.

The usual setup: draw the profile flat on the ground, select it, and stand it upright with [Rotate](/learn/moving-and-transforming/), placing it across the path's first segment. If the profile shares the working sketch with other drawing — the path, say — standing it upright splits it into its own sketch and leaves the rest where it was. The path itself must start on the profile's plane (snapping makes this exact — start the path's first line on the standing profile, or Move the standing profile onto the path's first point).

The profile doesn't have to be centered on the path — a molding offset from its spine is fine. Profiles with holes work too: the hole tunnels the whole length of the sweep.

## Closed paths

A path that closes into a loop — a circle, a rectangle, any drawn ring — sweeps into a closed ring solid with no end caps: a frame, a gasket, a torus. Sweep a profile around a drawn circle and you have a lathe: bottle rims, wheels, round handles.

For a loop of straight lines — a rectangle, any drawn ring of lines — place the profile across the *middle* of one of the runs, not at a corner: a profile at a corner has no clean seam to close on, and Hew refuses rather than guess.

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

- the profile isn't perpendicular to the path, or the path doesn't start on the profile's plane;
- the path branches, or the selection is in disconnected pieces;
- the path doubles back on itself — or turns nearly all the way around — at a corner: there is no clean way to miter a hairpin;
- the path bends tighter than the profile is wide — the sweep would fold into itself;
- the swept shape would run into itself further along the path;
- a profile reaches the lathe axis on a path that *isn't* a drawn circle — only a true circle revolves cleanly into a pole (see [Spheres and other poles](#spheres-and-other-poles)); elsewhere, keep the profile clear of the center.

Each refusal says what to change. Adjust the profile or path and click again — the picked path stays selected.

## Curves stay curves

A drawn circle swept along a straight run is a true cylinder, exactly as if you'd extruded it: it shades smooth, remembers its exact radius, and exports at any resolution — even when the run was drawn in several strokes. Around a path's turns the walls keep their honest facets and export at the resolution they were drawn.

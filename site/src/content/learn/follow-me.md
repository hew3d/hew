---
title: "Follow Me"
description: "Sweep a profile along a path — pipes, moldings, frames, and lathe-like shapes from one profile and one line to follow."
order: 7
---

Follow Me sweeps a closed profile along a path and answers with a new watertight solid. It's the tool for everything push/pull can't reach in one straight stroke: a pipe that turns corners, a molding run around a tabletop, a picture frame, a rounded or chamfered edge strip.

## The gesture

1. Select the path with the Select tool — a drawn line, curve, or any connected run of sketch edges.
2. Activate Follow Me (tool rail, or search "sweep" in the palette).
3. Click the profile — a closed sketch region sitting square across the path's end.

The sweep commits immediately: the profile is carried along the path, turning each corner with a clean miter, and the result is an ordinary solid — watertight badge, push/pull on its flat ends, booleans, export, all as usual.

You can also skip the preselection: activate Follow Me first, then click the path (clicking one line picks up the whole connected shape it belongs to), then click the profile. `Esc` steps back a stage.

## Setting up the profile

The profile must sit **perpendicular to the path where the sweep starts** — square across it, like a slice of the result. Hew doesn't rotate the profile into place for you; the solid lands exactly where the profile is.

The usual setup: draw the profile flat on the ground, select it, and stand it upright with [Rotate](/learn/moving-and-transforming/), placing it across the path's first segment. If the profile shares the working sketch with other drawing — the path, say — standing it upright splits it into its own sketch and leaves the rest where it was. The path itself must start on the profile's plane (snapping makes this exact — start the path's first line on the standing profile, or Move the standing profile onto the path's first point).

The profile doesn't have to be centered on the path — a molding offset from its spine is fine. Profiles with holes work too: the hole tunnels the whole length of the sweep.

## Closed paths

A path that closes into a loop — a circle, a rectangle, any drawn ring — sweeps into a closed ring solid with no end caps: a frame, a gasket, a faceted torus. Sweep a profile around a drawn circle and you have a lathe: bottle rims, wheels, round handles.

For a closed loop, place the profile across the *middle* of one of the loop's straight runs, not at a corner — a profile at a corner has no clean seam to close on, and Hew refuses rather than guess.

## Running around a face

Instead of a drawn path, you can click a **face of a solid**: the sweep runs around that face's outer boundary — crown molding around a tabletop's edge, a lip around a lid. The result is a separate new solid; the original object is untouched. To make it one piece, union them; to carve it away instead (a chamfer or a groove), subtract it ([Combining solids](/learn/combining-solids/)).

## What Follow Me refuses

Follow Me never commits a broken solid. It refuses, with the model untouched, when:

- the profile isn't perpendicular to the path, or the path doesn't start on the profile's plane;
- the path branches, or the selection is in disconnected pieces;
- the path doubles back on itself — or turns nearly all the way around — at a corner: there is no clean way to miter a hairpin;
- the path bends tighter than the profile is wide — the sweep would fold into itself;
- the swept shape would run into itself further along the path;
- a lathe profile touches its own axis of revolution — keep it clear of the center.

Each refusal says what to change. Adjust the profile or path and click again — the picked path stays selected.

## Curves stay curves

A drawn circle swept along a straight run is a true cylinder, exactly as if you'd extruded it: it shades smooth, remembers its exact radius, and exports at any resolution — even when the run was drawn in several strokes. Around a path's turns the walls keep their honest facets and export at the resolution they were drawn.

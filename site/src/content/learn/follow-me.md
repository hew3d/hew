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

The profile should sit **square across the path where the sweep starts** — a slice of the result, standing perpendicular to the direction the sweep travels. That's still the placement to aim for, but you no longer have to get it exact by hand first: draw the profile lying flat, right next to the path, and click it — Hew stands it upright for you automatically before sweeping, hinged wherever it actually touches the path. This is the classic first attempt (draw a circle for the path, draw the profile flat beside it) working on the first try, not a special case.

Hover a profile before you click and Hew tells you up front what will happen: an outline and a colored marker appear on the profile, and the status bar spells out the verdict — ready to sweep as drawn, will be stood upright automatically, refused (with the reason and what to change), or too close to call. You get that reading before you commit, not a refusal after the fact.

Standing the profile up by hand still works exactly as before, if you'd rather place it precisely yourself: draw it flat on the ground, select it, and stand it upright with [Rotate](/learn/moving-and-transforming/), placing it across the path's first segment. If the profile shares the working sketch with other drawing — the path, say — standing it upright splits it into its own sketch and leaves the rest where it was.

The profile doesn't have to land exactly on an open path's end, either. Set it down near the end — square to the path, or left flat and let auto-orientation square it up — but not quite touching, and Hew carries the path's shape over to wherever the profile actually stands and sweeps from there. So a profile that's a hair off the end still works; the sweep just starts where the profile is, not where the path's own end vertex sits.

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

Instead of a drawn path, run the sweep around a **face of a solid**: click the flat face whose rim you want the molding to follow — a tabletop's top for crown molding around its edge, a lid's top for a lip. The sweep runs around *that* face's outer boundary. By default the result is a separate new solid and the original is untouched — union them to make it one piece, or subtract it to carve it away (a chamfer or a groove); see [Combining solids](/learn/combining-solids/) and the merge gesture below, which does the subtract-or-union for you in one step.

Click the face itself, squarely. Unlike a drawn path, a solid's face can't be preselected — selecting the whole object gives Follow Me no path — so with the tool active you point at the face and click it directly. Hover it first: the rim that will be swept lights up before you click, so you can see which face you're about to pick. Aim at a clear part of the flat face, not *through* the standing profile — the profile sits in front of whatever's behind it, and a click that reaches past it picks the face beyond. If a click lands on nothing followable, the tool says so ("Click the flat face to run the profile around it") rather than sitting silent.

Stand the profile up beside the solid and Move it onto the rim; the midpoint and edge snaps land it exactly, and it can hang off the edge for a molding that sits proud. It doesn't have to be exact — as with a drawn path, a profile that isn't square to the face's rim yet is folded upright automatically before the sweep runs. What still refuses is a face too narrow to hold the folded profile ("that face is thinner than the profile is deep") — Hew names the wrong face instead of quietly shrinking the profile to fit it. Pick a wider face, or use a shallower profile.

A face reached through a component instance works as a path too — molding around one placement of a repeated part doesn't need exploding it first. A face that belongs to a grouped object needs you to open the group first (double-click into it), the same as any other edit inside a group; a face reached while editing a component's own definition isn't a valid path target yet — step back out first.

## A solid face as the profile

The profile doesn't have to be a sketch region — a **face of a solid** works as the profile too, holes and all (a hole in the profile tunnels the length of the sweep, same as a sketch region's hole). At the profile step, if your click doesn't land on a sketch region, Hew looks for a followable face under the cursor instead and sweeps that face's own boundary. The source solid is untouched; the sweep still lands as a new, separate object, unless the face you picked belongs to the very same solid the path is running around — see the merge gesture just below.

This is handy for repeating an existing profile: pull a matching shape off one part of a model and run it somewhere else, without redrawing it as a sketch.

## The merge gesture

Sweeping a molding around a tabletop and then having to union or subtract it yourself is one extra step too many for a common case. Hold **Ctrl (Cmd on macOS)** when you click the profile and Hew does that step for you: the swept shape merges straight into the solid the path runs around, in one undo step. Hew decides which operation makes sense — if the sweep carves into the solid's interior (a chamfer, a dado) it subtracts; if it only rides the surface (a molding, a bead) it unions. A plain click, no modifier, still births a separate object as usual.

This only applies to a sketch-region profile swept around a face-loop path — the status bar says "Ctrl/Cmd-click to merge with the solid" whenever it's available, and stays silent otherwise (an edge path, or a face reached through a component instance, has no solid of its own to merge into).

The one case that merges *without* the modifier is a solid-face profile taken from the very same solid the path runs around — picking a face of the tabletop itself as the profile, to sweep around another face of that same tabletop. There's no ambiguity to ask about there: the object identity already says it belongs together, so it always merges.

## Sweeping into a group

Follow Me works while you're editing a group, same as drawing or push/pull — no need to step back out first. The new solid lands as a member of the group you're in, rather than appearing loose at the top level.

## What Follow Me refuses

Follow Me never commits a broken solid. It refuses, with the model untouched, when:

- the path branches, or the selection is in disconnected pieces;
- on a closed path, the profile stands square to it but doesn't actually touch it anywhere (auto-orientation can square a profile up, but it can't relocate one that's genuinely nowhere near the path);
- the profile sits right at a corner and decisively hangs back over the incoming edge, folding into its own material as the sweep turns it;
- the path doubles back on itself — or turns nearly all the way around — at a corner: there is no clean way to miter a hairpin;
- the path bends tighter than the profile is wide — the sweep would fold into itself;
- the swept shape would run into itself further along the path;
- a profile reaches the lathe axis on a path that *isn't* a drawn circle — only a true circle revolves cleanly into a pole (see [Spheres and other poles](#spheres-and-other-poles)); elsewhere, keep the profile clear of the center.

Each refusal says what to change. Adjust the profile or path and click again — the picked path stays selected.

## Curves stay curves

A drawn circle swept along a straight run is a true cylinder, exactly as if you'd extruded it: it shades smooth, remembers its exact radius, and exports at any resolution — even when the run was drawn in several strokes. Around a path's turns the walls keep their honest facets and export at the resolution they were drawn.

Where the sweep turns a corner along one continuous drawn curve — following a smooth arc, or the round of a lathe — the wall shades smoothly across that turn instead of showing a hard facet line, the same soft shading a real curved surface has. A sharp corner in the path — a straight-line miter, a picture-frame joint — stays a crisp, visible edge, because it genuinely is one.

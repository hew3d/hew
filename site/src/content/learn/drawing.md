---
title: "Drawing"
description: "The four profile tools, offsetting a boundary, what changes when you draw on a face instead of the ground, and every way to type an exact dimension."
order: 5
---

Hew's four drawing tools create 2D profiles. On the empty ground plane, a closed profile becomes a **sketch region** ready to extrude. Drawn directly on a solid's face, the same tools **split the face**, carving regions you can push in or pull out. The tool decides which mode to use from what's under your cursor; there is no explicit switch.

The four tools share one working sketch on the ground, so shapes drawn with different tools combine: draw an arc, switch to Line, and close it with a chord — the face forms the moment the circuit closes, exactly as if one tool had drawn it all. An arch profile, for instance, is three Line segments topped with an Arc, extruded once.

Layered shapes edit like a drawing, too. Where two closed shapes share a line, that line splits them into separate regions; select it with the Select tool and press `Delete`, and the regions merge into one larger face. Draw a rectangle, cap it with an arc, delete the line between them — one arch-shaped region, one pull.

Selection matches what you drew. Clicking a straight line selects just that line. Clicking a drawn arc or circle selects the curve up to wherever it crosses or touches other geometry — an untouched circle selects whole; a circle welded into an outline selects one run at a time, so you can delete the part inside without losing the rounded corner. Clicking inside a filled region selects the connected shape it belongs to, and shapes drawn apart from each other are independent: selecting, deleting, or moving one never touches the other, even though they share the working sketch.

While any drawing tool is active, the status bar explains the next step, the top-right readout shows live dimensions, and everything you type goes into that readout (see "Typing exact values" below).

## Line (`L`)

Draws a chain of straight edges.

1. Click to anchor the first point.
2. Each further click commits one segment and continues the chain, with a rubber-band preview to the cursor.
3. Closing a loop (clicking back on your starting point, or completing any closed circuit) automatically forms a face and ends the chain.

To finish a chain *without* closing it: double-click, press `Enter` with nothing typed, or press `Esc` once. Pressing `Esc` again cancels the tool's gesture entirely.

**Exact lengths:** once a chain is started, type a length (e.g. `750mm`) and press `Enter` — the next segment commits at exactly that length along the direction you're pointing.

**Axis locking:** hold `Shift` to lock the segment to whichever axis it's already leaning toward, or press an arrow key for an explicit lock: `→` locks to X (red), `←` to Y (green), `↑` to Z (blue). Press `↓` (or the same arrow again) to unlock.

## Rectangle (`R`)

1. Click to set the first corner.
2. Click again to set the opposite corner.

**Exact dimensions:** after the first click, type both dimensions separated by a comma or an `x` (`2m,1m`, `50 x 30`, `3',18"`) and press `Enter`. A single value makes a square. The rectangle grows in the direction your cursor was heading.

## Circle (`C`)

1. Click to set the center.
2. Move outward (the readout shows the radius) and click to set a point on the rim, or type an exact radius and press `Enter`.

Hew's circles are stored as regular polygons, but they remember the exact circle you drew — center and radius ride along with the shape, power the Center/Quadrant/Tangent snaps, keep extruded walls smooth on screen, and let STL export re-facet the wall at whatever resolution you pick. The stored facet count adapts to the size of the circle: small circles get 24 sides, larger ones up to 96, keeping the worst chord deviation at about half a millimeter.

## Arc (`A`)

A two-point arc, like SketchUp's:

1. Click one endpoint.
2. Click the other endpoint to set the chord.
3. Move perpendicular to the chord to pull out the bulge, then click to commit.

`Esc` steps back one stage at a time. The readout shows the arc's radius. Typed values work at both stages: in the chord stage a typed length places the second endpoint at that distance, and in the bulge stage it sets the bulge depth. A flat, zero-bulge arc is refused ("Pull out the bulge"). Arcs facet at the same density as circles of the same radius, and like circles they carry their exact center and radius for snapping, smooth display, and export.

**Closing the arc:** press `Option`/`Alt` mid-gesture to cycle what the commit produces — the open arc, a **pie** closed to the center with two straight edges, or a **segment** closed with the chord. The preview draws the closing edges and the readout names the mode. Both closed forms are complete profiles: on the ground they become a region immediately, and on a face they split it, ready to push/pull. The chosen mode sticks for further arcs until you switch tools.

## Drawing on a face

Point any drawing tool at a solid's face and it works there instead of on the ground: edges cut the face, and closed shapes (a rectangle, circle, or closed line/arc loop) split it into regions. The new regions are immediately push/pullable — recesses, through-holes, and raised bosses all start this way. At the top level, any plain object's face is drawable directly — objects are immediately editable, just as they are with Push/Pull. Faces inside a **group or component** are the exception: double-click in first, and drawing is then scoped to what you entered.

## Offset (`F`)

Offset copies an existing boundary a set distance inward or outward — the quickest way to draw a border, a rim, or a wall of even thickness without measuring anything.

1. Click a filled sketch region or a solid's face.
2. Move the cursor inward or outward — a preview of the offset outline follows at the distance shown in the readout.
3. Click to commit, or type an exact distance and press `Enter`.

Every edge of the boundary moves by the same distance: straight edges stay parallel, and a drawn circle or arc offsets to a true concentric curve — same center, radius changed by exactly the offset — so the result snaps, displays, and exports like any freshly drawn curve. A region with a hole offsets both boundaries at once, keeping the band between them even.

On a sketch region, the offset outline joins the same sketch, so both the original and the new region are ready to extrude — offset a circle outward and you have a ring to pull into a pipe wall. On a solid's face, offsetting inward carves the face the way drawing on it does, leaving an inset region to push or pull; only inward offsets fit on a face.

If the distance is more than the shape can absorb — an inward offset past the middle, or an arc squeezed to nothing — the preview disappears and committing is refused with a message; nothing is committed halfway.

## Editing a sketch

**Deleting a line or curve.** With the Select tool, click any sketch line to select it — a facet of a drawn arc or circle selects the whole curve — and press `Delete`. If the line separated two regions, they merge; if it closed a region, the region opens back up. Deleting a shape's last line removes it entirely. Every line you can see is an ordinary line: pulling a region into a solid removes its outline from the sketch (the outline became the solid's base), so nothing invisible lingers behind to complicate later edits. What survives an extrusion — a wall shared with a neighboring shape, a stray construction line — deletes like anything else.

**Deleting or moving a whole shape.** Click inside a shape's filled region (or its row in the outliner) to select the connected shape, then `Delete` removes it or Move slides it — without disturbing anything else you've drawn. A move that would land one shape on top of another is refused rather than welded. (Curve-as-a-unit selection applies to arcs and circles drawn on the ground; lines drawn on a solid's face become part of that solid's geometry.)

**Moving a point.** For a free-standing sketch that hasn't been extruded yet, the **Edit Vertex** tool (Tools ▸ Edit Vertex, or find it in the palette) adjusts a single point:

1. Click a sketch vertex to grab it.
2. Click its new position — the connected edges stretch to follow.

If a move would break the sketch's topology (collapse a segment, fold a region), Hew refuses with a message and leaves the vertex where it was.

## Inference while drawing

Every click snaps. The colored dot and label at the cursor tell you what you're about to snap to — Endpoint (green), Center and Quadrant of a drawn circle or arc (teal), Midpoint (cyan), Intersection (amber — where a construction guide crosses an edge, a sketch line, or another guide), Tangent (violet — where an in-progress line just grazes a drawn circle), On Edge (red), On Face (blue), On Guide (purple), On Axis (the axis color), or Ground (gray). Lines you draw across each other need no cue of their own: crossings become real endpoints the moment they're drawn. A dashed helper line appears through the snap point when the snap has a direction, such as an axis alignment. Construction guides let you add snap targets of your own; [Precision, measurement, and guides](/learn/measurement-and-guides/) covers them.

## Typing exact values

There's no input box to click, and none appears. With a tool mid-gesture, start typing: what you type appears in the top-right readout, `Enter` commits, `Backspace` edits. Every length-driven tool accepts:

- A bare number, read in your current display unit (`1.5` = 1.5 m in Meters mode, 1.5" in an imperial mode).
- An explicit unit that overrides the display unit: `mm`, `cm`, `m`, `km`, `in`, `ft` — `250mm`, `3.5cm`, `6"`, `2'`.
- Feet-inches-fractions, SketchUp style: `5'3"`, `5' 3-1/2"`, `3 1/2"`, `5/8"`.

Angle tools (Rotate, Protractor) take plain degrees; Scale takes a plain factor. Display units are set in **Settings ▸ Units** ([Settings](/learn/settings/)).

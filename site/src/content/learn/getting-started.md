---
title: "Getting started"
description: "From empty window to exported STL in about ten minutes: draw a rectangle, pull it into a solid, look around, print."
order: 1
---

Hew runs in two places: as a web app at [app.hew3d.com](https://app.hew3d.com), with nothing to install and no account required, and as a native desktop app for macOS, Windows, and Linux (see [Download](/download/)). Both are the same application; this guide applies to either.

This walkthrough covers the handful of moves you'll use in almost every session: draw a profile, pull it into a solid, look around, and export a printable file.

## Before you start: the one idea that matters

Hew feels like SketchUp, but it is built around one rule: **every closed shape you extrude becomes a discrete, watertight solid called an Object.** Objects never fuse together just because they touch. Almost everything else the app does follows from that rule.

## 1. Draw a rectangle

Pick the **Rectangle** tool — click it in the tool rail on the left, or press `R`. The status bar at the bottom of the window always tells you what the current tool expects next.

1. Click once on the ground plane to set the first corner.
2. Move the cursor — a live preview follows, and a readout in the top-right corner shows the current width × depth.
3. Click again to set the opposite corner.

![A closed rectangle drawn on the ground plane, shown as a filled sketch region, with "Sketch 1" listed in the Outliner](/docs/first-rectangle.png)

The closed rectangle becomes a filled **sketch region** — you'll see it listed as a Sketch in the Outliner panel on the right. As you draw, watch the colored snap dot and label at the cursor: Hew's inference engine is calling out endpoints, midpoints, axis alignments, and other useful relationships, exactly the way SketchUp does.

**Exact dimensions:** after the first click, type `2m,1m` (or `50cm,30cm`, or `3',18"`) and press `Enter`. This surprises everyone the first time: there is no input field, and none appears — Hew is always listening while a tool is active, and what you type shows up in the top-right readout. See [Precision, measurement, and guides](/learn/measurement-and-guides/) for every accepted format.

## 2. Push/pull it into a solid

Select **Push/Pull** (press `P`), then:

1. Click the rectangle you just drew.
2. Move the cursor away from it; a live preview of the extrusion follows.
3. Click again to set the height, or type an exact height like `1.2m` and press `Enter`.

![A rectangular box created by push/pulling the sketch, selected, with Object Info reporting its geometry as Solid](/docs/first-box.png)

The moment you commit, Hew creates a real Object — a watertight solid. There is no separate "make this a group" step; extruding a closed profile *is* the step. Press `Space` to switch back to the **Select** tool and click the box: the Object Info panel reports its geometry as **Solid**, and the badge in the bottom-right corner of the status bar confirms every object in the model is solid.

## 3. Orbit, pan, and zoom

You can navigate at any time, with any tool active. Drag with the **middle mouse button** to orbit, drag with the **right mouse button** to pan, and scroll the wheel to zoom — Hew zooms toward the cursor.

If you prefer dedicated tools (or have no mouse wheel), press `O` for Orbit, `H` for Pan, or `Z` for Zoom and drag with the left button. The **Top / Iso / Front** buttons in the top-left of the viewport jump to standard views, and **Camera ▸ Zoom Extents** frames the whole model. More in [Viewing your model](/learn/viewing/).

## 4. Export an STL

Choose **File ▸ Export…**, pick **STL binary (.stl)** from the format list, and click Export.

![The Export dialog with its format selector open over a model](/docs/export-dialog.png)

Hew Objects are watertight by construction, so the STL you get is manifold: no gaps, flipped normals, or open shells for your slicer to repair. If any object in the model is not solid, Hew warns you first and lists the offending objects by name, rather than silently handing you a broken file. The exported STL is in millimeters, ready for any slicer.

## Where to go next

That's the core loop: draw a profile, push/pull it into a solid, export. Everything else builds on these moves:

- [Core concepts](/learn/core-concepts/) explains the ideas that make Hew behave differently from SketchUp once models get complicated.
- [The Hew interface](/learn/interface/) tours every panel and control.
- [Drawing](/learn/drawing/) and [Push/Pull](/learn/push-pull/) are the full drawing and modeling reference.

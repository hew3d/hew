---
title: "Getting started"
description: "Build a small desk-organizer set — tray, pen cup, scooped bin, and phone stand — in about ten minutes, and meet the tools you'll reach for every session."
order: 1
---

Hew runs in two places: as a web app at [app.hew3d.com](https://app.hew3d.com), with nothing to install and no account required, and as a native desktop app for macOS, Windows, and Linux (see [Download](/download/)). Both are the same application; this guide works in either.

Rather than draw one lonely box, you'll build something you could actually print: a little desk-organizer set — a tray holding a pen cup, a bin with a scooped front, and a phone stand. It touches the tools you'll use in almost every session, and it takes about ten minutes.

## The one idea that matters

Hew feels like SketchUp, but it is built around a single rule: **every closed shape you push/pull becomes a discrete, watertight solid — an Object — and Objects never fuse just because they touch.** The four parts of the organizer will sit on the tray, share edges, even overlap, and stay four separate things. When you *do* want two solids to become one, you say so. Watch for that idea; the whole app follows from it.

## 1. Draw the tray

Pick the **Rectangle** tool — click it in the tool rail on the left, or press `R`. The status bar along the bottom always spells out what the current tool wants next.

1. Click once on the ground plane to set the first corner.
2. Move the cursor. A live preview follows, and once the first corner is down, a measurement box appears in the top-right of the viewport tracking the width × depth.
3. Click again to set the opposite corner.

![The tray's rectangle drawn on the ground plane as a filled sketch region, listed as "Sketch 1" in the Outliner](/docs/organizer-sketch.png)

The closed rectangle fills in as a **sketch region** — you'll see it as a Sketch in the Outliner on the right. As you draw, notice the colored dot and label at the cursor: Hew's inference engine is calling out endpoints, midpoints, and axis alignments, the way SketchUp does.

**Type the size instead of eyeballing it.** You don't click into that measurement box — Hew is listening whenever a tool is active. After the first corner, just type `24cm,14cm` (or `9.5",5.5"`, or `0.24m,0.14m`) and press `Enter`; your keystrokes land in the box and set the exact size. [Precision, measurement, and guides](/learn/measurement-and-guides/) lists every accepted format.

## 2. Push/pull it into a solid

Switch to **Push/Pull** (`P`):

1. Click the rectangle.
2. Move the cursor up and away; a live preview of the extrusion follows.
3. Click to set the height, or type `1.5cm` and press `Enter`.

![The tray board selected, with Object Info reporting its geometry as Solid and the object dock showing Push/Pull, Move, Paint, Erase](/docs/organizer-tray.png)

That's your tray. The moment you commit, Hew turns the sketch into a real Object — a watertight solid. There's no separate "make it a group" or "make it solid" step; push/pulling a closed profile *is* that step. Press `Space` for the **Select** tool and click the board: Object Info reads **Solid**, and the badge at the bottom-right of the status bar confirms every object in the model is watertight.

### Look around while you work

Navigation is always available, whatever tool is active. Drag the **middle mouse button** to orbit, drag the **right mouse button** to pan, and scroll to zoom toward the cursor. No mouse wheel? Press `O`, `H`, or `Z` for Orbit, Pan, and Zoom and drag with the left button. The **Top / Iso / Front** buttons above the viewport jump to standard views, and **Camera ▸ Zoom Extents** frames everything. There's more in [Viewing your model](/learn/viewing/).

## 3. The pen cup

Two new moves here: drawing a circle, and hollowing a solid by pushing a face *inward*.

1. Pick the **Circle** tool (`C`), click a center point on the ground beside the tray, and type a radius — `4cm` — then `Enter`.
2. With **Push/Pull** (`P`), click the disk and pull it up to `9cm`. You now have a solid cylinder.
3. Hollow it: with the **Circle** tool, draw a smaller circle (`3cm`) directly on the cylinder's **top face** — it snaps to the face's center — then **Push/Pull** that inner disk *down* about `7.5cm`. Pushing a face inward carves a recess, so the cup becomes a cup.
4. Switch to **Select** (`Space`) and click the cup to select it, then press `M` for **Move** and drag it onto the tray. (Move and Rotate always act on the current selection, so select first, then transform.) Because it's its own Object, it rests on the tray without merging into it.

![The tray with the hollowed pen cup resting on it; the Outliner lists two objects, both solid](/docs/organizer-cup.png)

## 4. The bin with a scooped front

The bin starts like the tray — a rectangle pulled up into a block — then hollowed the same way you hollowed the cup. Its scooped front is the one place in this build where you'll *explicitly* combine solids.

1. **Rectangle** (`R`) → a `10cm,8cm` footprint on the ground; **Push/Pull** (`P`) up to `6cm`.
2. Hollow it: draw a rectangle on the block's top face, leaving a wall all around, and push it down to leave a floor — an open box.
3. Model the scoop as its own solid: with **Circle** (`C`) and **Push/Pull**, make a short cylinder. Select it (`Space`, then click), and use **Rotate** (`Q`) to lay it on its side and **Move** (`M`) to position it straddling the top of the bin's front wall.
4. Combine explicitly: with **Select**, click the bin, then `Shift`-click the cylinder, and choose **Edit ▸ Subtract**. The cylinder carves a smooth curved dip out of the front and vanishes, leaving one watertight bin — which stays selected, so press `M` and move it onto the tray.

![The tray with the pen cup and the hollow bin, its front wall dipping in a smooth curved scoop](/docs/organizer-bin.png)

Subtract is one of three booleans (with Union and Intersect); [Combining and splitting solids](/learn/combining-solids/) covers when to reach for each, and when — as with the hollowing above — Push/Pull already does the job.

## 5. The phone stand

The stand is a wedge, and the tidy way to make one is to draw its side profile flat, give it width, and tip it upright.

1. With the **Line** tool (`L`), draw the profile on the ground: a tall back, a slope down to a low front, and a small lip at the front to keep a phone from sliding off. Click point to point and close the loop back on the start — it fills into a region.
2. **Push/Pull** (`P`) the region to `7cm` of width.
3. Select the wedge (`Space`, then click). Press `Q` for **Rotate** and tip it 90° so the slope faces up, then **Move** (`M`) it onto the tray next to the bin.

![The finished grey set — tray, hollow pen cup, scooped bin, and the wedge phone stand — four solid objects on the tray](/docs/organizer-set.png)

Four parts, four Objects. They touch and overlap on the tray, and the status bar still reads four solids: nothing merged on its own.

## 6. Paint the parts

Expand **Materials** in the right-hand tray. Click **Add color**, pick a wood tone, and name it *Oak*; add a few more — a teal, a terracotta, a slate.

To paint a whole part, select it, click the swatch you want, and press **Fill selected object**. Do that for each of the four. (For finer control, the **Paint** tool (`B`) paints one face at a time.)

![The set painted — oak tray, teal pen cup, terracotta bin, slate stand — with the Materials palette open](/docs/organizer-materials.png)

Materials are per-document and they survive modeling: paint follows the geometry through push/pull, slicing, and booleans. See [Materials](/learn/materials/) for textures and opacity.

## 7. Name it, group it, tag it

A model with four "Object N" rows is already worth tidying.

1. Select each part and, in **Object Info**, type a real **Name** — *Tray*, *Pen cup*, *Bin*, *Phone stand* — pressing `Enter` after each.
2. Select all four (`⌘A` / `Ctrl+A`) and choose **Edit ▸ Group** (`⌘G` / `Ctrl+G`). Name the group *Desk organizer*. Now the whole set moves, hides, and selects as one, while the parts stay separate inside it.
3. With the group selected, click **+** next to **Tags** in Object Info and type `Desk/Set`. The **Tags** panel shows the tag tree; tags slice a model into show/hide categories independent of the group structure.

![The Outliner showing the Desk organizer group expanded into its four named parts, with the Desk / Set tag applied](/docs/organizer-organized.png)

[Organizing your model](/learn/organizing/) goes deeper on the Outliner, tags, and visibility.

## 8. Save, then export to print

Save the Hew document with `⌘S` / `Ctrl+S` (or **File ▸ Save**). The native format is **`.hew`** — an open container that keeps geometry, names, groups, materials, and tags together, and saves byte-for-byte identically each time. Hew also autosaves a recovery snapshot every 12 seconds, so a crash won't cost you the session ([Files, saving, and recovery](/learn/files-and-saving/)).

To print, choose **File ▸ Export…**, pick **STL binary (.stl)**, and click Export.

![The Export dialog with STL binary selected](/docs/export-dialog.png)

Because every Object is watertight by construction, the STL is manifold — no gaps, flipped normals, or open shells for your slicer to repair. If anything in the model *weren't* solid, Hew would warn you and name it rather than hand you a broken file. The mesh exports in millimeters, ready for any slicer.

## Where to go next

You've drawn, pushed, hollowed, subtracted, rotated, painted, organized, and exported — the moves behind most of what you'll model. From here:

- [Core concepts](/learn/core-concepts/) explains the ideas that make Hew diverge from SketchUp as models get complicated.
- [The Hew interface](/learn/interface/) tours every panel and control.
- [Drawing](/learn/drawing/) and [Push/Pull](/learn/push-pull/) are the full drawing-and-modeling reference.

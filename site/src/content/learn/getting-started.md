---
title: "Getting started"
description: "Build a small desk-organizer set — tray, pen cup, scooped bin, and phone stand — in about fifteen minutes, and meet the tools you'll reach for every session."
order: 1
---

Hew runs on your desktop as a native app (macOS, Windows, and even Linux - see [Download](/download/)) or as a web app at [app.hew3d.com](https://app.hew3d.com) (nothing to install and no account required). Both are the same application and work the same way.

Rather than draw one lonely box, you'll build a real project: a little desk-organizer set — a tray holding a pen cup, a bin with a scooped front, and a phone stand. It touches all of the tools you'll use in a typical Hew session, takes about fifteen minutes, and the same moves scale straight up to a bookshelf, a deck, or a house.

## 1. Choose centimeters on the welcome screen

On a fresh launch, a welcome screen offers the ability to start modeling in the blank document, open a file, or load a bundled sample.

Before moving on, set the **Units** dropdown. This guide works in centimeters, so choose **Centimeters**.

The unit you pick becomes the default for every length you type and read, and a small unit (centimeters, millimeters, or inches) also starts the view zoomed in to match, so a palm-sized part doesn't begin as a speck on a meter-scale grid.

Every step below types its unit explicitly, so any setting works — but with centimeters as the default, you can type "1" instead of "1cm" and it'll match.

Click **Start modeling** to head into the blank document.

![The welcome screen on a fresh launch: sample models, an Open card, and the Units dropdown set to Centimeters](/docs/welcome-screen.webp)

(Did you turn the welcome screen off earlier? The same choice lives in [Settings](/learn/settings/).)

## 2. Learn the elements of the Hew UI

Hew is laid out roughly in three columns.

![The default layout of a new Hew instance](/docs/default-hew-layout.webp)

On the left is the **tool rail** which shows the primary available tools for quick access, along with their keyboard hot keys. The search field just above the tool rail is the **command palette** - this is a very powerful way of searching for almost anything in Hew (tools, objects, components, materials, help, and more).

In the center is the primary Hew 3D viewport, which is where you interact with the model. It shows a **blue Z axis**, **red X axis**, and **green Y axis** that meet at the **origin**. The cursor will snap to an axis if it gets close. In the upper left are a few convenience buttons that will quick switch the camera. Centered on the bottom is the **contextual dock** - the contents of this dock change depending on Hew's current state to show you the most likely next tools or actions.

On the right are the information panes, mostly empty in a new document. The **Object Info** shows the specific info for the selected object; **Outliner** shows all of the objects in their physical hierarchical relationship; **Materials** shows available textures and paint colors; and **Tags** offers up a way to logically organize the objects.

On the very bottom of everything is the status bar. This shows the current tool in play as well as the first/next action you should be taking with that tool. If you're ever wondering what a tool can do, then the status bar is a good place to look first.

## 3. Sketch the 2D shape of a tray

Pick the **Rectangle** tool — click it in the tool rail on the left, press `R`, or search for it in the command palette. The status bar along the bottom always spells out what the current tool wants next.

1. **Click once** on the ground plane to set the first corner. The **origin** (where the three axes all meet) is a good starting point, but not at all necessary
2. Move the cursor. A live preview follows, and once the first corner is down, a measurement box appears in the top-right of the viewport tracking the width × depth.
3. Click again to set the opposite corner at roughly **24cm x 14cm**.

![A rectangle mid-draw with blue preview outline; VCB reading 24cm,14cm; status bar "Click the opposite corner"](/docs/tray-sketch.webp)

As you draw, notice the colored dot and label at the cursor - that's Hew calling out endpoints, midpoints, and axis alignments. That's essential for many Hew actions.

The closed rectangle fills in as a **sketch region** — you'll see it as a Sketch in the Outliner on the right. Sketches are not full Hew objects but rather the form of a solid to come later.

**Advanced: type the size directly instead of eyeballing it.** After the first corner, just type `24cm,14cm` (or `24,14` or `9.5",5.5"` or `0.24m,0.14m`) and press `Enter`. Hew is listening for input whenever a tool is active and so it will update the measurements directly and set the Sketch to that exact size. Most of the following steps will request a specific size - just type it in and it'll be recognized.

[Precision, measurement, and guides](/learn/measurement-and-guides/) lists every accepted format.

## 4. Push/pull it into a solid

In this step, we create our first Hew Object (our first solid).

Switch to **Push/Pull** (`P`):

1. Click the rectangle Sketch.
2. Move the cursor up and away; a live preview of the extrusion follows.
3. Click to set the height, or type `1.5cm` and press `Enter`.

![Creating the tray Object by Push/Pull on the Sketch](/docs/tray-pushpull.webp)

Congratulations - you made your first watertight solid Object in Hew! Press `Space` for the **Select** tool and click the tray. The **Object Info** pane now fills in the info about it and confirms that it's **Solid**.

## 5. Look around while you work

Navigation is always available, whatever tool is active. Drag the **middle mouse button** to orbit, drag the **right mouse button** to pan, and scroll to zoom toward the cursor.

No mouse wheel? Press `O`, `H`, or `Z` for Orbit, Pan, and Zoom and drag with the left button.

The **Top / Iso / Front** buttons above the viewport jump to standard views, and **Camera ▸ Zoom Extents** frames everything visible.

Try it out! **Orbit** and **Pan** around the model to get a feel for how the camera changes when you move around. To "reset" your view, just click the **Iso** button and it'll reset the camera to a "standard" view. The **Top** and **Front** buttons are similar - they show standardized views of the model that will be extremely handy at certain times.

But yeah, just get used to clicking the **Iso** button since that does a camera reset plus "Zoom Extents" in one fell swoop.

There's more in [Viewing your model](/learn/viewing/).

## 6. Build a hollowed-out pen cup

We'll learn two new moves here: drawing a circle and hollowing a solid by pushing a face *inward*.

1. Pick the **Circle** tool (`C`), click a center point on the ground beside the tray, type `3cm`, and press `Enter`. What you type is the **radius**, so this makes a 6 cm-wide Sketch of a disk.
2. With **Push/Pull** (`P`), click the disk and pull it up to `9cm`. You now have a solid cylinder.
3. Hollow it. Drawing works directly on a solid's face: with the **Circle** tool (`C`), click the center of the cylinder's top face (the cursor will snap to the center if you get close) and type `2.4cm`; then take **Push/Pull** (`P`) and push that inner disk *down* `7.5cm` (eyeballing it or type it in).
4. Switch to **Select** (`Space`) and simply **drag** the cup onto the tray. Or, for more precision, pressing `M` changes to the **Move** tool with the ability to click to a specific location (and to copy, but not now).

![The tray with the hollowed pen cup being moved into place](/docs/tray-with-cup.webp)

Drawing a shape on a solid and then pushing into the solid is a very common way of hollowing a part out. If you go all the way through, then you have a hole (or a pipe, if a cylinder) but if you stop before the end, you have a cup.

## 7. Work on a different plane

So far, all of our work has happened on parts on the "ground". This is the "blue" plane (or *Z-plane*) that crosses the origin. Blue planes can be anywhere up and down the Z-axis, though. There are two other planes - the "red" plane ("X-plane") and "green" plane ("Y-plane"). Likewise, they can move anywhere on the X-axis or Y-axis, resp.

There are several ways to switch to another plane. The two primary ways are to hover over a face that is already on your intended plane and pressing the **Shift** key to lock the cursor into that plane or to use the arrow keys to **lock** onto a particular axis. This is all pretty abstract, so let's make this concrete.

Start by creating a hollow box for use as our parts bin. We will try one new tool to help hollow out the rectangle.

1. **Rectangle** (`R`) → a `7cm,5cm` footprint on the ground; **Push/Pull** (`P`) up `6cm` into a solid block.
2. Hollow it: Choose the **Offset** (`F`) tool and select the top face of the block. Move the mouse around to see it work. Either eyeball a pleasing bin wall width or enter in `0.7cm`
3. **Push/Pull** (`P`) that inner rectangle *down* `5cm` to create an open box

Let's play with the axes now to get a feel for them. For the following, we'll start a move but never click the second point to commit to a location since we're just seeing what happens. If you do accidentally click a second time and choose a location, just do **Undo** (`Ctrl-Z / Cmd-Z`).

1. Choose the **Move** (`M`) tool and select the bin. Note how if you move the cursor around (but do not click a second time), that it moves freely around but will stick to the "ground" plane unless your cursor goes over an inference point that is higher up the Z-axis. You already did this when you moved the pen cup onto the tray.
2. Now press the **Up arrow** key to lock movement to the **blue / Z-axis**. Note how now the bin can only move straight up or straight down, along the blue axis. You can still move the cursor over other points in the model but those points will just dictate the height of the move, not where the move will terminate
3. Now press the **Left arrow** key to lock movement to the **green / Y-axis**. See how now the bin can only move along the green axis.
4. Then, press, the **Right arrow** key to lock movement to the **red / X-axis**. Same as the other two, only now locked to the red axis.
5. Finally, hit the `Esc` key to exit out of the Move without actually moving

It is worth saying that you don't always *have* to use the arrow keys to lock to an axis since the tools will tend to be magnetically attracted to the axes. The arrow keys just make it deterministic.

## 8. Create a "cutter" out of a cylinder

Our goal is to make a "cutter" out of a cylinder that we construct to lie *across* the bin. It'll be easier to do this if you **Orbit** the model to show the right face of the bin more clearly. Alternatively, you can choose **Camera ▸ Standard Views ▸ Right**. Play around with this - being able to master the camera is a critical skill working in 3D.

Drawing on the "right" face means locking to the **red / X-plane** (which is perpendicular to the X-axis, because that's how that works).

1. Choose the **Circle** (`C`) tool and hover over the bin's right face. Then press the **Right arrow** key. The Status Bar confirms that we're locked to the red plane and there are some visual clues as well.
2. Click the upper left corner of the right face as the circle center and enter a radius of `2cm`. We now have a disk Sketch on the right face of the bin, rather than on the ground
3. Choose **Push/Pull** (`P`) to create a cylinder the entire length of the bin (`7cm` or just choose a point on the far edge)

![The bin with a right face disk being extruded into a cylinder](/docs/bin-with-cutter.webp)

## 9. Cut a curve out of the bin

Hew's fundamental nature is to work with independent solids. It's why you could just move the pen cup on top of the tray without them combining or messing with each other. But there is one exception to this rule and that's when using the **Boolean** operations like **Union / Subtract / Intersect**. These are the only tools that will force the solids to interfere with each other in very specific ways.

We are starting with our bin that has a cylinder lying across it, from the previous step.

1. Choose the **Select** (`Space`) tool and select the bin. Then press the `Shift` key and select the cylinder. It's critical to do it in this order and to press `Shift` since this selects both and does it in the right order.
2. Choose the **Subtract** tool either via **Object ▸ Subtract** or do a search in the command palette (`Cmd-/` or `Ctrl-/`) and type in "sub" to filter it down.

![The command palette filtered down to the Subtract tool](/docs/command-palette-sub.webp)

The end result of the **Subtract** is a bin with a cut-out the shape of our cutter (since a **Subtract** removes any part of the initial solid that is shared with the second solid.)

![The bin with a curved cutout from the cutter](/docs/bin-with-cutout.webp)

Move the bin to the tray using your favorite method.

## 10. Edit a Sketch with guide lines

The stand is a wedge: a tall back, a slope for the phone to lean on, and a `1cm` lip at the front so it can't slide off. Drawing that sloped profile freehand is fiddly — the easy way starts from a plain rectangle, and introduces two tools you'll use constantly: guide lines and editing a sketch after the fact

"Guide lines" (also called "construction lines") are temporary lines created parallel to an edge, typically offset by a very specific amount. They are useful for creating inference points and then are simply deleted when they are no longer needed.

**Note**: The **Tape Measure** tool can be used for dramatically different purposes depending on if you start by clicking an **edge** or by clicking an **endpoint**. In this step, we're selecting an **edge**. If you do accidentally start from an **endpoint** and get a very different result, just **Undo** and try again.

1. **Rectangle** (`R`) → a `6cm,8cm` rectangle on the ground beside the tray. That's the profile's bounding box: 6 deep, 8 tall.
2. Press `T` for **Tape Measure** and click the rectangle's bottom line/edge then move upward — a guide line parallel to that edge follows the cursor. Type `1cm` and press `Enter` to pin it exactly 1 cm up.
3. Still on **Tape Measure**, click the rectangle's right line/edge then move left. Type `1cm` again and press `Enter`. You now have two guide lines perpendicular to each other and each 1cm off of an edge.
4. With the **Line** tool (`L`), draw one line from the rectangle's back top corner to the point where the two guide lines cross - watch for the amber **Intersection** snap - and then continue it to the right edge. This splits the rectangle into two angled sections.

![The stand profile Sketch: a 6 by 8 rectangle with two dashed guide lines 1 cm off the base and right edges and a Line splitting it into two regions](/docs/stand-sketch-with-lines.webp)

5. Switch to **Select** (`Space`) and delete the two lines you no longer need: click the rectangle's top edge and press `Delete`, then click the short piece of the right edge above the diagonal and press `Delete`. Each deletion updates the sketch — what's left filled in is the wedge profile.

![The stand profile Sketch with the top line already deleted and the right line selected to be deleted](/docs/stand-sketch-editing.webp)

6. **Edit ▸ Delete Guide Lines** clears the guide, its job done.
7. **Push/Pull** (`P`) the wedge region `5cm` wide.

## 11. Rotate the phone stand upright

The phone stand now looks right, but it's on its side. This step introduces the **Rotate** tool which is another super commonly used tool. Like the name implies, this rotates solids along a pivot point to a specific angle (in degrees).

1. **Orbit** the camera to show the left side of the phone stand (or use the **Left** standard view)
2. Press `Q` for **Rotate** and hover over the left face and press the `Shift` key. The cursor representation turns **red** to show that it's locked to the X-axis.
3. Select the lower right point on the left face as the pivot point, then select the top right point on the same face as the rotate radius point. Freely move around or type in `-90` to rotate the base down to the ground.

![Rotating the phone stand solid to move off its side onto its base](/docs/stand-solid-rotating.webp)

4. You can optionally do another rotate along the blue / Z-axis (hint: press `Shift` while the cursor is on the ground to lock to the blue axis) to turn the phone stand to face the same direction as the other parts.
5. **Move** (`M`) or drag the phone stand onto the tray

![The finished gray set — tray, hollow pen cup, scooped bin, and the wedge phone stand](/docs/organizer-set.webp)

## 12. Name it, group it, tag it

A model with four "Object N" rows is worth tidying.

1. Select each part and, in **Object Info**, type a real **Name** — *Tray*, *Pen cup*, *Bin*, *Phone stand* — pressing `Enter` after each.
2. Select all four (`⌘A` / `Ctrl+A`) and choose **Object ▸ Group** (`⌘G` / `Ctrl+G`, or `Group` in the contextual dock). Name the group *Desk organizer*. Now the whole set moves, hides, and selects as one, while the parts stay separate inside it.
3. With the group selected, click **+** next to **Tags** in Object Info and type `Desk/Set`. The **Tags** panel shows the tag tree; tags slice a model into show/hide categories independent of the group structure.

![The Outliner showing the Desk organizer group expanded into its four named parts, with the Desk / Set tag applied](/docs/organizer-organized.webp)

[Organizing your model](/learn/organizing/) goes deeper on the Outliner, tags, and visibility.

## 13. Save, then export to print

Save the Hew document with `⌘S` / `Ctrl+S` (or **File ▸ Save**). The native format is **`.hew`** — an open container that keeps geometry, names, groups, materials, and tags together, and saves byte-for-byte identically each time. Hew also autosaves a recovery snapshot every 12 seconds, so a crash won't cost you the session ([Files, saving, and recovery](/learn/files-and-saving/)).

To print, choose **File ▸ Export…**, pick **STL binary (.stl)**, and click Export.

![The Export dialog with STL binary selected](/docs/export-dialog.webp)

Because every Object is watertight by construction, the STL is manifold — no gaps, flipped normals, or open shells for your slicer to repair. If anything in the model *weren't* solid, Hew would warn you and name it rather than hand you a broken file. The mesh exports in millimeters, ready for any slicer.

## Where to go next

You've drawn, measured, pushed, hollowed, locked to alternate planes, subtracted, rotated, edited a sketch, organized, and exported — the moves behind most of what you'll model. From here:

- [Core concepts](/learn/core-concepts/) explains the ideas that make Hew diverge from SketchUp as models get complicated.
- [The Hew interface](/learn/interface/) tours every panel and control.
- [Drawing](/learn/drawing/) and [Push/Pull](/learn/push-pull/) are the full drawing-and-modeling reference.
- [Follow Me](/learn/follow-me/) sweeps a profile along a path — pipes, moldings, picture frames, even spheres and goblets
- [Hew on your phone](/learn/hew-on-your-phone/) covers Shop Mode — a touch viewer for taking a finished model to the bench

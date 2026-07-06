---
title: "Getting started"
description: "Draw a rectangle, push it into a solid, and export your first STL — the whole Hew workflow in five minutes."
order: 1
---

Hew runs today as a web app at [app.hew3d.com](https://app.hew3d.com) — nothing to install, no account required to start modeling. This walkthrough covers the same handful of moves you'll use in almost every session.

## 1. Draw a rectangle

Open the app and pick the Rectangle tool. Click once to set the first corner, move the cursor, and click again to set the opposite corner. Watch the inference cues as you draw — Hew will call out square corners, alignment to existing edges, and other useful relationships, the same way SketchUp does.

## 2. Push/pull it into a solid

Select the Push/Pull tool and click the face you just drew. Move the cursor away from the face and click again to set the height. The moment you do this, Hew creates a real, discrete Object — a watertight solid — automatically. There's no separate "make this a group" step; extruding a closed profile *is* the step.

## 3. Orbit, pan, and zoom

Use Orbit to rotate your view around the model, Pan to slide the view without rotating, and Zoom (scroll, or the Zoom tool) to move closer or farther away. These three moves are how you'll navigate almost constantly while modeling, so it's worth getting comfortable with them early.

## 4. Export an STL

Once you have a solid you're happy with, use Export and choose STL. Because Hew Objects are watertight by construction, the STL you get out is guaranteed manifold — it will not have the gaps, flipped normals, or non-closed shells that break slicers and 3D printers. If an Object isn't solid for some reason, Hew tells you before you export, rather than handing you a broken file.

That's the core loop: draw a profile, push/pull it into a solid, and export. Everything else in Hew — more drawing tools, combining Objects, materials, tags — builds on top of these four moves.

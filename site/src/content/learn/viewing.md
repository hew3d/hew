---
title: "Viewing your model"
description: "How to get around: mouse navigation, the dedicated camera tools, standard views, Zoom Extents, and visibility controls."
order: 4
---

Camera navigation never interrupts your work. With any tool active, at any moment, you can orbit, pan, or zoom, even halfway through drawing a line.

## With the mouse

| Action | Input |
|---|---|
| Orbit | Drag with the **middle mouse button** |
| Pan | Drag with the **right mouse button** |
| Zoom | **Scroll wheel** — zooms toward the cursor |

Zooming follows the cursor, so point at what you want to get closer to and scroll. The camera has gentle inertia and clamps its distance between 0.1 m and 50 m from its target.

## With the camera tools

If you're on a trackpad or prefer left-button navigation, activate a dedicated camera tool — the left mouse button then drives the camera instead of the current tool:

- **Orbit** — press `O`, or Camera ▸ Orbit
- **Pan** — press `H`, or Camera ▸ Pan
- **Zoom** — press `Z` (drag up/down), or Camera ▸ Zoom

While the Orbit tool is active, holding `Shift` temporarily pans instead, the same convention SketchUp users know.

When you're done navigating, press `Space` to return to the Select tool (or the shortcut of whatever tool you were using).

## Standard views and framing

- The viewport's top-left chips jump straight to **Top**, **Iso**, or **Front**.
- **Camera ▸ Standard Views** offers all seven: Top, Bottom, Front, Back, Left, Right, and Iso. All are also in the command palette ("Standard View: …").
- **Camera ▸ Zoom Extents** (palette: "zoom to fit") frames every visible thing in the model — solids and sketches alike (guides don't count) — the fastest way back when you've orbited into a corner. Hew also zooms to fit on its own when a model is opened, and a small unit chosen on the welcome screen starts the blank view zoomed in to match.

## Controlling what you see

- **View ▸ Axes** shows or hides the world axes.
- **View ▸ Grid** shows or hides the ground grid, independently of the axes. The ground is a virtual backdrop — it never hides your model, so a Bottom view sees the model straight through it, and geometry lying exactly on the ground stays visible.
- **View ▸ Guides** shows or hides all construction guides (see [Precision, measurement, and guides](/learn/measurement-and-guides/)). Hidden guides also stop participating in snapping.
- The **Outliner** and **Tags** panels have per-object and per-tag visibility toggles ([Organizing your model](/learn/organizing/)).
- When you double-click into a group, component, or object to edit it, the rest of the scene dims so your editing context is unmistakable. Press `Esc` to step back out one level.

## Reading the axes

The world axes use the conventional colors (**X red, Y green, Z blue**), drawn solid in the positive direction and dashed in the negative. Inference cues reuse these colors: when a drawing or move operation locks to an axis, the cue and preview take on that axis's color. Hew is a Z-up application: "up" in your model is the blue axis.

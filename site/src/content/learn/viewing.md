---
title: "Viewing your model"
description: "How to get around: mouse navigation, the dedicated camera tools, standard views, Zoom Extents, visibility controls, and the non-destructive Section Plane."
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

## Looking inside with a Section Plane

The **Section Plane** tool (Tools ▸ Section Plane) is a view aid, not a modeling operation: it clips away part of the model so you can look inside — check wall thickness, verify clearance between mating parts, spot a hidden void — without touching any geometry. It's the opposite of [Slice](/learn/combining-solids/#slice), which actually cuts a solid into two separate objects; a section plane changes only what the viewport draws, and it isn't saved with the file.

1. Activate the tool and click a face — the section plane is created coincident with, and normal to, that face, and becomes active immediately. Click empty ground instead for a horizontal section at ground level. The tool stays active so you can adjust the section right away; press `Space` to return to the Select tool when you're done.
2. Everything on the back side of the plane disappears, and cut walls render from the inside, so wall thickness reads directly off the exposed edges (the cut itself isn't filled in — there's no solid cap).
3. **Sweep the cut** — with the Section Plane tool active, click the translucent widget, then move the cursor to slide the plane along its own normal arrow through the model; click again to set it there (or type an exact distance and press Enter). This is the main way to inspect a design, front to back. Click a face instead to re-place the section somewhere new.
4. **Turn it off without losing it** — run **Tools ▸ Toggle Section Active** (also in the command palette as "Toggle Section Active") to see the whole model again. The widget stays put, drawn dashed, ready to switch back on.
5. **Remove it** — with the Section Plane tool active, press Delete or Backspace. The model returns to whole.

Only one section plane exists at a time — placing a new one replaces whichever was there before.

## Reading the axes

The world axes use the conventional colors (**X red, Y green, Z blue**), drawn solid in the positive direction and dashed in the negative. Inference cues reuse these colors: when a drawing or move operation locks to an axis, the cue and preview take on that axis's color. Hew is a Z-up application: "up" in your model is the blue axis.

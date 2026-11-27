---
title: "Curve tools in tool lists: passages for other chapters"
description: "Text held back from other chapters until the curve tools graduate from Preview."
order: 107
preview: [fillet, revolve, loft, shell, project, path-array, deform, smooth, soften, follow-me-twist]
---
This page is not published. It holds the text that describes the curve tools together, taken out of the chapters named below so that the published guide describes exactly what a production build has. When the feature graduates, move each section into the place it names and delete this page. Each section is the exact text to paste, between the fences.

## ## ## ## ## ## ##

## Section 1

Chapter **Keyboard shortcuts**, section **Tools**: add to the end of the paragraph that starts "Polygon, Flip, Follow Me...", after "...in Settings ▸ Controls." and before the sentence about macOS.

~~~markdown
So do Chamfer (the Fillet tool with the flat cut chosen), and the tools in the **Tools ▸ Shape** submenu (Revolve, Loft, Shell, Project, and Array along Path) and **Tools ▸ Deform** submenu (Twist, Bend, Taper, and Bend to Path), which SketchUp has no key for. Smooth and Soften Edges are in the **Object** menu and have no key either.
~~~

## Section 2

Chapter **Interface**, section **The tool rail**: the **Modify** bullet, in place of the shorter one published now.

~~~markdown
- **Modify** — Push/Pull `P`, Offset `F`, Follow Me, Fillet `⇧F`, Loft, Move `M`, Rotate `Q`, Scale `S`, Eraser `E`
~~~

## Section 3

Chapter **Interface**, section **The tool rail**: the paragraph that starts "More tools don't have rail slots", in place of the shorter one published now.

~~~markdown
More tools don't have rail slots: **Chamfer**, **Revolve**, **Shell**, **Project**, **Array along Path**, the four **Deform** modes, **Protractor**, **Slice**, **Edit Vertex**, **Text**, **Drawing Axes**, and **Position Texture**, plus the camera tools **Orbit** `O`, **Pan** `H`, **Zoom** `Z`, **Zoom Window**, **Position Camera**, **Look Around**, and **Walk**. Reach them from the **Tools** and **Camera** menus, or from the command palette.
~~~

## Section 4

Chapter **Interface**: a whole section, placed after the "More tools don't have rail slots" paragraph and before "Once the shortcuts are second nature".

~~~markdown
## The Tools menu

The Tools menu keeps the everyday tools on one flat list, in the order SketchUp uses: Select, Eraser, Paint, and Position Texture; Move, Rotate, and Scale; Push/Pull, Offset, Follow Me, Fillet, and Chamfer; then Slice and Edit Vertex; the measuring and annotating tools; and Drawing Axes and Section Plane. The tools that make or reshape a whole solid sit in two submenus:

- **Shape** — Revolve, Loft, Shell, and Project, then Array along Path
- **Deform** — Twist, Bend, Taper, and Bend to Path

Hover a submenu's name to open it. The desktop app's menu bar is laid out the same way.

The **Object** menu holds the commands that act on what you've selected, including **Smooth** (Off, 1, 2, 3, and Bake Smooth) and **Soften Edges…**. They're grayed out until a solid is selected. See [Smooth](/learn/smooth/).
~~~

## Section 5

Chapter **Interface**, section **The contextual dock**, the table: these two rows, in place of the shorter ones published now.

~~~markdown
| An Object | Push/Pull, Fillet, Move, Paint, Shell, Make Component, Save to Library, Erase |
| A sketch | Push/Pull, Revolve, Loft, Follow Me, Move, Rotate, Erase |
~~~

## Section 6

Chapter **Interface**, section **The contextual dock**, the paragraph under the table: these two sentences, in place of the one published now ("Hover a drawn shape with nothing selected and the sketch row previews, with Push/Pull ready to click.").

~~~markdown
With a drawn shape selected, Revolve starts at the axis step, because the shape you selected is already the one to turn. Hover a drawn shape with nothing selected and the sketch row previews, with Push/Pull, Revolve, and Loft ready to click.
~~~

## Section 7

Page **Compare ▸ SketchUp** (site/src/content/compare/sketchup.md), the feature tables: these rows were changed or added, in the places the published rows sit now. The published page keeps the rows as they were before the curve tools (Follow Me, Soften / Smooth edges, Fredo6 RoundCorner, Curviloft, Artisan), has an Eraser row that leaves out creasing (pasting the Eraser row below replaces that published row in site/src/content/compare/sketchup.md), and has no Revolve, Fillet / Chamfer, Loft/Deform/Shell/Project, FredoScale or Shape Bender row.

~~~markdown
| Eraser (`E`) | Erase, soften, and smooth edges by clicking or dragging | Click or drag across drawn lines to erase them, with one undo for the whole drag; a solid's edges are never erased, and `Ctrl`/`Cmd`-click creases one so it stays sharp under Smooth. |
| Follow Me | Yes | Yes — partial sweeps by dragging with a live length readout, flat profiles stood upright automatically, spheres and goblets via pole closure, a merge-as-you-sweep gesture, and a typed twist and end scale along an open path |
| Revolve (lathe) | Follow Me around a circle | A Revolve tool: pick a shape, pick an axis, set an angle; the walls are exact spheres, tori, and cones |
| Fillet / Chamfer | No built-in tool (RoundCorner, below) | Fillet (`⇧F`) and Chamfer on solid edges, rims, and hole mouths, with cove, bullnose, and router profiles; a fillet stays editable. Corners of line drawings too |
| Loft, Deform, Shell, Project | Extensions (Curviloft, FredoScale, and others), or by hand | Built-in Loft, Deform (Twist, Bend, Taper, Bend to Path), Shell, and Project onto curved faces |
| Soften / Smooth edges | Manual, with the Soften Edges angle slider | Automatic on drawn curves' facet seams, and **Soften edges under N°** in Object Info for an imported mesh; no per-edge soften gesture. Smooth (Off, 1, 2, 3) rounds a whole object |
| Fredo6 RoundCorner | Fillets and bevels | Built in: Fillet and Chamfer, with round, cove, bullnose, and router profiles you can draw and keep in the Library, and fillets you can resize or remove later |
| Curviloft | Lofting and skinning | Loft: closed shapes in order, an apex, up to two rails, and start markers to steer the match |
| FredoScale | Twist, bend, taper, and stretch a selection through a box | Deform: Twist, Bend, and Taper through a bounding-box gizmo on a whole solid; no free-form stretching of part of a shape |
| Shape Bender | Bend geometry along a curve | Deform ▸ Bend to Path, on a whole solid |
| Artisan | Subdivision and organic sculpting | Partly: Smooth (levels 1 to 3) softens a blocky shape you keep editing, with creased edges that stay sharp. No sculpting brushes, and Hew isn't for organic modeling |
~~~

## Section 8

Chapter **Keyboard shortcuts**, section **During a gesture**, the `⌘`/`Ctrl`-click row: add to the end of the description, before the closing pipe: ` · crease the edge of a solid, keeping it sharp under Smooth (Eraser)`. The Eraser itself is published; only creasing waits for Smooth.

~~~markdown
· crease the edge of a solid, keeping it sharp under Smooth (Eraser)
~~~

## Section 9

File **docs/ROADMAP.md** (not a guide chapter), in the feature overview: add as a paragraph after **Solids that stay solid.** and before **Precision.** Also delete the **Chamfer/fillet** bullet from **Near-term**; it was removed when the feature was built.

~~~markdown
**Curves.** Fillet and Chamfer round or bevel the edges of a solid,
including a rim, a hole's mouth, and a corner where three edges meet, with
round, cove, bullnose, and router-style profiles you can draw and keep in
the Library. A fillet stays editable: resize it, remove it, or push/pull
the face next to it. Revolve spins a shape about an axis, Follow Me can
twist and taper, and Loft, Deform (twist, bend, taper, bend to a path),
Shell, Project, and Array along a Path cover the shapes between. Smooth
turns a blocky model into a soft one while you keep editing the blocks, and
Soften edges makes an imported mesh shade smooth.
~~~

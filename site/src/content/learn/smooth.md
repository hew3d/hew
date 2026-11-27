---
title: "Smooth"
description: "Turn a blocky model into a soft one while you keep editing the blocks, keep chosen edges sharp, and soften imported meshes."
order: 7.3
preview: [smooth, soften]
---

Smooth rounds a solid off. You model with plain flat faces, the way you always do, and Hew draws a soft, pillowy surface over them. A box becomes a cushion, a cut-down block becomes a pebble. The blocky shape underneath is still what you edit.

Use Smooth for things you look at: cushions, pillows, organic forms, a chair's rolled back. For round things you measure, such as a bolt hole or a rounded corner you need at an exact radius, use a true circle or a [Fillet](/learn/fillet-and-chamfer/).

## Turning it on

Select an object and find **Smooth** in Object Info. Choose Off, 1, 2, or 3. Each step rounds the shape more. The same choices are in **Object ▸ Smooth**, where the current step has a check mark. You can also run **Smooth: Off**, **Smooth: 1**, **Smooth: 2**, or **Smooth: 3** from the [command palette](/learn/interface/), which works on everything you've selected.

An object that is too dense for the step you asked for, such as a high step on a model with a very large number of faces, says "This shape is too dense for that Smooth level. Try a lower level, or simplify the shape first." and stays as it was. If a smoothed shape grows too dense later (an import you combined it with, a face you split many times), its soft surface switches off, Object Info shows a badge beside Smooth, and the status bar says so once; lower the step or simplify the shape to bring it back.

## You keep editing the blocks

The faces you modeled are the **cage**. They are what Push/Pull, Move, Rotate, Scale, Offset, and drawing on a face work on, what snaps, and what measures. While any tool other than Select is active, a thin wire of the cage shows over the soft surface, so you can see what you're about to touch. Push/Pull a face of a cushion and the cushion reshapes.

Nothing snaps to the soft surface itself, because it isn't made of faces. Measure the cage.

## Keeping an edge sharp

Smooth rounds every edge unless you tell it not to. To keep an edge crisp (the seam of a cushion, the rim of a lid), **crease** it:

1. Press `E` for the Eraser.
2. `Ctrl`-click (`Cmd`-click on macOS) the edge of a solid. The status bar says "Crease: keeps this edge sharp under Smooth."
3. `Ctrl`-click a creased edge again to undo it: "Uncrease: lets this edge soften under Smooth."

An edge that's already a smooth seam (the facets of a cylinder, say) can't be creased, and the same goes for the edge around an opening.

Plain clicks with the Eraser erase *drawn* lines. A plain click on the edge of a solid doesn't erase anything, because solids don't lose edges; it just reminds you that `Ctrl`-click creases.

New edges you make with Push/Pull on a smooth object start uncreased, so an extrusion rounds with the rest.

## Combining smooth objects

A union, subtract, or intersect works on the cages, and the result keeps the Smooth step of the object you started from. Hew says "Combining smooth objects uses their cages. Bake first for an exact cut." If you need a clean hole drilled through a pillow's actual surface, bake it first.

## Bake

**Bake** (in Object Info beside Smooth, **Object ▸ Smooth ▸ Bake Smooth**, or **Bake Smooth** in the command palette) turns the soft surface into real faces and drops the Smooth setting. After a bake there is no cage any more, and the soft shape is what you edit, like any other solid. Undo brings the cage back. A cage too dense for its step cannot be baked either; the same sentence says so.

## Exporting

STL, 3MF, glTF, and USDZ files carry the soft surface, closed and ready to print. If you want the blocky shape instead, say for a shop drawing, check **Export cages instead** in the export dialog. It shows up when the model has a smooth object. SketchUp (`.skp`) files always get the cage. See [Import and export](/learn/import-export/).

## Soften edges: making an imported mesh look smooth

This is a different tool with a similar name. An STL file has no idea which edges were meant to be smooth, so every one of its triangles shows as a flat facet. Select the object and choose **Object ▸ Soften Edges…** (or find **Soften edges under** in Object Info yourself). The angle field is ready to type in. Enter an angle (it starts at 20°) and press **Apply**. Edges that bend by less than that angle stop showing as lines and shade smoothly, while the sharp corners stay sharp. Hew reports "Softened 12 edges." or "No edges to soften."

Soften doesn't change the shape, and it doesn't touch a creased edge. It's one undo step, like everything else.

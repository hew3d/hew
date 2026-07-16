---
title: "Combining and splitting solids"
description: "Union, Subtract, Intersect, and the Slice tool. When to use each, and when Push/Pull does the job without a boolean."
order: 10
---

Combining solids is always a command you issue; [Core concepts](/learn/core-concepts/) explains why Hew never does it for you. Every input to these commands is a watertight solid, and so is every result.

## Union, Subtract, Intersect

1. With the Select tool, click the first object or group, then **Shift-click** the second.
2. Choose **Edit ▸ Union**, **Edit ▸ Subtract**, or **Edit ▸ Intersect**.

![Two overlapping boxes, both selected, ready to combine](/docs/boolean-selection.png)

**Union** produces one object containing all material from both, and cleans up after itself: where two coplanar faces meet, the seam dissolves and they become a single face. **Subtract** cuts the second selected object away from the first. **Intersect** keeps only the overlapping material.

![The union result: a single watertight object](/docs/boolean-union.png)

When the result is one connected solid, it's a single new Object. When it comes apart — a Subtract that severs the target, or operands that never touched — each piece is its own watertight solid, and the pieces arrive together in a group named after the operands ("Bar − Cutter"), so nothing scatters. Painted materials survive the operation on the faces that survive. Like everything, booleans are undoable in one step.

If an input isn't solid, the operation is refused with an error naming the problem. Fix the leaky object first (check Object Info to find it).

### Combining groups

Either side of a boolean can be a whole [group](/learn/groups-and-components/) instead of a single object, or you can mix — subtract a group of cutters from one solid in a single command. Because you asked for volume algebra, the solids inside a group are first fused into one composite (grouping alone never merges anything), and the command then applies between the two composites. Every solid under a group operand has to be watertight; a leaky one anywhere refuses the whole operation and the message says which side to fix.

One thing a boolean won't do is quietly consume a component instance — instances share their geometry with every other copy, and a boolean destroys its inputs. If an operand is (or contains) an instance, the command refuses and tells you so; **Explode** the instance first (or **Make Unique**, then Explode) and combine the result.

## Slice

The **Slice** tool (Tools ▸ Slice) is the opposite move: cut one solid into two independent, watertight solids along a plane. It's the tool for splitting a model into printable halves.

1. Activate Slice and move the cursor over your solid. The cutting plane previews live:
   - Hovering a **face** lays the plane flat on that face's plane.
   - Otherwise the plane is perpendicular to a world axis: press `→` for X, `←` for Y, `↑`/`↓` for Z (Z, a horizontal slice, is the default).
2. Hold `Shift` to **pin** the current plane so it stops re-orienting as you move; press `Shift` again (or `Esc`) to release.
3. Position the plane — it follows the cursor's snap point, and you can type an exact offset along the plane's normal.
4. Click on the solid to commit.

![A box sliced by an angled plane, with the two halves moved apart](/docs/slice-halves.png)

Both halves are complete, closed solids (Hew caps the cut faces), and one of them is selected afterward so you can immediately Move it apart. Slicing empty space, or a plane that misses the solid, is refused with a message.

## Which tool when?

For a hole or a pocket you usually don't need booleans at all: draw on the face and Push/Pull inward or through ([Push/Pull](/learn/push-pull/)). Union is for merging parts into one printable body. To cut a complex shape out of another, model the cutter as its own solid, position it, and Subtract. To split a model for the print bed, Slice and move the halves apart. And if you only want parts to stay logically together, don't combine them at all — use a [group](/learn/groups-and-components/).

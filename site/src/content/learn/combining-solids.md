---
title: "Combining and splitting solids"
description: "Union, Subtract, Intersect, and the Slice tool. When to use each, and when Push/Pull does the job without a boolean."
order: 12
---

Combining solids is always a command you issue. [Core concepts](/learn/core-concepts/) explains why Hew never does it for you. Every input to these commands is a watertight solid, and so is every result.

<docs-video theme="dark" slug="combining-solids" label="combining solids demonstration"></docs-video>

## Union, Subtract, Intersect

1. With the Select tool, click the first object or group, then **Shift-click** the second.
2. Choose **Object ▸ Union**, **Object ▸ Subtract**, or **Object ▸ Intersect**.

![Two overlapping boxes, both selected, ready to combine](/docs/boolean-selection.webp)

**Union** produces one object containing all material from both, and cleans up after itself: where two coplanar faces meet, the seam dissolves and they become a single face. **Subtract** cuts the second selected object away from the first. **Intersect** keeps only the overlapping material.

![The union result: a single watertight object](/docs/boolean-union.webp)

When the result is one connected solid, it's a single new Object. When it comes apart — a Subtract that severs the target, or operands that never touched — each piece is its own watertight solid. The pieces arrive together in a group named after the operands ("Bar − Cutter"), so nothing scatters. Painted materials survive the operation on the faces that survive. Like everything, booleans are undoable in one step.

If an input isn't solid, the operation is refused and says so. Select the open one (Object Info shows **Open** for it) and choose **Object ▸ Make Solid…** to close it; see [Making imported shapes solid](/learn/import-export/#making-imported-shapes-solid).

A Subtract whose cutter doesn't reach into the first object is refused too: "Nothing was cut away" means the two solids only touch or sit apart, so there was nothing to remove. Both stay where they were. Move the cutter so it passes through the other solid and try again.

### Combining groups

Either side of a boolean can be a whole [group](/learn/groups-and-components/) instead of a single object. You can mix, too — subtract a group of cutters from one solid in a single command. Because you asked for volume algebra, Hew first fuses the solids inside each group operand into one composite (grouping alone never merges anything), then applies the command between the two composites. Every solid under a group operand has to be watertight — one that isn't solid anywhere refuses the whole operation.

A boolean never consumes a component instance silently — instances share their geometry with every other copy, and a boolean destroys its inputs. If an operand is a component inside a group, the command refuses and tells you so; **Explode** the instance first (or **Make Unique**, then Explode) and combine the result. A selected component or [linked copy](/learn/groups-and-components/#linked-copies) on its own is handled for you, as described next.

### Cutting a linked copy

**Subtract** with a linked copy as the thing being cut changes the shared shape, so **every copy shows the cut**. Cut a mortise into one of four table legs and all four legs have it. When the part has other copies, a strip above the status bar asks how far the cut reaches:

- **Cut all 4 copies** (the default; press `Enter`) cuts the shared shape.
- **Only this one** unlinks the selected copy first and cuts just that one. The status line says "Unlinked this copy and cut it", and one undo takes back both the unlink and the cut.

`Esc` cancels the Subtract. The cutter has to be one solid at a time: cutting a linked copy with a group, or with another copy, is refused with "Cut with one solid at a time". The result keeps the cut part's name, tags, and attributes.

**Union** and **Intersect** can't be shared across copies, because the result would belong to two shapes at once. Hew unlinks each linked copy you join, joins them, and announces it: "Unlinked 2 copies to join them". The undo step is labeled "Union (unlinked)" or "Intersect (unlinked)", and one undo restores the copies and their links. A Subtract whose *cutter* is a linked copy does the same.

Booleans also work between two individual members while a group is open for editing — Union, Subtract, or Intersect them just like top-level objects — and the result folds back in as the group's member when you close it.

## Slice

The **Slice** tool (Tools ▸ Slice) is the opposite move: cut one solid into two independent, watertight solids along a plane. It's the tool for splitting a model into printable halves.

1. Activate Slice and move the cursor over your solid. The cutting plane previews live:
   - Hovering a **face** lays the plane flat on that face's plane.
   - Otherwise the plane is perpendicular to a world axis: press `→` for X, `←` for Y, `↑`/`↓` for Z (Z, a horizontal slice, is the default).
2. Hold `Shift` to **pin** the current plane so it stops re-orienting as you move; press `Shift` again (or `Esc`) to release.
3. Position the plane — it follows the cursor's snap point, and you can type an exact offset along the plane's normal.
4. Click on the solid to commit.

![A box sliced by an angled plane, with the two halves moved apart](/docs/slice-halves.webp)

Both halves are complete, closed solids — Hew caps the cut faces. One of them is selected afterward so you can immediately Move it apart. Slicing empty space, or a plane that misses the solid, is refused with a message. Slice works on a group's members too, while the group is open for editing — both halves fold back in as members when you step out.

## Which tool when?

For a hole or a pocket you usually don't need booleans at all: draw on the face and Push/Pull inward or through ([Push/Pull](/learn/push-pull/)). Union is for merging parts into one printable body. To cut a complex shape out of another, model the cutter as its own solid, position it, and Subtract. To split a model for the print bed, Slice and move the halves apart. And if you only want parts to stay logically together, don't combine them at all — use a [group](/learn/groups-and-components/).

---
title: "Groups and components"
description: "Groups bundle things that move together; components repeat one definition across many instances. Neither merges geometry."
order: 13
---

Groups and components are how a model grows past a handful of boxes without becoming unmanageable. Both are organizational: they never merge geometry (that's what [booleans](/learn/combining-solids/) are for).

## Groups

A group bundles objects (and other groups — they nest) into one selectable, movable unit.

- **Create:** select two or more things and choose **Edit ▸ Group** (`⌘G` / `Ctrl+G`), or click **Group** on the contextual dock.
- **Dissolve:** select a group and choose **Edit ▸ Ungroup** (`⇧⌘G` / `Ctrl+Shift+G`); the members return to being independent, unchanged.
- **Edit the contents:** double-click the group (or click **Edit** on the contextual dock). You're now *inside* it: the rest of the scene dims, the Outliner marks the open group "editing", and the breadcrumb reads Model → the group's name → …. Every tool works on the members from here, not just selection and drawing — Push/Pull, Follow Me, Union/Subtract/Intersect between two members, Slice, and Move/Rotate/Scale on an individual member, the same set that works inside a component (below). Anything you create while you're in there — a new group, a component instance, an imported model, 3D text — becomes a member when you step out. Press `Esc`, or double-click empty space, to step back out one level.

Groups nest, and editing follows the nesting: double-click a member group to open it too, one more level on the breadcrumb, one more `Esc` to leave. A component instance inside a group opens for editing the ordinary way — double-click it — once you've drilled in through the group to reach it.

![A group named "Enclosure" selected: Object Info shows its name, type, and tag; the contextual dock offers Edit, Move, Scale, Make Component, Ungroup, Erase](/docs/organization.png)

Moving, rotating, or scaling a group transforms everything inside it together, and Move with `Option`/`Alt` held copies the whole group — nested groups, names, tags, and materials included; component instances inside come along as new instances of the same definition ([Move](/learn/moving-and-transforming/)). Groups also work as boolean operands: Union, Subtract, and Intersect accept a group anywhere they accept a solid ([Combining solids](/learn/combining-solids/)). Hiding a group (the eye toggle in the Outliner) hides all of its contents. Groups are also handy purely as selection sets — a group's name in the Outliner and Object Info makes big models legible.

Delete everything inside an open group and step out, and the group goes with it — undo brings it all back, same as a component left with nothing (below).

## Components

A component is shared geometry: one **definition**, any number of placed **instances**. Every instance has its own position, rotation, scale, and mirroring, but they all reference the same shape. Model one screw, place it eight times; fix the thread once, all eight update.

- **Create a definition:** select an object or group and choose **Edit ▸ Make Component**, or click **Make Component** on the contextual dock. The selection becomes the definition's geometry, and what you had selected is replaced by the first instance.
- **Place more instances:** select an instance and choose **Edit ▸ Place Copy**; the new instance lands just beside the original, ready to Move into position. Or Move an instance with copy mode on (tap `Option`/`Alt`) to drop copies where you want them — and type `x5` right after a copy to place a whole row ([Move](/learn/moving-and-transforming/)).
- **Edit the definition:** double-click any instance. Every other placement of the component disappears for the moment — there's only one copy of the shape, and it's yours to edit — while the members themselves behave exactly like ordinary top-level geometry. Every tool works on them, not just Push/Pull and Paint: draw new geometry (on a member's face, or on any plane — press an arrow key to lock it, exactly like drawing at the top level), Push/Pull, Follow Me, Union/Subtract/Intersect between two members, Slice, and Move/Rotate/Scale on an individual member. Anything you draw while you're in there becomes part of the component too. Step out (`Esc`, or double-click outside the component) and every placement reappears showing the change, wherever it's placed, however it's rotated, scaled, or mirrored — there's still only one shape underneath.

An instance that's mirrored, scaled unevenly on one axis, or has a sibling placement kept inside some *other* group, edits a little differently: its siblings stay visible the whole time instead of stepping out of view, because that placement can't be lifted out to edit alone. Everything else — the tools, drawing, stepping out — works the same, except that a definition edited this way still needs at least one member: deleting its last one is refused, rather than emptying the component.

Deleting a member otherwise works the same way you'd delete anything else. Delete every member and step out, and the whole component — and every instance of it — disappears with it (undo brings it all back).

Saving works normally while you're in the middle of editing a component, autosave included — your place in the edit holds, and the saved file reflects the component as it stands.

Construction guides (Tape Measure, Protractor) stay ground-truthed to the world even while you're inside a group or component — a guide dropped there won't follow the member around. Use guides at the top level, or on an exploded copy, when you need them for a component's own shape. The Tape Measure's resize gesture is the exception: measure and type a new length while a group or component is open, and it offers to resize just that container instead of the whole model ([Precision, measurement, and guides](/learn/measurement-and-guides/)).

### Names and tags

A component keeps the identity of what you made it from. If the object was named "The Box" and tagged `Objects/Boxes`, the new component's **definition name** is "The Box" and the first instance carries the tag. An unnamed selection gets a generated name ("Component 1", "Component 2", …).

The definition name is what every instance displays — place six copies of "The Box" and the Outliner shows six rows all named "The Box", which is how you can tell they're the same component. Two name fields in Object Info control this:

- **Definition Name** renames the component itself; every instance updates at once.
- **Instance Name** labels just this placement. The Outliner then shows it as "Instance Name (Definition Name)" — "Front Door (Door)" — so the relationship stays visible.

Object Info also counts the siblings: a selected instance's Type reads "Component (6 instances)", and clicking the count selects all six, in the viewport and the Outliner together.

### Breaking the link

Two commands take an instance out of the shared-definition world, both available on the contextual dock when an instance is selected:

- **Make Unique** detaches this instance into its own new definition. Use it when one screw needs to be different from its siblings. The new component is named "Screw Copy" (then "Screw Copy 2", and so on) — unless the instance had its own name, which becomes the new definition's name.
- **Explode** bakes the instance down into ordinary, independent geometry in place. The definition (and other instances) are unaffected.

## In the Outliner

Groups appear as folders you can expand; component instances get their own hexagon icon. Double-clicking a row in the Outliner opens every level between the top and that row in one step — group, nested group, instance, whatever the chain is — landing in the same state you'd reach clicking through the viewport one level at a time, with each level on the breadcrumb marked "editing". The breadcrumb at the top of the Outliner shows where you are and offers one-click exits.

## Current limits

Component definitions can't yet contain other components. That's planned.

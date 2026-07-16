---
title: "Groups and components"
description: "Groups bundle things that move together; components repeat one definition across many instances. Neither merges geometry."
order: 10
---

Groups and components are how a model grows past a handful of boxes without becoming unmanageable. Both are organizational: they never merge geometry (that's what [booleans](/learn/combining-solids/) are for).

## Groups

A group bundles objects (and other groups — they nest) into one selectable, movable unit.

- **Create:** select two or more things and choose **Edit ▸ Group** (`⌘G` / `Ctrl+G`).
- **Dissolve:** select a group and choose **Edit ▸ Ungroup** (`⇧⌘G` / `Ctrl+Shift+G`); the members return to being independent, unchanged.
- **Edit the contents:** double-click the group (or click **Edit** on the contextual dock). You're now *inside* the group's context: the rest of the scene dims, and selection and drawing are scoped to the group's members. Press `Esc` to step back out one level.

![A group named "Enclosure" selected: Object Info shows its name, type, and tag; the contextual dock offers Edit, Move, Scale, Ungroup, Erase](/docs/organization.png)

Moving, rotating, or scaling a group transforms everything inside it together, and Move with `Option`/`Alt` held copies the whole group — nested groups, names, tags, and materials included; component instances inside come along as new instances of the same definition ([Move](/learn/moving-and-transforming/)). Groups also work as boolean operands: Union, Subtract, and Intersect accept a group anywhere they accept a solid ([Combining solids](/learn/combining-solids/)). Hiding a group (the eye toggle in the Outliner) hides all of its contents. Groups are also handy purely as selection sets — a group's name in the Outliner and Object Info makes big models legible.

## Components

A component is shared geometry: one **definition**, any number of placed **instances**. Every instance has its own position, rotation, scale, and mirroring, but they all reference the same shape. Model one screw, place it eight times; fix the thread once, all eight update.

- **Create a definition:** select an object and choose **Edit ▸ Make Component**. The selection becomes the definition's geometry, and what you had selected is replaced by the first instance.
- **Place more instances:** select an instance and choose **Edit ▸ Place Copy**; the new instance lands just beside the original, ready to Move into position. Or Move an instance with `Option`/`Alt` held to drop copies where you want them ([Move](/learn/moving-and-transforming/)).
- **Edit the definition:** double-click any instance. Changes — push/pulls, added detail, paint — appear in every instance when you step out.

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

Groups appear as folders you can expand; component instances get their own hexagon icon. Double-clicking a row in the Outliner enters that item's editing context, same as double-clicking in the viewport. The breadcrumb at the top of the Outliner shows where you are and offers one-click exits.

## Current limits

Component definitions can't yet contain other components. That's planned.

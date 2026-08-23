---
title: "Groups and components"
description: "Groups bundle things that move together; components repeat one definition across many instances. Neither merges geometry."
order: 13
---

Groups and components are how a model grows past a handful of boxes without becoming unmanageable. Both are organizational: they never merge geometry (that's what [booleans](/learn/combining-solids/) are for).

## Groups

A group bundles objects (and other groups — they nest) into one selectable, movable unit.

- **Create:** select two or more things and choose **Object ▸ Group** (`⌘G` / `Ctrl+G`), or click **Group** on the contextual dock.
- **Dissolve:** select a group and choose **Object ▸ Ungroup** (`⇧⌘G` / `Ctrl+Shift+G`); the members return to being independent, unchanged.
- **Edit the contents:** double-click the group (or click **Edit** on the contextual dock). You're now *inside* it: the rest of the scene dims, the Outliner marks the open group "editing", and the breadcrumb reads Model → the group's name → …. Every tool works on the members from here, not just selection and drawing — Push/Pull, Follow Me, Union/Subtract/Intersect between two members, Slice, and Move/Rotate/Scale on an individual member, the same set that works inside a component (below). Anything you create while you're in there — a new group, a component instance, an imported model, 3D text — becomes a member when you step out. Press `Esc`, or double-click empty space, to step back out one level.

Groups nest, and editing follows the nesting: double-click a member group to open it too, one more level on the breadcrumb, one more `Esc` to leave. A component instance inside a group opens for editing the ordinary way — double-click it — once you've drilled in through the group to reach it.

![A group named "Enclosure" selected: Object Info shows its name, type, and tag; the contextual dock offers Edit, Move, Scale, Make Component, Ungroup, Erase](/docs/organization.webp)

Moving, rotating, or scaling a group transforms everything inside it together, and Move with `Option`/`Alt` held copies the whole group — nested groups, names, tags, and materials included; component instances inside come along as new instances of the same definition ([Move](/learn/moving-and-transforming/)). Groups also work as boolean operands: Union, Subtract, and Intersect accept a group anywhere they accept a solid ([Combining solids](/learn/combining-solids/)). Hiding a group (the eye toggle in the Outliner) hides all of its contents. Groups are also handy purely as selection sets — a group's name in the Outliner and Object Info makes big models legible.

Delete everything inside an open group and step out, and the group goes with it — undo brings it all back, same as a component left with nothing (below).

## Components

A component is shared geometry: one **definition**, any number of placed **instances**. Every instance has its own position, rotation, scale, and mirroring, but they all reference the same shape. Model one screw, place it eight times; fix the thread once, all eight update.

![Four Shelf Bracket instances in a row with one selected: Object Info reads "Component (4 instances)" and the contextual dock offers Edit, Make Unique, and Explode](/docs/components-instances.webp)

- **Create a definition:** select one or more objects, groups, or component instances and choose **Object ▸ Make Component**, or click **Make Component** on the contextual dock. The selection becomes the definition's members — a selected group stays whole, and a selected instance nests its definition inside the new one ([Nesting](#nesting)) — and what you had selected is replaced by the first instance.
- **Place more instances:** select an instance and choose **Object ▸ Place Copy**; the new instance lands just beside the original, ready to Move into position. Or Move an instance with copy mode on (tap `Option`/`Alt`) to drop copies where you want them — and type `x5` right after a copy to place a whole row ([Move](/learn/moving-and-transforming/)).
- **Edit the definition:** double-click any instance. Every other placement of the component disappears for the moment — there's only one copy of the shape, and it's yours to edit — while the members themselves behave exactly like ordinary top-level geometry. Every tool works on them, not just Push/Pull and Paint: draw new geometry (on a member's face, or on any plane — press an arrow key to lock it, exactly like drawing at the top level), Push/Pull, Follow Me, Union/Subtract/Intersect between two members, Slice, and Move/Rotate/Scale on an individual member. Anything you draw while you're in there becomes part of the component too. Step out (`Esc`, or double-click outside the component) and every placement reappears showing the change, wherever it's placed, however it's rotated, scaled, or mirrored — there's only one shape underneath.

An instance that's mirrored, scaled unevenly on one axis, or has a sibling placement kept inside some *other* group, edits a little differently: its siblings stay visible the whole time instead of stepping out of view, because that placement can't be lifted out to edit alone. Everything else — the tools, drawing, stepping out — works the same, with one exception: a definition edited this way always needs at least one member, so deleting its last one is refused rather than emptying the component.

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

## Nesting

A component definition can contain other components (and groups) as members, the same way it contains objects — place a wheel component inside a car component, and the car definition holds four wheel instances alongside its own geometry. Make Component builds this from a selection that includes instances: a selected group stays whole rather than being flattened, and a selected instance becomes a nested member instead of being refused.

Editing drills down the same way it does for groups. Double-click a component to open it; inside, a nested component instance is an ordinary instance, so double-clicking it opens it too, one more level on the breadcrumb. `Esc` steps back out one level at a time, the same order you drilled in. Placing a new instance while you're editing another — from the Library, or with Make Component on something you just drew — folds that instance into the definition you're editing once you step out. That's the everyday way an assembly gets built: open the car, place a wheel four times, step out.

A definition can't contain itself, directly or through some chain of nested definitions, and nesting is bounded to 64 levels deep — both refused outright rather than silently truncated.

Explode and Make Unique treat nesting differently. Exploding an assembly surfaces its nested components as ordinary instances in the right place, each sharing geometry with its own definition. Make Unique only copies one level: the copy's own nested instances share their inner definitions with the original, the same as SketchUp.

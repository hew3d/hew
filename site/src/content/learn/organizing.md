---
title: "Organizing your model"
description: "Names, tags, visibility toggles, and the Outliner keep a growing model legible and searchable."
order: 16
---

Once a model outgrows a dozen objects, you stop finding things by orbiting around looking for them. The right-hand tray is the answer: the **Outliner** shows everything, **Object Info** names and tags the selected thing, and **Tags** toggles whole categories on and off.

<docs-video theme="dark" slug="organizing" label="organizing demonstration"></docs-video>

![A named group with a tag: Object Info shows the name and tag chip, the Outliner shows the group row, and the Tags panel shows the tag tree with counts](/docs/organization.webp)

## The Outliner

The Outliner (View ▸ Model Info, `⇧⌘I` / `Ctrl+Shift+I`) lists the document tree under a root **Model** row: every object, group (expandable), component instance, and free-standing sketch.

- **Click** a row to select it, `Shift`/`⌘`/`Ctrl`-click for multi-select.
- **Double-click** a row to enter that item's editing context, exactly like double-clicking it in the viewport. The breadcrumb at the top shows your current context and steps you back out.
- **Drag** a row onto a group to move it in, or onto the Model row to pull it back out to the top level — see [Adding to an existing group](/learn/groups-and-components/#adding-to-an-existing-group).
- **The dot at the row's right edge** toggles visibility: ● shown, ○ hidden. Hiding a group hides everything inside it — a child of a hidden group shows a fainter ○ to say "hidden because its parent is."
- **Filter** the list with the search field above it: type a few letters and only matches (plus the collapsed groups standing between them and the top, shown dimmed) stay visible. Clear it to get everything back, including whatever you'd expanded or collapsed by hand.
- **Sort** the list with the three small buttons next to the search field: **Document order** (the default, the order things were made), **Name**, or **Kind** (objects, groups, components, then sketches, each by name). Names ignore capitals and read numbers as numbers, so "Group 2" comes before "Group 10". The choice applies at every level, is remembered for you rather than saved in the file, and only changes the order you see: an unnamed "Group 3" stays "Group 3".
- **Hide or show everything at once** from the Model row's own eye, or hover a group row for a second, stacked-eye button that hides or shows every one of its members in one click, reaching into nested groups too.
- **Linked copies** show a link icon in place of the hexagon, and a **×N** badge when more than one copy shares the shape. A copy reads as its own name if it has one, else the shared name, else the board's name, else "Copies". Hover the badge to light up the other copies; hover the row for a small **Copies** menu with Unlink, Unlink all, Make component, Select all copies, and Rename. See [Linked copies](/learn/groups-and-components/#linked-copies).
- Icons carry meaning: a cube is an object (a flat panel or curved sheet has a thin outline, and any other open shape has a dashed one), a folder is a group, a hexagon is a component instance, two linked rings are a linked copy, a pen curve is a sketch.

## Object Info: names and tags

Object Info (View ▸ Object Info, `⇧⌘O` / `Ctrl+Shift+O`) shows the single selected item:

- **Name**: type a new one and press `Enter`. Clear the field to fall back to the automatic label ("Object 1", "Group 2", …). Sketches can't be renamed.
- **Type**: Object, Group, Component, Linked copy, or Sketch.
- **Copies**, for a linked copy: **Linked copies: N**, a **Select all copies** link, and the **Unlink** and **Make component** buttons. The two name fields read **Name (all copies)**, which renames every copy, and **This copy**, which names only the selected one; leave **This copy** empty and the shared name shows.
- **Geometry**, for objects: **Solid**, **Flat panel**, **Curved sheet**, or what's open ("Open: 2 openings", "Open edges branch"). An object that isn't solid also gets the buttons that apply: **Close**, **Add thickness…**, **Keep as is**, and **Show in list**. [Core concepts](/learn/core-concepts/) explains why solids matter.
- **Dimensions**: the selection's overall size — `X × Y × Z` in your current units — as a world, axis-aligned bounding box. A rotated part's box is bigger than its own footprint, same as SketchUp's; it's the number that tells you whether the thing fits your print bed or stock, not its true local size.
- **Tags**: the item's tag chips. Click **+** to add a tag; click a chip's **×** to remove it. Click a chip's name to jump to that tag in the Tags panel: the panel opens if it was closed, the tag's parents open, and the tag's row is selected and scrolled into view.
- **Materials**: every material on the selection, each as a swatch and a name. It covers the item's own material and any single faces you painted. For a group or a component, it covers everything inside, so you can see at a glance what a whole kitchen is made of. Click one to jump to it in the Materials panel, which opens if it was closed. Nothing here removes a material; use the Paint tool for that. An unpainted selection reads **None**. After twelve materials, **+N more** shows the rest.

With several things selected, Object Info shows the count, the selection's combined Dimensions, and a **Tags** section for the whole selection: the tags that *every* selected item carries. **+** adds a tag to every selected item (it appears in the list at once) and a chip's **×** removes it from all of them — each as a single undo step. Chips there jump to the Tags panel the same way. A **Materials** list below it shows every material across the whole selection. A tag only some of the selection carries isn't listed; select those items on their own to see it. Select one item to edit its name and other details.

## Tags

Tags are labels for slicing a model into toggleable categories — Structure, Hardware, Reference, whatever fits your project. Unlike groups, tags don't affect the model tree; an object can carry any number of tags no matter where it lives.

- **Add a tag** in Object Info with the **+** button. Use `/` to nest: typing `Structure/Roof` creates (or reuses) a *Structure* parent with a *Roof* child. Adding a tag that's currently hidden hides the item at once.
- **The Tags panel** (View ▸ Tags, `⇧⌘T` / `Ctrl+Shift+T`) shows the resulting tree, with a count of tagged items on each row and an eye toggle that hides everything tagged at or under that path.
- **Click a tag** to select everything that carries it (or a tag nested under it) — the same gesture as clicking a row in the Outliner, and the quickest way to grab a whole category to move, group, or re-tag it.
- **Rename a tag** the way you rename a file: click the already-selected row a second time (a moment later, not a double-click), or double-click it, and its name opens for editing in place. Enter commits, Escape cancels. Only that segment changes — sub-tags and every item carrying the tag follow it, and the rename is undoable. A name that's already in use is refused.
- **Filter tags** with the search field above the list, the same way you filter the Outliner: type a few letters and only matching tags stay, plus the tags that hold them (shown dimmed). Tags are listed alphabetically, ignoring capitals and reading numbers as numbers.
- Tag visibility **composes** with Outliner visibility — an item hidden by either stays hidden until both show it again.
- **Delete a tag** with the **×** button on its row. This removes the tag (and any sub-tags) from every item that carries it — the items themselves are never deleted, and anything that was hidden only by that tag becomes visible again. Deleting a tag is undoable.
- Tags import from SketchUp files, so a tagged SketchUp model arrives pre-organized. They're saved in your `.hew` file and searchable in the command palette.

## Practical conventions

- Name objects as soon as a model has more than a few — future you (and the command palette, which searches model names) will thank you.
- Prefer **groups** for things that move together, **tags** for things that show/hide together, and **components** for repeated parts ([Groups and components](/learn/groups-and-components/)).
- The badge in the status bar tells you *whether* anything isn't solid; the Outliner's icons, Object Info, and **Object ▸ Model Health…** tell you *what*.

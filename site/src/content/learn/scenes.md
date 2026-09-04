---
title: "Scenes"
description: "Save named views of a model — camera, what's hidden, the section cut, and display toggles — and switch between them in one click, on the desktop or on your phone."
order: 17
---

A Scene is a saved view of your model: where the camera sits, which objects and tags are hidden, whether and where the section plane cuts, and whether the grid, axes, and guides show. Activate a Scene and all of that comes back in one step. Set up "Assembled", "Cut layout on the sheet", and "Section through the tenons" once at the desk, then flip between them at the bench.

<docs-video theme="dark" slug="scenes" label="scenes demonstration"></docs-video>

![The Scenes section of the tray with three Scenes; the active one shows a drift ring and an Update button, and its details are expanded](/docs/scenes-tray.webp)

Scenes are saved in the `.hew` file, so they travel with the model — to another machine, to the Library, and to [Shop Mode on your phone](/learn/hew-on-your-phone/#scenes-on-the-phone).

## Adding a Scene

Set up the view you want — orbit, hide what shouldn't show, place a section cut if you need one — then click **⊕** in the tray's **Scenes** section (or **View ▸ Scenes ▸ Add Scene**, or "Add Scene" in the command palette). Hew captures the current view, names it "Scene 1", "Scene 2", … and opens the name for editing right away: type a better one and press Enter, or press Escape to keep the auto-name. The new Scene lands after whichever Scene was active and becomes active itself.

Each Scene remembers five things, and you can turn each one on or off in its details (expand a row with the chevron at its right):

| Property | What it restores |
|---|---|
| **Camera** | Position, direction, field of view, and perspective vs. parallel projection |
| **Hidden objects** | Which objects, groups, and instances are hidden (the Outliner's eye toggles) |
| **Visible tags** | Which tags are hidden (the Tags panel's eye toggles) |
| **Section plane** | Whether a section is placed, where it cuts, and whether it's on |
| **Display** | The View ▸ Grid, Axes, and Guides toggles |

Uncheck a property and the Scene stops touching it — a "tags only" Scene that changes what shows without moving the camera is a Scene with Camera unchecked, nothing more. Unchecking drops what was captured; check it again to capture the current state.

Anything you create after a Scene exists is visible in it. Scenes remember what's *hidden*, so new geometry and new tags show up everywhere until you hide them and update.

## Activating a Scene

A document with Scenes opens on its first Scene, on the desktop and on the phone — so a model always shows one of its saved views rather than wherever the last edit left the camera. Click a Scene's row to switch. If you're inside a group or component, Hew steps out to the model first. The camera glides to the saved view (about half a second; **View ▸ Scenes ▸ Scene Transitions** turns the animation off, and it never animates when your system asks for reduced motion) while visibility, the section cut, and the display toggles switch immediately. Any camera input mid-glide stops it where it is.

**Page Down** and **Page Up** step to the next and previous Scene, wrapping at the ends; **View ▸ Scenes** and the command palette have the same Next Scene / Previous Scene entries.

The active Scene stays highlighted while you work. Orbit away, hide something, or move the section, and its dot becomes a ring: the view has *drifted* from what the Scene captured. Standard views, Zoom Extents, and eye toggles all count as drift; none of them deactivate the Scene. Click the row again to snap back, or —

## Updating a Scene

Click the **↻** that appears on a drifted row (or **View ▸ Scenes ▸ Update Scene**) to re-capture every checked property from the current view. There's no dialog: the checkboxes in the row's details are the durable answer to "which properties?". Update also refreshes the row's thumbnail.

If a Scene hides objects that have since been deleted, its details show a line like "2 captured objects no longer exist." — harmless, and Update clears it.

## Renaming, describing, ordering, deleting

- **Rename** by double-clicking the name. Names must be unique within a document; a clash is shown right under the field.
- **Describe** in the details' text box. The description shows on the phone under the Scene's name — "cut layout on a 4×8 sheet, ¾ ply" earns its keep there.
- **Reorder** with the ↑ ↓ buttons in the details footer. Order is the order Page Down walks, and the order the phone shows.
- **Delete** from the details footer. Hew asks first, because Scene changes are not undoable — they sit beside the camera and the visibility toggles as view state, outside the undo history. (Everything else in the model undoes normally; undoing a model edit never touches your Scenes.)

Adding, updating, renaming, reordering, and deleting Scenes mark the document as edited. Activating one does not.

## Thumbnails

Each row carries a small thumbnail captured when the Scene was added or updated (there's a **Refresh thumbnail** button in the details). Thumbnails are derived, not stored in the file: Hew keeps them in a local cache keyed to the saved document, so reopening the same file finds them again. A file opened elsewhere shows a camera placeholder until its Scenes are updated there.

## Scenes and the section plane

Placing a [section plane](/learn/viewing/#looking-inside-with-a-section-plane) saves it with the document, and a Scene captures the plane *by value* — its position, direction, and on/off state. Three Scenes can share one plane at three different depths: place it, add "Section A", sweep it, add "Section B", and each Scene brings its own cut back. Delete the plane and a Scene that captured one restores it.

## Scenes from the API

`hew.scenes.list`, `add`, `update`, `rename`, `describe`, `reorder`, `remove`, and `apply` drive Scenes from scripts and MCP, and `hew.view.snapshot` renders a named Scene headlessly (its camera and visibility; the section cut is not rendered off-screen). See the [Hew API reference](https://github.com/hew3d/hew/blob/main/docs/API_REFERENCE.gen.md).

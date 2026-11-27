---
title: "Fillet and Chamfer: passages for other chapters"
description: "Text held back from other chapters until Fillet and Chamfer graduates from Preview."
order: 101
preview: [fillet]
---
This page is not published. It holds the text that describes Fillet and Chamfer, taken out of the chapters named below so that the published guide describes exactly what a production build has. When the feature graduates, move each section into the place it names and delete this page. Each section is the exact text to paste, between the fences.

## ## ## ## ## ## ## ##

## Section 1

Chapter **Drawing**, section **Editing a sketch**, after the paragraph about dragging a vertex and before **Circle segments.**

~~~markdown
**Rounding a corner.** To round or bevel a corner where two straight lines meet, on the ground or on a face, use [Fillet or Chamfer](/learn/fillet-and-chamfer/#rounding-corners-in-a-drawing). The corner becomes a true arc.
~~~

## Section 2

Chapter **Viewing**, the **View ▸ Tangent Edges** bullet in the list of View menu toggles. It replaces the shorter bullet that is published now.

~~~markdown
- **View ▸ Tangent Edges** shows the thin lines where a fillet or a rounded corner meets a flat face, and where the straight walls of a rounded rectangle meet its corners. They're hidden by default, since a smooth join isn't an edge, and they print only when the setting is on. See [Fillet and Chamfer](/learn/fillet-and-chamfer/).
~~~

## Section 3

Chapter **Library**, the sentence before the list of item kinds: say "four kinds" instead of "three kinds".

~~~markdown
Items come in four kinds
~~~

## Section 4

Chapter **Library**, the list of item kinds: add as the fourth bullet, after **Models**.

~~~markdown
- **Profiles** — the cross-section of an edge, for the [Fillet tool](/learn/fillet-and-chamfer/): a chain of lines and arcs, not a thing you place in a model.
~~~

## Section 5

Chapter **Library**: a whole section, placed between the paragraph that ends "...which action comes first." and **Saving to the library**.

~~~markdown
## Profiles

A profile is the shape a Fillet cuts along an edge: an ogee, a bead, a cove-and-bead, whatever you'd put on a router. Hew comes with seven (Ogee, Roman ogee, Bead, Thumbnail, Cove and bead, Chamfer and bead, and Cavetto), and you can keep your own in the Library.

**Make one.** Draw a single open line of segments and arcs, the outline of the cut as you'd see it from the end of the board. Select it, choose the Fillet tool, open the **Edge** chip, and pick **Save profile…**. Name it and save. The dialog shows the **Detected width**, the width the profile cuts into the face it's measured on, which is also the width it gets when you use it. If the line you selected isn't one connected open chain, Hew says "Select a drawn open line or arc first, then save it as a profile."

To start from a built-in instead, choose it in the Edge chip and pick **Save profile…**: you get an editable copy. In the Library's Profiles category the built-ins also have **Save a copy**.

**Use one.** Pick it from the Edge chip, under **My profiles**, or open **Window ▸ Library**, choose Profiles, and press **Use**. Either way the Fillet tool is armed with that profile, and the size you drag or type is its width. A profile can't be inserted into a model like a component; it only shapes edges.

**Edit one.** In the Library, a profile you saved has **Open for editing**. It opens as a small document of its own: change the line, then use **File ▸ Save to Library…**, which offers **Replace "name"** so the same item is updated everywhere you use it. The built-in profiles are read-only, and carry a Built-in badge.
~~~

## Section 6

Chapter **Keyboard shortcuts**, section **Tools**, the tool table: add after the **Offset** row.

~~~markdown
| Fillet | `⇧F` (`Shift+F`) |
~~~

## Section 7

Chapter **Keyboard shortcuts**, section **During a gesture**, the `Shift`-click row: add to the end of the description, before the closing pipe: ` · collect edges and faces to fillet together (Fillet)`.

~~~markdown
· collect edges and faces to fillet together (Fillet)
~~~

## Section 8

Chapter **Keyboard shortcuts**, section **During a gesture**, three rows: add after the `Shift` + `⌘`/`Option`-click row.

~~~markdown
| `Option`/`Alt` (tap) | Switch between Round and Chamfer (Fillet) |
| `⌘A` / `Ctrl+A` | Pick every sharp edge of the object under the pointer (Fillet) |
| `Tab` | Move the typed value to the next field: radius, the stops, each radius point (Fillet) |
~~~

---
title: "The Hew interface"
description: "Find your way around the window, from tool rail to status bar, including the tools that don't have a button on screen."
order: 3
---

Hew's window has the same layout on the web, macOS, Windows, and Linux. The only difference is the chrome around it: macOS uses the system menu bar, Windows and Linux draw their own title bar, and the web app shows an in-app menu bar.

<docs-video theme="dark" slug="interface" label="interface demonstration"></docs-video>

![The default Hew window: menu bar, tool rail on the left, 3D viewport in the center, panels tray on the right, contextual dock at the bottom, status bar along the base](/docs/ui-default.webp)

## The viewport

The center of the window is the 3D viewport. A ground grid sits at height zero, and the three world axes cross at the origin: **X is red, Y is green, Z is blue**. Each axis is drawn solid on its positive side and dashed on its negative side. You can hide the axes with **View ▸ Axes** and the grid with **View ▸ Grid**.

A small **view die** sits in the top-right corner of the viewport. It's a cube with its corners cut flat, and it turns with the camera, so it always shows which way you're looking. Click a face (Top, Front, Left, and so on) to look straight at it, click a corner to look at the model from that corner, or drag the die to orbit. Under the die are four small squares called **pips**: red, green, blue, and violet. They show which drawing plane your next click lands on and let you lock it. The Drawing chapter covers them in full. The die appears in the editor only; Shop mode keeps its own Views sheet.

A floating readout in the bottom-right corner shows live measurements — distance, angle, scale factor — while a tool is **mid-gesture**: partway through an operation, after the first click but before the commit. You'll meet that term throughout this manual. Anything you type also appears in this readout.

## The tool rail

The left rail lists the everyday tools in three groups, each with its keyboard shortcut:

- **Draw** — Select `Space`, Line `L`, Rectangle `R`, Circle `C`, Polygon, Arc `A`
- **Modify** — Push/Pull `P`, Follow Me, Offset `F`, Move `M`, Rotate `Q`, Scale `S`
- **Inspect** — Tape Measure `T`, Dimension, Paint `B`, Section Plane

More tools don't have rail slots: **Protractor**, **Slice**, **Edit Vertex**, **Text**, **Drawing Axes**, and **Position Texture**, plus the camera tools **Orbit** `O`, **Pan** `H`, **Zoom** `Z`, **Zoom Window**, **Position Camera**, **Look Around**, and **Walk**. Reach them from the **Tools** and **Camera** menus, or from the command palette.

Once the shortcuts are second nature, the names can go. Click the chevron at the top of the rail, or choose **View ▸ Compact Tool Rail**, and the rail shrinks to a column of icons. Hover an icon to see its name and key; the search field becomes a magnifier that opens the same command palette. Click the chevron again to bring the names back.

## The command palette

The search field at the top of the tool rail opens the command palette. Or press `⌘/` on a Mac and `Ctrl+/` everywhere else — the same key in the desktop app and the browser. Type a few letters of anything: every tool, every menu action, and the objects, groups, and tags in your current model are all searchable. Synonyms work too — typing "extrude" finds Push/Pull, "slicer" finds Export.

![The command palette open with a query, showing a matched tool with its shortcut](/docs/command-palette.webp)

Press `↑`/`↓` to navigate, `Enter` to run, `Esc` to close. With an empty query the palette shows your recently used commands.

Commands that need a particular selection — the booleans, Group, Make Component, and friends — are always listed but shown grayed out with a "needs a selection" note until the selection qualifies, the same way the Object menu grays them.

## The contextual dock

The floating bar at the bottom-center of the viewport follows your selection and offers the most likely next actions:

| Selection | Dock shows |
|---|---|
| Nothing | Rectangle, Line, Circle, Arc |
| An Object | Push/Pull, Move, Paint, Make Component, Erase |
| A group | Edit, Move, Scale, Make Component, Ungroup, Erase |
| A component instance | Edit, Move, Scale, Make Unique, Explode |
| Several things | Move, Group, Erase |
| A sketch | Push/Pull, Move, Rotate, Scale, Erase |

Group and Make Component appear only when the selection qualifies (Group needs two or more siblings; Make Component takes objects and groups, not instances). Everything on the dock also lives in the menus (Edit is a double-click on the group or component); nothing is reachable only from it.

If you'd rather not have it, turn it off with **View ▸ Contextual Dock**. It stays off, across restarts too, until you turn it back on there.

## The panels tray

The right-hand tray holds a set of collapsible panels. Click a panel's header to expand or collapse it; drag the tray's left edge to resize it. A long panel can also be given more or less height: drag the handle at the bottom of its section, or focus the handle and press the Up or Down arrow key. A section keeps its automatic height until you resize it.

When you want the whole window for the model, hide the tray: click the chevron at its top-left, or choose **View ▸ Tray**. A small tab on the viewport's right edge brings it back at the width you left it. Asking for any one panel while the tray is hidden (from the View menu, or with that panel's shortcut) brings the tray back with that panel open. Hew remembers both the compact rail and the hidden tray: each open window keeps its own layout, and a new window, or the next launch, starts the way you last left them.

Each panel can also be shown or hidden from the **View** menu:

- **Object Info** (View ▸ Object Info, `⇧⌘O` / `Ctrl+Shift+O`) shows the selected item's name, type, solid status, bounding-box dimensions, tags, and materials. This is where you rename things and tag them.
- **Outliner** (View ▸ Model Info, `⇧⌘I` / `Ctrl+Shift+I`) is the document tree: every object, group, component instance, and sketch, with per-item visibility toggles.
- **Materials** (View ▸ Materials, `⇧⌘C` / `Ctrl+Shift+C`) holds the document's material palette, listed by name — rename and delete a material from here too.
- **Components** (View ▸ Components, `⇧⌘M` / `Ctrl+Shift+M`) lists every component definition by name with its live instance count; rename, delete, and Purge Unused (clear out whatever nothing references) live here.
- **Tags** (View ▸ Tags, `⇧⌘T` / `Ctrl+Shift+T`) shows the tag tree, with visibility toggles per tag.
- **Scenes** (View ▸ Scenes) lists the document's saved camera/visibility snapshots — see [Scenes](/learn/scenes/).
- **Changes** (View ▸ Changes) lists this session's undo history with a marker at your last save — see [Files, saving, and recovery](/learn/files-and-saving/#changes-panel).

On the desktop app, if you have more than one document window open, the Window menu also lists them by name at the bottom, with a checkmark beside whichever one you're in — click another to bring it to the front.

These panels are covered in depth in [Organizing your model](/learn/organizing/), [Groups and components](/learn/groups-and-components/), and [Materials](/learn/materials/).

## The status bar

The strip along the bottom always shows the active tool's name and a one-line hint about what it expects next. While a draw tool has Detached on (a tap of `Ctrl`, or `⌘` on a Mac; see [Detaching a shape](/learn/drawing/#detaching-a-shape)), the hint says so. If you're ever unsure what a tool wants, look here. On the right, the badge summarizes the whole model: "N objects ✓ solid", or "N objects · 3 not solid" when some aren't (click it to open the Model Health window, which lists them; see [Making imported shapes solid](/learn/import-export/#making-imported-shapes-solid)).

## Menus and title bar

The menu bar (system bar on macOS, in-app elsewhere) organizes everything: **File** (new/open/save/import/export/print), **Edit** (undo, delete, select), **Object** (group/component commands, booleans), **View** (display toggles, panels, Scenes, the command palette), **Camera**, **Draw**, **Tools**, **Window** (the Library and the open-windows list), and **Help** (the user guide, Report Bug…).

The window title shows the document name. A `•` in front of it (and an "Edited …" label next to it) means unsaved changes. There is no Save button — save with `⌘S` / `Ctrl+S`. [Files, saving, and recovery](/learn/files-and-saving/) covers how autosave has your back regardless.

## Light and dark

Hew follows your system appearance by default. To force light or dark, open **Settings ▸ Theme** (`⌘,` / `Ctrl+,`).

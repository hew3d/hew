---
title: "The Library"
description: "Save components, materials, and whole models to a personal library, then insert them into any document with a click."
order: 15
---

The Library holds reusable pieces that live outside any one document: components you want in every project, materials you keep reaching for, whole models worth keeping around. Save something once, insert it anywhere.

Open it with **Window ▸ Library**, the Library entry at the bottom of the tool rail, or `⇧L` (from a text field or the menus, `⇧⌘L` / `⇧Ctrl+L`). On the desktop it's a real window — resize it, put it on a second display, keep it open while you work. Items show as a thumbnail grid or a compact list; the toggle is in the header.

## What a library item is

Every item in the library is an ordinary `.hew` file in a folder you control — nothing is locked away in a private database. The default folder is `Hew Library` in your home directory; change it under **Settings ▸ Folders**. Because items are plain files, sharing your library between machines (or people) is as simple as syncing or sending the folder, and any item can be opened as a document in its own right.

Items come in three kinds, which the browser sorts into categories:

- **Components** — a part saved from a selection: a chair, a hinge, a bracket. Saved as a component definition, so inserting one places an instance.
- **Materials** — a single palette entry, texture and all.
- **Models** — a whole document: finished work, a downloaded and cleaned-up import, anything self-standing.

The distinction is curatorial, not mechanical. Any item can be **Inserted** into the current model or **Opened** as its own document — the category only decides which action comes first.

## Saving to the library

Select an object, group, or component instance and choose **Save to Library** from the action dock. You'll be asked for a name — that's all. Add keywords and collections later, in the library itself, where they're easier to think about. A saved item inserts by the bottom center of what you selected — the natural grab point for placing it on the ground or a face. For a different insertion point (a chair by its back-left foot, say), [place the drawing axes](/learn/moving-and-transforming/) there before saving; a deliberately placed axes origin always wins.

To save a whole document as a Model item, use **File ▸ Save to Library…**. To save a material, select its swatch in the Materials panel and use its **Save to Library** button.

A thumbnail renders in the background after each save; a small progress note shows and resolves on its own.

## Inserting

Click **Insert** on an item (or press `Enter` with it selected) and the browser closes: the item's geometry follows your cursor as a ghost, snapping with the full inference engine — endpoints, midpoints, faces, the ground — exactly like Move. A yellow crosshair marks the item's insertion origin. Click once to place it; `Esc` cancels. The whole insert is one undo step.

Inserting never links to the library file — it copies. Editing a library item later changes nothing in documents that already used it. Inserting the same component twice reuses the definition already in your document rather than creating "Chair (2)": both placements stay instances of one shared definition, exactly as if you'd copied the first one.

Materials insert differently: **Paint with this** copies the material into the document's palette and arms the Paint tool in one motion; **Add to palette** copies it without leaving the browser. A material already in your palette is detected and reused, never duplicated — the browser badges those "in palette".

You can also insert without opening the browser: type an item's name into the [command palette](/learn/interface/) (`⌘/` / `Ctrl+/`) and choose *Insert "…"* — it goes straight to cursor placement.

## Finding things

The browser searches names and keywords as you type. The scope chips narrow the grid to items already **in this model** (matched placements also get a blue dot) or **recently saved**. Collections in the sidebar are simple named shelves — assign an item to one from its detail pane.

The detail pane is also where you manage an item: rename it, add keywords, change its collection (collections nest with `/`, like `Hardware/Fasteners`, and you can create one right from the dropdown), re-render its thumbnail, reveal the file on disk, or delete it. Deleting an item removes the file — documents that already inserted it are unaffected, since inserts are copies.

Items remember which model they were saved from. If you're sharing an item and would rather not include that, **Remove Source Info** (in the item's menu or detail pane) scrubs it.

## Limits

- A model item's loose sketches and dimension annotations aren't carried along by an insert (its solids, components, materials, tags, and guides are). Hew tells you when it skips them.
- The library is a desktop feature; the web app doesn't have one.
- Inserting is refused, with an explanation, while a component is open for editing — close the session first.
- Inserting is refused if it would multiply a document's rendered component parts past one million. That's a guard against runaway or hostile files, not a ceiling a real model approaches — a full theater model expands to a few thousand parts.

---
title: "The Library"
description: "Save components, materials, and whole models to a personal library, then insert them into any document with a click."
order: 15
---

The Library holds reusable pieces that live outside any one document: components you want in every project, materials you keep reaching for, whole models worth keeping around. Save something once, insert it anywhere.

Open it with **Window ▸ Library**, the Library entry at the bottom of the tool rail, or `⇧L` (from a text field or the menus, `⇧⌘L` / `⇧Ctrl+L`). On the desktop it's a real window — resize it, put it on a second display, keep it open while you work. Items show as a thumbnail grid or a compact list; the toggle is in the header.

## What a library item is

Every item in the library is an ordinary `.hew` file — nothing is locked away in a private database. Because items are plain files, sharing your library between machines (or people) is as simple as syncing or sending them, and any item can be opened as a document in its own right.

## Where items live

**On the desktop**, the library is a folder on disk — `Hew Library` in your home directory by default; change it under **Settings ▸ Folders**. Put that folder inside a synced one (Dropbox, Google Drive, iCloud Drive, Syncthing) and your library follows you to every machine.

**In the browser**, items are kept by the browser itself, on this device — private to this browser profile, and gone if you clear the site's data. The item detail pane's **Download…** hands any item back as a plain `.hew` file whenever you want one out.

**In a browser with the folder-picker API** (Chrome and Edge; Brave ships it turned off, but `brave://flags` → "File System Access API" enables it), you can do better: **Settings ▸ Folders ▸ Change…** binds the library to a real folder on disk. Items become ordinary files you can see and back up — the same layout the desktop app uses, so the two can share one folder — and a cloud-synced folder makes the browser library follow you too. The browser may re-ask permission for the folder after a restart; the Library window shows a **Reconnect** button when it does.

Items come in three kinds, which the browser sorts into categories:

- **Components** — a part saved from a selection: a chair, a hinge, a bracket. Saved as a component definition, so inserting one places an instance.
- **Materials** — a single palette entry, texture and all.
- **Models** — a whole document: finished work, a downloaded and cleaned-up import, anything self-standing.

The distinction is curatorial, not mechanical. Any item can be **Inserted** into the current model or **Opened** as its own document — the category only decides which action comes first.

## Saving to the library

Select an object, group, or component instance and choose **Save to Library** from the action dock (or **Object ▸ Save Selection to Library…**). You'll be asked for a name — that's all. Add keywords and collections later, in the library itself, where they're easier to think about. A saved item inserts by the bottom center of what you selected — the natural grab point for placing it on the ground or a face. For a different insertion point (a chair by its back-left foot, say), [place the drawing axes](/learn/moving-and-transforming/) there before saving; a deliberately placed axes origin always wins.

To save a whole document as a Model item, use **File ▸ Save to Library…**. To save a material, select its swatch in the Materials panel and use its **Save to Library** button.

A thumbnail renders in the background after each save; a small progress note shows and resolves on its own.

## Inserting

Click **Insert** on an item (or press `Enter` with it selected) and the browser closes: the item's geometry follows your cursor as a ghost, snapping with the full inference engine — endpoints, midpoints, faces, the ground — exactly like Move. A yellow crosshair marks the item's insertion origin. Click once to place it; `Esc` cancels. The whole insert is one undo step.

Inserting copies the item into your model, and the model never changes by itself when the library does; [keeping parts up to date](#keeping-parts-up-to-date) below covers how Hew tells you, and how you choose. Inserting the same component twice reuses the definition already in your document rather than creating "Chair (2)": both placements stay instances of one shared definition, exactly as if you'd copied the first one.

Materials insert differently: **Paint with this** copies the material into the document's palette and arms the Paint tool in one motion; **Add to palette** copies it without leaving the browser. A material already in your palette is detected and reused, never duplicated — the browser badges those "in palette".

You can also insert without opening the browser: type an item's name into the [command palette](/learn/interface/) (`⌘/` / `Ctrl+/`) and choose *Insert "…"* — it goes straight to cursor placement.

## Keeping parts up to date

A part you inserted remembers the item it came from. When someone changes that item in the Library (you, in another model, or a teammate through a synced folder), Hew notices the next time you open the model, when the Library changes, and when you open the Library or the Components panel. Nothing changes until you say so.

When the Library has a newer version of parts in your model, a bar appears under the menu bar: **"2 parts have newer versions in the Library."** with **Update All**, **Review…**, and **Later**. **Later** hides the bar for this document until the Library holds something newer still. Renaming an item, or re-saving it without changing it, is not an update and never raises the bar.

**Review…** lists every part that differs from the Library, with what happened to it in plain words and how many copies your model has. Parts the Library changed start checked. A part that changed both in your model and in the Library starts unchecked, with a note that your changes there would be replaced, and so does a part Hew can't tell the direction of ("Different from the Library"). A locked part is shown but can't be checked; unlock it first. **Update Selected** brings in what is checked; **Update all that changed in the Library** brings in everything the Library changed.

An update replaces what the part is, in every copy, and each copy stays exactly where it was. It is one undo step, however many parts it covers, so **Undo** puts everything back at once. A part Hew can't update, because it is locked, open for editing, or the new version would contain itself, is skipped and named afterward; the rest go through. Close any open group or component first if a part says it is open for editing. A smaller part inside the one you update that you changed in this model, or that is locked, is left exactly as it is: the updated part gets the Library's version of it as a new component instead.

A small arrow beside a part in the Components panel and the Outliner shows where it stands: the Library has a newer version, you changed it here, or both. Hover it for the words. A part only changed in your model never raises the bar; it just gets the marker.

To send your own change the other way, select the part, choose **Save to Library**, and pick **Update "…" in the Library**. It replaces that item, keeping the name, keywords, and collection the Library has now, and other models that use it will be offered the update. If the Library item also changed since you inserted the part, or Hew can't tell which one is newer, it asks before replacing it. The option appears for the part you inserted or saved as a whole, not for a smaller part nested inside it, and not for a linked copy (make it a component or unlink it first). The previous version of the item is not kept. **Save as a new item** is right beneath it when you'd rather keep both.

If you'd rather Hew didn't look, turn off **Settings ▸ Folders ▸ Check the Library for newer parts**. An item that has been deleted from the Library says nothing; the part keeps working.

## Finding things

The browser searches names and keywords as you type. The scope chips narrow the grid to items already **in this model** (matched placements also get a blue dot) or **recently saved**. Collections in the sidebar are simple named shelves — assign an item to one from its detail pane.

The detail pane is also where you manage an item: rename it, add keywords, change its collection (collections nest with `/`, like `Hardware/Fasteners`, and you can create one right from the dropdown), re-render its thumbnail, reveal the file on disk, or delete it. Deleting an item removes the file — documents that already inserted it are unaffected; their parts keep working.

Items remember which model they were saved from. If you're sharing an item and would rather not include that, **Remove Source Info** (in the item's menu or detail pane) scrubs it.

## Limits

- A model item's loose sketches and dimension annotations aren't carried along by an insert (its solids, components, materials, tags, and guides are). Hew tells you when it skips them.
- Inserting or updating a part is refused, with an explanation, while a component is open for editing — close the session first.
- A part nested inside a larger item follows the larger item. If a table item holds a leg that is also an item of its own, updating the leg in the Library doesn't change the legs inside tables you already inserted; updating the table does.
- Inserting is refused if it would multiply a document's rendered component parts past one million. That's a guard against runaway or hostile files, not a ceiling a real model approaches — a full theater model expands to a few thousand parts.

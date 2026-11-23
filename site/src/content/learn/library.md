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

A part you inserted remembers the item it came from. When someone changes that item in the Library (you, in another model, a teammate through a synced folder, or an update from a [source](#sources)), Hew notices the next time you open the model, when the Library changes, and when you open the Library or the Components panel. Nothing changes until you say so.

When the Library has a newer version of parts in your model, a bar appears under the menu bar: **"2 parts have newer versions in the Library."** with **Update All**, **Review…**, and **Later**. **Later** hides the bar for this document until the Library holds something newer still. Renaming an item, or re-saving it without changing it, is not an update and never raises the bar.

**Review…** lists every part that differs from the Library, with what happened to it in plain words and how many copies your model has. Parts the Library changed start checked. A part that changed both in your model and in the Library starts unchecked, with a note that your changes there would be replaced, and so does a part Hew can't tell the direction of ("Different from the Library"). A locked part is shown but can't be checked; unlock it first. **Update Selected** brings in what is checked; **Update all that changed in the Library** brings in everything the Library changed.

An update replaces what the part is, in every copy, and each copy stays exactly where it was. It is one undo step, however many parts it covers, so **Undo** puts everything back at once. A part Hew can't update, because it is locked, open for editing, or the new version would contain itself, is skipped and named afterward; the rest go through. Close any open group or component first if a part says it is open for editing. A smaller part inside the one you update that you changed in this model, or that is locked, is left exactly as it is: the updated part gets the Library's version of it as a new component instead.

A small arrow beside a part in the Components panel and the Outliner shows where it stands: the Library has a newer version, you changed it here, or both. Hover it for the words. A part only changed in your model never raises the bar; it just gets the marker. The markers follow your edits as you make them, even while you are inside the part, and the Library shows the same marks: a tile gets the arrow, and the item's detail pane says in words where the part stands, with **Review…** when there is something to bring in.

To send your own change the other way, select the part, choose **Save to Library**, and pick **Update "…" in the Library**. It replaces that item, keeping the name, keywords, and collection the Library has now, and other models that use it will be offered the update. If the Library item also changed since you inserted the part, or Hew can't tell which one is newer, it asks before replacing it. The option appears for the part you inserted or saved as a whole, not for a smaller part nested inside it, and not for a linked copy (make it a component or unlink it first). The previous version of the item is not kept. **Save as a new item** is right beneath it when you'd rather keep both.

If you'd rather Hew didn't look, turn off **Settings ▸ Folders ▸ Check the Library for newer parts**. An item that has been deleted from the Library says nothing; the part keeps working.

## Editing an item

To change an item itself, select it in the Library and choose **Edit** (also in the item's menu). Hew opens the Library's own file in a window. The title reads **Leg (Library item)**, and a message says that saving updates the Library. A component opens as the part's definition: to change its geometry, select the part in the Components panel and press **Edit** in the dock, as you would for any component. Press `Cmd/Ctrl+S` and the item in the Library is replaced (keeping the name, keywords, and collection the Library has at that moment), its thumbnail is redrawn, and every other model that uses it is offered the update, as above. Nothing in those models changes until their owners say so.

On the desktop, Edit uses this window when it is empty, and a new window when it holds work, so your model stays open next to the part. On the web the item opens in this tab when it is empty and in a new tab when it holds work. If the item is already open for editing in another desktop window, that window comes to the front. On the web, a second tab for an item that is already open says so and opens nothing; switch to the first tab instead. Opening a file from inside the Library folder any other way, such as double-clicking it in Finder, edits it the same way. **Save As** makes a separate copy and ends the link to the item.

**Open as document** is still there when you want to start a new model from an item: it makes a separate copy, and saving it never touches the Library. Materials have no Edit; change one in a model, then use **Save to Library**. Profiles keep **Open for editing**.

## Finding things

The browser searches names and keywords as you type. The scope chips narrow the grid to items already **in this model** (matched placements also get a blue dot) or **recently saved**. Collections in the sidebar are simple named shelves — assign an item to one from its detail pane.

The detail pane is also where you manage an item: rename it, add keywords, change its collection (collections nest with `/`, like `Hardware/Fasteners`, and you can create one right from the dropdown), re-render its thumbnail, reveal the file on disk, or delete it. Deleting an item removes the file — documents that already inserted it are unaffected; their parts keep working.

Items remember which model they were saved from. If you're sharing an item and would rather not include that, **Remove Source Info** (in the item's menu or detail pane) scrubs it.

## Sources

A source is a set of Library items that someone has published on the web: on GitHub, GitLab, Codeberg, or any website. You look through a source inside the Library, add the items you want, and Hew tells you when the source has something newer. The Hew project keeps a catalog of sources and stores none of their items. Those stay on the publisher's own site.

The Library sidebar has a **Sources** block under your collections. It lists the sources you have added, then **Browse the catalog** and **Add a source…**. While that list is empty, Hew contacts no other site.

### Adding a source

**Browse the catalog** shows the sources the Hew project lists, each with its name, author, license, and how many items it holds. Click a source to look through it first; **Add** puts it in your sidebar.

A source does not have to be in the catalog. Choose **Add a source…**, paste its address, and press **Look up**. The address can be the page of a project on GitHub, GitLab, or Codeberg, the address of a folder on any `https` website, or a short form the publisher gives you, such as `github:owner/name`. Hew shows the source's name, author, host, license, and item count, and **Add** puts it in your list. For a source outside the catalog the dialog also says **"This source is not in the Hew catalog. Hew has not checked it."** Nobody has looked at such a source, so add it when you trust whoever published it.

Hew reads a source without signing in anywhere, so a source that needs a login can't be added.

### Taking items

Click a source in the sidebar to see its items as a grid or a list. The search box searches that one source. Each item says where it stands: **Not in your Library**, **In your Library**, or **Newer at the source**. Select an item and its detail pane shows the license before you take anything.

**Add to my Library** copies the item into your Library, filed in a collection named after the source. From then on it is an item like any other: insert it, edit it, move it to another collection. Its detail pane gains a line that reads **From Shop Hardware · CC-BY-4.0**, with the source's own name and license. On a component or a model, **Insert** adds the item if you don't have it yet and goes straight to placing it.

Sometimes the item is already in your Library and didn't come from a source, because someone sent you the file, for instance. **Add to my Library** then asks **"Replace the item in your Library?"** **Yes** swaps yours for the source's copy and ties it to the source. An item you already took from a different source can't be added from this one.

**Add all** takes every item at once. It shows its progress, and **Cancel** keeps what has already arrived. At the end it reports a count such as **"11 added, 1 failed"**, with **Retry** for the ones that failed.

**Follow** does what **Add all** does and then keeps watching: after each check, Hew offers the items the source has added since. An item you delete from your Library is not offered again. Click **Following** to stop.

### Updates from a source

When you open the Library, Hew looks at each source you added, at most once every six hours. **Refresh** in a source looks now. When a source has something newer, a bar appears at the top of the Library: **"3 items have newer versions at their sources."**, or for a source you follow, **"Shop Hardware has 2 new and 1 changed items"**. Its buttons are **Update all**, **Review…**, and **Later**. Nothing in your Library changes until you choose.

An update replaces your item with the source's version. It takes the source's name and keywords and keeps the collection you filed it in. Models that already use the part are then offered the new version the usual way, described in [keeping parts up to date](#keeping-parts-up-to-date).

An item you changed yourself, by editing it or by updating it from a model, is marked **changed here**, and **Update all** leaves it alone. **Review…** lists it with two choices. **Replace your changes** takes the source's version. **Keep mine** keeps yours, and Hew stops counting that version; it asks again when the source changes the item another time. Renaming an item or moving it to another collection doesn't count as a change, so a renamed item takes the source's name back when it updates. Review shows the name it will get.

To update one item, use **Update from source** in its menu or detail pane. **Unlink from Shop Hardware** (the menu names the source) cuts the tie instead: the item stays in your Library and gets no more updates.

If you'd rather Hew didn't look when the Library opens, turn off **Settings ▸ Folders ▸ Check sources for new and changed items**. Hew then checks a source only when you press **Refresh**.

### When a source changes or goes away

Your items are files in your own Library, and nothing a source does takes them away.

- **The source can't be reached.** Its row shows a warning mark, and its last list stays on screen with the time of the last good check.
- **The source dropped an item.** The item's detail pane says **"The source no longer has this item."**
- **The Hew catalog removed the source.** The source shows the reason. Adding and updating from it stop.
- **You chose Remove source.** It leaves your sidebar. Items you took from it say **"This source is not in your list. Add it again to get updates."**

The list of your sources is kept in the Library folder, so a Library in a synced folder shows the same sources on every machine.

### Desktop and browser

The desktop app can read any source. In the browser, Hew can read only sites that let web apps read them. When a site doesn't, Hew says **"Hew could not read this site from the browser. The site may be down, or it may not allow web apps to read it. The desktop app can read it either way."**

Whoever runs a self-hosted copy of Hew can turn sources off. The Sources block then reads **"Sources are turned off on this server."** and the rest of the Library works as before.

### What Hew checks, and what it doesn't

Hew checks that every item arrives exactly as the source lists it, which catches a damaged or half-changed file. It doesn't vouch for the publisher. A person reviews each source before it goes in the catalog; a source you add by address has had no review. An item is a model file, and Hew reads it the way it reads any file you open.

One item can be up to 64 MB, and one source can hold up to 20,000 items.

To share your own items this way, see [Publishing a source](/learn/publishing-a-source/).

## Limits

- A model item's loose sketches and dimension annotations aren't carried along by an insert (its solids, components, materials, tags, and guides are). Hew tells you when it skips them.
- Inserting or updating a part is refused, with an explanation, while a component is open for editing — close the session first.
- A part nested inside a larger item follows the larger item. If a table item holds a leg that is also an item of its own, updating the leg in the Library doesn't change the legs inside tables you already inserted; updating the table does.
- Inserting is refused if it would multiply a document's rendered component parts past one million. That's a guard against runaway or hostile files, not a ceiling a real model approaches — a full theater model expands to a few thousand parts.

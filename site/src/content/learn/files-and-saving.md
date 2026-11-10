---
title: "Files, saving, and recovery"
description: "Where your work lives: the open .hew format, opening and saving SketchUp files, saving in the browser vs. the desktop, and the 12-second autosave net."
order: 18
---

Hew's native format is `.hew`, an open, documented container (a zip holding a JSON manifest and binary geometry buffers) that saves everything: geometry, names, groups, components, materials, tags, guides, and your current view (camera projection, field of view, and viewpoint). Saving is deterministic: the same model always produces byte-identical output, which plays well with backups and version control.

## Saving and opening

- **Save**: `⌘S` / `Ctrl+S`, or File ▸ Save. The first save is a Save As: it asks where and in which format. After that it writes in place, in the format the file is already in.
- **Save As…** (`⇧⌘S` / `Ctrl+Shift+S`) shows the system save dialog with a **Format** list in it, the way every desktop app's does, and switches the document to the new file. In the browser, whose save dialog has no such list, Hew asks for the format first. See [SketchUp files](#sketchup-files) below.
- **Revert to Saved** (File menu, right after Save As) throws away everything you changed since the last save and reads the file back in. It asks once first, and Cancel is the default. It is available only when the document has a file and unsaved changes, and it clears Undo and Redo, so it can't be undone. Your window, camera, and panels stay where they are.
- **Open…**: `⌘O` / `Ctrl+O`. On desktop, this opens into the current window only if it's a brand-new, untouched "Untitled" document; otherwise it opens the picked file into a fresh window, leaving whatever you were doing untouched. Opening a file that's already open in another window brings that window forward — one file is never open in two windows at once. The web app, which can't open extra windows, offers to discard instead.
- **New**: `⌘N` / `Ctrl+N`. Same rule as Open: a non-pristine document opens a fresh window on desktop; the web app offers to discard.
- **Close** (`⌘W`, or File ▸ Close) closes the window (macOS desktop only).

The window title is the save-state indicator: a `•` before the name plus an "Edited …" label means unsaved changes; "Saved …" means you're clean. That dirty mark tracks the same undo history you can step through: undo back to exactly where you last saved and the mark clears on its own, no re-save needed — SketchUp and every other undo-aware editor works the same way.

Before anything would discard unsaved work — closing the window or tab, opening another file, or starting a new document — Hew shows what's about to be lost: the changes made since your last save, and **Save**, **Don't Save**, and **Cancel** to choose from. Save runs the normal save flow and, once it lands, continues whatever you were doing (opening the other file, starting the new document, or closing the window).

## SketchUp files

A SketchUp `.skp` file opens as a document, not only as an import. File ▸ Open takes a `.skp` of any SketchUp version, from SketchUp 3 through 2027 (the format that SketchUp 2021 and later save, or a release from 3 to 8 or 2013 to 2020) the same way it takes a `.hew`: the title shows `Model.skp`, the document starts clean, and Save writes the file back in the format it was opened in. Open Recent, drag-and-drop, and the desktop app's Open With menu all work the same way. File ▸ Import… is separate and unchanged: it adds a SketchUp file's contents to the current document and does not bind the document to that file.

**Save As…** offers sixteen formats:

- **Hew document (.hew)**, the native format.
- **SketchUp 2021 and later (.skp)**, the format SketchUp 2021 through 2027 save.
- **SketchUp 2020 (.skp)** down to **SketchUp 2013 (.skp)**, and **SketchUp 8 (.skp)** down to **SketchUp 3 (.skp)**, each the format of that release, which opens in it and every later release.

A file opened from a stream release saves back in its own release. A file from SketchUp 2021 to 2027 shares one container across those releases: it opens, and saves back in that container, and an option writes it so that every SketchUp since 2021 can open it. Between them, Open and Save As cover every SketchUp version from 3 through 2027.

The list starts on the document's current format (Hew for a new document). Changing it changes the name's extension to match, so `Model.skp` becomes `Model.hew`, never `Model.skp.hew`.

A SketchUp file cannot hold everything a Hew document can. The first time a Save or Save As to `.skp` leaves something out or simplifies it (for example, curved surfaces are written as smooth facets), Hew shows a notice listing what changed. It appears once per document, does not block anything, and has a **Save a .hew copy** button that writes a `.hew` through the Save As dialog without switching the document away from its `.skp`. Later saves of that document are silent. File ▸ Export… can also write both `.skp` flavors as copies; see [Import and export](/learn/import-export/).

## In the browser

The web app uses your browser's file access support:

- **Chrome, Edge, and other Chromium browsers** can open and save `.hew` and `.skp` files in place, like a desktop app (you'll grant permission the first time).
- **Firefox and Safari** fall back to standard uploads/downloads: Open picks a file, and each Save downloads a fresh copy to your Downloads folder.

The web app works offline once loaded — it's an installable PWA, so you can add it to your dock or home screen.

On a phone, opening the web app lands you in [Shop Mode](/learn/hew-on-your-phone/), a separate touch-first viewer built for taking a finished model to the bench rather than editing it there.

## On the desktop

The desktop app (macOS, Windows, Linux) uses native open/save dialogs, remembers your last **10 recent files** (File ▸ Open Recent), and registers the `.hew` file type so double-clicking a file opens it. On macOS, Hew also appears under Open With for `.skp` files; it does not take over double-clicking from SketchUp. File ▸ New opens additional windows, each with an independent document.

**File ▸ Open on Phone…** hands the open document to a phone on the same wifi network as a QR code. See [Hew on your phone](/learn/hew-on-your-phone/#getting-a-model-onto-your-phone) for the full handoff.

## Autosave and crash recovery

Hew autosaves a recovery snapshot of any unsaved changes every **12 seconds**. If the app or browser closes unexpectedly, the next launch offers the snapshot back:

> **Recover Unsaved Document?** — Hew found an autosaved version of "…" from a few minutes ago that wasn't saved before the app closed.

Choose **Recover** to pick up where you left off, or **Discard** to throw the snapshot away. Pressing `Esc` dismisses the dialog *without* discarding — the offer returns next launch, so an accidental keypress can't destroy work.

A recovered `.skp` document comes back still bound to its file, so Save writes it back in the same format.

On the desktop, each window keeps its own snapshot. If you crash with several windows of unsaved work, the dialog lists every document; **Recover All** brings each one back in its own window, and **Discard All** clears them all.

Recovery snapshots are an emergency net, not a save system. Save normally, and treat the snapshot as a fallback for when something went wrong.

## Undo history

Undo (`⌘Z` / `Ctrl+Z`) and redo (`⇧⌘Z` / `Ctrl+Shift+Z`) span the whole document — every modeling operation, transform, paint, rename, and delete. History lives for the session; it isn't stored in the file. The Edit menu names the specific step you're about to undo or redo ("Undo Push/Pull," "Redo Move"), not just the bare verb.

Your view is the one exception to both rules above. It's saved with the file, so reopening a model brings back the last view you had it framed in, but changing it is never an undo step — panning, orbiting, or walking around isn't something you'd want an accidental `Ctrl+Z` to reverse.

Opening and closing a group or component for editing is on that same undo list — `Ctrl+Z` right after double-clicking in really does back out of the edit — but it isn't a *change*: it never marks the document dirty, and it's left off the Changes panel below, since nothing about the model actually moved.

## Changes panel

The **Changes** section of the right-hand tray (View ▸ Changes) lists this session's history in order, with a **Saved** marker showing exactly where your last save landed. Anything past the marker is unsaved; anything you've undone past it is dimmed and struck through, since redoing it would bring it back. A handful of edits don't live on that undo list at all — editing Scenes, and adding materials or components from the Library — so the panel and the discard-warning dialog both call those out by name rather than leaving you to guess why the document is still marked dirty.

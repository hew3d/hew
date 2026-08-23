---
title: "Files, saving, and recovery"
description: "Where your work lives: the open .hew format, saving in the browser vs. the desktop, and the 12-second autosave net."
order: 18
---

Hew's native format is `.hew`, an open, documented container (a zip holding a JSON manifest and binary geometry buffers) that saves everything: geometry, names, groups, components, materials, tags, guides, and your current view (camera projection, field of view, and viewpoint). Saving is deterministic: the same model always produces byte-identical output, which plays well with backups and version control.

## Saving and opening

- **Save**: `⌘S` / `Ctrl+S`, or File ▸ Save. The first save asks where; after that it writes in place.
- **Save As…** (`⇧⌘S` / `Ctrl+Shift+S`) saves a copy under a new name and switches to it.
- **Open…**: `⌘O` / `Ctrl+O`. On desktop, this opens into the current window only if it's a brand-new, untouched "Untitled" document; otherwise it opens the picked file into a fresh window, leaving whatever you were doing untouched. Opening a file that's already open in another window brings that window forward — one file is never open in two windows at once. The web app, which can't open extra windows, offers to discard instead.
- **New**: `⌘N` / `Ctrl+N`. Same rule as Open: a non-pristine document opens a fresh window on desktop; the web app offers to discard.
- **Close** (`⌘W`, or File ▸ Close) closes the window (macOS desktop only).

The window title is the save-state indicator: a `•` before the name plus an "Edited …" label means unsaved changes; "Saved …" means you're clean. Hew warns you before anything would discard unsaved work — closing the window or tab, opening another file, or starting a new document.

## In the browser

The web app uses your browser's file access support:

- **Chrome, Edge, and other Chromium browsers** can open and save `.hew` files in place, like a desktop app (you'll grant permission the first time).
- **Firefox and Safari** fall back to standard uploads/downloads: Open picks a file, and each Save downloads a fresh copy to your Downloads folder.

The web app works offline once loaded — it's an installable PWA, so you can add it to your dock or home screen.

On a phone, opening the web app lands you in [Shop Mode](/learn/hew-on-your-phone/), a separate touch-first viewer built for taking a finished model to the bench rather than editing it there.

## On the desktop

The desktop app (macOS, Windows, Linux) uses native open/save dialogs, remembers your last **10 recent files** (File ▸ Open Recent), and registers the `.hew` file type so double-clicking a file opens it. File ▸ New opens additional windows, each with an independent document.

**File ▸ Open on Phone…** hands the open document to a phone on the same wifi network as a QR code. See [Hew on your phone](/learn/hew-on-your-phone/#getting-a-model-onto-your-phone) for the full handoff.

## Autosave and crash recovery

Hew autosaves a recovery snapshot of any unsaved changes every **12 seconds**. If the app or browser closes unexpectedly, the next launch offers the snapshot back:

> **Recover Unsaved Document?** — Hew found an autosaved version of "…" from a few minutes ago that wasn't saved before the app closed.

Choose **Recover** to pick up where you left off, or **Discard** to throw the snapshot away. Pressing `Esc` dismisses the dialog *without* discarding — the offer returns next launch, so an accidental keypress can't destroy work.

On the desktop, each window keeps its own snapshot. If you crash with several windows of unsaved work, the dialog lists every document; **Recover All** brings each one back in its own window, and **Discard All** clears them all.

Recovery snapshots are an emergency net, not a save system. Save normally, and treat the snapshot as a fallback for when something went wrong.

## Undo history

Undo (`⌘Z` / `Ctrl+Z`) and redo (`⇧⌘Z` / `Ctrl+Shift+Z`) span the whole document — every modeling operation, transform, paint, rename, and delete. History lives for the session; it isn't stored in the file.

Your view is the one exception to both rules above. It's saved with the file, so reopening a model brings back the last view you had it framed in, but changing it is never an undo step — panning, orbiting, or walking around isn't something you'd want an accidental `Ctrl+Z` to reverse.

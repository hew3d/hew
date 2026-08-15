---
title: "Hew on your phone"
description: "Shop Mode turns a phone into a touch viewer for a model you built at the desk: tap a part for its dimensions, check a cut length, work offline at the bench."
order: 19
---

Hew is a desktop and web modeler, but a model is often most useful away from the desk — at the bench, next to the material, checking a dimension before a cut. Shop Mode is a fullscreen, touch-first viewer for exactly that: open a model on your phone, orbit it, tap a part for its name and size, tap an edge for its length, and work through the whole thing with no wifi if the shop doesn't have any.

Shop Mode never edits. It issues no changes to the document — every part list, hidden part, or isolated view you set up on the phone is view-state that disappears when you close the tab. To actually change the model, you're back on the desktop.

## When Shop Mode activates

Open **app.hew3d.com** on a phone and Hew checks two things: whether the pointer is coarse (touch, not a mouse) and whether the smaller of the screen's two dimensions is under 600 CSS pixels — phone-sized, in portrait or landscape. Meeting both boots straight into Shop Mode; a tablet or a touch-capable laptop, which fails the size check, gets the full editor.

The heuristic is a default, not a lock, and it works in both directions:

- **From Shop Mode**, the overflow menu (**⋯**, top right) has **Use full editor**. It reloads into the regular desktop-style UI on the same device.
- **From the full editor**, on a touch device that didn't trip the automatic switch — a tablet, say — **Window ▸ Shop Mode** switches over by hand. (This item only appears on a touch-capable device running the web app — a mouse-driven session has no Shop Mode use case, and it's absent under the desktop app entirely, since Shop Mode is web-only.)

Either choice is remembered, so a phone you've pinned to the full editor — or a laptop you've pinned to Shop Mode for testing — stays that way across reloads until you switch back.

## Getting a model onto your phone

Three ways, depending on what's already open:

**Scan a QR code from the desktop.** With a model open in the desktop app, **File ▸ Open on Phone…** encrypts the document right there in the app, uploads the ciphertext to a one-shot relay, and shows a QR code.

![The Open on Phone dialog showing the QR code and handoff URL](/docs/open-on-phone-dialog.png)

The encryption key never leaves the QR code itself — the relay only ever handles opaque ciphertext, and it forgets it the moment your phone has fetched it once (or after ten minutes, whichever comes first). There's no account and nothing left behind on either machine afterward.

Scan it from **inside Shop Mode**: open Shop Mode on your phone, tap **From your desktop…** on the empty-state screen (or **Open from desktop…** in the **⋯** menu), and point the camera at the code — Shop Mode decodes it itself and loads the model directly, no trip through the Photos or Camera app. Scanning with your phone's regular camera app still works too, but it opens the code in Safari first, which then hands off into Shop Mode a step later — the in-app scanner is faster and stays in one place, so it's the one worth reaching for by default. Either way, a couple of things worth knowing:

- **Only an internet connection is required** — not the same wifi network the old LAN-based handoff needed. The desktop and the phone can be on different networks entirely, as long as both can reach the internet.
- The dialog works from any build of the desktop app, including running it from source with `tauri dev` — it no longer depends on the packaged app bundle the way the old LAN handoff did.

**Open from the Files app.** Any `.hew` file synced through iCloud Drive, Nextcloud, or another Files-integrated provider is now selectable from Shop Mode's **Open…** — the file picker used to grey out `.hew` files on iOS because it filtered by a file type the Files app didn't recognize; that filter is dropped on touch devices, so `.hew` shows up like any other file.

**Reopen from Recents.** Once a model has been opened on the phone once — by either route above — it's saved locally and shows up in a **Recents** list on Shop Mode's empty-state screen, no network needed at all.

## Working in Shop Mode

The viewport fills the whole screen. A thin strip along the top shows the document's name and the overflow menu; everything else — tool switching, part inspection, the parts list — lives in floating chips and a bottom sheet, reachable with a thumb.

**Navigation** is one-finger orbit by default, matching the touch gestures the desktop viewport already supports. A small chip cluster in the bottom-right switches between the three tools Shop Mode exposes: **Select**, **Orbit**, and **Tape Measure**.

**Tap to inspect.** Tap a part and a small card appears near your finger with its name, tag, and overall X/Y/Z dimensions. Tap an edge instead and the card shows just that edge's length — the same measurement the desktop's Tape Measure would give you, without switching tools first.

**Double-tap to zoom.** Double-tapping a part frames it in the viewport; double-tapping empty space zooms to fit the whole model (the same as the **Zoom Extents** chip in the bottom-left).

**Long-press to isolate.** Press and hold a part for about half a second and everything else hides, leaving just that part (and whatever it's grouped with) visible — useful for confirming exactly which piece you're holding against exactly which part of the model. A **Show all** chip appears in the bottom-left corner while isolated; tap it to bring everything back.

**The Parts sheet** is a bottom sheet you can drag by its handle through three heights — a peek showing just a part count, a half view, and a full list. It's built from the same Outliner and Tags data as the desktop, folded into one mobile list: every row shows a part's name and overall dimensions, with an eye icon to hide it; a tag section gets its own eye icon that hides everything under that tag at once. Tapping a row selects and zooms to that part, the same as tapping it in the viewport; long-pressing a row isolates it, the same as long-pressing in the viewport. Think of it as a live cutlist — drag it to half height, scroll through the parts, and hide the ones you've already cut so what's left to do stays visible.

**Tape Measure on touch.** Selecting the Tape Measure chip gives you the same click-an-edge, click-a-point measuring the desktop has, with the snap targets widened for a fingertip: touch devices get roughly double the pixel radius a mouse gets for picking up and holding onto an endpoint, midpoint, or edge, so a real fingertip doesn't have to land pixel-perfect to snap. [Precision, measurement, and guides](/learn/measurement-and-guides/) covers what the Tape Measure does in full; Shop Mode's copy behaves the same way, just at touch scale.

## Keeping models for offline use

Shop Mode is built to work at a bench with no signal.

- **Recents** are stored on the phone (up to 20 models, 50 MB total, oldest evicted first), so a model you've opened once is available again with no network at all.
- **Save a copy (.hew)**, in the overflow menu, downloads the exact bytes you opened straight to the Files app — a durable copy outside Recents' cap, for a model you want to keep around indefinitely.
- The app shell itself is a precached PWA: once you've loaded Shop Mode at least once, reopening it — and reopening anything already in Recents — works in airplane mode. Only fetching a brand-new model over QR or from a cloud-synced Files location needs a live connection.

**Add to Home Screen**, from your phone browser's share sheet, installs Shop Mode as a fullscreen app icon with no browser chrome around it — the same PWA install the desktop web app offers ([Files, saving, and recovery](/learn/files-and-saving/) covers the equivalent for a laptop).

## View in AR (iOS only)

On an iPhone or iPad running Safari, a **View in AR** chip appears next to Zoom Extents. Tap it and Hew exports the open document to USDZ on the spot and hands it to **AR Quick Look** — iOS's built-in AR viewer — which drops the model into the room in front of you at true, real-world scale: a 30 cm box shows up 30 cm across, not scaled to fit the screen.

The button only shows up in Safari itself — other iOS browsers (Chrome, Firefox, Edge for iOS) don't wire up the same system hand-off, even though they're built on the same engine — and it's absent entirely outside iOS. Everywhere else, the desktop's [Export dialog](/learn/import-export/#usdz--for-ar-quick-look-and-usd-pipelines) writes the same USDZ format to a file you can AirDrop or open from Files.

---
title: "Printing"
description: "Print a view on paper, print a drawing to exact scale across as many sheets as it takes, or save a PDF — one dialog for all three."
order: 20
---

**File ▸ Print…** (`⌘P` / `Ctrl+P`, near the bottom of the File menu — last on macOS, just above Exit elsewhere — also in the command palette) opens **Print Layout**, which puts the model on paper or into a PDF. It covers three different jobs: printing what's on screen for a quick reference, printing a drawing at an exact scale you can measure from with a ruler, and printing a part at 1:1 across several sheets to cut out and use as a template. One dialog, a mode switch at the top — **Standard** or **Scaled** — a live page preview on the left, controls on the right, and **Save PDF…** and **Print…** at the bottom. `Return` prints; `Escape` cancels.

![Print Layout in Scaled mode: the Café Table sample at 1:2 tiled across four Letter pages, with the preview on the left and the controls on the right](/docs/print-layout.webp)

## Print what you see (Standard)

Standard mode prints one page: whatever the viewport is currently framing, letterboxed onto the sheet. It works in perspective or parallel projection, and it's the fast path — open the dialog, glance at the preview, press **Print…**. **Zoom** decides the framing: **Current** prints exactly the viewport's zoom; **Fit** keeps the view direction but re-frames the visible model to fill the page right up to the margins.

Controls: **Zoom**, **Paper** (Letter, Legal, Tabloid, A5, A4, A3, or **Custom…** — type a width and height in your working units, anything from 50 mm to 1200 mm a side), **Orientation** (Auto picks landscape when the viewport — or, under Fit, the model — is wider than tall), **Margins** (Normal ½ in, Narrow ¼ in), **Style**, and **Include** (Dimensions & text, Guides, Grid & axes, Cut list page, Title block). If the document has [Scenes](/learn/scenes/), a **Pages** control appears — **Current view** or **Each Scene**.

The title block, when on, is a strip along the bottom: the document name over the view or Scene name at the left; page *x* of *y*, the date, and the Hew mark at the right. Click any page in the preview to see it at 100 % — real print size on the desktop, where Hew can measure the display; the browser's nominal 96 dpi on the web — and drag to pan around it; click again to go back.

## Scaled

Scaled mode prints a parallel-projection drawing at a ratio you choose — 1 cm = 1 m, ¼" = 1', 1:1, whatever the job needs. A perspective viewport is printed in parallel projection; only the print is — your camera stays exactly where you left it.

**View** picks the direction: **Current view** or one of the seven standard views (Top, Bottom, Front, Back, Left, Right, Iso).

**Scale** is a preset ladder for whichever unit family you're working in — metric runs 2:1 down to 1:200, imperial runs 2" = 1" down to 1/16" = 1' — each row spelling out both the ratio and its plain-language reading ("1:10 — 1 cm = 10 cm"), and a reading line under the control always says what the current scale means: "1 cm on paper = 10 cm in the model". Pick **Custom…** to type your own, either side ("1 in on paper = 2 ft in model"; "25mm", "1'", "3/8\"", and "2 ft" all parse; anything outside 1:1000 … 20:1 is refused). The **Fit** button sets the exact scale that fills one page to the margins — a custom scale, shown by its ratio ("1:8.84 (fit)"); pick a round scale from the ladder when the job needs one.

**Extent** decides what's drawn:

- **Model** — everything visible.
- **Selection** — only the selected objects; everything else is left off the page entirely, not just cropped out. Select a part, pick Top view, set 1"=1", and you have a 1:1 template of exactly that part.
- **Current view** — exactly what the viewport frames right now (its own zoom window), which needs the current view direction: choosing it sets **View** to Current view, and choosing a standard view falls back to Model.

**Paper**, **Orientation** (here, Auto means *fewer pages*, ties going to portrait), **Margins**, and **Style** work as in Standard. **Include** offers Dimensions & text, Guides, Hidden lines dashed (Line art only), and Cut list page; **On the page** groups the four things that live on the sheet itself — Overlap for gluing, Marks, Scale bar, Title block.

## Across many pages

A scale that doesn't fit one sheet tiles automatically — a grid of pages lettered by row and numbered by column: A1, A2, B1, B2, and so on, reading left to right, top to bottom; the preview shows the grid with each tile's id under it.

Three things make the tiles usable as a physical template:

- **Overlap for gluing** — a band along the right and bottom edge of each inner tile (10 mm with Normal margins, 5 mm with Narrow) so adjacent sheets can overlap when you glue them down instead of butting edge to edge. The band is reserved *inside* the printable area, so it always prints — nothing lands in the printer's unprintable border — and every tile steps by the same amount.
- **Marks** — corner crop marks at each tile's drawing rectangle (the bottom pair runs down into the ends of the title block, whose text steps aside for them), a dashed trim line with a small "✂ trim" caption wherever a sheet meets its neighbor, and arrows just inside the drawing edge naming the neighbor ("→ B3", "↓ C2"), so they survive trimming the band. The title block carries the tile id big enough to sort a pile by ("B2"), with "Page 5 of 6 · Tile B2" under it. A single scaled sheet gets corner crop marks only — a squareness check.
- **Reposition** — drag the model in the preview to slide it over the tile grid, so you can move a seam off an awkward spot — through the middle of a hole, say — before you print; a **Center** button appears while it's off-center. The arrow keys move it 1 mm at a time (`Shift` for 10 mm) once the preview has been clicked. The move is limited to the slack on the grid, so the page count never changes and no page is ever empty.

Trim on the dashed lines, overlap the marked bands, and glue up a multi-sheet template the same way you would a poster.

## Reading the scale bar and printing at 100 %

Every scaled page carries a **scale bar** in the title block — a graphic scale of four alternating black and white segments, ticked in round model units ("0 5 10 15 20 cm" at 1:5; "0 1 2 3 4 m" at 1:100), with its exact paper length beside it ("40 mm"). Printer drivers and "fit to page" settings can quietly rescale a page by a few percent; measure the bar with a ruler before you cut anything. If it isn't its stated length, something in the print path scaled the page.

Hew composes the pages at the exact paper size with zero margins; when the system print dialog offers to scale-to-fit, decline it and print at 100 % / Actual size, or the scale bar (and everything else on the page) will be off.

## Line art vs As shown

**As shown** prints the model roughly as it looks on screen, but always on white paper under fixed, light-theme lighting — a dark-theme session prints light regardless. It's a 300 dpi image.

**Line art** prints white faces and black edges, no textures or shading — what you'd measure or cut from; the silhouettes of curved walls (a leg's sides, a rim) are inked too, not just the hard edges. In Scaled mode, Line art is drawn as true vector line work: visible edges, silhouettes, section-cut outlines, and (with **Hidden lines dashed** on) the edges a solid hides behind another. Vector pages stay crisp at any zoom and produce a tiny PDF even across dozens of tiles. If a model is too complex for the vector pass, Hew falls back to a bitmap automatically and says so in the dialog.

Standard mode defaults to As shown; Scaled defaults to Line art — switch either way from the **Style** control.

## Save PDF… vs Print…

**Print…** renders the pages and hands them to your operating system's own print dialog — the macOS sheet (with its PDF menu), the Windows system dialog (including Microsoft Print to PDF), the Linux GTK dialog (with Print to File), or the browser's print dialog (with Save as PDF) on the web. Hew sets the paper size and margins it composed for; the OS handles printer selection, copies, and its own PDF export.

**Save PDF…** skips the OS print system entirely: Hew writes the PDF itself, with the identical pages — vector Line art pages stay vector in the file, not rasterized — and asks where to put it with the ordinary save dialog. It's the surest route to a PDF that measures exactly right, and it's the same code path on desktop, web, and phone. The file is named after the job — "Café table — 1:10.pdf".

If the system print dialog can't be opened for some reason, the dialog says so and offers the browser's print dialog instead.

## Each Scene and the cut list

With **Pages** set to **Each Scene**, the print runs once per [Scene](/learn/scenes/): one page (Standard) or one tile set (Scaled) per Scene, each using that Scene's own camera, hidden objects, and section cut, exactly as activating it would. The preview stacks one group per Scene, named; the Scene name goes in each page's title block; page numbers run continuously across the whole job — "Page 4 of 11" on the third page of the second Scene, say.

Turn on **Cut list page** and Hew appends a table of every part — each object and component in the document, once, whatever tags it carries — with Part, Qty, L, W, and H columns in your working units. Every placement of one component is the same part however it is turned in the world, so they fold into one row with a quantity and the component's own dimensions; identical objects (same name and size) fold the same way, and the title block totals them ("4 parts · 8 pieces"). The page is appended once per job, after every Scene, and counted into the page total.

## What prints and what doesn't

Dimensions and text print at the size they show on screen. Guides are off by default; turn them on if you want them on the page. A section cut prints if one is active — but the section plane widget itself never does. Grid and axes are off by default and only offered in Standard mode. Selection highlights, hover states, and tool previews never print, in either mode — the page always shows the model, not your editing session.

Print preferences — paper, margins, style per mode, the last scale you used — are remembered on the machine you're printing from, not saved into the `.hew` file. The very first time, paper defaults to your operating system's own default on desktop, or to Letter in the US, Canada, Mexico, and the Philippines and A4 everywhere else.

## Printing from your phone

Shop Mode has a Print Layout sheet of its own: tap the document name pill to open the document menu, then **Print…**. It's the phone-shaped essentials — Standard or Scaled, Paper, Scale (with Fit), Extent, Style, and a strip of the pages — with **Save PDF…** as the primary action: on iOS it opens the share sheet so you can save to Files; elsewhere it downloads. **Print…** uses the browser's print, which on iOS means AirPrint — and AirPrint ignores the paper size Hew composed for, so pick the matching paper size and 100 % scale in AirPrint's own sheet. There's no **Selection** extent, no nudge, and no Each Scene on the phone; see [Hew on your phone](/learn/hew-on-your-phone/#printing-from-the-phone) for the full picture.

## Exporting an SVG line drawing

For a laser cutter, a CNC workflow, or any vector tool, **File ▸ Export…** has an **SVG line drawing (.svg)** format — the same hidden-line engine as Scaled Line art, written as a true-size SVG in millimeters with hidden lines removed. See [Import and export](/learn/import-export/#svg-line-drawing--for-laser-cnc-and-vector-tools) for the export options; for paper or PDF output, this chapter is the one you want.

## For scripts and agents

`hew.print.pdf` prints a document to a PDF exactly like this dialog does — standard or scaled, any paper, any style — and `hew.view.line_drawing` returns the same hidden-line drawing as an SVG or as raw 2D segments, for a camera, a standard view, or a Scene. Both are available headlessly through `hew-cli` and over MCP as `hew_print_pdf` and `hew_line_drawing`. For example:

```
hew-cli dispatch hew.print.pdf '{"view":"top","scale":1,"paper":"letter","path":"out.pdf"}' --file model.hew
```

See the [Hew API reference](https://github.com/hew3d/hew/blob/main/docs/API_REFERENCE.gen.md) for the full parameter list.

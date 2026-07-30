---
title: "3D Text"
description: "Extruded, watertight lettering from any font on your machine — signs, labels, embossed and engraved text you can print."
order: 8
---

3D Text turns typed characters into real solids: watertight, printable, and made of the same geometry as anything else you model. Counters — the enclosed spaces in letters like **o**, **a**, and **B** — come out as genuine through-holes, not painted-on shapes.

## The gesture

1. Open **Draw ▸ 3D Text…**, or search "text" in the command palette.
2. Type your text. Line breaks work; each line sits below the last.
3. Pick a font (see [Choosing a font](#choosing-a-font) below).
4. Set **Height** — the type size, the same measurement a word processor calls font size — and **Extrusion depth**, how far the letters stand off the surface. Both are in your document's units.

   Individual letters come out shorter than the Height you type, because a font distributes that measurement across ascenders, descenders, and the space around them. A capital O at 150 mm Height stands about 108 mm tall in Onest, and a different font will land somewhere else. If you need a letter to be an exact size, place it and measure it, then scale.
5. Click **OK**, then click where you want it. The text places on whatever face you click, or on the ground plane.

The result is a single component instance named after the text, so `3D Text "OPEN"` shows up in the Outliner under that name. One `Esc` cancels before placement; one undo removes it after.

## Choosing a font

The font list has up to three sections:

**Bundled** — the families that ship with Hew, available everywhere including the browser. They were screened to extrude cleanly.

**System** — the fonts installed on your machine. On the desktop apps this list appears on its own. In a browser it needs your permission first: click **Use my system fonts…** and accept the prompt. Some browsers don't offer this at all, in which case the section doesn't appear and you can still load a font file by hand.

**Loaded** — any `.ttf` or `.otf` you've opened yourself this session. Loaded fonts last until you reload; the document stores the resulting geometry, never the font, so a file you send someone else opens fine without it.

Type in the filter box to narrow the list by family or style name. Families with more than one face get a style selector — Hew uses the real Bold or Italic face from the font, and never fakes one by slanting or thickening a Regular.

### When a font warns you

Some fonts — particularly variable fonts, whose stored default outline is what Hew reads — have glyphs whose outlines overlap themselves. Hew tells you when the text you've typed uses one, because self-overlapping outlines are exactly the case it can't classify reliably, and letters may come out with the wrong parts filled.

It's a warning, not a refusal. Place it and look: plenty of such fonts extrude fine anyway. If a letter comes out wrong, pick another face.

## Booleans with text

Text arrives as a component instance, and booleans need plain solids. So when you select 3D Text for **Subtract**, **Union**, or **Intersect**, Hew explodes it to solids first and tells you it did — that's the "Component exploded to solids for the boolean" message.

This is what makes engraving and embossing work: subtract text from a face to engrave it, union it to emboss. The explode is a real edit, so undoing back past a boolean of this kind takes a few steps rather than one.

## Printing considerations

Extruded text is watertight from the start, so it exports to STL without repair. Two things worth knowing:

- Very small text at very fine detail can produce slivers thinner than your printer can resolve. Raise the height rather than the depth if letters come out fragile.
- Engraved text (subtracted) leaves the letter shapes as voids; embossed text (unioned) leaves them standing proud. Both stay watertight.

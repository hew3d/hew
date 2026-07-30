/**
 * glyphRun — turns a font + text string into flattened, classified 2D
 * contours (docs/design/3d-text.md). The one opentype.js-aware layer atop
 * the pure geometry in `flatten.ts`.
 */

import type { Font } from 'opentype.js'
import {
  classifyContourFill,
  pathCommandsToContours,
  sagittaToleranceEm,
  weldContour,
  type FlattenCommand,
  type Pt2,
} from './flatten'

/** One glyph-run contour in the font's own em-square units (Y increasing
 *  DOWNWARD — opentype.js's canvas-style convention): its welded, flattened
 *  point loop, and whether it is glyph MATERIAL (`fill: true`) or a
 *  counter's own bare interior (`fill: false`, never extrude — see
 *  `classifyContourFill`'s doc comment). `glyphIndex` is which glyph of the
 *  run this contour came from (0-based, in layout order across the whole
 *  run including line breaks — a raw glyph slot, so a contour-less glyph
 *  like a space still consumes an index) — threaded through so a caller
 *  can scope a per-glyph decision (the sliver-area floor,
 *  `TextPlaceTool.ts`'s `selectFillRegions`) to "this glyph's own regions"
 *  rather than the whole run, since real text lays glyphs out at their own
 *  non-overlapping advance-width slots and a single small glyph (a period)
 *  next to a large one (a notdef box) would otherwise get judged against
 *  the WRONG glyph's scale. */
export interface GlyphRunContour {
  points: Pt2[]
  fill: boolean
  glyphIndex: number
}

/** Multiline line spacing, as a multiple of the font's em-square — the
 *  common typesetting default; this app doesn't expose a separate
 *  line-height control (a v1 scope call, not a design constraint). */
export const LINE_HEIGHT_EM_FACTOR = 1.2

/**
 * Lays out `text` (may contain `\n` for multiline) in `font`, flattens
 * every glyph's Bézier outline adaptively (`sagittaToleranceEm`, scaled for
 * a run placed `heightMeters` tall — the em-square's world size), welds
 * each contour, and classifies fill vs. counter. Font advance/kerning come
 * from `Font.getPaths`, which lays out a whole line in one call and hands
 * back one `Path` per glyph (`Font.getPath`'s combined-path sibling) — same
 * coordinate space, but keeping the glyph boundary lets every contour it
 * produces be tagged with `glyphIndex`; `fontSize` is passed as
 * `unitsPerEm` so the returned coordinates stay in raw em-square units
 * (scale 1:1) — the em→world scale is the caller's job (`heightMeters /
 * font.unitsPerEm`), kept separate from flattening so the tolerance math in
 * `flatten.ts` only ever has one scale factor to reason about. Blank lines
 * contribute no contours (nothing to flatten). Returns an empty array for
 * text with no visible glyphs at all (e.g. all whitespace) — callers must
 * not attempt to place that.
 */
export function layoutGlyphRun(font: Font, text: string, heightMeters: number): GlyphRunContour[] {
  const tolEm = sagittaToleranceEm(heightMeters, font.unitsPerEm)
  const lineHeight = font.unitsPerEm * LINE_HEIGHT_EM_FACTOR
  const allContours: Pt2[][] = []
  const glyphIndexOf: number[] = []
  let glyphIndex = 0

  text.split('\n').forEach((line, i) => {
    if (line.length === 0) return
    const paths = font.getPaths(line, 0, i * lineHeight, font.unitsPerEm)
    for (const path of paths) {
      const commands = path.commands as unknown as FlattenCommand[]
      for (const contour of pathCommandsToContours(commands, tolEm)) {
        const welded = weldContour(contour)
        if (welded.length >= 3) {
          allContours.push(welded)
          glyphIndexOf.push(glyphIndex)
        }
      }
      glyphIndex++
    }
  })

  const fills = classifyContourFill(allContours)
  return allContours.map((points, i) => ({ points, fill: fills[i], glyphIndex: glyphIndexOf[i] }))
}

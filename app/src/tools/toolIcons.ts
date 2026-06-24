/**
 * toolIcons — Material Symbols (Apache-2.0) icon registry for the
 * toolbar and tool-aware viewport cursors.
 *
 * Icons are imported as raw SVG strings (`?raw`) rather than via the
 * ligature webfont so the exact same path data can be reused both for
 * inline toolbar rendering AND for building a `cursor: url(data:...)` CSS
 * value — a font glyph can't be turned into a cursor image. This also
 * keeps the app offline-safe (no fonts.googleapis.com fetch).
 *
 * All icon names below were verified to exist in
 * `@material-symbols/svg-400/outlined/` — no substitutions were needed.
 */

import arrowSelectorToolSvg from '@material-symbols/svg-400/outlined/arrow_selector_tool.svg?raw'
import rectangleSvg from '@material-symbols/svg-400/outlined/rectangle.svg?raw'
import polylineSvg from '@material-symbols/svg-400/outlined/polyline.svg?raw'
import unfoldMoreSvg from '@material-symbols/svg-400/outlined/unfold_more.svg?raw'
import formatPaintSvg from '@material-symbols/svg-400/outlined/format_paint.svg?raw'
import openWithSvg from '@material-symbols/svg-400/outlined/open_with.svg?raw'
import rotateRightSvg from '@material-symbols/svg-400/outlined/rotate_right.svg?raw'
import aspectRatioSvg from '@material-symbols/svg-400/outlined/aspect_ratio.svg?raw'
import straightenSvg from '@material-symbols/svg-400/outlined/straighten.svg?raw'
import architectureSvg from '@material-symbols/svg-400/outlined/architecture.svg?raw'
import threeDRotationSvg from '@material-symbols/svg-400/outlined/3d_rotation.svg?raw'
import panToolSvg from '@material-symbols/svg-400/outlined/pan_tool.svg?raw'
import zoomInSvg from '@material-symbols/svg-400/outlined/zoom_in.svg?raw'
import contentCutSvg from '@material-symbols/svg-400/outlined/content_cut.svg?raw'
import dragPanSvg from '@material-symbols/svg-400/outlined/drag_pan.svg?raw'

/** Keep in sync with the `ToolName` union in App.tsx (not imported here to
 * avoid a UI-module -> App.tsx dependency edge; the key set is asserted by
 * the `toolIcons.test.ts` coverage test instead). */
export type ToolName =
  | 'Select'
  | 'Rectangle'
  | 'Line'
  | 'Push/Pull'
  | 'Paint'
  | 'Move'
  | 'Rotate'
  | 'Scale'
  | 'Tape Measure'
  | 'Protractor'
  | 'Slice'
  | 'Edit Vertex'
  | 'Orbit'
  | 'Pan'
  | 'Zoom'

/** Raw Material Symbols SVG markup for each tool. */
export const TOOL_ICON_SVG: Record<ToolName, string> = {
  'Select': arrowSelectorToolSvg,
  'Rectangle': rectangleSvg,
  'Line': polylineSvg,
  'Push/Pull': unfoldMoreSvg, // matches the up/down-arrow cursor stop-gap
  'Paint': formatPaintSvg,
  'Move': openWithSvg,
  'Rotate': rotateRightSvg,
  'Scale': aspectRatioSvg,
  'Tape Measure': straightenSvg,
  'Protractor': architectureSvg,
  'Slice': contentCutSvg,
  'Edit Vertex': dragPanSvg,
  'Orbit': threeDRotationSvg,
  'Pan': panToolSvg,
  'Zoom': zoomInSvg,
}

/** Per-tool cursor hotspot, expressed as a fraction of the 32x32 cursor
 * canvas (0..1). Center for transform tools (Move/Rotate/Scale/Orbit),
 * tip-of-glyph for point-and-click tools (Select/Paint/Tape Measure). */
const CURSOR_HOTSPOT: Record<ToolName, { x: number; y: number }> = {
  'Select': { x: 0.25, y: 0.1 },
  'Rectangle': { x: 0.1, y: 0.9 },
  'Line': { x: 0.1, y: 0.9 },
  'Push/Pull': { x: 0.5, y: 0.5 },
  'Paint': { x: 0.15, y: 0.9 },
  'Move': { x: 0.5, y: 0.5 },
  'Rotate': { x: 0.5, y: 0.5 },
  'Scale': { x: 0.5, y: 0.5 },
  'Tape Measure': { x: 0.1, y: 0.9 },
  'Protractor': { x: 0.1, y: 0.9 },
  'Slice': { x: 0.1, y: 0.9 },
  'Edit Vertex': { x: 0.5, y: 0.5 },
  'Orbit': { x: 0.5, y: 0.5 },
  'Pan': { x: 0.5, y: 0.5 },
  'Zoom': { x: 0.5, y: 0.5 },
}

const CURSOR_SIZE = 32
const GLYPH_SIZE = 22

/** Extract the inner markup (path/group elements) of a Material Symbols
 * SVG, i.e. everything between the opening and closing `<svg>` tags. The
 * source SVGs use `viewBox="0 -960 960 960"` with a single `<path>`. */
function innerMarkup(svg: string): string {
  const match = svg.match(/<svg[^>]*>([\s\S]*)<\/svg>/)
  return match ? match[1] : svg
}

/**
 * Build a CSS `cursor` value from a tool's Material Symbol glyph.
 *
 * A flat single-color glyph disappears against same-colored geometry, so
 * the glyph is rendered twice at the same position: once underneath with a
 * thick white stroke (the halo) and once on top with a solid dark fill —
 * giving contrast against both light and dark backgrounds.
 */
export function cursorFor(toolName: string): string {
  const icon = TOOL_ICON_SVG[toolName as ToolName] ?? TOOL_ICON_SVG['Select']
  const hotspot = CURSOR_HOTSPOT[toolName as ToolName] ?? CURSOR_HOTSPOT['Select']
  const glyph = innerMarkup(icon)

  const offset = (CURSOR_SIZE - GLYPH_SIZE) / 2
  const scale = GLYPH_SIZE / 960
  // Source icons use viewBox="0 -960 960 960" (x in [0,960], y in [-960,0]),
  // which already maps to a top-left origin under a plain positive scale —
  // no axis flip needed, just scale-down and re-center into the cursor canvas.
  const translate = offset + GLYPH_SIZE

  const svg =
    `<svg xmlns="http://www.w3.org/2000/svg" width="${CURSOR_SIZE}" height="${CURSOR_SIZE}" viewBox="0 0 ${CURSOR_SIZE} ${CURSOR_SIZE}">` +
    `<g transform="translate(${offset} ${translate}) scale(${scale})" ` +
    `fill="#111" stroke="#fff" stroke-width="${2.5 / scale}" stroke-linejoin="round" paint-order="stroke fill">` +
    glyph +
    `</g></svg>`

  const hotspotX = Math.round(hotspot.x * CURSOR_SIZE)
  const hotspotY = Math.round(hotspot.y * CURSOR_SIZE)

  return `url("data:image/svg+xml,${encodeURIComponent(svg)}") ${hotspotX} ${hotspotY}, auto`
}

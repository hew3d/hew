import { describe, it, expect } from 'vitest'
import { TOOL_ICON_SVG, cursorFor, type ToolName } from './toolIcons'
import colorizeSvg from '@material-symbols/svg-400/outlined/colorize.svg?raw'

/** The "X Y" hotspot pair out of a `cursorFor` value's trailing
 *  `url("...") X Y, auto` — independent of the embedded SVG's own encoding. */
function hotspotSuffix(cursor: string): string {
  const afterUrl = cursor.indexOf('") ') + 3
  return cursor.slice(afterUrl, cursor.indexOf(', auto'))
}

/** The `d` attribute of a markup string's first `<path>` — a glyph's most
 *  distinctive fingerprint (icons never share path data). */
function firstPathData(markup: string): string {
  const match = markup.match(/<path[^>]*\sd="([^"]+)"/)
  if (match === null) throw new Error('no <path> found')
  return match[1]
}

const ALL_TOOLS: ToolName[] = [
  'Select',
  'Rectangle',
  'Circle',
  'Polygon',
  'Arc',
  'Line',
  'Push/Pull',
  'Paint',
  'Position Texture',
  'Move',
  'Rotate',
  'Scale',
  'Tape Measure',
  'Protractor',
  'Slice',
  'Section Plane',
  'Edit Vertex',
  'Orbit',
  'Pan',
  'Zoom',
  'Zoom Window',
]

describe('TOOL_ICON_SVG', () => {
  it('covers every tool with non-empty SVG markup', () => {
    for (const t of ALL_TOOLS) {
      expect(TOOL_ICON_SVG[t]).toBeTruthy()
      expect(TOOL_ICON_SVG[t]).toContain('<svg')
      expect(TOOL_ICON_SVG[t]).toContain('<path')
    }
  })
})

describe('cursorFor', () => {
  it('returns a url(...) cursor value for every tool', () => {
    for (const t of ALL_TOOLS) {
      const cursor = cursorFor(t)
      expect(cursor.startsWith('url("data:image/svg+xml,')).toBe(true)
      expect(cursor.endsWith(', auto')).toBe(true)
    }
  })

  it('embeds a halo (white stroke) and a dark fill for contrast', () => {
    const cursor = cursorFor('Move')
    const decoded = decodeURIComponent(cursor.slice('url("'.length, cursor.indexOf('")')))
    expect(decoded).toContain('stroke="#fff"')
    expect(decoded).toContain('fill="#111"')
  })

  it('falls back to the Select cursor for an unrecognized tool name', () => {
    expect(cursorFor('NotARealTool')).toBe(cursorFor('Select'))
  })

  it('adds a haloed + badge with copyBadge (Move copy toggle), and only then', () => {
    const plain = cursorFor('Move')
    const badged = cursorFor('Move', true)
    expect(badged).not.toBe(plain)
    const decoded = decodeURIComponent(badged.slice('url("'.length, badged.indexOf('")')))
    // The plus is stroke-drawn twice: a white halo under a dark stroke.
    expect(decoded).toContain('M26 3v8M21.5 8.5h9')
    expect(cursorFor('Move', false)).toBe(plain)
  })

  it('with eyedropper, swaps the glyph to the eyedropper icon and the hotspot to Paint\'s, regardless of toolName', () => {
    const paintPlain = cursorFor('Paint')
    const paintEyedropper = cursorFor('Paint', false, true)
    const moveEyedropper = cursorFor('Move', false, true)

    // Same cursor value no matter which tool is active — both the glyph and
    // the hotspot are independent of `toolName` once `eyedropper` is set.
    expect(moveEyedropper).toBe(paintEyedropper)
    expect(paintEyedropper).not.toBe(paintPlain)

    const decodedEyedropper = decodeURIComponent(
      paintEyedropper.slice('url("'.length, paintEyedropper.indexOf('")')),
    )
    const decodedPaintPlain = decodeURIComponent(
      paintPlain.slice('url("'.length, paintPlain.indexOf('")')),
    )
    // The glyph is the eyedropper icon's own path data...
    expect(decodedEyedropper).toContain(firstPathData(colorizeSvg))
    // ...not Paint's format_paint glyph.
    expect(decodedEyedropper).not.toContain(firstPathData(decodedPaintPlain))

    // The hotspot matches Paint's own (a point-and-click tip), not Move's
    // (dead center) — proving it swapped to Paint's, not merely reused
    // whichever tool was passed in.
    expect(hotspotSuffix(paintEyedropper)).toBe(hotspotSuffix(paintPlain))
    expect(hotspotSuffix(paintEyedropper)).not.toBe(hotspotSuffix(cursorFor('Move')))
  })
})

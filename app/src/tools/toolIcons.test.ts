import { describe, it, expect } from 'vitest'
import { TOOL_ICON_SVG, cursorFor, type ToolName } from './toolIcons'

const ALL_TOOLS: ToolName[] = [
  'Select',
  'Rectangle',
  'Push/Pull',
  'Paint',
  'Move',
  'Rotate',
  'Scale',
  'Tape Measure',
  'Protractor',
  'Slice',
  'Orbit',
  'Pan',
  'Zoom',
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
})

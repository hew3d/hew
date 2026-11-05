import { describe, it, expect } from 'vitest'
import { TOOL_REGISTRY, TOOLS, RAIL_GROUPS, toolSpec, shortcutFor, toolsInGroup } from './toolRegistry'

describe('toolRegistry', () => {
  it('TOOLS matches TOOL_REGISTRY order and length', () => {
    expect(TOOLS).toEqual(TOOL_REGISTRY.map((t) => t.name))
  })

  it('has no duplicate tool names', () => {
    expect(new Set(TOOLS).size).toBe(TOOLS.length)
  })

  it('toolSpec looks up an entry by name', () => {
    expect(toolSpec('Select').actionId).toBe('tool-select')
  })

  it('groups the 03_tool_rail.md tools plus Arc, Polygon, Follow Me, Dimension, and Section Plane', () => {
    expect(toolsInGroup('Draw').map((t) => t.name)).toEqual(['Select', 'Line', 'Rectangle', 'Circle', 'Polygon', 'Arc'])
    expect(toolsInGroup('Modify').map((t) => t.name)).toEqual(['Push/Pull', 'Follow Me', 'Offset', 'Move', 'Rotate', 'Scale'])
    expect(toolsInGroup('Inspect').map((t) => t.name)).toEqual(['Tape Measure', 'Dimension', 'Paint', 'Section Plane'])
  })

  it('leaves Protractor/Slice/Edit Vertex/Drawing Axes/Text/Position Texture/camera tools off the rail (no group)', () => {
    const ungrouped = TOOL_REGISTRY.filter((t) => t.group === undefined).map((t) => t.name)
    expect(ungrouped).toEqual([
      'Protractor', 'Slice', 'Edit Vertex', 'Drawing Axes', 'Text', 'Position Texture',
      'Orbit', 'Pan', 'Zoom', 'Zoom Window',
      'Position Camera', 'Look Around', 'Walk',
    ])
  })

  it('RAIL_GROUPS is Draw/Modify/Inspect in spec order', () => {
    expect(RAIL_GROUPS).toEqual(['Draw', 'Modify', 'Inspect'])
  })

  it('every tool has its own command id', () => {
    const ids = TOOL_REGISTRY.map((t) => t.actionId)
    expect(new Set(ids).size).toBe(ids.length)
    for (const id of ids) expect(id).toMatch(/^tool-/)
  })

  describe('shortcutFor', () => {
    // The keys themselves live in the keymap (keymap/defaults.ts, tested per
    // platform in defaults.test.ts); shortcutFor shows the effective one.
    // The test environment is neither macOS nor the desktop shell.
    it('shows the SketchUp bare-letter key of a tool', () => {
      expect(shortcutFor('Select')).toBe('Spc')
      expect(shortcutFor('Line')).toBe('L')
      expect(shortcutFor('Rectangle')).toBe('R')
      expect(shortcutFor('Arc')).toBe('A')
      expect(shortcutFor('Push/Pull')).toBe('P')
      expect(shortcutFor('Orbit')).toBe('O')
      expect(shortcutFor('Zoom')).toBe('Z')
    })

    it('is empty for a tool with no key', () => {
      for (const name of ['Follow Me', 'Polygon', 'Protractor', 'Slice', 'Section Plane', 'Edit Vertex', 'Dimension', 'Text'] as const) {
        expect(shortcutFor(name)).toBe('')
      }
    })
  })
})

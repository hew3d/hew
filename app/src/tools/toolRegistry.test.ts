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
    expect(toolSpec('Select').macKey).toBe('Spc')
  })

  it('groups the 03_tool_rail.md tools plus Arc, 5/4/2 per group', () => {
    expect(toolsInGroup('Draw').map((t) => t.name)).toEqual(['Select', 'Line', 'Rectangle', 'Circle', 'Arc'])
    expect(toolsInGroup('Modify').map((t) => t.name)).toEqual(['Push/Pull', 'Move', 'Rotate', 'Scale'])
    expect(toolsInGroup('Inspect').map((t) => t.name)).toEqual(['Tape Measure', 'Paint'])
  })

  it('leaves Protractor/Slice/Edit Vertex/camera tools off the rail (no group)', () => {
    const ungrouped = TOOL_REGISTRY.filter((t) => t.group === undefined).map((t) => t.name)
    expect(ungrouped).toEqual(['Protractor', 'Slice', 'Edit Vertex', 'Orbit', 'Pan', 'Zoom'])
  })

  it('RAIL_GROUPS is Draw/Modify/Inspect in spec order', () => {
    expect(RAIL_GROUPS).toEqual(['Draw', 'Modify', 'Inspect'])
  })

  describe('shortcutFor', () => {
    it('macOS keeps its pre- Cmd-combo accelerators unchanged', () => {
      expect(shortcutFor('Rectangle', true)).toBe('⌘K')
      expect(shortcutFor('Line', true)).toBe('⌘L')
      expect(shortcutFor('Push/Pull', true)).toBe('⌘=')
      expect(shortcutFor('Move', true)).toBe('⌘0')
    })

    it('Windows/Linux/Web use the SketchUp-for-Windows bare-letter scheme', () => {
      expect(shortcutFor('Select', false)).toBe('Spc')
      expect(shortcutFor('Line', false)).toBe('L')
      expect(shortcutFor('Rectangle', false)).toBe('R')
      expect(shortcutFor('Circle', false)).toBe('C')
      expect(shortcutFor('Arc', false)).toBe('A') // SketchUp-for-Windows' real arc key
      expect(shortcutFor('Push/Pull', false)).toBe('P')
      expect(shortcutFor('Move', false)).toBe('M')
      expect(shortcutFor('Rotate', false)).toBe('Q')
      expect(shortcutFor('Scale', false)).toBe('S')
      expect(shortcutFor('Tape Measure', false)).toBe('T')
      expect(shortcutFor('Paint', false)).toBe('B')
    })

    it('non-spec tools (Protractor/Slice/Edit Vertex) have no shortcut on either platform', () => {
      for (const name of ['Protractor', 'Slice', 'Edit Vertex'] as const) {
        expect(shortcutFor(name, true)).toBe('')
        expect(shortcutFor(name, false)).toBe('')
      }
    })

    it('Arc uses Cmd+J on macOS — SketchUp\'s arc-family key (assigned)', () => {
      expect(shortcutFor('Arc', true)).toBe('⌘J')
    })

    it('camera tools use SketchUp\'s real O / H / Z on non-Mac', () => {
      expect(shortcutFor('Orbit', false)).toBe('O')
      expect(shortcutFor('Pan', false)).toBe('H')
      expect(shortcutFor('Zoom', false)).toBe('Z')
    })
  })
})

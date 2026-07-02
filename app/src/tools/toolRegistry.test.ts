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

  it('groups exactly the 10 tools 03_tool_rail.md covers, 4/4/2 per group', () => {
    expect(toolsInGroup('Draw').map((t) => t.name)).toEqual(['Select', 'Line', 'Rectangle', 'Circle'])
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

    it('camera tools keep their existing Ctrl-combo shortcuts on non-Mac (no spec bare letter)', () => {
      expect(shortcutFor('Orbit', false)).toBe('Ctrl+B')
      expect(shortcutFor('Pan', false)).toBe('Ctrl+R')
      expect(shortcutFor('Zoom', false)).toBe('Ctrl+\\')
    })
  })
})

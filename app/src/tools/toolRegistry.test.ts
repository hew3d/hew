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

  it('groups the 03_tool_rail.md tools plus Arc and Follow Me, 5/5/2 per group', () => {
    expect(toolsInGroup('Draw').map((t) => t.name)).toEqual(['Select', 'Line', 'Rectangle', 'Circle', 'Arc'])
    expect(toolsInGroup('Modify').map((t) => t.name)).toEqual(['Push/Pull', 'Follow Me', 'Move', 'Rotate', 'Scale'])
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
    it('macOS advertises the same bare-letter scheme the keydown handler dispatches there', () => {
      // The macOS playtest flagged that the rail displayed shortcuts that
      // didn't work (Cmd-combos live only in the native menu). The rail now
      // shows the bare letters, which App.tsx dispatches on every platform.
      expect(shortcutFor('Rectangle', true)).toBe('R')
      expect(shortcutFor('Line', true)).toBe('L')
      expect(shortcutFor('Push/Pull', true)).toBe('P')
      expect(shortcutFor('Move', true)).toBe('M')
      expect(shortcutFor('Circle', true)).toBe('C')
      expect(shortcutFor('Paint', true)).toBe('B')
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

    it('non-spec tools (Follow Me/Protractor/Slice/Edit Vertex) have no shortcut on either platform', () => {
      for (const name of ['Follow Me', 'Protractor', 'Slice', 'Edit Vertex'] as const) {
        expect(shortcutFor(name, true)).toBe('')
        expect(shortcutFor(name, false)).toBe('')
      }
    })

    it('Arc uses the bare A on macOS too — same key as everywhere else', () => {
      expect(shortcutFor('Arc', true)).toBe('A')
    })

    it('camera tools use SketchUp\'s real O / H / Z on every platform', () => {
      for (const isMac of [true, false]) {
        expect(shortcutFor('Orbit', isMac)).toBe('O')
        expect(shortcutFor('Pan', isMac)).toBe('H')
        expect(shortcutFor('Zoom', isMac)).toBe('Z')
      }
    })
  })
})

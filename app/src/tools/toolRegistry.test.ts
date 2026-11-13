import { describe, it, expect } from 'vitest'
import { TOOL_REGISTRY, TOOLS, RAIL_GROUPS, toolSpec, toolsInGroup, isToolAction } from './toolRegistry'

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

  it('groups the 03_tool_rail.md tools plus Arc, Polygon, Follow Me, Flip, Dimension, and Section Plane', () => {
    expect(toolsInGroup('Draw').map((t) => t.name)).toEqual(['Select', 'Line', 'Rectangle', 'Circle', 'Polygon', 'Arc'])
    expect(toolsInGroup('Modify').map((t) => t.name)).toEqual(['Push/Pull', 'Follow Me', 'Offset', 'Move', 'Rotate', 'Scale', 'Flip'])
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

  it('isToolAction tells tool commands from every other command', () => {
    for (const t of TOOL_REGISTRY) expect(isToolAction(t.actionId)).toBe(true)
    expect(isToolAction('edit-undo')).toBe(false)
    expect(isToolAction('toggle-parallel-projection')).toBe(false)
    expect(isToolAction('tool-')).toBe(false)
  })
})

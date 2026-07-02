import { describe, it, expect } from 'vitest'
import { paletteEntries, paletteShortcut } from './registry'
import { TOOLS } from '../tools/toolRegistry'

describe('paletteEntries', () => {
  const entries = paletteEntries()

  it('has no duplicate ids', () => {
    const ids = entries.map((e) => e.id)
    expect(new Set(ids).size).toBe(ids.length)
  })

  it('includes every tool, not just the rail-visible subset', () => {
    const toolLabels = entries.filter((e) => e.group === 'Tools').map((e) => e.label)
    expect(toolLabels.sort()).toEqual([...TOOLS].sort())
  })

  it('every entry has a non-empty label and description', () => {
    for (const e of entries) {
      expect(e.label.length).toBeGreaterThan(0)
      expect(e.description.length).toBeGreaterThan(0)
    }
  })

  it('includes core File/Edit actions', () => {
    const ids = entries.map((e) => e.id)
    expect(ids).toEqual(expect.arrayContaining(['new', 'open', 'save', 'save-as', 'undo', 'redo']))
  })
})

describe('paletteShortcut', () => {
  const entries = paletteEntries()
  const rectangle = entries.find((e) => e.id === 'tool-rectangle')!
  const saveAction = entries.find((e) => e.id === 'save')!

  it('returns the mac shortcut for a tool entry when isMac', () => {
    expect(paletteShortcut(rectangle, true)).toBe('⌘K')
  })

  it('returns the Windows/Linux/Web shortcut for a tool entry when !isMac', () => {
    expect(paletteShortcut(rectangle, false)).toBe('R')
  })

  it('returns "" for a non-tool action entry', () => {
    expect(paletteShortcut(saveAction, true)).toBe('')
    expect(paletteShortcut(saveAction, false)).toBe('')
  })
})

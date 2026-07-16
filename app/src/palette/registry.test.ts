import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { describe, it, expect } from 'vitest'
import { paletteEntries, paletteShortcut, PALETTE_EXCLUDED_ACTION_IDS } from './registry'
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

  it('includes the Edit object commands (booleans, group/component verbs)', () => {
    const ids = entries.map((e) => e.id)
    expect(ids).toEqual(
      expect.arrayContaining([
        'edit-group',
        'edit-ungroup',
        'edit-make-component',
        'edit-place-copy',
        'edit-explode',
        'edit-make-unique',
        'edit-union',
        'edit-subtract',
        'edit-intersect',
      ]),
    )
  })
})

describe('palette completeness against the menuActionRef switch', () => {
  // App.tsx's menuActionRef switch is the canonical action id space: every
  // surface (palette, in-app menus, native menu, contextual dock, keyboard)
  // dispatches through it. Extract its `case '…':` ids from the source and
  // diff them against the registry, so a newly added action can't silently
  // skip the palette — it must be registered or excused (with a reason) in
  // PALETTE_EXCLUDED_ACTION_IDS.
  const appSource = readFileSync(
    fileURLToPath(new URL('../App.tsx', import.meta.url)),
    'utf8',
  )
  // Capture EVERY quoted case label — a shape restriction here would let a
  // non-conforming id (camelCase, snake_case, leading digit) evade the
  // completeness check silently. Conformance is asserted separately below.
  const switchCaseIds = new Set(
    [...appSource.matchAll(/^\s*case '([^']+)':/gm)].map((m) => m[1]),
  )
  const registryIds = new Set(paletteEntries().map((e) => e.id))
  const excusedIds = new Set(Object.keys(PALETTE_EXCLUDED_ACTION_IDS))

  it('found the switch (guards the extraction against a refactor)', () => {
    // If App.tsx's dispatch moves or the case syntax changes, fail loudly
    // rather than silently comparing against an empty set.
    expect(switchCaseIds.size).toBeGreaterThan(40)
    expect(switchCaseIds).toContain('zoom-extents')
    expect(switchCaseIds).toContain('edit-union')
  })

  it('every action id is kebab-case', () => {
    // The id space is kebab-case by convention; enforce the shape here so a
    // non-conforming id fails loudly instead of slipping past the extraction.
    const nonKebab = [...switchCaseIds].filter((id) => !/^[a-z][a-z0-9-]*$/.test(id))
    expect(nonKebab, 'menu-action ids must be kebab-case').toEqual([])
  })

  it('every dispatchable action is in the palette or explicitly excused', () => {
    const missing = [...switchCaseIds].filter(
      (id) => !registryIds.has(id) && !excusedIds.has(id),
    )
    expect(missing, 'register these ids in palette/registry.ts (or excuse them in PALETTE_EXCLUDED_ACTION_IDS with a reason)').toEqual([])
  })

  it('every palette entry dispatches to a real switch case', () => {
    const dead = [...registryIds].filter((id) => !switchCaseIds.has(id))
    expect(dead, 'these palette ids have no menuActionRef case — they would silently no-op').toEqual([])
  })

  it('no id is both registered and excused', () => {
    const both = [...excusedIds].filter((id) => registryIds.has(id))
    expect(both).toEqual([])
  })

  it('every excused id still exists in the switch (no stale excuses)', () => {
    const stale = [...excusedIds].filter((id) => !switchCaseIds.has(id))
    expect(stale).toEqual([])
  })
})

describe('paletteShortcut', () => {
  const entries = paletteEntries()
  const rectangle = entries.find((e) => e.id === 'tool-rectangle')!
  const saveAction = entries.find((e) => e.id === 'save')!

  it('returns the mac shortcut for a tool entry when isMac', () => {
    // Unified bare-letter scheme — same key the rail shows and App.tsx
    // dispatches on macOS (the native menu's ⌘K remains a secondary accel).
    expect(paletteShortcut(rectangle, true)).toBe('R')
  })

  it('returns the Windows/Linux/Web shortcut for a tool entry when !isMac', () => {
    expect(paletteShortcut(rectangle, false)).toBe('R')
  })

  it('returns "" for a non-tool action entry', () => {
    expect(paletteShortcut(saveAction, true)).toBe('')
    expect(paletteShortcut(saveAction, false)).toBe('')
  })
})

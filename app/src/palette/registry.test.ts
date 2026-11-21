import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { describe, it, expect, vi } from 'vitest'

// Pin the platform: the keys shown depend on it, and this suite may run on a Mac.
vi.mock('../platform', async (orig) => ({ ...(await orig<typeof import('../platform')>()), isMac: false }))
vi.mock('../io/fileHost', async (orig) => ({ ...(await orig<typeof import('../io/fileHost')>()), isTauri: false }))

import { paletteEntries, paletteShortcut, PALETTE_EXCLUDED_ACTION_IDS } from './registry'
import { TOOLS } from '../tools/toolRegistry'
import { requiresPreview } from '../test/requiresPreview'
import { actionOffered } from '../curveActions'

describe('paletteEntries', () => {
  const entries = paletteEntries()

  it('has no duplicate ids', () => {
    const ids = entries.map((e) => e.id)
    expect(new Set(ids).size).toBe(ids.length)
  })

  it('includes every tool, not just the rail-visible subset', () => {
    const toolLabels = entries.filter((e) => e.group === 'Tools').map((e) => e.label)
    // Deform is one tool with four palette entries, one per mode; Loft also
    // answers to Skin; Chamfer is the Fillet tool's second entry.
    const expected = TOOLS.flatMap((t) => (t === 'Deform' ? ['Twist', 'Bend', 'Taper', 'Bend to Path'] : t === 'Loft' ? ['Loft', 'Skin'] : [t]))
    expect(toolLabels.sort()).toEqual([...expected, ...(actionOffered('tool-chamfer') ? ['Chamfer'] : [])].sort())
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

  it('offers Home beside Iso as a standard view, with no default key', () => {
    const ids = entries.map((e) => e.id)
    expect(ids.indexOf('view-home')).toBe(ids.indexOf('view-iso') + 1)
    expect(paletteShortcut(entries.find((e) => e.id === 'view-home')!)).toBe('')
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

describe('palette search words', () => {
  it("Fillet answers to SketchUp's RoundCorner words", (ctx) => {
    requiresPreview(ctx, 'fillet', 'loft', 'revolve', 'deform', 'path-array', 'shell', 'project', 'smooth', 'soften')
    const fillet = paletteEntries().find((e) => e.id === 'tool-fillet')
    expect(fillet?.synonyms).toEqual(expect.arrayContaining(['round corner', 'roundcorner', 'fredocorner']))
  })

  it("every Deform mode answers to FredoScale's name", (ctx) => {
    requiresPreview(ctx, 'deform')
    for (const id of ['tool-deform-twist', 'tool-deform-bend', 'tool-deform-taper']) {
      expect(paletteEntries().find((e) => e.id === id)?.synonyms).toContain('fredoscale')
    }
  })

  it('Soften Edges… is gated on a directly selected solid, the Smooth levels and Bake on any selection', (ctx) => {
    requiresPreview(ctx, 'smooth', 'soften')
    const gate = (id: string) => paletteEntries().find((e) => e.id === id)?.gate
    expect(gate('soften-edges')).toBe('canSoften')
    for (const id of ['smooth-off', 'smooth-1', 'smooth-2', 'smooth-3', 'bake-smooth']) expect(gate(id)).toBe('selection')
  })

  it('every Object-menu Smooth action is in the palette (Smooth levels, Bake, Soften Edges)', (ctx) => {
    requiresPreview(ctx, 'smooth', 'soften')
    const ids = paletteEntries().map((e) => e.id)
    expect(ids).toEqual(expect.arrayContaining(['smooth-off', 'smooth-1', 'smooth-2', 'smooth-3', 'bake-smooth', 'soften-edges']))
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
    // An action the Preview wall withholds is not in a production palette.
    const missing = [...switchCaseIds].filter(
      (id) => actionOffered(id) && !registryIds.has(id) && !excusedIds.has(id),
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
  const zoomExtents = entries.find((e) => e.id === 'zoom-extents')!

  // The test environment is neither macOS nor the desktop shell.
  it('shows a tool entry\'s key from the keymap', () => {
    expect(paletteShortcut(rectangle)).toBe('R')
  })

  it('shows an action entry\'s key too', () => {
    expect(paletteShortcut(saveAction)).toBe('Ctrl+S')
  })

  it('is empty for a command with no key', () => {
    expect(paletteShortcut(zoomExtents)).toBe('')
  })
})

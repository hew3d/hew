import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { describe, expect, it } from 'vitest'
import { normalizeChord } from './chord'
import { defaultBindings, type KeymapPlatform } from './defaults'
import { buildKeymap, isReservedChord } from './keymap'

const PLATFORMS: [string, KeymapPlatform][] = [
  ['web', { isMac: false, isDesktop: false }],
  ['Windows/Linux desktop', { isMac: false, isDesktop: true }],
  ['macOS web', { isMac: true, isDesktop: false }],
  ['macOS desktop', { isMac: true, isDesktop: true }],
]

/** Effective defaults, normalized, for one platform. */
function effective(platform: KeymapPlatform) {
  return buildKeymap(defaultBindings(platform), {}, platform.isMac).bindings
}

describe('default bindings', () => {
  const appSource = readFileSync(fileURLToPath(new URL('../App.tsx', import.meta.url)), 'utf8')
  const commandIds = new Set([...appSource.matchAll(/^\s*case '([^']+)':/gm)].map((m) => m[1]))

  it.each(PLATFORMS)('%s: every command id is a menuActionRef case', (_, platform) => {
    expect(commandIds.size).toBeGreaterThan(40)
    const unknown = Object.keys(defaultBindings(platform)).filter((id) => !commandIds.has(id))
    expect(unknown).toEqual([])
  })

  it.each(PLATFORMS)('%s: every chord parses, none is reserved, none is used twice', (_, platform) => {
    const seen = new Map<string, string>()
    for (const [id, chords] of Object.entries(defaultBindings(platform))) {
      for (const c of chords) {
        const n = normalizeChord(c, platform.isMac)
        expect(n, `${id}: ${c}`).not.toBeNull()
        expect(isReservedChord(n!), `${id}: ${c}`).toBe(false)
        expect(seen.get(n!), `${c} bound to both ${seen.get(n!)} and ${id}`).toBeUndefined()
        seen.set(n!, id)
      }
    }
  })

  it('keeps the shortcuts the hand-written handlers dispatched (Windows/Linux/web)', () => {
    const b = effective({ isMac: false, isDesktop: false })
    expect(b['tool-select']).toEqual(['Space', '1'])
    expect(b['tool-line']).toEqual(['L'])
    expect(b['tool-rectangle']).toEqual(['R', '2'])
    expect(b['tool-circle']).toEqual(['C'])
    expect(b['tool-arc']).toEqual(['A'])
    expect(b['tool-pushpull']).toEqual(['P', '3'])
    expect(b['tool-offset']).toEqual(['F'])
    expect(b['tool-move']).toEqual(['M', '4'])
    expect(b['tool-rotate']).toEqual(['Q', '5'])
    expect(b['tool-scale']).toEqual(['S', '6'])
    expect(b['tool-tape-measure']).toEqual(['T'])
    expect(b['tool-paint']).toEqual(['B'])
    expect(b['tool-orbit']).toEqual(['O'])
    expect(b['tool-pan']).toEqual(['H'])
    expect(b['tool-zoom']).toEqual(['Z'])
    expect(b['new']).toEqual(['Ctrl+N'])
    expect(b['open']).toEqual(['Ctrl+O'])
    expect(b['save']).toEqual(['Ctrl+S'])
    expect(b['save-as']).toEqual(['Ctrl+Shift+S'])
    expect(b['print']).toEqual(['Ctrl+P'])
    expect(b['undo']).toEqual(['Ctrl+Z'])
    expect(b['redo']).toEqual(['Ctrl+Shift+Z', 'Ctrl+Y'])
    expect(b['edit-cut']).toEqual(['Ctrl+X'])
    expect(b['edit-copy']).toEqual(['Ctrl+C'])
    expect(b['edit-paste']).toEqual(['Ctrl+V'])
    expect(b['edit-paste-in-place']).toEqual(['Ctrl+Shift+V'])
    expect(b['edit-delete']).toEqual(['Delete', 'Backspace'])
    expect(b['edit-select-all']).toEqual(['Ctrl+A'])
    expect(b['edit-select-none']).toEqual(['Ctrl+Shift+A'])
    expect(b['edit-group']).toEqual(['Ctrl+G'])
    expect(b['edit-ungroup']).toEqual(['Ctrl+Shift+G'])
    expect(b['toggle-model-info']).toEqual(['Ctrl+Shift+I'])
    expect(b['toggle-object-info']).toEqual(['Ctrl+Shift+O'])
    expect(b['toggle-materials']).toEqual(['Ctrl+Shift+C'])
    expect(b['toggle-components']).toEqual(['Ctrl+Shift+M'])
    expect(b['toggle-tags']).toEqual(['Ctrl+Shift+T'])
    expect(b['open-library']).toEqual(['Ctrl+Shift+L', 'Shift+L'])
    expect(b['scenes-next']).toEqual(['PageDown'])
    expect(b['scenes-previous']).toEqual(['PageUp'])
    expect(b['open-settings']).toEqual(['Ctrl+,'])
    expect(b['open-palette']).toEqual(['Ctrl+/', 'Ctrl+K'])
  })

  it('leaves SketchUp\'s keyless tools without a key', () => {
    const b = effective({ isMac: false, isDesktop: false })
    for (const id of ['tool-polygon', 'tool-follow-me', 'tool-dimension', 'tool-section-plane', 'tool-protractor', 'tool-slice', 'tool-edit-vertex', 'tool-text']) {
      expect(b[id] ?? []).toEqual([])
    }
  })

  it('binds Close only in the desktop shell (a browser tab cannot close itself)', () => {
    expect(effective({ isMac: false, isDesktop: false })['close']).toBeUndefined()
    expect(effective({ isMac: false, isDesktop: true })['close']).toEqual(['Ctrl+W'])
    expect(effective({ isMac: true, isDesktop: true })['close']).toEqual(['Meta+W'])
  })

  it('adds SketchUp for Mac\'s Cmd-combos only on the macOS desktop, after the bare key', () => {
    const desktop = effective({ isMac: true, isDesktop: true })
    expect(desktop['tool-rectangle']).toEqual(['R', '2', 'Meta+K'])
    expect(desktop['tool-tape-measure']).toEqual(['T', 'Meta+D'])
    expect(desktop['tool-orbit']).toEqual(['O', 'Meta+B'])
    expect(desktop['tool-zoom']).toEqual(['Z', 'Meta+\\'])
    // In a browser Cmd+R reloads and Cmd+D bookmarks.
    const web = effective({ isMac: true, isDesktop: false })
    expect(web['tool-rectangle']).toEqual(['R', '2'])
    expect(web['tool-pan']).toEqual(['H'])
  })

  it('leaves Cmd+K to Rectangle on macOS: the palette has only Cmd+/ there', () => {
    expect(effective({ isMac: true, isDesktop: true })['open-palette']).toEqual(['Meta+/'])
  })

  it('adds Ctrl+Y as a second redo binding everywhere except macOS', () => {
    expect(effective({ isMac: false, isDesktop: true })['redo']).toEqual(['Ctrl+Shift+Z', 'Ctrl+Y'])
    expect(effective({ isMac: true, isDesktop: true })['redo']).toEqual(['Shift+Meta+Z'])
    expect(effective({ isMac: true, isDesktop: false })['redo']).toEqual(['Shift+Meta+Z'])
  })

  it('leads Delete with the key the platform calls delete', () => {
    expect(effective({ isMac: true, isDesktop: true })['edit-delete']).toEqual(['Backspace', 'Delete'])
    expect(effective({ isMac: false, isDesktop: true })['edit-delete']).toEqual(['Delete', 'Backspace'])
  })
})

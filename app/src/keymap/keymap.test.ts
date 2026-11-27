// @vitest-environment jsdom
import { beforeEach, describe, expect, it } from 'vitest'
import {
  buildKeymap,
  chordsFor,
  commandForChord,
  defaultChordsFor,
  isCustomized,
  isMacSystemChord,
  isPinnedTo,
  isReservedChord,
  overrides,
  pinnedOwner,
  railShortcutLabel,
  resetAllChords,
  resetChords,
  setChords,
  shortcutLabel,
  subscribeKeymap,
} from './keymap'

describe('isReservedChord', () => {
  it('keeps the tools\' gesture keys', () => {
    for (const c of ['Escape', 'Enter', 'Tab', 'ArrowUp', 'Shift+ArrowLeft', 'Shift+Escape']) {
      expect(isReservedChord(c), c).toBe(true)
    }
  })

  it('frees a Ctrl/Alt/Cmd arrow and everything else', () => {
    for (const c of ['Ctrl+ArrowUp', 'Alt+ArrowLeft', 'Meta+ArrowDown', 'D', 'Ctrl++', 'F5', 'Shift+L']) {
      expect(isReservedChord(c), c).toBe(false)
    }
  })
})

describe('pinned and system chords', () => {
  const macDesktop = { isMac: true, isDesktop: true }

  it('pins the fixed native Edit chords on the macOS desktop only', () => {
    expect(pinnedOwner('Meta+Z', macDesktop)).toBe('undo')
    expect(pinnedOwner('Shift+Meta+A', macDesktop)).toBe('edit-select-none')
    expect(isPinnedTo('undo', 'Meta+Z', macDesktop)).toBe(true)
    expect(isPinnedTo('redo', 'Meta+Z', macDesktop)).toBe(false)
    expect(pinnedOwner('Meta+K', macDesktop)).toBeUndefined()
    expect(pinnedOwner('Meta+Z', { isMac: true, isDesktop: false })).toBeUndefined()
    expect(pinnedOwner('Meta+Z', { isMac: false, isDesktop: true })).toBeUndefined()
  })

  it('refuses Cmd+Q, Cmd+H and Cmd+M on a Mac only', () => {
    for (const c of ['Meta+Q', 'Meta+H', 'Meta+M']) {
      expect(isMacSystemChord(c, { isMac: true, isDesktop: false }), c).toBe(true)
      expect(isMacSystemChord(c, { isMac: false, isDesktop: true }), c).toBe(false)
    }
    expect(isMacSystemChord('Alt+Meta+H', { isMac: true, isDesktop: false })).toBe(true)
    expect(isMacSystemChord('Shift+Meta+M', macDesktop)).toBe(false)
  })
})

describe('buildKeymap', () => {
  const defaults = { a: ['X', 'Mod+X'], b: ['Y'], c: [] as string[] }

  it('normalizes chords and indexes them', () => {
    const km = buildKeymap(defaults, {}, false)
    expect(km.bindings).toEqual({ a: ['X', 'Ctrl+X'], b: ['Y'], c: [] })
    expect(km.byChord.get('Ctrl+X')).toBe('a')
  })

  it('lays overrides over the defaults; an empty list unbinds', () => {
    const km = buildKeymap(defaults, { a: [], c: ['Z'] }, false)
    expect(km.bindings.a).toEqual([])
    expect(km.bindings.c).toEqual(['Z'])
    expect(km.byChord.has('X')).toBe(false)
  })

  it('lets an override\'s chord win over another command\'s default', () => {
    const km = buildKeymap(defaults, { c: ['Y'] }, false)
    expect(km.byChord.get('Y')).toBe('c')
    expect(km.bindings.b).toEqual([])
  })

  it('lets the later of two overrides win a shared chord', () => {
    expect(buildKeymap(defaults, { c: ['Z'], d: ['Z'] }, false).byChord.get('Z')).toBe('d')
    expect(buildKeymap(defaults, { d: ['Z'], c: ['Z'] }, false).bindings.d).toEqual([])
  })

  it('drops malformed, reserved and repeated chords', () => {
    const km = buildKeymap(defaults, { c: ['Hyper+Q', 'Escape', 'Q', 'q'] }, false)
    expect(km.bindings.c).toEqual(['Q'])
  })

  it('keeps an override for a command with no defaults', () => {
    expect(buildKeymap(defaults, { d: ['D'] }, false).byChord.get('D')).toBe('d')
  })
})

// The live keymap: the test environment is neither macOS nor the desktop
// shell, so the defaults are the web table.
describe('the live keymap', () => {
  beforeEach(() => resetAllChords())

  it('starts on the defaults with nothing stored', () => {
    expect(chordsFor('tool-rectangle')).toEqual(['R', '2'])
    expect(commandForChord('R')).toBe('tool-rectangle')
    expect(shortcutLabel('save')).toBe('Ctrl+S')
    expect(shortcutLabel('zoom-extents')).toBe('')
    expect(overrides()).toEqual({})
  })

  it('binds a key to a command that had none, and stores only that', () => {
    setChords('tool-dimension', ['d'])
    expect(chordsFor('tool-dimension')).toEqual(['D'])
    expect(commandForChord('D')).toBe('tool-dimension')
    expect(isCustomized('tool-dimension')).toBe(true)
    expect(overrides()).toEqual({ 'tool-dimension': ['D'] })
    expect(localStorage.getItem('hew.settings.keybindings')).toBe('{"tool-dimension":["D"]}')
  })

  it('moves a chord from its owner when another command takes it', () => {
    setChords('zoom-extents', ['O'])
    expect(commandForChord('O')).toBe('zoom-extents')
    expect(chordsFor('tool-orbit')).toEqual([])
    expect(isCustomized('tool-orbit')).toBe(true)
  })

  it('stores only the taker\'s override when a chord moves', () => {
    setChords('zoom-extents', ['O'])
    expect(overrides()).toEqual({ 'zoom-extents': ['O'] })
  })

  it('gives the chord back to its previous owner when the taker resets', () => {
    setChords('zoom-extents', ['O'])
    resetChords('zoom-extents')
    expect(chordsFor('tool-orbit')).toEqual(['O'])
    expect(chordsFor('zoom-extents')).toEqual([])
    expect(isCustomized('tool-orbit')).toBe(false)
    expect(overrides()).toEqual({})
  })

  it('takes a reassigned chord back when the previous owner resets', () => {
    setChords('zoom-extents', ['O'])
    resetChords('tool-orbit')
    expect(chordsFor('tool-orbit')).toEqual(['O'])
    expect(chordsFor('zoom-extents')).toEqual([])
    // Zoom Extents is back to its default (no key): nothing left to store.
    expect(overrides()).toEqual({})
  })

  it('still gives the chord back after the previous owner was edited', () => {
    setChords('zoom-extents', ['O'])
    setChords('tool-orbit', [...chordsFor('tool-orbit'), 'Y'])
    expect(chordsFor('tool-orbit')).toEqual(['Y'])
    resetChords('zoom-extents')
    expect(chordsFor('tool-orbit')).toEqual(['Y', 'O'])
    expect(commandForChord('O')).toBe('tool-orbit')
    resetChords('tool-orbit')
    expect(overrides()).toEqual({})
  })

  it('ends a three-step chain with the chord on the original owner', () => {
    // A steals O from B, C steals it from A, B is edited, then C and A reset.
    setChords('zoom-extents', ['O']) // A
    setChords('tool-dimension', ['O']) // C
    expect(commandForChord('O')).toBe('tool-dimension')
    setChords('tool-orbit', [...chordsFor('tool-orbit'), 'Y']) // B
    expect(chordsFor('tool-orbit')).toEqual(['Y'])
    resetChords('tool-dimension')
    expect(commandForChord('O')).toBe('zoom-extents')
    resetChords('zoom-extents')
    expect(commandForChord('O')).toBe('tool-orbit')
    expect(chordsFor('tool-orbit')).toEqual(['Y', 'O'])
  })

  it('moves a chord between two customized commands, and back on reset', () => {
    setChords('tool-dimension', ['D'])
    setChords('zoom-extents', ['D'])
    expect(commandForChord('D')).toBe('zoom-extents')
    expect(chordsFor('tool-dimension')).toEqual([])
    resetChords('zoom-extents')
    expect(commandForChord('D')).toBe('tool-dimension')
    expect(chordsFor('tool-dimension')).toEqual(['D'])
  })

  it('drops an override that is back to the defaults', () => {
    setChords('tool-rectangle', ['R'])
    expect(overrides()).toEqual({ 'tool-rectangle': ['R'] })
    setChords('tool-rectangle', ['R', '2'])
    expect(overrides()).toEqual({})
    expect(isCustomized('tool-rectangle')).toBe(false)
  })

  it('removes a default binding', () => {
    setChords('tool-select', ['Space'])
    expect(commandForChord('1')).toBeUndefined()
    expect(defaultChordsFor('tool-select')).toEqual(['Space', '1'])
  })

  it('Reset All restores every default', () => {
    setChords('tool-dimension', ['D'])
    setChords('tool-orbit', ['Ctrl+H'])
    resetAllChords()
    expect(overrides()).toEqual({})
    expect(chordsFor('tool-orbit')).toEqual(['O'])
    expect(commandForChord('D')).toBeUndefined()
  })

  it('notifies subscribers', () => {
    let calls = 0
    const off = subscribeKeymap(() => { calls++ })
    setChords('tool-dimension', ['D'])
    off()
    setChords('tool-dimension', [])
    expect(calls).toBe(1)
  })

  it('the rail prefers a key with no Ctrl/Alt/Cmd', () => {
    expect(shortcutLabel('open-library')).toBe('Ctrl+Shift+L')
    expect(railShortcutLabel('open-library')).toBe('Shift+L')
    setChords('open-library', ['Ctrl+Shift+L'])
    expect(railShortcutLabel('open-library')).toBe('Ctrl+Shift+L')
  })
})

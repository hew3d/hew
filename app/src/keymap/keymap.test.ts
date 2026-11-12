// @vitest-environment jsdom
import { beforeEach, describe, expect, it } from 'vitest'
import {
  buildKeymap,
  chordsFor,
  commandForChord,
  defaultChordsFor,
  isCustomized,
  isReservedChord,
  overrides,
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

  it('takes a reassigned chord back on reset', () => {
    setChords('zoom-extents', ['O'])
    resetChords('tool-orbit')
    expect(chordsFor('tool-orbit')).toEqual(['O'])
    expect(chordsFor('zoom-extents')).toEqual([])
    // Zoom Extents is back to its default (no key): nothing left to store.
    expect(overrides()).toEqual({})
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

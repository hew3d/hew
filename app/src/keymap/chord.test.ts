import { describe, expect, it } from 'vitest'
import {
  chordFromEvent,
  chordToAccelerator,
  chordToString,
  formatChordString,
  hasCommandModifier,
  normalizeChord,
  parseChord,
} from './chord'

type KeyInit = Partial<Pick<KeyboardEvent, 'key' | 'code' | 'ctrlKey' | 'altKey' | 'shiftKey' | 'metaKey'>>

function key(init: KeyInit): Pick<KeyboardEvent, 'key' | 'code' | 'ctrlKey' | 'altKey' | 'shiftKey' | 'metaKey'> {
  return { key: '', code: '', ctrlKey: false, altKey: false, shiftKey: false, metaKey: false, ...init }
}

describe('parseChord / chordToString', () => {
  it('canonicalizes modifier order and key case', () => {
    expect(normalizeChord('shift+ctrl+l', false)).toBe('Ctrl+Shift+L')
    expect(normalizeChord('Alt+Shift+Ctrl+x', false)).toBe('Ctrl+Alt+Shift+X')
    expect(normalizeChord('space', false)).toBe('Space')
    expect(normalizeChord('PageDown', false)).toBe('PageDown')
  })

  it('resolves Mod to Cmd on macOS and Ctrl elsewhere', () => {
    expect(normalizeChord('Mod+S', true)).toBe('Meta+S')
    expect(normalizeChord('Mod+S', false)).toBe('Ctrl+S')
  })

  it('folds Meta into Ctrl off macOS, keeps them apart on macOS', () => {
    expect(normalizeChord('Meta+K', false)).toBe('Ctrl+K')
    expect(normalizeChord('Ctrl+K', true)).toBe('Ctrl+K')
    expect(normalizeChord('Ctrl+Meta+K', true)).toBe('Ctrl+Meta+K')
  })

  it('reads the plus key and punctuation', () => {
    expect(normalizeChord('Ctrl++', false)).toBe('Ctrl++')
    expect(normalizeChord('+', false)).toBe('+')
    expect(normalizeChord('Mod+/', false)).toBe('Ctrl+/')
    expect(normalizeChord('Mod+\\', true)).toBe('Meta+\\')
  })

  it('refuses malformed chords', () => {
    expect(parseChord('', false)).toBeNull()
    expect(parseChord('Ctrl+', false)).toBeNull()
    expect(parseChord('Hyper+K', false)).toBeNull()
    expect(parseChord('Shift', false)).toBeNull()
    expect(parseChord('Ctrl+Control', false)).toBeNull()
  })

  it('round-trips', () => {
    const chord = parseChord('Ctrl+Alt+Shift+Meta+F5', true)!
    expect(chordToString(chord)).toBe('Ctrl+Alt+Shift+Meta+F5')
  })
})

describe('chordFromEvent', () => {
  it('reads a produced ASCII letter, uppercased', () => {
    expect(chordToString(chordFromEvent(key({ key: 'r', code: 'KeyR' }), false)!)).toBe('R')
    expect(chordToString(chordFromEvent(key({ key: 'L', code: 'KeyL', shiftKey: true }), false)!)).toBe('Shift+L')
  })

  it('follows the layout\'s letter over the physical key (AZERTY A on KeyQ)', () => {
    expect(chordToString(chordFromEvent(key({ key: 'a', code: 'KeyQ' }), false)!)).toBe('A')
  })

  it('falls back to the physical key when no ASCII letter was produced', () => {
    // An idle IME's full-width letter, or macOS Option producing a symbol.
    expect(chordToString(chordFromEvent(key({ key: 'ｌ', code: 'KeyL' }), false)!)).toBe('L')
    expect(chordToString(chordFromEvent(key({ key: '¬', code: 'KeyL', altKey: true }), true)!)).toBe('Alt+L')
  })

  it('reads digits, falling back to the physical digit key', () => {
    expect(chordToString(chordFromEvent(key({ key: '1', code: 'Digit1' }), false)!)).toBe('1')
    expect(chordToString(chordFromEvent(key({ key: '&', code: 'Digit1' }), false)!)).toBe('1')
    expect(chordToString(chordFromEvent(key({ key: '4', code: 'Numpad4' }), false)!)).toBe('4')
  })

  it('names Space and other keys by KeyboardEvent.key', () => {
    expect(chordToString(chordFromEvent(key({ key: ' ', code: 'Space' }), false)!)).toBe('Space')
    expect(chordToString(chordFromEvent(key({ key: 'PageDown', code: 'PageDown' }), false)!)).toBe('PageDown')
    expect(chordToString(chordFromEvent(key({ key: '/', code: 'Slash', ctrlKey: true }), false)!)).toBe('Ctrl+/')
  })

  it('is null for a modifier pressed alone or a dead key', () => {
    expect(chordFromEvent(key({ key: 'Shift', code: 'ShiftLeft', shiftKey: true }), false)).toBeNull()
    expect(chordFromEvent(key({ key: 'Control', code: 'ControlLeft', ctrlKey: true }), false)).toBeNull()
    expect(chordFromEvent(key({ key: 'Dead', code: 'BracketLeft' }), false)).toBeNull()
  })

  it('treats Cmd as Ctrl off macOS and as its own modifier on macOS', () => {
    expect(chordToString(chordFromEvent(key({ key: 's', code: 'KeyS', metaKey: true }), false)!)).toBe('Ctrl+S')
    expect(chordToString(chordFromEvent(key({ key: 's', code: 'KeyS', metaKey: true }), true)!)).toBe('Meta+S')
    expect(chordToString(chordFromEvent(key({ key: 's', code: 'KeyS', ctrlKey: true }), true)!)).toBe('Ctrl+S')
  })
})

describe('display', () => {
  it('uses macOS glyphs in Apple\'s order', () => {
    expect(formatChordString('Ctrl+Alt+Shift+Meta+S', true)).toBe('⌃⌥⇧⌘S')
    expect(formatChordString('Meta+/', true)).toBe('⌘/')
  })

  it('spells modifiers out elsewhere', () => {
    expect(formatChordString('Ctrl+Shift+S', false)).toBe('Ctrl+Shift+S')
    expect(formatChordString('Space', false)).toBe('Spc')
    expect(formatChordString('PageDown', false)).toBe('Page Down')
  })
})

describe('native accelerators', () => {
  it('maps a chord to Tauri\'s accelerator syntax', () => {
    expect(chordToAccelerator(parseChord('Shift+Meta+S', true)!)).toBe('Shift+CmdOrCtrl+S')
    expect(chordToAccelerator(parseChord('Ctrl+Alt+K', true)!)).toBe('Ctrl+Alt+K')
  })

  it('refuses a bare or Shift-only chord (it would fire while typing)', () => {
    expect(chordToAccelerator(parseChord('R', true)!)).toBeNull()
    expect(chordToAccelerator(parseChord('Shift+L', true)!)).toBeNull()
    expect(hasCommandModifier(parseChord('Shift+L', true)!)).toBe(false)
  })
})

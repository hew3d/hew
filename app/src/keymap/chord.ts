/**
 * Key chords: the parse / format / match vocabulary of the user-configurable
 * keymap (`keymap.ts`).
 *
 * A chord is one keystroke with its modifiers. Its canonical string form is
 * what the defaults table and the persisted overrides store:
 *
 *   modifiers in the fixed order Ctrl, Alt, Shift, Meta, then the key —
 *   "Ctrl+Shift+L", "Meta+K", "Shift+L", "Space", "PageDown", "1", "/".
 *
 * `Mod` is accepted on input as the platform's primary modifier (Meta on
 * macOS, Ctrl elsewhere) so one defaults table reads true everywhere; it is
 * resolved at parse time and never stored.
 *
 * Key resolution follows the rules the hard-coded handlers used
 * (docs/LOCALIZATION.md "Keyboard and input methods"): a letter is the ASCII
 * letter the keystroke produced, with the physical key as the fallback
 * (`shortcutLetter`), so AZERTY and Dvorak users press the key cap the UI
 * shows and an idle IME still works; a digit likewise prefers the produced
 * digit and falls back to the physical DigitN key. Every other key is its
 * `KeyboardEvent.key` value.
 *
 * On Windows and Linux the Meta (Windows / Super) key folds into Ctrl, as
 * the old `metaKey || ctrlKey` test did; on macOS Ctrl and Cmd are distinct
 * modifiers, as in SketchUp for Mac.
 */

import { shortcutLetter } from '../input/keyboard'

export interface Chord {
  /** Canonical key token: "A"–"Z", "0"–"9", a named key ("Space",
   *  "PageDown", "Delete", "F5"), or a single punctuation character. */
  key: string
  ctrl: boolean
  alt: boolean
  shift: boolean
  meta: boolean
}

/** Keys that are modifiers on their own: pressing one is never a chord. */
const MODIFIER_KEYS: ReadonlySet<string> = new Set([
  // oxlint-disable-next-line lingui/no-unlocalized-strings -- KeyboardEvent.key identifiers
  'Shift', 'Control', 'Alt', 'Meta', 'AltGraph', 'CapsLock', 'OS', 'Hyper', 'Super', 'Fn', 'FnLock',
])

/** Turns a key token into its canonical form (case, the space bar). */
function canonicalKey(raw: string): string {
  // oxlint-disable-next-line lingui/no-unlocalized-strings -- KeyboardEvent.key identifier
  if (raw === ' ' || raw.toLowerCase() === 'space') return 'Space'
  if (raw.length === 1) return raw.toUpperCase()
  return raw
}

/**
 * Parses a chord string. `Mod` resolves to Meta on macOS and Ctrl
 * elsewhere; on Windows/Linux an explicit `Meta` folds into Ctrl. Returns
 * `null` for anything malformed (empty, unknown modifier, no key).
 */
export function parseChord(text: string, isMac: boolean): Chord | null {
  // "Ctrl++" names the plus key: split on "+" only where it separates parts.
  const parts = text === '+' ? ['+'] : text.split(/\+(?!$)/)
  const chord: Chord = { key: '', ctrl: false, alt: false, shift: false, meta: false }
  for (let i = 0; i < parts.length; i++) {
    const part = parts[i]
    if (i === parts.length - 1) {
      // A named key never contains "+": "Ctrl+" is a modifier missing its key.
      if (part === '' || (part.length > 1 && part.includes('+'))) return null
      chord.key = canonicalKey(part)
      break
    }
    switch (part.toLowerCase()) {
      case 'ctrl': case 'control': chord.ctrl = true; break
      case 'alt': case 'option': chord.alt = true; break
      case 'shift': chord.shift = true; break
      case 'meta': case 'cmd': case 'command':
        if (isMac) chord.meta = true
        else chord.ctrl = true
        break
      case 'mod':
        if (isMac) chord.meta = true
        else chord.ctrl = true
        break
      default: return null
    }
  }
  if (MODIFIER_KEYS.has(chord.key)) return null
  return chord
}

/** The canonical string for a chord — the form stored and compared. */
export function chordToString(chord: Chord): string {
  let out = ''
  // oxlint-disable lingui/no-unlocalized-strings -- canonical chord tokens, not copy
  if (chord.ctrl) out += 'Ctrl+'
  if (chord.alt) out += 'Alt+'
  if (chord.shift) out += 'Shift+'
  if (chord.meta) out += 'Meta+'
  // oxlint-enable lingui/no-unlocalized-strings
  return out + chord.key
}

/** Normalizes a chord string to its canonical form, or `null` if malformed. */
export function normalizeChord(text: string, isMac: boolean): string | null {
  const chord = parseChord(text, isMac)
  return chord === null ? null : chordToString(chord)
}

/**
 * The chord a keydown event stands for, or `null` for a bare modifier
 * press (or an event with no usable key).
 */
export function chordFromEvent(
  ev: Pick<KeyboardEvent, 'key' | 'code' | 'ctrlKey' | 'altKey' | 'shiftKey' | 'metaKey'>,
  isMac: boolean,
): Chord | null {
  if (ev.key === undefined || ev.key === '' || MODIFIER_KEYS.has(ev.key)) return null
  let key: string
  const letter = shortcutLetter(ev)
  if (/^[a-z]$/.test(letter)) {
    key = letter.toUpperCase()
  } else if (/^[0-9]$/.test(ev.key)) {
    key = ev.key
  } else {
    const digit = /^(?:Digit|Numpad)([0-9])$/.exec(ev.code ?? '')
    if (digit !== null) {
      key = digit[1]
    } else {
      // A dead key (Option+E on macOS) or an unidentified one names nothing
      // usable once the letter fallback above has failed.
      // oxlint-disable-next-line lingui/no-unlocalized-strings -- KeyboardEvent.key identifiers
      if (ev.key === 'Dead' || ev.key === 'Unidentified' || ev.key === 'Process') return null
      key = canonicalKey(ev.key)
    }
  }
  return {
    key,
    ctrl: ev.ctrlKey || (!isMac && ev.metaKey),
    alt: ev.altKey,
    shift: ev.shiftKey,
    meta: isMac && ev.metaKey,
  }
}

/** True when the chord carries Ctrl, Alt or Meta (Shift alone does not count:
 *  a shifted letter is still typed text in a field). */
export function hasCommandModifier(chord: Chord): boolean {
  return chord.ctrl || chord.alt || chord.meta
}

/* oxlint-disable lingui/no-unlocalized-strings -- keyboard key cap names and glyphs, not translated (docs/LOCALIZATION.md) */
const KEY_DISPLAY: Record<string, string> = {
  Space: 'Spc',
  PageDown: 'Page Down',
  PageUp: 'Page Up',
  Delete: 'Del',
  Backspace: '⌫',
  Escape: 'Esc',
  ArrowUp: '↑',
  ArrowDown: '↓',
  ArrowLeft: '←',
  ArrowRight: '→',
  Enter: '↵',
  Tab: '⇥',
  Home: 'Home',
  End: 'End',
  Insert: 'Ins',
}

/**
 * How a chord is shown: macOS glyphs in Apple's order with no separator
 * ("⌃⌥⇧⌘S"), "Ctrl+Alt+Shift+S" elsewhere.
 */
export function formatChord(chord: Chord, isMac: boolean): string {
  const key = KEY_DISPLAY[chord.key] ?? chord.key
  if (isMac) {
    return (chord.ctrl ? '⌃' : '') + (chord.alt ? '⌥' : '') + (chord.shift ? '⇧' : '') + (chord.meta ? '⌘' : '') + key
  }
  return (chord.ctrl ? 'Ctrl+' : '') + (chord.alt ? 'Alt+' : '') + (chord.shift ? 'Shift+' : '') + key
}
/* oxlint-enable lingui/no-unlocalized-strings */

/** `formatChord` over a chord string; an unparseable string shows as-is. */
export function formatChordString(text: string, isMac: boolean): string {
  const chord = parseChord(text, isMac)
  return chord === null ? text : formatChord(chord, isMac)
}

/**
 * The accelerator string Tauri's native menu understands for a chord
 * ("Shift+CmdOrCtrl+S"), or `null` for a chord the menu must not carry: a
 * bare key or a Shift-only chord would fire even while typing in a text
 * field. Used only on macOS, where the native menu bar is attached.
 */
export function chordToAccelerator(chord: Chord): string | null {
  if (!hasCommandModifier(chord)) return null
  const parts: string[] = []
  // oxlint-disable lingui/no-unlocalized-strings -- Tauri accelerator tokens
  if (chord.shift) parts.push('Shift')
  if (chord.ctrl) parts.push('Ctrl')
  if (chord.alt) parts.push('Alt')
  if (chord.meta) parts.push('CmdOrCtrl')
  // oxlint-enable lingui/no-unlocalized-strings
  parts.push(chord.key)
  return parts.join('+')
}

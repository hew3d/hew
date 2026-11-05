/**
 * The keymap: which chords run which commands, after the person's own
 * changes (Settings ▸ Controls) are laid over the defaults (`defaults.ts`).
 *
 * Only the overrides are persisted — `{ [commandId]: chord[] }`, where an
 * empty list means "deliberately unbound" — so a later release that changes
 * or adds a default reaches everyone who never touched that command.
 * Persistence and cross-window sync (the Settings window is its own webview
 * on the desktop) reuse the View settings' `makeSetting`: localStorage under
 * `hew.settings.keybindings`, plus the `settings-changed` Tauri event.
 *
 * Every consumer reads through here: the keydown dispatcher (`dispatch.ts`),
 * the tool rail, both menu bars and the command palette, so what is shown is
 * always what works.
 */

import { isTauri } from '../io/fileHost'
import { isMac } from '../platform'
import { makeSetting } from '../settings/view'
import { formatChordString, normalizeChord } from './chord'
import { defaultBindings, type Bindings, type KeymapPlatform } from './defaults'

export type Overrides = Readonly<Record<string, readonly string[]>>

/** Commands whose keys are fixed: Escape, Enter, Tab and the arrows drive
 *  the tools' own gestures (cancel, commit, axis lock) and the measurement
 *  box; a command bound to one would steal it from every tool. */
export function isReservedChord(chord: string): boolean {
  const key = chord.slice(chord.lastIndexOf('+', chord.length - 2) + 1)
  // oxlint-disable-next-line lingui/no-unlocalized-strings -- KeyboardEvent.key identifiers
  if (key === 'Escape' || key === 'Enter' || key === 'Tab') return true
  if (key.startsWith('Arrow')) {
    // Ctrl/Alt/Cmd + arrow is free; bare and Shift+arrow are the axis locks.
    return !/(^|\+)(Ctrl|Alt|Meta)\+/.test(chord)
  }
  return false
}

export interface Keymap {
  /** Effective bindings per command, in display order. */
  readonly bindings: Bindings
  /** Chord → command. */
  readonly byChord: ReadonlyMap<string, string>
}

/**
 * Lays `overrides` over `defaults`. Chords are normalized; malformed or
 * reserved ones are dropped. A chord claimed by an override wins over the
 * same chord in another command's defaults (the editor reassigns on
 * conflict, so this only matters for stale or hand-edited storage).
 */
export function buildKeymap(defaults: Bindings, overrides: Overrides, isMacPlatform: boolean): Keymap {
  const clean = (chords: readonly string[]): string[] => {
    const out: string[] = []
    for (const c of chords) {
      const n = normalizeChord(c, isMacPlatform)
      if (n !== null && !isReservedChord(n) && !out.includes(n)) out.push(n)
    }
    return out
  }
  const bindings: Record<string, string[]> = {}
  for (const [id, chords] of Object.entries(defaults)) {
    if (!Object.prototype.hasOwnProperty.call(overrides, id)) bindings[id] = clean(chords)
  }
  const overridden: Record<string, string[]> = {}
  for (const [id, chords] of Object.entries(overrides)) overridden[id] = clean(chords)

  const byChord = new Map<string, string>()
  for (const [id, chords] of Object.entries(overridden)) {
    for (const c of chords) if (!byChord.has(c)) byChord.set(c, id)
  }
  for (const [id, chords] of Object.entries(bindings)) {
    const kept = chords.filter((c) => !byChord.has(c))
    bindings[id] = kept
    for (const c of kept) byChord.set(c, id)
  }
  for (const [id, chords] of Object.entries(overridden)) {
    bindings[id] = chords.filter((c) => byChord.get(c) === id)
  }
  return { bindings, byChord }
}

/** Two binding lists are the same set of chords in the same order. */
function sameChords(a: readonly string[], b: readonly string[]): boolean {
  return a.length === b.length && a.every((c, i) => c === b[i])
}

function parseOverrides(raw: unknown): Overrides | undefined {
  let value: unknown = raw
  if (typeof raw === 'string') {
    try {
      value = JSON.parse(raw)
    } catch {
      return undefined
    }
  }
  if (value === null || typeof value !== 'object' || Array.isArray(value)) return undefined
  const out: Record<string, string[]> = {}
  for (const [id, chords] of Object.entries(value as Record<string, unknown>)) {
    if (Array.isArray(chords) && chords.every((c) => typeof c === 'string')) out[id] = chords as string[]
  }
  return out
}

const platform: KeymapPlatform = { isMac, isDesktop: isTauri }
const DEFAULTS: Bindings = defaultBindings(platform)

const overridesSetting = makeSetting<Overrides>(
  // oxlint-disable-next-line lingui/no-unlocalized-strings -- storage key
  'hew.settings.keybindings',
  // oxlint-disable-next-line lingui/no-unlocalized-strings -- broadcast payload key
  'keybindings',
  {},
  parseOverrides,
  (value) => JSON.stringify(value),
)

let cachedFor: Overrides | null = null
let cached: Keymap = buildKeymap(DEFAULTS, {}, isMac)

/** The effective keymap right now. */
export function currentKeymap(): Keymap {
  const overrides = overridesSetting.get()
  if (overrides !== cachedFor) {
    cached = buildKeymap(DEFAULTS, overrides, isMac)
    cachedFor = overrides
  }
  return cached
}

/** Every command's default chords on this platform. */
export function defaultKeymapBindings(): Bindings {
  return currentDefaults().bindings
}

/** The default chords for a command on this platform. */
export function defaultChordsFor(id: string): readonly string[] {
  return currentDefaults().bindings[id] ?? []
}

let defaultsKeymap: Keymap | null = null
function currentDefaults(): Keymap {
  defaultsKeymap ??= buildKeymap(DEFAULTS, {}, isMac)
  return defaultsKeymap
}

/** The chords a command answers to now. */
export function chordsFor(id: string): readonly string[] {
  return currentKeymap().bindings[id] ?? []
}

/** The command a chord runs, if any. */
export function commandForChord(chord: string): string | undefined {
  return currentKeymap().byChord.get(chord)
}

/** The shortcut to show next to a command (its first binding), formatted
 *  for this platform; '' when it has none. */
export function shortcutLabel(id: string): string {
  const first = chordsFor(id)[0]
  return first === undefined ? '' : formatChordString(first, isMac)
}

/** Like `shortcutLabel`, but prefers a key with no Ctrl/Alt/Cmd — what the
 *  tool rail shows (⇧L for the Library, while the menus show ⇧⌘L). */
export function railShortcutLabel(id: string): string {
  const chords = chordsFor(id)
  const bare = chords.find((c) => !/(^|\+)(Ctrl|Alt|Meta)\+/.test(c)) ?? chords[0]
  return bare === undefined ? '' : formatChordString(bare, isMac)
}

/** True when the person changed this command's keys. */
export function isCustomized(id: string): boolean {
  return !sameChords(chordsFor(id), defaultChordsFor(id))
}

/** Every command id that has a default or a custom binding. */
export function boundCommandIds(): string[] {
  return Object.keys(currentKeymap().bindings)
}

/** Replaces a command's chords. Chords already bound elsewhere are taken
 *  from that command (the editor asks first). Matching the defaults again
 *  clears the override. */
export function setChords(id: string, chords: readonly string[]): void {
  const normalized = chords.map((c) => normalizeChord(c, isMac)).filter((c): c is string => c !== null)
  const next: Record<string, readonly string[]> = { ...overridesSetting.get() }
  for (const c of normalized) {
    const owner = commandForChord(c)
    if (owner !== undefined && owner !== id) {
      next[owner] = chordsFor(owner).filter((x) => x !== c)
    }
  }
  next[id] = normalized
  // An override identical to the defaults is no override: dropping it keeps
  // the stored map minimal. (The defaults never conflict among themselves,
  // and every other override was just stripped of these chords, so the
  // effective keymap is unchanged by the drop.)
  for (const key of Object.keys(next)) {
    if (sameChords(next[key], defaultChordsFor(key))) delete next[key]
  }
  overridesSetting.set(next)
}

/** Restores one command's default chords (and takes them back from any
 *  command they were reassigned to). */
export function resetChords(id: string): void {
  setChords(id, defaultChordsFor(id))
}

/** Restores every default. */
export function resetAllChords(): void {
  overridesSetting.set({})
}

/** Notified on every keymap change, from this window or another one. */
export function subscribeKeymap(cb: () => void): () => void {
  return overridesSetting.subscribe(() => cb())
}

let recording = false

/** Settings ▸ Controls is recording a new chord: the next keystroke,
 *  Escape included, belongs to it (the Windows settings page leaves its own
 *  Escape-to-close alone meanwhile). */
export function isRecordingChord(): boolean {
  return recording
}

export function setRecordingChord(on: boolean): void {
  recording = on
}

/** Escape hatch for tests: the raw persisted overrides. */
export function overrides(): Overrides {
  return overridesSetting.get()
}

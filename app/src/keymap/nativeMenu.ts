/**
 * The keymap's view of the macOS native menu bar (`shells/tauri/src-tauri/
 * src/main.rs`), the one platform where that menu is attached.
 *
 * On macOS WebKit hands every keystroke to the page before AppKit consults
 * the menu's key equivalents, and a key the page claims (`preventDefault`)
 * never reaches the menu. The split stays what it was before the keymap:
 *
 * - A Cmd/Ctrl/Option chord of a command that has a native item is left to
 *   that item, whose accelerator the app keeps in sync with the keymap
 *   (`set_menu_accelerators`). The menu flashes its title, as macOS users
 *   expect, and the item's own gating (a greyed Group) applies.
 * - The Edit items in `JS_FIRST` are claimed by the page: their native items
 *   keep the standard Cmd+Z/X/C/V/A so a focused text field, which has no
 *   editing key equivalents of its own in WebKit, still gets them through
 *   the menu (App.tsx forwards those fires to the field).
 * - Bare keys never become native accelerators: the menu would fire them
 *   while typing in a text field.
 */

import { chordToAccelerator, hasCommandModifier, parseChord } from './chord'

// oxlint-disable lingui/no-unlocalized-strings -- command ids are identifiers

/** Commands the page claims even on the macOS desktop, with fixed native
 *  accelerators (see the module comment). */
export const JS_FIRST: ReadonlySet<string> = new Set([
  'undo',
  'redo',
  'edit-cut',
  'edit-copy',
  'edit-paste',
  'edit-paste-in-place',
  'edit-select-all',
  'edit-select-none',
  'edit-delete',
])

/**
 * The chords the fixed native accelerators of the `JS_FIRST` Edit items
 * carry, by command (the standard Cmd+Z/X/C/V/A and their Shift variants).
 * The shell never moves them (`set_menu_accelerators` skips `JS_FIRST`), so
 * on the macOS desktop they are pinned: another command cannot take one (two
 * native items would share a key), and the command itself cannot lose it
 * (the menu would keep firing it). `nativeMenuParity.test.ts` checks the
 * table against main.rs.
 */
export const PINNED_NATIVE_CHORDS: Readonly<Record<string, string>> = {
  'undo': 'Meta+Z',
  'redo': 'Shift+Meta+Z',
  'edit-cut': 'Meta+X',
  'edit-copy': 'Meta+C',
  'edit-paste': 'Meta+V',
  'edit-paste-in-place': 'Shift+Meta+V',
  'edit-select-all': 'Meta+A',
  'edit-select-none': 'Shift+Meta+A',
}

/** macOS system chords (Quit, Hide, Hide Others, Minimize) that AppKit's predefined key
 *  equivalents already own; no command may take them on a Mac. */
export const MAC_SYSTEM_CHORDS: ReadonlySet<string> = new Set(['Meta+Q', 'Meta+H', 'Alt+Meta+H', 'Meta+M'])

/**
 * The commands the native menu dispatches (`on_menu_event`'s match arms in
 * main.rs, kept in step by `nativeMenuParity.test.ts`). A command outside
 * this set has no native item, so even a Cmd-combo of it runs in the page.
 */
export const NATIVE_COMMANDS: ReadonlySet<string> = new Set([
  'close', 'draw-3d-text', 'edit-copy', 'edit-cut', 'edit-delete',
  'edit-delete-guides', 'edit-explode', 'edit-group', 'edit-intersect',
  'edit-invert-selection', 'edit-make-component', 'edit-make-unique',
  'edit-paste', 'edit-paste-in-place', 'edit-place-copy', 'edit-select-all',
  'edit-select-none', 'edit-subtract', 'edit-ungroup', 'edit-union', 'export',
  'import', 'import-image', 'make-solid', 'model-health', 'new', 'open',
  'open-guide', 'open-on-phone', 'open-palette', 'open-settings', 'print',
  'purge-unused', 'redo', 'report-bug', 'reset-axes', 'revert-to-saved',
  'save', 'save-as', 'save-to-library', 'save-to-library-doc', 'scenes-add',
  'scenes-next', 'scenes-previous', 'scenes-transitions', 'scenes-update',
  'sent-reports',
  'edit-flip-blue', 'edit-flip-green', 'edit-flip-red',
  'edit-paste-independent', 'edit-paste-linked', 'edit-select-all-copies',
  'edit-unlink', 'edit-unlink-all', 'toggle-part-axes', 'tool-flip',
  'toggle-axes', 'toggle-changes', 'toggle-compact-rail', 'toggle-components',
  'toggle-debug-log', 'toggle-dock', 'toggle-fly', 'toggle-grid',
  'toggle-guides', 'toggle-library', 'toggle-materials', 'toggle-model-info',
  'toggle-object-info', 'toggle-parallel-projection', 'toggle-scenes',
  'toggle-section-active', 'toggle-tags', 'toggle-tray', 'toggle-wireframe',
  'toggle-xray', 'tool-arc', 'tool-axes', 'tool-circle', 'tool-dimension',
  'tool-edit-vertex', 'tool-follow-me', 'tool-line', 'tool-look-around',
  'tool-move', 'tool-offset', 'tool-orbit', 'tool-paint', 'tool-pan',
  'tool-polygon', 'tool-position-camera', 'tool-position-texture',
  'tool-protractor', 'tool-pushpull', 'tool-rectangle', 'tool-rotate',
  'tool-scale', 'tool-section-plane', 'tool-select', 'tool-slice',
  'tool-tape-measure', 'tool-text', 'tool-walk', 'tool-zoom',
  'tool-zoom-window', 'undo', 'view-back', 'view-bottom', 'view-front',
  'view-iso', 'view-left', 'view-right', 'view-top', 'zoom-extents',
])

/**
 * Native menu ids' command ids that differ from the keymap's own: the
 * Library item dispatches the `toggle-library` alias of `open-library`.
 * Every other native item's command id is its keymap id.
 */
export const NATIVE_ALIASES: Readonly<Record<string, string>> = {
  'open-library': 'toggle-library',
}

// oxlint-enable lingui/no-unlocalized-strings

/** The chord a command's native item carries, given its keymap chords: the
 *  first one with a Cmd/Ctrl/Option modifier, or `null` for none. */
export function nativeChordFor(chords: readonly string[], isMac: boolean): string | null {
  for (const c of chords) {
    const chord = parseChord(c, isMac)
    if (chord !== null && hasCommandModifier(chord)) return c
  }
  return null
}

/**
 * The accelerator map to push to the native menu: command id (as the menu
 * dispatches it) → Tauri accelerator, or `null` to clear one. Covers every
 * bound or customized command except `JS_FIRST`; the shell ignores ids it
 * has no item for.
 */
export function nativeAccelerators(
  bindings: Readonly<Record<string, readonly string[]>>,
  isMac: boolean,
): Record<string, string | null> {
  const out: Record<string, string | null> = {}
  // A native command with no chord at all has no entry in `bindings`, yet its
  // item may still carry a key it had earlier: clear it explicitly.
  for (const id of NATIVE_COMMANDS) out[id] = null
  for (const [id, chords] of Object.entries(bindings)) {
    if (JS_FIRST.has(id)) continue
    const native = nativeChordFor(chords, isMac)
    const chord = native === null ? null : parseChord(native, isMac)
    const accel = chord === null ? null : chordToAccelerator(chord)
    out[NATIVE_ALIASES[id] ?? id] = accel
  }
  for (const id of JS_FIRST) delete out[id]
  return out
}

/** Command id → the chord its native item carries right now. Seeded with
 *  the defaults, which the shell builds the menu with; replaced by each
 *  `set_menu_accelerators` answer. */
const carried = new Map<string, string>()
let seeded = false

/** The chord a command's native item carries right now, or `null`. */
export function nativeChord(id: string): string | null {
  return carried.get(id) ?? null
}

/**
 * Keeps the macOS native menu's accelerators equal to the keymap: seeds
 * `carried` from the defaults, pushes the effective keymap now and on every
 * change, and records which chords the menu actually took. Returns the
 * unsubscribe. Only call it where the native menu bar is attached.
 */
export function startNativeMenuSync(deps: {
  isMac: boolean
  defaults: Readonly<Record<string, readonly string[]>>
  current: () => Readonly<Record<string, readonly string[]>>
  subscribe: (cb: () => void) => () => void
  invoke: (accelerators: Record<string, string | null>) => Promise<string[]>
}): () => void {
  if (!seeded) {
    for (const [id, chords] of Object.entries(deps.defaults)) {
      if (JS_FIRST.has(id) || !NATIVE_COMMANDS.has(NATIVE_ALIASES[id] ?? id)) continue
      const chord = nativeChordFor(chords, deps.isMac)
      if (chord !== null) carried.set(id, chord)
    }
    seeded = true
  }
  let generation = 0
  const push = () => {
    const gen = ++generation
    const bindings = deps.current()
    const accelerators = nativeAccelerators(bindings, deps.isMac)
    deps.invoke(accelerators).then((applied) => {
      if (gen !== generation) return
      const took = new Set(applied)
      const keymapIds = new Map<string, string>()
      for (const [alias, native] of Object.entries(NATIVE_ALIASES)) keymapIds.set(native, alias)
      for (const nativeId of Object.keys(accelerators)) {
        const id = keymapIds.get(nativeId) ?? nativeId
        const chord = nativeChordFor(bindings[id] ?? [], deps.isMac)
        if (took.has(nativeId) && chord !== null) carried.set(id, chord)
        else carried.delete(id)
      }
    }).catch((err) => console.warn('[keymap] native menu accelerator sync failed', err))
  }
  push()
  return deps.subscribe(push)
}

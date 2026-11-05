/**
 * The default key bindings — what every command answers to before anyone
 * customizes anything (Settings ▸ Controls).
 *
 * Keys are command ids: the `menuActionRef` payload strings of App.tsx
 * (`tool-line`, `save`, `zoom-extents`, …), the same id space the native
 * menu, the in-app menu bar and the command palette dispatch through.
 *
 * The scheme is SketchUp's (verified against the official SketchUp 2024
 * Windows Quick Reference Card): the bare-letter tool keys on every
 * platform, macOS included, plus the camera tools' O / H / Z. Tools
 * SketchUp ships without a key (Polygon, Follow Me, Protractor, Slice,
 * Section Plane, Edit Vertex, Dimension, Text, …) have none here either.
 * The digits 1–6 are Hew's own quick picks for the six most used tools.
 *
 * The macOS desktop adds SketchUp for Mac's Cmd-combos as second bindings
 * (Cmd+K Rectangle, Cmd+D Tape Measure, …); they are left out of the web
 * build, where most of them are browser keys (Cmd+R reload, Cmd+L the
 * address bar, Cmd+D bookmark).
 *
 * Bindings are chord strings (`chord.ts`); `Mod` is Cmd on macOS and Ctrl
 * elsewhere. The first binding of a command is the one menus, the tool rail
 * and the palette show, so a bare key comes before its Cmd-combo.
 */

export interface KeymapPlatform {
  isMac: boolean
  /** Running in the desktop shell (Tauri), not a browser tab. */
  isDesktop: boolean
}

export type Bindings = Readonly<Record<string, readonly string[]>>

// oxlint-disable lingui/no-unlocalized-strings -- command ids and chord strings are identifiers, not copy
export function defaultBindings({ isMac, isDesktop }: KeymapPlatform): Bindings {
  const macDesktop = isMac && isDesktop
  /** A second, macOS-desktop-only binding. */
  const mac = (chord: string): string[] => (macDesktop ? [chord] : [])
  return {
    // ---- Tools ----
    'tool-select': ['Space', '1'],
    'tool-line': ['L', ...mac('Mod+L')],
    'tool-rectangle': ['R', '2', ...mac('Mod+K')],
    'tool-circle': ['C'],
    'tool-arc': ['A', ...mac('Mod+J')],
    'tool-pushpull': ['P', '3', ...mac('Mod+=')],
    'tool-offset': ['F'],
    'tool-move': ['M', '4', ...mac('Mod+0')],
    'tool-rotate': ['Q', '5', ...mac('Mod+8')],
    'tool-scale': ['S', '6', ...mac('Mod+9')],
    'tool-tape-measure': ['T', ...mac('Mod+D')],
    'tool-paint': ['B'],
    'tool-orbit': ['O', ...mac('Mod+B')],
    'tool-pan': ['H', ...mac('Mod+R')],
    'tool-zoom': ['Z', ...mac('Mod+\\')],

    // ---- File ----
    'new': ['Mod+N'],
    'open': ['Mod+O'],
    'save': ['Mod+S'],
    'save-as': ['Mod+Shift+S'],
    'print': ['Mod+P'],
    // A browser tab cannot be closed from the page; only the desktop
    // shell's window close goes through this command.
    ...(isDesktop ? { 'close': ['Mod+W'] } : {}),

    // ---- Edit ----
    'undo': ['Mod+Z'],
    'redo': ['Mod+Shift+Z'],
    'edit-cut': ['Mod+X'],
    'edit-copy': ['Mod+C'],
    'edit-paste': ['Mod+V'],
    'edit-paste-in-place': ['Mod+Shift+V'],
    // The Mac keyboard's delete key is Backspace; elsewhere Del leads.
    'edit-delete': isMac ? ['Backspace', 'Delete'] : ['Delete', 'Backspace'],
    'edit-select-all': ['Mod+A'],
    // SketchUp for Mac's binding: the Windows Ctrl+T is the browser's new
    // tab key, so the Shift chord is the one that works everywhere.
    'edit-select-none': ['Mod+Shift+A'],
    'edit-group': ['Mod+G'],
    'edit-ungroup': ['Mod+Shift+G'],

    // ---- Window ----
    'toggle-model-info': ['Mod+Shift+I'],
    'toggle-object-info': ['Mod+Shift+O'],
    'toggle-materials': ['Mod+Shift+C'],
    'toggle-components': ['Mod+Shift+M'],
    'toggle-tags': ['Mod+Shift+T'],
    // Bare ⇧L is the Library design mock's own binding; the menus show the
    // Cmd-combo, which a native menu can carry.
    'open-library': ['Mod+Shift+L', 'Shift+L'],

    // ---- Scenes (SketchUp's own defaults) ----
    'scenes-next': ['PageDown'],
    'scenes-previous': ['PageUp'],

    // ---- App ----
    'open-settings': ['Mod+,'],
    // `/` is the one key free of SketchUp's scheme on every platform.
    // Ctrl+K stays as an alias for the muscle memory of everyone who used it
    // first; on macOS Cmd+K is Rectangle's classic SketchUp key instead.
    'open-palette': ['Mod+/', ...(isMac ? [] : ['Ctrl+K'])],
  }
}
// oxlint-enable lingui/no-unlocalized-strings

/**
 * Native-menu parity — a test-time drift check between the app's tool
 * registry and the Tauri shell's hand-built native menu.
 *
 * The two live in different languages with no shared source of truth: the
 * registry (`tools/toolRegistry.ts`) and `TOOL_MENU_IDS` (App.tsx) are
 * TypeScript, while the macOS menu is built item by item in
 * `shells/tauri/src-tauri/src/main.rs`. Follow Me shipped wired into the
 * dispatcher but missing from the native Tools submenu — exactly the drift
 * this test now catches by scraping the Rust source for each menu id
 * (`check_item(handle, …, "tool-…", …)`) and for the id's dispatch arm.
 *
 * A source scrape is deliberately chosen over nothing: a genuine
 * compile-time check would need the id list shared across the Rust/TS
 * boundary (codegen), which isn't worth the machinery for a short, stable
 * list. If the shell file moves, update MAIN_RS below — the failure message
 * says so.
 */

import { readFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { describe, expect, it } from 'vitest'
import { TOOL_MENU_IDS } from './App'
import { TOOLS } from './tools/toolRegistry'
import { dockVerbsFor } from './panels/dockLogic'
import { paletteEntries } from './palette/registry'
import { NATIVE_MENU_LABELS } from './i18n/nativeMenu'
import { PSEUDO_LOCALE, SHIPPED_LOCALES } from './i18n/locales'
import { NATIVE_ALIASES, NATIVE_COMMANDS, JS_FIRST, PINNED_NATIVE_CHORDS } from './keymap/nativeMenu'
import { chordToAccelerator, parseChord } from './keymap/chord'

const MAIN_RS = resolve(
  dirname(fileURLToPath(import.meta.url)),
  '../../shells/tauri/src-tauri/src/main.rs',
)
const MENU_BAR_TSX = resolve(dirname(fileURLToPath(import.meta.url)), './panels/MenuBar.tsx')

describe('native menu parity', () => {
  const source = readFileSync(MAIN_RS, 'utf8')

  it('TOOL_MENU_IDS covers every registry tool', () => {
    const missing = TOOLS.filter((t) => TOOL_MENU_IDS[t] === undefined)
    expect(missing, 'tools with no native menu id mapping').toEqual([])
  })

  it('every mapped menu id is built as a native menu item', () => {
    // check_item's first string literal is the item id; an id that never
    // appears as a literal was never built into any submenu.
    const missing = Object.entries(TOOL_MENU_IDS)
      .filter(([, id]) => !source.includes(`"${id}"`))
      .map(([tool, id]) => `${tool} (${id})`)
    expect(
      missing,
      `native menu items missing from ${MAIN_RS} — add the check_item + submenu entry (and its dispatch arm)`,
    ).toEqual([])
  })

  it('every mapped menu id has a dispatch arm forwarding to the app', () => {
    // The shell maps native menu ids onto menuActionRef payloads in a match
    // block (`"tool-…" => "tool-…"`). An item without an arm renders but
    // does nothing when clicked.
    const missing = Object.values(TOOL_MENU_IDS).filter(
      (id) => !new RegExp(`"${id}"\\s*=>\\s*"`).test(source),
    )
    expect(missing, 'native menu ids with no dispatch arm').toEqual([])
  })

  it('every built menu item is actually attached to a submenu', () => {
    // Building the item is not showing it: `let tool_x = check_item(…)?;`
    // creates a floating item that only a later `.item(&tool_x)` puts into
    // a SubmenuBuilder chain. Dropping just the attachment line leaves the
    // id string (and the dispatch arm) in the file, so the two scrapes
    // above stay green while the item silently vanishes from the menu —
    // extract each id's binding variable and require its attachment.
    //
    // Most tool ids bind via `check_item` (the active-tool radio group);
    // Zoom Window is the one exception — a plain, non-checkable
    // `MenuItemBuilder::with_id(…)` build, because that mode always springs
    // back to Select right after its drag and a persistent checkmark would
    // be misleading (matches the web MenuBar's plain item for it). Either
    // binding shape is accepted here; both still must be attached.
    const missing: string[] = []
    for (const [tool, id] of Object.entries(TOOL_MENU_IDS)) {
      const binding =
        new RegExp(`let\\s+(\\w+)\\s*=\\s*check_item\\([^;]*?"${id}"`, 's').exec(source) ??
        new RegExp(`let\\s+(\\w+)\\s*=\\s*MenuItemBuilder::with_id\\("${id}"`, 's').exec(source)
      expect(binding, `no check_item/MenuItemBuilder binding found for ${tool} (${id})`).not.toBeNull()
      const variable = (binding as RegExpExecArray)[1]
      if (!source.includes(`.item(&${variable})`)) {
        missing.push(`${tool} (${id} -> ${variable})`)
      }
    }
    expect(missing, 'menu items built but never attached to a SubmenuBuilder chain').toEqual([])
  })

  it('offers no standalone Field of View item on the native menu (camera-playtest2.md §2 — removed, not hidden)', () => {
    // A positive absence check, not a loosened/dropped assertion — Kurt's
    // playtest call was to remove the item entirely (fov stays reachable
    // only through Zoom's typed entry and Shift-drag/wheel). Checks BOTH
    // the id (`cam-field-of-view`) and the human label, so a re-add under
    // either spelling fails this test.
    expect(source).not.toContain('cam-field-of-view')
    expect(source).not.toContain('Field of View')
  })
})

/**
 * "3D Text…" discoverability (playtest finding 1): it is a one-off dialog
 * action, not a toggleable tool, so it is deliberately outside
 * `TOOL_MENU_IDS`/`TOOLS`/the checks above — those are scoped to the
 * persistent, checkable tool set. It shipped reachable only from the
 * command palette (`registry.ts`); this pins it onto every surface a
 * sibling Draw-menu tool lives on, the same way the tool-parity suite
 * above pins toggleable tools.
 */
describe('3D Text one-off action parity (not a TOOL_REGISTRY tool)', () => {
  const mainRsSource = readFileSync(MAIN_RS, 'utf8')
  const menuBarSource = readFileSync(MENU_BAR_TSX, 'utf8')
  const id = 'draw-3d-text'

  it('is built as a native menu item, attached to a submenu, with a dispatch arm', () => {
    // Built via `gated_item` (not a bare MenuItemBuilder): 3D Text refuses
    // while an explode session is open (ExplodeSessionScope), so the native
    // item registers in the gated map for sync_menu_state to disable —
    // matching the web MenuBar/palette gating.
    const binding = new RegExp(
      // A plain, gated, or macOS-aware (`edit_item`) binding — Paste's is a
      // platform `if` that picks the native item on macOS.
      `let\\s+(\\w+)[^=]*=\\s*(?:if[^{]*\\{[^}]*\\}\\s*else\\s*\\{\\s*)?(?:MenuItemBuilder::with_id\\(\\s*"${id}"|(?:gated_item|edit_item)\\(\\s*handle,\\s*&mut \\w+,\\s*"${id}")`,
    ).exec(mainRsSource)
    expect(binding, `no menu-item binding (with_id or gated_item) found for ${id}`).not.toBeNull()
    const variable = (binding as RegExpExecArray)[1]
    expect(
      mainRsSource.includes(`.item(&${variable})`),
      `${id} (-> ${variable}) is built but never attached to a SubmenuBuilder chain`,
    ).toBe(true)
    expect(
      new RegExp(`"${id}"\\s*=>\\s*"`).test(mainRsSource),
      `${id} has no dispatch arm forwarding it to the app`,
    ).toBe(true)
  })

  it('is offered from the web MenuBar\'s Draw menu', () => {
    expect(menuBarSource.includes('3D Text…')).toBe(true)
    expect(menuBarSource.includes('onDrawText')).toBe(true)
  })

  it('is offered from the empty-selection dock\'s DRAW verb row', () => {
    const ids = dockVerbsFor('empty').map((v) => v.id)
    expect(ids).toContain(id)
  })

  it('is offered from the command palette', () => {
    const ids = paletteEntries().map((e) => e.id)
    expect(ids).toContain(id)
  })
})

describe('native menu parity — non-tool commands', () => {
  // Reset Axes (tool-parity §4) shipped as a web-only MenuBar item with a
  // direct callback prop — unreachable on macOS, which renders the native
  // menu exclusively and never falls back to the in-app MenuBar. Not a
  // TOOL_MENU_IDS entry (it isn't a tool radio-group member), so it needs
  // its own drift check, mirroring the three checks above: built as a
  // native item, attached to its submenu, and given a dispatch arm.
  const source = readFileSync(MAIN_RS, 'utf8')
  const NON_TOOL_MENU_ACTIONS: Record<string, string> = {
    'reset-axes': 'view-reset-axes',
    // File ▸ Print… (docs/design/printing.md §11) — ⌘P native accelerator.
    print: 'file-print',
    // Help ▸ Hew Help — opens the online user guide via the opener plugin.
    'open-guide': 'help-guide',
  }

  it('every mapped id is built as a native menu item (plain MenuItemBuilder — not checkable)', () => {
    const missing = Object.entries(NON_TOOL_MENU_ACTIONS)
      .filter(([, id]) => !source.includes(`"${id}"`))
      .map(([action, id]) => `${action} (${id})`)
    expect(missing, `native menu items missing from ${MAIN_RS}`).toEqual([])
  })

  it('every mapped id has a dispatch arm forwarding to the app', () => {
    const missing = Object.entries(NON_TOOL_MENU_ACTIONS).filter(
      ([action, id]) => !new RegExp(`"${id}"\\s*=>\\s*"${action}"`).test(source),
    )
    expect(missing, 'native menu ids with no dispatch arm to their action').toEqual([])
  })

  it('every built item is attached to a submenu', () => {
    const missing: string[] = []
    for (const [action, id] of Object.entries(NON_TOOL_MENU_ACTIONS)) {
      const binding = new RegExp(
        `let\\s+(\\w+)\\s*=\\s*MenuItemBuilder::with_id\\(\\s*"${id}"`,
        's',
      ).exec(source)
      expect(binding, `no MenuItemBuilder binding found for ${action} (${id})`).not.toBeNull()
      const variable = (binding as RegExpExecArray)[1]
      if (!source.includes(`.item(&${variable})`)) {
        missing.push(`${action} (${id} -> ${variable})`)
      }
    }
    expect(missing, 'menu items built but never attached to a SubmenuBuilder chain').toEqual([])
  })
})

/**
 * File ▸ Revert to Saved: a GATED native item (enabled only with a file and
 * unsaved changes, pushed by sync_menu_state) beside Save As, mirrored by the
 * web MenuBar's item and handled by App's menu-action switch.
 */
describe('Revert to Saved menu item parity', () => {
  const source = readFileSync(MAIN_RS, 'utf8')
  const menuBarSource = readFileSync(MENU_BAR_TSX, 'utf8')
  const appSource = readFileSync(resolve(dirname(fileURLToPath(import.meta.url)), './App.tsx'), 'utf8')

  it('file-revert is a gated native item, attached right after Save As, dispatching to revert-to-saved', () => {
    const binding = /let\s+(\w+)\s*=\s*gated_item\(\s*handle,\s*&mut \w+,\s*"file-revert",\s*lbl!\("file-revert"\)/.exec(source)
    expect(binding, 'no gated_item binding found for file-revert').not.toBeNull()
    const variable = (binding as RegExpExecArray)[1]
    expect(
      new RegExp(`\\.item\\(&file_save_as\\)\\s*\\.item\\(&${variable}\\)`).test(source),
      'file-revert is not attached right after Save As',
    ).toBe(true)
    expect(/"file-revert"\s*=>\s*"revert-to-saved"/.test(source)).toBe(true)
  })

  it('is offered from the web MenuBar and handled by the app', () => {
    expect(menuBarSource.includes('Revert to Saved')).toBe(true)
    expect(appSource.includes("case 'revert-to-saved'")).toBe(true)
    expect(appSource.includes("'file-revert'")).toBe(true)
  })
})

/**
 * Library effort: Window ▸ Library (a CHECKABLE item — the dialog's open/
 * closed state, like the other panes on that menu) and File ▸ Save to
 * Library… (a plain item) — two different binding shapes, so this gets its
 * own block rather than folding into `NON_TOOL_MENU_ACTIONS` above (which
 * hardcodes the plain-`MenuItemBuilder` shape and would false-negative on
 * win-library's `check_item` binding).
 */
describe('Library menu items parity (Window ▸ Library, File ▸ Save to Library…)', () => {
  const source = readFileSync(MAIN_RS, 'utf8')
  const menuBarSource = readFileSync(MENU_BAR_TSX, 'utf8')

  it('win-library is built via check_item, attached to a submenu, and dispatches to toggle-library', () => {
    const binding = new RegExp(`let\\s+(\\w+)\\s*=\\s*check_item\\([^;]*?"win-library"`, 's').exec(source)
    expect(binding, 'no check_item binding found for win-library').not.toBeNull()
    const variable = (binding as RegExpExecArray)[1]
    expect(
      source.includes(`.item(&${variable})`),
      'win-library is built but never attached to a SubmenuBuilder chain',
    ).toBe(true)
    expect(
      /"win-library"\s*=>\s*"toggle-library"/.test(source),
      'win-library has no dispatch arm to toggle-library',
    ).toBe(true)
  })

  it('file-save-to-library is built as a plain item, attached to a submenu, and dispatches to save-to-library-doc', () => {
    const binding = new RegExp(
      `let\\s+(\\w+)\\s*=\\s*MenuItemBuilder::with_id\\(\\s*"file-save-to-library"`,
      's',
    ).exec(source)
    expect(binding, 'no MenuItemBuilder binding found for file-save-to-library').not.toBeNull()
    const variable = (binding as RegExpExecArray)[1]
    expect(
      source.includes(`.item(&${variable})`),
      'file-save-to-library is built but never attached to a SubmenuBuilder chain',
    ).toBe(true)
    expect(
      /"file-save-to-library"\s*=>\s*"save-to-library-doc"/.test(source),
      'file-save-to-library has no dispatch arm to save-to-library-doc',
    ).toBe(true)
  })

  it('is offered from the web MenuBar (File ▸ Save to Library…, Window ▸ Library)', () => {
    expect(menuBarSource.includes('Save to Library…')).toBe(true)
    expect(menuBarSource.includes('onSaveToLibrary')).toBe(true)
    expect(menuBarSource.includes('onToggleLibrary')).toBe(true)
  })
})

/**
 * v1.1 assets lane: the Components panel toggle (View ▸ Components, a
 * checkable pane toggle like win-materials/win-tags) and File ▸ Purge
 * Unused… (a plain item like file-save-to-library) — same two binding
 * shapes as the Library block above, pinned separately since they're a
 * different feature.
 */
describe('Components/Purge Unused menu items parity (v1.1 assets lane)', () => {
  const source = readFileSync(MAIN_RS, 'utf8')
  const menuBarSource = readFileSync(MENU_BAR_TSX, 'utf8')

  it('win-components is built via check_item, attached to a submenu, and dispatches to toggle-components', () => {
    const binding = new RegExp(`let\\s+(\\w+)\\s*=\\s*check_item\\([^;]*?"win-components"`, 's').exec(source)
    expect(binding, 'no check_item binding found for win-components').not.toBeNull()
    const variable = (binding as RegExpExecArray)[1]
    expect(
      source.includes(`.item(&${variable})`),
      'win-components is built but never attached to a SubmenuBuilder chain',
    ).toBe(true)
    expect(
      /"win-components"\s*=>\s*"toggle-components"/.test(source),
      'win-components has no dispatch arm to toggle-components',
    ).toBe(true)
  })

  it('file-purge-unused is built as a plain item, attached to a submenu, and dispatches to purge-unused', () => {
    const binding = new RegExp(
      `let\\s+(\\w+)\\s*=\\s*MenuItemBuilder::with_id\\(\\s*"file-purge-unused"`,
      's',
    ).exec(source)
    expect(binding, 'no MenuItemBuilder binding found for file-purge-unused').not.toBeNull()
    const variable = (binding as RegExpExecArray)[1]
    expect(
      source.includes(`.item(&${variable})`),
      'file-purge-unused is built but never attached to a SubmenuBuilder chain',
    ).toBe(true)
    expect(
      /"file-purge-unused"\s*=>\s*"purge-unused"/.test(source),
      'file-purge-unused has no dispatch arm to purge-unused',
    ).toBe(true)
  })

  it('is offered from the web MenuBar (View ▸ Components, File ▸ Purge Unused…)', () => {
    expect(menuBarSource.includes('Components')).toBe(true)
    expect(menuBarSource.includes('onToggleComponents')).toBe(true)
    expect(menuBarSource.includes('Purge Unused…')).toBe(true)
    expect(menuBarSource.includes('onPurgeUnused')).toBe(true)
  })
})

/**
 * Copy/Cut/Paste/Paste In Place (v1.1-cycle.md Lane D) — gated `MenuItemBuilder`
 * items (macOS-only accelerators; App.tsx's keydown effect owns the keyboard
 * path on both platforms — see the comment beside `edit_delete` in main.rs), each
 * built, attached, dispatched, and offered from the web MenuBar's Edit menu.
 */
describe('Copy/Cut/Paste/Paste In Place menu parity', () => {
  const source = readFileSync(MAIN_RS, 'utf8')
  const menuBarSource = readFileSync(MENU_BAR_TSX, 'utf8')
  const IDS: Record<string, string> = {
    'edit-cut': 'edit-cut',
    'edit-copy': 'edit-copy',
    'edit-paste': 'edit-paste',
    'edit-paste-in-place': 'edit-paste-in-place',
  }

  it('every id is built via gated_item, attached to a submenu, and has a dispatch arm', () => {
    for (const [action, id] of Object.entries(IDS)) {
      // A `gated_item` binding, the macOS-aware `edit_item` one, or (Paste)
      // the platform `if` that picks the native item on macOS.
      const binding = new RegExp(
        `let\\s+(\\w+)[^=]*=\\s*(?:if[^{]*\\{[^}]*\\}\\s*else\\s*\\{\\s*)?(?:gated_item|edit_item)\\([^;]*?"${id}"`,
        's',
      ).exec(source)
      expect(binding, `no gated_item/edit_item binding found for ${action} (${id})`).not.toBeNull()
      const variable = (binding as RegExpExecArray)[1]
      expect(
        source.includes(`.item(&${variable})`),
        `${action} (${id} -> ${variable}) is built but never attached to a SubmenuBuilder chain`,
      ).toBe(true)
      expect(
        new RegExp(`"${id}"\\s*=>\\s*"${action}"`).test(source),
        `${action} (${id}) has no dispatch arm forwarding it to the app`,
      ).toBe(true)
    }
  })

  it('is offered from the web MenuBar\'s Edit menu with the right gates', () => {
    expect(menuBarSource).toContain('label={t`Cut`}')
    expect(menuBarSource).toContain('label={t`Copy`}')
    expect(menuBarSource).toContain('label={t`Paste`}')
    expect(menuBarSource).toContain('label={t`Paste In Place`}')
    expect(menuBarSource).toContain('hasStructuralSelection')
    expect(menuBarSource).toContain('clipboardHasContent')
  })

  it('is offered from the command palette with the matching gates', () => {
    const entries = paletteEntries()
    const byId = new Map(entries.map((e) => [e.id, e]))
    expect(byId.get('edit-cut')?.gate).toBe('hasStructuralSelection')
    expect(byId.get('edit-copy')?.gate).toBe('hasStructuralSelection')
    expect(byId.get('edit-paste')?.gate).toBe('clipboardHasContent')
    expect(byId.get('edit-paste-in-place')?.gate).toBe('clipboardHasContent')
  })
})

/**
 * View ▸ Compact Tool Rail, Tray, and Contextual Dock: checkable items like
 * the pane toggles, placed ahead of them, with no accelerators (layout
 * choices, not worth a reserved chord), and check marks pushed through
 * sync_menu_state by App.tsx.
 */
describe('Compact Tool Rail / Tray / Contextual Dock menu items parity', () => {
  const source = readFileSync(MAIN_RS, 'utf8')
  const menuBarSource = readFileSync(MENU_BAR_TSX, 'utf8')
  const appSource = readFileSync(resolve(dirname(fileURLToPath(import.meta.url)), './App.tsx'), 'utf8')

  it.each([
    ['view-compact-rail', 'toggle-compact-rail'],
    ['view-tray', 'toggle-tray'],
    ['view-dock', 'toggle-dock'],
  ])('%s is built via check_item, attached to a submenu, dispatches to %s, and is synced', (id, action) => {
    const binding = new RegExp(`let\\s+(\\w+)\\s*=\\s*check_item\\([^;]*?"${id}"`, 's').exec(source)
    expect(binding, `no check_item binding found for ${id}`).not.toBeNull()
    const variable = (binding as RegExpExecArray)[1]
    expect(source.includes(`.item(&${variable})`), `${id} is built but never attached to a SubmenuBuilder chain`).toBe(true)
    expect(new RegExp(`"${id}"\\s*=>\\s*"${action}"`).test(source), `${id} has no dispatch arm to ${action}`).toBe(true)
    expect(appSource.includes(`'${id}':`), `App.tsx never pushes ${id}'s check mark through sync_menu_state`).toBe(true)
  })

  it.each(['view-compact-rail', 'view-tray', 'view-dock'])('%s takes no accelerator', (id) => {
    expect(new RegExp(`check_item\\([^;]*?"${id}",[^;]*?None,\\s*None,?\\s*\\)`, 's').test(source)).toBe(true)
  })

  it('is offered from the web MenuBar', () => {
    expect(menuBarSource.includes('label={t`Compact Tool Rail`}')).toBe(true)
    expect(menuBarSource.includes('label={t`Tray`}')).toBe(true)
    expect(menuBarSource.includes('label={t`Contextual Dock`}')).toBe(true)
  })
})

/**
 * Object ▸ Save Selection to Library…: the contextual dock's Save to Library
 * verb, mirrored into the menus so it stays reachable with the dock turned
 * off. A gated plain item like Make Component, enabled through
 * sync_menu_state.
 */
describe('Save Selection to Library menu item parity', () => {
  const source = readFileSync(MAIN_RS, 'utf8')
  const appSource = readFileSync(resolve(dirname(fileURLToPath(import.meta.url)), './App.tsx'), 'utf8')

  it('is built via gated_item, attached to a submenu, dispatches to save-to-library, and is gated', () => {
    const binding = new RegExp(`let\\s+(\\w+)\\s*=\\s*gated_item\\([^;]*?"edit-save-selection-to-library"`, 's').exec(source)
    expect(binding, 'no gated_item binding found for edit-save-selection-to-library').not.toBeNull()
    const variable = (binding as RegExpExecArray)[1]
    expect(source.includes(`.item(&${variable})`)).toBe(true)
    expect(/"edit-save-selection-to-library"\s*=>\s*"save-to-library"/.test(source)).toBe(true)
    expect(appSource.includes("'edit-save-selection-to-library':")).toBe(true)
  })
})

/**
 * Camera ▸ Fly (docs/design/fly-navigation.md §10): the LATCHED fly mode
 * for trackpads — a checkable item like Parallel Projection, no accelerator
 * (its keys are live only while flying), synced from the webview's fly
 * state through sync_menu_state, mirrored in the web MenuBar and the
 * palette. The right-button hold needs no menu at all.
 */
describe('Camera ▸ Fly menu item parity', () => {
  const source = readFileSync(MAIN_RS, 'utf8')
  const menuBarSource = readFileSync(MENU_BAR_TSX, 'utf8')
  const appSource = readFileSync(resolve(dirname(fileURLToPath(import.meta.url)), './App.tsx'), 'utf8')

  it('cam-fly is built via check_item, attached to the Camera submenu, dispatches to toggle-fly, and is synced', () => {
    const binding = /let\s+(\w+)\s*=\s*check_item\([^;]*?"cam-fly"/s.exec(source)
    expect(binding, 'no check_item binding found for cam-fly').not.toBeNull()
    const variable = (binding as RegExpExecArray)[1]
    expect(source.includes(`.item(&${variable})`), 'cam-fly is built but never attached to a SubmenuBuilder chain').toBe(true)
    expect(/"cam-fly"\s*=>\s*"toggle-fly"/.test(source), 'cam-fly has no dispatch arm to toggle-fly').toBe(true)
    expect(appSource.includes(`'cam-fly':`), "App.tsx never pushes cam-fly's check mark through sync_menu_state").toBe(true)
    expect(/case 'toggle-fly':/.test(appSource), 'App.tsx has no toggle-fly dispatch case').toBe(true)
  })

  it('cam-fly takes no accelerator', () => {
    expect(/check_item\([^;]*?"cam-fly",[^;]*?None,\s*None,?\s*\)/s.test(source)).toBe(true)
  })

  it('is offered from the web MenuBar and the command palette', () => {
    expect(menuBarSource.includes('label={t`Fly`}')).toBe(true)
    expect(paletteEntries().some((e) => e.id === 'toggle-fly')).toBe(true)
  })
})

describe('native menu labels come from the one table (docs/LOCALIZATION.md "Native menus")', () => {
  // The shell builds every item with `lbl("<key>")`, a lookup into the
  // per-locale JSON that scripts/i18n-menus.mjs generates from
  // src/i18n/nativeMenu.ts. A key used in Rust but missing from the table
  // would show its raw key; a table entry no Rust site uses is dead copy a
  // translator would still be asked for. Both directions are checked, and
  // the committed English JSON must be what the generator produces now.
  const source = readFileSync(MAIN_RS, 'utf8')
  // `lbl!("…")` during construction; `labels.get("…")` or `current_menu_labels(…).get("…")` where
  // part of the menu is rebuilt at runtime (the Open Recent submenu) or a
  // shell-built window or dialog is titled.
  const rustKeys = new Set(
    Array.from(source.matchAll(/(?:\blbl!|\b(?:labels|current_menu_labels\([^)]*\))\s*\.\s*get)\(\s*"([^"]+)"\s*\)/g), (m) => m[1]),
  )
  const tableKeys = new Set(Object.keys(NATIVE_MENU_LABELS))

  it('every lbl() key in main.rs has a table entry', () => {
    const missing = [...rustKeys].filter((k) => !tableKeys.has(k))
    expect(missing, 'add these to NATIVE_MENU_LABELS').toEqual([])
  })

  it('every table entry is used by main.rs', () => {
    const unused = [...tableKeys].filter((k) => !rustKeys.has(k))
    expect(unused, 'remove these from NATIVE_MENU_LABELS or use them in main.rs').toEqual([])
  })

  it('no menu item is still built from a literal label', () => {
    // Every id-bearing builder must take its label through lbl().
    const literal = Array.from(
      source.matchAll(/(?:with_id\(\s*"[^"]+"|check_item\(\s*handle,\s*&mut \w+,\s*"[^"]+"|gated_item\(\s*handle,\s*&mut \w+,\s*"[^"]+"|edit_item\(\s*handle,\s*&mut \w+,\s*"[^"]+")\s*,\s*"([^"]+)"/g),
      (m) => m[1],
    )
    expect(literal, 'labels still hard-coded in main.rs').toEqual([])
  })

  it('the shell embeds a menu file for every shipped locale, and only those', () => {
    // menu_labels.rs's MENU_LOCALES is a hand-kept list of include_str!
    // lines; SHIPPED_LOCALES (src/i18n/locales.ts) is the app's. A shipped
    // locale missing here would show English menus; an extra one would
    // embed a file the generator no longer maintains.
    const rs = readFileSync(
      resolve(dirname(fileURLToPath(import.meta.url)), '../../shells/tauri/src-tauri/src/menu_labels.rs'),
      'utf8',
    )
    const embedded = Array.from(rs.matchAll(/\(\s*"([a-zA-Z-]+)",\s*include_str!\("\.\.\/locales\/menu\.([a-zA-Z-]+)\.json"\),?\s*\)/g), (m) => {
      expect(m[1], 'locale tag and file name disagree in MENU_LOCALES').toBe(m[2])
      return m[1]
    })
    // The pseudo-locale's file is embedded as a debug aid, never shipped.
    expect([...embedded].sort()).toEqual([...SHIPPED_LOCALES, PSEUDO_LOCALE].sort())
  })

  it('the committed menu.en.json is what the generator produces', async () => {
    const { menuFileFor, readTableRows, MENU_LOCALES_DIR } = await import('../scripts/i18n-menus-lib.mjs')
    const rows = readTableRows()
    expect(rows.length).toBe(tableKeys.size)
    const committed = readFileSync(resolve(MENU_LOCALES_DIR, 'menu.en.json'), 'utf8')
    expect(committed, 'run `pnpm --dir app i18n:menus`').toBe(menuFileFor(rows, 'en'))
  })
})

/**
 * File ▸ Import Image… (Place Image): a GATED native item (disabled while a
 * component is open, pushed by sync_menu_state like Import…) right after
 * Import…, mirrored by the web MenuBar's item, the command palette, and
 * App's menu-action switch.
 */
describe('Import Image menu item parity', () => {
  const source = readFileSync(MAIN_RS, 'utf8')
  const menuBarSource = readFileSync(MENU_BAR_TSX, 'utf8')
  const appSource = readFileSync(resolve(dirname(fileURLToPath(import.meta.url)), './App.tsx'), 'utf8')

  it('file-import-image is a gated native item, attached right after Import…, dispatching to import-image', () => {
    const binding = /let\s+(\w+)\s*=\s*gated_item\(\s*handle,\s*&mut \w+,\s*"file-import-image",\s*lbl!\("file-import-image"\)/.exec(source)
    expect(binding, 'no gated_item binding found for file-import-image').not.toBeNull()
    const variable = (binding as RegExpExecArray)[1]
    expect(
      new RegExp(`\\.item\\(&file_import\\)\\s*\\.item\\(&${variable}\\)`).test(source),
      'file-import-image is not attached right after Import…',
    ).toBe(true)
    expect(/"file-import-image"\s*=>\s*"import-image"/.test(source)).toBe(true)
  })

  it('is offered from the web MenuBar and the palette, and handled and gated by the app', () => {
    expect(menuBarSource.includes('label={t`Import Image…`}')).toBe(true)
    expect(paletteEntries().map((e) => e.id)).toContain('import-image')
    expect(appSource.includes("case 'import-image'")).toBe(true)
    expect(appSource.includes("'file-import-image': !componentFrameOpen")).toBe(true)
  })
})

/**
 * Linked copies: Unlink, Unlink all, Select all copies (Object menu) and
 * Paste independent / Paste linked (Edit menu) are gated native items, each
 * attached to a submenu and dispatching to the same action id the web menu,
 * the palette, and (for the first three) the contextual dock use.
 */
describe('linked copy menu items parity', () => {
  const source = readFileSync(MAIN_RS, 'utf8')
  const menuBarSource = readFileSync(MENU_BAR_TSX, 'utf8')
  const ids = [
    'edit-unlink',
    'edit-unlink-all',
    'edit-select-all-copies',
    'edit-paste-independent',
    'edit-paste-linked',
  ]

  it.each(ids)('%s is a gated native item, attached to a submenu, with a dispatch arm', (id) => {
    const binding = new RegExp(`let\\s+(\\w+)\\s*=\\s*gated_item\\(\\s*handle,\\s*&mut \\w+,\\s*"${id}"`).exec(source)
    expect(binding, `no gated_item binding found for ${id}`).not.toBeNull()
    const variable = (binding as RegExpExecArray)[1]
    expect(source.includes(`.item(&${variable})`), `${id} is built but never attached to a submenu`).toBe(true)
    expect(new RegExp(`"${id}"\\s*=>\\s*"${id}"`).test(source), `${id} has no dispatch arm`).toBe(true)
    // Disabled until the app's selection says otherwise.
    expect(source.includes(`&${variable},`), `${id} is not in the initial disable list`).toBe(true)
  })

  it('the web menu and the palette offer the same actions', () => {
    for (const id of ids) {
      expect(menuBarSource.includes(`'${id}'`), `MenuBar lacks ${id}`).toBe(true)
      expect(paletteEntries().map((e) => e.id)).toContain(id)
      expect(NATIVE_MENU_LABELS[id], `no native label for ${id}`).toBeDefined()
    }
  })

  it('the contextual dock offers Unlink and Unlink all for a linked copy', () => {
    const verbs = dockVerbsFor('instance', { canGroup: false, canMakeComponent: true, canUnlink: true, canUnlinkAll: true }).map((v) => v.id)
    expect(verbs).toEqual(expect.arrayContaining(['edit-unlink', 'edit-unlink-all']))
  })
})

describe('native menu ↔ keymap', () => {
  const source = readFileSync(MAIN_RS, 'utf8')
  const fnStart = source.indexOf('fn menu_action_for(')
  const fnBody = source.slice(fnStart, source.indexOf('_ => return None', fnStart))
  const arms = new Set([...fnBody.matchAll(/"[^"]+"\s*=>\s*"([^"]+)"/g)].map((m) => m[1]))

  it('found menu_action_for (guards the extraction against a refactor)', () => {
    expect(fnStart).toBeGreaterThan(0)
    expect(arms.size).toBeGreaterThan(80)
  })

  it('NATIVE_COMMANDS is exactly the set of commands the native menu dispatches', () => {
    // The keymap leaves a chord to the native menu only for these commands;
    // a drift either way would run a chord twice or not at all on macOS.
    expect([...NATIVE_COMMANDS].sort()).toEqual([...arms].sort())
  })

  it('every native alias names a command the menu dispatches', () => {
    for (const nativeId of Object.values(NATIVE_ALIASES)) expect(arms).toContain(nativeId)
  })

  it('PINNED_NATIVE_CHORDS matches the fixed accelerators the shell builds for the JS_FIRST items', () => {
    // The accelerator is the first string after the item's label, whether
    // the item is built by gated_item, edit_item or accel(MenuItemBuilder…).
    for (const id of JS_FIRST) {
      const nativeId = /"([a-z-]+)" => "([a-z-]+)"/g
      let native: string | undefined
      for (const m of source.matchAll(nativeId)) if (m[2] === id) native = m[1]
      expect(native, `no menu id dispatches ${id}`).toBeDefined()
      const built = new RegExp(`lbl!\\("${native}"\\)\\)?,\\s*(?:Some\\()?"([^"]+)"`).exec(source)
      const pinned = PINNED_NATIVE_CHORDS[id]
      if (built === null) {
        expect(pinned, `${id} has no fixed accelerator, so no pinned chord`).toBeUndefined()
      } else {
        expect(pinned, `${id} carries ${built[1]}`).toBeDefined()
        const accel = chordToAccelerator(parseChord(pinned, true)!)
        expect(accel).toBe(built[1])
      }
    }
    for (const id of Object.keys(PINNED_NATIVE_CHORDS)) expect(JS_FIRST.has(id), id).toBe(true)
  })

  it('on_menu_event and set_menu_accelerators both go through menu_action_for', () => {
    expect(/let Some\(action\) = menu_action_for\(id\) else/.test(source)).toBe(true)
    expect(/fn set_menu_accelerators\(/.test(source)).toBe(true)
    expect(/generate_handler!\[[^\]]*\bset_menu_accelerators\b/s.test(source)).toBe(true)
  })
})

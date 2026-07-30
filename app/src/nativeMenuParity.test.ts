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
    const binding = new RegExp(
      `let\\s+(\\w+)\\s*=\\s*MenuItemBuilder::with_id\\(\\s*"${id}"`,
    ).exec(mainRsSource)
    expect(binding, `no MenuItemBuilder::with_id binding found for ${id}`).not.toBeNull()
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

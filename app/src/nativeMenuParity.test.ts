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

const MAIN_RS = resolve(
  dirname(fileURLToPath(import.meta.url)),
  '../../shells/tauri/src-tauri/src/main.rs',
)

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
    const missing: string[] = []
    for (const [tool, id] of Object.entries(TOOL_MENU_IDS)) {
      const binding = new RegExp(
        `let\\s+(\\w+)\\s*=\\s*check_item\\([^;]*?"${id}"`,
        's',
      ).exec(source)
      expect(binding, `no check_item binding found for ${tool} (${id})`).not.toBeNull()
      const variable = (binding as RegExpExecArray)[1]
      if (!source.includes(`.item(&${variable})`)) {
        missing.push(`${tool} (${id} -> ${variable})`)
      }
    }
    expect(missing, 'menu items built but never attached to a SubmenuBuilder chain').toEqual([])
  })
})

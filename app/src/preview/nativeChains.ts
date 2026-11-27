/* oxlint-disable lingui/no-unlocalized-strings -- menu ids and the compact text form are identifiers for a test, never shown */
/**
 * Reads the macOS native menu's submenu chains out of the Tauri shell's
 * `main.rs` text, and derives what a production build's chain must be: the
 * Preview wall's commands removed, empty submenus collapsed, doubled and edge
 * separators dropped (the same rules as `pruneMenuEntries` for the web menus).
 *
 * Test support for `wall.test.ts` and `nativeMenuParity.test.ts`; nothing in
 * the app imports it. A source scrape, like the parity test it serves.
 */

export type ChainItem =
  | { kind: 'item'; native: string; action: string }
  | { kind: 'separator' }
  | { kind: 'submenu'; id: string; items: ChainItem[] }

/** Native item id -> the command the shell forwards to the app (`"a" => "b"`). */
export function dispatchMap(source: string): Map<string, string> {
  const out = new Map<string, string>()
  for (const m of source.matchAll(/"([a-z0-9:-]+)"\s*=>\s*"([a-z0-9-]+)"/g)) out.set(m[1], m[2])
  return out
}

/** Binding variable -> native item id, for every menu item built with a literal id. */
function itemVars(source: string): Map<string, string> {
  const out = new Map<string, string>()
  const re =
    /let\s+(?:mut\s+)?(\w+)\s*(?::[^=]+)?=\s*(?:check_item|gated_item|edit_item|MenuItemBuilder::with_id)\(\s*(?:handle,\s*&mut\s+\w+,\s*)?"([^"]+)"/g
  for (const m of source.matchAll(re)) out.set(m[1], m[2])
  return out
}

/** Binding variable -> submenu id, for every `let x = SubmenuBuilder::with_id(handle, "submenu:…"`. */
function submenuVars(source: string): Map<string, string> {
  const out = new Map<string, string>()
  const re = /let\s+(\w+)\s*=\s*SubmenuBuilder::with_id\(\s*handle,\s*"(submenu:[a-z-]+)"/g
  for (const m of source.matchAll(re)) out.set(m[1], m[2])
  return out
}

/** The chain of submenu `id`, nested submenus expanded. Throws if not found. */
export function parseSubmenu(source: string, id: string): ChainItem[] {
  const items = itemVars(source)
  const subs = submenuVars(source)
  const dispatch = dispatchMap(source)
  const start = source.search(new RegExp(`SubmenuBuilder::with_id\\(\\s*handle,\\s*"${id}"`))
  if (start < 0) throw new Error(`no SubmenuBuilder for ${id}`)
  const end = source.indexOf('.build()', start)
  const body = source.slice(start, end)
  const out: ChainItem[] = []
  for (const m of body.matchAll(/\.separator\(\)|\.item\(&PredefinedMenuItem::separator\(handle\)\?\)|\.item\(&(\w+)\)/g)) {
    const variable = m[1]
    if (variable === undefined) {
      out.push({ kind: 'separator' })
      continue
    }
    const nested = subs.get(variable)
    if (nested !== undefined) {
      out.push({ kind: 'submenu', id: nested, items: parseSubmenu(source, nested) })
      continue
    }
    const native = items.get(variable) ?? variable
    out.push({ kind: 'item', native, action: dispatch.get(native) ?? native })
  }
  return out
}

/** `items` without the commands `offered` rejects; empty submenus, and
 *  separators that lead, trail or double up, go with them. */
export function pruneChain(items: readonly ChainItem[], offered: (action: string) => boolean): ChainItem[] {
  const kept: ChainItem[] = []
  for (const item of items) {
    if (item.kind === 'submenu') {
      const inner = pruneChain(item.items, offered)
      if (inner.length > 0) kept.push({ ...item, items: inner })
    } else if (item.kind === 'separator' || offered(item.action)) {
      kept.push(item)
    }
  }
  const out: ChainItem[] = []
  for (const item of kept) {
    const last = out[out.length - 1]
    if (item.kind === 'separator' && (last === undefined || last.kind === 'separator')) continue
    out.push(item)
  }
  while (out.length > 0 && out[out.length - 1].kind === 'separator') out.pop()
  return out
}

/** A compact one-line form: `a, b, -, c, [submenu:x: d, e]`. */
export function chainText(items: readonly ChainItem[]): string {
  return items
    .map((i) => (i.kind === 'separator' ? '-' : i.kind === 'submenu' ? `[${i.id}: ${chainText(i.items)}]` : i.native))
    .join(', ')
}

/** Every item (not separator) of a chain, nested ones included. */
export function chainItems(items: readonly ChainItem[]): Array<Extract<ChainItem, { kind: 'item' }>> {
  return items.flatMap((i) => (i.kind === 'item' ? [i] : i.kind === 'submenu' ? chainItems(i.items) : []))
}

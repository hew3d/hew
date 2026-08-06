/**
 * fileNaming — the on-disk filename for a saved library item.
 *
 * A slugified display name plus a short suffix from the item's own uuid
 * (`hew.library` meta's `id`, minted at save — see `App.tsx`'s save-to-
 * library flow): collision-proof enough for a personal library folder
 * without a directory listing round trip (two saves of "Box" get two
 * different suffixes, since each mints its own uuid), and still readable in
 * a Finder/Explorer window. Prefixed with the category's subfolder
 * (`Components/`, `Materials/`, `Models/`) — the library folder is
 * organized by type — and ending `.hew`, matching
 * `valid_library_item_name` (`shells/tauri/src-tauri/src/main.rs`) — the
 * only shape `library_write` accepts.
 */

import type { LibraryCategory } from './types'

/** Category → the library folder's subfolder for that category. */
const CATEGORY_DIR: Record<LibraryCategory, string> = {
  component: 'Components',
  material: 'Materials',
  model: 'Models',
}

/** Slugify `name`, append a 6-hex-char suffix from `id`, and prefix with
 * `category`'s subfolder. Pure — no I/O, no randomness of its own (the
 * suffix comes entirely from the caller's id), so the same (name, id,
 * category) triple always names the same file. */
export function itemFileName(name: string, id: string, category: LibraryCategory): string {
  const slug = name
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
  const base = slug !== '' ? slug : 'item'
  const hex = id.replace(/[^a-f0-9]/gi, '').toLowerCase()
  const suffix = (hex.length >= 6 ? hex.slice(0, 6) : hex.padEnd(6, '0')) || '000000'
  return `${CATEGORY_DIR[category]}/${base}-${suffix}.hew`
}

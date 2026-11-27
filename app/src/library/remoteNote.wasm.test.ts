/**
 * The remote note on item bytes, against the REAL compiled kernel
 * (docs/design/shared-library-sources.md 7.2). Needs `pnpm --dir app wasm`
 * (app/src/wasm/pkg) like libraryUpdates.wasm.test.ts.
 *
 * Sites covered here: `persistMeta` (a rename, keywords or collection
 * written through `updateItemMeta` keeps the note and leaves `edited`
 * alone); `onLibraryOpenAsDocument` and File > Open of a Library path (the
 * opened bytes lose the note, through `withoutRemoteNote`).
 *
 * REQUEST: add to itemFiles.ts
 *   withoutRemoteNote(bytes: Uint8Array): Promise<Uint8Array>
 * Returns the item's bytes with `hew.library` / `remote` removed and every
 * other key and the content untouched (a `remote: null` through the wasm
 * `update_item_meta`, or a dedicated wasm export). Both open paths call it.
 */

import { readFile } from 'node:fs/promises'
import { resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { beforeAll, describe, expect, it } from 'vitest'
import init, { Scene, read_item_summary_json } from '../wasm/pkg/wasm_api.js'
import { updateItemMeta, withoutRemoteNote } from './itemFiles'
import type { LibraryItemSummary } from './types'

const __dirname = fileURLToPath(new URL('.', import.meta.url))

beforeAll(async () => {
  await init({ module_or_path: await readFile(resolve(__dirname, '../wasm/pkg/wasm_api_bg.wasm')) })
})

const NOTE = {
  source: 'github:hew3d/example',
  name: 'Example',
  path: 'Components/chair.hew',
  sha256: 'ab'.repeat(32),
  commit: 'c0ffee',
  license: 'CC0-1.0',
  fetchedAt: '2026-11-01T00:00:00.000Z',
  edited: true,
}

/** An item with the given `hew.library` keys, made through the kernel. */
function itemBytes(meta: Record<string, unknown>): Uint8Array {
  return new Uint8Array(new Scene().extract_document_item(JSON.stringify(meta)))
}

function libAttrs(bytes: Uint8Array): Record<string, unknown> {
  const summary = JSON.parse(read_item_summary_json(bytes)) as LibraryItemSummary
  return summary.doc_attrs['hew.library'] ?? {}
}

describe('the remote note through the real kernel', () => {
  it('persistMeta path: updateItemMeta keeps the note and leaves edited alone', async () => {
    const bytes = itemBytes({ id: 'a', name: 'Chair', category: 'model', remote: NOTE })
    const out = await updateItemMeta(bytes, { name: 'Dining Chair', keywords: ['oak'], collection: 'Furniture' })
    const attrs = libAttrs(out)
    expect(attrs.name).toBe('Dining Chair')
    expect(attrs.remote).toEqual(NOTE)
  })

  it('wasm apply_library_meta (extract_document_item) writes no note unless the caller passes one', async () => {
    const scene = new Scene()
    // A document that carries a stale note at document level, as one opened
    // from a noted item does, saved again as a new item with no note passed.
    const first = new Uint8Array(scene.extract_document_item(JSON.stringify({ id: 'a', remote: NOTE })))
    const reopened = new Scene()
    reopened.load(first)
    const second = new Uint8Array(reopened.extract_document_item(JSON.stringify({ id: 'b', name: 'Copy' })))
    expect(libAttrs(second).remote).toBeUndefined()
    const third = new Uint8Array(reopened.extract_document_item(JSON.stringify({ id: 'c', remote: NOTE })))
    expect(libAttrs(third).remote).toEqual(NOTE)
  })

  it('withoutRemoteNote drops only the note', async () => {
    const bytes = itemBytes({ id: 'a', name: 'Chair', category: 'model', keywords: ['oak'], remote: NOTE })
    const out = await withoutRemoteNote(bytes)
    const attrs = libAttrs(out)
    expect(attrs.remote).toBeUndefined()
    expect(attrs.id).toBe('a')
    expect(attrs.name).toBe('Chair')
    expect(attrs.keywords).toEqual(['oak'])
  })

  it('withoutRemoteNote leaves a note-free item alone', async () => {
    const bytes = itemBytes({ id: 'a', name: 'Chair', category: 'model' })
    expect(libAttrs(await withoutRemoteNote(bytes))).toEqual(libAttrs(bytes))
  })
})

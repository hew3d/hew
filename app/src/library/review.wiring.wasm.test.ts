/* oxlint-disable lingui/no-unlocalized-strings -- test fixtures */
// Review round (F11) against the real kernel: bytes written as an ordinary
// document lose the remote note a Library item's document carries.
import { readFile } from 'node:fs/promises'
import { resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { beforeAll, describe, expect, it } from 'vitest'
import init, { Scene, read_item_summary_json } from '../wasm/pkg/wasm_api.js'
import { withoutRemoteNoteIfAny } from './itemFiles'
import type { LibraryItemSummary } from './types'

const __dirname = fileURLToPath(new URL('.', import.meta.url))
beforeAll(async () => {
  await init({ module_or_path: await readFile(resolve(__dirname, '../wasm/pkg/wasm_api_bg.wasm')) })
})

const NOTE = { source: 'github:hew3d/example', name: 'Example', path: 'Components/chair.hew', sha256: 'ab'.repeat(32), fetchedAt: '2026-11-01T00:00:00.000Z', edited: false }
const attrs = (b: Uint8Array): Record<string, unknown> => (JSON.parse(read_item_summary_json(b)) as LibraryItemSummary).doc_attrs['hew.library'] ?? {}

describe('F11: Save As of a document opened from a noted item', () => {
  it('the document keeps the note in memory, scene.save() writes it, withoutRemoteNoteIfAny drops it', async () => {
    const item = new Uint8Array(new Scene().extract_document_item(JSON.stringify({ id: 'a', name: 'Chair', category: 'model', keywords: ['oak'], remote: NOTE })))
    const scene = new Scene()
    scene.load(item)
    const saved = new Uint8Array(scene.save())
    expect(attrs(saved).remote).toEqual(NOTE)
    const out = await withoutRemoteNoteIfAny(saved)
    expect(attrs(out).remote).toBeUndefined()
    expect(attrs(out).id).toBe('a')
    expect(attrs(out).keywords).toEqual(['oak'])
  })

  it('a document with no note comes back as the same bytes', async () => {
    const scene = new Scene()
    const saved = new Uint8Array(scene.save())
    expect(await withoutRemoteNoteIfAny(saved)).toBe(saved)
  })
})

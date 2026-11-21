/**
 * Library updates against the REAL compiled kernel (v1.5 OW-2): the contract
 * the notice bar rests on. Renaming an item in the Library is a meta-only
 * change and must not announce an update; a real change must, once; the
 * update is one undo step; and an item saved before prints existed is read
 * from its bytes.
 */

import { readFile } from 'node:fs/promises'
import { resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { beforeAll, describe, expect, it } from 'vitest'
import init, { Scene, read_item_summary_json, update_item_meta } from '../wasm/pkg/wasm_api.js'
import { parseLibraryStatus, parseUpdateOutcome, pendingNewer, updateCount, type LibraryLink } from './libraryUpdates'
import { metaForEditSave } from './libraryModel'
import type { LibraryItemSummary } from './types'

const __dirname = fileURLToPath(new URL('.', import.meta.url))

beforeAll(async () => {
  await init({ module_or_path: await readFile(resolve(__dirname, '../wasm/pkg/wasm_api_bg.wasm')) })
})

const IDENTITY = new Float64Array([1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0])

function makeBox(scene: Scene, height: number): bigint {
  const sketch = scene.begin_sketch_on_plane(0, 0, 0, 0, 0, 1)
  const c: Array<[number, number]> = [[0, 0], [1, 0], [1, 1], [0, 1]]
  for (let i = 0; i < 4; i++) {
    const [ax, ay] = c[i]!
    const [bx, by] = c[(i + 1) % 4]!
    scene.sketch_add_segment(sketch, ax, ay, 0, bx, by, 0)
  }
  const regions = Array.from(scene.sketch_regions(sketch))
  return scene.extrude_region(sketch, regions[0]!, height)
}

/** A Library item holding one box component of `height`, under `id`. */
function itemBytes(id: string, height: number, name = 'Leg'): Uint8Array {
  const scene = new Scene()
  const inst = scene.make_component(new Uint8Array([0]), new BigUint64Array([makeBox(scene, height)]))
  const meta = { id, name, category: 'component', savedAt: '2026-01-01T00:00:00Z' }
  return scene.extract_item(new Uint8Array([2]), new BigUint64Array([inst]), true, name, JSON.stringify(meta), undefined)
}

/** What the app hands the kernel for one item: the prints its manifest carries. */
function manifestPrints(bytes: Uint8Array): Record<string, string> | undefined {
  const summary = JSON.parse(read_item_summary_json(bytes)) as LibraryItemSummary
  return summary.doc_attrs['hew.library']?.prints as Record<string, string> | undefined
}

function insertedModel(bytes: Uint8Array, id: string): Scene {
  const scene = new Scene()
  scene.insert_item(bytes, IDENTITY, id, 'hash-1', undefined)
  return scene
}

function status(scene: Scene, library: Record<string, unknown>): LibraryLink[] {
  return parseLibraryStatus(scene.library_status_json(library)).links
}

describe('renaming an item in the Library', () => {
  it('does not announce an update: it is a meta-only change', () => {
    const v1 = itemBytes('item-1', 1)
    const model = insertedModel(v1, 'item-1')
    expect(status(model, { 'item-1': { prints: manifestPrints(v1) } }).map((l) => l.state)).toEqual(['in_step'])

    // The Library dialog renames: name changes, the content does not, the file hash does.
    const renamed = update_item_meta(v1, JSON.stringify({ name: 'Oak leg' }))
    expect(renamed).not.toEqual(v1)
    const links = status(model, { 'item-1': { prints: manifestPrints(renamed) } })
    expect(links.map((l) => l.state)).toEqual(['in_step'])
    expect(pendingNewer(links, new Set())).toEqual([])
  })
})

describe('a changed item', () => {
  it('reads newer once, updates in one undo step, and undo brings the notice back', () => {
    const v1 = itemBytes('item-1', 1)
    const v2 = itemBytes('item-1', 2)
    const model = insertedModel(v1, 'item-1')
    const library = { 'item-1': { prints: manifestPrints(v2) } }
    const before = status(model, library)
    expect(before.map((l) => l.state)).toEqual(['library_newer'])
    expect(updateCount(pendingNewer(before, new Set()))).toBe(1)

    const depth = model.undo_depth()
    const outcome = parseUpdateOutcome(
      model.update_definitions_from_items({ all: true, items: { 'item-1': { bytes: v2, contentHash: 'hash-2' } } }),
    )
    expect(outcome?.updated).toEqual([before[0]!.definition])
    expect(outcome?.skipped).toEqual([])
    expect(model.undo_depth()).toBe(depth + 1)
    expect(status(model, library).map((l) => l.state)).toEqual(['in_step'])

    model.scene_undo()
    expect(model.undo_depth()).toBe(depth)
    expect(status(model, library).map((l) => l.state)).toEqual(['library_newer'])
  })
})

describe('an item saved before prints existed', () => {
  it('is read from its bytes, and the prints come back to cache', () => {
    const v2 = itemBytes('item-1', 2)
    const model = insertedModel(itemBytes('item-1', 1), 'item-1')
    const parsed = parseLibraryStatus(model.library_status_json({ 'item-1': { bytes: v2 } }))
    expect(parsed.links.map((l) => l.state)).toEqual(['library_newer'])
    const cached = parsed.itemPrints['item-1']
    expect(cached).toBeDefined()
    // Passed back as prints, the answer is the same: the bytes need not be read again.
    expect(status(model, { 'item-1': { prints: cached } }).map((l) => l.state)).toEqual(['library_newer'])
  })

  it('an item the Library no longer holds says nothing', () => {
    const model = insertedModel(itemBytes('item-1', 1), 'item-1')
    expect(status(model, {}).map((l) => l.state)).toEqual(['item_missing'])
  })
})

describe('a locked part', () => {
  it('is skipped and named, with the lock as the reason, and nothing changes', () => {
    const v1 = itemBytes('item-1', 1)
    const v2 = itemBytes('item-1', 2)
    const model = insertedModel(v1, 'item-1')
    const [link] = status(model, { 'item-1': { prints: manifestPrints(v2) } })
    const inst = Array.from(model.instances_of(BigInt(link!.definition)))
    model.set_nodes_locked(new Uint8Array(inst.map(() => 2)), new BigUint64Array(inst), true)
    const outcome = parseUpdateOutcome(
      model.update_definitions_from_items({ definitions: [link!.definition], items: { 'item-1': { bytes: v2, contentHash: 'h2' } } }),
    )
    expect(outcome?.updated).toEqual([])
    expect(outcome?.skipped.map((s) => s.code)).toEqual(['LockedNode'])
  })
})

describe('which part is the root of its item', () => {
  /** A Table (a component) holding a Leg (a component), saved as one item. */
  function tableItem(id: string): Uint8Array {
    const scene = new Scene()
    const leg = scene.make_component(new Uint8Array([0]), new BigUint64Array([makeBox(scene, 1)]))
    scene.set_component_name(scene.instance_def(leg)!, 'Leg')
    const table = scene.make_component(new Uint8Array([2]), new BigUint64Array([leg]))
    scene.set_component_name(scene.instance_def(table)!, 'Table')
    const meta = { id, name: 'Table', category: 'component', savedAt: '2026-01-01T00:00:00Z' }
    return scene.extract_item(new Uint8Array([2]), new BigUint64Array([table]), true, 'Table', JSON.stringify(meta), undefined)
  }

  it('the summary names the outer part as the root, and an exploded nested part is not it', () => {
    const bytes = tableItem('table-1')
    const rootSid = (JSON.parse(read_item_summary_json(bytes)) as LibraryItemSummary).first_component_sid
    const model = insertedModel(bytes, 'table-1')
    const links = status(model, { 'table-1': { prints: manifestPrints(bytes) } })
    expect(links.map((l) => l.name).sort()).toEqual(['Leg', 'Table'])
    const roots = links.filter((l) => l.defSid === rootSid)
    expect(roots.map((l) => l.name)).toEqual(['Table'])
    // Explode the Table: its Leg now stands in the model by itself, still carrying the Table's item id.
    const tableDef = BigInt(roots[0]!.definition)
    const tableInst = Array.from(model.instances_of(tableDef))[0]!
    model.explode_instance(tableInst)
    const after = status(model, { 'table-1': { prints: manifestPrints(bytes) } })
    const leg = after.find((l) => l.name === 'Leg')!
    expect(leg.item).toBe('table-1')
    expect(leg.defSid).not.toBe(rootSid)
  })
})

describe('saving a linked set over its item', () => {
  // Why the app does not offer Update in Library for a linked copy: the extract
  // carries the set's "linked" mark into the item, so every insert would be a
  // linked set, and the usual save path un-links and renames the set first.
  it('the extract carries the linked mark into the item', () => {
    const scene = new Scene()
    const inst = scene.wrap_linked(0, makeBox(scene, 1))
    expect(scene.is_linked(inst)).toBe(true)
    const meta = { id: 'item-1', name: 'Leg', category: 'component', savedAt: '2026-01-01T00:00:00Z' }
    const bytes = scene.extract_item(new Uint8Array([2]), new BigUint64Array([inst]), true, 'Leg', JSON.stringify(meta), undefined)
    expect(scene.is_linked(inst)).toBe(true)
    const model = insertedModel(bytes, 'item-1')
    const placed = Array.from(model.instance_ids())
    for (const i of placed) expect(model.is_linked(i)).toBe(true)
    expect(placed.length).toBeGreaterThan(0)
  })
})

describe('editing an item in place', () => {
  /** Load `bytes` as a document, open its part and pull the top face up, as an edit window does. */
  function editedItemScene(bytes: Uint8Array): Scene {
    const scene = new Scene()
    scene.load(bytes)
    scene.open_explode_session(scene.instance_ids()[0]!)
    const object = scene.explode_session_objects()![0]!
    // The session's one box: find its top face by its normal (face handles are slot keys, 1-based).
    let top: bigint | undefined
    for (let i = 1n; i <= 16n && top === undefined; i++) {
      const face = (1n << 32n) | i
      try {
        if (scene.face_normal(object, face)[2]! > 0.9) top = face
      } catch {
        /* not a face */
      }
    }
    scene.push_pull(object, top!, 0.5)
    return scene
  }

  it('Save restamps the prints, keeps the item identity, drops the document id, and the model sees it as newer', () => {
    const v1 = itemBytes('item-1', 1)
    const model = insertedModel(v1, 'item-1')
    const before = JSON.parse(read_item_summary_json(v1)) as LibraryItemSummary

    const edit = editedItemScene(v1)
    const saved = edit.extract_document_item(JSON.stringify({ savedAt: '2026-02-02T00:00:00Z' }))
    const after = JSON.parse(read_item_summary_json(saved)) as LibraryItemSummary
    expect(manifestPrints(saved)).not.toEqual(manifestPrints(v1))
    expect(after.doc_attrs['hew.library']?.id).toBe('item-1')
    expect(after.first_component_sid).toBe(before.first_component_sid)
    const reloaded = new Scene()
    reloaded.load(saved)
    expect(reloaded.document_id()).toBeUndefined()

    const links = status(model, { 'item-1': { prints: manifestPrints(saved) } })
    expect(links.map((l) => l.state)).toEqual(['library_newer'])
    const depth = model.undo_depth()
    const outcome = parseUpdateOutcome(
      model.update_definitions_from_items({ all: true, items: { 'item-1': { bytes: saved, contentHash: 'hash-2' } } }),
    )
    expect(outcome?.updated).toEqual([links[0]!.definition])
    expect(model.undo_depth()).toBe(depth + 1)
  })

  it('a rename and a new keyword made in the Library while the edit window was open survive the save', () => {
    const v1 = itemBytes('item-1', 1)
    const edit = editedItemScene(v1) // loaded with the old name and no keywords
    // Meanwhile the Library renames the item and adds a keyword.
    const current = update_item_meta(v1, JSON.stringify({ name: 'Oak leg', keywords: ['oak'], collection: 'Furniture' }))
    const currentMeta = (JSON.parse(read_item_summary_json(current)) as LibraryItemSummary).doc_attrs['hew.library']
    const saved = edit.extract_document_item(JSON.stringify(metaForEditSave(currentMeta, '2026-02-02T00:00:00Z')))
    const after = (JSON.parse(read_item_summary_json(saved)) as LibraryItemSummary).doc_attrs['hew.library']
    expect(after?.name).toBe('Oak leg')
    expect(after?.keywords).toEqual(['oak'])
    expect(after?.collection).toBe('Furniture')
    expect(after?.id).toBe('item-1')
    expect(after?.savedAt).toBe('2026-02-02T00:00:00Z')
    expect(manifestPrints(saved)).not.toEqual(manifestPrints(v1))
  })

  it('a plain save leaves the prints stale, which is why Edit never uses it', () => {
    const v1 = itemBytes('item-1', 1)
    const edit = editedItemScene(v1)
    const plain = edit.save()
    expect(manifestPrints(plain)).toEqual(manifestPrints(v1))
  })
})


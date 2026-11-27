/**
 * Package S10 oracle, part B: taking one item from a source and keeping it
 * in step (docs/design/shared-library-sources.md 5.2 non-UI rows, 7.4 "Add to
 * my Library" and "Update", decisions 14, 15, 21, 30, 31, 34): addItem,
 * updateItem, keepMine, unlink, noteDeleted.
 *
 * Fixtures: sourcesTestKit.ts. Part A: sources.test.ts. Part C:
 * sources.runs.test.ts.
 */
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { itemFileName } from './fileNaming'
import type { AddItemOk, Failure } from './sources'
import { MAX_INDEX, SRC, SRC2, collectionOf, makeEnv, noteOf, parseFake, parts, uuid, type Env, type PublishItem } from './sourcesTestKit'
import type { LibraryRemoteNote } from './types'

let env: Env
beforeEach(() => {
  env = makeEnv()
})
afterEach(() => {
  env.assertClean()
})

const ID = uuid(1)
const FILE = itemFileName('Part 1', ID, 'component') // Components/part-1-000001.hew
const PATH = 'Components/part-1.hew' // the kit's index path for "Part 1"
/** Same first six hex digits as ID, so the same minted path for the same name. */
const CLASH_ID = '000001ff-0000-4000-8000-0000000000ff'

const item1 = (over: Partial<PublishItem> = {}): PublishItem => ({ id: ID, name: 'Part 1', collection: 'Hinges', keywords: ['brass'], ...over })

function note(over: Partial<LibraryRemoteNote> = {}): LibraryRemoteNote {
  return { source: SRC, name: 'Shop Hardware', path: PATH, sha256: 'a'.repeat(64), fetchedAt: '2026-01-01T00:00:00.000Z', edited: false, ...over }
}

function itemBytesOf(file: string): Uint8Array {
  const b = env.store.items.get(file)
  if (!b) throw new Error(`no item at ${file}`)
  return b
}

const quiet = (): void => {
  env.fetcher.reset()
  env.store.resetLogs()
  env.events.length = 0
  env.stampCalls.length = 0
}

const indexAndResolveCalls = (): number => env.fetcher.calls.filter((c) => c.url.includes('/hew.json') || c.url.includes('/commit/')).length

/** The source added with one item; nothing in the Library yet. */
async function withSource(items: PublishItem[] = [item1()], extra: { license?: string | null; name?: string } = {}) {
  const pub = await env.addSource(SRC, { name: extra.name ?? 'Shop Hardware', license: extra.license ?? null, items })
  quiet()
  return pub
}

/** A local item linked to SRC at version 1 with a newer version 2 published (commit c2) and checked. */
async function newerSetup(over: Partial<PublishItem> = {}, localNote: Partial<LibraryRemoteNote> = {}) {
  const pub1 = await env.addSource(SRC, { name: 'Shop Hardware', items: [item1()] })
  env.store.seed(FILE, { id: ID, name: 'Part 1', collection: 'Mine/Hardware', remote: note({ sha256: pub1.sha[ID], commit: 'c1', ...localNote }) })
  const pub2 = env.server.publish(SRC, { name: 'Shop Hardware', commit: 'c2', items: [item1({ body: 'v2', ...over })] })
  await env.sources.check(SRC)
  quiet()
  return { pub1, pub2 }
}

const expectUntouched = (before: Map<string, Uint8Array>): void => {
  expect(env.store.writes).toEqual([])
  expect(env.store.removes).toEqual([])
  expect(env.store.items).toEqual(before)
}

describe('addItem: the happy path', () => {
  it('fetches the file at the entry\'s bytes cap from the checked commit, with no new index or resolve request', async () => {
    const pub = await withSource()
    const r = await env.sources.addItem(SRC, ID)
    expect(r).toMatchObject({ ok: true, file: FILE, id: ID, replaced: false, sha256: pub.sha[ID] })
    expect(env.fetcher.calls).toEqual([{ url: env.server.urlFor(SRC, PATH, 'c1'), maxBytes: pub.index.items[0].bytes }])
    expect(indexAndResolveCalls()).toBe(0)
  })

  it('stamps the note {source, name, path, sha256, commit, license, fetchedAt, edited:false} and the collection <source name>/<file collection>', async () => {
    const pub = await withSource([item1()], { license: 'CC-BY-4.0' })
    env.clock.advance(5000)
    await env.sources.addItem(SRC, ID)
    const bytes = itemBytesOf(FILE)
    expect(noteOf(bytes)).toEqual({
      source: SRC,
      name: 'Shop Hardware',
      path: PATH,
      sha256: pub.sha[ID],
      commit: 'c1',
      license: 'CC-BY-4.0',
      fetchedAt: env.clock.iso(),
      edited: false,
    })
    expect(collectionOf(bytes)).toBe('Shop Hardware/Hinges')
    expect(parseFake(bytes).lib.id).toBe(ID)
    expect(parseFake(bytes).body).toBe(parseFake(pub.bytes[ID]).body)
  })

  it('writes the item exactly once, and stamps in memory before that write', async () => {
    await withSource()
    await env.sources.addItem(SRC, ID)
    expect(env.store.writes).toEqual([FILE])
    expect(env.store.removes).toEqual([])
    expect(env.events.filter((e) => e === 'write')).toHaveLength(1)
    expect(env.events.indexOf('stamp')).toBeGreaterThan(-1)
    expect(env.events.indexOf('stamp')).toBeLessThan(env.events.indexOf('write'))
    expect(env.events.indexOf('sha')).toBeLessThan(env.events.indexOf('stamp'))
  })

  it('a web source has no commit in the note', async () => {
    const web = 'web:https://pages.example.test/hinges/hew.json'
    env.server.publish(web, { name: 'Pages', items: [item1()] })
    await env.sources.addSource(web)
    quiet()
    await env.sources.addItem(web, ID)
    expect(noteOf(itemBytesOf(FILE))).not.toHaveProperty('commit')
    expect(noteOf(itemBytesOf(FILE))?.source).toBe(web)
  })

  it('an item\'s own license overrides the source\'s; the source\'s is used when the entry has none; with neither the note has no license', async () => {
    await withSource([item1({ license: 'MIT' }), item1({ id: uuid(2), name: 'Part 2' })], { license: 'CC-BY-4.0' })
    await env.sources.addItem(SRC, ID)
    await env.sources.addItem(SRC, uuid(2))
    expect(noteOf(itemBytesOf(FILE))?.license).toBe('MIT')
    expect(noteOf(itemBytesOf(itemFileName('Part 2', uuid(2), 'component')))?.license).toBe('CC-BY-4.0')
  })

  it('with no license anywhere the note carries none', async () => {
    await withSource()
    await env.sources.addItem(SRC, ID)
    expect(noteOf(itemBytesOf(FILE))).not.toHaveProperty('license')
  })

  it('a file with no collection lands in the source-name collection alone', async () => {
    await withSource([item1({ collection: '' })])
    await env.sources.addItem(SRC, ID)
    expect(collectionOf(itemBytesOf(FILE))).toBe('Shop Hardware')
  })

  it('the path is minted from the FILE\'s own name, not the listing\'s (the file is the truth)', async () => {
    await withSource([item1({ name: 'Real name', listedName: 'Old listing name' })])
    const r = await env.sources.addItem(SRC, ID)
    expect(r).toMatchObject({ ok: true, file: itemFileName('Real name', ID, 'component') })
    expect(env.store.writes).toEqual([itemFileName('Real name', ID, 'component')])
  })

  it('a material lands under Materials/', async () => {
    await withSource([item1({ category: 'material', name: 'Birch' })])
    const r = await env.sources.addItem(SRC, ID)
    expect(r).toMatchObject({ ok: true, file: itemFileName('Birch', ID, 'material') })
    expect(env.store.writes[0].startsWith('Materials/')).toBe(true)
  })

  it('reads the commit the newest check saw', async () => {
    await withSource()
    env.server.publish(SRC, { name: 'Shop Hardware', commit: 'c2', items: [item1()] })
    await env.sources.check(SRC)
    quiet()
    await env.sources.addItem(SRC, ID)
    expect(env.fetcher.urls()[0]).toContain('/c2/')
    expect(noteOf(itemBytesOf(FILE))?.commit).toBe('c2')
  })
})

describe('addItem: the hash check', () => {
  it('a mismatch runs ONE fresh check and tries once more; the second fetch reads the new commit', async () => {
    await withSource()
    const old = env.server.urlFor(SRC, PATH, 'c1')
    // The publisher pushed after our check: the file at the commit we hold no longer matches its listing...
    const v2 = env.server.publish(SRC, { name: 'Shop Hardware', commit: 'c2', items: [item1({ body: 'v2' })] })
    env.server.put(old, v2.bytes[ID])
    env.fetcher.reset()
    const r = (await env.sources.addItem(SRC, ID)) as AddItemOk
    expect(r.ok).toBe(true)
    expect(env.fetcher.urls()).toEqual([old, expect.stringContaining('/commit/'), expect.stringContaining('/c2/hew.json'), env.server.urlFor(SRC, PATH, 'c2')])
    expect(env.fetcher.calls[3].maxBytes).toBe(v2.index.items[0].bytes)
    expect(env.store.writes).toEqual([FILE])
    expect(noteOf(itemBytesOf(FILE))).toMatchObject({ sha256: v2.sha[ID], commit: 'c2' })
  })

  it('a second mismatch is hash_mismatch: two file requests, one fresh check, nothing written', async () => {
    await withSource()
    env.server.put(env.server.urlFor(SRC, PATH, 'c1'), new TextEncoder().encode('{"lib":{"id":"x"},"body":"wrong"}'))
    env.fetcher.reset()
    const before = env.store.snapshot()
    expect(await env.sources.addItem(SRC, ID)).toMatchObject({ ok: false, code: 'hash_mismatch' })
    const files = env.fetcher.calls.filter((c) => c.url.includes(PATH))
    expect(files).toHaveLength(2)
    expect(env.fetcher.calls.filter((c) => c.url.includes('/commit/'))).toHaveLength(1)
    expect(env.fetcher.calls.filter((c) => c.url.includes('/hew.json'))).toHaveLength(1)
    expectUntouched(before)
    expect(env.stampCalls).toHaveLength(0)
  })

  it('a body longer than the entry\'s bytes (too_large) counts as a mismatch: re-check, retry, then hash_mismatch', async () => {
    await withSource()
    env.server.put(env.server.urlFor(SRC, PATH, 'c1'), new Uint8Array(100_000))
    env.fetcher.reset()
    const before = env.store.snapshot()
    expect(await env.sources.addItem(SRC, ID)).toMatchObject({ ok: false, code: 'hash_mismatch' })
    expect(env.fetcher.calls.filter((c) => c.url.includes(PATH))).toHaveLength(2)
    expectUntouched(before)
  })
})

describe('addItem: refusals leave the Library untouched', () => {
  it('an entry whose id disagrees with the file is listing_incorrect', async () => {
    await withSource([item1({ listedId: uuid(50) })])
    const before = env.store.snapshot()
    expect(await env.sources.addItem(SRC, uuid(50))).toMatchObject({ ok: false, code: 'listing_incorrect' })
    expectUntouched(before)
  })

  it('an entry whose category disagrees with the file is listing_incorrect', async () => {
    await withSource([item1({ listedCategory: 'Models' })])
    const before = env.store.snapshot()
    expect(await env.sources.addItem(SRC, ID)).toMatchObject({ ok: false, code: 'listing_incorrect' })
    expectUntouched(before)
  })

  it('a different id already at the minted path is name_clash and nothing is written', async () => {
    await withSource()
    env.store.seed(FILE, { id: CLASH_ID, name: 'Part 1' })
    quiet()
    const before = env.store.snapshot()
    expect(await env.sources.addItem(SRC, ID)).toMatchObject({ ok: false, code: 'name_clash' })
    expectUntouched(before)
  })

  it('an id already in the Library with no note asks first (needs_confirm replace_item), fetching and writing nothing', async () => {
    await withSource()
    env.store.seed(FILE, { id: ID, name: 'Part 1', collection: 'Mine' })
    quiet()
    const before = env.store.snapshot()
    expect(await env.sources.addItem(SRC, ID)).toMatchObject({ ok: false, code: 'needs_confirm', confirm: 'replace_item' })
    expect(env.fetcher.calls).toHaveLength(0)
    expectUntouched(before)
  })

  it('replace:true replaces that item, stamps it and writes once', async () => {
    const pub = await withSource()
    env.store.seed(FILE, { id: ID, name: 'Part 1', collection: 'Mine', body: 'my own' })
    quiet()
    const r = await env.sources.addItem(SRC, ID, { replace: true })
    expect(r).toMatchObject({ ok: true, file: FILE, replaced: true })
    expect(env.store.writes).toEqual([FILE])
    expect(noteOf(itemBytesOf(FILE))).toMatchObject({ source: SRC, sha256: pub.sha[ID], edited: false })
    expect(parseFake(itemBytesOf(FILE)).body).toBe(parseFake(pub.bytes[ID]).body)
  })

  it('replace:true where the user\'s file sits at another path writes the minted path first, then removes the old file', async () => {
    await withSource()
    const old = 'Components/my-hinge-000001.hew'
    env.store.seed(old, { id: ID, name: 'My hinge' })
    quiet()
    const r = await env.sources.addItem(SRC, ID, { replace: true })
    expect(r).toMatchObject({ ok: true, file: FILE, replaced: true })
    expect(env.store.log.filter((l) => l.startsWith('write ') || l.startsWith('remove '))).toEqual([`write ${FILE}`, `remove ${old}`])
    expect(env.store.items.has(old)).toBe(false)
  })

  it('an id whose note names another source is unavailable_other_source, even with replace:true, with no request', async () => {
    await withSource()
    await env.addSource(SRC2, { name: 'Slides', items: [] })
    env.store.seed(FILE, { id: ID, name: 'Part 1', remote: note({ source: SRC2, name: 'Slides' }) })
    quiet()
    const before = env.store.snapshot()
    expect(await env.sources.addItem(SRC, ID)).toMatchObject({ ok: false, code: 'unavailable_other_source', otherSource: 'Slides' })
    expect(await env.sources.addItem(SRC, ID, { replace: true })).toMatchObject({ ok: false, code: 'unavailable_other_source' })
    expect(env.fetcher.calls).toHaveLength(0)
    expectUntouched(before)
  })

  it('a note naming a source that is not in the list also counts as another source', async () => {
    await withSource()
    env.store.seed(FILE, { id: ID, name: 'Part 1', remote: note({ source: 'github:gone/away', name: 'Away' }) })
    quiet()
    expect(await env.sources.addItem(SRC, ID)).toMatchObject({ ok: false, code: 'unavailable_other_source', otherSource: 'Away' })
  })

  it('an item already in the Library from this very source is already_in_library (use updateItem)', async () => {
    const pub = await withSource()
    env.store.seed(FILE, { id: ID, name: 'Part 1', remote: note({ sha256: pub.sha[ID] }) })
    quiet()
    expect(await env.sources.addItem(SRC, ID)).toMatchObject({ ok: false, code: 'already_in_library' })
    expect(env.store.writes).toEqual([])
  })

  it('a source not in the list is unknown_source; an id not in the index is item_not_found', async () => {
    expect(await env.sources.addItem(SRC, ID)).toMatchObject({ ok: false, code: 'unknown_source' })
    await withSource()
    expect(await env.sources.addItem(SRC, uuid(99))).toMatchObject({ ok: false, code: 'item_not_found' })
    expect(env.store.writes).toEqual([])
  })

  it.each([
    ['network', undefined],
    ['timeout', undefined],
    ['rate_limited', undefined],
    ['http_status', 404],
  ] as const)('a %s failure fetching the file is returned as that code and the Library is untouched', async (code, status) => {
    await withSource()
    env.server.fail((u) => u.includes(PATH), code, { status })
    const before = env.store.snapshot()
    const r = (await env.sources.addItem(SRC, ID)) as Failure
    expect(r).toMatchObject({ ok: false, code })
    if (status) expect(r.status).toBe(status)
    expectUntouched(before)
  })

  it('a removed source is unavailable: source_removed with the reason and link, no file request', async () => {
    await withSource()
    env.server.publishCatalog({ removed: [{ source: SRC, severity: 'removed', reason: 'Took it down.', link: 'https://example.com/why' }] })
    await env.sources.catalog()
    quiet()
    expect(await env.sources.addItem(SRC, ID)).toMatchObject({ ok: false, code: 'source_removed', reason: 'Took it down.', link: 'https://example.com/why' })
    expect(env.fetcher.calls).toHaveLength(0)
    expect(env.store.writes).toEqual([])
  })

  it('an unreachable source: each attempt fails with the fetch code (the cached listing stays)', async () => {
    await withSource()
    env.server.fail(() => true, 'network')
    await env.sources.check(SRC)
    const before = env.store.snapshot()
    expect(await env.sources.addItem(SRC, ID)).toMatchObject({ ok: false, code: 'network' })
    expectUntouched(before)
    expect(await env.sources.itemStates(SRC)).toMatchObject({ ok: true })
  })

  it('a failing store.write is write_failed and nothing else changes', async () => {
    await withSource()
    env.store.failWrite.add(FILE)
    const before = env.store.snapshot()
    expect(await env.sources.addItem(SRC, ID)).toMatchObject({ ok: false, code: 'write_failed' })
    expect(env.store.removes).toEqual([])
    expect(env.store.items).toEqual(before)
  })

  it('is Off: off:true, no request, no write', async () => {
    await withSource()
    env.fetcher.on = false
    expect(await env.sources.addItem(SRC, ID)).toEqual({ off: true })
    expect(env.fetcher.calls).toHaveLength(0)
    expect(env.store.writes).toEqual([])
  })
})

describe('updateItem', () => {
  it('replaces the file, keeps the local collection, takes the source\'s keywords, refreshes the note and clears edited', async () => {
    const { pub2 } = await newerSetup({ keywords: ['steel'] })
    env.clock.advance(60_000)
    const r = await env.sources.updateItem(SRC, ID)
    expect(r).toMatchObject({ ok: true, file: FILE, replaced: true, sha256: pub2.sha[ID] })
    expect(env.fetcher.calls).toEqual([{ url: env.server.urlFor(SRC, PATH, 'c2'), maxBytes: pub2.index.items[0].bytes }])
    expect(env.store.writes).toEqual([FILE])
    expect(env.store.removes).toEqual([])
    const bytes = itemBytesOf(FILE)
    expect(collectionOf(bytes)).toBe('Mine/Hardware')
    expect(parseFake(bytes).lib.keywords).toEqual(['steel'])
    expect(parseFake(bytes).body).toBe('v2')
    expect(noteOf(bytes)).toEqual({ source: SRC, name: 'Shop Hardware', path: PATH, sha256: pub2.sha[ID], commit: 'c2', license: undefined, fetchedAt: env.clock.iso(), edited: false })
  })

  it('stamps the local collection, not the source\'s, and sets no collection of its own choosing', async () => {
    await newerSetup()
    await env.sources.updateItem(SRC, ID)
    expect(env.stampCalls.at(-1)?.patch.collection).toBe('Mine/Hardware')
  })

  it('a successful update clears the Kept record', async () => {
    await newerSetup()
    await env.sources.keepMine(SRC, ID)
    expect(env.store.entryFor(SRC)?.kept).not.toEqual({})
    await env.sources.updateItem(SRC, ID)
    expect(env.store.entryFor(SRC)?.kept).toEqual({})
  })

  it('afterwards the item is in step with the source', async () => {
    await newerSetup()
    await env.sources.updateItem(SRC, ID)
    expect((await env.sources.localStates())[0]).toMatchObject({ id: ID, state: 'in_step', edited: false })
    const states = await env.sources.itemStates(SRC)
    expect('rows' in states && states.rows[0]).toMatchObject({ state: 'in_step' })
  })

  it('a changed minted path writes the new file first, then removes the old one', async () => {
    await newerSetup({ name: 'Renamed part' })
    const fresh = itemFileName('Renamed part', ID, 'component')
    const r = await env.sources.updateItem(SRC, ID)
    expect(r).toMatchObject({ ok: true, file: fresh })
    expect(env.store.log.filter((l) => l.startsWith('write ') || l.startsWith('remove '))).toEqual([`write ${fresh}`, `remove ${FILE}`])
    expect(env.store.items.has(FILE)).toBe(false)
    expect(collectionOf(itemBytesOf(fresh))).toBe('Mine/Hardware')
  })

  it('a failed remove of the old file is reported on the row; the new file is in place', async () => {
    await newerSetup({ name: 'Renamed part' })
    const fresh = itemFileName('Renamed part', ID, 'component')
    env.store.failRemove.add(FILE)
    const r = await env.sources.updateItem(SRC, ID)
    expect(r).toMatchObject({ ok: true, file: fresh, removeFailed: true })
    expect(env.store.items.has(fresh)).toBe(true)
  })

  it('a changed minted path that holds a different id is name_clash and nothing is written or removed', async () => {
    await newerSetup({ name: 'Renamed part' })
    env.store.seed(itemFileName('Renamed part', CLASH_ID, 'component'), { id: CLASH_ID, name: 'Renamed part' })
    quiet()
    const before = env.store.snapshot()
    expect(await env.sources.updateItem(SRC, ID)).toMatchObject({ ok: false, code: 'name_clash' })
    expectUntouched(before)
  })

  it('when the new file cannot be written the old one is not removed', async () => {
    await newerSetup({ name: 'Renamed part' })
    env.store.failWrite.add(itemFileName('Renamed part', ID, 'component'))
    const before = env.store.snapshot()
    expect(await env.sources.updateItem(SRC, ID)).toMatchObject({ ok: false, code: 'write_failed' })
    expect(env.store.removes).toEqual([])
    expect(env.store.items).toEqual(before)
  })

  it('an edited item asks first: needs_confirm replace_changes, nothing fetched or written', async () => {
    await newerSetup({}, { edited: true })
    const before = env.store.snapshot()
    expect(await env.sources.updateItem(SRC, ID)).toMatchObject({ ok: false, code: 'needs_confirm', confirm: 'replace_changes' })
    expect(env.fetcher.calls).toHaveLength(0)
    expectUntouched(before)
  })

  it('decision "replace" updates an edited item and clears edited', async () => {
    await newerSetup({}, { edited: true })
    expect(await env.sources.updateItem(SRC, ID, { decision: 'replace' })).toMatchObject({ ok: true, file: FILE })
    expect(noteOf(itemBytesOf(FILE))?.edited).toBe(false)
    expect(parseFake(itemBytesOf(FILE)).body).toBe('v2')
  })

  it('decision "keep" records the declined hash through writeSources and touches no item and no network', async () => {
    const { pub2 } = await newerSetup({}, { edited: true })
    const before = env.store.snapshot()
    const r = await env.sources.updateItem(SRC, ID, { decision: 'keep' })
    expect(r).toMatchObject({ ok: true, kept: true })
    expect(env.fetcher.calls).toHaveLength(0)
    expectUntouched(before)
    expect(env.store.entryFor(SRC)?.kept).toEqual({ [ID]: pub2.sha[ID] })
    expect(env.store.sourceWrites.every((w) => w.source === SRC)).toBe(true)
  })

  it('an item open for editing is skipped as open_for_editing: no request, no write', async () => {
    await newerSetup()
    env.store.editing = [FILE]
    const before = env.store.snapshot()
    expect(await env.sources.updateItem(SRC, ID)).toMatchObject({ ok: false, code: 'open_for_editing' })
    expect(env.fetcher.calls).toHaveLength(0)
    expectUntouched(before)
  })

  it('other items open for editing do not block this one', async () => {
    await newerSetup()
    env.store.editing = ['Components/something-else-aaaaaa.hew']
    expect(await env.sources.updateItem(SRC, ID)).toMatchObject({ ok: true })
  })

  it('a hash mismatch re-checks once and retries, as Add does', async () => {
    await newerSetup()
    const old = env.server.urlFor(SRC, PATH, 'c2')
    const v3 = env.server.publish(SRC, { name: 'Shop Hardware', commit: 'c3', items: [item1({ body: 'v3' })] })
    env.server.put(old, v3.bytes[ID])
    env.fetcher.reset()
    expect(await env.sources.updateItem(SRC, ID)).toMatchObject({ ok: true, sha256: v3.sha[ID] })
    expect(env.fetcher.calls.filter((c) => c.url.includes('/hew.json'))).toHaveLength(1)
    expect(env.store.writes).toEqual([FILE])
  })

  it('is not offered for an item that is already in step (not_updatable, state in_step)', async () => {
    const pub = await withSource()
    env.store.seed(FILE, { id: ID, name: 'Part 1', remote: note({ sha256: pub.sha[ID] }) })
    quiet()
    expect(await env.sources.updateItem(SRC, ID)).toMatchObject({ ok: false, code: 'not_updatable', state: 'in_step' })
    expect(env.fetcher.calls).toHaveLength(0)
  })

  it('is not offered for R-gone (the source no longer lists it): not_updatable, state gone', async () => {
    await withSource([item1({ id: uuid(2), name: 'Part 2' })])
    env.store.seed(FILE, { id: ID, name: 'Part 1', remote: note() })
    quiet()
    expect(await env.sources.updateItem(SRC, ID)).toMatchObject({ ok: false, code: 'not_updatable', state: 'gone' })
    expect(env.fetcher.calls).toHaveLength(0)
    expect(env.store.writes).toEqual([])
  })

  it('an item with no note is not_linked; an id not in the Library is not_found', async () => {
    await withSource()
    env.store.seed(FILE, { id: ID, name: 'Part 1' })
    expect(await env.sources.updateItem(SRC, ID)).toMatchObject({ ok: false, code: 'not_linked' })
    expect(await env.sources.updateItem(SRC, uuid(77))).toMatchObject({ ok: false, code: 'not_found' })
  })

  it('a source not in the list (R-orphan) is unknown_source: no update, no request', async () => {
    env.store.seed(FILE, { id: ID, name: 'Part 1', remote: note({ source: 'github:gone/away', name: 'Away' }) })
    expect(await env.sources.updateItem('github:gone/away', ID)).toMatchObject({ ok: false, code: 'unknown_source' })
    expect(env.fetcher.calls).toHaveLength(0)
  })

  it('a removed source is unavailable: source_removed', async () => {
    await newerSetup()
    env.server.publishCatalog({ removed: [{ source: SRC, severity: 'removed', reason: 'Gone.' }] })
    await env.sources.catalog()
    quiet()
    expect(await env.sources.updateItem(SRC, ID)).toMatchObject({ ok: false, code: 'source_removed', reason: 'Gone.' })
    expect(env.fetcher.calls).toHaveLength(0)
  })

  it('a fetch failure is returned as its code and the item is untouched', async () => {
    await newerSetup()
    env.server.fail((u) => u.includes(PATH), 'network')
    const before = env.store.snapshot()
    expect(await env.sources.updateItem(SRC, ID)).toMatchObject({ ok: false, code: 'network' })
    expectUntouched(before)
  })

  it('is Off: off:true, no request', async () => {
    await newerSetup()
    env.fetcher.on = false
    expect(await env.sources.updateItem(SRC, ID)).toEqual({ off: true })
    expect(env.fetcher.calls).toHaveLength(0)
  })
})

describe('keepMine', () => {
  it('records id -> the declined sha256 in the source\'s entry, keeping the rest of it, with no item write and no request', async () => {
    const { pub2 } = await newerSetup()
    const entry = env.store.entryFor(SRC)!
    env.store.sourcesFile = { hew: 1, sources: [{ ...entry, skipped: [uuid(70)], followed: true }] }
    const before = env.store.snapshot()
    expect(await env.sources.keepMine(SRC, ID)).toEqual({ ok: true })
    expect(env.store.entryFor(SRC)).toMatchObject({ followed: true, skipped: [uuid(70)], kept: { [ID]: pub2.sha[ID] } })
    expect(env.store.sourceWrites).toHaveLength(1)
    expect(env.fetcher.calls).toHaveLength(0)
    expectUntouched(before)
  })

  it('the row still reads newer but is kept, and it is not counted in the bar', async () => {
    await newerSetup()
    expect(await env.sources.newCounts(SRC)).toEqual({ new: 0, changed: 1 })
    await env.sources.keepMine(SRC, ID)
    const states = await env.sources.itemStates(SRC)
    expect('rows' in states && states.rows[0]).toMatchObject({ state: 'newer', kept: true })
    expect(await env.sources.newCounts(SRC)).toEqual({ new: 0, changed: 0 })
  })

  it('a kept item can still be updated by hand', async () => {
    await newerSetup()
    await env.sources.keepMine(SRC, ID)
    expect(await env.sources.updateItem(SRC, ID)).toMatchObject({ ok: true })
  })

  it('is refused for an item that is not newer, and for an id not in the Library', async () => {
    const pub = await withSource()
    env.store.seed(FILE, { id: ID, name: 'Part 1', remote: note({ sha256: pub.sha[ID] }) })
    expect(await env.sources.keepMine(SRC, ID)).toMatchObject({ ok: false, code: 'not_updatable' })
    expect(await env.sources.keepMine(SRC, uuid(77))).toMatchObject({ ok: false, code: 'not_found' })
    expect(env.store.sourceWrites).toHaveLength(0)
  })
})

describe('unlink', () => {
  it('removes the note (one write, nothing else changes in the file), and the item reads "In your Library"', async () => {
    await newerSetup()
    const bodyBefore = parseFake(itemBytesOf(FILE)).body
    expect(await env.sources.unlink(SRC, ID)).toEqual({ ok: true })
    expect(env.store.writes).toEqual([FILE])
    expect(env.stampCalls.at(-1)?.patch).toEqual({ remote: null })
    expect(noteOf(itemBytesOf(FILE))).toBeUndefined()
    expect(parseFake(itemBytesOf(FILE)).body).toBe(bodyBefore)
    expect(collectionOf(itemBytesOf(FILE))).toBe('Mine/Hardware')
    const states = await env.sources.itemStates(SRC)
    expect('rows' in states && states.rows[0]).toMatchObject({ state: 'in_library', file: FILE })
    expect(await env.sources.localStates()).toEqual([])
  })

  it('removes the Kept record, and does NOT put the id on the skip list (even for a followed source)', async () => {
    await newerSetup()
    await env.sources.keepMine(SRC, ID)
    const entry = env.store.entryFor(SRC)!
    env.store.sourcesFile = { hew: 1, sources: [{ ...entry, followed: true }] }
    await env.sources.unlink(SRC, ID)
    expect(env.store.entryFor(SRC)?.kept).toEqual({})
    expect(env.store.entryFor(SRC)?.skipped).toEqual([])
  })

  it('works for an orphan (the source is gone from the list) and needs no network', async () => {
    env.store.seed(FILE, { id: ID, name: 'Part 1', remote: note({ source: 'github:gone/away', name: 'Away' }) })
    env.fetcher.on = false
    expect(await env.sources.unlink('github:gone/away', ID)).toEqual({ ok: true })
    expect(noteOf(itemBytesOf(FILE))).toBeUndefined()
    expect(env.store.sourceWrites).toHaveLength(0)
    expect(env.fetcher.calls).toHaveLength(0)
  })

  it('an item with no note, or one naming another source, is not_linked; a missing item is not_found; nothing is written', async () => {
    env.store.seed(FILE, { id: ID, name: 'Part 1' })
    env.store.seed('Components/other-000002.hew', { id: uuid(2), name: 'Other', remote: note({ source: SRC2, name: 'Slides' }) })
    expect(await env.sources.unlink(SRC, ID)).toMatchObject({ ok: false, code: 'not_linked' })
    expect(await env.sources.unlink(SRC, uuid(2))).toMatchObject({ ok: false, code: 'not_linked' })
    expect(await env.sources.unlink(SRC, uuid(9))).toMatchObject({ ok: false, code: 'not_found' })
    expect(env.store.writes).toEqual([])
  })
})

describe('noteDeleted (a UI delete)', () => {
  const followed = async (): Promise<void> => {
    await env.addSource(SRC, { name: 'Shop Hardware', items: [item1()] })
    const entry = env.store.entryFor(SRC)!
    env.store.sourcesFile = { hew: 1, sources: [{ ...entry, followed: true, kept: { [ID]: 'f'.repeat(64) } }] }
    env.store.resetLogs()
  }

  it('an item from a followed source goes on its skip list and its Kept record goes, in one writeSources', async () => {
    await followed()
    await env.sources.noteDeleted(ID, { source: SRC })
    expect(env.store.sourceWrites).toHaveLength(1)
    expect(env.store.entryFor(SRC)).toMatchObject({ skipped: [ID], kept: {}, followed: true })
  })

  it('is idempotent: the id is on the list once', async () => {
    await followed()
    await env.sources.noteDeleted(ID, { source: SRC })
    await env.sources.noteDeleted(ID, { source: SRC })
    expect(env.store.entryFor(SRC)?.skipped).toEqual([ID])
  })

  it('a source that is not followed gets no skip entry', async () => {
    await env.addSource(SRC, { name: 'Shop Hardware', items: [item1()] })
    env.store.resetLogs()
    await env.sources.noteDeleted(ID, { source: SRC })
    expect(env.store.sourceWrites).toHaveLength(0)
    expect(env.store.entryFor(SRC)?.skipped).toEqual([])
  })

  it('an item with no note, or a note naming a source not in the list, writes nothing', async () => {
    await followed()
    await env.sources.noteDeleted(ID, undefined)
    await env.sources.noteDeleted(ID, { source: 'github:gone/away' })
    expect(env.store.sourceWrites).toHaveLength(0)
  })

  it('a skipped item is no longer new', async () => {
    await env.addSource(SRC, { name: 'Shop Hardware', items: [item1(), item1({ id: uuid(2), name: 'Part 2' })] })
    const entry = env.store.entryFor(SRC)!
    env.store.sourcesFile = { hew: 1, sources: [{ ...entry, followed: true }] }
    expect(await env.sources.newCounts(SRC)).toEqual({ new: 2, changed: 0 })
    await env.sources.noteDeleted(ID, { source: SRC })
    expect(await env.sources.newCounts(SRC)).toEqual({ new: 1, changed: 0 })
    const states = await env.sources.itemStates(SRC)
    expect('rows' in states && states.rows.find((r) => r.id === ID)).toMatchObject({ skipped: true, state: 'not_in_library' })
  })
})

describe('hygiene on item writes', () => {
  it('never reads more than the 6.4 caps and never writes a dot-file (checked after each test)', async () => {
    await withSource()
    await env.sources.addItem(SRC, ID)
    expect(env.fetcher.calls.every((c) => c.maxBytes <= MAX_INDEX)).toBe(true)
    expect(env.store.writes.every((w) => !w.startsWith('.'))).toBe(true)
  })
})

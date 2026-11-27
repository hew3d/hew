/**
 * Package S10, review round 1: reproducing tests for the findings fixed after
 * the adversarial review (F1, F4, F6, F10, F12, F16). Each block names its
 * finding. Fixtures: sourcesTestKit.ts.
 */
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { itemFileName } from './fileNaming'
import type { CatalogSourceEntry } from './sources'
import { MAX_INDEX, SRC, makeEnv, parts, uuid, type Env, type PublishItem } from './sourcesTestKit'
import type { LibraryRemoteNote } from './types'

let env: Env
beforeEach(() => {
  env = makeEnv()
})
afterEach(() => {
  env.assertClean()
})

const enc = new TextEncoder()
const ID = uuid(1)
const FILE = itemFileName('Part 1', ID, 'component') // Components/part-1-000001.hew
const PATH = 'Components/part-1.hew'
const CLASH_ID = '000001ff-0000-4000-8000-0000000000ff'

const item1 = (over: Partial<PublishItem> = {}): PublishItem => ({ id: ID, name: 'Part 1', collection: 'Hinges', ...over })

function note(over: Partial<LibraryRemoteNote> = {}): LibraryRemoteNote {
  return { source: SRC, name: 'Shop Hardware', path: PATH, sha256: 'a'.repeat(64), fetchedAt: '2026-01-01T00:00:00.000Z', edited: false, ...over }
}

const quiet = (): void => {
  env.fetcher.reset()
  env.store.resetLogs()
  env.events.length = 0
  env.stampCalls.length = 0
}

async function withSource(items: PublishItem[] = [item1()]) {
  const pub = await env.addSource(SRC, { name: 'Shop Hardware', items })
  quiet()
  return pub
}

/** A local item linked to SRC at version 1, a newer version 2 published and checked. */
async function newerSetup(over: Partial<PublishItem> = {}) {
  const pub1 = await env.addSource(SRC, { name: 'Shop Hardware', items: [item1()] })
  env.store.seed(FILE, { id: ID, name: 'Part 1', collection: 'Mine/Hardware', remote: note({ sha256: pub1.sha[ID], commit: 'c1' }) })
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

/** Runs `hook` inside the first index request, before it answers: a change
 * the user makes while a check is in flight. */
function duringIndexFetch(hook: () => Promise<unknown>): void {
  const real = env.fetcher.fetchBytes.bind(env.fetcher)
  let fired = false
  env.fetcher.fetchBytes = async (url: string, maxBytes: number): Promise<Uint8Array> => {
    if (!fired && url.endsWith('/hew.json')) {
      fired = true
      await hook()
    }
    return real(url, maxBytes)
  }
}

const followed = (on: boolean): void => {
  const e = env.store.sourcesFile?.sources.find((s) => s.source === SRC)
  if (e === undefined) throw new Error('no entry')
  e.followed = on
}

// --------------------------------------------------------------------- F4

describe('F4: a file at the minted path that the Library scan cannot read is never overwritten', () => {
  it('a damaged file at the minted path is name_clash and stays as it was', async () => {
    await withSource()
    env.store.items.set(FILE, enc.encode('truncated'))
    const before = env.store.snapshot()
    expect(await env.sources.addItem(SRC, ID)).toMatchObject({ ok: false, code: 'name_clash' })
    expectUntouched(before)
  })

  it('a file with no id at the minted path (not a Library item, or a newer format) is name_clash', async () => {
    await withSource()
    env.store.items.set(FILE, enc.encode(JSON.stringify({ lib: { name: 'Part 1' }, body: 'from a newer Hew' })))
    const before = env.store.snapshot()
    expect(await env.sources.addItem(SRC, ID)).toMatchObject({ ok: false, code: 'name_clash' })
    expectUntouched(before)
  })

  it('an update whose new minted path holds an unreadable file is name_clash and removes nothing', async () => {
    await newerSetup({ name: 'Renamed part' })
    env.store.items.set(itemFileName('Renamed part', ID, 'component'), enc.encode('damaged'))
    const before = env.store.snapshot()
    expect(await env.sources.updateItem(SRC, ID)).toMatchObject({ ok: false, code: 'name_clash' })
    expectUntouched(before)
  })

  it('a different id at the minted path spelled in another case is name_clash', async () => {
    await withSource()
    env.store.seed('Components/Part-1-000001.hew', { id: CLASH_ID, name: 'Part 1' })
    const before = env.store.snapshot()
    expect(await env.sources.addItem(SRC, ID)).toMatchObject({ ok: false, code: 'name_clash' })
    expectUntouched(before)
  })

  it('an unreadable file at the minted path spelled in another case is name_clash', async () => {
    await withSource()
    env.store.items.set('Components/PART-1-000001.hew', enc.encode('damaged'))
    const before = env.store.snapshot()
    expect(await env.sources.addItem(SRC, ID)).toMatchObject({ ok: false, code: 'name_clash' })
    expectUntouched(before)
  })

  it('the same item at the minted path in another case is replaced in place, never written beside and then removed', async () => {
    await withSource()
    const mine = 'Components/Part-1-000001.hew'
    env.store.seed(mine, { id: ID, name: 'Part 1' })
    const r = await env.sources.addItem(SRC, ID, { replace: true })
    expect(r).toMatchObject({ ok: true, file: mine, replaced: true })
    // On a case-insensitive disk, writing the lower-case path and then
    // removing this one would delete the item just written.
    expect(env.store.writes).toEqual([mine])
    expect(env.store.removes).toEqual([])
  })
})

// --------------------------------------------------------------------- F6

describe('F6: Add with replace refuses an item open for editing, as Update does', () => {
  it('replace:true on an item open in an edit window is open_for_editing; nothing is fetched or written', async () => {
    await withSource()
    env.store.seed(FILE, { id: ID, name: 'Part 1', collection: 'Mine' })
    env.store.editing = [FILE]
    quiet()
    const before = env.store.snapshot()
    expect(await env.sources.addItem(SRC, ID, { replace: true })).toMatchObject({ ok: false, code: 'open_for_editing' })
    expect(env.fetcher.calls).toHaveLength(0)
    expectUntouched(before)
  })

  it('without replace it refuses before asking, so the user is not asked a question whose Yes cannot work', async () => {
    await withSource()
    env.store.seed(FILE, { id: ID, name: 'Part 1' })
    env.store.editing = [FILE]
    expect(await env.sources.addItem(SRC, ID)).toMatchObject({ ok: false, code: 'open_for_editing' })
  })

  it('an item that is not open still asks and then replaces', async () => {
    await withSource()
    env.store.seed(FILE, { id: ID, name: 'Part 1' })
    env.store.editing = ['Components/something-else-000009.hew']
    expect(await env.sources.addItem(SRC, ID)).toMatchObject({ ok: false, code: 'needs_confirm' })
    expect(await env.sources.addItem(SRC, ID, { replace: true })).toMatchObject({ ok: true, file: FILE })
  })
})

// -------------------------------------------------------------------- F10

describe('F10: a check in flight does not overwrite changes made meanwhile', () => {
  it('a delete during the check keeps its skip-list entry', async () => {
    await withSource([item1(), ...parts(2).slice(1)])
    followed(true)
    duringIndexFetch(() => env.sources.noteDeleted(ID, { source: SRC }))
    expect(await env.sources.check(SRC)).toMatchObject({ ok: true })
    expect(env.store.entryFor(SRC)?.skipped).toEqual([ID])
    expect(env.store.entryFor(SRC)?.lastCheck).toMatchObject({ ok: true })
  })

  it('unfollow during the check stays unfollowed', async () => {
    await withSource()
    followed(true)
    duringIndexFetch(() => env.sources.unfollow(SRC))
    await env.sources.check(SRC)
    expect(env.store.entryFor(SRC)?.followed).toBe(false)
  })

  it('Keep mine during the check keeps its Kept record', async () => {
    const { pub2 } = await newerSetup()
    duringIndexFetch(() => env.sources.keepMine(SRC, ID))
    await env.sources.check(SRC)
    expect(env.store.entryFor(SRC)?.kept).toEqual({ [ID]: pub2.sha[ID] })
  })

  it('a source removed during the check is not added back', async () => {
    await withSource()
    duringIndexFetch(() => env.sources.removeSource(SRC))
    await env.sources.check(SRC)
    expect(await env.sources.listSources()).toEqual([])
    expect(env.store.entryFor(SRC)).toBeUndefined()
  })

  it('on a failed check: the skip list survives and a removed source is not added back', async () => {
    await withSource()
    followed(true)
    env.server.fail((u) => u.endsWith('/hew.json'), 'network')
    duringIndexFetch(() => env.sources.noteDeleted(ID, { source: SRC }))
    expect(await env.sources.check(SRC)).toMatchObject({ ok: false, code: 'network' })
    expect(env.store.entryFor(SRC)?.skipped).toEqual([ID])
    expect(env.store.entryFor(SRC)?.lastCheck).toMatchObject({ ok: false, code: 'network' })

    duringIndexFetch(() => env.sources.removeSource(SRC))
    await env.sources.check(SRC)
    expect(env.store.entryFor(SRC)).toBeUndefined()
  })

  it('the check still records what it owns: lastCheck, lastGoodAt, commit and name', async () => {
    await withSource()
    env.server.publish(SRC, { name: 'Shop Hardware Two', commit: 'c9', items: [item1()] })
    env.clock.advance(1000)
    await env.sources.check(SRC)
    expect(env.store.entryFor(SRC)).toMatchObject({ name: 'Shop Hardware Two', commit: 'c9', lastGoodAt: env.clock.iso(), lastCheck: { at: env.clock.iso(), ok: true, code: null } })
  })
})

// -------------------------------------------------------------------- F12

describe('F12: a failed remove of the old file reaches every result', () => {
  it('an Update all row whose old file cannot be removed reports removeFailed', async () => {
    await newerSetup({ name: 'Renamed part' })
    env.store.failRemove.add(FILE)
    const r = await env.sources.updateAll(SRC)
    if (!('ok' in r) || !r.ok) throw new Error('update all failed')
    expect(r.rows).toEqual([{ id: ID, result: expect.objectContaining({ ok: true, removeFailed: true }) }])
  })

  it('Add with replace whose old file cannot be removed reports removeFailed', async () => {
    await withSource()
    const old = 'Components/my-hinge-000001.hew'
    env.store.seed(old, { id: ID, name: 'My hinge' })
    env.store.failRemove.add(old)
    expect(await env.sources.addItem(SRC, ID, { replace: true })).toMatchObject({ ok: true, file: FILE, removeFailed: true })
  })

  it('a failed write of the sources file after the item landed does not throw away the result', async () => {
    const { pub2 } = await newerSetup({ name: 'Renamed part' })
    await env.sources.keepMine(SRC, ID)
    env.store.failRemove.add(FILE)
    env.store.writeSources = async () => {
      throw new Error('disk full')
    }
    const r = await env.sources.updateItem(SRC, ID)
    expect(r).toMatchObject({ ok: true, file: itemFileName('Renamed part', ID, 'component'), removeFailed: true, sha256: pub2.sha[ID] })
  })
})

// -------------------------------------------------------------------- F16

describe('F16: opening an added source with no cached listing makes no request when the check setting is off', () => {
  async function addedWithoutCache(): Promise<void> {
    await env.addSource(SRC, { name: 'Shop Hardware', items: parts(2) })
    env.store.cache.clear()
    quiet()
  }

  it('addSource returns the entry, an empty summary and needsCheck, with no request and no write', async () => {
    env.options.autoCheck = false
    await addedWithoutCache()
    const r = await env.sources.addSource(SRC)
    expect(r).toMatchObject({ ok: true, existing: true, needsCheck: true, entry: { source: SRC, name: 'Shop Hardware' } })
    if (!('ok' in r) || !r.ok) return
    expect(r.summary).toMatchObject({ name: 'Shop Hardware', itemCount: 0, totalBytes: 0 })
    expect(env.fetcher.calls).toHaveLength(0)
    expect(env.store.sourceWrites).toHaveLength(0)
    expect(env.store.cacheWrites).toHaveLength(0)
  })

  it('peekSource does the same', async () => {
    env.options.autoCheck = false
    await addedWithoutCache()
    const r = await env.sources.peekSource(SRC)
    expect(r).toMatchObject({ ok: true, existing: true, needsCheck: true, source: SRC, summary: { name: 'Shop Hardware', itemCount: 0 } })
    expect(env.fetcher.calls).toHaveLength(0)
  })

  it('with the setting on, addSource still checks and returns the listing', async () => {
    await addedWithoutCache()
    const r = await env.sources.addSource(SRC)
    expect(r).toMatchObject({ ok: true, existing: true, summary: { itemCount: 2 } })
    expect(r).not.toHaveProperty('needsCheck')
    expect(env.fetcher.calls.length).toBeGreaterThan(0)
  })

  it('with the setting off and a cached listing, the summary comes from the cache as before', async () => {
    env.options.autoCheck = false
    await env.addSource(SRC, { name: 'Shop Hardware', items: parts(2) })
    quiet()
    const r = await env.sources.addSource(SRC)
    expect(r).toMatchObject({ ok: true, existing: true, summary: { itemCount: 2 } })
    expect(r).not.toHaveProperty('needsCheck')
    expect(env.fetcher.calls).toHaveLength(0)
  })
})

// --------------------------------------------------------------------- F1

describe('F1: catalog entries are validated like parse_catalog, and a snapshot never leaves the catalog folder', () => {
  const good = (over: Partial<CatalogSourceEntry> & Record<string, unknown> = {}): CatalogSourceEntry =>
    ({ source: SRC, kind: 'github', name: 'Shop', items: 1, bytes: 10, categories: {}, stale: false, snapshot: 'sources/a.json', ...over }) as CatalogSourceEntry

  it('drops an entry whose snapshot, source, kind, commit, homepage or licenseUrl breaks the rules', async () => {
    const bad: Array<Record<string, unknown>> = [
      { snapshot: '/victim/repo/main/hew.json' },
      { snapshot: '../../../victim/repo/main/hew.json' },
      { snapshot: 'sources/../../x.json' },
      { snapshot: 'https://elsewhere.test/x.json' },
      { snapshot: '//elsewhere.test/x.json' },
      { snapshot: 'sources\\a.json' },
      { snapshot: 'sources/a.json?x=1' },
      { snapshot: 'sources/a.json#x' },
      { snapshot: '%2e%2e/%2e%2e/x.json' },
      { snapshot: 'sources/%2E%2E/x.json' },
      { snapshot: 'sources/a.txt' },
      { snapshot: '' },
      { snapshot: undefined },
      { snapshot: 7 },
      { source: 'not a reference' },
      { source: ' github:acme/hinges' },
      { kind: 'svn' },
      { commit: 'abc123' },
      { commit: 'A'.repeat(40) },
      { homepage: 'http://example.com' },
      { licenseUrl: 'javascript:alert(1)' },
      { source: 'web:https://parts.test/hew.json', kind: 'web', commit: 'a'.repeat(40) },
    ]
    env.server.publishCatalog({
      sources: [
        good({ name: 'kept-1' }),
        ...bad.map((o, i) => good({ name: `bad-${i}`, ...o })),
        good({ name: 'kept-2', commit: 'b'.repeat(40), homepage: 'https://example.com', licenseUrl: 'https://example.com/l' }),
        good({ name: 'kept-3', commit: 'c'.repeat(64) }),
      ],
    })
    const r = await env.sources.catalog()
    if (!('ok' in r) || !r.ok) throw new Error('catalog failed')
    expect(r.catalog.sources.map((s) => s.name)).toEqual(['kept-1', 'kept-2', 'kept-3'])
  })

  it('keeps a removed entry but drops a link that is not https', async () => {
    env.server.publishCatalog({ removed: [{ source: SRC, severity: 'removed', reason: 'x', link: 'http://example.com/why' }] })
    const r = await env.sources.catalog()
    if (!('ok' in r) || !r.ok) throw new Error('catalog failed')
    expect(r.catalog.removed).toEqual([{ source: SRC, severity: 'removed', reason: 'x' }])
  })

  it.each([
    '/victim/repo/main/hew.json',
    '../../../victim/repo/main/hew.json',
    '../catalog-two/x.json',
    '%2e%2e/%2e%2e/victim/hew.json',
    'https://elsewhere.test/x.json',
    'sources/a.json?x',
  ])('catalogPreview refuses snapshot %s with catalog_invalid and makes no request', async (snapshot) => {
    const r = await env.sources.catalogPreview(good({ snapshot }))
    expect(r).toMatchObject({ ok: false, code: 'catalog_invalid' })
    expect(env.fetcher.calls).toHaveLength(0)
  })

  it('catalogPreview reads a good snapshot from inside the catalog folder', async () => {
    const pub = env.server.publish(SRC, { name: 'Shop', items: parts(1) })
    const url = `https://catalog.example.test/data/sources/a.json`
    env.server.put(url, enc.encode(JSON.stringify(pub.index)))
    quiet()
    expect(await env.sources.catalogPreview(good())).toMatchObject({ ok: true, stale: false })
    expect(env.fetcher.calls).toEqual([{ url, maxBytes: MAX_INDEX }])
  })
})

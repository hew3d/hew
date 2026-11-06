// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createWebFollowHost, mergeThread, startWebLaunchCheck } from './webFollowHost'
import { FOLLOWED_REPORTS_CORRUPT_KEY, FOLLOWED_REPORTS_KEY, LAUNCH_CHECK_AT_KEY, loadWebFollowRecords, recordWebFollow, saveWebFollowRecords, type WebFollowRecord } from './webFollowStore'

const SECRET = 'S'.repeat(43)
const ID = 'HEW-AAAA-BBBB'

function rec(o: Partial<WebFollowRecord> = {}): WebFollowRecord {
  return {
    id: ID, secret: SECRET, sentAt: Date.now() - 1000, appVersion: '1.3.1', excerpt: 'x', fileName: null, status: 'open', statusVersion: null,
    statusAt: null, messages: [], latest: 0, seenUpTo: 0, lastCheckedAt: null, state: 'following', seenSynced: 0, ...o,
  }
}

interface Call { url: string; method: string; headers: Record<string, string>; body?: string }
let calls: Call[]
function fakeFetch(respond: (c: Call) => Response | 'network') {
  calls = []
  vi.stubGlobal('fetch', vi.fn(async (url: string, init: RequestInit) => {
    const c: Call = { url, method: init.method ?? 'GET', headers: init.headers as Record<string, string>, body: init.body as string | undefined }
    calls.push(c)
    const a = respond(c)
    if (a === 'network') throw new TypeError('x')
    return a
  }))
}
const j = (status: number, body: unknown, headers?: Record<string, string>) => new Response(JSON.stringify(body), { status, headers })
const thread = (o: Record<string, unknown> = {}) => ({
  format: 1, status: 'open', statusVersion: null, statusAt: null, latest: 0, messages: [], now: 1, ...o,
})
const m = (seq: number, author = 'maintainer', kind = 'text') => ({ seq, at: seq, author, kind, text: `m${seq}` })

beforeEach(() => localStorage.clear())
afterEach(() => {
  vi.unstubAllGlobals()
  vi.useRealTimers()
})

describe('store', () => {
  it('tolerates corrupt JSON, wrong shapes, and bad records', async () => {
    const host = createWebFollowHost()
    localStorage.setItem(FOLLOWED_REPORTS_KEY, '{not json')
    expect(await host.list()).toEqual([])
    localStorage.setItem(FOLLOWED_REPORTS_KEY, '"str"')
    expect(await host.list()).toEqual([])
    localStorage.setItem(FOLLOWED_REPORTS_KEY, JSON.stringify([null, { id: 1 }, { id: 'A', secret: '' }, rec(), rec()]))
    expect(await host.list()).toHaveLength(1)
  })
  it('list never exposes the secret', async () => {
    saveWebFollowRecords([rec()])
    const out = await createWebFollowHost().list()
    expect(JSON.stringify(out)).not.toContain(SECRET)
  })
})

describe('mergeThread', () => {
  it('merges by seq without duplicates and counts news', () => {
    const base = rec({ messages: [m(1) as never], latest: 1, seenUpTo: 1 })
    const r = mergeThread(base, { status: 'need-info', statusVersion: null, statusAt: null, latest: 3, messages: [m(1), m(2), m(3, 'reporter')] as never }, 5)
    expect(r.rec.messages.map((x) => x.seq)).toEqual([1, 2, 3])
    expect(r.newCount).toBe(1)
    expect(r.rec.status).toBe('need-info')
    expect(r.rec.lastCheckedAt).toBe(5)
  })
})

describe('check', () => {
  it('fetches after=latest with the secret header, merges, and summarizes', async () => {
    saveWebFollowRecords([rec({ latest: 2, messages: [m(2) as never], seenUpTo: 2 })])
    fakeFetch(() => j(200, thread({ status: 'fixed', statusVersion: '1.3.2', statusAt: 9, latest: 4, messages: [m(3), m(4, 'system', 'status')] })))
    const res = await createWebFollowHost().check('manual')
    expect(calls[0].url).toBe(`/report/${ID}/thread?after=2`)
    expect(calls[0].headers['Hew-Follow-Secret']).toBe(SECRET)
    expect(res.summary).toEqual({ checked: 1, withNews: 1, newCount: 2, failed: 0 })
    expect(res.reports[0]).toMatchObject({ status: 'fixed', statusVersion: '1.3.2', latest: 4, state: 'following' })
    expect(JSON.stringify(res)).not.toContain(SECRET)
  })
  it('403 makes the report gone and done', async () => {
    saveWebFollowRecords([rec()])
    fakeFetch(() => j(403, { error: 'forbidden' }))
    const res = await createWebFollowHost().check('manual')
    expect(res.reports[0]).toMatchObject({ status: 'gone', state: 'done' })
    expect(res.summary.failed).toBe(0)
  })
  it('counts network failures and leaves the record untouched', async () => {
    saveWebFollowRecords([rec()])
    fakeFetch(() => 'network')
    const res = await createWebFollowHost().check('manual')
    expect(res.summary).toMatchObject({ checked: 1, failed: 1 })
    expect(res.reports[0]!.lastCheckedAt).toBeNull()
  })
  it('launch reason skips recently checked, done, and old reports', async () => {
    const now = Date.now()
    saveWebFollowRecords([
      rec({ id: 'A-1', lastCheckedAt: now - 1000 }),
      rec({ id: 'B-1', state: 'done' }),
      rec({ id: 'C-1', sentAt: now - 400 * 86_400_000 }),
      rec({ id: 'D-1' }),
    ])
    fakeFetch(() => j(200, thread()))
    await createWebFollowHost().check('launch')
    expect(calls.map((c) => c.url)).toEqual(['/report/D-1/thread?after=0'])
  })
  it('manual check of one id works on an old report', async () => {
    saveWebFollowRecords([rec({ sentAt: Date.now() - 400 * 86_400_000 })])
    fakeFetch(() => j(200, thread()))
    await createWebFollowHost().check('manual', ID)
    expect(calls).toHaveLength(1)
  })
})

describe('markSeen', () => {
  it('moves to done when the final status is seen, and tells the service', async () => {
    saveWebFollowRecords([rec({ status: 'fixed', latest: 2, messages: [m(1) as never, m(2, 'system', 'status') as never] })])
    fakeFetch(() => new Response(null, { status: 204 }))
    const host = createWebFollowHost()
    expect((await host.markSeen(ID, 1))!.state).toBe('following')
    const done = await host.markSeen(ID, 99)
    expect(done).toMatchObject({ state: 'done', seenUpTo: 2 })
    expect(calls.map((c) => c.url)).toEqual([`/report/${ID}/thread/seen`, `/report/${ID}/thread/seen`])
    expect(JSON.parse(calls[1].body!)).toEqual({ upTo: 2 })
  })
  it('keeps the local mark when the service call fails; 403 makes it gone', async () => {
    saveWebFollowRecords([rec({ latest: 1, messages: [m(1) as never] })])
    fakeFetch(() => 'network')
    expect((await createWebFollowHost().markSeen(ID, 1))!.seenUpTo).toBe(1)
    saveWebFollowRecords([rec({ latest: 1, messages: [m(1) as never] })])
    fakeFetch(() => j(403, { error: 'forbidden' }))
    expect((await createWebFollowHost().markSeen(ID, 1))!.status).toBe('gone')
  })
})

describe('post, forget, attach', () => {
  it('post sends JSON then refreshes; 429 carries retryAfter', async () => {
    saveWebFollowRecords([rec()])
    fakeFetch((c) => (c.method === 'POST' ? j(201, { seq: 1 }) : j(200, thread({ latest: 1, messages: [m(1, 'reporter')] }))))
    const out = await createWebFollowHost().post(ID, 'hello')
    expect(JSON.parse(calls[0].body!)).toEqual({ text: 'hello' })
    expect(out.messages.map((x) => x.seq)).toEqual([1])
    fakeFetch(() => j(429, { error: 'rate-limited' }, { 'Retry-After': '30' }))
    await expect(createWebFollowHost().post(ID, 'again')).rejects.toMatchObject({ kind: 'rateLimited', retryAfter: 30 })
  })
  it('forget deletes best-effort then removes the record', async () => {
    saveWebFollowRecords([rec()])
    fakeFetch(() => 'network')
    await createWebFollowHost().forget(ID)
    expect(calls[0].method).toBe('DELETE')
    expect(loadWebFollowRecords()).toEqual([])
  })
  it('attach starts a follow-up with the parent and secret', async () => {
    saveWebFollowRecords([rec()])
    fakeFetch((c) => {
      if (c.method === 'POST' && c.url === '/report/') return j(201, { id: 'HEW-NEW0-0001', token: 't'.repeat(43) })
      if (c.url.endsWith('/commit')) return j(201, { id: 'HEW-NEW0-0001' })
      return j(200, thread())
    })
    const r = await createWebFollowHost().attach(ID, new Uint8Array([1, 2, 3]))
    expect(r).toEqual({ id: 'HEW-NEW0-0001', following: false })
    expect(calls[0].headers['Hew-Follow-Up']).toBe(ID)
    expect(calls[0].headers['Hew-Follow-Secret']).toBe(SECRET)
  })
})

describe('subscribe', () => {
  it('fires on local writes and on storage events, and unsubscribes', () => {
    const host = createWebFollowHost()
    const cb = vi.fn()
    const off = host.subscribe(cb)
    recordWebFollow({ id: ID, secret: SECRET, excerpt: 'x', fileName: null })
    expect(cb).toHaveBeenCalledTimes(1)
    window.dispatchEvent(new StorageEvent('storage', { key: FOLLOWED_REPORTS_KEY }))
    window.dispatchEvent(new StorageEvent('storage', { key: 'other' }))
    expect(cb).toHaveBeenCalledTimes(2)
    off()
    window.dispatchEvent(new StorageEvent('storage', { key: FOLLOWED_REPORTS_KEY }))
    expect(cb).toHaveBeenCalledTimes(2)
  })
})

describe('startWebLaunchCheck', () => {
  it('checks once after the delay and not again within 6 hours', async () => {
    vi.useFakeTimers()
    saveWebFollowRecords([rec()])
    fakeFetch(() => j(200, thread()))
    const host = createWebFollowHost()
    startWebLaunchCheck(host, 15_000)
    await vi.advanceTimersByTimeAsync(14_999)
    expect(calls).toHaveLength(0)
    await vi.advanceTimersByTimeAsync(2)
    expect(calls).toHaveLength(1)
    expect(localStorage.getItem(LAUNCH_CHECK_AT_KEY)).not.toBeNull()
    startWebLaunchCheck(host, 15_000)
    await vi.advanceTimersByTimeAsync(20_000)
    expect(calls).toHaveLength(1)
  })
  it('cancel prevents the check', async () => {
    vi.useFakeTimers()
    saveWebFollowRecords([rec()])
    fakeFetch(() => j(200, thread()))
    startWebLaunchCheck(createWebFollowHost(), 100)()
    await vi.advanceTimersByTimeAsync(1000)
    expect(calls).toHaveLength(0)
  })
})

describe('concurrent changes are not clobbered', () => {
  function deferred() {
    let resolve!: (r: Response) => void
    const promise = new Promise<Response>((r) => (resolve = r))
    return { promise, resolve }
  }

  it('a check keeps a seenUpTo advanced while the fetch was in flight', async () => {
    saveWebFollowRecords([rec({ latest: 1, messages: [m(1) as never] })])
    const d = deferred()
    vi.stubGlobal('fetch', vi.fn(() => d.promise))
    const host = createWebFollowHost()
    const pending = host.check('manual')
    // The user views the thread while the request is out.
    saveWebFollowRecords([rec({ latest: 1, messages: [m(1) as never], seenUpTo: 1, seenSynced: 1 })])
    d.resolve(j(200, thread({ latest: 2, messages: [m(2)] })))
    await pending
    expect(loadWebFollowRecords()[0]).toMatchObject({ seenUpTo: 1, seenSynced: 1, latest: 2 })
    expect(loadWebFollowRecords()[0]!.messages.map((x) => x.seq)).toEqual([1, 2])
  })

  it('a check does not resurrect a report forgotten meanwhile', async () => {
    saveWebFollowRecords([rec()])
    const d = deferred()
    vi.stubGlobal('fetch', vi.fn(() => d.promise))
    const pending = createWebFollowHost().check('manual')
    saveWebFollowRecords([])
    d.resolve(j(200, thread({ latest: 1, messages: [m(1)] })))
    await pending
    expect(loadWebFollowRecords()).toEqual([])
  })

  it('a failed refresh after post writes nothing and returns the stored report', async () => {
    saveWebFollowRecords([rec()])
    fakeFetch((c) => (c.method === 'POST' ? j(201, { seq: 1 }) : 'network'))
    const out = await createWebFollowHost().post(ID, 'hello')
    expect(out.id).toBe(ID)
    expect(loadWebFollowRecords()[0]!.lastCheckedAt).toBeNull()
  })

  it('an older view does not roll the status back', () => {
    const base = rec({ status: 'fixed', statusVersion: '1.3.2', latest: 5 })
    const r = mergeThread(base, { status: 'open', statusVersion: null, statusAt: null, latest: 3, messages: [] }, 9)
    expect(r.rec).toMatchObject({ status: 'fixed', statusVersion: '1.3.2', latest: 5 })
  })
})

describe('403 handling', () => {
  it('a plain 403 (no forbidden body) is a transient failure, not gone', async () => {
    saveWebFollowRecords([rec()])
    fakeFetch(() => new Response('<html>blocked</html>', { status: 403 }))
    const res = await createWebFollowHost().check('manual')
    expect(res.summary.failed).toBe(1)
    expect(res.reports[0]).toMatchObject({ status: 'open', state: 'following' })
    fakeFetch(() => j(403, { error: 'other' }))
    expect((await createWebFollowHost().check('manual')).summary.failed).toBe(1)
  })
})

describe('seen retry', () => {
  it('resends the seen mark until the service acknowledges it', async () => {
    saveWebFollowRecords([rec({ latest: 1, messages: [m(1) as never] })])
    fakeFetch(() => 'network')
    const host = createWebFollowHost()
    await host.markSeen(ID, 1)
    expect(loadWebFollowRecords()[0]).toMatchObject({ seenUpTo: 1, seenSynced: 0 })
    fakeFetch(() => new Response(null, { status: 204 }))
    await host.markSeen(ID, 1)
    expect(calls).toHaveLength(1)
    expect(JSON.parse(calls[0].body!)).toEqual({ upTo: 1 })
    expect(loadWebFollowRecords()[0]).toMatchObject({ seenUpTo: 1, seenSynced: 1 })
    await host.markSeen(ID, 1)
    expect(calls).toHaveLength(1)
  })
  it('a later check resends a seen mark that never got through', async () => {
    saveWebFollowRecords([rec({ latest: 2, messages: [m(2) as never], seenUpTo: 2, seenSynced: 0 })])
    fakeFetch((c) => (c.url.endsWith('/seen') ? new Response(null, { status: 204 }) : j(200, thread({ latest: 2 }))))
    await createWebFollowHost().check('manual')
    expect(calls.map((c) => c.url)).toEqual([`/report/${ID}/thread?after=2`, `/report/${ID}/thread/seen`])
    expect(loadWebFollowRecords()[0]).toMatchObject({ seenUpTo: 2, seenSynced: 2 })
    // Acknowledged now: the next check does not post again.
    await createWebFollowHost().check('manual')
    expect(calls.filter((c) => c.url.endsWith('/seen'))).toHaveLength(1)
  })
  it('a failed seen post stays unsynced until a check succeeds in sending it', async () => {
    saveWebFollowRecords([rec({ latest: 1, messages: [m(1) as never], seenUpTo: 0 })])
    fakeFetch(() => 'network')
    await createWebFollowHost().markSeen(ID, 1)
    expect(loadWebFollowRecords()[0]).toMatchObject({ seenUpTo: 1, seenSynced: 0 })
    fakeFetch((c) => (c.url.endsWith('/seen') ? new Response(null, { status: 204 }) : j(200, thread({ latest: 1 }))))
    await createWebFollowHost().check('manual')
    expect(loadWebFollowRecords()[0]).toMatchObject({ seenUpTo: 1, seenSynced: 1 })
  })
  it('a launch check resends the unacknowledged final seen mark of a done report without fetching it', async () => {
    saveWebFollowRecords([rec({ state: 'done', status: 'fixed', latest: 3, messages: [m(3, 'system', 'status') as never], seenUpTo: 3, seenSynced: 0 })])
    fakeFetch((c) => (c.url.endsWith('/seen') ? new Response(null, { status: 204 }) : j(200, thread({ latest: 3 }))))
    await createWebFollowHost().check('launch')
    expect(calls.map((c) => c.url)).toEqual([`/report/${ID}/thread/seen`])
    expect(loadWebFollowRecords()[0]).toMatchObject({ state: 'done', seenUpTo: 3, seenSynced: 3 })
    // A manual check leaves done reports alone, as before.
    await createWebFollowHost().check('manual')
    expect(calls).toHaveLength(1)
  })
  it('never exposes seenSynced in the public view', async () => {
    saveWebFollowRecords([rec({ seenSynced: 3 })])
    expect(JSON.stringify(await createWebFollowHost().list())).not.toContain('seenSynced')
  })
})

describe('store durability', () => {
  it('stashes a corrupt value before overwriting it', () => {
    localStorage.setItem(FOLLOWED_REPORTS_KEY, '{not json')
    expect(saveWebFollowRecords([rec()])).toBe(true)
    expect(localStorage.getItem(FOLLOWED_REPORTS_CORRUPT_KEY)).toBe('{not json')
    expect(loadWebFollowRecords()).toHaveLength(1)
  })
  it('does not overwrite when the store cannot be read, and reports failed writes', () => {
    const spy = vi.spyOn(Object.getPrototypeOf(localStorage) as Storage, 'getItem').mockImplementation(() => {
      throw new Error('denied')
    })
    const set = vi.spyOn(Object.getPrototypeOf(localStorage) as Storage, 'setItem')
    expect(saveWebFollowRecords([rec()])).toBe(false)
    expect(set).not.toHaveBeenCalled()
    spy.mockRestore()
    set.mockRestore()
    vi.spyOn(Object.getPrototypeOf(localStorage) as Storage, 'setItem').mockImplementation(() => {
      throw new Error('quota')
    })
    expect(saveWebFollowRecords([rec()])).toBe(false)
    vi.restoreAllMocks()
  })
  it('recordWebFollow returns whether the write stuck', () => {
    expect(recordWebFollow({ id: ID, secret: SECRET, excerpt: 'x', fileName: null })).toBe(true)
    vi.spyOn(Object.getPrototypeOf(localStorage) as Storage, 'setItem').mockImplementation(() => {
      throw new Error('quota')
    })
    expect(recordWebFollow({ id: 'HEW-CCCC-DDDD', secret: SECRET, excerpt: 'x', fileName: null })).toBe(false)
    vi.restoreAllMocks()
  })
})

describe('launch stamp', () => {
  it('a future launch stamp (clock set back) does not suppress the check', async () => {
    vi.useFakeTimers()
    saveWebFollowRecords([rec()])
    localStorage.setItem(LAUNCH_CHECK_AT_KEY, String(Date.now() + 5 * 3600_000))
    fakeFetch(() => j(200, thread()))
    startWebLaunchCheck(createWebFollowHost(), 1000)
    await vi.advanceTimersByTimeAsync(1500)
    expect(calls).toHaveLength(1)
  })
})

describe('launch stamp after the check', () => {
  it('leaves the stamp alone when every fetch fails, so a reload retries', async () => {
    vi.useFakeTimers()
    saveWebFollowRecords([rec()])
    const old = String(Date.now() - 7 * 3600_000)
    localStorage.setItem(LAUNCH_CHECK_AT_KEY, old)
    fakeFetch(() => j(503, { error: 'down' }))
    startWebLaunchCheck(createWebFollowHost(), 1000)
    await vi.advanceTimersByTimeAsync(1500)
    expect(calls).toHaveLength(1)
    expect(localStorage.getItem(LAUNCH_CHECK_AT_KEY)).toBe(old)
  })
  it('writes the stamp when at least one fetch succeeds', async () => {
    vi.useFakeTimers()
    saveWebFollowRecords([rec(), rec({ id: 'HEW-CCCC-DDDD' })])
    fakeFetch((c) => (c.url.includes(ID) ? j(503, { error: 'down' }) : j(200, thread())))
    startWebLaunchCheck(createWebFollowHost(), 1000)
    await vi.advanceTimersByTimeAsync(1500)
    expect(calls).toHaveLength(2)
    expect(Number(localStorage.getItem(LAUNCH_CHECK_AT_KEY))).toBeGreaterThan(0)
  })
})

describe('request timeout covers the body', () => {
  it('a body that stalls is aborted by the timer instead of hanging', async () => {
    vi.useFakeTimers()
    saveWebFollowRecords([rec()])
    vi.stubGlobal(
      'fetch',
      vi.fn(async (_url: string, init: RequestInit) => ({
        status: 200,
        headers: new Headers(),
        json: () =>
          new Promise((_, reject) => {
            init.signal!.addEventListener('abort', () => reject(new Error('aborted')))
          }),
      })),
    )
    const pending = createWebFollowHost().check('launch')
    await vi.advanceTimersByTimeAsync(11_000)
    const res = await pending
    expect(res.summary.failed).toBe(1)
  })
})

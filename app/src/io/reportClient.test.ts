// @vitest-environment jsdom
/**
 * io/reportClient.ts — canSend()'s origin/proxy gating, the desktop invoke
 * path (mocked, mirroring relayClient.test.ts's pattern), and the web
 * path's piece-by-piece upload: start, pieces, commit, the one-retry rule,
 * and every §8 status mapped to its documented kind.
 *
 * `isTauri` is read from `./fileHost` at import time from
 * `window.__TAURI_INTERNALS__`, so — like `settings/server.test.ts` — the
 * module under test is imported dynamically, AFTER that flag is set, with
 * `vi.resetModules()` between loads.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const mockInvoke = vi.hoisted(() => vi.fn())
vi.mock('@tauri-apps/api/core', () => ({ invoke: mockInvoke }))

const PIECE = 1_900_000
const TOKEN = 'a'.repeat(43)

/** Stands in for `reportBundle.ts`'s `send.gzip` — the gzip magic, then a body. */
const GZIP_BYTES = new Uint8Array([0x1f, 0x8b, 0x08, 0x00, 1, 2, 3])

async function loadTauriModule() {
  Object.defineProperty(window, '__TAURI_INTERNALS__', { value: {}, configurable: true })
  vi.resetModules()
  return import('./reportClient')
}

async function loadWebModule() {
  // @ts-expect-error — jsdom has no such property unless a test set it.
  delete window.__TAURI_INTERNALS__
  vi.resetModules()
  return import('./reportClient')
}

function setOrigin(origin: string) {
  Object.defineProperty(window, 'location', { value: new URL(origin), configurable: true, writable: true })
}

function json(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status })
}

interface Call {
  url: string
  method: string
  headers: Record<string, string>
  bodyLength: number
}

/** A fake intake service: records every request and answers from `respond`. */
function fakeService(respond: (call: Call, attempt: number) => Response | 'network') {
  const calls: Call[] = []
  const attempts = new Map<string, number>()
  global.fetch = vi.fn(async (url: RequestInfo | URL, init?: RequestInit) => {
    const body = init?.body as Uint8Array | undefined
    const call: Call = {
      url: String(url),
      method: init?.method ?? 'GET',
      headers: (init?.headers ?? {}) as Record<string, string>,
      bodyLength: body?.byteLength ?? 0,
    }
    calls.push(call)
    const key = `${call.method} ${call.url}`
    const attempt = (attempts.get(key) ?? 0) + 1
    attempts.set(key, attempt)
    const answer = respond(call, attempt)
    if (answer === 'network') throw new TypeError('Failed to fetch')
    return answer
  }) as typeof fetch
  return calls
}

/** The happy path: start issues an upload, pieces are accepted, commit stores. */
function happyPath(call: Call): Response {
  if (call.method === 'POST' && call.url === '/report/') {
    return json(201, { id: 'HEW-AAAA-BBBB', token: TOKEN, pieceBytes: PIECE })
  }
  if (call.method === 'PUT') return new Response(null, { status: 204 })
  if (call.url.endsWith('/commit')) return json(201, { id: 'HEW-AAAA-BBBB' })
  return json(500, { error: 'unavailable' })
}

beforeEach(() => {
  mockInvoke.mockReset()
  setOrigin('http://localhost/')
})

afterEach(() => {
  // @ts-expect-error — see loadWebModule.
  delete window.__TAURI_INTERNALS__
  vi.unstubAllGlobals()
  vi.restoreAllMocks()
})

describe('canSend', () => {
  it('is always true on desktop, whatever the origin', async () => {
    const m = await loadTauriModule()
    expect(m.canSend()).toBe(true)
  })

  it('is true on the web only at the production app origin', async () => {
    const m = await loadWebModule()
    expect(m.canSend()).toBe(false)
    setOrigin('https://app.hew3d.com/')
    expect(m.canSend()).toBe(true)
  })

  it('is true on the web off that origin only when the dev proxy flag is set', async () => {
    const m = await loadWebModule()
    expect(m.canSend()).toBe(false)
    vi.stubGlobal('__HEW_REPORT_DEV_PROXY__', true)
    expect(m.canSend()).toBe(true)
  })
})

describe('submitReport — desktop', () => {
  it('sends the gzip bytes as the RAW invoke body and returns the id', async () => {
    const m = await loadTauriModule()
    mockInvoke.mockImplementation(async () => ({ id: 'HEW-7K3F-Q9XB' }))
    const result = await m.submitReport(GZIP_BYTES)
    expect(result).toEqual({ id: 'HEW-7K3F-Q9XB', following: false })
    expect(mockInvoke).toHaveBeenCalledWith('report_submit', GZIP_BYTES)
  })

  it('surfaces a rejected invoke as a typed ReportError', async () => {
    const m = await loadTauriModule()
    mockInvoke.mockImplementation(() =>
      Promise.reject({ kind: 'tooLarge', message: 'the report is too large to send' }),
    )
    await expect(m.submitReport(GZIP_BYTES)).rejects.toMatchObject({ kind: 'tooLarge' })
  })

  it('passes the follow option in the hew-options header and returns following', async () => {
    const m = await loadTauriModule()
    mockInvoke.mockImplementation(async () => ({ id: 'HEW-7K3F-Q9XB', following: true }))
    const result = await m.submitReport(GZIP_BYTES, undefined, { follow: true, excerpt: 'It crashes', fileName: 'a.skp' })
    expect(result).toEqual({ id: 'HEW-7K3F-Q9XB', following: true })
    const [cmd, body, opts] = mockInvoke.mock.calls[0]
    expect(cmd).toBe('report_submit')
    expect(body).toBe(GZIP_BYTES)
    expect(JSON.parse(decodeURIComponent(opts.headers['hew-options']))).toEqual({ follow: true, excerpt: 'It crashes', fileName: 'a.skp' })
  })

  it('treats an unrecognized rejection shape as unreachable', async () => {
    const m = await loadTauriModule()
    mockInvoke.mockImplementation(() => Promise.reject('ipc failure'))
    await expect(m.submitReport(GZIP_BYTES)).rejects.toMatchObject({ kind: 'unreachable' })
  })
})

describe('submitReport — web upload', () => {
  const originalFetch = global.fetch

  afterEach(() => {
    global.fetch = originalFetch
  })

  it('sends a one-piece report as start then commit, with the declared length and token', async () => {
    const m = await loadWebModule()
    const calls = fakeService(happyPath)
    const result = await m.submitReport(GZIP_BYTES)

    expect(result).toEqual({ id: 'HEW-AAAA-BBBB', following: false })
    expect(calls.map((c) => `${c.method} ${c.url}`)).toEqual(['POST /report/', 'POST /report/HEW-AAAA-BBBB/commit'])
    expect(calls[0].headers).toEqual({ 'Content-Type': 'application/gzip', 'Hew-Upload-Length': '7' })
    expect(calls[0].bodyLength).toBe(7)
    expect(calls[1].headers).toEqual({ 'Hew-Upload-Token': TOKEN })
  })

  it('splits a larger report into pieces sent in order, reporting progress after each', async () => {
    const m = await loadWebModule()
    const calls = fakeService(happyPath)
    const bytes = new Uint8Array(PIECE * 2 + 5)
    const progress: number[] = []

    await m.submitReport(bytes, (p) => {
      expect(p.totalBytes).toBe(bytes.byteLength)
      progress.push(p.sentBytes)
    })

    expect(calls.map((c) => `${c.method} ${c.url} ${c.bodyLength}`)).toEqual([
      `POST /report/ ${PIECE}`,
      `PUT /report/HEW-AAAA-BBBB/1 ${PIECE}`,
      'PUT /report/HEW-AAAA-BBBB/2 5',
      'POST /report/HEW-AAAA-BBBB/commit 0',
    ])
    expect(calls[1].headers).toEqual({ 'Content-Type': 'application/gzip', 'Hew-Upload-Token': TOKEN })
    expect(progress).toEqual([PIECE, PIECE * 2, PIECE * 2 + 5])
  })

  it('retries a piece once after a network error', async () => {
    const m = await loadWebModule()
    const calls = fakeService((call, attempt) => (call.method === 'PUT' && attempt === 1 ? 'network' : happyPath(call)))
    await expect(m.submitReport(new Uint8Array(PIECE + 1))).resolves.toEqual({ id: 'HEW-AAAA-BBBB', following: false })
    expect(calls.filter((c) => c.method === 'PUT')).toHaveLength(2)
  })

  it('gives up as offline when the retry also fails at the network level', async () => {
    const m = await loadWebModule()
    fakeService((call) => (call.method === 'PUT' ? 'network' : happyPath(call)))
    await expect(m.submitReport(new Uint8Array(PIECE + 1))).rejects.toMatchObject({ kind: 'offline' })
  })

  it('gives up as unreachable after a 5xx and its retry', async () => {
    const m = await loadWebModule()
    const calls = fakeService((call) => (call.method === 'PUT' ? json(503, { error: 'unavailable' }) : happyPath(call)))
    await expect(m.submitReport(new Uint8Array(PIECE + 1))).rejects.toMatchObject({ kind: 'unreachable' })
    expect(calls.filter((c) => c.method === 'PUT')).toHaveLength(2)
  })

  it('does not retry a 4xx, and maps an out-of-order piece to the status kind', async () => {
    const m = await loadWebModule()
    const calls = fakeService((call) =>
      call.method === 'PUT' ? json(409, { error: 'out-of-order', expected: 3 }) : happyPath(call),
    )
    await expect(m.submitReport(new Uint8Array(PIECE + 1))).rejects.toMatchObject({ kind: 'status', status: 409 })
    expect(calls.filter((c) => c.method === 'PUT')).toHaveLength(1)
  })

  it('retries a lost commit, which the service answers 201 again', async () => {
    const m = await loadWebModule()
    fakeService((call, attempt) => {
      if (call.url.endsWith('/commit') && attempt === 1) return 'network'
      return happyPath(call)
    })
    await expect(m.submitReport(GZIP_BYTES)).resolves.toEqual({ id: 'HEW-AAAA-BBBB', following: false })
  })

  it('never counts a retried commit that finds no upload as sent', async () => {
    const m = await loadWebModule()
    fakeService((call, attempt) => {
      if (call.url.endsWith('/commit')) return attempt === 1 ? json(503, { error: 'unavailable' }) : json(404, { error: 'not-found' })
      return happyPath(call)
    })
    await expect(m.submitReport(GZIP_BYTES)).rejects.toMatchObject({ kind: 'status', status: 404 })
  })

  it('reports a 404 on a first commit attempt as the status kind', async () => {
    const m = await loadWebModule()
    fakeService((call) => (call.url.endsWith('/commit') ? json(404, { error: 'not-found' }) : happyPath(call)))
    await expect(m.submitReport(GZIP_BYTES)).rejects.toMatchObject({ kind: 'status', status: 404 })
  })

  it.each([
    [400, { error: 'invalid' }, 'invalid'],
    [413, { error: 'too-large', maxBytes: 1073741824 }, 'tooLarge'],
    [429, { error: 'rate-limited' }, 'rateLimited'],
    [507, { error: 'full' }, 'full'],
    [500, { error: 'unavailable' }, 'unreachable'],
    [411, { error: 'length-required' }, 'status'],
  ])('maps a start status %i to kind %s', async (status, body, kind) => {
    const m = await loadWebModule()
    fakeService((call) => (call.url === '/report/' ? json(status, body) : happyPath(call)))
    await expect(m.submitReport(GZIP_BYTES)).rejects.toMatchObject({ kind })
  })

  it('reports offline when the start never reaches the network', async () => {
    const m = await loadWebModule()
    fakeService(() => 'network')
    await expect(m.submitReport(GZIP_BYTES)).rejects.toMatchObject({ kind: 'offline' })
  })

  it('reports the status kind when the start body is malformed', async () => {
    const m = await loadWebModule()
    fakeService((call) => (call.url === '/report/' ? new Response('not json', { status: 201 }) : happyPath(call)))
    await expect(m.submitReport(GZIP_BYTES)).rejects.toMatchObject({ kind: 'status' })
  })
})

describe('submitReport — web follow', () => {
  const originalFetch = global.fetch
  const SECRET = 's'.repeat(43)
  afterEach(() => {
    global.fetch = originalFetch
    localStorage.clear()
  })

  it('sends Hew-Follow:1, stores the record only after COMMIT 201, and returns following', async () => {
    const m = await loadWebModule()
    const calls = fakeService((call) =>
      call.method === 'POST' && call.url === '/report/' ? json(201, { id: 'HEW-AAAA-BBBB', token: TOKEN, follow: SECRET }) : happyPath(call),
    )
    const result = await m.submitReport(GZIP_BYTES, undefined, { follow: true, excerpt: 'It crashes', fileName: null })
    expect(result).toEqual({ id: 'HEW-AAAA-BBBB', following: true })
    expect(calls[0].headers['Hew-Follow']).toBe('1')
    const stored = JSON.parse(localStorage.getItem('hew.followedReports') as string)
    expect(stored[0]).toMatchObject({ id: 'HEW-AAAA-BBBB', secret: SECRET, excerpt: 'It crashes', state: 'following' })
  })

  it('reports following:false when the record could not be stored', async () => {
    const m = await loadWebModule()
    fakeService((call) =>
      call.method === 'POST' && call.url === '/report/' ? json(201, { id: 'HEW-AAAA-BBBB', token: TOKEN, follow: SECRET }) : happyPath(call),
    )
    const spy = vi.spyOn(Object.getPrototypeOf(localStorage) as Storage, 'setItem').mockImplementation(() => {
      throw new Error('quota')
    })
    try {
      const result = await m.submitReport(GZIP_BYTES, undefined, { follow: true, excerpt: 'x', fileName: null })
      expect(result).toEqual({ id: 'HEW-AAAA-BBBB', following: false })
    } finally {
      spy.mockRestore()
    }
  })

  it('stores nothing when the commit fails', async () => {
    const m = await loadWebModule()
    fakeService((call) =>
      call.url === '/report/' ? json(201, { id: 'HEW-AAAA-BBBB', token: TOKEN, follow: SECRET }) : call.url.endsWith('/commit') ? json(507, { error: 'full' }) : happyPath(call),
    )
    await expect(m.submitReport(GZIP_BYTES, undefined, { follow: true, excerpt: 'x', fileName: null })).rejects.toMatchObject({ kind: 'full' })
    expect(localStorage.getItem('hew.followedReports')).toBeNull()
  })

  it('is not following against a service that mints no secret, and sends no header when not asked', async () => {
    const m = await loadWebModule()
    const calls = fakeService(happyPath)
    expect(await m.submitReport(GZIP_BYTES, undefined, { follow: true, excerpt: 'x', fileName: null })).toEqual({ id: 'HEW-AAAA-BBBB', following: false })
    await m.submitReport(GZIP_BYTES, undefined, { follow: false, excerpt: 'x', fileName: null })
    expect(calls[2].headers['Hew-Follow']).toBeUndefined()
    expect(localStorage.getItem('hew.followedReports')).toBeNull()
  })

  it('a follow-up upload sends the parent and secret on START only', async () => {
    const m = await loadWebModule()
    const calls = fakeService(happyPath)
    const r = await m.uploadFollowUpWeb('HEW-PARENT-0001', SECRET, GZIP_BYTES)
    expect(r).toEqual({ id: 'HEW-AAAA-BBBB', following: false })
    expect(calls[0].headers['Hew-Follow-Up']).toBe('HEW-PARENT-0001')
    expect(calls[0].headers['Hew-Follow-Secret']).toBe(SECRET)
    expect(JSON.stringify(calls.slice(1).map((c) => c.headers))).not.toContain(SECRET)
  })
})

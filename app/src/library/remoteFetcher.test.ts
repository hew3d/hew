// @vitest-environment jsdom
/**
 * Package S7 oracle, part B: the RemoteFetcher (docs/design/
 * shared-library-sources.md 6.4 "Fetch rules", 6.6 messages, 7.1, decisions
 * 25 and 28). Web fetcher on an injected fetch; desktop fetcher on an injected
 * invoke; the Off rule on three kinds of page; and the logging rule.
 *
 * Decisions taken here where the doc is silent (each is also a question in
 * the report):
 *  - Off rule: no CSP meta -> not off; connect-src containing `https:` or `*`
 *    -> not off; any other connect-src (including a host-only list such as
 *    `https://example.com`) -> off. A CSP meta with no connect-src directive
 *    falls back to default-src, by CSP rules.
 *  - webFetcher takes optional deps { fetch, document, onLine }.
 */
import { readFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const mockInvoke = vi.hoisted(() => vi.fn())
vi.mock('@tauri-apps/api/core', () => ({ invoke: mockInvoke }))

import { RemoteFetchError, desktopFetcher, webFetcher, webSourcesOff, type RemoteFetchCode, type TauriInvoke } from './remoteFetcher'

const URL_OK = 'https://raw.example.test/owner/repo/hew.json'
const HOST = 'raw.example.test'

const bytes = (n: number, fill = 7) => new Uint8Array(n).fill(fill)
const resp = (body: Uint8Array | null, init: { status?: number; headers?: Record<string, string> } = {}) =>
  new Response(body as BodyInit | null, { status: init.status ?? 200, headers: init.headers })

/** A body stream that records pulls and cancels; chunks are handed out one per pull. */
function chunked(chunks: Uint8Array[]) {
  const stats = { pulls: 0, cancelled: false }
  let i = 0
  const stream = new ReadableStream<Uint8Array>(
    {
      pull(c) {
        stats.pulls++
        if (i < chunks.length) c.enqueue(chunks[i++])
        else c.close()
      },
      cancel() {
        stats.cancelled = true
      },
    },
    { highWaterMark: 0 },
  )
  return { stream, stats }
}

function fetcherOn(fetchImpl: (...a: unknown[]) => Promise<Response>, extra: { onLine?: () => boolean; document?: Document } = {}) {
  const spy = vi.fn(fetchImpl)
  return { spy, f: webFetcher({ fetch: spy as unknown as typeof fetch, onLine: () => true, ...extra }) }
}

async function codeOf(p: Promise<unknown>): Promise<RemoteFetchError> {
  try {
    await p
  } catch (e) {
    expect(e).toBeInstanceOf(RemoteFetchError)
    return e as RemoteFetchError
  }
  throw new Error('expected a RemoteFetchError, but the call resolved')
}

beforeEach(() => {
  document.head.innerHTML = ''
})
afterEach(() => {
  vi.useRealTimers()
  vi.unstubAllGlobals()
})

describe('web fetcher: request shape', () => {
  it('rejects a non-https URL as not_https before any fetch call', async () => {
    const { spy, f } = fetcherOn(async () => resp(bytes(1)))
    for (const url of ['http://example.test/a', 'ftp://example.test/a', 'data:text/plain,x', '//example.test/a', 'not a url', 'javascript:alert(1)']) {
      const e = await codeOf(f.fetchBytes(url, 100))
      expect(e.code, url).toBe('not_https')
    }
    expect(spy).not.toHaveBeenCalled()
  })

  it('sends no credentials, no referrer, an abort signal and no custom headers', async () => {
    const { spy, f } = fetcherOn(async () => resp(bytes(3)))
    await f.fetchBytes(URL_OK, 100)
    expect(spy).toHaveBeenCalledTimes(1)
    const [url, init] = spy.mock.calls[0] as unknown as [string | URL, RequestInit]
    expect(String(url)).toBe(URL_OK)
    expect(init.credentials).toBe('omit')
    expect(init.referrerPolicy === 'no-referrer' || init.referrer === '').toBe(true)
    expect(init.signal).toBeDefined()
    expect(typeof (init.signal as AbortSignal).aborted).toBe('boolean')
    const headers = init.headers
    expect(headers === undefined || Object.keys(headers as object).length === 0 || [...new Headers(headers)].length === 0).toBe(true)
    expect(init.method === undefined || init.method === 'GET').toBe(true)
  })

  it('returns the body bytes', async () => {
    const { f } = fetcherOn(async () => resp(new Uint8Array([1, 2, 3, 4])))
    const out = await f.fetchBytes(URL_OK, 100)
    expect(out).toBeInstanceOf(Uint8Array)
    expect(Array.from(out)).toEqual([1, 2, 3, 4])
  })
})

describe('web fetcher: status mapping', () => {
  it('429 is rate_limited', async () => {
    const { f } = fetcherOn(async () => resp(null, { status: 429 }))
    expect((await codeOf(f.fetchBytes(URL_OK, 100))).code).toBe('rate_limited')
  })

  it('403 with x-ratelimit-remaining 0 is rate_limited', async () => {
    const { f } = fetcherOn(async () => resp(null, { status: 403, headers: { 'x-ratelimit-remaining': '0' } }))
    expect((await codeOf(f.fetchBytes(URL_OK, 100))).code).toBe('rate_limited')
  })

  it('403 with retry-after is rate_limited', async () => {
    const { f } = fetcherOn(async () => resp(null, { status: 403, headers: { 'retry-after': '60' } }))
    expect((await codeOf(f.fetchBytes(URL_OK, 100))).code).toBe('rate_limited')
  })

  it('a plain 403, 404 and 500 are http_status carrying the status', async () => {
    for (const status of [403, 404, 500, 503]) {
      const { f } = fetcherOn(async () => resp(null, { status }))
      const e = await codeOf(f.fetchBytes(URL_OK, 100))
      expect(e.code, String(status)).toBe('http_status')
      expect(e.status).toBe(status)
    }
  })

  it('a 403 with x-ratelimit-remaining above zero is not a rate limit', async () => {
    const { f } = fetcherOn(async () => resp(null, { status: 403, headers: { 'x-ratelimit-remaining': '12' } }))
    expect((await codeOf(f.fetchBytes(URL_OK, 100))).code).toBe('http_status')
  })
})

describe('web fetcher: size caps', () => {
  it('a Content-Length over maxBytes is too_large before the body is read', async () => {
    const { stream, stats } = chunked([bytes(5), bytes(5)])
    const { f } = fetcherOn(async () => resp(stream as unknown as Uint8Array, { headers: { 'content-length': '11' } }))
    expect((await codeOf(f.fetchBytes(URL_OK, 10))).code).toBe('too_large')
    expect(stats.pulls).toBe(0)
  })

  it('a stream that runs past maxBytes without a length is too_large and the reader is cancelled', async () => {
    const { stream, stats } = chunked([bytes(6), bytes(6), bytes(6)])
    const { f } = fetcherOn(async () => resp(stream as unknown as Uint8Array))
    expect((await codeOf(f.fetchBytes(URL_OK, 10))).code).toBe('too_large')
    expect(stats.cancelled).toBe(true)
    expect(stats.pulls).toBeLessThanOrEqual(2)
  })

  it('a body of exactly maxBytes is accepted, one byte more is not', async () => {
    const ok = fetcherOn(async () => resp(bytes(10)))
    expect((await ok.f.fetchBytes(URL_OK, 10)).length).toBe(10)
    const over = fetcherOn(async () => resp(bytes(11)))
    expect((await codeOf(over.f.fetchBytes(URL_OK, 10))).code).toBe('too_large')
  })

  it('an empty body is an empty result, not an error', async () => {
    const { f } = fetcherOn(async () => resp(new Uint8Array(0)))
    expect((await f.fetchBytes(URL_OK, 10)).length).toBe(0)
  })
})

describe('web fetcher: time and network failures', () => {
  it('aborts after 120 s in all and reports timeout', async () => {
    vi.useFakeTimers()
    let signal: AbortSignal | undefined
    const { f } = fetcherOn(
      (_url, init) =>
        new Promise<Response>((_res, rej) => {
          signal = (init as RequestInit).signal as AbortSignal
          signal.addEventListener('abort', () => rej(new DOMException('aborted', 'AbortError')))
        }),
    )
    const p = codeOf(f.fetchBytes(URL_OK, 100))
    await vi.advanceTimersByTimeAsync(119_000)
    expect(signal!.aborted).toBe(false)
    await vi.advanceTimersByTimeAsync(1_500)
    expect(signal!.aborted).toBe(true)
    expect((await p).code).toBe('timeout')
  })

  it('the 120 s covers the body too: a stalled stream times out', async () => {
    vi.useFakeTimers()
    const { f } = fetcherOn(async (_url, init) => {
      const signal = (init as RequestInit).signal as AbortSignal
      const stream = new ReadableStream<Uint8Array>({
        start(c) {
          c.enqueue(bytes(1))
          signal.addEventListener('abort', () => c.error(new DOMException('aborted', 'AbortError')))
        },
        pull: () => new Promise<void>(() => {}),
      })
      return resp(stream as unknown as Uint8Array)
    })
    const p = codeOf(f.fetchBytes(URL_OK, 100))
    await vi.advanceTimersByTimeAsync(121_000)
    expect((await p).code).toBe('timeout')
  })

  it('a fetch TypeError is network', async () => {
    const { f } = fetcherOn(async () => {
      throw new TypeError('Failed to fetch')
    })
    expect((await codeOf(f.fetchBytes(URL_OK, 100))).code).toBe('network')
  })

  it('a fetch TypeError while navigator.onLine is false is offline', async () => {
    const { f } = fetcherOn(
      async () => {
        throw new TypeError('Failed to fetch')
      },
      { onLine: () => false },
    )
    expect((await codeOf(f.fetchBytes(URL_OK, 100))).code).toBe('offline')
  })

  it('with no deps it reads the global fetch and navigator.onLine', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => {
      throw new TypeError('Failed to fetch')
    }))
    const onLine = vi.spyOn(window.navigator, 'onLine', 'get')
    onLine.mockReturnValue(false)
    expect((await codeOf(webFetcher().fetchBytes(URL_OK, 10))).code).toBe('offline')
    onLine.mockReturnValue(true)
    expect((await codeOf(webFetcher().fetchBytes(URL_OK, 10))).code).toBe('network')
    onLine.mockRestore()
  })
})

describe('errors never carry the URL (decision 28)', () => {
  const secret = 'https://private-host.invalid/users/kurt/secret-folder/hew.json?token=abc'
  const failures: [string, () => Promise<Response>][] = [
    ['network', async () => { throw new TypeError(`Failed to fetch ${secret}`) }],
    ['status', async () => resp(null, { status: 404 })],
    ['rate', async () => resp(null, { status: 429 })],
    ['size', async () => resp(bytes(50)) ],
  ]
  for (const [name, impl] of failures) {
    it(`${name}: message, own properties and cause omit the URL`, async () => {
      const { f } = fetcherOn(impl)
      const e = await codeOf(f.fetchBytes(secret, 10))
      const dump = JSON.stringify({ message: e.message, name: e.name, own: Object.fromEntries(Object.entries(e)), cause: String((e as { cause?: unknown }).cause ?? '') })
      for (const needle of ['private-host', 'secret-folder', 'token=abc', 'kurt']) expect(dump, needle).not.toContain(needle)
      expect(e.stack ?? '').not.toContain('private-host')
    })
  }

  it('a not_https error omits the URL too', async () => {
    const { f } = fetcherOn(async () => resp(bytes(1)))
    const e = await codeOf(f.fetchBytes('http://private-host.invalid/secret-folder', 10))
    expect(`${e.message} ${JSON.stringify(Object.entries(e))}`).not.toContain('private-host')
  })
})

describe('the Off rule, webSourcesOff(doc)', () => {
  const page = (csp: string | null): Document => {
    const doc = document.implementation.createHTMLDocument('t')
    if (csp !== null) {
      const m = doc.createElement('meta')
      m.setAttribute('http-equiv', 'Content-Security-Policy')
      m.setAttribute('content', csp)
      doc.head.appendChild(m)
    }
    return doc
  }

  it('a page with no CSP meta (dev, the E2E servers) is not Off', () => {
    expect(webSourcesOff(page(null))).toBe(false)
  })

  it('a default page, whose connect-src has https:, is not Off', () => {
    expect(webSourcesOff(page("default-src 'self'; connect-src 'self' blob: https:; object-src 'none'"))).toBe(false)
  })

  it('a strict page, whose connect-src has no https:, is Off', () => {
    expect(webSourcesOff(page("default-src 'self'; connect-src 'self' blob:; object-src 'none'"))).toBe(true)
  })

  it('connect-src * allows any host, so it is not Off', () => {
    expect(webSourcesOff(page("default-src 'self'; connect-src *"))).toBe(false)
  })

  it('a host-only list such as https://example.com cannot reach a source, so it is Off', () => {
    expect(webSourcesOff(page("default-src 'self'; connect-src 'self' https://example.com"))).toBe(true)
  })

  it('reads the directive, not a substring: https: inside another directive does not count', () => {
    expect(webSourcesOff(page("img-src https:; connect-src 'self' blob:"))).toBe(true)
  })

  it('a CSP with no connect-src falls back to default-src', () => {
    expect(webSourcesOff(page("default-src 'self'"))).toBe(true)
    expect(webSourcesOff(page('default-src https:'))).toBe(false)
  })

  it('a meta that is not the CSP is ignored', () => {
    const doc = page(null)
    const m = doc.createElement('meta')
    m.setAttribute('http-equiv', 'refresh')
    m.setAttribute('content', "connect-src 'self'")
    doc.head.appendChild(m)
    expect(webSourcesOff(doc)).toBe(false)
  })

  it('the web fetcher is available except when Off, and available() makes no request', async () => {
    const spy = vi.fn(async () => resp(bytes(1)))
    const deps = { fetch: spy as unknown as typeof fetch, onLine: () => true }
    expect(webFetcher({ ...deps, document: page(null) }).available()).toBe(true)
    expect(webFetcher({ ...deps, document: page("connect-src 'self' blob: https:") }).available()).toBe(true)
    const off = webFetcher({ ...deps, document: page("connect-src 'self' blob:") })
    expect(off.available()).toBe(false)
    expect(spy).not.toHaveBeenCalled() // available() alone never requests
  })
})

describe('desktop fetcher', () => {
  // A plain function, not vi.fn: with Vitest 4 a plain-object rejection (the Rust wire shape) from a mock is reported as an unhandled "Unknown Error".
  let calls: unknown[][] = []
  let impl: () => Promise<unknown> = async () => undefined
  const invoke: TauriInvoke = async (...a) => {
    calls.push(a)
    return impl()
  }
  beforeEach(() => {
    calls = []
  })
  const rejectWith = (thrown: unknown) => {
    impl = async () => {
      throw thrown
    }
  }
  const resolveWith = (value: unknown) => {
    impl = async () => value
  }

  it('is never Off', () => {
    document.head.innerHTML = `<meta http-equiv="Content-Security-Policy" content="connect-src 'self' blob:">`
    expect(desktopFetcher(invoke).available()).toBe(true)
  })

  it('invokes remote_fetch with { url, maxBytes } and returns Uint8Array from an ArrayBuffer', async () => {
    resolveWith(new Uint8Array([9, 8, 7]).buffer)
    const out = await desktopFetcher(invoke).fetchBytes(URL_OK, 4096)
    expect(calls).toEqual([['remote_fetch', { url: URL_OK, maxBytes: 4096 }]])
    expect(out).toBeInstanceOf(Uint8Array)
    expect(Array.from(out)).toEqual([9, 8, 7])
  })

  it('accepts a number[] reply', async () => {
    resolveWith([1, 2, 3])
    const out = await desktopFetcher(invoke).fetchBytes(URL_OK, 10)
    expect(out).toBeInstanceOf(Uint8Array)
    expect(Array.from(out)).toEqual([1, 2, 3])
  })

  it('accepts a Uint8Array reply', async () => {
    resolveWith(new Uint8Array([5]))
    expect(Array.from(await desktopFetcher(invoke).fetchBytes(URL_OK, 10))).toEqual([5])
  })

  it('maps each Rust error object to its code', async () => {
    const codes: RemoteFetchCode[] = ['not_https', 'too_large', 'timeout', 'too_many_redirects', 'rate_limited', 'network']
    for (const code of codes) {
      rejectWith({ error: code })
      expect((await codeOf(desktopFetcher(invoke).fetchBytes(URL_OK, 10))).code, code).toBe(code)
    }
  })

  it('maps http_status with its status code', async () => {
    rejectWith({ error: 'http_status', code: 404 })
    const e = await codeOf(desktopFetcher(invoke).fetchBytes(URL_OK, 10))
    expect(e.code).toBe('http_status')
    expect(e.status).toBe(404)
  })

  it('anything else the bridge throws is network, and never offline', async () => {
    const onLine = vi.spyOn(window.navigator, 'onLine', 'get').mockReturnValue(false)
    for (const thrown of ['ipc down', new Error('boom'), { error: 'something_new' }, null]) {
      rejectWith(thrown)
      expect((await codeOf(desktopFetcher(invoke).fetchBytes(URL_OK, 10))).code).toBe('network')
    }
    onLine.mockRestore()
  })

  it('error objects omit the URL', async () => {
    rejectWith({ error: 'network', url: URL_OK, message: `failed ${URL_OK}` })
    const e = await codeOf(desktopFetcher(invoke).fetchBytes(URL_OK, 10))
    expect(`${e.message}${JSON.stringify(Object.entries(e))}`).not.toContain(HOST)
  })
})

describe('remoteFetcher() picks the platform', () => {
  afterEach(() => {
    delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__
    vi.resetModules()
  })

  it('in a browser it is the web fetcher (available with no CSP meta)', async () => {
    vi.resetModules()
    const m = await import('./remoteFetcher')
    expect(m.remoteFetcher().available()).toBe(true)
    document.head.innerHTML = `<meta http-equiv="Content-Security-Policy" content="connect-src 'self' blob:">`
    expect(m.remoteFetcher().available()).toBe(false)
  })

  it('in the desktop shell it is the desktop fetcher: invoke remote_fetch, never Off', async () => {
    Object.defineProperty(window, '__TAURI_INTERNALS__', { value: {}, configurable: true })
    vi.resetModules()
    mockInvoke.mockReset()
    mockInvoke.mockResolvedValue(new Uint8Array([1]).buffer)
    document.head.innerHTML = `<meta http-equiv="Content-Security-Policy" content="connect-src 'self' blob:">`
    const m = await import('./remoteFetcher')
    const f = m.remoteFetcher()
    expect(f.available()).toBe(true)
    await f.fetchBytes(URL_OK, 10)
    expect(mockInvoke).toHaveBeenCalledWith('remote_fetch', { url: URL_OK, maxBytes: 10 })
  })
})

describe('app-side logging (decision 28)', () => {
  const source = (): string => {
    const here = dirname(fileURLToPath(import.meta.url))
    return readFileSync(join(here, 'remoteFetcher.ts'), 'utf8')
  }
  // Positive control: the scan must be reading the real implementation.
  it('the scanned file is the implementation (control)', () => {
    const s = source()
    expect(s).toContain('remote_fetch')
    expect(s).toContain('credentials')
    expect(s.length).toBeGreaterThan(1500)
  })

  it('remoteFetcher.ts makes no console, logger or diagnostic-log call', () => {
    const code = source().replace(/\/\*[\s\S]*?\*\//g, '').replace(/^\s*\/\/.*$/gm, '')
    expect(code).not.toMatch(/\bconsole\s*\./)
    expect(code).not.toMatch(/\blogUi\b|\blogStore\b|\bLogStore\b|from\s+['"][^'"]*\/log\//)
    expect(code).not.toMatch(/\b(?:log|logger|warn|debug)\s*\(/)
  })
})

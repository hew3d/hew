// The one way the app reads a Library source over the network
// (docs/design/shared-library-sources.md 6.4 "Fetch rules", 6.6, 7.1,
// decisions 25 and 28). The web page fetches with the browser's `fetch`
// under its Content Security Policy; the desktop shell asks the Rust
// `remote_fetch` command, which applies the same rules natively.
//
// Rules held here (web column): https only, on the first URL; no
// credentials, no referrer, no custom headers; 120 s for the request and the
// body together; the body is counted against `maxBytes` and cut off there.
// An error carries a code and, for `http_status`, the status; never the URL,
// because a source may sit on a private address and bug reports carry logs.
// This file makes no console or log call for the same reason.

export type RemoteFetchCode =
  | 'not_https'
  | 'too_large'
  | 'timeout'
  | 'too_many_redirects'
  | 'rate_limited'
  | 'http_status'
  | 'network'
  | 'offline'

/** Carries a code, never the URL. `status` is set for `http_status`. */
export class RemoteFetchError extends Error {
  readonly code: RemoteFetchCode
  readonly status?: number
  constructor(code: RemoteFetchCode, status?: number) {
    super(code)
    this.name = 'RemoteFetchError'
    this.code = code
    this.status = status
  }
}

export interface RemoteFetcher {
  /** False only in the Off state (web page whose CSP connect-src has no https:). */
  available(): boolean
  fetchBytes(url: string, maxBytes: number): Promise<Uint8Array>
}

export interface WebFetcherDeps {
  /** Defaults to the global fetch. */
  fetch?: typeof fetch
  /** Defaults to the global document (read for the Off rule). */
  document?: Document
  /** Defaults to `navigator.onLine !== false`. */
  onLine?: () => boolean
}

export type TauriInvoke = (command: string, args?: Record<string, unknown>) => Promise<unknown>

const TOTAL_TIMEOUT_MS = 120_000

/**
 * True when the page's Content Security Policy keeps it from reaching a
 * source: a policy <meta> exists and its connect-src (or default-src when
 * there is none) allows neither `https:` nor `*`. A page with no policy
 * (dev, the E2E servers) is not Off.
 */
export function webSourcesOff(doc: Document): boolean {
  let csp: string | null = null
  for (const m of Array.from(doc.querySelectorAll('meta'))) {
    if ((m.getAttribute('http-equiv') ?? '').trim().toLowerCase() === 'content-security-policy') {
      csp = m.getAttribute('content') ?? ''
      break
    }
  }
  if (csp === null) return false
  const directives = new Map<string, string[]>()
  for (const part of csp.split(';')) {
    const tokens = part.trim().split(/\s+/).filter(Boolean)
    if (tokens.length === 0) continue
    const name = tokens[0].toLowerCase()
    if (!directives.has(name)) directives.set(name, tokens.slice(1))
  }
  const sources = directives.get('connect-src') ?? directives.get('default-src')
  if (sources === undefined) return false // no connect-src and no default-src: unrestricted
  return !sources.some((s) => s === '*' || s.toLowerCase() === 'https:')
}

function isHttps(url: string): boolean {
  try {
    return new URL(url).protocol === 'https:'
  } catch {
    return false
  }
}

function rateLimited(res: Response): boolean {
  if (res.status === 429) return true
  if (res.status !== 403) return false
  if (res.headers.get('retry-after') !== null) return true
  const remaining = res.headers.get('x-ratelimit-remaining')
  return remaining !== null && remaining.trim() === '0'
}

export function webFetcher(deps: WebFetcherDeps = {}): RemoteFetcher {
  const doFetch = (...a: Parameters<typeof fetch>) => (deps.fetch ?? fetch)(...a)
  const onLine = () => (deps.onLine ? deps.onLine() : typeof navigator === 'undefined' || navigator.onLine !== false)
  return {
    available: () => !webSourcesOff(deps.document ?? document),
    async fetchBytes(url, maxBytes) {
      if (!isHttps(url)) throw new RemoteFetchError('not_https')
      const ctl = new AbortController()
      let timedOut = false
      const timer = setTimeout(() => {
        timedOut = true
        ctl.abort()
      }, TOTAL_TIMEOUT_MS)
      try {
        let res: Response
        try {
          res = await doFetch(url, { method: 'GET', credentials: 'omit', referrerPolicy: 'no-referrer', redirect: 'follow', signal: ctl.signal })
        } catch {
          throw new RemoteFetchError(timedOut ? 'timeout' : onLine() ? 'network' : 'offline')
        }
        if (rateLimited(res)) {
          void res.body?.cancel().catch(() => undefined)
          throw new RemoteFetchError('rate_limited')
        }
        if (!res.ok) {
          void res.body?.cancel().catch(() => undefined)
          throw new RemoteFetchError('http_status', res.status)
        }
        const declared = Number(res.headers.get('content-length'))
        if (res.headers.get('content-length') !== null && Number.isFinite(declared) && declared > maxBytes) {
          void res.body?.cancel().catch(() => undefined)
          throw new RemoteFetchError('too_large')
        }
        if (!res.body) return new Uint8Array(0)
        const reader = res.body.getReader()
        const chunks: Uint8Array[] = []
        let total = 0
        try {
          for (;;) {
            const { done, value } = await reader.read()
            if (done) break
            total += value.length
            if (total > maxBytes) {
              void reader.cancel().catch(() => undefined)
              throw new RemoteFetchError('too_large')
            }
            chunks.push(value)
          }
        } catch (e) {
          if (e instanceof RemoteFetchError) throw e
          throw new RemoteFetchError(timedOut ? 'timeout' : onLine() ? 'network' : 'offline')
        }
        const out = new Uint8Array(total)
        let at = 0
        for (const c of chunks) {
          out.set(c, at)
          at += c.length
        }
        return out
      } finally {
        clearTimeout(timer)
      }
    },
  }
}

const DESKTOP_CODES: readonly RemoteFetchCode[] = ['not_https', 'too_large', 'timeout', 'too_many_redirects', 'rate_limited', 'http_status', 'network']

function fromRust(thrown: unknown): RemoteFetchError {
  if (typeof thrown === 'object' && thrown !== null) {
    const o = thrown as { error?: unknown; code?: unknown }
    if (typeof o.error === 'string' && (DESKTOP_CODES as readonly string[]).includes(o.error)) {
      if (o.error === 'http_status') return new RemoteFetchError('http_status', typeof o.code === 'number' ? o.code : undefined)
      return new RemoteFetchError(o.error as RemoteFetchCode)
    }
  }
  return new RemoteFetchError('network')
}

/** The desktop fetcher: the Rust `remote_fetch` command applies the rules. Never Off. */
export function desktopFetcher(invoke: TauriInvoke): RemoteFetcher {
  return {
    available: () => true,
    async fetchBytes(url, maxBytes) {
      let reply: unknown
      try {
        reply = await invoke('remote_fetch', { url, maxBytes })
      } catch (e) {
        throw fromRust(e)
      }
      if (reply instanceof Uint8Array) return reply
      if (reply instanceof ArrayBuffer) return new Uint8Array(reply)
      if (Array.isArray(reply)) return Uint8Array.from(reply as number[])
      throw new RemoteFetchError('network')
    },
  }
}

/** The fetcher for this platform: the desktop shell's, or the web page's. */
export function remoteFetcher(): RemoteFetcher {
  if (typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window) {
    return desktopFetcher(async (command, args) => {
      const { invoke } = await import('@tauri-apps/api/core')
      return invoke(command, args)
    })
  }
  return webFetcher()
}

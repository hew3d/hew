/**
 * Relay conformance suite — black-box HTTP tests for the "Open on Phone"
 * relay contract, run against a LIVE server over real HTTP (docs/design/
 * self-hosting-relay.md §6). Two implementations must satisfy it: this
 * directory's Cloudflare Worker (`../src/handlers.ts` is the spec) and the
 * self-hostable Rust binary (`crates/hew-relay`). CI (`relay-contract` in
 * .github/workflows/ci.yml) and `scripts/verify-full.sh` run it against
 * both; `handlers.test.ts` stays the fast in-process unit suite.
 *
 * Same style as the unit suite: `node --test`, zero installed dependencies
 * (Node's own `fetch`, `node:http`, `node:assert`). Parameterized entirely
 * through the environment:
 *
 *   HEW_RELAY_URL              REQUIRED. The relay's ORIGIN, no path — e.g.
 *                              http://127.0.0.1:8787. The suite exercises
 *                              both `/drop` and `/relay/drop` under it.
 *   HEW_RELAY_UPLOAD_KEY       If set, the server is expected to require it
 *                              (identity reports auth: "bearer", a keyless
 *                              PUT is 401). If unset, uploads must be open.
 *   HEW_RELAY_ALLOWED_ORIGIN   An origin the server's CORS allowlist admits
 *                              (default https://app.hew3d.com — always on the
 *                              Worker's base list; pass it to hew-relay via
 *                              --allow-origin).
 *   HEW_RELAY_TTL_MS           The server's drop TTL, if it was started with a
 *                              short one (hew-relay --ttl-secs 2 → 2000).
 *                              Enables the expiry case; the Worker's TTL is
 *                              fixed at 10 minutes, so its run leaves this
 *                              unset and the expiry case is skipped.
 *   HEW_RELAY_MAX_TOTAL_BYTES  The server's total memory cap, if it was
 *                              started with a tiny one (hew-relay
 *                              --max-total-bytes N). Enables the "relay full"
 *                              503 case; the Worker has no such cap.
 *   HEW_RELAY_MAX_BYTES        Per-drop cap the server enforces (default the
 *                              contract's 32 MiB). The oversized cases upload
 *                              this + 1 byte, so a small value keeps a run
 *                              fast; the identity route must report it.
 *   HEW_RELAY_PREFIX_ONLY      Set to 1 when HEW_RELAY_URL is a WEB ORIGIN
 *                              that proxies only /relay/ to the relay (the
 *                              nginx layout in docs/SELF_HOSTING.md) rather
 *                              than the relay itself: every case then runs
 *                              under /relay only, and the bare-root and
 *                              double-prefix cases are skipped (the proxy
 *                              strips one prefix, the relay would strip
 *                              another). Validates a real deployment.
 *   HEW_RELAY_BUFFERS_BODY     Set to 1 when something in front of the relay
 *                              buffers a request body to completion before the
 *                              relay sees it. `wrangler dev`'s local proxy does
 *                              (a PUT whose declared Content-Length is never
 *                              met gets no answer, and the client's eventual
 *                              disconnect crashes the dev server), so the
 *                              header-only fail-fast cases are skipped on that
 *                              leg. Production Cloudflare and hew-relay stream
 *                              and must pass them.
 *
 * Every drop this suite creates is consumed or deleted before the test ends,
 * so a run leaves the server empty (matters for the total-bytes case).
 */

import { test, describe } from 'node:test'
import assert from 'node:assert/strict'
import { request as httpRequest } from 'node:http'
import { request as httpsRequest } from 'node:https'

const ORIGIN = (process.env.HEW_RELAY_URL ?? '').replace(/\/+$/, '')
if (ORIGIN === '') {
  throw new Error('HEW_RELAY_URL is required (the relay origin, e.g. http://127.0.0.1:8787)')
}
const UPLOAD_KEY = process.env.HEW_RELAY_UPLOAD_KEY ?? ''
const ALLOWED_ORIGIN = process.env.HEW_RELAY_ALLOWED_ORIGIN ?? 'https://app.hew3d.com'
const TTL_MS = process.env.HEW_RELAY_TTL_MS !== undefined ? Number(process.env.HEW_RELAY_TTL_MS) : null
const MAX_TOTAL_BYTES =
  process.env.HEW_RELAY_MAX_TOTAL_BYTES !== undefined ? Number(process.env.HEW_RELAY_MAX_TOTAL_BYTES) : null
const MAX_BYTES = process.env.HEW_RELAY_MAX_BYTES !== undefined ? Number(process.env.HEW_RELAY_MAX_BYTES) : 32 * 1024 * 1024
const BUFFERS_BODY = process.env.HEW_RELAY_BUFFERS_BODY === '1'
const SKIP_HEADER_ONLY = BUFFERS_BODY ? 'HEW_RELAY_BUFFERS_BODY=1 — a proxy buffers bodies, header-only fail-fast unobservable' : false
const PREFIX_ONLY = process.env.HEW_RELAY_PREFIX_ONLY === '1'
const SKIP_BARE_ROOT = PREFIX_ONLY ? 'HEW_RELAY_PREFIX_ONLY=1 — only /relay/ reaches the relay' : false
/** Path prefix for the cases that are not explicitly parameterized over
 *  both spellings — bare in a direct run, `/relay` behind a proxy. */
const ROOT = PREFIX_ONLY ? '/relay' : ''

/** The contract's fixed values — mirrored from `handlers.ts` on purpose:
 *  this suite must not import the implementation it is checking. */
const CONTRACT_VERSION = 1
const DEFAULT_TTL_MS = 10 * 60 * 1000
const TOKEN_RE = /^[A-Za-z0-9_-]{22}$/
const DENIED_ORIGIN = 'https://evil.example'

/** Both spellings of the drop path — every routing case runs under each
 *  (only the prefixed one behind a proxy). */
const PREFIXES: readonly string[] = PREFIX_ONLY ? ['/relay'] : ['', '/relay']

function authHeaders(): Record<string, string> {
  return UPLOAD_KEY === '' ? {} : { authorization: `Bearer ${UPLOAD_KEY}` }
}

/** PUT `bytes` (authorized when a key is configured); returns the token. */
async function put(bytes: Uint8Array, prefix: string = ROOT, extraHeaders: Record<string, string> = {}): Promise<string> {
  const res = await fetch(`${ORIGIN}${prefix}/drop`, {
    method: 'PUT',
    body: bytes,
    headers: { ...authHeaders(), ...extraHeaders },
  })
  if (res.status !== 200) {
    assert.fail(`PUT ${prefix}/drop: ${res.status} ${await res.text()}`)
  }
  const { token } = (await res.json()) as { token: string }
  return token
}

async function del(token: string, prefix: string = ROOT): Promise<Response> {
  return fetch(`${ORIGIN}${prefix}/drop/${token}`, { method: 'DELETE' })
}

/** Deterministic non-trivial fill so a byte-for-byte round trip is a real
 *  check, not a comparison of zeros. */
function fillPattern(size: number): Uint8Array {
  const bytes = new Uint8Array(size)
  for (let i = 0; i < size; i++) bytes[i] = (i * 2654435761) & 0xff
  return bytes
}

/** A raw `node:http` PUT that reads the response CONCURRENTLY with writing
 *  the body — resolves with the status the moment the server answers, even
 *  if the body is still (or never) being sent. Two contract cases need
 *  this and `fetch` can do neither:
 *
 *   - `declared` larger than the bytes actually written (a LYING
 *     Content-Length; `fetch` refuses to send a mismatch): the server must
 *     answer from the header alone, before reading the body.
 *   - an honest oversized upload: a server that rejects on the header and
 *     closes without draining the body (hew-relay does; nginx does) makes
 *     the client's write fail with EPIPE/ECONNRESET — undici's `fetch`
 *     surfaces that as a failure without ever reading the 413 that was
 *     sent first. Reading concurrently gets the status.
 *
 *  `actualBytes` are written in chunks; when `end` is true the body is
 *  completed, otherwise left hanging (only sensible with a lying
 *  `declared`). A transport error BEFORE any response rejects. */
function putRaw(opts: { declared: number; actualBytes: number; end: boolean }): Promise<number> {
  const url = new URL(`${ROOT}/drop`, ORIGIN)
  const req = (url.protocol === 'https:' ? httpsRequest : httpRequest)(url, {
    method: 'PUT',
    headers: { 'content-length': String(opts.declared), ...authHeaders() },
  })
  return new Promise((resolve, reject) => {
    let answered = false
    req.on('response', (res) => {
      answered = true
      res.resume()
      resolve(res.statusCode ?? 0)
      req.destroy()
    })
    req.on('error', (err) => {
      // A server that has already answered may reset the connection while
      // this side is still sending — that error is harmless once the promise
      // is settled; one before any response is a real failure.
      if (!answered) reject(err)
    })
    // A server that insists on reading a never-completed body before
    // answering would hang this forever — fail it instead.
    req.setTimeout(10_000, () => {
      req.destroy(new Error('no response within 10s'))
    })
    const CHUNK = 64 * 1024
    let remaining = opts.actualBytes
    const pump = (): void => {
      while (remaining > 0) {
        const n = Math.min(CHUNK, remaining)
        remaining -= n
        if (!req.write(new Uint8Array(n))) {
          req.once('drain', pump)
          return
        }
      }
      if (opts.end) req.end()
    }
    pump()
  })
}

/** Convenience for the lying-length case: declare `declared`, write a
 *  handful of bytes, never finish. */
function putWithDeclaredLength(declared: number, actualBytes: number): Promise<number> {
  return putRaw({ declared, actualBytes, end: false })
}

// ---------------------------------------------------------------------------
// Identity route
// ---------------------------------------------------------------------------

describe('identity route', () => {
  // Behind a proxy the slash-less `/relay` is a 308 to `/relay/` (the
  // shipped nginx stanza) — `fetch` follows it, so the case still holds.
  for (const path of PREFIX_ONLY ? ['/relay', '/relay/'] : ['/', '/relay', '/relay/']) {
    test(`GET ${path} answers the identity document`, async () => {
      const res = await fetch(`${ORIGIN}${path}`)
      assert.equal(res.status, 200)
      assert.match(res.headers.get('content-type') ?? '', /^application\/json/)
      assert.equal(res.headers.get('cache-control'), 'no-store')
      const body = (await res.json()) as Record<string, unknown>
      assert.equal(body.service, 'hew-relay')
      assert.equal(body.contract, CONTRACT_VERSION)
      assert.equal(body.maxBytes, MAX_BYTES)
      assert.equal(body.ttlMs, TTL_MS ?? DEFAULT_TTL_MS)
      assert.equal(body.auth, UPLOAD_KEY === '' ? 'none' : 'bearer')
    })
  }

  test('the identity route is GET-only', async () => {
    assert.equal((await fetch(`${ORIGIN}/relay/`, { method: 'HEAD' })).status, 404)
    assert.equal((await fetch(`${ORIGIN}/relay/`, { method: 'PUT', body: new Uint8Array([1]) })).status, 404)
    assert.equal((await fetch(`${ORIGIN}/relay/`, { method: 'DELETE' })).status, 404)
    if (!PREFIX_ONLY) assert.equal((await fetch(`${ORIGIN}/`, { method: 'HEAD' })).status, 404)
  })

  test('the identity body never contains the upload key', { skip: UPLOAD_KEY === '' }, async () => {
    const text = await (await fetch(`${ORIGIN}/relay/`)).text()
    assert.equal(text.includes(UPLOAD_KEY), false)
  })
})

// ---------------------------------------------------------------------------
// Routing — 404s, prefix handling
// ---------------------------------------------------------------------------

describe('routing', () => {
  test('unrelated paths are 404, with and without the prefix', async () => {
    const paths = PREFIX_ONLY
      ? ['/relay/nope', '/relay/drop', '/relay/drop/']
      : ['/nope', '/relay/nope', '/drop/', '/relay/drop', '/relayx', '/relayx/drop']
    for (const path of paths) {
      const res = await fetch(`${ORIGIN}${path}`)
      assert.equal(res.status, 404, path)
    }
  })

  test('the prefix is stripped exactly once', { skip: SKIP_BARE_ROOT }, async () => {
    const res = await fetch(`${ORIGIN}/relay/relay/drop`, { method: 'PUT', body: new Uint8Array([1]), headers: authHeaders() })
    assert.equal(res.status, 404)
  })

  test('PUT to a token path and GET/HEAD /drop (no token) are 404', async () => {
    const token = await put(new Uint8Array([1]))
    try {
      assert.equal((await fetch(`${ORIGIN}${ROOT}/drop/${token}`, { method: 'PUT', body: new Uint8Array([1]), headers: authHeaders() })).status, 404)
      assert.equal((await fetch(`${ORIGIN}${ROOT}/drop`)).status, 404)
      assert.equal((await fetch(`${ORIGIN}${ROOT}/drop`, { method: 'HEAD' })).status, 404)
      assert.equal((await fetch(`${ORIGIN}${ROOT}/drop/${token}/extra`)).status, 404)
    } finally {
      await del(token)
    }
  })
})

// ---------------------------------------------------------------------------
// PUT / GET / HEAD / DELETE — under both prefixes
// ---------------------------------------------------------------------------

for (const prefix of PREFIXES) {
  describe(`drop lifecycle under "${prefix || '/'}"`, () => {
    test('PUT returns a 22-char base64url token; tokens are distinct', async () => {
      const a = await put(new Uint8Array([1]), prefix)
      const b = await put(new Uint8Array([2]), prefix)
      try {
        assert.match(a, TOKEN_RE)
        assert.match(b, TOKEN_RE)
        assert.notEqual(a, b)
      } finally {
        await del(a, prefix)
        await del(b, prefix)
      }
    })

    test('GET round-trips the exact bytes, once; the second GET is 404', async () => {
      const input = fillPattern(Math.min(70_000, MAX_BYTES))
      const token = await put(input, prefix)
      const first = await fetch(`${ORIGIN}${prefix}/drop/${token}`)
      assert.equal(first.status, 200)
      assert.equal(first.headers.get('content-type'), 'application/octet-stream')
      assert.equal(first.headers.get('cache-control'), 'no-store')
      const bytes = new Uint8Array(await first.arrayBuffer())
      assert.deepEqual(bytes, input)
      const second = await fetch(`${ORIGIN}${prefix}/drop/${token}`)
      assert.equal(second.status, 404)
    })

    test('a drop written under one spelling is readable under the other', { skip: SKIP_BARE_ROOT }, async () => {
      const other = prefix === '' ? '/relay' : ''
      const token = await put(new Uint8Array([7, 7, 7]), prefix)
      assert.equal((await fetch(`${ORIGIN}${other}/drop/${token}`, { method: 'HEAD' })).status, 200)
      const res = await fetch(`${ORIGIN}${other}/drop/${token}`)
      assert.equal(res.status, 200)
      assert.deepEqual(Array.from(new Uint8Array(await res.arrayBuffer())), [7, 7, 7])
    })

    test('HEAD peeks without consuming: 200 while present, then a GET still succeeds, then 404', async () => {
      const token = await put(new Uint8Array([5, 6]), prefix)
      assert.equal((await fetch(`${ORIGIN}${prefix}/drop/${token}`, { method: 'HEAD' })).status, 200)
      assert.equal((await fetch(`${ORIGIN}${prefix}/drop/${token}`, { method: 'HEAD' })).status, 200)
      const res = await fetch(`${ORIGIN}${prefix}/drop/${token}`)
      assert.equal(res.status, 200)
      assert.deepEqual(Array.from(new Uint8Array(await res.arrayBuffer())), [5, 6])
      const after = await fetch(`${ORIGIN}${prefix}/drop/${token}`, { method: 'HEAD' })
      assert.equal(after.status, 404)
      assert.equal((await after.arrayBuffer()).byteLength, 0)
    })

    test('DELETE is 204 always — existing, twice, unknown, malformed — and a deleted drop is gone', async () => {
      const token = await put(new Uint8Array([1]), prefix)
      assert.equal((await del(token, prefix)).status, 204)
      assert.equal((await del(token, prefix)).status, 204)
      assert.equal((await fetch(`${ORIGIN}${prefix}/drop/${token}`)).status, 404)
      assert.equal((await del('A'.repeat(22), prefix)).status, 204)
      assert.equal((await del('garbage', prefix)).status, 204)
    })

    test('unknown token: GET and HEAD are 404 with empty HEAD body', async () => {
      const unknown = 'B'.repeat(22)
      assert.equal((await fetch(`${ORIGIN}${prefix}/drop/${unknown}`)).status, 404)
      const head = await fetch(`${ORIGIN}${prefix}/drop/${unknown}`, { method: 'HEAD' })
      assert.equal(head.status, 404)
      assert.equal((await head.arrayBuffer()).byteLength, 0)
    })

    test('malformed token: GET and HEAD are 400', async () => {
      for (const bad of ['short', 'a'.repeat(23), 'has.dots.xxxxxxxxxxxxx', 'has spaces xxxxxxxxxx']) {
        assert.equal((await fetch(`${ORIGIN}${prefix}/drop/${encodeURIComponent(bad)}`)).status, 400, bad)
        assert.equal((await fetch(`${ORIGIN}${prefix}/drop/${encodeURIComponent(bad)}`, { method: 'HEAD' })).status, 400, bad)
      }
    })
  })
}

// ---------------------------------------------------------------------------
// One-shot atomicity
// ---------------------------------------------------------------------------

describe('one-shot', () => {
  test('two concurrent GETs for one token: exactly one 200, one 404', async () => {
    const token = await put(fillPattern(Math.min(50_000, MAX_BYTES)))
    const [a, b] = await Promise.all([fetch(`${ORIGIN}${ROOT}/drop/${token}`), fetch(`${ORIGIN}${ROOT}/drop/${token}`)])
    assert.deepEqual([a.status, b.status].sort(), [200, 404])
    await Promise.all([a.arrayBuffer(), b.arrayBuffer()])
  })

  test('many concurrent GETs for one token: still exactly one 200', async () => {
    const token = await put(fillPattern(Math.min(10_000, MAX_BYTES)))
    const results = await Promise.all(Array.from({ length: 8 }, () => fetch(`${ORIGIN}${ROOT}/drop/${token}`)))
    const statuses = results.map((r) => r.status)
    assert.equal(statuses.filter((s) => s === 200).length, 1)
    assert.equal(statuses.filter((s) => s === 404).length, 7)
    await Promise.all(results.map((r) => r.arrayBuffer()))
  })
})

// ---------------------------------------------------------------------------
// Size caps
// ---------------------------------------------------------------------------

describe('size caps', () => {
  test('an empty PUT is 400', async () => {
    const res = await fetch(`${ORIGIN}${ROOT}/drop`, { method: 'PUT', body: new Uint8Array(0), headers: authHeaders() })
    assert.equal(res.status, 400)
    assert.deepEqual(await res.json(), { error: 'empty body' })
  })

  test('a body of exactly maxBytes round-trips byte-for-byte (the multi-batch storage path at the default cap)', async () => {
    // At the contract's 32 MiB cap this crosses every internal batching
    // boundary either server has (the Worker's RPC batches, hew-relay's
    // single buffer) — a regression in reassembly order or windowing shows
    // up here, not just in the in-process unit suites.
    const input = fillPattern(MAX_BYTES)
    const token = await put(input)
    const res = await fetch(`${ORIGIN}${ROOT}/drop/${token}`)
    assert.equal(res.status, 200)
    const bytes = new Uint8Array(await res.arrayBuffer())
    assert.equal(bytes.byteLength, MAX_BYTES)
    // Compare in slices so a mismatch reports WHERE, without a 32 MiB diff.
    for (let offset = 0; offset < MAX_BYTES; offset += 4 * 1024 * 1024) {
      const end = Math.min(offset + 4 * 1024 * 1024, MAX_BYTES)
      assert.ok(
        Buffer.compare(bytes.subarray(offset, end), input.subarray(offset, end)) === 0,
        `bytes differ in [${offset}, ${end})`,
      )
    }
    assert.equal((await fetch(`${ORIGIN}${ROOT}/drop/${token}`)).status, 404)
  })

  test('a body one byte over maxBytes is 413', async () => {
    // Raw client (see `putRaw`): a server that rejects on the header and
    // closes without draining makes `fetch` fail on the write before it
    // reads the response — the status still has to be observed.
    const status = await putRaw({ declared: MAX_BYTES + 1, actualBytes: MAX_BYTES + 1, end: true })
    assert.equal(status, 413)
    const res = await fetch(`${ORIGIN}/relay/`)
    assert.equal(res.status, 200) // the server is still healthy after the refusal
  })

  test('an oversized CHUNKED upload (no Content-Length) still gets its 413, not a reset', { skip: SKIP_HEADER_ONLY }, async () => {
    // Nothing to reject from the header here — the server can only notice
    // the overflow mid-stream, and must still deliver the answer.
    const url = new URL(`${ROOT}/drop`, ORIGIN)
    const status = await new Promise<number>((resolve, reject) => {
      const req = (url.protocol === 'https:' ? httpsRequest : httpRequest)(url, {
        method: 'PUT',
        headers: { 'transfer-encoding': 'chunked', ...authHeaders() },
      })
      let answered = false
      req.on('response', (res) => {
        answered = true
        res.resume()
        resolve(res.statusCode ?? 0)
        req.destroy()
      })
      req.on('error', (err) => {
        if (!answered) reject(err)
      })
      req.setTimeout(10_000, () => req.destroy(new Error('no response within 10s')))
      const CHUNK = 64 * 1024
      let remaining = MAX_BYTES + CHUNK
      const pump = (): void => {
        while (remaining > 0) {
          const n = Math.min(CHUNK, remaining)
          remaining -= n
          if (!req.write(new Uint8Array(n))) {
            req.once('drain', pump)
            return
          }
        }
        req.end()
      }
      pump()
    })
    assert.equal(status, 413)
  })

  test('a declared Content-Length over maxBytes is 413 before the body is read', { skip: SKIP_HEADER_ONLY }, async () => {
    const status = await putWithDeclaredLength(MAX_BYTES + 1000, 16)
    assert.equal(status, 413)
  })
})

// ---------------------------------------------------------------------------
// CORS
// ---------------------------------------------------------------------------

describe('CORS', () => {
  test('preflight from an allowed origin: 204 with the full header set, on any path', async () => {
    const paths = PREFIX_ONLY
      ? ['/relay/drop', '/relay/drop/xyz', '/relay/', '/relay/anything']
      : ['/drop', '/relay/drop', '/drop/xyz', '/relay/', '/anything']
    for (const path of paths) {
      const res = await fetch(`${ORIGIN}${path}`, { method: 'OPTIONS', headers: { origin: ALLOWED_ORIGIN } })
      assert.equal(res.status, 204, path)
      assert.equal(res.headers.get('access-control-allow-origin'), ALLOWED_ORIGIN)
      assert.equal(res.headers.get('access-control-allow-methods'), 'PUT, GET, HEAD, DELETE, OPTIONS')
      assert.equal(res.headers.get('access-control-allow-headers'), 'content-type, authorization')
      assert.equal(res.headers.get('access-control-max-age'), '86400')
      assert.match(res.headers.get('vary') ?? '', /origin/i)
    }
  })

  test('preflight from a denied or absent origin: bare 403 with no CORS headers', async () => {
    const denied = await fetch(`${ORIGIN}${ROOT}/drop`, { method: 'OPTIONS', headers: { origin: DENIED_ORIGIN } })
    assert.equal(denied.status, 403)
    assert.equal(denied.headers.get('access-control-allow-origin'), null)
    const absent = await fetch(`${ORIGIN}${ROOT}/drop`, { method: 'OPTIONS' })
    assert.equal(absent.status, 403)
    assert.equal(absent.headers.get('access-control-allow-origin'), null)
  })

  test('responses echo an allowed origin (never a wildcard), and nothing for a denied/absent one', async () => {
    const token = await put(new Uint8Array([1]), ROOT, { origin: ALLOWED_ORIGIN })
    try {
      const allowed = await fetch(`${ORIGIN}${ROOT}/drop/${token}`, { method: 'HEAD', headers: { origin: ALLOWED_ORIGIN } })
      assert.equal(allowed.headers.get('access-control-allow-origin'), ALLOWED_ORIGIN)
      assert.match(allowed.headers.get('vary') ?? '', /origin/i)
      const denied = await fetch(`${ORIGIN}${ROOT}/drop/${token}`, { method: 'HEAD', headers: { origin: DENIED_ORIGIN } })
      assert.equal(denied.status, 200) // the request still happens; the browser just can't read it
      assert.equal(denied.headers.get('access-control-allow-origin'), null)
      const absent = await fetch(`${ORIGIN}${ROOT}/drop/${token}`, { method: 'HEAD' })
      assert.equal(absent.headers.get('access-control-allow-origin'), null)
      const identity = await fetch(`${ORIGIN}/relay/`, { headers: { origin: ALLOWED_ORIGIN } })
      assert.equal(identity.headers.get('access-control-allow-origin'), ALLOWED_ORIGIN)
      const notFound = await fetch(`${ORIGIN}${ROOT}/nope`, { headers: { origin: ALLOWED_ORIGIN } })
      assert.equal(notFound.status, 404)
      assert.equal(notFound.headers.get('access-control-allow-origin'), ALLOWED_ORIGIN)
    } finally {
      await del(token)
    }
  })
})

// ---------------------------------------------------------------------------
// Upload key
// ---------------------------------------------------------------------------

describe('upload key', () => {
  test('with a key configured: keyless / wrong-key PUT is 401, GET/HEAD/DELETE stay keyless', { skip: UPLOAD_KEY === '' }, async () => {
    for (const prefix of PREFIXES) {
      const noKey = await fetch(`${ORIGIN}${prefix}/drop`, { method: 'PUT', body: new Uint8Array([1]) })
      assert.equal(noKey.status, 401)
      assert.deepEqual(await noKey.json(), { error: 'unauthorized' })
      const wrong = await fetch(`${ORIGIN}${prefix}/drop`, {
        method: 'PUT',
        body: new Uint8Array([1]),
        headers: { authorization: `Bearer ${UPLOAD_KEY}x` },
      })
      assert.equal(wrong.status, 401)
      const badScheme = await fetch(`${ORIGIN}${prefix}/drop`, {
        method: 'PUT',
        body: new Uint8Array([1]),
        headers: { authorization: `Basic ${UPLOAD_KEY}` },
      })
      assert.equal(badScheme.status, 401)
    }
    const token = await put(new Uint8Array([3]))
    assert.equal((await fetch(`${ORIGIN}${ROOT}/drop/${token}`, { method: 'HEAD' })).status, 200)
    const res = await fetch(`${ORIGIN}${ROOT}/drop/${token}`)
    assert.equal(res.status, 200)
    await res.arrayBuffer()
    assert.equal((await del(token)).status, 204)
  })

  test('the 401 wins over the size checks (unauthorized oversized declared length is 401, not 413)', { skip: UPLOAD_KEY === '' ? 'no upload key configured' : SKIP_HEADER_ONLY }, async () => {
    const url = new URL(`${ROOT}/drop`, ORIGIN)
    const status = await new Promise<number>((resolve, reject) => {
      const req = (url.protocol === 'https:' ? httpsRequest : httpRequest)(url, {
        method: 'PUT',
        headers: { 'content-length': String(MAX_BYTES + 1000) },
      })
      req.on('response', (res) => {
        res.resume()
        resolve(res.statusCode ?? 0)
        req.destroy()
      })
      req.on('error', reject)
      req.setTimeout(10_000, () => req.destroy(new Error('no response within 10s')))
      req.write(new Uint8Array(16))
    })
    assert.equal(status, 401)
  })

  test('with NO key configured: a PUT carrying any Authorization header is still accepted', { skip: UPLOAD_KEY !== '' }, async () => {
    const res = await fetch(`${ORIGIN}${ROOT}/drop`, {
      method: 'PUT',
      body: new Uint8Array([1]),
      headers: { authorization: 'Bearer whatever' },
    })
    assert.equal(res.status, 200)
    const { token } = (await res.json()) as { token: string }
    await del(token)
  })
})

// ---------------------------------------------------------------------------
// TTL expiry (short-TTL server runs only)
// ---------------------------------------------------------------------------

describe('TTL', () => {
  test(
    'a drop expires: HEAD and GET are 404 after ttlMs',
    { skip: TTL_MS === null || TTL_MS > 15_000 ? 'HEW_RELAY_TTL_MS unset or too long for a live test' : false },
    async () => {
      const token = await put(new Uint8Array([1, 2, 3]))
      assert.equal((await fetch(`${ORIGIN}${ROOT}/drop/${token}`, { method: 'HEAD' })).status, 200)
      await new Promise((resolve) => setTimeout(resolve, (TTL_MS ?? 0) + 500))
      assert.equal((await fetch(`${ORIGIN}${ROOT}/drop/${token}`, { method: 'HEAD' })).status, 404)
      assert.equal((await fetch(`${ORIGIN}${ROOT}/drop/${token}`)).status, 404)
    },
  )
})

// ---------------------------------------------------------------------------
// Total memory cap (bounded-store server runs only)
// ---------------------------------------------------------------------------

describe('relay full', () => {
  test(
    'PUTs past the total cap are 503 {"error":"relay full"} with Retry-After; freeing a drop makes room',
    { skip: MAX_TOTAL_BYTES === null ? 'HEW_RELAY_MAX_TOTAL_BYTES unset' : false },
    async () => {
      const cap = MAX_TOTAL_BYTES ?? 0
      const chunk = Math.max(1, Math.floor(cap / 4))
      const tokens: string[] = []
      try {
        // Fill until refused (bounded loop — the cap must bite within a
        // handful of chunk-sized PUTs).
        let full: Response | null = null
        for (let i = 0; i < 16 && full === null; i++) {
          const res = await fetch(`${ORIGIN}${ROOT}/drop`, { method: 'PUT', body: new Uint8Array(chunk), headers: authHeaders() })
          if (res.status === 200) {
            tokens.push(((await res.json()) as { token: string }).token)
          } else {
            full = res
          }
        }
        assert.ok(full !== null, 'the relay never reported full')
        assert.equal(full.status, 503)
        assert.deepEqual(await full.json(), { error: 'relay full' })
        assert.ok(full.headers.get('retry-after') !== null, 'Retry-After missing')

        // A declared Content-Length that would overflow is also refused, before the body is read.
        if (!BUFFERS_BODY) {
          const declared = await putWithDeclaredLength(chunk, 16)
          assert.equal(declared, 503)
        }

        // Free one, and the same-sized PUT succeeds again.
        const freed = tokens.pop()
        assert.ok(freed !== undefined)
        assert.equal((await del(freed)).status, 204)
        const again = await fetch(`${ORIGIN}${ROOT}/drop`, { method: 'PUT', body: new Uint8Array(chunk), headers: authHeaders() })
        assert.equal(again.status, 200)
        tokens.push(((await again.json()) as { token: string }).token)
      } finally {
        await Promise.all(tokens.map((t) => del(t)))
      }
    },
  )
})

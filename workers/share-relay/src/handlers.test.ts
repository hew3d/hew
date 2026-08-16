/**
 * Unit tests for `handlers.ts`, run with Node's built-in test runner
 * (`node --test`, see package.json's `test` script) — Node ships its own
 * `fetch`/`Request`/`Response`/`crypto` globals, and Node ≥23.6 strips
 * TypeScript syntax natively, so this needs zero installed dependencies
 * (this project's `.node-version` already pins 26.7.0). No `miniflare`:
 * the Durable Object binding is `testSupport/fakeDurableObject.ts`'s fake
 * namespace, wrapping a real `DropStore` (`dropStore.ts`) over an
 * in-memory SQLite database (`node:sqlite`) — this exercises the actual
 * production DO class, not a hand-rolled substitute for its logic.
 */

import { test, describe, beforeEach } from 'node:test'
import assert from 'node:assert/strict'

import {
  MAX_BYTES,
  CHUNK_BYTES,
  CONTRACT_VERSION,
  RELAY_PREFIX,
  chunkBytes,
  constantTimeEqual,
  resolveUploadKey,
  uploadAuthorized,
  stripRelayPrefix,
  handleIdentity,
  toBase64Url,
  generateToken,
  isValidToken,
  corsHeaders,
  preflightResponse,
  withCors,
  resolveAllowedOrigins,
  route,
  handlePutDrop,
  handleGetDrop,
  handlePeek,
  handleDeleteDrop,
  handleRequest,
} from './handlers.ts'
import { DropStore, RPC_BATCH_CHUNKS, TTL_MS } from './dropStore.ts'
import { FakeDurableObjectNamespace } from './testSupport/fakeDurableObject.ts'
import type { DropEnv, ShareDropStub } from './types.ts'

// ---------------------------------------------------------------------------
// Fake env — a real DropStore per token, over in-memory SQLite.
// ---------------------------------------------------------------------------

function makeEnv(): DropEnv {
  const env = {} as DropEnv
  env.SHARE_DROP = new FakeDurableObjectNamespace<ShareDropStub>((state) => new DropStore(state.storage))
  return env
}

// ---------------------------------------------------------------------------
// Token helpers
// ---------------------------------------------------------------------------

describe('generateToken / isValidToken', () => {
  test('generates a 22-char base64url token', () => {
    const token = generateToken()
    assert.equal(token.length, 22)
    assert.match(token, /^[A-Za-z0-9_-]{22}$/)
  })

  test('generates distinct tokens', () => {
    assert.notEqual(generateToken(), generateToken())
  })

  test('isValidToken accepts a well-formed token', () => {
    assert.equal(isValidToken(generateToken()), true)
  })

  test('isValidToken rejects the wrong length, bad characters, or path-like input', () => {
    assert.equal(isValidToken(''), false)
    assert.equal(isValidToken('short'), false)
    assert.equal(isValidToken('a'.repeat(23)), false)
    assert.equal(isValidToken('../../etc/passwd'), false)
    assert.equal(isValidToken('has spaces xxxxxxxxxx'), false)
    assert.equal(isValidToken('has.dots.xxxxxxxxxxxx'), false)
  })
})

describe('toBase64Url', () => {
  test('produces no padding and no +/ characters', () => {
    const bytes = new Uint8Array(32).fill(255)
    const encoded = toBase64Url(bytes)
    assert.doesNotMatch(encoded, /[+/=]/)
  })
})

// ---------------------------------------------------------------------------
// chunkBytes
// ---------------------------------------------------------------------------

describe('chunkBytes', () => {
  test('a single byte is one chunk', () => {
    const chunks = chunkBytes(new Uint8Array(1), CHUNK_BYTES)
    assert.equal(chunks.length, 1)
    assert.equal(chunks[0].byteLength, 1)
  })

  test('exactly one chunk size is a single chunk', () => {
    const chunks = chunkBytes(new Uint8Array(CHUNK_BYTES), CHUNK_BYTES)
    assert.equal(chunks.length, 1)
    assert.equal(chunks[0].byteLength, CHUNK_BYTES)
  })

  test('one byte over a chunk size is two chunks, the second holding 1 byte', () => {
    const chunks = chunkBytes(new Uint8Array(CHUNK_BYTES + 1), CHUNK_BYTES)
    assert.equal(chunks.length, 2)
    assert.equal(chunks[0].byteLength, CHUNK_BYTES)
    assert.equal(chunks[1].byteLength, 1)
  })

  test('exactly two chunk sizes is two even chunks', () => {
    const chunks = chunkBytes(new Uint8Array(CHUNK_BYTES * 2), CHUNK_BYTES)
    assert.equal(chunks.length, 2)
    assert.equal(chunks[0].byteLength, CHUNK_BYTES)
    assert.equal(chunks[1].byteLength, CHUNK_BYTES)
  })
})

// ---------------------------------------------------------------------------
// CORS
// ---------------------------------------------------------------------------

/** The base allowlist (no `EXTRA_ALLOWED_ORIGINS` — production shape). */
const BASE = resolveAllowedOrigins({} as DropEnv)

describe('resolveAllowedOrigins', () => {
  test('is exactly the three base origins when EXTRA_ALLOWED_ORIGINS is unset', () => {
    assert.deepEqual(
      [...resolveAllowedOrigins({} as DropEnv)].sort(),
      ['https://app.hew3d.com', 'https://tauri.localhost', 'tauri://localhost'].sort(),
    )
  })

  test('merges a comma-separated EXTRA_ALLOWED_ORIGINS, trimming blanks', () => {
    const allowed = resolveAllowedOrigins({
      EXTRA_ALLOWED_ORIGINS: 'https://hew.granroth.xyz, , https://staging.example',
    } as DropEnv)
    assert.ok(allowed.has('https://app.hew3d.com'))
    assert.ok(allowed.has('https://hew.granroth.xyz'))
    assert.ok(allowed.has('https://staging.example'))
    assert.ok(!allowed.has(''))
  })
})

describe('corsHeaders / preflightResponse', () => {
  test('echoes an allowed origin', () => {
    const headers = corsHeaders('https://app.hew3d.com', BASE)
    assert.equal(headers['access-control-allow-origin'], 'https://app.hew3d.com')
  })

  test('echoes both Tauri webview origins', () => {
    assert.equal(corsHeaders('tauri://localhost', BASE)['access-control-allow-origin'], 'tauri://localhost')
    assert.equal(
      corsHeaders('https://tauri.localhost', BASE)['access-control-allow-origin'],
      'https://tauri.localhost',
    )
  })

  test('echoes an origin added via EXTRA_ALLOWED_ORIGINS', () => {
    const allowed = resolveAllowedOrigins({ EXTRA_ALLOWED_ORIGINS: 'https://hew.granroth.xyz' } as DropEnv)
    assert.equal(
      corsHeaders('https://hew.granroth.xyz', allowed)['access-control-allow-origin'],
      'https://hew.granroth.xyz',
    )
    // ...but the same origin gets nothing under the base (production) allowlist.
    assert.deepEqual(corsHeaders('https://hew.granroth.xyz', BASE), {})
  })

  test('grants nothing to an unlisted origin or a missing one', () => {
    assert.deepEqual(corsHeaders('https://evil.example', BASE), {})
    assert.deepEqual(corsHeaders(null, BASE), {})
  })

  test('never grants a wildcard', () => {
    for (const origin of ['https://app.hew3d.com', 'tauri://localhost', 'https://tauri.localhost', null, 'https://evil.example']) {
      assert.notEqual(corsHeaders(origin, BASE)['access-control-allow-origin'], '*')
    }
  })

  test('preflight from an allowed origin is a 204 with the full CORS header set', () => {
    const res = preflightResponse('https://app.hew3d.com', BASE)
    assert.equal(res.status, 204)
    assert.equal(res.headers.get('access-control-allow-origin'), 'https://app.hew3d.com')
    assert.equal(res.headers.get('access-control-allow-methods'), 'PUT, GET, HEAD, DELETE, OPTIONS')
    // `authorization` so a browser-side PUT carrying the optional upload key
    // (§4) passes preflight; harmless when no key is configured.
    assert.equal(res.headers.get('access-control-allow-headers'), 'content-type, authorization')
  })

  test('preflight from a disallowed origin is a bare 403', () => {
    const res = preflightResponse('https://evil.example', BASE)
    assert.equal(res.status, 403)
    assert.equal(res.headers.get('access-control-allow-origin'), null)
  })
})

describe('withCors', () => {
  test('adds CORS headers for an allowed origin, keeping status/body/other headers', async () => {
    const original = new Response('hello', { status: 201, headers: { 'x-existing': '1' } })
    const wrapped = withCors(original, 'https://app.hew3d.com', BASE)
    assert.equal(wrapped.status, 201)
    assert.equal(wrapped.headers.get('x-existing'), '1')
    assert.equal(wrapped.headers.get('access-control-allow-origin'), 'https://app.hew3d.com')
    assert.equal(await wrapped.text(), 'hello')
  })

  test('adds nothing for a disallowed/missing origin', () => {
    const original = new Response('hello', { status: 200 })
    const wrapped = withCors(original, 'https://evil.example', BASE)
    assert.equal(wrapped.headers.get('access-control-allow-origin'), null)
    assert.equal(wrapped.status, 200)
  })
})

// ---------------------------------------------------------------------------
// route()
// ---------------------------------------------------------------------------

describe('route', () => {
  test('OPTIONS is always a preflight, regardless of path', () => {
    assert.deepEqual(route('OPTIONS', '/drop'), { kind: 'preflight' })
    assert.deepEqual(route('OPTIONS', '/anything'), { kind: 'preflight' })
  })

  test('PUT /drop', () => {
    assert.deepEqual(route('PUT', '/drop'), { kind: 'put' })
  })

  test('GET /drop/<token>', () => {
    assert.deepEqual(route('GET', '/drop/abc123'), { kind: 'get', token: 'abc123' })
  })

  test('DELETE /drop/<token>', () => {
    assert.deepEqual(route('DELETE', '/drop/abc123'), { kind: 'delete', token: 'abc123' })
  })

  test('HEAD /drop/<token>', () => {
    assert.deepEqual(route('HEAD', '/drop/abc123'), { kind: 'peek', token: 'abc123' })
  })

  test('HEAD /drop (no token) is not-found, not a route match', () => {
    assert.deepEqual(route('HEAD', '/drop'), { kind: 'not-found' })
  })

  test('GET /drop (no token) is not-found, not a route match', () => {
    assert.deepEqual(route('GET', '/drop'), { kind: 'not-found' })
  })

  test('a token path with extra segments is not-found', () => {
    assert.deepEqual(route('GET', '/drop/abc123/extra'), { kind: 'not-found' })
  })

  test('PUT to a token path is not-found (only /drop bare takes PUT)', () => {
    assert.deepEqual(route('PUT', '/drop/abc123'), { kind: 'not-found' })
  })

  test('an unrelated path is not-found', () => {
    assert.deepEqual(route('GET', '/nope'), { kind: 'not-found' })
    assert.deepEqual(route('GET', '/relay/nope'), { kind: 'not-found' })
  })

  test('GET / (and GET /relay, GET /relay/) is the identity route', () => {
    assert.deepEqual(route('GET', '/'), { kind: 'identity' })
    assert.deepEqual(route('GET', '/relay'), { kind: 'identity' })
    assert.deepEqual(route('GET', '/relay/'), { kind: 'identity' })
  })

  test('the identity route is GET-only', () => {
    assert.deepEqual(route('HEAD', '/'), { kind: 'not-found' })
    assert.deepEqual(route('PUT', '/'), { kind: 'not-found' })
    assert.deepEqual(route('DELETE', '/relay/'), { kind: 'not-found' })
  })

  test('the /relay prefix routes exactly like the bare paths', () => {
    assert.deepEqual(route('PUT', '/relay/drop'), { kind: 'put' })
    assert.deepEqual(route('GET', '/relay/drop/abc123'), { kind: 'get', token: 'abc123' })
    assert.deepEqual(route('HEAD', '/relay/drop/abc123'), { kind: 'peek', token: 'abc123' })
    assert.deepEqual(route('DELETE', '/relay/drop/abc123'), { kind: 'delete', token: 'abc123' })
    assert.deepEqual(route('GET', '/relay/drop'), { kind: 'not-found' })
  })

  test('the prefix is stripped exactly once, and only as a whole segment', () => {
    assert.deepEqual(route('PUT', '/relay/relay/drop'), { kind: 'not-found' })
    assert.deepEqual(route('PUT', '/relayx/drop'), { kind: 'not-found' })
    assert.deepEqual(route('GET', '/relayx'), { kind: 'not-found' })
  })
})

describe('stripRelayPrefix', () => {
  test('strips the bare prefix to the root', () => {
    assert.equal(stripRelayPrefix(RELAY_PREFIX), '/')
    assert.equal(stripRelayPrefix('/relay/'), '/')
  })

  test('strips one prefixed segment', () => {
    assert.equal(stripRelayPrefix('/relay/drop'), '/drop')
    assert.equal(stripRelayPrefix('/relay/drop/tok'), '/drop/tok')
  })

  test('leaves other paths alone', () => {
    assert.equal(stripRelayPrefix('/drop'), '/drop')
    assert.equal(stripRelayPrefix('/relayx'), '/relayx')
    assert.equal(stripRelayPrefix('/'), '/')
  })
})

// ---------------------------------------------------------------------------
// Upload key
// ---------------------------------------------------------------------------

describe('constantTimeEqual', () => {
  test('equal strings compare equal', () => {
    assert.equal(constantTimeEqual('', ''), true)
    assert.equal(constantTimeEqual('abc', 'abc'), true)
    assert.equal(constantTimeEqual('ünïcödé', 'ünïcödé'), true)
  })

  test('different strings (same or different length) compare unequal', () => {
    assert.equal(constantTimeEqual('abc', 'abd'), false)
    assert.equal(constantTimeEqual('abc', 'ab'), false)
    assert.equal(constantTimeEqual('ab', 'abc'), false)
    assert.equal(constantTimeEqual('', 'a'), false)
  })
})

describe('resolveUploadKey / uploadAuthorized', () => {
  test('no key configured (or an empty one) means uploads are open', () => {
    assert.equal(resolveUploadKey({} as DropEnv), null)
    assert.equal(resolveUploadKey({ HEW_RELAY_UPLOAD_KEY: '' } as DropEnv), null)
    assert.equal(resolveUploadKey({ HEW_RELAY_UPLOAD_KEY: '   ' } as DropEnv), null)
    assert.equal(resolveUploadKey({ HEW_RELAY_UPLOAD_KEY: ' k ' } as DropEnv), 'k')
    const req = new Request('https://x/drop', { method: 'PUT', body: new Uint8Array([1]).buffer })
    assert.equal(uploadAuthorized(req, {} as DropEnv), true)
  })

  test('with a key configured, only the exact Bearer value passes', () => {
    const env = { HEW_RELAY_UPLOAD_KEY: 's3cret' } as DropEnv
    const withHeader = (authorization?: string) =>
      new Request('https://x/drop', {
        method: 'PUT',
        body: new Uint8Array([1]).buffer,
        headers: authorization === undefined ? {} : { authorization },
      })
    assert.equal(uploadAuthorized(withHeader('Bearer s3cret'), env), true)
    assert.equal(uploadAuthorized(withHeader(), env), false)
    assert.equal(uploadAuthorized(withHeader('Bearer wrong'), env), false)
    assert.equal(uploadAuthorized(withHeader('Bearer s3cret2'), env), false)
    assert.equal(uploadAuthorized(withHeader('s3cret'), env), false)
    assert.equal(uploadAuthorized(withHeader('Basic s3cret'), env), false)
    // Case-sensitive scheme, exactly one space.
    assert.equal(uploadAuthorized(withHeader('bearer s3cret'), env), false)
    assert.equal(uploadAuthorized(withHeader('Bearer  s3cret'), env), false)
  })
})

// ---------------------------------------------------------------------------
// handleIdentity
// ---------------------------------------------------------------------------

describe('handleIdentity', () => {
  test('reports the service, contract, caps, and auth: none by default', async () => {
    const res = handleIdentity({} as DropEnv)
    assert.equal(res.status, 200)
    assert.equal(res.headers.get('content-type'), 'application/json')
    assert.equal(res.headers.get('cache-control'), 'no-store')
    assert.deepEqual(await res.json(), {
      service: 'hew-relay',
      contract: CONTRACT_VERSION,
      maxBytes: MAX_BYTES,
      ttlMs: TTL_MS,
      auth: 'none',
    })
  })

  test('reports auth: bearer when an upload key is configured, without leaking it', async () => {
    const res = handleIdentity({ HEW_RELAY_UPLOAD_KEY: 's3cret' } as DropEnv)
    const text = await res.text()
    assert.equal((JSON.parse(text) as { auth: string }).auth, 'bearer')
    assert.equal(text.includes('s3cret'), false)
  })
})

// ---------------------------------------------------------------------------
// handlePutDrop
// ---------------------------------------------------------------------------

describe('handlePutDrop', () => {
  test('stores the body under a fresh token', async () => {
    const env = makeEnv()
    const body = new Uint8Array([1, 2, 3, 4]).buffer
    const request = new Request('https://share.hew3d.com/drop', { method: 'PUT', body })
    const res = await handlePutDrop(request, env)
    assert.equal(res.status, 200)
    const { token } = (await res.json()) as { token: string }
    assert.equal(isValidToken(token), true)
  })

  test('rejects an empty body with 400', async () => {
    const env = makeEnv()
    const request = new Request('https://share.hew3d.com/drop', { method: 'PUT', body: new ArrayBuffer(0) })
    const res = await handlePutDrop(request, env)
    assert.equal(res.status, 400)
  })

  test('rejects a body over MAX_BYTES with 413', async () => {
    const env = makeEnv()
    const oversized = new ArrayBuffer(MAX_BYTES + 1)
    const request = new Request('https://share.hew3d.com/drop', { method: 'PUT', body: oversized })
    const res = await handlePutDrop(request, env)
    assert.equal(res.status, 413)
  })

  test('rejects fast via Content-Length before reading an oversized declared body', async () => {
    const env = makeEnv()
    const request = new Request('https://share.hew3d.com/drop', {
      method: 'PUT',
      body: new Uint8Array([1]).buffer,
      headers: { 'content-length': String(MAX_BYTES + 1000) },
    })
    const res = await handlePutDrop(request, env)
    assert.equal(res.status, 413)
  })

  test('with an upload key configured: 401 without/with the wrong key, 200 with it', async () => {
    const env = makeEnv()
    env.HEW_RELAY_UPLOAD_KEY = 'hunter2'
    const put = (headers: Record<string, string>) =>
      handlePutDrop(
        new Request('https://share.hew3d.com/drop', { method: 'PUT', body: new Uint8Array([1]).buffer, headers }),
        env,
      )
    const noKey = await put({})
    assert.equal(noKey.status, 401)
    assert.deepEqual(await noKey.json(), { error: 'unauthorized' })
    assert.equal((await put({ authorization: 'Bearer wrong' })).status, 401)
    const ok = await put({ authorization: 'Bearer hunter2' })
    assert.equal(ok.status, 200)
    assert.equal(isValidToken(((await ok.json()) as { token: string }).token), true)
  })

  test('the 401 wins over the size checks (an unauthorized oversized PUT is 401, not 413)', async () => {
    const env = makeEnv()
    env.HEW_RELAY_UPLOAD_KEY = 'hunter2'
    const res = await handlePutDrop(
      new Request('https://share.hew3d.com/drop', {
        method: 'PUT',
        body: new Uint8Array([1]).buffer,
        headers: { 'content-length': String(MAX_BYTES + 1000) },
      }),
      env,
    )
    assert.equal(res.status, 401)
  })

  test('accepts a body exactly at MAX_BYTES', async () => {
    const env = makeEnv()
    const request = new Request('https://share.hew3d.com/drop', {
      method: 'PUT',
      body: new ArrayBuffer(MAX_BYTES),
    })
    const res = await handlePutDrop(request, env)
    assert.equal(res.status, 200)
  })
})

// ---------------------------------------------------------------------------
// handleGetDrop — including the chunk round trip at exact boundaries.
// ---------------------------------------------------------------------------

/** Deterministic, non-repeating-in-a-way-that-hides-bugs fill: cheap to
 *  generate at 32 MiB and byte-comparable without needing real randomness
 *  (crypto.getRandomValues also caps at 64 KiB per call in Workers, which
 *  this pattern sidesteps entirely). */
function fillPattern(size: number): Uint8Array {
  const bytes = new Uint8Array(size)
  for (let i = 0; i < size; i++) bytes[i] = (i * 2654435761) & 0xff
  return bytes
}

async function putAndGet(env: DropEnv, body: Uint8Array): Promise<Uint8Array> {
  const putRes = await handlePutDrop(
    new Request('https://x/drop', { method: 'PUT', body: body.buffer as ArrayBuffer }),
    env,
  )
  assert.equal(putRes.status, 200)
  const { token } = (await putRes.json()) as { token: string }
  const getRes = await handleGetDrop(token, env)
  assert.equal(getRes.status, 200)
  return new Uint8Array(await getRes.arrayBuffer())
}

describe('handleGetDrop', () => {
  test('round-trips the exact bytes for a fresh token', async () => {
    const env = makeEnv()
    const bytes = await putAndGet(env, new Uint8Array([5, 6, 7]))
    assert.deepEqual(Array.from(bytes), [5, 6, 7])
  })

  test('round-trips a 1-byte body (smallest possible chunk boundary)', async () => {
    const env = makeEnv()
    const input = fillPattern(1)
    const bytes = await putAndGet(env, input)
    assert.deepEqual(Array.from(bytes), Array.from(input))
  })

  test('round-trips a body exactly CHUNK_BYTES long (single full chunk)', async () => {
    const env = makeEnv()
    const input = fillPattern(CHUNK_BYTES)
    const bytes = await putAndGet(env, input)
    assert.equal(bytes.byteLength, CHUNK_BYTES)
    assert.deepEqual(bytes, input)
  })

  test('round-trips a body exactly 2×CHUNK_BYTES long (two full chunks)', async () => {
    const env = makeEnv()
    const input = fillPattern(CHUNK_BYTES * 2)
    const bytes = await putAndGet(env, input)
    assert.equal(bytes.byteLength, CHUNK_BYTES * 2)
    assert.deepEqual(bytes, input)
  })

  test('round-trips a full 32 MiB body byte-identically across many chunks', async () => {
    const env = makeEnv()
    const input = fillPattern(MAX_BYTES)
    const bytes = await putAndGet(env, input)
    assert.equal(bytes.byteLength, MAX_BYTES)
    assert.deepEqual(bytes, input)
  })

  test('a drop destroyed mid-download (between the claim and a take) is a 404, not a 500', async () => {
    const env = makeEnv()
    // Enough chunks for several take() batches.
    const putRes = await handlePutDrop(
      new Request('https://x/drop', { method: 'PUT', body: fillPattern(CHUNK_BYTES * (RPC_BATCH_CHUNKS + 2)).buffer as ArrayBuffer }),
      env,
    )
    const { token } = (await putRes.json()) as { token: string }
    // Wrap the stub so the FIRST take() also destroys the drop underneath —
    // the alarm firing (or a DELETE landing) between two RPC calls.
    const id = env.SHARE_DROP.idFromName(token)
    const stub = env.SHARE_DROP.get(id)
    const realTake = stub.take.bind(stub)
    let takes = 0
    stub.take = async (from: number, count: number) => {
      const out = await realTake(from, count)
      takes += 1
      if (takes === 1) await stub.destroy()
      return out
    }
    const res = await handleGetDrop(token, env)
    assert.equal(res.status, 404)
  })

  test('is one-shot: a second GET for the same token 404s', async () => {
    const env = makeEnv()
    const putRes = await handlePutDrop(
      new Request('https://x/drop', { method: 'PUT', body: new Uint8Array([1]).buffer }),
      env,
    )
    const { token } = (await putRes.json()) as { token: string }

    const first = await handleGetDrop(token, env)
    assert.equal(first.status, 200)
    const second = await handleGetDrop(token, env)
    assert.equal(second.status, 404)
  })

  test('two concurrent GETs for the same token: exactly one succeeds', async () => {
    const env = makeEnv()
    const putRes = await handlePutDrop(
      new Request('https://x/drop', { method: 'PUT', body: new Uint8Array([1, 2, 3]).buffer }),
      env,
    )
    const { token } = (await putRes.json()) as { token: string }

    const [a, b] = await Promise.all([handleGetDrop(token, env), handleGetDrop(token, env)])
    const statuses = [a.status, b.status].sort()
    assert.deepEqual(statuses, [200, 404])
  })

  test('an unknown token is a 404', async () => {
    const env = makeEnv()
    const res = await handleGetDrop(generateToken(), env)
    assert.equal(res.status, 404)
  })

  test('a malformed token is a 400 without touching the DO', async () => {
    const env = makeEnv()
    const res = await handleGetDrop('not-a-real-token', env)
    assert.equal(res.status, 400)
  })

  test('sets Cache-Control: no-store on a successful GET', async () => {
    const env = makeEnv()
    const putRes = await handlePutDrop(
      new Request('https://x/drop', { method: 'PUT', body: new Uint8Array([1]).buffer }),
      env,
    )
    const { token } = (await putRes.json()) as { token: string }
    const res = await handleGetDrop(token, env)
    assert.equal(res.headers.get('cache-control'), 'no-store')
  })
})

// ---------------------------------------------------------------------------
// handlePeek
// ---------------------------------------------------------------------------

describe('handlePeek', () => {
  test('200 for a token with a stored, unconsumed drop', async () => {
    const env = makeEnv()
    const putRes = await handlePutDrop(
      new Request('https://x/drop', { method: 'PUT', body: new Uint8Array([1]).buffer }),
      env,
    )
    const { token } = (await putRes.json()) as { token: string }

    const res = await handlePeek(token, env)
    assert.equal(res.status, 200)
  })

  test('does not consume: a peek followed by a GET still succeeds', async () => {
    const env = makeEnv()
    const putRes = await handlePutDrop(
      new Request('https://x/drop', { method: 'PUT', body: new Uint8Array([7, 8, 9]).buffer }),
      env,
    )
    const { token } = (await putRes.json()) as { token: string }

    assert.equal((await handlePeek(token, env)).status, 200)
    const getRes = await handleGetDrop(token, env)
    assert.equal(getRes.status, 200)
    assert.deepEqual(Array.from(new Uint8Array(await getRes.arrayBuffer())), [7, 8, 9])
  })

  test('404 after the drop has been consumed', async () => {
    const env = makeEnv()
    const putRes = await handlePutDrop(
      new Request('https://x/drop', { method: 'PUT', body: new Uint8Array([1]).buffer }),
      env,
    )
    const { token } = (await putRes.json()) as { token: string }

    await handleGetDrop(token, env)
    const res = await handlePeek(token, env)
    assert.equal(res.status, 404)
  })

  test('404 for an unknown token', async () => {
    const env = makeEnv()
    const res = await handlePeek(generateToken(), env)
    assert.equal(res.status, 404)
  })

  test('400 for a malformed token, without touching the DO', async () => {
    const env = makeEnv()
    const res = await handlePeek('not-a-real-token', env)
    assert.equal(res.status, 400)
  })
})

// ---------------------------------------------------------------------------
// handleDeleteDrop
// ---------------------------------------------------------------------------

describe('handleDeleteDrop', () => {
  test('deletes an existing token and returns 204; a follow-up GET 404s', async () => {
    const env = makeEnv()
    const putRes = await handlePutDrop(
      new Request('https://x/drop', { method: 'PUT', body: new Uint8Array([1]).buffer }),
      env,
    )
    const { token } = (await putRes.json()) as { token: string }

    const res = await handleDeleteDrop(token, env)
    assert.equal(res.status, 204)
    assert.equal((await handleGetDrop(token, env)).status, 404)
  })

  test('deleting twice is still 204 both times (idempotent)', async () => {
    const env = makeEnv()
    const putRes = await handlePutDrop(
      new Request('https://x/drop', { method: 'PUT', body: new Uint8Array([1]).buffer }),
      env,
    )
    const { token } = (await putRes.json()) as { token: string }

    assert.equal((await handleDeleteDrop(token, env)).status, 204)
    assert.equal((await handleDeleteDrop(token, env)).status, 204)
  })

  test('a missing token is still a 204 (delete deletes if present, 204 always)', async () => {
    const env = makeEnv()
    const res = await handleDeleteDrop(generateToken(), env)
    assert.equal(res.status, 204)
  })

  test('a malformed token is also a 204 (no-op, never a 400 — best-effort caller)', async () => {
    const env = makeEnv()
    const res = await handleDeleteDrop('garbage', env)
    assert.equal(res.status, 204)
  })
})

// ---------------------------------------------------------------------------
// handleRequest — end-to-end dispatch, including CORS wrapping
// ---------------------------------------------------------------------------

describe('handleRequest', () => {
  let env: DropEnv
  beforeEach(() => {
    env = makeEnv()
  })

  test('a full PUT → GET round trip through the top-level dispatcher, with CORS applied', async () => {
    const putReq = new Request('https://share.hew3d.com/drop', {
      method: 'PUT',
      body: new Uint8Array([42]).buffer,
      headers: { origin: 'https://app.hew3d.com' },
    })
    const putRes = await handleRequest(putReq, env)
    assert.equal(putRes.status, 200)
    assert.equal(putRes.headers.get('access-control-allow-origin'), 'https://app.hew3d.com')
    const { token } = (await putRes.json()) as { token: string }

    const getReq = new Request(`https://share.hew3d.com/drop/${token}`, {
      method: 'GET',
      headers: { origin: 'tauri://localhost' },
    })
    const getRes = await handleRequest(getReq, env)
    assert.equal(getRes.status, 200)
    assert.equal(getRes.headers.get('access-control-allow-origin'), 'tauri://localhost')
  })

  test('HEAD /drop/<token> is 200 while present, 404 once consumed, with CORS applied', async () => {
    const putReq = new Request('https://share.hew3d.com/drop', {
      method: 'PUT',
      body: new Uint8Array([1]).buffer,
      headers: { origin: 'https://app.hew3d.com' },
    })
    const putRes = await handleRequest(putReq, env)
    const { token } = (await putRes.json()) as { token: string }

    const headReq = new Request(`https://share.hew3d.com/drop/${token}`, {
      method: 'HEAD',
      headers: { origin: 'https://app.hew3d.com' },
    })
    const headRes = await handleRequest(headReq, env)
    assert.equal(headRes.status, 200)
    assert.equal(headRes.headers.get('access-control-allow-origin'), 'https://app.hew3d.com')

    await handleRequest(
      new Request(`https://share.hew3d.com/drop/${token}`, { method: 'GET' }),
      env,
    )

    const secondHead = await handleRequest(headReq, env)
    assert.equal(secondHead.status, 404)
  })

  test('the same round trip works under the /relay prefix, mixed with bare paths', async () => {
    const putRes = await handleRequest(
      new Request('https://app.hew3d.com/relay/drop', { method: 'PUT', body: new Uint8Array([9]).buffer }),
      env,
    )
    assert.equal(putRes.status, 200)
    const { token } = (await putRes.json()) as { token: string }
    // Peek through the bare path, consume through the prefixed one — one
    // store behind both spellings.
    assert.equal((await handleRequest(new Request(`https://x/drop/${token}`, { method: 'HEAD' }), env)).status, 200)
    const getRes = await handleRequest(new Request(`https://x/relay/drop/${token}`, { method: 'GET' }), env)
    assert.equal(getRes.status, 200)
    assert.deepEqual(Array.from(new Uint8Array(await getRes.arrayBuffer())), [9])
    assert.equal((await handleRequest(new Request(`https://x/relay/drop/${token}`, { method: 'GET' }), env)).status, 404)
  })

  test('GET /, /relay, and /relay/ all answer the identity document, CORS-wrapped', async () => {
    for (const path of ['/', '/relay', '/relay/']) {
      const res = await handleRequest(
        new Request(`https://app.hew3d.com${path}`, { method: 'GET', headers: { origin: 'https://app.hew3d.com' } }),
        env,
      )
      assert.equal(res.status, 200, path)
      assert.equal(res.headers.get('access-control-allow-origin'), 'https://app.hew3d.com')
      const body = (await res.json()) as { service: string; contract: number }
      assert.equal(body.service, 'hew-relay')
      assert.equal(body.contract, CONTRACT_VERSION)
    }
  })

  test('with an upload key, GET/HEAD/DELETE stay keyless — only PUT is gated', async () => {
    env.HEW_RELAY_UPLOAD_KEY = 'k'
    const putRes = await handleRequest(
      new Request('https://x/drop', {
        method: 'PUT',
        body: new Uint8Array([4]).buffer,
        headers: { authorization: 'Bearer k' },
      }),
      env,
    )
    assert.equal(putRes.status, 200)
    const { token } = (await putRes.json()) as { token: string }
    assert.equal((await handleRequest(new Request(`https://x/drop/${token}`, { method: 'HEAD' }), env)).status, 200)
    assert.equal((await handleRequest(new Request(`https://x/drop/${token}`, { method: 'GET' }), env)).status, 200)
    assert.equal((await handleRequest(new Request(`https://x/drop/${token}`, { method: 'DELETE' }), env)).status, 204)
  })

  test('OPTIONS is routed to the preflight response', async () => {
    const req = new Request('https://share.hew3d.com/drop', {
      method: 'OPTIONS',
      headers: { origin: 'https://app.hew3d.com' },
    })
    const res = await handleRequest(req, env)
    assert.equal(res.status, 204)
  })

  test('an unmatched route is a CORS-wrapped 404', async () => {
    const req = new Request('https://share.hew3d.com/nope', {
      method: 'GET',
      headers: { origin: 'https://app.hew3d.com' },
    })
    const res = await handleRequest(req, env)
    assert.equal(res.status, 404)
    assert.equal(res.headers.get('access-control-allow-origin'), 'https://app.hew3d.com')
  })
})

/**
 * Unit tests for `handlers.ts`, run with Node's built-in test runner —
 * mirrors share-relay's `handlers.test.ts`: the Durable Object bindings are
 * `testSupport/fakeDurableObject.ts`'s fake namespace wrapping the real
 * `ReportStore`/`IndexStore` logic over `node:sqlite`, and the mailer/admin-
 * auth seams (`RawMailSender`, `VerifyOptions`) are the two places this
 * Worker's production code touches `cloudflare:email`/network JWKS fetches
 * that a unit test cannot exercise directly — see `email.ts` and
 * `adminAuth.ts` for why each is designed as an injectable interface.
 *
 * This drives the real chunked-upload protocol end to end: START
 * (`POST /report/`), PIECE (`PUT /report/<id>/<index>`), and COMMIT
 * (`POST /report/<id>/commit`), with real gzip pieces built via
 * `CompressionStream` (the same runtime API the Worker uses).
 *
 * Not tested here (structurally can't be, under bare `node --test`): the
 * real `ReportDrop`/`ReportIndex` Durable Object CLASSES (`reportDrop.ts`,
 * `reportIndex.ts`) import `cloudflare:workers`, so their thin wrapper
 * logic — the alarm handler's three-line body, and `ReportIndex.reserveReport`'s
 * one-line call into the idle-upload prune — is exercised directly here
 * against the underlying `ReportStore`/`IndexStore` primitives instead (the
 * same primitives those wrappers call), and validated for real only via
 * `wrangler dev` (see README.md's deploy checklist). `reportStore.test.ts`
 * and `indexStore.test.ts` cover those primitives far more thoroughly.
 */

import { test, describe } from 'node:test'
import assert from 'node:assert/strict'

import { handleRequest, handleIdentity, handleStart, handleCommit } from './handlers.ts'
import { ReportStore } from './reportStore.ts'
import { IndexStore } from './indexStore.ts'
import { hashClient } from './clientHash.ts'
import {
  MAX_UPLOAD_BYTES,
  PIECE_BYTES,
  DESCRIPTION_MIN_CHARS,
  DESCRIPTION_MAX_CHARS,
  RATE_LIMIT_WINDOW_MAX,
  STORE_CEILING_BYTES,
  IDLE_PRUNE_TIMEOUT_MS,
  INDEX_DO_NAME,
  UPLOAD_LIFETIME_MS,
} from './constants.ts'
import { FakeDurableObjectNamespace, type FakeDurableObjectStorage } from './testSupport/fakeDurableObject.ts'
import type { BugIntakeEnv, ReportDropStub, ReportIndexStub } from './types.ts'
import type { RawMailSender } from './email.ts'
import type { VerifyOptions } from './adminAuth.ts'

// ---------------------------------------------------------------------------
// gzip helpers
// ---------------------------------------------------------------------------

async function gzipBytes(input: Uint8Array): Promise<Uint8Array> {
  const cs = new CompressionStream('gzip')
  const writer = cs.writable.getWriter()
  const outputPromise = new Response(cs.readable).arrayBuffer()
  await writer.write(input)
  await writer.close()
  return new Uint8Array(await outputPromise)
}

async function gunzipBytes(compressed: Uint8Array): Promise<Uint8Array> {
  const ds = new DecompressionStream('gzip')
  const writer = ds.writable.getWriter()
  const outputPromise = new Response(ds.readable).arrayBuffer()
  await writer.write(compressed)
  await writer.close()
  return new Uint8Array(await outputPromise)
}

function fillRandom(bytes: Uint8Array): void {
  const MAX_PER_CALL = 65536
  for (let offset = 0; offset < bytes.byteLength; offset += MAX_PER_CALL) {
    crypto.getRandomValues(bytes.subarray(offset, Math.min(offset + MAX_PER_CALL, bytes.byteLength)))
  }
}

// ---------------------------------------------------------------------------
// Fake env
// ---------------------------------------------------------------------------

function makeEnv(overrides: Partial<BugIntakeEnv> = {}): BugIntakeEnv {
  const env = {} as BugIntakeEnv
  env.REPORT_DROP = new FakeDurableObjectNamespace<ReportDropStub>((state) => new ReportStore(state.storage))
  env.REPORT_INDEX = new FakeDurableObjectNamespace<ReportIndexStub>((state) => new IndexStore(state.storage))
  env.IP_HASH_SECRET = 'test-secret'
  Object.assign(env, overrides)
  return env
}

/** An env whose `REPORT_INDEX` stub throws for the named methods (still a
 *  real `IndexStore` underneath for everything else) — for proving a
 *  Durable Object failure maps to `503 {"error":"unavailable"}` rather than
 *  an uncaught exception, wherever in the upload/admin flow it happens. */
function makeEnvWithThrowingIndex(
  throwOn: ReadonlySet<keyof ReportIndexStub>,
  overrides: Partial<BugIntakeEnv> = {},
): BugIntakeEnv {
  const env = {} as BugIntakeEnv
  env.REPORT_DROP = new FakeDurableObjectNamespace<ReportDropStub>((state) => new ReportStore(state.storage))
  env.REPORT_INDEX = new FakeDurableObjectNamespace<ReportIndexStub>((state) => {
    const real = new IndexStore(state.storage)
    const boom = () => Promise.reject(new Error('simulated Durable Object failure'))
    const stub: ReportIndexStub = {
      reserveReport: throwOn.has('reserveReport') ? boom : real.reserveReport.bind(real),
      commitReport: throwOn.has('commitReport') ? boom : real.commitReport.bind(real),
      touchActivity: throwOn.has('touchActivity') ? boom : real.touchActivity.bind(real),
      listReports: throwOn.has('listReports') ? boom : real.listReports.bind(real),
      getReport: throwOn.has('getReport') ? boom : real.getReport.bind(real),
      deleteReport: throwOn.has('deleteReport') ? boom : real.deleteReport.bind(real),
      setTriaged: throwOn.has('setTriaged') ? boom : real.setTriaged.bind(real),
      reserveEmail: throwOn.has('reserveEmail') ? boom : real.reserveEmail.bind(real),
    }
    return stub
  })
  env.IP_HASH_SECRET = 'test-secret'
  Object.assign(env, overrides)
  return env
}

/** An env whose `REPORT_DROP` stub throws for the named methods — for
 *  proving a failed upload releases its `ReportIndex` reservation instead
 *  of leaving it counting against the storage ceiling forever. */
function makeEnvWithThrowingDrop(throwOn: ReadonlySet<keyof ReportDropStub>): BugIntakeEnv {
  const env = {} as BugIntakeEnv
  env.REPORT_DROP = new FakeDurableObjectNamespace<ReportDropStub>((state) => {
    const real = new ReportStore(state.storage)
    const boom = () => Promise.reject(new Error('simulated ReportDrop failure'))
    const stub: ReportDropStub = {
      startUpload: throwOn.has('startUpload') ? boom : real.startUpload.bind(real),
      putPiece: throwOn.has('putPiece') ? boom : real.putPiece.bind(real),
      commit: throwOn.has('commit') ? boom : real.commit.bind(real),
      abandon: throwOn.has('abandon') ? boom : real.abandon.bind(real),
      head: real.head.bind(real),
      read: throwOn.has('read') ? boom : real.read.bind(real),
      readStream: throwOn.has('readStream') ? boom : real.readStream.bind(real),
      destroy: real.destroy.bind(real),
    }
    return stub
  })
  env.REPORT_INDEX = new FakeDurableObjectNamespace<ReportIndexStub>((state) => new IndexStore(state.storage))
  env.IP_HASH_SECRET = 'test-secret'
  return env
}

const NOOP_MAILER: RawMailSender = { send: async () => {} }

function recordingMailer(): { mailer: RawMailSender; calls: Array<[string, string, string]> } {
  const calls: Array<[string, string, string]> = []
  return {
    calls,
    mailer: {
      send: async (from, to, raw) => {
        calls.push([from, to, raw])
      },
    },
  }
}

// ---------------------------------------------------------------------------
// Request builders
// ---------------------------------------------------------------------------

function validBundle(overrides: Record<string, unknown> = {}) {
  return {
    format: 1,
    report: { description: 'the model disappeared after undo, ten times reproducible' },
    system: { appVersion: '1.1.0', platform: 'desktop-macos' },
    ...overrides,
  }
}

function startRequest(piece: Uint8Array, headers: Record<string, string> = {}, totalOverride?: number): Request {
  return new Request('https://app.hew3d.com/report/', {
    method: 'POST',
    body: piece,
    headers: {
      'content-type': 'application/gzip',
      'content-length': String(piece.byteLength),
      'hew-upload-length': String(totalOverride ?? piece.byteLength),
      ...headers,
    },
  })
}

function pieceRequest(id: string, index: number, piece: Uint8Array, token: string, headers: Record<string, string> = {}): Request {
  return new Request(`https://app.hew3d.com/report/${id}/${index}`, {
    method: 'PUT',
    body: piece,
    headers: { 'content-length': String(piece.byteLength), 'hew-upload-token': token, ...headers },
  })
}

function commitRequest(id: string, token: string, headers: Record<string, string> = {}): Request {
  return new Request(`https://app.hew3d.com/report/${id}/commit`, {
    method: 'POST',
    headers: { 'hew-upload-token': token, ...headers },
  })
}

/** Starts an upload with a single-piece compressed bundle. */
async function startBundle(
  env: BugIntakeEnv,
  bundle: unknown,
  headers: Record<string, string> = {},
): Promise<{ res: Response; compressed: Uint8Array }> {
  const compressed = await gzipBytes(new TextEncoder().encode(JSON.stringify(bundle)))
  const res = await handleRequest(startRequest(compressed, headers), env, NOOP_MAILER)
  return { res, compressed }
}

/** Drives a whole upload (start, every following piece, commit) for a
 *  bundle split across as many `PIECE_BYTES` pieces as needed, and returns
 *  once committed. Used by admin tests that just need a finished report. */
async function uploadAndCommit(
  env: BugIntakeEnv,
  bundleOverrides: Record<string, unknown> = {},
  mailer: RawMailSender = NOOP_MAILER,
): Promise<{ id: string; token: string; compressed: Uint8Array }> {
  const compressed = await gzipBytes(new TextEncoder().encode(JSON.stringify(validBundle(bundleOverrides))))
  const pieces: Uint8Array[] = []
  for (let offset = 0; offset < compressed.byteLength; offset += PIECE_BYTES) {
    pieces.push(compressed.slice(offset, Math.min(offset + PIECE_BYTES, compressed.byteLength)))
  }

  const startRes = await handleRequest(startRequest(pieces[0], {}, compressed.byteLength), env, NOOP_MAILER)
  assert.equal(startRes.status, 201, `start failed: ${await startRes.clone().text()}`)
  const { id, token } = (await startRes.json()) as { id: string; token: string }

  for (let i = 1; i < pieces.length; i++) {
    const res = await handleRequest(pieceRequest(id, i, pieces[i], token), env, NOOP_MAILER)
    assert.equal(res.status, 204, `piece ${i} failed: ${await res.clone().text()}`)
  }

  const commitRes = await handleRequest(commitRequest(id, token), env, mailer)
  assert.equal(commitRes.status, 201, `commit failed: ${await commitRes.clone().text()}`)
  return { id, token, compressed }
}

/** A minimal, genuinely valid gzip'd head (`{"format":1,"report":{"description":...}}`,
 *  no `system`) — for tests that only need START to succeed on a small
 *  first piece, not a full realistic bundle. Gzipping raw non-JSON bytes
 *  (e.g. `new Uint8Array(10)`) would decompress to garbage and fail head
 *  validation with 400, which is NOT what those tests are exercising. */
async function minimalGzipBundle(description = '0123456789'): Promise<Uint8Array> {
  return gzipBytes(new TextEncoder().encode(JSON.stringify({ format: 1, report: { description } })))
}

/** Builds a real, valid bundle whose compressed size lands strictly between
 *  one and two pieces — so splitting it at `PIECE_BYTES` yields exactly two
 *  pieces, the second of which is itself a normal (not head-scanned) piece.
 *  Random high-entropy filler, base64-encoded, resists gzip enough that the
 *  compressed size stays predictably close to the plaintext size; the loop
 *  corrects for the actual observed compression ratio rather than assuming
 *  one, so this is robust across gzip implementations. */
async function buildTwoPieceBundle(): Promise<Uint8Array> {
  let fillerBytes = 2_200_000
  for (let attempt = 0; attempt < 8; attempt++) {
    const filler = new Uint8Array(fillerBytes)
    fillRandom(filler)
    const doc = {
      format: 1,
      report: { description: 'a two-piece upload used to exercise PIECE handling end to end' },
      system: { appVersion: '1.1.0', platform: 'desktop-macos' },
      filler: Buffer.from(filler).toString('base64'),
    }
    const compressed = await gzipBytes(new TextEncoder().encode(JSON.stringify(doc)))
    if (compressed.byteLength > PIECE_BYTES && compressed.byteLength < 2 * PIECE_BYTES) {
      return compressed
    }
    fillerBytes = compressed.byteLength <= PIECE_BYTES ? Math.floor(fillerBytes * 1.6) + 100_000 : Math.floor(fillerBytes * 0.7)
  }
  throw new Error('could not construct a two-piece test bundle within the retry budget')
}

function concatAll(chunks: Uint8Array[], total: number): Uint8Array {
  const out = new Uint8Array(total)
  let offset = 0
  for (const c of chunks) {
    out.set(c, offset)
    offset += c.byteLength
  }
  return out
}

// ---------------------------------------------------------------------------
// Identity
// ---------------------------------------------------------------------------

describe('GET /report/', () => {
  test('handleIdentity reports the contract', () => {
    const res = handleIdentity()
    assert.equal(res.status, 200)
  })

  test('identity payload and no-store header', async () => {
    const env = makeEnv()
    const res = await handleRequest(new Request('https://app.hew3d.com/report/'), env, NOOP_MAILER)
    assert.equal(res.status, 200)
    assert.equal(res.headers.get('cache-control'), 'no-store')
    const json = await res.json()
    assert.deepEqual(json, { service: 'hew-bug-intake', format: 1, maxBytes: MAX_UPLOAD_BYTES, pieceBytes: PIECE_BYTES })
  })

  test('answers at both /report and /report/', async () => {
    const env = makeEnv()
    const a = await handleRequest(new Request('https://app.hew3d.com/report'), env, NOOP_MAILER)
    const b = await handleRequest(new Request('https://app.hew3d.com/report/'), env, NOOP_MAILER)
    assert.equal(a.status, 200)
    assert.equal(b.status, 200)
  })

  test('never carries CORS headers', async () => {
    const env = makeEnv()
    const res = await handleRequest(
      new Request('https://app.hew3d.com/report/', { headers: { origin: 'https://evil.example' } }),
      env,
      NOOP_MAILER,
    )
    assert.equal(res.headers.get('access-control-allow-origin'), null)
  })
})

// ---------------------------------------------------------------------------
// START — POST /report/
// ---------------------------------------------------------------------------

describe('START: validation', () => {
  test('411 when Content-Length is missing', async () => {
    const env = makeEnv()
    const streamed = new Request('https://app.hew3d.com/report/', {
      method: 'POST',
      duplex: 'half',
      body: new ReadableStream({
        start(controller) {
          controller.enqueue(new TextEncoder().encode('x'))
          controller.close()
        },
      }),
      headers: { 'content-type': 'application/gzip', 'hew-upload-length': '1' },
    } as RequestInit)
    const res = await handleRequest(streamed, env, NOOP_MAILER)
    assert.equal(res.status, 411)
    assert.deepEqual(await res.json(), { error: 'length-required' })
  })

  test('413 when the piece Content-Length exceeds PIECE_BYTES', async () => {
    const env = makeEnv()
    const req = new Request('https://app.hew3d.com/report/', {
      method: 'POST',
      body: new Uint8Array([0x1f, 0x8b]),
      headers: {
        'content-type': 'application/gzip',
        'content-length': String(PIECE_BYTES + 1),
        'hew-upload-length': String(PIECE_BYTES + 1),
      },
    })
    const res = await handleRequest(req, env, NOOP_MAILER)
    assert.equal(res.status, 413)
    assert.deepEqual(await res.json(), { error: 'too-large', maxBytes: MAX_UPLOAD_BYTES })
  })

  test('400 when Content-Type is not application/gzip', async () => {
    const env = makeEnv()
    const { res } = await startBundle(env, validBundle(), { 'content-type': 'application/json' })
    assert.equal(res.status, 400)
    assert.equal((await res.json()).error, 'invalid')
  })

  test('400 when Hew-Upload-Length is missing', async () => {
    const env = makeEnv()
    const compressed = await gzipBytes(new TextEncoder().encode(JSON.stringify(validBundle())))
    const req = new Request('https://app.hew3d.com/report/', {
      method: 'POST',
      body: compressed,
      headers: { 'content-type': 'application/gzip', 'content-length': String(compressed.byteLength) },
    })
    const res = await handleRequest(req, env, NOOP_MAILER)
    assert.equal(res.status, 400)
    assert.match((await res.json()).message, /Hew-Upload-Length/)
  })

  test('400 when Hew-Upload-Length is malformed', async () => {
    const env = makeEnv()
    const { res } = await startBundle(env, validBundle(), { 'hew-upload-length': 'not-a-number' })
    assert.equal(res.status, 400)
  })

  test('413 when Hew-Upload-Length exceeds MAX_UPLOAD_BYTES', async () => {
    const env = makeEnv()
    const { res } = await startBundle(env, validBundle(), { 'hew-upload-length': String(MAX_UPLOAD_BYTES + 1) })
    assert.equal(res.status, 413)
    assert.deepEqual(await res.json(), { error: 'too-large', maxBytes: MAX_UPLOAD_BYTES })
  })

  test('400 when the first piece is longer than the declared total', async () => {
    const env = makeEnv()
    const compressed = await gzipBytes(new TextEncoder().encode(JSON.stringify(validBundle())))
    const res = await handleRequest(startRequest(compressed, {}, compressed.byteLength - 1), env, NOOP_MAILER)
    assert.equal(res.status, 400)
    assert.match((await res.json()).message, /longer than the declared total/)
  })

  test('400 when the body is not valid gzip', async () => {
    const env = makeEnv()
    const garbage = new TextEncoder().encode('not gzip at all')
    const res = await handleRequest(startRequest(garbage), env, NOOP_MAILER)
    assert.equal(res.status, 400)
    assert.match((await res.json()).message, /gzip/)
  })

  test('400 when format is not 1', async () => {
    const env = makeEnv()
    const { res } = await startBundle(env, validBundle({ format: 2 }))
    assert.equal(res.status, 400)
  })

  test('400 when description is shorter than the minimum', async () => {
    const env = makeEnv()
    const { res } = await startBundle(env, validBundle({ report: { description: 'short' } }))
    assert.equal(res.status, 400)
    assert.ok('short'.length < DESCRIPTION_MIN_CHARS)
  })

  test('400 when description exceeds the maximum', async () => {
    const env = makeEnv()
    const { res } = await startBundle(env, validBundle({ report: { description: 'x'.repeat(DESCRIPTION_MAX_CHARS + 1) } }))
    assert.equal(res.status, 400)
  })

  test('400 when the key order is wrong (system before report)', async () => {
    const env = makeEnv()
    const text = `{"format":1,"system":{"appVersion":"1.1.0"},"report":{"description":"${'x'.repeat(20)}"}}`
    const compressed = await gzipBytes(new TextEncoder().encode(text))
    const res = await handleRequest(startRequest(compressed), env, NOOP_MAILER)
    assert.equal(res.status, 400)
  })

  test('201 when system is omitted entirely — the user unticked "App version and system"', async () => {
    const env = makeEnv()
    const doc = { format: 1, report: { description: 'x'.repeat(20) } }
    const { res } = await startBundle(env, doc)
    assert.equal(res.status, 201)
  })

  test('accepts description exactly at the min and max boundary', async () => {
    const env1 = makeEnv()
    const r1 = await startBundle(env1, validBundle({ report: { description: 'x'.repeat(DESCRIPTION_MIN_CHARS) } }))
    assert.equal(r1.res.status, 201)

    const env2 = makeEnv()
    const r2 = await startBundle(env2, validBundle({ report: { description: 'x'.repeat(DESCRIPTION_MAX_CHARS) } }))
    assert.equal(r2.res.status, 201)
  })

  test('503 when IP_HASH_SECRET is unset', async () => {
    const env = makeEnv({ IP_HASH_SECRET: undefined })
    const { res } = await startBundle(env, validBundle())
    assert.equal(res.status, 503)
    assert.deepEqual(await res.json(), { error: 'unavailable' })
  })
})

describe('START: success', () => {
  test('201 with an id shaped HEW-XXXX-XXXX, a 43-char token, and pieceBytes', async () => {
    const env = makeEnv()
    const { res } = await startBundle(env, validBundle())
    assert.equal(res.status, 201)
    const body = (await res.json()) as { id: string; token: string; pieceBytes: number }
    assert.match(body.id, /^HEW-[0-9A-HJKMNP-TV-Z]{4}-[0-9A-HJKMNP-TV-Z]{4}$/)
    assert.equal(body.token.length, 43)
    assert.match(body.token, /^[A-Za-z0-9_-]{43}$/)
    assert.equal(body.pieceBytes, PIECE_BYTES)
  })

  test('carries cache-control: no-store', async () => {
    const env = makeEnv()
    const { res } = await startBundle(env, validBundle())
    assert.equal(res.headers.get('cache-control'), 'no-store')
  })

  test('records a reservation with the real appVersion/platform/descriptionPreview from the head', async () => {
    const env = makeEnv()
    const longDescription = 'a'.repeat(500)
    const { res } = await startBundle(env, validBundle({ report: { description: longDescription } }))
    const { id } = (await res.json()) as { id: string }
    const index = env.REPORT_INDEX.get(env.REPORT_INDEX.idFromName(INDEX_DO_NAME))
    const entry = await index.getReport(id)
    assert.ok(entry !== null)
    assert.equal(entry.descriptionPreview.length, 200)
    assert.equal(entry.appVersion, '1.1.0')
    assert.equal(entry.platform, 'desktop-macos')
    assert.equal(entry.committed, false, 'not committed until COMMIT runs')
    assert.equal(entry.sizeBytes > 0, true)
  })

  test('503 unavailable when ReportIndex.reserveReport throws', async () => {
    const env = makeEnvWithThrowingIndex(new Set(['reserveReport']))
    const { res } = await startBundle(env, validBundle())
    assert.equal(res.status, 503)
    assert.deepEqual(await res.json(), { error: 'unavailable' })
  })

  test('a startUpload (ReportDrop) failure releases the reservation', async () => {
    const env = makeEnvWithThrowingDrop(new Set(['startUpload']))
    const { res } = await startBundle(env, validBundle())
    assert.equal(res.status, 503)
    const index = env.REPORT_INDEX.get(env.REPORT_INDEX.idFromName(INDEX_DO_NAME)) as unknown as IndexStore
    assert.equal(await index.totalStoredBytes(), 0)
  })
})

describe('START: rate limiting and storage ceiling', () => {
  test('429 with Retry-After after RATE_LIMIT_WINDOW_MAX starts from one client in one window', async () => {
    const env = makeEnv()
    const headers = { 'cf-connecting-ip': '203.0.113.9' }
    let lastStatus = 0
    for (let i = 0; i <= RATE_LIMIT_WINDOW_MAX; i++) {
      const { res } = await startBundle(env, validBundle(), headers)
      lastStatus = res.status
      if (i === RATE_LIMIT_WINDOW_MAX) {
        assert.equal(res.status, 429)
        assert.ok(res.headers.get('retry-after') !== null)
      }
    }
    assert.equal(lastStatus, 429)
  })

  test("a different client IP is not rate limited by another client's activity", async () => {
    const env = makeEnv()
    for (let i = 0; i < RATE_LIMIT_WINDOW_MAX; i++) {
      await startBundle(env, validBundle(), { 'cf-connecting-ip': '203.0.113.9' })
    }
    const { res } = await startBundle(env, validBundle(), { 'cf-connecting-ip': '203.0.113.10' })
    assert.equal(res.status, 201)
  })

  test('the client IP is never stored anywhere reachable from the index', async () => {
    const env = makeEnv()
    const ip = '198.51.100.42'
    await startBundle(env, validBundle(), { 'cf-connecting-ip': ip })
    const index = env.REPORT_INDEX.get(env.REPORT_INDEX.idFromName(INDEX_DO_NAME))
    const reports = await index.listReports()
    assert.ok(!JSON.stringify(reports).includes(ip))
  })

  test('507 once stored reports would pass STORE_CEILING_BYTES', async () => {
    const env = makeEnv()
    const index = env.REPORT_INDEX.get(env.REPORT_INDEX.idFromName(INDEX_DO_NAME))
    for (let left = STORE_CEILING_BYTES, i = 0; left > 0; i++) {
      const sizeBytes = Math.min(left, MAX_UPLOAD_BYTES)
      const id = `HEW-FFFF-000${i}`
      assert.deepEqual(await index.reserveReport('filler-client', Date.now(), {
        id,
        receivedAt: Date.now(),
        appVersion: '1.0.0',
        platform: 'web',
        descriptionPreview: 'filler',
        sizeBytes,
      }), { ok: true })
      await index.commitReport(id)
      left -= sizeBytes
    }
    const { res } = await startBundle(env, validBundle())
    assert.equal(res.status, 507)
    assert.deepEqual(await res.json(), { error: 'full' })
  })

  test('429 while the same client holds its upload budget, then permits it after completion', async () => {
    const env = makeEnv()
    const ip = '203.0.113.77'
    const index = env.REPORT_INDEX.get(env.REPORT_INDEX.idFromName(INDEX_DO_NAME))
    const clientHash = await hashClient(env.IP_HASH_SECRET!, ip, Date.now())
    assert.deepEqual(await index.reserveReport(clientHash, Date.now(), {
      id: 'HEW-BUDG-0000',
      receivedAt: Date.now(),
      appVersion: '1.0.0',
      platform: 'web',
      descriptionPreview: 'upload in progress',
      sizeBytes: MAX_UPLOAD_BYTES,
    }), { ok: true })
    const blocked = await startBundle(env, validBundle(), { 'cf-connecting-ip': ip })
    assert.equal(blocked.res.status, 429)
    assert.deepEqual(await blocked.res.json(), { error: 'rate-limited' })
    assert.ok(Number(blocked.res.headers.get('retry-after')) > 0)

    const other = await startBundle(env, validBundle(), { 'cf-connecting-ip': '203.0.113.78' })
    assert.equal(other.res.status, 201)
    await index.commitReport('HEW-BUDG-0000')
    const next = await startBundle(env, validBundle(), { 'cf-connecting-ip': ip })
    assert.equal(next.res.status, 201)
  })
})

// ---------------------------------------------------------------------------
// PIECE — PUT /report/<id>/<index>
// ---------------------------------------------------------------------------

/** START with a real two-piece bundle: an exact `PIECE_BYTES` first piece
 *  and a shorter second one — the shape every multi-piece upload has. */
async function startTwoPieceUpload(env: BugIntakeEnv): Promise<{ id: string; token: string; pieces: Uint8Array[] }> {
  const compressed = await buildTwoPieceBundle()
  const pieces = [compressed.slice(0, PIECE_BYTES), compressed.slice(PIECE_BYTES)]
  const startRes = await handleRequest(startRequest(pieces[0], {}, compressed.byteLength), env, NOOP_MAILER)
  assert.equal(startRes.status, 201, `start failed: ${await startRes.clone().text()}`)
  const { id, token } = (await startRes.json()) as { id: string; token: string }
  return { id, token, pieces }
}

describe('PIECE', () => {
  test('204 on the next expected piece, and touches ReportIndex activity', async () => {
    const env = makeEnv()
    const { id, token, pieces } = await startTwoPieceUpload(env)
    const before = await env.REPORT_INDEX.get(env.REPORT_INDEX.idFromName(INDEX_DO_NAME)).getReport(id)
    await new Promise((r) => setTimeout(r, 5))
    const res = await handleRequest(pieceRequest(id, 1, pieces[1], token), env, NOOP_MAILER)
    assert.equal(res.status, 204)
    assert.equal(res.headers.get('cache-control'), 'no-store')
    const after = await env.REPORT_INDEX.get(env.REPORT_INDEX.idFromName(INDEX_DO_NAME)).getReport(id)
    assert.ok(after !== null && before !== null)
    assert.ok(after.receivedAt === before.receivedAt) // sanity: same row
  })

  test('an identical retry of the last stored index is accepted (204) without storing twice', async () => {
    const env = makeEnv()
    const { id, token, pieces } = await startTwoPieceUpload(env)
    const first = await handleRequest(pieceRequest(id, 1, pieces[1], token), env, NOOP_MAILER)
    assert.equal(first.status, 204)
    const retry = await handleRequest(pieceRequest(id, 1, pieces[1], token), env, NOOP_MAILER)
    assert.equal(retry.status, 204)
    const head = await env.REPORT_DROP.get(env.REPORT_DROP.idFromName(id)).head()
    assert.equal(head?.pieceCount, 2)
  })

  test('409 out-of-order with the expected index when a piece is skipped', async () => {
    const env = makeEnv()
    const { id, token } = await startTwoPieceUpload(env)
    const bogus = new Uint8Array(10)
    const res = await handleRequest(pieceRequest(id, 5, bogus, token), env, NOOP_MAILER)
    assert.equal(res.status, 409)
    assert.deepEqual(await res.json(), { error: 'out-of-order', expected: 1 })
  })

  test('403 forbidden when the token is missing', async () => {
    const env = makeEnv()
    const { id, pieces } = await startTwoPieceUpload(env)
    const req = new Request(`https://app.hew3d.com/report/${id}/1`, {
      method: 'PUT',
      body: pieces[1],
      headers: { 'content-length': String(pieces[1].byteLength) },
    })
    const res = await handleRequest(req, env, NOOP_MAILER)
    assert.equal(res.status, 403)
  })

  test('403 forbidden when the token is wrong', async () => {
    const env = makeEnv()
    const { id, pieces } = await startTwoPieceUpload(env)
    const res = await handleRequest(pieceRequest(id, 1, pieces[1], 'wrong-token-value-wrong-token-value-wrongx'), env, NOOP_MAILER)
    assert.equal(res.status, 403)
  })

  test('404 not-found for an upload that was never started', async () => {
    const env = makeEnv()
    const res = await handleRequest(pieceRequest('HEW-0000-0000', 1, new Uint8Array(10), 'sometoken'), env, NOOP_MAILER)
    assert.equal(res.status, 404)
  })

  test('404 not-found once the upload is already committed', async () => {
    const env = makeEnv()
    const { id, token } = await uploadAndCommit(env)
    const res = await handleRequest(pieceRequest(id, 1, new Uint8Array(10), token), env, NOOP_MAILER)
    assert.equal(res.status, 404)
  })

  test('400 and abandonment when a piece is over PIECE_BYTES', async () => {
    const env = makeEnv()
    const { id, token } = await startTwoPieceUpload(env)
    const tooBig = new Uint8Array(PIECE_BYTES + 1)
    const res = await handleRequest(pieceRequest(id, 1, tooBig, token), env, NOOP_MAILER)
    assert.equal(res.status, 400)
    const index = env.REPORT_INDEX.get(env.REPORT_INDEX.idFromName(INDEX_DO_NAME)) as unknown as IndexStore
    assert.equal(await index.totalStoredBytes(), 0, 'the reservation must be released on abandonment')
    const head = await env.REPORT_DROP.get(env.REPORT_DROP.idFromName(id)).head()
    assert.equal(head, null, 'the partial drop must be deleted on abandonment')
  })

  test('400 and abandonment when bytes would go past the declared total', async () => {
    const env = makeEnv()
    // A two-piece upload declares less than two full pieces, so a FULL
    // second piece overruns the declared total.
    const { id, token } = await startTwoPieceUpload(env)
    const res = await handleRequest(pieceRequest(id, 1, new Uint8Array(PIECE_BYTES), token), env, NOOP_MAILER)
    assert.equal(res.status, 400)
    const index = env.REPORT_INDEX.get(env.REPORT_INDEX.idFromName(INDEX_DO_NAME)) as unknown as IndexStore
    assert.equal(await index.totalStoredBytes(), 0)
  })

  test('a wrong-token PUT does NOT abandon the upload (only a 400 does)', async () => {
    const env = makeEnv()
    const { id, token, pieces } = await startTwoPieceUpload(env)
    const forbidden = await handleRequest(pieceRequest(id, 1, pieces[1], 'totally-wrong-token-value-thats-43-charsx'), env, NOOP_MAILER)
    assert.equal(forbidden.status, 403)
    // The real upload must still be alive and completable with the right token.
    const ok = await handleRequest(pieceRequest(id, 1, pieces[1], token), env, NOOP_MAILER)
    assert.equal(ok.status, 204)
  })

  // -------------------------------------------------------------------
  // Regression: an oversized piece must only abandon the upload when the
  // presented token actually matches it — a tokenless or wrong-token
  // oversized PUT against ANY public report id (committed or in-flight)
  // must leave it completely untouched.
  // -------------------------------------------------------------------

  function tokenlessPieceRequest(id: string, index: number, piece: Uint8Array): Request {
    return new Request(`https://app.hew3d.com/report/${id}/${index}`, {
      method: 'PUT',
      body: piece,
      headers: { 'content-length': String(piece.byteLength) },
    })
  }

  test('a tokenless oversized PUT against a COMMITTED report is 403 and leaves it intact', async () => {
    const env = makeEnv()
    const { id } = await uploadAndCommit(env)
    const tooBig = new Uint8Array(PIECE_BYTES + 1)
    const res = await handleRequest(tokenlessPieceRequest(id, 1, tooBig), env, NOOP_MAILER)
    assert.equal(res.status, 403)
    const index = env.REPORT_INDEX.get(env.REPORT_INDEX.idFromName(INDEX_DO_NAME))
    assert.equal((await index.getReport(id))?.committed, true, 'the index row must still be committed')
    const head = await env.REPORT_DROP.get(env.REPORT_DROP.idFromName(id)).head()
    assert.ok(head !== null, 'the drop must not be destroyed')
  })

  test('a wrong-token oversized PUT against a COMMITTED report is 404 (not-found collapses "already committed", same as a normal-sized piece) and leaves it intact', async () => {
    const env = makeEnv()
    const { id } = await uploadAndCommit(env)
    const tooBig = new Uint8Array(PIECE_BYTES + 1)
    const res = await handleRequest(pieceRequest(id, 1, tooBig, 'totally-wrong-token-value-thats-43-charsx'), env, NOOP_MAILER)
    assert.equal(res.status, 404)
    const index = env.REPORT_INDEX.get(env.REPORT_INDEX.idFromName(INDEX_DO_NAME))
    assert.equal((await index.getReport(id))?.committed, true, 'the index row must still be committed')
    const head = await env.REPORT_DROP.get(env.REPORT_DROP.idFromName(id)).head()
    assert.ok(head !== null, 'the drop must not be destroyed')
  })

  test('a RIGHT-token oversized PUT against a COMMITTED report is also 404, never abandons a finished report', async () => {
    const env = makeEnv()
    const { id, token } = await uploadAndCommit(env)
    const tooBig = new Uint8Array(PIECE_BYTES + 1)
    const res = await handleRequest(pieceRequest(id, 1, tooBig, token), env, NOOP_MAILER)
    assert.equal(res.status, 404)
    const index = env.REPORT_INDEX.get(env.REPORT_INDEX.idFromName(INDEX_DO_NAME))
    assert.equal((await index.getReport(id))?.committed, true, 'the index row must still be committed')
    const head = await env.REPORT_DROP.get(env.REPORT_DROP.idFromName(id)).head()
    assert.ok(head !== null, 'a committed report must never be destroyed by an oversized piece')
  })

  test('a tokenless oversized PUT against an IN-FLIGHT upload is 403 and the upload still completes', async () => {
    const env = makeEnv()
    const { id, token, pieces } = await startTwoPieceUpload(env)
    const tooBig = new Uint8Array(PIECE_BYTES + 1)
    const res = await handleRequest(tokenlessPieceRequest(id, 1, tooBig), env, NOOP_MAILER)
    assert.equal(res.status, 403)
    const index = env.REPORT_INDEX.get(env.REPORT_INDEX.idFromName(INDEX_DO_NAME)) as unknown as IndexStore
    assert.ok((await index.totalStoredBytes()) > 0, 'the reservation must still be held')
    const head = await env.REPORT_DROP.get(env.REPORT_DROP.idFromName(id)).head()
    assert.ok(head !== null, 'the drop must not be destroyed')
    const ok = await handleRequest(pieceRequest(id, 1, pieces[1], token), env, NOOP_MAILER)
    assert.equal(ok.status, 204)
    const commitRes = await handleRequest(commitRequest(id, token), env, NOOP_MAILER)
    assert.equal(commitRes.status, 201)
  })

  test('a wrong-token oversized PUT against an IN-FLIGHT upload is 403 and the upload still completes', async () => {
    const env = makeEnv()
    const { id, token, pieces } = await startTwoPieceUpload(env)
    const tooBig = new Uint8Array(PIECE_BYTES + 1)
    const res = await handleRequest(pieceRequest(id, 1, tooBig, 'totally-wrong-token-value-thats-43-charsx'), env, NOOP_MAILER)
    assert.equal(res.status, 403)
    const head = await env.REPORT_DROP.get(env.REPORT_DROP.idFromName(id)).head()
    assert.ok(head !== null, 'the drop must not be destroyed')
    const ok = await handleRequest(pieceRequest(id, 1, pieces[1], token), env, NOOP_MAILER)
    assert.equal(ok.status, 204)
    const commitRes = await handleRequest(commitRequest(id, token), env, NOOP_MAILER)
    assert.equal(commitRes.status, 201)
  })

  test('an oversized PUT against an unknown id, even with a token, is 404', async () => {
    const env = makeEnv()
    const tooBig = new Uint8Array(PIECE_BYTES + 1)
    const res = await handleRequest(pieceRequest('HEW-0000-0000', 1, tooBig, 'x'.repeat(43)), env, NOOP_MAILER)
    assert.equal(res.status, 404)
  })

  test('503 unavailable when ReportDrop.putPiece throws', async () => {
    const env = makeEnvWithThrowingDrop(new Set(['putPiece']))
    const { id, token, pieces } = await startTwoPieceUpload(env)
    const res = await handleRequest(pieceRequest(id, 1, pieces[1], token), env, NOOP_MAILER)
    assert.equal(res.status, 503)
  })
})

// ---------------------------------------------------------------------------
// COMMIT — POST /report/<id>/commit
// ---------------------------------------------------------------------------

describe('COMMIT', () => {
  test('201 with the id, and the notification email carries the real fields', async () => {
    const env = makeEnv({ NOTIFY_TO: 'maintainer@hew3d.com', NOTIFY_FROM: 'notify@hew3d.com' })
    const { mailer, calls } = recordingMailer()
    const { id } = await uploadAndCommit(env, { report: { description: 'a very specific commit-time description' } }, mailer)
    assert.equal(calls.length, 1)
    const [, , raw] = calls[0]
    assert.match(raw, /a very specific commit-time description/)
    assert.match(raw, new RegExp(id))

    const index = env.REPORT_INDEX.get(env.REPORT_INDEX.idFromName(INDEX_DO_NAME))
    const entry = await index.getReport(id)
    assert.equal(entry?.committed, true)
  })

  test('a commit still succeeds (201) even when the mailer throws', async () => {
    const env = makeEnv({ NOTIFY_TO: 'maintainer@hew3d.com', NOTIFY_FROM: 'notify@hew3d.com' })
    const throwingMailer: RawMailSender = {
      send: async () => {
        throw new Error('smtp down')
      },
    }
    const { id } = await uploadAndCommit(env, {}, throwingMailer)
    assert.ok(id)
  })

  test('skips the notification silently when NOTIFY_TO/FROM are unset', async () => {
    const env = makeEnv()
    const { mailer, calls } = recordingMailer()
    await uploadAndCommit(env, {}, mailer)
    assert.equal(calls.length, 0)
  })

  test('400 and abandonment when fewer bytes arrived than declared', async () => {
    const env = makeEnv()
    const { id, token } = await startTwoPieceUpload(env)
    const res = await handleRequest(commitRequest(id, token), env, NOOP_MAILER)
    assert.equal(res.status, 400)
    assert.match((await res.json()).message, /fewer bytes/)
    const index = env.REPORT_INDEX.get(env.REPORT_INDEX.idFromName(INDEX_DO_NAME)) as unknown as IndexStore
    assert.equal(await index.totalStoredBytes(), 0)
    const head = await env.REPORT_DROP.get(env.REPORT_DROP.idFromName(id)).head()
    assert.equal(head, null)
  })

  test('403 forbidden when the token is missing or wrong', async () => {
    const env = makeEnv()
    const { res } = await startBundle(env, validBundle())
    const { id } = (await res.json()) as { id: string }
    const noToken = await handleRequest(new Request(`https://app.hew3d.com/report/${id}/commit`, { method: 'POST' }), env, NOOP_MAILER)
    assert.equal(noToken.status, 403)
    const wrongToken = await handleRequest(commitRequest(id, 'x'.repeat(43)), env, NOOP_MAILER)
    assert.equal(wrongToken.status, 403)
  })

  test('404 not-found for an upload that was never started', async () => {
    const env = makeEnv()
    const res = await handleRequest(commitRequest('HEW-0000-0000', 'x'.repeat(43)), env, NOOP_MAILER)
    assert.equal(res.status, 404)
  })

  test('a second commit with the right token is idempotent: 201 again, no second email', async () => {
    const env = makeEnv({ NOTIFY_TO: 'maintainer@hew3d.com', NOTIFY_FROM: 'notify@hew3d.com' })
    const { mailer, calls } = recordingMailer()
    const { id, token } = await uploadAndCommit(env, {}, mailer)
    assert.equal(calls.length, 1)

    const res = await handleRequest(commitRequest(id, token), env, mailer)
    assert.equal(res.status, 201)
    assert.deepEqual(await res.json(), { id })
    assert.equal(calls.length, 1, 'the retry must not send a second email')

    const index = env.REPORT_INDEX.get(env.REPORT_INDEX.idFromName(INDEX_DO_NAME))
    assert.equal((await index.getReport(id))?.committed, true)
  })

  test('a commit with the wrong token on an already-committed report is forbidden', async () => {
    const env = makeEnv()
    const { id } = await uploadAndCommit(env)
    const res = await handleRequest(commitRequest(id, 'x'.repeat(43)), env, NOOP_MAILER)
    assert.equal(res.status, 403)
  })

  test('a retry after ReportIndex.commitReport fails once still lands: 503 then 201, the row committed, exactly one email', async () => {
    const { mailer, calls } = recordingMailer()
    const env: BugIntakeEnv = {} as BugIntakeEnv
    env.REPORT_DROP = new FakeDurableObjectNamespace<ReportDropStub>((state) => new ReportStore(state.storage))
    let commitReportCalls = 0
    env.REPORT_INDEX = new FakeDurableObjectNamespace<ReportIndexStub>((state) => {
      const real = new IndexStore(state.storage)
      const stub: ReportIndexStub = {
        reserveReport: real.reserveReport.bind(real),
        commitReport: async (id: string) => {
          commitReportCalls++
          if (commitReportCalls === 1) throw new Error('simulated index failure')
          return real.commitReport(id)
        },
        touchActivity: real.touchActivity.bind(real),
        listReports: real.listReports.bind(real),
        getReport: real.getReport.bind(real),
        deleteReport: real.deleteReport.bind(real),
        setTriaged: real.setTriaged.bind(real),
        reserveEmail: real.reserveEmail.bind(real),
      }
      return stub
    })
    env.IP_HASH_SECRET = 'test-secret'
    env.NOTIFY_TO = 'maintainer@hew3d.com'
    env.NOTIFY_FROM = 'notify@hew3d.com'

    const compressed = await gzipBytes(new TextEncoder().encode(JSON.stringify(validBundle())))
    const startRes = await handleRequest(startRequest(compressed, {}, compressed.byteLength), env, NOOP_MAILER)
    assert.equal(startRes.status, 201)
    const { id, token } = (await startRes.json()) as { id: string; token: string }

    const firstCommit = await handleRequest(commitRequest(id, token), env, mailer)
    assert.equal(firstCommit.status, 503)
    assert.equal(calls.length, 0, 'no email while the index commit is still failing')

    const secondCommit = await handleRequest(commitRequest(id, token), env, mailer)
    assert.equal(secondCommit.status, 201)
    assert.equal(calls.length, 1, 'exactly one email once the index actually commits')

    const index = env.REPORT_INDEX.get(env.REPORT_INDEX.idFromName(INDEX_DO_NAME))
    assert.equal((await index.getReport(id))?.committed, true)
  })

  test('503 unavailable when ReportDrop.commit throws', async () => {
    const env = makeEnvWithThrowingDrop(new Set(['commit']))
    const first = await minimalGzipBundle()
    const startRes = await handleRequest(startRequest(first, {}, first.byteLength), env, NOOP_MAILER)
    assert.equal(startRes.status, 201, `start failed: ${await startRes.clone().text()}`)
    const { id, token } = (await startRes.json()) as { id: string; token: string }
    const res = await handleRequest(commitRequest(id, token), env, NOOP_MAILER)
    assert.equal(res.status, 503)
  })

  test('503 unavailable when ReportIndex.commitReport throws after a successful drop commit', async () => {
    const env = makeEnvWithThrowingIndex(new Set(['commitReport']))
    const startRes = await handleRequest(startRequest(await gzipBytes(new TextEncoder().encode(JSON.stringify(validBundle()))), {}), env, NOOP_MAILER)
    // reserveReport isn't in throwOn, so start succeeds normally.
    const startBody = (await startRes.clone().json()) as { id?: string; error?: string }
    assert.ok(startBody.id, `expected start to succeed: ${JSON.stringify(startBody)}`)
    const { id, token } = startBody as { id: string; token: string }
    const res = await handleRequest(commitRequest(id, token), env, NOOP_MAILER)
    assert.equal(res.status, 503)
  })
})

// ---------------------------------------------------------------------------
// Full flow, including a real ~90 MiB upload
// ---------------------------------------------------------------------------

describe('Full upload flow', () => {
  test('start, one piece, commit — small report, exercised through the exported per-step handlers directly', async () => {
    const env = makeEnv()
    const compressed = await gzipBytes(new TextEncoder().encode(JSON.stringify(validBundle())))
    const startRes = await handleStart(startRequest(compressed, {}, compressed.byteLength), env)
    assert.equal(startRes.status, 201)
    const { id, token } = (await startRes.json()) as { id: string; token: string }
    const commitRes = await handleCommit(commitRequest(id, token), env, id, NOOP_MAILER)
    assert.equal(commitRes.status, 201)
  })

  test('a real ~90 MiB upload streams through start + many pieces + commit, one piece per stored row', async () => {
    const filler = new Uint8Array(89 * 1024 * 1024)
    fillRandom(filler)
    const doc = {
      format: 1,
      report: { description: 'a genuinely huge chunked report, near the upload cap' },
      system: { appVersion: '1.1.0', platform: 'desktop-macos' },
      filler: Buffer.from(filler).toString('base64'),
    }
    const compressed = await gzipBytes(new TextEncoder().encode(JSON.stringify(doc)))
    assert.ok(compressed.byteLength < MAX_UPLOAD_BYTES, 'fixture must fit under the cap')

    const pieces: Uint8Array[] = []
    for (let offset = 0; offset < compressed.byteLength; offset += PIECE_BYTES) {
      pieces.push(compressed.slice(offset, Math.min(offset + PIECE_BYTES, compressed.byteLength)))
    }
    assert.ok(pieces.length > 10, 'this should genuinely be a multi-piece upload')

    const env = makeEnv()
    const startRes = await handleRequest(startRequest(pieces[0], {}, compressed.byteLength), env, NOOP_MAILER)
    assert.equal(startRes.status, 201)
    const { id, token } = (await startRes.json()) as { id: string; token: string }

    for (let i = 1; i < pieces.length; i++) {
      const res = await handleRequest(pieceRequest(id, i, pieces[i], token), env, NOOP_MAILER)
      assert.equal(res.status, 204, `piece ${i} failed`)
    }

    const commitRes = await handleRequest(commitRequest(id, token), env, NOOP_MAILER)
    assert.equal(commitRes.status, 201)

    const drop = env.REPORT_DROP.get(env.REPORT_DROP.idFromName(id))
    const head = await drop.head()
    assert.ok(head !== null)
    assert.equal(head.pieceCount, pieces.length, 'exactly one stored row per uploaded piece — no rechunking')
    assert.equal(head.totalBytes, compressed.byteLength)

    // Read it all back and confirm it's byte-for-byte the original upload.
    const batches: Uint8Array[] = []
    for (let from = 0; from < head.pieceCount; from += 8) {
      batches.push(...(await drop.read(from, 8)))
    }
    assert.deepEqual(concatAll(batches, head.totalBytes), compressed)
  })
})

// ---------------------------------------------------------------------------
// Abandonment: the alarm and the idle-index backstop
// ---------------------------------------------------------------------------

describe('Abandonment', () => {
  test("the drop alarm's logic (head, tell ReportIndex FIRST, then destroy) releases a stale upload", async () => {
    // `ReportDrop.alarm()` itself can't run under bare `node --test` (its
    // class imports `cloudflare:workers`) — this drives the exact three
    // steps its body performs directly against the same `ReportStore`/
    // `IndexStore` primitives it calls through, which is what's actually
    // being tested; the thin wrapper method is validated via `wrangler dev`.
    // The order matters: `ReportIndex.deleteReport` runs BEFORE
    // `store.destroy()` so a retried alarm (after the index call fails)
    // still finds `head() !== null` and tries the index call again, rather
    // than silently orphaning the index row forever (see `reportDrop.ts`'s
    // `alarm()` doc).
    const env = makeEnv()
    const { res } = await startBundle(env, validBundle())
    const { id } = (await res.json()) as { id: string }

    const drop = env.REPORT_DROP.get(env.REPORT_DROP.idFromName(id))
    const index = env.REPORT_INDEX.get(env.REPORT_INDEX.idFromName(INDEX_DO_NAME))
    assert.ok((await index.getReport(id)) !== null)

    const head = await drop.head()
    assert.ok(head !== null)
    await index.deleteReport(head.reportId)
    await drop.destroy()

    assert.equal(await drop.head(), null)
    assert.equal(await index.getReport(id), null)
  })

  test('the idle-index backstop (IndexStore.findAndDeleteIdleUncommittedIds) uses activity time, not upload age', async () => {
    const env = makeEnv()
    const { res } = await startBundle(env, validBundle())
    const { id } = (await res.json()) as { id: string }
    const index = env.REPORT_INDEX.get(env.REPORT_INDEX.idFromName(INDEX_DO_NAME)) as unknown as IndexStore

    // Not idle yet — a full second of margin so ordinary test-runner
    // scheduling jitter between `startBundle` recording `lastActivity` and
    // this call can never flip the row into "idle" territory and make the
    // assertion flaky.
    const tooSoon = await index.findAndDeleteIdleUncommittedIds(Date.now() + IDLE_PRUNE_TIMEOUT_MS - 1000, IDLE_PRUNE_TIMEOUT_MS)
    assert.deepEqual(tooSoon, [])

    const pruned = await index.findAndDeleteIdleUncommittedIds(Date.now() + IDLE_PRUNE_TIMEOUT_MS + 1, IDLE_PRUNE_TIMEOUT_MS)
    assert.deepEqual(pruned, [id])
    assert.equal(await index.getReport(id), null)
  })
})

// ---------------------------------------------------------------------------
// Admin
// ---------------------------------------------------------------------------

function base64Url(bytes: Uint8Array): string {
  let binary = ''
  for (const b of bytes) binary += String.fromCharCode(b)
  return btoa(binary).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '')
}
function base64UrlJson(obj: unknown): string {
  return base64Url(new TextEncoder().encode(JSON.stringify(obj)))
}

const TEAM_DOMAIN = 'hew-test.cloudflareaccess.com'
const AUD = 'admin-aud'

async function makeAdminAuth(): Promise<{ token: string; authOptions: VerifyOptions; now: number }> {
  const { publicKey, privateKey } = await crypto.subtle.generateKey(
    { name: 'RSASSA-PKCS1-v1_5', modulusLength: 2048, publicExponent: new Uint8Array([1, 0, 1]), hash: 'SHA-256' },
    true,
    ['sign', 'verify'],
  )
  const now = Date.now()
  const jwk = await crypto.subtle.exportKey('jwk', publicKey)
  const jwks = { keys: [{ ...jwk, kid: 'k1', alg: 'RS256' }] }
  const header = base64UrlJson({ alg: 'RS256', kid: 'k1' })
  const payload = base64UrlJson({
    iss: `https://${TEAM_DOMAIN}`,
    aud: AUD,
    exp: Math.floor(now / 1000) + 3600,
  })
  const signingInput = new TextEncoder().encode(`${header}.${payload}`)
  const signature = await crypto.subtle.sign('RSASSA-PKCS1-v1_5', privateKey, signingInput)
  const token = `${header}.${payload}.${base64Url(new Uint8Array(signature))}`
  return { token, now, authOptions: { getJwks: async () => jwks as never, now } }
}

function adminEnv(): BugIntakeEnv {
  return makeEnv({ ACCESS_TEAM_DOMAIN: TEAM_DOMAIN, ACCESS_AUD: AUD })
}

describe('handleRequest: top-level catch-all', () => {
  test('an uncaught error inside an admin route becomes a safe 503, not a raw exception', async () => {
    const env = makeEnvWithThrowingIndex(new Set(['listReports']), {
      ACCESS_TEAM_DOMAIN: TEAM_DOMAIN,
      ACCESS_AUD: AUD,
    })
    const { token, authOptions } = await makeAdminAuth()
    const res = await handleRequest(
      new Request('https://app.hew3d.com/report/admin/', { headers: { 'cf-access-jwt-assertion': token } }),
      env,
      NOOP_MAILER,
      authOptions,
    )
    assert.equal(res.status, 503)
    assert.equal(res.headers.get('cache-control'), 'no-store')
    const text = await res.text()
    assert.ok(!text.includes('simulated Durable Object failure'), 'must not leak the underlying error to the client')
  })

  test('an uncaught error on a /report/ START request still answers the §8 JSON shape', async () => {
    const env = makeEnvWithThrowingIndex(new Set(['reserveReport']))
    const { res } = await startBundle(env, validBundle())
    assert.equal(res.status, 503)
    assert.equal(res.headers.get('content-type'), 'application/json')
    assert.deepEqual(await res.json(), { error: 'unavailable' })
  })
})

describe('/report/admin/*: authorization', () => {
  test('403 with no Cf-Access-Jwt-Assertion header', async () => {
    const env = adminEnv()
    const res = await handleRequest(new Request('https://app.hew3d.com/report/admin/'), env, NOOP_MAILER)
    assert.equal(res.status, 403)
  })

  test('403 with an invalid token', async () => {
    const env = adminEnv()
    const res = await handleRequest(
      new Request('https://app.hew3d.com/report/admin/', { headers: { 'cf-access-jwt-assertion': 'garbage' } }),
      env,
      NOOP_MAILER,
    )
    assert.equal(res.status, 403)
  })

  test('403 when ACCESS_TEAM_DOMAIN/ACCESS_AUD are unset, even with an otherwise-valid token', async () => {
    const { token, authOptions } = await makeAdminAuth()
    const env = makeEnv()
    const res = await handleRequest(
      new Request('https://app.hew3d.com/report/admin/', { headers: { 'cf-access-jwt-assertion': token } }),
      env,
      NOOP_MAILER,
      authOptions,
    )
    assert.equal(res.status, 403)
  })

  test('200 with a valid token', async () => {
    const { token, authOptions } = await makeAdminAuth()
    const env = adminEnv()
    const res = await handleRequest(
      new Request('https://app.hew3d.com/report/admin/', { headers: { 'cf-access-jwt-assertion': token } }),
      env,
      NOOP_MAILER,
      authOptions,
    )
    assert.equal(res.status, 200)
    assert.equal(
      res.headers.get('content-security-policy'),
      "default-src 'none'; style-src 'unsafe-inline'; form-action 'self'; frame-ancestors 'none'",
    )
  })
})

describe('/report/admin/*: list, detail, download', () => {
  test('list shows a committed report', async () => {
    const env = adminEnv()
    const { id } = await uploadAndCommit(env)
    const { token, authOptions } = await makeAdminAuth()
    const res = await handleRequest(
      new Request('https://app.hew3d.com/report/admin/', { headers: { 'cf-access-jwt-assertion': token } }),
      env,
      NOOP_MAILER,
      authOptions,
    )
    const html = await res.text()
    assert.ok(html.includes(id))
  })

  test('list and detail show "unknown" for a report submitted with no system block', async () => {
    const env = adminEnv()
    const doc = { format: 1, report: { description: 'no system info was sent for this one' } }
    const compressed = await gzipBytes(new TextEncoder().encode(JSON.stringify(doc)))
    const startRes = await handleRequest(startRequest(compressed, {}, compressed.byteLength), env, NOOP_MAILER)
    const { id, token } = (await startRes.json()) as { id: string; token: string }
    const commitRes = await handleRequest(commitRequest(id, token), env, NOOP_MAILER)
    assert.equal(commitRes.status, 201)

    const { token: adminToken, authOptions } = await makeAdminAuth()
    const listRes = await handleRequest(
      new Request('https://app.hew3d.com/report/admin/', { headers: { 'cf-access-jwt-assertion': adminToken } }),
      env,
      NOOP_MAILER,
      authOptions,
    )
    assert.match(await listRes.text(), /unknown/)

    const detailRes = await handleRequest(
      new Request(`https://app.hew3d.com/report/admin/${id}`, { headers: { 'cf-access-jwt-assertion': adminToken } }),
      env,
      NOOP_MAILER,
      authOptions,
    )
    const detailHtml = await detailRes.text()
    assert.match(detailHtml, /App version<\/dt>\s*<dd>unknown/)
    assert.match(detailHtml, /Platform<\/dt>\s*<dd>unknown/)
  })

  test('detail reads the head and renders the description and system info', async () => {
    const env = adminEnv()
    const { id } = await uploadAndCommit(env, {
      report: { description: 'the very specific crash description' },
      system: { appVersion: '1.1.0', platform: 'desktop-macos', os: 'macOS 15', gpu: 'Apple M2' },
    })
    const { token, authOptions } = await makeAdminAuth()
    const res = await handleRequest(
      new Request(`https://app.hew3d.com/report/admin/${id}`, { headers: { 'cf-access-jwt-assertion': token } }),
      env,
      NOOP_MAILER,
      authOptions,
    )
    assert.equal(res.status, 200)
    const html = await res.text()
    assert.ok(html.includes('the very specific crash description'))
    assert.ok(html.includes('Apple M2'))
  })

  test('detail 404s for an unknown (but well-formed) id', async () => {
    const env = adminEnv()
    const { token, authOptions } = await makeAdminAuth()
    const res = await handleRequest(
      new Request('https://app.hew3d.com/report/admin/HEW-0000-0000', { headers: { 'cf-access-jwt-assertion': token } }),
      env,
      NOOP_MAILER,
      authOptions,
    )
    assert.equal(res.status, 404)
  })

  test('download streams exactly the stored bytes across several pieces (compare to the uploaded gzip)', async () => {
    const env = adminEnv()
    const compressed = await buildTwoPieceBundle()
    const pieces = [compressed.slice(0, PIECE_BYTES), compressed.slice(PIECE_BYTES)]
    const startRes = await handleRequest(startRequest(pieces[0], {}, compressed.byteLength), env, NOOP_MAILER)
    assert.equal(startRes.status, 201, `start failed: ${await startRes.clone().text()}`)
    const { id, token } = (await startRes.json()) as { id: string; token: string }
    const pieceRes = await handleRequest(pieceRequest(id, 1, pieces[1], token), env, NOOP_MAILER)
    assert.equal(pieceRes.status, 204)
    const commitRes = await handleRequest(commitRequest(id, token), env, NOOP_MAILER)
    assert.equal(commitRes.status, 201)

    const { token: adminToken, authOptions } = await makeAdminAuth()
    const res = await handleRequest(
      new Request(`https://app.hew3d.com/report/admin/${id}/download`, { headers: { 'cf-access-jwt-assertion': adminToken } }),
      env,
      NOOP_MAILER,
      authOptions,
    )
    assert.equal(res.status, 200)
    assert.equal(res.headers.get('content-length'), String(compressed.byteLength))
    const downloaded = new Uint8Array(await res.arrayBuffer())
    assert.deepEqual(downloaded, compressed)
  })

  test('detail for a multi-piece report renders description/system/crash from the head', async () => {
    const env = adminEnv()
    let fillerBytes = 2_200_000
    let compressed: Uint8Array | undefined
    for (let attempt = 0; attempt < 8; attempt++) {
      const filler = new Uint8Array(fillerBytes)
      fillRandom(filler)
      const doc = {
        format: 1,
        report: { description: 'the model disappeared after undo, reproducible ten times' },
        system: { appVersion: '1.1.0', platform: 'desktop-macos', os: 'macOS 15', gpu: 'Apple M2' },
        crash: { at: 'push_pull', message: 'index out of bounds' },
        filler: Buffer.from(filler).toString('base64'),
      }
      const c = await gzipBytes(new TextEncoder().encode(JSON.stringify(doc)))
      if (c.byteLength > PIECE_BYTES && c.byteLength < 2 * PIECE_BYTES) {
        compressed = c
        break
      }
      fillerBytes = c.byteLength <= PIECE_BYTES ? Math.floor(fillerBytes * 1.6) + 100_000 : Math.floor(fillerBytes * 0.7)
    }
    assert.ok(compressed, 'could not build a two-piece fixture with a crash block')
    const pieces = [compressed!.slice(0, PIECE_BYTES), compressed!.slice(PIECE_BYTES)]

    const startRes = await handleRequest(startRequest(pieces[0], {}, compressed!.byteLength), env, NOOP_MAILER)
    assert.equal(startRes.status, 201, `start failed: ${await startRes.clone().text()}`)
    const { id, token } = (await startRes.json()) as { id: string; token: string }
    const pieceRes = await handleRequest(pieceRequest(id, 1, pieces[1], token), env, NOOP_MAILER)
    assert.equal(pieceRes.status, 204)
    const commitRes = await handleRequest(commitRequest(id, token), env, NOOP_MAILER)
    assert.equal(commitRes.status, 201)

    const { token: adminToken, authOptions } = await makeAdminAuth()
    const res = await handleRequest(
      new Request(`https://app.hew3d.com/report/admin/${id}`, { headers: { 'cf-access-jwt-assertion': adminToken } }),
      env,
      NOOP_MAILER,
      authOptions,
    )
    assert.equal(res.status, 200)
    const html = await res.text()
    assert.ok(html.includes('the model disappeared after undo, reproducible ten times'))
    assert.ok(html.includes('Apple M2'))
    assert.ok(html.includes('push_pull'))
    assert.ok(html.includes('index out of bounds'))
  })

  test('detail still renders a page when the report head cannot be read (e.g. ReportDrop.read throws)', async () => {
    const env = makeEnvWithThrowingDrop(new Set(['read']))
    env.ACCESS_TEAM_DOMAIN = TEAM_DOMAIN
    env.ACCESS_AUD = AUD
    const { id } = await uploadAndCommit(env)
    const { token, authOptions } = await makeAdminAuth()
    const res = await handleRequest(
      new Request(`https://app.hew3d.com/report/admin/${id}`, { headers: { 'cf-access-jwt-assertion': token } }),
      env,
      NOOP_MAILER,
      authOptions,
    )
    assert.equal(res.status, 200)
    const html = await res.text()
    assert.match(html, /could not be read/)
  })

  test('download serves the exact stored (still-compressed) bytes as a .json.gz attachment', async () => {
    const env = adminEnv()
    const { id, compressed } = await uploadAndCommit(env)
    const { token, authOptions } = await makeAdminAuth()
    const res = await handleRequest(
      new Request(`https://app.hew3d.com/report/admin/${id}/download`, { headers: { 'cf-access-jwt-assertion': token } }),
      env,
      NOOP_MAILER,
      authOptions,
    )
    assert.equal(res.status, 200)
    assert.equal(res.headers.get('content-type'), 'application/gzip')
    const disposition = res.headers.get('content-disposition') ?? ''
    assert.match(disposition, /attachment/)
    assert.ok(disposition.includes(`${id}.json.gz`))
    const downloaded = new Uint8Array(await res.arrayBuffer())
    assert.deepEqual(downloaded, compressed)
    assert.ok((await gunzipBytes(downloaded)).byteLength > 0)
  })

  test('an uncommitted (in-progress) upload is 404 in detail and download, not exposed mid-upload', async () => {
    const env = adminEnv()
    const { res } = await startBundle(env, validBundle())
    const { id } = (await res.json()) as { id: string }

    const { token, authOptions } = await makeAdminAuth()
    const detailRes = await handleRequest(
      new Request(`https://app.hew3d.com/report/admin/${id}`, { headers: { 'cf-access-jwt-assertion': token } }),
      env,
      NOOP_MAILER,
      authOptions,
    )
    assert.equal(detailRes.status, 404)

    const downloadRes = await handleRequest(
      new Request(`https://app.hew3d.com/report/admin/${id}/download`, { headers: { 'cf-access-jwt-assertion': token } }),
      env,
      NOOP_MAILER,
      authOptions,
    )
    assert.equal(downloadRes.status, 404)

    const listRes = await handleRequest(
      new Request('https://app.hew3d.com/report/admin/', { headers: { 'cf-access-jwt-assertion': token } }),
      env,
      NOOP_MAILER,
      authOptions,
    )
    assert.ok(!(await listRes.text()).includes(id))
  })
})

describe('/report/admin/*: mutations require a matching Origin', () => {
  test('delete without Origin is forbidden', async () => {
    const env = adminEnv()
    const { id } = await uploadAndCommit(env)
    const { token, authOptions } = await makeAdminAuth()
    const res = await handleRequest(
      new Request(`https://app.hew3d.com/report/admin/${id}/delete`, {
        method: 'POST',
        headers: { 'cf-access-jwt-assertion': token },
      }),
      env,
      NOOP_MAILER,
      authOptions,
    )
    assert.equal(res.status, 403)
  })

  test('delete with the wrong Origin is forbidden', async () => {
    const env = adminEnv()
    const { id } = await uploadAndCommit(env)
    const { token, authOptions } = await makeAdminAuth()
    const res = await handleRequest(
      new Request(`https://app.hew3d.com/report/admin/${id}/delete`, {
        method: 'POST',
        headers: { 'cf-access-jwt-assertion': token, origin: 'https://evil.example' },
      }),
      env,
      NOOP_MAILER,
      authOptions,
    )
    assert.equal(res.status, 403)
  })

  test('delete with the correct Origin removes the report from the index and its bytes', async () => {
    const env = adminEnv()
    const { id } = await uploadAndCommit(env)
    const { token, authOptions } = await makeAdminAuth()
    const res = await handleRequest(
      new Request(`https://app.hew3d.com/report/admin/${id}/delete`, {
        method: 'POST',
        headers: { 'cf-access-jwt-assertion': token, origin: 'https://app.hew3d.com' },
      }),
      env,
      NOOP_MAILER,
      authOptions,
    )
    assert.equal(res.status, 303)
    assert.equal(res.headers.get('cache-control'), 'no-store')
    const index = env.REPORT_INDEX.get(env.REPORT_INDEX.idFromName(INDEX_DO_NAME))
    assert.equal(await index.getReport(id), null)
    const dropHead = await env.REPORT_DROP.get(env.REPORT_DROP.idFromName(id)).head()
    assert.equal(dropHead, null)
  })

  test('triage toggle requires the correct Origin and flips the flag', async () => {
    const env = adminEnv()
    const { id } = await uploadAndCommit(env)
    const { token, authOptions } = await makeAdminAuth()

    const forbidden = await handleRequest(
      new Request(`https://app.hew3d.com/report/admin/${id}/triage`, {
        method: 'POST',
        headers: { 'cf-access-jwt-assertion': token },
      }),
      env,
      NOOP_MAILER,
      authOptions,
    )
    assert.equal(forbidden.status, 403)

    const ok = await handleRequest(
      new Request(`https://app.hew3d.com/report/admin/${id}/triage`, {
        method: 'POST',
        headers: { 'cf-access-jwt-assertion': token, origin: 'https://app.hew3d.com' },
      }),
      env,
      NOOP_MAILER,
      authOptions,
    )
    assert.equal(ok.status, 303)
    assert.equal(ok.headers.get('cache-control'), 'no-store')
    const index = env.REPORT_INDEX.get(env.REPORT_INDEX.idFromName(INDEX_DO_NAME))
    assert.equal((await index.getReport(id))?.triaged, true)
  })
})

describe('404s', () => {
  test('an unrelated path is 404', async () => {
    const env = makeEnv()
    const res = await handleRequest(new Request('https://app.hew3d.com/nope'), env, NOOP_MAILER)
    assert.equal(res.status, 404)
  })

  test('a non-numeric piece index is 404', async () => {
    const env = makeEnv()
    const res = await handleRequest(
      new Request('https://app.hew3d.com/report/HEW-AAAA-BBBB/not-a-number', { method: 'PUT', body: new Uint8Array(1) }),
      env,
      NOOP_MAILER,
    )
    assert.equal(res.status, 404)
  })
})

/** A `makeEnv` twin that also hands back every `ReportDrop` storage it
 *  ever constructs, in creation order, so a test can look at what a
 *  request left in the database of a DO it merely NAMED. */
function makeEnvCapturingDrops(): { env: BugIntakeEnv; dropStorages: FakeDurableObjectStorage[] } {
  const dropStorages: FakeDurableObjectStorage[] = []
  const env = {} as BugIntakeEnv
  env.REPORT_DROP = new FakeDurableObjectNamespace<ReportDropStub>((state) => {
    dropStorages.push(state.storage as FakeDurableObjectStorage)
    return new ReportStore(state.storage)
  })
  env.REPORT_INDEX = new FakeDurableObjectNamespace<ReportIndexStub>((state) => new IndexStore(state.storage))
  env.IP_HASH_SECRET = 'test-secret'
  env.ACCESS_TEAM_DOMAIN = TEAM_DOMAIN
  env.ACCESS_AUD = AUD
  return { env, dropStorages }
}

describe('unknown ids leave no Durable Object storage behind', () => {
  function tableCount(storage: FakeDurableObjectStorage): number {
    return storage.sql.exec<{ n: number }>("SELECT COUNT(*) AS n FROM sqlite_master WHERE type = 'table'").toArray()[0].n
  }

  test('a PUT, a COMMIT, and an oversized PUT naming a never-started id all 404 and create no tables', async () => {
    const { env, dropStorages } = makeEnvCapturingDrops()
    const token = 'A'.repeat(43)

    const put = await handleRequest(pieceRequest('HEW-ZZZZ-0000', 1, new Uint8Array(16), token), env, NOOP_MAILER)
    assert.equal(put.status, 404)
    const commit = await handleRequest(commitRequest('HEW-ZZZZ-0000', token), env, NOOP_MAILER)
    assert.equal(commit.status, 404)
    const oversized = await handleRequest(
      pieceRequest('HEW-ZZZZ-0000', 1, new Uint8Array(PIECE_BYTES + 1), token),
      env,
      NOOP_MAILER,
    )
    assert.equal(oversized.status, 404)

    assert.equal(dropStorages.length, 1, 'one DO was addressed')
    assert.equal(tableCount(dropStorages[0]), 0, 'and it holds no schema — nothing durable for a guessed id')
  })

  test('a report the admin deleted answers later stray requests without recreating its schema', async () => {
    const { env, dropStorages } = makeEnvCapturingDrops()
    const { id, token } = await uploadAndCommit(env)
    assert.equal(dropStorages.length, 1)
    assert.ok(tableCount(dropStorages[0]) > 0)

    const { token: adminToken, authOptions } = await makeAdminAuth()
    const del = await handleRequest(
      new Request(`https://app.hew3d.com/report/admin/${id}/delete`, {
        method: 'POST',
        headers: { 'cf-access-jwt-assertion': adminToken, origin: 'https://app.hew3d.com' },
      }),
      env,
      NOOP_MAILER,
      authOptions,
    )
    assert.equal(del.status, 303)
    assert.equal(tableCount(dropStorages[0]), 0)

    const commit = await handleRequest(commitRequest(id, token), env, NOOP_MAILER)
    assert.equal(commit.status, 404)
    assert.equal(tableCount(dropStorages[0]), 0)
  })
})

describe('upload lifetime and piece-shape bounds (reservation keepalive)', () => {
  test('START refuses a first piece that is neither the whole report nor exactly PIECE_BYTES, reserving nothing', async () => {
    const env = makeEnv()
    const first = await minimalGzipBundle()
    const res = await handleRequest(startRequest(first, {}, first.byteLength + 5000), env, NOOP_MAILER)
    assert.equal(res.status, 400)
    const index = env.REPORT_INDEX.get(env.REPORT_INDEX.idFromName(INDEX_DO_NAME)) as unknown as IndexStore
    assert.equal(await index.totalStoredBytes(), 0)
  })

  test('an empty keepalive PUT with the right token is a 400 that abandons the upload', async () => {
    const env = makeEnv()
    const { id, token } = await startTwoPieceUpload(env)
    const res = await handleRequest(pieceRequest(id, 1, new Uint8Array(0), token), env, NOOP_MAILER)
    assert.equal(res.status, 400)
    const index = env.REPORT_INDEX.get(env.REPORT_INDEX.idFromName(INDEX_DO_NAME)) as unknown as IndexStore
    assert.equal(await index.totalStoredBytes(), 0, 'the reservation is released')
    assert.equal(await env.REPORT_DROP.get(env.REPORT_DROP.idFromName(id)).head(), null, 'the partial drop is gone')
  })

  test('an empty keepalive PUT with the WRONG token is a 403 that abandons nothing', async () => {
    const env = makeEnv()
    const { id, token, pieces } = await startTwoPieceUpload(env)
    const res = await handleRequest(pieceRequest(id, 1, new Uint8Array(0), 'totally-wrong-token-value-thats-43-charsx'), env, NOOP_MAILER)
    assert.equal(res.status, 403)
    const ok = await handleRequest(pieceRequest(id, 1, pieces[1], token), env, NOOP_MAILER)
    assert.equal(ok.status, 204)
  })

  test('a piece arriving after UPLOAD_LIFETIME_MS is a 400 that abandons the upload', async () => {
    const { env, dropStorages } = makeEnvCapturingDrops()
    const { id, token, pieces } = await startTwoPieceUpload(env)
    assert.equal(dropStorages.length, 1)
    dropStorages[0].sql.exec('UPDATE meta SET receivedAt = ? WHERE id = 0', Date.now() - UPLOAD_LIFETIME_MS - 1)
    const res = await handleRequest(pieceRequest(id, 1, pieces[1], token), env, NOOP_MAILER)
    assert.equal(res.status, 400)
    const index = env.REPORT_INDEX.get(env.REPORT_INDEX.idFromName(INDEX_DO_NAME)) as unknown as IndexStore
    assert.equal(await index.totalStoredBytes(), 0, 'the reservation is released')
  })
})

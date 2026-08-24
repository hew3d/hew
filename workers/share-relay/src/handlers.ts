/**
 * share-relay — the E2E-encrypted "Open on Phone" dead-drop (README.md has
 * the full picture: desktop encrypts, this Worker only ever sees
 * ciphertext, phone decrypts). This module is the pure request/response
 * logic, written as plain functions over the `DropEnv` binding interface
 * (`types.ts`) so it unit-tests without `miniflare` or a real Durable
 * Object — `handlers.test.ts` runs it against a fake DO namespace
 * (`testSupport/fakeDurableObject.ts`) wrapping a real `ShareDrop`
 * (`shareDrop.ts`) over an in-memory SQLite database. `index.ts` is the
 * thin `fetch` glue that wires this to the real Worker runtime.
 *
 * No auth, by design: a token is a bearer capability (128 bits of
 * randomness, unguessable), the server only ever stores opaque ciphertext,
 * and the decryption key never reaches it — it rides the receiving URL's
 * fragment, which browsers never send over the network. See README.md's
 * "Security model" section for the full argument.
 */

import type { DropEnv } from './types.ts'
import { RPC_BATCH_CHUNKS, TTL_MS } from './dropStore.ts'

/** Upload size ceiling — PUT /drop rejects anything larger with a 413. */
export const MAX_BYTES = 32 * 1024 * 1024

/** The relay contract version reported by the identity route (`GET /`).
 *  Bumped only for an incompatible change; every addition so far — the
 *  `/relay` prefix, this identity route, the optional bearer upload key —
 *  is additive, so old clients keep working against contract 1. The
 *  self-hostable Rust twin (`crates/hew-relay`) reports the same number;
 *  the black-box conformance suite (`contract/`) pins both to it. */
export const CONTRACT_VERSION = 1

/** Optional path prefix. Self-hosting serves the relay from the SAME origin
 *  as the web app under `/relay/` (docs/design/self-hosting-relay.md §2), and
 *  the public deployment adds an `app.hew3d.com/relay/*` route to this Worker
 *  alongside the legacy `share.hew3d.com/*` one. `route()` strips this once
 *  and dispatches, so `/relay/drop` and `/drop` are the same route — a
 *  reverse proxy that strips the prefix itself (nginx `proxy_pass` with a
 *  URI part) and one that forwards it verbatim both work. */
export const RELAY_PREFIX = '/relay'

/** Each upload is split into pieces this size before being handed to
 *  `ShareDrop.store` — a single SQLite row/BLOB in a Durable Object caps at
 *  2 MB, so a `MAX_BYTES`-sized upload can't be one row. 1.9 MB leaves
 *  comfortable headroom under that cap; `MAX_BYTES / CHUNK_BYTES` is at
 *  most ~17 rows per drop, nowhere near the 100k-rows-written/day free-tier
 *  ceiling for any plausible usage. */
export const CHUNK_BYTES = 1_900_000

/** Splits `bytes` into `chunkSize`-sized pieces (the last one shorter,
 *  unless `bytes.byteLength` divides evenly). Never called with an empty
 *  `bytes` — `handlePutDrop` rejects a 0-byte body before this runs — so
 *  this always returns at least one chunk for any input it actually sees. */
export function chunkBytes(bytes: Uint8Array, chunkSize: number): Uint8Array[] {
  const chunks: Uint8Array[] = []
  for (let offset = 0; offset < bytes.byteLength; offset += chunkSize) {
    chunks.push(bytes.subarray(offset, Math.min(offset + chunkSize, bytes.byteLength)))
  }
  return chunks
}

/** Placeholder passed as `ShareDrop.store`'s `name` — this Worker's PUT
 *  body is pure opaque ciphertext with no name field of its own (see
 *  `shareDrop.ts`'s `store` doc), so there's nothing real to put here. */
const DROP_NAME = 'ciphertext'

/** 128 bits of token entropy, base64url-encoded (22 chars, no padding). */
const TOKEN_BYTES = 16

/** The always-on origins: the hosted phone app plus the two Tauri-webview
 *  origins the desktop dialog runs under — no wildcard, per README.md.
 *  `tauri://localhost` is the Windows/Linux WebView2/WebKitGTK origin;
 *  `https://tauri.localhost` is macOS's. (The desktop upload actually goes
 *  through the native HTTP plugin now, bypassing browser CORS, but the two
 *  tauri origins stay allowed harmlessly.) */
const BASE_ALLOWED_ORIGINS = ['https://app.hew3d.com', 'tauri://localhost', 'https://tauri.localhost']

/** The effective allowlist for one request: the base origins plus any set
 *  in the `EXTRA_ALLOWED_ORIGINS` env var (comma-separated). That var is the
 *  supported way to point a self-hosted HTTPS test origin — e.g. a homelab
 *  `https://hew.granroth.xyz` serving the shop build — at this relay without
 *  editing (or shipping) it in the committed default. Unset in production, so
 *  production stays exactly the three base origins. */
export function resolveAllowedOrigins(env: DropEnv): Set<string> {
  const extra = (env.EXTRA_ALLOWED_ORIGINS ?? '')
    .split(',')
    .map((o) => o.trim())
    .filter((o) => o.length > 0)
  return new Set([...BASE_ALLOWED_ORIGINS, ...extra])
}

// ---------------------------------------------------------------------------
// Optional upload key (docs/design/self-hosting-relay.md §4)
// ---------------------------------------------------------------------------

/** The configured upload key, or `null` when uploads are open (production:
 *  the public relay's `PUT /drop` is unauthenticated by design — README.md's
 *  "Security model"). A self-hoster who exposes their relay to the internet
 *  sets `HEW_RELAY_UPLOAD_KEY` (a Worker secret here; an env/flag on the
 *  Rust binary) so only their own desktops can fill its memory. Only PUT
 *  checks it: GET/HEAD/DELETE stay keyless because the token is the
 *  capability and the phone never holds the upload key. */
export function resolveUploadKey(env: DropEnv): string | null {
  // Trimmed, and whitespace-only counts as unset — the same reading
  // hew-relay gives its `--upload-key`/env twin, so a stray space in a
  // secret field never silently becomes "the key is a single space".
  const key = (env.HEW_RELAY_UPLOAD_KEY ?? '').trim()
  return key.length > 0 ? key : null
}

/** Constant-time string equality over UTF-8 bytes — the loop always runs to
 *  the longer length and folds every byte (and the length difference) into
 *  one accumulator, so a mismatch takes the same time wherever it is. Not
 *  `crypto.subtle.timingSafeEqual`: that is a Cloudflare extension absent
 *  from Node, and this module's tests run under bare `node --test`. */
export function constantTimeEqual(a: string, b: string): boolean {
  const enc = new TextEncoder()
  const ab = enc.encode(a)
  const bb = enc.encode(b)
  let diff = ab.length ^ bb.length
  const n = Math.max(ab.length, bb.length)
  for (let i = 0; i < n; i++) diff |= (ab[i] ?? 0) ^ (bb[i] ?? 0)
  return diff === 0
}

/** Whether `request` carries `Authorization: Bearer <key>` for the configured
 *  upload key. Always true when no key is configured. The key is never
 *  logged or echoed anywhere — a failure is a bare 401. */
export function uploadAuthorized(request: Request, env: DropEnv): boolean {
  const key = resolveUploadKey(env)
  if (key === null) return true
  const header = request.headers.get('authorization') ?? ''
  const prefix = 'Bearer '
  if (!header.startsWith(prefix)) return false
  return constantTimeEqual(header.slice(prefix.length), key)
}

// ---------------------------------------------------------------------------
// Token
// ---------------------------------------------------------------------------

/** Base64url (RFC 4648 §5), no padding — used for both the token and (on
 *  the desktop side, `app/src/io/shareCrypto.ts`) the AES key riding the
 *  URL fragment. `btoa`/`atob` are available as Worker globals exactly like
 *  in a browser. */
export function toBase64Url(bytes: Uint8Array): string {
  let binary = ''
  for (const b of bytes) binary += String.fromCharCode(b)
  return btoa(binary).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '')
}

/** Generates a fresh drop token: 128 random bits, base64url. */
export function generateToken(): string {
  const bytes = new Uint8Array(TOKEN_BYTES)
  crypto.getRandomValues(bytes)
  return toBase64Url(bytes)
}

/** A valid token is exactly what `generateToken` produces: 22 base64url
 *  characters (16 bytes, unpadded). Checked before ever touching the DO —
 *  not a security boundary (a DO id derived from an arbitrary string is
 *  perfectly safe; there's no injection here), just a cheap way to turn an
 *  obviously-malformed request into a clean 400 instead of a pointless DO
 *  round trip. */
export function isValidToken(token: string): boolean {
  return /^[A-Za-z0-9_-]{22}$/.test(token)
}

// ---------------------------------------------------------------------------
// CORS
// ---------------------------------------------------------------------------

/** Headers to merge into any response for a request from `origin` — empty
 *  when `origin` is absent or not on the allowlist, which leaves the
 *  browser unable to read the response (the request itself, for a
 *  same-origin-blind method like GET, may still have "happened" server
 *  side, but that's true of any CORS policy: the allowlist protects what
 *  a browser page can READ, not whether the request reaches the server). */
export function corsHeaders(origin: string | null, allowed: Set<string>): Record<string, string> {
  if (origin === null || !allowed.has(origin)) return {}
  return {
    'access-control-allow-origin': origin,
    vary: 'origin',
  }
}

/** The OPTIONS preflight response. An origin outside the allowlist gets a
 *  bare 403 with no CORS headers at all — the browser's own preflight
 *  failure will block the real request regardless, so this just fails
 *  fast without echoing anything back to an unrecognized origin. */
export function preflightResponse(origin: string | null, allowed: Set<string>): Response {
  if (origin === null || !allowed.has(origin)) {
    return new Response(null, { status: 403 })
  }
  return new Response(null, {
    status: 204,
    headers: {
      'access-control-allow-origin': origin,
      'access-control-allow-methods': 'PUT, GET, HEAD, DELETE, OPTIONS',
      'access-control-allow-headers': 'content-type, authorization',
      'access-control-max-age': '86400',
      vary: 'origin',
    },
  })
}

/** Merges `corsHeaders(origin)` onto an existing response's headers,
 *  preserving its status/body — used to wrap every non-preflight response
 *  on the way out. */
export function withCors(response: Response, origin: string | null, allowed: Set<string>): Response {
  const cors = corsHeaders(origin, allowed)
  if (Object.keys(cors).length === 0) return response
  const headers = new Headers(response.headers)
  for (const [k, v] of Object.entries(cors)) headers.set(k, v)
  return new Response(response.body, { status: response.status, headers })
}

// ---------------------------------------------------------------------------
// Routing (pure — no bindings, no I/O) — mirrors phone_share.rs's `route()`
// classifier from the LAN-server design this Worker replaces: unit-testable
// dispatch logic kept separate from the handlers that do real I/O.
// ---------------------------------------------------------------------------

export type Route =
  | { kind: 'preflight' }
  | { kind: 'identity' }
  | { kind: 'put' }
  | { kind: 'get'; token: string }
  | { kind: 'peek'; token: string }
  | { kind: 'delete'; token: string }
  | { kind: 'not-found' }

/** Strips one leading `RELAY_PREFIX` segment: `/relay` → `/`, `/relay/x` →
 *  `/x`; anything else (including `/relayx`) is returned unchanged. Applied
 *  exactly once — `/relay/relay/drop` is NOT `/drop`. */
export function stripRelayPrefix(pathname: string): string {
  if (pathname === RELAY_PREFIX) return '/'
  if (pathname.startsWith(`${RELAY_PREFIX}/`)) return pathname.slice(RELAY_PREFIX.length)
  return pathname
}

export function route(method: string, rawPathname: string): Route {
  if (method === 'OPTIONS') return { kind: 'preflight' }
  const pathname = stripRelayPrefix(rawPathname)
  // The identity route answers at the prefix root with AND without the
  // trailing slash (`/relay`, `/relay/`, and bare `/`) — clients always end
  // the identity URL with `/` (design §2: a bare `/relay` on nginx falls
  // into `try_files` unless the 308 in `deploy/hew.d/relay.conf` is present),
  // but the server is lenient so a hand-typed check works too.
  if (pathname === '/' && method === 'GET') return { kind: 'identity' }
  if (pathname === '/drop' && method === 'PUT') return { kind: 'put' }
  const match = /^\/drop\/([^/]+)$/.exec(pathname)
  if (match) {
    const token = match[1]
    if (method === 'GET') return { kind: 'get', token }
    if (method === 'HEAD') return { kind: 'peek', token }
    if (method === 'DELETE') return { kind: 'delete', token }
  }
  return { kind: 'not-found' }
}

// ---------------------------------------------------------------------------
// Request handlers — one per route kind, each a plain function of
// (request-ish input, env) so `handlers.test.ts` can call them directly
// against a fake DO namespace.
// ---------------------------------------------------------------------------

function jsonResponse(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  })
}

/** `GET /` (or `GET /relay`, `GET /relay/`) — the identity route: what a
 *  desktop's *Test connection* hits, and where a new desktop reads the real
 *  size cap / TTL instead of its mirrored constants. `auth` tells the client
 *  whether PUT wants a bearer key (§4). `cache-control: no-store` so a proxy
 *  never serves a stale answer after the admin flips a setting. */
export function handleIdentity(env: DropEnv): Response {
  return new Response(
    JSON.stringify({
      service: 'hew-relay',
      contract: CONTRACT_VERSION,
      maxBytes: MAX_BYTES,
      ttlMs: TTL_MS,
      auth: resolveUploadKey(env) === null ? 'none' : 'bearer',
    }),
    { status: 200, headers: { 'content-type': 'application/json', 'cache-control': 'no-store' } },
  )
}

/** `PUT /drop` — the request body is opaque ciphertext (the desktop side
 *  never sends this Worker anything else); chunked (`chunkBytes`) and
 *  stored under a fresh token's `ShareDrop` DO, which stamps its own
 *  `uploadedAt` and arms the TTL alarm (`shareDrop.ts`'s `store`). Rejects
 *  an empty body (400) or one over `MAX_BYTES` (413) — checked against
 *  `Content-Length` first (fails fast, before reading anything) and again
 *  against the actually-read byte count (a missing or lying
 *  `Content-Length` can't be trusted alone). */
/** Sentinel returned by `readBodyCapped` when the body exceeds the cap. */
const TOO_LARGE = Symbol('too-large')

/** Read a request body chunk by chunk, aborting as soon as the running total
 *  exceeds `maxBytes` so an oversized or unlabelled (chunked) upload never
 *  buffers past the cap in Worker memory. Returns the assembled bytes, or
 *  `TOO_LARGE`. A body-less request yields an empty array (the caller's
 *  empty-body 400 handles it). */
async function readBodyCapped(
  request: Request,
  maxBytes: number,
): Promise<Uint8Array | typeof TOO_LARGE> {
  const reader = request.body?.getReader()
  if (!reader) return new Uint8Array(0)
  const chunks: Uint8Array[] = []
  let total = 0
  for (;;) {
    const { done, value } = await reader.read()
    if (done) break
    if (value) {
      total += value.byteLength
      if (total > maxBytes) {
        await reader.cancel()
        return TOO_LARGE
      }
      chunks.push(value)
    }
  }
  const out = new Uint8Array(total)
  let offset = 0
  for (const chunk of chunks) {
    out.set(chunk, offset)
    offset += chunk.byteLength
  }
  return out
}

export async function handlePutDrop(request: Request, env: DropEnv): Promise<Response> {
  // Auth first, before the size checks and long before the body is read: an
  // unauthorized upload should cost the relay nothing but a header read.
  if (!uploadAuthorized(request, env)) {
    return jsonResponse(401, { error: 'unauthorized' })
  }

  const contentLength = request.headers.get('content-length')
  if (contentLength !== null) {
    const declared = Number(contentLength)
    if (Number.isFinite(declared) && declared > MAX_BYTES) {
      return jsonResponse(413, { error: 'payload too large' })
    }
  }

  // Stream the body with a running cap instead of buffering it whole: a
  // request with no (or a lying) Content-Length — trivially produced with
  // Transfer-Encoding: chunked — would otherwise sail past the pre-check
  // above and have `arrayBuffer()` buffer the entire payload before any size
  // check ran. This mirrors the Rust relay's frame-by-frame cap.
  const capped = await readBodyCapped(request, MAX_BYTES)
  if (capped === TOO_LARGE) {
    return jsonResponse(413, { error: 'payload too large' })
  }
  if (capped.byteLength === 0) {
    return jsonResponse(400, { error: 'empty body' })
  }
  const body = capped

  const token = generateToken()
  const chunks = chunkBytes(body, CHUNK_BYTES)
  const id = env.SHARE_DROP.idFromName(token)
  const stub = env.SHARE_DROP.get(id)
  // Batched: one RPC call may not carry more than 32 MiB of arguments, and a
  // maximum-size drop is exactly that (`dropStore.ts`'s protocol note). The
  // first batch rides `store` (which declares the full count, so the DO can
  // tell "still uploading" from "complete"); the rest ride `append`.
  await stub.store(DROP_NAME, chunks.length, body.byteLength, rpcBatch(chunks, 0))
  for (let from = RPC_BATCH_CHUNKS; from < chunks.length; from += RPC_BATCH_CHUNKS) {
    await stub.append(rpcBatch(chunks, from))
  }
  return jsonResponse(200, { token })
}

/** One RPC batch of `chunks` starting at `from`, each chunk COPIED into its
 *  own buffer. `chunkBytes` returns `subarray` views over the single upload
 *  buffer, and structured clone (which is what RPC serialization is) of a
 *  typed-array view ships its ENTIRE backing buffer — so a batch of eight
 *  1.9 MB views of a 32 MiB body would still serialize as 32 MiB and hit the
 *  cap. Copying at the boundary is what makes the batching real. */
function rpcBatch(chunks: Uint8Array[], from: number): Uint8Array[] {
  return chunks.slice(from, from + RPC_BATCH_CHUNKS).map((chunk) => chunk.slice())
}

/** `GET /drop/<token>` — one-shot: `ShareDrop.consume()` CLAIMS the drop in
 *  one synchronous burst inside the single-threaded DO, so this is genuinely
 *  atomic against a concurrent second GET for the same token (see
 *  `shareDrop.ts`'s class doc) — unlike the old R2-backed version, which only
 *  ever narrowed that race. The bytes then come back through `take` in
 *  RPC-sized batches (each of which deletes what it returns; the last one
 *  wipes the drop) — a single return value may not carry a whole 32 MiB
 *  drop (`dropStore.ts`'s protocol note). `consume()` folds "never existed",
 *  "already consumed", "still uploading", and "expired" into the same `null`
 *  — the receiving phone gets no signal to distinguish those cases, which is
 *  the point (no information leak either way). */
export async function handleGetDrop(token: string, env: DropEnv): Promise<Response> {
  if (!isValidToken(token)) {
    return jsonResponse(400, { error: 'invalid token' })
  }

  const id = env.SHARE_DROP.idFromName(token)
  const stub = env.SHARE_DROP.get(id)
  const head = await stub.consume()

  if (head === null) {
    return jsonResponse(404, { error: 'not found' })
  }

  const bytes = new Uint8Array(head.totalBytes)
  let offset = 0
  for (let from = 0; from < head.chunkCount; from += RPC_BATCH_CHUNKS) {
    for (const chunk of await stub.take(from, RPC_BATCH_CHUNKS)) {
      bytes.set(chunk, offset)
      offset += chunk.byteLength
    }
  }
  if (offset !== head.totalBytes) {
    // The rows did not add up to the declared size: the drop was destroyed
    // between the claim and the last `take` — the TTL alarm fired, or the
    // desktop's dialog-close DELETE landed mid-download (`take` answers an
    // empty array for a vanished drop rather than throwing). From the
    // phone's side that is exactly "gone": the same 404 as any other
    // consumed/expired token, never truncated ciphertext (which would only
    // fail the auth-tag check with a misleading message).
    return jsonResponse(404, { error: 'not found' })
  }

  return new Response(bytes, {
    status: 200,
    headers: { 'content-type': 'application/octet-stream', 'cache-control': 'no-store' },
  })
}

/** `HEAD /drop/<token>` — a non-consuming existence check: does this token
 *  still have an unconsumed, unexpired drop sitting behind it? Backs the
 *  desktop dialog's pickup-detection poll (`app/src/panels/PhoneShareDialog.tsx`):
 *  it needs to notice the phone has fetched the drop (or that it expired)
 *  WITHOUT itself being the request that consumes it — a GET here would
 *  race the phone's own GET and could win, destroying the drop the phone
 *  was about to read. `ShareDrop.peek()` never calls `destroy()`/`deleteAll()`
 *  (see `dropStore.ts`'s `peek` doc), so any number of HEADs are harmless to
 *  a drop still waiting to be consumed. Body is always empty either way —
 *  only the status carries the answer, same "no information leak" posture
 *  as `handleGetDrop` folding "never existed"/"already consumed"/"expired"
 *  into one 404. */
export async function handlePeek(token: string, env: DropEnv): Promise<Response> {
  if (!isValidToken(token)) {
    return new Response(null, { status: 400 })
  }

  const id = env.SHARE_DROP.idFromName(token)
  const stub = env.SHARE_DROP.get(id)
  const { exists } = await stub.peek()

  return new Response(null, { status: exists ? 200 : 404 })
}

/** `DELETE /drop/<token>` — the desktop dialog's best-effort close-time
 *  invalidation. Always 204, whether or not the token existed (README.md:
 *  "DELETE deletes if present, 204 always") — the caller has already
 *  closed the dialog by the time this fires; there is nothing useful a
 *  different status code could tell it. `ShareDrop.destroy()` is idempotent,
 *  so calling it against an unpopulated or already-consumed DO is a
 *  harmless no-op. */
export async function handleDeleteDrop(token: string, env: DropEnv): Promise<Response> {
  if (isValidToken(token)) {
    const id = env.SHARE_DROP.idFromName(token)
    const stub = env.SHARE_DROP.get(id)
    await stub.destroy()
  }
  return new Response(null, { status: 204 })
}

// ---------------------------------------------------------------------------
// Top-level dispatch — `index.ts`'s `fetch` calls straight into this.
// ---------------------------------------------------------------------------

export async function handleRequest(request: Request, env: DropEnv): Promise<Response> {
  const url = new URL(request.url)
  const origin = request.headers.get('origin')
  const allowed = resolveAllowedOrigins(env)
  const matched = route(request.method, url.pathname)

  switch (matched.kind) {
    case 'preflight':
      return preflightResponse(origin, allowed)
    case 'identity':
      return withCors(handleIdentity(env), origin, allowed)
    case 'put':
      return withCors(await handlePutDrop(request, env), origin, allowed)
    case 'get':
      return withCors(await handleGetDrop(matched.token, env), origin, allowed)
    case 'peek':
      return withCors(await handlePeek(matched.token, env), origin, allowed)
    case 'delete':
      return withCors(await handleDeleteDrop(matched.token, env), origin, allowed)
    case 'not-found':
      return withCors(new Response(null, { status: 404 }), origin, allowed)
  }
}

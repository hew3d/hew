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

/** Upload size ceiling — PUT /drop rejects anything larger with a 413. */
export const MAX_BYTES = 32 * 1024 * 1024

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
      'access-control-allow-headers': 'content-type',
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
  | { kind: 'put' }
  | { kind: 'get'; token: string }
  | { kind: 'peek'; token: string }
  | { kind: 'delete'; token: string }
  | { kind: 'not-found' }

export function route(method: string, pathname: string): Route {
  if (method === 'OPTIONS') return { kind: 'preflight' }
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

/** `PUT /drop` — the request body is opaque ciphertext (the desktop side
 *  never sends this Worker anything else); chunked (`chunkBytes`) and
 *  stored under a fresh token's `ShareDrop` DO, which stamps its own
 *  `uploadedAt` and arms the TTL alarm (`shareDrop.ts`'s `store`). Rejects
 *  an empty body (400) or one over `MAX_BYTES` (413) — checked against
 *  `Content-Length` first (fails fast, before reading anything) and again
 *  against the actually-read byte count (a missing or lying
 *  `Content-Length` can't be trusted alone). */
export async function handlePutDrop(request: Request, env: DropEnv): Promise<Response> {
  const contentLength = request.headers.get('content-length')
  if (contentLength !== null) {
    const declared = Number(contentLength)
    if (Number.isFinite(declared) && declared > MAX_BYTES) {
      return jsonResponse(413, { error: 'payload too large' })
    }
  }

  const body = await request.arrayBuffer()
  if (body.byteLength === 0) {
    return jsonResponse(400, { error: 'empty body' })
  }
  if (body.byteLength > MAX_BYTES) {
    return jsonResponse(413, { error: 'payload too large' })
  }

  const token = generateToken()
  const chunks = chunkBytes(new Uint8Array(body), CHUNK_BYTES)
  const id = env.SHARE_DROP.idFromName(token)
  const stub = env.SHARE_DROP.get(id)
  await stub.store(DROP_NAME, chunks)
  return jsonResponse(200, { token })
}

/** `GET /drop/<token>` — one-shot: `ShareDrop.consume()` reads and wipes
 *  the drop's storage as a single synchronous-then-gated sequence, so this
 *  is genuinely atomic against a concurrent second GET for the same token
 *  (see `shareDrop.ts`'s class doc) — unlike the old R2-backed version,
 *  which only ever narrowed that race. `consume()` folds "never existed",
 *  "already consumed", and "expired" into the same `null` — the receiving
 *  phone gets no signal to distinguish those cases, which is the point (no
 *  information leak either way). */
export async function handleGetDrop(token: string, env: DropEnv): Promise<Response> {
  if (!isValidToken(token)) {
    return jsonResponse(400, { error: 'invalid token' })
  }

  const id = env.SHARE_DROP.idFromName(token)
  const stub = env.SHARE_DROP.get(id)
  const drop = await stub.consume()

  if (drop === null) {
    return jsonResponse(404, { error: 'not found' })
  }

  return new Response(drop.bytes, {
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

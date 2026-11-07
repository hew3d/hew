/**
 * Upload tokens: `UPLOAD_TOKEN_BYTES` (32) random bytes, base64url-encoded
 * (43 characters, no padding) — docs/design/report-bug.md §8. A token
 * authorizes only its own upload's pieces and commit; this Worker never
 * stores the raw token, only its SHA-256 hex digest (`hashToken`), and
 * every comparison against a presented token's hash is constant-time.
 *
 * Not `crypto.subtle.timingSafeEqual`: that's a Cloudflare-only extension
 * absent from Node (see share-relay's `handlers.ts` for the same note),
 * and this module needs to run under bare `node --test`.
 */

import { UPLOAD_TOKEN_BYTES, FOLLOW_SECRET_BYTES } from './constants.ts'

/** Base64url (RFC 4648 §5), no padding. */
function toBase64Url(bytes: Uint8Array): string {
  let binary = ''
  for (const b of bytes) binary += String.fromCharCode(b)
  return btoa(binary).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '')
}

function randomBase64Url(byteCount: number): string {
  const bytes = new Uint8Array(byteCount)
  crypto.getRandomValues(bytes)
  return toBase64Url(bytes)
}

/** A fresh upload token — 32 random bytes, base64url. */
export function generateUploadToken(): string {
  return randomBase64Url(UPLOAD_TOKEN_BYTES)
}

/** A fresh follow secret (docs/design/follow-a-report.md §4.1) — the same
 *  shape as an upload token, from the same generator. Stored and compared
 *  only as `hashToken`'s digest, exactly like the token. */
export function generateFollowSecret(): string {
  return randomBase64Url(FOLLOW_SECRET_BYTES)
}

/** The hex SHA-256 digest of `token` — what's actually stored and compared,
 *  never the raw token. */
export async function hashToken(token: string): Promise<string> {
  const digest = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(token))
  return [...new Uint8Array(digest)].map((b) => b.toString(16).padStart(2, '0')).join('')
}

/** Constant-time string equality over UTF-8 bytes — the loop always runs to
 *  the longer length and folds every byte (and the length difference) into
 *  one accumulator, so a mismatch takes the same time wherever it occurs.
 *  Used to compare a presented token's hash against the stored one. */
export function constantTimeEqual(a: string, b: string): boolean {
  const enc = new TextEncoder()
  const ab = enc.encode(a)
  const bb = enc.encode(b)
  let diff = ab.length ^ bb.length
  const n = Math.max(ab.length, bb.length)
  for (let i = 0; i < n; i++) diff |= (ab[i] ?? 0) ^ (bb[i] ?? 0)
  return diff === 0
}

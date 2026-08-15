/**
 * shareCrypto — pure AES-256-GCM helpers for the "Open on Phone" E2E
 * dead-drop handoff (`PhoneShareDialog.tsx` uploads; the phone-side
 * receive path, wave B, decrypts with these same functions). The server
 * (`workers/share-relay`) only ever sees the bytes `encrypt` produces and
 * never sees the key — see that Worker's README.md "Security model" for
 * the full argument.
 *
 * ## Wire format
 *
 * `encrypt`'s output, and therefore the exact bytes PUT to
 * `workers/share-relay`'s `/drop`, is:
 *
 *     [ 12-byte IV ][ AES-256-GCM ciphertext, auth tag appended ]
 *
 * The 16-byte GCM authentication tag is already part of WebCrypto's
 * `encrypt` output (its documented behavior, not something this module
 * adds) — `decrypt` relies on that same convention, so the two halves of
 * this format are exactly what SubtleCrypto natively expects on either
 * side, plus the IV prepended so a single opaque blob carries everything
 * needed to invert it (short of the key itself).
 *
 * ## URL fragment grammar (built by `PhoneShareDialog.tsx`, consumed by
 * wave B's receive path)
 *
 *     #recv=<token>.<base64url-key>.<urlencoded-name>
 *
 * - `token` — the share-relay-issued drop id verbatim (base64url, 22
 *   chars, never contains `.`).
 * - `base64url-key` — `toBase64Url(rawKey)`, the raw 32-byte AES key
 *   (base64url, 43 chars, never contains `.`).
 * - `urlencoded-name` — `encodeURIComponent(displayName)`. Unlike the
 *   first two fields, `encodeURIComponent` does NOT escape `.` (it's in
 *   its unreserved set), so a document name containing a literal `.`
 *   (e.g. "v2.hew") can and will appear in this segment. A parser MUST
 *   split on only the first two `.` characters — token, then key — and
 *   treat everything after the second `.` as the (still URL-encoded) name,
 *   never split the name segment itself on `.`.
 */

const IV_BYTES = 12
const KEY_BYTES = 32 // AES-256

/** Generates a fresh random 256-bit AES key as raw bytes (not a
 *  `CryptoKey` — `encrypt`/`decrypt` both import it internally, and the
 *  caller needs the raw bytes anyway to embed in the URL fragment). */
export function generateKey(): Uint8Array {
  return crypto.getRandomValues(new Uint8Array(KEY_BYTES))
}

async function importAesKey(rawKey: Uint8Array, usage: KeyUsage): Promise<CryptoKey> {
  return crypto.subtle.importKey('raw', rawKey as BufferSource, 'AES-GCM', false, [usage])
}

/** Encrypts `plaintext` under `rawKey`, returning `IV || ciphertext` (see
 *  the module doc's wire format). A fresh random IV is generated per call —
 *  callers must never reuse a key across encryptions with a caller-chosen
 *  IV, but this module never gives them the chance to: the IV is always
 *  freshly randomized here. */
export async function encrypt(rawKey: Uint8Array, plaintext: Uint8Array): Promise<Uint8Array> {
  const key = await importAesKey(rawKey, 'encrypt')
  const iv = crypto.getRandomValues(new Uint8Array(IV_BYTES))
  const ciphertext = new Uint8Array(
    await crypto.subtle.encrypt({ name: 'AES-GCM', iv }, key, plaintext as BufferSource),
  )
  const out = new Uint8Array(IV_BYTES + ciphertext.length)
  out.set(iv, 0)
  out.set(ciphertext, IV_BYTES)
  return out
}

/** Inverts `encrypt`: splits `payload` back into its leading IV and the
 *  AES-GCM ciphertext, and decrypts. Throws (SubtleCrypto's own
 *  `OperationError`) if the auth tag doesn't verify — a wrong key or
 *  corrupted/truncated payload is a hard failure here, never a silent
 *  garbage result. */
export async function decrypt(rawKey: Uint8Array, payload: Uint8Array): Promise<Uint8Array> {
  if (payload.length < IV_BYTES) {
    throw new Error('payload is too short to contain an IV')
  }
  const key = await importAesKey(rawKey, 'decrypt')
  const iv = payload.slice(0, IV_BYTES)
  const ciphertext = payload.slice(IV_BYTES)
  const plaintext = await crypto.subtle.decrypt(
    { name: 'AES-GCM', iv: iv as BufferSource },
    key,
    ciphertext as BufferSource,
  )
  return new Uint8Array(plaintext)
}

/** Base64url (RFC 4648 §5), no padding — used for the AES key riding the
 *  URL fragment (and, server-side, the drop token itself; see
 *  `workers/share-relay/src/handlers.ts`'s identical `toBase64Url`). */
export function toBase64Url(bytes: Uint8Array): string {
  let binary = ''
  for (const b of bytes) binary += String.fromCharCode(b)
  return btoa(binary).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '')
}

/** Inverts `toBase64Url`. Throws on input containing characters outside
 *  the base64url alphabet (via `atob` rejecting the re-padded string) —
 *  wave B's receive path should treat that as "malformed fragment", the
 *  same bucket as a missing/short token. */
export function fromBase64Url(value: string): Uint8Array {
  const padded = value.replace(/-/g, '+').replace(/_/g, '/')
  const padLength = (4 - (padded.length % 4)) % 4
  const binary = atob(padded + '='.repeat(padLength))
  const bytes = new Uint8Array(binary.length)
  for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i)
  return bytes
}

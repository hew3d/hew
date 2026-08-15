/**
 * shareCrypto.test.ts — round-trip + wire-format coverage for the "Open on
 * Phone" E2E dead-drop crypto helpers. Runs on vitest's plain `node`
 * project (vitest.config.ts) — Node's global `crypto.subtle`/
 * `crypto.getRandomValues` cover everything here with no jsdom needed,
 * same as `itemFiles.ts`'s existing `sha256Hex` (browser SubtleCrypto, no
 * wasm, no DOM).
 */

import { describe, expect, it } from 'vitest'
import { decrypt, encrypt, fromBase64Url, generateKey, toBase64Url } from './shareCrypto'

describe('generateKey', () => {
  it('returns 32 raw bytes (AES-256)', () => {
    const key = generateKey()
    expect(key).toBeInstanceOf(Uint8Array)
    expect(key.byteLength).toBe(32)
  })

  it('returns a fresh key on every call', () => {
    const a = generateKey()
    const b = generateKey()
    expect(a).not.toEqual(b)
  })
})

describe('encrypt / decrypt round trip', () => {
  it('recovers the exact original plaintext', async () => {
    const key = generateKey()
    const plaintext = new TextEncoder().encode('a small hew document, or pretending to be one')
    const ciphertext = await encrypt(key, plaintext)
    const recovered = await decrypt(key, ciphertext)
    expect(recovered).toEqual(plaintext)
  })

  it('round-trips empty bytes', async () => {
    const key = generateKey()
    const recovered = await decrypt(key, await encrypt(key, new Uint8Array(0)))
    expect(recovered).toEqual(new Uint8Array(0))
  })

  it('round-trips a larger binary payload with every byte value present', async () => {
    const key = generateKey()
    const plaintext = new Uint8Array(4096)
    for (let i = 0; i < plaintext.length; i++) plaintext[i] = i % 256
    const recovered = await decrypt(key, await encrypt(key, plaintext))
    expect(recovered).toEqual(plaintext)
  })

  it('prepends a 12-byte IV ahead of the ciphertext (wire format)', async () => {
    const key = generateKey()
    const plaintext = new Uint8Array([1, 2, 3])
    const encrypted = await encrypt(key, plaintext)
    // AES-GCM ciphertext = plaintext length + 16-byte auth tag; total
    // output = 12-byte IV + that.
    expect(encrypted.byteLength).toBe(12 + plaintext.byteLength + 16)
  })

  it('produces a different IV (and therefore different ciphertext) on every call', async () => {
    const key = generateKey()
    const plaintext = new Uint8Array([9, 9, 9])
    const a = await encrypt(key, plaintext)
    const b = await encrypt(key, plaintext)
    expect(a).not.toEqual(b)
    expect(a.slice(0, 12)).not.toEqual(b.slice(0, 12)) // the IVs themselves differ
  })

  it('fails to decrypt under the wrong key', async () => {
    const key = generateKey()
    const wrongKey = generateKey()
    const encrypted = await encrypt(key, new Uint8Array([1, 2, 3]))
    await expect(decrypt(wrongKey, encrypted)).rejects.toThrow()
  })

  it('fails to decrypt a corrupted (bit-flipped) payload', async () => {
    const key = generateKey()
    const encrypted = await encrypt(key, new TextEncoder().encode('tamper-evident'))
    const corrupted = encrypted.slice()
    corrupted[corrupted.length - 1] ^= 0xff // flip a byte inside the auth tag
    await expect(decrypt(key, corrupted)).rejects.toThrow()
  })

  it('rejects a payload too short to contain an IV', async () => {
    const key = generateKey()
    await expect(decrypt(key, new Uint8Array(4))).rejects.toThrow(/too short/i)
  })
})

describe('base64url encode/decode', () => {
  it('round-trips arbitrary bytes', () => {
    const bytes = generateKey() // any 32 random bytes will do
    expect(fromBase64Url(toBase64Url(bytes))).toEqual(bytes)
  })

  it('round-trips every byte value (0-255)', () => {
    const bytes = new Uint8Array(256)
    for (let i = 0; i < 256; i++) bytes[i] = i
    expect(fromBase64Url(toBase64Url(bytes))).toEqual(bytes)
  })

  it('produces no padding and no +/ characters — safe to embed unescaped in a URL fragment', () => {
    const bytes = new Uint8Array(32).fill(255)
    const encoded = toBase64Url(bytes)
    expect(encoded).not.toMatch(/[+/=]/)
  })

  it('never produces a literal "." — the fragment grammar reserves it as a field delimiter', () => {
    // Exercise a wide spread of byte patterns since "." only ever comes
    // from base64's own alphabet, which toBase64Url never uses (only
    // A-Za-z0-9-_ are possible outputs).
    for (let seed = 0; seed < 50; seed++) {
      const bytes = new Uint8Array(32)
      for (let i = 0; i < bytes.length; i++) bytes[i] = (seed * 37 + i * 91) % 256
      expect(toBase64Url(bytes)).not.toContain('.')
    }
  })

  it('a 32-byte key encodes to 43 base64url characters (no padding)', () => {
    expect(toBase64Url(new Uint8Array(32)).length).toBe(43)
  })
})

// @vitest-environment jsdom
import { readFileSync } from 'node:fs'
import { createHash } from 'node:crypto'
import { join } from 'node:path'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { requiresPreview } from '../../test/requiresPreview'
import { REALM_CSP as PROBE_REALM_CSP } from '../probe/realm'
import { BOOT_HASH_LITERAL as PROBE_HASH } from '../probe/bootProbe'
import { BOOT_HASH, BOOT_SOURCE, REALM_CSP, createRealmFrame, makeNonce, realmSrcdoc } from './frame'

const NONCE = 'abcdef0123456789abcdef0123456789'

describe('realmSrcdoc', () => {
  beforeEach((ctx) => requiresPreview(ctx, 'plugins'))
  const doc = realmSrcdoc(NONCE)

  it('holds exactly one script, the committed boot script byte for byte', () => {
    expect(doc.match(/<script/g)).toHaveLength(1)
    const start = doc.indexOf('<script>') + '<script>'.length
    const script = doc.slice(start, doc.lastIndexOf('</script>'))
    expect(script).toBe(BOOT_SOURCE)
    const file = readFileSync(join(process.cwd(), 'src/plugins/boot/boot-1.js'))
    expect(Buffer.from(script, 'utf8').equals(file)).toBe(true)
    expect(createHash('sha256').update(file).digest('base64')).toBe(BOOT_HASH)
  })

  it("carries the probe's realm policy verbatim, with this boot script's hash", () => {
    expect(REALM_CSP).toBe(PROBE_REALM_CSP.replace(PROBE_HASH, BOOT_HASH))
    expect(doc.indexOf(`<meta http-equiv="Content-Security-Policy" content="${REALM_CSP}">`)).toBeLessThan(doc.indexOf('<script>'))
  })

  it('puts the nonce only in its meta element, and nothing per instance in the script', () => {
    expect(doc.split(NONCE)).toHaveLength(2)
    expect(doc).toContain(`<meta name="hew-nonce" content="${NONCE}">`)
    expect(doc).toContain('<meta name="hew-realm" content="background">')
    expect(BOOT_SOURCE).not.toMatch(/[0-9a-f]{32}/)
  })

  it('takes only a 32-digit hex nonce', () => {
    expect(() => realmSrcdoc('"><script>x</script>')).toThrow()
    expect(makeNonce()).toMatch(/^[0-9a-f]{32}$/)
    expect(makeNonce()).not.toBe(makeNonce())
  })
})

describe('createRealmFrame', () => {
  beforeEach((ctx) => requiresPreview(ctx, 'plugins'))
  afterEach(() => {
    vi.useRealTimers()
    document.body.innerHTML = ''
  })

  it('makes a frame sandboxed to scripts alone', () => {
    const mount = document.createElement('div')
    document.body.appendChild(mount)
    void createRealmFrame(mount, { handshakeMs: 10, onNavigate: () => {} }).catch(() => {})
    const frame = mount.querySelector('iframe')!
    expect(frame.getAttribute('sandbox')).toBe('allow-scripts')
    expect(frame.srcdoc).toContain(BOOT_SOURCE)
  })

  it('rejects handshake_timeout and removes the frame when it never says ready', async () => {
    vi.useFakeTimers()
    const mount = document.createElement('div')
    document.body.appendChild(mount)
    const p = createRealmFrame(mount, { handshakeMs: 5_000, onNavigate: () => {} })
    const expectation = expect(p).rejects.toThrow('handshake_timeout')
    await vi.advanceTimersByTimeAsync(5_001)
    await expectation
    expect(mount.querySelector('iframe')).toBeNull()
  })

  it("ignores a ready message from another window, another origin, or with the wrong nonce", async () => {
    vi.useFakeTimers()
    const mount = document.createElement('div')
    document.body.appendChild(mount)
    const p = createRealmFrame(mount, { handshakeMs: 1_000, onNavigate: () => {} })
    const frame = mount.querySelector('iframe')!
    const nonce = /hew-nonce" content="([0-9a-f]{32})"/.exec(frame.srcdoc)![1]
    const post = vi.spyOn(frame.contentWindow!, 'postMessage')
    // The page itself, with the right nonce.
    window.dispatchEvent(new MessageEvent('message', { data: { hew: 'ready', nonce }, source: window, origin: 'null' }))
    // The frame, with a wrong nonce.
    window.dispatchEvent(new MessageEvent('message', { data: { hew: 'ready', nonce: NONCE }, source: frame.contentWindow, origin: 'null' }))
    // The frame, with the nonce, from a real origin (it navigated somewhere).
    window.dispatchEvent(new MessageEvent('message', { data: { hew: 'ready', nonce }, source: frame.contentWindow, origin: 'https://example.com' }))
    expect(post).not.toHaveBeenCalled()
    const expectation = expect(p).rejects.toThrow('handshake_timeout')
    await vi.advanceTimersByTimeAsync(1_001)
    await expectation
  })

  it('hands the frame a control port once it says ready with its nonce', async () => {
    const mount = document.createElement('div')
    document.body.appendChild(mount)
    const p = createRealmFrame(mount, { handshakeMs: 1_000, onNavigate: () => {} })
    const frame = mount.querySelector('iframe')!
    const nonce = /hew-nonce" content="([0-9a-f]{32})"/.exec(frame.srcdoc)![1]
    const post = vi.spyOn(frame.contentWindow!, 'postMessage').mockImplementation(() => {})
    window.dispatchEvent(new MessageEvent('message', { data: { hew: 'ready', nonce }, source: frame.contentWindow, origin: 'null' }))
    const realm = await p
    expect(post).toHaveBeenCalledTimes(1)
    const [message, target, transfer] = post.mock.calls[0] as unknown as [unknown, string, Transferable[]]
    expect(message).toEqual({ hew: 'port', nonce })
    expect(target).toBe('*')
    expect(transfer).toHaveLength(1)
    realm.remove()
    expect(mount.querySelector('iframe')).toBeNull()
  })
})

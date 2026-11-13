// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from 'vitest'
import { BOOT_HASH_LITERAL, BOOT_PROBE_SOURCE } from './bootProbe'
import { REALM_CSP, createRealmFrame, probeSrcdoc } from './realm'

const NONCE = 'abcdef0123456789abcdef0123456789'

describe('probeSrcdoc', () => {
  const doc = probeSrcdoc({ nonce: NONCE, mode: 'surface' })

  it('contains exactly one script whose content is the boot source', () => {
    const scripts = doc.match(/<script>/g) ?? []
    expect(scripts).toHaveLength(1)
    const start = doc.indexOf('<script>') + '<script>'.length
    expect(doc.slice(start, doc.indexOf('</script>'))).toBe(BOOT_PROBE_SOURCE)
  })

  it('the realm meta carries the hash', () => {
    expect(REALM_CSP).toContain(`'sha256-${BOOT_HASH_LITERAL}'`)
    expect(doc).toContain(`<meta http-equiv="Content-Security-Policy" content="${REALM_CSP}">`)
    expect(REALM_CSP).not.toContain('<BOOT_HASH>')
  })

  it('the nonce appears only in the meta element', () => {
    expect(doc.split(NONCE)).toHaveLength(2)
    expect(doc).toContain(`<meta name="hew-nonce" content="${NONCE}">`)
    expect(BOOT_PROBE_SOURCE).not.toContain(NONCE)
  })
})

describe('createRealmFrame', () => {
  afterEach(() => {
    vi.restoreAllMocks()
    document.body.innerHTML = ''
  })

  it('rejects with handshake_timeout when the frame never answers', async () => {
    const fake = { postMessage: vi.fn() } as unknown as Window
    vi.spyOn(HTMLIFrameElement.prototype, 'contentWindow', 'get').mockReturnValue(fake)
    await expect(createRealmFrame('background', document.body, { handshakeMs: 30 })).rejects.toThrow(
      'handshake_timeout',
    )
  })

  it('sandboxes with allow-scripts only (surface adds allow-forms)', () => {
    const fake = { postMessage: vi.fn() } as unknown as Window
    vi.spyOn(HTMLIFrameElement.prototype, 'contentWindow', 'get').mockReturnValue(fake)
    void createRealmFrame('background', document.body, { handshakeMs: 5 }).catch(() => undefined)
    void createRealmFrame('surface', document.body, { handshakeMs: 5 }).catch(() => undefined)
    const frames = document.querySelectorAll('iframe')
    expect(frames[0].getAttribute('sandbox')).toBe('allow-scripts')
    expect(frames[1].getAttribute('sandbox')).toBe('allow-scripts allow-forms')
  })
})

import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { describe, expect, it } from 'vitest'
import { BOOT_HASH_LITERAL, BOOT_PROBE_SOURCE, bootHash } from './bootProbe'

describe('boot probe hash', () => {
  it('the runtime hash equals the pasted literal', async () => {
    expect(await bootHash()).toBe(BOOT_HASH_LITERAL)
  })

  it('the Tauri probe overlay pins the same hash', () => {
    const path = fileURLToPath(new URL('../../../../shells/tauri/src-tauri/probe.conf.json', import.meta.url))
    const conf = JSON.parse(readFileSync(path, 'utf8')) as { app: { security: { csp: string } } }
    expect(conf.app.security.csp).toContain(`'sha256-${BOOT_HASH_LITERAL}'`)
  })

  it('the boot script carries no nonce or per-instance data', () => {
    expect(BOOT_PROBE_SOURCE).not.toMatch(/[0-9a-f]{32}/)
    expect(BOOT_PROBE_SOURCE.startsWith('(() => {')).toBe(true)
    expect(BOOT_PROBE_SOURCE.endsWith('})();')).toBe(true)
  })
})

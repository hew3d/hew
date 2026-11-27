import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { beforeEach, describe, expect, it } from 'vitest'
import { requiresPreview } from '../../test/requiresPreview'
import { BOOT_HASH_LITERAL, BOOT_PROBE_SOURCE, bootHash } from './bootProbe'

describe('boot probe hash', () => {
  beforeEach((ctx) => requiresPreview(ctx, 'plugins'))

  it('the runtime hash equals the pasted literal', async () => {
    expect(await bootHash()).toBe(BOOT_HASH_LITERAL)
  })

  it('both probe policies pin the same hash', () => {
    const path = fileURLToPath(new URL('../../../../shells/csp/policies.json', import.meta.url))
    const policies = JSON.parse(readFileSync(path, 'utf8')) as Record<'web' | 'desktop', { probe: string }>
    expect(policies.web.probe).toContain(`'sha256-${BOOT_HASH_LITERAL}'`)
    expect(policies.desktop.probe).toContain(`'sha256-${BOOT_HASH_LITERAL}'`)
  })

  it('the boot script carries no nonce or per-instance data', () => {
    expect(BOOT_PROBE_SOURCE).not.toMatch(/[0-9a-f]{32}/)
    expect(BOOT_PROBE_SOURCE.startsWith('(() => {')).toBe(true)
    expect(BOOT_PROBE_SOURCE.endsWith('})();')).toBe(true)
  })
})

/**
 * The realm boot script is committed as built (`boot-1.js`) with its hash
 * (`boot-1.hash`), and that hash is pasted into the two preview policies
 * and the Plugins registry entry's markers. This pins all of them to each
 * other and to a fresh build of `sdk/plugin`.
 *
 * On drift, regenerate with:
 *
 *   pnpm --dir sdk/plugin build
 *
 * then paste the printed hash into shells/csp/policies.json (web.preview
 * and desktop.preview, as 'sha256-<hash>' in script-src) and into the
 * markers of the "plugins" entry of preview-features.json, replacing the
 * old one.
 */

import { execFileSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { readFileSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { beforeEach, describe, expect, it } from 'vitest'
import { requiresPreview } from '../../test/requiresPreview'

const HERE = dirname(fileURLToPath(import.meta.url))
const ROOT = resolve(HERE, '../../../..')
const REGEN = 'pnpm --dir sdk/plugin build'

const boot = readFileSync(join(HERE, 'boot-1.js'))
const hashFile = readFileSync(join(HERE, 'boot-1.hash'), 'utf8')
const hash = hashFile.trim()
const policies = JSON.parse(readFileSync(join(ROOT, 'shells/csp/policies.json'), 'utf8')) as Record<
  'web' | 'desktop',
  Record<'production' | 'preview' | 'probe', string>
>
const scriptSrc = (policy: string): string[] =>
  (policy.split('; ').find((d) => d.startsWith('script-src ')) ?? '').split(' ').slice(1)

describe('the realm boot script artifacts', () => {
  beforeEach((ctx) => requiresPreview(ctx, 'plugins'))

  it('boot-1.hash is the SHA-256 of boot-1.js', () => {
    expect(hashFile, `boot-1.hash is one base64 line; run ${REGEN}`).toMatch(/^[A-Za-z0-9+/]{43}=\n$/)
    expect(createHash('sha256').update(boot).digest('base64'), `stale hash; run ${REGEN}`).toBe(hash)
  })

  it('a fresh build of sdk/plugin gives the committed bytes', () => {
    const out = execFileSync(process.execPath, [join(ROOT, 'sdk/plugin/build.mjs'), '--print'], { encoding: 'utf8' })
    const fresh = JSON.parse(out.trim()) as { hash: string; bytes: number }
    expect(fresh, `the committed boot script is stale; run ${REGEN}`).toEqual({ hash, bytes: boot.byteLength })
  })

  it('both preview policies add the boot hash to script-src, and nothing else', () => {
    for (const shell of ['web', 'desktop'] as const) {
      const preview = policies[shell].preview
      const production = policies[shell].production
      expect(scriptSrc(preview), `${shell}.preview; run ${REGEN} and paste the hash`).toEqual([
        ...scriptSrc(production),
        `'sha256-${hash}'`,
      ])
      // Every other directive is production's, unchanged: no blob: script,
      // no isolation, nothing wider.
      const rest = (p: string): string[] => p.split('; ').filter((d) => !d.startsWith('script-src '))
      expect(rest(preview), shell).toEqual(rest(production))
      expect(production, `${shell}.production must not carry the boot hash`).not.toContain(hash)
    }
  })

  it('the Plugins registry entry lists the hash as a marker, so no production artifact can carry it', () => {
    const registry = JSON.parse(readFileSync(join(ROOT, 'preview-features.json'), 'utf8')) as {
      features: { id: string; loader?: { markers: string[] } }[]
    }
    const markers = registry.features.find((f) => f.id === 'plugins')?.loader?.markers ?? []
    expect(markers, `paste the new hash into preview-features.json; run ${REGEN}`).toContain(hash)
    // One boot hash at a time: an old one left behind would be dead text.
    expect(markers.filter((m) => /^[A-Za-z0-9+/]{43}=$/.test(m))).toEqual([hash])
  })

  it('is safe inside a srcdoc: printable ASCII, LF line ends, nothing that ends or comments a script', () => {
    const text = boot.toString('utf8')
    expect(/[^\x09\x0a\x20-\x7e]/.test(text)).toBe(false)
    expect(text).not.toMatch(/<\/script|<!--/i)
  })
})

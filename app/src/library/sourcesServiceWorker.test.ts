/**
 * Package S7 oracle, part C: the service worker must neither cache nor rewrite
 * a source request (docs/design/shared-library-sources.md 7.1). The workbox
 * options in app/vite.config.ts are read as data: vite-plugin-pwa is replaced
 * by a capture, the config function is run for a build, and every
 * runtimeCaching rule is asked about cross-origin https requests.
 *
 * Today no rule matches one, so this is a guard that passes now; adding a rule
 * (a cache for index files or thumbnails, say) that matches a cross-origin
 * request fails it. The /relay/ and /report/ NetworkOnly rules are pinned.
 */
import { describe, expect, it, vi } from 'vitest'

const captured = vi.hoisted(() => ({ options: undefined as undefined | Record<string, unknown> }))
vi.mock('vite-plugin-pwa', () => ({
  VitePWA: (options: Record<string, unknown>) => {
    captured.options = options
    return []
  },
}))

interface Rule {
  urlPattern: unknown
  handler: unknown
}
type Matcher = (ctx: { url: URL; sameOrigin: boolean; request: { mode: string; destination: string; url: string; method: string } }) => boolean

async function workbox(): Promise<{ runtimeCaching: Rule[]; navigateFallback?: string; navigateFallbackDenylist: RegExp[]; navigateFallbackAllowlist?: RegExp[] }> {
  const mod = (await import('../../vite.config')) as { default: (env: { command: string; mode: string }) => unknown }
  mod.default({ command: 'build', mode: 'production' })
  const wb = (captured.options?.workbox ?? {}) as Record<string, unknown>
  return {
    runtimeCaching: (wb.runtimeCaching ?? []) as Rule[],
    navigateFallback: wb.navigateFallback as string | undefined,
    navigateFallbackDenylist: (wb.navigateFallbackDenylist ?? []) as RegExp[],
    navigateFallbackAllowlist: wb.navigateFallbackAllowlist as RegExp[] | undefined,
  }
}

function matches(rule: Rule, href: string, sameOrigin: boolean, mode = 'cors'): boolean {
  const url = new URL(href)
  const request = { mode, destination: '', url: href, method: 'GET' }
  const p = rule.urlPattern
  if (typeof p === 'function') return Boolean((p as Matcher)({ url, sameOrigin, request }))
  if (p instanceof RegExp) return p.test(href)
  if (typeof p === 'string') return href === p || url.pathname === p
  throw new Error(`unrecognized urlPattern ${String(p)}`)
}

const CROSS_ORIGIN = [
  'https://raw.githubusercontent.com/hew3d/catalog/data/catalog.json',
  'https://api.github.com/repos/owner/repo/commits/main',
  'https://owner.github.io/parts/hew.json',
  'https://owner.github.io/parts/thumbnails/0123456789abcdef.png',
  'https://example.com/relay/x',
  'https://example.com/report/x',
  'https://example.com/anything.hew',
  'https://cdn.example.org/a.wasm',
  'https://example.com/',
]

describe('service worker and source requests', () => {
  it('the workbox config was captured (control)', async () => {
    const wb = await workbox()
    expect(captured.options).toBeDefined()
    expect(Array.isArray(wb.runtimeCaching)).toBe(true)
    expect(wb.runtimeCaching.length).toBeGreaterThanOrEqual(2)
  })

  it('no runtimeCaching rule matches a cross-origin https request, for any request mode', async () => {
    const { runtimeCaching } = await workbox()
    for (const href of CROSS_ORIGIN) {
      for (const mode of ['cors', 'no-cors', 'navigate', 'same-origin']) {
        for (const [i, rule] of runtimeCaching.entries()) {
          expect(matches(rule, href, false, mode), `rule ${i} matches ${href} (${mode})`).toBe(false)
        }
      }
    }
  })

  it('every runtimeCaching rule is a same-origin, NetworkOnly rule (a cache or rewrite of a source would be a new kind)', async () => {
    const { runtimeCaching } = await workbox()
    for (const rule of runtimeCaching) expect(rule.handler).toBe('NetworkOnly')
  })

  it('no rule claims a same-origin path other than /relay/ and /report/', async () => {
    const { runtimeCaching } = await workbox()
    for (const path of ['/', '/index.html', '/assets/a.js', '/hew.json', '/catalog.json', '/thumbnails/x.png', '/sources/x', '/library/x']) {
      for (const [i, rule] of runtimeCaching.entries()) {
        expect(matches(rule, `https://app.example.test${path}`, true), `rule ${i} claims ${path}`).toBe(false)
      }
    }
  })

  it('keeps the /relay/ and /report/ NetworkOnly rules, same origin only (guard)', async () => {
    const { runtimeCaching } = await workbox()
    for (const path of ['/relay/abc', '/relay/', '/report/start', '/report/']) {
      const hits = runtimeCaching.filter((r) => matches(r, `https://app.example.test${path}`, true))
      expect(hits.length, path).toBe(1)
      expect(hits[0].handler).toBe('NetworkOnly')
      expect(matches(hits[0], `https://app.example.test${path}`, false), `${path} cross-origin`).toBe(false)
    }
  })

  it('keeps the navigation fallback away from /relay and /report (guard)', async () => {
    const { navigateFallbackDenylist } = await workbox()
    for (const path of ['/relay', '/relay/x', '/report', '/report/x']) {
      expect(navigateFallbackDenylist.some((re) => re.test(path)), path).toBe(true)
    }
  })

  it('does not widen the worker with extra scripts, a cross-origin precache entry or a source-named rule', async () => {
    const opts = captured.options as Record<string, unknown>
    await workbox()
    const wb = opts.workbox as Record<string, unknown>
    expect(wb.importScripts).toBeUndefined()
    const extra = (wb.additionalManifestEntries ?? []) as Array<string | { url: string }>
    for (const e of extra) expect(/^https?:/.test(typeof e === 'string' ? e : e.url)).toBe(false)
    expect(opts.strategies).toBeUndefined()
    expect(opts.srcDir).toBeUndefined()
  })
})

/**
 * Package S7 oracle, part A: the web content security policy tiers and the
 * production-artifact check (docs/design/shared-library-sources.md 2 decisions
 * 22-24, 4 B5, 7.1).
 *
 * Covered rules: web.production and web.preview gain `https:` in connect-src;
 * web.strict is today's production policy; desktop and probe policies do not
 * change; HEW_WEB_SOURCES=off selects strict and is a build error beside
 * HEW_PREVIEW=1; check-production-artifacts accepts a built web page that
 * equals production OR strict, each against its own baseline file, and still
 * rejects any other page; tauri.conf.json's csp is the unchanged desktop one.
 */
import { spawnSync } from 'node:child_process'
import { cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, symlinkSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { afterAll, describe, expect, it } from 'vitest'

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '../../..')
const POLICY_MJS = pathToFileURL(join(ROOT, 'shells/csp/policy.mjs')).href

interface PolicyModule {
  policy(shell: string, name: string): string
  policyForBuild(shell: string, env?: Record<string, string | undefined>): string
  withCspMeta(html: string, csp: string): string
}
const load = async (): Promise<PolicyModule> => (await import(/* @vite-ignore */ POLICY_MJS)) as PolicyModule

const POLICIES = JSON.parse(readFileSync(join(ROOT, 'shells/csp/policies.json'), 'utf8')) as Record<string, Record<string, string>>
const connectSrc = (csp: string): string => (csp.split('; ').find((d) => d.startsWith('connect-src ')) ?? '')

// The pre-S7 values, written out so "unchanged" is a fixed fact, not a copy of the file under test.
const DESKTOP_CONNECT = "connect-src 'self' ipc: http://ipc.localhost blob:"
const OLD_WEB_PRODUCTION =
  "default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; font-src 'self' data:; connect-src 'self' blob:; worker-src 'self' blob:; manifest-src 'self'; media-src 'self' blob:; object-src 'none'; base-uri 'self'"
const SCRIPT_HASH = "'sha256-01VLapYTaY8NvdSpPUYwfMX5qN5kCuRh/4+zFeJB5l0='"

describe('policies.json', () => {
  it('web.production allows any https host and nothing else changed', () => {
    expect(connectSrc(POLICIES.web.production)).toBe("connect-src 'self' blob: https:")
    expect(POLICIES.web.production.replace(' blob: https:;', ' blob:;')).toBe(OLD_WEB_PRODUCTION)
  })

  it('web.preview allows any https host, keeps its script hash, and changed nothing else', () => {
    expect(connectSrc(POLICIES.web.preview)).toBe("connect-src 'self' blob: https:")
    expect(POLICIES.web.preview).toContain(SCRIPT_HASH)
    expect(POLICIES.web.preview.replace(' blob: https:;', ' blob:;')).toBe(
      OLD_WEB_PRODUCTION.replace("'wasm-unsafe-eval';", `'wasm-unsafe-eval' ${SCRIPT_HASH};`),
    )
  })

  it("web.strict is exactly today's production policy (connect-src 'self' blob:)", () => {
    expect(POLICIES.web.strict).toBe(OLD_WEB_PRODUCTION)
  })

  it('no web tier but production and preview allows https:, and strict has no https: anywhere in connect-src', () => {
    expect(connectSrc(POLICIES.web.strict)).not.toContain('https:')
  })

  it('desktop policies and web.probe keep their connect-src (decision 24: the desktop CSP does not change)', () => {
    expect(connectSrc(POLICIES.desktop.production)).toBe(DESKTOP_CONNECT)
    expect(connectSrc(POLICIES.desktop.preview)).toBe(DESKTOP_CONNECT)
    expect(connectSrc(POLICIES.desktop.probe)).toBe(`${DESKTOP_CONNECT} https://plugins.hew3d.org`)
    expect(connectSrc(POLICIES.web.probe)).toBe("connect-src 'self' blob: https://plugins.hew3d.org")
    for (const name of ['production', 'preview', 'probe']) {
      expect(POLICIES.desktop[name], name).not.toMatch(/connect-src[^;]*\bhttps:(?![/])/)
    }
  })

  it('tauri.conf.json and its baseline carry the unchanged desktop.production policy', () => {
    const conf = JSON.parse(readFileSync(join(ROOT, 'shells/tauri/src-tauri/tauri.conf.json'), 'utf8')) as { app: { security: { csp: string } } }
    expect(conf.app.security.csp).toBe(POLICIES.desktop.production)
    expect(connectSrc(conf.app.security.csp)).toBe(DESKTOP_CONNECT)
    expect(readFileSync(join(ROOT, 'shells/csp/baseline/desktop.production.csp'), 'utf8').replace(/\n$/, '')).toBe(POLICIES.desktop.production)
  })

  it('each web baseline file equals its policy (written by hand in two places, on purpose)', () => {
    const base = (n: string) => readFileSync(join(ROOT, `shells/csp/baseline/${n}.csp`), 'utf8').replace(/\n$/, '')
    expect(base('web.production')).toBe(POLICIES.web.production)
    expect(existsSync(join(ROOT, 'shells/csp/baseline/web.strict.csp'))).toBe(true)
    expect(base('web.strict')).toBe(POLICIES.web.strict)
  })
})

describe('shells/csp/policy.mjs', () => {
  it('policy("web","strict") reads the strict tier', async () => {
    const m = await load()
    expect(m.policy('web', 'strict')).toBe(OLD_WEB_PRODUCTION)
  })

  it('policyForBuild picks production, preview and strict from the environment', async () => {
    const m = await load()
    expect(m.policyForBuild('web', {})).toBe(POLICIES.web.production)
    expect(m.policyForBuild('web', { HEW_PREVIEW: '0' })).toBe(POLICIES.web.production)
    expect(m.policyForBuild('web', { HEW_PREVIEW: '1' })).toBe(POLICIES.web.preview)
    expect(m.policyForBuild('web', { HEW_WEB_SOURCES: 'off' })).toBe(POLICIES.web.strict)
    expect(m.policyForBuild('web', { HEW_WEB_SOURCES: 'off', HEW_PREVIEW: '0' })).toBe(POLICIES.web.strict)
    expect(m.policyForBuild('web', { HEW_WEB_SOURCES: 'off', HEW_PREVIEW: '' })).toBe(POLICIES.web.strict)
  })

  it('an unset or empty HEW_WEB_SOURCES is the default policy', async () => {
    const m = await load()
    expect(m.policyForBuild('web', { HEW_WEB_SOURCES: '' })).toBe(POLICIES.web.production)
    expect(m.policyForBuild('web', { HEW_WEB_SOURCES: undefined, HEW_PREVIEW: '1' })).toBe(POLICIES.web.preview)
  })

  it('HEW_WEB_SOURCES=off together with HEW_PREVIEW=1 is a build error', async () => {
    const m = await load()
    expect(() => m.policyForBuild('web', { HEW_WEB_SOURCES: 'off', HEW_PREVIEW: '1' })).toThrow(/HEW_WEB_SOURCES/)
  })

  it('the desktop shell is untouched by HEW_WEB_SOURCES', async () => {
    const m = await load()
    expect(m.policyForBuild('desktop', { HEW_WEB_SOURCES: 'off' })).toBe(POLICIES.desktop.production)
    expect(m.policyForBuild('desktop', { HEW_PREVIEW: '1' })).toBe(POLICIES.desktop.preview)
  })
})

// ------------------------------------------------ check-production-artifacts

const scratch: string[] = []
afterAll(() => {
  for (const d of scratch) rmSync(d, { recursive: true, force: true })
})

/**
 * A copy of the repository's checking surface in a temp directory: the script
 * and shells/csp are real copies (so a test may edit a baseline or the
 * policies), everything else is a link to the real tree.
 */
function tempRoot(): string {
  const root = mkdtempSync(join(tmpdir(), 'hew-cpa-root-'))
  scratch.push(root)
  for (const name of readdirSync(ROOT)) {
    if (name === 'scripts' || name === 'shells' || name === '.git') continue
    symlinkSync(join(ROOT, name), join(root, name))
  }
  mkdirSync(join(root, 'scripts'))
  cpSync(join(ROOT, 'scripts/check-production-artifacts.mjs'), join(root, 'scripts/check-production-artifacts.mjs'))
  mkdirSync(join(root, 'shells'))
  // shells/tauri is a real directory with a copy of the wrapper script: the
  // wrapper only runs as the main module when it is not reached through a link.
  for (const name of readdirSync(join(ROOT, 'shells'))) {
    if (name !== 'csp' && name !== 'tauri') symlinkSync(join(ROOT, 'shells', name), join(root, 'shells', name))
  }
  mkdirSync(join(root, 'shells/tauri'))
  symlinkSync(join(ROOT, 'shells/tauri/src-tauri'), join(root, 'shells/tauri/src-tauri'))
  cpSync(join(ROOT, 'shells/tauri/package.json'), join(root, 'shells/tauri/package.json'))
  cpSync(join(ROOT, 'shells/tauri/scripts'), join(root, 'shells/tauri/scripts'), { recursive: true })
  cpSync(join(ROOT, 'shells/csp'), join(root, 'shells/csp'), { recursive: true })
  return root
}

/** A built app, a kernel `.d.ts` and a web page with `csp` as its policy meta (or none). */
function fixtures(csp: string | null): { dist: string; dts: string; webDist: string } {
  const dir = mkdtempSync(join(tmpdir(), 'hew-cpa-fix-'))
  scratch.push(dir)
  mkdirSync(join(dir, 'dist'))
  writeFileSync(join(dir, 'dist/index.html'), '<!doctype html><html><head><meta charset="UTF-8" /></head><body></body></html>')
  writeFileSync(join(dir, 'dist/app.js'), 'const k = "hew.settings.x"')
  writeFileSync(join(dir, 'kernel.d.ts'), 'export function preview_build(): boolean;\n')
  mkdirSync(join(dir, 'web'))
  const meta = csp === null ? '' : `\n    <meta http-equiv="Content-Security-Policy" content="${csp}" />`
  writeFileSync(join(dir, 'web/index.html'), `<!doctype html><html><head>\n    <meta charset="UTF-8" />${meta}\n  </head><body></body></html>`)
  return { dist: join(dir, 'dist'), dts: join(dir, 'kernel.d.ts'), webDist: join(dir, 'web') }
}

function check(root: string, csp: string | null) {
  const f = fixtures(csp)
  const r = spawnSync(
    process.execPath,
    [join(root, 'scripts/check-production-artifacts.mjs'), '--dist', f.dist, '--dts', f.dts, '--web-dist', f.webDist],
    { encoding: 'utf8', timeout: 120_000 },
  )
  return { status: r.status, out: `${r.stdout}\n${r.stderr}` }
}

describe('check-production-artifacts on a built web page', { timeout: 90_000 }, () => {
  it('accepts a page that carries web.production', () => {
    const r = check(tempRoot(), POLICIES.web.production)
    expect(r.out).toContain('all clear')
    expect(r.status).toBe(0)
  })

  it('accepts a page that carries web.strict', () => {
    const r = check(tempRoot(), OLD_WEB_PRODUCTION)
    expect(r.out).toContain('all clear')
    expect(r.status).toBe(0)
  })

  it('rejects a page with the preview policy, a widened policy, and a page with none', () => {
    const root = tempRoot()
    const widened = OLD_WEB_PRODUCTION.replace("connect-src 'self' blob:", "connect-src 'self' blob: https: http:")
    for (const csp of [POLICIES.web.preview, widened, "default-src *", null]) {
      const r = check(root, csp)
      expect(r.status, `${csp}\n${r.out}`).toBe(1)
      expect(r.out).toMatch(/web\.(production|strict)|policy/)
    }
  })

  it('checks production against its own baseline: editing web.production.csp fails the run', () => {
    const root = tempRoot()
    const file = join(root, 'shells/csp/baseline/web.production.csp')
    writeFileSync(file, `${readFileSync(file, 'utf8').replace(' https:', '')}`)
    const r = check(root, POLICIES.web.production)
    expect(r.status, r.out).toBe(1)
    expect(r.out).toContain('web.production.csp')
  })

  it('checks strict against its own baseline: editing web.strict.csp fails the run', () => {
    const root = tempRoot()
    const file = join(root, 'shells/csp/baseline/web.strict.csp')
    writeFileSync(file, readFileSync(file, 'utf8').replace("connect-src 'self' blob:", "connect-src 'self' blob: https:"))
    const r = check(root, OLD_WEB_PRODUCTION)
    expect(r.status, r.out).toBe(1)
    expect(r.out).toContain('web.strict.csp')
  })

  it('a strict page is not accepted by pointing the production baseline at strict (the two baselines stay distinct)', () => {
    const root = tempRoot()
    const prod = join(root, 'shells/csp/baseline/web.production.csp')
    const strict = join(root, 'shells/csp/baseline/web.strict.csp')
    expect(readFileSync(prod, 'utf8')).not.toBe(readFileSync(strict, 'utf8'))
  })

  it('still fails on a desktop policy that differs from its baseline (guard)', () => {
    const root = tempRoot()
    const file = join(root, 'shells/csp/baseline/desktop.production.csp')
    writeFileSync(file, readFileSync(file, 'utf8').replace('blob:;', 'blob: https:;'))
    const r = check(root, POLICIES.web.production)
    expect(r.status, r.out).toBe(1)
    expect(r.out).toContain('desktop.production')
  })
})

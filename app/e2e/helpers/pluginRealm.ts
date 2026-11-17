/**
 * What the plugin browser specs share: building a `.hewplugin` in the test
 * (from an example's own files, or from a hostile fixture with a manifest
 * made here), installing it through Settings > Plugins with real events,
 * calling a plugin's command over the host's test connection, and reading
 * the realm states and the log.
 */

import { readFileSync, readdirSync, statSync } from 'node:fs'
import { dirname, join, relative, resolve } from 'node:path'
import { createHash } from 'node:crypto'
import { fileURLToPath } from 'node:url'
import { expect, type Page } from '@playwright/test'
import { isProductionShape } from './preview'
import { storedZipOf as storedZip } from '../../src/plugins/storedZip'

const HERE = dirname(fileURLToPath(import.meta.url))
export const EXAMPLES_DIR = resolve(HERE, '../../../plugins/examples')
export const HOSTILE_DIR = join(EXAMPLES_DIR, 'hostile')

export type RealmState = 'enabled' | 'activating' | 'activated' | 'crashed'

interface PluginHooks {
  call(frame: string): Promise<string>
  states(): Record<string, RealmState>
}
type Harness = { plugins?: PluginHooks }

// -------------------------------------------------------------- packages

function* filesUnder(dir: string): Generator<string> {
  for (const name of readdirSync(dir)) {
    const path = join(dir, name)
    if (statSync(path).isDirectory()) yield* filesUnder(path)
    else yield path
  }
}

export interface Pkg {
  buffer: Buffer
  id: string
  name: string
}

/** An example directory as a `.hewplugin`. */
export function examplePackage(dirName: string): Pkg {
  const dir = join(EXAMPLES_DIR, dirName)
  const entries: Record<string, Uint8Array> = {}
  for (const file of filesUnder(dir)) entries[relative(dir, file).split('\\').join('/')] = readFileSync(file)
  const manifest = JSON.parse(readFileSync(join(dir, 'hew-plugin.json'), 'utf8')) as { id: string; name: string }
  return { buffer: Buffer.from(storedZip(entries)), id: manifest.id, name: manifest.name }
}

export interface CommandSpec {
  id: string
  class?: 'read_only' | 'mutating'
  timeout_ms?: number
}

export interface HostileSpec {
  /** Appended to the id so one fixture can be installed more than once. */
  suffix?: string
  permissions?: string[]
  commands?: CommandSpec[]
  activation?: string[]
  /** Values the fixture reads from the global `HOSTILE`. */
  vars?: unknown
}

/** The parameters the fixtures take; each is open, because a closed schema is the only kind a manifest may declare. */
const PARAMS = ['how', 'op', 'key', 'value', 'count', 'depth', 'chars', 'char', 'limit', 'shape']

const pretty = (row: string): string =>
  'Hostile ' + row.replace(/(^|-)([a-z])/g, (_m, sep: string, c: string) => (sep === '' ? '' : ' ') + c.toUpperCase())

/**
 * A hostile fixture (`plugins/examples/hostile/<row>.js`) as a `.hewplugin`.
 * The manifest is made here; the plugin's `main` is the shared raw-port
 * helper, a line that sets `HOSTILE` to `vars`, and the fixture.
 */
export function hostilePackage(row: string, spec: HostileSpec = {}): Pkg {
  const suffix = spec.suffix === undefined ? '' : `-${spec.suffix}`
  const id = `com.example.hostile-${row}${suffix}`
  const name = pretty(row) + (spec.suffix === undefined ? '' : ` ${spec.suffix.toUpperCase()}`)
  const commands = (spec.commands ?? [{ id: 'run' }]).map((c) => ({
    id: c.id,
    class: c.class ?? 'read_only',
    summary: `The ${c.id} command of a test fixture.`,
    params: { type: 'object', properties: Object.fromEntries(PARAMS.map((n) => [n, {}])), additionalProperties: false },
    result: { type: 'object', properties: { r: {} }, additionalProperties: false },
    ...(c.timeout_ms === undefined ? {} : { timeout_ms: c.timeout_ms }),
  }))
  const manifest = {
    manifest: 1,
    id,
    name,
    version: '0.1.0',
    description: 'A test fixture that misbehaves on purpose.',
    api: 1,
    main: 'dist/background.js',
    permissions: spec.permissions ?? ['model.read'],
    activation: spec.activation ?? ['on_command'],
    contributes: { commands },
  }
  const main =
    readFileSync(join(HOSTILE_DIR, '_raw.js'), 'utf8') +
    `\nvar HOSTILE = ${JSON.stringify(spec.vars ?? {})};\n` +
    readFileSync(join(HOSTILE_DIR, `${row}.js`), 'utf8')
  const buffer = Buffer.from(storedZip({ 'hew-plugin.json': JSON.stringify(manifest), 'dist/background.js': main }))
  return { buffer, id, name }
}

/** The method name a plugin's command is called by. */
export const methodOf = (pkg: Pkg, command = 'run'): string => `${pkg.id}.${command}`

// ------------------------------------------------------------- the page

export const switchOn = async (page: Page): Promise<void> => {
  await page.addInitScript(() => {
    localStorage.setItem('hew.settings.preview', JSON.stringify({ off: [], on: ['plugins'] }))
  })
}

const PREVIEW_CSP = (
  JSON.parse(readFileSync(resolve(HERE, '../../../shells/csp/policies.json'), 'utf8')) as { web: { preview: string } }
).web.preview

/** Paths no real server answers: any hit on one is a request that left a realm. */
export const PROBE_PATH = '/__plugin_probe__/'

export interface Leaks {
  /** Requests that got past every policy and reached the network. */
  hits: string[]
}

/**
 * Serves the page under the policy a preview build ships (`web.preview` in
 * shells/csp/policies.json), sent as a header on the document, plus the
 * hashes of the inline scripts the dev server adds (its React-refresh
 * preamble). Nothing else of the policy changes. Without this the dev
 * server sends no policy at all and the realm would be tested under the
 * frame's own policy only. A request to `PROBE_PATH` that reaches the
 * network is answered with CORS allowed, so the only thing that can stop
 * it is a policy, and it is counted.
 */
export async function applyPreviewPolicy(page: Page): Promise<Leaks> {
  const leaks: Leaks = { hits: [] }
  if (isProductionShape) return leaks
  const origin = `http://127.0.0.1:${Number(process.env.HEW_E2E_PORT ?? 4173)}`
  // The dev server's hot-reload socket is refused by the browser's local-network
  // check once the document is served from here, and its client then waits a
  // long time before it lets the app start. Nothing in these specs needs it.
  await page.routeWebSocket(/^ws:\/\/127\.0\.0\.1:\d+\/\?token=/, () => {})
  await page.route(
    (url) => url.origin === origin && (url.pathname === '/' || url.pathname.startsWith(PROBE_PATH)),
    async (route) => {
      const req = route.request()
      const url = new URL(req.url())
      if (url.pathname.startsWith(PROBE_PATH)) {
        leaks.hits.push(`${req.method()} ${url.pathname}`)
        return route.fulfill({ status: 200, contentType: 'text/plain', headers: { 'access-control-allow-origin': '*' }, body: 'LEAK' })
      }
      if (req.resourceType() !== 'document') return route.fallback()
      const res = await route.fetch()
      const body = await res.text()
      const hashes = [...body.matchAll(/<script(?![^>]*\ssrc=)[^>]*>([\s\S]*?)<\/script>/g)].map(
        (m) => `'sha256-${createHash('sha256').update(m[1]).digest('base64')}'`,
      )
      const csp = PREVIEW_CSP.replace("'wasm-unsafe-eval'", `'wasm-unsafe-eval' ${hashes.join(' ')}`)
      const headers: Record<string, string> = { ...res.headers(), 'content-security-policy': csp }
      delete headers['content-length']
      return route.fulfill({ response: res, body, headers })
    },
  )
  return leaks
}

export async function boot(page: Page): Promise<Leaks> {
  const leaks = await applyPreviewPolicy(page)
  await page.goto('/')
  await page.waitForFunction(
    () => (window as unknown as { __hew_test?: { isReady(): boolean } }).__hew_test?.isReady() === true,
    null,
    { timeout: 90_000 },
  )
  return leaks
}

/** Boots with the switch on and Settings open on the Plugins tab. */
export async function bootWithPlugins(page: Page): Promise<Leaks> {
  await switchOn(page)
  const leaks = await boot(page)
  await openPlugins(page)
  await page.waitForFunction(() => (window as unknown as { __hew_test?: Harness }).__hew_test?.plugins !== undefined, null, {
    timeout: 30_000,
  })
  return leaks
}

export async function openPlugins(page: Page): Promise<void> {
  await page.getByRole('button', { name: 'Settings' }).click()
  await page.getByRole('tab', { name: 'Plugins' }).click()
}

/** Chooses the package, reads the consent sheet's title, installs. */
export async function installPackage(page: Page, pkg: Pkg): Promise<void> {
  await page.locator('input[type=file][aria-label="Plugin package"]').setInputFiles({
    name: `${pkg.id}.hewplugin`,
    mimeType: 'application/octet-stream',
    buffer: pkg.buffer,
  })
  const sheet = page.getByRole('dialog', { name: `Install ${pkg.name}?` })
  const refused = page.getByRole('dialog', { name: 'Settings' }).getByRole('alert')
  await expect(sheet.or(refused).first()).toBeVisible({ timeout: 15_000 })
  if (await refused.isVisible()) throw new Error(`installing ${pkg.id} was refused: ${await refused.innerText()}`)
  await sheet.getByRole('button', { name: 'Install' }).click()
  await expect(sheet).toBeHidden()
  await expect(page.locator(`[data-plugin-row="${pkg.id}"]`)).toBeVisible()
  await waitForState(page, pkg.id, 'enabled')
}

export async function states(page: Page): Promise<Record<string, RealmState>> {
  return page.evaluate(() => (window as unknown as { __hew_test: Harness }).__hew_test.plugins!.states()).catch(() => ({}) as Record<string, RealmState>)
}

export async function waitForState(page: Page, id: string, state: RealmState, timeout = 15_000): Promise<void> {
  await page.waitForFunction(([i, s]) => (window as unknown as { __hew_test: Harness }).__hew_test.plugins?.states()[i] === s, [id, state] as const, { timeout, polling: 100 })
}

export interface Reply {
  jsonrpc: '2.0'
  id: unknown
  result?: Record<string, unknown>
  error?: { code: number; message: string; data?: unknown }
}

let nextId = 1000

/** Calls a method on the host's test connection; resolves with the parsed reply. */
export async function call(page: Page, method: string, params: unknown = {}): Promise<Reply> {
  const id = nextId++
  const text = await page.evaluate(
    ([frame]) => (window as unknown as { __hew_test: Harness }).__hew_test.plugins!.call(frame),
    [JSON.stringify({ jsonrpc: '2.0', id, method, params })] as const,
  )
  const reply = JSON.parse(text) as Reply
  // A hostile fixture answers in the one open slot its manifest declares.
  const r = reply.result
  if (r !== undefined && Object.keys(r).length === 1 && 'r' in r) reply.result = r.r as Record<string, unknown>
  return reply
}

export interface LogLine {
  level: string
  source: string
  message: string
  /** Milliseconds since the epoch. */
  at: number
}

/** Everything the app has logged so far, from the same LogStore the app uses. */
export async function logLines(page: Page): Promise<LogLine[]> {
  return page.evaluate(async () => {
    // The page's own module, by the URL the dev server serves it at; a variable keeps the type checker out of it.
    const url = '/src/log/LogStore.ts'
    const store = (await import(/* @vite-ignore */ url)) as { getEntries(): { level: string; source: string; message: string; timestamp: Date }[] }
    return store.getEntries().map((e) => ({ level: e.level, source: e.source, message: e.message, at: e.timestamp.getTime() }))
  })
}

// ------------------------------------------------------ document and Hew

/** The kernel's hash of the whole document state. */
export const stateHash = (page: Page): Promise<string> =>
  page.evaluate(() => (window as unknown as { __hew_test: { getStateHash(): string } }).__hew_test.getStateHash())

export const objectCount = (page: Page): Promise<number> =>
  page.evaluate(() => (window as unknown as { __hew_test: { getObjectCount(): number } }).__hew_test.getObjectCount())

/** Draws a box through the harness: proof Hew still takes input. */
export async function drawBox(page: Page): Promise<number> {
  const before = await objectCount(page)
  await page.evaluate(() =>
    (window as unknown as { __hew_test: { drawBox(a: number[], b: number[], h: number): string } }).__hew_test.drawBox(
      [5, 5, 0],
      [6, 6, 0],
      1,
    ),
  )
  const after = await objectCount(page)
  expect(after).toBe(before + 1)
  return after
}

/** What a page that is alive does: answers an evaluate and a frame. */
export async function responsive(page: Page): Promise<number> {
  return page.evaluate(() => new Promise<number>((r) => requestAnimationFrame(() => r(performance.now()))))
}

/** Records a measurement into the test output as one compact JSON row. */
export function observe(info: { annotations: { type: string; description?: string }[] }, row: string, data: unknown): void {
  const text = JSON.stringify(data)
  info.annotations.push({ type: `observed:${row}`, description: text })
  console.log(`OBSERVED ${JSON.stringify({ row, data })}`)
}

// -------------------------------------------------------------- crashes

/** The reason and detail of the plugin's stop, from the app's log, or null. */
export async function crashInfo(page: Page, pkg: Pkg): Promise<{ reason: string; detail: string; level: string } | null> {
  const prefix = `Plugin ${pkg.id} 0.1.0 stopped (`
  for (const line of await logLines(page)) {
    if (line.source === 'plugins' && line.message.startsWith(prefix)) {
      const rest = line.message.slice(prefix.length)
      const close = rest.indexOf('): ')
      return { reason: rest.slice(0, close), detail: rest.slice(close + 3), level: line.level }
    }
  }
  return null
}

/** The number of realm frames in the page. */
export const frameCount = (page: Page): Promise<number> => page.evaluate(() => document.querySelectorAll('iframe[sandbox]').length)

/**
 * Waits for the plugin to be stopped, then checks everything a stop must
 * leave: the state, the error line, the notice, and a frame removed.
 */
export async function expectStopped(
  page: Page,
  pkg: Pkg,
  want: { reason: string; detail?: string | RegExp; framesLeft?: number },
  timeout = 30_000,
): Promise<{ reason: string; detail: string }> {
  await waitForState(page, pkg.id, 'crashed', timeout)
  const info = await crashInfo(page, pkg)
  expect(info, 'the stop is in the log').not.toBeNull()
  expect(info?.level).toBe('error')
  expect(info?.reason).toBe(want.reason)
  if (typeof want.detail === 'string') expect(info?.detail).toContain(want.detail)
  else if (want.detail !== undefined) expect(info?.detail).toMatch(want.detail)
  await expect(page.getByText(`${pkg.name} stopped working.`).first()).toBeVisible()
  await expect.poll(() => frameCount(page)).toBe(want.framesLeft ?? 0)
  return { reason: info?.reason ?? '', detail: info?.detail ?? '' }
}

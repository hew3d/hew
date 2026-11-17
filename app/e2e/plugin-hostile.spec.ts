import { readFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { expect, test, type Page } from '@playwright/test'
import { requiresPreview } from './helpers/preview'
import {
  bootWithPlugins,
  call,
  crashInfo,
  drawBox,
  examplePackage,
  expectStopped,
  frameCount,
  PROBE_PATH,
  hostilePackage,
  installPackage,
  logLines,
  methodOf,
  objectCount,
  observe,
  responsive,
  stateHash,
  states,
  waitForState,
  type Leaks,
  type Pkg,
} from './helpers/pluginRealm'

/**
 * Plugins that misbehave on purpose, run in a real browser against the real
 * sandboxed realm. For each row the plugin is stopped (or refused) in the
 * exact way the host promises, the document is unchanged, and Hew still
 * takes input and still answers the well-behaved plugin installed beside
 * it. The fixtures are in plugins/examples/hostile/ and exist only for
 * these specs.
 */

requiresPreview('plugins')
test.setTimeout(120_000)

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

const HERE = dirname(fileURLToPath(import.meta.url))
const METHODS_DOC = resolve(HERE, '../../docs/API_REFERENCE.gen.md')

/** What reached the network from the page being tested. */
let leaks: Leaks = { hits: [] }

/** Hello beside the hostile plugin: the one that must keep answering. */
async function withHello(page: Page): Promise<Pkg> {
  leaks = await bootWithPlugins(page)
  const hello = examplePackage('hello')
  await installPackage(page, hello)
  const first = await call(page, methodOf(hello, 'greet'), {})
  expect(first.result).toMatchObject({ greeting: 'Hello, Hew!' })
  return hello
}

/** Hew still takes input and the well-behaved plugin still answers. */
async function expectHewAlive(page: Page, hello: Pkg): Promise<void> {
  await drawBox(page)
  const reply = await call(page, methodOf(hello, 'greet'), { name: 'again' })
  expect(reply.result).toMatchObject({ greeting: 'Hello, again!' })
  expect((await states(page))[hello.id]).toBe('activated')
}

/** The longest gap between two answers from the page while `during` runs. */
async function worstLatency(page: Page, during: Promise<unknown>): Promise<number> {
  let done = false
  void during.finally(() => {
    done = true
  })
  let worst = 0
  while (!done) {
    const t0 = Date.now()
    await responsive(page)
    worst = Math.max(worst, Date.now() - t0)
    await new Promise((r) => setTimeout(r, 100))
  }
  return worst
}

// ------------------------------------------------------------ CPU spins

test('a spin at the top level of main is stopped when it never starts, and Hew stays responsive', async ({ page }) => {
  const hello = await withHello(page)
  const pkg = hostilePackage('spin-top')
  await installPackage(page, pkg)
  const t0 = Date.now()
  const pending = call(page, methodOf(pkg), {})
  const worst = await worstLatency(page, pending)
  const reply = await pending
  const took = Date.now() - t0
  expect(reply.error).toBeDefined()
  const stop = await expectStopped(page, pkg, { reason: 'start', detail: 'did not start in time', framesLeft: 1 })
  observe(test.info(), 'spin-top', { stopMs: took, worstPageLatencyMs: worst, reply: reply.error, stop })
  expect(took).toBeLessThan(20_000)
  expect(worst).toBeLessThan(2_000)
  await expectHewAlive(page, hello)
})

test('a spin inside a handler is stopped by the missed pings, and Hew stays responsive', async ({ page }) => {
  const hello = await withHello(page)
  // The longest declared timeout, so only the pings can end it.
  const pkg = hostilePackage('spin-handler', { commands: [{ id: 'run', timeout_ms: 50_000 }] })
  await installPackage(page, pkg)
  const hash0 = await stateHash(page)
  const t0 = Date.now()
  const pending = call(page, methodOf(pkg), {})
  const worst = await worstLatency(page, pending)
  const reply = await pending
  const took = Date.now() - t0
  expect(reply.error).toBeDefined()
  const stop = await expectStopped(page, pkg, { reason: 'unresponsive', detail: 'two pings went unanswered', framesLeft: 1 })
  observe(test.info(), 'spin-handler', { stopMs: took, worstPageLatencyMs: worst, reply: reply.error, stop })
  // A ping every 2 s; two missed in a row stop it: silent for 4 to 6 s.
  expect(took).toBeLessThan(10_000)
  expect(worst).toBeLessThan(2_000)
  expect(await stateHash(page)).toBe(hash0)
  await expectHewAlive(page, hello)
})

test('a spin that the declared timeout ends first is a timeout', async ({ page }) => {
  const hello = await withHello(page)
  const pkg = hostilePackage('spin-handler', { suffix: 'short', commands: [{ id: 'run', timeout_ms: 1_000 }] })
  await installPackage(page, pkg)
  const t0 = Date.now()
  const reply = await call(page, methodOf(pkg), {})
  const took = Date.now() - t0
  expect(reply.error).toBeDefined()
  const stop = await expectStopped(page, pkg, { reason: 'timeout', detail: 'did not answer within 1 s', framesLeft: 1 })
  observe(test.info(), 'spin-timeout', { stopMs: took, reply: reply.error, stop })
  expect(took).toBeLessThan(8_000)
  await expectHewAlive(page, hello)
})

// ----------------------------------------------------------- the port

for (const [label, cheap, count] of [
  ['of cheap calls', true, 3000],
  ['of calls that each cost the host real work', false, 2500],
] as const) {
  test(`a flood ${label} over the SDK is stopped, and Hew stays responsive meanwhile`, async ({ page }) => {
    test.setTimeout(240_000)
    const hello = await withHello(page)
    const pkg = hostilePackage('flood', { suffix: cheap ? 'cheap' : 'costly', vars: { cheap, count } })
    await installPackage(page, pkg)
    const hash0 = await stateHash(page)
    const t0 = Date.now()
    const pending = call(page, methodOf(pkg), {})
    const worst = await worstLatency(page, pending)
    const reply = await pending
    const took = Date.now() - t0
    const stop = await crashInfo(page, pkg)
    observe(test.info(), `flood-${cheap ? 'cheap' : 'costly'}`, { count, ms: took, worstPageLatencyMs: worst, reply: reply.error ?? reply.result, stop })
    expect(reply.error, 'the call is answered with an error').toBeDefined()
    await expectStopped(page, pkg, { reason: 'violation', detail: 'more than the allowed messages in one second', framesLeft: 1 })
    expect(await stateHash(page)).toBe(hash0)
    expect(worst).toBeLessThan(2_000)
    await expectHewAlive(page, hello)
  })
}

// What each junk shape posted straight on the port is stopped for.
const SHAPES: [string, string | RegExp][] = [
  ['oversize-frame', 'frame is longer than'],
  ['oversize-console-arg', 'holds something other than short text'],
  ['forged-forward-reply', 'answered a forward it was not given'],
  ['forged-reply-shape', 'answered with something that is not a reply'],
  ['forged-progress', 'progress on a forward it was not given'],
  ['forged-bus-forward', 'named a forward it was not given'],
  ['reused-storage-id', 'reused a storage request id'],
  ['negative-storage-id', 'id is not a non-negative integer'],
  ['forged-hello-ack', 'started twice'],
  ['unknown-type', 'unknown message type'],
  ['extra-field', 'too many fields'],
  ['bad-console-level', 'unknown console level'],
  ['bad-fraction', 'fraction is not null or 0 to 1'],
  ['bad-forward-type', 'forward is not a non-negative integer'],
  ['proto-key', 'too many fields'],
  ['many-keys', 'too many fields'],
  ['string-message', 'not an object'],
  ['number-message', 'not an object'],
  ['null-message', 'not an object'],
  ['array-message', 'not an object'],
  ['buffer-message', 'not an object'],
  ['port-transfer', 'the plugin sent a port'],
  ['big-transferred-buffer', 'unknown field "buffer"'],
  ['message-flood', 'more than the allowed messages in one second'],
  ['byte-flood', 'more than the allowed text in one second'],
]

test('every junk shape posted straight on the port stops the plugin, for the reason named, with the document unchanged', async ({ page }) => {
  test.setTimeout(240_000)
  const hello = await withHello(page)
  const hash0 = await stateHash(page)
  const rows: Record<string, unknown> = {}
  for (const [shape, detail] of SHAPES) {
    await test.step(shape, async () => {
      const pkg = hostilePackage('raw-port', { suffix: shape, vars: { shape } })
      await installPackage(page, pkg)
      const t0 = Date.now()
      const reply = await call(page, methodOf(pkg), {})
      const took = Date.now() - t0
      expect.soft(reply.error, `${shape}: the call is answered with an error`).toBeDefined()
      await waitForState(page, pkg.id, 'crashed')
      const info = await crashInfo(page, pkg)
      expect.soft(info?.reason, `${shape}: reason`).toBe('violation')
      if (typeof detail === 'string') expect.soft(info?.detail, `${shape}: detail`).toContain(detail)
      else expect.soft(info?.detail, `${shape}: detail`).toMatch(detail)
      rows[shape] = { ms: took, reason: info?.reason, detail: info?.detail }
    })
  }
  observe(test.info(), 'raw-port-shapes', rows)
  expect(await stateHash(page)).toBe(hash0)
  expect(await frameCount(page)).toBe(1)
  await expectHewAlive(page, hello)
})

test('a plugin can reach the realm port, and what it asks of the files it was packaged with is bounded', async ({ page }) => {
  const hello = await withHello(page)
  const pkg = hostilePackage('raw-port', { vars: { shape: 'files' } })
  const peer = hostilePackage('raw-port', { suffix: 'peer' })
  await installPackage(page, peer)
  await installPackage(page, pkg)
  const reply = await call(page, methodOf(pkg), {})
  expect(reply.error).toBeUndefined()
  const got = reply.result as { entries: { path: string; bytes: number }[]; missing: string[] }
  observe(test.info(), 'files-traversal', got)
  // Its own files are readable; nothing outside its package is.
  expect(got.entries.map((e) => e.path).sort()).toEqual(['dist/background.js', 'hew-plugin.json'])
  expect(got.missing.sort()).toEqual(
    [
      '../com.example.hostile-peer/hew-plugin.json',
      'dist/../../com.example.hostile-peer/hew-plugin.json',
      '/etc/passwd',
      'C:\\Windows\\win.ini',
      'dist//background.js',
      './dist/background.js',
      'nothing.txt',
    ].sort(),
  )
  expect((await states(page))[pkg.id]).toBe('activated')
  await expectHewAlive(page, hello)
})

// ---------------------------------------------------------- memory

for (const [kind, capMiB] of [
  ['heap', 6144],
  ['buffer', 3072],
] as const) {
  test(`a memory bomb (${kind}) leaves the tab alive`, async ({ page }) => {
    test.setTimeout(240_000)
    const hello = await withHello(page)
    const pkg = hostilePackage('memory-bomb', { vars: { kind, capMiB }, commands: [{ id: 'run', timeout_ms: 50_000 }] })
    await installPackage(page, pkg)
    let tabCrashed = false
    page.on('crash', () => {
      tabCrashed = true
    })
    const t0 = Date.now()
    const outcome = await call(page, methodOf(pkg), {}).then(
      (r) => ({ reply: r.error ?? r.result }),
      (e: unknown) => ({ threw: String(e).slice(0, 120) }),
    )
    const took = Date.now() - t0
    const alive = tabCrashed ? false : await page.evaluate(() => 1 + 1).then(() => true, () => false)
    const row: Record<string, unknown> = { kind, capMiB, ms: took, tabCrashed, pageAnswers: alive, outcome }
    if (alive) {
      row.states = await states(page)
      row.frames = await frameCount(page)
      row.stop = await crashInfo(page, pkg)
      row.pageJsHeapMiB = await page.evaluate(() => Math.round(((performance as unknown as { memory?: { usedJSHeapSize: number } }).memory?.usedJSHeapSize ?? 0) / 1048576))
    }
    observe(test.info(), `memory-bomb-${kind}`, row)
    expect(tabCrashed, 'the tab must survive a plugin running out of memory').toBe(false)
    expect(alive, 'the page still answers').toBe(true)
    await expectHewAlive(page, hello)
  })
}

// --------------------------------------------------------- probes

test('from the realm Worker there is no IPC, no network, no foreign script, no navigation and no storage', async ({ page, baseURL }) => {
  test.setTimeout(180_000)
  const hello = await withHello(page)
  const pkg = hostilePackage('probe-worker', { vars: { origin: new URL(baseURL ?? 'http://127.0.0.1:4391').origin } })
  await installPackage(page, pkg)
  const reply = await call(page, methodOf(pkg), {})
  expect(reply.error).toBeUndefined()
  const seen = reply.result as Record<string, string>
  observe(test.info(), 'probe-worker', seen)
  observe(test.info(), 'probe-worker-requests-that-reached-the-network', leaks.hits)

  // Nothing of the desktop shell, the page, or the DOM exists there.
  for (const name of [
    '__TAURI__',
    '__TAURI_INTERNALS__',
    '__TAURI_IPC__',
    '__TAURI_METADATA__',
    'ipc',
    'window',
    'parent',
    'top',
    'document',
    'localStorage',
    'sessionStorage',
  ]) {
    expect(seen[`typeof ${name}`], `typeof ${name}`).toBe('undefined')
  }
  expect(seen['typeof navigator.sendBeacon']).toBe('undefined')
  expect(seen['typeof navigator.serviceWorker']).toBe('undefined')

  // The runtime's lockdown removed every storage, network and messaging API
  // before main ran, here and in the Workers a plugin starts; no way back.
  for (const name of ['XMLHttpRequest', 'WebSocket', 'EventSource', 'fetch', 'importScripts', 'indexedDB', 'caches', 'BroadcastChannel', 'SharedWorker', 'Notification', 'WebTransport', 'cookieStore']) {
    expect(seen[`typeof ${name}`], `typeof ${name}`).toBe('undefined')
  }
  expect(seen['typeof navigator.storage']).toBe('undefined')
  expect(seen['typeof Worker']).toBe('function')
  expect(seen['lockdown: left in the realm']).toBe('(none)')
  expect(seen['lockdown: left in a nested Worker']).toBe('(none)')
  expect(seen['lockdown: left in a nested-nested Worker']).toBe('(none)')
  expect(seen['recover: prototype walk']).toBe('ok: (none)')
  expect(seen['recover: self.constructor.prototype.caches']).toBe('ok: undefined')
  expect(seen['recover: Reflect descriptors of caches']).toBe('ok: undefined fixed')
  expect(seen['recover: navigator descriptors of locks']).toBe('ok: undefined')
  expect(seen['recover: delete self.caches']).toBe('ok: undefined')
  expect(seen['recover: Worker from data:']).toMatch(/^threw SecurityError/)
  expect(seen['recover: module Worker']).toMatch(/^threw SecurityError/)
  expect(seen['recover: Worker.prototype.constructor']).toBe('ok: the wrapper')
  expect(seen['location.protocol']).toBe('blob:')
  expect(seen['origin']).toBe('null')

  // No request ever left: none is ok, none got an answer, and the browser saw none.
  expect(seen['fetch http']).toMatch(/^threw TypeError/)
  expect(seen['fetch data']).toMatch(/^threw TypeError/)
  expect(seen['fetch blob']).toMatch(/^threw TypeError/)
  expect(seen['XMLHttpRequest']).toMatch(/^threw TypeError/)
  expect(seen['WebSocket']).toMatch(/^threw TypeError/)
  expect(seen['EventSource']).toMatch(/^threw TypeError/)
  expect(seen['sendBeacon']).toMatch(/^threw/)
  expect(leaks.hits, 'no request reached the network').toEqual([])

  // No script can be loaded or made, except the Worker's own blob children.
  expect(seen['importScripts http']).toMatch(/^threw TypeError/)
  expect(seen['importScripts blob']).toMatch(/^threw TypeError/)
  expect(seen['importScripts data']).toMatch(/^threw TypeError/)
  expect(seen['import() data']).toMatch(/^threw/)
  expect(seen['import() http']).toMatch(/^threw/)
  expect(seen['import() blob']).toMatch(/^threw/)
  expect(seen['eval']).toMatch(/^threw EvalError/)
  expect(seen['new Function']).toMatch(/^threw EvalError/)
  expect(seen['Worker http']).toMatch(/^threw SecurityError/)
  expect(seen['Worker data']).toMatch(/^threw SecurityError/)
  expect(seen['Worker blob']).toMatch(/^ok: message/)

  // Navigation changes nothing; the frame stayed as it was.
  expect(await frameCount(page)).toBe(2)

  // No storage of the page's origin.
  expect(seen['indexedDB.open']).toMatch(/^threw TypeError/)
  expect(seen['caches.open']).toMatch(/^threw TypeError/)
  expect(seen['navigator.storage.getDirectory']).toMatch(/^threw/)
  expect(seen['localStorage']).toMatch(/^threw ReferenceError/)
  expect(seen['cookie']).toMatch(/^threw/)

  // And the plugin is neither stopped nor stopped from answering.
  expect((await states(page))[pkg.id]).toBe('activated')
  await expectHewAlive(page, hello)
})

test('from the realm frame: opaque origin, no parent or top reach, one script, no IPC, no network', async ({ page }) => {
  const hello = await withHello(page)
  const frame = page.frames().find((f) => f !== page.mainFrame())
  expect(frame, 'the realm frame exists').toBeDefined()
  const element = await page.evaluate(() => {
    const f = document.querySelector('iframe[sandbox]') as HTMLIFrameElement
    let doc: string
    try {
      doc = f.contentDocument === null ? 'null' : 'readable'
    } catch (e) {
      doc = (e as Error).name
    }
    let href: string
    try {
      href = String(f.contentWindow?.location.href)
    } catch (e) {
      href = (e as Error).name
    }
    return {
      sandbox: f.getAttribute('sandbox'),
      referrerpolicy: f.getAttribute('referrerpolicy'),
      allow: f.getAttribute('allow'),
      src: f.getAttribute('src'),
      doc,
      href,
    }
  })
  observe(test.info(), 'frame-element', element)
  expect(element.sandbox).toBe('allow-scripts')
  expect(element.allow).toBeNull()
  expect(element.src).toBeNull()
  expect(['null', 'SecurityError']).toContain(element.doc)
  expect(element.href).toBe('SecurityError')

  const inside = await (frame as NonNullable<typeof frame>).evaluate(async () => {
    const out: Record<string, string> = {}
    const attempt = async (name: string, fn: () => unknown): Promise<void> => {
      try {
        const v = await fn()
        out[name] = `ok: ${String(v).slice(0, 80)}`
      } catch (e) {
        out[name] = `threw ${(e as Error).name}`
      }
    }
    const violations: string[] = []
    document.addEventListener('securitypolicyviolation', (e) => violations.push(`${e.violatedDirective} ${e.blockedURI.slice(0, 40)}`))
    const g = globalThis as unknown as Record<string, unknown>
    for (const n of ['__TAURI__', '__TAURI_INTERNALS__', '__TAURI_IPC__', 'ipc']) out[`typeof ${n}`] = typeof g[n]
    out['origin'] = String(location.origin)
    out['scripts'] = String(document.scripts.length)
    out['scripts with src'] = String(document.querySelectorAll('script[src]').length)
    out['body children'] = String(document.body.children.length)
    out['csp meta'] = String(document.querySelectorAll('meta[http-equiv="Content-Security-Policy"]').length)
    await attempt('window.parent.document', () => window.parent.document.title)
    await attempt('window.top.location.href', () => window.top?.location.href)
    await attempt('window.parent.__hew_test', () => (window.parent as unknown as Record<string, unknown>).__hew_test)
    await attempt('parent.postMessage', () => window.parent.postMessage({ hew: 'ready', nonce: 'x'.repeat(32) }, '*'))
    await attempt('window.top === window.parent', () => window.top === window.parent)
    await attempt('localStorage', () => localStorage.length)
    await attempt('indexedDB', () => indexedDB.databases())
    await attempt('document.cookie', () => document.cookie)
    await attempt('fetch', () => fetch('/__plugin_probe__/frame'))
    await attempt('XMLHttpRequest', () => {
      const x = new XMLHttpRequest()
      x.open('GET', '/__plugin_probe__/frame', false)
      x.send()
    })
    await attempt('sendBeacon', () => navigator.sendBeacon('/__plugin_probe__/frame', 'x'))
    await attempt('new WebSocket', () => new WebSocket('ws://127.0.0.1:1/__plugin_probe__/frame'))
    await attempt('eval', () => eval('1+1'))
    await attempt('inline script injected', () => {
      const s = document.createElement('script')
      s.textContent = 'window.__injected = 1'
      document.head.appendChild(s)
      return (window as unknown as { __injected?: number }).__injected
    })
    await attempt('script src injected', () => {
      const s = document.createElement('script')
      s.src = URL.createObjectURL(new Blob(['window.__injected2 = 1'], { type: 'text/javascript' }))
      document.head.appendChild(s)
      return (window as unknown as { __injected2?: number }).__injected2
    })
    await attempt('nested iframe', () => {
      const f = document.createElement('iframe')
      f.src = 'about:blank'
      document.body.appendChild(f)
      return f.contentWindow === null ? 'blocked' : 'created'
    })
    await attempt('form submit', () => {
      const f = document.createElement('form')
      f.action = '/__plugin_probe__/form'
      f.method = 'post'
      document.body.appendChild(f)
      f.submit()
    })
    // Violations are reported in a task of their own.
    await new Promise((r) => setTimeout(r, 200))
    out['violations'] = violations.join(' | ')
    return out
  })
  observe(test.info(), 'frame-probe', inside)
  for (const n of ['__TAURI__', '__TAURI_INTERNALS__', '__TAURI_IPC__', 'ipc']) expect(inside[`typeof ${n}`]).toBe('undefined')
  expect(inside['origin']).toBe('null')
  expect(inside['scripts']).toBe('1')
  expect(inside['scripts with src']).toBe('0')
  expect(inside['window.parent.document']).toMatch(/^threw SecurityError/)
  expect(inside['window.top.location.href']).toMatch(/^threw SecurityError/)
  expect(inside['window.parent.__hew_test']).toMatch(/^threw SecurityError/)
  expect(inside['localStorage']).toMatch(/^threw SecurityError/)
  expect(inside['indexedDB']).toMatch(/^threw SecurityError/)
  expect(inside['document.cookie']).toMatch(/^threw SecurityError/)
  expect(inside['fetch']).toMatch(/^threw TypeError/)
  expect(inside['XMLHttpRequest']).toMatch(/^threw/)
  expect(inside['inline script injected']).toMatch(/^ok: undefined/)
  expect(inside['script src injected']).toMatch(/^ok: undefined/)
  // Nothing it tried got a request out, and it did not move: it is the one document it started as.
  expect(leaks.hits, 'no request reached the network').toEqual([])
  expect((await states(page))[hello.id]).toBe('activated')
  await expectHewAlive(page, hello)
  expect(await frameCount(page)).toBe(1)
})

test('a realm frame that navigates is stopped', async ({ page }) => {
  const hello = await withHello(page)
  const pkg = hostilePackage('probe-channels', { commands: [{ id: 'listen' }, { id: 'report' }] })
  await installPackage(page, pkg)
  await call(page, methodOf(pkg, 'report'), {})
  await waitForState(page, pkg.id, 'activated')
  // Only the page-side stand-in can do this: no plugin code runs in the frame.
  const frames = page.frames().filter((f) => f !== page.mainFrame())
  expect(frames).toHaveLength(2)
  const frame = frames[1]
  await frame.evaluate((path) => location.assign(path), `${PROBE_PATH}nav`).catch(() => undefined)
  const stop = await expectStopped(page, pkg, { reason: 'navigated', detail: 'loaded another page', framesLeft: 1 }, 15_000)
  observe(test.info(), 'frame-navigation', { stop, networkHits: leaks.hits })
  await expectHewAlive(page, hello)
})

test('a realm hears none of the store’s notices and its forged ones reach nobody', async ({ page }) => {
  const hello = await withHello(page)
  const pkg = hostilePackage('probe-channels', { commands: [{ id: 'listen' }, { id: 'report' }] })
  await installPackage(page, pkg)
  // What this page hears on the store's channel from now on.
  await page.evaluate(() => {
    const heard: unknown[] = []
    const ch = new BroadcastChannel('hew-plugins')
    ch.onmessage = (e) => heard.push(e.data)
    ;(window as unknown as { __heard: unknown[] }).__heard = heard
    ;(window as unknown as { __channel: BroadcastChannel }).__channel = ch
  })
  // The runtime removed BroadcastChannel from the realm before main ran.
  expect((await call(page, methodOf(pkg, 'listen'), {})).result).toEqual({ listening: false, broadcastChannel: 'undefined' })
  // The page posts a notice of the kind the store makes, on the same channel
  // name, after the realm is listening: a plugin storage change.
  await call(page, methodOf(hello, 'greet'), {})
  await page.evaluate(() => {
    const ch = new BroadcastChannel('hew-plugins')
    ch.postMessage({ kind: 'storage', id: 'com.example.hostile-probe-channels', keys: ['x'] })
    ch.close()
  })
  // A negative result needs a wait: a notice that were to cross would arrive within a task or two.
  await new Promise((r) => setTimeout(r, 1000))
  const report = await call(page, methodOf(pkg, 'report'), {})
  const pageHeard = await page.evaluate(() => (window as unknown as { __heard: unknown[] }).__heard)
  observe(test.info(), 'broadcast-channel', { realmHeard: report.result, pageHeard })
  expect((report.result as { heard: string[] }).heard).toEqual([])
  expect(pageHeard.filter((m) => (m as { id?: string }).id === 'com.example.hostile-peer')).toEqual([])
  expect((await states(page))[pkg.id]).toBe('activated')
  await expectHewAlive(page, hello)
})

test('a plugin that closes its own Worker is stopped by the missed pings', async ({ page }) => {
  const hello = await withHello(page)
  const pkg = hostilePackage('self-close', { commands: [{ id: 'run', timeout_ms: 50_000 }] })
  await installPackage(page, pkg)
  const t0 = Date.now()
  const reply = await call(page, methodOf(pkg), {})
  const took = Date.now() - t0
  expect(reply.error).toBeDefined()
  const stop = await expectStopped(page, pkg, { reason: 'unresponsive', framesLeft: 1 })
  observe(test.info(), 'self-close', { stopMs: took, reply: reply.error, stop })
  // A ping every 2 s; two missed in a row stop it: silent for 4 to 6 s.
  expect(took).toBeLessThan(10_000)
  await expectHewAlive(page, hello)
})

test('a plugin whose nested Worker computes for 10 s without yielding keeps answering, and is not stopped', async ({ page }) => {
  const hello = await withHello(page)
  const pkg = hostilePackage('nested-spin', { commands: [{ id: 'run' }, { id: 'ping' }, { id: 'result', timeout_ms: 30_000 }] })
  await installPackage(page, pkg)
  const t0 = Date.now()
  expect((await call(page, methodOf(pkg), { count: 10_000 })).result).toEqual({ started: true })
  // Throughout the nested computation, and past several ping intervals, the realm answers.
  const slowest: number[] = []
  while (Date.now() - t0 < 9_000) {
    const p0 = Date.now()
    expect((await call(page, methodOf(pkg, 'ping'), {})).result).toEqual({ alive: true })
    slowest.push(Date.now() - p0)
    await new Promise((r) => setTimeout(r, 500))
  }
  const result = await call(page, methodOf(pkg, 'result'), {})
  observe(test.info(), 'nested-spin', { ms: Date.now() - t0, pings: slowest.length, slowestPingMs: Math.max(...slowest), result: result.result })
  expect(result.result).toEqual({ computed: true })
  expect(Date.now() - t0).toBeGreaterThanOrEqual(10_000)
  expect((await states(page))[pkg.id]).toBe('activated')
  expect(await crashInfo(page, pkg)).toBeNull()
  await expectHewAlive(page, hello)
})

// ------------------------------------------------------- the bus

/** The commands that need a permission a view-only plugin lacks, from the API reference. */
function writeMethods(): string[] {
  const text = readFileSync(METHODS_DOC, 'utf8')
  const out: string[] = []
  const re = /^### `(hew\.[a-z_.]+)`[\s\S]*?\*\*Permission:\*\* ([a-z.]+)/gm
  for (let m = re.exec(text); m !== null; m = re.exec(text)) {
    if (!['always', 'model.read', 'view'].includes(m[2])) out.push(m[1])
  }
  return out.sort()
}

test('a plugin that may only read and change the view is refused every write, by every route, and the document is unchanged', async ({ page }) => {
  test.setTimeout(180_000)
  const hello = await withHello(page)
  await drawBox(page)
  const methods = writeMethods()
  expect(methods.length).toBeGreaterThan(100)
  const pkg = hostilePackage('view-only', { permissions: ['model.read', 'view'], vars: { methods } })
  await installPackage(page, pkg)
  const hash0 = await stateHash(page)
  const depth = async (): Promise<number> => ((await call(page, 'hew.history.status', {})).result as { undo_depth: number }).undo_depth
  const d0 = await depth()
  const reply = await call(page, methodOf(pkg), {})
  expect(reply.error).toBeUndefined()
  const r = reply.result as {
    direct: Record<string, number | string>
    inside: Record<string, number | string>
    dry: number | string
    mixed: { code: number } | string
    hello: number | string
    helloGrant: { plugin: string; permissions: string[] } | null
    attach: number | string
    batch: unknown
    read: number | string
    view: number | string
  }
  const codes = (o: Record<string, number | string>): Record<string, number> => {
    const c: Record<string, number> = {}
    for (const v of Object.values(o)) c[String(v)] = (c[String(v)] ?? 0) + 1
    return c
  }
  observe(test.info(), 'view-only', {
    methods: methods.length,
    directCodes: codes(r.direct),
    insideCodes: codes(r.inside),
    dry: r.dry,
    mixed: r.mixed,
    hello: r.hello,
    helloGrant: r.helloGrant,
    attach: r.attach,
    batch: r.batch,
    read: r.read,
    view: r.view,
  })
  const notRefused = (o: Record<string, number | string>): string[] =>
    Object.entries(o)
      .filter(([, v]) => v !== -32001)
      .map(([k, v]) => `${k}=${v}`)
  expect(notRefused(r.direct), 'every write named directly is refused -32001').toEqual([])
  expect(notRefused(r.inside), 'every write named inside a transaction is refused -32001').toEqual([])
  // A rehearsal changes nothing and needs only read permissions, by design; the document check below covers it.
  expect(r.dry, 'a rehearsed write').toBe('RESULT')
  expect((r.mixed as { code: number }).code, 'a write next to a read in one transaction').toBe(-32001)
  // Saying hello again may be answered, but it names no new identity: the grant is the one it was given.
  if (r.helloGrant !== null) {
    expect(r.helloGrant.plugin).toBe(pkg.id)
    expect([...r.helloGrant.permissions].sort()).toEqual(['model.read', 'view'])
  }
  expect(r.attach, 'attaching to another document').not.toBe('RESULT')
  // What the grant does allow still works.
  expect(r.read).toBe('RESULT')
  expect(r.view).toBe('RESULT')
  // And the document did not move: not the state, not the history.
  expect(await stateHash(page)).toBe(hash0)
  expect(await depth()).toBe(d0)
  expect((await states(page))[pkg.id]).toBe('activated')
  await expectHewAlive(page, hello)
})

test('a plugin that calls its own command from inside it is answered, not deadlocked', async ({ page }) => {
  const hello = await withHello(page)
  const pkg = hostilePackage('reentrant', { commands: [{ id: 'run', timeout_ms: 5_000 }], permissions: ['model.read'] })
  await installPackage(page, pkg)
  const t0 = Date.now()
  const reply = await call(page, methodOf(pkg), {})
  const took = Date.now() - t0
  observe(test.info(), 'reentrant', { ms: took, reply, state: (await states(page))[pkg.id], stop: await crashInfo(page, pkg) })
  expect(took, 'answered well before the declared timeout').toBeLessThan(4_000)
  expect(reply.result).toEqual({ direct: { depth: 1, reached: true }, viaTransact: 'RESULT' })
  expect((await states(page))[pkg.id]).toBe('activated')
  await expectHewAlive(page, hello)
})

test('a command that answers while a nested command of its own still runs leaves the document free and the history whole', async ({ page }) => {
  const hello = await withHello(page)
  const pkg = hostilePackage('nested-orphan', {
    permissions: ['model.read', 'model.write'],
    commands: [
      { id: 'run', class: 'mutating', timeout_ms: 10_000 },
      { id: 'slow', class: 'mutating', timeout_ms: 10_000 },
    ],
  })
  await installPackage(page, pkg)
  const isBusy = (): Promise<boolean> => page.evaluate(() => window.__hew_test!.isBusy())
  const undoDepth = (): Promise<number> => page.evaluate(() => window.__hew_test!.getUndoDepth())
  const objects0 = await objectCount(page)
  const depth0 = await undoDepth()
  const reply = await call(page, methodOf(pkg), {})
  expect(reply.error, JSON.stringify(reply.error)).toBeUndefined()
  expect(reply.result).toEqual({ fired: true })
  // The inner command was ended with the outer one: nothing holds the document.
  await expect.poll(() => isBusy()).toBe(false)
  // Let the inner handler's own timer run out; nothing more lands.
  await page.waitForTimeout(3_500)
  const after = {
    busy: await isBusy(),
    objects: await objectCount(page),
    depth: await undoDepth(),
    state: (await states(page))[pkg.id],
    stop: await crashInfo(page, pkg),
  }
  observe(test.info(), 'nested-orphan', { objects0, depth0, ...after })
  expect(after.busy).toBe(false)
  // One entry, the outer box; the inner box was taken back.
  expect(after.objects).toBe(objects0 + 1)
  expect(after.depth).toBe(depth0 + 1)
  expect(after.state).toBe('activated')
  expect(after.stop).toBeNull()
  // The history is whole: the entry undoes to where it began.
  await page.evaluate(() => window.__hew_test!.undo())
  expect(await objectCount(page)).toBe(objects0)
  expect(await undoDepth()).toBe(depth0)
  await expectHewAlive(page, hello)
})

test('saying hello again under another name does not change who an undo entry belongs to', async ({ page }) => {
  const hello = await withHello(page)
  const pkg = hostilePackage('mutator', { permissions: ['model.read', 'model.write'], commands: [{ id: 'spoof', class: 'mutating' }] })
  await installPackage(page, pkg)
  const reply = await call(page, methodOf(pkg, 'spoof'), {})
  const status = await call(page, 'hew.history.status', {})
  observe(test.info(), 'identity-spoof', { reply: reply.error ?? reply.result, history: status.result })
  expect(reply.error).toBeUndefined()
  expect(JSON.stringify(status.result)).not.toContain('org.hew3d')
  await expectHewAlive(page, hello)
})

// -------------------------------------------------- handler faults

const FAULTS: { how: string; stops?: { reason: string; detail?: string | RegExp }; refusal?: string }[] = [
  { how: 'throw', stops: { reason: 'error', detail: 'Error: on purpose' } },
  { how: 'reject', stops: { reason: 'error', detail: 'Error: rejected on purpose' } },
  { how: 'bigint', stops: { reason: 'error', detail: 'could not be sent' } },
  { how: 'cycle', stops: { reason: 'error', detail: 'could not be sent' } },
  { how: 'function', stops: { reason: 'violation', detail: 'not a reply' } },
  { how: 'huge', stops: { reason: 'error', detail: 'too large to send' } },
  { how: 'refuse', refusal: 'not_today' },
]

test('a handler that throws, returns what cannot be sent, or answers too big is stopped; a refusal is not', async ({ page }) => {
  test.setTimeout(180_000)
  const hello = await withHello(page)
  const rows: Record<string, unknown> = {}
  for (const f of FAULTS) {
    await test.step(f.how, async () => {
      const pkg = hostilePackage('handler-faults', { suffix: f.how, vars: { how: f.how } })
      await installPackage(page, pkg)
      const reply = await call(page, methodOf(pkg), {})
      rows[f.how] = { error: reply.error, result: reply.result }
      expect.soft(reply.result, `${f.how}: no result`).toBeUndefined()
      expect.soft(reply.error, `${f.how}: an error`).toBeDefined()
      if (f.stops !== undefined) {
        await waitForState(page, pkg.id, 'crashed')
        const info = await crashInfo(page, pkg)
        expect.soft(info?.reason, `${f.how}: reason`).toBe(f.stops.reason)
        if (typeof f.stops.detail === 'string') expect.soft(info?.detail, `${f.how}: detail`).toContain(f.stops.detail)
        rows[f.how] = { ...(rows[f.how] as object), stop: info }
      } else {
        expect.soft(JSON.stringify(reply.error), `${f.how}: the refusal's name`).toContain(f.refusal)
        expect.soft((await states(page))[pkg.id], `${f.how}: still running`).toBe('activated')
      }
    })
  }
  observe(test.info(), 'handler-faults', rows)
  await expectHewAlive(page, hello)
})

test('a handler that returns nothing, and one that never answers', async ({ page }) => {
  const hello = await withHello(page)
  const none = hostilePackage('handler-faults', { suffix: 'none', vars: { how: 'undefined' } })
  const never = hostilePackage('handler-faults', { suffix: 'never', vars: { how: 'never' }, commands: [{ id: 'run', timeout_ms: 1_000 }] })
  await installPackage(page, none)
  await installPackage(page, never)
  const nothing = await call(page, methodOf(none), {})
  observe(test.info(), 'handler-returns-undefined', { reply: nothing, state: (await states(page))[none.id] })
  const t0 = Date.now()
  const late = await call(page, methodOf(never), {})
  const took = Date.now() - t0
  expect(late.error).toBeDefined()
  const stop = await expectStopped(page, never, { reason: 'timeout', detail: 'did not answer within 1 s', framesLeft: 2 })
  observe(test.info(), 'handler-never-answers', { ms: took, reply: late.error, stop })
  expect(took).toBeLessThan(6_000)
  await expectHewAlive(page, hello)
})

test('a plugin that floods the console is held to a few lines and keeps working', async ({ page }) => {
  const hello = await withHello(page)
  const pkg = hostilePackage('console-flood', { commands: [{ id: 'run' }, { id: 'ping' }] })
  await installPackage(page, pkg)
  const before = (await logLines(page)).length
  const reply = await call(page, methodOf(pkg), {})
  expect(reply.result).toEqual({ done: true })
  const ping = await call(page, methodOf(pkg, 'ping'), {})
  expect(ping.result).toEqual({ alive: true })
  const lines = (await logLines(page)).slice(before).filter((l) => l.message.startsWith(`[${pkg.id}]`))
  // The most lines in any one second.
  let busiest = 0
  for (const a of lines) busiest = Math.max(busiest, lines.filter((b) => b.at >= a.at && b.at < a.at + 1000).length)
  observe(test.info(), 'console-flood', { logged: lines.length, busiestSecond: busiest, longest: Math.max(0, ...lines.map((l) => l.message.length)), sample: lines[0] })
  expect(busiest).toBeLessThanOrEqual(40)
  expect(Math.max(0, ...lines.map((l) => l.message.length))).toBeLessThanOrEqual(2_100)
  expect((await states(page))[pkg.id]).toBe('activated')
  await expectHewAlive(page, hello)
})

// A stopped plugin stays stopped until it is turned off and on.
test('a stopped plugin answers unavailable and starts again after it is turned off and on', async ({ page }) => {
  const hello = await withHello(page)
  const pkg = hostilePackage('handler-faults', { vars: { how: 'throw' } })
  await installPackage(page, pkg)
  await call(page, methodOf(pkg), {})
  await waitForState(page, pkg.id, 'crashed')
  const again = await call(page, methodOf(pkg), {})
  expect(again.error).toBeDefined()
  const row = page.locator(`[data-plugin-row="${pkg.id}"]`)
  const toggle = row.getByRole('switch', { name: `Turn ${pkg.name} on or off` })
  await toggle.click()
  await expect.poll(async () => (await states(page))[pkg.id]).toBeUndefined()
  await toggle.click()
  await page.getByRole('dialog', { name: 'Turn on this plugin?' }).getByRole('button', { name: 'Turn On' }).click()
  await waitForState(page, pkg.id, 'enabled')
  // It starts again, and fails again the same way: it is the same plugin.
  await call(page, methodOf(pkg), {})
  await waitForState(page, pkg.id, 'crashed')
  await expectHewAlive(page, hello)
  expect(await objectCount(page)).toBe(1)
})

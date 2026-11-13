/* oxlint-disable lingui/no-unlocalized-strings -- dev-only probe: protocol strings and scripts for the engine record, never user-facing */
import { isTauri } from '../../io/fileHost'
import { BOOT_PROBE_SOURCE } from './bootProbe'
import { createRealmFrame, makeNonce, probeSrcdoc, type PortMessage, type RealmHandle } from './realm'
import { WORKER_PROBE_SOURCE } from './workerProbe'
import {
  SURFACE_CLICK,
  SURFACE_ENV,
  SURFACE_HIT,
  SURFACE_IMG,
  SURFACE_NAVIGATE_REMOTE,
  SURFACE_NAVIGATE_SELF,
  SURFACE_SPIN,
  SURFACE_STYLES,
} from './surfaceScripts'

/**
 * The realm probe runner (plugin system stage P0): measures, on the engine it
 * runs in, the browser behaviors the sandboxed-realm design relies on. Dev
 * only; reached through the `#plugin-probe` hash root (ProbeRoot.tsx).
 */

export type RowOutcome = {
  row: string
  status: 'pass' | 'fail' | 'recorded' | 'skipped'
  detail: string
  data?: unknown
}
export type ProbeResults = { engine: string; ua: string; isolatedHost: boolean; rows: RowOutcome[] }

type RowResult = Omit<RowOutcome, 'row'>
interface Ctx {
  mount: HTMLElement
  viewport?: HTMLElement
  /** True when a driver clicks the surface for us (Playwright); desktop runs leave it false. */
  interactive: boolean
  /** True when the driver has cut the network (and counts requests) for this run. */
  offline: boolean
  engine: string
  frames: HTMLElement[]
}

export const ROW_IDS = [
  'boot-hash',
  'worker-blob',
  'worker-module-type',
  'worker-env',
  'worker-denials',
  'worker-wasm',
  'worker-isolation',
  'eval-blocked',
  'worker-spin-host-alive',
  'surface-boot',
  'surface-styles',
  'surface-img-self',
  'surface-navigate-remote',
  'surface-navigate-self',
  'surface-click-activation',
  'surface-webgl2',
  'surface-hit-regions',
  'offline-boot',
  'latency',
  'throughput',
  'transfer',
  'surface-spin',
] as const

/** Rows that never run unless listed explicitly (they can hang the page). */
const EXPLICIT_ONLY = new Set(['surface-spin'])

export function detectEngine(): string {
  const ua = navigator.userAgent
  const plat = navigator.platform
  if (isTauri) {
    if (/Mac/.test(plat)) return 'wkwebview'
    if (/Win/.test(plat)) return 'webview2'
    if (/Linux|X11/.test(plat)) return 'webkitgtk'
    return 'unknown'
  }
  if (/Firefox\//.test(ua)) return 'firefox'
  if (/Chrome\/|Chromium\/|Edg\//.test(ua)) return 'chromium'
  if (/AppleWebKit\//.test(ua)) return 'webkit'
  return 'unknown'
}

const sleep = (ms: number): Promise<void> => new Promise((r) => setTimeout(r, ms))
const errText = (e: unknown): string => (e instanceof Error ? `${e.name}: ${e.message}` : String(e))

function pct(sorted: number[], p: number): number {
  return sorted[Math.min(sorted.length - 1, Math.floor((sorted.length * p) / 100))]
}
function stats(samples: number[]): { p50: number; p95: number } {
  const s = [...samples].sort((a, b) => a - b)
  return { p50: +pct(s, 50).toFixed(3), p95: +pct(s, 95).toFixed(3) }
}

async function realm(ctx: Ctx, mode: 'background' | 'surface', parent?: HTMLElement): Promise<RealmHandle> {
  const h = await createRealmFrame(mode, parent ?? ctx.mount)
  ctx.frames.push(h.frame)
  if (!parent) h.frame.style.cssText = 'width:200px;height:100px;border:1px solid #888'
  return h
}

async function withWorker(ctx: Ctx, module = false, live = true): Promise<RealmHandle> {
  const h = await realm(ctx, 'background')
  await h.request({ t: 'start_worker', code: WORKER_PROBE_SOURCE, module }, 'worker_started', 2000)
  // worker_started only says the frame called `new Worker`; a script that fails
  // to load still reports it. Ping the Worker so every worker row starts from a
  // Worker that is actually running.
  if (live) await ask(h, { t: 'echo', n: 0 }, 'echo', 2000)
  return h
}
/** Sends a request to the Worker and resolves with its reply (the `from_worker` payload). */
async function ask(h: RealmHandle, data: PortMessage, replyT: string, ms: number, transfer?: Transferable[]): Promise<PortMessage> {
  const from = h.log.length
  h.send({ t: 'to_worker', data, transfer }, transfer)
  try {
    const m = await h.waitFor((x) => x.t === 'from_worker' && x.data?.t === replyT, ms, from)
    return m.data as PortMessage
  } catch (e) {
    const errs = h.log.filter((m) => m.t === 'worker_error').map((m) => m.message)
    throw new Error(`${errText(e)}${errs.length ? ` [worker_error: ${errs.join('; ')}]` : ' [no worker_error]'}`)
  }
}

interface SurfaceRun {
  h: RealmHandle
  report: PortMessage | null
  ready: boolean
}
async function runSurface(
  ctx: Ctx,
  script: string,
  opts: { wantReport?: boolean; ms?: number; parent?: HTMLElement } = {},
): Promise<SurfaceRun> {
  const h = await realm(ctx, 'surface', opts.parent)
  const from = h.log.length
  h.send({ t: 'surface', html: '<head></head><body></body>', scripts: [{ code: script }] })
  let ready = false
  try {
    await h.waitFor((m) => m.t === 'surface_ready', opts.ms ?? 2000, from)
    ready = true
  } catch {
    return { h, report: null, ready }
  }
  if (opts.wantReport === false) return { h, report: null, ready }
  try {
    const m = await h.waitFor((x) => x.t === 'surface_report', opts.ms ?? 2000, from)
    return { h, report: m.data as PortMessage, ready }
  } catch {
    return { h, report: null, ready }
  }
}

type RowFn = (ctx: Ctx) => Promise<RowResult>

const pass = (detail: string, data?: unknown): RowResult => ({ status: 'pass', detail, data })
const fail = (detail: string, data?: unknown): RowResult => ({ status: 'fail', detail, data })
const recorded = (detail: string, data?: unknown): RowResult => ({ status: 'recorded', detail, data })

const ROWS: Record<string, RowFn> = {
  async 'boot-hash'(ctx) {
    const first = await realm(ctx, 'background')
    first.frame.remove()
    // Same srcdoc with one extra space inside the script: the hash no longer
    // matches, so a policy that enforces hashes keeps the script from running.
    const nonce = makeNonce()
    const doc = probeSrcdoc({ nonce, mode: 'background' }).replace(
      `<script>${BOOT_PROBE_SOURCE}</script>`,
      `<script> ${BOOT_PROBE_SOURCE}</script>`,
    )
    if (doc === probeSrcdoc({ nonce, mode: 'background' })) return fail('probe bug: tamper did not change the srcdoc')
    const frame = document.createElement('iframe')
    frame.setAttribute('sandbox', 'allow-scripts')
    frame.srcdoc = doc
    ctx.frames.push(frame)
    let ran = false
    const on = (ev: MessageEvent): void => {
      if (ev.source === frame.contentWindow && ev.data?.hew === 'ready') ran = true
    }
    window.addEventListener('message', on)
    ctx.mount.appendChild(frame)
    await sleep(2000)
    window.removeEventListener('message', on)
    return ran
      ? fail('tampered boot script ran: this engine ignores inherited hashes in srcdoc (boot by URL on ' + ctx.engine + ')')
      : pass('hashed boot script ran; a one-space variant was blocked')
  },

  async 'worker-blob'(ctx) {
    const t0 = performance.now()
    const h = await withWorker(ctx, false, false)
    const t1 = performance.now()
    const r = await ask(h, { t: 'echo', n: 7 }, 'echo', 2000)
    const timing = { startMs: Math.round(t1 - t0), echoMs: Math.round(performance.now() - t1) }
    return r.n === 7 ? pass(`worker_started then echo (${timing.startMs} ms + ${timing.echoMs} ms)`, timing) : fail('echo mismatch', r)
  },

  // Not in the stage table: the stage's boot script asks for a module worker,
  // which a sandboxed opaque-origin frame cannot start from a blob URL on
  // Chromium. The boot script starts a classic worker; this row records what
  // a module worker does on the engine so the realm design has the fact.
  async 'worker-module-type'(ctx) {
    const h = await withWorker(ctx, true, false)
    try {
      const r = await ask(h, { t: 'echo', n: 3 }, 'echo', 2000)
      return recorded('module worker from a blob URL works', r)
    } catch {
      const err = h.log.find((m) => m.t === 'worker_error')
      return recorded(`module worker from a blob URL does not run (${err ? err.message : 'no echo'})`, { workerError: err?.message ?? null })
    }
  },

  async 'worker-env'(ctx) {
    const h = await withWorker(ctx)
    const e = await ask(h, { t: 'env' }, 'env', 2000)
    const ok = e.rtc === 'undefined' && e.ws === 'function' && e.origin === 'null'
    return ok ? pass('rtc undefined, ws present, origin null', e) : recorded('differs from expected', e)
  },

  async 'worker-denials'(ctx) {
    const h = await withWorker(ctx)
    const r = await ask(h, { t: 'denials', hostOrigin: location.origin }, 'denials', 25000)
    const results = r.results as { name: string; outcome: string; detail: string; csp?: string; recordOnly?: boolean; required?: boolean }[]
    const allowed = results.filter((x) => x.outcome === 'allowed' && !x.recordOnly)
    if (allowed.length) return fail('allowed: ' + allowed.map((x) => x.name).join(', '), results)
    // A stalled request is not a block: a timeout, or an API that is missing,
    // on any non-record-only entry means the row measured nothing for it.
    const unproven = results.filter((x) => !x.recordOnly && x.outcome === 'timeout')
    if (unproven.length) return fail('stalled, not blocked: ' + unproven.map((x) => x.name).join(', '), results)
    // An absent API cannot be called at all (no Cache API in an opaque-origin
    // blob worker on Chromium): safe, but the policy was never exercised, so
    // the required entries are recorded, not passed.
    const absent = results.filter((x) => x.required && x.outcome === 'absent')
    if (absent.length) return recorded('none allowed; required entries not exercised, API absent: ' + absent.map((x) => x.name).join(', '), results)
    // Blocked rests on policy only when a CSP violation was reported for it;
    // the rest (CORS, network, API refusal, unparseable relative URL) are
    // listed so the record shows what each block rested on.
    const blocked = results.filter((x) => x.outcome === 'blocked')
    const noPolicy = blocked.filter((x) => x.csp === 'none' || x.detail.startsWith('url_unparseable'))
    const data = { results, blockedCount: blocked.length, policyEvidenced: blocked.length - noPolicy.length, noPolicyEvidence: noPolicy.map((x) => x.name) }
    if (noPolicy.length === blocked.length)
      return recorded('none allowed, but no securitypolicyviolation was reported for any entry on this engine', data)
    return pass(`none allowed; required Cache.add entries blocked; ${blocked.length - noPolicy.length} of ${blocked.length} blocks reported a CSP violation`, data)
  },

  async 'worker-wasm'(ctx) {
    const h = await withWorker(ctx)
    const r = await ask(h, { t: 'wasm' }, 'wasm', 2000)
    return r.ok === true ? pass('instantiated') : fail(String(r.error), r)
  },

  // Recorded only: cross-origin isolation is not used, so the host sends no
  // COOP/COEP and hew.supports('wasm.threads') is false on every engine.
  async 'worker-isolation'(ctx) {
    const h = await withWorker(ctx)
    const e = await ask(h, { t: 'env' }, 'env', 2000)
    const host = self.crossOriginIsolated === true
    return recorded(`host=${host} frame=${h.hello.isolated} worker=${e.isolated} sab=${e.sab}`, { host, frame: h.hello.isolated, worker: e.isolated, sab: e.sab })
  },

  async 'eval-blocked'(ctx) {
    const h = await realm(ctx, 'background')
    const r = await h.request({ t: 'eval_in_frame', code: '1+1', id: 1 }, 'eval_result', 2000)
    return r.ok === false && String(r.error).startsWith('EvalError') ? pass(String(r.error)) : fail('eval was not blocked', r)
  },

  async 'worker-spin-host-alive'(ctx) {
    const h = await withWorker(ctx)
    h.send({ t: 'to_worker', data: { t: 'spin' } })
    // Control: a spinning worker must stop answering, else the row measured nothing.
    await sleep(100)
    const probeFrom = h.log.length
    h.send({ t: 'to_worker', data: { t: 'echo', n: 1 } })
    await sleep(300)
    if (h.log.slice(probeFrom).some((m) => m.t === 'from_worker')) return fail('probe bug: worker still answered after spin')
    let frames = 0
    const t0 = performance.now()
    await new Promise<void>((res) => {
      const tick = (): void => {
        frames++
        if (performance.now() - t0 >= 2000) res()
        else requestAnimationFrame(tick)
      }
      requestAnimationFrame(tick)
    })
    h.frame.remove()
    return frames >= 30 ? pass(`${frames} frames in 2 s`, { frames }) : fail(`host starved: ${frames} frames in 2 s`, { frames })
  },

  async 'surface-boot'(ctx) {
    const s = await runSurface(ctx, SURFACE_ENV)
    if (!s.ready || !s.report) return fail('no surface_ready/surface_report within 2 s')
    const r = s.report
    const ok = r.origin === 'null' && r.localStorage === 'SecurityError' && (r.cookie === 'SecurityError' || r.cookie === 'ok')
    return (ok ? pass : fail)(`origin=${r.origin} localStorage=${r.localStorage} cookie=${r.cookie}`, r)
  },

  async 'surface-styles'(ctx) {
    const s = await runSurface(ctx, SURFACE_STYLES, { ms: 4000 })
    if (!s.report) return fail('no report')
    const r = s.report
    const ok = r.styleTag === '17px' && r.styleAttr === '23px' && r.blobSheet === '29px'
    return (ok ? pass : fail)(`styleTag=${r.styleTag} styleAttr=${r.styleAttr} blobSheet=${r.blobSheet}`, r)
  },

  async 'surface-img-self'(ctx) {
    const s = await runSurface(ctx, SURFACE_IMG, { ms: 6000 })
    if (!s.report) return fail('no report')
    const r = s.report
    if (r.dataImg !== 'load') return fail('control data: image did not load; row inconclusive', r)
    // Control: the asset must exist, else an error is just a 404.
    const asset = await fetch('/favicon.svg').then((x) => x.ok, () => false)
    if (!asset) return fail('control: /favicon.svg is not served by the host; row inconclusive', { ...r, asset })
    return (r.selfImg === 'error' ? pass : fail)(`selfImg=${r.selfImg} dataImg=${r.dataImg}`, r)
  },

  async 'surface-navigate-remote'(ctx) {
    const s = await runSurface(ctx, SURFACE_NAVIGATE_REMOTE)
    // Control: the script must have run (it reports before navigating).
    if (!s.ready || s.report?.ran !== true) return fail('navigation script never ran; row inconclusive', { ready: s.ready })
    await sleep(2000)
    // Route hits are counted by the driving test, which combines them with this.
    return s.h.loadCount === 0
      ? pass('loadCount 0 (the driver adds the request count)', { loadCount: s.h.loadCount })
      : recorded(`frame loaded ${s.h.loadCount} time(s); the kill rule bounds it`, { loadCount: s.h.loadCount })
  },

  async 'surface-navigate-self'(ctx) {
    const s = await runSurface(ctx, SURFACE_NAVIGATE_SELF)
    if (!s.ready || s.report?.ran !== true) return fail('navigation script never ran; row inconclusive', { ready: s.ready })
    await sleep(2000)
    return recorded(`loadCount ${s.h.loadCount}`, { loadCount: s.h.loadCount })
  },

  async 'surface-click-activation'(ctx) {
    const s = await runSurface(ctx, SURFACE_CLICK, { wantReport: false })
    if (!s.ready) return fail('surface did not start')
    if (!ctx.interactive) return { status: 'skipped', detail: 'needs a driver to click the button' }
    let clicked = true
    try {
      await s.h.waitFor((m) => m.t === 'surface_report' && m.data?.clicked === true, 15000)
    } catch {
      clicked = false
    }
    if (!clicked) return fail('no click observed')
    const out: Record<string, string> = {}
    try {
      await navigator.clipboard.writeText('hew-probe')
      out.write = 'ok'
    } catch (e) {
      out.write = errText(e)
    }
    try {
      out.read = await navigator.clipboard.readText()
    } catch (e) {
      out.read = 'error ' + errText(e)
    }
    const readOk = out.read === 'hew-probe'
    if (out.write === 'ok' && readOk) return pass(`write ok, read=${out.read}`, out)
    if (out.write === 'ok' && ctx.engine === 'webkit') return recorded(`write ok, readText did not return the text: read=${out.read}`, out)
    return fail(`write=${out.write} read=${out.read}`, out)
  },

  async 'surface-webgl2'(ctx) {
    const s = await runSurface(ctx, SURFACE_ENV)
    if (!s.report) return fail('no report')
    return (s.report.webgl2 === true ? pass : fail)(`webgl2=${s.report.webgl2} rtc=${s.report.rtc} prefetchLink=${s.report.prefetchLink}`, s.report)
  },

  async 'surface-hit-regions'(ctx) {
    if (!ctx.viewport) return { status: 'skipped', detail: 'no viewport element' }
    if (!ctx.interactive) return { status: 'skipped', detail: 'needs a driver to click' }
    const vp = ctx.viewport
    let hostClicks = 0
    const onDown = (ev: PointerEvent): void => {
      if (ev.target === vp) hostClicks++
    }
    vp.addEventListener('pointerdown', onDown)
    try {
      const h = await createRealmFrame('surface', vp)
      ctx.frames.push(h.frame)
      h.frame.style.cssText =
        'position:absolute;left:0;top:0;width:100%;height:100%;border:0;pointer-events:auto;' +
        'clip-path:polygon(0 0, 50% 0, 50% 50%, 0 50%)'
      h.send({ t: 'surface', html: '<head></head><body></body>', scripts: [{ code: SURFACE_HIT }] })
      const hits = (): number => h.log.filter((m) => m.t === 'surface_report' && m.data?.hit).length
      try {
        await h.waitFor((m) => m.t === 'surface_report' && !!m.data?.hit, 15000)
      } catch {
        return fail('first click never reached the surface')
      }
      const afterFirst = hostClicks
      const deadline = performance.now() + 15000
      while (hostClicks === 0 && performance.now() < deadline) await sleep(25)
      await sleep(500)
      const ok = afterFirst === 0 && hostClicks >= 1 && hits() === 1
      return (ok ? pass : fail)(`hostClicksAfterFirst=${afterFirst} hostClicks=${hostClicks} surfaceHits=${hits()}`, { afterFirst, hostClicks, surfaceHits: hits() })
    } finally {
      vp.removeEventListener('pointerdown', onDown)
    }
  },

  async 'offline-boot'(ctx) {
    const a = await ROWS['boot-hash'](ctx)
    const b = await ROWS['surface-boot'](ctx)
    const data = { onLine: navigator.onLine, offlineExercised: ctx.offline, bootHash: a.status, bootHashDetail: a.detail, surfaceBoot: b.status, surfaceBootDetail: b.detail }
    if (a.status !== 'pass' || b.status !== 'pass')
      return fail(`a handshake failed: boot-hash ${a.status} (${a.detail}); surface-boot ${b.status} (${b.detail})`, data)
    // Only a driver that cuts the network and counts requests can say the
    // boot needed none; without one this row proves nothing about offline.
    return ctx.offline
      ? pass(`both handshakes completed with the network cut (onLine=${navigator.onLine}); the driver counts requests`, data)
      : recorded(`offline not exercised (onLine=${navigator.onLine}, no request counting here): both handshakes completed`, data)
  },

  async latency(ctx) {
    const h = await withWorker(ctx)
    const worker: number[] = []
    for (let i = 0; i < 550; i++) {
      const t0 = performance.now()
      await ask(h, { t: 'echo', n: i }, 'echo', 2000)
      if (i >= 50) worker.push(performance.now() - t0)
    }
    const f = await realm(ctx, 'surface')
    const frame: number[] = []
    for (let i = 0; i < 550; i++) {
      const t0 = performance.now()
      await f.request({ t: 'eval_in_frame', code: '1', id: i }, 'eval_result', 2000)
      if (i >= 50) frame.push(performance.now() - t0)
    }
    const data = { workerRoundTrip: stats(worker), frameRoundTrip: stats(frame), samples: 500 }
    return recorded(`worker p50=${data.workerRoundTrip.p50} p95=${data.workerRoundTrip.p95} ms; frame p50=${data.frameRoundTrip.p50} p95=${data.frameRoundTrip.p95} ms`, data)
  },

  async throughput(ctx) {
    const h = await withWorker(ctx)
    const from = h.log.length
    const t0 = performance.now()
    h.send({ t: 'to_worker', data: { t: 'burst', count: 1000 } })
    await h.waitFor((m) => m.t === 'from_worker' && m.data?.t === 'burst_done', 10000, from)
    const ms = performance.now() - t0
    const items = h.log.slice(from).filter((m) => m.t === 'from_worker' && m.data?.t === 'burst_item').length
    return recorded(`1000 messages in ${ms.toFixed(1)} ms (${items} items)`, { ms: +ms.toFixed(1), items, perSecond: Math.round((items / ms) * 1000) })
  },

  async transfer(ctx) {
    const h = await withWorker(ctx)
    const size = 64 * 1024 * 1024
    const buffer = new ArrayBuffer(size)
    const t0 = performance.now()
    const r = await ask(h, { t: 'transfer', buffer }, 'transfer', 10000, [buffer])
    const ms = performance.now() - t0
    const back = r.buffer as ArrayBuffer
    const detached = buffer.byteLength === 0
    const data = { workerByteLength: r.byteLength, hostAfterSend: buffer.byteLength, returned: back?.byteLength, roundTripMs: +ms.toFixed(1) }
    return r.byteLength === size && detached && back?.byteLength === size
      ? pass(`transferred, not copied; round trip ${ms.toFixed(1)} ms`, data)
      : fail('buffer was copied or lost', data)
  },

  async 'surface-spin'(ctx) {
    const s = await runSurface(ctx, SURFACE_SPIN)
    if (!s.ready || s.report?.spinning !== true) return fail('spin script never started; row inconclusive', { ready: s.ready })
    // Control: a spinning surface must stop answering, else nothing was measured.
    await sleep(100)
    const from = s.h.log.length
    s.h.send({ t: 'eval_in_frame', code: '1', id: 1 })
    await sleep(300)
    if (s.h.log.slice(from).some((m) => m.t === 'eval_result')) return fail('probe bug: surface still answered after spin')
    // Host responsiveness is measured while the spinning frame is still alive.
    let frames = 0
    const t0 = performance.now()
    await new Promise<void>((res) => {
      const tick = (): void => {
        frames++
        if (performance.now() - t0 >= 2000) res()
        else requestAnimationFrame(tick)
      }
      requestAnimationFrame(tick)
    })
    return recorded(frames >= 30 ? `host responsive while the surface spun (${frames} frames in 2 s)` : `host starved while the surface spun (${frames} frames in 2 s)`, { frames, spinConfirmed: true })
  },
}

export async function runProbe(
  mount: HTMLElement,
  opts: { rows?: string[]; viewport?: HTMLElement; interactive?: boolean; offline?: boolean } = {},
): Promise<ProbeResults> {
  const engine = detectEngine()
  const out: ProbeResults = { engine, ua: navigator.userAgent, isolatedHost: self.crossOriginIsolated === true, rows: [] }
  for (const id of ROW_IDS) {
    if (opts.rows ? !opts.rows.includes(id) : EXPLICIT_ONLY.has(id)) continue
    const ctx: Ctx = { mount, viewport: opts.viewport, interactive: opts.interactive === true, offline: opts.offline === true, engine, frames: [] }
    try {
      out.rows.push({ row: id, ...(await ROWS[id](ctx)) })
    } catch (e) {
      out.rows.push({ row: id, status: 'fail', detail: errText(e) })
    } finally {
      for (const f of ctx.frames) f.remove()
    }
  }
  return out
}

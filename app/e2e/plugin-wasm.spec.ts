import { expect, test, type Page } from '@playwright/test'
import { requiresPreview } from './helpers/preview'
import { bootWithPlugins, examplePackage, installPackage, methodOf, observe, type Pkg } from './helpers/pluginRealm'
import { callIn, openShop, statesIn, waitStateIn, type Where } from './helpers/pluginStage'

/**
 * Two proofs the plugin runtime makes about the browser it runs in, in the
 * editor and in Shop Mode: nothing a plugin runs in is cross-origin isolated
 * (so there is no shared memory and no thread pool), and a real
 * single-threaded WebAssembly job still uses every core through nested
 * Workers that are all handed one compiled module.
 */

requiresPreview('plugins')
test.setTimeout(180_000)

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

interface Slice {
  index: number
  primes: number
  ms: number
  isModule: boolean
  exportsNamed: string[]
  compiles: number
  receivedKeys: string[]
  isolated: boolean
}
interface Job {
  limit: number
  workers: number
  primes: number
  slices: Slice[]
  compiles: number
  wallMs: number
  bytes: number
}

const LIMIT = 8_000_000
/** The primes below 8,000,000. */
const PRIMES_BELOW_LIMIT = 539_777

/** The editor with the plugin installed; and Shop beside it when asked. */
async function setup(page: Page, pkg: Pkg, context: import('@playwright/test').BrowserContext, where: Where): Promise<Page> {
  await bootWithPlugins(page)
  await installPackage(page, pkg)
  if (where === 'editor') return page
  const shop = await openShop(context)
  // The host answers once it has read the store: the plugin shows as enabled.
  await waitStateIn(shop.page, 'shop', pkg.id, 'enabled', 60_000)
  return shop.page
}

for (const where of ['editor', 'shop'] as const) {
  test.describe(`${where}`, () => {
    let pkg: Pkg
    let target: Page

    test.beforeEach(async ({ page, context }) => {
      pkg = examplePackage('wasm-job')
      target = await setup(page, pkg, context, where)
    })

    test('nothing a plugin runs in is cross-origin isolated: the page, the frame, the Worker and a nested Worker', async ({}, info) => {
      const reply = await callIn(target, where, methodOf(pkg, 'iso'))
      expect(reply.error).toBeUndefined()
      const worker = reply.result as { isolated: unknown; nested: unknown; sharedArrayBuffer: unknown; cores: unknown }
      const pageIsolated = await target.evaluate(() => crossOriginIsolated)
      // The realm frame: sandboxed with an opaque origin; the driver can still ask it.
      const frames = target.frames().filter((f) => f !== target.mainFrame())
      expect(frames.length, 'a realm frame exists once the plugin runs').toBeGreaterThan(0)
      const frameIsolated: unknown[] = []
      for (const f of frames) frameIsolated.push(await f.evaluate(() => crossOriginIsolated))
      observe(info, `isolation-${where}`, { page: pageIsolated, frames: frameIsolated, worker: worker.isolated, nested: worker.nested, sab: worker.sharedArrayBuffer })
      expect(pageIsolated).toBe(false)
      expect(frameIsolated).toEqual(frames.map(() => false))
      expect(worker.isolated).toBe(false)
      expect(worker.nested).toBe(false)
      // Without isolation there is no shared memory to build a thread pool on.
      expect(worker.sharedArrayBuffer).toBe('undefined')
    })

    test('a WebAssembly job runs in nested Workers that all take one compiled module, and the realm stays responsive', async ({}, info) => {
      const cores = await target.evaluate(() => navigator.hardwareConcurrency)
      const workers = Math.min(8, cores)
      const method = methodOf(pkg, 'run')

      // One slice alone: what a Worker costs when it has the cores to itself.
      const single = (await callIn(target, where, method, { limit: LIMIT, workers, only: 0 })).result as unknown as Job
      expect(single.slices).toHaveLength(1)
      const slice1 = single.wallMs

      // All slices at once, while the realm is asked to answer in the meantime.
      let jobDone = false
      const job = callIn(target, where, method, { limit: LIMIT, workers }).finally(() => {
        jobDone = true
      })
      let pings = 0
      let slowestPing = 0
      while (!jobDone) {
        const t0 = Date.now()
        const p = await callIn(target, where, methodOf(pkg, 'ping'))
        const took = Date.now() - t0
        expect(p.result).toMatchObject({ alive: true })
        if (!jobDone) {
          pings++
          slowestPing = Math.max(slowestPing, took)
        }
      }
      const reply = await job
      expect(reply.error).toBeUndefined()
      const all = reply.result as unknown as Job

      // The answer is right: the combined count, and each slice's share.
      expect(all.primes).toBe(PRIMES_BELOW_LIMIT)
      expect(all.slices.reduce((n, s) => n + s.primes, 0)).toBe(PRIMES_BELOW_LIMIT)
      expect(all.slices.map((s) => s.index).sort()).toEqual([...Array(workers).keys()])
      // The module is only ever compiled once, by the realm's Worker.
      expect(all.compiles).toBe(1)
      for (const s of all.slices) {
        expect(s.isModule, 'a nested Worker was handed a compiled module').toBe(true)
        expect(s.exportsNamed).toEqual(['count'])
        expect(s.compiles, 'a nested Worker never compiled anything').toBe(0)
        // No bytes were sent: the message held the module and its slice, nothing else.
        expect(s.receivedKeys).toEqual(['chunk', 'index', 'module', 'stop', 'workers'])
        expect(s.isolated).toBe(false)
      }
      // The realm answered while the job ran, and the plugin is still running.
      expect(pings, 'pings answered while the job ran').toBeGreaterThan(0)
      expect((await statesIn(target, where))[pkg.id]).toBe('activated')
      const after = await callIn(target, where, methodOf(pkg, 'ping'))
      expect(after.result).toMatchObject({ alive: true })

      const sliceMs = all.slices.map((s) => Math.round(s.ms))
      observe(info, `wasm-job-${where}`, {
        cores,
        workers,
        wasmBytes: all.bytes,
        compiles: all.compiles,
        singleSliceWallMs: Math.round(slice1),
        allSlicesWallMs: Math.round(all.wallMs),
        serialEstimateMs: Math.round(slice1 * workers),
        speedup: Number(((slice1 * workers) / all.wallMs).toFixed(2)),
        perWorkerMs: sliceMs,
        pingsDuring: pings,
        slowestPingMs: slowestPing,
      })
      if (workers > 1) {
        // Loose: run in series the slices would take workers x one slice.
        expect(all.wallMs).toBeLessThan(0.8 * workers * slice1)
      }
      await waitStateIn(target, where, pkg.id, 'activated')
    })
  })
}

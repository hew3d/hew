/**
 * Plugin system stage P0: the realm probe inside the real desktop webview
 * (WebKitGTK under tauri-driver). Run against the probe build
 * (`pnpm --dir shells/tauri e2e:build:probe`), which carries the proposed
 * parent CSP from src-tauri/probe.conf.json.
 *
 * The desktop run cannot grant clipboard permissions, click through frames,
 * or count network requests, so those rows record what the page observed
 * (the click and hit-region rows report `skipped`). `surface-spin` is never
 * run here.
 */
import { mkdirSync, writeFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

export {}

type Row = { row: string; status: string; detail: string; data?: unknown }
type Results = { engine: string; ua: string; isolatedHost: boolean; rows: Row[] }

declare global {
  interface Window {
    __hew_probe?: { done: boolean; results?: Results }
  }
}

const ROWS = [
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
]

describe('Hew desktop realm probe', () => {
  it('runs every pass and recorded row in the desktop webview', async () => {
    await browser.waitUntil(
      async () => (await browser.execute(() => (window as unknown as { __hew_test?: { isReady(): boolean } }).__hew_test?.isReady() === true)) === true,
      { timeout: 30_000, timeoutMsg: 'harness/kernel never became ready' },
    )

    const rows = ROWS.join(',')
    await browser.execute((r: string) => {
      window.location.hash = `#plugin-probe?rows=${r}`
      window.location.reload()
    }, rows)

    await browser.waitUntil(
      async () => (await browser.execute(() => window.__hew_probe?.done === true)) === true,
      { timeout: 100_000, timeoutMsg: 'probe never finished' },
    )
    const results = (await browser.execute(() => window.__hew_probe!.results)) as Results

    const here = dirname(fileURLToPath(import.meta.url))
    const dir = resolve(here, 'results')
    mkdirSync(dir, { recursive: true })
    writeFileSync(resolve(dir, `${process.platform}.json`), JSON.stringify(results, null, 2))
    console.log(JSON.stringify(results, null, 2))

    // The page must have produced a row for every id; individual outcomes are
    // the record, not assertions (a fallback is a valid result).
    expect(results.rows.map((r) => r.row)).toEqual(ROWS)
  })
})

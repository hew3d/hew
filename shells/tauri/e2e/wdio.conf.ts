import { spawn, type ChildProcess } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import { dirname, resolve } from 'node:path'
import { existsSync, readFileSync } from 'node:fs'

/**
 * WebdriverIO + tauri-driver config for the **real desktop binary** (
 * docs/DEVELOPMENT.md). tauri-driver is Tauri's official WebDriver intermediary; it
 * shims a W3C session onto the platform webview driver — on Linux that's
 * `WebKitWebDriver` (apt: `webkit2gtk-driver`). **macOS is unsupported** (WKWebView
 * has no WebDriver), which is fine: CI is Linux. See e2e/README.md for the
 * one-time host setup.
 *
 * The spec drives the same `window.__hew_test` semantic harness the web E2E uses
 * — so this layer proves the *desktop wiring* (Tauri shell + WebKitGTK +
 * WASM kernel) boots and round-trips, without re-testing modeling logic the
 * harness/kernel suites already cover.
 */

const here = dirname(fileURLToPath(import.meta.url))
const srcTauri = resolve(here, '../src-tauri')

// Derive the binary name from Cargo.toml so this survives the planned
// `hew-desktop` → `hew` rename (and `Hew.app` on macOS) with no edit here.
function cargoBinName(): string {
  const toml = readFileSync(resolve(srcTauri, 'Cargo.toml'), 'utf8')
  const m = toml.match(/^\s*name\s*=\s*"([^"]+)"/m)
  if (!m) throw new Error('wdio.conf: could not read package name from src-tauri/Cargo.toml')
  return m[1]
}

// The harness-enabled debug binary built by `pnpm --dir shells/tauri e2e:build`
// (VITE_HEW_TEST=1 tauri build --debug --no-bundle). Override with HEW_DESKTOP_BIN.
const application =
  process.env.HEW_DESKTOP_BIN ?? resolve(srcTauri, 'target/debug', cargoBinName())

// tauri-driver delegates to the native webview driver. Default to PATH lookup;
// override the WebKitWebDriver path with WEBKIT_WEBDRIVER when it isn't on PATH.
const nativeDriver = process.env.WEBKIT_WEBDRIVER

let tauriDriver: ChildProcess | undefined

export const config: WebdriverIO.Config = {
  runner: 'local',
  specs: ['./specs/**/*.e2e.ts'],
  maxInstances: 1,

  // tauri-driver listens here; WebdriverIO talks to it as the WebDriver endpoint.
  hostname: '127.0.0.1',
  port: 4444,

  capabilities: [
    {
      // `tauri:options` is consumed by tauri-driver, not a standard W3C cap.
      // @ts-expect-error custom Tauri capability
      'tauri:options': { application },
      'wdio:maxInstances': 1,
    },
  ],

  logLevel: 'warn',
  framework: 'mocha',
  reporters: ['spec'],
  mochaOpts: { ui: 'bdd', timeout: 120_000 },

  onPrepare() {
    if (!existsSync(application)) {
      throw new Error(
        `Desktop binary not found at:\n  ${application}\n` +
          `Build it first:  pnpm --dir shells/tauri e2e:build`,
      )
    }
  },

  // tauri-driver must run for the duration of each session.
  beforeSession() {
    const args = ['--port', '4444']
    if (nativeDriver) args.push('--native-driver', nativeDriver)
    tauriDriver = spawn('tauri-driver', args, {
      stdio: [null, process.stdout, process.stderr],
    })
  },
  afterSession() {
    tauriDriver?.kill()
  },
}

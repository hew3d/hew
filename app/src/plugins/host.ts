/* oxlint-disable lingui/no-unlocalized-strings -- protocol words and log lines; English only until the feature graduates. */
/**
 * Starts the plugin host for one document window. Reached only through
 * `preview/loaderRoots.ts` (`loadPluginHost`), which a production build
 * compiles to `null`, and started by `App.tsx` only while the Plugins
 * Preview switch is on.
 *
 * It fills the forward router (so the live bridge, where the registry turns
 * that on, asks this host for a plugin's answer), and, when the test
 * harness exists, adds `__hew_test.plugins`. A plugin's request for more
 * permissions is put to the person through the store's `setGrant` (on the
 * desktop, the shell's native confirmation). On the desktop it also shows
 * the live API's request to load a plugin from a folder
 * (`DevRequestNotice`) in a corner of this window, and tells the shell
 * that this window runs a host (`plugin_host_present`) from start to stop,
 * so the live API refuses `plugins_off` while no window does.
 */

import { createElement } from 'react'
import { createRoot } from 'react-dom/client'
import { setForwardRouter } from '../api/forwardRouter'
import type { LiveBridgeDeps } from '../api/liveBridge'
import { isTauri } from '../io/fileHost'
import * as LogStore from '../log/LogStore'
import * as kernelModule from '../wasm/pkg/wasm_api.js'
import { DevRequestNotice } from './DevRequestNotice'
import { createPluginHost, type BlockReason, type ConsoleLine, type PluginHost, type RealmChannel, type RealmState, type ReportedState } from './hostCore'
import { noteConsole, noteHostState, resetHostState } from './hostState'
import { toPluginError } from './pluginError'
import { pluginInstaller } from './installer'
import { createRealmFrame } from './realm/frame'
import { desktopStorage, webStorage } from './storageBackend'
import { webGrantDecider } from './grantSheet'
import { invokeShell, storeAccessForWindow, storeGrantDecider } from './storeAccess'

export interface PluginHostOptions {
  bridge: LiveBridgeDeps
  toast(message: string): void
  /** `'shop'` for Shop Mode: viewer grants, no permission requests, no
   *  desktop request notice. Absent in a document window. */
  platform?: 'shop'
  /** Where the test hooks (`plugins`) go: the window's harness object
   *  (Shop's `__hew_shop_test`). Absent: `window.__hew_test`. */
  harness?: Record<string, unknown> | null
}

/** What the test harness can do with the host. */
export interface PluginTestHooks {
  /** Dispatches a frame on a first-party connection of this window, as a
   *  socket client would, with forwards served by the host. */
  call(frame: string): Promise<string>
  states(): Record<string, RealmState>
  /** Why each `blocked` plugin is (`states()` says `blocked`). */
  reasons(): Record<string, BlockReason>
  /** Stands in for a person's act in plugin `id`'s own UI: for a few
   *  seconds the plugin may ask for an optional permission, once. Nothing
   *  else sets it until plugins have UI of their own. */
  activate(id: string): void
  /** The plugins a listener connection is open for. */
  listening(): string[]
}

function grantJson(id: string, document: string, namesJson: string, viewer: boolean): string {
  const fn = (kernelModule as unknown as Record<string, unknown>).plugin_grant_json
  if (typeof fn !== 'function') throw new Error('this kernel cannot grant plugins')
  return (fn as (a: string, b: string, c: string, d: boolean) => string)(id, document, namesJson, viewer)
}

function mountPoint(): HTMLElement {
  const mount = document.createElement('div')
  mount.setAttribute('aria-hidden', 'true')
  mount.style.cssText = 'position:fixed;width:0;height:0;overflow:hidden;pointer-events:none;left:-1px;top:-1px'
  document.body.appendChild(mount)
  return mount
}

async function openRealm(mount: HTMLElement, main: ArrayBuffer, onNavigate: () => void): Promise<RealmChannel> {
  const frame = await createRealmFrame(mount, { handshakeMs: 5_000, onNavigate })
  const channel = new MessageChannel()
  frame.control.postMessage({ t: 'boot', code: main }, [main, channel.port2])
  return {
    control: frame.control,
    port: channel.port1,
    remove: () => frame.remove(),
  }
}

/**
 * The shell's record of a plugin's states and console (desktop). A refusal
 * (`report_too_often`, `console_rate_limited`, a plugin just uninstalled)
 * is logged once per plugin and kind, as an info line, and never reaches
 * the plugin. A refused state report also rejects the promise `report`
 * returns, so the host knows the state was not taken.
 *
 * Each state report carries `seq`, strictly increasing within this page:
 * `max(now in ms, last + 1)`, so it also increases across a reload of the
 * page (a new recorder starts from the clock). The shell keeps, per
 * window, the report with the highest `seq` and ignores an older one that
 * arrives after it (the calls run on their own threads and can land out
 * of order).
 */
export function shellRecorder(invoke: typeof invokeShell = invokeShell): {
  report(id: string, state: ReportedState, reason: string | null): Promise<void>
  console(id: string, lines: ConsoleLine[]): void
} {
  const told = new Set<string>()
  let lastSeq = 0
  const swallow = (id: string, what: string) => (e: unknown) => {
    const key = `${id} ${what} ${toPluginError(e).refusal}`
    if (told.has(key)) return
    told.add(key)
    LogStore.log.info('plugins', `the shell did not take ${what} for ${id}: ${String(e)}`)
  }
  return {
    report(id, state, reason) {
      const seq = Math.max(Date.now(), lastSeq + 1)
      lastSeq = seq
      const logRefusal = swallow(id, 'a state')
      return invoke('plugin_report_state', reason === null ? { id, state, seq } : { id, state, reason, seq }).then(
        () => undefined,
        (e: unknown) => {
          logRefusal(e)
          throw e
        },
      )
    },
    console(id, lines) {
      void invoke('plugin_console_append', { id, lines }).catch(swallow(id, 'console lines'))
    },
  }
}

/** Starts the host; returns the function that stops it. */
export function startPluginHost(opts: PluginHostOptions): () => void {
  const mount = mountPoint()
  const store = storeAccessForWindow()
  const shell = isTauri ? shellRecorder() : null
  const platform = opts.platform ?? (isTauri ? 'desktop' : 'web')
  // Who puts a plugin's request to the person: the shell's own
  // confirmation on the desktop, the page's sheet on the web, nobody on
  // Shop (which refuses every request first).
  const grants = platform === 'desktop' ? storeGrantDecider(store) : platform === 'web' ? webGrantDecider : undefined
  const host: PluginHost = createPluginHost({
    scene: () => opts.bridge.getScene(),
    bridge: opts.bridge,
    platform,
    store,
    ...(grants === undefined ? {} : { grants }),
    storage: isTauri ? desktopStorage(invokeShell) : webStorage(),
    grantJson,
    openRealm: (main, onNavigate) => openRealm(mount, main, onNavigate),
    toast: (message) => opts.toast(message),
    report(id, state, reason) {
      noteHostState(id, state, reason)
      return shell?.report(id, state, reason)
    },
    console(id, lines) {
      noteConsole(id, lines)
      shell?.console(id, lines)
    },
    log: (level, message) => LogStore.log[level]('plugins', message),
    locale: () => document.documentElement.lang || navigator.language || 'en',
    theme: () => (window.matchMedia?.('(prefers-color-scheme: light)').matches ? 'light' : 'dark'),
  })
  setForwardRouter(host)
  void host.sync()

  const harness = opts.harness !== undefined ? opts.harness : (window as { __hew_test?: Record<string, unknown> }).__hew_test
  let testConn: number | null = null
  if (typeof harness === 'object' && harness !== null) {
    const hooks: PluginTestHooks = {
      async call(frame) {
        const scene = opts.bridge.getScene()
        if (scene === null) throw new Error('__hew_test.plugins.call: no document')
        if (testConn === null) {
          const conn = scene.api_connection_open()
          await host.dispatch(conn, '{"jsonrpc":"2.0","id":"test-hello","method":"hew.meta.hello","params":{"protocol":1}}')
          await host.dispatch(conn, '{"jsonrpc":"2.0","id":"test-attach","method":"hew.doc.attach","params":{}}')
          testConn = conn
        }
        const reply = await host.dispatch(testConn, frame)
        if (reply === undefined) throw new Error('__hew_test.plugins.call: the frame answered nothing')
        return reply
      },
      states: () => host.states(),
      reasons: () => host.reasons(),
      activate: (id) => host.noteUserActivation(id),
      listening: () => host.listening(),
    }
    harness.plugins = hooks
  }

  let stopNotice: (() => void) | null = null
  if (isTauri && platform === 'desktop') {
    void invokeShell('plugin_host_present', { present: true }).catch(() => undefined)
    const box = document.createElement('div')
    document.body.appendChild(box)
    const root = createRoot(box)
    root.render(createElement(DevRequestNotice, { installer: pluginInstaller(), placement: 'window' }))
    stopNotice = () => {
      root.unmount()
      box.remove()
      void invokeShell('plugin_host_present', { present: false }).catch(() => undefined)
    }
  }

  return () => {
    stopNotice?.()
    host.dispose()
    setForwardRouter(null)
    if (typeof harness === 'object' && harness !== null) delete harness.plugins
    if (testConn !== null) opts.bridge.getScene()?.api_connection_close(testConn)
    mount.remove()
    resetHostState()
  }
}

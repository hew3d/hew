/* oxlint-disable lingui/no-unlocalized-strings -- protocol words and log lines; English only until the feature graduates. */
/**
 * Starts the plugin host for one document window. Reached only through
 * `preview/loaderRoots.ts` (`loadPluginHost`), which a production build
 * compiles to `null`, and started by `App.tsx` only while the Plugins
 * Preview switch is on.
 *
 * It fills the forward router (so the live bridge, where the registry turns
 * that on, asks this host for a plugin's answer), and, when the test
 * harness exists, adds `__hew_test.plugins`.
 */

import { setForwardRouter } from '../api/forwardRouter'
import type { LiveBridgeDeps } from '../api/liveBridge'
import { isTauri } from '../io/fileHost'
import * as LogStore from '../log/LogStore'
import * as kernelModule from '../wasm/pkg/wasm_api.js'
import { createPluginHost, type PluginHost, type RealmChannel, type RealmState } from './hostCore'
import { createRealmFrame } from './realm/frame'
import { desktopStorage, webStorage } from './storageBackend'
import { invokeShell, storeAccessForWindow } from './storeAccess'

export interface PluginHostOptions {
  bridge: LiveBridgeDeps
  toast(message: string): void
}

/** What the test harness can do with the host. */
export interface PluginTestHooks {
  /** Dispatches a frame on a first-party connection of this window, as a
   *  socket client would, with forwards served by the host. */
  call(frame: string): Promise<string>
  states(): Record<string, RealmState>
}

function grantJson(id: string, document: string, namesJson: string): string {
  const fn = (kernelModule as unknown as Record<string, unknown>).plugin_grant_json
  if (typeof fn !== 'function') throw new Error('this kernel cannot grant plugins')
  return (fn as (a: string, b: string, c: string) => string)(id, document, namesJson)
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

/** Starts the host; returns the function that stops it. */
export function startPluginHost(opts: PluginHostOptions): () => void {
  const mount = mountPoint()
  const host: PluginHost = createPluginHost({
    scene: () => opts.bridge.getScene(),
    bridge: opts.bridge,
    platform: isTauri ? 'desktop' : 'web',
    store: storeAccessForWindow(),
    storage: isTauri ? desktopStorage(invokeShell) : webStorage(),
    grantJson,
    openRealm: (main, onNavigate) => openRealm(mount, main, onNavigate),
    toast: (message) => opts.toast(message),
    log: (level, message) => LogStore.log[level]('plugins', message),
    locale: () => document.documentElement.lang || navigator.language || 'en',
    theme: () => (window.matchMedia?.('(prefers-color-scheme: light)').matches ? 'light' : 'dark'),
  })
  setForwardRouter(host)
  void host.sync()

  const harness = (window as { __hew_test?: Record<string, unknown> }).__hew_test
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
    }
    harness.plugins = hooks
  }

  return () => {
    host.dispose()
    setForwardRouter(null)
    if (typeof harness === 'object' && harness !== null) delete harness.plugins
    if (testConn !== null) opts.bridge.getScene()?.api_connection_close(testConn)
    mount.remove()
  }
}

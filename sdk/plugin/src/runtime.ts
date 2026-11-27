/**
 * The SDK runtime's entry, the first part of the one script a realm Worker
 * runs: it locks the Worker's global scope down (`lockdown.ts`), defines
 * `globalThis.hew` and `globalThis.HewRefusal`, then the plugin's `main`
 * (appended after it by the boot script) runs.
 *
 * The realm port arrives in the first `{ t: 'port' }` message the boot
 * script posts to the Worker. Calls the plugin makes before that wait in
 * the runtime's outbox.
 */

import { createRuntime, HewRefusal } from './core'
import { lockdown } from './lockdown'

// Before anything else, and so before the plugin's `main`.
lockdown(globalThis)

interface WorkerScope {
  addEventListener(type: 'message', fn: (ev: MessageEvent) => void): void
  addEventListener(type: 'error', fn: (ev: ErrorEvent) => void): void
  addEventListener(type: 'unhandledrejection', fn: (ev: PromiseRejectionEvent) => void): void
  removeEventListener(type: 'message', fn: (ev: MessageEvent) => void): void
  console: Console
}

const scope = globalThis as unknown as WorkerScope & { hew?: unknown; HewRefusal?: unknown }

let port: MessagePort | null = null
const runtime = createRuntime({
  post(message) {
    port?.postMessage(message)
  },
  console: scope.console,
  now: () => Date.now(),
})

// Until the port arrives, `post` has nowhere to go; the runtime keeps
// everything but `pong` in its outbox until the host's hello, and the host
// sends nothing before the port is in place.
const onPort = (ev: MessageEvent): void => {
  const data = ev.data as { t?: unknown } | null
  if (port !== null || data === null || typeof data !== 'object' || data.t !== 'port' || ev.ports.length !== 1) return
  scope.removeEventListener('message', onPort)
  port = ev.ports[0]
  port.onmessage = (e: MessageEvent) => runtime.receive(e.data)
  port.start()
}
scope.addEventListener('message', onPort)

scope.addEventListener('error', (ev) => {
  const err = ev.error as unknown
  runtime.reportError(
    err instanceof Error ? `${err.name}: ${err.message}` : String(ev.message),
    err instanceof Error ? err.stack : undefined,
  )
})
scope.addEventListener('unhandledrejection', (ev) => {
  const reason = ev.reason as unknown
  scope.console.error('an unhandled promise rejection:', reason instanceof Error ? `${reason.name}: ${reason.message}` : reason)
})

scope.hew = runtime.hew
scope.HewRefusal = HewRefusal

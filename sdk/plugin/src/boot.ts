/**
 * The boot script of a plugin realm's sandboxed frame. Built into
 * `app/src/plugins/boot/boot-1.js`, which the host places between the
 * `<script>` tags of the frame's `srcdoc`; both the page's policy and the
 * frame's own pin it by SHA-256, so it carries no per-instance data: the
 * nonce and the realm kind travel in `<meta>` elements beside it.
 *
 * What it does, and nothing more:
 *
 *  1. Posts `{ hew: 'ready', nonce }` to the parent page.
 *  2. Takes the control port from the parent's `{ hew: 'port', nonce }`
 *     message (only from `window.parent`, only with the right nonce).
 *  3. On `{ t: 'boot', code }` with the realm port alongside, starts one
 *     classic Worker from a `blob:` whose text is the SDK runtime followed
 *     by the plugin's `main`, hands it the realm port, and answers
 *     `{ t: 'booted' }`.
 *  4. Reports the Worker's uncaught errors on the control port.
 *
 * The runtime and `main` are one blob because the realm Worker inherits the
 * page's policy, whose `script-src` has no `blob:`: a Worker cannot
 * `importScripts` or `import()` a second blob, only be constructed from one
 * (`worker-src blob:`). Nothing here evaluates code in the frame itself.
 */

declare const __HEW_RUNTIME__: string

;(() => {
  'use strict'
  const meta = (name: string): string => document.querySelector(`meta[name="${name}"]`)?.getAttribute('content') ?? ''
  const nonce = meta('hew-nonce')
  if (!/^[0-9a-f]{32}$/.test(nonce) || meta('hew-realm') !== 'background') return

  let control: MessagePort | null = null
  const fail = (message: unknown): void => {
    control?.postMessage({ t: 'error', message: String(message).slice(0, 2000) })
  }

  const onBoot = (ev: MessageEvent): void => {
    const m = ev.data as { t?: unknown; code?: unknown } | null
    if (control === null || m === null || typeof m !== 'object' || m.t !== 'boot') return
    if (!(m.code instanceof ArrayBuffer) || ev.ports.length !== 1) return
    // Once: a second boot message is ignored.
    control.onmessage = null
    let main: string
    try {
      main = new TextDecoder('utf-8', { fatal: true }).decode(m.code)
    } catch {
      fail("the plugin's main file is not UTF-8 text")
      return
    }
    let worker: Worker
    try {
      const url = URL.createObjectURL(new Blob([__HEW_RUNTIME__, '\n;\n', main], { type: 'text/javascript' }))
      worker = new Worker(url)
    } catch (e) {
      fail(e instanceof Error ? `${e.name}: ${e.message}` : e)
      return
    }
    worker.onerror = (e: ErrorEvent) => fail(e.message || 'the plugin stopped with an error')
    worker.onmessageerror = () => fail('the plugin sent a message that could not be read')
    worker.postMessage({ t: 'port' }, [ev.ports[0]])
    control.postMessage({ t: 'booted' })
  }

  window.addEventListener('message', function once(ev: MessageEvent) {
    if (ev.source !== window.parent) return
    const d = ev.data as { hew?: unknown; nonce?: unknown } | null
    if (d === null || typeof d !== 'object' || d.hew !== 'port' || d.nonce !== nonce || ev.ports.length !== 1) return
    window.removeEventListener('message', once)
    control = ev.ports[0]
    control.onmessage = onBoot
    control.start()
  })
  window.parent.postMessage({ hew: 'ready', nonce }, '*')
})()

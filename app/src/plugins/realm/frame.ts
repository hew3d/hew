/* oxlint-disable lingui/no-unlocalized-strings -- protocol words, policy text and error names; nothing here is shown to a person. */
/**
 * A plugin realm's frame: a sandboxed `srcdoc` iframe (`allow-scripts` only,
 * so its origin is opaque) whose one inline script is the hash-pinned boot
 * script, and the handshake that gives it a control port.
 *
 * The page trusts nothing the frame says until it has checked, on the
 * window `message` event, that the message came from this frame's own
 * window (`event.source`) and carries the nonce written into this frame's
 * `srcdoc`. After that the frame and its Worker are reached only over the
 * two `MessagePort`s made here: the control port to the boot script, and
 * the realm port the boot script hands to the Worker.
 */

import bootSource from '../boot/boot-1.js?raw'
import bootHashFile from '../boot/boot-1.hash?raw'

/** The base64 SHA-256 of the boot script, as both policies pin it. */
export const BOOT_HASH = bootHashFile.trim()

/** The boot script, exactly as it goes between the `<script>` tags. */
export const BOOT_SOURCE: string = bootSource

/**
 * The frame's own policy, applied by a `<meta>` before the boot script.
 * The page's policy applies too (a `srcdoc` frame inherits it), so a script
 * runs only when both allow it: the boot script by its hash, and nothing
 * else (the page allows no `blob:` script). Workers come from `blob:`.
 */
export const REALM_CSP =
  `default-src 'none'; script-src blob: 'sha256-${BOOT_HASH}' 'wasm-unsafe-eval'; ` +
  `worker-src blob:; style-src blob: 'unsafe-inline'; img-src blob: data:; font-src blob: data:; ` +
  `media-src blob:; connect-src 'none'; frame-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'`

/** 16 random bytes, hex: the boot script accepts nothing else. */
export function makeNonce(): string {
  return Array.from(crypto.getRandomValues(new Uint8Array(16)), (b) => b.toString(16).padStart(2, '0')).join('')
}

/** The frame's whole document. `nonce` is hex, so nothing in it needs escaping. */
export function realmSrcdoc(nonce: string): string {
  if (!/^[0-9a-f]{32}$/.test(nonce)) throw new Error('realm nonce must be 32 hex digits')
  return (
    '<!doctype html><html><head>' +
    `<meta http-equiv="Content-Security-Policy" content="${REALM_CSP}">` +
    `<meta name="hew-nonce" content="${nonce}">` +
    '<meta name="hew-realm" content="background">' +
    `<script>${BOOT_SOURCE}</script>` +
    '</head><body></body></html>'
  )
}

export interface RealmFrame {
  frame: HTMLIFrameElement
  /** To and from the boot script. */
  control: MessagePort
  /** Frame `load` events after the first: anything above zero means the
   *  frame navigated, which a realm never does. */
  readonly extraLoads: number
  /** Removes the frame (which ends its Worker) and closes the control port. */
  remove(): void
}

export interface FrameOptions {
  /** How long the frame has to say it is ready. */
  handshakeMs: number
  /** Called once if the frame loads a second document. */
  onNavigate(): void
}

/**
 * Creates the frame under `mount` and completes the handshake: resolves
 * with the control port once the boot script has taken it, rejects with
 * `handshake_timeout` (and removes the frame) when it never says ready.
 */
export function createRealmFrame(mount: HTMLElement, opts: FrameOptions): Promise<RealmFrame> {
  const nonce = makeNonce()
  const frame = document.createElement('iframe')
  frame.setAttribute('sandbox', 'allow-scripts')
  frame.setAttribute('aria-hidden', 'true')
  frame.setAttribute('tabindex', '-1')
  frame.setAttribute('referrerpolicy', 'no-referrer')
  frame.style.cssText = 'position:absolute;width:0;height:0;border:0;visibility:hidden'
  let loads = 0
  let removed = false
  frame.addEventListener('load', () => {
    loads++
    if (loads > 1) opts.onNavigate()
  })
  frame.srcdoc = realmSrcdoc(nonce)
  mount.appendChild(frame)

  return new Promise<RealmFrame>((resolve, reject) => {
    const remove = (): void => {
      if (removed) return
      removed = true
      frame.remove()
    }
    const timer = setTimeout(() => {
      globalThis.window?.removeEventListener('message', onReady)
      remove()
      reject(new Error('handshake_timeout'))
    }, opts.handshakeMs)

    function onReady(ev: MessageEvent): void {
      // Only this frame's own window, in its opaque origin, with its nonce.
      const win = frame.contentWindow
      if (win === null || ev.source !== win) return
      if (ev.origin !== 'null') return
      const d = ev.data as { hew?: unknown; nonce?: unknown } | null
      if (d === null || typeof d !== 'object' || d.hew !== 'ready' || d.nonce !== nonce) return
      window.removeEventListener('message', onReady)
      clearTimeout(timer)
      const channel = new MessageChannel()
      win.postMessage({ hew: 'port', nonce }, '*', [channel.port2])
      resolve({
        frame,
        control: channel.port1,
        get extraLoads() {
          return Math.max(0, loads - 1)
        },
        remove() {
          channel.port1.onmessage = null
          channel.port1.close()
          remove()
        },
      })
    }
    window.addEventListener('message', onReady)
  })
}

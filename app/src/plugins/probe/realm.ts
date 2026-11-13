/* oxlint-disable lingui/no-unlocalized-strings -- dev-only probe: protocol strings and scripts for the engine record, never user-facing */
import { BOOT_HASH_LITERAL, BOOT_PROBE_SOURCE } from './bootProbe'

/**
 * The realm policy (spec section 8.2), verbatim; the real realm (P3) uses the
 * same string. The boot script is pinned by hash; everything else is blob:.
 */
export const REALM_CSP =
  `default-src 'none'; script-src blob: 'sha256-${BOOT_HASH_LITERAL}' 'wasm-unsafe-eval'; ` +
  `worker-src blob:; style-src blob: 'unsafe-inline'; img-src blob: data:; font-src blob: data:; ` +
  `media-src blob:; connect-src 'none'; frame-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'`

export type RealmMode = 'background' | 'surface'

export function probeSrcdoc(opts: { nonce: string; mode: RealmMode }): string {
  return (
    '<!doctype html><html><head>' +
    `<meta http-equiv="Content-Security-Policy" content="${REALM_CSP}">` +
    `<meta name="hew-nonce" content="${opts.nonce}">` +
    `<meta name="hew-realm" content="${opts.mode}">` +
    `<script>${BOOT_PROBE_SOURCE}</script>` +
    '</head><body></body></html>'
  )
}

/** 16 random bytes, hex. */
export function makeNonce(): string {
  const b = crypto.getRandomValues(new Uint8Array(16))
  return Array.from(b, (x) => x.toString(16).padStart(2, '0')).join('')
}

// oxlint-disable-next-line typescript/no-explicit-any
export type PortMessage = Record<string, any> & { t: string }

export interface RealmHandle {
  frame: HTMLIFrameElement
  port: MessagePort
  nonce: string
  /** The `hello_ack` the boot script sent. */
  hello: PortMessage
  /** Every message received on the port so far, in order. */
  log: PortMessage[]
  /** Frame `load` events after the first (the kill-rule signal). */
  readonly loadCount: number
  send(msg: unknown, transfer?: Transferable[]): void
  /** Sends `msg`, then resolves with the first message arriving after it whose `t` equals `replyType`. */
  request(msg: unknown, replyType: string, timeoutMs: number): Promise<PortMessage>
  /** Resolves with the first message matching `pred`, looking at the log from index `from`. */
  waitFor(pred: (m: PortMessage) => boolean, timeoutMs: number, from?: number): Promise<PortMessage>
}

export interface RealmOptions {
  /** Handshake timeout; the stage fixes 5 s, tests shorten it. */
  handshakeMs?: number
}

export function createRealmFrame(
  mode: RealmMode,
  parent: HTMLElement,
  opts: RealmOptions = {},
): Promise<RealmHandle> {
  const nonce = makeNonce()
  const frame = document.createElement('iframe')
  frame.setAttribute('sandbox', mode === 'surface' ? 'allow-scripts allow-forms' : 'allow-scripts')
  let loads = 0
  frame.addEventListener('load', () => {
    loads++
  })
  frame.srcdoc = probeSrcdoc({ nonce, mode })
  parent.appendChild(frame)

  return new Promise<RealmHandle>((resolve, reject) => {
    const timer = setTimeout(() => {
      // The timer can outlive the page (a test environment torn down first).
      globalThis.window?.removeEventListener('message', onReady)
      reject(new Error('handshake_timeout'))
    }, opts.handshakeMs ?? 5000)

    function onReady(ev: MessageEvent): void {
      if (ev.source !== frame.contentWindow) return
      const d = ev.data
      if (!d || d.hew !== 'ready' || d.nonce !== nonce) return
      window.removeEventListener('message', onReady)

      const ch = new MessageChannel()
      const log: PortMessage[] = []
      const waiters = new Set<(m: PortMessage) => void>()
      ch.port1.onmessage = (e) => {
        const m = e.data as PortMessage
        log.push(m)
        for (const w of [...waiters]) w(m)
      }
      const waitFor = (
        pred: (m: PortMessage) => boolean,
        ms: number,
        from = 0,
      ): Promise<PortMessage> =>
        new Promise((res, rej) => {
          const hit = log.slice(from).find(pred)
          if (hit) return res(hit)
          const t = setTimeout(() => {
            waiters.delete(w)
            rej(new Error('reply_timeout'))
          }, ms)
          const w = (m: PortMessage): void => {
            if (!pred(m)) return
            clearTimeout(t)
            waiters.delete(w)
            res(m)
          }
          waiters.add(w)
        })

      // hello_ack must be awaited before the port is returned.
      waitFor((m) => m.t === 'hello_ack', opts.handshakeMs ?? 5000).then(
        (hello) => {
          clearTimeout(timer)
          const send = (msg: unknown, transfer: Transferable[] = []): void =>
            ch.port1.postMessage(msg, transfer)
          resolve({
            frame,
            port: ch.port1,
            nonce,
            hello,
            log,
            get loadCount() {
              return Math.max(0, loads - 1)
            },
            send,
            request(msg, replyType, timeoutMs) {
              const p = waitFor((m) => m.t === replyType, timeoutMs, log.length)
              send(msg)
              return p
            },
            waitFor,
          })
        },
        reject,
      )
      frame.contentWindow!.postMessage({ hew: 'port', nonce }, '*', [ch.port2])
    }
    window.addEventListener('message', onReady)
  })
}

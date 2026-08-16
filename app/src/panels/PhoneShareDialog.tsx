/**
 * PhoneShareDialog — File ▸ Open on Phone… (docs/design/shop-mode.md §4,
 * workers/share-relay/README.md): encrypts the document client-side,
 * uploads the ciphertext to the configured relay's one-shot dead-drop, and
 * shows a QR code (rendered natively via `qr_svg`, `qr.rs`) encoding a
 * `<origin>/#recv=…` URL that carries the drop's token and decryption key.
 * The server never sees plaintext and never sees the key — see
 * `shareCrypto.ts`'s module doc for the exact wire format and
 * `workers/share-relay/README.md`'s "Security model" for the full
 * argument.
 *
 * Which server: Settings ▸ Advanced ▸ Server (`settings/server.ts`) — Hew
 * cloud (app.hew3d.com) or a self-hosted origin, optionally with an upload
 * key. Every relay request goes through the Rust relay client
 * (`io/relayClient.ts` → `relay_client.rs`), which reads that setting itself
 * and derives `<origin>/relay/…`; this dialog never handles a relay URL and
 * only derives the QR text (`<origin>/#recv=…`) from the same setting. Rust
 * also does TLS through the platform verifier, so a self-hoster's own CA
 * trusted in the OS keychain works. Errors arrive typed (`RelayError.kind`)
 * and are worded specifically below.
 *
 * Tauri-only — `qr_svg` and the relay commands are Tauri commands, and the
 * web editor doesn't offer this yet (App.tsx only renders the File ▸ Open on
 * Phone… menu item under `isTauri`). `getDocument` supplies the exact bytes
 * + display name `Save` would write (App.tsx already has that path via
 * `scene.save()`); this dialog never touches the kernel itself, the same
 * "renders chrome, the caller owns the data" split ExportDialog/
 * StlUnitsDialog use.
 *
 * Closing the dialog needs nothing torn down server-side (unlike the old
 * LAN-server design, which owned a listening socket) — it best-effort
 * invalidates the uploaded drop with a `DELETE /drop/<token>` instead, so
 * an abandoned QR can't be scanned later. That's advisory, not a security
 * boundary: the drop already expires on its own (10 minutes, or the first
 * successful GET, whichever comes first).
 *
 * Once `ready`, this dialog also polls the relay every `POLL_INTERVAL_MS`
 * with a non-consuming `HEAD /drop/<token>` (`workers/share-relay/src/
 * handlers.ts`'s `handlePeek` — a plain `GET` would race the phone's own
 * pickup GET and could win, destroying the drop the phone was about to
 * read). A 404 means the token is gone; whether that's a genuine pickup or
 * a TTL expiry is told apart purely by elapsed wall-clock time against
 * `TTL_MS` — there is no separate "why" signal from the relay (by design,
 * see its README's "no information leak" note), so this is the best this
 * dialog can do without one. A pickup auto-closes the dialog after a brief
 * confirmation; an expiry leaves it open with a message, same as any other
 * error state here.
 */

import { useCallback, useEffect, useState } from 'react'
import { encrypt, generateKey, toBase64Url } from '../io/shareCrypto'
import { receiveUrlFor } from '../io/shareRelay'
import { RelayError, relayDelete, relayIdentity, relayPeek, relayPut } from '../io/relayClient'
import { effectiveOrigin, getServerSetting } from '../settings/server'

export interface PhoneShareDocument {
  bytes: Uint8Array
  /** Display name, `.hew` suffix included — same convention `docSession`
   *  names carry elsewhere (`saveAsDocument`'s `suggestedName`). */
  name: string
}


/** Matches share-relay's own `MAX_BYTES` (workers/share-relay/src/
 *  handlers.ts) — checked here too so an oversized upload fails instantly
 *  with a clear message instead of waiting on a round trip to learn the
 *  same thing from a 413. The FALLBACK: the relay's identity route reports
 *  its real cap (a self-hosted relay may run a smaller one), which wins
 *  when available. */
const MAX_UPLOAD_BYTES = 32 * 1024 * 1024

/** How often to poll the relay for pickup once a QR is up. Frequent enough
 *  to feel immediate, cheap enough (one `HEAD` every 2s, only while this
 *  dialog is open) that it's not worth debouncing further. */
const POLL_INTERVAL_MS = 2000

/** Matches share-relay's own drop TTL (workers/share-relay/src/dropStore.ts's
 *  `TTL_MS`) — mirrored here (not imported; the Worker and this app are
 *  separate deployables) so a 404 that arrives after this much time has
 *  elapsed since upload is attributed to expiry rather than pickup. The
 *  FALLBACK, like `MAX_UPLOAD_BYTES`: the identity route's `ttlMs` wins. */
const DROP_TTL_MS = 10 * 60 * 1000

/** Safety margin subtracted from `DROP_TTL_MS` when telling a genuine expiry
 *  apart from a real pickup (shop-mode playtest adversarial review): elapsed
 *  is measured from the client's own post-PUT timestamp against a mirrored —
 *  not server-authoritative — TTL, so without a margin a drop that expired
 *  unread right at the boundary could be misreported as "Opened on your
 *  phone". A real pickup happens within seconds of upload, far below this
 *  threshold, so biasing the final window toward "expired" is safe: one poll
 *  interval plus a generous clock-skew allowance. */
const EXPIRY_MARGIN_MS = POLL_INTERVAL_MS + 10_000

type PhoneShareState =
  | { kind: 'starting' }
  | {
      kind: 'ready'
      url: string
      qrSvg: string
      token: string
      uploadedAt: number
      /** The relay's own TTL (identity route), for the expiry-vs-pickup call. */
      ttlMs: number
      /** Shown under the QR when the server is not the Hew cloud. */
      serverHost: string | null
    }
  | { kind: 'picked-up' }
  | { kind: 'expired' }
  | { kind: 'error'; message: string }

interface PhoneShareDialogProps {
  /**
   * Called once, at mount, to get the bytes to share. `null` means there
   * is nothing to share — handled defensively here even though the
   * caller's File ▸ Open on Phone… menu item is already disabled on an
   * empty document, rather than assuming that gate can never be bypassed.
   */
  getDocument: () => PhoneShareDocument | null
  /** Abort/close the dialog (Escape, backdrop click, or the Close button —
   *  all three call this the same way; see the module doc for why that's
   *  also what best-effort invalidates the uploaded drop). */
  onClose: () => void
}

const OVERLAY_STYLE: React.CSSProperties = {
  position: 'fixed',
  inset: 0,
  background: 'var(--backdrop-dim, rgba(0,0,0,0.6))',
  display: 'flex',
  alignItems: 'center',
  justifyContent: 'center',
  zIndex: 2000,
}

const DIALOG_STYLE: React.CSSProperties = {
  background: 'var(--surface-overlay, #2a2a2a)',
  border: '1px solid var(--border-strong, #4a4a4a)',
  borderRadius: 'var(--radius-control, 6px)',
  boxShadow: 'var(--shadow-palette, 0 8px 32px rgba(0,0,0,0.6))',
  padding: '20px 24px',
  minWidth: '320px',
  maxWidth: '400px',
  fontFamily: 'var(--font-family-ui, system-ui, sans-serif)',
  color: 'var(--text-secondary, #ddd)',
}

const HEADING_STYLE: React.CSSProperties = {
  fontSize: '15px',
  fontWeight: 600,
  color: 'var(--text-primary, #eee)',
  marginBottom: '14px',
}

const HINT_STYLE: React.CSSProperties = {
  fontSize: 'var(--font-size-body, 12px)',
  color: 'var(--text-tertiary, #ccc)',
  textAlign: 'center',
  margin: '12px 0 0',
}

const URL_INPUT_STYLE: React.CSSProperties = {
  width: '100%',
  padding: '7px 8px',
  background: 'var(--surface-input, #1c1c1c)',
  color: 'var(--text-primary, #eee)',
  border: '1px solid var(--border-strong, #4a4a4a)',
  borderRadius: 'var(--radius-control, 4px)',
  fontSize: 'var(--font-size-body, 12px)',
  fontFamily: 'var(--font-family-mono, monospace)',
  boxSizing: 'border-box',
}

const QR_WRAP_STYLE: React.CSSProperties = {
  display: 'flex',
  justifyContent: 'center',
  padding: '8px 0 14px',
}

const STATUS_STYLE: React.CSSProperties = {
  fontSize: 'var(--font-size-body, 12px)',
  color: 'var(--text-tertiary, #ccc)',
  textAlign: 'center',
  padding: '24px 0',
}

const ERROR_STYLE: React.CSSProperties = {
  fontSize: 'var(--font-size-body, 12px)',
  color: 'var(--danger-text, #e88)',
  padding: '12px 0',
  whiteSpace: 'pre-wrap',
}

const SUCCESS_STYLE: React.CSSProperties = {
  fontSize: 'var(--font-size-body, 12px)',
  color: 'var(--text-secondary, #ddd)',
  textAlign: 'center',
  padding: '24px 0',
}

const CHECKMARK_STYLE: React.CSSProperties = {
  fontSize: '28px',
  lineHeight: 1,
  color: 'var(--success-text, #8c8)',
  marginBottom: '8px',
}

const BUTTON_ROW_STYLE: React.CSSProperties = {
  display: 'flex',
  justifyContent: 'flex-end',
  marginTop: '16px',
}

const CLOSE_BUTTON_STYLE: React.CSSProperties = {
  padding: '6px 20px',
  background: 'var(--surface-input, #444)',
  color: 'var(--text-primary, #eee)',
  border: '1px solid var(--border-strong, transparent)',
  borderRadius: 'var(--radius-control, 4px)',
  fontSize: 'var(--font-size-menu-item, 13px)',
  fontFamily: 'var(--font-family-ui, system-ui, sans-serif)',
  cursor: 'pointer',
}

/** Turns a thrown upload/QR error into a message worth showing a user. The
 *  Rust relay client classifies every failure (`RelayError.kind`, docs/design/
 *  self-hosting-relay.md §3's error mapping); `serverHost` is null for the
 *  Hew cloud, else the self-hosted host the messages can name. */
export function describeUploadError(err: unknown, serverHost: string | null, maxBytes: number): string {
  if (err instanceof RelayError) {
    const where = serverHost === null ? 'the share server' : serverHost
    switch (err.kind) {
      case 'unreachable':
        return serverHost === null
          ? 'Could not reach the share server — check your internet connection and try again.'
          : `Could not reach ${where} — check your connection and the server address in Settings ▸ Advanced.`
      case 'tls':
        return `The server's certificate isn't trusted by this computer — install its certificate authority in the system keychain, then try again.`
      case 'unauthorized':
        return 'The server rejected the upload key — check Settings ▸ Advanced.'
      case 'full':
        return 'The relay is full — try again in a minute, or ask its admin to raise the memory cap.'
      case 'tooLarge':
        return `This document is too large to share this way (${Math.floor(maxBytes / (1024 * 1024))} MB max).`
      case 'notARelay':
        return serverHost === null
          ? 'The share server isn’t answering as a Hew relay — try again later.'
          : `Reachable, but ${where} isn’t serving a Hew relay at /relay/ — check Settings ▸ Advanced.`
      case 'status':
        return `Could not reach ${where} (status ${err.status ?? '?'}).`
      case 'invalidOrigin':
      case 'io':
        return err.message
    }
  }
  return err instanceof Error ? err.message : String(err)
}

/** The host name to show for a non-cloud origin, or null for the cloud. */
function hostForDisplay(origin: string, isCloud: boolean): string | null {
  if (isCloud) return null
  try {
    return new URL(origin).host
  } catch {
    return origin
  }
}

export function PhoneShareDialog({ getDocument, onClose }: PhoneShareDialogProps) {
  const [state, setState] = useState<PhoneShareState>({ kind: 'starting' })

  // Encrypts and uploads on mount; best-effort invalidates the drop on
  // unmount — regardless of which of the three close triggers caused it
  // (module doc). Runs exactly once: a fresh "Open on Phone…" click always
  // mounts a brand new dialog instance (App.tsx's `phoneShareOpen &&`
  // conditional render), so there is nothing for this effect to re-run for.
  useEffect(() => {
    let cancelled = false
    let uploadedToken: string | null = null
    const doc = getDocument()
    if (doc === null) {
      setState({ kind: 'error', message: 'Nothing to share yet — the document is empty.' })
      return
    }
    // Resolved before anything else so the error wording can name the
    // server even when the very first request fails.
    let serverHost: string | null = null
    let maxBytes = MAX_UPLOAD_BYTES
    void (async () => {
      try {
        const setting = await getServerSetting()
        const appOrigin = effectiveOrigin(setting)
        serverHost = hostForDisplay(appOrigin, setting.mode === 'cloud')

        // Fast fail against the contract's cap before encrypting anything;
        // the relay's own (possibly smaller) cap is checked again below.
        if (doc.bytes.byteLength > MAX_UPLOAD_BYTES) {
          throw new RelayError('tooLarge', 'too large')
        }

        // Identity first (docs/design/self-hosting-relay.md §2): the relay's
        // real size cap and TTL replace the mirrored fallbacks above, and a
        // wrong server address or an untrusted certificate fails HERE with
        // its specific message rather than as a mysterious upload status.
        // Runs concurrently with the encryption. A server that answers but
        // is NOT a relay at /relay/ (a proxy that forwards /relay/drop but
        // not the identity GET, an older relay without the route) degrades
        // to the mirrored constants — the design's stated fallback — and
        // the upload proceeds; only a transport/TLS failure short-circuits,
        // since the PUT would fail the same way.
        const key = generateKey()
        const [identity, ciphertext] = await Promise.all([
          relayIdentity().catch((err: unknown) => {
            if (err instanceof RelayError && err.kind === 'notARelay') return null
            throw err
          }),
          encrypt(key, doc.bytes),
        ])
        maxBytes = identity?.maxBytes ?? MAX_UPLOAD_BYTES
        const ttlMs = identity?.ttlMs ?? DROP_TTL_MS

        if (doc.bytes.byteLength > maxBytes) {
          throw new RelayError('tooLarge', 'too large')
        }
        if (identity?.auth === 'bearer' && setting.mode === 'self-hosted' && setting.uploadKey === '') {
          throw new RelayError('unauthorized', 'key required')
        }

        // The upload itself: raw bytes to the Rust command, which PUTs them
        // to `<origin>/relay/drop` with the configured key (self-hosted only).
        const { token } = await relayPut(ciphertext)
        uploadedToken = token
        const uploadedAt = Date.now()

        // See shareCrypto.ts's module doc for this grammar, including why
        // the name segment (last, urlencoded) must never be re-split on
        // its own embedded "." characters.
        const fragment = `recv=${token}.${toBase64Url(key)}.${encodeURIComponent(doc.name)}`
        const url = receiveUrlFor(appOrigin, fragment)

        const { invoke } = await import('@tauri-apps/api/core')
        const qrSvg = await invoke<string>('qr_svg', { text: url })

        if (!cancelled) setState({ kind: 'ready', url, qrSvg, token, uploadedAt, ttlMs, serverHost })
      } catch (err) {
        if (!cancelled) setState({ kind: 'error', message: describeUploadError(err, serverHost, maxBytes) })
      }
    })()
    return () => {
      cancelled = true
      // Best-effort: an upload still in flight when this fires has no
      // token yet to invalidate, and one that already succeeded gets
      // invalidated here — either way the drop never outlives the dialog
      // by more than this one fire-and-forget request (module doc). Never
      // blocks the close on network latency, and a failure here is
      // harmless: the drop expires on its own regardless (shareCrypto.ts /
      // share-relay's 10-minute TTL).
      if (uploadedToken !== null) {
        void relayDelete(uploadedToken).catch(() => {
          /* best-effort only */
        })
      }
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  // Polls the relay for pickup once the QR is up. A `HEAD` never consumes
  // the drop (module doc), so this can run indefinitely alongside whatever
  // GET the phone eventually issues without racing it. Stops polling (and
  // this effect's own cleanup clears the interval) the moment the dialog
  // leaves `ready` — either because a poll tick found the token gone, or
  // because the component is unmounting.
  const readyToken = state.kind === 'ready' ? state.token : null
  const readyUploadedAt = state.kind === 'ready' ? state.uploadedAt : null
  const readyTtlMs = state.kind === 'ready' ? state.ttlMs : DROP_TTL_MS
  useEffect(() => {
    if (readyToken === null || readyUploadedAt === null) return
    let cancelled = false
    // Whether a poll has ever CONFIRMED the drop present (a 200 from the HEAD
    // peek). A 404 is trusted as a real pickup/expiry ONLY after the drop has
    // been seen alive at least once. Rationale: a relay that predates the HEAD
    // peek route answers 404 to every HEAD, and the desktop just created this
    // drop — so a 404 on the very first look cannot be a legitimate pickup;
    // treating it as one would falsely close a still-valid QR (which is
    // exactly what a stale deploy did). If the first look 404s, the peek is
    // unsupported/unreliable: auto-close is silently disabled and the QR
    // stays up. The phone's GET handoff is unaffected either way.
    let seenAlive = false

    const interval = setInterval(() => {
      void (async () => {
        // The Rust relay client's HEAD peek; a rejection means the request
        // never got a 200/404 (offline / DNS / refused / a proxy 5xx) — a
        // missed tick, not a signal about the drop's state, so this poll is
        // simply skipped and retried next interval instead of closing or
        // erroring the dialog out.
        let answer: 'present' | 'gone'
        try {
          answer = await relayPeek(readyToken)
        } catch {
          return
        }
        if (cancelled) return
        if (answer === 'present') {
          seenAlive = true // drop confirmed present — keep polling
          return
        }

        clearInterval(interval)
        if (!seenAlive) return // 404 before the drop was ever confirmed alive:
        // the relay doesn't support the HEAD peek (or is unreliable) — leave
        // the QR up rather than report a phantom pickup.
        const elapsed = Date.now() - readyUploadedAt
        setState(elapsed >= readyTtlMs - EXPIRY_MARGIN_MS ? { kind: 'expired' } : { kind: 'picked-up' })
      })()
    }, POLL_INTERVAL_MS)

    return () => {
      cancelled = true
      clearInterval(interval)
    }
  }, [readyToken, readyUploadedAt, readyTtlMs])

  // A pickup is a success, not an error — auto-dismiss after a beat so the
  // confirmation is visible but doesn't linger. An expiry (the other 404
  // cause) does NOT auto-dismiss, matching the plain `error` state: the
  // person needs to notice and re-open.
  useEffect(() => {
    if (state.kind !== 'picked-up') return
    const timer = setTimeout(onClose, 1500)
    return () => clearTimeout(timer)
  }, [state.kind, onClose])

  const handleKeyDown = useCallback(
    (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault()
        e.stopPropagation()
        onClose()
      }
    },
    [onClose],
  )

  useEffect(() => {
    document.addEventListener('keydown', handleKeyDown)
    return () => document.removeEventListener('keydown', handleKeyDown)
  }, [handleKeyDown])

  return (
    <div style={OVERLAY_STYLE} onClick={onClose}>
      <div
        style={DIALOG_STYLE}
        onClick={(e) => e.stopPropagation()}
        role="dialog"
        aria-modal="true"
        aria-label="Open on Phone"
      >
        <div style={HEADING_STYLE}>Open on Phone</div>

        {state.kind === 'starting' && <div style={STATUS_STYLE}>Starting…</div>}

        {state.kind === 'error' && (
          <div style={ERROR_STYLE} role="alert">
            {state.message}
          </div>
        )}

        {state.kind === 'picked-up' && (
          <div style={SUCCESS_STYLE} role="status">
            <div style={CHECKMARK_STYLE} aria-hidden="true">
              ✓
            </div>
            Opened on your phone
          </div>
        )}

        {state.kind === 'expired' && (
          <div style={ERROR_STYLE} role="alert">
            This code expired — reopen to try again.
          </div>
        )}

        {state.kind === 'ready' && (
          <>
            <div style={QR_WRAP_STYLE}>
              {/* An <img> over a data: URI, not dangerouslySetInnerHTML —
                  keeps the QR's SVG markup out of this page's own DOM
                  entirely. `data:` is already allowed by the app's CSP
                  (img-src 'self' data: blob:), and the SVG text itself
                  comes only from the Rust `qrcode` crate's own renderer
                  (qr.rs's `qr_svg`), never from anything user-typed. */}
              <img
                src={`data:image/svg+xml;utf8,${encodeURIComponent(state.qrSvg)}`}
                alt="QR code to open this model on your phone"
                width={220}
                height={220}
              />
            </div>
            <input
              readOnly
              value={state.url}
              onFocus={(e) => e.currentTarget.select()}
              style={URL_INPUT_STYLE}
              aria-label="Handoff URL"
            />
            <p style={HINT_STYLE}>
              Scan from Shop Mode on your phone (From your desktop…), or with your camera.
            </p>
            {state.serverHost !== null && (
              <p style={HINT_STYLE} data-testid="phone-share-server">
                Server: {state.serverHost} — your phone must be able to reach it too.
              </p>
            )}
          </>
        )}

        <div style={BUTTON_ROW_STYLE}>
          <button style={CLOSE_BUTTON_STYLE} onClick={onClose}>
            Close
          </button>
        </div>
      </div>
    </div>
  )
}

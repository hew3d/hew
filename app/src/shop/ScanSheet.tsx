/**
 * ScanSheet — Shop Mode's in-app QR scanner, replacing
 * `ScanFromDesktopSheet.tsx`'s "point your OWN camera app at it" explainer
 * now that Shop Mode can decode a QR itself (`qrEngine.ts`: native
 * `BarcodeDetector` where the browser has it, `jsQR` otherwise). Reachable
 * from the empty state's "From your desktop…" button and the document
 * menu's "Open from desktop…" row (`DocumentMenu.tsx`), in either
 * orientation.
 *
 * Scope ends at decoding: this component only acquires the camera, runs the
 * scan loop, and — the instant a frame decodes to a valid "Open on Phone"
 * handoff (`shellMode.ts`'s `parseRecvParams`, which accepts both a full
 * `https://app.hew3d.com/#recv=…` URL and a bare `#recv=…` payload, so a
 * decode from either a same-origin QR or a repeated-scan edge case both
 * work) — stops the camera and hands the parsed params to `onDecoded`.
 * `ShopApp.tsx` owns everything after that (the actual fetch/decrypt/load,
 * shared with the boot-time `#recv=` hash path so a camera-app scan that
 * lands in Safari works identically).
 *
 * Camera lifecycle is the one thing here that is NOT allowed to have a
 * bug: every `MediaStreamTrack` this component ever acquires is stopped on
 * close, on unmount, AND on a successful decode (not just one of the
 * three) — a leaked track means the camera's hardware indicator light
 * stays lit after the sheet is gone. `stopStream` is the single choke
 * point every exit path calls.
 *
 * Same visual family as `UnitPicker.tsx` — portrait bottom sheet, landscape
 * centered 360px card, same scrim/handle/radius/shadow chrome — the camera
 * preview replaces the row list.
 */
import { useCallback, useEffect, useRef, useState } from 'react'
import { createQrEngine, type QrEngine } from './qrEngine'
import { parseRecvParams, type RecvParams } from './shellMode'
import { QrIcon } from './icons'
import type { ShopOrientation } from './orientation'

export interface ScanSheetProps {
  open: boolean
  orientation: ShopOrientation
  onClose: () => void
  /** Called once, the instant a frame decodes to a valid handoff — the
   *  camera is already stopped by the time this fires. */
  onDecoded: (params: RecvParams) => void
}

type ScanState =
  | { kind: 'requesting' }
  | { kind: 'denied' }
  | { kind: 'unavailable' }
  // The page is not a secure context, so the browser hides the camera API
  // entirely (`navigator.mediaDevices` is undefined). This is NOT "no
  // camera" — it's iOS/WebKit refusing camera access over plain http, which
  // is exactly what a LAN `vite preview` (http://<ip>:4173) is. Called out
  // separately so the message points at the real fix (open the installed
  // https app) instead of falsely claiming the device has no camera.
  | { kind: 'insecure' }
  | { kind: 'scanning'; notHew: boolean }

/** How often the scan loop grabs a frame and runs it through the decode
 *  engine — fast enough that a QR held steady in frame decodes within a
 *  couple hundred ms, cheap enough (one canvas draw + one decode call) not
 *  to visibly compete with the live video preview's own frame rate. */
const SCAN_INTERVAL_MS = 200

export function ScanSheet({ open, orientation, onClose, onDecoded }: ScanSheetProps) {
  const [state, setState] = useState<ScanState>({ kind: 'requesting' })
  const videoRef = useRef<HTMLVideoElement | null>(null)
  const streamRef = useRef<MediaStream | null>(null)
  const canvasRef = useRef<HTMLCanvasElement | null>(null)
  const engineRef = useRef<QrEngine | null>(null)
  const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null)
  // Guards against a decode landing after the scan loop's own interval was
  // ALREADY told to stop (e.g. two frames decode in quick succession before
  // the first one's stopStream() call has torn the interval down) — without
  // this, a second `onDecoded` could fire for a sheet the first call already
  // closed.
  const decodedRef = useRef(false)

  /** The one choke point every exit path (close, unmount, successful
   *  decode) calls — see the module doc's camera-lifecycle paragraph. Safe
   *  to call more than once (idempotent): a `null` stream/interval is a
   *  no-op. */
  const stopStream = useCallback(() => {
    streamRef.current?.getTracks().forEach((track) => track.stop())
    streamRef.current = null
    if (intervalRef.current !== null) {
      clearInterval(intervalRef.current)
      intervalRef.current = null
    }
  }, [])

  useEffect(() => {
    if (!open) return
    decodedRef.current = false
    setState({ kind: 'requesting' })

    if (typeof navigator === 'undefined' || navigator.mediaDevices?.getUserMedia === undefined) {
      // Distinguish "the browser hid the camera API because this isn't a
      // secure context" (plain-http origin — the LAN preview) from a genuine
      // no-camera device: `window.isSecureContext` is false only in the
      // former. `mediaDevices` being absent WHILE the context is secure is
      // the real no-camera case.
      const insecure = typeof window !== 'undefined' && window.isSecureContext === false
      setState({ kind: insecure ? 'insecure' : 'unavailable' })
      return
    }

    let cancelled = false

    async function tick(): Promise<void> {
      // `cancelled` (not just `decodedRef`) — a decode landing after the
      // effect's own cleanup ran (sheet closed, unmounted, or `open`
      // flipped false, all of which set `cancelled` via the cleanup below)
      // must never fire `onDecoded` for a caller that already tore this
      // sheet down (adversarial-review finding 7). `decodedRef` alone only
      // guarded against a SECOND decode racing a FIRST one while the sheet
      // was still open — it says nothing about whether the sheet is still
      // open at all, so a close/unmount mid-`await engine.detect(...)`
      // (the one real await in this loop) used to sail straight through.
      if (cancelled || decodedRef.current) return
      const video = videoRef.current
      const engine = engineRef.current
      if (video === null || engine === null) return
      if (video.readyState < 2 /* HAVE_CURRENT_DATA */ || video.videoWidth === 0 || video.videoHeight === 0) return
      let canvas = canvasRef.current
      if (canvas === null) {
        canvas = document.createElement('canvas')
        canvasRef.current = canvas
      }
      canvas.width = video.videoWidth
      canvas.height = video.videoHeight
      const ctx = canvas.getContext('2d', { willReadFrequently: true })
      if (ctx === null) return
      ctx.drawImage(video, 0, 0, canvas.width, canvas.height)
      const imageData = ctx.getImageData(0, 0, canvas.width, canvas.height)
      let text: string | null
      try {
        text = await engine.detect(imageData)
      } catch {
        return
      }
      // Re-check after the await — this is the actual race window (see the
      // comment above the entry check): `cancelled`/`decodedRef` may have
      // flipped while `engine.detect` was in flight.
      if (cancelled || decodedRef.current) return
      if (text === null) {
        setState((prev) => (prev.kind === 'scanning' && !prev.notHew ? prev : { kind: 'scanning', notHew: false }))
        return
      }
      const params = parseRecvParams(text)
      if (params === null) {
        setState((prev) => (prev.kind === 'scanning' && prev.notHew ? prev : { kind: 'scanning', notHew: true }))
        return
      }
      decodedRef.current = true
      stopStream()
      onDecoded(params)
    }

    void (async () => {
      // Permissions API pre-check (playtest finding 2): querying
      // `'camera'` costs nothing and never itself prompts the user — unlike
      // calling `getUserMedia` straight away, which shows the OS/browser
      // permission prompt EVERY time on a browser that hasn't (or can't)
      // remember a prior decision. A `'granted'` read here means the prompt
      // has already been settled, so `getUserMedia` below is guaranteed to
      // resolve silently; a `'denied'` read skips the acquisition entirely
      // rather than firing it just to catch the SAME denial `getUserMedia`
      // would throw anyway. `'prompt'` (undecided) — or the API being
      // absent/throwing (Safari has historically not supported the
      // `'camera'` permission name at all) — falls straight through to
      // today's unconditional `getUserMedia` call, unchanged.
      //
      // Honest limitation: iOS Safari/PWA is known not to reliably persist
      // a camera grant ACROSS APP LAUNCHES regardless of what this check
      // reports mid-session — this reduces how often the OS prompt
      // reappears, it does not guarantee eliminating it there.
      if (navigator.permissions?.query !== undefined) {
        try {
          const status = await navigator.permissions.query({ name: 'camera' as PermissionName })
          if (cancelled) return
          if (status.state === 'denied') {
            setState({ kind: 'denied' })
            return
          }
        } catch {
          // 'camera' unsupported as a Permissions-API name, or the query
          // itself threw — fall through to getUserMedia exactly as if this
          // pre-check didn't exist.
        }
      }

      let stream: MediaStream
      try {
        stream = await navigator.mediaDevices.getUserMedia({ video: { facingMode: 'environment' } })
      } catch (err) {
        if (cancelled) return
        const name = err instanceof DOMException ? err.name : ''
        setState(
          name === 'NotAllowedError' || name === 'PermissionDeniedError' ? { kind: 'denied' } : { kind: 'unavailable' },
        )
        return
      }
      if (cancelled) {
        stream.getTracks().forEach((track) => track.stop())
        return
      }
      streamRef.current = stream
      const video = videoRef.current
      if (video !== null) {
        video.srcObject = stream
        try {
          await video.play()
        } catch {
          // Autoplay quirks (rare with muted+playsInline) — the scan loop
          // below waits on video.readyState anyway, so nothing to recover.
        }
      }
      engineRef.current = await createQrEngine()
      if (cancelled) return
      setState({ kind: 'scanning', notHew: false })
      intervalRef.current = setInterval(() => void tick(), SCAN_INTERVAL_MS)
    })()

    return () => {
      cancelled = true
      stopStream()
    }
  }, [open, stopStream, onDecoded])

  const handleClose = useCallback(() => {
    stopStream()
    onClose()
  }, [stopStream, onClose])

  if (!open) return null

  const isLandscape = orientation === 'landscape'
  const scanning = state.kind === 'scanning'

  return (
    <>
      <div
        data-testid="shop-scan-scrim"
        aria-hidden="true"
        onClick={handleClose}
        style={{ position: 'absolute', inset: 0, background: 'rgba(27,26,23,.35)', zIndex: 55 }}
      />
      <div
        role="dialog"
        aria-label="Scan from desktop"
        style={
          isLandscape
            ? {
                position: 'absolute', left: '50%', transform: 'translateX(-50%)',
                top: 'max(20px, env(safe-area-inset-top))', bottom: 'max(20px, env(safe-area-inset-bottom))',
                width: '360px', zIndex: 56,
                background: 'var(--surface-sheet)', borderRadius: '18px',
                padding: '12px 10px', boxShadow: '0 18px 48px -14px rgba(27,26,23,.5)',
                overflowY: 'auto', display: 'flex', flexDirection: 'column',
              }
            : {
                position: 'absolute', left: 0, right: 0, bottom: 0, zIndex: 56,
                background: 'var(--surface-sheet)', borderRadius: '18px 18px 0 0',
                padding: '10px 10px max(20px, calc(env(safe-area-inset-bottom) + 10px))',
                boxShadow: '0 -14px 40px -12px rgba(27,26,23,.5)',
              }
        }
      >
        <div style={{ width: '40px', height: '5px', borderRadius: '3px', background: 'var(--shop-hairline)', margin: '0 auto 10px' }} />
        <div style={{ display: 'flex', alignItems: 'center', gap: '10px', padding: '0 14px 4px' }}>
          <span aria-hidden="true" style={{ display: 'flex', color: 'var(--shop-accent)' }}>
            <QrIcon size={22} />
          </span>
          <span style={{ fontFamily: 'var(--font-family-ui)', fontSize: '17px', fontWeight: 600, color: 'var(--shop-text)' }}>
            Scan from desktop
          </span>
        </div>

        <div
          data-testid="shop-scan-preview"
          style={{
            position: 'relative', margin: '10px 14px 4px', height: '260px',
            borderRadius: '14px', overflow: 'hidden',
            background: 'color-mix(in srgb, var(--shop-text) 8%, transparent)',
            display: 'flex', alignItems: 'center', justifyContent: 'center',
          }}
        >
          {/* Always mounted (never conditionally rendered) so `videoRef`
              stays live across every state transition above — the effect
              attaches `stream` to it the instant getUserMedia resolves,
              which can land before `state` itself flips to 'scanning'. */}
          <video
            ref={videoRef}
            data-testid="shop-scan-video"
            autoPlay
            playsInline
            muted
            style={{
              position: 'absolute', inset: 0, width: '100%', height: '100%',
              objectFit: 'cover', display: scanning ? 'block' : 'none',
            }}
          />
          {scanning && (
            // A subtle viewfinder frame (design's own "no other copy" —
            // just a border, no reticle graphics) — pointer-events none so
            // it never intercepts the scrim/close gestures around it.
            <div
              aria-hidden="true"
              data-testid="shop-scan-viewfinder"
              style={{
                position: 'absolute', inset: '28px', borderRadius: '16px',
                border: '2px solid color-mix(in srgb, #fff 70%, transparent)',
                boxShadow: '0 0 0 999px rgba(0,0,0,.28)', pointerEvents: 'none',
              }}
            />
          )}
          {!scanning && (
            <span aria-hidden="true" style={{ color: 'var(--shop-text-faint)', display: 'flex' }}>
              <QrIcon size={40} />
            </span>
          )}
        </div>

        <div style={{ padding: '6px 14px 4px', display: 'flex', flexDirection: 'column', gap: '4px', minHeight: '40px' }}>
          {state.kind === 'requesting' && (
            <p style={hintStyle}>Requesting camera access…</p>
          )}
          {state.kind === 'denied' && (
            <>
              <p style={hintStyle}>Camera access is off for this site.</p>
              <p style={hintStyle}>Or scan the QR with your camera app instead.</p>
            </>
          )}
          {state.kind === 'unavailable' && (
            <>
              <p style={hintStyle}>No camera is available on this device.</p>
              <p style={hintStyle}>Or scan the QR with your camera app instead.</p>
            </>
          )}
          {state.kind === 'insecure' && (
            <>
              <p style={hintStyle}>The camera needs a secure (https) connection — open the installed Hew app rather than a local preview.</p>
              <p style={hintStyle}>Or scan the QR with your camera app instead.</p>
            </>
          )}
          {scanning && (state as { kind: 'scanning'; notHew: boolean }).notHew && (
            <p data-testid="shop-scan-not-hew" style={hintStyle}>That doesn't look like a Hew code — keep scanning.</p>
          )}
          {scanning && !(state as { kind: 'scanning'; notHew: boolean }).notHew && (
            <p style={hintStyle}>Point the camera at the QR code on the desktop.</p>
          )}
        </div>

        <button
          type="button"
          onClick={handleClose}
          style={{
            display: 'block', width: 'calc(100% - 20px)', margin: '4px 10px 0', height: '48px',
            background: 'color-mix(in srgb, var(--shop-text) 7%, transparent)', color: 'var(--shop-text)',
            border: 'none', borderRadius: '13px', cursor: 'pointer',
            fontFamily: 'var(--font-family-ui)', fontSize: '15px', fontWeight: 600,
          }}
        >
          Cancel
        </button>
      </div>
    </>
  )
}

const hintStyle: React.CSSProperties = {
  fontFamily: 'var(--font-family-ui)', fontSize: '13px', lineHeight: 1.5,
  color: 'var(--shop-text-muted)', margin: 0,
}

/**
 * ScanSheet tests — the camera/permission state machine (requesting →
 * scanning/denied/unavailable) and, above everything else, the camera
 * lifecycle contract: every acquired `MediaStreamTrack` is stopped on
 * close, on unmount, and on a successful decode. jsdom has no real camera
 * or canvas 2D backend, so `navigator.mediaDevices.getUserMedia` and
 * `HTMLCanvasElement.prototype.getContext` are stubbed per test; the decode
 * step itself is driven through `qrEngine.ts`'s own test seam
 * (`installFakeQrEngine`) rather than a real BarcodeDetector/jsQR pass —
 * `qrEngine.ts` has no dedicated test file of its own since its only real
 * logic (engine selection) is exercised here, through the one component
 * that ever calls `createQrEngine`.
 */
import { render, screen, fireEvent, waitFor } from '@testing-library/react'
import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest'
import { ScanSheet } from './ScanSheet'
import { installFakeQrEngine, clearFakeQrEngine } from './qrEngine'
import * as qrEngineModule from './qrEngine'

/** A fake 2D canvas context — jsdom has no real canvas backend, so
 *  `HTMLCanvasElement.prototype.getContext('2d')` returns `null` unless
 *  stubbed. The actual pixel content is irrelevant here: every decode in
 *  this file comes from the injected fake `QrEngine`, which ignores its
 *  `ImageData` argument entirely. */
function stubCanvasContext(): void {
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue({
    drawImage: vi.fn(),
    getImageData: vi.fn(() => ({ data: new Uint8ClampedArray(4), width: 1, height: 1 })),
  } as unknown as CanvasRenderingContext2D)
}

/** Makes the scan loop's own readiness gate (`video.readyState`/
 *  `videoWidth`/`videoHeight`) pass — jsdom's `<video>` never loads real
 *  media, so these stay at their all-zero defaults unless overridden. */
function markVideoReady(video: HTMLVideoElement): void {
  Object.defineProperty(video, 'readyState', { configurable: true, value: 2 })
  Object.defineProperty(video, 'videoWidth', { configurable: true, value: 320 })
  Object.defineProperty(video, 'videoHeight', { configurable: true, value: 240 })
}

/** A fake camera stream — `getTracks()` returns spies so a test can assert
 *  `stop()` was called on every exit path (close/unmount/decode). */
function makeFakeStream() {
  const track = { stop: vi.fn() }
  const stream = { getTracks: () => [track] } as unknown as MediaStream
  return { stream, track }
}

/** Waits for the scan loop's own 200ms interval to run at least once —
 *  real timers, not fake ones (module doc: simpler than fighting fake-timer/
 *  microtask interaction for a handful of short real waits). */
async function waitOneTick(): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, 260))
}

const VALID_TOKEN = 'a'.repeat(22)
const VALID_KEY = 'k'.repeat(43)
const VALID_RECV = `#recv=${VALID_TOKEN}.${VALID_KEY}.Bench`

beforeEach(() => {
  stubCanvasContext()
})

afterEach(() => {
  clearFakeQrEngine()
  vi.restoreAllMocks()
  // @ts-expect-error — jsdom doesn't define mediaDevices at all by default;
  // deleting whatever a test stubbed in keeps the next test's "no camera"
  // baseline honest.
  delete navigator.mediaDevices
  // @ts-expect-error — jsdom doesn't define permissions either; every test
  // in this file besides the pre-check describe block below relies on
  // `navigator.permissions?.query` reading as absent/undefined (falling
  // straight through to getUserMedia, unchanged from before finding 2).
  delete navigator.permissions
})

describe('ScanSheet — permission/availability states', () => {
  it('shows "unavailable" when the browser has no mediaDevices at all', async () => {
    render(<ScanSheet open orientation="portrait" onClose={vi.fn()} onDecoded={vi.fn()} />)
    await waitFor(() => expect(screen.getByText(/no camera is available/i)).toBeInTheDocument())
    expect(screen.getByText(/scan the qr with your camera app instead/i)).toBeInTheDocument()
  })

  it('shows "denied" when getUserMedia rejects with NotAllowedError', async () => {
    const getUserMedia = vi.fn(async () => {
      throw new DOMException('denied', 'NotAllowedError')
    })
    Object.defineProperty(navigator, 'mediaDevices', { configurable: true, value: { getUserMedia } })

    render(<ScanSheet open orientation="portrait" onClose={vi.fn()} onDecoded={vi.fn()} />)
    await waitFor(() => expect(screen.getByText(/camera access is off/i)).toBeInTheDocument())
    expect(screen.getByText(/scan the qr with your camera app instead/i)).toBeInTheDocument()
  })

  it('treats a non-permission getUserMedia failure (e.g. NotFoundError) as "unavailable"', async () => {
    const getUserMedia = vi.fn(async () => {
      throw new DOMException('no camera', 'NotFoundError')
    })
    Object.defineProperty(navigator, 'mediaDevices', { configurable: true, value: { getUserMedia } })

    render(<ScanSheet open orientation="portrait" onClose={vi.fn()} onDecoded={vi.fn()} />)
    await waitFor(() => expect(screen.getByText(/no camera is available/i)).toBeInTheDocument())
  })

  it('requests `facingMode: environment` (the rear camera)', async () => {
    const { stream } = makeFakeStream()
    const getUserMedia = vi.fn(async () => stream)
    Object.defineProperty(navigator, 'mediaDevices', { configurable: true, value: { getUserMedia } })

    render(<ScanSheet open orientation="portrait" onClose={vi.fn()} onDecoded={vi.fn()} />)
    await waitFor(() => expect(getUserMedia).toHaveBeenCalledWith({ video: { facingMode: 'environment' } }))
  })
})

describe('ScanSheet — Permissions API pre-check (finding 2)', () => {
  it('a granted permission proceeds straight to getUserMedia (no extra prompt state)', async () => {
    const { stream } = makeFakeStream()
    const getUserMedia = vi.fn(async () => stream)
    Object.defineProperty(navigator, 'mediaDevices', { configurable: true, value: { getUserMedia } })
    const query = vi.fn(async () => ({ state: 'granted' }))
    Object.defineProperty(navigator, 'permissions', { configurable: true, value: { query } })

    render(<ScanSheet open orientation="portrait" onClose={vi.fn()} onDecoded={vi.fn()} />)
    await waitFor(() => expect(screen.getByTestId('shop-scan-viewfinder')).toBeInTheDocument())
    expect(query).toHaveBeenCalledWith({ name: 'camera' })
    expect(getUserMedia).toHaveBeenCalledTimes(1)
  })

  it('a denied permission shows the denied copy WITHOUT ever calling getUserMedia', async () => {
    const getUserMedia = vi.fn()
    Object.defineProperty(navigator, 'mediaDevices', { configurable: true, value: { getUserMedia } })
    const query = vi.fn(async () => ({ state: 'denied' }))
    Object.defineProperty(navigator, 'permissions', { configurable: true, value: { query } })

    render(<ScanSheet open orientation="portrait" onClose={vi.fn()} onDecoded={vi.fn()} />)
    await waitFor(() => expect(screen.getByText(/camera access is off/i)).toBeInTheDocument())
    expect(getUserMedia).not.toHaveBeenCalled()
  })

  it('a "prompt" (undecided) permission falls through to the normal getUserMedia prompt', async () => {
    const { stream } = makeFakeStream()
    const getUserMedia = vi.fn(async () => stream)
    Object.defineProperty(navigator, 'mediaDevices', { configurable: true, value: { getUserMedia } })
    const query = vi.fn(async () => ({ state: 'prompt' }))
    Object.defineProperty(navigator, 'permissions', { configurable: true, value: { query } })

    render(<ScanSheet open orientation="portrait" onClose={vi.fn()} onDecoded={vi.fn()} />)
    await waitFor(() => expect(screen.getByTestId('shop-scan-viewfinder')).toBeInTheDocument())
    expect(getUserMedia).toHaveBeenCalledTimes(1)
  })

  it('a Permissions API that throws (e.g. an unsupported "camera" name) falls through to getUserMedia unchanged', async () => {
    const { stream } = makeFakeStream()
    const getUserMedia = vi.fn(async () => stream)
    Object.defineProperty(navigator, 'mediaDevices', { configurable: true, value: { getUserMedia } })
    const query = vi.fn(async () => { throw new TypeError('unsupported permission name') })
    Object.defineProperty(navigator, 'permissions', { configurable: true, value: { query } })

    render(<ScanSheet open orientation="portrait" onClose={vi.fn()} onDecoded={vi.fn()} />)
    await waitFor(() => expect(screen.getByTestId('shop-scan-viewfinder')).toBeInTheDocument())
    expect(getUserMedia).toHaveBeenCalledTimes(1)
  })
})

describe('ScanSheet — camera lifecycle (ALWAYS stop tracks)', () => {
  it('stops every camera track on Cancel', async () => {
    const { stream, track } = makeFakeStream()
    Object.defineProperty(navigator, 'mediaDevices', {
      configurable: true,
      value: { getUserMedia: vi.fn(async () => stream) },
    })

    render(<ScanSheet open orientation="portrait" onClose={vi.fn()} onDecoded={vi.fn()} />)
    await waitFor(() => expect(screen.getByTestId('shop-scan-viewfinder')).toBeInTheDocument())
    expect(track.stop).not.toHaveBeenCalled()

    fireEvent.click(screen.getByRole('button', { name: /^cancel$/i }))
    expect(track.stop).toHaveBeenCalledTimes(1)
  })

  it('stops every camera track on unmount, even mid-acquisition', async () => {
    const { stream, track } = makeFakeStream()
    Object.defineProperty(navigator, 'mediaDevices', {
      configurable: true,
      value: { getUserMedia: vi.fn(async () => stream) },
    })

    const { unmount } = render(<ScanSheet open orientation="portrait" onClose={vi.fn()} onDecoded={vi.fn()} />)
    await waitFor(() => expect(screen.getByTestId('shop-scan-viewfinder')).toBeInTheDocument())

    unmount()
    expect(track.stop).toHaveBeenCalledTimes(1)
  })

  it('stops every camera track on tapping the scrim', async () => {
    const { stream, track } = makeFakeStream()
    Object.defineProperty(navigator, 'mediaDevices', {
      configurable: true,
      value: { getUserMedia: vi.fn(async () => stream) },
    })
    const onClose = vi.fn()

    render(<ScanSheet open orientation="portrait" onClose={onClose} onDecoded={vi.fn()} />)
    await waitFor(() => expect(screen.getByTestId('shop-scan-viewfinder')).toBeInTheDocument())

    fireEvent.click(screen.getByTestId('shop-scan-scrim'))
    expect(track.stop).toHaveBeenCalledTimes(1)
    expect(onClose).toHaveBeenCalledTimes(1)
  })

  it('never acquires a second stream across re-opens, and each open stops its OWN previous stream', async () => {
    const { stream: streamA, track: trackA } = makeFakeStream()
    const { stream: streamB, track: trackB } = makeFakeStream()
    const getUserMedia = vi.fn()
      .mockResolvedValueOnce(streamA)
      .mockResolvedValueOnce(streamB)
    Object.defineProperty(navigator, 'mediaDevices', { configurable: true, value: { getUserMedia } })

    const { rerender } = render(<ScanSheet open={false} orientation="portrait" onClose={vi.fn()} onDecoded={vi.fn()} />)
    rerender(<ScanSheet open orientation="portrait" onClose={vi.fn()} onDecoded={vi.fn()} />)
    await waitFor(() => expect(getUserMedia).toHaveBeenCalledTimes(1))
    await waitFor(() => expect(screen.getByTestId('shop-scan-viewfinder')).toBeInTheDocument())

    // Close, then re-open — the FIRST stream's track must be stopped by the
    // time the second acquisition starts (no overlap where both cameras are
    // live at once).
    rerender(<ScanSheet open={false} orientation="portrait" onClose={vi.fn()} onDecoded={vi.fn()} />)
    expect(trackA.stop).toHaveBeenCalledTimes(1)

    rerender(<ScanSheet open orientation="portrait" onClose={vi.fn()} onDecoded={vi.fn()} />)
    await waitFor(() => expect(getUserMedia).toHaveBeenCalledTimes(2))
    await waitFor(() => expect(screen.getByTestId('shop-scan-viewfinder')).toBeInTheDocument())
    expect(trackB.stop).not.toHaveBeenCalled()
  })
})

describe('ScanSheet — decoding', () => {
  async function openScanning(onDecoded: (params: unknown) => void) {
    const { stream, track } = makeFakeStream()
    Object.defineProperty(navigator, 'mediaDevices', {
      configurable: true,
      value: { getUserMedia: vi.fn(async () => stream) },
    })
    render(<ScanSheet open orientation="portrait" onClose={vi.fn()} onDecoded={onDecoded as never} />)
    await waitFor(() => expect(screen.getByTestId('shop-scan-viewfinder')).toBeInTheDocument())
    markVideoReady(screen.getByTestId('shop-scan-video') as HTMLVideoElement)
    return { track }
  }

  it('a valid Hew code decodes, calls onDecoded with the parsed params, and stops the camera', async () => {
    installFakeQrEngine(`https://app.hew3d.com/${VALID_RECV}`)
    const onDecoded = vi.fn()
    const { track } = await openScanning(onDecoded)

    await waitFor(() => expect(onDecoded).toHaveBeenCalledTimes(1))
    expect(onDecoded).toHaveBeenCalledWith({ token: VALID_TOKEN, key: VALID_KEY, name: 'Bench' })
    expect(track.stop).toHaveBeenCalledTimes(1)
  })

  it('an unrecognized QR shows a "not a Hew code" hint and keeps scanning', async () => {
    installFakeQrEngine('https://example.com/definitely-not-hew')
    const onDecoded = vi.fn()
    await openScanning(onDecoded)

    await waitOneTick()
    expect(screen.getByTestId('shop-scan-not-hew')).toBeInTheDocument()
    expect(onDecoded).not.toHaveBeenCalled()
    // Still live — the viewfinder/video are still up, not replaced by an
    // error state.
    expect(screen.getByTestId('shop-scan-viewfinder')).toBeInTheDocument()
  })

  it('recovers from a "not a Hew code" hint once a valid code enters frame', async () => {
    installFakeQrEngine('not a url at all')
    const onDecoded = vi.fn()
    await openScanning(onDecoded)
    await waitOneTick()
    expect(screen.getByTestId('shop-scan-not-hew')).toBeInTheDocument()

    installFakeQrEngine(VALID_RECV)
    await waitFor(() => expect(onDecoded).toHaveBeenCalledTimes(1))
    expect(onDecoded).toHaveBeenCalledWith({ token: VALID_TOKEN, key: VALID_KEY, name: 'Bench' })
  })

  it('an ordinary frame with nothing decodable (null) shows no hint at all', async () => {
    installFakeQrEngine(null)
    const onDecoded = vi.fn()
    await openScanning(onDecoded)
    await waitOneTick()

    expect(screen.queryByTestId('shop-scan-not-hew')).not.toBeInTheDocument()
    expect(onDecoded).not.toHaveBeenCalled()
  })

  // Adversarial-review finding 7: `decodedRef` alone only guards against a
  // SECOND decode racing a first one while the sheet is still open — it
  // says nothing about whether the sheet is still open at all. The one real
  // `await` in the scan loop (`engine.detect`) is exactly where a close/
  // unmount can land mid-flight; `installFakeQrEngine`'s own fake engine
  // resolves near-instantly, so this test injects a hand-held Promise via
  // `createQrEngine` itself to hold a decode open across the close.
  it('a decode that resolves after the sheet has closed never calls onDecoded, and the camera stays stopped', async () => {
    const { stream, track } = makeFakeStream()
    Object.defineProperty(navigator, 'mediaDevices', {
      configurable: true,
      value: { getUserMedia: vi.fn(async () => stream) },
    })

    let resolveDetect: (value: string | null) => void = () => {}
    const pendingDetect = new Promise<string | null>((resolve) => { resolveDetect = resolve })
    const createSpy = vi.spyOn(qrEngineModule, 'createQrEngine').mockResolvedValue({
      detect: vi.fn(() => pendingDetect),
    })

    const onDecoded = vi.fn()
    const { rerender } = render(<ScanSheet open orientation="portrait" onClose={vi.fn()} onDecoded={onDecoded} />)
    await waitFor(() => expect(screen.getByTestId('shop-scan-viewfinder')).toBeInTheDocument())
    markVideoReady(screen.getByTestId('shop-scan-video') as HTMLVideoElement)

    // Let the scan interval fire at least once, so `tick()` is now sitting
    // inside its `await engine.detect(...)` — the fake engine's promise is
    // deliberately still pending, so this is the exact race window the fix
    // closes.
    await waitOneTick()

    // The sheet closes — the parent flips `open` to false, exactly as
    // `ShopApp.tsx` does in response to `onClose` — WHILE that decode is
    // still in flight.
    rerender(<ScanSheet open={false} orientation="portrait" onClose={vi.fn()} onDecoded={onDecoded} />)
    expect(track.stop).toHaveBeenCalledTimes(1)

    // The in-flight decode now resolves to a VALID handoff, after the sheet
    // has already closed — it must never reach `onDecoded`, and the
    // already-stopped track must not be touched again.
    resolveDetect(`https://app.hew3d.com/${VALID_RECV}`)
    await new Promise((resolve) => setTimeout(resolve, 50))
    expect(onDecoded).not.toHaveBeenCalled()
    expect(track.stop).toHaveBeenCalledTimes(1)

    createSpy.mockRestore()
  })
})

/**
 * qrEngine — the QR-decode engine `ScanSheet.tsx` pulls one video frame
 * through at a time, factored behind one tiny interface (`QrEngine`) so the
 * caller never branches on which concrete decoder is live and an E2E spec
 * (no real camera in CI) can inject a canned result instead of either one.
 *
 * Two real engines, picked at scan-start (not per-frame — `createQrEngine`
 * is called once per `ScanSheet` open, its result reused for every
 * `detect()` call in that session):
 *
 *  - `BarcodeDetector`, when the browser has it (Chrome/Android's native
 *    decoder) — zero bundle cost, nothing to import.
 *  - `jsQR` otherwise (Safari/iOS has no `BarcodeDetector` as of this
 *    writing — the whole reason Shop Mode needs an in-app decoder at all
 *    rather than leaning on the OS's own barcode recognition). Pulled in
 *    via a DYNAMIC `import()`, so its ~40KB never lands in the app's main
 *    bundle — only a real scan session pays for it, and only once
 *    (`app/package.json`'s pinned `jsqr` dependency).
 */

export interface QrEngine {
  /** Decode one frame. `null` means "no QR code found in this frame" —
   *  never throws for an ordinary empty/blurry frame; a caller just tries
   *  the next one. */
  detect(imageData: ImageData): Promise<string | null>
}

/** Test-only override — `undefined` (the default) means "use the real
 *  BarcodeDetector/jsQR selection". Set via `installFakeQrEngine` (wired
 *  through `testHarness.ts`'s `setFakeQrDecode`, the same debug/test-build
 *  gate every other Shop Mode test hook uses) so a Playwright spec can
 *  simulate a decoded QR — or a frame with nothing decodable (`null`) —
 *  without a real camera. Read LIVE by the fake engine's own `detect`
 *  below (not captured once at engine-creation time), so a spec can change
 *  the value mid-scan: e.g. an unrecognized code first (to exercise the
 *  "not a Hew code" hint), then a valid one. */
let fakeDecodeValue: string | null | undefined = undefined

export function installFakeQrEngine(value: string | null): void {
  fakeDecodeValue = value
}

export function clearFakeQrEngine(): void {
  fakeDecodeValue = undefined
}

/** Minimal shape this module needs from the real `BarcodeDetector` API —
 *  hand-rolled rather than pulled from `lib.dom.d.ts` (not yet typed
 *  there), matching `workers/share-relay`'s own precedent of small
 *  hand-rolled shims over a full types package for a narrow surface. */
interface BarcodeDetectorLike {
  detect(source: ImageData): Promise<{ rawValue: string }[]>
}
interface BarcodeDetectorCtor {
  new (options: { formats: string[] }): BarcodeDetectorLike
}

declare global {
  interface Window {
    BarcodeDetector?: BarcodeDetectorCtor
  }
}

function nativeDetectorAvailable(): boolean {
  return typeof window !== 'undefined' && typeof window.BarcodeDetector === 'function'
}

/** Builds the engine a `ScanSheet` session should use. Called once per
 *  sheet-open, not per frame. */
export async function createQrEngine(): Promise<QrEngine> {
  if (fakeDecodeValue !== undefined) {
    // Reads the module-level variable live on every call (not a value
    // captured here) — see its own doc comment for why that matters.
    return { detect: async () => fakeDecodeValue as string | null }
  }
  if (nativeDetectorAvailable()) {
    const detector = new window.BarcodeDetector!({ formats: ['qr_code'] })
    return {
      detect: async (imageData) => {
        const results = await detector.detect(imageData)
        return results.length > 0 ? results[0].rawValue : null
      },
    }
  }
  const { default: jsQR } = await import('jsqr')
  return {
    detect: async (imageData) => {
      const result = jsQR(imageData.data, imageData.width, imageData.height)
      return result?.data ?? null
    },
  }
}

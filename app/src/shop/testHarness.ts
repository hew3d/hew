/**
 * Shop Mode's own semantic E2E harness — `window.__hew_shop_test`. A
 * deliberately tiny SIBLING of `test/harness.ts`'s editor harness, not a
 * reuse of it: Shop Mode issues zero kernel transactions (`ShopApp.tsx`'s
 * module doc), so most of the editor harness's surface (`drawBox`,
 * `pushPull`, `boolean`, `groupNodes`, …) would be actively misleading
 * here — none of it is reachable from Shop Mode's real UI, and exposing it
 * would let an E2E spec drive a mutation Shop Mode's own chrome can never
 * trigger.
 *
 * Scope: just enough to get a fixture document into Shop Mode without
 * automating a real OS file picker (Playwright can't drive one — the
 * editor's own E2E specs solve the identical problem the same way, via
 * `test/harness.ts`'s `load(bytes)`). Everything else a Shop Mode E2E spec
 * needs — visibility state, inspect-card content, isolate — is already
 * observable in the DOM (a row's eye-icon `aria-label`, the "Show all"
 * chip, `InspectCard`'s rendered text), so no state-inspection surface is
 * added here either.
 *
 * Installed only in debug/test builds (dev server, or a build with
 * VITE_HEW_TEST=1 for the Playwright E2E build) — the same gate
 * `App.tsx`'s `installTestHarness` call uses.
 */
import type { Scene } from '../wasm/loader'
import type { ViewportApi } from '../viewport/Viewport'
// Type-only: erased at compile time, so this never pulls the editor
// harness's runtime module into the web bundle — just reuses its
// `CameraPose` shape so an E2E spec's camera-pinning helper (`setCamera`)
// works identically against either harness.
import type { CameraPose } from '../test/harness'
import { installFakeQrEngine } from './qrEngine'

export interface ShopTestHarnessDeps {
  getScene: () => Scene | null
  getViewportApi: () => ViewportApi | null
  /** Load bytes through ShopApp's real open-apply path (parse + seed hidden
   *  state + push + camera restore + record-to-recents) — the same
   *  consequences a real picker/recents-tap open has. Returns false if the
   *  parse was rejected. */
  loadBytes: (bytes: Uint8Array) => boolean
}

export interface ShopTestHarness {
  /** True once the kernel has booted and a scene exists — poll this before
   *  calling anything else, same convention as the editor harness's
   *  `isReady`. */
  isReady(): boolean
  /** `bytes` as a plain number array (structured-clone-safe across the
   *  `page.evaluate` boundary, matching `test/harness.ts`'s `load`). */
  load(bytes: number[]): boolean
  /** Pin the viewport camera — needed to turn a known world point into a
   *  reliable canvas pixel for a real pointer tap (docs/dev/DEVELOPMENT.md's
   *  "pixel interaction" strategy; `test/harness.ts`'s identical method,
   *  reused here since Shop Mode's tap-to-inspect gesture is exactly what
   *  such a tap needs to exercise). View-state only — not a kernel call,
   *  so this is in scope for Shop Mode's harness unlike the drawing/editing
   *  methods the module doc excludes. */
  setCamera(pose: CameraPose): void
  /**
   * The camera's current pose (`ViewportApi.getCamera`'s own read) — the
   * complement of `setCamera` above, added for the Tape Measure loupe's
   * CONTRACT test (round-3 playtest finding 4): "OrbitControls must not
   * orbit while the loupe is engaged" is otherwise unobservable from the
   * DOM (nothing in Shop Mode's chrome reflects camera orientation), so
   * this is compared before/after a held gesture instead. A QUERY, not a
   * mutation — same rationale as `getObjectBounds`/`sessionDepth` above.
   */
  getCameraPose(): { position: [number, number, number]; target: [number, number, number]; fovDeg: number }
  /**
   * World-axis-aligned bounding box of `object`'s current mesh, as
   * `[minX,minY,minZ,maxX,maxY,maxZ]` (meters) — `test/harness.ts`'s
   * identical method, re-exposed here for exactly one purpose: proving the
   * read-only CONTRACT itself (shop-mode playtest finding 1) — that a
   * press-drag on a part in Shop Mode never moves it. A QUERY, not an
   * editing capability (nothing here mutates the scene), so it doesn't
   * reopen the module doc's "no state-inspection surface" exclusion, which
   * was about drawing/editing methods Shop Mode's real UI can never reach —
   * this is the one piece of scene state Shop Mode's design specifically
   * promises never changes, with nothing in the DOM (InspectCard shows
   * dims, never world position) able to prove that promise on its own.
   */
  getObjectBounds(id: string): [number, number, number, number, number, number]
  /**
   * The number of open group/component edit sessions (`ViewportApi.
   * sessionStack()`'s own length) — a QUERY, same rationale as
   * `getObjectBounds` above: proving the read-only CONTRACT (shop-mode
   * adversarial-review finding 2) that a double-tap on a group/instance in
   * Shop Mode never opens a session, which is otherwise unobservable —
   * ShopApp renders no session/breadcrumb chrome at all (its own `Viewport`
   * usage comment lists `onSessionChange` among the deliberately-unwired
   * callbacks), so nothing in the DOM would prove a session's absence.
   */
  sessionDepth(): number
  /**
   * `ScanSheet.tsx`'s in-app QR scanner E2E hook (`qrEngine.ts`): force
   * every frame the NEXT-opened scanner "decodes" to resolve to `value`
   * instead of running a real BarcodeDetector/jsQR pass over the camera
   * feed — CI has no camera to point at a QR code. Pass a full `#recv=…`
   * (or `https://app.hew3d.com/#recv=…`) payload to simulate a successful
   * scan, or `null` to simulate an ordinary frame with nothing decodable
   * (the steady scanning state), or any other string to exercise the "not
   * a Hew code" hint. Live, not one-shot — a spec can call this again
   * mid-scan to change what the NEXT frame decodes to (e.g. an
   * unrecognized code first, then a valid one).
   */
  setFakeQrDecode(value: string | null): void
}

declare global {
  interface Window {
    __hew_shop_test?: ShopTestHarness
  }
}

/** Install `window.__hew_shop_test`. Returns an uninstall function (for
 *  HMR / unmount). Caller gates on a debug/test build. */
export function installShopTestHarness(deps: ShopTestHarnessDeps): () => void {
  const harness: ShopTestHarness = {
    isReady: () => deps.getScene() !== null,
    load: (bytes) => deps.loadBytes(new Uint8Array(bytes)),
    setCamera: (pose) => {
      const api = deps.getViewportApi()
      if (api === null) throw new Error('__hew_shop_test: viewport not ready')
      api.setCamera(pose.position, pose.target, pose.up ?? [0, 0, 1], pose.fovDeg ?? 45)
    },
    getCameraPose: () => {
      const api = deps.getViewportApi()
      if (api === null) throw new Error('__hew_shop_test: viewport not ready')
      return api.getCamera()
    },
    getObjectBounds: (id) => {
      const scene = deps.getScene()
      if (scene === null) throw new Error('__hew_shop_test: scene not ready')
      const mesh = scene.object_mesh(BigInt(id))
      try {
        const pos = mesh.positions()
        if (pos.length < 3) throw new Error(`getObjectBounds: object ${id} has no geometry`)
        let minX = pos[0], maxX = pos[0]
        let minY = pos[1], maxY = pos[1]
        let minZ = pos[2], maxZ = pos[2]
        for (let i = 3; i < pos.length; i += 3) {
          const x = pos[i], y = pos[i + 1], z = pos[i + 2]
          if (x < minX) minX = x
          if (x > maxX) maxX = x
          if (y < minY) minY = y
          if (y > maxY) maxY = y
          if (z < minZ) minZ = z
          if (z > maxZ) maxZ = z
        }
        return [minX, minY, minZ, maxX, maxY, maxZ]
      } finally {
        mesh.free()
      }
    },
    sessionDepth: () => deps.getViewportApi()?.sessionStack().length ?? 0,
    setFakeQrDecode: (value) => installFakeQrEngine(value),
  }
  window.__hew_shop_test = harness
  return () => {
    if (window.__hew_shop_test === harness) delete window.__hew_shop_test
  }
}

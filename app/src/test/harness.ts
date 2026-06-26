/**
 * `window.__hew_test` — the semantic test harness (docs/DEVELOPMENT.md).
 *
 * Debug/test builds only. It lets a driver (Playwright in, or the console)
 * issue **semantic** actions and read state *by logic, not canvas pixels* — the
 * answer to "the viewport is an opaque WebGL canvas" (docs/DEVELOPMENT.md). Most E2E
 * tests should drive through this rather than synthesize pointer events.
 *
 * Design:
 * - Modeling actions call the same kernel ops the tools commit (`begin_ground_
 *   sketch`/`sketch_add_segment`/`extrude_region`/`push_pull`/`boolean`/…), then
 *   reconcile the app exactly as a tool commit would, so state stays faithful.
 * - **Handles cross the boundary as decimal strings, never bigint.** u64 kernel
 *   handles and the `state_hash` exceed `Number.MAX_SAFE_INTEGER` and bigint is
 *   not structured-cloneable out of `page.evaluate`; strings are exact and
 *   portable. Args accept a string and are `BigInt()`-converted internally.
 * - Picking is pixel-free: `pickFace` casts a **world-space ray** (e.g. straight
 *   down onto a top face), so `pushPull(obj, face, …)` gets a real face handle
 *   without any screen projection.
 * - `startRecording`/`takeRecording` tie the high-level (`Scene`) and
 *   low-level (`inputRecorder`) streams into one session artifact.
 *
 * Not exposed (deliberately): `selectFace`/`selectEdge`/`hoverPoint` from the
 * sketch list — the app has no *persistent* sub-element selection (faces/edges
 * are picked transiently inside tools), and `pushPull` takes an explicit face
 * from `pickFace` instead. Object-level selection is real, via `selectObjects`.
 */

import type { Scene } from '../wasm/loader'
import type { ViewportApi } from '../viewport/Viewport'
import type { NodeRef } from '../panels/treeModel'
import * as inputRecorder from '../recording/inputRecorder'
import { buildSessionRecording } from '../recording/sessionRecording'

type Vec3 = [number, number, number]

export interface CameraPose {
  position: Vec3
  target: Vec3
  up?: Vec3
  fovDeg?: number
}

/** What the harness needs from the app; all live (read at call time). */
export interface HarnessDeps {
  getScene: () => Scene | null
  getViewportApi: () => ViewportApi | null
  /** Reconcile + re-render after a mutation (the app's document-changed path). */
  reconcile: () => void
  /** Current object/group/instance selection. */
  getSelection: () => NodeRef[]
  /** Replace the selection with these object handles. */
  setSelectedObjects: (ids: bigint[]) => void
  /**
   * Reload `.hew` bytes through the app's real Open path (the same
   * `scene.load` + UI reset + viewport `notifyLoaded` re-tessellation a user's
   * File→Open runs). Returns false if the load was rejected..
   */
  loadBytes: (bytes: Uint8Array) => boolean
}

export interface HewTestHarness {
  isReady(): boolean
  // modeling
  drawRectangle(p0: Vec3, p1: Vec3): { sketch: string; region: string }
  extrudeRegion(sketch: string, region: string, distance: number): string
  /** Convenience: rectangle on the ground from p0→p1, extruded `height`. */
  drawBox(p0: Vec3, p1: Vec3, height: number): string
  pickFace(rayOrigin: Vec3, rayDir: Vec3): { object: string; face: string } | null
  pushPull(object: string, face: string, distance: number): void
  boolean(op: number, a: string, b: string): string
  deleteObject(id: string): void
  selectObjects(ids: string[]): void
  setCamera(pose: CameraPose): void
  replay(recordingJson: string): string
  // serialization ( — round-trips the live `.hew` container through the
  // app's real save/open path). Bytes cross `page.evaluate` as a plain number[]
  // (portable + structured-cloneable); a box is tiny, so the cost is moot.
  save(): number[]
  load(bytes: number[]): void
  // recording ( high + low, one artifact)
  startRecording(): void
  stopRecording(): void
  isRecording(): boolean
  takeRecording(): string
  // queries
  getStateHash(): string
  getObjectCount(): number
  getObjectIds(): string[]
  getSelection(): { kind: string; id: string }[]
  getLastError(): string | null
}

declare global {
  interface Window {
    __hew_test?: HewTestHarness
  }
}

/**
 * Install `window.__hew_test`. Returns an uninstall function (for HMR / unmount).
 * Caller gates on a debug/test build.
 */
export function installTestHarness(deps: HarnessDeps): () => void {
  let lastError: string | null = null

  const scene = (): Scene => {
    const s = deps.getScene()
    if (s === null) throw new Error('__hew_test: scene not ready')
    return s
  }

  // Run a mutation, reconcile + re-tessellate on success, and track the last
  // error. When the viewport is mounted we drive its `refreshScene` (which
  // re-tessellates the new geometry to the GPU *and* reconciles the app, exactly
  // like a tool commit) — a bare `reconcile()` updates React state but leaves the
  // canvas stale, so harness geometry would never render. Headless
  // callers with no viewport fall back to a plain reconcile.
  function act<T>(fn: (s: Scene) => T): T {
    try {
      const out = fn(scene())
      const api = deps.getViewportApi()
      if (api !== null) api.refreshScene()
      else deps.reconcile()
      lastError = null
      return out
    } catch (e) {
      lastError = e instanceof Error ? e.message : String(e)
      throw e
    }
  }
  function query<T>(fn: (s: Scene) => T): T {
    try {
      return fn(scene())
    } catch (e) {
      lastError = e instanceof Error ? e.message : String(e)
      throw e
    }
  }

  // Add the four edges of the axis-aligned rectangle p0→p1 (on p0's z plane) to
  // `sketch`, returning the first closed region handle it forms.
  function addRectangle(s: Scene, sketch: bigint, p0: Vec3, p1: Vec3): bigint {
    const z = p0[2]
    const corners: Vec3[] = [
      [p0[0], p0[1], z],
      [p1[0], p0[1], z],
      [p1[0], p1[1], z],
      [p0[0], p1[1], z],
    ]
    for (let i = 0; i < 4; i++) {
      const a = corners[i]
      const b = corners[(i + 1) % 4]
      s.sketch_add_segment(sketch, a[0], a[1], a[2], b[0], b[1], b[2])
    }
    const regions = s.sketch_regions(sketch)
    if (regions.length === 0) {
      throw new Error('drawRectangle: no closed region formed')
    }
    return regions[0]
  }

  const harness: HewTestHarness = {
    isReady: () => deps.getScene() !== null,

    drawRectangle: (p0, p1) =>
      act((s) => {
        const sketch = s.begin_ground_sketch()
        const region = addRectangle(s, sketch, p0, p1)
        return { sketch: sketch.toString(), region: region.toString() }
      }),

    extrudeRegion: (sketch, region, distance) =>
      act((s) =>
        s.extrude_region(BigInt(sketch), BigInt(region), distance).toString(),
      ),

    drawBox: (p0, p1, height) =>
      act((s) => {
        const sketch = s.begin_ground_sketch()
        const region = addRectangle(s, sketch, p0, p1)
        return s.extrude_region(sketch, region, height).toString()
      }),

    pickFace: (o, d) =>
      query((s) => {
        // FacePickJs exposes object()/face() as methods returning bigint;
        // pick_face returns undefined on a miss.
        const p = s.pick_face(o[0], o[1], o[2], d[0], d[1], d[2])
        return p ? { object: p.object().toString(), face: p.face().toString() } : null
      }),

    pushPull: (object, face, distance) => {
      act((s) => s.push_pull(BigInt(object), BigInt(face), distance))
    },

    boolean: (op, a, b) =>
      act((s) => s.boolean(op, BigInt(a), BigInt(b)).toString()),

    deleteObject: (id) => {
      act((s) => s.delete_node(0, BigInt(id))) // kind 0 = object
    },

    selectObjects: (ids) => deps.setSelectedObjects(ids.map((id) => BigInt(id))),

    setCamera: (pose) => {
      const api = deps.getViewportApi()
      if (api === null) throw new Error('__hew_test: viewport not ready')
      api.setCamera(pose.position, pose.target, pose.up ?? [0, 0, 1], pose.fovDeg ?? 45)
    },

    replay: (json) => act((s) => s.replay(json).toString()),

    save: () => query((s) => Array.from(s.save())),
    load: (bytes) => {
      // Route through the app's real Open path, not a bare `scene.load`, so the
      // viewport re-tessellates exactly as File→Open does. `loadBytes` reports
      // failure via its boolean (it has already toasted); surface it as both a
      // lastError and a throw, matching the `act` helper's contract.
      const ok = deps.loadBytes(new Uint8Array(bytes))
      if (!ok) {
        lastError = '__hew_test: load rejected'
        throw new Error(lastError)
      }
      lastError = null
    },

    startRecording: () => {
      scene().start_recording()
      inputRecorder.start()
    },
    stopRecording: () => {
      scene().stop_recording()
      inputRecorder.stop()
    },
    isRecording: () => scene().is_recording(),
    takeRecording: () =>
      buildSessionRecording(scene().take_recording(), inputRecorder.take()),

    getStateHash: () => query((s) => s.state_hash().toString()),
    getObjectCount: () => query((s) => s.object_ids().length),
    getObjectIds: () => query((s) => Array.from(s.object_ids()).map(String)),
    getSelection: () =>
      deps.getSelection().map((n) => ({ kind: n.kind, id: n.id.toString() })),
    getLastError: () => lastError,
  }

  window.__hew_test = harness
  return () => {
    if (window.__hew_test === harness) delete window.__hew_test
  }
}

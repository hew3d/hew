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
import { arcPolylineOnPlane } from '../tools/arcMath'
import { facePlaneBasis, type V3 } from '../viewport/geoHelpers'
import {
  formatLength,
  getLengthUnit,
  parseLengthToMeters,
  setLengthUnit,
  type LengthFormat,
} from '../settings/units'

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
  /** True once the kernel scene AND the viewport API are both wired — wait on
   * this before driving any harness op (camera/draw/pick need the viewport). */
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

  // -------- NEW in  --------

  /**
   * Draw a chain of line segments in a new ground sketch. Points are an ordered
   * list of positions; segments are added between consecutive pairs. Returns the
   * sketch handle (string) and all closed regions that formed (as handle strings).
   * Equivalent to what LineTool commits: begin_ground_sketch → N × sketch_add_segment.
   */
  drawLineChain(points: Vec3[]): { sketch: string; regions: string[] }

  /**
   * Draw a regular N-gon approximation of a circle in a new ground sketch.
   * `center` is the XY centroid (Z ignored), `radius` in meters, `nSegments`
   * defaults to 24 (same as CircleTool). Returns the sketch and the one closed
   * region that forms (the N-gon). Equivalent to CircleTool's commit.
   */
  drawCircle(center: Vec3, radius: number, nSegments?: number): { sketch: string; region: string }

  /**
   * Draw a faceted 2-point arc in a new ground sketch. `a`/`b` are the
   * chord endpoints (z should be 0 — ground plane), `sagitta` is the signed
   * bulge distance from the chord midpoint (positive on the CCW side of a→b,
   * viewed from +Z). With `close`, also adds the chord b→a so the arc closes
   * into a region. Equivalent to ArcTool's ground commit:
   * begin_ground_sketch → N × sketch_add_segment along `arcPolylineOnPlane`.
   */
  drawArc(a: Vec3, b: Vec3, sagitta: number, close?: boolean): { sketch: string; regions: string[] }

  /**
   * Cut `face` of `object` along a faceted 2-point arc. `a`/`b` must
   * lie on the face (endpoints on its boundary for a boundary-to-boundary
   * cut); `sagitta` is signed in the face's `facePlaneBasis(normal)` (u, v)
   * frame. Equivalent to ArcTool's face commit: one `split_face` call with
   * the arc polyline path.
   */
  drawArcOnFace(object: string, face: string, a: Vec3, b: Vec3, sagitta: number): void

  /**
   * Move `object` by a world translation (meters). Calls `transform_object` with
   * a pure-translation 3×4 affine, exactly as MoveTool commits. The handle
   * is unchanged; the document state_hash changes.
   */
  moveObject(object: string, dx: number, dy: number, dz: number): void

  /**
   * Copy `object` to a new object offset by `(dx, dy, dz)` meters. Calls
   * `duplicate_node(0, id, affine)` with a translation affine, matching the
   * "Option-drag" branch of MoveTool. Returns the new node's kind and id.
   */
  copyObject(id: string, dx: number, dy: number, dz: number): { kind: string; id: string }

  /**
   * Rotate `object` by `angleDeg` degrees around `axis` (default: Z = [0,0,1]).
   * Builds a Rodrigues rotation matrix and calls `transform_object`, matching
   * RotateTool's commit.
   */
  rotateObject(object: string, angleDeg: number, axis?: Vec3): void

  /**
   * Slice a watertight solid by a plane. `plane` is 6 floats `[px,py,pz,nx,ny,nz]`
   * (a point on the plane + its normal). Returns `[positiveId, negativeId]` as
   * decimal strings. Calls `slice_object` directly (SliceTool's commit).
   */
  sliceObject(
    object: string,
    plane: [number, number, number, number, number, number],
  ): [string, string]

  /**
   * Add a construction guide line through `(ox,oy,oz)` along direction
   * `(dx,dy,dz)`. Returns the guide handle as a decimal string. Matches
   * TapeMeasureTool's parallel-guide commit and ProtractorTool's angular-guide
   * commit.
   */
  addGuideLine(ox: number, oy: number, oz: number, dx: number, dy: number, dz: number): string

  /**
   * Add a construction guide point at `(x,y,z)`. Returns the guide handle as a
   * decimal string. Matches TapeMeasureTool's point-mode commit.
   */
  addGuidePoint(x: number, y: number, z: number): string

  /** Delete one construction guide by handle string. */
  deleteGuide(id: string): void

  /** Delete all construction guides in one undo step. */
  deleteAllGuides(): void

  /** Handles of all currently visible construction guides, as decimal strings. */
  getGuideIds(): string[]

  /**
   * Document-level undo (same kernel path as Cmd+Z). Calls `scene_undo()` and
   * reconciles the viewport. Throws if there is nothing to undo.
   */
  undo(): void

  /**
   * Document-level redo. Calls `scene_redo()` and reconciles the viewport.
   * Throws if there is nothing to redo.
   */
  redo(): void

  /** True if there is a document action to undo. */
  canUndo(): boolean

  /** True if there is a document action to redo. */
  canRedo(): boolean

  /**
   * Set the active display/parse length unit ( VCB). `format` is one
   * of `'m'|'cm'|'mm'|'arch'|'frac_in'|'dec_in'`. Writes through to
   * `setLengthUnit` (the same singleton the tools use) so subsequent
   * `formatLength` / `parseLength` calls reflect the new format.
   */
  setLengthUnit(format: string): void

  /** The currently active length format string (e.g. `'cm'`). */
  getLengthUnit(): string

  /**
   * Format `meters` (a kernel f64 length) using the current display unit, as
   * the VCB and status bar do. E.g. 1.5 m → "150 cm" when unit is cm.
   */
  formatLength(meters: number): string

  /**
   * Parse a typed length string to meters using the current unit ( VCB).
   * Returns `null` on empty/invalid input. E.g. "100 cm" → 1.0.
   */
  parseLength(input: string): number | null

  // -------- materials --------

  /**
   * Add a solid-color material to the palette and return its handle (decimal
   * string). `r`/`g`/`b`/`a` are 0–255; `a` < 255 is translucent. Wraps
   * `add_material`. Palette additions are not individually undoable — only
   * assignment (via `paintObject`/`paintFace`) is.
   */
  addMaterial(name: string, r: number, g: number, b: number, a: number): string

  /**
   * Set `object`'s base material — the color the whole solid (and faces grown
   * later by extrude/boolean) renders with, unless a face is explicitly painted.
   * Wraps `set_object_material`. Pass `null` to clear back to the renderer
   * default. Undoable.
   */
  paintObject(object: string, material: string | null): void

  /**
   * Paint a single `face` of `object` with `material`, overriding the object's
   * base material for that face. Wraps `paint_face`. Pass `null` for `material`
   * to reset the face to the (unpainted) default. Undoable.
   */
  paintFace(object: string, face: string, material: string | null): void
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

  // The kernel's "no/clear material" sentinel is u64::MAX (see paint_face /
  // set_object_material in wasm-api). Map the harness's `null` to it.
  const MATERIAL_NONE = (1n << 64n) - 1n
  const materialHandle = (m: string | null): bigint => (m === null ? MATERIAL_NONE : BigInt(m))

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
    // Ready = kernel scene live AND the viewport API wired. Both are needed
    // before callers drive the harness: drawBox/setCamera/pickFace go through the
    // viewport (setCamera throws "viewport not ready" without it). The viewport
    // registers its API a tick after the scene becomes non-null, so gating only
    // on the scene leaves a race that a slower-mounting engine (webkit) loses —
    // `waitForFunction(isReady)` then `setCamera` flaked. Gate on both.
    isReady: () => deps.getScene() !== null && deps.getViewportApi() !== null,

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

    // -------- NEW in  --------

    drawLineChain: (points) =>
      act((s) => {
        if (points.length < 2) throw new Error('drawLineChain: need at least 2 points')
        const sketch = s.begin_ground_sketch()
        const regionsAll = new Set<bigint>()
        for (let i = 0; i < points.length - 1; i++) {
          const a = points[i]
          const b = points[i + 1]
          const added = s.sketch_add_segment(sketch, a[0], a[1], a[2], b[0], b[1], b[2])
          for (const r of added.regions_created()) regionsAll.add(r)
        }
        return {
          sketch: sketch.toString(),
          regions: Array.from(regionsAll).map(String),
        }
      }),

    drawCircle: (center, radius, nSegments = 24) =>
      act((s) => {
        if (radius <= 0) throw new Error('drawCircle: radius must be positive')
        const sketch = s.begin_ground_sketch()
        const pts: Vec3[] = []
        for (let i = 0; i < nSegments; i++) {
          const theta = (2 * Math.PI * i) / nSegments
          pts.push([center[0] + radius * Math.cos(theta), center[1] + radius * Math.sin(theta), center[2]])
        }
        let lastRegion: bigint | null = null
        for (let i = 0; i < nSegments; i++) {
          const a = pts[i]
          const b = pts[(i + 1) % nSegments]
          const added = s.sketch_add_segment(sketch, a[0], a[1], a[2], b[0], b[1], b[2])
          for (const r of added.regions_created()) lastRegion = r
        }
        if (lastRegion === null) {
          // Fallback: ask the scene for any open regions
          const regions = s.sketch_regions(sketch)
          lastRegion = regions.length > 0 ? regions[0] : null
        }
        if (lastRegion === null) throw new Error('drawCircle: no region formed')
        return { sketch: sketch.toString(), region: lastRegion.toString() }
      }),

    drawArc: (a, b, sagitta, close = false) =>
      act((s) => {
        const pts = arcPolylineOnPlane(a, b, sagitta, [1, 0, 0], [0, 1, 0])
        if (pts === null) throw new Error('drawArc: degenerate chord or flat sagitta')
        const sketch = s.begin_ground_sketch()
        const regionsAll = new Set<bigint>()
        const addSeg = (p: V3, q: V3): void => {
          const added = s.sketch_add_segment(sketch, p[0], p[1], p[2], q[0], q[1], q[2])
          for (const r of added.regions_created()) regionsAll.add(r)
        }
        for (let i = 0; i < pts.length - 1; i++) addSeg(pts[i], pts[i + 1])
        if (close) addSeg(pts[pts.length - 1], pts[0])
        return {
          sketch: sketch.toString(),
          regions: Array.from(regionsAll).map(String),
        }
      }),

    drawArcOnFace: (object, face, a, b, sagitta) => {
      act((s) => {
        const objId = BigInt(object)
        const faceId = BigInt(face)
        const n = s.face_normal(objId, faceId)
        const basis = facePlaneBasis([n[0], n[1], n[2]])
        if (basis === null) throw new Error('drawArcOnFace: degenerate face normal')
        const pts = arcPolylineOnPlane(a, b, sagitta, basis.u, basis.v)
        if (pts === null) throw new Error('drawArcOnFace: degenerate chord or flat sagitta')
        const path = new Float64Array(pts.length * 3)
        for (let i = 0; i < pts.length; i++) {
          path[i * 3 + 0] = pts[i][0]
          path[i * 3 + 1] = pts[i][1]
          path[i * 3 + 2] = pts[i][2]
        }
        s.split_face(objId, faceId, path).free()
      })
    },

    moveObject: (object, dx, dy, dz) => {
      // Pure-translation 3×4 row-major affine: identity rotation + (dx,dy,dz) column.
      const affine = new Float64Array([1, 0, 0, dx, 0, 1, 0, dy, 0, 0, 1, dz])
      act((s) => s.transform_object(BigInt(object), affine))
    },

    copyObject: (id, dx, dy, dz) => {
      const affine = new Float64Array([1, 0, 0, dx, 0, 1, 0, dy, 0, 0, 1, dz])
      return act((s) => {
        const node = s.duplicate_node(0, BigInt(id), affine)
        return { kind: node.kind, id: node.id.toString() }
      })
    },

    rotateObject: (object, angleDeg, axis = [0, 0, 1]) => {
      // Rodrigues rotation matrix around a normalized axis by angleDeg degrees.
      const [ux, uy, uz] = axis
      const len = Math.hypot(ux, uy, uz)
      if (len < 1e-12) throw new Error('rotateObject: axis must be non-zero')
      const ax = ux / len, ay = uy / len, az = uz / len
      const theta = (angleDeg * Math.PI) / 180
      const c = Math.cos(theta), s_ = Math.sin(theta), t = 1 - c
      // Row-major 3×4 affine; translation column is zero.
      const affine = new Float64Array([
        t * ax * ax + c,       t * ax * ay - s_ * az, t * ax * az + s_ * ay, 0,
        t * ay * ax + s_ * az, t * ay * ay + c,       t * ay * az - s_ * ax, 0,
        t * az * ax - s_ * ay, t * az * ay + s_ * ax, t * az * az + c,       0,
      ])
      act((s) => s.transform_object(BigInt(object), affine))
    },

    sliceObject: (object, plane) =>
      act((s) => {
        const ids = s.slice_object(BigInt(object), new Float64Array(plane))
        return [ids[0].toString(), ids[1].toString()] as [string, string]
      }),

    addGuideLine: (ox, oy, oz, dx, dy, dz) =>
      act((s) => s.add_guide_line(ox, oy, oz, dx, dy, dz).toString()),

    addGuidePoint: (x, y, z) =>
      act((s) => s.add_guide_point(x, y, z).toString()),

    deleteGuide: (id) => {
      act((s) => s.delete_guide(BigInt(id)))
    },

    deleteAllGuides: () => {
      act((s) => s.delete_all_guides())
    },

    getGuideIds: () => query((s) => Array.from(s.guide_ids()).map(String)),

    undo: () => {
      act((s) => s.scene_undo())
    },

    redo: () => {
      act((s) => s.scene_redo())
    },

    canUndo: () => query((s) => s.can_scene_undo()),
    canRedo: () => query((s) => s.can_scene_redo()),

    setLengthUnit: (format) => setLengthUnit(format as LengthFormat),
    getLengthUnit: () => getLengthUnit(),
    formatLength: (meters) => formatLength(meters),
    parseLength: (input) => parseLengthToMeters(input),

    // -------- materials --------

    addMaterial: (name, r, g, b, a) =>
      act((s) => s.add_material(name, r, g, b, a).toString()),

    paintObject: (object, material) => {
      act((s) => s.set_object_material(BigInt(object), materialHandle(material)))
    },

    paintFace: (object, face, material) => {
      act((s) => s.paint_face(BigInt(object), BigInt(face), materialHandle(material)))
    },
  }

  window.__hew_test = harness
  return () => {
    if (window.__hew_test === harness) delete window.__hew_test
  }
}

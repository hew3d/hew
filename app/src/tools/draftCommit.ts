/**
 * Shared bracket for committing a draw-tool gesture through the drafting
 * engine (unified-drafting design §0–§1): the PLANE is the drafting
 * context. `runDraft` opens the one draft on a target, runs the tool's body
 * (segments, curve brackets), ends it — lowering the whole edit as ONE undo
 * step — and returns the parsed `DraftReport`.
 *
 * Every point handed to the draft calls is in WORLD coordinates: the
 * `*_in_instance` begin variants map through the instance pose themselves,
 * so no `toLocal` is needed (unlike the old `runSketchGesture`).
 *
 * Error contract: on ANY throw — from the body or from `draft_end` (a
 * refusal at end leaves the draft open) — the draft is cancelled and the
 * error is rethrown UNCHANGED, so the kernel's error strings keep their
 * existing toast path (kernelErrors.ts).
 */

import type { DrawPlane } from './drawPlane'
import { applyAffine3x4 } from '../viewport/geoHelpers'
import type { SketchTarget } from './sketchGesture'
import type { Scene as WasmScene } from '../wasm/loader'

/** What a draft lowered: objects whose face topology changed, sketches
 *  (owned or free) that gained edges, and the loose segments kept as
 *  sketch drawing. */
export interface DraftReport {
  objects: bigint[]
  sketches: bigint[]
  keptAsSketch: Array<[[number, number, number], [number, number, number]]>
}

/** The drafting context a draw tool commits into. `instance` is the entered
 *  component instance's handle (definition-owned target) or `null`. */
export type DraftTarget =
  | { kind: 'plane'; plane: DrawPlane; instance: bigint | null; detached?: true }
  | { kind: 'face'; object: bigint; face: bigint; instance: bigint | null; detached?: true }
  | { kind: 'sketch'; handle: bigint; instance: bigint | null }
  /** The exact canonical plane context string a `plane_units_json` listing
   *  used (planeUnits.ts) — the one way to edit listed unit keys. */
  | { kind: 'ctx'; ctx: string }

/** The draft target a tool's plane-mode `SketchTarget` commits into: a
 *  hover-adopted sketch opens as that sketch, a plane as the plane. A
 *  `detached` shape (separate-shapes design §6.4) always opens a fresh
 *  detached sketch on its plane. */
export function draftTargetFromSketchTarget(
  t: SketchTarget,
  detached = false,
  /** The world plane the shape is drawn on; a detached shape opens as that
   *  plane even when a hovered sketch was adopted (it joins nothing). */
  plane?: DrawPlane,
): DraftTarget {
  if (detached) {
    const on = t.kind === 'plane' ? t.plane : plane
    if (on !== undefined) return { kind: 'plane', plane: on, instance: t.instance, detached: true }
  }
  return t.kind === 'existing'
    ? { kind: 'sketch', handle: t.handle, instance: t.instance }
    : { kind: 'plane', plane: t.plane, instance: t.instance }
}

function openDraft(wasmScene: WasmScene, target: DraftTarget, label: string): bigint {
  switch (target.kind) {
    case 'ctx':
      return wasmScene.draft_begin_ctx(target.ctx, label)
    case 'plane': {
      const { plane, instance } = target
      const [px, py, pz] = plane.origin
      const [nx, ny, nz] = plane.normal
      if (target.detached === true) {
        // A detached shape: its own sketch, joined to no arrangement
        // (detached shapes §6.4).
        return wasmScene.draft_begin_detached(
          JSON.stringify({
            plane: [px, py, pz, nx, ny, nz],
            ...(instance !== null ? { instance: Number(instance) } : {}),
          }),
          label,
        )
      }
      return instance !== null
        ? wasmScene.draft_begin_plane_in_instance(instance, px, py, pz, nx, ny, nz, label)
        : wasmScene.draft_begin_plane(px, py, pz, nx, ny, nz, label)
    }
    case 'face':
      if (target.detached === true) {
        return wasmScene.draft_begin_detached(
          JSON.stringify({
            face: { object: Number(target.object), face: Number(target.face) },
            ...(target.instance !== null ? { instance: Number(target.instance) } : {}),
          }),
          label,
        )
      }
      return target.instance !== null
        ? wasmScene.draft_begin_face_in_instance(target.instance, target.object, target.face, label)
        : wasmScene.draft_begin_face(target.object, target.face, label)
    case 'sketch': {
      if (wasmScene.sketch_plane(target.handle) === undefined) {
        throw new Error('UnknownSketch: the hovered sketch is no longer there')
      }
      if (target.instance === null) return wasmScene.draft_begin_sketch(target.handle, label)
      // `draft_begin_sketch` has no instance form: a definition-owned sketch
      // opens as its plane (mapped to world through the pose) in the instance.
      const sp = wasmScene.sketch_plane(target.handle)
      const pose = wasmScene.instance_pose(target.instance)
      if (sp === undefined || pose === undefined) {
        throw new Error('UnknownSketch: the hovered sketch is no longer there')
      }
      const [px, py, pz] = applyAffine3x4(pose, [sp[0], sp[1], sp[2]])
      const nx = pose[0] * sp[3] + pose[1] * sp[4] + pose[2] * sp[5]
      const ny = pose[4] * sp[3] + pose[5] * sp[4] + pose[6] * sp[5]
      const nz = pose[8] * sp[3] + pose[9] * sp[4] + pose[10] * sp[5]
      return wasmScene.draft_begin_plane_in_instance(target.instance, px, py, pz, nx, ny, nz, label)
    }
  }
}

function parseReport(json: string): DraftReport {
  const raw = JSON.parse(json) as {
    objects?: number[]
    sketches?: number[]
    kept_as_sketch?: DraftReport['keptAsSketch']
  }
  return {
    objects: (raw.objects ?? []).map((n) => BigInt(n)),
    sketches: (raw.sketches ?? []).map((n) => BigInt(n)),
    keptAsSketch: raw.kept_as_sketch ?? [],
  }
}

/**
 * Open a draft on `target`, run `body(draft)`, end it as ONE undo step, and
 * return both the body's result and the parsed report. Cancels the draft and
 * rethrows on any error.
 */
export function runDraft<T>(
  wasmScene: WasmScene,
  target: DraftTarget,
  label: string,
  body: (d: bigint) => T,
): { result: T; report: DraftReport } {
  const d = openDraft(wasmScene, target, label)
  try {
    const result = body(d)
    const report = parseReport(wasmScene.draft_end(d))
    return { result, report }
  } catch (err) {
    try {
      wasmScene.draft_cancel(d)
    } catch {
      /* the original error is the one worth surfacing */
    }
    throw err
  }
}

/** Run `segment` (a `draft_add_segment` call) and return its created region
 *  handles, freeing the wasm report. */
export function addDraftSegment(
  wasmScene: WasmScene,
  d: bigint,
  a: readonly [number, number, number],
  b: readonly [number, number, number],
): bigint[] {
  const report = wasmScene.draft_add_segment(d, a[0], a[1], a[2], b[0], b[1], b[2])
  try {
    return Array.from(report.regions_created())
  } finally {
    report.free()
  }
}

/** An analytic curve bracket around a chain of facet segments. */
export interface DraftCurve {
  kind: 'circle' | 'polygon'
  /** World-space center on the draft plane. */
  center: readonly [number, number, number]
  /** World-space radius (m). */
  radius: number
}

/**
 * Add a closed loop of facet segments (`verts[i]` → `verts[i+1]`, wrapping)
 * to the draft, optionally inside a curve bracket so the facets stay one
 * selectable curve carrying its exact analytic center/radius. Returns the
 * last segment's created regions.
 */
export function addDraftLoop(
  wasmScene: WasmScene,
  d: bigint,
  verts: ReadonlyArray<readonly [number, number, number]>,
  curve: DraftCurve | null,
): bigint[] {
  let last: bigint[] = []
  if (curve !== null) {
    const c = new Float64Array([curve.center[0], curve.center[1], curve.center[2]])
    if (curve.kind === 'circle') wasmScene.draft_begin_curve_with(d, c, curve.radius)
    else wasmScene.draft_begin_polygon_with(d, c, curve.radius)
  }
  try {
    for (let i = 0; i < verts.length; i++) {
      last = addDraftSegment(wasmScene, d, verts[i], verts[(i + 1) % verts.length])
    }
  } finally {
    if (curve !== null) wasmScene.draft_end_curve(d)
  }
  return last
}

/**
 * Call the refresh hooks matching what a draft's report says changed:
 * `onObject` per object whose face topology changed (the face-imprint /
 * object refresh); `onSketch` when sketches changed, or when nothing
 * visible did (so the document-changed bookkeeping still runs).
 */
export function refreshAfterDraft(
  report: DraftReport,
  hooks: { onObject: (object: bigint) => void; onSketch: (sketch: bigint) => void },
): void {
  for (const object of report.objects) hooks.onObject(object)
  if (report.sketches.length > 0 || report.objects.length === 0) {
    hooks.onSketch(report.sketches[0] ?? 0n)
  }
}

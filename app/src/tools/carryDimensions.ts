/**
 * carryDimensions — a linear dimension follows the drawing it measures when
 * that drawing is moved (#98).
 *
 * A drawn shape is not a document node, so a dimension picked on one carries
 * FREE anchors (`node: none`) and the kernel's node re-anchoring never sees
 * the move. The follow is geometric, with no stored link: before a transform
 * commits, every free-anchored linear dimension whose two anchors BOTH lie on
 * the transformed drawing's edges is noted; once the drawing has moved, the
 * dimension is re-issued with its anchors, offset and plane carried through
 * the same affine (`update_linear_dimension`, inside the gesture's history
 * bracket, so one undo restores shape and dimension together).
 *
 * Only a rigid map (translation, rotation) is followed. A scale changes the
 * measured length, so the dimension stays where it was. A dimension with just
 * one end on the moved drawing stays too: stretching it is a different
 * feature. Node-anchored dimensions are the kernel's own business.
 */
import type { Scene as WasmScene } from '../wasm/loader'
import { applyAffineToPoint } from './transformMath'

type V3 = [number, number, number]
type Seg = readonly [V3, V3]

/** How far from a drawn edge a free anchor may sit and still count as on it
 *  (metres). Whole-sketch segments come back from the kernel as f32. */
const EPS = 1e-4

export interface CarriedDimension {
  id: bigint
  a: V3
  b: V3
  offset: V3
  plane: [number, number, number, number, number, number]
  textOverride: string | undefined
}

function onSegment(p: V3, [a, b]: Seg): boolean {
  const ab: V3 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]]
  const ap: V3 = [p[0] - a[0], p[1] - a[1], p[2] - a[2]]
  const len2 = ab[0] * ab[0] + ab[1] * ab[1] + ab[2] * ab[2]
  const t = len2 < 1e-18 ? 0 : Math.max(0, Math.min(1, (ap[0] * ab[0] + ap[1] * ab[1] + ap[2] * ab[2]) / len2))
  const dx = ap[0] - t * ab[0]
  const dy = ap[1] - t * ab[1]
  const dz = ap[2] - t * ab[2]
  return dx * dx + dy * dy + dz * dz <= EPS * EPS
}

/** True when `affine` (3x4 row-major) is a rotation plus translation. */
export function isRigid(affine: ArrayLike<number>): boolean {
  const c = (i: number): V3 => [affine[i], affine[i + 4], affine[i + 8]]
  const dot = (p: V3, q: V3) => p[0] * q[0] + p[1] * q[1] + p[2] * q[2]
  const [c0, c1, c2] = [c(0), c(1), c(2)]
  const tol = 1e-9
  return (
    Math.abs(dot(c0, c0) - 1) < tol && Math.abs(dot(c1, c1) - 1) < tol && Math.abs(dot(c2, c2) - 1) < tol &&
    Math.abs(dot(c0, c1)) < tol && Math.abs(dot(c0, c2)) < tol && Math.abs(dot(c1, c2)) < tol &&
    // A mirror would flip the dimension's handedness.
    dot(c0, [c1[1] * c2[2] - c1[2] * c2[1], c1[2] * c2[0] - c1[0] * c2[2], c1[0] * c2[1] - c1[1] * c2[0]]) > 0
  )
}

/** The free-anchored linear dimensions with both ends on `segments` (the
 *  drawing about to move, in its pre-move positions). Empty for a non-rigid
 *  `affine`. */
export function planDimensionCarry(
  wasmScene: WasmScene,
  segments: readonly Seg[],
  affine: ArrayLike<number>,
): CarriedDimension[] {
  if (segments.length === 0 || !isRigid(affine)) return []
  const out: CarriedDimension[] = []
  for (const id of wasmScene.annotation_ids()) {
    if (wasmScene.annotation_kind(id) !== 'linear') continue
    if (wasmScene.annotation_anchor_node_kind(id, 0) !== -1 || wasmScene.annotation_anchor_node_kind(id, 1) !== -1) continue
    if (wasmScene.annotation_detached(id) === true) continue
    const pa = wasmScene.annotation_anchor_point(id, 0)
    const pb = wasmScene.annotation_anchor_point(id, 1)
    const offset = wasmScene.annotation_offset(id)
    const plane = wasmScene.annotation_plane(id)
    if (pa === undefined || pb === undefined || offset === undefined || plane === undefined) continue
    const a: V3 = [pa[0], pa[1], pa[2]]
    const b: V3 = [pb[0], pb[1], pb[2]]
    if (!segments.some((s) => onSegment(a, s)) || !segments.some((s) => onSegment(b, s))) continue
    out.push({
      id,
      a,
      b,
      offset: [offset[0], offset[1], offset[2]],
      plane: [plane[0], plane[1], plane[2], plane[3], plane[4], plane[5]],
      textOverride: wasmScene.annotation_text_override(id),
    })
  }
  return out
}

function rotate(affine: ArrayLike<number>, v: V3): V3 {
  return [
    affine[0] * v[0] + affine[1] * v[1] + affine[2] * v[2],
    affine[4] * v[0] + affine[5] * v[1] + affine[6] * v[2],
    affine[8] * v[0] + affine[9] * v[1] + affine[10] * v[2],
  ]
}

/** Re-issue each planned dimension carried through `affine`. Call inside the
 *  gesture's history bracket, after the drawing has moved. */
export function applyDimensionCarry(
  wasmScene: WasmScene,
  carry: readonly CarriedDimension[],
  affine: ArrayLike<number>,
): void {
  for (const d of carry) {
    const p = applyAffineToPoint(affine, [d.plane[0], d.plane[1], d.plane[2]])
    const n = rotate(affine, [d.plane[3], d.plane[4], d.plane[5]])
    wasmScene.update_linear_dimension(
      d.id,
      -1, 0n, new Float64Array(applyAffineToPoint(affine, d.a)),
      -1, 0n, new Float64Array(applyAffineToPoint(affine, d.b)),
      new Float64Array(rotate(affine, d.offset)),
      new Float64Array([...p, ...n]),
      d.textOverride,
    )
  }
}

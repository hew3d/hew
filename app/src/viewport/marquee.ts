/**
 * Marquee (rubber-band) selection — screen-space hit testing.
 *
 * SketchUp semantics: a left→right drag is a **window** selection (an entity
 * is selected only when it lies entirely inside the rectangle); a right→left
 * drag is a **crossing** selection (touching the rectangle is enough).
 *
 * Testing happens client-side against the render meshes: each candidate's
 * vertices are taken to camera (view) space once, edges are clipped to the
 * near plane there, and the surviving 2D screen segments/triangles are tested
 * against the drag rectangle. The kernel has no screen-space concept — this
 * is viewport ephemera, exactly like the drag rectangle itself.
 */

import * as THREE from 'three'

/** Drag rectangle in canvas pixels, normalized so min ≤ max. */
export interface MarqueeRect {
  minX: number
  minY: number
  maxX: number
  maxY: number
}

/** Window = fully contained (L→R drag); crossing = touched (R→L drag). */
export type MarqueeMode = 'window' | 'crossing'

export function normalizedRect(x0: number, y0: number, x1: number, y1: number): MarqueeRect {
  return {
    minX: Math.min(x0, x1),
    minY: Math.min(y0, y1),
    maxX: Math.max(x0, x1),
    maxY: Math.max(y0, y1),
  }
}

// ───────────────────────────── pure 2D predicates ──────────────────────────

export function pointInRect(x: number, y: number, r: MarqueeRect): boolean {
  return x >= r.minX && x <= r.maxX && y >= r.minY && y <= r.maxY
}

/** Proper/improper 2D segment intersection via signed-area orientation. */
export function segmentsIntersect(
  ax: number, ay: number, bx: number, by: number,
  cx: number, cy: number, dx: number, dy: number,
): boolean {
  const orient = (px: number, py: number, qx: number, qy: number, rx: number, ry: number): number => {
    const v = (qx - px) * (ry - py) - (qy - py) * (rx - px)
    return v > 0 ? 1 : v < 0 ? -1 : 0
  }
  const onSeg = (px: number, py: number, qx: number, qy: number, rx: number, ry: number): boolean =>
    Math.min(px, qx) <= rx && rx <= Math.max(px, qx) &&
    Math.min(py, qy) <= ry && ry <= Math.max(py, qy)

  const o1 = orient(ax, ay, bx, by, cx, cy)
  const o2 = orient(ax, ay, bx, by, dx, dy)
  const o3 = orient(cx, cy, dx, dy, ax, ay)
  const o4 = orient(cx, cy, dx, dy, bx, by)
  if (o1 !== o2 && o3 !== o4) return true
  if (o1 === 0 && onSeg(ax, ay, bx, by, cx, cy)) return true
  if (o2 === 0 && onSeg(ax, ay, bx, by, dx, dy)) return true
  if (o3 === 0 && onSeg(cx, cy, dx, dy, ax, ay)) return true
  if (o4 === 0 && onSeg(cx, cy, dx, dy, bx, by)) return true
  return false
}

/** A 2D segment touches the rect if an endpoint is inside or it crosses an edge. */
export function segmentIntersectsRect(
  ax: number, ay: number, bx: number, by: number, r: MarqueeRect,
): boolean {
  if (pointInRect(ax, ay, r) || pointInRect(bx, by, r)) return true
  return (
    segmentsIntersect(ax, ay, bx, by, r.minX, r.minY, r.maxX, r.minY) ||
    segmentsIntersect(ax, ay, bx, by, r.maxX, r.minY, r.maxX, r.maxY) ||
    segmentsIntersect(ax, ay, bx, by, r.maxX, r.maxY, r.minX, r.maxY) ||
    segmentsIntersect(ax, ay, bx, by, r.minX, r.maxY, r.minX, r.minY)
  )
}

export function pointInTriangle(
  px: number, py: number,
  ax: number, ay: number, bx: number, by: number, cx: number, cy: number,
): boolean {
  const s1 = (bx - ax) * (py - ay) - (by - ay) * (px - ax)
  const s2 = (cx - bx) * (py - by) - (cy - by) * (px - bx)
  const s3 = (ax - cx) * (py - cy) - (ay - cy) * (px - cx)
  const hasNeg = s1 < 0 || s2 < 0 || s3 < 0
  const hasPos = s1 > 0 || s2 > 0 || s3 > 0
  return !(hasNeg && hasPos)
}

/** A 2D triangle touches the rect: vertex inside, edge crossing, or rect inside it. */
export function triangleIntersectsRect(
  ax: number, ay: number, bx: number, by: number, cx: number, cy: number, r: MarqueeRect,
): boolean {
  return (
    segmentIntersectsRect(ax, ay, bx, by, r) ||
    segmentIntersectsRect(bx, by, cx, cy, r) ||
    segmentIntersectsRect(cx, cy, ax, ay, r) ||
    pointInTriangle(r.minX, r.minY, ax, ay, bx, by, cx, cy)
  )
}

// ─────────────────────── projection + geometry testing ─────────────────────

/**
 * Projects world geometry into canvas pixels for one camera pose. View-space
 * intermediates let edges be clipped against the near plane before the 2D
 * tests — a vertex behind the camera has no meaningful screen position.
 */
export class MarqueeProjector {
  private readonly view = new THREE.Matrix4()
  private readonly projection: THREE.Matrix4
  private readonly near: number
  private readonly width: number
  private readonly height: number
  /** Scratch model-view matrix — hit tests run per candidate mesh over
   * potentially huge vertex counts, so the hot loops allocate nothing. */
  private readonly mv = new THREE.Matrix4()
  /** Scratch projected-point slots (ax, ay, bx, by, cx, cy). */
  private readonly px = new Float64Array(6)

  constructor(camera: THREE.PerspectiveCamera, width: number, height: number) {
    camera.updateMatrixWorld()
    this.view.copy(camera.matrixWorldInverse)
    this.projection = camera.projectionMatrix
    this.near = camera.near
    this.width = width
    this.height = height
  }

  /** View-space (camera-frame) positions: xyz triples. */
  toView(positions: ArrayLike<number>, matrixWorld: THREE.Matrix4): Float64Array {
    const e = this.mv.multiplyMatrices(this.view, matrixWorld).elements
    const out = new Float64Array(positions.length)
    for (let i = 0; i < positions.length; i += 3) {
      const x = positions[i], y = positions[i + 1], z = positions[i + 2]
      out[i] = e[0] * x + e[4] * y + e[8] * z + e[12]
      out[i + 1] = e[1] * x + e[5] * y + e[9] * z + e[13]
      out[i + 2] = e[2] * x + e[6] * y + e[10] * z + e[14]
    }
    return out
  }

  /** True when a view-space z is on the visible side of the near plane. */
  inFront(viewZ: number): boolean {
    return viewZ <= -this.near
  }

  /** Project an in-front view-space point into `this.px[slot..slot+1]`. */
  private projectInto(slot: number, x: number, y: number, z: number): void {
    const e = this.projection.elements
    const cx = e[0] * x + e[4] * y + e[8] * z + e[12]
    const cy = e[1] * x + e[5] * y + e[9] * z + e[13]
    const cw = e[3] * x + e[7] * y + e[11] * z + e[15]
    this.px[slot] = ((cx / cw) * 0.5 + 0.5) * this.width
    this.px[slot + 1] = (0.5 - (cy / cw) * 0.5) * this.height
  }

  /** Clip a view-space segment to the near plane and project it into
   * `this.px[0..3]`; false when the segment is fully behind. */
  private clipSegmentInto(view: Float64Array, ia: number, ib: number): boolean {
    let ax = view[ia], ay = view[ia + 1], az = view[ia + 2]
    let bx = view[ib], by = view[ib + 1], bz = view[ib + 2]
    const aFront = this.inFront(az)
    const bFront = this.inFront(bz)
    if (!aFront && !bFront) return false
    if (!aFront || !bFront) {
      const t = (-this.near - az) / (bz - az)
      const cx = ax + (bx - ax) * t
      const cy = ay + (by - ay) * t
      const cz = -this.near
      if (!aFront) { ax = cx; ay = cy; az = cz } else { bx = cx; by = cy; bz = cz }
    }
    this.projectInto(0, ax, ay, az)
    this.projectInto(2, bx, by, bz)
    return true
  }

  /**
   * Window test: every vertex is in front of the camera and inside the rect.
   * Transform and test are fused per vertex so the common miss exits on the
   * first outside vertex without touching the rest of the buffer. Empty
   * geometry is never "inside" — a node with nothing on screen is not
   * silently swept up.
   */
  allVerticesInRect(
    positions: ArrayLike<number>, matrixWorld: THREE.Matrix4, rect: MarqueeRect,
  ): boolean {
    if (positions.length === 0) return false
    const e = this.mv.multiplyMatrices(this.view, matrixWorld).elements
    for (let i = 0; i < positions.length; i += 3) {
      const x = positions[i], y = positions[i + 1], z = positions[i + 2]
      const vx = e[0] * x + e[4] * y + e[8] * z + e[12]
      const vy = e[1] * x + e[5] * y + e[9] * z + e[13]
      const vz = e[2] * x + e[6] * y + e[10] * z + e[14]
      if (!this.inFront(vz)) return false
      this.projectInto(0, vx, vy, vz)
      if (!pointInRect(this.px[0], this.px[1], rect)) return false
    }
    return true
  }

  /**
   * Crossing test for an indexed triangle mesh: any triangle touches the
   * rect. Triangle edges are near-plane clipped; the rect-inside-triangle
   * case is only tested for fully in-front triangles (a rect swallowed by a
   * partially-clipped triangle still hits via its clipped edges in practice).
   */
  meshTouchesRect(
    positions: ArrayLike<number>,
    indices: ArrayLike<number> | null,
    matrixWorld: THREE.Matrix4,
    rect: MarqueeRect,
  ): boolean {
    const view = this.toView(positions, matrixWorld)
    const px = this.px
    const triCount = indices !== null ? indices.length / 3 : positions.length / 9
    for (let t = 0; t < triCount; t++) {
      const ia = 3 * (indices !== null ? Number(indices[3 * t]) : 3 * t)
      const ib = 3 * (indices !== null ? Number(indices[3 * t + 1]) : 3 * t + 1)
      const ic = 3 * (indices !== null ? Number(indices[3 * t + 2]) : 3 * t + 2)

      const allFront =
        this.inFront(view[ia + 2]) && this.inFront(view[ib + 2]) && this.inFront(view[ic + 2])
      if (allFront) {
        this.projectInto(0, view[ia], view[ia + 1], view[ia + 2])
        this.projectInto(2, view[ib], view[ib + 1], view[ib + 2])
        this.projectInto(4, view[ic], view[ic + 1], view[ic + 2])
        if (triangleIntersectsRect(px[0], px[1], px[2], px[3], px[4], px[5], rect)) return true
      } else {
        if (
          (this.clipSegmentInto(view, ia, ib) &&
            segmentIntersectsRect(px[0], px[1], px[2], px[3], rect)) ||
          (this.clipSegmentInto(view, ib, ic) &&
            segmentIntersectsRect(px[0], px[1], px[2], px[3], rect)) ||
          (this.clipSegmentInto(view, ic, ia) &&
            segmentIntersectsRect(px[0], px[1], px[2], px[3], rect))
        ) {
          return true
        }
      }
    }
    return false
  }

  /** Crossing test for a segment soup (xyz pairs): any segment touches. */
  segmentsTouchRect(
    positions: ArrayLike<number>, matrixWorld: THREE.Matrix4, rect: MarqueeRect,
  ): boolean {
    const view = this.toView(positions, matrixWorld)
    const px = this.px
    for (let i = 0; i + 5 < view.length; i += 6) {
      if (
        this.clipSegmentInto(view, i, i + 3) &&
        segmentIntersectsRect(px[0], px[1], px[2], px[3], rect)
      ) {
        return true
      }
    }
    return false
  }
}

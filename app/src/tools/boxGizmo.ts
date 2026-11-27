/**
 * boxGizmo — the bounding-box grip gizmo shared by Scale and Deform.
 *
 * A box outline with square grip markers (face centers, corners, edge
 * midpoints) that keep a constant on-screen size at any zoom, and a grip
 * picker that measures in each grip's own screen-size tolerance. Scale
 * drives all 26 grips; Deform uses only the six face grips.
 *
 * `BoxGizmo` owns the drawn objects and the picking math; the tool owns the
 * selection, the box it asks for, and what a grab means.
 */

import * as THREE from 'three'
import type { Ray } from '../viewport/math'
import { screenConstantWorldHalfFromWorldPerPixel } from '../viewport/math'
import { axisColorsForTheme } from '../viewport/axisColors'
import { getResolvedTheme } from '../settings/theme'
import {
  GRIP_SCREEN_PX,
  MIN_GRIP_WORLD_HALF,
  FALLBACK_GRIP_HALF_M,
  GRIP_PICK_MULTIPLIER,
  GRIP_NEUTRAL_COLOR,
} from './gripStyle'

export type Vec3 = [number, number, number]

/** A grip on the gizmo box: its world position, the grip that anchors it by
 * default (the geometric opposite), and which axes it drives. `axisIndex`
 * is `null` for a corner (uniform, all three axes via one shared ratio along
 * the pivot→grab diagonal); otherwise it names the single world axis a face
 * grip drives, OR is read alongside `axisIndex2` for a two-axis edge grip. */
export interface Grip {
  pos: Vec3
  opposite: Vec3
  /** `null` = corner (uniform). Otherwise the first (or only) driven axis. */
  axisIndex: 0 | 1 | 2 | null
  /** The second driven axis for a two-axis edge grip; `null` for a face or
   * corner grip. */
  axisIndex2: 0 | 1 | 2 | null
}

/** Neutral color for the 8 uniform (corner) grips — face/edge grips are
 * colored by the axis (axes) they drive instead. */
export const GIZMO_CORNER_COLOR = GRIP_NEUTRAL_COLOR
/** Box outline color and opacity (depth-tested off, like the Rotate
 * protractor, so the gizmo stays visible through the selection's own mesh). */
export const GIZMO_OUTLINE_COLOR = 0x999999
export const GIZMO_OUTLINE_OPACITY = 0.6

/** Average two 0xRRGGBB colors channel-wise — used to give a 2-axis edge
 * grip a color distinct from either of its driven axes' face grips (see
 * `gripColor`), instead of reading identically to its FIRST axis alone
 * (a known deferred nit: an X-Z edge grip used to look exactly like an X
 * face grip). */
function blendAxisColors(a: number, b: number): number {
  const ar = (a >> 16) & 0xff, ag = (a >> 8) & 0xff, ab = a & 0xff
  const br = (b >> 16) & 0xff, bg = (b >> 8) & 0xff, bb = b & 0xff
  return (Math.round((ar + br) / 2) << 16) | (Math.round((ag + bg) / 2) << 8) | Math.round((ab + bb) / 2)
}

/** A grip's marker color: neutral for a corner (uniform), the driven axis's
 * color for a face (single axis), or a BLEND of both driven axes' colors for
 * an edge — distinct from a same-axis face grip instead of just repeating
 * its first axis, so the three grip kinds read apart by color as well as
 * position. */
export function gripColor(grip: Grip, axisColors: readonly number[]): number {
  if (grip.axisIndex === null) return GIZMO_CORNER_COLOR
  if (grip.axisIndex2 === null) return axisColors[grip.axisIndex]
  return blendAxisColors(axisColors[grip.axisIndex], axisColors[grip.axisIndex2])
}

/** The 14 (or 26 with edges) grips of an axis-aligned box `[min, max]`. */
export function gripsFromBox(min: Vec3, max: Vec3, includeEdgeGrips: boolean): Grip[] {
  const c: Vec3 = [(min[0] + max[0]) / 2, (min[1] + max[1]) / 2, (min[2] + max[2]) / 2]
  const grips: Grip[] = []

  // 6 face-center grips (1D stretch).
  for (const axis of [0, 1, 2] as const) {
    for (const side of [0, 1] as const) {
      const pos: Vec3 = [...c]
      const opposite: Vec3 = [...c]
      pos[axis] = side === 0 ? min[axis] : max[axis]
      opposite[axis] = side === 0 ? max[axis] : min[axis]
      grips.push({ pos, opposite, axisIndex: axis, axisIndex2: null })
    }
  }

  // 8 corner grips (uniform).
  for (const x of [min[0], max[0]]) {
    for (const y of [min[1], max[1]]) {
      for (const z of [min[2], max[2]]) {
        const pos: Vec3 = [x, y, z]
        const opposite: Vec3 = [
          x === min[0] ? max[0] : min[0],
          y === min[1] ? max[1] : min[1],
          z === min[2] ? max[2] : min[2],
        ]
        grips.push({ pos, opposite, axisIndex: null, axisIndex2: null })
      }
    }
  }

  // 12 edge-midpoint grips (2D scale) — optional (DESIGN §1).
  if (includeEdgeGrips) {
    const AXES: readonly (0 | 1 | 2)[] = [0, 1, 2]
    for (const fixed of AXES) {
      const [a, b] = AXES.filter((ax) => ax !== fixed)
      for (const sa of [0, 1] as const) {
        for (const sb of [0, 1] as const) {
          const pos: Vec3 = [...c]
          const opposite: Vec3 = [...c]
          pos[a] = sa === 0 ? min[a] : max[a]
          pos[b] = sb === 0 ? min[b] : max[b]
          opposite[a] = sa === 0 ? max[a] : min[a]
          opposite[b] = sb === 0 ? max[b] : min[b]
          grips.push({ pos, opposite, axisIndex: a, axisIndex2: b })
        }
      }
    }
  }

  return grips
}

/** SHIP the optional 12 edge (2-axis) grips alongside the required 6 face +
 * 8 corner grips. */
export const INCLUDE_EDGE_GRIPS = true


/** The drawn gizmo and its pick math. One per tool instance. */
export class BoxGizmo {
  /** The drawn group, or null when the gizmo isn't showing. */
  group: THREE.Group | null = null
  /** The rendered grip meshes, parallel to the grips passed to `draw`;
   *  `updateScale` rescales them in place every frame. */
  meshes: THREE.Mesh[] | null = null
  /** The `worldPerPixel` callback from the most recent `updateScale` tick,
   *  cached so `pick` (which only has a ray) reproduces the same on-screen
   *  size for its tolerance. Null before the first tick (unit tests never
   *  drive a real render loop). */
  private worldPerPixel: ((dist: number) => number) | null = null

  /** Draw the outline and one marker per grip into `preview`. */
  draw(preview: THREE.Group, min: Vec3, max: Vec3, grips: Grip[], name: string): void {
    const group = new THREE.Group()
    group.name = name

    const box3 = new THREE.Box3(
      new THREE.Vector3(min[0], min[1], min[2]),
      new THREE.Vector3(max[0], max[1], max[2]),
    )
    const outline = new THREE.Box3Helper(box3, GIZMO_OUTLINE_COLOR)
    const outlineMat = outline.material as THREE.LineBasicMaterial
    outlineMat.depthTest = false
    outlineMat.transparent = true
    outlineMat.opacity = GIZMO_OUTLINE_OPACITY
    group.add(outline)

    // Grips are drawn as a UNIT cube (local half-extent 1); `updateScale`
    // sets each mesh's `.scale` per frame so its WORLD half-size keeps its
    // on-screen size constant. Start at a small placeholder — corrected on
    // the very next render tick.
    const geo = new THREE.BoxGeometry(2, 2, 2)
    const axisColors = axisColorsForTheme(getResolvedTheme())
    const meshes: THREE.Mesh[] = []
    for (const grip of grips) {
      const color = gripColor(grip, axisColors)
      // Always-on-top, like the outline: a grabbable handle stays visible
      // and clickable even where it sits on the selection's own surface.
      const mat = new THREE.MeshBasicMaterial({ color, depthTest: false })
      const mesh = new THREE.Mesh(geo, mat)
      mesh.position.set(grip.pos[0], grip.pos[1], grip.pos[2])
      mesh.scale.setScalar(FALLBACK_GRIP_HALF_M)
      group.add(mesh)
      meshes.push(mesh)
    }

    preview.add(group)
    this.group = group
    this.meshes = meshes
  }

  /** Dispose and remove the drawn gizmo from `preview`. */
  hide(preview: THREE.Group): void {
    if (this.group === null) return
    this.group.traverse((child) => {
      if (child instanceof THREE.Mesh || child instanceof THREE.LineSegments) {
        child.geometry.dispose()
        const m = child.material
        if (Array.isArray(m)) m.forEach((mm) => mm.dispose())
        else if (m instanceof THREE.Material) m.dispose()
      }
    })
    preview.remove(this.group)
    this.group = null
    this.meshes = null
  }

  /** Forget the drawn objects without disposing them — for a tool whose
   *  `cancel()` already emptied the shared preview group. */
  forget(): void {
    this.group = null
    this.meshes = null
  }

  /**
   * Keep every grip marker a small, CONSTANT on-screen size regardless of
   * camera distance or model scale — called once per frame by the Viewport
   * render loop, BEFORE `renderer.render()`. Each marker's world half-extent
   * is `GRIP_SCREEN_PX` pixels' worth of world space at its own distance from
   * the camera, from a projection-agnostic `worldPerPixel` callback. No-op
   * when the gizmo isn't showing.
   */
  updateScale(camera: THREE.Camera, worldPerPixel: (dist: number) => number): void {
    if (this.meshes === null) return
    this.worldPerPixel = worldPerPixel
    for (const mesh of this.meshes) {
      const dist = camera.position.distanceTo(mesh.position)
      const half = screenConstantWorldHalfFromWorldPerPixel(GRIP_SCREEN_PX, worldPerPixel(dist), MIN_GRIP_WORLD_HALF)
      mesh.scale.setScalar(half)
    }
  }

  /** The pick tolerance (world units) at `pos`, matching what `pos` renders
   *  at on screen right now. Falls back to a fixed tolerance before the first
   *  render tick (unit tests, and the very first click). */
  pickToleranceAt(ray: Ray, pos: Vec3): number {
    if (this.worldPerPixel === null) {
      return FALLBACK_GRIP_HALF_M * GRIP_PICK_MULTIPLIER
    }
    const dx = pos[0] - ray.origin[0], dy = pos[1] - ray.origin[1], dz = pos[2] - ray.origin[2]
    const dist = Math.sqrt(dx * dx + dy * dy + dz * dz)
    const half = screenConstantWorldHalfFromWorldPerPixel(
      GRIP_SCREEN_PX,
      this.worldPerPixel(dist),
      MIN_GRIP_WORLD_HALF,
    )
    return half * GRIP_PICK_MULTIPLIER
  }

  /** Nearest grip to `ray` in units of each grip's OWN pick tolerance, or
   *  null when even the best is outside its tolerance. Distance measured in
   *  tolerances is (up to the shared multiplier) distance in screen pixels,
   *  which is the contest the user is actually playing. */
  pick(ray: Ray, grips: Grip[]): Grip | null {
    let best: Grip | null = null
    let bestRatioSq = Infinity
    for (const grip of grips) {
      const d2 = distSqToRay(ray, grip.pos)
      const tol = this.pickToleranceAt(ray, grip.pos)
      const ratioSq = d2 / (tol * tol)
      if (ratioSq < bestRatioSq) {
        bestRatioSq = ratioSq
        best = grip
      }
    }
    if (best === null || bestRatioSq > 1) return null
    return best
  }
}

/** Squared perpendicular distance from `point` to the ray (clamped to the
 *  ray's forward half — a grip "behind" the camera never wins). */
export function distSqToRay(ray: Ray, point: Vec3): number {
  const [ox, oy, oz] = ray.origin
  const [dx, dy, dz] = ray.direction
  const dLenSq = dx * dx + dy * dy + dz * dz
  const wx = point[0] - ox, wy = point[1] - oy, wz = point[2] - oz
  let t = dLenSq < 1e-18 ? 0 : (wx * dx + wy * dy + wz * dz) / dLenSq
  if (t < 0) t = 0
  const cx = ox + t * dx, cy = oy + t * dy, cz = oz + t * dz
  const ex = point[0] - cx, ey = point[1] - cy, ez = point[2] - cz
  return ex * ex + ey * ey + ez * ez
}

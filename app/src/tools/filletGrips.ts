/* oxlint-disable lingui/no-unlocalized-strings -- True Curves strings are not yet wrapped for translation: this file is English-only until they are */
/**
 * filletGrips — the gizmo of the Fillet tool: the radius handle, the stop
 * and variable-radius grips.
 *
 * After the arming click a round handle sits off the picked edge along the
 * bisector of its two faces, at the current size, with a line back to the
 * edge: dragging it along that line sizes the blend. On a single open chain
 * a diamond sits at each end; dragging one slides it along the chain and
 * sets how far the blend stops short of that end. A second grip, offset from
 * the end along the bisector by the radius there, sets the radius at that
 * end. A click on the chain adds a radius point (a radius at that place)
 * with its own pair of grips.
 *
 * The geometry here is pure (chain polylines, projections, the stop and
 * handle bookkeeping) so it is testable in Node; `FilletGrips` is the thin
 * three.js shell that draws the markers at a constant on-screen size using
 * the shared `gripStyle`.
 */

import * as THREE from 'three'
import type { V3 } from './filletMath'
import {
  FALLBACK_GRIP_HALF_M,
  GRIP_NEUTRAL_COLOR,
  GRIP_PICK_MULTIPLIER,
  GRIP_SCREEN_PX,
  MIN_GRIP_WORLD_HALF,
} from './gripStyle'
import { screenConstantWorldHalfFromWorldPerPixel } from '../viewport/math'

// ── Chain geometry (pure) ───────────────────────────────────────────────────

function dist(a: V3, b: V3): number {
  return Math.hypot(a[0] - b[0], a[1] - b[1], a[2] - b[2])
}

/**
 * The chain's points in order from one end to the other, rebuilt from its
 * segment list (xyz pairs per edge, each edge's two ends in either order).
 * A closed chain returns its ring without repeating the first point; an
 * empty or disconnected list returns what chains from the first segment.
 */
export function chainPolyline(segments: ArrayLike<number>): V3[] {
  const n = Math.floor(segments.length / 6)
  if (n === 0) return []
  const seg = (i: number): [V3, V3] => [
    [segments[i * 6], segments[i * 6 + 1], segments[i * 6 + 2]],
    [segments[i * 6 + 3], segments[i * 6 + 4], segments[i * 6 + 5]],
  ]
  const same = (a: V3, b: V3): boolean => dist(a, b) < 1e-7
  let [a, b] = seg(0)
  if (n > 1) {
    const [c, d] = seg(1)
    // Orient the first segment so that its end touches the second.
    if (same(a, c) || same(a, d)) [a, b] = [b, a]
  }
  const pts: V3[] = [a, b]
  for (let i = 1; i < n; i++) {
    const [p, q] = seg(i)
    const tail = pts[pts.length - 1]
    if (same(tail, p)) pts.push(q)
    else if (same(tail, q)) pts.push(p)
    else {
      // Out of order: the chain is not one run; stop at what connected.
      break
    }
  }
  if (pts.length > 2 && same(pts[0], pts[pts.length - 1])) pts.pop()
  return pts
}

export function polylineLength(pts: readonly V3[]): number {
  let l = 0
  for (let i = 1; i < pts.length; i++) l += dist(pts[i - 1], pts[i])
  return l
}

/** The point `s` meters along the polyline (clamped to its ends). */
export function pointAtLength(pts: readonly V3[], s: number): V3 {
  if (pts.length === 0) return [0, 0, 0]
  let left = Math.max(0, s)
  for (let i = 1; i < pts.length; i++) {
    const seg = dist(pts[i - 1], pts[i])
    if (left <= seg || i === pts.length - 1) {
      const t = seg > 0 ? Math.min(1, left / seg) : 0
      return [
        pts[i - 1][0] + (pts[i][0] - pts[i - 1][0]) * t,
        pts[i - 1][1] + (pts[i][1] - pts[i - 1][1]) * t,
        pts[i - 1][2] + (pts[i][2] - pts[i - 1][2]) * t,
      ]
    }
    left -= seg
  }
  return pts[pts.length - 1]
}

/**
 * The point of the polyline nearest the pointer ray, its arc length from the
 * start, and the distance between the two. Standard closest-points between a
 * ray and each segment, minimized over segments.
 */
export function closestOnPolyline(
  pts: readonly V3[],
  origin: V3,
  dir: V3,
): { s: number; point: V3; distance: number } | null {
  if (pts.length < 2) return null
  const dl = Math.hypot(dir[0], dir[1], dir[2])
  if (!(dl > 0)) return null
  const d: V3 = [dir[0] / dl, dir[1] / dl, dir[2] / dl]
  let best: { s: number; point: V3; distance: number } | null = null
  let walked = 0
  for (let i = 1; i < pts.length; i++) {
    const p = pts[i - 1]
    const q = pts[i]
    const e: V3 = [q[0] - p[0], q[1] - p[1], q[2] - p[2]]
    const len = Math.hypot(e[0], e[1], e[2])
    if (len > 0) {
      const w: V3 = [p[0] - origin[0], p[1] - origin[1], p[2] - origin[2]]
      const a = e[0] * e[0] + e[1] * e[1] + e[2] * e[2]
      const b = e[0] * d[0] + e[1] * d[1] + e[2] * d[2]
      const c = d[0] * d[0] + d[1] * d[1] + d[2] * d[2]
      const dd = e[0] * w[0] + e[1] * w[1] + e[2] * w[2]
      const ee = d[0] * w[0] + d[1] * w[1] + d[2] * w[2]
      const denom = a * c - b * b
      // Parameter along the segment of the closest point to the ray line.
      let t = denom > 1e-12 ? (b * ee - c * dd) / denom : 0
      t = Math.max(0, Math.min(1, t))
      const onSeg: V3 = [p[0] + e[0] * t, p[1] + e[1] * t, p[2] + e[2] * t]
      // Distance from that point to the ray (forward only).
      const v: V3 = [onSeg[0] - origin[0], onSeg[1] - origin[1], onSeg[2] - origin[2]]
      const along = Math.max(0, v[0] * d[0] + v[1] * d[1] + v[2] * d[2])
      const distance = Math.hypot(v[0] - d[0] * along, v[1] - d[1] * along, v[2] - d[2] * along)
      if (best === null || distance < best.distance) {
        best = { s: walked + len * t, point: onSeg, distance }
      }
    }
    walked += len
  }
  return best
}

/**
 * The parameter along the line `base + t · axis` (unit `axis`) nearest the
 * pointer ray — how far along the bisector the pointer has been dragged.
 */
export function lineParamNearRay(base: V3, axis: V3, origin: V3, dir: V3): number | null {
  const dl = Math.hypot(dir[0], dir[1], dir[2])
  if (!(dl > 0)) return null
  const d: V3 = [dir[0] / dl, dir[1] / dl, dir[2] / dl]
  const w: V3 = [base[0] - origin[0], base[1] - origin[1], base[2] - origin[2]]
  const b = axis[0] * d[0] + axis[1] * d[1] + axis[2] * d[2]
  const dd = axis[0] * w[0] + axis[1] * w[1] + axis[2] * w[2]
  const ee = d[0] * w[0] + d[1] * w[1] + d[2] * w[2]
  const denom = 1 - b * b
  if (denom < 1e-9) return null
  return (b * ee - dd) / denom
}

// ── Stop and handle bookkeeping (pure) ──────────────────────────────────────

export interface GripHandle {
  /** Arc length from the chain start. */
  at: number
  radius: number
}

export interface GripState {
  /** Length left sharp at each end; 0 = reaches the end. */
  stops: { start: number; end: number }
  /** Variable-radius handles. The end handles (at 0 and at the chain's
   *  length) come from dragging an end's radius grip. */
  handles: GripHandle[]
}

export function emptyGripState(): GripState {
  return { stops: { start: 0, end: 0 }, handles: [] }
}

/** True once any grip has been moved (the spec then carries stops/handles). */
export function gripStateIsActive(g: GripState): boolean {
  return g.stops.start > 0 || g.stops.end > 0 || g.handles.length > 0
}

/** Sets the stop at one end from a pointer position `s` (arc length from the
 *  start); a stop cannot reach past the other stop. */
export function withStop(g: GripState, which: 'start' | 'end', s: number, length: number): GripState {
  const d = which === 'start' ? Math.max(0, s) : Math.max(0, length - s)
  const other = which === 'start' ? g.stops.end : g.stops.start
  const clamped = Math.min(d, Math.max(0, length - other - 1e-6))
  return {
    ...g,
    stops: { ...g.stops, [which]: Math.abs(clamped) < 1e-9 ? 0 : clamped },
  }
}

/** Sets (or adds) the handle at an end (`0` or `length`) or at arc length
 *  `at`, replacing one within `tol` of it. Kept sorted by position. */
export function withHandle(g: GripState, at: number, radius: number, tol: number): GripState {
  const rest = g.handles.filter((h) => Math.abs(h.at - at) > tol)
  const handles = [...rest, { at, radius }].sort((a, b) => a.at - b.at)
  return { ...g, handles }
}

/** Moves the handle at index `i` to arc length `at` (radius kept). */
export function movedHandle(g: GripState, i: number, at: number, length: number): GripState {
  const h = g.handles[i]
  if (h === undefined) return g
  const next = g.handles.slice()
  next[i] = { ...h, at: Math.max(0, Math.min(length, at)) }
  next.sort((a, b) => a.at - b.at)
  return { ...g, handles: next }
}

/** Handles as the kernel wants them: only when there are any. */
export function handlesForSpec(g: GripState): GripHandle[] | undefined {
  return g.handles.length > 0 ? g.handles : undefined
}

// ── Markers (three.js) ──────────────────────────────────────────────────────

export type GripId =
  /** The main handle: the blend's size, off the picked edge. */
  | { kind: 'radius' }
  | { kind: 'stop'; end: 'start' | 'end' }
  | { kind: 'endRadius'; end: 'start' | 'end' }
  | { kind: 'handlePos'; index: number }
  | { kind: 'handleRadius'; index: number }

export function sameGrip(a: GripId | null, b: GripId | null): boolean {
  if (a === null || b === null) return a === b
  if (a.kind !== b.kind) return false
  switch (a.kind) {
    case 'radius':
      return true
    case 'stop':
    case 'endRadius':
      return a.end === (b as { end: 'start' | 'end' }).end
    default:
      return a.index === (b as { index: number }).index
  }
}

export interface GripMarker {
  id: GripId
  pos: V3
  /** Where the marker's lead line starts (the radius handle's edge point). */
  from?: V3
  /** The active field: drawn in the accent color. */
  active?: boolean
}

const STOP_COLOR = GRIP_NEUTRAL_COLOR
const RADIUS_COLOR = 0xffaa00
const ACTIVE_COLOR = 0xff7a00

/** Draws grip markers at a constant on-screen size and answers picks. */
export class FilletGrips {
  readonly group = new THREE.Group()
  private markers: GripMarker[] = []
  private meshes: THREE.Mesh[] = []
  private leads: THREE.Line[] = []
  private worldPerPixel: ((dist: number) => number) | null = null
  /** Diamonds for the grips that sit on the chain (stops, radius points). */
  private readonly diamondGeo = new THREE.OctahedronGeometry(1.4)
  /** Spheres for the size handles (the radius handle, end and point radii). */
  private readonly sphereGeo = new THREE.SphereGeometry(1.1, 16, 12)

  constructor() {
    this.group.name = 'FilletGrips'
  }

  get isShown(): boolean {
    return this.markers.length > 0
  }

  get all(): readonly GripMarker[] {
    return this.markers
  }

  setMarkers(markers: GripMarker[]): void {
    this.clear()
    this.markers = markers
    for (const m of markers) {
      const onChain = m.id.kind === 'stop' || m.id.kind === 'handlePos'
      const color = m.active === true ? ACTIVE_COLOR : onChain ? STOP_COLOR : RADIUS_COLOR
      const mesh = new THREE.Mesh(
        onChain ? this.diamondGeo : this.sphereGeo,
        new THREE.MeshBasicMaterial({ color, depthTest: false }),
      )
      mesh.position.set(m.pos[0], m.pos[1], m.pos[2])
      mesh.scale.setScalar(FALLBACK_GRIP_HALF_M)
      mesh.renderOrder = 1000
      mesh.name = `FilletGrip:${m.id.kind}`
      this.group.add(mesh)
      this.meshes.push(mesh)
      if (m.from !== undefined) {
        const geo = new THREE.BufferGeometry().setFromPoints([
          new THREE.Vector3(m.from[0], m.from[1], m.from[2]),
          new THREE.Vector3(m.pos[0], m.pos[1], m.pos[2]),
        ])
        const line = new THREE.Line(geo, new THREE.LineBasicMaterial({ color, depthTest: false }))
        line.renderOrder = 999
        line.name = 'FilletGripLead'
        this.group.add(line)
        this.leads.push(line)
      }
    }
  }

  clear(): void {
    for (const mesh of this.meshes) {
      ;(mesh.material as THREE.Material).dispose()
      this.group.remove(mesh)
    }
    for (const line of this.leads) {
      line.geometry.dispose()
      ;(line.material as THREE.Material).dispose()
      this.group.remove(line)
    }
    this.meshes = []
    this.leads = []
    this.markers = []
  }

  dispose(): void {
    this.clear()
    this.diamondGeo.dispose()
    this.sphereGeo.dispose()
  }

  /** The marker of `id`, or null. */
  marker(id: GripId): GripMarker | null {
    return this.markers.find((m) => sameGrip(m.id, id)) ?? null
  }

  /** Per-frame resize to a constant `GRIP_SCREEN_PX` (the Tool's
   *  `updateGripScale` forwards here). `camera` position is in world space,
   *  the markers in the grips' parent space, so `toWorld` maps the latter. */
  updateScale(camera: THREE.Camera, worldPerPixel: (dist: number) => number): void {
    this.worldPerPixel = worldPerPixel
    this.group.updateWorldMatrix(true, false)
    const world = new THREE.Vector3()
    for (const mesh of this.meshes) {
      world.copy(mesh.position).applyMatrix4(this.group.matrixWorld)
      const distance = camera.position.distanceTo(world)
      mesh.scale.setScalar(
        screenConstantWorldHalfFromWorldPerPixel(GRIP_SCREEN_PX, worldPerPixel(distance), MIN_GRIP_WORLD_HALF),
      )
    }
  }

  /** The pick tolerance (parent-space meters) for a grip at `pos` seen from
   *  `eye`; a fixed fallback before the first render tick. */
  private tolerance(eye: V3, pos: V3): number {
    if (this.worldPerPixel === null) return FALLBACK_GRIP_HALF_M * GRIP_PICK_MULTIPLIER
    const half = screenConstantWorldHalfFromWorldPerPixel(
      GRIP_SCREEN_PX,
      this.worldPerPixel(dist(eye, pos)),
      MIN_GRIP_WORLD_HALF,
    )
    return half * GRIP_PICK_MULTIPLIER
  }

  /** The grip nearest the ray, in units of its own pick tolerance, or null. */
  pick(origin: V3, dir: V3): GripMarker | null {
    const dl = Math.hypot(dir[0], dir[1], dir[2])
    if (!(dl > 0)) return null
    const d: V3 = [dir[0] / dl, dir[1] / dl, dir[2] / dl]
    let best: { m: GripMarker; score: number } | null = null
    for (const m of this.markers) {
      const v: V3 = [m.pos[0] - origin[0], m.pos[1] - origin[1], m.pos[2] - origin[2]]
      const t = Math.max(0, v[0] * d[0] + v[1] * d[1] + v[2] * d[2])
      const off = Math.hypot(v[0] - d[0] * t, v[1] - d[1] * t, v[2] - d[2] * t)
      const score = off / this.tolerance(origin, m.pos)
      if (score <= 1 && (best === null || score < best.score)) best = { m, score }
    }
    if (best === null) return null
    // At a small radius a radius grip sits on top of its end grip (or a
    // radius point's grip on the point): the grip on the chain is the one
    // the pointer is on, so it wins when the two coincide on screen.
    const onChain = (m: GripMarker): boolean => m.id.kind === 'stop' || m.id.kind === 'handlePos'
    if (!onChain(best.m)) {
      const near = best.m.pos
      const twin = this.markers.find(
        (m) => onChain(m) && dist(m.pos, near) <= 0.5 * this.tolerance(origin, m.pos),
      )
      if (twin !== undefined) return twin
    }
    return best.m
  }
}

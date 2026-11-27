/**
 * viewDieGeometry — pure math for the view die (docs/design/view-die.md §2):
 * a cube with its eight corners cut flat, drawn with an orthographic
 * projection from the live camera orientation.
 *
 * No three.js import: everything works from a camera quaternion
 * `[x, y, z, w]` (camera space → world space, as `camera.quaternion`) so the
 * module is testable in Node and the SVG component stays thin.
 *
 * World convention is Hew's Z-up frame: X red (Right is +X), Y green (Front
 * looks at -Y, Back is +Y), Z blue (Top is +Z). Camera space is the usual
 * three.js one (x right, y up, z toward the viewer).
 */

export type V3 = readonly [number, number, number]
export type Quat = readonly [number, number, number, number]
export type V2 = readonly [number, number]

export type DieFaceId = 'top' | 'bottom' | 'front' | 'back' | 'left' | 'right'
/** Corner direction: each component is +1 or -1. */
export type CornerSigns = readonly [-1 | 1, -1 | 1, -1 | 1]

/** Edge cut off every corner, as a fraction of the cube's edge length. */
export const CHAMFER = 0.26
/** Half the cube's edge in model units. */
const H = 1
/** Where a cut lands along an edge, measured from the cube centre. */
const CUT = H - CHAMFER * 2 * H

/** Hit-test slop around a facet, css px (design §2.4). */
export const FACET_SLOP_PX = 1.5
/** Minimum facet hit radius, css px (design §2.4). */
export const FACET_MIN_RADIUS_PX = 6
/**
 * Facets seen more edge-on than this (cosine of the angle to the viewer) get
 * neither the slop nor the minimum-radius disc: a sliver would otherwise
 * claim clicks inside the faces beside it. Their plain polygon still hits.
 */
export const FACET_MIN_FACING_COS = 0.2
/** A view is "on" a standard direction within this angle (design §2.2). */
export const ON_VIEW_DEG = 0.5

interface FaceDef {
  id: DieFaceId
  /** Outward normal. */
  n: V3
  /** Label "up" direction on the face (world). */
  up: V3
}

export const FACE_DEFS: readonly FaceDef[] = [
  { id: 'top', n: [0, 0, 1], up: [0, 1, 0] },
  { id: 'bottom', n: [0, 0, -1], up: [0, -1, 0] },
  { id: 'front', n: [0, -1, 0], up: [0, 0, 1] },
  { id: 'back', n: [0, 1, 0], up: [0, 0, 1] },
  { id: 'right', n: [1, 0, 0], up: [0, 0, 1] },
  { id: 'left', n: [-1, 0, 0], up: [0, 0, 1] },
]

/** The standard-view eye direction (target to camera) of a face. */
export function faceEye(id: DieFaceId): V3 {
  return FACE_DEFS.find((f) => f.id === id)!.n
}

export const CORNERS: readonly CornerSigns[] = [
  [-1, -1, -1], [1, -1, -1], [-1, 1, -1], [1, 1, -1],
  [-1, -1, 1], [1, -1, 1], [-1, 1, 1], [1, 1, 1],
]

// ---------------------------------------------------------------------------
// vector helpers
// ---------------------------------------------------------------------------

const dot = (a: V3, b: V3): number => a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
const cross = (a: V3, b: V3): V3 => [
  a[1] * b[2] - a[2] * b[1],
  a[2] * b[0] - a[0] * b[2],
  a[0] * b[1] - a[1] * b[0],
]
const len = (a: V3): number => Math.hypot(a[0], a[1], a[2])
const norm = (a: V3): V3 => {
  const l = len(a)
  return l === 0 ? [0, 0, 0] : [a[0] / l, a[1] / l, a[2] / l]
}
const angleDeg = (a: V3, b: V3): number => {
  const d = Math.max(-1, Math.min(1, dot(norm(a), norm(b))))
  return (Math.acos(d) * 180) / Math.PI
}

/** Rotate `v` by the unit quaternion `q`. */
export function rotateByQuat(q: Quat, v: V3): V3 {
  const [x, y, z, w] = q
  // t = 2 * cross(q.xyz, v); v' = v + w*t + cross(q.xyz, t)
  const tx = 2 * (y * v[2] - z * v[1])
  const ty = 2 * (z * v[0] - x * v[2])
  const tz = 2 * (x * v[1] - y * v[0])
  return [
    v[0] + w * tx + (y * tz - z * ty),
    v[1] + w * ty + (z * tx - x * tz),
    v[2] + w * tz + (x * ty - y * tx),
  ]
}

/** The camera's axes in world space. `back` points from the scene to the eye. */
export interface CameraBasis {
  right: V3
  up: V3
  back: V3
}

export function cameraBasis(q: Quat): CameraBasis {
  return {
    right: rotateByQuat(q, [1, 0, 0]),
    up: rotateByQuat(q, [0, 1, 0]),
    back: rotateByQuat(q, [0, 0, 1]),
  }
}

/**
 * Orientation of a camera sitting along `eye` (target to camera), looking at
 * the target with world +Z up. Test helper and the inverse of
 * {@link cameraBasis}; the die itself never needs it.
 */
export function quatForEye(eye: V3): Quat {
  const back = norm(eye)
  let right = cross([0, 0, 1], back)
  if (len(right) < 1e-9) right = [1, 0, 0]
  right = norm(right)
  const up = cross(back, right)
  // Rotation matrix columns right, up, back to quaternion.
  const m00 = right[0], m10 = right[1], m20 = right[2]
  const m01 = up[0], m11 = up[1], m21 = up[2]
  const m02 = back[0], m12 = back[1], m22 = back[2]
  const tr = m00 + m11 + m22
  let x: number, y: number, z: number, w: number
  if (tr > 0) {
    const s = Math.sqrt(tr + 1) * 2
    w = 0.25 * s
    x = (m21 - m12) / s
    y = (m02 - m20) / s
    z = (m10 - m01) / s
  } else if (m00 > m11 && m00 > m22) {
    const s = Math.sqrt(1 + m00 - m11 - m22) * 2
    w = (m21 - m12) / s
    x = 0.25 * s
    y = (m01 + m10) / s
    z = (m02 + m20) / s
  } else if (m11 > m22) {
    const s = Math.sqrt(1 + m11 - m00 - m22) * 2
    w = (m02 - m20) / s
    x = (m01 + m10) / s
    y = 0.25 * s
    z = (m12 + m21) / s
  } else {
    const s = Math.sqrt(1 + m22 - m00 - m11) * 2
    w = (m10 - m01) / s
    x = (m02 + m20) / s
    y = (m12 + m21) / s
    z = 0.25 * s
  }
  return [x, y, z, w]
}

// ---------------------------------------------------------------------------
// the truncated cube
// ---------------------------------------------------------------------------

/** The eight-sided outline of one face, in world space, counter-clockwise seen from outside. */
function faceOutline(def: FaceDef): V3[] {
  const axis = def.n[0] !== 0 ? 0 : def.n[1] !== 0 ? 1 : 2
  const sign = def.n[axis]
  const a = (axis + 1) % 3
  const b = (axis + 2) % 3
  // Walk the octagon in the (a, b) plane, then flip for the far-side faces so
  // every outline winds counter-clockwise seen from outside.
  const ring: [number, number][] = [
    [CUT, H], [-CUT, H], [-H, CUT], [-H, -CUT], [-CUT, -H], [CUT, -H], [H, -CUT], [H, CUT],
  ]
  // (a, b, axis) is a right-handed frame for every cyclic axis choice, so a
  // clockwise-in-(a,b) ring is clockwise seen from +axis; reverse it there.
  const ordered = sign > 0 ? [...ring].reverse() : ring
  return ordered.map(([pa, pb]) => {
    const v: [number, number, number] = [0, 0, 0]
    v[axis] = sign * H
    v[a] = pa
    v[b] = pb
    return v
  })
}

function facetOutline(c: CornerSigns): V3[] {
  const [sx, sy, sz] = c
  return [
    [sx * CUT, sy * H, sz * H],
    [sx * H, sy * CUT, sz * H],
    [sx * H, sy * H, sz * CUT],
  ]
}

// ---------------------------------------------------------------------------
// projection
// ---------------------------------------------------------------------------

export interface DieLayout {
  /** Width and height of the widget square, css px. */
  size: number
  /** Screen px per model unit (half the cube's edge). */
  scale: number
}

export const DIE_SIZE_PX = 76
export const DIE_LAYOUT: DieLayout = { size: DIE_SIZE_PX, scale: 22 }

export interface ProjectedFace {
  kind: 'face'
  id: DieFaceId
  polygon: V2[]
  /** Screen centre of the face. */
  center: V2
  /** Visible to the camera (normal toward the eye). */
  facing: boolean
  /** Cosine between the face normal and the view direction. */
  facingCos: number
  /** Depth of the centre toward the viewer (larger is nearer). */
  depth: number
  /** Unit screen vectors of the label's x axis and its down axis, foreshortened. */
  labelX: V2
  labelY: V2
}

export interface ProjectedFacet {
  kind: 'facet'
  corner: CornerSigns
  polygon: V2[]
  center: V2
  facing: boolean
  facingCos: number
  depth: number
}

export interface ProjectedStub {
  axis: 0 | 1 | 2
  from: V2
  to: V2
}

export interface DieProjection {
  faces: ProjectedFace[]
  facets: ProjectedFacet[]
  /** Convex hull of the whole solid, the silhouette. */
  silhouette: V2[]
  /** Axis stubs on visible edges, from the min corner. */
  stubs: ProjectedStub[]
  basis: CameraBasis
  /** Nearest named view and whether the camera is on it. */
  view: NearestView
}

function project(basis: CameraBasis, p: V3, layout: DieLayout): V2 {
  const c = layout.size / 2
  return [c + layout.scale * dot(p, basis.right), c - layout.scale * dot(p, basis.up)]
}

function projectDir(basis: CameraBasis, d: V3): V2 {
  return [dot(d, basis.right), -dot(d, basis.up)]
}

function polygonCenter(poly: V2[]): V2 {
  let x = 0
  let y = 0
  for (const p of poly) {
    x += p[0]
    y += p[1]
  }
  return [x / poly.length, y / poly.length]
}

function convexHull(points: V2[]): V2[] {
  const pts = [...points].sort((p, q) => p[0] - q[0] || p[1] - q[1])
  const crossZ = (o: V2, a: V2, b: V2): number => (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])
  const lower: V2[] = []
  for (const p of pts) {
    while (lower.length >= 2 && crossZ(lower[lower.length - 2]!, lower[lower.length - 1]!, p) <= 0) lower.pop()
    lower.push(p)
  }
  const upper: V2[] = []
  for (let i = pts.length - 1; i >= 0; i--) {
    const p = pts[i]!
    while (upper.length >= 2 && crossZ(upper[upper.length - 2]!, upper[upper.length - 1]!, p) <= 0) upper.pop()
    upper.push(p)
  }
  lower.pop()
  upper.pop()
  return lower.concat(upper)
}

/** Length of an axis stub in model units, from the min corner. */
const STUB_LEN = 1.2

export function projectDie(q: Quat, layout: DieLayout = DIE_LAYOUT): DieProjection {
  const basis = cameraBasis(q)
  const faces: ProjectedFace[] = FACE_DEFS.map((def) => {
    const polygon = faceOutline(def).map((p) => project(basis, p, layout))
    const cos = dot(def.n, basis.back)
    const u = cross(def.up, def.n)
    const px = projectDir(basis, u)
    const py = projectDir(basis, [-def.up[0], -def.up[1], -def.up[2]])
    return {
      kind: 'face',
      id: def.id,
      polygon,
      center: project(basis, def.n, layout),
      facing: cos > 1e-6,
      facingCos: cos,
      depth: cos,
      labelX: px,
      labelY: py,
    }
  })
  const facets: ProjectedFacet[] = CORNERS.map((corner) => {
    const outline = facetOutline(corner)
    const polygon = outline.map((p) => project(basis, p, layout))
    const n = norm(corner)
    const cos = dot(n, basis.back)
    return { kind: 'facet', corner, polygon, center: polygonCenter(polygon), facing: cos > 1e-6, facingCos: cos, depth: cos }
  })
  const silhouette = convexHull(faces.flatMap((f) => f.polygon))

  const origin: V3 = [-H, -H, -H]
  const faceFacing = (id: DieFaceId): boolean => faces.find((f) => f.id === id)!.facing
  // Each stub rides one of the three cube edges that meet at the min corner;
  // the two faces that edge separates decide whether it is on the visible side.
  const stubFaces: [DieFaceId, DieFaceId][] = [
    ['front', 'bottom'], // X edge at y=-1, z=-1
    ['left', 'bottom'], // Y edge at x=-1, z=-1
    ['left', 'front'], // Z edge at x=-1, y=-1
  ]
  const stubs: ProjectedStub[] = []
  for (const axis of [0, 1, 2] as const) {
    const [fa, fb] = stubFaces[axis]!
    if (!faceFacing(fa) && !faceFacing(fb)) continue
    const end: [number, number, number] = [origin[0], origin[1], origin[2]]
    end[axis] += STUB_LEN
    stubs.push({ axis, from: project(basis, origin, layout), to: project(basis, end, layout) })
  }

  return { faces, facets, silhouette, stubs, basis, view: nearestView(basis.back) }
}

// ---------------------------------------------------------------------------
// views
// ---------------------------------------------------------------------------

export type NearestView =
  | { kind: 'face'; face: DieFaceId; angleDeg: number; on: boolean }
  | { kind: 'corner'; corner: CornerSigns; angleDeg: number; on: boolean }

/** The corner view direction: the same elevation as the Iso standard view. */
export function cornerEye(c: CornerSigns): V3 {
  return [c[0], c[1], c[2]]
}

/** Nearest of the 14 named views to a view direction (target to camera). */
export function nearestView(back: V3): NearestView {
  let best: NearestView | null = null
  for (const f of FACE_DEFS) {
    const a = angleDeg(back, f.n)
    if (best === null || a < best.angleDeg) best = { kind: 'face', face: f.id, angleDeg: a, on: a <= ON_VIEW_DEG }
  }
  for (const c of CORNERS) {
    const a = angleDeg(back, cornerEye(c))
    if (best === null || a < best.angleDeg) best = { kind: 'corner', corner: c, angleDeg: a, on: a <= ON_VIEW_DEG }
  }
  return best!
}

/** The face the camera is looking straight at, or null (design §2.3 step triangles). */
export function faceOnView(back: V3): DieFaceId | null {
  const v = nearestView(back)
  return v.kind === 'face' && v.on ? v.face : null
}

// ---------------------------------------------------------------------------
// step triangles
// ---------------------------------------------------------------------------

export type StepSide = 'up' | 'down' | 'left' | 'right'

export interface StepTriangle {
  side: StepSide
  /** The face a click turns to. */
  target: DieFaceId
  polygon: V2[]
}

/** Gap between the face edge and a step triangle, css px. */
const STEP_GAP = 3
/** Step triangle depth and width, css px. */
const STEP_DEPTH = 7
const STEP_WIDTH = 12

/**
 * Triangles outside the four sides of a face-on die. The target of each is
 * the face whose normal lies closest to that screen direction.
 */
export function stepTriangles(proj: DieProjection, layout: DieLayout = DIE_LAYOUT): StepTriangle[] {
  const on = faceOnView(proj.basis.back)
  if (on === null) return []
  const c = layout.size / 2
  const edge = layout.scale + STEP_GAP
  const dirs: [StepSide, V3][] = [
    ['up', proj.basis.up],
    ['down', [-proj.basis.up[0], -proj.basis.up[1], -proj.basis.up[2]]],
    ['right', proj.basis.right],
    ['left', [-proj.basis.right[0], -proj.basis.right[1], -proj.basis.right[2]]],
  ]
  return dirs.map(([side, d]) => {
    let target: DieFaceId = 'front'
    let bestDot = -2
    for (const f of FACE_DEFS) {
      const k = dot(f.n, d)
      if (k > bestDot) {
        bestDot = k
        target = f.id
      }
    }
    const horizontal = side === 'left' || side === 'right'
    const sgn = side === 'right' || side === 'down' ? 1 : -1
    const tip = edge + STEP_DEPTH
    const polygon: V2[] = horizontal
      ? [[c + sgn * edge, c - STEP_WIDTH / 2], [c + sgn * edge, c + STEP_WIDTH / 2], [c + sgn * tip, c]]
      : [[c - STEP_WIDTH / 2, c + sgn * edge], [c + STEP_WIDTH / 2, c + sgn * edge], [c, c + sgn * tip]]
    return { side, target, polygon }
  })
}

// ---------------------------------------------------------------------------
// hit testing
// ---------------------------------------------------------------------------

export type DieHit =
  | { kind: 'face'; face: DieFaceId }
  | { kind: 'facet'; corner: CornerSigns }
  | { kind: 'step'; side: StepSide; target: DieFaceId }

function pointInPolygon(p: V2, poly: V2[]): boolean {
  let inside = false
  for (let i = 0, j = poly.length - 1; i < poly.length; j = i++) {
    const [xi, yi] = poly[i]!
    const [xj, yj] = poly[j]!
    if (yi > p[1] !== yj > p[1] && p[0] < ((xj - xi) * (p[1] - yi)) / (yj - yi) + xi) inside = !inside
  }
  return inside
}

function distToSegment(p: V2, a: V2, b: V2): number {
  const dx = b[0] - a[0]
  const dy = b[1] - a[1]
  const l2 = dx * dx + dy * dy
  const t = l2 === 0 ? 0 : Math.max(0, Math.min(1, ((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / l2))
  return Math.hypot(p[0] - (a[0] + t * dx), p[1] - (a[1] + t * dy))
}

function distToPolygon(p: V2, poly: V2[]): number {
  if (pointInPolygon(p, poly)) return 0
  let d = Infinity
  for (let i = 0, j = poly.length - 1; i < poly.length; j = i++) d = Math.min(d, distToSegment(p, poly[j]!, poly[i]!))
  return d
}

/**
 * What a point (widget css px) is over. Step triangles first, then facets
 * (1.5 px slop, 6 px minimum radius, front-facing only), then faces. Edges
 * are not targets of their own: a point on one belongs to whichever polygon
 * claims it.
 */
export function hitTest(proj: DieProjection, p: V2, layout: DieLayout = DIE_LAYOUT): DieHit | null {
  for (const s of stepTriangles(proj, layout)) {
    if (distToPolygon(p, s.polygon) <= FACET_SLOP_PX) return { kind: 'step', side: s.side, target: s.target }
  }
  let bestFacet: ProjectedFacet | null = null
  let bestDist = Infinity
  for (const f of proj.facets) {
    if (!f.facing) continue
    const dPoly = distToPolygon(p, f.polygon)
    const dCentre = Math.hypot(p[0] - f.center[0], p[1] - f.center[1])
    const generous = f.facingCos >= FACET_MIN_FACING_COS
    const hit = dPoly === 0 || (generous && (dPoly <= FACET_SLOP_PX || dCentre <= FACET_MIN_RADIUS_PX))
    if (!hit) continue
    // Prefer the facet whose centre is nearest the pointer.
    if (dCentre < bestDist) {
      bestDist = dCentre
      bestFacet = f
    }
  }
  if (bestFacet !== null) return { kind: 'facet', corner: bestFacet.corner }
  let bestFace: ProjectedFace | null = null
  for (const f of proj.faces) {
    if (!f.facing) continue
    if (!pointInPolygon(p, f.polygon)) continue
    if (bestFace === null || f.depth > bestFace.depth) bestFace = f
  }
  return bestFace === null ? null : { kind: 'face', face: bestFace.id }
}

/**
 * Eye direction a click on `hit` asks for. A second click on the face the
 * camera is already on flips to the opposite face (design §2.3).
 */
export function eyeForHit(hit: DieHit, proj: DieProjection): V3 {
  if (hit.kind === 'facet') return cornerEye(hit.corner)
  if (hit.kind === 'step') return faceEye(hit.target)
  const eye = faceEye(hit.face)
  const on = proj.view.kind === 'face' && proj.view.on ? proj.view.face : null
  return on === hit.face ? [0 - eye[0], 0 - eye[1], 0 - eye[2]] : eye
}

// ---------------------------------------------------------------------------
// plane fill
// ---------------------------------------------------------------------------

/**
 * Which pair of die faces a plane normal is parallel to (within 0.5°), as the
 * pair's shared axis index, or null for a custom plane (design §2.5).
 */
export function worldAxisOfNormal(normal: V3): 0 | 1 | 2 | null {
  for (const axis of [0, 1, 2] as const) {
    const e: [number, number, number] = [0, 0, 0]
    e[axis] = 1
    const a = angleDeg(normal, e)
    if (a <= ON_VIEW_DEG || a >= 180 - ON_VIEW_DEG) return axis
  }
  return null
}

/** The die faces parallel to the world axis `axis`. */
export function facesOfAxis(axis: 0 | 1 | 2): readonly [DieFaceId, DieFaceId] {
  return axis === 0 ? ['right', 'left'] : axis === 1 ? ['back', 'front'] : ['top', 'bottom']
}

/** Corner name parts for tooltips: horizontal / depth / vertical. */
export function cornerParts(c: CornerSigns): { right: boolean; back: boolean; below: boolean } {
  return { right: c[0] > 0, back: c[1] > 0, below: c[2] < 0 }
}

/**
 * Part axes — the View > Part Axes overlay (plugins M2 C12).
 *
 * Draws the frame of the selected or edited component (an origin square and
 * three colored arms, red x, green y, blue z) and, for a selected Object or
 * group that holds a `hew.part.frame` override, the triad at that frame. It
 * shows which way a part's length runs; setting the axes is not here.
 *
 * Frame resolution is pure over two scene reads (`instance_pose`,
 * `part_frame_json`) so it tests without a renderer. The group built here is
 * rebuilt by the viewport on selection, document, and session changes and
 * rescaled every rendered frame so the arms keep a constant screen length.
 */
import * as THREE from 'three'
import { Line2 } from 'three/examples/jsm/lines/Line2.js'
import { LineMaterial } from 'three/examples/jsm/lines/LineMaterial.js'
import { LineGeometry } from 'three/examples/jsm/lines/LineGeometry.js'
import type { Scene as WasmScene } from '../wasm/loader'
import { nodeKindToNumber, type NodeRef } from '../panels/treeModel'
import { axisColorsForTheme } from './axisColors'
import { DEPTH_BIAS } from './depthPolicy'
import type { V3 } from './geoHelpers'

export interface PartFrame { origin: V3; x: V3; y: V3; z: V3 }

/** What the renderer hides: the leaf Objects and placements left out of the
 *  drawing, which already folds in every hidden group above a node and every
 *  hidden tag on it or above it. */
export interface HiddenLeaves {
  objects: ReadonlySet<bigint>
  instances: ReadonlySet<bigint>
}

/** The most triads drawn at once; the rest of a large selection is skipped. */
export const MAX_PART_TRIADS = 20
/** On-screen length of each arm. */
export const PART_AXIS_ARM_PX = 60
const ARM_WIDTH_PX = 3
const ORIGIN_MARK_PX = 7

type FrameScene = Pick<
  WasmScene,
  | 'instance_pose' | 'part_frame_json' | 'explode_session_instance' | 'explode_session_component'
  | 'component_sketch_frame' | 'owner_definition' | 'instances_of' | 'node_effectively_hidden'
>

/** Row-major 3x4 affine product `a * b` (apply b, then a). */
function mul34(a: ArrayLike<number>, b: ArrayLike<number>): number[] {
  const o: number[] = []
  for (let r = 0; r < 3; r++) {
    for (let c = 0; c < 4; c++) {
      o.push(a[r * 4] * b[c] + a[r * 4 + 1] * b[4 + c] + a[r * 4 + 2] * b[8 + c] + (c === 3 ? a[r * 4 + 3] : 0))
    }
  }
  return o
}

/** Inverse of a row-major 3x4 affine, or null when singular. */
function inv34(m: ArrayLike<number>): number[] | null {
  const [a, b, c, d, e, f, g, h, i] = [m[0], m[1], m[2], m[4], m[5], m[6], m[8], m[9], m[10]]
  const det = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g)
  if (Math.abs(det) < 1e-300) return null
  const r = [
    (e * i - f * h) / det, (c * h - b * i) / det, (b * f - c * e) / det,
    (f * g - d * i) / det, (a * i - c * g) / det, (c * d - a * f) / det,
    (d * h - e * g) / det, (b * g - a * h) / det, (a * e - b * d) / det,
  ]
  const t = [m[3], m[7], m[11]]
  return [
    r[0], r[1], r[2], -(r[0] * t[0] + r[1] * t[1] + r[2] * t[2]),
    r[3], r[4], r[5], -(r[3] * t[0] + r[4] * t[1] + r[5] * t[2]),
    r[6], r[7], r[8], -(r[6] * t[0] + r[7] * t[1] + r[8] * t[2]),
  ]
}

/**
 * An instance's WORLD pose (row-major 3x4), or null when it has none to draw.
 * Three cases: the instance an explode session entered is hidden by the
 * kernel, so its pose is the inverse of the session's sketch frame; an
 * instance owned by a definition has a pose in that definition's frame, so it
 * composes with its owner's single placement (ambiguous or deeper than
 * {@link MAX_OWNER_DEPTH} gives none); an ordinary world instance reads direct.
 * Limit: override frames of Objects inside an entered component are not drawn.
 */
const MAX_OWNER_DEPTH = 8
function worldPose(scene: FrameScene, id: bigint, depth = 0): number[] | null {
  if (depth > MAX_OWNER_DEPTH) return null
  const direct = scene.instance_pose(id)
  if (direct === undefined) {
    if (scene.explode_session_instance() !== id) return null
    const comp = scene.explode_session_component()
    if (comp === undefined) return null
    return inv34(scene.component_sketch_frame(comp))
  }
  if (direct.length < 12) return null
  const owner = scene.owner_definition(2, id)
  if (owner === undefined) return Array.from(direct)
  const placements = scene.instances_of(owner)
  if (placements.length !== 1) return null
  // A part nested in a hidden placement is not drawn, so it has no triad.
  if (scene.node_effectively_hidden(2, placements[0])) return null
  const outer = worldPose(scene, placements[0], depth + 1)
  return outer === null ? null : mul34(outer, direct)
}

function unit(v: V3): V3 {
  const n = Math.hypot(v[0], v[1], v[2])
  return n > 1e-12 ? [v[0] / n, v[1] / n, v[2] / n] : [v[0], v[1], v[2]]
}

/** An instance's frame: its pose's translation and normalized columns. A
 *  mirrored pose draws z as it is (no flip). */
function instanceFrame(scene: FrameScene, id: bigint, hidden?: HiddenLeaves): PartFrame | null {
  if (scene.node_effectively_hidden(2, id) || hidden?.instances.has(id) === true) return null
  const m = worldPose(scene, id)
  if (m === null) return null
  return {
    origin: [m[3], m[7], m[11]],
    x: unit([m[0], m[4], m[8]]),
    y: unit([m[1], m[5], m[9]]),
    z: unit([m[2], m[6], m[10]]),
  }
}

/** An Object's or group's override frame, or null when it has none. */
function overrideFrame(scene: FrameScene, node: NodeRef, hidden?: HiddenLeaves): PartFrame | null {
  const kind = nodeKindToNumber(node.kind)
  if (kind !== 0 && kind !== 1) return null
  try {
    if (scene.node_effectively_hidden(kind, node.id)) return null
    if (kind === 0 && hidden?.objects.has(node.id) === true) return null
    const f = JSON.parse(scene.part_frame_json(kind, node.id)) as {
      source?: string; origin?: V3; x?: V3; y?: V3; z?: V3
    }
    if (f.source !== 'attribute' || !f.origin || !f.x || !f.y || !f.z) return null
    return { origin: f.origin, x: unit(f.x), y: unit(f.y), z: unit(f.z) }
  } catch {
    return null // stale handle, hidden, or nothing to measure
  }
}

/** The instance being edited: the deepest INSTANCE entry of the active
 *  context (an Object context pushed above it does not hide it), else the
 *  explode-session instance. */
export function editedInstance(
  context: readonly NodeRef[],
  sessionInstance: bigint | null,
): bigint | null {
  for (let i = context.length - 1; i >= 0; i--) {
    if (context[i].kind === 'instance') return context[i].id
  }
  return sessionInstance
}

/**
 * The triads to draw: one per selected instance, one for the edited
 * instance, and one per selected Object or group with a frame override.
 * Capped at {@link MAX_PART_TRIADS}. A node the renderer hides (`hidden`:
 * itself, a group above it, or a tag on it or above it) gets no triad.
 */
export function framesForSelection(
  scene: FrameScene,
  selection: readonly NodeRef[],
  editingInstance: bigint | null,
  hidden?: HiddenLeaves,
): PartFrame[] {
  const out: PartFrame[] = []
  const seen = new Set<bigint>()
  const push = (f: PartFrame | null): void => {
    if (f !== null && out.length < MAX_PART_TRIADS) out.push(f)
  }
  if (editingInstance !== null) {
    seen.add(editingInstance)
    try { push(instanceFrame(scene, editingInstance, hidden)) } catch { /* stale handle */ }
  }
  for (const node of selection) {
    if (out.length >= MAX_PART_TRIADS) break
    if (node.kind === 'instance') {
      if (seen.has(node.id)) continue
      seen.add(node.id)
      try { push(instanceFrame(scene, node.id, hidden)) } catch { /* stale handle */ }
    } else {
      push(overrideFrame(scene, node, hidden))
    }
  }
  return out
}

/** A small white cube outline at the origin, a Line2 path so it takes the same
 *  depth bias as the arms (GL points ignore polygon offset). Sized in the
 *  triad's local units: {@link ORIGIN_MARK_PX} of the {@link PART_AXIS_ARM_PX}. */
function originMark(): Line2 {
  const h = ORIGIN_MARK_PX / 2 / PART_AXIS_ARM_PX
  const geo = new LineGeometry()
  // A small cube outline (one path) so it reads from any view direction.
  geo.setPositions([
    -h, -h, -h, h, -h, -h, h, h, -h, -h, h, -h, -h, -h, -h,
    -h, -h, h, h, -h, h, h, h, h, -h, h, h, -h, -h, h,
  ])
  const mat = new LineMaterial({
    color: 0xffffff,
    linewidth: 2,
    depthTest: true,
    polygonOffset: true,
    polygonOffsetFactor: DEPTH_BIAS.AXES,
    polygonOffsetUnits: DEPTH_BIAS.AXES,
  })
  const line = new Line2(geo, mat)
  line.renderOrder = 3
  line.frustumCulled = false
  return line
}

function arm(to: V3, color: number, bias: boolean): Line2 {
  const geo = new LineGeometry()
  geo.setPositions([0, 0, 0, to[0], to[1], to[2]])
  const mat = new LineMaterial({
    color,
    linewidth: ARM_WIDTH_PX,
    depthTest: true,
    polygonOffset: bias,
    polygonOffsetFactor: DEPTH_BIAS.AXES,
    polygonOffsetUnits: DEPTH_BIAS.AXES,
  })
  const line = new Line2(geo, mat)
  line.renderOrder = 2
  line.frustumCulled = false
  return line
}

/** One triad per frame, in a group named `PartAxes`. Arms are unit length in
 *  each triad's local space; {@link updatePartAxesScale} sizes them. */
export function buildPartAxes(frames: readonly PartFrame[], theme: 'light' | 'dark'): THREE.Group {
  const group = new THREE.Group()
  group.name = 'PartAxes'
  const colors = axisColorsForTheme(theme)
  for (const frame of frames) {
    const triad = new THREE.Group()
    triad.name = 'PartAxesTriad'
    triad.matrixAutoUpdate = false
    triad.userData.frame = frame
    triad.add(arm([1, 0, 0], colors[0], true))
    triad.add(arm([0, 1, 0], colors[1], true))
    triad.add(arm([0, 0, 1], colors[2], true))
    const mark = originMark()
    triad.add(mark)
    group.add(triad)
  }
  updatePartAxesScale(group, 1 / PART_AXIS_ARM_PX)
  return group
}

/**
 * Sizes every triad so its arms span {@link PART_AXIS_ARM_PX} on screen.
 * `worldPerPixel` is one number for the whole group, or a function of a
 * triad's origin when the caller's camera makes the scale depend on depth.
 */
export function updatePartAxesScale(
  group: THREE.Group,
  worldPerPixel: number | ((origin: V3) => number),
): void {
  for (const child of group.children) {
    const frame = child.userData.frame as PartFrame | undefined
    if (frame === undefined) continue
    const wpp = typeof worldPerPixel === 'number' ? worldPerPixel : worldPerPixel(frame.origin)
    const s = PART_AXIS_ARM_PX * wpp
    child.matrix.makeBasis(
      new THREE.Vector3(...frame.x).multiplyScalar(s),
      new THREE.Vector3(...frame.y).multiplyScalar(s),
      new THREE.Vector3(...frame.z).multiplyScalar(s),
    ).setPosition(frame.origin[0], frame.origin[1], frame.origin[2])
    child.matrixWorldNeedsUpdate = true
  }
}

/** Sets the fat-line resolution on every arm (canvas size in pixels). */
export function setPartAxesResolution(group: THREE.Group, width: number, height: number): void {
  group.traverse((o) => {
    if (o instanceof Line2) (o.material as LineMaterial).resolution.set(width, height)
  })
}

/** Releases the geometry and materials of a built group. */
export function disposePartAxes(group: THREE.Group): void {
  group.traverse((o) => {
    if (o instanceof Line2) {
      o.geometry.dispose()
      ;(o.material as THREE.Material).dispose()
    }
  })
}

/**
 * FlipTool against the REAL compiled wasm module: the gizmo's squares, the
 * hover ghost, a click that flips, the copy toggle, face picks (including
 * the selection's own faces), Escape,
 * picking a target with nothing selected, and drawing in the selection.
 */

import { readFile } from 'node:fs/promises'
import { resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import * as THREE from 'three'
import { beforeAll, describe, expect, it, vi } from 'vitest'
import init, { Scene } from '../wasm/pkg/wasm_api.js'
import { FlipTool, flipAlongAxis, flipSelectionBounds } from './FlipTool'
import type { Snap } from './types'
import type { Ray } from '../viewport/math'
import type { NodeRef } from '../panels/treeModel'
import { setCopiesLinked } from '../settings/editing'

const __dirname = fileURLToPath(new URL('.', import.meta.url))

beforeAll(async () => {
  const wasmBytes = await readFile(resolve(__dirname, '../wasm/pkg/wasm_api_bg.wasm'))
  await init({ module_or_path: wasmBytes })
})

type V3 = [number, number, number]

/** A closed ground outline extruded `height` tall. */
function extrude(scene: Scene, pts: Array<[number, number]>, height: number): bigint {
  const sketch = scene.begin_sketch_on_plane(0, 0, 0, 0, 0, 1)
  for (let i = 0; i < pts.length; i++) {
    const [ax, ay] = pts[i]
    const [bx, by] = pts[(i + 1) % pts.length]
    scene.sketch_add_segment(sketch, ax, ay, 0, bx, by, 0)
  }
  const regions = Array.from(scene.sketch_regions(sketch))
  return scene.extrude_region(sketch, regions[0], height)
}

function box(scene: Scene, x0: number, x1: number, y0: number, y1: number, h: number): bigint {
  return extrude(scene, [[x0, y0], [x1, y0], [x1, y1], [x0, y1]], h)
}

/** An L-shaped board in x in [2, 6], y in [0, 3]: no mirror symmetry. */
function lBoard(scene: Scene): bigint {
  return extrude(scene, [[2, 0], [6, 0], [6, 1], [4, 1], [4, 3], [2, 3]], 0.5)
}

const obj = (id: bigint): NodeRef => ({ kind: 'object', id })

/** An objects group holding a mesh named for `id` (the ghost is cloned from it). */
function objectsGroupFor(scene: Scene, ids: bigint[]): THREE.Group {
  const g = new THREE.Group()
  for (const id of ids) {
    const b = scene.object_bounds(id, undefined)
    const mesh = new THREE.Mesh(new THREE.BoxGeometry(b[3] - b[0], b[4] - b[1], b[5] - b[2]))
    mesh.position.set((b[0] + b[3]) / 2, (b[1] + b[4]) / 2, (b[2] + b[5]) / 2)
    const holder = new THREE.Group()
    holder.name = `Object_${id}`
    holder.add(mesh)
    g.add(holder)
  }
  return g
}

function makeTool(scene: Scene, selection: NodeRef[], objectsGroup: THREE.Group | null = null) {
  const preview = new THREE.Group()
  const onCommit = vi.fn()
  const onToast = vi.fn()
  const onCopy = vi.fn()
  const tool = new FlipTool(scene, preview, objectsGroup, selection, onCommit, onToast, null, onCopy)
  tool.activate()
  return { tool, preview, onCommit, onToast, onCopy }
}

const ground = (x: number, y: number, z = 0): Snap => ({ x, y, z, kind: 'ground' })

/** A ray along -x through (y, z): it meets only the square across red. */
const redRay = (y: number, z: number): Ray => ({ origin: [50, y, z], direction: [-1, 0, 0] })
/** A ray along +y through (x, z): it meets only the square across green. */
const greenRay = (x: number, z: number): Ray => ({ origin: [x, -50, z], direction: [0, 1, 0] })
/** A ray down through (x, y), far from any square side-on. */
const blueRay = (x: number, y: number): Ray => ({ origin: [x, y, 50], direction: [0, 0, -1] })
/** A ray that misses every square. */
const missRay: Ray = { origin: [100, 100, 100], direction: [0, 0, 1] }

function minMax(scene: Scene, id: bigint): { min: V3; max: V3 } {
  const b = scene.object_bounds(id, undefined)
  return { min: [b[0], b[1], b[2]], max: [b[3], b[4], b[5]] }
}

function vertexSet(scene: Scene, id: bigint): string[] {
  const mesh = scene.object_mesh(id)
  const pos = mesh.positions()
  const out = new Set<string>()
  for (let i = 0; i < pos.length; i += 3) {
    out.add(`${pos[i].toFixed(6)},${pos[i + 1].toFixed(6)},${pos[i + 2].toFixed(6)}`)
  }
  return [...out].sort()
}

function det(p: ArrayLike<number>): number {
  return (
    p[0] * (p[5] * p[10] - p[6] * p[9]) - p[1] * (p[4] * p[10] - p[6] * p[8]) + p[2] * (p[4] * p[9] - p[5] * p[8])
  )
}

describe('FlipTool', () => {
  it('hovering_a_gizmo_square_highlights_it_and_ghosts_the_mirror', () => {
    const scene = new Scene()
    const b = lBoard(scene)
    const { tool, preview } = makeTool(scene, [obj(b)], objectsGroupFor(scene, [b]))
    const gizmo = preview.getObjectByName('FlipGizmo')
    expect(gizmo).toBeDefined()
    const squares = [0, 1, 2].map((a) => preview.getObjectByName(`FlipSquare_${a}`) as THREE.Mesh)
    expect(squares.every((s) => s !== undefined)).toBe(true)
    // The squares pass through the box center (4, 1.5, 0.25).
    for (const s of squares) expect(s.position.toArray()).toEqual([4, 1.5, 0.25])

    tool.onPointerMove(ground(0, 0), redRay(1.5, 0.3))
    expect(tool.hoveredPlane()).toEqual({ kind: 'axis', axis: 0 })
    const opacity = (s: THREE.Mesh) => (s.material as THREE.MeshBasicMaterial).opacity
    expect(opacity(squares[0])).toBeGreaterThan(opacity(squares[1]))
    expect(opacity(squares[1])).toBe(opacity(squares[2]))
    const ghost = preview.getObjectByName('FlipGhost')
    expect(ghost).toBeDefined()
    ;(ghost as THREE.Object3D).updateMatrixWorld(true)
    expect((ghost as THREE.Object3D).matrixWorld.determinant()).toBeLessThan(0)

    // Moving to another square swaps the highlight; off the gizmo clears it.
    tool.onPointerMove(ground(0, 0), greenRay(4.2, 0.3))
    expect(tool.hoveredPlane()).toEqual({ kind: 'axis', axis: 1 })
    expect(opacity(squares[1])).toBeGreaterThan(opacity(squares[0]))
    tool.onPointerMove(null, missRay)
    expect(tool.hoveredPlane()).toBeNull()
    expect(preview.getObjectByName('FlipGhost')).toBeUndefined()
  })

  it('clicking_a_square_flips_in_one_commit', () => {
    const scene = new Scene()
    const b = lBoard(scene)
    const before = scene.state_hash()
    const depth = scene.undo_depth()
    const { tool, onCommit } = makeTool(scene, [obj(b)])
    // Across green (y = 1.5): the notch moves from y in [1, 3] to [0, 2].
    tool.onPointerDown(ground(0, 0), greenRay(4.2, 0.3))
    expect(onCommit).toHaveBeenCalledTimes(1)
    expect(onCommit.mock.calls[0][0]).toEqual([obj(b)])
    expect(onCommit.mock.calls[0][1]).toBe(false)
    expect(scene.undo_depth()).toBe(depth + 1)
    expect(scene.state_hash()).not.toBe(before)
    expect(minMax(scene, b)).toEqual({ min: [2, 0, 0], max: [6, 3, 0.5] })
    const verts = vertexSet(scene, b)
    expect(verts).toContain('6.000000,3.000000,0.000000')
    expect(verts).not.toContain('6.000000,0.000000,0.000000')
    scene.scene_undo()
    expect(scene.state_hash()).toBe(before)
  })

  it('the_copy_toggle_makes_a_mirrored_copy', () => {
    setCopiesLinked(true)
    const scene = new Scene()
    const b = lBoard(scene)
    const { tool, onCommit, onCopy } = makeTool(scene, [obj(b)])
    tool.toggleCopyMode()
    expect(onCopy).toHaveBeenLastCalledWith('primary', true)
    tool.onPointerDown(ground(0, 0), redRay(1.5, 0.3))
    expect(onCommit).toHaveBeenCalledTimes(1)
    const [nodes, fullRefresh] = onCommit.mock.calls[0] as [NodeRef[], boolean]
    expect(fullRefresh).toBe(true)
    expect(nodes).toHaveLength(1)
    expect(nodes[0].kind).toBe('instance')
    const pose = scene.instance_pose(nodes[0].id) as Float64Array
    expect(det(pose)).toBeLessThan(0)
    // The board was wrapped as a linked set: two placements, one shape.
    expect(scene.instance_ids()).toHaveLength(2)
    expect(scene.object_ids()).not.toContain(b)
    const def = scene.instance_def(nodes[0].id)
    expect(scene.instance_ids().every((i) => scene.instance_def(i) === def)).toBe(true)
    // One undo returns the loose board.
    scene.scene_undo()
    expect(scene.object_ids()).toContain(b)
    expect(scene.instance_ids()).toHaveLength(0)

    // The other kind of copy: independent, a new object.
    tool.setSelection([obj(b)])
    tool.toggleCopyMode()
    expect(onCopy).toHaveBeenLastCalledWith('alternate', false)
    tool.onPointerDown(ground(0, 0), redRay(1.5, 0.3))
    const [copies, full2] = onCommit.mock.calls[1] as [NodeRef[], boolean]
    expect(full2).toBe(false)
    expect(copies[0].kind).toBe('object')
    expect(scene.object_ids()).toHaveLength(2)
  })

  it('a_hot_gizmo_square_hides_the_snap_cue', () => {
    const scene = new Scene()
    const b = lBoard(scene)
    const { tool } = makeTool(scene, [obj(b)], objectsGroupFor(scene, [b]))
    expect(tool.hidesSnapCue()).toBe(false)
    tool.onPointerMove(ground(0, 0), redRay(1.5, 0.3))
    expect(tool.hidesSnapCue()).toBe(true)
    tool.onPointerMove(ground(0, 0), missRay)
    expect(tool.hidesSnapCue()).toBe(false)
  })

  it('a_face_pick_uses_that_faces_plane', () => {
    const scene = new Scene()
    const b = lBoard(scene)
    const wall = box(scene, 10, 11, 0, 3, 1)
    const { tool, onCommit } = makeTool(scene, [obj(b)])
    // The wall's face at x = 10, picked along +x below the gizmo's squares.
    const ray: Ray = { origin: [8, 2.5, 0.9], direction: [1, 0, 0] }
    const pick = scene.pick_face(...ray.origin, ...ray.direction)
    expect(pick?.object()).toBe(wall)
    const snap: Snap = { x: 10, y: 2.5, z: 0.9, kind: 'on-face', elementKind: 'face', object: wall, element: pick!.face() }
    tool.onPointerMove(snap, ray)
    expect(tool.hoveredPlane()).toMatchObject({ kind: 'face', normal: [-1, 0, 0] })
    // A face plane is a real snap, so its cue stays.
    expect(tool.hidesSnapCue()).toBe(false)
    tool.onPointerDown(snap, ray)
    expect(onCommit).toHaveBeenCalledTimes(1)
    // Mirrored across x = 10: x in [2, 6] lands at [14, 18].
    const { min, max } = minMax(scene, b)
    expect(min[0]).toBeCloseTo(14, 9)
    expect(max[0]).toBeCloseTo(18, 9)
  })

  /** A 2 x 2 x 1 box on x in [0, 2]; its center is (1, 1, 0.5). */
  const ownFaceBox = (scene: Scene) => box(scene, 0, 2, 0, 2, 1)
  /** A ray into the box's x = 2 face at (2, 0.3, 0.95), rising so it leaves
   *  through the top before it can meet any of the three squares. */
  const ownFaceRay: Ray = { origin: [5, 0.3, -0.55], direction: [-1, 0, 0.5] }

  it('a_selections_own_face_flips_it_across_that_face_in_place', () => {
    const scene = new Scene()
    const b = ownFaceBox(scene)
    const { tool, onCommit } = makeTool(scene, [obj(b)])
    const pick = scene.pick_face(...ownFaceRay.origin, ...ownFaceRay.direction)
    expect(pick?.object()).toBe(b)
    tool.onPointerMove(null, ownFaceRay)
    expect(tool.hoveredPlane()).toMatchObject({ kind: 'face', normal: [1, 0, 0] })
    tool.onPointerDown(null, ownFaceRay)
    expect(onCommit).toHaveBeenCalledTimes(1)
    // Across x = 2, x in [0, 2] lands on [2, 4].
    const { min, max } = minMax(scene, b)
    expect(min[0]).toBeCloseTo(2, 9)
    expect(max[0]).toBeCloseTo(4, 9)
    expect(scene.object_ids()).toHaveLength(1)
  })

  it('a_selections_own_face_with_the_copy_modifier_adds_the_mirrored_twin', () => {
    setCopiesLinked(false)
    const scene = new Scene()
    const b = ownFaceBox(scene)
    const { tool, onCommit } = makeTool(scene, [obj(b)])
    tool.toggleCopyMode()
    tool.onPointerDown(null, ownFaceRay)
    expect(onCommit).toHaveBeenCalledTimes(1)
    const ids = Array.from(scene.object_ids())
    expect(ids).toHaveLength(2)
    const original = minMax(scene, b)
    expect([original.min[0], original.max[0]]).toEqual([0, 2])
    const twin = ids.find((i) => i !== b) as bigint
    const t = minMax(scene, twin)
    expect(t.min[0]).toBeCloseTo(2, 9)
    expect(t.max[0]).toBeCloseTo(4, 9)
  })

  it('a_face_pick_ignores_an_edge_or_vertex_snap_and_a_missing_snap', () => {
    const scene = new Scene()
    const b = lBoard(scene)
    const wall = box(scene, 10, 11, 0, 3, 1)
    const ray: Ray = { origin: [8, 2.5, 0.9], direction: [1, 0, 0] }
    const snaps: Array<Snap | null> = [
      null,
      { x: 10, y: 3, z: 0.9, kind: 'on-edge', elementKind: 'edge', object: wall, element: 0n },
      { x: 10, y: 3, z: 1, kind: 'endpoint', elementKind: 'vertex', object: wall, element: 0n },
      ground(10, 2.5),
    ]
    for (const snap of snaps) {
      const { tool, onCommit } = makeTool(scene, [obj(b)])
      tool.onPointerMove(snap, ray)
      expect(tool.hoveredPlane()).toMatchObject({ kind: 'face', normal: [-1, 0, 0] })
      // The cue would name a point the click does not use, unless the snap
      // itself sits on a face.
      expect(tool.hidesSnapCue()).toBe(true)
      tool.onPointerDown(snap, ray)
      expect(onCommit).toHaveBeenCalledTimes(1)
      scene.scene_undo()
    }
  })

  it('a_colored_plane_beats_a_face_under_the_same_ray', () => {
    const scene = new Scene()
    const b = lBoard(scene)
    box(scene, -2, -1, 0, 3, 1)
    // Along -x through the board's middle: the red plane at x = 4 is on the
    // ray, and so is the board's own x = 6 face before it.
    const { tool } = makeTool(scene, [obj(b)])
    tool.onPointerMove(null, redRay(1.5, 0.3))
    expect(tool.hoveredPlane()).toEqual({ kind: 'axis', axis: 0 })
    expect(tool.hidesSnapCue()).toBe(true)
  })

  it('faces_the_host_excludes_are_not_picked', () => {
    const scene = new Scene()
    const b = lBoard(scene)
    const wall = box(scene, 10, 11, 0, 3, 1)
    const { tool, onCommit } = makeTool(scene, [obj(b)])
    tool.setFaceEligibility((o) => o !== wall)
    const ray: Ray = { origin: [8, 2.5, 0.9], direction: [1, 0, 0] }
    tool.onPointerMove(null, ray)
    expect(tool.hoveredPlane()).toBeNull()
    tool.onPointerDown(null, ray)
    expect(onCommit).not.toHaveBeenCalled()
  })

  it('every_square_stands_on_its_axis_and_matches_the_pick_extents', () => {
    const scene = new Scene()
    // Extents 4 x 3 x 0.5 (center 4, 1.5, 0.25), so a swapped width and
    // height, or a wrong normal, cannot match.
    const b = lBoard(scene)
    const { tool, preview } = makeTool(scene, [obj(b)])
    const ext: V3 = [4, 3, 0.5]
    const center: V3 = [4, 1.5, 0.25]
    const side = (e: number): number => Math.max(e, 4 * 0.25) * 1.15
    for (const axis of [0, 1, 2] as const) {
      const mesh = preview.getObjectByName(`FlipSquare_${axis}`) as THREE.Mesh
      mesh.updateMatrixWorld(true)
      const normal = new THREE.Vector3(0, 0, 1).transformDirection(mesh.matrixWorld)
      const want = [0, 0, 0]
      want[axis] = 1
      expect(normal.x).toBeCloseTo(want[0], 9)
      expect(normal.y).toBeCloseTo(want[1], 9)
      expect(normal.z).toBeCloseTo(want[2], 9)
      // The drawn corners span the pick extents: width along the lower
      // in-plane axis, height along the higher one.
      const size = new THREE.Box3().setFromObject(mesh).getSize(new THREE.Vector3()).toArray()
      const [u, v] = ([0, 1, 2] as const).filter((k) => k !== axis)
      expect(size[axis]).toBeCloseTo(0, 9)
      expect(size[u]).toBeCloseTo(side(ext[u]), 5)
      expect(size[v]).toBeCloseTo(side(ext[v]), 5)
      // And picking agrees: a hair inside each extent hits, a hair outside
      // misses.
      const ray = (du: number, dv: number): Ray => {
        const origin: V3 = [center[0], center[1], center[2]]
        origin[u] += du
        origin[v] += dv
        origin[axis] += 50
        const direction: V3 = [0, 0, 0]
        direction[axis] = -1
        return { origin, direction }
      }
      const hu = side(ext[u]) / 2
      const hv = side(ext[v]) / 2
      tool.onPointerMove(null, ray(hu - 0.01, hv - 0.01))
      expect(tool.hoveredPlane()).toEqual({ kind: 'axis', axis })
      tool.onPointerMove(null, ray(hu + 0.01, 0))
      expect(tool.hoveredPlane()?.kind).not.toBe('axis')
      tool.onPointerMove(null, ray(0, hv + 0.01))
      expect(tool.hoveredPlane()?.kind).not.toBe('axis')
    }
  })

  it('escape_cancels', () => {
    const scene = new Scene()
    const b = lBoard(scene)
    const before = scene.state_hash()
    const { tool, preview, onCommit } = makeTool(scene, [obj(b)], objectsGroupFor(scene, [b]))
    tool.onPointerMove(ground(0, 0), blueRay(3, 0.5))
    expect(tool.hoveredPlane()).toEqual({ kind: 'axis', axis: 2 })
    expect(tool.hasArmedGesture()).toBe(true)
    tool.onKey({ key: 'Escape', repeat: false } as KeyboardEvent)
    expect(tool.hoveredPlane()).toBeNull()
    expect(tool.hasArmedGesture()).toBe(false)
    expect(preview.getObjectByName('FlipGhost')).toBeUndefined()
    expect(preview.getObjectByName('FlipGizmo')).toBeDefined()
    expect(onCommit).not.toHaveBeenCalled()
    expect(scene.state_hash()).toBe(before)
  })

  it('nothing_selected_selects_under_the_cursor', () => {
    const scene = new Scene()
    const b = lBoard(scene)
    const before = scene.state_hash()
    const { tool, preview, onCommit, onToast } = makeTool(scene, [])
    expect(preview.getObjectByName('FlipGizmo')).toBeUndefined()
    // Empty space: a hint, nothing else.
    tool.setSelectionAcquirer(() => null)
    tool.onPointerDown(ground(0, 0), missRay)
    expect(onToast).toHaveBeenCalledTimes(1)
    // Over the board: it is picked and its gizmo shows; nothing flips yet.
    const acquire = vi.fn(() => [obj(b)])
    tool.setSelectionAcquirer(acquire)
    tool.onPointerDown(ground(3, 0.5), blueRay(3, 0.5))
    expect(acquire).toHaveBeenCalledTimes(1)
    expect(preview.getObjectByName('FlipGizmo')).toBeDefined()
    expect(onCommit).not.toHaveBeenCalled()
    expect(scene.state_hash()).toBe(before)
    // The next click on a square flips it.
    tool.onPointerDown(ground(0, 0), blueRay(3, 0.5))
    expect(onCommit).toHaveBeenCalledTimes(1)
  })

  it('drawing_in_the_selection_flips_with_it', () => {
    const scene = new Scene()
    const b = lBoard(scene)
    // A drawn line on the ground beside the board, its own sketch.
    const s = scene.begin_sketch_on_plane(0, 0, 0, 0, 0, 1)
    scene.sketch_add_segment(s, 7, 0, 0, 8, 2, 0)
    const sel: NodeRef[] = [obj(b), { kind: 'sketch', id: s }]
    const box0 = flipSelectionBounds(scene, sel)
    expect(box0).toEqual({ min: [2, 0, 0], max: [8, 3, 0.5] })
    const depth = scene.undo_depth()
    const { tool, onCommit } = makeTool(scene, sel)
    // Across red through the joint center x = 5.
    tool.onPointerDown(ground(0, 0), redRay(1.5, 0.3))
    expect(onCommit).toHaveBeenCalledTimes(1)
    expect(scene.undo_depth()).toBe(depth + 1)
    const lines = Array.from(scene.sketch_lines(s))
    const xs = lines.filter((_, i) => i % 3 === 0).sort((p, q) => p - q)
    expect(xs[0]).toBeCloseTo(2, 6)
    expect(xs[xs.length - 1]).toBeCloseTo(3, 6)
    const { min, max } = minMax(scene, b)
    expect(min[0]).toBeCloseTo(4, 9)
    expect(max[0]).toBeCloseTo(8, 9)
  })

  it('copy_is_refused_inside_an_open_component', () => {
    setCopiesLinked(true)
    const scene = new Scene()
    const b = lBoard(scene)
    const inst = scene.make_component(new Uint8Array([0]), new BigUint64Array([b]))
    const def = scene.instance_def(inst) as bigint
    const member = scene.instance_expanded_members(inst)[0]
    const before = scene.state_hash()
    const { tool, onCommit, onToast } = makeTool(scene, [obj(member)])
    tool.setEditContext({ kind: 'instance', id: inst, component: def })
    tool.setSelection([obj(member)])
    tool.toggleCopyMode()
    tool.onPointerDown(ground(0, 0), redRay(1.5, 0.3))
    expect(onCommit).not.toHaveBeenCalled()
    expect(onToast).toHaveBeenCalledTimes(1)
    expect(onToast.mock.calls[0][0]).toMatch(/component/i)
    expect(onToast.mock.calls[0][1]).toBeUndefined()
    expect(scene.state_hash()).toBe(before)
    // Flipping in place there still works.
    tool.toggleCopyMode()
    tool.toggleCopyMode()
    tool.onPointerDown(ground(0, 0), redRay(1.5, 0.3))
    expect(onCommit).toHaveBeenCalledTimes(1)
  })

  it('a_face_pick_mirrors_across_the_faces_exact_plane', () => {
    const scene = new Scene()
    const b = lBoard(scene)
    const wall = box(scene, 10, 11, 0, 3, 1)
    const { tool, onCommit } = makeTool(scene, [obj(b)])
    const ray: Ray = { origin: [8, 2.5, 0.9], direction: [1, 0, 0] }
    const pick = scene.pick_face(...ray.origin, ...ray.direction)
    // A snap a little off the face (as an inference snap can sit) still
    // mirrors across the face's own plane, x = 10.
    const snap: Snap = { x: 10.3, y: 2.5, z: 0.9, kind: 'on-face', elementKind: 'face', object: wall, element: pick!.face() }
    tool.onPointerDown(snap, ray)
    expect(onCommit).toHaveBeenCalledTimes(1)
    const { min, max } = minMax(scene, b)
    expect(min[0]).toBeCloseTo(14, 9)
    expect(max[0]).toBeCloseTo(18, 9)
  })

  it('a_selected_members_own_face_is_a_mirror_plane_inside_its_component', () => {
    const scene = new Scene()
    const b = lBoard(scene)
    const inst = scene.make_component(new Uint8Array([0]), new BigUint64Array([b]))
    const def = scene.instance_def(inst) as bigint
    const member = scene.instance_expanded_members(inst)[0]
    const { tool } = makeTool(scene, [obj(member)])
    tool.setEditContext({ kind: 'instance', id: inst, component: def })
    tool.setSelection([obj(member)])
    // The board's top near its x = 2 end, struck on a slant that stays clear
    // of the squares: the member's own face, picked through its placement.
    const ray: Ray = { origin: [5, 0.3, 1.95], direction: [-1, 0, -0.5] }
    const pick = scene.pick_face(...ray.origin, ...ray.direction)
    expect(pick?.object()).toBe(member)
    tool.onPointerMove(null, ray)
    expect(tool.hoveredPlane()).toMatchObject({ kind: 'face', normal: [0, 0, 1] })
  })

  it('the_hover_ghost_disposes_its_own_materials', () => {
    const scene = new Scene()
    const b = lBoard(scene)
    const objects = objectsGroupFor(scene, [b])
    const live = (objects.getObjectByName(`Object_${b}`)!.children[0] as THREE.Mesh).material as THREE.Material
    const liveDispose = vi.spyOn(live, 'dispose')
    const { tool, preview } = makeTool(scene, [obj(b)], objects)
    tool.onPointerMove(ground(0, 0), redRay(1.5, 0.3))
    const ghost = preview.getObjectByName('FlipGhost') as THREE.Object3D
    const mats: THREE.Material[] = []
    ghost.traverse((c) => {
      if (c instanceof THREE.Mesh) mats.push(c.material as THREE.Material)
    })
    expect(mats.length).toBeGreaterThan(0)
    const spies = mats.map((m) => vi.spyOn(m, 'dispose'))
    tool.onPointerMove(null, missRay)
    for (const sp of spies) expect(sp).toHaveBeenCalled()
    expect(liveDispose).not.toHaveBeenCalled()
  })

  it('flip_along_with_nothing_to_flip_reports_it', () => {
    const scene = new Scene()
    const s = scene.begin_sketch_on_plane(0, 0, 0, 0, 0, 1)
    const before = scene.state_hash()
    expect(flipAlongAxis(scene, [{ kind: 'sketch', id: s }], 0)).toBeNull()
    expect(flipAlongAxis(scene, [], 1)).toBeNull()
    expect(scene.state_hash()).toBe(before)
    const b = lBoard(scene)
    expect(flipAlongAxis(scene, [obj(b)], 1)).toEqual([obj(b)])
  })
})

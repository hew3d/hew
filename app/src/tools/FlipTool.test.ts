/**
 * FlipTool against the REAL compiled wasm module: the gizmo's squares, the
 * hover ghost, a click that flips, the copy toggle, a face pick, Escape,
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
    // A face of the selection itself is not a mirror plane.
    tool.setSelection([obj(b), obj(wall)])
    tool.onPointerMove(snap, missRay)
    expect(tool.hoveredPlane()).toBeNull()
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

  it('a_selected_members_own_face_is_not_a_mirror_plane_inside_its_component', () => {
    const scene = new Scene()
    const b = lBoard(scene)
    const inst = scene.make_component(new Uint8Array([0]), new BigUint64Array([b]))
    const def = scene.instance_def(inst) as bigint
    const member = scene.instance_expanded_members(inst)[0]
    const { tool } = makeTool(scene, [obj(member)])
    tool.setEditContext({ kind: 'instance', id: inst, component: def })
    tool.setSelection([obj(member)])
    const pick = scene.pick_face(3, 0.5, 5, 0, 0, -1)
    expect(pick?.object()).toBe(member)
    const snap: Snap = {
      x: 3, y: 0.5, z: 0.5, kind: 'on-face', elementKind: 'face', object: member, element: pick!.face(), instance: inst,
    }
    tool.onPointerMove(snap, missRay)
    expect(tool.hoveredPlane()).toBeNull()
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

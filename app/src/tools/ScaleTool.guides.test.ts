/**
 * ScaleTool carrying construction guides (`setSelectedGuides`), against a
 * mocked WasmScene: guides selected with the nodes preview and scale with
 * them through `transform_mixed_selection` in one step, a retyped factor
 * redoes them, inside a component they take the world scale in the members'
 * history bracket, and guides alone are refused.
 */

import { describe, it, expect, vi } from 'vitest'
import * as THREE from 'three'
import { ScaleTool } from './ScaleTool'
import type { Snap } from './types'
import type { Ray } from '../viewport/math'
import type { NodeRef } from '../panels/treeModel'

const ID = 9n
const OBJ: NodeRef = { kind: 'object', id: ID }
/** The box the gizmo sizes its grips from: [0, 2] x [0, 2] x [0, 1]. */
const TOP: [number, number, number] = [1, 1, 1]
const DOUBLED: [number, number, number] = [1, 1, 2]

const snapAt = (p: [number, number, number]): Snap => ({ x: p[0], y: p[1], z: p[2], kind: 'ground' })
/** A ray that hits `target` exactly from an oblique direction (see
 *  ScaleTool.test.ts's `rayAt`), so it grazes no other grip. */
function rayAt(target: [number, number, number]): Ray {
  const d = [0.31, -0.47, -0.83]
  const n = Math.hypot(d[0], d[1], d[2])
  const dir: [number, number, number] = [d[0] / n, d[1] / n, d[2] / n]
  return { origin: [target[0] - 5 * dir[0], target[1] - 5 * dir[1], target[2] - 5 * dir[2]], direction: dir }
}
const key = (k: string): KeyboardEvent => ({ key: k }) as KeyboardEvent

/** The members a guide-carrying scale calls. Every commit, undo and redo
 *  moves the history generation, as the kernel's would. */
function makeScene() {
  let gen = 1n
  const bump = () => { gen += 1n }
  return {
    history_generation: vi.fn(() => gen),
    axes: vi.fn(() => new Float64Array([0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1])),
    guide_kind: vi.fn(() => 'line'),
    guide_geometry: vi.fn(() => new Float64Array([0, 0, 0, 1, 0, 0])),
    transform_selection: vi.fn(() => bump()),
    transform_mixed_selection: vi.fn(
      (_k: Uint8Array, _i: BigUint64Array, _s: BigUint64Array, _g: BigUint64Array, _a: Float64Array) => bump(),
    ),
    transform_def_selection: vi.fn((_instance: bigint, ..._rest: unknown[]) => bump()),
    instance_pose: vi.fn(() => new Float64Array([1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0])),
    history_bracket_open: vi.fn(),
    history_bracket_commit: vi.fn(() => bump()),
    history_bracket_abort: vi.fn(),
    scene_undo: vi.fn(() => { bump(); return { free: () => { /* no-op */ } } }),
    scene_redo: vi.fn(() => { bump(); return { free: () => { /* no-op */ } } }),
  }
}

/** An objects group holding the box's mesh, named as the Viewport names it. */
function boxGroup(): THREE.Group {
  const group = new THREE.Group()
  const mesh = new THREE.Mesh(new THREE.BoxGeometry(2, 2, 1))
  mesh.name = `Object_${ID}`
  mesh.position.set(1, 1, 0.5)
  group.add(mesh)
  return group
}

/** A tool on `nodes` carrying `guides`, as the Viewport pushes them. */
function makeTool(nodes: NodeRef[], guides: bigint[]) {
  const scene = makeScene()
  const preview = new THREE.Group()
  const onCommit = vi.fn()
  const onToast = vi.fn()
  const tool = new ScaleTool(scene as never, preview, boxGroup(), nodes, onCommit, onToast)
  tool.setSelectedGuides(guides)
  return { tool, scene, preview, onCommit, onToast }
}

/** Grab the +Z face grip and drag it to twice the height. */
function doubleHeight(tool: ScaleTool): void {
  tool.onPointerDown(snapAt(TOP), rayAt(TOP))
  tool.onPointerMove(snapAt(DOUBLED), rayAt(DOUBLED))
  tool.onPointerDown(snapAt(DOUBLED), rayAt(DOUBLED))
}

describe('ScaleTool — guides carried with the selection', () => {
  it('scales the nodes and every guide in one transform_mixed_selection, the ghost showing the guides', () => {
    const t = makeTool([OBJ], [7n, 8n])
    t.tool.onPointerDown(snapAt(TOP), rayAt(TOP))
    const lines: THREE.LineSegments[] = []
    t.preview.traverse((c) => { if (c instanceof THREE.LineSegments && c.material instanceof THREE.LineBasicMaterial && c.renderOrder === 999) lines.push(c) })
    expect(lines).toHaveLength(2)
    t.tool.onPointerMove(snapAt(DOUBLED), rayAt(DOUBLED))
    t.tool.onPointerDown(snapAt(DOUBLED), rayAt(DOUBLED))

    expect(t.scene.transform_selection).not.toHaveBeenCalled()
    expect(t.scene.transform_mixed_selection).toHaveBeenCalledTimes(1)
    const [kinds, ids, sketches, guides, affine] = t.scene.transform_mixed_selection.mock.calls[0]
    expect([...kinds]).toEqual([0])
    expect([...ids]).toEqual([ID])
    expect([...sketches]).toEqual([])
    expect([...guides]).toEqual([7n, 8n])
    expect(affine[0]).toBeCloseTo(1, 9)
    expect(affine[10]).toBeCloseTo(2, 9)
    expect(t.scene.history_bracket_open).not.toHaveBeenCalled() // one kernel step
    expect(t.onCommit).toHaveBeenCalledWith([OBJ], [7n, 8n])
  })

  it('a retyped factor redoes the scale with the same guides', () => {
    const t = makeTool([OBJ], [7n])
    doubleHeight(t.tool)
    t.tool.onKey(key('3'))
    t.tool.onKey(key('Enter'))
    expect(t.scene.scene_undo).toHaveBeenCalledTimes(1)
    expect(t.scene.transform_mixed_selection).toHaveBeenCalledTimes(2)
    const [, , , guides, affine] = t.scene.transform_mixed_selection.mock.calls[1]
    expect([...guides]).toEqual([7n])
    expect(affine[10]).toBeCloseTo(3, 9)
    expect(t.onCommit).toHaveBeenLastCalledWith([OBJ], [7n])
  })

  it('a refused scale toasts the kernel error and lands nothing', () => {
    const t = makeTool([OBJ], [7n])
    t.scene.transform_mixed_selection.mockImplementationOnce(() => {
      throw new Error('UnknownGuide: no such guide in this document')
    })
    doubleHeight(t.tool)
    expect(t.onToast).toHaveBeenCalledTimes(1)
    expect(t.onCommit).not.toHaveBeenCalled()
  })

  it('inside a component the members scale in the definition and the guides in the world, in one bracket', () => {
    const t = makeTool([OBJ], [7n])
    t.tool.setEditContext({ kind: 'instance', id: 40n, component: 41n })
    doubleHeight(t.tool)
    expect(t.scene.transform_def_selection).toHaveBeenCalledTimes(1)
    expect(t.scene.transform_def_selection.mock.calls[0][0]).toBe(40n)
    const [kinds, ids, sketches, guides, affine] = t.scene.transform_mixed_selection.mock.calls[0]
    expect([...kinds, ...ids, ...sketches]).toEqual([])
    expect([...guides]).toEqual([7n])
    expect(affine[10]).toBeCloseTo(2, 9)
    expect(t.scene.history_bracket_open).toHaveBeenCalledTimes(1)
    expect(t.scene.history_bracket_commit).toHaveBeenCalledTimes(1)
    expect(t.scene.history_bracket_abort).not.toHaveBeenCalled()
  })

  it('guides alone are refused: nothing to scale, nothing picked', () => {
    const t = makeTool([], [7n, 8n])
    const acquire = vi.fn(() => [OBJ])
    t.tool.setSelectionAcquirer(acquire)
    expect(t.tool.statusHint()).toMatch(/Guides can't be scaled on their own/)
    t.tool.onPointerDown(snapAt(TOP), rayAt(TOP))
    expect(t.onToast).toHaveBeenCalledWith('Nothing to scale')
    expect(acquire).not.toHaveBeenCalled()
    expect(t.scene.transform_mixed_selection).not.toHaveBeenCalled()
    expect(t.onCommit).not.toHaveBeenCalled()
  })
})

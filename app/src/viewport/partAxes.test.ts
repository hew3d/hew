import { describe, expect, it } from 'vitest'
import * as THREE from 'three'
import type { NodeRef } from '../panels/treeModel'
import {
  MAX_PART_TRIADS,
  PART_AXIS_ARM_PX,
  buildPartAxes,
  framesForSelection,
  updatePartAxesScale,
  editedInstance,
} from './partAxes'

type Pose = number[]
function fakeScene(poses: Record<string, Pose>, frames: Record<string, unknown> = {}) {
  return {
    instance_pose: (id: bigint) => {
      const p = poses[String(id)]
      return p === undefined ? undefined : Float64Array.from(p)
    },
    explode_session_instance: () => undefined,
    explode_session_component: () => undefined,
    component_sketch_frame: () => Float64Array.from([1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0]),
    owner_definition: () => undefined,
    instances_of: () => new BigUint64Array(),
    node_effectively_hidden: (_kind: number, _id: bigint): boolean => false,
    part_frame_json: (kind: number, id: bigint) => {
      const f = frames[`${kind}:${id}`]
      if (f === undefined) throw new Error('NoPartFrame')
      return JSON.stringify(f)
    },
  }
}
const inst = (id: number): NodeRef => ({ kind: 'instance', id: BigInt(id) })
const obj = (id: number): NodeRef => ({ kind: 'object', id: BigInt(id) })
const group = (id: number): NodeRef => ({ kind: 'group', id: BigInt(id) })

// Row-major 3x4 of a rotation about z by `deg`, scaled by `s`, at (tx, ty, tz).
function rotZ(deg: number, s: number, t: [number, number, number]): Pose {
  const c = Math.cos((deg * Math.PI) / 180) * s
  const n = Math.sin((deg * Math.PI) / 180) * s
  return [c, -n, 0, t[0], n, c, 0, t[1], 0, 0, s, t[2]]
}

describe('partAxes', () => {
  it('the_edited_instance_is_the_deepest_instance_in_the_context', () => {
    expect(editedInstance([inst(1), obj(5)], null)).toBe(1n)
    expect(editedInstance([inst(1), inst(2), obj(5)], null)).toBe(2n)
    expect(editedInstance([obj(5)], 9n)).toBe(9n)
    expect(editedInstance([], null)).toBeNull()
  })

  it('an_instance_frame_is_its_pose_columns_normalized', () => {
    const [f] = framesForSelection(fakeScene({ '1': rotZ(30, 2, [1, 2, 3]) }), [inst(1)], null)
    expect(f.origin).toEqual([1, 2, 3])
    expect(f.x[0]).toBeCloseTo(Math.cos(Math.PI / 6))
    expect(f.x[1]).toBeCloseTo(Math.sin(Math.PI / 6))
    expect(Math.hypot(...f.x)).toBeCloseTo(1)
    expect(Math.hypot(...f.y)).toBeCloseTo(1)
    expect(f.z).toEqual([0, 0, 1])
  })

  it('a_mirrored_instance_draws_its_pose_as_it_is', () => {
    // x flipped: the columns are x = -X, y = Y, z = Z; no correction to z.
    const mirrored: Pose = [-1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0]
    const [f] = framesForSelection(fakeScene({ '1': mirrored }), [inst(1)], null)
    expect(f.x).toEqual([-1, 0, 0])
    expect(f.y).toEqual([0, 1, 0])
    expect(f.z).toEqual([0, 0, 1])
  })

  it('an_object_with_an_override_gets_a_triad_at_the_override_frame', () => {
    const frame = { source: 'attribute', origin: [4, 5, 6], x: [0, 2, 0], y: [-1, 0, 0], z: [0, 0, 1] }
    const out = framesForSelection(fakeScene({}, { '0:7': frame, '1:8': frame }), [obj(7), group(8)], null)
    expect(out).toHaveLength(2)
    expect(out[0].origin).toEqual([4, 5, 6])
    expect(out[0].x).toEqual([0, 1, 0])
  })

  it('an_object_without_an_override_gets_none', () => {
    const face = { source: 'face', origin: [0, 0, 0], x: [1, 0, 0], y: [0, 1, 0], z: [0, 0, 1] }
    const scene = fakeScene({}, { '0:7': face })
    expect(framesForSelection(scene, [obj(7)], null)).toEqual([])
    expect(framesForSelection(scene, [obj(99)], null)).toEqual([]) // refused read
  })

  it('a_node_the_renderer_hides_gets_no_triad', () => {
    // Hidden by a group or tag above it: not the node's own flag, but its
    // leaf is in the hidden set the renderer draws by.
    const frame = { source: 'attribute', origin: [4, 5, 6], x: [1, 0, 0], y: [0, 1, 0], z: [0, 0, 1] }
    const scene = fakeScene({ '1': rotZ(0, 1, [1, 2, 3]), '2': rotZ(0, 1, [4, 2, 3]) }, { '0:7': frame })
    const hidden = { objects: new Set([7n]), instances: new Set([1n]) }
    expect(framesForSelection(scene, [inst(1), inst(2), obj(7)], null, hidden)).toHaveLength(1)
    expect(framesForSelection(scene, [inst(1)], 1n, hidden)).toEqual([])
    // Shown again: the triads return.
    const shown = { objects: new Set<bigint>(), instances: new Set<bigint>() }
    expect(framesForSelection(scene, [inst(1), inst(2), obj(7)], null, shown)).toHaveLength(3)
  })

  it('a_node_hidden_through_an_ancestor_or_a_tag_gets_no_triad', () => {
    const frame = { source: 'attribute', origin: [1, 0, 0], x: [1, 0, 0], y: [0, 1, 0], z: [0, 0, 1] }
    const scene = {
      ...fakeScene({ '1': rotZ(0, 1, [0, 0, 0]) }, { '1:7': frame, '0:5': frame }),
      // The kernel answers for the node, its groups above, and their tags.
      node_effectively_hidden: (kind: number, id: bigint) => (kind === 1 && id === 7n) || (kind === 2 && id === 1n),
    }
    expect(framesForSelection(scene, [group(7), inst(1)], null)).toHaveLength(0)
    expect(framesForSelection(scene, [obj(5)], null)).toHaveLength(1)
  })

  it('a_part_nested_in_a_hidden_placement_gets_no_triad', () => {
    // Instance 2 lives in a definition placed once, by instance 1.
    const nested = (outerHidden: boolean) => ({
      ...fakeScene({ '1': rotZ(0, 1, [5, 0, 0]), '2': rotZ(0, 1, [1, 0, 0]) }),
      owner_definition: (_kind: number, id: bigint) => (id === 2n ? 40n : undefined),
      instances_of: () => BigUint64Array.from([1n]),
      node_effectively_hidden: (kind: number, id: bigint) => outerHidden && kind === 2 && id === 1n,
    })
    const [shown] = framesForSelection(nested(false), [inst(2)], null)
    expect(shown.origin).toEqual([6, 0, 0])
    expect(framesForSelection(nested(true), [inst(2)], null)).toHaveLength(0)
  })

  it('the_edited_instance_gets_a_triad_during_a_session', () => {
    const scene = fakeScene({ '3': rotZ(0, 1, [9, 0, 0]) })
    expect(framesForSelection(scene, [], 3n)).toHaveLength(1)
    // Selected and edited at once: one triad, not two.
    expect(framesForSelection(scene, [inst(3)], 3n)).toHaveLength(1)
  })

  it('at_most_twenty_triads_are_built', () => {
    const poses: Record<string, Pose> = {}
    const sel: NodeRef[] = []
    for (let i = 1; i <= 30; i++) {
      poses[String(i)] = rotZ(0, 1, [i, 0, 0])
      sel.push(inst(i))
    }
    const frames = framesForSelection(fakeScene(poses), sel, null)
    expect(frames).toHaveLength(MAX_PART_TRIADS)
    expect(buildPartAxes(frames, 'dark').children).toHaveLength(MAX_PART_TRIADS)
  })

  it('arm_length_is_constant_in_pixels', () => {
    const frames = framesForSelection(fakeScene({ '1': rotZ(30, 1, [1, 2, 3]) }), [inst(1)], null)
    const g = buildPartAxes(frames, 'light')
    for (const wpp of [0.001, 0.02, 1]) {
      updatePartAxesScale(g, wpp)
      const m = g.children[0].matrix
      const tip = new THREE.Vector3(1, 0, 0).applyMatrix4(m)
      const origin = new THREE.Vector3(0, 0, 0).applyMatrix4(m)
      expect(tip.distanceTo(origin) / wpp).toBeCloseTo(PART_AXIS_ARM_PX)
    }
    // A depth-dependent scale function is honored per triad origin.
    updatePartAxesScale(g, (o) => o[0] * 0.01)
    const tip = new THREE.Vector3(0, 0, 1).applyMatrix4(g.children[0].matrix)
    expect(tip.z - 3).toBeCloseTo(PART_AXIS_ARM_PX * 0.01)
  })
})

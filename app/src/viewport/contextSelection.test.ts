import { describe, it, expect } from 'vitest'
import { ctxOfPlane } from '../tools/planeUnits'
import { planeUnitsMock, ringUnit } from '../tools/planeUnitsMock'
import { nodeKey, type NodeRef } from '../panels/treeModel'
import { contextSelectableRefs, marqueeDrawnRefs } from './contextSelection'

const IDENT = [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0] // identity 3x4
const FACE = ctxOfPlane([0, 0, 0.5], [0, 0, 1], 5n)!
const FLOOR = ctxOfPlane([0, 0, 0], [0, 0, 1], 5n)!

function scene() {
  const mock = planeUnitsMock({
    // A shape drawn on member 7's top face, seen through instance 5.
    [FACE]: { units: [ringUnit(10, 100, [[0, 0, 0.5], [1, 0, 0.5], [1, 1, 0.5]], { kind: 'topology', object: 7 })] },
    // The definition's own sketch (sketch 3).
    [FLOOR]: { units: [ringUnit(20, 200, [[0, 0, 0], [1, 0, 0], [1, 1, 0]], { kind: 'free', sketch: 3 })] },
  })
  return {
    ...mock,
    instance_def: (i: bigint) => (i === 5n ? 50n : undefined),
    instance_pose: (i: bigint) => (i === 5n ? IDENT : undefined),
    component_member_objects: () => [7n, 8n],
    component_member_sketches: () => [3n],
    sketch_plane: (s: bigint) => (s === 3n ? new Float64Array([0, 0, 0, 0, 0, 1]) : undefined),
    object_drawing_planes: (o: bigint) => (o === 7n ? [0, 0, 0.5, 0, 0, 1] : []),
  }
}

const keys = (r: NodeRef[]): string[] => r.map(nodeKey)

describe('contextSelectableRefs', () => {
  it('is null at the top level and inside a group', () => {
    expect(contextSelectableRefs(scene() as never, [], () => false)).toBeNull()
    expect(contextSelectableRefs(scene() as never, [{ kind: 'group', id: 1n }], () => false)).toBeNull()
  })

  it('inside an instance lists the definition members, its sketches and drawing on its members', () => {
    const out = contextSelectableRefs(scene() as never, [{ kind: 'instance', id: 5n }], () => false)!
    expect(keys(out.objects)).toEqual([nodeKey({ kind: 'object', id: 7n }), nodeKey({ kind: 'object', id: 8n })])
    expect(out.drawn).toHaveLength(2)
    expect(out.drawn.every((r) => r.kind === 'draft-island')).toBe(true)
  })

  it('leaves hidden members out', () => {
    const out = contextSelectableRefs(scene() as never, [{ kind: 'instance', id: 5n }], (o) => o === 8n)!
    expect(keys(out.objects)).toEqual([nodeKey({ kind: 'object', id: 7n })])
  })

  it('inside a single solid lists only the drawing on it, possibly nothing', () => {
    const none = contextSelectableRefs(scene() as never, [{ kind: 'object', id: 9n }], () => false)!
    expect(none.objects).toEqual([])
    expect(none.drawn).toEqual([])
  })
})

describe('marqueeDrawnRefs', () => {
  const world: NodeRef[] = [{ kind: 'sketch', id: 9n }]
  it('picks the world drawn shapes at the top level and inside an open group', () => {
    expect(marqueeDrawnRefs(scene() as never, [], () => false, () => world)).toEqual(world)
    expect(marqueeDrawnRefs(scene() as never, [{ kind: 'group', id: 1n }], () => false, () => world)).toEqual(world)
  })

  it('scopes to the instance inside an instance context, never reading the world', () => {
    const out = marqueeDrawnRefs(scene() as never, [{ kind: 'instance', id: 5n }], () => false, () => {
      throw new Error('world not read')
    })
    expect(out).toHaveLength(2)
  })
})

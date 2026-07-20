import { describe, it, expect } from 'vitest'
import {
  meshWorldBounds,
  unionBounds,
  boundsExtents,
  worldBoundsForSelection,
  type Bounds,
} from './objectBounds'
import { rotationZAffine } from '../tools/transformMath'
import type { Scene as WasmScene } from '../wasm/loader'
import type { NodeRef } from './treeModel'

/** A mesh stand-in shaped like the real `MeshJs` FFI surface (matching
 * `SceneRenderer.test.ts`'s `makeMesh`) — only `positions`/`free` matter here. */
function makeMesh(positions: Float32Array) {
  return {
    positions: () => positions,
    free: () => {},
  }
}

/** Corners of an axis-aligned box centered at the origin with the given
 * half-extents, as a flat [x,y,z, ...] position buffer (not a real
 * triangulated mesh — meshWorldBounds only cares about the extremal
 * coordinates, so eight loose corner points are sufficient). */
function boxCorners(hx: number, hy: number, hz: number): Float32Array {
  const pts: number[] = []
  for (const sx of [-1, 1]) {
    for (const sy of [-1, 1]) {
      for (const sz of [-1, 1]) {
        pts.push(sx * hx, sy * hy, sz * hz)
      }
    }
  }
  return new Float32Array(pts)
}

/** Minimal mock WasmScene — only the accessors `worldBoundsForSelection`
 * touches, matching `panels/scenePanels.test.tsx`'s plain-object style. */
function makeScene(opts: {
  objectMeshes?: Record<string, Float32Array>
  groups?: Record<string, NodeRef[]>
  instances?: Record<string, { def: bigint; pose: number[]; memberIds: bigint[] }>
  componentMembers?: Record<string, bigint[]>
}): WasmScene {
  const objectMeshes = opts.objectMeshes ?? {}
  const groups = opts.groups ?? {}
  const instances = opts.instances ?? {}
  return {
    object_mesh: (id: bigint) => {
      const positions = objectMeshes[id.toString()]
      if (positions === undefined) throw new Error(`unexpected object_mesh(${id})`)
      return makeMesh(positions)
    },
    group_members: (id: bigint) => groups[id.toString()] ?? [],
    instance_pose: (id: bigint) => {
      const inst = instances[id.toString()]
      return inst !== undefined ? Float64Array.from(inst.pose) : undefined
    },
    instance_def: (id: bigint) => instances[id.toString()]?.def,
    component_member_objects: (componentId: bigint) =>
      BigUint64Array.from(opts.componentMembers?.[componentId.toString()] ?? []),
  } as unknown as WasmScene
}

describe('meshWorldBounds', () => {
  it('a 20 mm cube has extents of 0.02 m on each axis', () => {
    const cube = boxCorners(0.01, 0.01, 0.01) // half-extent 10mm -> 20mm cube
    const bounds = meshWorldBounds(cube)
    expect(bounds).not.toBeNull()
    const extents = boundsExtents(bounds as Bounds)
    expect(extents[0]).toBeCloseTo(0.02, 6)
    expect(extents[1]).toBeCloseTo(0.02, 6)
    expect(extents[2]).toBeCloseTo(0.02, 6)
  })

  it('returns null for an empty position buffer', () => {
    expect(meshWorldBounds(new Float32Array())).toBeNull()
  })

  it('a 45deg-rotated box has a larger world AABB on the rotated axes', () => {
    // A 2x2 square footprint (half-extent 1 in X and Y), height 2 (half-extent
    // 1 in Z). Local AABB extents: [2, 2, 2].
    const box = boxCorners(1, 1, 1)
    const localBounds = meshWorldBounds(box) as Bounds
    expect(boundsExtents(localBounds)).toEqual([2, 2, 2])

    // Rotate 45 deg about world +Z: a square's AABB grows to its diagonal
    // (2 * sqrt(2)) in the plane it's rotating within; Z (the rotation axis)
    // is untouched.
    const pose = rotationZAffine(Math.PI / 4)
    const worldBounds = meshWorldBounds(box, pose) as Bounds
    const worldExtents = boundsExtents(worldBounds)
    expect(worldExtents[0]).toBeGreaterThan(2)
    expect(worldExtents[1]).toBeGreaterThan(2)
    expect(worldExtents[0]).toBeCloseTo(2 * Math.SQRT2, 6)
    expect(worldExtents[1]).toBeCloseTo(2 * Math.SQRT2, 6)
    expect(worldExtents[2]).toBeCloseTo(2, 6)
  })
})

describe('unionBounds', () => {
  it('unions two boxes into the box containing both', () => {
    const a: Bounds = { min: [0, 0, 0], max: [1, 1, 1] }
    const b: Bounds = { min: [-1, 2, 0.5], max: [0.5, 3, 4] }
    expect(unionBounds(a, b)).toEqual({ min: [-1, 0, 0], max: [1, 3, 4] })
  })

  it('passes the other side through when one side is null', () => {
    const a: Bounds = { min: [0, 0, 0], max: [1, 1, 1] }
    expect(unionBounds(a, null)).toEqual(a)
    expect(unionBounds(null, a)).toEqual(a)
    expect(unionBounds(null, null)).toBeNull()
  })
})

describe('worldBoundsForSelection', () => {
  it('returns null for an empty selection', () => {
    expect(worldBoundsForSelection(makeScene({}), [])).toBeNull()
  })

  it('returns null for a mesh-less selection (a sketch)', () => {
    const scene = makeScene({})
    const selection: NodeRef[] = [{ kind: 'sketch-island', id: 1n }]
    expect(worldBoundsForSelection(scene, selection)).toBeNull()
  })

  it('unions a two-object selection into one AABB', () => {
    const scene = makeScene({
      objectMeshes: {
        '1': boxCorners(0.5, 0.5, 0.5), // [-0.5,0.5] on each axis
        '2': new Float32Array([4.5, 4.5, 4.5, 5.5, 5.5, 5.5]), // [4.5,5.5] on each axis
      },
    })
    const selection: NodeRef[] = [
      { kind: 'object', id: 1n },
      { kind: 'object', id: 2n },
    ]
    const bounds = worldBoundsForSelection(scene, selection)
    expect(bounds).toEqual({ min: [-0.5, -0.5, -0.5], max: [5.5, 5.5, 5.5] })
  })

  it('unions a Group\'s leaf-object meshes (already baked/world-space — no pose applied)', () => {
    const scene = makeScene({
      objectMeshes: {
        '1': boxCorners(1, 1, 1),
        '2': new Float32Array([2, 2, 2, 3, 3, 3]),
      },
      groups: {
        '10': [{ kind: 'object', id: 1n }, { kind: 'object', id: 2n }],
      },
    })
    const bounds = worldBoundsForSelection(scene, [{ kind: 'group', id: 10n }])
    expect(bounds).toEqual({ min: [-1, -1, -1], max: [3, 3, 3] })
  })

  it("poses a Component instance's member mesh by its own affine", () => {
    // Definition-local member: unit cube at the origin (half-extent 0.5).
    const scene = makeScene({
      objectMeshes: {
        '100': boxCorners(0.5, 0.5, 0.5),
      },
      componentMembers: {
        '5': [100n],
      },
      instances: {
        '1': { def: 5n, pose: [1, 0, 0, 10, 0, 1, 0, 20, 0, 0, 1, 30], memberIds: [100n] },
      },
    })
    const bounds = worldBoundsForSelection(scene, [{ kind: 'instance', id: 1n }])
    expect(bounds).toEqual({ min: [9.5, 19.5, 29.5], max: [10.5, 20.5, 30.5] })
  })

  it('returns null for a stale instance (no pose)', () => {
    const scene = makeScene({})
    const bounds = worldBoundsForSelection(scene, [{ kind: 'instance', id: 999n }])
    expect(bounds).toBeNull()
  })

  it('does not throw when object_mesh throws for a stale object id (returns null)', () => {
    // `object_mesh` throws (UnknownObject) on a stale/deleted id — the mock
    // reproduces this by throwing for any id with no registered mesh. A
    // selection can name a just-removed object (undo bumps docRev without
    // pruning it), so the readout must degrade to null, not crash.
    const scene = makeScene({})
    let bounds: Bounds | null = null
    expect(() => {
      bounds = worldBoundsForSelection(scene, [{ kind: 'object', id: 42n }])
    }).not.toThrow()
    expect(bounds).toBeNull()
  })

  it('skips a throwing (stale) leaf and unions only the valid ones', () => {
    // Object 1 has a live mesh; object 2 is stale (object_mesh throws). The
    // union is just object 1's box — the stale leaf contributes nothing.
    const scene = makeScene({
      objectMeshes: {
        '1': boxCorners(0.5, 0.5, 0.5),
      },
    })
    let bounds: Bounds | null = null
    expect(() => {
      bounds = worldBoundsForSelection(scene, [
        { kind: 'object', id: 1n },
        { kind: 'object', id: 2n },
      ])
    }).not.toThrow()
    expect(bounds).toEqual({ min: [-0.5, -0.5, -0.5], max: [0.5, 0.5, 0.5] })
  })
})

/**
 * Tests for SceneRenderer: double-sided rendering of open (non-watertight)
 * shells, and the targeted (incremental) refresh path (`refreshTouched`) that
 * rebuilds only touched groups on large documents.
 *
 * An open shell's tessellation can carry inward-wound triangles, so a
 * single-sided material (`FrontSide`/`BackSide`) renders those faces
 * invisible from the "wrong" side (looks like an empty wireframe). Watertight
 * solids keep the previous single-sided behavior (`FrontSide`, or `BackSide`
 * for a reflected instance pose); non-watertight ones always render
 * `THREE.DoubleSide` regardless of reflection.
 *
 * Only object construction is exercised — no renderer/WebGL — so a plain
 * `THREE.Scene()` stands in for the live scene, matching the pattern already
 * used by `InfiniteGrid.test.ts`. The mock WasmScene follows the same
 * plain-object style as `panels/scenePanels.test.tsx`.
 */

import { describe, expect, it, vi } from 'vitest'
import * as THREE from 'three'
import type { Scene as WasmScene } from '../wasm/loader'
import { SceneRenderer } from './SceneRenderer'

const SENTINEL = BigInt('18446744073709551615') // u64::MAX — default material group

/** A one-triangle mesh, shaped like the real `MeshJs` FFI surface. */
function makeMesh(watertight: boolean) {
  return {
    positions: () => new Float32Array([0, 0, 0, 1, 0, 0, 0, 1, 0]),
    normals: () => new Float32Array([0, 0, 1, 0, 0, 1, 0, 0, 1]),
    indices: () => new Uint32Array([0, 1, 2]),
    colors: () => new Float32Array([1, 1, 1, 1, 1, 1, 1, 1, 1]),
    uvs: () => new Float32Array([0, 0, 1, 0, 0, 1]),
    group_material_ids: () => new BigUint64Array([SENTINEL]),
    group_starts: () => new Uint32Array([0]),
    group_counts: () => new Uint32Array([3]),
    edge_positions: () => new Float32Array([0, 0, 0, 1, 0, 0]),
    watertight: () => watertight,
    free: vi.fn(),
  }
}

/** Mock WasmScene; only the methods SceneRenderer's `refresh`/instance path
 * touches are provided. `objects`/`instances` map id → watertight (objects)
 * or → { def, pose, memberWatertight } (instances). */
function makeScene(opts: {
  objects?: Record<string, boolean>
  instances?: Record<string, { def: bigint; pose: number[]; memberIds: bigint[]; memberWatertight: Record<string, boolean> }>
}): WasmScene {
  const objects = opts.objects ?? {}
  const instances = opts.instances ?? {}
  return {
    object_ids: () => BigUint64Array.from(Object.keys(objects).map(BigInt)),
    instance_ids: () => BigUint64Array.from(Object.keys(instances).map(BigInt)),
    // vi.fn so the targeted-refresh tests can count exactly which meshes get
    // re-pulled across the (mock) wasm boundary.
    object_mesh: vi.fn((id: bigint) => {
      if (id.toString() in objects) return makeMesh(objects[id.toString()])
      for (const inst of Object.values(instances)) {
        if (inst.memberIds.includes(id)) return makeMesh(inst.memberWatertight[id.toString()])
      }
      throw new Error(`unexpected object_mesh(${id})`)
    }),
    instance_def: (id: bigint) => instances[id.toString()]?.def,
    instance_pose: (id: bigint) =>
      instances[id.toString()] !== undefined ? Float64Array.from(instances[id.toString()].pose) : undefined,
    component_member_objects: (componentId: bigint) => {
      const inst = Object.values(instances).find((i) => i.def === componentId)
      return BigUint64Array.from(inst?.memberIds ?? [])
    },
    sketch_ids: () => new BigUint64Array(),
    guide_ids: () => new BigUint64Array(),
  } as unknown as WasmScene
}

/** Identity pose (row-major 3x4: no rotation/reflection, zero translation). */
const IDENTITY_POSE = [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0]
/** A pose with a negative determinant (mirrored / reflected instance). */
const REFLECTED_POSE = [-1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0]

function facesMaterial(root: THREE.Group, name: string): THREE.MeshPhongMaterial {
  const group = root.getObjectByName(name)
  if (group === undefined) throw new Error(`no group named ${name}`)
  const facesMesh = group.children[0] as THREE.Mesh
  const mat = facesMesh.material
  return (Array.isArray(mat) ? mat[0] : mat) as THREE.MeshPhongMaterial
}

describe('SceneRenderer — double-sided rendering for open shells', () => {
  it('a watertight object keeps THREE.FrontSide', () => {
    const scene = makeScene({ objects: { '1': true } })
    const renderer = new SceneRenderer(new THREE.Scene(), scene)
    renderer.refresh()
    expect(facesMaterial(renderer.objectsGroup, 'Object_1').side).toBe(THREE.FrontSide)
  })

  it('a non-watertight (open) object renders THREE.DoubleSide', () => {
    const scene = makeScene({ objects: { '2': false } })
    const renderer = new SceneRenderer(new THREE.Scene(), scene)
    renderer.refresh()
    expect(facesMaterial(renderer.objectsGroup, 'Object_2').side).toBe(THREE.DoubleSide)
  })

  it('re-refreshing after an edit updates the side (e.g. a leaky object closed up to watertight)', () => {
    const objects: Record<string, boolean> = { '3': false }
    const scene = makeScene({ objects })
    const renderer = new SceneRenderer(new THREE.Scene(), scene)
    renderer.refresh()
    expect(facesMaterial(renderer.objectsGroup, 'Object_3').side).toBe(THREE.DoubleSide)

    objects['3'] = true
    renderer.refresh()
    expect(facesMaterial(renderer.objectsGroup, 'Object_3').side).toBe(THREE.FrontSide)
  })

  it('a watertight instance member with an identity pose keeps THREE.FrontSide', () => {
    const scene = makeScene({
      instances: {
        '10': { def: 100n, pose: IDENTITY_POSE, memberIds: [1n], memberWatertight: { '1': true } },
      },
    })
    const renderer = new SceneRenderer(new THREE.Scene(), scene)
    renderer.refresh()
    expect(facesMaterial(renderer.instancesGroup, 'Instance_10').side).toBe(THREE.FrontSide)
  })

  it('a watertight instance member with a reflected pose keeps THREE.BackSide', () => {
    const scene = makeScene({
      instances: {
        '11': { def: 101n, pose: REFLECTED_POSE, memberIds: [1n], memberWatertight: { '1': true } },
      },
    })
    const renderer = new SceneRenderer(new THREE.Scene(), scene)
    renderer.refresh()
    expect(facesMaterial(renderer.instancesGroup, 'Instance_11').side).toBe(THREE.BackSide)
  })

  it('a non-watertight instance member renders THREE.DoubleSide even with a reflected pose', () => {
    const scene = makeScene({
      instances: {
        '12': { def: 102n, pose: REFLECTED_POSE, memberIds: [1n], memberWatertight: { '1': false } },
      },
    })
    const renderer = new SceneRenderer(new THREE.Scene(), scene)
    renderer.refresh()
    expect(facesMaterial(renderer.instancesGroup, 'Instance_12').side).toBe(THREE.DoubleSide)
  })
})

/** The mock's `object_mesh` as a spy, for call-count assertions. */
function meshSpy(scene: WasmScene): ReturnType<typeof vi.fn> {
  return (scene as unknown as { object_mesh: ReturnType<typeof vi.fn> }).object_mesh
}

describe('SceneRenderer — targeted refresh (refreshTouched)', () => {
  it('rebuilds only the touched object group and leaves untouched groups alone', () => {
    const scene = makeScene({ objects: { '1': true, '2': true } })
    const renderer = new SceneRenderer(new THREE.Scene(), scene)
    renderer.refresh()
    const untouchedBefore = renderer.objectsGroup.getObjectByName('Object_2')
    const touchedBefore = renderer.objectsGroup.getObjectByName('Object_1')
    meshSpy(scene).mockClear()

    renderer.refreshTouched({ objectIds: [1n] })

    // Only the touched object crossed the wasm boundary again.
    expect(meshSpy(scene)).toHaveBeenCalledTimes(1)
    expect(meshSpy(scene)).toHaveBeenCalledWith(1n)
    // Untouched group is the SAME THREE.Group (no dispose/re-upload);
    // the touched one was replaced.
    expect(renderer.objectsGroup.getObjectByName('Object_2')).toBe(untouchedBefore)
    expect(renderer.objectsGroup.getObjectByName('Object_1')).not.toBe(touchedBefore)
  })

  it('disposes the replaced group\'s GPU resources on a targeted rebuild', () => {
    const scene = makeScene({ objects: { '1': true } })
    const renderer = new SceneRenderer(new THREE.Scene(), scene)
    renderer.refresh()
    const group = renderer.objectsGroup.getObjectByName('Object_1') as THREE.Group
    const oldFaces = group.children[0] as THREE.Mesh
    const disposeSpy = vi.spyOn(oldFaces.geometry, 'dispose')

    renderer.refreshTouched({ objectIds: [1n] })

    expect(disposeSpy).toHaveBeenCalled()
  })

  it('catches creations and deletions the caller could not enumerate (id-set diff)', () => {
    const objects: Record<string, boolean> = { '1': true, '2': true }
    const scene = makeScene({ objects })
    const renderer = new SceneRenderer(new THREE.Scene(), scene)
    renderer.refresh()
    const survivorBefore = renderer.objectsGroup.getObjectByName('Object_1')

    // A through-cut-like mutation: object 2 consumed, object 3 created —
    // the caller only knows "something changed", passes no touched ids.
    delete objects['2']
    objects['3'] = true
    renderer.refreshTouched({})

    expect(renderer.objectsGroup.getObjectByName('Object_2')).toBeUndefined()
    expect(renderer.objectsGroup.getObjectByName('Object_3')).toBeDefined()
    // The untouched survivor was not rebuilt.
    expect(renderer.objectsGroup.getObjectByName('Object_1')).toBe(survivorBefore)
  })

  it('a touched instance rebuilds only that placement, without re-pulling shared member geometry', () => {
    const scene = makeScene({
      instances: {
        '10': { def: 100n, pose: IDENTITY_POSE, memberIds: [1n], memberWatertight: { '1': true } },
        '11': { def: 100n, pose: IDENTITY_POSE, memberIds: [1n], memberWatertight: { '1': true } },
      },
    })
    const renderer = new SceneRenderer(new THREE.Scene(), scene)
    renderer.refresh()
    const otherBefore = renderer.instancesGroup.getObjectByName('Instance_11')
    const touchedBefore = renderer.instancesGroup.getObjectByName('Instance_10')
    meshSpy(scene).mockClear()

    renderer.refreshTouched({ instanceIds: [10n] })

    // Member cache intact (a pose change never invalidates def geometry).
    expect(meshSpy(scene)).not.toHaveBeenCalled()
    expect(renderer.instancesGroup.getObjectByName('Instance_11')).toBe(otherBefore)
    expect(renderer.instancesGroup.getObjectByName('Instance_10')).not.toBe(touchedBefore)
  })

  it('a def-member edit invalidates ALL placements of that definition', () => {
    const scene = makeScene({
      instances: {
        '10': { def: 100n, pose: IDENTITY_POSE, memberIds: [1n], memberWatertight: { '1': true } },
        '11': { def: 100n, pose: IDENTITY_POSE, memberIds: [1n], memberWatertight: { '1': true } },
      },
    })
    const renderer = new SceneRenderer(new THREE.Scene(), scene)
    renderer.refresh()
    const g10 = renderer.instancesGroup.getObjectByName('Instance_10')
    const g11 = renderer.instancesGroup.getObjectByName('Instance_11')
    meshSpy(scene).mockClear()

    // e.g. push_pull_in_component / painting instanced geometry commits the
    // definition MEMBER object id (1n).
    renderer.refreshTouched({ objectIds: [1n] })

    // Shared geometry re-pulled exactly once (cache dropped, then re-filled
    // by the first placement rebuild), and BOTH placements rebuilt.
    expect(meshSpy(scene)).toHaveBeenCalledTimes(1)
    expect(meshSpy(scene)).toHaveBeenCalledWith(1n)
    expect(renderer.instancesGroup.getObjectByName('Instance_10')).not.toBe(g10)
    expect(renderer.instancesGroup.getObjectByName('Instance_11')).not.toBe(g11)
  })

  it('a touched component id also invalidates all placements (components_touched path)', () => {
    const scene = makeScene({
      instances: {
        '10': { def: 100n, pose: IDENTITY_POSE, memberIds: [1n], memberWatertight: { '1': true } },
        '11': { def: 100n, pose: IDENTITY_POSE, memberIds: [1n], memberWatertight: { '1': true } },
      },
    })
    const renderer = new SceneRenderer(new THREE.Scene(), scene)
    renderer.refresh()
    const g10 = renderer.instancesGroup.getObjectByName('Instance_10')
    const g11 = renderer.instancesGroup.getObjectByName('Instance_11')
    meshSpy(scene).mockClear()

    renderer.refreshTouched({ componentIds: [100n] })

    expect(meshSpy(scene)).toHaveBeenCalledTimes(1)
    expect(renderer.instancesGroup.getObjectByName('Instance_10')).not.toBe(g10)
    expect(renderer.instancesGroup.getObjectByName('Instance_11')).not.toBe(g11)
  })

  it('full refresh() still rebuilds every group (fallback path unchanged)', () => {
    const scene = makeScene({ objects: { '1': true, '2': true } })
    const renderer = new SceneRenderer(new THREE.Scene(), scene)
    renderer.refresh()
    const g1 = renderer.objectsGroup.getObjectByName('Object_1')
    const g2 = renderer.objectsGroup.getObjectByName('Object_2')
    meshSpy(scene).mockClear()

    renderer.refresh()

    expect(meshSpy(scene)).toHaveBeenCalledTimes(2)
    expect(renderer.objectsGroup.getObjectByName('Object_1')).not.toBe(g1)
    expect(renderer.objectsGroup.getObjectByName('Object_2')).not.toBe(g2)
  })

  it('refreshTouched returns the full watertight map, like refresh()', () => {
    const scene = makeScene({ objects: { '1': true, '2': false } })
    const renderer = new SceneRenderer(new THREE.Scene(), scene)
    renderer.refresh()

    const wt = renderer.refreshTouched({ objectIds: [1n] })

    expect(wt.get(1n)).toBe(true)
    expect(wt.get(2n)).toBe(false)
  })
})

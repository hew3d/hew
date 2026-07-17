/**
 * Tests for SceneRenderer: double-sided rendering of open (non-watertight)
 * shells, the targeted (incremental) refresh path (`refreshTouched`) that
 * rebuilds only touched groups on large documents, GPU-instanced component
 * placements (RR16), and the interactions between the per-instance visibility
 * mechanisms — hidden set, selection materialization, isolation, and the
 * pose fast path — plus instance-aware bounds for zoom-extents (RR17).
 *
 * An open shell's tessellation can carry inward-wound triangles, so a
 * single-sided material (`FrontSide`/`BackSide`) renders those faces
 * invisible from the "wrong" side (looks like an empty wireframe). Watertight
 * solids keep the previous single-sided behavior; non-watertight ones always
 * render `THREE.DoubleSide` regardless of reflection. Reflected (det < 0)
 * poses need side compensation only on the BATCH path (`BackSide` bucket —
 * per-slot poses live in a shader attribute), never on a MATERIALIZED group,
 * whose `group.matrix` pose already gets the renderer's own winding flip.
 *
 * Instances draw as one THREE.InstancedMesh (+ one instanced-edge
 * LineSegments) per (definition member, side bucket); per-instance state
 * (selection, transform preview) MATERIALIZES the placement into a classic
 * `Instance_${id}` Group and zeroes its batch slot.
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
import { buildInstancePreviewClone } from '../tools/transformPreview'
import { buildSelectionPreview } from '../tools/transformSelection'
import { ScaleTool } from '../tools/ScaleTool'
import type { NodeRef } from '../panels/treeModel'

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

/** All instanced face batches under the renderer's instances group. */
function instancedBatches(root: THREE.Group): THREE.InstancedMesh[] {
  const out: THREE.InstancedMesh[] = []
  root.traverse((o) => {
    if ((o as THREE.InstancedMesh).isInstancedMesh === true) out.push(o as THREE.InstancedMesh)
  })
  return out
}

/** All instanced edge objects (LineSegments over an InstancedBufferGeometry). */
function instancedEdges(root: THREE.Group): THREE.LineSegments[] {
  const out: THREE.LineSegments[] = []
  root.traverse((o) => {
    if (
      o instanceof THREE.LineSegments &&
      (o.geometry as THREE.InstancedBufferGeometry).isInstancedBufferGeometry === true
    ) {
      out.push(o)
    }
  })
  return out
}

/** All MATERIALIZED per-instance groups (`Instance_${id}`). */
function materializedGroups(root: THREE.Group): THREE.Group[] {
  const out: THREE.Group[] = []
  root.traverse((o) => {
    if (o instanceof THREE.Group && o.name.startsWith('Instance_')) out.push(o)
  })
  return out
}

function batchMaterial(mesh: THREE.InstancedMesh): THREE.MeshPhongMaterial {
  const mat = mesh.material
  return (Array.isArray(mat) ? mat[0] : mat) as THREE.MeshPhongMaterial
}

function slotMatrices(mesh: THREE.InstancedMesh): THREE.Matrix4[] {
  const out: THREE.Matrix4[] = []
  for (let i = 0; i < mesh.count; i++) {
    const m = new THREE.Matrix4()
    mesh.getMatrixAt(i, m)
    out.push(m)
  }
  return out
}

/** True when the slot matrix's linear part is all-zero (suppressed slot —
 * zero scale draws nothing; translation is kept for bounding boxes). */
function isDegenerate(m: THREE.Matrix4): boolean {
  const e = m.elements
  return [0, 1, 2, 4, 5, 6, 8, 9, 10].every((i) => e[i] === 0)
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
    const batches = instancedBatches(renderer.instancesGroup)
    expect(batches).toHaveLength(1)
    expect(batchMaterial(batches[0]).side).toBe(THREE.FrontSide)
  })

  it('a watertight instance member with a reflected pose keeps THREE.BackSide', () => {
    const scene = makeScene({
      instances: {
        '11': { def: 101n, pose: REFLECTED_POSE, memberIds: [1n], memberWatertight: { '1': true } },
      },
    })
    const renderer = new SceneRenderer(new THREE.Scene(), scene)
    renderer.refresh()
    const batches = instancedBatches(renderer.instancesGroup)
    expect(batches).toHaveLength(1)
    expect(batchMaterial(batches[0]).side).toBe(THREE.BackSide)
  })

  it('a non-watertight instance member renders THREE.DoubleSide even with a reflected pose', () => {
    const scene = makeScene({
      instances: {
        '12': { def: 102n, pose: REFLECTED_POSE, memberIds: [1n], memberWatertight: { '1': false } },
      },
    })
    const renderer = new SceneRenderer(new THREE.Scene(), scene)
    renderer.refresh()
    const batches = instancedBatches(renderer.instancesGroup)
    expect(batches).toHaveLength(1)
    expect(batchMaterial(batches[0]).side).toBe(THREE.DoubleSide)
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

  it('a touched instance updates its batch slot in place, without re-pulling shared member geometry', () => {
    const instances = {
      '10': { def: 100n, pose: [...IDENTITY_POSE], memberIds: [1n], memberWatertight: { '1': true } },
      '11': { def: 100n, pose: [...IDENTITY_POSE], memberIds: [1n], memberWatertight: { '1': true } },
    }
    const scene = makeScene({ instances })
    const renderer = new SceneRenderer(new THREE.Scene(), scene)
    renderer.refresh()
    const batchBefore = instancedBatches(renderer.instancesGroup)[0]
    meshSpy(scene).mockClear()

    // Move instance 10 by +5 in X (row-major 3×4: tx is element 3).
    instances['10'].pose = [1, 0, 0, 5, 0, 1, 0, 0, 0, 0, 1, 0]
    renderer.refreshTouched({ instanceIds: [10n] })

    // Member cache intact (a pose change never invalidates def geometry),
    // and the batch is the SAME object — no GPU buffer rebuild.
    expect(meshSpy(scene)).not.toHaveBeenCalled()
    const batchAfter = instancedBatches(renderer.instancesGroup)[0]
    expect(batchAfter).toBe(batchBefore)
    // One slot carries the new translation; the other is untouched.
    const tx = slotMatrices(batchAfter).map((m) => m.elements[12])
    expect(tx).toContain(5)
    expect(tx).toContain(0)
    // The instanced edge rows follow the same pose (imRow0.w = tx).
    const edgeGeo = instancedEdges(renderer.instancesGroup)[0]
      .geometry as THREE.InstancedBufferGeometry
    const row0 = edgeGeo.getAttribute('imRow0') as THREE.InstancedBufferAttribute
    const edgeTx = [row0.getW(0), row0.getW(1)]
    expect(edgeTx).toContain(5)
    expect(edgeTx).toContain(0)
  })

  it('a def-member edit rebuilds the member batch once for ALL placements', () => {
    const scene = makeScene({
      instances: {
        '10': { def: 100n, pose: IDENTITY_POSE, memberIds: [1n], memberWatertight: { '1': true } },
        '11': { def: 100n, pose: IDENTITY_POSE, memberIds: [1n], memberWatertight: { '1': true } },
      },
    })
    const renderer = new SceneRenderer(new THREE.Scene(), scene)
    renderer.refresh()
    const batchBefore = instancedBatches(renderer.instancesGroup)[0]
    meshSpy(scene).mockClear()

    // e.g. push_pull_in_component / painting instanced geometry commits the
    // definition MEMBER object id (1n).
    renderer.refreshTouched({ objectIds: [1n] })

    // Shared geometry re-pulled exactly once (cache dropped, then re-filled
    // by the batch rebuild), and the batch replaced, still covering both
    // placements.
    expect(meshSpy(scene)).toHaveBeenCalledTimes(1)
    expect(meshSpy(scene)).toHaveBeenCalledWith(1n)
    const batches = instancedBatches(renderer.instancesGroup)
    expect(batches).toHaveLength(1)
    expect(batches[0]).not.toBe(batchBefore)
    expect(batches[0].count).toBe(2)
  })

  it('a touched component id also rebuilds the member batches (components_touched path)', () => {
    const scene = makeScene({
      instances: {
        '10': { def: 100n, pose: IDENTITY_POSE, memberIds: [1n], memberWatertight: { '1': true } },
        '11': { def: 100n, pose: IDENTITY_POSE, memberIds: [1n], memberWatertight: { '1': true } },
      },
    })
    const renderer = new SceneRenderer(new THREE.Scene(), scene)
    renderer.refresh()
    const batchBefore = instancedBatches(renderer.instancesGroup)[0]
    meshSpy(scene).mockClear()

    renderer.refreshTouched({ componentIds: [100n] })

    expect(meshSpy(scene)).toHaveBeenCalledTimes(1)
    const batches = instancedBatches(renderer.instancesGroup)
    expect(batches).toHaveLength(1)
    expect(batches[0]).not.toBe(batchBefore)
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

describe('SceneRenderer — GPU-instanced placements (RR16)', () => {
  /** Three placements of one single-member definition. */
  function threePlacements() {
    return makeScene({
      instances: {
        '10': { def: 100n, pose: [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0], memberIds: [1n], memberWatertight: { '1': true } },
        '11': { def: 100n, pose: [1, 0, 0, 2, 0, 1, 0, 0, 0, 0, 1, 0], memberIds: [1n], memberWatertight: { '1': true } },
        '12': { def: 100n, pose: [1, 0, 0, 4, 0, 1, 0, 0, 0, 0, 1, 0], memberIds: [1n], memberWatertight: { '1': true } },
      },
    })
  }

  it('N placements of one definition draw as ONE batch (InstancedMesh + instanced edges), not N groups', () => {
    const renderer = new SceneRenderer(new THREE.Scene(), threePlacements())
    renderer.refresh()

    const batches = instancedBatches(renderer.instancesGroup)
    expect(batches).toHaveLength(1)
    expect(batches[0].count).toBe(3)
    const edges = instancedEdges(renderer.instancesGroup)
    expect(edges).toHaveLength(1)
    expect((edges[0].geometry as THREE.InstancedBufferGeometry).instanceCount).toBe(3)
    expect(materializedGroups(renderer.instancesGroup)).toHaveLength(0)

    // Draw-object count via traversal: exactly 2 renderable objects (the
    // batch mesh + the batch edges) regardless of placement count.
    let drawObjects = 0
    renderer.instancesGroup.traverse((o) => {
      if ((o as THREE.Mesh).isMesh === true || o instanceof THREE.LineSegments) drawObjects++
    })
    expect(drawObjects).toBe(2)

    // Every slot carries its placement's translation.
    const tx = slotMatrices(batches[0]).map((m) => m.elements[12]).sort()
    expect(tx).toEqual([0, 2, 4])
  })

  it('selecting an instance materializes it (orange edges) and zero-scales its batch slot; deselecting restores it', () => {
    const renderer = new SceneRenderer(new THREE.Scene(), threePlacements())
    renderer.refresh()

    renderer.setSelectedInstances([11n])

    const group = renderer.instancesGroup.getObjectByName('Instance_11') as THREE.Group
    expect(group).toBeDefined()
    const edgeLines = group.children.find((c) => c instanceof THREE.LineSegments) as THREE.LineSegments
    expect((edgeLines.material as THREE.LineBasicMaterial).color.getHex()).toBe(0xffaa00)
    // The materialized group carries the placement pose.
    expect(group.matrix.elements[12]).toBe(2)
    // Its batch slot is suppressed (zero linear part, translation kept);
    // the other two slots still draw.
    const batch = instancedBatches(renderer.instancesGroup)[0]
    const slots = slotMatrices(batch)
    expect(slots.filter(isDegenerate)).toHaveLength(1)
    expect(slots.find(isDegenerate)?.elements[12]).toBe(2)

    renderer.setSelectedInstances([])

    expect(renderer.instancesGroup.getObjectByName('Instance_11')).toBeUndefined()
    expect(slotMatrices(batch).filter(isDegenerate)).toHaveLength(0)
  })

  it('hidden instances zero their slot without materializing; setHidden round-trips', () => {
    const renderer = new SceneRenderer(new THREE.Scene(), threePlacements())
    renderer.refresh()
    const batch = instancedBatches(renderer.instancesGroup)[0]

    renderer.setHidden([], [10n])

    expect(materializedGroups(renderer.instancesGroup)).toHaveLength(0)
    expect(slotMatrices(batch).filter(isDegenerate)).toHaveLength(1)

    renderer.setHidden([], [])

    expect(slotMatrices(batch).filter(isDegenerate)).toHaveLength(0)
  })

  it('reflected placements split into their own bucket with BackSide', () => {
    const scene = makeScene({
      instances: {
        '10': { def: 100n, pose: IDENTITY_POSE, memberIds: [1n], memberWatertight: { '1': true } },
        '11': { def: 100n, pose: REFLECTED_POSE, memberIds: [1n], memberWatertight: { '1': true } },
      },
    })
    const renderer = new SceneRenderer(new THREE.Scene(), scene)
    renderer.refresh()

    const batches = instancedBatches(renderer.instancesGroup)
    expect(batches).toHaveLength(2)
    expect(batches.map((b) => b.count)).toEqual([1, 1])
    const sides = batches.map((b) => batchMaterial(b).side).sort()
    expect(sides).toEqual([THREE.FrontSide, THREE.BackSide].sort())
    expect(instancedEdges(renderer.instancesGroup)).toHaveLength(2)
  })

  it('a selected reflected placement materializes with FrontSide (renderer already flips winding for det<0 world matrices)', () => {
    const scene = makeScene({
      instances: {
        '11': { def: 101n, pose: REFLECTED_POSE, memberIds: [1n], memberWatertight: { '1': true } },
      },
    })
    const renderer = new SceneRenderer(new THREE.Scene(), scene)
    renderer.refresh()

    renderer.setSelectedInstances([11n])

    const group = renderer.instancesGroup.getObjectByName('Instance_11') as THREE.Group
    expect(group).toBeDefined()
    // The materialized group carries the reflected pose as an Object3D
    // matrix — WebGLRenderer reverses the front-face winding for any Mesh
    // whose world matrix has a negative determinant, so the materials must
    // stay FrontSide. BackSide here double-flips: the solid renders
    // inside-out and per-face paint disappears behind the culled faces.
    expect(group.matrix.determinant()).toBeLessThan(0)
    const faceMesh = group.children.find((c) => (c as THREE.Mesh).isMesh === true) as THREE.Mesh
    const mats = Array.isArray(faceMesh.material) ? faceMesh.material : [faceMesh.material]
    expect(mats.length).toBeGreaterThan(0)
    for (const m of mats) {
      expect((m as THREE.MeshPhongMaterial).side).toBe(THREE.FrontSide)
    }
    // The batch bucket keeps its BackSide compensation — there the pose
    // lives in a per-slot shader attribute the renderer's determinant
    // check cannot see.
    const batch = instancedBatches(renderer.instancesGroup)[0]
    expect(batchMaterial(batch).side).toBe(THREE.BackSide)
  })

  it('a non-watertight member of a selected reflected placement still materializes DoubleSide', () => {
    const scene = makeScene({
      instances: {
        '12': { def: 102n, pose: REFLECTED_POSE, memberIds: [1n], memberWatertight: { '1': false } },
      },
    })
    const renderer = new SceneRenderer(new THREE.Scene(), scene)
    renderer.refresh()

    renderer.setSelectedInstances([12n])

    const group = renderer.instancesGroup.getObjectByName('Instance_12') as THREE.Group
    const faceMesh = group.children.find((c) => (c as THREE.Mesh).isMesh === true) as THREE.Mesh
    const mats = Array.isArray(faceMesh.material) ? faceMesh.material : [faceMesh.material]
    for (const m of mats) {
      expect((m as THREE.MeshPhongMaterial).side).toBe(THREE.DoubleSide)
    }
  })

  it('getInstanceGroup materializes on demand for transform preview and restores on the next refresh', () => {
    const renderer = new SceneRenderer(new THREE.Scene(), threePlacements())
    renderer.refresh()
    expect(materializedGroups(renderer.instancesGroup)).toHaveLength(0)

    const group = renderer.getInstanceGroup(12n)

    expect(group).not.toBeNull()
    expect(group?.name).toBe('Instance_12')
    expect(renderer.instancesGroup.getObjectByName('Instance_12')).toBe(group)
    const batch = instancedBatches(renderer.instancesGroup)[0]
    expect(slotMatrices(batch).filter(isDegenerate)).toHaveLength(1)

    // The commit that ends a preview refreshes the scene — the placement
    // returns to its batch.
    renderer.refreshTouched({})

    expect(renderer.instancesGroup.getObjectByName('Instance_12')).toBeUndefined()
    expect(slotMatrices(batch).filter(isDegenerate)).toHaveLength(0)
  })

  it('a selected instance survives a full refresh materialized (selection re-applied)', () => {
    const renderer = new SceneRenderer(new THREE.Scene(), threePlacements())
    renderer.refresh()
    renderer.setSelectedInstances([10n])

    renderer.refresh()

    const group = renderer.instancesGroup.getObjectByName('Instance_10') as THREE.Group
    expect(group).toBeDefined()
    const edgeLines = group.children.find((c) => c instanceof THREE.LineSegments) as THREE.LineSegments
    expect((edgeLines.material as THREE.LineBasicMaterial).color.getHex()).toBe(0xffaa00)
  })

  it('buildExportScene still emits node-per-instance with shared geometry', () => {
    const scene = makeScene({
      objects: { '1': true },
      instances: {
        '10': { def: 100n, pose: [1, 0, 0, 3, 0, 1, 0, 0, 0, 0, 1, 0], memberIds: [2n], memberWatertight: { '2': true } },
        '11': { def: 100n, pose: IDENTITY_POSE, memberIds: [2n], memberWatertight: { '2': true } },
      },
    })
    const renderer = new SceneRenderer(new THREE.Scene(), scene)
    renderer.refresh()

    const root = renderer.buildExportScene()

    expect(root.getObjectByName('Object_1')).toBeDefined()
    const node10 = root.getObjectByName('Instance_10') as THREE.Group
    const node11 = root.getObjectByName('Instance_11') as THREE.Group
    expect(node10).toBeDefined()
    expect(node11).toBeDefined()
    // The node carries the pose as a node transform.
    expect(node10.matrix.elements[12]).toBe(3)
    const member = node10.getObjectByName('Instance_10_member_2') as THREE.Mesh
    expect(member).toBeDefined()
    // Geometry is shared by reference with the live batch; materials are
    // fresh MeshStandardMaterial.
    const batch = instancedBatches(renderer.instancesGroup)[0]
    expect(member.geometry).toBe(batch.geometry)
    const mat = Array.isArray(member.material) ? member.material[0] : member.material
    expect((mat as THREE.MeshStandardMaterial).isMeshStandardMaterial).toBe(true)

    renderer.disposeExportScene(root)
  })

  it('adding a placement of an existing definition rebuilds its batch to cover it', () => {
    const instances: Record<string, { def: bigint; pose: number[]; memberIds: bigint[]; memberWatertight: Record<string, boolean> }> = {
      '10': { def: 100n, pose: IDENTITY_POSE, memberIds: [1n], memberWatertight: { '1': true } },
    }
    const scene = makeScene({ instances })
    const renderer = new SceneRenderer(new THREE.Scene(), scene)
    renderer.refresh()
    expect(instancedBatches(renderer.instancesGroup)[0].count).toBe(1)

    instances['11'] = { def: 100n, pose: [1, 0, 0, 7, 0, 1, 0, 0, 0, 0, 1, 0], memberIds: [1n], memberWatertight: { '1': true } }
    renderer.refreshTouched({ instanceIds: [11n] })

    const batches = instancedBatches(renderer.instancesGroup)
    expect(batches).toHaveLength(1)
    expect(batches[0].count).toBe(2)
    const tx = slotMatrices(batches[0]).map((m) => m.elements[12]).sort()
    expect(tx).toEqual([0, 7])
  })
})

describe('buildInstancePreviewClone — drag ghost sits on the selected instance', () => {
  /** World-space bounding-box center of an object (after world matrices update). */
  function worldCenter(obj: THREE.Object3D): THREE.Vector3 {
    return new THREE.Box3().setFromObject(obj).getCenter(new THREE.Vector3())
  }

  // Definition-local geometry (one triangle + one edge) spans x[0,1] y[0,1]
  // z[0,0]; its local bbox center is (0.5, 0.5, 0). The OLD buggy clone reset
  // the pose to identity, so the ghost sat here regardless of the placement.
  const DEF_ORIGIN_CENTER: [number, number, number] = [0.5, 0.5, 0]

  // Poses with axis-aligned (diagonal) linear parts so the transformed AABB
  // center is exactly the pose applied to the local center — covers the four
  // cases the bug report names: plain, translated-far, mirrored, scaled.
  const cases: Array<[string, number[]]> = [
    ['plain (identity)', IDENTITY_POSE],
    ['translated far from the origin', [1, 0, 0, 100, 0, 1, 0, -50, 0, 0, 1, 30]],
    ['mirrored (negative determinant)', REFLECTED_POSE],
    ['non-uniformly scaled', [2, 0, 0, 4, 0, 3, 0, -7, 0, 0, 0.5, 9]],
  ]

  for (const [label, pose] of cases) {
    it(`preview world center matches the instance, not the definition origin — ${label}`, () => {
      const scene3 = new THREE.Scene()
      const renderer = new SceneRenderer(scene3, makeScene({
        instances: {
          '20': { def: 200n, pose, memberIds: [1n], memberWatertight: { '1': true } },
        },
      }))
      renderer.refresh()

      const live = renderer.getInstanceGroup(20n)
      expect(live).not.toBeNull()
      const preview = buildInstancePreviewClone(live)
      expect(preview).not.toBeNull()
      scene3.add(preview!)
      scene3.updateMatrixWorld(true)

      const previewC = worldCenter(preview!)
      const liveC = worldCenter(live!)
      expect(previewC.x).toBeCloseTo(liveC.x, 6)
      expect(previewC.y).toBeCloseTo(liveC.y, 6)
      expect(previewC.z).toBeCloseTo(liveC.z, 6)
    })
  }

  it('a far placement ghost sits at the placement, not the definition origin (regression guard)', () => {
    const scene3 = new THREE.Scene()
    const renderer = new SceneRenderer(scene3, makeScene({
      instances: {
        '20': { def: 200n, pose: [1, 0, 0, 100, 0, 1, 0, -50, 0, 0, 1, 30], memberIds: [1n], memberWatertight: { '1': true } },
      },
    }))
    renderer.refresh()

    const preview = buildInstancePreviewClone(renderer.getInstanceGroup(20n))!
    scene3.add(preview)
    scene3.updateMatrixWorld(true)

    const c = worldCenter(preview)
    // Pose applied to the local center (0.5, 0.5, 0): (100.5, -49.5, 30).
    expect(c.x).toBeCloseTo(100.5, 6)
    expect(c.y).toBeCloseTo(-49.5, 6)
    expect(c.z).toBeCloseTo(30, 6)
    // NOT at the definition origin — the exact symptom the maintainer saw.
    expect(c.distanceTo(new THREE.Vector3(...DEF_ORIGIN_CENTER))).toBeGreaterThan(50)
  })

  it('selecting the SECOND of two far-apart placements ghosts on the second, not the first', () => {
    const scene3 = new THREE.Scene()
    const renderer = new SceneRenderer(scene3, makeScene({
      instances: {
        '20': { def: 200n, pose: IDENTITY_POSE, memberIds: [1n], memberWatertight: { '1': true } },
        '21': { def: 200n, pose: [1, 0, 0, 80, 0, 1, 0, 0, 0, 0, 1, 0], memberIds: [1n], memberWatertight: { '1': true } },
      },
    }))
    renderer.refresh()

    const preview = buildInstancePreviewClone(renderer.getInstanceGroup(21n))!
    scene3.add(preview)
    scene3.updateMatrixWorld(true)

    const c = worldCenter(preview)
    // The second placement is at x≈80.5, not the first at x≈0.5.
    expect(c.x).toBeCloseTo(80.5, 6)
  })

  it('driving the wrapper position translates the ghost from the instance pose', () => {
    const scene3 = new THREE.Scene()
    const renderer = new SceneRenderer(scene3, makeScene({
      instances: {
        '20': { def: 200n, pose: [1, 0, 0, 100, 0, 1, 0, -50, 0, 0, 1, 30], memberIds: [1n], memberWatertight: { '1': true } },
      },
    }))
    renderer.refresh()

    const preview = buildInstancePreviewClone(renderer.getInstanceGroup(20n))!
    scene3.add(preview)
    scene3.updateMatrixWorld(true)
    const start = worldCenter(preview)

    // Mirror how MoveTool drives the ghost: set the outer wrapper's position
    // to the world-space drag delta.
    preview.position.set(3, 7, -2)
    scene3.updateMatrixWorld(true)
    const moved = worldCenter(preview)

    expect(moved.x).toBeCloseTo(start.x + 3, 6)
    expect(moved.y).toBeCloseTo(start.y + 7, 6)
    expect(moved.z).toBeCloseTo(start.z - 2, 6)
  })
})

describe('group drag preview + scale pivot include leaf INSTANCES, not just objects', () => {
  // A group of one world object (near the origin) and one component instance
  // placed far away in +X. The object's mesh spans x[0,1]; the instance is at
  // x≈100. `node_leaf_objects` stops at instances, so a group preview built
  // from it alone would omit the instance entirely.
  function groupOfObjectAndFarInstance() {
    return new SceneRenderer(new THREE.Scene(), makeScene({
      objects: { '1': true },
      instances: {
        '20': { def: 200n, pose: [1, 0, 0, 100, 0, 1, 0, 0, 0, 0, 1, 0], memberIds: [2n], memberWatertight: { '2': true } },
      },
    }))
  }

  // Minimal wasm surface the group-preview / pivot enumeration needs: a group
  // whose members are the world object AND the placed instance.
  const groupMembersScene = {
    group_members: () => [
      { kind: 'object', id: 1n },
      { kind: 'instance', id: 20n },
    ],
  } as unknown as WasmScene

  const groupNode: NodeRef = { kind: 'group', id: 999n }

  it('the group drag ghost spans BOTH the object and the far instance', () => {
    const renderer = groupOfObjectAndFarInstance()
    renderer.refresh()

    const preview = buildSelectionPreview(
      groupMembersScene,
      renderer.objectsGroup,
      (id) => renderer.getInstanceGroup(id),
      [groupNode],
    )
    expect(preview).not.toBeNull()

    const scene3 = new THREE.Scene()
    scene3.add(preview!)
    scene3.updateMatrixWorld(true)
    const box = new THREE.Box3().setFromObject(preview!)

    // Object leaf contributes near the origin; instance leaf contributes at
    // x≈100. Without the instance the ghost would stop near x=1 (the bug).
    expect(box.min.x).toBeLessThan(1.5)
    expect(box.max.x).toBeGreaterThan(99)
  })

  it('ScaleTool pivot for the group sits at the combined center, not just the object', () => {
    const renderer = groupOfObjectAndFarInstance()
    renderer.refresh()

    const tool = new ScaleTool(
      groupMembersScene,
      new THREE.Group(),
      renderer.objectsGroup,
      [],
      () => { /* onCommit */ },
      () => { /* onToast */ },
      (id) => renderer.getInstanceGroup(id),
    )
    const center = (tool as unknown as {
      _selectionCenter(nodes: NodeRef[]): [number, number, number] | null
    })._selectionCenter([groupNode])

    expect(center).not.toBeNull()
    // Object center x≈0.5, instance center x≈100.5 → combined center x≈50.5.
    // Object-only (the bug) would land near x≈0.5.
    expect(center![0]).toBeGreaterThan(40)
    expect(center![0]).toBeLessThan(60)
  })
})

describe('SceneRenderer — selection highlight survives object rebuilds', () => {
  const EDGE_SELECTED = 0xffaa00
  const EDGE_NORMAL = 0x1a1a1a

  /** The edge-lines material color of the named object group. */
  function edgeColorOf(root: THREE.Group, name: string): number {
    const group = root.getObjectByName(name)
    if (group === undefined) throw new Error(`no group named ${name}`)
    const edges = group.children[1] as THREE.LineSegments
    return (edges.material as THREE.LineBasicMaterial).color.getHex()
  }

  it('a selected object keeps its orange edges across a targeted refresh (transform-commit path)', () => {
    // The maintainer's repro: rotate a selected cube 90° — the commit's
    // targeted refresh rebuilt the object's scene nodes and the outline
    // vanished while the Outliner still showed it selected. The rebuild
    // must re-apply the highlight to the fresh nodes.
    const scene = makeScene({ objects: { '1': true } })
    const renderer = new SceneRenderer(new THREE.Scene(), scene)
    renderer.refresh()
    renderer.setSelected([1n])
    expect(edgeColorOf(renderer.objectsGroup, 'Object_1')).toBe(EDGE_SELECTED)

    renderer.refreshTouched({ objectIds: [1n] })
    expect(edgeColorOf(renderer.objectsGroup, 'Object_1')).toBe(EDGE_SELECTED)
  })

  it('a selected object keeps its orange edges across a full refresh (boolean/undo path)', () => {
    const scene = makeScene({ objects: { '1': true } })
    const renderer = new SceneRenderer(new THREE.Scene(), scene)
    renderer.refresh()
    renderer.setSelected([1n])

    renderer.refresh()
    expect(edgeColorOf(renderer.objectsGroup, 'Object_1')).toBe(EDGE_SELECTED)
  })

  it('deselecting after a rebuild restores the normal edge color (selection list bookkeeping)', () => {
    // Guards the fix's bookkeeping: the rebuilt object must be BACK in the
    // renderer's selected list, or a later setSelected([]) would skip the
    // restore and strand the orange edges.
    const scene = makeScene({ objects: { '1': true } })
    const renderer = new SceneRenderer(new THREE.Scene(), scene)
    renderer.refresh()
    renderer.setSelected([1n])
    renderer.refreshTouched({ objectIds: [1n] })

    renderer.setSelected([])
    expect(edgeColorOf(renderer.objectsGroup, 'Object_1')).toBe(EDGE_NORMAL)
  })

  it('a deleted object drops cleanly out of the selection list (no resurrection)', () => {
    const objects: Record<string, boolean> = { '1': true, '2': true }
    const scene = makeScene({ objects })
    const renderer = new SceneRenderer(new THREE.Scene(), scene)
    renderer.refresh()
    renderer.setSelected([1n, 2n])

    delete objects['2']
    renderer.refresh()
    expect(renderer.objectsGroup.getObjectByName('Object_2')).toBeUndefined()
    // The survivor keeps its highlight; the dead id is gone from the list.
    expect(edgeColorOf(renderer.objectsGroup, 'Object_1')).toBe(EDGE_SELECTED)
    renderer.setSelected([])
    expect(edgeColorOf(renderer.objectsGroup, 'Object_1')).toBe(EDGE_NORMAL)
  })
})

describe('SceneRenderer — instance-aware bounds for zoom-extents (RR17)', () => {
  it('batch edge geometry bounds follow the instance poses, not the definition-space soup', () => {
    // One placement far from the origin. The edge geometry's own position
    // attribute spans [0,1] in definition space; without the bounds
    // delegation to the face InstancedMesh, Box3.expandByObject would union
    // in that phantom region at the origin.
    const scene = makeScene({
      instances: {
        '10': { def: 100n, pose: [1, 0, 0, 100, 0, 1, 0, 0, 0, 0, 1, 0], memberIds: [1n], memberWatertight: { '1': true } },
      },
    })
    const renderer = new SceneRenderer(new THREE.Scene(), scene)
    renderer.refresh()

    const box = new THREE.Box3().expandByObject(renderer.instancesGroup)
    expect(box.min.x).toBeGreaterThanOrEqual(99)
    expect(box.max.x).toBeLessThanOrEqual(102)

    // The bounding sphere delegates the same way.
    const edgeGeo = instancedEdges(renderer.instancesGroup)[0].geometry
    edgeGeo.computeBoundingSphere()
    expect(edgeGeo.boundingSphere?.center.x).toBeGreaterThanOrEqual(99)
  })

  it('a pose fast-path write invalidates the edge bounds (zoom-extents follows the move)', () => {
    const instances = {
      '10': { def: 100n, pose: [...IDENTITY_POSE], memberIds: [1n], memberWatertight: { '1': true } },
    }
    const scene = makeScene({ instances })
    const renderer = new SceneRenderer(new THREE.Scene(), scene)
    renderer.refresh()

    // Compute (and thereby cache) the bounds once at the identity pose.
    const before = new THREE.Box3().expandByObject(renderer.instancesGroup)
    expect(before.max.x).toBeLessThanOrEqual(2)

    // Move the instance via the in-place slot-write fast path.
    instances['10'].pose = [1, 0, 0, 50, 0, 1, 0, 0, 0, 0, 1, 0]
    renderer.refreshTouched({ instanceIds: [10n] })

    const after = new THREE.Box3().expandByObject(renderer.instancesGroup)
    expect(after.min.x).toBeGreaterThanOrEqual(49)
    expect(after.max.x).toBeLessThanOrEqual(52)
  })

  it('a hidden placement contributes nothing to bounds, not even its translation', () => {
    // The guest-house regression: a file-hidden stray instance ~900 m out.
    // Its degenerate slot keeps the translation (so restoring is a matrix
    // write), but bounds must skip it entirely — otherwise zoom-extents
    // frames the stray and the camera re-frames past its own far plane.
    const scene = makeScene({
      instances: {
        '10': { def: 100n, pose: [...IDENTITY_POSE], memberIds: [1n], memberWatertight: { '1': true } },
        '11': { def: 100n, pose: [1, 0, 0, -921, 0, 1, 0, -144, 0, 0, 1, 0], memberIds: [1n], memberWatertight: { '1': true } },
      },
    })
    const renderer = new SceneRenderer(new THREE.Scene(), scene)
    renderer.refresh()
    renderer.setHidden([], [11n])

    const box = new THREE.Box3().expandByObject(renderer.instancesGroup)
    expect(box.min.x).toBeGreaterThanOrEqual(-1)
    expect(box.max.x).toBeLessThanOrEqual(2)

    // Unhide: the stray placement's true pose returns to the bounds.
    renderer.setHidden([], [])
    const unhidden = new THREE.Box3().expandByObject(renderer.instancesGroup)
    expect(unhidden.min.x).toBeLessThanOrEqual(-920)
  })

  it('a batch whose every placement is hidden yields empty bounds', () => {
    const scene = makeScene({
      instances: {
        '10': { def: 100n, pose: [1, 0, 0, 77, 0, 1, 0, 0, 0, 0, 1, 0], memberIds: [1n], memberWatertight: { '1': true } },
      },
    })
    const renderer = new SceneRenderer(new THREE.Scene(), scene)
    renderer.refresh()
    renderer.setHidden([], [10n])

    const box = new THREE.Box3().expandByObject(renderer.instancesGroup)
    expect(box.isEmpty()).toBe(true)
  })
})

describe('SceneRenderer — hidden/selected/isolation interactions (RR17)', () => {
  /** Three placements of one single-member definition, with mutable poses. */
  function threePlacementsMutable() {
    const instances: Record<string, { def: bigint; pose: number[]; memberIds: bigint[]; memberWatertight: Record<string, boolean> }> = {
      '10': { def: 100n, pose: [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0], memberIds: [1n], memberWatertight: { '1': true } },
      '11': { def: 100n, pose: [1, 0, 0, 2, 0, 1, 0, 0, 0, 0, 1, 0], memberIds: [1n], memberWatertight: { '1': true } },
      '12': { def: 100n, pose: [1, 0, 0, 4, 0, 1, 0, 0, 0, 0, 1, 0], memberIds: [1n], memberWatertight: { '1': true } },
    }
    return { instances, scene: makeScene({ instances }) }
  }

  it('hiding a selected (materialized) instance hides its Group; unhide restores visibility, slot stays suppressed', () => {
    const { scene } = threePlacementsMutable()
    const renderer = new SceneRenderer(new THREE.Scene(), scene)
    renderer.refresh()
    renderer.setSelectedInstances([11n])
    const group = renderer.instancesGroup.getObjectByName('Instance_11') as THREE.Group
    const batch = instancedBatches(renderer.instancesGroup)[0]
    expect(group.visible).toBe(true)

    renderer.setHidden([], [11n])

    expect(group.visible).toBe(false)
    // The slot stays suppressed (it was already, for materialization).
    expect(slotMatrices(batch).filter(isDegenerate)).toHaveLength(1)

    renderer.setHidden([], [])

    // Unhidden but still selected: visible group, slot still suppressed so
    // the placement is not drawn twice.
    expect(group.visible).toBe(true)
    expect(slotMatrices(batch).filter(isDegenerate)).toHaveLength(1)
  })

  it('selecting an already-hidden instance materializes it invisible; deselect leaves the slot degenerate (still hidden)', () => {
    const { scene } = threePlacementsMutable()
    const renderer = new SceneRenderer(new THREE.Scene(), scene)
    renderer.refresh()
    const batch = instancedBatches(renderer.instancesGroup)[0]
    renderer.setHidden([], [11n])

    renderer.setSelectedInstances([11n])

    const group = renderer.instancesGroup.getObjectByName('Instance_11') as THREE.Group
    expect(group).toBeDefined()
    // The materialized group respects the hidden set.
    expect(group.visible).toBe(false)
    expect(slotMatrices(batch).filter(isDegenerate)).toHaveLength(1)

    renderer.setSelectedInstances([])

    // Deselecting must NOT restore the slot to the live pose — the instance
    // is still hidden.
    expect(renderer.instancesGroup.getObjectByName('Instance_11')).toBeUndefined()
    const slots = slotMatrices(batch)
    expect(slots.filter(isDegenerate)).toHaveLength(1)
    expect(slots.find(isDegenerate)?.elements[12]).toBe(2)
  })

  it('a pose fast-path write on a hidden instance keeps the slot degenerate; the new pose lands on unhide', () => {
    const { instances, scene } = threePlacementsMutable()
    const renderer = new SceneRenderer(new THREE.Scene(), scene)
    renderer.refresh()
    renderer.setHidden([], [10n])
    const batch = instancedBatches(renderer.instancesGroup)[0]

    // Move the hidden instance via the in-place slot-write fast path.
    instances['10'].pose = [1, 0, 0, 9, 0, 1, 0, 0, 0, 0, 1, 0]
    renderer.refreshTouched({ instanceIds: [10n] })

    // The write must not resurrect the instance: still exactly one
    // suppressed slot, now carrying the NEW translation (kept for bounds).
    const slots = slotMatrices(batch)
    const degenIdx = slots.findIndex(isDegenerate)
    expect(slots.filter(isDegenerate)).toHaveLength(1)
    expect(slots[degenIdx].elements[12]).toBe(9)
    // The instanced edge rows are suppressed the same way (zero linear part,
    // translation kept in imRow0.w).
    const edgeGeo = instancedEdges(renderer.instancesGroup)[0]
      .geometry as THREE.InstancedBufferGeometry
    const row0 = edgeGeo.getAttribute('imRow0') as THREE.InstancedBufferAttribute
    expect(row0.getX(degenIdx)).toBe(0)
    expect(row0.getW(degenIdx)).toBe(9)

    renderer.setHidden([], [])

    // Unhiding restores the live pose written while hidden.
    const live = slotMatrices(batch)
    expect(live.filter(isDegenerate)).toHaveLength(0)
    expect(live.map((m) => m.elements[12])).toContain(9)
  })

  it('isolation (setActiveContext) materializes the lit instance, dims the batch, and exit restores the batch', () => {
    const { scene } = threePlacementsMutable()
    const renderer = new SceneRenderer(new THREE.Scene(), scene)
    renderer.refresh()
    const batch = instancedBatches(renderer.instancesGroup)[0]

    renderer.setActiveContext(null, new Set([11n]))

    // The lit placement materializes at full strength...
    const group = renderer.instancesGroup.getObjectByName('Instance_11') as THREE.Group
    expect(group).toBeDefined()
    const faceMesh = group.children.find((c) => (c as THREE.Mesh).isMesh === true) as THREE.Mesh
    const faceMat = (Array.isArray(faceMesh.material) ? faceMesh.material[0] : faceMesh.material) as THREE.MeshPhongMaterial
    expect(faceMat.opacity).toBe(1)
    // ...its batch slot is suppressed so it doesn't double-draw...
    expect(slotMatrices(batch).filter(isDegenerate)).toHaveLength(1)
    // ...and the remaining batched placements dim.
    expect(batchMaterial(batch).opacity).toBeCloseTo(0.15)
    const batchEdges = instancedEdges(renderer.instancesGroup)[0]
    expect((batchEdges.material as THREE.LineBasicMaterial).opacity).toBeCloseTo(0.15)

    renderer.setActiveContext(null, null)

    // Exit: the placement returns to its batch, and the batch un-dims.
    expect(renderer.instancesGroup.getObjectByName('Instance_11')).toBeUndefined()
    expect(slotMatrices(batch).filter(isDegenerate)).toHaveLength(0)
    expect(batchMaterial(batch).opacity).toBe(1)
    expect((batchEdges.material as THREE.LineBasicMaterial).opacity).toBe(1)
  })
})

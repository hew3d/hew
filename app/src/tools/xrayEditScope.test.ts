/**
 * Editing scope (Shaded and X-ray), app side (docs/design/xray-edit-scope-pick.md):
 *  - a scope push invalidates the per-event pick memos (FacePickCache);
 *  - `resolveSelectableRef` falls through to the (scoped) face pick when the
 *    snap's object is out of scope, in Shaded or X-ray with a scope open (not Wireframe);
 *  - Push/Pull's region-vs-face comparison prefers the in-scope candidate
 *    whatever the depth, and hover, cue and click agree on it.
 */
import { describe, it, expect, vi, afterEach } from 'vitest'
import * as THREE from 'three'
import { FacePickCache } from './faceDraw'
import { bumpPickScopeGeneration, setPickRenderMode } from './pickPolicy'
import { resolveSelectableRef, selectionScopeActive, type ResolveDeps, type SelectScene } from './snapSelection'
import { PushPullTool, regionInFrontOfFace } from './PushPullTool'
import { ctxOfPlane } from './planeUnits'
import { planeUnitsMock } from './planeUnitsMock'
import type { Scene as WasmScene } from '../wasm/loader'
import type { Snap } from './types'
import type { Ray } from '../viewport/math'
import type { NodeRef } from '../panels/treeModel'

afterEach(() => setPickRenderMode('shaded'))

const RAY: Ray = { origin: [0.5, 0.5, 5], direction: [0, 0, -1] }

describe('FacePickCache and the scope generation', () => {
  it('re-picks for the same Ray object after a scope push', () => {
    const pick = vi.fn(() => ({ object: () => 1n, face: () => 2n, instance: () => undefined, depth: () => 1, free: vi.fn() }))
    const scene = { pick_face: pick } as unknown as WasmScene
    const cache = new FacePickCache()
    cache.pickFor(scene, RAY, () => true)
    cache.pickFor(scene, RAY, () => true)
    expect(pick).toHaveBeenCalledTimes(1)
    bumpPickScopeGeneration()
    cache.pickFor(scene, RAY, () => true)
    expect(pick).toHaveBeenCalledTimes(2)
  })
})

// ── resolveSelectableRef fall-through ───────────────────────────────────────

const OUTER = 1n // the enclosing box: out of scope, the snap lands on it
const MEMBER = 2n // the component's member: in scope, the scoped pick returns it

function selectScene(): SelectScene {
  return {
    pick_face: () => ({ object: () => MEMBER, face: () => 4n, instance: () => undefined, depth: () => 3, free: vi.fn() }),
    pick_sketch_region: () => undefined,
    pick_sketch_edge: () => undefined,
  } as unknown as SelectScene
}

function selectDeps(over: Partial<ResolveDeps>): ResolveDeps {
  return {
    scene: selectScene(),
    context: [],
    resolveObject: (object) => (object === MEMBER ? ({ kind: 'object', id: object } as unknown as NodeRef) : null),
    cameraForward: [0, 0, -1],
    cameraFar: 100,
    worldPerPixelAt: () => 1e-3,
    ...over,
  }
}
const snapOnOuter: Snap = { x: 0, y: 0, z: 0, kind: 'endpoint', object: OUTER }

describe('resolveSelectableRef — out-of-scope snap in Shaded and X-ray', () => {
  it('falls through to the scoped face pick when a scope is open (X-ray and Shaded)', () => {
    for (const facesOcclude of [false, true]) {
      const ref = resolveSelectableRef(snapOnOuter, RAY, selectDeps({ facesOcclude, scopeActive: true }))
      expect(ref).toMatchObject({ kind: 'object', id: MEMBER })
    }
  })

  it('selects nothing when the scope is not active (top level, or Wireframe), as before', () => {
    expect(resolveSelectableRef(snapOnOuter, RAY, selectDeps({ facesOcclude: true, scopeActive: false }))).toBeNull()
    expect(resolveSelectableRef(snapOnOuter, RAY, selectDeps({}))).toBeNull()
  })

  it('selects nothing when no scope is open (top level), as before', () => {
    expect(resolveSelectableRef(snapOnOuter, RAY, selectDeps({ facesOcclude: false }))).toBeNull()
    expect(resolveSelectableRef(snapOnOuter, RAY, selectDeps({ facesOcclude: false, scopeActive: false }))).toBeNull()
  })

  it('still selects nothing when the scoped pick itself lands out of scope', () => {
    const scene = {
      ...selectScene(),
      pick_face: () => ({ object: () => OUTER, face: () => 4n, instance: () => undefined, depth: () => 3, free: vi.fn() }),
    } as unknown as SelectScene
    expect(resolveSelectableRef(snapOnOuter, RAY, selectDeps({ scene, facesOcclude: false, scopeActive: true }))).toBeNull()
  })

  it('is deterministic: the same snap, ray and deps resolve the same twice', () => {
    const d = selectDeps({ facesOcclude: false, scopeActive: true })
    const first = resolveSelectableRef(snapOnOuter, RAY, d)
    expect(resolveSelectableRef(snapOnOuter, RAY, d)).toEqual(first)
    expect(first).not.toBeNull()
  })
})

describe('selectionScopeActive — the scope counts in Shaded and X-ray, not Wireframe', () => {
  const resolve = (mode: string, depth: number, session: boolean) =>
    resolveSelectableRef(
      snapOnOuter, RAY,
      selectDeps({ facesOcclude: mode === 'shaded', scopeActive: selectionScopeActive(mode, depth, session) }),
    )
  it('Wireframe with a scope selects nothing, as on main', () => {
    expect(resolve('wireframe', 1, false)).toBeNull()
    expect(resolve('wireframe', 0, true)).toBeNull()
  })
  it('X-ray and Shaded with a context or a session scope fall through', () => {
    for (const mode of ['xray', 'shaded']) {
      expect(resolve(mode, 1, false)).toMatchObject({ kind: 'object', id: MEMBER })
      expect(resolve(mode, 0, true)).toMatchObject({ kind: 'object', id: MEMBER })
    }
  })
  it('X-ray or Shaded with no scope selects nothing', () => {
    expect(resolve('xray', 0, false)).toBeNull()
    expect(resolve('shaded', 0, false)).toBeNull()
  })
})

// ── regionInFrontOfFace ─────────────────────────────────────────────────────

describe('regionInFrontOfFace — scope standing', () => {
  const face = { object: 3n, face: 4n, instance: undefined, depth: 5 }
  const front = { sketch: 8n, region: 1n, depth: 1, detached: false }
  const behind = { sketch: 8n, region: 1n, depth: 9, detached: false }

  it('a foreign region in front of an in-scope face loses', () => {
    expect(regionInFrontOfFace(front, face, true, false)).toBe(false)
    // Unscoped, depth alone decides as it always did.
    expect(regionInFrontOfFace(front, face, true)).toBe(true)
    expect(regionInFrontOfFace(front, face, true, null)).toBe(true)
  })

  it('an in-scope region behind a foreign face wins', () => {
    expect(regionInFrontOfFace(behind, face, false, true)).toBe(true)
    expect(regionInFrontOfFace(behind, face, false)).toBe(false)
  })

  it('two candidates of the same standing compare by depth, as before', () => {
    expect(regionInFrontOfFace(front, face, true, true)).toBe(true)
    expect(regionInFrontOfFace(behind, face, true, true)).toBe(false)
    expect(regionInFrontOfFace(behind, face, false, false)).toBe(false)
  })
})

// ── Push/Pull: hint, cue and commit agree on the buried face ────────────────

const GROUND = ctxOfPlane([0, 0, 0], [0, 0, 1]) as string
const sq = (x0: number, y0: number, x1: number, y1: number): Array<[number, number, number]> =>
  [[x0, y0, 0], [x1, y0, 0], [x1, y1, 0], [x0, y1, 0]]

function pushPullScene() {
  const facePick = { object: () => 3n, face: () => 4n, instance: () => undefined, depth: () => 5, free: vi.fn() }
  const regionPick = { sketch: () => 8n, region: () => 1n, depth: () => 1, detached: () => false, free: vi.fn() }
  return {
    ...planeUnitsMock({
      [GROUND]: {
        regions: [{ key: 1, outer: sq(0, 0, 2, 2), over: { object: 9, face: 6 }, extrude: { sketch: 8, region: 1 } }],
      },
    }),
    history_generation: vi.fn(() => 1n),
    pick_face: vi.fn(() => facePick),
    session_stack: vi.fn(() => [{ kind: 'instance', id: 77n }]),
    explode_session_sketches: vi.fn(() => new BigUint64Array([])),
    pick_sketch_region: vi.fn(() => regionPick),
    pick_sketch_region_in_instance: vi.fn(() => regionPick),
    instance_pose: vi.fn(() => new Float64Array([1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0])),
    node_parent: vi.fn(() => undefined),
    face_normal: vi.fn(() => new Float64Array([0, 0, 1])),
    face_plane: vi.fn(() => new Float64Array([0, 0, 0, 0, 0, 1])),
    sketch_plane: vi.fn(() => new Float64Array([0, 0, 0, 0, 0, 1])),
    region_boundary: vi.fn(() => new Float32Array([])),
    face_boundary: vi.fn(() => new Float32Array([])),
    extrude_region: vi.fn(() => 55n),
    extrude_region_in_instance: vi.fn(() => 55n),
    push_pull: vi.fn(() => ({ is_through: () => false, result_objects: () => new BigUint64Array([]), free: vi.fn() })),
    extrude_face_as_new_object: vi.fn(() => 42n),
  } as unknown as WasmScene
}

function pushPullTool(scene: WasmScene) {
  const onToast = vi.fn()
  const tool = new PushPullTool(scene, new THREE.Group(), vi.fn(), onToast)
  // Object 3 (a member of the open component) is editable; object 9 is not.
  tool.setFaceEligibility((object) => object === 3n)
  return { tool, onToast }
}
const snapAt = (z: number): Snap => ({ x: 0.5, y: 0.5, z, kind: 'endpoint' })

describe('Push/Pull — a foreign region in front of the in-scope buried face', () => {
  it('X-ray with a session open: hint, cue and commit all land on the in-scope face', () => {
    setPickRenderMode('xray')
    const scene = pushPullScene()
    const { tool, onToast } = pushPullTool(scene)

    tool.onPointerMove(snapAt(0), RAY)
    expect(tool.hoverHint()).toBeNull()
    expect(tool.snapConstraint(RAY)).toEqual({
      constraintPlane: { point: [0, 0, 0], normal: [0, 0, 1] },
      facesOnly: true,
    })

    tool.onPointerDown(snapAt(0), RAY)
    tool.onPointerDown(snapAt(2), RAY)
    expect(scene.extrude_region).not.toHaveBeenCalled()
    const call = (scene.push_pull as ReturnType<typeof vi.fn>).mock.calls[0]
    expect(call.slice(0, 2)).toEqual([3n, 4n])
    expect(onToast).not.toHaveBeenCalled()
  })

  it('Shaded with a session open: the in-scope face wins too, whatever the depth', () => {
    setPickRenderMode('shaded')
    const scene = pushPullScene()
    const { tool } = pushPullTool(scene)

    tool.onPointerDown(snapAt(0), RAY)
    tool.onPointerDown(snapAt(2), RAY)
    expect(scene.extrude_region).not.toHaveBeenCalled()
    const call = (scene.push_pull as ReturnType<typeof vi.fn>).mock.calls[0]
    expect(call.slice(0, 2)).toEqual([3n, 4n])
  })

  it('Wireframe keeps the depth rule: the nearer region takes the click as today', () => {
    setPickRenderMode('wireframe')
    const scene = pushPullScene()
    const { tool } = pushPullTool(scene)

    tool.onPointerDown(snapAt(0), RAY)
    tool.onPointerDown(snapAt(2), RAY)
    expect(scene.push_pull).not.toHaveBeenCalled()
    expect(scene.extrude_region).toHaveBeenCalled()
  })
})

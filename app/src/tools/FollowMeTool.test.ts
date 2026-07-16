/**
 * FollowMeTool unit tests — path preselection, in-tool path picking (edge
 * island / face loop), the profile-click commit, and typed-refusal handling.
 * Mirrors the fake-WasmScene pattern used by PushPullTool.test.ts.
 */
import { describe, it, expect, vi } from 'vitest'
import * as THREE from 'three'
import { FollowMeTool } from './FollowMeTool'
import type { NodeRef } from '../panels/treeModel'
import type { Scene as WasmScene } from '../wasm/loader'
import type { Ray } from '../viewport/math'

const RAY: Ray = { origin: [0, 0, 5], direction: [0, 0, -1] }

function makeEdgePick(sketch: bigint, edge: bigint) {
  return { sketch: () => sketch, edge: () => edge, free: vi.fn() }
}

function makeFacePick(object: bigint, face: bigint) {
  return {
    object: () => object,
    face: () => face,
    instance: () => undefined,
    free: vi.fn(),
  }
}

function makeRegionPick(sketch: bigint, region: bigint) {
  return { sketch: () => sketch, region: () => region, free: vi.fn() }
}

function makeWasmScene(opts: {
  edgePick?: ReturnType<typeof makeEdgePick>
  facePick?: ReturnType<typeof makeFacePick>
  regionPick?: ReturnType<typeof makeRegionPick>
  islandEdges?: bigint[]
  commitError?: string
} = {}): WasmScene {
  const follow = () => {
    if (opts.commitError !== undefined) throw new Error(opts.commitError)
    return 77n
  }
  return {
    pick_sketch_edge: vi.fn(() => opts.edgePick),
    pick_face: vi.fn(() => opts.facePick),
    pick_sketch_region: vi.fn(() => opts.regionPick),
    sketch_edge_island: vi.fn(() => 5n),
    sketch_island_edges: vi.fn(() => new BigUint64Array(opts.islandEdges ?? [])),
    sketch_curve_edges: vi.fn(() => new BigUint64Array([])),
    sketch_edge_endpoints: vi.fn(() => new Float64Array([0, 0, 0, 1, 0, 0])),
    face_boundary: vi.fn(() => new Float32Array([0, 0, 1, 1, 0, 1, 1, 1, 1])),
    follow_me_along_edges: vi.fn(follow),
    follow_me_around_face: vi.fn(follow),
  } as unknown as WasmScene
}

function makeTool(scene: WasmScene, selection: NodeRef[] = []) {
  const preview = new THREE.Group()
  const onCommit = vi.fn()
  const onToast = vi.fn()
  const tool = new FollowMeTool(scene, preview, onCommit, onToast, selection)
  return { tool, preview, onCommit, onToast }
}

describe('FollowMeTool — path from preselection', () => {
  it('starts at pick-profile when sketch edges are preselected, and commits on a region click', () => {
    const regionPick = makeRegionPick(20n, 21n)
    const scene = makeWasmScene({ regionPick })
    const selection: NodeRef[] = [
      { kind: 'sketch-edge', id: 1n, sketch: 9n },
      { kind: 'sketch-edge', id: 2n, sketch: 9n },
    ]
    const { tool, onCommit, onToast } = makeTool(scene, selection)

    expect(tool.statusHint()).toContain('profile')

    tool.onPointerDown(null, RAY)

    expect(scene.follow_me_along_edges).toHaveBeenCalledTimes(1)
    const call = (scene.follow_me_along_edges as ReturnType<typeof vi.fn>).mock.calls[0]
    expect(call[0]).toBe(20n) // profile sketch
    expect(call[1]).toBe(21n) // profile region
    expect(call[2]).toBe(9n) // path sketch
    expect(Array.from(call[3] as BigUint64Array)).toEqual([1n, 2n])
    expect(onCommit).toHaveBeenCalledWith(77n)
    expect(onToast).not.toHaveBeenCalled()
  })

  it('expands a preselected curve to its facet edges', () => {
    const regionPick = makeRegionPick(20n, 21n)
    const scene = makeWasmScene({ regionPick })
    ;(scene.sketch_curve_edges as ReturnType<typeof vi.fn>).mockReturnValue(
      new BigUint64Array([4n, 5n, 6n]),
    )
    const selection: NodeRef[] = [{ kind: 'sketch-curve', id: 3n, sketch: 9n }]
    const { tool } = makeTool(scene, selection)

    tool.onPointerDown(null, RAY)

    const call = (scene.follow_me_along_edges as ReturnType<typeof vi.fn>).mock.calls[0]
    expect(Array.from(call[3] as BigUint64Array)).toEqual([4n, 5n, 6n])
    expect(tool.statusHint()).toContain('path') // reset after commit
  })

  it('ignores a preselection spanning two sketches (starts at pick-path)', () => {
    const scene = makeWasmScene()
    const selection: NodeRef[] = [
      { kind: 'sketch-edge', id: 1n, sketch: 9n },
      { kind: 'sketch-edge', id: 2n, sketch: 10n },
    ]
    const { tool } = makeTool(scene, selection)
    expect(tool.statusHint()).toContain('path')
  })
})

describe('FollowMeTool — in-tool path picking', () => {
  it('clicking a sketch edge takes its whole island as the path', () => {
    const edgePick = makeEdgePick(9n, 1n)
    const regionPick = makeRegionPick(20n, 21n)
    const scene = makeWasmScene({ edgePick, regionPick, islandEdges: [1n, 2n, 3n] })
    const { tool, onCommit } = makeTool(scene)

    tool.onPointerDown(null, RAY) // pick path
    expect(tool.statusHint()).toContain('profile')

    tool.onPointerDown(null, RAY) // pick profile → commit
    const call = (scene.follow_me_along_edges as ReturnType<typeof vi.fn>).mock.calls[0]
    expect(call[2]).toBe(9n)
    expect(Array.from(call[3] as BigUint64Array)).toEqual([1n, 2n, 3n])
    expect(onCommit).toHaveBeenCalledWith(77n)
  })

  it('clicking a solid face runs the sweep around the face boundary', () => {
    const facePick = makeFacePick(30n, 31n)
    const regionPick = makeRegionPick(20n, 21n)
    const scene = makeWasmScene({ facePick, regionPick })
    const { tool, onCommit } = makeTool(scene)

    tool.onPointerDown(null, RAY) // no edge under cursor → face path
    tool.onPointerDown(null, RAY) // profile → commit

    expect(scene.follow_me_around_face).toHaveBeenCalledWith(20n, 21n, 30n, 31n)
    expect(onCommit).toHaveBeenCalledWith(77n)
  })

  it('a click over nothing stays in pick-path', () => {
    const scene = makeWasmScene()
    const { tool } = makeTool(scene)
    tool.onPointerDown(null, RAY)
    expect(tool.statusHint()).toContain('path')
  })
})

describe('FollowMeTool — refusals and cancel', () => {
  it('surfaces a typed kernel refusal as a toast and keeps the picked path', () => {
    const edgePick = makeEdgePick(9n, 1n)
    const regionPick = makeRegionPick(20n, 21n)
    const scene = makeWasmScene({
      edgePick,
      regionPick,
      islandEdges: [1n],
      commitError: 'PathTooTight: path bends tighter than the profile is wide',
    })
    const { tool, onCommit, onToast } = makeTool(scene)

    tool.onPointerDown(null, RAY)
    tool.onPointerDown(null, RAY)

    expect(onCommit).not.toHaveBeenCalled()
    expect(onToast).toHaveBeenCalledTimes(1)
    expect(onToast.mock.calls[0][1]).toBe('PathTooTight')
    // The path survives the refusal so the user can adjust and re-click.
    expect(tool.statusHint()).toContain('profile')
  })

  it('Escape steps back one stage; cancel clears the preview', () => {
    const edgePick = makeEdgePick(9n, 1n)
    const scene = makeWasmScene({ edgePick, islandEdges: [1n] })
    const { tool, preview } = makeTool(scene)

    tool.onPointerDown(null, RAY)
    expect(preview.children.length).toBeGreaterThan(0) // path highlight drawn
    tool.onKey({ key: 'Escape' } as KeyboardEvent)
    expect(tool.statusHint()).toContain('path')
    expect(preview.children.length).toBe(0)
  })
})

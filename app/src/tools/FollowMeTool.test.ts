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

function makeFacePick(object: bigint, face: bigint, instance: bigint | undefined = undefined) {
  return {
    object: () => object,
    face: () => face,
    instance: () => instance,
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
  /** Parent node of the picked face's object; `undefined` = plain, top-level
   *  (the followable default). A defined parent = a grouped, ineligible face. */
  faceParent?: bigint
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
    // The shared `defaultFaceEligible` fallback (no injected predicate in these
    // unit tests) reads node_parent to reject grouped faces.
    node_parent: vi.fn(() => opts.faceParent),
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

  it('expands a single preselected edge to its whole connected island (the one-click promise)', () => {
    const regionPick = makeRegionPick(20n, 21n)
    const scene = makeWasmScene({ regionPick, islandEdges: [1n, 2n] })
    // A Select click on one line of an L yields exactly this selection.
    const selection: NodeRef[] = [{ kind: 'sketch-edge', id: 1n, sketch: 9n }]
    const { tool, onCommit } = makeTool(scene, selection)

    expect(tool.statusHint()).toContain('Click the profile')
    tool.onPointerDown(null, RAY)

    const call = (scene.follow_me_along_edges as ReturnType<typeof vi.fn>).mock.calls[0]
    expect(call[2]).toBe(9n)
    // BOTH island edges swept, not just the selected one.
    expect(Array.from(call[3] as BigUint64Array)).toEqual([1n, 2n])
    expect(onCommit).toHaveBeenCalledWith(77n)
  })

  it('honors an explicit multi-edge preselection as picked (deliberate partial path)', () => {
    const regionPick = makeRegionPick(20n, 21n)
    const scene = makeWasmScene({ regionPick, islandEdges: [1n, 2n, 3n, 4n] })
    const selection: NodeRef[] = [
      { kind: 'sketch-edge', id: 1n, sketch: 9n },
      { kind: 'sketch-edge', id: 2n, sketch: 9n },
    ]
    const { tool } = makeTool(scene, selection)

    tool.onPointerDown(null, RAY)

    const call = (scene.follow_me_along_edges as ReturnType<typeof vi.fn>).mock.calls[0]
    expect(Array.from(call[3] as BigUint64Array)).toEqual([1n, 2n])
    expect(scene.sketch_edge_island).not.toHaveBeenCalled()
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

  it('a click over nothing stays in pick-path AND says what to aim at (no silent no-op)', () => {
    const scene = makeWasmScene()
    const { tool, onToast } = makeTool(scene)
    tool.onPointerDown(null, RAY)
    expect(tool.statusHint()).toContain('Click the path to follow')
    // A path-stage miss is never silent — a solid face can't be preselected,
    // so the tool must say to click the flat face directly.
    expect(onToast).toHaveBeenCalledTimes(1)
    expect(onToast.mock.calls[0][0]).toMatch(/flat face/i)
    // The code slot is empty — this is guidance, not a typed kernel refusal.
    expect(onToast.mock.calls[0][1]).toBeUndefined()
  })

  it('repeated empty clicks do not stack identical miss toasts, but a fresh target re-arms', () => {
    const scene = makeWasmScene()
    const { tool, onToast } = makeTool(scene)
    tool.onPointerDown(null, RAY) // miss → 1 toast
    tool.onPointerDown(null, RAY) // still nothing → suppressed
    expect(onToast).toHaveBeenCalledTimes(1)

    // A real face appears under the cursor (hover) — the miss is stale, so the
    // NEXT genuine miss speaks up again.
    ;(scene.pick_face as ReturnType<typeof vi.fn>).mockReturnValueOnce(makeFacePick(30n, 31n))
    tool.onPointerMove(null, RAY)
    tool.onPointerDown(null, RAY) // nothing again → toast fires a second time
    expect(onToast).toHaveBeenCalledTimes(2)
  })
})

describe('FollowMeTool — hover preview of the path target', () => {
  it('hovering a solid face at pick-path previews the face loop that will be swept', () => {
    const facePick = makeFacePick(30n, 31n)
    const scene = makeWasmScene({ facePick })
    const { tool, preview } = makeTool(scene)

    expect(preview.children.length).toBe(0)
    tool.onPointerMove(null, RAY)
    // The face-boundary loop is drawn as a hover preview before any click.
    expect(preview.children.length).toBe(1)
    expect(scene.face_boundary).toHaveBeenCalledWith(30n, 31n)
  })

  it('the hover preview clears when the cursor moves off all geometry', () => {
    const scene = makeWasmScene({ facePick: makeFacePick(30n, 31n) })
    const { tool, preview } = makeTool(scene)
    tool.onPointerMove(null, RAY)
    expect(preview.children.length).toBe(1)
    // Nothing under the cursor now.
    ;(scene.pick_face as ReturnType<typeof vi.fn>).mockReturnValueOnce(undefined)
    tool.onPointerMove(null, RAY)
    expect(preview.children.length).toBe(0)
  })

  it('picking the path replaces the hover preview with the persistent path highlight', () => {
    const facePick = makeFacePick(30n, 31n)
    const scene = makeWasmScene({ facePick })
    const { tool, preview } = makeTool(scene)
    tool.onPointerMove(null, RAY) // hover preview
    tool.onPointerDown(null, RAY) // commit to that face as the path
    // Exactly one highlight remains — the picked path, not a leftover hover.
    expect(preview.children.length).toBe(1)
    expect(tool.statusHint()).toContain('profile')
  })

  it('an INSTANCED face is never previewed as a target (frame guard)', () => {
    // face_boundary/follow_me_around_face take only (object, face), so an
    // instanced (definition-local) face would draw the loop in the wrong place.
    const scene = makeWasmScene({ facePick: makeFacePick(30n, 31n, 99n) })
    const { tool, preview } = makeTool(scene)
    tool.onPointerMove(null, RAY)
    expect(preview.children.length).toBe(0)
    expect(scene.face_boundary).not.toHaveBeenCalled()
  })
})

describe('FollowMeTool — face frame guard (only plain top-level faces sweep)', () => {
  it('an INSTANCED face refuses with component guidance and locks no path', () => {
    const scene = makeWasmScene({ facePick: makeFacePick(30n, 31n, 99n) })
    const { tool, onCommit, onToast } = makeTool(scene)

    tool.onPointerDown(null, RAY) // click the instanced face at the path stage
    expect(onToast).toHaveBeenCalledTimes(1)
    expect(onToast.mock.calls[0][0]).toMatch(/component/i)
    expect(onToast.mock.calls[0][1]).toBeUndefined() // guidance, not a kernel code
    // No path was locked — still choosing the path, nothing swept.
    expect(tool.statusHint()).toContain('Click the path to follow')
    expect(onCommit).not.toHaveBeenCalled()
    expect(scene.follow_me_around_face).not.toHaveBeenCalled()
  })

  it('a GROUPED face refuses with group guidance and locks no path', () => {
    // Plain instance, but the object hangs under a group parent.
    const scene = makeWasmScene({ facePick: makeFacePick(30n, 31n), faceParent: 88n })
    const { tool, onCommit, onToast } = makeTool(scene)

    tool.onPointerDown(null, RAY)
    expect(onToast).toHaveBeenCalledTimes(1)
    expect(onToast.mock.calls[0][0]).toMatch(/inside a group/i)
    expect(tool.statusHint()).toContain('Click the path to follow')
    expect(onCommit).not.toHaveBeenCalled()
  })

  it('an in-context face refuses (Follow Me runs at the top level)', () => {
    const scene = makeWasmScene({ facePick: makeFacePick(30n, 31n) })
    const { tool, onToast } = makeTool(scene)
    tool.setContextScoped(true) // e.g. a group/instance/object is being edited

    tool.onPointerDown(null, RAY)
    expect(onToast).toHaveBeenCalledTimes(1)
    expect(onToast.mock.calls[0][0]).toMatch(/top level|step out/i)
    expect(tool.statusHint()).toContain('Click the path to follow')
  })

  it('the stale-preselection face recovery is frame-gated too (an instanced face is refused)', () => {
    const facePick = makeFacePick(30n, 31n, 99n)
    const regionPick = makeRegionPick(20n, 21n)
    const scene = makeWasmScene({ facePick, regionPick })
    // A leftover single-edge preselection → the tool starts at pick-profile.
    const selection: NodeRef[] = [{ kind: 'sketch-edge', id: 1n, sketch: 9n }]
    const { tool, onCommit, onToast } = makeTool(scene, selection)

    // "Click the box's top face", but it's a component instance → refused, and
    // the stale preselection is NOT swapped for a wrong-frame face.
    ;(scene.pick_sketch_region as ReturnType<typeof vi.fn>).mockReturnValueOnce(undefined)
    tool.onPointerDown(null, RAY)
    expect(onToast).toHaveBeenCalledTimes(1)
    expect(onToast.mock.calls[0][0]).toMatch(/component/i)
    expect(onCommit).not.toHaveBeenCalled()
    expect(scene.follow_me_around_face).not.toHaveBeenCalled()
  })

  it('repeated ineligible-face clicks during stale-preselection recovery do not stack toasts', () => {
    const facePick = makeFacePick(30n, 31n, 99n) // instanced → ineligible
    const scene = makeWasmScene({ facePick }) // no regionPick → every region pick misses
    const selection: NodeRef[] = [{ kind: 'sketch-edge', id: 1n, sketch: 9n }]
    const { tool, onCommit, onToast } = makeTool(scene, selection)

    // Three clicks that each miss the region and land on the same instanced
    // face: the refusal speaks once, then stays quiet — the same anti-spam
    // dedup the path stage uses, not a fresh toast per click.
    tool.onPointerDown(null, RAY)
    tool.onPointerDown(null, RAY)
    tool.onPointerDown(null, RAY)
    expect(onToast).toHaveBeenCalledTimes(1)
    expect(onToast.mock.calls[0][0]).toMatch(/component/i)
    expect(onCommit).not.toHaveBeenCalled()
    expect(scene.follow_me_around_face).not.toHaveBeenCalled()
  })

  it('a plain, top-level face still sweeps (the maintainer tabletop path)', () => {
    const facePick = makeFacePick(30n, 31n) // instance undefined, no group parent
    const regionPick = makeRegionPick(20n, 21n)
    const scene = makeWasmScene({ facePick, regionPick })
    const { tool, onCommit } = makeTool(scene)

    tool.onPointerDown(null, RAY) // pick the plain face as the path
    expect(tool.statusHint()).toContain('profile')
    tool.onPointerDown(null, RAY) // profile → commit
    expect(scene.follow_me_around_face).toHaveBeenCalledWith(20n, 21n, 30n, 31n)
    expect(onCommit).toHaveBeenCalledWith(77n)
  })
})

describe('FollowMeTool — face-path refusals get face-specific copy', () => {
  it('a parallel face refusal names the FACE, not the profile placement', () => {
    const facePick = makeFacePick(30n, 31n)
    const regionPick = makeRegionPick(20n, 21n)
    const scene = makeWasmScene({
      facePick,
      regionPick,
      commitError: 'ProfileNotPerpendicular: no perpendicular segment',
    })
    const { tool, onToast } = makeTool(scene)
    tool.onPointerDown(null, RAY) // pick the face path
    tool.onPointerDown(null, RAY) // profile → refused

    expect(onToast).toHaveBeenCalledTimes(1)
    expect(onToast.mock.calls[0][0]).toMatch(/face is parallel to the profile/i)
    expect(onToast.mock.calls[0][1]).toBe('ProfileNotPerpendicular')
  })

  it('a too-thin face refusal names the FACE thickness', () => {
    const facePick = makeFacePick(30n, 31n)
    const regionPick = makeRegionPick(20n, 21n)
    const scene = makeWasmScene({
      facePick,
      regionPick,
      commitError: 'PathTooTight: advance check failed',
    })
    const { tool, onToast } = makeTool(scene)
    tool.onPointerDown(null, RAY)
    tool.onPointerDown(null, RAY)

    expect(onToast.mock.calls[0][0]).toMatch(/thinner than the profile is deep/i)
    expect(onToast.mock.calls[0][1]).toBe('PathTooTight')
  })

  it('an EDGE-path refusal keeps the generic drawn-path copy (face wording is face-only)', () => {
    const edgePick = makeEdgePick(9n, 1n)
    const regionPick = makeRegionPick(20n, 21n)
    const scene = makeWasmScene({
      edgePick,
      regionPick,
      islandEdges: [1n],
      commitError: 'ProfileNotPerpendicular: no perpendicular segment',
    })
    const { tool, onToast } = makeTool(scene)
    tool.onPointerDown(null, RAY) // pick edge island path
    tool.onPointerDown(null, RAY) // profile → refused

    // Not the face wording — the shared kernelErrors copy for a drawn path.
    expect(onToast.mock.calls[0][0]).not.toMatch(/face is parallel/i)
    expect(onToast.mock.calls[0][1]).toBe('ProfileNotPerpendicular')
  })

  it('a solid-face click at the profile stage RE-PICKS the path (stale-preselection recovery)', () => {
    const facePick = makeFacePick(30n, 31n)
    const regionPick = makeRegionPick(20n, 21n)
    const scene = makeWasmScene({ facePick, regionPick })
    // The maintainer trap: an edge of the profile's own outline was left
    // selected from placing it, so the tool silently starts at
    // pick-profile with that edge as the path.
    const selection: NodeRef[] = [{ kind: 'sketch-edge', id: 1n, sketch: 9n }]
    const { tool, onCommit, onToast } = makeTool(scene, selection)
    expect(tool.statusHint()).toContain('Click the profile')

    // "Click the box's top face": no region there — before the fix a dead
    // no-op; now the face becomes the path.
    ;(scene.pick_sketch_region as ReturnType<typeof vi.fn>).mockReturnValueOnce(undefined)
    tool.onPointerDown(null, RAY)
    expect(tool.statusHint()).toContain('Click the profile')
    expect(onToast).not.toHaveBeenCalled()

    // The profile click now sweeps around the FACE, not the stale edges.
    tool.onPointerDown(null, RAY)
    expect(scene.follow_me_around_face).toHaveBeenCalledWith(20n, 21n, 30n, 31n)
    expect(scene.follow_me_along_edges).not.toHaveBeenCalled()
    expect(onCommit).toHaveBeenCalledWith(77n)
  })

  it('a stray face click does NOT replace a path picked in-tool (no silent face substitution)', () => {
    const edgePick = makeEdgePick(9n, 1n)
    const facePick = makeFacePick(30n, 31n)
    const regionPick = makeRegionPick(20n, 21n)
    const scene = makeWasmScene({ edgePick, facePick, regionPick, islandEdges: [1n, 2n] })
    // No preselection: the user picks the path deliberately in the tool.
    const { tool, onCommit, onToast } = makeTool(scene)
    tool.onPointerDown(null, RAY) // step 2: pick the edge island as the path
    expect(tool.statusHint()).toContain('Click the profile')
    // The in-tool hint drops the "solid-face click follows that face" promise.
    expect(tool.statusHint()).not.toContain('follows that face')

    // A profile click that grazes an unrelated solid face: no region there,
    // a face IS under the cursor — but the path was chosen deliberately, so
    // the face must not hijack it. The face is never even consulted.
    ;(scene.pick_sketch_region as ReturnType<typeof vi.fn>).mockReturnValueOnce(undefined)
    tool.onPointerDown(null, RAY)
    expect(scene.pick_face).not.toHaveBeenCalled()
    expect(onCommit).not.toHaveBeenCalled()
    expect(onToast).not.toHaveBeenCalled()
    expect(tool.statusHint()).toContain('Click the profile')

    // The next, better-aimed profile click commits against the ORIGINAL
    // edge path — never the grazed face.
    tool.onPointerDown(null, RAY)
    expect(scene.follow_me_around_face).not.toHaveBeenCalled()
    const call = (scene.follow_me_along_edges as ReturnType<typeof vi.fn>).mock.calls[0]
    expect(Array.from(call[3] as BigUint64Array)).toEqual([1n, 2n])
    expect(onCommit).toHaveBeenCalledWith(77n)
  })

  it('a sketch-edge near-miss at the profile stage stays a no-op (an edge must not steal the path)', () => {
    const edgePick = makeEdgePick(9n, 1n)
    const scene = makeWasmScene({ edgePick, islandEdges: [1n, 2n] })
    const { tool, onCommit } = makeTool(scene, [{ kind: 'sketch-edge', id: 7n, sketch: 9n }])
    expect(tool.statusHint()).toContain('Click the profile')

    // Region miss, no face, but an edge within the pick cone (a near-miss
    // of a small profile's interior lands here constantly): nothing may
    // happen — edges are never consulted at this stage, so the picked
    // path survives for the next, better-aimed click.
    tool.onPointerDown(null, RAY)
    expect(scene.pick_sketch_edge).not.toHaveBeenCalled()
    expect(onCommit).not.toHaveBeenCalled()
    expect(tool.statusHint()).toContain('Click the profile')
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

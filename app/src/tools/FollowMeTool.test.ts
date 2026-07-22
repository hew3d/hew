/**
 * FollowMeTool unit tests — path preselection, in-tool path picking (edge
 * island / face loop), the profile-click commit, and typed-refusal handling.
 * Mirrors the fake-WasmScene pattern used by PushPullTool.test.ts.
 */
import { describe, it, expect, vi } from 'vitest'
import * as THREE from 'three'
import { LineSegments2 } from 'three/examples/jsm/lines/LineSegments2.js'
import { LineMaterial } from 'three/examples/jsm/lines/LineMaterial.js'
import { FollowMeTool, cueColors, pathHoverColors } from './FollowMeTool'
import type { NodeRef } from '../panels/treeModel'
import type { Scene as WasmScene } from '../wasm/loader'
import type { Ray } from '../viewport/math'
import { clearPreview } from './transformPreview'

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
  const onMeasurement = vi.fn()
  const tool = new FollowMeTool(scene, preview, onCommit, onToast, onMeasurement, selection)
  return { tool, preview, onCommit, onToast, onMeasurement }
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

    // Trailing arg is the new optional partial-sweep `stop_len` — undefined
    // for a plain click (full sweep), exactly as before.
    expect(scene.follow_me_around_face).toHaveBeenCalledWith(20n, 21n, 30n, 31n, undefined)
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

  it('the pre-click hover preview renders as a fat line (LineSegments2), in the hover tint', () => {
    // Replaces the old 1px native THREE.LineSegments — nearly invisible —
    // with a fat line built by makeFatSegments, colored with the
    // theme-resolved (dark, in this Node test env) hover tint.
    const facePick = makeFacePick(30n, 31n)
    const scene = makeWasmScene({ facePick })
    const { tool, preview } = makeTool(scene)
    tool.onPointerMove(null, RAY)

    expect(preview.children).toHaveLength(1)
    const hover = preview.children[0]
    expect(hover).toBeInstanceOf(LineSegments2)
    expect(hover).not.toBeInstanceOf(THREE.LineSegments) // not the native line type
    const material = (hover as LineSegments2).material
    expect(material).toBeInstanceOf(LineMaterial)
    expect(material.color.getHex()).toBe(pathHoverColors().hover)
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
    // The hover preview is gone and the picked path is drawn in its place, as
    // a fat line (LineSegments2/LineMaterial) in the selection-highlight color
    // — not the old 1px native LineSegments/raw hex. The preview group also
    // carries the START AFFORDANCE now: a face loop's segments are never
    // curve-attributed, so under the new corner-seam rule EVERY one of its
    // vertices is a POTENTIAL legal start (green ring), not a blocked corner
    // — only a joint between two DIFFERENT drawn curves stays blocked, which
    // a face loop never has. Asserted by color rather than by a bare child
    // count, so the two concerns stay apart.
    const colors = preview.children.map((c) =>
      ((c as THREE.Mesh).material as THREE.LineBasicMaterial).color.getHex(),
    )
    expect(colors.filter((c) => c === pathHoverColors().path)).toHaveLength(1) // the picked path
    expect(colors).not.toContain(pathHoverColors().hover) // no leftover hover preview
    expect(colors.filter((c) => c === cueColors().ok)).toHaveLength(3) // 3 potential corner starts
    expect(colors).not.toContain(cueColors().blocked)
    const pathObj = preview.children.find(
      (c) => ((c as THREE.Mesh).material as THREE.LineBasicMaterial).color.getHex() === pathHoverColors().path,
    )
    expect(pathObj).toBeInstanceOf(LineSegments2)
    expect((pathObj as LineSegments2).material).toBeInstanceOf(LineMaterial)
    // The original assertion here was a bare `children.length === 1`, which
    // also pinned "nothing unaccounted-for is ever left in the shared preview
    // group". The count legitimately changed (1 highlight + 3 corner-start
    // markers), so the bound is restated rather than dropped.
    expect(preview.children).toHaveLength(1 + 3)
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
    expect(scene.follow_me_around_face).toHaveBeenCalledWith(20n, 21n, 30n, 31n, undefined)
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
    expect(scene.follow_me_around_face).toHaveBeenCalledWith(20n, 21n, 30n, 31n, undefined)
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

/**
 * The start affordance: the markers that say where on the picked path a
 * profile may begin, and the hover verdict that says — before the click —
 * whether the profile under the cursor would be refused.
 *
 * The geometry rule itself is specified in `followMeStart.test.ts`; these
 * cover the wiring: what gets drawn, in what color, and what the status bar
 * says. `follow-me-start-cue.spec.ts` cross-checks the prediction against the
 * real kernel.
 */
describe('FollowMeTool — start affordance', () => {
  const CIRCLE_EDGES = [1n, 2n, 3n, 4n, 5n, 6n, 7n, 8n]
  /** Radius-1 octagon on the ground, every edge one analytic curve chain —
   *  the shape a drawn circle path has. */
  const RIM = CIRCLE_EDGES.map((_, k) => {
    const a = (k * 2 * Math.PI) / CIRCLE_EDGES.length
    return [Math.cos(a), Math.sin(a), 0] as const
  })

  /** A scene whose path is that circle and whose profile region lives in a
   *  sketch on `profilePlane` (point + normal, as `sketch_plane` returns). */
  function circleScene(profilePlane: number[]): WasmScene {
    return {
      pick_sketch_edge: vi.fn(() => makeEdgePick(9n, 1n)),
      pick_face: vi.fn(() => undefined),
      pick_sketch_region: vi.fn(() => makeRegionPick(20n, 21n)),
      sketch_edge_island: vi.fn(() => 5n),
      sketch_island_edges: vi.fn(() => new BigUint64Array(CIRCLE_EDGES)),
      sketch_curve_edges: vi.fn(() => new BigUint64Array([])),
      sketch_edge_endpoints: vi.fn((_s: bigint, e: bigint) => {
        const k = CIRCLE_EDGES.indexOf(e)
        const a = RIM[k]
        const b = RIM[(k + 1) % RIM.length]
        return new Float64Array([a[0], a[1], a[2], b[0], b[1], b[2]])
      }),
      sketch_edge_curve: vi.fn(() => 44n),
      sketch_curve_geom: vi.fn(() => new Float64Array([0, 0, 0, 1])),
      // The PATH sketch is the ground; the PROFILE sketch is the argument.
      sketch_plane: vi.fn((s: bigint) =>
        s === 9n ? new Float64Array([0, 0, 0, 0, 0, 1]) : new Float64Array(profilePlane),
      ),
      region_boundary: vi.fn(() => new Float32Array([1, 0, 0, 1, 0, 0.2, 1.2, 0, 0.2])),
      node_parent: vi.fn(() => undefined),
      follow_me_along_edges: vi.fn(() => 77n),
      follow_me_around_face: vi.fn(() => 77n),
    } as unknown as WasmScene
  }

  /** Hex colors of everything currently in the preview group. */
  function colorsOf(preview: THREE.Group): number[] {
    return preview.children.map(
      (c) => ((c as THREE.Mesh).material as THREE.MeshBasicMaterial).color.getHex(),
    )
  }

  it('marks a circle path’s four quadrants once the path is picked', () => {
    const scene = circleScene([0, 0, 0, 0, 1, 0])
    const { tool, preview } = makeTool(scene)
    tool.onPointerDown(null, RAY) // pick the circle as the path

    expect(colorsOf(preview).filter((c) => c === cueColors().ok)).toHaveLength(4)
    // A circle's vertices are facet joints, not corners — nothing is blocked.
    expect(colorsOf(preview)).not.toContain(cueColors().blocked)
    const marks = preview.children.filter(
      (c) =>
        ((c as THREE.Mesh).material as THREE.MeshBasicMaterial).color.getHex() ===
        cueColors().ok,
    )
    for (const m of marks) {
      expect(Math.hypot(m.position.x, m.position.y)).toBeCloseTo(1, 9)
      expect(m.position.z).toBeCloseTo(0, 9)
    }
  })

  it('holds the markers at a constant on-screen size', () => {
    const scene = circleScene([0, 0, 0, 0, 1, 0])
    const { tool, preview } = makeTool(scene)
    tool.onPointerDown(null, RAY)
    const marker = preview.children.find(
      (c) =>
        ((c as THREE.Mesh).material as THREE.MeshBasicMaterial).color.getHex() ===
        cueColors().ok,
    )!

    const camera = new THREE.PerspectiveCamera(45, 1, 0.1, 100)
    camera.position.set(0, 0, 10)
    tool.updateGripScale(camera, 800)
    const near = marker.scale.x
    const nearDist = camera.position.distanceTo(marker.position)
    camera.position.set(0, 0, 20)
    tool.updateGripScale(camera, 800)
    // World size tracks camera distance exactly, so the PIXEL size is
    // unchanged — the property a `K · dist` shorthand also has.
    const farDist = camera.position.distanceTo(marker.position)
    expect(marker.scale.x / near).toBeCloseTo(farDist / nearDist, 9)
    const atFov45 = marker.scale.x
    // …and, unlike that shorthand, both fov and viewport height are honoured
    // rather than baked into the constant: halving the viewport height doubles
    // the world size, and widening the fov widens the marker with it.
    tool.updateGripScale(camera, 400)
    expect(marker.scale.x).toBeCloseTo(atFov45 * 2, 9)
    camera.fov = 90
    tool.updateGripScale(camera, 400)
    expect(marker.scale.x).toBeCloseTo(atFov45 * 2 * (1 / Math.tan(Math.PI / 8)), 9)
  })

  it('warns in the status bar while hovering a profile the kernel would refuse', () => {
    // A profile lying FLAT on the ground: correctly picked region, hopeless
    // placement. Before this, the user learned that from a post-click toast.
    const scene = circleScene([0, 0, 0, 0, 0, 1])
    const { tool, preview } = makeTool(scene)
    tool.onPointerDown(null, RAY)
    tool.onPointerMove(null, RAY)

    expect(tool.statusHint()).toContain('quadrant')
    expect(tool.statusHint()).toContain('refused')
    expect(colorsOf(preview)).toContain(cueColors().blocked) // region outlined as bad
  })

  it('confirms a radial profile before the click', () => {
    const scene = circleScene([0, 0, 0, 0, 1, 0]) // the y = 0 plane: radial
    const { tool, preview } = makeTool(scene)
    tool.onPointerDown(null, RAY)
    tool.onPointerMove(null, RAY)

    expect(tool.statusHint()).toContain('starts cleanly')
    expect(colorsOf(preview).filter((c) => c === cueColors().ok).length).toBeGreaterThan(4)
  })

  it('drops the verdict and the markers when the path is released', () => {
    const scene = circleScene([0, 0, 0, 0, 0, 1])
    const { tool, preview } = makeTool(scene)
    tool.onPointerDown(null, RAY)
    tool.onPointerMove(null, RAY)
    expect(tool.statusHint()).toContain('refused')

    tool.onKey({ key: 'Escape' } as KeyboardEvent)
    expect(tool.statusHint()).toContain('path')
    expect(preview.children.length).toBe(0)
  })
})

/**
 * The remaining refusal reasons and the deliberately-silent `unknown`, driven
 * through the tool's own hover wiring rather than through `evaluateStart`
 * alone — the unit specs prove the geometry rule, these prove the tool
 * actually plumbs each answer to the status bar and the viewport.
 */
describe('FollowMeTool — start affordance, the other verdicts', () => {
  const RECT_EDGES = [1n, 2n, 3n, 4n]
  /** A 2 × 1 rectangle path on the ground: plain segments, so every vertex is
   *  a corner the sweep cannot start on. */
  const CORNERS = [
    [0, 0, 0],
    [2, 0, 0],
    [2, 1, 0],
    [0, 1, 0],
  ] as const

  function rectScene(profilePlane: number[], regionBoundary?: Float32Array): WasmScene {
    return {
      pick_sketch_edge: vi.fn(() => makeEdgePick(9n, 1n)),
      pick_face: vi.fn(() => undefined),
      pick_sketch_region: vi.fn(() => makeRegionPick(20n, 21n)),
      sketch_edge_island: vi.fn(() => 5n),
      sketch_island_edges: vi.fn(() => new BigUint64Array(RECT_EDGES)),
      sketch_curve_edges: vi.fn(() => new BigUint64Array([])),
      sketch_edge_endpoints: vi.fn((_s: bigint, e: bigint) => {
        const k = RECT_EDGES.indexOf(e)
        const a = CORNERS[k]
        const b = CORNERS[(k + 1) % CORNERS.length]
        return new Float64Array([a[0], a[1], a[2], b[0], b[1], b[2]])
      }),
      sketch_edge_curve: vi.fn(() => undefined), // plain segments, no curve
      sketch_curve_geom: vi.fn(() => undefined),
      sketch_plane: vi.fn((s: bigint) =>
        s === 9n ? new Float64Array([0, 0, 0, 0, 0, 1]) : new Float64Array(profilePlane),
      ),
      region_boundary: vi.fn(
        () => regionBoundary ?? new Float32Array([0, 0, 0, 0, 0, 0.2, 0.2, 0, 0.2]),
      ),
      node_parent: vi.fn(() => undefined),
      follow_me_along_edges: vi.fn(() => 77n),
      follow_me_around_face: vi.fn(() => 77n),
    } as unknown as WasmScene
  }

  it('names the CORNER when the profile decisively hangs back over it, before any click', () => {
    // Plane x = 0 meets the rectangle at corner (0,0,0) (and, symmetrically,
    // at (0,1,0) — see followMeStart.ts's evaluateClosed corner-touch branch).
    // rectScene's DEFAULT region_boundary ring (all points at y = 0) sits
    // exactly AT the fold boundary and reads `ok` (see the companion
    // "accepting" test below) — this ring instead straddles y up to 0.2,
    // decisively over the (0,0,0) corner's wrong flank (the fold test
    // measures `-p.y`, so y > 0 folds back over the corner into the swept
    // material), so it must read `refused`/`corner-overhang`.
    const overhangRing = new Float32Array([0, 0.2, 0, 0, 0.2, 0.2, 0.2, 0.2, 0.2, 0.2, 0.2, 0])
    const scene = rectScene([0, 0, 0, 1, 0, 0], overhangRing) // plane x = 0, through (0,0,0)
    const { tool, onToast } = makeTool(scene)
    tool.onPointerDown(null, RAY) // pick the rectangle as the path
    tool.onPointerMove(null, RAY)

    expect(tool.statusHint()).toContain('corner')
    expect(tool.statusHint()).toContain('folding into its own material')
    expect(onToast).not.toHaveBeenCalled() // a hover never toasts
  })

  it('accepts a profile sitting on a corner when its ring is entirely beyond it', () => {
    // Same corner (0,0,0), same plane x = 0 — but the ring sits at y ≤ 0,
    // entirely beyond the corner along the fold direction, so the kernel's
    // advance check would not fold back into the swept material. A closed-
    // path CORNER is no longer unconditionally refused (design §2b): it is
    // legal exactly when the profile's own boundary clears the corner.
    const clearRing = new Float32Array([0, -0.2, 0, 0, -0.2, 0.2, 0.2, -0.2, 0.2])
    const scene = rectScene([0, 0, 0, 1, 0, 0], clearRing)
    const { tool, preview, onToast, onCommit } = makeTool(scene)
    tool.onPointerDown(null, RAY) // pick the rectangle as the path
    tool.onPointerMove(null, RAY)

    expect(tool.statusHint()).toContain('starts cleanly')
    expect(colorsOfGroup(preview)).toContain(cueColors().ok)
    expect(onToast).not.toHaveBeenCalled()

    // The kernel is still the sole authority — a predicted `ok` still lets
    // the click through. rectScene supplies sketch_plane/region_boundary, so
    // the E4 drag gesture can build a walk here: the profile press ARMS it,
    // and the release on the same ray (negligible movement) commits the FULL
    // sweep (stop_len undefined), exactly as a plain click always has — a
    // corner is a legal seam now, so nothing refuses it.
    tool.onPointerDown(null, RAY) // arm
    tool.onPointerUp(null, RAY) // release on the same ray — commits full sweep
    const call = (scene.follow_me_along_edges as ReturnType<typeof vi.fn>).mock.calls[0]
    expect(call[0]).toBe(20n)
    expect(call[1]).toBe(21n)
    expect(call[2]).toBe(9n)
    expect(call[4]).toBeUndefined() // full sweep, not partial
    expect(onCommit).toHaveBeenCalledWith(77n)
  })

  it('marks all four corners of a rectangle path as potential legal starts, not blocked', () => {
    // Under the new corner-seam rule every vertex of a plain-segment closed
    // path is a POTENTIAL corner-seam start (green ring) — the kernel's
    // Corner anchor only needs ONE adjacent flank to be a plain segment,
    // which is always true here. Only a joint between two DIFFERENT drawn
    // curves has no kernel anchor and stays blocked (not exercised by this
    // all-plain rectangle).
    const scene = rectScene([0, 0, 0, 1, 0, 0])
    const { tool, preview } = makeTool(scene)
    tool.onPointerDown(null, RAY)
    const legal = preview.children.filter(
      (c) =>
        ((c as THREE.Mesh).material as THREE.MeshBasicMaterial).color.getHex() ===
        cueColors().ok,
    )
    expect(legal).toHaveLength(4)
    const blocked = preview.children.filter(
      (c) =>
        ((c as THREE.Mesh).material as THREE.MeshBasicMaterial).color.getHex() ===
        cueColors().blocked,
    )
    expect(blocked).toHaveLength(0)
  })

  it('says the profile is square to the path but detached', () => {
    const scene = rectScene([5, 0, 0, 1, 0, 0]) // plane x = 5: nowhere near it
    const { tool } = makeTool(scene)
    tool.onPointerDown(null, RAY)
    tool.onPointerMove(null, RAY)
    expect(tool.statusHint()).toContain('does not touch')
  })

  it('claims NOTHING — no badge, no warning — for an undecidable placement', () => {
    // A face-loop path is f32, far coarser than the kernel's 1e-9 tolerances,
    // so a correct-looking placement is `unknown`: the generic stage guidance
    // stays, and no verdict badge is drawn.
    const scene = makeWasmScene({ facePick: makeFacePick(30n, 31n), regionPick: makeRegionPick(20n, 21n) })
    // Square to the loop's first segment ((0,0,1)→(1,0,1)) and crossing its
    // interior at x = 0.5 — the placement the kernel would accept, and which
    // an f64 sketch-edge path WOULD be told starts cleanly.
    ;(scene as unknown as { sketch_plane: unknown }).sketch_plane = vi.fn(
      () => new Float64Array([0.5, 0, 1, 1, 0, 0]),
    )
    ;(scene as unknown as { region_boundary: unknown }).region_boundary = vi.fn(
      () => new Float32Array([0, 0, 1, 0.2, 0, 1, 0.2, 0.2, 1]),
    )
    const { tool, preview } = makeTool(scene)
    tool.onPointerDown(null, RAY) // face loop becomes the path
    // Picking the path ALSO draws persistent green "legal" corner-seam start
    // markers now (a face loop's segments are never curve-attributed, so
    // every vertex is a potential legal start — see the fix-#2 test above).
    // Those are legitimately `ok`-colored and already present BEFORE the
    // hover this test examines, so the color check below is scoped to only
    // what the HOVER added, not the whole preview group.
    const beforeHover = preview.children.length
    tool.onPointerMove(null, RAY)

    expect(tool.statusHint()).toContain('Click the profile to sweep along')
    expect(tool.statusHint()).not.toContain('refused')
    expect(tool.statusHint()).not.toContain('starts cleanly')
    const addedByHover = preview.children.slice(beforeHover)
    const addedColors = addedByHover.map(
      (c) => ((c as THREE.Mesh).material as THREE.MeshBasicMaterial).color.getHex(),
    )
    expect(addedColors).not.toContain(cueColors().ok)
    // Only the region outline was added — no verdict badge alongside it.
    expect(preview.children.length).toBe(beforeHover + 1)
  })

  it('re-draws the cue on activate(), after the outgoing tool clears the shared preview', () => {
    // ToolController.setTool() constructs the incoming tool FIRST and only
    // then calls the OUTGOING tool's cancel(), which empties the shared
    // preview group — so a preselected path's markers, painted in the
    // constructor, were wiped before the tool ever went live.
    const scene = rectScene([0, 0, 0, 1, 0, 0])
    const preview = new THREE.Group()
    const selection: NodeRef[] = [{ kind: 'sketch-edge', id: 1n, sketch: 9n }]
    const tool = new FollowMeTool(scene, preview, vi.fn(), vi.fn(), vi.fn(), selection)
    expect(preview.children.length).toBeGreaterThan(0)

    clearPreview(preview) // what the outgoing tool's cancel() does
    expect(preview.children.length).toBe(0)

    tool.activate()
    expect(preview.children.length).toBeGreaterThan(0)
    expect(tool.statusHint()).toContain('profile')
  })
})

/**
 * Design §2a: an OPEN path's detached-but-perpendicular end is never refused
 * for the detachment alone — it is CARRIED rigidly to wherever the profile
 * stands. `followMeStart.test.ts` proves the geometry rule; this proves the
 * tool actually reads `carried` off the verdict and lets the click through.
 */
describe('FollowMeTool — open-path carried (detached-but-perpendicular) end', () => {
  /** A single straight open-path edge, (0,0,0) → (4,0,0). The profile plane
   *  is perpendicular to it (normal (1,0,0)) but not passed through either
   *  end vertex — the "square but detached" case design §2a carries rather
   *  than refuses. No `region_boundary` on this fixture: `_buildWalk` then
   *  fails and a click commits the full sweep immediately, exactly as it did
   *  before the E4 drag gesture existed (the drag gesture itself is covered
   *  separately, in the fixture below that DOES supply it).
   */
  function openPathScene(profilePlane: number[]): WasmScene {
    return {
      pick_sketch_edge: vi.fn(() => makeEdgePick(9n, 1n)),
      pick_face: vi.fn(() => undefined),
      pick_sketch_region: vi.fn(() => makeRegionPick(20n, 21n)),
      sketch_edge_island: vi.fn(() => undefined),
      sketch_island_edges: vi.fn(() => new BigUint64Array([])),
      sketch_curve_edges: vi.fn(() => new BigUint64Array([])),
      sketch_edge_endpoints: vi.fn(() => new Float64Array([0, 0, 0, 4, 0, 0])),
      sketch_edge_curve: vi.fn(() => undefined),
      sketch_curve_geom: vi.fn(() => undefined),
      sketch_plane: vi.fn((s: bigint) =>
        s === 9n ? new Float64Array([0, 0, 0, 0, 0, 1]) : new Float64Array(profilePlane),
      ),
      node_parent: vi.fn(() => undefined),
      follow_me_along_edges: vi.fn(() => 77n),
      follow_me_around_face: vi.fn(() => 77n),
    } as unknown as WasmScene
  }

  it('reads ok/carried and lets a detached-but-perpendicular end commit (never refused for detachment)', () => {
    // Plane x = 5: perpendicular to the path's (1,0,0) run, but nowhere near
    // either end vertex (both are well inside x ∈ [0,4]).
    const scene = openPathScene([5, 0, 0, 1, 0, 0])
    const { tool, onCommit, onToast } = makeTool(scene)
    tool.onPointerDown(null, RAY) // pick the open edge as the path
    tool.onPointerMove(null, RAY) // hover the profile

    expect(tool.statusHint()).toContain('follows the path’s shape')
    expect(tool.statusHint()).not.toContain('refused')

    tool.onPointerDown(null, RAY) // click — commits (no walk fixture here, so immediately)
    const call = (scene.follow_me_along_edges as ReturnType<typeof vi.fn>).mock.calls[0]
    expect(call[0]).toBe(20n)
    expect(call[1]).toBe(21n)
    expect(call[4]).toBeUndefined() // full sweep
    expect(onCommit).toHaveBeenCalledWith(77n)
    expect(onToast).not.toHaveBeenCalled()
  })
})

/**
 * E4 — drag the profile along the picked path for a partial sweep. Driven
 * entirely through the tool's public API: onPointerDown/onPointerMove/onKey/
 * statusHint, exactly as the Viewport calls it (this codebase has no
 * pointer-up — see the module doc's gesture note — so "release" is always
 * the NEXT press).
 */
describe('FollowMeTool — drag-to-partial-sweep (E4)', () => {
  /** A single straight open-path edge, (0,0,0) → (4,0,0), 4 m long, with a
   *  profile plane through (0,0,0) exactly perpendicular to it — an ordinary
   *  legal, non-carried start. UNLIKE `openPathScene` above, this fixture
   *  answers `region_boundary` for BOTH the profile's sketch (20n) and,
   *  incidentally, the path's (never queried for an edge path) — the one
   *  `_buildWalk` needs — so a profile press here ARMS the drag stage
   *  instead of falling back to an immediate commit.
   */
  function dragPathScene(): WasmScene {
    return {
      pick_sketch_edge: vi.fn(() => makeEdgePick(9n, 1n)),
      pick_face: vi.fn(() => undefined),
      pick_sketch_region: vi.fn(() => makeRegionPick(20n, 21n)),
      sketch_edge_island: vi.fn(() => undefined),
      sketch_island_edges: vi.fn(() => new BigUint64Array([])),
      sketch_curve_edges: vi.fn(() => new BigUint64Array([])),
      sketch_edge_endpoints: vi.fn(() => new Float64Array([0, 0, 0, 4, 0, 0])),
      sketch_edge_curve: vi.fn(() => undefined),
      sketch_curve_geom: vi.fn(() => undefined),
      sketch_plane: vi.fn((s: bigint) =>
        s === 9n ? new Float64Array([0, 0, 0, 0, 0, 1]) : new Float64Array([0, 0, 0, 1, 0, 0]),
      ),
      region_boundary: vi.fn(() => new Float32Array([0, 0, 0, 0, 0.1, 0, 0.1, 0.1, 0])),
      node_parent: vi.fn(() => undefined),
      follow_me_along_edges: vi.fn(() => 77n),
      follow_me_around_face: vi.fn(() => 77n),
    } as unknown as WasmScene
  }

  // RAY (module-level) looks straight down the z-axis through x = 0, y = 0 —
  // the walk's own seam point — so it arms at arc length 0. RAY_AT_2 looks
  // down through x = 2, y = 0, two meters further along the same straight
  // walk.
  const RAY_AT_2: Ray = { origin: [2, 0, 5], direction: [0, 0, -1] }

  it('pressing a valid profile region ARMS the drag stage rather than committing immediately', () => {
    const scene = dragPathScene()
    const { tool, onCommit } = makeTool(scene)
    tool.onPointerDown(null, RAY) // pick the open edge as the path
    tool.onPointerDown(null, RAY) // press the profile — arms, does not commit

    expect(tool.statusHint()).toContain('partial sweep')
    expect(scene.follow_me_along_edges).not.toHaveBeenCalled()
    expect(onCommit).not.toHaveBeenCalled()
  })

  it('onPointerMove while dragging reports a live, non-empty length to the VCB', () => {
    const scene = dragPathScene()
    const { tool, onMeasurement } = makeTool(scene)
    tool.onPointerDown(null, RAY) // pick path
    tool.onPointerDown(null, RAY) // arm at arc length 0
    onMeasurement.mockClear()

    tool.onPointerMove(null, RAY_AT_2)
    expect(onMeasurement).toHaveBeenCalled()
    const last = onMeasurement.mock.calls.at(-1)?.[0] as string
    expect(last).not.toBe('')
  })

  it('a release with negligible movement commits the FULL sweep (stop_len undefined)', () => {
    const scene = dragPathScene()
    const { tool, onCommit } = makeTool(scene)
    tool.onPointerDown(null, RAY) // pick path
    tool.onPointerDown(null, RAY) // arm at arc length 0
    tool.onPointerUp(null, RAY) // same ray — negligible movement — commits full

    const call = (scene.follow_me_along_edges as ReturnType<typeof vi.fn>).mock.calls[0]
    expect(call[0]).toBe(20n)
    expect(call[1]).toBe(21n)
    expect(call[4]).toBeUndefined()
    expect(onCommit).toHaveBeenCalledWith(77n)
  })

  it('a release after a real move commits a PARTIAL sweep, stop_len close to the dragged arc length', () => {
    const scene = dragPathScene()
    const { tool, onCommit } = makeTool(scene)
    tool.onPointerDown(null, RAY) // pick path
    tool.onPointerDown(null, RAY) // arm at arc length 0
    tool.onPointerMove(null, RAY_AT_2) // drag two meters along
    tool.onPointerUp(null, RAY_AT_2) // release — commits the partial sweep

    const call = (scene.follow_me_along_edges as ReturnType<typeof vi.fn>).mock.calls[0]
    expect(call[4]).toBeCloseTo(2, 6)
    expect(onCommit).toHaveBeenCalledWith(77n)
  })

  it('a second press while already dragging is inert — only onPointerUp ends the gesture', () => {
    // Real mice always alternate down/up, so this shouldn't happen — but
    // must stay harmless (no double-arm, no spurious commit) if it does.
    const scene = dragPathScene()
    const { tool, onCommit } = makeTool(scene)
    tool.onPointerDown(null, RAY) // pick path
    tool.onPointerDown(null, RAY) // arm
    tool.onPointerDown(null, RAY_AT_2) // stray second down — must be a no-op
    expect(onCommit).not.toHaveBeenCalled()
    expect(scene.follow_me_along_edges).not.toHaveBeenCalled()
    tool.onPointerUp(null, RAY) // the real release still commits (full sweep)
    expect(onCommit).toHaveBeenCalledWith(77n)
  })

  it('Escape while dragging returns to pick-profile (path stays highlighted), not pick-path', () => {
    const scene = dragPathScene()
    const { tool, preview, onMeasurement } = makeTool(scene)
    tool.onPointerDown(null, RAY) // pick path
    tool.onPointerDown(null, RAY) // arm
    expect(preview.children.length).toBeGreaterThan(0) // path highlight (+ start cues)
    onMeasurement.mockClear()

    tool.onKey({ key: 'Escape' } as KeyboardEvent)

    expect(tool.statusHint()).not.toContain('Drag along')
    expect(tool.statusHint()).toContain('profile')
    expect(tool.statusHint()).not.toContain('Click the path to follow')
    expect(onMeasurement).toHaveBeenCalledWith('')
    // The picked-path highlight (and start markers) survive — only the
    // in-progress drag overlay is dropped.
    expect(preview.children.length).toBeGreaterThan(0)
  })

  it('typing a length at pick-profile (no drag armed) then Enter commits a partial sweep against the hovered region', () => {
    const scene = dragPathScene()
    const { tool, onCommit } = makeTool(scene)
    tool.onPointerDown(null, RAY) // pick path → pick-profile
    tool.onPointerMove(null, RAY) // hover the profile — sets hoveredRegion, arms nothing

    tool.onKey({ key: '2' } as KeyboardEvent)
    tool.onKey({ key: 'Enter' } as KeyboardEvent)

    const call = (scene.follow_me_along_edges as ReturnType<typeof vi.fn>).mock.calls[0]
    expect(call[0]).toBe(20n)
    expect(call[1]).toBe(21n)
    expect(call[4]).toBe(2)
    expect(onCommit).toHaveBeenCalledWith(77n)
  })

  it('capturesKey gates digit/length keys behind a picked path, and never swallows a bare letter', () => {
    const scene = dragPathScene()
    const { tool } = makeTool(scene)
    expect(tool.capturesKey('5')).toBe(false) // pick-path: nothing picked yet
    expect(tool.capturesKey('r')).toBe(false)

    tool.onPointerDown(null, RAY) // pick path → pick-profile
    expect(tool.capturesKey('5')).toBe(true)
    expect(tool.capturesKey('r')).toBe(false)

    tool.onPointerDown(null, RAY) // arm the drag
    expect(tool.capturesKey('5')).toBe(true)
    expect(tool.capturesKey('r')).toBe(false)
  })

  it('capturesKey does NOT swallow a bare unit-suffix letter at pick-profile until a digit has started the buffer', () => {
    // isLengthInputKey accepts m/c/f (unit suffixes: meters/centimeters/
    // offset-tool... no — Move/Circle/Offset are ALSO real tool-switch
    // shortcuts. With an EMPTY buffer, merely hovering a profile (pick-
    // profile, nothing typed) must not swallow those letters — otherwise
    // pressing 'm' to switch to Move while just looking at a profile would
    // silently do nothing (or worse, get eaten as a VCB no-op). Once a digit
    // has started the buffer, the SAME letters must capture — that's what
    // lets "5m" (5 meters) type as a whole without "m" defecting to a tool
    // switch mid-entry.
    const scene = dragPathScene()
    const { tool } = makeTool(scene)
    tool.onPointerDown(null, RAY) // pick path → pick-profile
    for (const letter of ['m', 'c', 'f']) {
      expect(tool.capturesKey(letter)).toBe(false)
    }
    tool.onKey({ key: '5' } as KeyboardEvent) // starts the typed buffer
    for (const letter of ['m', 'c', 'f']) {
      expect(tool.capturesKey(letter)).toBe(true)
    }
  })

  it('capturesKey captures unit-suffix letters immediately while dragging (the active gesture)', () => {
    // Unlike pick-profile, `dragging` is the active gesture (MoveTool's own
    // convention: full keyboard capture once a drag has actually started),
    // so a letter is captured even before any digit starts the buffer.
    const scene = dragPathScene()
    const { tool } = makeTool(scene)
    tool.onPointerDown(null, RAY) // pick path
    tool.onPointerDown(null, RAY) // arm the drag
    for (const letter of ['m', 'c', 'f']) {
      expect(tool.capturesKey(letter)).toBe(true)
    }
  })

  it("onMeasurement clears ('') on cancel(), on Escape back to pick-path, and after a successful commit", () => {
    const scene = dragPathScene()
    const { tool, onMeasurement } = makeTool(scene)

    tool.onPointerDown(null, RAY) // pick path
    onMeasurement.mockClear()
    tool.cancel()
    expect(onMeasurement).toHaveBeenCalledWith('')

    onMeasurement.mockClear()
    tool.onPointerDown(null, RAY) // pick path again
    tool.onKey({ key: 'Escape' } as KeyboardEvent) // back to pick-path
    expect(onMeasurement).toHaveBeenCalledWith('')
    expect(tool.statusHint()).toContain('Click the path to follow')

    onMeasurement.mockClear()
    tool.onPointerDown(null, RAY) // pick path
    tool.onPointerDown(null, RAY) // arm
    tool.onPointerUp(null, RAY) // same ray — commits full sweep
    expect(onMeasurement).toHaveBeenCalledWith('')
  })
})

/** Hex colors of everything in a preview group (module-scope helper). */
function colorsOfGroup(preview: THREE.Group): number[] {
  return preview.children.map(
    (c) => ((c as THREE.Mesh).material as THREE.MeshBasicMaterial).color.getHex(),
  )
}

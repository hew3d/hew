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
  /** An instance's pose (row-major 3×4), for `instance_pose` — only consulted
   *  by an instance-face path/profile pick. `undefined` = no instance in play
   *  in most tests, but note `instance_pose` itself still needs a fallback
   *  (identity) so a test that DOES pick an instanced face doesn't crash on a
   *  missing mock. */
  instancePose?: number[]
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
    instance_pose: vi.fn(
      () => new Float64Array(opts.instancePose ?? [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0]),
    ),
    // The shared `defaultFaceEligible` fallback (no injected predicate in these
    // unit tests) reads node_parent to reject grouped faces.
    node_parent: vi.fn(() => opts.faceParent),
    follow_me_along_edges: vi.fn(follow),
    follow_me_around_face: vi.fn(follow),
    follow_me_around_instance_face: vi.fn(follow),
    follow_me_merged_around_face: vi.fn(follow),
    follow_me_face_along_edges: vi.fn(follow),
    follow_me_face_around_face: vi.fn(follow),
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
    expect(scene.follow_me_around_face).toHaveBeenCalledWith(20n, 21n, 30n, 31n, undefined, null)
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
    // — and nothing else: the path no longer carries any start-affordance
    // markers (see `followMeStart.ts`'s module docs — that layer was removed;
    // only the profile-hover verdict, drawn on the NEXT hover, remains).
    const colors = preview.children.map((c) =>
      ((c as THREE.Mesh).material as THREE.LineBasicMaterial).color.getHex(),
    )
    expect(colors).toEqual([pathHoverColors().path]) // just the picked path
    expect(preview.children).toHaveLength(1)
    const pathObj = preview.children[0]
    expect(pathObj).toBeInstanceOf(LineSegments2)
    expect((pathObj as LineSegments2).material).toBeInstanceOf(LineMaterial)
    expect(tool.statusHint()).toContain('profile')
  })

  it('an INSTANCED face IS now previewed as a target, pose-mapped into world space', () => {
    // §2e: an instanced face is a legal PATH now — `_faceLoopWorld` maps the
    // definition-local `face_boundary` loop through `instance_pose`.
    const scene = makeWasmScene({ facePick: makeFacePick(30n, 31n, 99n) })
    const { tool, preview } = makeTool(scene)
    tool.onPointerMove(null, RAY)
    expect(preview.children.length).toBe(1)
    expect(scene.face_boundary).toHaveBeenCalledWith(30n, 31n)
    expect(scene.instance_pose).toHaveBeenCalledWith(99n)
  })
})

describe('FollowMeTool — face frame guard (only plain top-level faces sweep)', () => {
  it('an INSTANCED face is a legal PATH now, UNCONDITIONALLY, routed through follow_me_around_instance_face', () => {
    // §2e: no longer refused outright — and, unlike a plain/grouped face,
    // this needs no `_faceEligible` injection at all: an instance path is a
    // read-only geometric reference, not an edit, so it bypasses that
    // policy entirely (see `_faceFollowable`'s doc) — it works even at the
    // DEFAULT policy, which hardcodes "no instance" for every OTHER purpose.
    const facePick = makeFacePick(30n, 31n, 99n)
    const regionPick = makeRegionPick(20n, 21n)
    const scene = makeWasmScene({ facePick, regionPick })
    const { tool, onCommit } = makeTool(scene)

    tool.onPointerDown(null, RAY) // click the instanced face at the path stage
    expect(tool.statusHint()).toContain('profile')
    tool.onPointerDown(null, RAY) // profile → commit
    expect(scene.follow_me_around_instance_face).toHaveBeenCalledWith(
      20n, 21n, 99n, 30n, 31n, undefined,
    )
    expect(scene.follow_me_around_face).not.toHaveBeenCalled()
    expect(onCommit).toHaveBeenCalledWith(77n)
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

  it('a component-DEFINITION context refuses wholesale (the scoped gap)', () => {
    const scene = makeWasmScene({ facePick: makeFacePick(30n, 31n) })
    const { tool, onToast } = makeTool(scene)
    tool.setComponentContext(50n) // editing a component's shared definition

    tool.onPointerDown(null, RAY)
    expect(onToast).toHaveBeenCalledTimes(1)
    expect(onToast.mock.calls[0][0]).toMatch(/component.*definition|step out/i)
    expect(onToast.mock.calls[0][1]).toBeUndefined() // guidance, not a kernel code
    expect(tool.statusHint()).toContain('Click the path to follow')
  })

  it('a GROUP editing context is fully legal now (design §2f) — no refusal', () => {
    const facePick = makeFacePick(30n, 31n)
    const regionPick = makeRegionPick(20n, 21n)
    const scene = makeWasmScene({ facePick, regionPick })
    const { tool, onToast, onCommit } = makeTool(scene)
    tool.setContextScoped(true) // hint-wording-only now; not a gate
    tool.setActiveGroup(70n)

    tool.onPointerDown(null, RAY) // path
    tool.onPointerDown(null, RAY) // profile → commit
    expect(onToast).not.toHaveBeenCalled()
    // The trailing arg is the group birth (design §2f) — the sweep lands
    // inside the group being edited instead of at top level.
    expect(scene.follow_me_around_face).toHaveBeenCalledWith(20n, 21n, 30n, 31n, undefined, 70n)
    expect(onCommit).toHaveBeenCalledWith(77n)
  })

  it('the stale-preselection face recovery is frame-gated too (a component-definition context refuses)', () => {
    const facePick = makeFacePick(30n, 31n)
    const regionPick = makeRegionPick(20n, 21n)
    const scene = makeWasmScene({ facePick, regionPick })
    // A leftover single-edge preselection → the tool starts at pick-profile.
    const selection: NodeRef[] = [{ kind: 'sketch-edge', id: 1n, sketch: 9n }]
    const { tool, onCommit, onToast } = makeTool(scene, selection)
    tool.setComponentContext(50n)

    // "Click the box's top face" while editing a definition → refused, and
    // the stale preselection is NOT swapped for a wrong-frame face.
    ;(scene.pick_sketch_region as ReturnType<typeof vi.fn>).mockReturnValueOnce(undefined)
    tool.onPointerDown(null, RAY)
    expect(onToast).toHaveBeenCalledTimes(1)
    expect(onToast.mock.calls[0][0]).toMatch(/component.*definition|step out/i)
    expect(onCommit).not.toHaveBeenCalled()
    expect(scene.follow_me_around_face).not.toHaveBeenCalled()
  })

  it('repeated ineligible-face clicks during stale-preselection recovery do not stack toasts', () => {
    // Grouped, not instanced — an instanced face is unconditionally a legal
    // PATH now (see `_faceFollowable`'s doc), so it can no longer stand in
    // for "ineligible" here; a grouped face still can.
    const facePick = makeFacePick(30n, 31n)
    const scene = makeWasmScene({ facePick, faceParent: 88n }) // no regionPick → every region pick misses
    const selection: NodeRef[] = [{ kind: 'sketch-edge', id: 1n, sketch: 9n }]
    const { tool, onCommit, onToast } = makeTool(scene, selection)

    // Three clicks that each miss the region and land on the same ineligible
    // face: the refusal speaks once, then stays quiet — the same anti-spam
    // dedup the path stage uses, not a fresh toast per click.
    tool.onPointerDown(null, RAY)
    tool.onPointerDown(null, RAY)
    tool.onPointerDown(null, RAY)
    expect(onToast).toHaveBeenCalledTimes(1)
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
    expect(scene.follow_me_around_face).toHaveBeenCalledWith(20n, 21n, 30n, 31n, undefined, null)
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
    expect(scene.follow_me_around_face).toHaveBeenCalledWith(20n, 21n, 30n, 31n, undefined, null)
    expect(scene.follow_me_along_edges).not.toHaveBeenCalled()
    expect(onCommit).toHaveBeenCalledWith(77n)
  })

  it('a stray face graze at the profile stage becomes the FALLBACK PROFILE (design §3a), never a PATH substitution', () => {
    const edgePick = makeEdgePick(9n, 1n)
    const facePick = makeFacePick(30n, 31n)
    const scene = makeWasmScene({ edgePick, facePick, islandEdges: [1n, 2n] })
    // No preselection: the user picks the path deliberately in the tool.
    const { tool, onCommit } = makeTool(scene)
    tool.onPointerDown(null, RAY) // pick the edge island as the path
    expect(tool.statusHint()).toContain('Click the profile')
    // The in-tool hint drops the "solid-face click follows that face" promise
    // (that recovery is preselection-only) — it offers the merge gesture
    // instead, which does nothing for an edge path (no merged entry point).
    expect(tool.statusHint()).not.toContain('follows that face')

    // No sketch region under the cursor, but an eligible solid face is: it
    // becomes the PROFILE. The PATH is never silently substituted — it's
    // still the edge island, passed to `follow_me_face_along_edges` verbatim.
    tool.onPointerDown(null, RAY)
    expect(scene.follow_me_around_face).not.toHaveBeenCalled()
    expect(scene.follow_me_along_edges).not.toHaveBeenCalled()
    const call = (scene.follow_me_face_along_edges as ReturnType<typeof vi.fn>).mock.calls[0]
    expect(call[0]).toBe(30n) // profile object
    expect(call[1]).toBe(31n) // profile face
    expect(call[2]).toBe(9n) // path sketch
    expect(Array.from(call[3] as BigUint64Array)).toEqual([1n, 2n]) // path edges, untouched
    expect(onCommit).toHaveBeenCalledWith(77n)
  })

  it('an ineligible stray face graze at the profile stage stays quiet and the path survives', () => {
    const edgePick = makeEdgePick(9n, 1n)
    const facePick = makeFacePick(30n, 31n, 99n) // instanced, no injected eligibility → ineligible
    const scene = makeWasmScene({ edgePick, facePick, islandEdges: [1n, 2n] })
    const { tool, onCommit, onToast } = makeTool(scene)
    tool.onPointerDown(null, RAY) // pick the edge island as the path

    // A deliberate face pick that's ineligible as a profile IS named (unlike a
    // bare near-miss of empty space) — but the path is never touched.
    tool.onPointerDown(null, RAY)
    expect(onToast).toHaveBeenCalledTimes(1)
    expect(onCommit).not.toHaveBeenCalled()
    expect(tool.statusHint()).toContain('Click the profile')

    // The next, better-aimed profile click commits against the ORIGINAL
    // edge path — never the grazed face.
    ;(scene.pick_sketch_region as ReturnType<typeof vi.fn>).mockReturnValueOnce(makeRegionPick(20n, 21n))
    tool.onPointerDown(null, RAY)
    expect(scene.follow_me_around_face).not.toHaveBeenCalled()
    expect(scene.follow_me_face_along_edges).not.toHaveBeenCalled()
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

describe('FollowMeTool — merge gesture (Ctrl/Cmd-click, design §3b)', () => {
  it('a plain click on a face-loop path births a SEPARATE object (no merge)', () => {
    const facePick = makeFacePick(30n, 31n)
    const regionPick = makeRegionPick(20n, 21n)
    const scene = makeWasmScene({ facePick, regionPick })
    const { tool, onCommit } = makeTool(scene)

    tool.onPointerDown(null, RAY) // path: face loop
    tool.onPointerDown(null, RAY) // profile, no modifier → separate birth
    expect(scene.follow_me_around_face).toHaveBeenCalledWith(20n, 21n, 30n, 31n, undefined, null)
    expect(scene.follow_me_merged_around_face).not.toHaveBeenCalled()
    expect(onCommit).toHaveBeenCalledWith(77n)
  })

  it('Ctrl/Cmd-click on a face-loop path commits the MERGED sweep instead', () => {
    const facePick = makeFacePick(30n, 31n)
    const regionPick = makeRegionPick(20n, 21n)
    const scene = makeWasmScene({ facePick, regionPick })
    const { tool, onCommit } = makeTool(scene)

    tool.onPointerDown(null, RAY) // path: face loop
    expect(tool.statusHint()).toContain('Ctrl/Cmd-click to merge')
    tool.setMergeModifier(true) // the Viewport's live read, right before dispatch
    tool.onPointerDown(null, RAY) // profile, merge held
    expect(scene.follow_me_merged_around_face).toHaveBeenCalledWith(20n, 21n, 30n, 31n, undefined)
    expect(scene.follow_me_around_face).not.toHaveBeenCalled()
    expect(onCommit).toHaveBeenCalledWith(77n)
  })

  it('is a no-op on an EDGE path — no merged entry point, and no hint promises one', () => {
    const edgePick = makeEdgePick(9n, 1n)
    const regionPick = makeRegionPick(20n, 21n)
    const scene = makeWasmScene({ edgePick, regionPick, islandEdges: [1n] })
    const { tool, onCommit } = makeTool(scene)

    tool.onPointerDown(null, RAY) // path: sketch edges
    expect(tool.statusHint()).not.toContain('merge')
    tool.setMergeModifier(true)
    tool.onPointerDown(null, RAY)
    expect(scene.follow_me_along_edges).toHaveBeenCalledTimes(1)
    expect(scene.follow_me_merged_around_face).not.toHaveBeenCalled()
    expect(onCommit).toHaveBeenCalledWith(77n)
  })

  it('is a no-op on an INSTANCE-face path — still routes through follow_me_around_instance_face', () => {
    const facePick = makeFacePick(30n, 31n, 99n)
    const regionPick = makeRegionPick(20n, 21n)
    const scene = makeWasmScene({ facePick, regionPick })
    const { tool, onCommit } = makeTool(scene)

    tool.onPointerDown(null, RAY) // path: instanced face
    tool.setMergeModifier(true)
    tool.onPointerDown(null, RAY)
    expect(scene.follow_me_around_instance_face).toHaveBeenCalledWith(
      20n, 21n, 99n, 30n, 31n, undefined,
    )
    expect(scene.follow_me_merged_around_face).not.toHaveBeenCalled()
    expect(onCommit).toHaveBeenCalledWith(77n)
  })

  it('a live release read (onPointerUp) decides the drag-commit path, not the arming press', () => {
    // The seam-walk needs sketch_plane/region_boundary to build (unlike most
    // fixtures here, which omit them and so fall back to an immediate
    // commit) — supply both so this exercises the REAL onPointerUp release.
    const facePick = makeFacePick(30n, 31n)
    const regionPick = makeRegionPick(20n, 21n)
    const scene = makeWasmScene({ facePick, regionPick })
    ;(scene as unknown as { sketch_plane: unknown }).sketch_plane = vi.fn(
      () => new Float64Array([0, 0, 0, 0, 0, 1]),
    )
    ;(scene as unknown as { region_boundary: unknown }).region_boundary = vi.fn(
      () => new Float32Array([0, 0, 0, 0, 0, 0.2, 0.2, 0, 0.2]),
    )
    const { tool, onCommit } = makeTool(scene)

    tool.onPointerDown(null, RAY) // path: face loop
    tool.setMergeModifier(false) // not held at the press
    tool.onPointerDown(null, RAY) // arm the drag (no merge yet)
    tool.setMergeModifier(true) // held by the time of the RELEASE
    tool.onPointerUp(null, RAY) // release on the same ray — full sweep, merged
    expect(scene.follow_me_merged_around_face).toHaveBeenCalledWith(20n, 21n, 30n, 31n, undefined)
    expect(onCommit).toHaveBeenCalledWith(77n)
  })
})

describe('FollowMeTool — K4 merge fallback (corner/edge-only contact)', () => {
  it('a DegenerateContact merge refusal falls back to the separate-birth commit, with a status message', () => {
    const facePick = makeFacePick(30n, 31n)
    const regionPick = makeRegionPick(20n, 21n)
    const scene = makeWasmScene({ facePick, regionPick })
    // Only the MERGED entry point refuses — the separate-birth one this
    // falls back to (`follow_me_around_face`) keeps `makeWasmScene`'s
    // default success behavior.
    ;(scene as unknown as { follow_me_merged_around_face: unknown }).follow_me_merged_around_face =
      vi.fn(() => {
        throw new Error('DegenerateContact: objects only touch at a corner')
      })
    const { tool, onCommit, onToast } = makeTool(scene)

    tool.onPointerDown(null, RAY) // path: face loop
    tool.setMergeModifier(true)
    tool.onPointerDown(null, RAY) // profile, merge held — merge refuses, falls back

    expect(scene.follow_me_merged_around_face).toHaveBeenCalledTimes(1)
    // The fallback call is the ordinary separate-birth commit for this exact
    // path/profile pair — same args a plain (unmerged) click would send.
    expect(scene.follow_me_around_face).toHaveBeenCalledWith(20n, 21n, 30n, 31n, undefined, null)
    expect(onCommit).toHaveBeenCalledWith(77n)
    expect(onToast).toHaveBeenCalledTimes(1)
    expect(onToast.mock.calls[0][0]).toMatch(/edge or corner.*separate object/i)
    // A status note, not an error toast — no kernel error code attached.
    expect(onToast.mock.calls[0][1]).toBeUndefined()
  })

  it('the separate-birth fallback itself failing surfaces ITS OWN refusal, not the superseded merge one', () => {
    const facePick = makeFacePick(30n, 31n)
    const regionPick = makeRegionPick(20n, 21n)
    const scene = makeWasmScene({ facePick, regionPick })
    ;(scene as unknown as { follow_me_merged_around_face: unknown }).follow_me_merged_around_face =
      vi.fn(() => {
        throw new Error('DegenerateContact: objects only touch at a corner')
      })
    ;(scene as unknown as { follow_me_around_face: unknown }).follow_me_around_face = vi.fn(() => {
      throw new Error('ProfileNotPerpendicular: no perpendicular segment')
    })
    const { tool, onCommit, onToast } = makeTool(scene)

    tool.onPointerDown(null, RAY)
    tool.setMergeModifier(true)
    tool.onPointerDown(null, RAY)

    expect(onCommit).not.toHaveBeenCalled()
    expect(onToast).toHaveBeenCalledTimes(1)
    expect(onToast.mock.calls[0][0]).toMatch(/face is parallel to the profile/i)
    expect(onToast.mock.calls[0][1]).toBe('ProfileNotPerpendicular')
  })

  it('a DegenerateContact refusal on a PLAIN (non-merge) commit is never treated as a fallback candidate', () => {
    // `merge` is false here — `_invokeFollowMe` never even calls the merged
    // wasm entry point for a plain commit (see its doc), so this refusal
    // can only be the ordinary follow_me_around_face path failing on its
    // own — no retry, no status message, the plain refusal copy.
    const facePick = makeFacePick(30n, 31n)
    const regionPick = makeRegionPick(20n, 21n)
    const scene = makeWasmScene({
      facePick,
      regionPick,
      commitError: 'DegenerateContact: objects only touch at a corner',
    })
    const { tool, onCommit, onToast } = makeTool(scene)

    tool.onPointerDown(null, RAY) // path: face loop
    tool.onPointerDown(null, RAY) // profile, NO merge modifier

    expect(scene.follow_me_merged_around_face).not.toHaveBeenCalled()
    expect(onCommit).not.toHaveBeenCalled()
    expect(onToast).toHaveBeenCalledTimes(1)
    expect(onToast.mock.calls[0][1]).toBe('DegenerateContact')
  })

  it('other merge refusals (not DegenerateContact) surface unchanged, no fallback attempted', () => {
    const facePick = makeFacePick(30n, 31n)
    const regionPick = makeRegionPick(20n, 21n)
    const scene = makeWasmScene({
      facePick,
      regionPick,
      commitError: 'ProfileNotPerpendicular: no perpendicular segment',
    })
    const { tool, onCommit, onToast } = makeTool(scene)

    tool.onPointerDown(null, RAY)
    tool.setMergeModifier(true)
    tool.onPointerDown(null, RAY)

    expect(scene.follow_me_around_face).not.toHaveBeenCalled()
    expect(onCommit).not.toHaveBeenCalled()
    expect(onToast.mock.calls[0][0]).toMatch(/face is parallel to the profile/i)
    expect(onToast.mock.calls[0][1]).toBe('ProfileNotPerpendicular')
  })
})

describe('FollowMeTool — solid-face PROFILES (design §3a)', () => {
  it('a face profile around a face-loop path routes through follow_me_face_around_face', () => {
    const facePick = makeFacePick(30n, 31n)
    const scene = makeWasmScene({ facePick }) // no regionPick → every region pick misses
    const { tool, onCommit } = makeTool(scene)

    tool.onPointerDown(null, RAY) // path: face loop (30n/31n)
    // A second, different face becomes the profile.
    ;(scene.pick_face as ReturnType<typeof vi.fn>).mockReturnValue(makeFacePick(40n, 41n))
    tool.onPointerDown(null, RAY) // no region under the cursor → face-profile fallback
    expect(scene.follow_me_face_around_face).toHaveBeenCalledWith(40n, 41n, 30n, 31n, undefined)
    expect(onCommit).toHaveBeenCalledWith(77n)
  })

  it('never offered for an INSTANCE-face path (no kernel entry point combines the two)', () => {
    const facePick = makeFacePick(30n, 31n, 99n)
    const scene = makeWasmScene({ facePick })
    const { tool, onCommit, onToast } = makeTool(scene)

    tool.onPointerDown(null, RAY) // path: instanced face
    tool.onPointerDown(null, RAY) // no region; a face IS under the cursor
    expect(onCommit).not.toHaveBeenCalled()
    expect(onToast).not.toHaveBeenCalled() // stays quiet — a scoped-out combination, not a refusal
    expect(scene.follow_me_face_along_edges).not.toHaveBeenCalled()
    expect(scene.follow_me_face_around_face).not.toHaveBeenCalled()
  })

  it('a component-DEFINITION context blocks the fallback too', () => {
    const edgePick = makeEdgePick(9n, 1n)
    const facePick = makeFacePick(30n, 31n)
    const scene = makeWasmScene({ edgePick, facePick, islandEdges: [1n] })
    const { tool, onToast, onCommit } = makeTool(scene)

    tool.onPointerDown(null, RAY) // path: edges
    tool.setComponentContext(50n)
    tool.onPointerDown(null, RAY) // face under cursor, no region
    expect(onToast).toHaveBeenCalledTimes(1)
    expect(onToast.mock.calls[0][0]).toMatch(/component.*definition|step out/i)
    expect(onCommit).not.toHaveBeenCalled()
    expect(scene.follow_me_face_along_edges).not.toHaveBeenCalled()
  })

  it('the hover cue names the merge when the face belongs to the SAME solid the path runs on', () => {
    const scene = makeWasmScene({ facePick: makeFacePick(30n, 31n) }) // no regionPick
    const { tool } = makeTool(scene)
    tool.onPointerDown(null, RAY) // path: face loop on object 30n
    tool.onPointerMove(null, RAY) // hover the SAME object's another face
    expect(tool.statusHint()).toContain('merges straight into the solid')
  })

  it('the hover cue is plain when the face belongs to a DIFFERENT solid', () => {
    const facePick = makeFacePick(30n, 31n)
    const scene = makeWasmScene({ facePick }) // no regionPick
    const { tool } = makeTool(scene)
    tool.onPointerDown(null, RAY) // path: face loop on object 30n
    ;(scene.pick_face as ReturnType<typeof vi.fn>).mockReturnValue(makeFacePick(40n, 41n))
    tool.onPointerMove(null, RAY) // hover a DIFFERENT object's face
    expect(tool.statusHint()).toBe('Click this face to use it as the profile.')
  })

  it('discloses the group-birth gap while editing a group (no group-birth surface on this route)', () => {
    // `follow_me_face_along_edges`/`follow_me_face_around_face` have no
    // `group` parameter at all (unlike the plain sketch-region routes) — a
    // commit here always lands top-level even while editing a group. The
    // hint says so instead of landing silently somewhere the user didn't
    // expect (a real gap two independent reviewers confirmed).
    const edgePick = makeEdgePick(9n, 1n)
    const facePick = makeFacePick(30n, 31n)
    const scene = makeWasmScene({ edgePick, facePick, islandEdges: [1n] })
    const { tool } = makeTool(scene)
    tool.setActiveGroup(70n)
    tool.onPointerDown(null, RAY) // path: edges
    tool.onPointerMove(null, RAY) // hover the fallback face profile
    expect(tool.statusHint()).toContain('will land at the top level')
    expect(tool.statusHint()).toContain('not inside the group')
  })

  it('does NOT disclose the group-birth gap when no group is being edited', () => {
    const edgePick = makeEdgePick(9n, 1n)
    const facePick = makeFacePick(30n, 31n)
    const scene = makeWasmScene({ edgePick, facePick, islandEdges: [1n] })
    const { tool } = makeTool(scene)
    tool.onPointerDown(null, RAY) // path: edges
    tool.onPointerMove(null, RAY) // hover the fallback face profile
    expect(tool.statusHint()).not.toContain('top level')
  })
})

describe('FollowMeTool — group-birth gap disclosure on an instance-face path', () => {
  it('discloses the gap for an instance-face path with a sketch-region profile', () => {
    // follow_me_around_instance_face also has no group parameter — same
    // silent-top-level-landing gap, reached through the PRIMARY (not
    // fallback) profile flow this time. `sketch_plane` is mocked with a
    // normal square to nothing on the (z = 1) face loop, so a real 'orient'
    // verdict computes instead of falling through to 'unknown' on a missing
    // mock — the branch this test exists to exercise.
    const facePick = makeFacePick(30n, 31n, 99n)
    const regionPick = makeRegionPick(20n, 21n)
    const scene = makeWasmScene({ facePick, regionPick })
    ;(scene as unknown as { sketch_plane: unknown }).sketch_plane = vi.fn(
      () => new Float64Array([0.5, 0.5, 1, 0, 0, 1]),
    )
    const { tool } = makeTool(scene)
    tool.setActiveGroup(70n)
    tool.onPointerDown(null, RAY) // path: instanced face
    tool.onPointerMove(null, RAY) // hover the sketch-region profile
    expect(tool.statusHint()).toContain('upright')
    expect(tool.statusHint()).toContain('will land at the top level')
  })

  it('does not disclose the gap for a PLAIN face-loop path (group threads correctly there)', () => {
    const facePick = makeFacePick(30n, 31n)
    const regionPick = makeRegionPick(20n, 21n)
    const scene = makeWasmScene({ facePick, regionPick })
    ;(scene as unknown as { sketch_plane: unknown }).sketch_plane = vi.fn(
      () => new Float64Array([0.5, 0.5, 1, 0, 0, 1]),
    )
    const { tool } = makeTool(scene)
    tool.setActiveGroup(70n)
    tool.onPointerDown(null, RAY) // path: plain face loop
    tool.onPointerMove(null, RAY) // hover the sketch-region profile
    expect(tool.statusHint()).toContain('upright') // same verdict as above…
    expect(tool.statusHint()).not.toContain('top level') // …but no gap here
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
 * The start verdict: hovering a profile region outlines it in a verdict
 * color, badges its centre, and replaces the status hint with what is wrong
 * (or, now, what will be fixed automatically) — before the click. (An earlier
 * version of this affordance also marked every legal start ON the path itself
 * before a profile was even picked; that layer was removed — see
 * `followMeStart.ts`'s module docs — and these specs were trimmed with it.)
 *
 * The geometry rule itself is specified in `followMeStart.test.ts`; these
 * cover the wiring: what gets drawn, in what color, and what the status bar
 * says. `follow-me-start-cue.spec.ts` cross-checks the prediction against the
 * real kernel.
 */
describe('FollowMeTool — start verdict', () => {
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

  it('picking the path draws only the path highlight — no start markers', () => {
    const scene = circleScene([0, 0, 0, 0, 1, 0])
    const { tool, preview } = makeTool(scene)
    tool.onPointerDown(null, RAY) // pick the circle as the path

    expect(preview.children).toHaveLength(1) // just the path highlight
    expect(colorsOf(preview)).not.toContain(cueColors().ok)
    expect(colorsOf(preview)).not.toContain(cueColors().blocked)
  })

  it('holds the verdict badge at a constant on-screen size', () => {
    const scene = circleScene([0, 0, 0, 0, 1, 0]) // radial: an 'ok' verdict
    const { tool, preview } = makeTool(scene)
    tool.onPointerDown(null, RAY) // pick the circle as the path
    tool.onPointerMove(null, RAY) // hover the profile: draws the verdict badge
    // The badge is the one Mesh with regular (non-fat-line) geometry in the
    // preview group — the path/hover highlights are LineSegments2.
    const marker = preview.children.find(
      (c) => c instanceof THREE.Mesh && !(c instanceof LineSegments2),
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

  it('informs (not warns) in the status bar while hovering a profile auto-orientation will fold up', () => {
    // A profile lying FLAT on the ground: correctly picked region, a
    // placement that used to be hopeless. Auto-orientation (design §2c) now
    // folds it into the lathe instead of refusing, so the cue is
    // informational (the `orient` color), never the red "refused" one.
    const scene = circleScene([0, 0, 0, 0, 0, 1])
    const { tool, preview } = makeTool(scene)
    tool.onPointerDown(null, RAY)
    tool.onPointerMove(null, RAY)

    expect(tool.statusHint()).toContain('upright')
    expect(tool.statusHint()).not.toContain('refused')
    expect(colorsOf(preview)).toContain(cueColors().orient) // region outlined as "will be oriented"
    expect(colorsOf(preview)).not.toContain(cueColors().blocked)
  })

  it('confirms a radial profile before the click', () => {
    const scene = circleScene([0, 0, 0, 0, 1, 0]) // the y = 0 plane: radial
    const { tool, preview } = makeTool(scene)
    tool.onPointerDown(null, RAY)
    tool.onPointerMove(null, RAY)

    expect(tool.statusHint()).toContain('starts cleanly')
    // The region outline plus the verdict badge — both 'ok'-colored now that
    // no on-path markers exist any more.
    expect(colorsOf(preview).filter((c) => c === cueColors().ok)).toHaveLength(2)
  })

  it('drops the verdict when the path is released', () => {
    const scene = circleScene([0, 0, 0, 0, 0, 1])
    const { tool, preview } = makeTool(scene)
    tool.onPointerDown(null, RAY)
    tool.onPointerMove(null, RAY)
    expect(tool.statusHint()).toContain('upright')

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
describe('FollowMeTool — start verdict, the other outcomes', () => {
  const RECT_EDGES = [1n, 2n, 3n, 4n]
  /** A 2 × 1 rectangle path on the ground: plain segments. */
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
    tool.onPointerDown(null, RAY) // face loop becomes the path — just the highlight
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

  it('re-draws the path highlight on activate(), after the outgoing tool clears the shared preview', () => {
    // ToolController.setTool() constructs the incoming tool FIRST and only
    // then calls the OUTGOING tool's cancel(), which empties the shared
    // preview group — so a preselected path's highlight, painted in the
    // constructor, was wiped before the tool ever went live.
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

describe('FollowMeTool — K2 direction-aware drag (signed partial sweep)', () => {
  /**
   * A closed 4×4 square loop path, (0,0,0) → (4,0,0) → (4,4,0) → (0,4,0) →
   * back — perimeter 16. The profile's plane (sketch 20) is normal +x
   * through the origin; its `region_boundary`'s mean is exactly (2,0,0),
   * the midpoint of the bottom edge — so the seam lands there exactly
   * (`seamWalk`'s own nearest-point-to-centroid rule), with 2 m of room on
   * either side along that edge before the walk turns a corner.
   *
   * Hand-verified forward walk (`followMeDrag.ts`'s `closedSeamWalk`):
   * points = [seam (2,0,0), (4,0,0), (4,4,0), (0,4,0), (0,0,0), seam
   * (2,0,0)], cumulative arc lengths [0, 2, 6, 10, 14, 16]. So a point on
   * the ORIGINAL (0,0,0)→(2,0,0) half of the bottom edge (x < 2) reads a
   * RAW forward arc length just under 16 — the walk's very last leg back to
   * the seam — which is exactly the "wraps to near-total instead of a
   * small reverse distance" shape K2 fixes.
   */
  function closedDragPathScene(): WasmScene {
    const endpoints: Record<string, [number, number, number, number, number, number]> = {
      '1': [0, 0, 0, 4, 0, 0],
      '2': [4, 0, 0, 4, 4, 0],
      '3': [4, 4, 0, 0, 4, 0],
      '4': [0, 4, 0, 0, 0, 0],
    }
    return {
      pick_sketch_edge: vi.fn(() => makeEdgePick(9n, 1n)),
      pick_face: vi.fn(() => undefined),
      pick_sketch_region: vi.fn(() => makeRegionPick(20n, 21n)),
      sketch_edge_island: vi.fn(() => 5n),
      sketch_island_edges: vi.fn(() => new BigUint64Array([1n, 2n, 3n, 4n])),
      sketch_curve_edges: vi.fn(() => new BigUint64Array([])),
      sketch_edge_endpoints: vi.fn(
        (_s: bigint, e: bigint) => new Float64Array(endpoints[e.toString()]),
      ),
      sketch_edge_curve: vi.fn(() => undefined),
      sketch_curve_geom: vi.fn(() => undefined),
      sketch_plane: vi.fn((s: bigint) =>
        s === 20n ? new Float64Array([0, 0, 0, 1, 0, 0]) : new Float64Array([0, 0, 0, 0, 0, 1]),
      ),
      // Mean of these three points is exactly (2, 0, 0).
      region_boundary: vi.fn(() => new Float32Array([2, -0.1, 0, 2.1, 0.05, 0, 1.9, 0.05, 0])),
      node_parent: vi.fn(() => undefined),
      follow_me_along_edges: vi.fn(() => 77n),
      follow_me_around_face: vi.fn(() => 77n),
    } as unknown as WasmScene
  }

  /** A ray straight down onto `(x, 0, 0)` on the picked path's bottom edge. */
  const RAY_AT = (x: number): Ray => ({ origin: [x, 0, 5], direction: [0, 0, -1] })
  /** A ray straight down onto an arbitrary point of the loop's boundary. */
  const RAY_ON = (x: number, y: number): Ray => ({ origin: [x, y, 5], direction: [0, 0, -1] })
  /** Arms the drag at signed length 0 (the seam sits exactly at x = 2). */
  const SEAM = RAY_AT(2)

  it('a forward drag commits a POSITIVE stop_len, unchanged from the pre-K2 mapping', () => {
    const scene = closedDragPathScene()
    const { tool, onCommit } = makeTool(scene)
    tool.onPointerDown(null, SEAM) // pick path
    tool.onPointerDown(null, SEAM) // arm at the seam
    tool.onPointerMove(null, RAY_AT(3)) // 1 m forward
    tool.onPointerUp(null, RAY_AT(3))

    const call = (scene.follow_me_along_edges as ReturnType<typeof vi.fn>).mock.calls[0]
    expect(call[4]).toBeCloseTo(1, 6)
    expect(onCommit).toHaveBeenCalledWith(77n)
  })

  it('a reverse drag (the OTHER way from the seam) commits a NEGATIVE stop_len', () => {
    const scene = closedDragPathScene()
    const { tool, onCommit } = makeTool(scene)
    tool.onPointerDown(null, SEAM)
    tool.onPointerDown(null, SEAM) // arm at the seam
    tool.onPointerMove(null, RAY_AT(1)) // 1 m the OTHER way
    tool.onPointerUp(null, RAY_AT(1))

    const call = (scene.follow_me_along_edges as ReturnType<typeof vi.fn>).mock.calls[0]
    // Before K2 this landed near +15 (almost the whole 16 m loop) — the
    // exact bug this fixes: a small reverse drag reading as a near-total
    // FORWARD sweep. It must be a small NEGATIVE value instead.
    expect(call[4]).toBeCloseTo(-1, 6)
    expect(onCommit).toHaveBeenCalledWith(77n)
  })

  it('crossing the seam mid-drag flips the sign smoothly, not via a raw near-total jump', () => {
    const scene = closedDragPathScene()
    const { tool, onCommit } = makeTool(scene)
    tool.onPointerDown(null, SEAM)
    tool.onPointerDown(null, SEAM) // arm at the seam
    tool.onPointerMove(null, RAY_AT(2.2)) // 0.2 m forward first
    tool.onPointerMove(null, RAY_AT(1.5)) // then cross the seam, 0.5 m reverse
    tool.onPointerUp(null, RAY_AT(1.5))

    const call = (scene.follow_me_along_edges as ReturnType<typeof vi.fn>).mock.calls[0]
    expect(call[4]).toBeCloseTo(-0.5, 6)
    expect(onCommit).toHaveBeenCalledWith(77n)
  })

  it('a full-wrap-and-back drag settles back near stop_len 0, not a stale huge value', () => {
    const scene = closedDragPathScene()
    const { tool, onCommit } = makeTool(scene)
    tool.onPointerDown(null, SEAM)
    tool.onPointerDown(null, SEAM) // arm at the seam
    // Walk most of the way around forward, in small steps (a real drag
    // never jumps more than a fraction of the loop between two frames),
    // then all the way back to the seam.
    for (const x of [2.5, 3, 3.5, 3.9]) tool.onPointerMove(null, RAY_AT(x))
    tool.onPointerMove(null, RAY_AT(2)) // back at the seam
    tool.onPointerUp(null, RAY_AT(2))

    // Negligible net movement from the arm point — reads as a plain click
    // (full sweep), exactly like the non-K2 click-vs-drag threshold.
    const call = (scene.follow_me_along_edges as ReturnType<typeof vi.fn>).mock.calls[0]
    expect(call[4]).toBeUndefined()
    expect(onCommit).toHaveBeenCalledWith(77n)
  })

  it('a drag that genuinely OVERSHOOTS a full lap, then reverses a little, still reads as the full sweep', () => {
    // Regression for a review finding: `_advanceDragLen`'s accumulator used
    // to CLAMP its own stored value every frame, which (once a drag pushed
    // past a full lap) desynced the stored total from the cursor's own
    // still-unclamped raw position — a small reverse tick right after the
    // overshoot then read as a real partial-sweep shortfall (~15.7 of 16)
    // instead of staying pinned at the full sweep. This walks the ENTIRE
    // loop forward in real per-frame steps (each well under half the
    // 16 m total, so the seam-crossing wrap logic never misfires), continues
    // 0.5 m into a SECOND lap, then reverses by 0.3 m — a small enough
    // reverse that a buggy per-frame clamp would read ~15.7 forward de-
    // spite the true (unclamped) position still being past a full lap.
    const scene = closedDragPathScene()
    const { tool, onCommit } = makeTool(scene)
    tool.onPointerDown(null, SEAM)
    tool.onPointerDown(null, SEAM) // arm at the seam (signed length 0)
    // Once around the 4×4 loop, in real per-frame steps along every edge:
    // bottom → right → top → left → back past the seam into a 2nd lap.
    tool.onPointerMove(null, RAY_AT(4)) // (4,0,0) — rawU 2
    tool.onPointerMove(null, RAY_ON(4, 4)) // (4,4,0) — rawU 6
    tool.onPointerMove(null, RAY_ON(0, 4)) // (0,4,0) — rawU 10
    tool.onPointerMove(null, RAY_AT(0)) // (0,0,0) — rawU 14
    tool.onPointerMove(null, RAY_AT(1)) // (1,0,0) — rawU 15
    tool.onPointerMove(null, RAY_AT(2.5)) // (2.5,0,0), crossing the seam — 0.5 m into lap 2
    tool.onPointerMove(null, RAY_AT(2.2)) // reverse 0.3 m, still past one full lap
    tool.onPointerUp(null, RAY_AT(2.2))

    const call = (scene.follow_me_along_edges as ReturnType<typeof vi.fn>).mock.calls[0]
    // The true (unclamped) position never dropped below the loop's own
    // 16 m total, so this must clamp to the full sweep — a defined
    // `stop_len` very close to 16, never the ~15.7 the clamp-desync bug
    // would have produced.
    expect(call[4]).toBeGreaterThan(15.9)
    expect(call[4]).toBeLessThanOrEqual(16)
    expect(onCommit).toHaveBeenCalledWith(77n)
  })

  it('the live VCB readout during a reverse drag stays POSITIVE with a "reverse" indicator', () => {
    const scene = closedDragPathScene()
    const { tool, onMeasurement } = makeTool(scene)
    tool.onPointerDown(null, SEAM)
    tool.onPointerDown(null, SEAM) // arm
    onMeasurement.mockClear()
    tool.onPointerMove(null, RAY_AT(1)) // 1 m reverse

    const last = onMeasurement.mock.calls.at(-1)?.[0] as string
    expect(last).toMatch(/^reverse /)
    expect(last).not.toMatch(/-/) // never a bare signed number
  })

  it('a forward drag reports the plain readout with NO direction indicator', () => {
    const scene = closedDragPathScene()
    const { tool, onMeasurement } = makeTool(scene)
    tool.onPointerDown(null, SEAM)
    tool.onPointerDown(null, SEAM) // arm
    onMeasurement.mockClear()
    tool.onPointerMove(null, RAY_AT(3)) // 1 m forward

    const last = onMeasurement.mock.calls.at(-1)?.[0] as string
    expect(last).not.toMatch(/reverse/)
  })

  it('a typed length + Enter always commits FORWARD, even mid-reverse-drag', () => {
    const scene = closedDragPathScene()
    const { tool, onCommit } = makeTool(scene)
    tool.onPointerDown(null, SEAM)
    tool.onPointerDown(null, SEAM) // arm
    tool.onPointerMove(null, RAY_AT(1)) // dragged reverse — liveLen is negative

    tool.onKey({ key: '2' } as KeyboardEvent)
    tool.onKey({ key: 'Enter' } as KeyboardEvent)

    const call = (scene.follow_me_along_edges as ReturnType<typeof vi.fn>).mock.calls[0]
    expect(call[4]).toBe(2) // positive, not -2 and not the live drag length
    expect(onCommit).toHaveBeenCalledWith(77n)
  })

  it("a reversed-stop refusal (PathTooTight) gets direction-specific copy that doesn't overclaim WHY, not the generic 'turns tighter' text", () => {
    // This fixture's seam is a mid-EDGE (Split) anchor, not an actual path
    // corner (see the fixture's own doc comment) — the kernel can return
    // this same PathTooTight code for a reversed stop from EITHER a corner
    // seam (one-directional by design) OR a genuinely tight bend folding
    // under the reversed walk (the generic advance check, anchor-agnostic
    // — confirmed by reading `Object::from_follow_me_impl`). The copy must
    // therefore read correctly for BOTH, so it deliberately never claims
    // "corner" specifically.
    const scene = closedDragPathScene()
    ;(scene as unknown as { follow_me_along_edges: unknown }).follow_me_along_edges = vi.fn(() => {
      throw new Error('PathTooTight: every anchor candidate refused')
    })
    const { tool, onToast } = makeTool(scene)
    tool.onPointerDown(null, SEAM)
    tool.onPointerDown(null, SEAM) // arm
    tool.onPointerMove(null, RAY_AT(1)) // reverse drag — negative stop_len
    tool.onPointerUp(null, RAY_AT(1))

    expect(onToast).toHaveBeenCalledTimes(1)
    expect(onToast.mock.calls[0][0]).not.toMatch(/turns tighter/i)
    expect(onToast.mock.calls[0][0]).not.toMatch(/corner/i)
    expect(onToast.mock.calls[0][0]).toMatch(/one direction/i)
    expect(onToast.mock.calls[0][1]).toBe('PathTooTight')
  })

  it('the SAME PathTooTight code on a FORWARD stop keeps the generic copy (only a negative stop reframes it)', () => {
    const scene = closedDragPathScene()
    ;(scene as unknown as { follow_me_along_edges: unknown }).follow_me_along_edges = vi.fn(() => {
      throw new Error('PathTooTight: the walk folds into itself')
    })
    const { tool, onToast } = makeTool(scene)
    tool.onPointerDown(null, SEAM)
    tool.onPointerDown(null, SEAM) // arm
    tool.onPointerMove(null, RAY_AT(3)) // forward drag — positive stop_len
    tool.onPointerUp(null, RAY_AT(3))

    expect(onToast.mock.calls[0][0]).toMatch(/turns tighter/i)
    expect(onToast.mock.calls[0][1]).toBe('PathTooTight')
  })
})

/** Hex colors of everything in a preview group (module-scope helper). */
function colorsOfGroup(preview: THREE.Group): number[] {
  return preview.children.map(
    (c) => ((c as THREE.Mesh).material as THREE.MeshBasicMaterial).color.getHex(),
  )
}

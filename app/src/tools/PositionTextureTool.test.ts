/**
 * PositionTextureTool unit tests — the tile-corner pin model.
 *
 * These tests pin what the USER can observe, not internal frame numerics
 * for their own sake: where the pins ARE relative to the face and the
 * texture tile, what ABSOLUTE angle the texture sits at (measured against
 * the face's own planar axes), where 1x scale is, and that snaps actually
 * take. The "absolute rotation" suite is the canary that fails if rotation
 * silently becomes relative to a drag's start again.
 *
 * Fake-WasmScene pattern mirrors PaintTool.test.ts.
 */
import { describe, it, expect, vi } from 'vitest'
import * as THREE from 'three'
import { PositionTextureTool } from './PositionTextureTool'
import type { Snap } from './types'
import type { Scene as WasmScene } from '../wasm/loader'
import type { Ray } from '../viewport/math'
import {
  frameToArray,
  planarDefaultFrame,
  frameToLocal2D,
  frameAbsolute,
  tessellatePlaneBasis,
  local2ToWorld,
  type UvFrameComponents,
} from './uvFrameMath'
import { applyAffine3x4, applyAffine3x4Linear, type V3 } from '../viewport/geoHelpers'

/** Minimal KeyboardEvent-shaped fake — onKey only reads `.key` and (for
 *  Tab) calls `.preventDefault()`. */
function keyEvent(key: string): KeyboardEvent {
  return { key, preventDefault: () => { /* no-op */ } } as unknown as KeyboardEvent
}

/** Applies a UvFrameComponents to a world point, mirroring the kernel's
 *  `UvFrame::apply` (`uv = s·p + u0, t·p + v0`). */
function applyFrame(f: UvFrameComponents, p: readonly [number, number, number]): [number, number] {
  return [
    f.sx * p[0] + f.sy * p[1] + f.sz * p[2] + f.u0,
    f.tx * p[0] + f.ty * p[1] + f.tz * p[2] + f.v0,
  ]
}

function makeSnap(): Snap {
  return { x: 0, y: 0, z: 0, kind: 'on-face' }
}

/** A discrete POINT snap (the kind a dragged pin must land on exactly). */
function pointSnapAt(x: number, y: number, z: number, kind = 'endpoint'): Snap {
  return { x, y, z, kind }
}

/** Ray straight down the +Z axis onto the ground-plane face at z=0 (the
 *  fixture face's plane), hitting world point (x, y, 0). */
function rayAt(x: number, y: number): Ray {
  return { origin: [x, y, 5], direction: [0, 0, -1] }
}

function makeFacePick(object: bigint, face: bigint, instance?: bigint) {
  return {
    object: () => object,
    face: () => face,
    instance: () => instance,
    free: vi.fn(),
  }
}

function makeFaceMaterial(face: bigint, objectDefault: bigint) {
  return {
    face: () => face,
    object_default: () => objectDefault,
    free: vi.fn(),
  }
}

const MATERIAL_SENTINEL = BigInt('18446744073709551615')

function makeMaterialInfo(hasTexture: boolean, worldW = 1, worldH = 1) {
  return {
    has_texture: () => hasTexture,
    world_w: () => worldW,
    world_h: () => worldH,
    free: vi.fn(),
  }
}

interface SceneOpts {
  facePick?: ReturnType<typeof makeFacePick> | undefined
  faceMaterial?: ReturnType<typeof makeFaceMaterial> | undefined
  materialInfo?: ReturnType<typeof makeMaterialInfo> | undefined
  /** `face_uv_frame` result: `undefined` (stale), `[]` (unset/planar
   *  default), or an 8-element array (explicit frame). */
  uvFrame?: number[] | undefined
  setFrameThrows?: string
  facePlane?: { anchor: readonly [number, number, number]; normal: readonly [number, number, number] }
  historyGeneration?: () => bigint
  instancePose?: number[]
}

const FACE_NORMAL: [number, number, number] = [0, 0, 1]
const FACE_ANCHOR: [number, number, number] = [0, 0, 0]

/** The session basis the tool uses — `tessellatePlaneBasis`, the ABSOLUTE
 *  rotation reference (the axes an untextured face's default projection
 *  aligns to). For the fixture normal: u = (0,-1,0), v = (1,0,0). */
const TESS = tessellatePlaneBasis(FACE_NORMAL)

/** Decomposes a committed/previewed frame into the user-facing absolute
 *  quantities, exactly as the tool derives them. */
function absOf(frame: UvFrameComponents, worldW = 1, worldH = 1) {
  const local = frameToLocal2D(frame, FACE_ANCHOR, TESS.u, TESS.v)
  return frameAbsolute(local, worldW, worldH)!
}

function makeWasmScene(opts: SceneOpts = {}): WasmScene {
  return {
    pick_face: vi.fn(() => opts.facePick),
    face_material: vi.fn(() => opts.faceMaterial),
    material_info: vi.fn(() => opts.materialInfo),
    face_plane: vi.fn(() => {
      const { anchor, normal } = opts.facePlane ?? { anchor: FACE_ANCHOR, normal: FACE_NORMAL }
      return new Float64Array([...anchor, ...normal])
    }),
    face_uv_frame: vi.fn(() => (opts.uvFrame === undefined ? undefined : new Float64Array(opts.uvFrame))),
    set_face_uv_frame: vi.fn(() => {
      if (opts.setFrameThrows !== undefined) throw new Error(opts.setFrameThrows)
    }),
    // Not used by the tile-corner tool; stubbed so the red-check against the
    // pre-rework implementation (which fetched the boundary for its
    // spawn-clamp) fails on BEHAVIOR, not on a missing mock.
    face_boundary: vi.fn(() => new Float32Array([])),
    history_generation: vi.fn(opts.historyGeneration ?? (() => 0n)),
    instance_pose: vi.fn(() => (opts.instancePose === undefined ? undefined : new Float64Array(opts.instancePose))),
  } as unknown as WasmScene
}

function makeTool(scene: WasmScene) {
  const gizmoGroup = new THREE.Group()
  const previewFaceUv = vi.fn()
  const onCommit = vi.fn()
  const onToast = vi.fn()
  const onMeasurement = vi.fn()
  const tool = new PositionTextureTool(scene, gizmoGroup, previewFaceUv, onCommit, onToast, onMeasurement)
  return { tool, gizmoGroup, previewFaceUv, onCommit, onToast, onMeasurement }
}

/** A textured-face fixture: object 3, face 4, its own material 9 is
 *  textured with a 1x1m tile, no explicit UvFrame (planar default). */
function texturedFixtureScene(overrides: SceneOpts = {}): WasmScene {
  return makeWasmScene({
    facePick: makeFacePick(3n, 4n),
    faceMaterial: makeFaceMaterial(9n, MATERIAL_SENTINEL),
    materialInfo: makeMaterialInfo(true, 1, 1),
    uvFrame: [],
    ...overrides,
  })
}

/** The last frame previewed. */
function lastPreviewFrame(previewFaceUv: ReturnType<typeof vi.fn>): UvFrameComponents {
  return previewFaceUv.mock.calls[previewFaceUv.mock.calls.length - 1][2] as UvFrameComponents
}

/** The single committed frame. */
function committedFrame(scene: WasmScene): UvFrameComponents {
  const calls = (scene.set_face_uv_frame as ReturnType<typeof vi.fn>).mock.calls
  const arr = calls[calls.length - 1][2] as Float64Array
  return {
    sx: arr[0], sy: arr[1], sz: arr[2],
    tx: arr[3], ty: arr[4], tz: arr[5],
    u0: arr[6], v0: arr[7],
  }
}

/** The three pin meshes, in build order: [0]=red, [1]=green, [2]=blue. */
function gizmoMeshes(gizmoGroup: THREE.Group): THREE.Mesh[] {
  return gizmoGroup.children.filter((c): c is THREE.Mesh => c instanceof THREE.Mesh)
}

/** Enters a session with a plain click (press + sub-threshold release), so
 *  the session is open, idle, with the seed frame untouched. */
function enterIdle(tool: PositionTextureTool, x = 0, y = 0): void {
  tool.onPointerDown(makeSnap(), rayAt(x, y))
  tool.onPointerUp(makeSnap(), rayAt(x, y))
}

describe('PositionTextureTool — entry gating', () => {
  it('a click on an untextured (solid-color) face toasts and does not enter positioning', () => {
    const scene = texturedFixtureScene({ materialInfo: makeMaterialInfo(false) })
    const { tool, onToast, previewFaceUv } = makeTool(scene)

    tool.onPointerDown(makeSnap(), rayAt(0, 0))

    expect(onToast).toHaveBeenCalledTimes(1)
    expect(previewFaceUv).not.toHaveBeenCalled()
  })

  it('a click on a wholly unpainted face toasts and does not enter positioning', () => {
    const scene = texturedFixtureScene({
      faceMaterial: makeFaceMaterial(MATERIAL_SENTINEL, MATERIAL_SENTINEL),
    })
    const { tool, onToast } = makeTool(scene)

    tool.onPointerDown(makeSnap(), rayAt(0, 0))

    expect(onToast).toHaveBeenCalledTimes(1)
  })

  it('a stale pick (pick_face misses) is a silent no-op', () => {
    const scene = makeWasmScene({ facePick: undefined })
    const { tool, onToast, previewFaceUv } = makeTool(scene)

    tool.onPointerDown(makeSnap(), rayAt(0, 0))

    expect(onToast).not.toHaveBeenCalled()
    expect(previewFaceUv).not.toHaveBeenCalled()
  })

  it('a face resolved through a component instance is refused with the enter-the-component hint when no context is entered', () => {
    const scene = texturedFixtureScene({ facePick: makeFacePick(3n, 4n, 99n) })
    const { tool, onToast } = makeTool(scene)

    tool.onPointerDown(makeSnap(), rayAt(0, 0))

    expect(onToast).toHaveBeenCalledTimes(1)
  })

  it('a click on a textured face enters positioning and starts an immediate translate drag (a live preview fires)', () => {
    const scene = texturedFixtureScene()
    const { tool, previewFaceUv, onToast } = makeTool(scene)

    tool.onPointerDown(makeSnap(), rayAt(0, 0))
    tool.onPointerMove(makeSnap(), rayAt(0.5, 0))

    expect(onToast).not.toHaveBeenCalled()
    expect(previewFaceUv).toHaveBeenCalled()
    const [obj, face] = previewFaceUv.mock.calls[previewFaceUv.mock.calls.length - 1]
    expect(obj).toBe(3n)
    expect(face).toBe(4n)
  })
})

describe('PositionTextureTool — pins spawn at the tile corners, ON the clicked face', () => {
  it('the RED pin spawns at the UV lattice corner nearest the click — near the face far from the origin, NOT at the world origin', () => {
    // A face 20-ish meters from the origin; the planar default maps
    // uv = (-y, x), so uv(0,0) sits at the WORLD ORIGIN — the exact defect
    // the playtest reported. The red pin must instead sit on the corner of
    // the tile that contains the click.
    const scene = texturedFixtureScene({ facePlane: { anchor: [10, 20, 0], normal: FACE_NORMAL } })
    const { tool, gizmoGroup } = makeTool(scene)

    // Click at (10.3, 19.6): uv there is (-19.6, 10.3), which floors to the
    // lattice point (-20, 10) — the world point (10, 20, 0).
    enterIdle(tool, 10.3, 19.6)

    const [pin, green, blue] = gizmoMeshes(gizmoGroup)
    expect(pin.position.x).toBeCloseTo(10)
    expect(pin.position.y).toBeCloseTo(20)
    expect(pin.position.z).toBeCloseTo(0)
    // Within one tile of the click, i.e. on/next to the face — nowhere near
    // the origin. (The stronger containment property is asserted by the
    // flooring test below; this bound only rules out the origin defect.)
    expect(Math.hypot(pin.position.x - 10.3, pin.position.y - 19.6)).toBeLessThan(Math.SQRT1_2 + 1e-9)

    // GREEN and BLUE are the ADJACENT TILE CORNERS: exactly one tile along
    // the texture's U and V axes — not radiating at screen-px distances.
    // uv(-19, 10) → (10, 19, 0); uv(-20, 11) → (11, 20, 0).
    expect(green.position.x).toBeCloseTo(10)
    expect(green.position.y).toBeCloseTo(19)
    expect(blue.position.x).toBeCloseTo(11)
    expect(blue.position.y).toBeCloseTo(20)
  })

  it('pin placement FLOORS to the tile CONTAINING the click, so the drawn tile is never a full tile off it', () => {
    // This test previously asserted NEAREST-corner rounding, and argued for it
    // on the grounds that the pin then lands within half a tile of the click.
    // That optimizes the wrong quantity. The pin does not stand alone:
    // `_cornersLocal` spans the tile [m, m+1] x [n, n+1] from it, and THAT
    // rectangle is what the user sees and drags. Rounding keeps the pin near
    // the click while putting the rectangle on the far side of it — so with a
    // single texture repeat across a face, the whole gizmo floated clean OFF
    // the face, sharing only its far edge, and every rotation then pivoted
    // about that off-face corner. Playtesting reported exactly that.
    //
    // Click at (10.7, 19.4): uv there is (-19.4, 10.7). Flooring gives
    // (-20, 10), whose tile spans u in [-20, -19] — and -19.4 lies inside
    // that span, so the click is INSIDE the drawn tile. Rounding gives
    // (-19, 11), whose tile spans u in [-19, -18]: the click falls outside
    // it. The DRAWN TILE is displaced by a full tile in u (from [-20, -19]
    // to [-19, -18]) — the click itself lies 0.4 past that tile's near edge,
    // which is what makes the rectangle miss the clicked spot entirely.
    const scene = texturedFixtureScene({ facePlane: { anchor: [10, 20, 0], normal: FACE_NORMAL } })
    const { tool, gizmoGroup } = makeTool(scene)

    enterIdle(tool, 10.7, 19.4)

    const [pin, green, blue] = gizmoMeshes(gizmoGroup)
    expect(pin.position.x).toBeCloseTo(10)
    expect(pin.position.y).toBeCloseTo(20)
    // The contract that actually matters, checked as genuine CONTAINMENT in
    // the tile's own (u, v) basis rather than as a distance bound — it was a
    // distance bound that let the off-by-a-tile placement look acceptable.
    const eu = { x: green.position.x - pin.position.x, y: green.position.y - pin.position.y }
    const ev = { x: blue.position.x - pin.position.x, y: blue.position.y - pin.position.y }
    const d = { x: 10.7 - pin.position.x, y: 19.4 - pin.position.y }
    const det = eu.x * ev.y - eu.y * ev.x
    const a = (d.x * ev.y - d.y * ev.x) / det
    const b = (eu.x * d.y - eu.y * d.x) / det
    expect(a).toBeGreaterThanOrEqual(-1e-9)
    expect(a).toBeLessThanOrEqual(1 + 1e-9)
    expect(b).toBeGreaterThanOrEqual(-1e-9)
    expect(b).toBeLessThanOrEqual(1 + 1e-9)
    // The adjacent corners follow the pin: uv(-19, 10) → (10, 19, 0);
    // uv(-20, 11) → (11, 20, 0).
    expect(green.position.x).toBeCloseTo(10)
    expect(green.position.y).toBeCloseTo(19)
    expect(blue.position.x).toBeCloseTo(11)
    expect(blue.position.y).toBeCloseTo(20)
  })

  it('the corner pins sit one MATERIAL TILE apart — a 2m tile puts green exactly 2m from red', () => {
    const scene = texturedFixtureScene({ materialInfo: makeMaterialInfo(true, 2, 2) })
    const { tool, gizmoGroup } = makeTool(scene)

    enterIdle(tool, 0, 0)

    const [pin, green, blue] = gizmoMeshes(gizmoGroup)
    const dGreen = pin.position.distanceTo(green.position)
    const dBlue = pin.position.distanceTo(blue.position)
    expect(dGreen).toBeCloseTo(2)
    expect(dBlue).toBeCloseTo(2)
  })

  it('the tile outline connects the three pins and the far corner; the DASHED reference rectangle marks the natural 1x tile even when the texture is scaled 2x', () => {
    // Explicit frame: the planar default scaled 2x (tile edges 2m on a
    // 1m-natural material) — s = u/2, t = v/2 in the tessellate basis.
    const scene = texturedFixtureScene({ uvFrame: [0, -0.5, 0, 0.5, 0, 0, 0, 0] })
    const { tool, gizmoGroup } = makeTool(scene)

    enterIdle(tool, 0, 0)

    const loops = gizmoGroup.children.filter((c): c is THREE.LineLoop => c instanceof THREE.LineLoop)
    expect(loops.length).toBe(2)
    const [tile, ref] = loops

    const [pin, green] = gizmoMeshes(gizmoGroup)
    expect(pin.position.x).toBeCloseTo(0)
    expect(pin.position.y).toBeCloseTo(0)
    // The 2x tile's green corner: uv(1,0) → -y/2 = 1 → (0, -2, 0).
    expect(green.position.x).toBeCloseTo(0)
    expect(green.position.y).toBeCloseTo(-2)

    // Tile outline vertex 1 is the green corner (2m out)...
    const tileAttr = tile.geometry.getAttribute('position') as THREE.BufferAttribute
    expect(tileAttr.getX(1)).toBeCloseTo(0)
    expect(tileAttr.getY(1)).toBeCloseTo(-2)

    // ...while the DASHED reference's corresponding vertex sits at the
    // NATURAL tile size (1m) along the same direction — the visible "this
    // is where 1x is" landmark.
    const refAttr = ref.geometry.getAttribute('position') as THREE.BufferAttribute
    expect(refAttr.getX(0)).toBeCloseTo(0)
    expect(refAttr.getY(0)).toBeCloseTo(0)
    expect(refAttr.getX(1)).toBeCloseTo(0)
    expect(refAttr.getY(1)).toBeCloseTo(-1)
    expect((ref.material as THREE.LineDashedMaterial).isLineDashedMaterial).toBe(true)
  })
})

describe('PositionTextureTool — translate drag', () => {
  it('dragging translates u0/v0 by the world delta projected through s/t, and commits it as ONE kernel call on Enter', () => {
    const scene = texturedFixtureScene()
    const { tool, previewFaceUv, onCommit } = makeTool(scene)

    tool.onPointerDown(makeSnap(), rayAt(0, 0)) // enter + begin translate at (0,0)
    tool.onPointerMove(makeSnap(), rayAt(2, 0)) // drag to (2,0) -> delta (2,0) in world
    tool.onPointerUp(makeSnap(), rayAt(2, 0))

    const lastPreview = lastPreviewFrame(previewFaceUv)
    const planar = planarDefaultFrame(FACE_NORMAL, 1, 1)
    // Drag right by world (2,0,0): new u0 = u0 - s . (2,0,0), new v0 similarly.
    expect(lastPreview.u0).toBeCloseTo(planar.u0 - (planar.sx * 2))
    expect(lastPreview.v0).toBeCloseTo(planar.v0 - (planar.tx * 2))
    // M (s/t gradients) must be untouched by a pure translate.
    expect(lastPreview.sx).toBeCloseTo(planar.sx)
    expect(lastPreview.ty).toBeCloseTo(planar.ty)

    ;(scene.set_face_uv_frame as ReturnType<typeof vi.fn>).mockClear()
    tool.onKey(keyEvent('Enter'))

    expect(scene.set_face_uv_frame).toHaveBeenCalledTimes(1)
    const [obj, face, arr] = (scene.set_face_uv_frame as ReturnType<typeof vi.fn>).mock.calls[0]
    expect(obj).toBe(3n)
    expect(face).toBe(4n)
    expect(Array.from(arr as Float64Array)).toEqual(frameToArray(lastPreview).map((n) => expect.closeTo(n, 5)))
    expect(onCommit).toHaveBeenCalledWith(3n)
  })

  it('a session with no actual drag commits nothing on Enter (no spurious undo entry)', () => {
    const scene = texturedFixtureScene()
    const { tool, onCommit } = makeTool(scene)

    tool.onPointerDown(makeSnap(), rayAt(0, 0))
    tool.onKey(keyEvent('Enter'))

    expect(scene.set_face_uv_frame).not.toHaveBeenCalled()
    expect(onCommit).not.toHaveBeenCalled()
  })

  it('a click away from the face (after releasing the first drag) commits and exits', () => {
    const scene = texturedFixtureScene()
    const { tool, onCommit } = makeTool(scene)

    tool.onPointerDown(makeSnap(), rayAt(0, 0))
    tool.onPointerMove(makeSnap(), rayAt(1, 0))
    tool.onPointerUp(makeSnap(), rayAt(1, 0))

    // Click away: pick_face now misses (nothing under the cursor).
    ;(scene.pick_face as ReturnType<typeof vi.fn>).mockReturnValueOnce(undefined)
    tool.onPointerDown(makeSnap(), rayAt(50, 50))

    expect(scene.set_face_uv_frame).toHaveBeenCalledTimes(1)
    expect(onCommit).toHaveBeenCalledWith(3n)

    // The session has ended: a further Enter is a no-op.
    ;(scene.set_face_uv_frame as ReturnType<typeof vi.fn>).mockClear()
    tool.onKey(keyEvent('Enter'))
    expect(scene.set_face_uv_frame).not.toHaveBeenCalled()
  })

  it('a refused set_face_uv_frame reports through onToast, not a thrown exception', () => {
    const scene = texturedFixtureScene({ setFrameThrows: 'UnknownFace: face is not in the object' })
    const { tool, onToast } = makeTool(scene)

    tool.onPointerDown(makeSnap(), rayAt(0, 0))
    tool.onPointerMove(makeSnap(), rayAt(1, 0))
    tool.onPointerUp(makeSnap(), rayAt(1, 0))

    expect(() => tool.onKey(keyEvent('Enter'))).not.toThrow()
    expect(onToast).toHaveBeenCalledTimes(1)
  })
})

describe('PositionTextureTool — the RED pin lands where you point (and snaps take)', () => {
  it('grabbing the red pin and dragging puts the PIN at the cursor point — not offset by where within the pick tolerance the press landed', () => {
    const scene = texturedFixtureScene()
    const { tool, previewFaceUv } = makeTool(scene)
    enterIdle(tool) // pin at (0, 0, 0)

    // Press within the pin's pick tolerance but clearly OFF its center...
    tool.onPointerDown(makeSnap(), rayAt(0.05, 0.03))
    // ...and drag to a specific point.
    tool.onPointerMove(makeSnap(), rayAt(3.7, 1.2))

    // The PIN's lattice UV (0,0) now maps to the CURSOR point exactly: the
    // pin followed the cursor, so it can be placed at a specific location.
    const uv = applyFrame(lastPreviewFrame(previewFaceUv), [3.7, 1.2, 0])
    expect(uv[0]).toBeCloseTo(0, 6)
    expect(uv[1]).toBeCloseTo(0, 6)
  })

  it('an ENDPOINT snap during a red-pin drag actually TAKES: the pin lands on the snapped point, not the raw ray hit', () => {
    const scene = texturedFixtureScene()
    const { tool, previewFaceUv } = makeTool(scene)
    enterIdle(tool)

    tool.onPointerDown(makeSnap(), rayAt(0, 0)) // grab the pin at its center
    // The ray points at (2, 0) but the snap engine reports an endpoint at
    // (2.6, -1.4, 0) — the pin must sit on the ENDPOINT.
    tool.onPointerMove(pointSnapAt(2.6, -1.4, 0), rayAt(2, 0))

    const uvAtSnap = applyFrame(lastPreviewFrame(previewFaceUv), [2.6, -1.4, 0])
    expect(uvAtSnap[0]).toBeCloseTo(0, 6)
    expect(uvAtSnap[1]).toBeCloseTo(0, 6)

    // And it commits that way.
    tool.onPointerUp(pointSnapAt(2.6, -1.4, 0), rayAt(2, 0))
    tool.onKey(keyEvent('Enter'))
    const committed = committedFrame(scene)
    const uvCommitted = applyFrame(committed, [2.6, -1.4, 0])
    expect(uvCommitted[0]).toBeCloseTo(0, 6)
    expect(uvCommitted[1]).toBeCloseTo(0, 6)
  })

  it('a broad-area snap kind (on-face) does NOT hijack the drag — the raw ray hit is used', () => {
    const scene = texturedFixtureScene()
    const { tool, previewFaceUv } = makeTool(scene)
    enterIdle(tool)

    tool.onPointerDown(makeSnap(), rayAt(0, 0))
    tool.onPointerMove({ x: 9, y: 9, z: 0, kind: 'on-face' }, rayAt(2, 0))

    const uv = applyFrame(lastPreviewFrame(previewFaceUv), [2, 0, 0])
    expect(uv[0]).toBeCloseTo(0, 6)
    expect(uv[1]).toBeCloseTo(0, 6)
  })

  it('snapConstraint pins the snap engine to the face plane while a session is open, and is null otherwise', () => {
    const scene = texturedFixtureScene()
    const { tool } = makeTool(scene)

    expect(tool.snapConstraint()).toBeNull()

    enterIdle(tool)
    const constraint = tool.snapConstraint()!
    expect(constraint.constraintPlane).toBeDefined()
    expect(constraint.constraintPlane!.point).toEqual([0, 0, 0])
    expect(constraint.constraintPlane!.normal[2]).toBeCloseTo(1)
  })
})

describe('PositionTextureTool — green corner drag (scale/rotate about the red pin)', () => {
  it('the dragged corner lands EXACTLY at the cursor (it maps to the adjacent lattice UV) and the red pin holds still', () => {
    const scene = texturedFixtureScene()
    const { tool, previewFaceUv, gizmoGroup } = makeTool(scene)
    enterIdle(tool) // pin (0,0,0); green corner (0,-1,0); blue (1,0,0)

    tool.onPointerDown(makeSnap(), rayAt(0, -1)) // grab green at its corner
    tool.onPointerMove(makeSnap(), rayAt(0.9, -1.3))

    const frame = lastPreviewFrame(previewFaceUv)
    const uvAtCursor = applyFrame(frame, [0.9, -1.3, 0])
    expect(uvAtCursor[0]).toBeCloseTo(1, 6)
    expect(uvAtCursor[1]).toBeCloseTo(0, 6)
    const uvAtPin = applyFrame(frame, [0, 0, 0])
    expect(uvAtPin[0]).toBeCloseTo(0, 6)
    expect(uvAtPin[1]).toBeCloseTo(0, 6)

    // The rendered green pin tracks the cursor exactly — no arbitrary
    // radiating point.
    const green = gizmoMeshes(gizmoGroup)[1]
    expect(green.position.x).toBeCloseTo(0.9)
    expect(green.position.y).toBeCloseTo(-1.3)
  })

  it('a point snap takes on the green corner too', () => {
    const scene = texturedFixtureScene()
    const { tool, previewFaceUv } = makeTool(scene)
    enterIdle(tool)

    tool.onPointerDown(makeSnap(), rayAt(0, -1))
    tool.onPointerMove(pointSnapAt(1.5, -0.5, 0, 'midpoint'), rayAt(1.4, -0.6))

    const uv = applyFrame(lastPreviewFrame(previewFaceUv), [1.5, -0.5, 0])
    expect(uv[0]).toBeCloseTo(1, 6)
    expect(uv[1]).toBeCloseTo(0, 6)
  })
})

describe('PositionTextureTool — blue corner drag (shear about the red pin)', () => {
  it('the dragged corner maps to (m, n+1); red AND green hold still', () => {
    const scene = texturedFixtureScene()
    const { tool, previewFaceUv } = makeTool(scene)
    enterIdle(tool) // blue corner at (1, 0, 0)

    tool.onPointerDown(makeSnap(), rayAt(1, 0)) // grab blue
    tool.onPointerMove(makeSnap(), rayAt(1.2, 0.5))

    const frame = lastPreviewFrame(previewFaceUv)
    const uvAtCursor = applyFrame(frame, [1.2, 0.5, 0])
    expect(uvAtCursor[0]).toBeCloseTo(0, 6)
    expect(uvAtCursor[1]).toBeCloseTo(1, 6)
    // Red and green fixed:
    const uvAtPin = applyFrame(frame, [0, 0, 0])
    expect(uvAtPin[0]).toBeCloseTo(0, 6)
    expect(uvAtPin[1]).toBeCloseTo(0, 6)
    const uvAtGreen = applyFrame(frame, [0, -1, 0])
    expect(uvAtGreen[0]).toBeCloseTo(1, 6)
    expect(uvAtGreen[1]).toBeCloseTo(0, 6)
  })

  it('a degenerate blue drag (V edge collapsed onto the pin) is refused: no preview, no dirty commit', () => {
    const scene = texturedFixtureScene()
    const { tool, previewFaceUv } = makeTool(scene)
    enterIdle(tool)

    tool.onPointerDown(makeSnap(), rayAt(1, 0))
    previewFaceUv.mockClear()
    tool.onPointerMove(makeSnap(), rayAt(0, 0)) // drag blue onto the pin

    expect(previewFaceUv).not.toHaveBeenCalled()
    tool.onPointerUp(makeSnap(), rayAt(0, 0))
    tool.onKey(keyEvent('Enter'))
    expect(scene.set_face_uv_frame).not.toHaveBeenCalled()
  })

  it('a blue drag landing near-PARALLEL to the U edge is refused the same way', () => {
    const scene = texturedFixtureScene()
    const { tool, previewFaceUv } = makeTool(scene)
    enterIdle(tool)

    tool.onPointerDown(makeSnap(), rayAt(1, 0))
    previewFaceUv.mockClear()
    // The U edge points along (0,-1,0); drag blue to a point on that same
    // line through the pin.
    tool.onPointerMove(makeSnap(), rayAt(0, 2.5))

    expect(previewFaceUv).not.toHaveBeenCalled()
  })
})

describe('PositionTextureTool — ABSOLUTE rotation and scale (the relative-regression canary)', () => {
  // An explicit frame already rotated 30 deg from the face's own axes, at
  // scale 1: E_u = cos30·û + sin30·v̂ in the tessellate basis (û=(0,-1,0),
  // v̂=(1,0,0)).
  const COS30 = Math.cos(Math.PI / 6)
  const SIN30 = Math.sin(Math.PI / 6)
  const ROT30_FRAME = [SIN30, -COS30, 0, COS30, SIN30, 0, 0, 0]

  it('typing 45 + Enter with NO drag puts the texture at ABSOLUTE 45 deg — not 45 deg past the arbitrary 30 deg it started at', () => {
    const scene = texturedFixtureScene({ uvFrame: ROT30_FRAME })
    const { tool, previewFaceUv } = makeTool(scene)
    enterIdle(tool)

    // Sanity: the seed really is at 30 deg absolute.
    expect(absOf({
      sx: ROT30_FRAME[0], sy: ROT30_FRAME[1], sz: ROT30_FRAME[2],
      tx: ROT30_FRAME[3], ty: ROT30_FRAME[4], tz: ROT30_FRAME[5],
      u0: 0, v0: 0,
    }).angle).toBeCloseTo(Math.PI / 6)

    tool.onKey(keyEvent('4'))
    tool.onKey(keyEvent('5'))
    tool.onKey(keyEvent('Enter'))

    const abs = absOf(lastPreviewFrame(previewFaceUv))
    // THE canary: 45 means 45 from the face's own axes. If rotation ever
    // becomes relative to the starting frame again, this reads 75 and fails.
    expect((abs.angle * 180) / Math.PI).toBeCloseTo(45, 6)
    expect(abs.scaleU).toBeCloseTo(1, 6)

    // A second Enter commits it.
    tool.onKey(keyEvent('Enter'))
    expect(scene.set_face_uv_frame).toHaveBeenCalledTimes(1)
    expect((absOf(committedFrame(scene)).angle * 180) / Math.PI).toBeCloseTo(45, 6)
  })

  it('typing 1x + Enter on a 2x-scaled texture returns it to natural size — scale is ABSOLUTE, measured against the material tile', () => {
    const scene = texturedFixtureScene({ uvFrame: [0, -0.5, 0, 0.5, 0, 0, 0, 0] }) // planar default at 2x
    const { tool } = makeTool(scene)
    enterIdle(tool)

    tool.onKey(keyEvent('1'))
    tool.onKey(keyEvent('x'))
    tool.onKey(keyEvent('Enter'))
    tool.onKey(keyEvent('Enter'))

    const abs = absOf(committedFrame(scene))
    expect(abs.scaleU).toBeCloseTo(1, 6)
    expect(abs.scaleV).toBeCloseTo(1, 6)
    expect((abs.angle * 180) / Math.PI).toBeCloseTo(0, 6)
  })

  it('typing an absolute angle WHILE dragging green keeps the drag\'s live scale and sets the exact angle', () => {
    const scene = texturedFixtureScene()
    const { tool } = makeTool(scene)
    enterIdle(tool)

    tool.onPointerDown(makeSnap(), rayAt(0, -1)) // grab green
    tool.onPointerMove(makeSnap(), rayAt(0, -2)) // drag straight out: scale 2, angle 0

    tool.onKey(keyEvent('4'))
    tool.onKey(keyEvent('5'))
    tool.onKey(keyEvent('Enter'))
    tool.onKey(keyEvent('Enter'))

    const abs = absOf(committedFrame(scene))
    expect((abs.angle * 180) / Math.PI).toBeCloseTo(45, 6)
    expect(abs.scaleU).toBeCloseTo(2, 6)
  })

  it('Tab flips a bare number to a SCALE factor', () => {
    const scene = texturedFixtureScene()
    const { tool } = makeTool(scene)
    enterIdle(tool)

    tool.onKey(keyEvent('Tab'))
    tool.onKey(keyEvent('2'))
    tool.onKey(keyEvent('Enter'))
    tool.onKey(keyEvent('Enter'))

    const abs = absOf(committedFrame(scene))
    expect(abs.scaleU).toBeCloseTo(2, 6)
    expect((abs.angle * 180) / Math.PI).toBeCloseTo(0, 6)
  })

  it('a non-positive typed scale factor is refused: no preview, session still open', () => {
    const scene = texturedFixtureScene()
    const { tool, previewFaceUv, onToast } = makeTool(scene)
    enterIdle(tool)
    previewFaceUv.mockClear()

    tool.onKey(keyEvent('0'))
    tool.onKey(keyEvent('x'))
    tool.onKey(keyEvent('Enter'))

    expect(previewFaceUv).not.toHaveBeenCalled()
    expect(tool.capturingInput()).toBe(true) // session survived
    // "Survived the first refusal" is not enough — a refusal that failed
    // to clear the buffer would ALSO leave the session open, while wedging
    // Enter forever. The refusal must have said why, and the next Enter
    // must still reach the session-level close.
    expect(onToast).toHaveBeenCalledTimes(1)
    expect(onToast.mock.calls[0][0]).toMatch(/scale/i)
    tool.onKey(keyEvent('Enter'))
    expect(tool.capturingInput()).toBe(false)
  })

  it('the idle measurement readout shows the CURRENT absolute state from the moment the session opens', () => {
    const scene = texturedFixtureScene({ uvFrame: [0, -0.5, 0, 0.5, 0, 0, 0, 0] }) // 2x
    const { tool, onMeasurement } = makeTool(scene)
    enterIdle(tool)

    const last = onMeasurement.mock.calls[onMeasurement.mock.calls.length - 1][0] as string
    expect(last).toContain('×2.00')
    expect(last).toContain('0.0°')
  })
})

describe('PositionTextureTool — typed entry without a drag (blue target via a plain click)', () => {
  it('typing works whenever the session is open: capturingInput is TRUE between grabs, and capturesKey narrows to buffer keys', () => {
    const scene = texturedFixtureScene()
    const { tool } = makeTool(scene)

    expect(tool.capturingInput()).toBe(false)
    expect(tool.capturesKey('5')).toBe(false)

    enterIdle(tool) // session open, NO grab

    expect(tool.capturingInput()).toBe(true)
    expect(tool.capturesKey('5')).toBe(true)
    expect(tool.capturesKey('x')).toBe(true)
    expect(tool.capturesKey('.')).toBe(true)
    expect(tool.capturesKey('Enter')).toBe(true)
    expect(tool.capturesKey('Tab')).toBe(true)
    // Letters that aren't buffer tokens still fall through (tool switching).
    expect(tool.capturesKey('g')).toBe(false)
    expect(tool.capturesKey(' ')).toBe(false)
  })

  it('a plain CLICK on the blue pin (no drag) retargets typed entry: 2x sets the absolute V-scale, leaving U untouched', () => {
    const scene = texturedFixtureScene()
    const { tool } = makeTool(scene)
    enterIdle(tool)

    // Plain click on blue at (1, 0, 0).
    tool.onPointerDown(makeSnap(), rayAt(1, 0))
    tool.onPointerUp(makeSnap(), rayAt(1, 0))

    tool.onKey(keyEvent('2'))
    tool.onKey(keyEvent('x'))
    tool.onKey(keyEvent('Enter'))
    tool.onKey(keyEvent('Enter'))

    const committed = committedFrame(scene)
    const abs = absOf(committed)
    expect(abs.scaleV).toBeCloseTo(2, 6)
    expect(abs.scaleU).toBeCloseTo(1, 6)
    expect(abs.skew).toBeCloseTo(0, 6)
    // Green corner untouched:
    const uvAtGreen = applyFrame(committed, [0, -1, 0])
    expect(uvAtGreen[0]).toBeCloseTo(1, 6)
    expect(uvAtGreen[1]).toBeCloseTo(0, 6)
  })

  it('after a blue click, a bare number is the absolute SKEW in degrees (0 = rectangular)', () => {
    const scene = texturedFixtureScene()
    const { tool } = makeTool(scene)
    enterIdle(tool)

    tool.onPointerDown(makeSnap(), rayAt(1, 0))
    tool.onPointerUp(makeSnap(), rayAt(1, 0))

    tool.onKey(keyEvent('1'))
    tool.onKey(keyEvent('0'))
    tool.onKey(keyEvent('Enter'))
    tool.onKey(keyEvent('Enter'))

    const abs = absOf(committedFrame(scene))
    expect((abs.skew * 180) / Math.PI).toBeCloseTo(10, 6)
    expect(abs.scaleV).toBeCloseTo(1, 6)
    expect((abs.angle * 180) / Math.PI).toBeCloseTo(0, 6) // U edge untouched
  })
})

describe('PositionTextureTool — typed length on the red (translate) pin', () => {
  it('typed length + Enter commits an EXACT move along the established drag direction', () => {
    const scene = texturedFixtureScene()
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap(), rayAt(0, 0)) // enter + begin translate
    tool.onPointerMove(makeSnap(), rayAt(2, 0)) // establishes direction +x, 2m

    tool.onKey(keyEvent('3'))
    tool.onKey(keyEvent('Enter'))
    tool.onKey(keyEvent('Enter'))

    // Moved exactly 3m along +x: the point (3, 0, 0) now maps to the UV the
    // origin had.
    const committed = committedFrame(scene)
    const uv = applyFrame(committed, [3, 0, 0])
    expect(uv[0]).toBeCloseTo(0, 6)
    expect(uv[1]).toBeCloseTo(0, 6)
  })

  it('a typed length with NO established drag direction commits nothing (nothing to move along) — and the refusal leaves the tool able to close', () => {
    const scene = texturedFixtureScene()
    const { tool, onToast } = makeTool(scene)

    tool.onPointerDown(makeSnap(), rayAt(0, 0)) // grab, never move
    tool.onKey(keyEvent('2'))
    tool.onKey(keyEvent('Enter'))
    tool.onKey(keyEvent('Enter'))

    expect(scene.set_face_uv_frame).not.toHaveBeenCalled()
    // "Not committed" alone cannot tell correct refusal from a wedged
    // session (a wedge also never commits — exactly how the original bug
    // hid behind this test's name). The refusal must have SAID why and the
    // second Enter must have actually closed the session.
    expect(onToast).toHaveBeenCalledTimes(1)
    expect(onToast.mock.calls[0][0]).toMatch(/direction/i)
    expect(tool.capturingInput()).toBe(false)
  })

  it('Escape clears the typed buffer FIRST (grab and session stay open) — a second Escape cancels the whole session', () => {
    const scene = texturedFixtureScene()
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap(), rayAt(0, 0))
    tool.onPointerMove(makeSnap(), rayAt(2, 0))
    tool.onKey(keyEvent('3'))

    tool.onKey(keyEvent('Escape'))
    expect(tool.capturingInput()).toBe(true) // session survived the first Escape

    tool.onKey(keyEvent('Escape'))
    expect(tool.capturingInput()).toBe(false)
    expect(scene.set_face_uv_frame).not.toHaveBeenCalled()
  })
})

describe('PositionTextureTool — a refused typed value can NEVER wedge Enter (every refusal clears the buffer and says why)', () => {
  it('press-and-hold + typed length with no drag direction: the refusal toasts, clears the buffer, and the NEXT Enter still closes the session', () => {
    const scene = texturedFixtureScene()
    const { tool, onToast } = makeTool(scene)

    // The class doc's own anticipated gesture: press-and-hold opens the
    // session with an immediate translate grab; type without ever moving.
    tool.onPointerDown(makeSnap(), rayAt(0, 0))
    tool.onKey(keyEvent('2'))
    tool.onKey(keyEvent('Enter')) // refused: no direction to move along

    expect(onToast).toHaveBeenCalledTimes(1)
    expect(onToast.mock.calls[0][0]).toMatch(/direction/i)

    // THE wedge this suite guards against: the buffer must be cleared by
    // the refusal, so the very next Enter reaches the session-level
    // commit/close instead of re-entering the same dead branch forever.
    tool.onKey(keyEvent('Enter'))
    expect(tool.capturingInput()).toBe(false) // session closed
    expect(scene.set_face_uv_frame).not.toHaveBeenCalled() // nothing was dirty
  })

  it('the refusal does not discard EARLIER real drag progress in the same session — the next Enter commits it', () => {
    const scene = texturedFixtureScene()
    const { tool, onToast } = makeTool(scene)

    // A real drag first...
    tool.onPointerDown(makeSnap(), rayAt(0, 0))
    tool.onPointerMove(makeSnap(), rayAt(2, 0))
    tool.onPointerUp(makeSnap(), rayAt(2, 0))
    // ...then grab the red pin, hold still, and type a length.
    tool.onPointerDown(makeSnap(), rayAt(2, 0))
    tool.onKey(keyEvent('3'))
    tool.onKey(keyEvent('Enter')) // refused: this grab has no direction yet

    expect(onToast).toHaveBeenCalledTimes(1)

    tool.onKey(keyEvent('Enter')) // must now commit the whole session
    expect(scene.set_face_uv_frame).toHaveBeenCalledTimes(1)
    const committed = committedFrame(scene)
    const planar = planarDefaultFrame(FACE_NORMAL, 1, 1)
    expect(committed.v0).toBeCloseTo(planar.v0 - planar.tx * 2) // the earlier 2m drag survived
  })

  it('a non-positive typed scale factor toasts, clears, and the next Enter closes the session', () => {
    const scene = texturedFixtureScene()
    const { tool, previewFaceUv, onToast } = makeTool(scene)
    enterIdle(tool)
    previewFaceUv.mockClear()

    tool.onKey(keyEvent('0'))
    tool.onKey(keyEvent('x'))
    tool.onKey(keyEvent('Enter'))

    expect(previewFaceUv).not.toHaveBeenCalled()
    expect(onToast).toHaveBeenCalledTimes(1)
    expect(onToast.mock.calls[0][0]).toMatch(/scale/i)
    expect(tool.capturingInput()).toBe(true) // session survived the refusal...

    tool.onKey(keyEvent('Enter')) // ...and Enter is NOT wedged
    expect(tool.capturingInput()).toBe(false)
  })

  it('an unparseable buffer (a lone minus sign) toasts, clears, and does not wedge', () => {
    const scene = texturedFixtureScene()
    const { tool, onToast } = makeTool(scene)
    enterIdle(tool)

    tool.onKey(keyEvent('-'))
    tool.onKey(keyEvent('Enter'))

    expect(onToast).toHaveBeenCalledTimes(1)

    tool.onKey(keyEvent('Enter'))
    expect(tool.capturingInput()).toBe(false)
  })

  it('a typed value whose RESULT would be degenerate (blue skew 90 = V edge parallel to U) toasts, clears, and does not wedge', () => {
    const scene = texturedFixtureScene()
    const { tool, previewFaceUv, onToast } = makeTool(scene)
    enterIdle(tool)

    // Target blue via a plain click, then ask for a 90-degree skew — the V
    // edge would land exactly parallel to the U edge, an unrecoverable
    // basis the gesture math refuses.
    tool.onPointerDown(makeSnap(), rayAt(1, 0))
    tool.onPointerUp(makeSnap(), rayAt(1, 0))
    previewFaceUv.mockClear()

    tool.onKey(keyEvent('9'))
    tool.onKey(keyEvent('0'))
    tool.onKey(keyEvent('Enter'))

    expect(previewFaceUv).not.toHaveBeenCalled()
    expect(onToast).toHaveBeenCalledTimes(1)

    tool.onKey(keyEvent('Enter'))
    expect(tool.capturingInput()).toBe(false)
    expect(scene.set_face_uv_frame).not.toHaveBeenCalled()
  })

  it('refusals fire ONLY on Enter — never on intermediate keystrokes: a bare 2 before the x, a lone minus, a lone dot toast nothing', () => {
    const scene = texturedFixtureScene()
    const { tool, onToast } = makeTool(scene)
    enterIdle(tool)

    // A momentarily-incomplete buffer at every stage of typing "2x", then
    // "-", then "." — none of these is the user asking for anything yet.
    tool.onKey(keyEvent('2'))
    tool.onKey(keyEvent('x'))
    tool.onKey(keyEvent('Backspace'))
    tool.onKey(keyEvent('Backspace'))
    tool.onKey(keyEvent('-'))
    tool.onKey(keyEvent('Backspace'))
    tool.onKey(keyEvent('.'))
    tool.onKey(keyEvent('Backspace'))
    expect(onToast).not.toHaveBeenCalled()

    // Positive control: completing the value and pressing Enter applies it
    // silently (no refusal on a valid value either).
    tool.onKey(keyEvent('2'))
    tool.onKey(keyEvent('x'))
    tool.onKey(keyEvent('Enter'))
    expect(onToast).not.toHaveBeenCalled()
    tool.onKey(keyEvent('Enter')) // session-level commit
    expect(absOf(committedFrame(scene)).scaleU).toBeCloseTo(2, 6)
  })

  it('a held (autorepeating) Enter on a refused buffer toasts exactly ONCE, then closes the session — never a toast storm, never a wedge', () => {
    const scene = texturedFixtureScene()
    const { tool, onToast } = makeTool(scene)
    enterIdle(tool)

    tool.onKey(keyEvent('-')) // an unparseable buffer
    // Key autorepeat delivers a burst of Enters.
    tool.onKey(keyEvent('Enter')) // refusal: toast + clear
    tool.onKey(keyEvent('Enter')) // buffer now empty: session-level close
    tool.onKey(keyEvent('Enter')) // session gone: no-op
    tool.onKey(keyEvent('Enter')) // still a no-op

    expect(onToast).toHaveBeenCalledTimes(1)
    expect(tool.capturingInput()).toBe(false)
    expect(scene.set_face_uv_frame).not.toHaveBeenCalled() // nothing was dirty
  })
})

describe('PositionTextureTool — the VCB press-ray replay cannot pollute a typed commit (the threshold gate)', () => {
  it('replaying the OFF-CENTER press ray after every keystroke does not bake hidden noise into a typed-degrees commit', () => {
    const scene = texturedFixtureScene()
    const { tool } = makeTool(scene)
    enterIdle(tool)

    // Press within the green corner's tolerance but off its center — the
    // Viewport will replay THIS ray after every keystroke.
    const pressRay = rayAt(0.03, -0.98)
    tool.onPointerDown(makeSnap(), pressRay)
    // Keystroke replays: same ray, zero travel — must write nothing.
    tool.onPointerMove(makeSnap(), pressRay)
    tool.onKey(keyEvent('4'))
    tool.onPointerMove(makeSnap(), pressRay)
    tool.onKey(keyEvent('5'))
    tool.onPointerMove(makeSnap(), pressRay)
    tool.onKey(keyEvent('Enter'))
    tool.onKey(keyEvent('Enter'))

    const abs = absOf(committedFrame(scene))
    expect((abs.angle * 180) / Math.PI).toBeCloseTo(45, 9)
    // If the press offset had leaked through as a phantom drag, the scale
    // would be |pressPoint - pin| ≈ 0.98, not exactly 1.
    expect(abs.scaleU).toBeCloseTo(1, 9)
  })

  it('the same replay on a red-pin grab cannot establish a phantom direction for a typed length', () => {
    const scene = texturedFixtureScene()
    const { tool, onToast } = makeTool(scene)
    enterIdle(tool)

    const pressRay = rayAt(0.02, 0.01) // within the pin's tolerance, off-center
    tool.onPointerDown(makeSnap(), pressRay)
    tool.onPointerMove(makeSnap(), pressRay)
    tool.onKey(keyEvent('2'))
    tool.onPointerMove(makeSnap(), pressRay)
    tool.onKey(keyEvent('Enter'))
    tool.onKey(keyEvent('Enter'))

    // No real drag ever happened: no direction, no commit — and, just as
    // important, no WEDGE: the refusal said why (once — the replays
    // themselves must never toast) and the second Enter closed the
    // session. Asserting only "not committed" cannot tell those apart.
    expect(scene.set_face_uv_frame).not.toHaveBeenCalled()
    expect(onToast).toHaveBeenCalledTimes(1)
    expect(onToast.mock.calls[0][0]).toMatch(/direction/i)
    expect(tool.capturingInput()).toBe(false)
  })

  it('a SNAP cannot yank the gate open: travel is measured on the RAW ray hit, so a nearby endpoint does not turn a motionless hold into a drag', () => {
    // The gate's documented defence: a point snap can move the MAPPED
    // point far past the click/drag threshold while the mouse has not
    // moved at all (press on the pin, an endpoint within the snap ring).
    // If the gate measured the snapped point, the very first VCB replay
    // would latch hasMoved, silently drag the pin onto the snap, and hand
    // a typed length a phantom direction — the exact pollution the gate
    // exists to stop.
    const scene = texturedFixtureScene()
    const { tool, previewFaceUv, onToast } = makeTool(scene)
    enterIdle(tool)
    previewFaceUv.mockClear()

    const pressRay = rayAt(0, 0) // press dead-center on the red pin...
    tool.onPointerDown(pointSnapAt(0.5, 0, 0), pressRay)
    // ...and replay the SAME ray with a live endpoint snap half a meter
    // away — 50x the fallback drag threshold, zero actual mouse travel.
    tool.onPointerMove(pointSnapAt(0.5, 0, 0), pressRay)
    tool.onKey(keyEvent('2'))
    tool.onPointerMove(pointSnapAt(0.5, 0, 0), pressRay)

    // Nothing may have been written: the hold is still a hold.
    expect(previewFaceUv).not.toHaveBeenCalled()

    // And the typed length still has no direction to move along — the
    // snap never leaked into lastDelta.
    tool.onKey(keyEvent('Enter'))
    expect(onToast).toHaveBeenCalledTimes(1)
    expect(onToast.mock.calls[0][0]).toMatch(/direction/i)
    expect(scene.set_face_uv_frame).not.toHaveBeenCalled()
  })
})

describe('PositionTextureTool — component instances', () => {
  /** Forward-maps a LOCAL (definition-space) 3D point into a WORLD ray that
   *  hits it straight down the LOCAL normal — the exact inverse of what
   *  `_rayToSessionSpace` maps it back to, by construction. */
  function worldRayForLocalPoint(pose: ArrayLike<number>, localPoint: V3): Ray {
    const localRay: Ray = {
      origin: [localPoint[0] + FACE_NORMAL[0] * 5, localPoint[1] + FACE_NORMAL[1] * 5, localPoint[2] + FACE_NORMAL[2] * 5],
      direction: [-FACE_NORMAL[0], -FACE_NORMAL[1], -FACE_NORMAL[2]],
    }
    return {
      origin: applyAffine3x4(pose, localRay.origin),
      direction: applyAffine3x4Linear(pose, localRay.direction),
    }
  }

  function instanceFixtureScene(pose: number[], overrides: SceneOpts = {}): WasmScene {
    return makeWasmScene({
      facePick: makeFacePick(3n, 4n, 77n),
      faceMaterial: makeFaceMaterial(9n, MATERIAL_SENTINEL),
      materialInfo: makeMaterialInfo(true, 1, 1),
      uvFrame: [],
      instancePose: pose,
      ...overrides,
    })
  }

  it('positions a face on a NON-UNIFORMLY-SCALED instance successfully — no AmbiguousInstanceScale-style refusal', () => {
    const pose = [2, 0, 0, 10, 0, 0.5, 0, 20, 0, 0, 1, 30]
    const scene = instanceFixtureScene(pose)
    const { tool, previewFaceUv, onCommit, onToast } = makeTool(scene)
    tool.setEditContext({ kind: 'instance', id: 77n, component: 55n })

    const local1: V3 = [0, 0, 0]
    const local2: V3 = [2, 0, 0]
    tool.onPointerDown(makeSnap(), worldRayForLocalPoint(pose, local1)) // enter + begin translate
    tool.onPointerMove(makeSnap(), worldRayForLocalPoint(pose, local2)) // drag by local delta (2,0)
    tool.onPointerUp(makeSnap(), worldRayForLocalPoint(pose, local2))

    // No refusal of any kind — the exact regression this pins.
    expect(onToast).not.toHaveBeenCalled()

    const lastPreview = lastPreviewFrame(previewFaceUv)
    const planar = planarDefaultFrame(FACE_NORMAL, 1, 1)
    expect(lastPreview.u0).toBeCloseTo(planar.u0 - planar.sx * 2)
    expect(lastPreview.v0).toBeCloseTo(planar.v0 - planar.tx * 2)
    expect(lastPreview.sx).toBeCloseTo(planar.sx)
    expect(lastPreview.ty).toBeCloseTo(planar.ty)

    tool.onKey(keyEvent('Enter'))
    expect(scene.set_face_uv_frame).toHaveBeenCalledTimes(1)
    const [obj, face] = (scene.set_face_uv_frame as ReturnType<typeof vi.fn>).mock.calls[0]
    expect(obj).toBe(3n)
    expect(face).toBe(4n)
    expect(onCommit).toHaveBeenCalledWith(3n)
  })

  it('a top-level click on an instanced face (not the active edit context) toasts and does not enter positioning or change anything', () => {
    const pose = [2, 0, 0, 10, 0, 0.5, 0, 20, 0, 0, 1, 30]
    const scene = instanceFixtureScene(pose)
    const { tool, previewFaceUv, onCommit, onToast } = makeTool(scene)

    tool.onPointerDown(makeSnap(), worldRayForLocalPoint(pose, [0, 0, 0]))

    expect(onToast).toHaveBeenCalledTimes(1)
    expect(onToast.mock.calls[0][0]).toMatch(/enter the component/i)
    expect(previewFaceUv).not.toHaveBeenCalled()
    expect(scene.set_face_uv_frame).not.toHaveBeenCalled()
    expect(onCommit).not.toHaveBeenCalled()
  })

  it('the SAME instanced face becomes positionable once its instance is the active edit context, and stops being once the context is left', () => {
    const pose = [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0] // identity pose — isolates the eligibility gate
    const scene = instanceFixtureScene(pose)
    const { tool, onToast } = makeTool(scene)

    tool.onPointerDown(makeSnap(), worldRayForLocalPoint(pose, [0, 0, 0]))
    expect(onToast).toHaveBeenCalledTimes(1)
    onToast.mockClear()

    tool.setEditContext({ kind: 'instance', id: 77n, component: 55n })
    tool.onPointerDown(makeSnap(), worldRayForLocalPoint(pose, [0, 0, 0]))
    expect(onToast).not.toHaveBeenCalled()
    expect(tool.capturingInput()).toBe(true) // a session opened

    tool.setEditContext({ kind: 'instance', id: 88n, component: 55n })
    expect(tool.capturingInput()).toBe(false)
    tool.onPointerDown(makeSnap(), worldRayForLocalPoint(pose, [0, 0, 0]))
    expect(onToast).toHaveBeenCalledTimes(1)
  })

  it('dragging the green corner on a MIRRORED instance commits the SAME local frame the identical local drag produces on a plain object', () => {
    const mirrorPose = [-1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0]
    const mirroredScene = instanceFixtureScene(mirrorPose)
    const plainScene = texturedFixtureScene()

    // The tile corners of the planar default: pin (0,0,0), green (0,-1,0).
    const pin3D: V3 = [0, 0, 0]
    const green3D: V3 = [0, -1, 0]
    const dragTargetLocal: [number, number] = [1.3, -0.9]
    const dragTarget3D = local2ToWorld(FACE_ANCHOR, TESS.u, TESS.v, dragTargetLocal)

    const { tool: plainTool, previewFaceUv: plainPreview } = makeTool(plainScene)
    plainTool.onPointerDown(makeSnap(), { origin: [pin3D[0], pin3D[1], 5], direction: [0, 0, -1] })
    plainTool.onPointerUp(makeSnap(), { origin: [pin3D[0], pin3D[1], 5], direction: [0, 0, -1] })
    plainTool.onPointerDown(makeSnap(), { origin: [green3D[0], green3D[1], 5], direction: [0, 0, -1] })
    plainTool.onPointerMove(makeSnap(), { origin: [dragTarget3D[0], dragTarget3D[1], 5], direction: [0, 0, -1] })
    const plainFrame = lastPreviewFrame(plainPreview)

    const { tool: mirroredTool, previewFaceUv: mirroredPreview } = makeTool(mirroredScene)
    mirroredTool.setEditContext({ kind: 'instance', id: 77n, component: 55n })
    mirroredTool.onPointerDown(makeSnap(), worldRayForLocalPoint(mirrorPose, pin3D))
    mirroredTool.onPointerUp(makeSnap(), worldRayForLocalPoint(mirrorPose, pin3D))
    mirroredTool.onPointerDown(makeSnap(), worldRayForLocalPoint(mirrorPose, green3D))
    mirroredTool.onPointerMove(makeSnap(), worldRayForLocalPoint(mirrorPose, dragTarget3D))
    const mirroredFrame = lastPreviewFrame(mirroredPreview)

    for (const key of ['sx', 'sy', 'sz', 'tx', 'ty', 'tz', 'u0', 'v0'] as const) {
      expect(mirroredFrame[key]).toBeCloseTo(plainFrame[key], 6)
    }
  })
})

describe('PositionTextureTool — Esc revert', () => {
  it('reverts the live preview to the exact pre-gesture frame and commits nothing', () => {
    const scene = texturedFixtureScene()
    const { tool, previewFaceUv, onCommit } = makeTool(scene)

    tool.onPointerDown(makeSnap(), rayAt(0, 0))
    tool.onPointerMove(makeSnap(), rayAt(3, 1))
    tool.onPointerUp(makeSnap(), rayAt(3, 1))

    previewFaceUv.mockClear()
    tool.onKey(keyEvent('Escape'))

    expect(scene.set_face_uv_frame).not.toHaveBeenCalled()
    expect(onCommit).not.toHaveBeenCalled()
    expect(previewFaceUv).toHaveBeenCalledTimes(1)
    const reverted = previewFaceUv.mock.calls[0][2] as UvFrameComponents
    const planar = planarDefaultFrame(FACE_NORMAL, 1, 1)
    expect(reverted.u0).toBeCloseTo(planar.u0)
    expect(reverted.v0).toBeCloseTo(planar.v0)
    expect(reverted.sx).toBeCloseTo(planar.sx)

    tool.onKey(keyEvent('Enter'))
    expect(scene.set_face_uv_frame).not.toHaveBeenCalled()
  })

  it('cancel() (tool switch) also reverts an uncommitted preview', () => {
    const scene = texturedFixtureScene()
    const { tool, previewFaceUv, onCommit } = makeTool(scene)

    tool.onPointerDown(makeSnap(), rayAt(0, 0))
    tool.onPointerMove(makeSnap(), rayAt(2, 2))
    tool.onPointerUp(makeSnap(), rayAt(2, 2))

    previewFaceUv.mockClear()
    tool.cancel()

    expect(scene.set_face_uv_frame).not.toHaveBeenCalled()
    expect(onCommit).not.toHaveBeenCalled()
    expect(previewFaceUv).toHaveBeenCalledTimes(1)
  })

  it('an unrelated history-generation bump does NOT turn Escape into an alarming abort — quiet revert, no toast', () => {
    let gen = 0n
    const scene = texturedFixtureScene({ historyGeneration: () => gen })
    const { tool, previewFaceUv, onToast, onCommit } = makeTool(scene)

    tool.onPointerDown(makeSnap(), rayAt(0, 0))
    tool.onPointerMove(makeSnap(), rayAt(2, 0))
    tool.onPointerUp(makeSnap(), rayAt(2, 0))

    gen = 1n

    previewFaceUv.mockClear()
    tool.onKey(keyEvent('Escape'))

    expect(previewFaceUv).toHaveBeenCalledTimes(1)
    const reverted = previewFaceUv.mock.calls[0][2] as UvFrameComponents
    const planar = planarDefaultFrame(FACE_NORMAL, 1, 1)
    expect(reverted.u0).toBeCloseTo(planar.u0)
    expect(reverted.v0).toBeCloseTo(planar.v0)
    expect(onToast).not.toHaveBeenCalled()
    expect(scene.set_face_uv_frame).not.toHaveBeenCalled()
    expect(onCommit).not.toHaveBeenCalled()

    tool.onKey(keyEvent('Enter'))
    expect(scene.set_face_uv_frame).not.toHaveBeenCalled()
  })

  it('the same unrelated history-generation bump, on tool-switch cancel() rather than Escape, is equally quiet', () => {
    let gen = 0n
    const scene = texturedFixtureScene({ historyGeneration: () => gen })
    const { tool, previewFaceUv, onToast, onCommit } = makeTool(scene)

    tool.onPointerDown(makeSnap(), rayAt(0, 0))
    tool.onPointerMove(makeSnap(), rayAt(2, 0))
    tool.onPointerUp(makeSnap(), rayAt(2, 0))

    gen = 1n

    previewFaceUv.mockClear()
    tool.cancel()

    expect(previewFaceUv).toHaveBeenCalledTimes(1)
    expect(onToast).not.toHaveBeenCalled()
    expect(scene.set_face_uv_frame).not.toHaveBeenCalled()
    expect(onCommit).not.toHaveBeenCalled()
  })

  it('sibling: a COMMIT attempt (not a cancel) after the same unrelated generation bump still toasts and aborts', () => {
    let gen = 0n
    const scene = texturedFixtureScene({ historyGeneration: () => gen })
    const { tool, onToast, onCommit } = makeTool(scene)

    tool.onPointerDown(makeSnap(), rayAt(0, 0))
    tool.onPointerMove(makeSnap(), rayAt(2, 0))
    tool.onPointerUp(makeSnap(), rayAt(2, 0))

    gen = 1n

    tool.onKey(keyEvent('Enter'))

    expect(scene.set_face_uv_frame).not.toHaveBeenCalled()
    expect(onCommit).not.toHaveBeenCalled()
    expect(onToast).toHaveBeenCalledTimes(1)
    expect(onToast.mock.calls[0][0]).toMatch(/model changed/i)
  })
})

describe('PositionTextureTool — an explicit pre-existing frame seeds the session (not the planar default)', () => {
  it('reads back the explicit frame via face_uv_frame and Esc reverts to IT, not the planar default', () => {
    const explicit = [1, 0, 0, 0, 1, 0, 0.4, -0.2]
    const scene = texturedFixtureScene({ uvFrame: explicit })
    const { tool, previewFaceUv } = makeTool(scene)

    tool.onPointerDown(makeSnap(), rayAt(0, 0))
    tool.onPointerMove(makeSnap(), rayAt(5, 5))
    tool.onPointerUp(makeSnap(), rayAt(5, 5))

    previewFaceUv.mockClear()
    tool.onKey(keyEvent('Escape'))

    const reverted = previewFaceUv.mock.calls[0][2] as UvFrameComponents
    expect(reverted.u0).toBeCloseTo(0.4)
    expect(reverted.v0).toBeCloseTo(-0.2)
  })
})

describe('PositionTextureTool — history-generation guard (mid-session external mutation)', () => {
  it('commit-blocked: a generation change between the drag and Enter aborts with no kernel write', () => {
    let gen = 0n
    const scene = texturedFixtureScene({ historyGeneration: () => gen })
    const { tool, onToast, onCommit } = makeTool(scene)

    tool.onPointerDown(makeSnap(), rayAt(0, 0))
    tool.onPointerMove(makeSnap(), rayAt(2, 0))
    tool.onPointerUp(makeSnap(), rayAt(2, 0))

    gen = 1n

    tool.onKey(keyEvent('Enter'))

    expect(scene.set_face_uv_frame).not.toHaveBeenCalled()
    expect(onCommit).not.toHaveBeenCalled()
    expect(onToast).toHaveBeenCalledTimes(1)
    expect(onToast.mock.calls[0][0]).toMatch(/model changed/i)

    onToast.mockClear()
    tool.onKey(keyEvent('Enter'))
    expect(onToast).not.toHaveBeenCalled()
  })

  it('commit-blocked: the same guard fires on a click-away commit, not just Enter', () => {
    let gen = 0n
    const scene = texturedFixtureScene({ historyGeneration: () => gen })
    const { tool, onToast } = makeTool(scene)

    tool.onPointerDown(makeSnap(), rayAt(0, 0))
    tool.onPointerMove(makeSnap(), rayAt(2, 0))
    tool.onPointerUp(makeSnap(), rayAt(2, 0))

    gen = 1n

    ;(scene.pick_face as ReturnType<typeof vi.fn>).mockReturnValueOnce(undefined)
    tool.onPointerDown(makeSnap(), rayAt(50, 50))

    expect(scene.set_face_uv_frame).not.toHaveBeenCalled()
    expect(onToast).toHaveBeenCalledTimes(1)
  })

  it('drag-blocked: a generation change mid-drag aborts AND reverts the in-place preview back to the original frame', () => {
    let gen = 0n
    const scene = texturedFixtureScene({ historyGeneration: () => gen })
    const { tool, previewFaceUv, onToast } = makeTool(scene)

    tool.onPointerDown(makeSnap(), rayAt(0, 0))
    tool.onPointerMove(makeSnap(), rayAt(2, 0)) // real preview applied

    gen = 1n
    previewFaceUv.mockClear()
    tool.onPointerMove(makeSnap(), rayAt(3, 0)) // guarded: abort + revert

    expect(onToast).toHaveBeenCalledTimes(1)
    expect(previewFaceUv).toHaveBeenCalledTimes(1)
    const reverted = previewFaceUv.mock.calls[0][2] as UvFrameComponents
    const planar = planarDefaultFrame(FACE_NORMAL, 1, 1)
    expect(reverted.u0).toBeCloseTo(planar.u0)
    expect(reverted.v0).toBeCloseTo(planar.v0)
    expect(scene.set_face_uv_frame).not.toHaveBeenCalled()
  })

  it('release-blocked: onPointerUp is guarded too — a mid-drag generation change plus a release aborts rather than resolving through a stale basis', () => {
    let gen = 0n
    const scene = texturedFixtureScene({ historyGeneration: () => gen })
    const { tool, onToast } = makeTool(scene)

    tool.onPointerDown(makeSnap(), rayAt(0, 0))
    tool.onPointerMove(makeSnap(), rayAt(2, 0))

    gen = 1n
    tool.onPointerUp(makeSnap(), rayAt(2, 0))

    expect(onToast).toHaveBeenCalledTimes(1)
    expect(tool.capturingInput()).toBe(false) // session closed
    expect(scene.set_face_uv_frame).not.toHaveBeenCalled()
  })

  it('a recycled object/face handle at abort time skips the preview patch entirely but still toasts and closes the session', () => {
    let gen = 0n
    let faceResolves = true
    const faceMaterial = makeFaceMaterial(9n, MATERIAL_SENTINEL)
    const scene = texturedFixtureScene({ historyGeneration: () => gen })
    ;(scene.face_material as ReturnType<typeof vi.fn>).mockImplementation(() =>
      faceResolves ? faceMaterial : undefined,
    )
    const { tool, previewFaceUv, onToast } = makeTool(scene)

    tool.onPointerDown(makeSnap(), rayAt(0, 0))
    tool.onPointerMove(makeSnap(), rayAt(2, 0))

    gen = 1n
    faceResolves = false
    previewFaceUv.mockClear()
    tool.onKey(keyEvent('Enter'))

    expect(previewFaceUv).not.toHaveBeenCalled() // nothing left to patch
    expect(onToast).toHaveBeenCalledTimes(1)
    expect(tool.capturingInput()).toBe(false)
  })
})

describe('PositionTextureTool — click vs. drag threshold', () => {
  it('a sub-threshold jitter between press and release reads as a click: no commit, no spurious undo entry', () => {
    const scene = texturedFixtureScene()
    const { tool, onCommit } = makeTool(scene)

    tool.onPointerDown(makeSnap(), rayAt(0, 0))
    tool.onPointerMove(makeSnap(), rayAt(0.0005, 0))
    tool.onPointerUp(makeSnap(), rayAt(0.0005, 0))

    tool.onKey(keyEvent('Enter'))

    expect(scene.set_face_uv_frame).not.toHaveBeenCalled()
    expect(onCommit).not.toHaveBeenCalled()
  })

  it('a sub-threshold jitter reverts the live preview back to the untouched frame', () => {
    const scene = texturedFixtureScene()
    const { tool, previewFaceUv } = makeTool(scene)

    tool.onPointerDown(makeSnap(), rayAt(0, 0))
    tool.onPointerMove(makeSnap(), rayAt(0.0005, 0))
    tool.onPointerUp(makeSnap(), rayAt(0.0005, 0))

    const lastPreview = lastPreviewFrame(previewFaceUv)
    const planar = planarDefaultFrame(FACE_NORMAL, 1, 1)
    expect(lastPreview.u0).toBeCloseTo(planar.u0)
    expect(lastPreview.v0).toBeCloseTo(planar.v0)
  })

  it('a click-threshold release does not swallow a REAL earlier drag in the same session', () => {
    const scene = texturedFixtureScene()
    const { tool, onCommit } = makeTool(scene)

    tool.onPointerDown(makeSnap(), rayAt(0, 0))
    tool.onPointerMove(makeSnap(), rayAt(2, 0))
    tool.onPointerUp(makeSnap(), rayAt(2, 0))

    tool.onPointerDown(makeSnap(), rayAt(2, 0))
    tool.onPointerMove(makeSnap(), rayAt(2.0005, 0))
    tool.onPointerUp(makeSnap(), rayAt(2.0005, 0))

    tool.onKey(keyEvent('Enter'))

    expect(scene.set_face_uv_frame).toHaveBeenCalledTimes(1)
    expect(onCommit).toHaveBeenCalledWith(3n)
  })

  it('a real drag past the threshold is unaffected by the guard (companion sanity check)', () => {
    const scene = texturedFixtureScene()
    const { tool, onCommit } = makeTool(scene)

    tool.onPointerDown(makeSnap(), rayAt(0, 0))
    tool.onPointerMove(makeSnap(), rayAt(2, 0))
    tool.onPointerUp(makeSnap(), rayAt(2, 0))

    tool.onKey(keyEvent('Enter'))

    expect(scene.set_face_uv_frame).toHaveBeenCalledTimes(1)
    expect(onCommit).toHaveBeenCalledWith(3n)
  })
})

/**
 * These tests used to call `updateGripScale(camera, 800)` — a viewport HEIGHT
 * in pixels — and assert the old `(px * dist * tanHalfFov) / height` formula.
 * The Viewport's render loop has never passed that: it passes a
 * `worldPerPixel(dist)` CALLBACK derived from the active `CameraRig`, the
 * projection-agnostic contract every other widget tool implements. Because the
 * call site reaches the hook through an `as` cast, nothing checked the
 * mismatch, and these tests could not catch it either — they were written
 * against the tool's own mistaken assumption instead of against the caller.
 * The result in the real app: the callback landed where a number was expected,
 * every arithmetic result went NaN, and three drew no pin at all — the pins
 * were invisible AND unpickable (the same cached values feed
 * `_pickToleranceAt`). Two playtest rounds reported "no handles/dots".
 *
 * They now drive the REAL contract. `Tool` declares the hook, so a future
 * signature drift is a compile error rather than a silent NaN.
 */
describe('PositionTextureTool — screen-constant gizmo sizing (updateGripScale)', () => {
  const GIZMO_SCREEN_PX = 8 // mirrors the tool's own constant

  /** A perspective-style `worldPerPixel`, matching what `CameraRig` supplies:
   *  world units per pixel at distance `dist`. */
  function perspectiveWorldPerPixel(fovDeg: number, heightPx: number) {
    const tanHalf = Math.tan((fovDeg * Math.PI) / 360)
    return (dist: number) => (2 * dist * tanHalf) / heightPx
  }

  it('the pin boxes are sized so their WORLD half-extent projects to the same on-screen pixel size at two different camera distances', () => {
    const scene = texturedFixtureScene()
    const { tool, gizmoGroup } = makeTool(scene)

    enterIdle(tool)

    const meshes = gizmoMeshes(gizmoGroup)
    expect(meshes.length).toBe(3) // red + green + blue

    const wpp = perspectiveWorldPerPixel(50, 800)

    const near = new THREE.PerspectiveCamera(50, 1, 0.01, 1000)
    near.position.set(0, 0, 2)
    tool.updateGripScale(near, wpp)
    const nearHalves = meshes.map((m) => {
      const dist = near.position.distanceTo(m.position)
      const expected = GIZMO_SCREEN_PX * wpp(dist) * 0.5
      expect(m.scale.x).toBeCloseTo(expected, 10)
      // A NaN scale is the exact runtime shape of the signature bug, and
      // `toBeCloseTo` against a NaN expectation would NOT catch it (both
      // sides NaN reads as "close"). Assert finiteness outright.
      expect(Number.isFinite(m.scale.x)).toBe(true)
      return m.scale.x
    })

    const far = new THREE.PerspectiveCamera(50, 1, 0.01, 1000)
    far.position.set(0, 0, 200)
    tool.updateGripScale(far, wpp)
    const farHalves = meshes.map((m) => {
      const dist = far.position.distanceTo(m.position)
      const expected = GIZMO_SCREEN_PX * wpp(dist) * 0.5
      expect(m.scale.x).toBeCloseTo(expected, 10)
      expect(Number.isFinite(m.scale.x)).toBe(true)
      return m.scale.x
    })

    expect(Math.min(...farHalves)).toBeGreaterThan(Math.max(...nearHalves))
  })

  it('gizmo size does NOT scale with the material world tile size — only with screen distance', () => {
    const camera = new THREE.PerspectiveCamera(50, 1, 0.01, 1000)
    camera.position.set(5, 5, 5)
    const wpp = perspectiveWorldPerPixel(50, 800)

    function pinScaleFor(tileSize: number): number {
      const scene = texturedFixtureScene({ materialInfo: makeMaterialInfo(true, tileSize, tileSize) })
      const { tool, gizmoGroup } = makeTool(scene)
      enterIdle(tool)
      tool.updateGripScale(camera, wpp)
      const pin = gizmoMeshes(gizmoGroup)[0]
      return pin.scale.x
    }

    const tiny = pinScaleFor(0.01)
    const huge = pinScaleFor(500)

    expect(tiny).toBeCloseTo(huge, 10)
    expect(Number.isFinite(tiny)).toBe(true)
  })

  it('sizes the pins under PARALLEL projection too — an orthographic camera is not skipped', () => {
    // The tool previously bailed out on `!(camera instanceof
    // PerspectiveCamera)`, which is the very guard ScaleTool's doc comment
    // records having removed because it "silently hid every grip under
    // parallel projection". Under parallel projection `worldPerPixel` is
    // constant in `dist`, so every pin ends up the SAME world size — the
    // property that makes the widget legible at any zoom.
    const scene = texturedFixtureScene()
    const { tool, gizmoGroup } = makeTool(scene)
    enterIdle(tool)

    const ortho = new THREE.OrthographicCamera(-5, 5, 5, -5, 0.01, 1000)
    ortho.position.set(0, 0, 20)
    const constantWpp = () => 10 / 800 // frustum height / viewport height

    tool.updateGripScale(ortho, constantWpp)

    const meshes = gizmoMeshes(gizmoGroup)
    const expected = GIZMO_SCREEN_PX * (10 / 800) * 0.5
    for (const m of meshes) {
      expect(m.scale.x).toBeCloseTo(expected, 10)
      expect(Number.isFinite(m.scale.x)).toBe(true)
      expect(m.scale.x).toBeGreaterThan(0)
    }
  })
})

describe('PositionTextureTool — tilted face: pins on the tile lattice, near the click, no jump', () => {
  it('on a non-axis-aligned face the pins sit on INTEGER UVs of the already-rendered default frame, within half a tile of the click', () => {
    const rawNormal: [number, number, number] = [1, 2, 3]
    const len = Math.hypot(...rawNormal)
    const tiltedNormal: [number, number, number] = [rawNormal[0] / len, rawNormal[1] / len, rawNormal[2] / len]
    const anchor: [number, number, number] = [0.2, -0.3, 0.4]

    const scene = texturedFixtureScene({ facePlane: { anchor, normal: tiltedNormal } })
    const { tool, gizmoGroup } = makeTool(scene)

    // A ray straight down the normal, hitting exactly `anchor`.
    const entryRay: Ray = {
      origin: [anchor[0] + tiltedNormal[0] * 5, anchor[1] + tiltedNormal[1] * 5, anchor[2] + tiltedNormal[2] * 5],
      direction: [-tiltedNormal[0], -tiltedNormal[1], -tiltedNormal[2]],
    }
    tool.onPointerDown(makeSnap(), entryRay)

    const planar = planarDefaultFrame(tiltedNormal, 1, 1)
    const [pin, green, blue] = gizmoMeshes(gizmoGroup)

    // The red pin maps to an INTEGER lattice UV of the frame that's ALREADY
    // rendering — no visual jump, and a legible tile corner.
    const pinUv = applyFrame(planar, [pin.position.x, pin.position.y, pin.position.z])
    expect(pinUv[0]).toBeCloseTo(Math.round(pinUv[0]), 6)
    expect(pinUv[1]).toBeCloseTo(Math.round(pinUv[1]), 6)

    // Within half a tile (diagonal) of the click — ON the face, not at some
    // remote origin projection.
    const dist = Math.hypot(pin.position.x - anchor[0], pin.position.y - anchor[1], pin.position.z - anchor[2])
    expect(dist).toBeLessThan(Math.SQRT1_2 + 1e-9)

    // Green and blue are the ADJACENT lattice corners.
    const greenUv = applyFrame(planar, [green.position.x, green.position.y, green.position.z])
    expect(greenUv[0]).toBeCloseTo(pinUv[0] + 1, 6)
    expect(greenUv[1]).toBeCloseTo(pinUv[1], 6)
    const blueUv = applyFrame(planar, [blue.position.x, blue.position.y, blue.position.z])
    expect(blueUv[0]).toBeCloseTo(pinUv[0], 6)
    expect(blueUv[1]).toBeCloseTo(pinUv[1] + 1, 6)
  })
})

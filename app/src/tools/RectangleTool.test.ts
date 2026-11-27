import { describe, it, expect, vi, beforeAll, afterAll, beforeEach, afterEach } from 'vitest'
import * as THREE from 'three'
import { RectangleTool } from './RectangleTool'
import type { Snap } from './types'
import { draftMock } from './draftMock'
import { setLengthUnit, getLengthUnit, setPrecision, DEFAULT_PRECISION } from '../settings/units'
import type { Scene as WasmScene } from '../wasm/loader'
import { setPickRenderMode } from './pickPolicy'
import type { Ray } from '../viewport/math'

/** A ray straight down (−Z) through world (x, y) — hits a z=1 top face at (x, y, 1). */
function rayThrough(x: number, y: number): Ray {
  return { origin: [x, y, 5], direction: [0, 0, -1] }
}

function makeSnap(overrides: Partial<Snap> = {}): Snap {
  return { x: 0, y: 0, z: 0, kind: 'ground', ...overrides }
}

function makePick(object: bigint, face: bigint, instance?: bigint) {
  return {
    object: () => object,
    face: () => face,
    depth: () => 1,
    instance: () => instance,
    free: vi.fn(),
  }
}

/** Minimal WasmScene stub — only the members RectangleTool calls. */
function makeWasmScene(opts: {
  pick?: () => ReturnType<typeof makePick> | undefined
  /** node_parent(0, id) result per object (a grouped object's group id). */
  parents?: Map<bigint, bigint>
} = {}): WasmScene {
  return {
    history_generation: vi.fn(() => 1n),
    ...draftMock().members,
    pick_face: vi.fn(() => opts.pick?.()),
    pick_sketch: vi.fn(() => undefined), // no committed sketches in these fixtures
    sketch_plane: vi.fn(() => new Float64Array([0, 0, 0, 0, 0, 1])), // every minted sketch is on the ground plane
    node_parent: vi.fn((_kind: number, id: bigint) => opts.parents?.get(id)),
    // A top face at z=1, normal +Z.
    face_normal: vi.fn(() => new Float64Array([0, 0, 1])),
    face_plane: vi.fn(() => new Float64Array([0, 0, 1, 0, 0, 1])),
    instance_pose: vi.fn(() => new Float64Array([1, 0, 0, 5, 0, 1, 0, 0, 0, 0, 1, 0])), // translated +5 in x
  } as unknown as WasmScene
}

/** The four corners (segment start points) of the nth draft's rectangle as a
 *  flat xyz Float64Array — the loop a face draft carries, now read from its
 *  four `draft_add_segment` calls (each draft adds exactly four). */
function draftLoop(scene: WasmScene, nth = 0): Float64Array {
  const calls = (scene.draft_add_segment as ReturnType<typeof vi.fn>).mock.calls.slice(nth * 4, nth * 4 + 4)
  expect(calls).toHaveLength(4)
  const out = new Float64Array(12)
  calls.forEach((c, i) => {
    out[i * 3] = c[1] as number
    out[i * 3 + 1] = c[2] as number
    out[i * 3 + 2] = c[3] as number
  })
  return out
}

function makeTool(scene: WasmScene) {
  const preview = new THREE.Group()
  const onCommit = vi.fn()
  const onToast = vi.fn()
  const onFaceImprint = vi.fn()
  const onMeasurement = vi.fn()
  const tool = new RectangleTool(scene, preview, onCommit, onToast, onFaceImprint, onMeasurement)
  return { tool, preview, onCommit, onToast, onFaceImprint, onMeasurement }
}

// Deliberate contract change (selection-UX overhaul): drawing on a PLAIN
// solid's face at the top level no longer requires double-clicking into the
// object first — clicking its face with a draw tool means "draw on that
// face". Groups and Components keep the explicit edit step.
describe('RectangleTool — top-level draw-on-face', () => {
  it('two clicks on a plain object\'s face draw on it via one draft_begin_face draft (no edit context)', () => {
    const scene = makeWasmScene({ pick: () => makePick(7n, 3n) })
    const { tool, onFaceImprint, onToast } = makeTool(scene)

    // First corner on the face, then the opposite corner 1×2 away.
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 1, kind: 'face' }), rayThrough(0, 0))
    tool.onPointerDown(makeSnap({ x: 1, y: 2, z: 1, kind: 'face' }), rayThrough(1, 2))

    expect(scene.draft_begin_face).toHaveBeenCalledTimes(1)
    const [object, face] = (scene.draft_begin_face as ReturnType<typeof vi.fn>).mock.calls[0]
    expect(object).toBe(7n)
    expect(face).toBe(3n)
    expect(scene.draft_add_segment).toHaveBeenCalledTimes(4) // 4 corners, one segment each
    expect(scene.draft_end).toHaveBeenCalledTimes(1)
    expect(onFaceImprint).toHaveBeenCalledWith(7n)
    // No plane draft was opened.
    expect(scene.draft_begin_plane).not.toHaveBeenCalled()
    expect(onToast).not.toHaveBeenCalled()
  })

  it('a face of a GROUPED object is drawn on as FREE drawing on its world plane (not the ground, not inside the group)', () => {
    const scene = makeWasmScene({
      pick: () => makePick(7n, 3n),
      parents: new Map([[7n, 5n]]), // object 7 lives inside group 5
    })
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 1, kind: 'face' }), rayThrough(0, 0))
    tool.onPointerDown(makeSnap({ x: 1, y: 2, z: 1, kind: 'face' }), rayThrough(1, 2))

    expect(scene.draft_begin_face).not.toHaveBeenCalled()
    expect(scene.draft_begin_plane).toHaveBeenCalledTimes(1)
    // The face's world plane z = 1 with its +Z normal — not the ground.
    expect((scene.draft_begin_plane as ReturnType<typeof vi.fn>).mock.calls[0].slice(0, 6)).toEqual([0, 0, 1, 0, 0, 1])
    expect(scene.draft_begin_plane_in_instance).not.toHaveBeenCalled()
    expect(scene.draft_add_segment).toHaveBeenCalledTimes(4) // free rectangle on the plane
    expect(tool.snapProjected()).toBe(false) // reads "On Face", not "On Face Projected"
  })

  it('a face of a component instance is drawn on as FREE drawing on its world plane (pose applied, no instance draft)', () => {
    const scene = makeWasmScene({ pick: () => makePick(7n, 3n, 12n) })
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 5, y: 0, z: 1, kind: 'face' }), rayThrough(5, 0))
    tool.onPointerDown(makeSnap({ x: 6, y: 2, z: 1, kind: 'face' }), rayThrough(6, 2))

    expect(scene.draft_begin_face).not.toHaveBeenCalled()
    expect(scene.draft_begin_plane).toHaveBeenCalledTimes(1)
    // The face plane z = 1 of the instance (translated +5 in x), in WORLD space.
    const a = (scene.draft_begin_plane as ReturnType<typeof vi.fn>).mock.calls[0].slice(0, 6) as number[]
    expect(a[2]).toBeCloseTo(1)
    expect(a.slice(3)).toEqual([0, 0, 1])
    expect(scene.draft_begin_plane_in_instance).not.toHaveBeenCalled()
    expect(scene.draft_add_segment).toHaveBeenCalledTimes(4)
  })

  it('idle snapConstraint over a GROUPED object\'s face locks to its world plane so the chip reads On Face', () => {
    const scene = makeWasmScene({ pick: () => makePick(7n, 3n), parents: new Map([[7n, 5n]]) })
    const { tool } = makeTool(scene)
    expect(tool.snapConstraint(rayThrough(0, 0))?.constraintPlane).toEqual({ point: [0, 0, 1], normal: [0, 0, 1] })
  })

  it.each([
    ['GROUPED', () => makeWasmScene({ pick: () => makePick(7n, 3n), parents: new Map([[7n, 5n]]) }), 0],
    ['instanced', () => makeWasmScene({ pick: () => makePick(7n, 3n, 12n) }), 5],
  ])('a first click ON the corner of a %s top draws free on its world plane, not the ground', (_n, mk, dx) => {
    const scene = mk()
    // The plain ray MISSES at the corner (strict polygon test); only the
    // nudged boundary probes hit the face.
    const plain = rayThrough(dx, 0).direction
    const hit = (scene.pick_face as ReturnType<typeof vi.fn>).getMockImplementation()!
    ;(scene.pick_face as ReturnType<typeof vi.fn>).mockImplementation((...a: number[]) =>
      a[3] === plain[0] && a[4] === plain[1] && a[5] === plain[2] ? undefined : (hit as (...x: number[]) => unknown)(...a))
    const { tool } = makeTool(scene)
    const corner = makeSnap({ x: dx, y: 0, z: 1, kind: 'endpoint', object: 7n, elementKind: 'vertex' })
    tool.onPointerDown(corner, rayThrough(dx, 0))
    tool.onPointerDown(makeSnap({ x: dx + 1, y: 2, z: 1, kind: 'face' }), rayThrough(dx + 1, 2))
    expect(scene.draft_begin_face).not.toHaveBeenCalled()
    const a = (scene.draft_begin_plane as ReturnType<typeof vi.fn>).mock.calls[0].slice(0, 6) as number[]
    expect(a[2]).toBeCloseTo(1)
    expect(a.slice(3)).toEqual([0, 0, 1])
    expect(scene.draft_begin_plane_in_instance).not.toHaveBeenCalled()
    expect(scene.draft_add_segment).toHaveBeenCalledTimes(4)
  })

  it('idle snapConstraint locks to a plain object\'s face plane so the first corner lands on it', () => {
    const scene = makeWasmScene({ pick: () => makePick(7n, 3n) })
    const { tool } = makeTool(scene)

    const constraint = tool.snapConstraint(rayThrough(0, 0))
    expect(constraint?.constraintPlane).toEqual({ point: [0, 0, 1], normal: [0, 0, 1] })
  })

  it('a mid-gesture GROUND rectangle is not hijacked by hovering a face for the second corner', () => {
    let hovering = false
    const scene = makeWasmScene({ pick: () => (hovering ? makePick(7n, 3n) : undefined) })
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0)) // ground anchor
    hovering = true // cursor drifts over a solid mid-gesture
    tool.onPointerDown(makeSnap({ x: 1, y: 2, z: 0 }), rayThrough(1, 2))

    expect(scene.draft_add_segment).toHaveBeenCalledTimes(4) // stayed a ground rectangle
    expect(scene.draft_begin_face).not.toHaveBeenCalled()
  })

  it('inside an entered object context only that object\'s faces are drawable (unchanged)', () => {
    const scene = makeWasmScene({ pick: () => makePick(999n, 3n) })
    const { tool } = makeTool(scene)
    tool.setEditContext({ kind: 'object', id: 7n })

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 1 }), rayThrough(0, 0))

    // Ignored outright — no face anchor, and no plane draft either.
    expect(tool.capturingInput()).toBe(false)
    expect(scene.draft_begin_plane).not.toHaveBeenCalled()
  })
})

describe('RectangleTool — instance editing context (component-edit-parity.md phase A2)', () => {
  const INSTANCE = 42n
  const COMPONENT = 5n

  it('face mode routes to draft_begin_face_in_instance, never the world draft_begin_face', () => {
    const scene = makeWasmScene({ pick: () => makePick(7n, 3n, INSTANCE) })
    const { tool, onFaceImprint } = makeTool(scene)
    tool.setEditContext({ kind: 'instance', id: INSTANCE, component: COMPONENT })
    tool.setFaceEligibility((_object, instance) => instance === INSTANCE)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 1, kind: 'face' }), rayThrough(0, 0))
    tool.onPointerDown(makeSnap({ x: 1, y: 2, z: 1, kind: 'face' }), rayThrough(1, 2))

    expect(scene.draft_begin_face_in_instance).toHaveBeenCalledTimes(1)
    const [instance, object, face] = (scene.draft_begin_face_in_instance as ReturnType<typeof vi.fn>).mock.calls[0]
    expect(instance).toBe(INSTANCE)
    expect(object).toBe(7n)
    expect(face).toBe(3n)
    expect(scene.draft_begin_face).not.toHaveBeenCalled()
    expect(onFaceImprint).toHaveBeenCalledWith(7n)
  })

  it('plane mode on empty space opens a plane draft in the instance via draft_begin_plane_in_instance', () => {
    const scene = makeWasmScene({ pick: () => undefined })
    const { tool } = makeTool(scene)
    tool.setEditContext({ kind: 'instance', id: INSTANCE, component: COMPONENT })

    tool.onPointerDown(makeSnap({ x: 6, y: 1, z: 0 }), rayThrough(6, 1))
    tool.onPointerDown(makeSnap({ x: 7, y: 3, z: 0 }), rayThrough(7, 3))

    expect(scene.draft_begin_plane_in_instance).toHaveBeenCalledTimes(1)
    expect((scene.draft_begin_plane_in_instance as ReturnType<typeof vi.fn>).mock.calls[0][0]).toBe(INSTANCE)
    expect(scene.draft_begin_plane).not.toHaveBeenCalled()
    expect(scene.draft_add_segment).toHaveBeenCalledTimes(4)
    // Points go straight through in WORLD space — the engine maps them into
    // the definition through the instance pose itself (no toLocal here).
    const xs: number[] = []
    for (const call of (scene.draft_add_segment as ReturnType<typeof vi.fn>).mock.calls) {
      xs.push(call[1] as number, call[4] as number)
    }
    expect(Math.min(...xs)).toBe(6)
    expect(Math.max(...xs)).toBe(7)
  })

  // The face_normal-as-world-space finding: every OTHER instance-context test
  // in this file uses a translation-only pose, where the LOCAL normal
  // face_normal reports happens to equal the WORLD one — a mapping bug would
  // go unnoticed. This one genuinely rotates the instance (90° about X),
  // turning the member face's local +Z normal into world −Y — before the
  // fix, the tool used the raw local (0,0,1) normal directly and would have
  // drawn the loop into the WRONG (Z-constant) plane; after it, the loop
  // lands in the correctly posed (Y-constant) plane.
  it('face mode poses the LOCAL face_normal into WORLD space through a rotated instance pose', () => {
    const scene = makeWasmScene({ pick: () => makePick(7n, 3n, INSTANCE) })
    ;(scene.instance_pose as ReturnType<typeof vi.fn>).mockReturnValue(
      new Float64Array([1, 0, 0, 0, 0, 0, -1, 0, 0, 1, 0, 0]),
    )
    // The local face plane z = -5, which this pose (y' = -z) puts at world
    // Y = 5: the plane the clicks below lie on. (The first click is now
    // checked against the face's real plane, so the fixture's plane has to
    // be the one its story describes.)
    ;(scene.face_plane as ReturnType<typeof vi.fn>).mockReturnValue(new Float64Array([0, 0, -5, 0, 0, 1]))
    const { tool } = makeTool(scene)
    tool.setEditContext({ kind: 'instance', id: INSTANCE, component: COMPONENT })
    tool.setFaceEligibility((_object, instance) => instance === INSTANCE)

    // Anchor at world (0,5,1) — on the posed (Y=5) plane. Second click's ray
    // travels along +Y from (2,0,3), landing on the SAME plane at (2,5,3):
    // a nonzero, non-degenerate rectangle in BOTH in-plane directions.
    tool.onPointerDown(makeSnap({ x: 0, y: 5, z: 1, kind: 'face' }), {
      origin: [0, 0, 1],
      direction: [0, 1, 0],
    })
    tool.onPointerDown(makeSnap({ x: 2, y: 5, z: 3, kind: 'face' }), {
      origin: [2, 0, 3],
      direction: [0, 1, 0],
    })

    expect(scene.draft_begin_face_in_instance).toHaveBeenCalledTimes(1)
    const loopPts = draftLoop(scene)
    const ys: number[] = []
    for (let i = 1; i < loopPts.length; i += 3) ys.push(loopPts[i])
    // Every corner lies in the posed Y=5 plane — proof the normal was
    // actually mapped through the pose, not left at the raw local (0,0,1).
    for (const y of ys) expect(y).toBeCloseTo(5, 9)
  })
})

// Post-commit dimension retype (SketchUp: click both corners, then type
// `W,D` + Enter and the rectangle you just drew redraws to that size).
describe('RectangleTool — retype dimensions after the second click', () => {
  /** The base stub plus the history surface the retype window uses: a
   *  generation that moves on every recorded action, undo, and redo, like
   *  the kernel's. `draft_add_segment` bumps it once per gesture is more
   *  than the tool needs — one bump per commit is what matters. */
  function makeRetypeScene(opts: Parameters<typeof makeWasmScene>[0] = {}) {
    const base = makeWasmScene(opts) as unknown as Record<string, unknown>
    let gen = 1n
    let addCalls = 0
    let failAddAt = -1
    const baseAdd = base.draft_add_segment as (...args: unknown[]) => unknown
    const baseEnd = base.draft_end as (...args: unknown[]) => unknown
    const scene = {
      ...base,
      history_generation: vi.fn(() => gen),
      draft_add_segment: vi.fn((...args: unknown[]) => {
        addCalls += 1
        if (addCalls === failAddAt) throw new Error('PointOffPlane: nope')
        return baseAdd(...args)
      }),
      /** Test hook: the Nth `draft_add_segment` call overall is refused. */
      __failAddAt: (n: number) => { failAddAt = n },
      // A draft that ends records ONE step; a refused draft is cancelled
      // (draft_cancel) and records nothing.
      draft_end: vi.fn((...args: unknown[]) => { const r = baseEnd(...args); gen += 1n; return r }),
      scene_undo: vi.fn(() => { gen += 1n; return { free: vi.fn() } }),
      scene_redo: vi.fn(() => { gen += 1n; return { free: vi.fn() } }),
      /** Test hook: an unrelated recorded action. */
      __bump: () => { gen += 1n },
    }
    return scene as unknown as WasmScene & { __bump(): void; __failAddAt(n: number): void }
  }

  function key(tool: RectangleTool, k: string) {
    tool.onKey({ key: k } as KeyboardEvent)
  }

  function typeDims(tool: RectangleTool, text: string) {
    for (const ch of text) key(tool, ch)
    key(tool, 'Enter')
  }

  /** The (x, y) corners of the last four `draft_add_segment` calls. */
  function lastRectangle(scene: WasmScene): [number, number][] {
    const calls = (scene.draft_add_segment as ReturnType<typeof vi.fn>).mock.calls
    return calls.slice(-4).map((c) => [c[1] as number, c[2] as number])
  }

  function drawGround(tool: RectangleTool, a: [number, number], b: [number, number]) {
    tool.onPointerDown(makeSnap({ x: a[0], y: a[1], z: 0 }), rayThrough(a[0], a[1]))
    tool.onPointerDown(makeSnap({ x: b[0], y: b[1], z: 0 }), rayThrough(b[0], b[1]))
  }

  it('is closed before any rectangle exists — digits are not captured', () => {
    const { tool } = makeTool(makeRetypeScene())
    expect(tool.capturesKey('5')).toBe(false)
    expect(tool.hasArmedGesture()).toBe(false)
  })

  it('the retype line beats the inferred-plane line while the window is open', () => {
    const scene = Object.assign(makeRetypeScene(), { axes: vi.fn(() => new Float64Array([0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1])) })
    const { tool } = makeTool(scene)
    drawGround(tool, [0, 0], [2, 1])
    // Front view, hovering empty space: the green plane is inferred.
    const front = new THREE.PerspectiveCamera(45, 1.5, 0.1, 1000)
    front.position.set(0, -10, 0.5)
    front.up.set(0, 0, 1)
    front.lookAt(0, 0, 0.5)
    front.updateMatrixWorld(true)
    tool.updateCamera(front)
    const ray: Ray = { origin: [0, -10, 0.5], direction: [0, 1, 0] }
    tool.snapConstraint(ray)
    tool.onPointerMove(makeSnap({ x: 0, y: 0, z: 0.5, kind: 'plane' }), ray)
    expect((tool as unknown as { _idleInferred: { axis: number } | null })._idleInferred?.axis).toBe(1)
    expect(tool.statusHint()).toContain('Type exact dimensions to resize the rectangle you just drew')
  })

  it('after a two-click ground rectangle, typing W,D + Enter undoes it once and redraws it at that size, growing the same way', () => {
    const scene = makeRetypeScene()
    const { tool, onCommit, onMeasurement } = makeTool(scene)
    // Second corner toward −x, −y: the retyped rectangle must grow that way too.
    drawGround(tool, [1, 1], [0, -1])
    expect(scene.draft_add_segment).toHaveBeenCalledTimes(4)
    expect(onCommit).toHaveBeenCalledTimes(1)
    expect(tool.statusHint()).toContain('resize')

    // A digit opens the window; the readout shows the buffer.
    expect(tool.capturesKey('2')).toBe(true)
    typeDims(tool, '2,3')
    expect(onMeasurement).toHaveBeenCalledWith(expect.stringContaining('2'))

    expect(scene.scene_undo).toHaveBeenCalledTimes(1)
    expect(scene.scene_redo).not.toHaveBeenCalled()
    expect(scene.draft_add_segment).toHaveBeenCalledTimes(8)
    // Anchor (1,1), far corner (1−2, 1−3) = (−1, −2).
    const xs = lastRectangle(scene).map((c) => c[0])
    const ys = lastRectangle(scene).map((c) => c[1])
    expect(Math.min(...xs)).toBeCloseTo(-1, 9)
    expect(Math.max(...xs)).toBeCloseTo(1, 9)
    expect(Math.min(...ys)).toBeCloseTo(-2, 9)
    expect(Math.max(...ys)).toBeCloseTo(1, 9)
    expect(onCommit).toHaveBeenCalledTimes(2)
    // Readout cleared after the commit.
    expect(onMeasurement).toHaveBeenLastCalledWith('')
  })

  it('a single value makes a square; the window stays open so a second size can be typed', () => {
    const scene = makeRetypeScene()
    const { tool } = makeTool(scene)
    drawGround(tool, [0, 0], [1, 2])
    typeDims(tool, '5')
    let xs = lastRectangle(scene).map((c) => c[0])
    let ys = lastRectangle(scene).map((c) => c[1])
    expect(Math.max(...xs)).toBeCloseTo(5, 9)
    expect(Math.max(...ys)).toBeCloseTo(5, 9)

    typeDims(tool, '1,4')
    expect(scene.scene_undo).toHaveBeenCalledTimes(2)
    xs = lastRectangle(scene).map((c) => c[0])
    ys = lastRectangle(scene).map((c) => c[1])
    expect(Math.max(...xs)).toBeCloseTo(1, 9)
    expect(Math.max(...ys)).toBeCloseTo(4, 9)
  })

  it('with an empty buffer, letters and Space keep their global meaning; once a digit is in, the dimension grammar is captured', () => {
    const { tool } = makeTool(makeRetypeScene())
    drawGround(tool, [0, 0], [1, 1])
    for (const k of ['m', 'c', 'p', 'f', ' ', 'Enter', 'Backspace', ',']) {
      expect(tool.capturesKey(k)).toBe(false)
    }
    key(tool, '2')
    for (const k of ['m', ',', 'x', "'", '"', '/', 'Backspace', 'Enter', '5']) {
      expect(tool.capturesKey(k)).toBe(true)
    }
    // Space is the global reset-to-Select even with a buffer open.
    expect(tool.capturesKey(' ')).toBe(false)
    // Still not a tool-switch-blocking letter for keys outside the grammar.
    expect(tool.capturesKey('p')).toBe(false)
    expect(tool.capturesKey('q')).toBe(false)
    // An open buffer is an armed gesture for Escape's purposes.
    expect(tool.hasArmedGesture()).toBe(true)
  })

  it('the idle hover re-run after each captured key keeps the typed readout (the key router re-hovers)', () => {
    const scene = makeRetypeScene()
    const { tool, onMeasurement } = makeTool(scene)
    drawGround(tool, [0, 0], [1, 1])
    key(tool, '3')
    onMeasurement.mockClear()
    tool.onPointerMove(makeSnap({ x: 2, y: 2, z: 0 }), rayThrough(2, 2))
    expect(onMeasurement).not.toHaveBeenCalledWith('')
    // Face-mode idle hover too.
    const faceScene = makeRetypeScene({ pick: () => makePick(7n, 3n) })
    const f = makeTool(faceScene)
    f.tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 1, kind: 'face' }), rayThrough(0, 0))
    f.tool.onPointerDown(makeSnap({ x: 1, y: 2, z: 1, kind: 'face' }), rayThrough(1, 2))
    key(f.tool, '3')
    f.onMeasurement.mockClear()
    f.tool.onPointerMove(makeSnap({ x: 0.5, y: 0.5, z: 1, kind: 'face' }), rayThrough(0.5, 0.5))
    expect(f.onMeasurement).not.toHaveBeenCalledWith('')
  })

  it('Escape closes the window without touching the rectangle', () => {
    const scene = makeRetypeScene()
    const { tool, onMeasurement } = makeTool(scene)
    drawGround(tool, [0, 0], [1, 1])
    key(tool, '3')
    key(tool, 'Escape')
    expect(onMeasurement).toHaveBeenLastCalledWith('')
    expect(tool.capturesKey('5')).toBe(false)
    expect(tool.hasArmedGesture()).toBe(false)
    // A stray Enter now does nothing.
    key(tool, 'Enter')
    expect(scene.scene_undo).not.toHaveBeenCalled()
  })

  it('a new first click closes the window; the digits then belong to the new rectangle', () => {
    const scene = makeRetypeScene()
    const { tool } = makeTool(scene)
    drawGround(tool, [0, 0], [1, 1])
    key(tool, '3')
    tool.onPointerDown(makeSnap({ x: 5, y: 5, z: 0 }), rayThrough(5, 5))
    expect(tool.capturingInput()).toBe(true) // anchored on the next rectangle
    typeDims(tool, '1,1')
    // The typed commit is a fresh rectangle from (5,5), not a retype.
    expect(scene.scene_undo).not.toHaveBeenCalled()
    expect(scene.draft_add_segment).toHaveBeenCalledTimes(8)
    expect(Math.min(...lastRectangle(scene).map((c) => c[0]))).toBeCloseTo(5, 9)
  })

  it('disarmRetype (the host\'s explicit undo/redo/delete hook) closes the window quietly', () => {
    const scene = makeRetypeScene()
    const { tool } = makeTool(scene)
    drawGround(tool, [0, 0], [1, 1])
    tool.disarmRetype()
    expect(tool.capturesKey('5')).toBe(false)
  })

  it('refuses the retype when the history generation moved (an intervening action) and says so', () => {
    const scene = makeRetypeScene()
    const { tool, onToast } = makeTool(scene)
    drawGround(tool, [0, 0], [1, 1])
    scene.__bump() // something else was recorded meanwhile
    typeDims(tool, '2,2')
    expect(scene.scene_undo).not.toHaveBeenCalled()
    expect(scene.draft_add_segment).toHaveBeenCalledTimes(4)
    expect(onToast).toHaveBeenCalledWith(expect.stringContaining('model changed'))
    // The window is gone.
    expect(tool.capturesKey('5')).toBe(false)
  })

  it('a refused re-commit is rolled forward with a redo, so the original rectangle survives', () => {
    const scene = makeRetypeScene()
    const { tool, onToast } = makeTool(scene)
    drawGround(tool, [0, 0], [1, 1])
    const add = scene.draft_add_segment as ReturnType<typeof vi.fn>
    add.mockImplementationOnce(() => { throw new Error('PointOffPlane: nope') })
    typeDims(tool, '2,2')
    expect(scene.scene_undo).toHaveBeenCalledTimes(1)
    expect(scene.scene_redo).toHaveBeenCalledTimes(1)
    expect(onToast).toHaveBeenCalledTimes(1)
    // Still open, re-stamped: a second attempt goes through.
    typeDims(tool, '2,2')
    expect(scene.scene_undo).toHaveBeenCalledTimes(2)
    expect(Math.max(...lastRectangle(scene).map((c) => c[0]))).toBeCloseTo(2, 9)
  })

  // Semantic change under unified drafting: a re-commit is ONE draft, so a
  // refused segment cancels the whole draft (draft_cancel) and records NOTHING
  // — there is no longer a "partial step" to retract. The refusal therefore
  // rolls forward with a plain redo (the old test expected two undos and a
  // fresh redraw of the original).
  it('a refused later segment of the re-commit cancels the whole draft (atomic) and the original is restored with one redo', () => {
    const scene = makeRetypeScene()
    const { tool, onToast } = makeTool(scene)
    drawGround(tool, [0, 0], [1, 1])
    // First segment of the retry lands, the second (6th call overall, after
    // the 4 of the original draw) is refused: the draft is cancelled.
    scene.__failAddAt(6)
    typeDims(tool, '2,2')
    expect(onToast).toHaveBeenCalledTimes(1)
    expect(scene.draft_cancel).toHaveBeenCalledTimes(1)
    // Only the original draft ever ended; the retry recorded nothing.
    expect(scene.draft_end).toHaveBeenCalledTimes(1)
    expect(scene.scene_undo).toHaveBeenCalledTimes(1)
    expect(scene.scene_redo).toHaveBeenCalledTimes(1)
    // Still open: a good retype now goes through.
    typeDims(tool, '3,3')
    expect(scene.scene_undo).toHaveBeenCalledTimes(2)
    expect(Math.max(...lastRectangle(scene).map((c) => c[0]))).toBeCloseTo(3, 9)
    expect(Math.max(...lastRectangle(scene).map((c) => c[1]))).toBeCloseTo(3, 9)
  })

  it('Escape with an open but untyped window is not armed: it closes the window quietly and is not consumed', () => {
    const scene = makeRetypeScene()
    const { tool } = makeTool(scene)
    drawGround(tool, [0, 0], [1, 1])
    expect(tool.hasArmedGesture()).toBe(false)
    key(tool, 'Escape')
    expect(tool.capturesKey('5')).toBe(false)
  })

  it('a face rectangle retypes through a fresh face draft with the loop rebuilt on the face plane', () => {
    const scene = makeRetypeScene({ pick: () => makePick(7n, 3n) })
    const { tool, onFaceImprint } = makeTool(scene)
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 1, kind: 'face' }), rayThrough(0, 0))
    tool.onPointerDown(makeSnap({ x: 1, y: 2, z: 1, kind: 'face' }), rayThrough(1, 2))
    expect(scene.draft_begin_face).toHaveBeenCalledTimes(1)

    typeDims(tool, '3,4')
    expect(scene.scene_undo).toHaveBeenCalledTimes(1)
    expect(scene.draft_begin_face).toHaveBeenCalledTimes(2)
    const [object, face] = (scene.draft_begin_face as ReturnType<typeof vi.fn>).mock.calls[1]
    expect(object).toBe(7n)
    expect(face).toBe(3n)
    const pts = draftLoop(scene, 1)
    const xs = [pts[0], pts[3], pts[6], pts[9]]
    const ys = [pts[1], pts[4], pts[7], pts[10]]
    const zs = [pts[2], pts[5], pts[8], pts[11]]
    expect(Math.min(...xs)).toBeCloseTo(0, 9)
    expect(Math.min(...ys)).toBeCloseTo(0, 9)
    // 3 × 4 on the +Z face: one extent is 3 and the other 4 (the face basis
    // fixes which axis is which; the growth direction is preserved either way).
    const extents = [Math.max(...xs), Math.max(...ys)].sort((a, b) => a - b)
    expect(extents[0]).toBeCloseTo(3, 9)
    expect(extents[1]).toBeCloseTo(4, 9)
    for (const z of zs) expect(z).toBeCloseTo(1, 9)
    expect(onFaceImprint).toHaveBeenCalledTimes(2)
  })

  it('a typed (Enter) commit arms the window too', () => {
    const scene = makeRetypeScene()
    const { tool } = makeTool(scene)
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0 }), rayThrough(0, 0))
    typeDims(tool, '1,1')
    expect(scene.draft_add_segment).toHaveBeenCalledTimes(4)
    typeDims(tool, '2,2')
    expect(scene.scene_undo).toHaveBeenCalledTimes(1)
    expect(Math.max(...lastRectangle(scene).map((c) => c[0]))).toBeCloseTo(2, 9)
  })
})

describe('RectangleTool — clicks on an edge or corner adopt the face through that point (inference-permissiveness fixes)', () => {
  /** `pick_face` misses on the exact ray (strict at the boundary) but hits
   *  face 3 of object 7 for any nudged direction — the boundary probe ring. */
  function boundaryScene(opts: { facePlane?: [number, number, number, number, number, number] } = {}) {
    const scene = makeWasmScene()
    const plane = opts.facePlane ?? [0, 0, 1, 0, 0, 1]
    ;(scene.pick_face as ReturnType<typeof vi.fn>).mockImplementation(
      (_ox: number, _oy: number, _oz: number, dx: number, dy: number, dz: number) =>
        dx === 0 && dy === 0 && dz === -1 ? undefined : makePick(7n, 3n),
    )
    ;(scene.face_plane as ReturnType<typeof vi.fn>).mockImplementation(() => new Float64Array(plane))
    ;(scene.face_normal as ReturnType<typeof vi.fn>).mockImplementation(() => new Float64Array(plane.slice(3)))
    return scene
  }

  it('a first click on an edge midpoint (the ray itself misses every face) anchors FACE mode on the neighbouring face, not the ground', () => {
    const scene = boundaryScene()
    const { tool } = makeTool(scene)
    tool.onPointerDown(
      makeSnap({ x: 0.5, y: 0, z: 1, kind: 'midpoint', object: 7n, element: 11n, elementKind: 'edge' }),
      rayThrough(0.5, 0),
    )
    tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 1, kind: 'endpoint', object: 7n, element: 2n, elementKind: 'vertex' }), rayThrough(1, 1))
    expect(scene.draft_begin_face).toHaveBeenCalledTimes(1)
    expect(scene.draft_begin_plane).not.toHaveBeenCalled()
  })

  it('a probed neighbour whose plane does NOT hold the snapped point is rejected (falls through to plane mode)', () => {
    const scene = boundaryScene({ facePlane: [0, 0, 5, 0, 0, 1] }) // a face at z = 5, not through the snap
    const { tool } = makeTool(scene)
    tool.onPointerDown(
      makeSnap({ x: 0.5, y: 0, z: 0, kind: 'midpoint', object: 7n, element: 11n, elementKind: 'edge' }),
      rayThrough(0.5, 0),
    )
    tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 0 }), rayThrough(1, 1))
    expect(scene.draft_begin_face).not.toHaveBeenCalled()
    expect(scene.draft_begin_plane).toHaveBeenCalledTimes(1)
    expect(scene.draft_add_segment).toHaveBeenCalledTimes(4)
  })

  it('a non-boundary snap over a miss stays in plane mode — the ring probe only runs for object vertices/edges', () => {
    const scene = boundaryScene()
    const { tool } = makeTool(scene)
    tool.onPointerDown(makeSnap({ x: 0.5, y: 0, z: 0, kind: 'ground' }), rayThrough(0.5, 0))
    expect(scene.pick_face).toHaveBeenCalledTimes(1)
    tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 0 }), rayThrough(1, 1))
    expect(scene.draft_add_segment).toHaveBeenCalledTimes(4)
  })

  it('Shift over an edge midpoint pins the FACE that edge belongs to, not the ground the plain pick fell through to', () => {
    const scene = boundaryScene({ facePlane: [0, 0, 0, 0, 1, 0] }) // a vertical wall, plane y = 0
    const { tool } = makeTool(scene)
    const edgeRay = rayThrough(0.5, 0)
    tool.snapConstraint(edgeRay)
    tool.onPointerMove(makeSnap({ x: 0.5, y: 0, z: 0.5, kind: 'midpoint', object: 7n, element: 11n, elementKind: 'edge' }), edgeRay)
    tool.setShiftHeld(true); tool.setShiftHeld(false)
    expect(tool.statusHint()).toMatch(/Pinned/)
    expect(tool.snapConstraint(rayThrough(5, 5))?.constraintPlane).toEqual({ point: [0, 0, 0], normal: [0, 1, 0] })
  })
})

describe('RectangleTool — face mode honours the snap (the chip and the rectangle agree)', () => {
  it('the opposite corner lands on the snapped Endpoint exactly, not on the ray\'s own plane hit a few mm off', () => {
    const scene = makeWasmScene({ pick: () => makePick(7n, 3n) })
    const { tool } = makeTool(scene)
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 1, kind: 'on-face' }), rayThrough(0, 0))
    // The cursor ray pierces the plane at (1.004, 2.003) but the kernel
    // snapped the corner at (1, 2) — the corner wins.
    tool.onPointerDown(
      makeSnap({ x: 1, y: 2, z: 1, kind: 'endpoint', object: 7n, element: 2n, elementKind: 'vertex' }),
      rayThrough(1.004, 2.003),
    )
    const pts = Array.from(draftLoop(scene))
    expect(pts.slice(6, 9)).toEqual([1, 2, 1]) // the far corner
  })

  it('with no snap the ray∩plane point is still used (past the face\'s extent)', () => {
    const scene = makeWasmScene({ pick: () => makePick(7n, 3n) })
    const { tool } = makeTool(scene)
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 1, kind: 'on-face' }), rayThrough(0, 0))
    tool.onPointerDown(null, rayThrough(1.5, 2.5))
    const pts = Array.from(draftLoop(scene))
    expect(pts[6]).toBeCloseTo(1.5)
    expect(pts[7]).toBeCloseTo(2.5)
    expect(pts[8]).toBeCloseTo(1)
  })
})

describe('RectangleTool — Shift pins the hovered plane (GitHub issue 14)', () => {
  it('Shift over a face pins its plane: hovers elsewhere are constrained to it and the first click anchors PLANE mode on it', () => {
    const scene = makeWasmScene({ pick: () => undefined })
    const faceRay = rayThrough(0, 0)
    // Over the face: pick hits, plane at z=1. Everywhere else: nothing.
    ;(scene.pick_face as ReturnType<typeof vi.fn>).mockImplementationOnce(() => makePick(7n, 3n))
    const { tool } = makeTool(scene)
    tool.snapConstraint(faceRay)
    tool.onPointerMove(makeSnap({ x: 0, y: 0, z: 1, kind: 'on-face', object: 7n, element: 3n, elementKind: 'face' }), faceRay)

    tool.setShiftHeld(true)
    tool.setShiftHeld(false)
    expect(tool.statusHint()).toMatch(/Pinned/)
    expect(tool.hasArmedGesture()).toBe(true)

    // Off the face, over empty ground: still constrained to the face plane.
    const groundRay = rayThrough(5, 5)
    expect(tool.snapConstraint(groundRay)?.constraintPlane).toEqual({ point: [0, 0, 1], normal: [0, 0, 1] })

    // The gesture lands on that plane as a sketch, not as a face imprint
    // and not on the ground.
    tool.onPointerDown(makeSnap({ x: 5, y: 5, z: 1, kind: 'plane' }), groundRay)
    expect(tool.statusHint()).toMatch(/opposite corner/)
    tool.onPointerDown(makeSnap({ x: 6, y: 7, z: 1, kind: 'plane' }), rayThrough(6, 7))
    expect(scene.draft_begin_face).not.toHaveBeenCalled()
    // One plane draft on the pinned z = 1 plane (not the ground, not a face).
    expect(scene.draft_begin_plane).toHaveBeenCalledTimes(1)
    const begin = (scene.draft_begin_plane as ReturnType<typeof vi.fn>).mock.calls[0]
    expect(begin[2]).toBeCloseTo(1, 9)
    expect(begin.slice(3, 6)).toEqual([0, 0, 1])
    expect(scene.draft_add_segment).toHaveBeenCalledTimes(4)
    const seg = (scene.draft_add_segment as ReturnType<typeof vi.fn>).mock.calls[0]
    expect(seg.slice(1, 7)).toEqual([5, 5, 1, 6, 5, 1])

    // The pin survives the completed gesture (like the arrow lock) …
    expect(tool.statusHint()).toMatch(/Pinned/)
    // … a second Shift press releases it; keydown autorepeat never toggles.
    tool.setShiftHeld(true)
    tool.setShiftHeld(true)
    expect(tool.statusHint()).not.toMatch(/Pinned/)
    tool.setShiftHeld(false)
    expect(tool.snapConstraint(groundRay)).toBeNull()
  })

  it('Escape while idle releases the pin; an arrow lock displaces it; Escape mid-gesture keeps it (like the arrow lock)', () => {
    const scene = makeWasmScene({ pick: () => makePick(7n, 3n) })
    const { tool } = makeTool(scene)
    const faceRay = rayThrough(0, 0)
    tool.onPointerMove(makeSnap({ x: 0, y: 0, z: 1, kind: 'on-face' }), faceRay)
    tool.setShiftHeld(true); tool.setShiftHeld(false)
    expect(tool.statusHint()).toMatch(/Pinned/)
    // Mid-gesture Escape aborts the rectangle but keeps the pin.
    tool.onPointerDown(makeSnap({ x: 5, y: 5, z: 1, kind: 'plane' }), rayThrough(5, 5))
    expect(tool.statusHint()).toMatch(/opposite corner/)
    tool.onKey({ key: 'Escape' } as KeyboardEvent)
    expect(tool.statusHint()).toMatch(/Pinned/)
    // Idle Escape releases it.
    tool.onKey({ key: 'Escape' } as KeyboardEvent)
    expect(tool.statusHint()).not.toMatch(/Pinned/)

    tool.onPointerMove(makeSnap({ x: 0, y: 0, z: 1, kind: 'on-face' }), faceRay)
    tool.setShiftHeld(true); tool.setShiftHeld(false)
    expect(tool.statusHint()).toMatch(/Pinned/)
    tool.onKey({ key: 'ArrowUp' } as KeyboardEvent)
    expect(tool.statusHint()).toMatch(/Locked to the blue plane/)
    expect(tool.statusHint()).not.toMatch(/Pinned/)
  })
})

// A typed W,D right after the first click — before the cursor has moved off
// the anchor (a click on a snapped guide intersection leaves it exactly
// there) — grows the way the face reads on screen: right and up as seen from
// outside it, +X then +Y on a horizontal face. It must not depend on the
// arbitrary handedness of the face's basis axes, which on a −Y wall points
// the width toward −X (off the left end of a face the click sits near).
describe('RectangleTool — typed size before any cursor movement grows right and up', () => {
  function typeOnFace(normal: [number, number, number], anchor: V3Tuple) {
    const scene = makeWasmScene({ pick: () => makePick(7n, 3n) })
    ;(scene.face_normal as ReturnType<typeof vi.fn>).mockReturnValue(new Float64Array(normal))
    ;(scene.face_plane as ReturnType<typeof vi.fn>).mockReturnValue(new Float64Array([...anchor, ...normal]))
    const { tool } = makeTool(scene)
    tool.onPointerDown(
      makeSnap({ x: anchor[0], y: anchor[1], z: anchor[2], kind: 'face' }),
      { origin: [anchor[0] - normal[0] * 5, anchor[1] - normal[1] * 5, anchor[2] - normal[2] * 5], direction: [normal[0] * -1, normal[1] * -1, normal[2] * -1] },
    )
    for (const ch of '3,2') tool.onKey({ key: ch } as KeyboardEvent)
    tool.onKey({ key: 'Enter' } as KeyboardEvent)
    expect(scene.draft_begin_face).toHaveBeenCalledTimes(1)
    const pts = draftLoop(scene)
    const axis = (k: number) => [pts[k], pts[3 + k], pts[6 + k], pts[9 + k]]
    const range = (k: number): [number, number] => [Math.min(...axis(k)), Math.max(...axis(k))]
    return { x: range(0), y: range(1), z: range(2) }
  }
  type V3Tuple = [number, number, number]

  it('on a −Y wall (the front face) the width grows toward +X and the height toward +Z', () => {
    const r = typeOnFace([0, -1, 0], [1, 0, 1])
    expect(r.x[0]).toBeCloseTo(1, 9)
    expect(r.x[1]).toBeCloseTo(4, 9)
    expect(r.z[0]).toBeCloseTo(1, 9)
    expect(r.z[1]).toBeCloseTo(3, 9)
  })

  it('on a +Y wall (seen from behind) the width grows toward −X, which is right as seen from outside', () => {
    const r = typeOnFace([0, 1, 0], [1, 0, 1])
    expect(r.x[0]).toBeCloseTo(-2, 9)
    expect(r.x[1]).toBeCloseTo(1, 9)
    expect(r.z[1]).toBeCloseTo(3, 9)
  })

  it('on a +X wall the width grows toward +Y and the height toward +Z', () => {
    const r = typeOnFace([1, 0, 0], [0, 1, 1])
    expect(r.y[0]).toBeCloseTo(1, 9)
    expect(r.y[1]).toBeCloseTo(4, 9)
    expect(r.z[0]).toBeCloseTo(1, 9)
    expect(r.z[1]).toBeCloseTo(3, 9)
  })

  it('on a top face the width grows toward +X and the depth toward +Y, like the ground', () => {
    const r = typeOnFace([0, 0, 1], [1, 1, 0])
    expect(r.x[0]).toBeCloseTo(1, 9)
    expect(r.x[1]).toBeCloseTo(4, 9)
    expect(r.y[0]).toBeCloseTo(1, 9)
    expect(r.y[1]).toBeCloseTo(3, 9)
  })
})


describe('RectangleTool — a clicked value is exactly the displayed value', () => {
  // These pin the Exact digits (9.399 mm); the default mm rounding is coarser.
  beforeAll(() => setPrecision('exact', 'mm'))
  afterAll(() => setPrecision(DEFAULT_PRECISION.mm, 'mm'))
  const calls = (scene: WasmScene) => (scene.draft_add_segment as ReturnType<typeof vi.fn>).mock.calls
  const corners = (scene: WasmScene) => calls(scene).slice(-4).map((c) => [c[1] as number, c[2] as number, c[3] as number])
  const prev = getLengthUnit()
  function withMm<T>(fn: () => T): T {
    setLengthUnit('mm')
    try {
      return fn()
    } finally {
      setLengthUnit(prev)
    }
  }

  it('free ground click at an off-grid cursor commits the same corners as typing "9.399,3.787"', () => {
    withMm(() => {
      const clicked = makeWasmScene()
      const a = makeTool(clicked)
      a.tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 0 }), rayThrough(1, 1))
      a.tool.onPointerMove(makeSnap({ x: 1.00939876, y: 1.00378732, z: 0 }), rayThrough(1.00939876, 1.00378732))
      a.tool.onPointerDown(makeSnap({ x: 1.00939876, y: 1.00378732, z: 0 }), rayThrough(1.00939876, 1.00378732))
      expect(a.onMeasurement).toHaveBeenCalledWith('9.399 mm × 3.787 mm')

      const typed = makeWasmScene()
      const b = makeTool(typed)
      b.tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 0 }), rayThrough(1, 1))
      b.tool.onPointerMove(makeSnap({ x: 1.00939876, y: 1.00378732, z: 0 }), rayThrough(1.00939876, 1.00378732))
      for (const ch of '9.399,3.787') b.tool.onKey({ key: ch } as KeyboardEvent)
      b.tool.onKey({ key: 'Enter' } as KeyboardEvent)

      expect(calls(clicked).length).toBe(4)
      expect(corners(clicked)).toEqual(corners(typed))
      // Bit-for-bit: far corner offset equals the typed 9.399 mm / 3.787 mm.
      const xs = corners(clicked).map((c) => c[0])
      expect(Math.max(...xs)).toBe(1 + 9.399 * 0.001)
    })
  })

  it('a hard-snapped (endpoint) ground click keeps the snapped point unchanged', () => {
    withMm(() => {
      const scene = makeWasmScene()
      const { tool } = makeTool(scene)
      tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 0 }), rayThrough(1, 1))
      tool.onPointerDown(makeSnap({ x: 1.00939876, y: 1.00378732, z: 0, kind: 'endpoint' }), rayThrough(1.00939876, 1.00378732))
      const far = corners(scene)[2]
      expect(far[0]).toBe(1.00939876)
      expect(far[1]).toBe(1.00378732)
    })
  })

  it('free face click commits the displayed extents; endpoint keeps the point', () => {
    withMm(() => {
      const scene = makeWasmScene({ pick: () => makePick(7n, 3n) })
      const { tool } = makeTool(scene)
      tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 1, kind: 'face' }), rayThrough(0, 0))
      const s = makeSnap({ x: 0.00939876, y: 0.00378732, z: 1, kind: 'on-face' })
      tool.onPointerMove(s, rayThrough(s.x, s.y))
      tool.onPointerDown(s, rayThrough(s.x, s.y))
      const loop = draftLoop(scene)
      const xs = [0, 3, 6, 9].map((i) => loop[i])
      const ys = [1, 4, 7, 10].map((i) => loop[i])
      expect(Math.max(...xs)).toBe(9.399 * 0.001)
      expect(Math.max(...ys)).toBe(3.787 * 0.001)

      const scene2 = makeWasmScene({ pick: () => makePick(7n, 3n) })
      const t2 = makeTool(scene2).tool
      t2.onPointerDown(makeSnap({ x: 0, y: 0, z: 1, kind: 'face' }), rayThrough(0, 0))
      const e = makeSnap({ x: 0.00939876, y: 0.00378732, z: 1, kind: 'endpoint' })
      t2.onPointerMove(e, rayThrough(e.x, e.y))
      t2.onPointerDown(e, rayThrough(e.x, e.y))
      const loop2 = draftLoop(scene2)
      expect(Math.max(...[0, 3, 6, 9].map((i) => loop2[i]))).toBe(0.00939876)
    })
  })
})

// #51a audit: the rectangle's plane is fixed by the first corner and the
// cursor only says which way each side grows, so a Blue hover straight over
// a ground corner (flattened onto it) still draws the typed W,D on the
// ground, growing +X/+Y like a typed size before any cursor movement.
describe('RectangleTool — typed size with the cursor on the Blue axis (#51a)', () => {
  it('draws the typed W,D on the ground plane instead of doing nothing', () => {
    const scene = makeWasmScene()
    const { tool, onCommit, onToast } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 1, y: 2, z: 0 }), rayThrough(1, 2))
    tool.onPointerMove(makeSnap({ x: 1, y: 2, z: 3, kind: 'on-axis', direction: [0, 0, 1] }), rayThrough(1, 2))
    for (const ch of '3,2') tool.onKey({ key: ch } as KeyboardEvent)
    tool.onKey({ key: 'Enter' } as KeyboardEvent)

    expect(onToast).not.toHaveBeenCalled()
    expect(onCommit).toHaveBeenCalledTimes(1)
    const pts = draftLoop(scene)
    const xs = [pts[0], pts[3], pts[6], pts[9]]
    const ys = [pts[1], pts[4], pts[7], pts[10]]
    const zs = [pts[2], pts[5], pts[8], pts[11]]
    expect([Math.min(...xs), Math.max(...xs)]).toEqual([1, 4])
    expect([Math.min(...ys), Math.max(...ys)]).toEqual([2, 4])
    expect(zs).toEqual([0, 0, 0, 0])
  })
})

describe('RectangleTool — X-ray: a hidden snap off the face drops onto it, disclosed (cursorOnFrozenPlane)', () => {
  const down = (x: number, y: number): Ray => ({ origin: [x, y, 5], direction: [0, 0, -1] })
  // Runs under the mode it is named for.
  beforeEach(() => setPickRenderMode('xray'))
  afterEach(() => setPickRenderMode('shaded'))

  it('opts in to see-through points, lands on the point dropped onto the face (not the ray hit), says "projected", and clears it when idle', () => {
    const scene = makeWasmScene({ pick: () => makePick(7n, 3n) })
    const { tool } = makeTool(scene)
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 1, kind: 'on-face' }), down(0, 0))
    expect(tool.snapConstraint(down(0, 0))?.seeThroughPoints).toBe(true)
    // Shaded's own case is unchanged: an on-face snap is taken exactly.
    tool.onPointerMove(makeSnap({ x: 1, y: 0.5, z: 1, kind: 'endpoint' }), down(1, 0.5))
    expect(tool.snapProjected()).toBe(false)
    // A hidden midpoint 0.4 below the face, the cursor ray elsewhere.
    const hidden = makeSnap({ x: 1, y: 0.5, z: 0.6, kind: 'midpoint' })
    tool.onPointerMove(hidden, down(1.2, 0.5))
    expect(tool.snapProjected()).toBe(true)
    tool.onPointerDown(hidden, down(1.2, 0.5))
    const segs = (scene.draft_add_segment as ReturnType<typeof vi.fn>).mock.calls.map((c) => c.slice(1, 7) as number[])
    expect(segs.length).toBeGreaterThan(0)
    for (const s of segs) {
      expect(s[2]).toBeCloseTo(1, 12)
      expect(s[5]).toBeCloseTo(1, 12)
    }
    // The far corner is the hidden point dropped straight onto the face.
    const far = Array.from(draftLoop(scene)).slice(6, 9)
    expect(far[0]).toBeCloseTo(1, 12)
    expect(far[1]).toBeCloseTo(0.5, 12)

    // Back to idle: no stale "projected" on the next ordinary hover.
    expect(tool.snapProjected()).toBe(false)
    tool.onPointerMove(makeSnap({ x: 0.3, y: 0.3, z: 1, kind: 'on-face' }), down(0.3, 0.3))
    expect(tool.snapProjected()).toBe(false)
  })
})

describe('RectangleTool — X-ray: a hidden point that is no part of the face, clicked first, drops onto the face (firstPointOnFace)', () => {
  const down = (x: number, y: number): Ray => ({ origin: [x, y, 5], direction: [0, 0, -1] })
  // Runs under the mode it is named for.
  beforeEach(() => setPickRenderMode('xray'))
  afterEach(() => setPickRenderMode('shaded'))

  it('the idle hover says "projected", the first click anchors ON the face plane, and the gesture stays on it', () => {
    const scene = makeWasmScene({ pick: () => makePick(7n, 3n) })
    const { tool } = makeTool(scene)
    // A crossing 0.4 below the top face: no vertex or edge of the face, so
    // no face is adopted through it.
    const crossing = makeSnap({ x: 1, y: 0.5, z: 0.6, kind: 'intersection' })
    tool.onPointerMove(crossing, down(1.2, 0.5))
    expect(tool.snapProjected(), 'the hover says what the click will do').toBe(true)
    tool.onPointerDown(crossing, down(1.2, 0.5))
    expect(tool.snapProjected(), 'the click dropped it').toBe(true)
    tool.onPointerDown(makeSnap({ x: 1.6, y: 0.9, z: 1, kind: 'on-face' }), down(1.6, 0.9))
    const segs = (scene.draft_add_segment as ReturnType<typeof vi.fn>).mock.calls.map((c) => c.slice(1, 7) as number[])
    expect(segs.length).toBeGreaterThan(0)
    for (const s of segs) {
      expect(s[2]).toBeCloseTo(1, 12)
      expect(s[5]).toBeCloseTo(1, 12)
    }
    // The rectangle's first corner is the crossing dropped onto the face.
    expect(Array.from(draftLoop(scene)).slice(0, 3).map((v) => Number(v.toFixed(12)))).toEqual([1, 0.5, 1])

    // An ordinary on-face hover is not "projected".
    tool.onPointerMove(makeSnap({ x: 0.3, y: 0.3, z: 1, kind: 'on-face' }), down(0.3, 0.3))
    expect(tool.snapProjected()).toBe(false)
  })

  it('the idle hover over a face opts in to hidden points only in X-ray', () => {
    const scene = makeWasmScene({ pick: () => makePick(7n, 3n) })
    const { tool } = makeTool(scene)
    try {
      for (const [mode, expected] of [['shaded', false], ['wireframe', false], ['xray', true]] as const) {
        setPickRenderMode(mode)
        expect(tool.snapConstraint(down(0.5, 0.5))?.seeThroughPoints ?? false, mode).toBe(expected)
      }
    } finally {
      setPickRenderMode('shaded')
    }
  })
})

// ---------------------------------------------------------------------------
// The boundary pick's hover memory under an arrow lock or a Shift pin
// (view-plane design §4.2 rule 2): a lock or pin skips the face pick, but
// the memory must keep following the cursor, so a release at a corner
// remembers the face the cursor crossed last, not the one before the lock.
// ---------------------------------------------------------------------------

describe('RectangleTool — hover memory keeps tracking under a lock or pin', () => {
  type Vec = [number, number, number]
  const sub = (a: Vec, b: Vec): Vec => [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
  const add = (a: Vec, b: Vec): Vec => [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
  const mul = (a: Vec, k: number): Vec => [a[0] * k, a[1] * k, a[2] * k]
  const dot = (a: Vec, b: Vec): number => a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
  const cross = (a: Vec, b: Vec): Vec => [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
  const unit = (a: Vec): Vec => mul(a, 1 / Math.hypot(a[0], a[1], a[2]))

  const CUBE = 7n
  const FRONT = 1n
  const RIGHT = 4n
  const TOP = 6n
  /** The unit cube [0,1]³, each face wound counter-clockwise about its outward normal. */
  const FACES: { face: bigint; verts: Vec[] }[] = [
    { face: FRONT, verts: [[0, 0, 0], [1, 0, 0], [1, 0, 1], [0, 0, 1]] },
    { face: 2n, verts: [[0, 1, 0], [0, 1, 1], [1, 1, 1], [1, 1, 0]] },
    { face: 3n, verts: [[0, 0, 0], [0, 0, 1], [0, 1, 1], [0, 1, 0]] },
    { face: RIGHT, verts: [[1, 0, 0], [1, 1, 0], [1, 1, 1], [1, 0, 1]] },
    { face: 5n, verts: [[0, 0, 0], [0, 1, 0], [1, 1, 0], [1, 0, 0]] },
    { face: TOP, verts: [[0, 0, 1], [1, 0, 1], [1, 1, 1], [0, 1, 1]] },
  ]
  const normalOf = (f: { verts: Vec[] }): Vec => unit(cross(sub(f.verts[1], f.verts[0]), sub(f.verts[2], f.verts[0])))

  /** A scene double that really casts against the cube: `pick_face` is the
   *  nearest face the ray crosses STRICTLY inside (a ray through a corner
   *  misses, as the kernel's does). */
  function castingScene(): WasmScene {
    const byFace = new Map(FACES.map((f) => [f.face, f]))
    return {
      history_generation: vi.fn(() => 1n),
      ...draftMock().members,
      axes: vi.fn(() => new Float64Array([0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1])),
      pick_sketch: vi.fn(() => undefined),
      sketch_plane: vi.fn(() => undefined),
      node_parent: vi.fn(() => undefined),
      pick_face: vi.fn((ox: number, oy: number, oz: number, dx: number, dy: number, dz: number) => {
        const o: Vec = [ox, oy, oz]
        const d = unit([dx, dy, dz])
        let best: { face: bigint; t: number } | null = null
        for (const f of FACES) {
          const n = normalOf(f)
          const den = dot(d, n)
          if (Math.abs(den) < 1e-12) continue
          const t = dot(sub(f.verts[0], o), n) / den
          if (t <= 0) continue
          const hit = add(o, mul(d, t))
          let inside = true
          for (let i = 0; i < f.verts.length; i++) {
            const a = f.verts[i]
            const b = f.verts[(i + 1) % f.verts.length]
            if (dot(cross(sub(b, a), sub(hit, a)), n) <= 1e-12) { inside = false; break }
          }
          if (inside && (best === null || t < best.t)) best = { face: f.face, t }
        }
        if (best === null) return undefined
        return makePick(CUBE, best.face)
      }),
      face_normal: vi.fn((_o: bigint, face: bigint) => new Float64Array(normalOf(byFace.get(face)!))),
      face_plane: vi.fn((_o: bigint, face: bigint) => {
        const f = byFace.get(face)!
        return new Float64Array([...f.verts[0], ...normalOf(f)])
      }),
    } as unknown as WasmScene
  }

  /** The home view (eye offset [3.5, −3, 2.5] from the target), framing the cube. */
  const EYE: Vec = add([0.5, 0.5, 0.5], mul([3.5, -3.0, 2.5], 2))
  const N: Vec = [1, 0, 1] // the corner the front, right and top faces share

  function setup() {
    const scene = castingScene()
    const { tool } = makeTool(scene)
    const cam = new THREE.PerspectiveCamera(45, 1.5, 0.1, 1000)
    cam.position.set(...EYE)
    cam.up.set(0, 0, 1)
    cam.lookAt(0.5, 0.5, 0.5)
    cam.updateMatrixWorld(true)
    tool.updateCamera(cam)
    /** One pointer event, in the Viewport's order: the snap constraint
     *  first, then the move. */
    const hoverAt = (p: Vec, snap: Snap) => {
      const ray: Ray = { origin: [...EYE], direction: unit(sub(p, EYE)) }
      tool.snapConstraint(ray)
      tool.onPointerMove(snap, ray)
      return ray
    }
    const hoverFace = (p: Vec) => hoverAt(p, makeSnap({ x: p[0], y: p[1], z: p[2], kind: 'face' }))
    const cornerSnap = makeSnap({ x: N[0], y: N[1], z: N[2], kind: 'endpoint', object: CUBE, elementKind: 'vertex' })
    /** Replay at the shared corner, the cursor exactly on it (inside the
     *  lean dead zone, so the hover memory decides): the face a click
     *  there would start on. */
    const cornerFace = () => {
      const ray = hoverAt(N, cornerSnap)
      const pick = (tool as unknown as { _facePickAt(s: Snap, r: Ray): { face: bigint } | null })._facePickAt(cornerSnap, ray)
      return pick?.face ?? null
    }
    const key = (k: string) => tool.onKey({ key: k } as KeyboardEvent)
    return { tool, hoverFace, cornerFace, key }
  }

  const ON_FRONT: Vec = [0.6, 0, 0.5]
  const ON_RIGHT: Vec = [1, 0.5, 0.5]

  it('unlocked control: the corner remembers the face the cursor came from', () => {
    const a = setup()
    a.hoverFace(ON_FRONT)
    expect(a.cornerFace()).toBe(FRONT)
    const b = setup()
    b.hoverFace(ON_FRONT)
    b.hoverFace(ON_RIGHT)
    expect(b.cornerFace()).toBe(RIGHT)
  })

  it('crossing a face under an arrow lock is remembered once the lock is released at the corner', () => {
    const { hoverFace, cornerFace, key } = setup()
    hoverFace(ON_FRONT)
    key('ArrowUp') // lock blue
    hoverFace(ON_RIGHT)
    key('ArrowUp') // release
    expect(cornerFace()).toBe(RIGHT)
  })

  it('…and under a Shift pin', () => {
    const { tool, hoverFace, cornerFace } = setup()
    hoverFace(ON_FRONT)
    tool.setShiftHeld(true) // pin the front face's plane
    tool.setShiftHeld(false)
    hoverFace(ON_RIGHT)
    tool.setShiftHeld(true) // unpin
    tool.setShiftHeld(false)
    expect(cornerFace()).toBe(RIGHT)
  })
})

describe('RectangleTool — a document reset forgets memoized picks', () => {
  it('onDocumentReset clears the face pick cache', () => {
    const { tool } = makeTool(makeWasmScene())
    const cache = (tool as unknown as { _pickCache: { clear(): void } })._pickCache
    const clear = vi.spyOn(cache, 'clear')
    tool.onDocumentReset()
    expect(clear).toHaveBeenCalled()
  })
})

// Over a standing sketch's filled interior (view-plane design §2 step 4b):
// the hover adopts that sketch's plane, so Shift pins IT (not the ground or
// the plane the view faces) and a click draws into it.
describe('RectangleTool — hovering inside a standing sketch\'s fill', () => {
  const RED_SKETCH = 61n
  /** From the Front side, off-centre, into a rectangle standing on the red
   *  plane x = 0: it crosses that plane at (0, 0.9, 0.37). */
  const FILL_RAY: Ray = { origin: [3, -3, 0.5], direction: [-3, 3.9, -0.13] }
  const FILL_HIT = makeSnap({ x: 0, y: 0.9, z: 0.37, kind: 'plane' })

  function fillScene(): WasmScene {
    const scene = makeWasmScene({ pick: () => undefined }) as unknown as Record<string, unknown>
    scene.sketch_plane = vi.fn((h: bigint) =>
      new Float64Array(h === RED_SKETCH ? [0, 0, 0, 1, 0, 0] : [0, 0, 0, 0, 0, 1]))
    scene.sketch_lines = vi.fn(() => new Float32Array())
    scene.axes = vi.fn(() => new Float64Array([0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1]))
    // The fill pick hits the red rectangle wherever the ray goes.
    scene.pick_sketch_region = vi.fn(() => ({
      sketch: () => RED_SKETCH, region: () => 1n, depth: () => 5, detached: () => false, free: vi.fn(),
    }))
    return scene as unknown as WasmScene
  }

  /** A level Front camera, so over nothing the tool would infer green. */
  function frontCamera(): THREE.PerspectiveCamera {
    const cam = new THREE.PerspectiveCamera(45, 4 / 3, 0.1, 1000)
    cam.position.set(0, -8, 0.5)
    cam.up.set(0, 0, 1)
    cam.lookAt(0, 0, 0.5)
    cam.updateMatrixWorld(true)
    return cam
  }

  it('Shift pins the sketch\'s plane: constrained to x = 0, and the click draws on it', () => {
    const scene = fillScene()
    const { tool } = makeTool(scene)
    tool.updateCamera(frontCamera())
    tool.snapConstraint(FILL_RAY)
    tool.onPointerMove(FILL_HIT, FILL_RAY)
    expect(tool.statusHint()).not.toMatch(/Facing/)

    tool.setShiftHeld(true)
    tool.setShiftHeld(false)
    expect(tool.statusHint()).toContain('Pinned to the hovered plane')
    const normal = tool.snapConstraint(FILL_RAY)?.constraintPlane?.normal
    expect(normal).toBeDefined()
    expect(Math.abs(normal![0])).toBeCloseTo(1, 12)
    expect(normal![1]).toBeCloseTo(0, 12)
    expect(normal![2]).toBeCloseTo(0, 12)

    tool.onPointerDown(FILL_HIT, FILL_RAY)
    tool.onPointerDown(makeSnap({ x: 0, y: 1.2, z: 0.45, kind: 'plane' }), FILL_RAY)
    expect(scene.draft_begin_plane).toHaveBeenCalledTimes(1)
    const begin = (scene.draft_begin_plane as ReturnType<typeof vi.fn>).mock.calls[0] as number[]
    expect(begin[0]).toBeCloseTo(0, 12) // on x = 0
    expect(Math.abs(begin[3])).toBeCloseTo(1, 12)
    expect(scene.draft_add_segment).toHaveBeenCalledTimes(4)
  })

  it('unpinned, a click inside the fill draws into that sketch (sketch target)', () => {
    const scene = fillScene()
    const { tool } = makeTool(scene)
    tool.updateCamera(frontCamera())
    tool.onPointerMove(FILL_HIT, FILL_RAY)
    tool.onPointerDown(FILL_HIT, FILL_RAY)
    tool.onPointerDown(makeSnap({ x: 0, y: 1.2, z: 0.45, kind: 'plane' }), FILL_RAY)
    expect(scene.draft_begin_sketch).toHaveBeenCalledTimes(1)
    expect((scene.draft_begin_sketch as ReturnType<typeof vi.fn>).mock.calls[0][0]).toBe(RED_SKETCH)
    expect(scene.draft_begin_plane).not.toHaveBeenCalled()
  })
})

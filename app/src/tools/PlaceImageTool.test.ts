import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import * as THREE from 'three'
import { PlaceImageTool, pictureFrame, pictureRect, parsePlacedImage, PICTURE_MIN_SIDE_M, PICTURE_MAX_SIDE_M, type ImagePlacement } from './PlaceImageTool'
import type { Snap } from './types'
import type { Ray } from '../viewport/math'
import type { Scene as WasmScene } from '../wasm/loader'
import { getLengthUnit, setLengthUnit } from '../settings/units'

/** Straight down onto the ground from above (x, y). */
const down = (x = 0, y = 0): Ray => ({ origin: [x, y, 10], direction: [0, 0, -1] })
const ground = (x: number, y: number): Snap => ({ x, y, z: 0, kind: 'ground' })
const key = (k: string, repeat = false): KeyboardEvent =>
  ({ key: k, repeat, preventDefault: () => {} }) as unknown as KeyboardEvent

function makePick(object: bigint, face: bigint, instance?: bigint) {
  return { object: () => object, face: () => face, instance: () => instance, depth: () => 1, free: vi.fn() }
}

/** A world-axis drawing frame; `pick` is the face under every ray (none by
 *  default), its plane `facePlane` (point + normal). */
function makeScene(opts: {
  pick?: ReturnType<typeof makePick>
  facePlane?: number[]
  placeThrows?: Error
  /** The struck instance's pose (row-major 3x4), when the pick has one. */
  instancePose?: number[]
} = {}) {
  const place = vi.fn((..._args: unknown[]) => {
    if (opts.placeThrows) throw opts.placeThrows
    return { definition: 7n, instance: 9n, width: 2, height: 1 }
  })
  const scene = {
    axes: vi.fn(() => new Float64Array([0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1])),
    pick_face: vi.fn(() => opts.pick),
    face_normal: vi.fn(() => new Float64Array((opts.facePlane ?? [0, 0, 0, 0, 0, 1]).slice(3))),
    face_plane: vi.fn(() => new Float64Array(opts.facePlane ?? [0, 0, 0, 0, 0, 1])),
    instance_pose: vi.fn(() => (opts.instancePose ? new Float64Array(opts.instancePose) : undefined)),
    place_image: place,
  } as unknown as WasmScene
  return { scene, place }
}

function placement(): ImagePlacement {
  // 400 x 200 pixels: twice as wide as tall.
  return { bytes: new Uint8Array([1, 2, 3]), format: 0, name: 'Plan', pixelWidth: 400, pixelHeight: 200, texture: null }
}

function makeTool(opts: Parameters<typeof makeScene>[0] = {}) {
  const { scene, place } = makeScene(opts)
  const preview = new THREE.Group()
  const placed = vi.fn()
  const done = vi.fn()
  const toast = vi.fn()
  const measure = vi.fn()
  const tool = new PlaceImageTool(scene, preview, placement(), placed, done, toast, measure)
  return { tool, place, placed, done, toast, measure, preview }
}

const close = (a: number[], b: number[]) => a.every((v, i) => Math.abs(v - b[i]) < 1e-9)

let savedUnit: ReturnType<typeof getLengthUnit>
beforeEach(() => { savedUnit = getLengthUnit(); setLengthUnit('m') })
afterEach(() => setLengthUnit(savedUnit))

describe('pictureFrame', () => {
  it('lays the picture on the ground reading upright from above: right along +X, up along +Y', () => {
    const f = pictureFrame([0, 0, 0], [0, 0, 1], [0, 0, -1], [0, 0, 1], [0, 1, 0])!
    expect(close(f.u, [1, 0, 0]) && close(f.v, [0, 1, 0]) && close(f.normal, [0, 0, 1])).toBe(true)
  })

  it('stands the picture upright on a wall, facing the viewer', () => {
    // A wall whose normal faces away from a viewer looking along +Y: the
    // picture turns to face the viewer, its up is +Z, its right +X.
    const f = pictureFrame([0, 5, 0], [0, 1, 0], [0, 1, 0], [0, 0, 1], [0, 1, 0])!
    expect(close(f.normal, [0, -1, 0])).toBe(true)
    expect(close(f.v, [0, 0, 1])).toBe(true)
    expect(close(f.u, [1, 0, 0])).toBe(true)
  })
})

describe('pictureRect', () => {
  const f = pictureFrame([0, 0, 0], [0, 0, 1], [0, 0, -1], [0, 0, 1], [0, 1, 0])!

  it('keeps the aspect: the width follows the cursor along u, the height follows from it', () => {
    const r = pictureRect(f, [1, 1, 0], [4, 1.2, 0], 2)!
    expect(r.width).toBeCloseTo(3)
    expect(r.height).toBeCloseTo(1.5)
    expect(close(r.origin, [1, 1, 0])).toBe(true)
  })

  it('flips to whichever side of the corner the cursor is on, without mirroring', () => {
    const r = pictureRect(f, [1, 1, 0], [-1, -0.5, 0], 2)!
    expect(r.width).toBeCloseTo(2)
    expect(r.height).toBeCloseTo(1)
    // Lower-left corner moved left by the width and down by the height.
    expect(close(r.origin, [-1, 0, 0])).toBe(true)
    expect([r.su, r.sv]).toEqual([-1, -1])
  })

  it('has no size until the cursor leaves the corner along u', () => {
    expect(pictureRect(f, [1, 1, 0], [1, 3, 0], 2)).toBeNull()
  })

  it('takes a typed width on the side the cursor shows', () => {
    const r = pictureRect(f, [0, 0, 0], [-0.1, 0.1, 0], 2, 4)!
    expect(r.width).toBe(4)
    expect(close(r.origin, [-4, 0, 0])).toBe(true)
  })
})

describe('PlaceImageTool', () => {
  it('shows a 1 m ghost at the cursor before the first click', () => {
    const { tool } = makeTool()
    tool.onPointerMove(ground(2, 3), down(2, 3))
    const c = tool.ghostCorners()!
    expect(close(c[0], [2, 3, 0])).toBe(true)
    expect(close(c[2], [3, 3.5, 0])).toBe(true)
    expect(tool.statusHint()).toContain('Click the first corner of "Plan"')
  })

  it('click, drag, click places one picture with its aspect locked', () => {
    const { tool, place, placed, done, measure } = makeTool()
    tool.onPointerDown(ground(1, 1), down(1, 1))
    expect(tool.debugState().stage).toBe('anchored')
    expect(tool.statusHint()).toContain('opposite corner')
    tool.onPointerMove(ground(3, 5), down(3, 5))
    expect(measure).toHaveBeenLastCalledWith(expect.stringContaining('2'))
    const c = tool.ghostCorners()!
    expect(close(c[2], [3, 2, 0])).toBe(true) // 2 m wide, 1 m tall
    tool.onPointerDown(ground(3, 5), down(3, 5))
    expect(place).toHaveBeenCalledTimes(1)
    const [bytes, format, name, origin, u, v, width] = place.mock.calls[0]
    expect(bytes).toEqual(new Uint8Array([1, 2, 3]))
    expect(format).toBe(0)
    expect(name).toBe('Plan')
    expect(Array.from(origin as Float64Array)).toEqual([1, 1, 0])
    expect(Array.from(u as Float64Array)).toEqual([1, 0, 0])
    expect(Array.from(v as Float64Array)).toEqual([0, 1, 0])
    expect(width).toBeCloseTo(2)
    expect(placed).toHaveBeenCalledWith({ definition: 7n, instance: 9n, width: 2, height: 1 })
    expect(done).toHaveBeenCalledWith(true, true)
    expect(tool.hasArmedGesture()).toBe(false)
  })

  it('flips the placement to the side the cursor is on', () => {
    const { tool, place } = makeTool()
    tool.onPointerDown(ground(1, 1), down(1, 1))
    tool.onPointerDown(ground(-1, 0), down(-1, 0))
    const [, , , origin, , , width] = place.mock.calls[0]
    expect(width).toBeCloseTo(2)
    expect(Array.from(origin as Float64Array)).toEqual([-1, 0, 0])
  })

  it('a typed width + Enter commits at that width', () => {
    const { tool, place, measure } = makeTool()
    tool.onPointerDown(ground(0, 0), down(0, 0))
    tool.onPointerMove(ground(0.5, 0.1), down(0.5, 0.1))
    expect(tool.capturingInput()).toBe(true)
    tool.onKey(key('3'))
    expect(measure).toHaveBeenLastCalledWith(expect.stringContaining('3'))
    tool.onKey(key('Enter'))
    expect(place).toHaveBeenCalledTimes(1)
    expect(place.mock.calls[0][6]).toBe(3)
  })

  it('a typed width sizes the ghost at once, wherever the cursor goes', () => {
    const { tool, measure } = makeTool()
    tool.onPointerDown(ground(0, 0), down(0, 0))
    tool.onPointerMove(ground(0.5, 0.1), down(0.5, 0.1))
    tool.onKey(key('3'))
    // 3 m wide, 1.5 m tall, from the corner on the cursor's side.
    let c = tool.ghostCorners()!
    expect(close(c[0], [0, 0, 0]) && close(c[2], [3, 1.5, 0])).toBe(true)
    expect(measure).toHaveBeenLastCalledWith(expect.stringContaining('3'))
    // Moving keeps the typed width (and the readout); only the side follows.
    tool.onPointerMove(ground(-1, -0.2), down(-1, -0.2))
    c = tool.ghostCorners()!
    expect(close(c[0], [-3, -1.5, 0]) && close(c[2], [0, 0, 0])).toBe(true)
    expect(measure).toHaveBeenLastCalledWith(expect.stringContaining('3'))
    // Backspace to empty hands the ghost back to the cursor.
    tool.onKey(key('Backspace'))
    c = tool.ghostCorners()!
    expect(close(c[0], [-1, -0.5, 0])).toBe(true)
  })

  it('a click with a typed width pending places the typed width, on the side clicked', () => {
    const { tool, place } = makeTool()
    tool.onPointerDown(ground(0, 0), down(0, 0))
    tool.onPointerMove(ground(0.5, 0.1), down(0.5, 0.1))
    tool.onKey(key('3'))
    tool.onPointerDown(ground(-1, 0.2), down(-1, 0.2))
    expect(place).toHaveBeenCalledTimes(1)
    const [, , , origin, , , width] = place.mock.calls[0]
    expect(width).toBe(3)
    expect(Array.from(origin as Float64Array)).toEqual([-3, 0, 0])
  })

  it('refuses a picture too small to build, with the reason, and stays armed', () => {
    const { tool, place, toast, done } = makeTool()
    tool.onPointerDown(ground(0, 0), down(0, 0))
    // 2:1, so this width passes the corner test but the height is under
    // the minimum side.
    tool.onPointerDown(ground(1.5 * PICTURE_MIN_SIDE_M, 0), down(1.5 * PICTURE_MIN_SIDE_M, 0))
    expect(place).not.toHaveBeenCalled()
    expect(toast).toHaveBeenCalledTimes(1)
    expect(toast.mock.calls[0][1]).toBe('ImageTooSmall')
    expect(toast.mock.calls[0][0]).toContain('too small')
    expect(done).not.toHaveBeenCalled()
    expect(tool.debugState().stage).toBe('anchored')
  })

  it('refuses a picture too big with its own toast, naming the size, for a drag and for a typed width', () => {
    const { tool, place, toast, done } = makeTool()
    tool.onPointerDown(ground(0, 0), down(0, 0))
    // A dragged width past the limit (2:1, so the height is half of it).
    tool.onPointerDown(ground(2 * PICTURE_MAX_SIDE_M, 0), down(2 * PICTURE_MAX_SIDE_M, 0))
    expect(place).not.toHaveBeenCalled()
    expect(toast).toHaveBeenCalledTimes(1)
    expect(toast.mock.calls[0][1]).toBe('ImageTooBig')
    expect(toast.mock.calls[0][0]).toContain('2,000,000 \u00d7 1,000,000 m')
    expect(toast.mock.calls[0][0]).toContain('at most 1,000,000 m')
    expect(toast.mock.calls[0][0]).not.toContain('too small')

    // A typed width past the limit refuses on Enter, keeping the buffer.
    for (const k of '1000000001') tool.onKey(key(k))
    tool.onKey(key('Enter'))
    expect(place).not.toHaveBeenCalled()
    expect(toast.mock.calls.map((call) => call[1])).toEqual(['ImageTooBig', 'ImageTooBig'])
    expect(done).not.toHaveBeenCalled()
    expect(tool.debugState()).toMatchObject({ stage: 'anchored', typed: '1000000001' })

    // Exactly the limit places.
    for (let i = 0; i < 10; i++) tool.onKey(key('Backspace'))
    for (const k of '1000000') tool.onKey(key(k))
    tool.onKey(key('Enter'))
    expect(place).toHaveBeenCalledTimes(1)
    expect(place.mock.calls[0][6]).toBe(1e6)
  })

  it('a typed width under the minimum refuses on Enter and on a click, keeping the buffer, never using the cursor', () => {
    const { tool, place, toast, done } = makeTool()
    tool.onPointerDown(ground(0, 0), down(0, 0))
    tool.onPointerMove(ground(0.5, 0.1), down(0.5, 0.1))
    for (const k of '0.0000005') tool.onKey(key(k))
    // The ghost shows the typed size, not the cursor's 0.5 m.
    const c = tool.ghostCorners()
    expect(c === null || Math.abs(c[2][0] - c[0][0]) < 1e-6).toBe(true)
    tool.onKey(key('Enter'))
    tool.onPointerDown(ground(0.5, 0.1), down(0.5, 0.1))
    expect(place).not.toHaveBeenCalled()
    expect(toast).toHaveBeenCalledTimes(2)
    expect(toast.mock.calls.map((call) => call[1])).toEqual(['ImageTooSmall', 'ImageTooSmall'])
    expect(done).not.toHaveBeenCalled()
    expect(tool.debugState()).toMatchObject({ stage: 'anchored', typed: '0.0000005' })

    // A typed zero refuses the same way.
    for (let i = 0; i < 9; i++) tool.onKey(key('Backspace'))
    tool.onKey(key('0'))
    tool.onKey(key('Enter'))
    tool.onPointerDown(ground(0.5, 0.1), down(0.5, 0.1))
    expect(place).not.toHaveBeenCalled()
    expect(toast).toHaveBeenCalledTimes(4)
    expect(tool.debugState().typed).toBe('0')
  })

  it('a second click on the corner itself does nothing', () => {
    const { tool, place } = makeTool()
    tool.onPointerDown(ground(1, 1), down(1, 1))
    tool.onPointerDown(ground(1, 1), down(1, 1))
    expect(place).not.toHaveBeenCalled()
    expect(tool.debugState().stage).toBe('anchored')
  })

  it('Esc after the first click backs up; Esc before it cancels', () => {
    const { tool, done, place } = makeTool()
    tool.onPointerDown(ground(1, 1), down(1, 1))
    tool.onKey(key('Escape'))
    expect(tool.debugState().stage).toBe('idle')
    expect(done).not.toHaveBeenCalled()
    expect(tool.hasArmedGesture()).toBe(true)
    tool.onKey(key('Escape'))
    expect(done).toHaveBeenCalledWith(false, true)
    expect(place).not.toHaveBeenCalled()
  })

  it('an arrow locks the plane; the same arrow or Esc unlocks before Esc cancels', () => {
    const { tool, done, place } = makeTool()
    tool.onKey(key('ArrowLeft')) // green: the plane square to Y
    expect(tool.debugState().lock).toBe(1)
    expect(tool.statusHint()).toContain('green plane')
    tool.onKey(key('ArrowLeft', true)) // autorepeat does not toggle
    expect(tool.debugState().lock).toBe(1)
    // Clicking through a point on the locked plane, then along +X.
    tool.onPointerDown({ x: 0, y: 0, z: 0, kind: 'endpoint' }, { origin: [0, -10, 0.5], direction: [0, 1, 0] })
    tool.onPointerDown({ x: 2, y: 0, z: 0.3, kind: 'on-face' }, { origin: [2, -10, 0.3], direction: [0, 1, 0] })
    const [, , , , u, v] = place.mock.calls[0]
    expect(Array.from(v as Float64Array).map((c) => Math.round(c))).toEqual([0, 0, 1]) // upright
    expect(Math.abs((u as Float64Array)[0])).toBeCloseTo(1)
    const fresh = makeTool()
    fresh.tool.onKey(key('ArrowUp'))
    fresh.tool.onKey(key('Escape'))
    expect(fresh.tool.debugState().lock).toBeNull()
    expect(fresh.done).not.toHaveBeenCalled()
    fresh.tool.onKey(key('ArrowUp'))
    fresh.tool.onKey(key('ArrowUp'))
    expect(fresh.tool.debugState().lock).toBeNull()
    void done
  })

  it('places on the face under the cursor, upright and facing the viewer', () => {
    // A wall at y = 5 whose normal faces +Y, seen from -Y.
    const { tool, place } = makeTool({ pick: makePick(1n, 2n), facePlane: [0, 5, 0, 0, 1, 0] })
    const ray = (x: number, z: number): Ray => ({ origin: [x, -10, z], direction: [0, 1, 0] })
    tool.onPointerDown({ x: 1, y: 5, z: 1, kind: 'on-face' }, ray(1, 1))
    expect(tool.snapConstraint()).toEqual({ constraintPlane: { point: [1, 5, 1], normal: [-0, -1, -0] } })
    tool.onPointerDown({ x: 3, y: 5, z: 1.5, kind: 'on-face' }, ray(3, 1.5))
    const [, , , origin, u, v, width] = place.mock.calls[0]
    expect(Array.from(origin as Float64Array)).toEqual([1, 5, 1])
    expect(Array.from(u as Float64Array).map((c) => Math.round(c) + 0)).toEqual([1, 0, 0])
    expect(Array.from(v as Float64Array).map((c) => Math.round(c) + 0)).toEqual([0, 0, 1])
    expect(width).toBeCloseTo(2)
  })

  it('places on a face struck through an instance, its plane mapped through the pose once', () => {
    // The face is a wall at local y = 5 (normal +Y); the instance turns it
    // a quarter turn about Z and moves it 10 m along X, so in the world it
    // is the plane x = 5 facing -X. Mapping twice would put it at y = 5.
    const pose = [0, -1, 0, 10, 1, 0, 0, 0, 0, 0, 1, 0]
    const { tool, place } = makeTool({ pick: makePick(1n, 2n, 4n), facePlane: [0, 5, 0, 0, 1, 0], instancePose: pose })
    const ray = (y: number, z: number): Ray => ({ origin: [20, y, z], direction: [-1, 0, 0] })
    tool.onPointerDown({ x: 5, y: 1, z: 1, kind: 'on-face' }, ray(1, 1))
    const plane = tool.snapConstraint()!.constraintPlane!
    expect(close(plane.point, [5, 1, 1])).toBe(true)
    // Turned toward the viewer, who is on +X.
    expect(close(plane.normal, [1, 0, 0])).toBe(true)
    tool.onPointerDown({ x: 5, y: 3, z: 1.5, kind: 'on-face' }, ray(3, 1.5))
    const [, , , origin, u, v, width] = place.mock.calls[0]
    expect(close(Array.from(origin as Float64Array), [5, 1, 1])).toBe(true)
    expect(close(Array.from(u as Float64Array), [0, 1, 0])).toBe(true)
    expect(close(Array.from(v as Float64Array), [0, 0, 1])).toBe(true)
    expect(width).toBeCloseTo(2)
  })

  it('a refused placement toasts the reason and stays armed', () => {
    const { tool, toast, done } = makeTool({ placeThrows: new Error('ExplodeSessionScope: a component is open') })
    tool.onPointerDown(ground(0, 0), down(0, 0))
    tool.onPointerDown(ground(2, 0), down(2, 0))
    expect(toast).toHaveBeenCalledTimes(1)
    expect(toast.mock.calls[0][1]).toBe('ExplodeSessionScope')
    expect(done).not.toHaveBeenCalled()
    expect(tool.debugState().stage).toBe('anchored')
  })

  it('a takeover by another tool retires it without claiming the user cancelled', () => {
    const { tool, done } = makeTool()
    tool.cancel()
    expect(done).toHaveBeenCalledWith(false, false)
    tool.cancel()
    expect(done).toHaveBeenCalledTimes(1)
  })
})

describe('parsePlacedImage', () => {
  it('reads the handles and size, tolerating a malformed result', () => {
    expect(parsePlacedImage({ definition: 3n, instance: 4n, width: 1.5, height: 0.5 })).toEqual({
      definition: 3n, instance: 4n, width: 1.5, height: 0.5,
    })
    expect(parsePlacedImage(undefined)).toEqual({ definition: 0n, instance: 0n, width: 0, height: 0 })
  })
})

describe('view-plane inference', () => {
  const camera = (eye: [number, number, number]) => {
    const cam = new THREE.PerspectiveCamera(45, 1.5, 0.1, 1000)
    cam.position.set(...eye)
    cam.up.set(0, 0, 1)
    cam.lookAt(0, 0, 0)
    cam.updateMatrixWorld(true)
    return cam
  }
  const rayFrom = (eye: [number, number, number], p: [number, number, number]): Ray => {
    const d = [p[0] - eye[0], p[1] - eye[1], p[2] - eye[2]]
    const len = Math.hypot(d[0], d[1], d[2])
    return { origin: eye, direction: [d[0] / len, d[1] / len, d[2] / len] }
  }
  const free = (p: [number, number, number]): Snap => ({ x: p[0], y: p[1], z: p[2], kind: 'plane' })

  it('stands the picture on the green plane from a Front view and says so', () => {
    const { tool } = makeTool()
    const eye: [number, number, number] = [0, -10, 0]
    tool.updateCamera(camera(eye))
    tool.onPointerMove(free([1, 0, 1]), rayFrom(eye, [1, 0, 1]))
    expect(tool.statusHint()).toContain('Facing the green plane')
    const c = tool.ghostCorners()!
    // Every corner lies on y = 0, the green plane through the origin.
    expect(c.every((p) => Math.abs(p[1]) < 1e-9)).toBe(true)
  })

  it('names the plane with the placement copy, and offers it only as the snap fallback', () => {
    const { tool } = makeTool()
    const eye: [number, number, number] = [0, -10, 0]
    tool.updateCamera(camera(eye))
    tool.onPointerMove(free([1, 0, 1]), rayFrom(eye, [1, 0, 1]))
    expect(tool.statusHint()).toMatch(/^Facing the green plane — click the first corner of ".*" to place it there; ↑ for the ground\. Esc cancels\.$/)
    const c = tool.snapConstraint()
    expect(c?.constraintPlane).toBeUndefined()
    expect(c?.fallbackPlane?.normal).toEqual([0, 1, 0])
    expect(c?.fallbackPlane?.point).toEqual([0, 0, 0])
  })

  it('no snap fallback from the home view, under an arrow lock, or with the plane behind the eye', () => {
    const home = makeTool()
    home.tool.updateCamera(camera([3.5, -3, 2.5]))
    expect(home.tool.snapConstraint()).toBeNull()

    const locked = makeTool()
    locked.tool.updateCamera(camera([0, -10, 0]))
    locked.tool.onKey({ key: 'ArrowUp', repeat: false, preventDefault: () => { /* no-op */ } } as unknown as KeyboardEvent)
    expect(locked.tool.snapConstraint()).toBeNull()

    const behind = makeTool()
    const cam = new THREE.PerspectiveCamera(45, 1.5, 0.1, 1000)
    cam.position.set(0, 2, 0)
    cam.up.set(0, 0, 1)
    cam.lookAt(0, 12, 0)
    cam.updateMatrixWorld(true)
    behind.tool.updateCamera(cam)
    expect(behind.tool.snapConstraint()).toBeNull()
    behind.tool.onPointerMove(null, rayFrom([0, 2, 0], [1, 8, -1]))
    expect(behind.tool.statusHint()).not.toContain('Facing')
  })

  it('keeps the ground from the home view, and with no camera fed', () => {
    const eye: [number, number, number] = [3.5, -3, 2.5]
    const home = makeTool()
    home.tool.updateCamera(camera(eye))
    home.tool.onPointerMove(null, rayFrom(eye, [1, 1, 0]))
    expect(home.tool.statusHint()).not.toContain('Facing')
    expect(home.tool.ghostCorners()!.every((p) => Math.abs(p[2]) < 1e-9)).toBe(true)

    const bare = makeTool()
    bare.tool.onPointerMove(ground(0, 0), down(0, 0))
    expect(bare.tool.statusHint()).not.toContain('Facing')
  })
})

describe('PlaceImageTool — a document reset', () => {
  it('forgets the memoized face picks and the hover memory', () => {
    const { tool } = makeTool()
    const cache = (tool as unknown as { pickCache: { clear(): void } }).pickCache
    const clear = vi.spyOn(cache, 'clear')
    tool.onDocumentReset()
    expect(clear).toHaveBeenCalled()
  })
})

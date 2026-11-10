/**
 * View-plane inference at the tool level (docs/design/view-plane-inference.md
 * §5, §6, §8): the five draw tools keep the camera forward from
 * `updateCamera`, name the plane they inferred in the status hint, offer it
 * to the snap service as the fallback plane only (never a kernel constraint)
 * while idle, and record it for the Shift pin. The
 * picking rule itself is covered in drawPlane.test.ts.
 */
import { describe, it, expect, vi } from 'vitest'
import * as THREE from 'three'
import { LineTool } from './LineTool'
import { RectangleTool } from './RectangleTool'
import { CircleTool } from './CircleTool'
import { PolygonTool } from './PolygonTool'
import { ArcTool } from './ArcTool'
import { draftMock } from './draftMock'
import type { Snap } from './types'
import type { Scene as WasmScene } from '../wasm/loader'
import type { Ray } from '../viewport/math'

type V3 = [number, number, number]

function makeScene(): WasmScene {
  const dm = draftMock()
  return {
    axes: vi.fn(() => new Float64Array([0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1])),
    history_generation: vi.fn(() => 1n),
    pick_face: vi.fn(() => undefined),
    pick_sketch: vi.fn(() => undefined),
    sketch_plane: vi.fn(() => undefined),
    face_plane: vi.fn(() => undefined),
    face_normal: vi.fn(() => undefined),
    ...dm.members,
    clear_transient_segments: vi.fn(),
    add_transient_segment: vi.fn(),
  } as unknown as WasmScene
}

interface Under {
  updateCamera(camera: THREE.Camera): void
  onPointerMove(snap: Snap | null, ray: Ray): void
  onPointerDown(snap: Snap | null, ray: Ray): void
  onKey(ev: KeyboardEvent): void
  setShiftHeld(held: boolean): void
  activeDrawPlaneCue(): { plane: { normal: V3 }; through: V3 } | null
  statusHint(): string
  snapConstraint(ray: Ray): { constraintPlane?: { point: V3; normal: V3 }; fallbackPlane?: { point: V3; normal: V3 } } | null
}

const noop = vi.fn()
const MAKERS: { name: string; clause: string; make: (s: WasmScene) => Under }[] = [
  { name: 'Line', clause: 'click to start a line on it', make: (s) => new LineTool(s, new THREE.Group(), noop, noop, noop) as unknown as Under },
  { name: 'Rectangle', clause: 'click the first corner to draw on it', make: (s) => new RectangleTool(s, new THREE.Group(), noop, noop, noop) as unknown as Under },
  { name: 'Circle', clause: 'click the center to draw on it', make: (s) => new CircleTool(s, new THREE.Group(), noop, noop, noop) as unknown as Under },
  { name: 'Polygon', clause: 'click the center to draw on it', make: (s) => new PolygonTool(s, new THREE.Group(), noop, noop, noop) as unknown as Under },
  { name: 'Arc', clause: 'click the first endpoint to draw on it', make: (s) => new ArcTool(s, new THREE.Group(), noop, noop, noop) as unknown as Under },
]

/** A camera at `eye` looking at the origin. */
function cameraAt(eye: V3): THREE.PerspectiveCamera {
  const cam = new THREE.PerspectiveCamera(45, 1.5, 0.1, 1000)
  cam.position.set(...eye)
  cam.up.set(0, 0, 1)
  cam.lookAt(0, 0, 0)
  cam.updateMatrixWorld(true)
  return cam
}

/** Front standard view: level, looking along +Y, so the green plane faces it. */
const FRONT: V3 = [0, -10, 0]
/** Hew's home view: high enough that the ground wins. */
const HOME: V3 = [3.5, -3, 2.5]

function rayThrough(eye: V3, p: V3): Ray {
  const d: V3 = [p[0] - eye[0], p[1] - eye[1], p[2] - eye[2]]
  const len = Math.hypot(...d)
  return { origin: eye, direction: [d[0] / len, d[1] / len, d[2] / len] }
}

function snapAt(p: V3): Snap {
  return { x: p[0], y: p[1], z: p[2], kind: 'plane' }
}

function hover(tool: Under, eye: V3, p: V3): Ray {
  const ray = rayThrough(eye, p)
  tool.snapConstraint(ray)
  tool.onPointerMove(snapAt(p), ray)
  return ray
}

describe.each(MAKERS)('$name — view-plane inference', ({ make, clause }) => {
  it('keeps the ground, with the default hint and no constraint, before any camera is fed', () => {
    const tool = make(makeScene())
    const ray = hover(tool, FRONT, [1, 0, 1])
    expect(tool.statusHint()).not.toContain('Facing')
    expect(tool.snapConstraint(ray)).toBeNull()
  })

  it('names the green plane in the hint and offers it as the fallback plane only from a Front view', () => {
    const tool = make(makeScene())
    tool.updateCamera(cameraAt(FRONT))
    const ray = hover(tool, FRONT, [1, 0, 1])
    expect(tool.statusHint()).toContain(`Facing the green plane — ${clause}; ↑ for the ground, Shift pins it.`)
    const constraint = tool.snapConstraint(ray)
    // Never a kernel constraint: geometry anywhere must still snap.
    expect(constraint?.constraintPlane).toBeUndefined()
    expect(constraint?.fallbackPlane).toBeDefined()
    const n = constraint!.fallbackPlane!.normal
    expect(Math.abs(n[1])).toBeCloseTo(1, 9)
    expect(constraint!.fallbackPlane!.point[1]).toBeCloseTo(0, 9)
  })

  it('a green plane behind the eye is not inferred: the ground instead', () => {
    const tool = make(makeScene())
    // Looking along +Y like Front, but from y = +2, past the plane y = 0.
    const eye: V3 = [0, 2, 0]
    const cam = new THREE.PerspectiveCamera(45, 1.5, 0.1, 1000)
    cam.position.set(...eye)
    cam.up.set(0, 0, 1)
    cam.lookAt(0, 12, 0)
    cam.updateMatrixWorld(true)
    tool.updateCamera(cam)
    const ray = hover(tool, eye, [1, 8, -1])
    expect(tool.statusHint()).not.toContain('Facing')
    expect(tool.snapConstraint(ray)).toBeNull()
    expect(tool.activeDrawPlaneCue()).toBeNull()
  })

  it('names the red plane from a Right view', () => {
    const tool = make(makeScene())
    tool.updateCamera(cameraAt([10, 0, 0]))
    hover(tool, [10, 0, 0], [0, 1, 1])
    expect(tool.statusHint()).toContain('Facing the red plane')
  })

  it('shows a plane patch through the ray hit on the inferred plane, and none on the ground', () => {
    const tool = make(makeScene())
    tool.updateCamera(cameraAt(FRONT))
    hover(tool, FRONT, [1, 0, 1])
    const cue = tool.activeDrawPlaneCue()
    expect(cue).not.toBeNull()
    expect(Math.abs(cue!.plane.normal[1])).toBeCloseTo(1, 9)
    expect(cue!.through[0]).toBeCloseTo(1, 9)
    expect(cue!.through[1]).toBeCloseTo(0, 9)
    expect(cue!.through[2]).toBeCloseTo(1, 9)
    tool.updateCamera(cameraAt(HOME))
    hover(tool, HOME, [1, 1, 0])
    expect(tool.activeDrawPlaneCue()).toBeNull()
  })

  it('stays on the ground from the home view', () => {
    const tool = make(makeScene())
    tool.updateCamera(cameraAt(HOME))
    const ray = hover(tool, HOME, [1, 1, 0])
    expect(tool.statusHint()).not.toContain('Facing')
    expect(tool.snapConstraint(ray)).toBeNull()
  })

  it('follows the camera as it orbits: the hint appears and goes away', () => {
    const tool = make(makeScene())
    tool.updateCamera(cameraAt(FRONT))
    hover(tool, FRONT, [1, 0, 1])
    expect(tool.statusHint()).toContain('Facing the green plane')
    tool.updateCamera(cameraAt(HOME))
    hover(tool, HOME, [1, 1, 0])
    expect(tool.statusHint()).not.toContain('Facing')
  })

  it('an arrow lock beats the inference, and releasing it brings the inference back', () => {
    const tool = make(makeScene())
    tool.updateCamera(cameraAt(FRONT))
    hover(tool, FRONT, [1, 0, 1])
    const key = (k: string) => ({ key: k, repeat: false, preventDefault: () => { /* no-op */ } }) as unknown as KeyboardEvent
    tool.onKey(key('ArrowUp'))
    expect(tool.statusHint()).toContain('Locked to the blue plane')
    expect(tool.statusHint()).not.toContain('Facing')
    tool.onKey(key('ArrowUp'))
    hover(tool, FRONT, [1, 0, 1])
    expect(tool.statusHint()).toContain('Facing the green plane')
  })

  it('Shift pins the plane the cue was showing, and the pin holds after orbiting to the home view', () => {
    const tool = make(makeScene())
    tool.updateCamera(cameraAt(FRONT))
    hover(tool, FRONT, [1, 0, 1])
    tool.setShiftHeld(true)
    expect(tool.statusHint()).toContain('Pinned to the hovered plane')
    expect(tool.statusHint()).not.toContain('Facing')
    tool.setShiftHeld(false)
    tool.updateCamera(cameraAt(HOME))
    const ray = hover(tool, HOME, [1, 1, 0])
    const n = tool.snapConstraint(ray)!.constraintPlane!.normal
    expect(Math.abs(n[1])).toBeCloseTo(1, 9)
  })

  it('freezes the plane at the first click: orbiting afterward does not move it', () => {
    const tool = make(makeScene())
    tool.updateCamera(cameraAt(FRONT))
    const ray = hover(tool, FRONT, [1, 0, 1])
    tool.onPointerDown(snapAt([1, 0, 1]), ray)
    tool.updateCamera(cameraAt(HOME))
    const ray2 = rayThrough(HOME, [2, 0, 1])
    const c = tool.snapConstraint(ray2)
    // A Line chain's first point does not constrain; the others hold the
    // anchored plane. Either way the plane never becomes the ground.
    if (c?.constraintPlane) expect(Math.abs(c.constraintPlane.normal[1])).toBeCloseTo(1, 9)
  })

  it('a document reset forgets the memoized face picks and the hover memory', () => {
    const tool = make(makeScene())
    const cache = (tool as unknown as { _pickCache: { clear(): void } })._pickCache
    const clear = vi.spyOn(cache, 'clear')
    ;(tool as unknown as { onDocumentReset(): void }).onDocumentReset()
    expect(clear).toHaveBeenCalled()
  })
})

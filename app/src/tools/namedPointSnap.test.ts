/**
 * A snap to a named point is that point (wireframe-faces-and-solids design
 * §2, stage 1): real tool gestures on the shared draft mock with
 * `enforcePlane`, so a commit that left its plane would be refused the way
 * the kernel refuses it.
 *
 * Kurt's case: a 1 m Rectangle on the ground, a guide point 1 m above its
 * center. The cursor sits over the Rectangle's fill (picker step 4b chooses
 * the ground) while snapped to the guide point in the air.
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
import type { DrawPlaneState } from './drawPlane'
import type { Scene as WasmScene } from '../wasm/loader'
import type { Ray } from '../viewport/math'

type V3 = [number, number, number]

/** Hew's home view: high enough that the ground wins over empty space. */
const HOME: V3 = [3.5, -3, 2.5]

function cameraAt(eye: V3): THREE.PerspectiveCamera {
  const cam = new THREE.PerspectiveCamera(45, 1.5, 0.1, 1000)
  cam.position.set(...eye)
  cam.up.set(0, 0, 1)
  cam.lookAt(0, 0, 0)
  cam.updateMatrixWorld(true)
  return cam
}

function rayThrough(p: V3, eye: V3 = HOME): Ray {
  const d: V3 = [p[0] - eye[0], p[1] - eye[1], p[2] - eye[2]]
  const len = Math.hypot(...d)
  return { origin: [...eye], direction: [d[0] / len, d[1] / len, d[2] / len] }
}

function snap(p: V3, kind = 'endpoint'): Snap {
  return { x: p[0], y: p[1], z: p[2], kind }
}

function key(k: string): KeyboardEvent {
  return { key: k, repeat: false, preventDefault: () => { /* no-op */ } } as unknown as KeyboardEvent
}

/** The guide point of Kurt's case, and the ground Rectangle's corners. */
const APEX: V3 = [0.5, 0.5, 1]
const CORNER: V3 = [0, 0, 0]

/**
 * A scene on the draft mock with plane enforcement. `fill` makes the fill
 * pick report a ground sketch under every ray (Kurt's Rectangle).
 */
function makeScene(opts: { fill?: boolean } = {}) {
  const dm = draftMock({ enforcePlane: true })
  const fill = opts.fill === true
    ? { sketch: () => 7n, detached: () => false, free: vi.fn() }
    : undefined
  const scene = {
    axes: vi.fn(() => new Float64Array([0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1])),
    history_generation: vi.fn(() => 1n),
    pick_face: vi.fn(() => undefined),
    pick_sketch: vi.fn(() => undefined),
    sketch_plane: vi.fn((h: bigint) => (h === 7n ? new Float64Array([0, 0, 0, 0, 0, 1]) : undefined)),
    face_plane: vi.fn(() => undefined),
    face_normal: vi.fn(() => undefined),
    ...dm.members,
    pick_sketch_region: vi.fn(() => fill),
    pick_sketch_region_in_instance: vi.fn(() => undefined),
    clear_transient_segments: vi.fn(),
    add_transient_segment: vi.fn(),
  }
  return { scene: scene as unknown as WasmScene, log: dm.log }
}

const noop = (): void => { /* no-op */ }
function lineTool(scene: WasmScene, onToast: (m: string) => void = noop) {
  const tool = new LineTool(scene, new THREE.Group(), noop, onToast, noop)
  tool.updateCamera(cameraAt(HOME))
  return tool
}

/** Hover then press at `p`, the way the viewport delivers a click. */
function click(tool: { snapConstraint(r: Ray): unknown; onPointerMove(s: Snap | null, r: Ray): void; onPointerDown(s: Snap | null, r: Ray): void }, s: Snap, rayAtPoint: V3 = [s.x, s.y, s.z]): void {
  const ray = rayThrough(rayAtPoint)
  tool.snapConstraint(ray)
  tool.onPointerMove(s, ray)
  tool.onPointerDown(s, ray)
}

describe('Line — a snap to a named point is that point', () => {
  it('a guide point in the air starts a Line there and a ground corner ends it (commits (0.5, 0.5, 1) to the corner)', () => {
    const { scene, log } = makeScene({ fill: true })
    const tool = lineTool(scene)
    // The cursor ray is over the ground fill beyond the guide point.
    click(tool, snap(APEX), [0.5, 0.5, 0])
    click(tool, snap(CORNER))
    expect(log.segments).toEqual([[0.5, 0.5, 1, 0, 0, 0]])
    // The chain left the plane through the point: a re-homed plane holds both ends.
    const begin = log.begins[0].args as number[]
    expect(begin.slice(0, 3)).toEqual(APEX)
  })

  it('the reverse order commits the same segment', () => {
    const { scene, log } = makeScene({ fill: true })
    const tool = lineTool(scene)
    click(tool, snap(CORNER), CORNER)
    click(tool, snap(APEX), [0.5, 0.5, 0])
    expect(log.segments).toEqual([[0, 0, 0, 0.5, 0.5, 1]])
  })

  it('a bare ground corner to the top of a vertical Line keeps the height', () => {
    const { scene, log } = makeScene()
    const tool = lineTool(scene)
    click(tool, snap([1, 0, 0]))
    click(tool, snap([0, 0, 1]), [0, 0, 0.2])
    expect(log.segments).toEqual([[1, 0, 0, 0, 0, 1]])
  })

  it('an on-edge snap at height still drops onto the ground and says projected', () => {
    const { scene, log } = makeScene()
    const tool = lineTool(scene)
    click(tool, snap(CORNER))
    const edge = snap([1, 1, 1], 'on-edge')
    tool.onPointerMove(edge, rayThrough([1, 1, 1]))
    expect(tool.snapProjected()).toBe(true)
    tool.onPointerDown(edge, rayThrough([1, 1, 1]))
    expect(log.segments).toEqual([[0, 0, 0, 1, 1, 0]])
    expect(log.begins).toHaveLength(1)
  })

  it('a named point a float hair off the ground commits exactly z = 0 on the ground, as Rectangle and Circle do', () => {
    const { scene, log } = makeScene()
    const tool = lineTool(scene)
    click(tool, snap(CORNER))
    const near = snap([1, 1, 1e-13])
    tool.onPointerMove(near, rayThrough([1, 1, 0]))
    expect(tool.snapProjected()).toBe(false)
    click(tool, near, [1, 1, 0])
    expect(log.segments).toEqual([[0, 0, 0, 1, 1, 0]])
    expect(log.begins).toHaveLength(1)
  })

  it('a named point a hair below the ground commits exactly z = 0 too', () => {
    const { scene, log } = makeScene()
    const tool = lineTool(scene)
    click(tool, snap(CORNER))
    const near = snap([1, 0, -1e-9])
    tool.onPointerMove(near, rayThrough([1, 0, 0]))
    expect(tool.snapProjected()).toBe(false)
    click(tool, near, [1, 0, 0])
    expect(log.segments).toEqual([[0, 0, 0, 1, 0, 0]])
    expect(log.begins).toHaveLength(1)
  })

  it('a named point below the ground still lifts the plane: a plane hides nothing (R18 a)', () => {
    const { scene } = makeScene()
    const tool = lineTool(scene)
    click(tool, snap([0.5, 0.5, -1]), [0.5, 0.5, -1])
    const anchor = (tool as unknown as { planeStage: { plane: { ground: boolean }; anchor: V3 } }).planeStage
    expect(anchor.plane.ground).toBe(false)
    expect(anchor.anchor).toEqual([0.5, 0.5, -1])
    expect(tool.snapProjected()).toBe(false)
  })

  describe('a held plane does not re-home: named points drop and say projected', () => {
    it('Shift-pinned ground', () => {
      const { scene, log } = makeScene()
      const tool = lineTool(scene)
      tool.onPointerMove(snap(CORNER, 'ground'), rayThrough(CORNER))
      tool.setShiftHeld(true)
      tool.setShiftHeld(false)
      click(tool, snap(CORNER, 'ground'))
      const high = snap(APEX)
      tool.onPointerMove(high, rayThrough(APEX))
      expect(tool.snapProjected()).toBe(true)
      tool.onPointerDown(high, rayThrough(APEX))
      expect(log.segments).toEqual([[0, 0, 0, 0.5, 0.5, 0]])
      expect(log.begins).toHaveLength(1)
    })

    it('arrow-locked ground (blue)', () => {
      const { scene, log } = makeScene()
      const tool = lineTool(scene)
      tool.onKey(key('ArrowUp'))
      click(tool, snap(CORNER))
      const high = snap(APEX)
      tool.onPointerMove(high, rayThrough(APEX))
      expect(tool.snapProjected()).toBe(true)
      tool.onPointerDown(high, rayThrough(APEX))
      expect(log.segments).toEqual([[0, 0, 0, 0.5, 0.5, 0]])
      expect(log.begins).toHaveLength(1)
    })

    it('arrow-locked vertical plane (red): the point drops onto x = 1', () => {
      const { scene, log } = makeScene()
      const tool = lineTool(scene)
      tool.onKey(key('ArrowRight'))
      click(tool, snap([1, 2, 0]))
      const off = snap([4, 5, 2])
      tool.onPointerMove(off, rayThrough([4, 5, 2]))
      expect(tool.snapProjected()).toBe(true)
      tool.onPointerDown(off, rayThrough([4, 5, 2]))
      expect(log.segments).toEqual([[1, 2, 0, 1, 5, 2]])
      expect(log.begins).toHaveLength(1)
    })
  })
})

const FIRST_CLICK_TOOLS: { name: string; make: (s: WasmScene) => ToolUnderTest }[] = [
  { name: 'Line', make: (s) => lineTool(s) as unknown as ToolUnderTest },
  { name: 'Rectangle', make: (s) => new RectangleTool(s, new THREE.Group(), noop, noop, noop) as unknown as ToolUnderTest },
  { name: 'Circle', make: (s) => new CircleTool(s, new THREE.Group(), noop, noop, noop) as unknown as ToolUnderTest },
  { name: 'Polygon', make: (s) => new PolygonTool(s, new THREE.Group(), noop, noop, noop) as unknown as ToolUnderTest },
  { name: 'Arc', make: (s) => new ArcTool(s, new THREE.Group(), noop, noop, noop) as unknown as ToolUnderTest },
]

interface ToolUnderTest {
  updateCamera(camera: THREE.Camera): void
  snapConstraint(ray: Ray): {
    constraintPlane?: { point: V3; normal: V3 }
    offPlanePoints?: boolean
  } | null
  onPointerMove(snap: Snap | null, ray: Ray): void
  onPointerDown(snap: Snap | null, ray: Ray): void
  onKey(ev: KeyboardEvent): void
  snapProjected(): boolean
  drawPlaneState(): DrawPlaneState
}

describe.each(FIRST_CLICK_TOOLS)('$name — the first click on a named point in the air', ({ make }) => {
  function fresh(): ToolUnderTest {
    const { scene } = makeScene({ fill: true })
    const tool = make(scene)
    tool.updateCamera(cameraAt(HOME))
    return tool
  }

  it('previews the plane through the point while hovering', () => {
    const tool = fresh()
    const ray = rayThrough(APEX)
    tool.snapConstraint(ray)
    tool.onPointerMove(snap(APEX), ray)
    const state = tool.drawPlaneState()
    expect(state.hold).toBe('inferred')
    expect(state.plane.normal).toEqual([0, 0, 1])
    expect(state.plane.origin[2]).toBe(1)
    expect(state.plane.ground).toBe(false)
  })

  it('starts there: the gesture lies on the parallel plane through the point', () => {
    const tool = fresh()
    click(tool, snap(APEX))
    const state = tool.drawPlaneState()
    expect(state.hold).toBe('anchored')
    expect(state.plane.origin[2]).toBe(1)
    expect(state.plane.ground).toBe(false)
    // The cursor ray through the point meets the gesture's plane at the point.
    for (let i = 0; i < 3; i++) expect(state.hover!.point[i]).toBeCloseTo(APEX[i], 9)
    const constraint = tool.snapConstraint(rayThrough([2, 2, 1]))
    if (constraint !== null && constraint.constraintPlane !== undefined) {
      expect(constraint.constraintPlane.point[2]).toBe(1)
      expect(constraint.constraintPlane.normal).toEqual([0, 0, 1])
    }
  })

  it('a point on the picked plane changes nothing: a ground corner anchors on the ground', () => {
    const tool = fresh()
    click(tool, snap(CORNER))
    const state = tool.drawPlaneState()
    expect(state.hold).toBe('anchored')
    expect(state.plane.ground).toBe(true)
  })
})

describe('Rectangle — started on an air point', () => {
  it('commits a rectangle lying on the plane z = 1', () => {
    const { scene, log } = makeScene({ fill: true })
    const tool = new RectangleTool(scene, new THREE.Group(), noop, noop, noop)
    tool.updateCamera(cameraAt(HOME))
    click(tool, snap(APEX))
    click(tool, snap([1.5, 1.5, 1]))
    expect(log.segments).toHaveLength(4)
    for (const s of log.segments) {
      expect(s[2]).toBe(1)
      expect(s[5]).toBe(1)
    }
    const xs = log.segments.flatMap((s) => [s[0], s[3]])
    expect(Math.min(...xs)).toBe(0.5)
    expect(Math.max(...xs)).toBe(1.5)
  })
})

const FROZEN_TOOLS = FIRST_CLICK_TOOLS.filter((t) => t.name !== 'Line')

describe.each(FROZEN_TOOLS)('$name — a frozen plane projects an off-plane named point and discloses it', ({ make }) => {
  function fresh(): ToolUnderTest {
    const { scene } = makeScene()
    const tool = make(scene)
    tool.updateCamera(cameraAt(HOME))
    return tool
  }

  it('on the vertical plane x = 1 it asks the engine for off-plane points and says projected', () => {
    const tool = fresh()
    tool.onKey(key('ArrowRight'))
    click(tool, snap([1, 2, 0]))
    const constraint = tool.snapConstraint(rayThrough([4, 5, 2]))
    expect(constraint?.offPlanePoints).toBe(true)
    expect(constraint?.constraintPlane?.normal).toEqual([1, 0, 0])
    const off = snap([4, 5, 2])
    tool.onPointerMove(off, rayThrough([4, 5, 2]))
    expect(tool.snapProjected()).toBe(true)
  })

  it('on the ground a named point in the air drops through cursorOnFrozenPlane: projected, not kept', () => {
    const tool = fresh()
    click(tool, snap(CORNER))
    const off = snap([2, 0, 1])
    tool.onPointerMove(off, rayThrough([2, 0, 1]))
    expect(tool.snapProjected()).toBe(true)
    const onGround = snap([2, 0, 0])
    tool.onPointerMove(onGround, rayThrough([2, 0, 0]))
    expect(tool.snapProjected()).toBe(false)
  })
})

describe('ground commits stay bit-identical', () => {
  it('a Rectangle on the ground commits z = 0 corners from snaps that carry height', () => {
    const { scene, log } = makeScene()
    const tool = new RectangleTool(scene, new THREE.Group(), noop, noop, noop)
    tool.updateCamera(cameraAt(HOME))
    click(tool, snap([0.1, 0.2, 0], 'ground'))
    // A named point above the far corner drops straight down.
    click(tool, snap([1.3, 1.7, 0.9]))
    expect(log.segments).toEqual([
      [0.1, 0.2, 0, 1.3, 0.2, 0],
      [1.3, 0.2, 0, 1.3, 1.7, 0],
      [1.3, 1.7, 0, 0.1, 1.7, 0],
      [0.1, 1.7, 0, 0.1, 0.2, 0],
    ])
    expect(log.begins).toHaveLength(1)
    expect((log.begins[0].args as number[]).slice(0, 6)).toEqual([0, 0, 0, 0, 0, 1])
  })
})

// ---------------------------------------------------------------------------
// A face gives way to a named point in front of it (ruling R1).

interface FaceTool extends ToolUnderTest {
  setShiftHeld(held: boolean): void
  setEditContext(ctx: { kind: 'object'; id: bigint }): void
}

/** A scene whose every ray strikes the top face (z = 1, normal +Z) of object 7. */
function makeFaceScene(): WasmScene {
  const { scene } = makeScene()
  return {
    ...scene,
    pick_face: vi.fn(() => ({ object: () => 7n, face: () => 3n, depth: () => 1, instance: () => undefined, free: vi.fn() })),
    node_parent: vi.fn(() => undefined),
    face_normal: vi.fn(() => new Float64Array([0, 0, 1])),
    face_plane: vi.fn(() => new Float64Array([0, 0, 1, 0, 0, 1])),
  } as unknown as WasmScene
}

const IN_FRONT: V3 = [0.5, 0.5, 2] // above the face, nearer the eye than the face
const BEHIND: V3 = [0.5, 0.5, 0.2] // below the face: the ray strikes the face first

describe.each(FIRST_CLICK_TOOLS)('$name — a face gives way to a named point in front of it', ({ make }) => {
  function fresh(): FaceTool {
    const tool = make(makeFaceScene()) as FaceTool
    tool.updateCamera(cameraAt(HOME))
    return tool
  }

  it('the idle constraint over a non-ground plane asks for off-plane points', () => {
    const tool = fresh()
    expect(tool.snapConstraint(rayThrough([0.5, 0.5, 1]))?.offPlanePoints).toBe(true)
  })

  it('a point in front: plane mode on the axis plane through it, not projected', () => {
    const tool = fresh()
    const ray = rayThrough(IN_FRONT)
    tool.snapConstraint(ray)
    tool.onPointerMove(snap(IN_FRONT), ray)
    expect(tool.snapProjected()).toBe(false)
    tool.onPointerDown(snap(IN_FRONT), ray)
    const state = tool.drawPlaneState()
    expect(state.plane.normal).toEqual([0, 0, 1])
    expect(state.plane.origin[2]).toBe(2)
  })

  it('a point behind the face stays face mode and says projected', () => {
    const tool = fresh()
    const ray = rayThrough(BEHIND)
    tool.onPointerMove(snap(BEHIND), ray)
    expect(tool.snapProjected()).toBe(true)
    tool.onPointerDown(snap(BEHIND), ray)
    expect(tool.drawPlaneState().plane.origin[2]).toBe(1)
  })

  it('inside an entered Object it stays face mode and the point drops, disclosed', () => {
    const tool = fresh()
    tool.setEditContext({ kind: 'object', id: 7n })
    const ray = rayThrough(IN_FRONT)
    tool.onPointerMove(snap(IN_FRONT), ray)
    expect(tool.snapProjected()).toBe(true)
    tool.onPointerDown(snap(IN_FRONT), ray)
    expect(tool.drawPlaneState().plane.origin[2]).toBe(1)
  })

  it('a pinned idle constraint keeps named points off the plane (R18 c), and Shift over a lifted hover pins the lifted plane', () => {
    const tool = fresh()
    const ray = rayThrough(IN_FRONT)
    tool.onPointerMove(snap(IN_FRONT), ray)
    tool.setShiftHeld(true)
    const constraint = tool.snapConstraint(ray)
    expect(constraint?.constraintPlane?.point[2]).toBe(2)
    expect(constraint?.offPlanePoints).toBe(true)
    expect(tool.drawPlaneState().plane.origin[2]).toBe(2)
  })
})

/**
 * TapeMeasureTool unit tests — fake-WasmScene pattern like ArcTool.test.ts.
 * Focused on first-click mode selection: an edge snap (world Object OR
 * committed sketch edge) enters parallel-guide mode and commits
 * `add_guide_line`; anything else falls to measure mode.
 */
import { describe, it, expect, vi } from 'vitest'
import * as THREE from 'three'
import { TapeMeasureTool } from './TapeMeasureTool'
import type { Snap } from './types'
import type { Scene as WasmScene } from '../wasm/loader'
import type { Ray } from '../viewport/math'
import { axisColorForDirection, axisColorsForTheme } from '../viewport/axisColors'
import { getResolvedTheme } from '../settings/theme'
import { GUIDE_COLOR } from '../viewport/guideColors'
import { AXIS_ALONG_EDGE_COS } from './tapeOffset'
import { formatLength } from '../settings/units'

const RAY: Ray = { origin: [0, 0, 5], direction: [0, 0, -1] }

/** Reach into the tool's private preview-segment fields — the most direct,
 *  least-fragile way to identify which LineSegments is the long guide-
 *  preview line vs. the short offset connector (WP-6), rather than
 *  discriminating them by geometry length/position heuristics. `private` is
 *  a compile-time-only restriction, so this cast is safe at runtime. */
function previewInternals(tool: TapeMeasureTool): {
  previewLine: THREE.LineSegments | null
  previewConnector: THREE.LineSegments | null
} {
  return tool as unknown as { previewLine: THREE.LineSegments | null; previewConnector: THREE.LineSegments | null }
}

/** The color TapeMeasureTool's own axis-color rule would assign a
 *  direction, computed the same way (not hardcoded) so this file doesn't
 *  depend on which theme resolves in the vitest environment — mirrors
 *  LineTool.axisPreview.test.ts's `expectedAxisColorHex` idiom. */
function expectedPreviewColorHex(direction: readonly [number, number, number]): number {
  const match = axisColorForDirection(direction, AXIS_ALONG_EDGE_COS, axisColorsForTheme(getResolvedTheme()))
  const color = match?.color ?? GUIDE_COLOR
  return new THREE.LineBasicMaterial({ color }).color.getHex()
}

function makeSnap(overrides: Partial<Snap> = {}): Snap {
  return { x: 0, y: 0, z: 0, kind: 'ground', ...overrides }
}

function makeWasmScene() {
  const guideLines: number[][] = []
  const guidePoints: number[][] = []
  const scene = {
    // Both endpoint queries return a unit-X edge from (0,0,0) to (2,0,0).
    edge_endpoints: vi.fn((_o: bigint, _e: bigint) => new Float64Array([0, 0, 0, 2, 0, 0])),
    sketch_edge_endpoints: vi.fn((_s: bigint, _e: bigint) => new Float64Array([0, 0, 0, 2, 0, 0])),
    // In-instance variants return a DIFFERENT (pose-mapped) direction than
    // their plain counterparts above, so tests can tell which path resolved
    // an edge from just the committed guide's direction.
    edge_endpoints_in_instance: vi.fn((_i: bigint, _o: bigint, _e: bigint) => new Float64Array([0, 0, 0, 0, 2, 0])),
    sketch_edge_endpoints_in_instance: vi.fn((_i: bigint, _s: bigint, _e: bigint) => new Float64Array([0, 0, 0, 0, 0, 2])),
    add_guide_line: vi.fn((ox: number, oy: number, oz: number, dx: number, dy: number, dz: number) => {
      guideLines.push([ox, oy, oz, dx, dy, dz])
    }),
    add_guide_point: vi.fn((x: number, y: number, z: number) => {
      guidePoints.push([x, y, z])
    }),
    // No committed sketches under the cursor in these fixtures (plane-lock /
    // sketch-hover-adopt is covered separately in TapeMeasureTool.plane.test.ts).
    pick_sketch: vi.fn(() => undefined),
    sketch_plane: vi.fn(() => undefined),
    rescale_document: vi.fn(),
    rescale_session: vi.fn(),
    // No face under the cursor in these fixtures (WP-4's offset-plane
    // face-pick fallback is covered separately in TapeMeasureTool.plane.test.ts).
    pick_face: vi.fn(() => undefined),
    // World-identity drawing axes (WP-6's preview axis-color rule reads this
    // via getDrawingAxes on every preview rebuild) — a moved frame is
    // covered separately in TapeMeasureTool.plane.test.ts / the axis-color
    // tests below.
    axes: vi.fn(() => new Float64Array([0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1])),
  }
  return { scene: scene as unknown as WasmScene, guideLines, guidePoints }
}

function makeTool(scene: WasmScene) {
  const onGuideCreated = vi.fn()
  const onToast = vi.fn()
  const onMeasurement = vi.fn()
  const onRescaleArmed = vi.fn()
  const onRescaleApplied = vi.fn()
  const tool = new TapeMeasureTool(
    scene,
    new THREE.Group(),
    onGuideCreated,
    onToast,
    onMeasurement,
    onRescaleArmed,
    onRescaleApplied,
  )
  return { tool, onGuideCreated, onToast, onMeasurement, onRescaleArmed, onRescaleApplied }
}

/** Feed a plain digit/decimal string then Enter into the tool's VCB. */
const typeAndEnter = (tool: TapeMeasureTool, buf: string): void => {
  const key = (k: string) => ({ key: k, preventDefault: () => { /* no-op */ } }) as unknown as KeyboardEvent
  for (const ch of buf) tool.onKey(key(ch))
  tool.onKey(key('Enter'))
}

/** Feed one raw key event to the tool's `onKey` — the idle-recall tests
 *  below press keys one at a time rather than always ending in Enter. */
const pressKey = (tool: TapeMeasureTool, k: string): void => {
  tool.onKey({ key: k, preventDefault: () => { /* no-op */ } } as unknown as KeyboardEvent)
}

describe('TapeMeasureTool — parallel-guide mode entry', () => {
  it('an object-edge snap enters parallel mode and commits a guide line', () => {
    const { scene, guideLines } = makeWasmScene()
    const { tool, onGuideCreated } = makeTool(scene)

    tool.onPointerDown(
      makeSnap({ x: 1, y: 0, z: 0, kind: 'on-edge', elementKind: 'edge', object: 7n, element: 3n }),
      RAY,
    )
    tool.onPointerMove(makeSnap({ x: 1, y: 0.5, z: 0 }), RAY) // pull sideways
    tool.onPointerDown(makeSnap({ x: 1, y: 0.5, z: 0 }), RAY) // commit

    expect(guideLines.length).toBe(1)
    expect(onGuideCreated).toHaveBeenCalledTimes(1)
    // Direction is the edge direction (+X).
    const [, , , dx, dy, dz] = guideLines[0]
    expect([dx, dy, dz]).toEqual([1, 0, 0])
  })

  it('a SKETCH-edge snap enters parallel mode too (a rectangle sketch edge is the common case)', () => {
    const { scene, guideLines, guidePoints } = makeWasmScene()
    const { tool, onGuideCreated } = makeTool(scene)

    tool.onPointerDown(
      makeSnap({ x: 1, y: 0, z: 0, kind: 'on-edge', elementKind: 'sketch-edge', sketch: 42n, element: 5n }),
      RAY,
    )
    tool.onPointerMove(makeSnap({ x: 1, y: 0.5, z: 0 }), RAY)
    tool.onPointerDown(makeSnap({ x: 1, y: 0.5, z: 0 }), RAY)

    expect(guideLines.length).toBe(1)
    expect(guidePoints.length).toBe(0) // NOT the old bug: no stray guide point
    expect(onGuideCreated).toHaveBeenCalledTimes(1)
    const wasm = scene as unknown as { sketch_edge_endpoints: ReturnType<typeof vi.fn> }
    expect(wasm.sketch_edge_endpoints).toHaveBeenCalledWith(42n, 5n)
    const [, , , dx, dy, dz] = guideLines[0]
    expect([dx, dy, dz]).toEqual([1, 0, 0])
  })

  it('a consumed/stale sketch edge (endpoints undefined) falls back to measure mode', () => {
    const { scene, guideLines, guidePoints } = makeWasmScene()
    ;(scene as unknown as { sketch_edge_endpoints: ReturnType<typeof vi.fn> })
      .sketch_edge_endpoints.mockReturnValue(undefined)
    const { tool } = makeTool(scene)

    tool.onPointerDown(
      makeSnap({ x: 1, y: 0, z: 0, kind: 'on-edge', elementKind: 'sketch-edge', sketch: 42n, element: 5n }),
      RAY,
    )
    // Second click in empty space: measure mode drops a guide point AND
    // (WP-7 item 3) a guide line through the two measured points.
    tool.onPointerDown(makeSnap({ x: 3, y: 3, z: 0, kind: 'ground' }), RAY)

    expect(guideLines.length).toBe(1)
    expect(guidePoints.length).toBe(1)
  })

  // component-edit-parity.md phase A2: guides stay world-space in v1 by
  // design (the module doc's OUT-OF-SCOPE note) — `edge_endpoints` still
  // only resolves live world Objects, so a component member's edge (which
  // reports `undefined` here, exactly like the K1 fix made `sketch_edge_
  // endpoints` do for a def-owned sketch edge above) continues to degrade
  // to measure mode instead of a misplaced or crashing guide. `setEditContext`
  // is exercised too — it changes nothing about this fallback, on purpose.
  it('a member OBJECT edge (endpoints undefined) falls back to measure mode, even inside its own instance context', () => {
    const { scene, guideLines, guidePoints } = makeWasmScene()
    ;(scene as unknown as { edge_endpoints: ReturnType<typeof vi.fn> })
      .edge_endpoints.mockReturnValue(undefined)
    const { tool } = makeTool(scene)
    tool.setEditContext({ kind: 'instance', id: 42n, component: 5n })

    tool.onPointerDown(
      makeSnap({ x: 1, y: 0, z: 0, kind: 'on-edge', elementKind: 'edge', object: 7n, element: 3n }),
      RAY,
    )
    tool.onPointerDown(makeSnap({ x: 3, y: 3, z: 0, kind: 'ground' }), RAY)

    // WP-7 item 3: the empty-space commit also drops a guide line.
    expect(guideLines.length).toBe(1)
    expect(guidePoints.length).toBe(1)
  })

  it('a plain ground click (no edge provenance) stays in measure mode', () => {
    const { scene, guideLines, guidePoints } = makeWasmScene()
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 0, kind: 'ground' }), RAY)
    tool.onPointerDown(makeSnap({ x: 3, y: 3, z: 0, kind: 'ground' }), RAY)

    // WP-7 item 3: the empty-space commit also drops a guide line.
    expect(guideLines.length).toBe(1)
    expect(guidePoints.length).toBe(1)
  })
})

describe('TapeMeasureTool — component-instance and group member edges', () => {
  it('an instanced solid-edge snap (object+element+instance) enters parallel mode via the pose-mapped in-instance lookup', () => {
    const { scene, guideLines } = makeWasmScene()
    const { tool, onGuideCreated } = makeTool(scene)

    tool.onPointerDown(
      makeSnap({
        x: 1, y: 0, z: 0, kind: 'on-edge', elementKind: 'edge',
        object: 7n, element: 3n, instance: 99n,
      }),
      RAY,
    )
    tool.onPointerMove(makeSnap({ x: 1, y: 0, z: 0.5 }), RAY) // pull sideways (edge runs +Y here)
    tool.onPointerDown(makeSnap({ x: 1, y: 0, z: 0.5 }), RAY) // commit

    const wasm = scene as unknown as {
      edge_endpoints_in_instance: ReturnType<typeof vi.fn>
      edge_endpoints: ReturnType<typeof vi.fn>
    }
    expect(wasm.edge_endpoints_in_instance).toHaveBeenCalledWith(99n, 7n, 3n)
    expect(wasm.edge_endpoints).not.toHaveBeenCalled()
    expect(guideLines.length).toBe(1)
    expect(onGuideCreated).toHaveBeenCalledTimes(1)
    // Direction is the pose-mapped in-instance edge direction (+Y), not the
    // plain edge_endpoints stub's (+X) — proves the right path resolved it.
    const [, , , dx, dy, dz] = guideLines[0]
    expect([dx, dy, dz]).toEqual([0, 1, 0])
  })

  it('a definition-owned sketch-edge snap with no instance of its own resolves through the instance edit context', () => {
    const { scene, guideLines } = makeWasmScene()
    const { tool, onGuideCreated } = makeTool(scene)
    tool.setEditContext({ kind: 'instance', id: 42n, component: 5n })

    tool.onPointerDown(
      makeSnap({ x: 1, y: 0, z: 0, kind: 'on-edge', elementKind: 'sketch-edge', sketch: 8n, element: 6n }),
      RAY,
    )
    tool.onPointerMove(makeSnap({ x: 1.5, y: 0, z: 0 }), RAY) // pull sideways (edge runs +Z here)
    tool.onPointerDown(makeSnap({ x: 1.5, y: 0, z: 0 }), RAY) // commit

    const wasm = scene as unknown as {
      sketch_edge_endpoints_in_instance: ReturnType<typeof vi.fn>
      sketch_edge_endpoints: ReturnType<typeof vi.fn>
    }
    expect(wasm.sketch_edge_endpoints_in_instance).toHaveBeenCalledWith(42n, 8n, 6n)
    expect(wasm.sketch_edge_endpoints).not.toHaveBeenCalled()
    expect(guideLines.length).toBe(1)
    expect(onGuideCreated).toHaveBeenCalledTimes(1)
    // Direction is the in-instance stub's (+Z), confirming the edit-context
    // instance id (not just a snap-carried one) drove the resolution.
    const [, , , dx, dy, dz] = guideLines[0]
    expect([dx, dy, dz]).toEqual([0, 0, 1])
  })

  it('a plain WORLD sketch hovered inside an instance edit context falls back to sketch_edge_endpoints', () => {
    const { scene, guideLines } = makeWasmScene()
    ;(scene as unknown as { sketch_edge_endpoints_in_instance: ReturnType<typeof vi.fn> })
      .sketch_edge_endpoints_in_instance.mockReturnValue(undefined) // kernel refuses: owner mismatch
    const { tool, onGuideCreated } = makeTool(scene)
    tool.setEditContext({ kind: 'instance', id: 42n, component: 5n })

    tool.onPointerDown(
      makeSnap({ x: 1, y: 0, z: 0, kind: 'on-edge', elementKind: 'sketch-edge', sketch: 8n, element: 6n }),
      RAY,
    )
    tool.onPointerMove(makeSnap({ x: 1, y: 0.5, z: 0 }), RAY) // pull sideways (edge runs +X here)
    tool.onPointerDown(makeSnap({ x: 1, y: 0.5, z: 0 }), RAY) // commit

    const wasm = scene as unknown as {
      sketch_edge_endpoints_in_instance: ReturnType<typeof vi.fn>
      sketch_edge_endpoints: ReturnType<typeof vi.fn>
    }
    expect(wasm.sketch_edge_endpoints_in_instance).toHaveBeenCalledWith(42n, 8n, 6n)
    expect(wasm.sketch_edge_endpoints).toHaveBeenCalledWith(8n, 6n)
    expect(guideLines.length).toBe(1)
    expect(onGuideCreated).toHaveBeenCalledTimes(1)
    // Direction is the plain sketch_edge_endpoints stub's (+X) — the fallback ran.
    const [, , , dx, dy, dz] = guideLines[0]
    expect([dx, dy, dz]).toEqual([1, 0, 0])
  })

  it('a GROUP-member edge (a world-space object nested under a group) still resolves via the plain edge_endpoints path', () => {
    const { scene, guideLines } = makeWasmScene()
    const { tool, onGuideCreated } = makeTool(scene)

    // A grouped object is still is_world() in the kernel — no `instance` on
    // the snap, exactly like an ungrouped object edge. This regression test
    // just confirms that groups were never routed through the new
    // in-instance path and stay that way.
    tool.onPointerDown(
      makeSnap({ x: 1, y: 0, z: 0, kind: 'on-edge', elementKind: 'edge', object: 12n, element: 4n }),
      RAY,
    )
    tool.onPointerMove(makeSnap({ x: 1, y: 0.5, z: 0 }), RAY)
    tool.onPointerDown(makeSnap({ x: 1, y: 0.5, z: 0 }), RAY)

    const wasm = scene as unknown as {
      edge_endpoints: ReturnType<typeof vi.fn>
      edge_endpoints_in_instance: ReturnType<typeof vi.fn>
    }
    expect(wasm.edge_endpoints).toHaveBeenCalledWith(12n, 4n)
    expect(wasm.edge_endpoints_in_instance).not.toHaveBeenCalled()
    expect(guideLines.length).toBe(1)
    expect(onGuideCreated).toHaveBeenCalledTimes(1)
    const [, , , dx, dy, dz] = guideLines[0]
    expect([dx, dy, dz]).toEqual([1, 0, 0])
  })
})

describe('TapeMeasureTool — parallel guides from axes and guide lines (playtest: guide off an axis)', () => {
  it('an on-axis snap enters parallel mode and commits a guide parallel to the axis', () => {
    const { scene, guideLines, guidePoints } = makeWasmScene()
    const { tool, onGuideCreated } = makeTool(scene)

    // The kernel resolves the axis ANALYTICALLY: the snap carries the
    // on-line point and the axis direction, no element handle at all.
    tool.onPointerDown(
      makeSnap({ x: 2, y: 0, z: 0, kind: 'on-axis', direction: [1, 0, 0] }),
      RAY,
    )
    tool.onPointerMove(makeSnap({ x: 2, y: 1, z: 0 }), RAY) // pull off the axis
    tool.onPointerDown(makeSnap({ x: 2, y: 1, z: 0 }), RAY) // commit

    expect(guideLines.length).toBe(1)
    expect(guidePoints.length).toBe(0)
    expect(onGuideCreated).toHaveBeenCalledTimes(1)
    const [ox, oy, oz, dx, dy, dz] = guideLines[0]
    expect([dx, dy, dz]).toEqual([1, 0, 0]) // parallel to the red axis
    expect([ox, oy, oz]).toEqual([2, 1, 0]) // through the pulled-to point
  })

  it('a typed exact offset commits the axis-parallel guide at that distance', () => {
    const { scene, guideLines } = makeWasmScene()
    const { tool } = makeTool(scene)

    tool.onPointerDown(
      makeSnap({ x: 0.05, y: 0, z: 0, kind: 'on-axis', direction: [1, 0, 0] }),
      RAY,
    )
    tool.onPointerMove(makeSnap({ x: 0.05, y: 0.005, z: 0 }), RAY) // cm-scale pull
    const key = (k: string) => ({ key: k, preventDefault: () => { /* no-op */ } }) as unknown as KeyboardEvent
    for (const k of ['0', '.', '0', '2']) tool.onKey(key(k))
    tool.onKey(key('Enter'))

    expect(guideLines.length).toBe(1)
    const [ox, oy, oz, dx, dy, dz] = guideLines[0]
    expect([dx, dy, dz]).toEqual([1, 0, 0])
    expect(ox).toBeCloseTo(0.05, 12)
    expect(oy).toBeCloseTo(0.02, 12) // exactly 2 cm off the axis
    expect(oz).toBeCloseTo(0, 12)
  })

  it('an on-guide snap sources a parallel guide from the existing guide line', () => {
    const { scene, guideLines } = makeWasmScene()
    const { tool } = makeTool(scene)

    tool.onPointerDown(
      makeSnap({ x: 1, y: 2, z: 0, kind: 'on-guide', direction: [0, 1, 0] }),
      RAY,
    )
    tool.onPointerMove(makeSnap({ x: 1.5, y: 2, z: 0 }), RAY)
    tool.onPointerDown(makeSnap({ x: 1.5, y: 2, z: 0 }), RAY)

    expect(guideLines.length).toBe(1)
    const [ox, , , dx, dy, dz] = guideLines[0]
    expect([dx, dy, dz]).toEqual([0, 1, 0])
    expect(ox).toBeCloseTo(1.5, 12)
  })

  it('an on-axis snap WITHOUT a direction falls back to measure mode (no throw)', () => {
    const { scene, guideLines, guidePoints } = makeWasmScene()
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 2, y: 0, z: 0, kind: 'on-axis' }), RAY)
    tool.onPointerDown(makeSnap({ x: 3, y: 1, z: 0, kind: 'ground' }), RAY)

    // WP-7 item 3: measure ended in empty space → point AND line, no throw.
    expect(guideLines.length).toBe(1)
    expect(guidePoints.length).toBe(1)
  })

  it('the measure flow still drops a guide point in empty space', () => {
    const { scene, guidePoints } = makeWasmScene()
    const { tool, onGuideCreated } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 0, kind: 'ground' }), RAY)
    tool.onPointerMove(makeSnap({ x: 2, y: 1, z: 0, kind: 'ground' }), RAY)
    tool.onPointerDown(makeSnap({ x: 2, y: 1, z: 0, kind: 'ground' }), RAY)

    expect(guidePoints).toEqual([[2, 1, 0]])
    expect(onGuideCreated).toHaveBeenCalledTimes(1)
  })
})

// From-point axis inference (WP-5, Kurt's requirement #5 — the headline
// feature of this work package): `anchor: p0` turns on the kernel's soft
// 5°-cone `SnapKind::OnAxis` inference for a from-point measurement.
describe('TapeMeasureTool — from-point axis inference', () => {
  it('measure-stage snapConstraint() always includes anchor: p0 once a gesture has started', () => {
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 1, y: 2, z: 3, kind: 'ground' }), RAY)
    expect(tool.snapConstraint(RAY)).toEqual({ anchor: [1, 2, 3] })
  })

  it('idle snapConstraint() carries no anchor — nothing to anchor to yet', () => {
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)

    // No sketch under the cursor in this fixture (see makeWasmScene) —
    // idle, unconstrained: null, not an anchor-carrying constraint.
    expect(tool.snapConstraint(RAY)).toBeNull()
  })

  it('the anchor clears once the gesture ends — the NEXT gesture reports its own p0', () => {
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 0, kind: 'ground' }), RAY)
    tool.onPointerDown(makeSnap({ x: 2, y: 2, z: 0, kind: 'ground' }), RAY) // commit, back to idle

    expect(tool.snapConstraint(RAY)).toBeNull() // idle again, no sketch hovered

    tool.onPointerDown(makeSnap({ x: 5, y: 6, z: 0, kind: 'ground' }), RAY)
    expect(tool.snapConstraint(RAY)).toEqual({ anchor: [5, 6, 0] })
  })
})

// Resize-the-model confirmation (design tool-parity §3): typing a length
// right after measuring between two REAL, already-known points arms a
// confirmation instead of dropping a guide point immediately.
describe('TapeMeasureTool — resize-the-model confirmation', () => {
  it('arms the confirmation when both measured points rest on real geometry', () => {
    const { scene, guidePoints } = makeWasmScene()
    const { tool, onRescaleArmed } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint' }), RAY)
    tool.onPointerMove(makeSnap({ x: 2, y: 0, z: 0, kind: 'endpoint' }), RAY)
    typeAndEnter(tool, '3')

    expect(onRescaleArmed).toHaveBeenCalledTimes(1)
    const info = onRescaleArmed.mock.calls[0][0]
    expect(info.currentDistance).toBeCloseTo(2)
    expect(info.typedDistance).toBeCloseTo(3)
    expect(info.factor).toBeCloseTo(1.5)
    // The measurement's FIRST point (group-session.md's scoped-rescale
    // anchor) — the second click's endpoint plays no part in it.
    expect(info.anchor).toEqual([0, 0, 0])
    // Nothing committed yet — the decision is still pending.
    expect(guidePoints.length).toBe(0)
    expect(scene.rescale_document).not.toHaveBeenCalled()
    expect(tool.capturingInput()).toBe(true)
  })

  it('confirmRescale applies rescale_document with the armed factor and returns to idle', () => {
    const { scene } = makeWasmScene()
    const { tool, onRescaleApplied } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint' }), RAY)
    tool.onPointerMove(makeSnap({ x: 2, y: 0, z: 0, kind: 'endpoint' }), RAY)
    typeAndEnter(tool, '3')

    tool.confirmRescale()

    expect(scene.rescale_document).toHaveBeenCalledWith(1.5)
    // The factor is passed through to the callback so the Viewport can
    // re-scale the camera about the same world-origin pivot (tool-parity §3
    // "the view jumps around" fix) — see viewport/math.test.ts for the pure
    // scaling math itself. `scoped` is false — no `confirmRescale` argument
    // was passed, the pre-existing whole-model default.
    expect(onRescaleApplied).toHaveBeenCalledWith(1.5, false)
    expect(tool.capturingInput()).toBe(false)
  })

  // Scoped rescale (docs/design/group-session.md's "Tape Measure scoped
  // rescale"): `confirmRescale(true, …)` — the caller's decision, made from
  // its own `sessionStack` at arm time and threaded straight through here —
  // calls `rescale_session` anchored at the measurement's first point
  // instead of `rescale_document`, and reports `scoped: true` to the
  // Viewport so it skips the camera/grid companion scaling.
  it('confirmRescale(true, …) applies rescale_session anchored at the measured first point', () => {
    const { scene } = makeWasmScene()
    const { tool, onRescaleApplied } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 1, y: 2, z: 3, kind: 'endpoint' }), RAY)
    tool.onPointerMove(makeSnap({ x: 3, y: 2, z: 3, kind: 'endpoint' }), RAY)
    typeAndEnter(tool, '3')

    tool.confirmRescale(true, 'Group 1')

    expect(scene.rescale_document).not.toHaveBeenCalled()
    expect(scene.rescale_session).toHaveBeenCalledWith(1.5, 1, 2, 3)
    expect(onRescaleApplied).toHaveBeenCalledWith(1.5, true)
    expect(tool.capturingInput()).toBe(false)
  })

  it('a refused rescale_session toasts a scoped-aware message and still returns to idle', () => {
    const { scene } = makeWasmScene()
    ;(scene as unknown as { rescale_session: ReturnType<typeof vi.fn> }).rescale_session.mockImplementation(() => {
      throw new Error('ExplodeSessionNotOpen: no session frame is open')
    })
    const { tool, onToast } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint' }), RAY)
    tool.onPointerMove(makeSnap({ x: 2, y: 0, z: 0, kind: 'endpoint' }), RAY)
    typeAndEnter(tool, '3')

    tool.confirmRescale(true, 'Group 1')

    expect(onToast).toHaveBeenCalledWith(expect.stringContaining("Couldn't resize Group 1:"))
    expect(tool.capturingInput()).toBe(false)
  })

  it('cancelRescale falls through to the normal guide-point commit', () => {
    const { scene, guidePoints } = makeWasmScene()
    const { tool, onGuideCreated } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint' }), RAY)
    tool.onPointerMove(makeSnap({ x: 2, y: 0, z: 0, kind: 'endpoint' }), RAY)
    typeAndEnter(tool, '3')

    tool.cancelRescale()

    expect(scene.rescale_document).not.toHaveBeenCalled()
    expect(guidePoints.length).toBe(1)
    // The 3 m typed distance along +X from the origin.
    expect(guidePoints[0][0]).toBeCloseTo(3)
    expect(onGuideCreated).toHaveBeenCalledTimes(1)
    expect(tool.capturingInput()).toBe(false)
  })

  it('cancelRescale uses the createGuides snapshot from ARM time, not the live value (Fix 3 regression)', () => {
    // Armed while `createGuides` was true (the default — Ctrl/Cmd not held
    // at typing time); Ctrl/Cmd then gets held WHILE the confirmation dialog
    // is open (dispatched live from window-level listeners, per
    // setGuideCreationSuppressed's own doc) before Cancel is clicked. The
    // fallback guide drop must still behave as armed — guides ARE created —
    // not as the live (now-false) value would suggest.
    const { scene, guidePoints } = makeWasmScene()
    const { tool, onGuideCreated } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint' }), RAY)
    tool.onPointerMove(makeSnap({ x: 2, y: 0, z: 0, kind: 'endpoint' }), RAY)
    typeAndEnter(tool, '3') // arms with createGuides === true

    tool.setGuideCreationSuppressed(true) // Ctrl held while the dialog is open — live createGuides -> false
    tool.cancelRescale()

    expect(guidePoints.length).toBe(1) // still created, per the ARMED (true) snapshot
    expect(onGuideCreated).toHaveBeenCalledTimes(1)
    // The snapshot only governs this one call — createGuides is back to its
    // normal default afterward, via _resetToIdle().
    expect((tool as unknown as { createGuides: boolean }).createGuides).toBe(true)
  })

  it('the symmetric case: armed while createGuides was false, live value toggled back to true before Cancel — still no guide', () => {
    const { scene, guidePoints } = makeWasmScene()
    const { tool, onGuideCreated } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint' }), RAY)
    tool.setGuideCreationSuppressed(true) // Ctrl held before/at arm time
    tool.onPointerMove(makeSnap({ x: 2, y: 0, z: 0, kind: 'endpoint' }), RAY)
    typeAndEnter(tool, '3') // arms with createGuides === false

    tool.setGuideCreationSuppressed(false) // Ctrl released while the dialog is open — live createGuides -> true
    tool.cancelRescale()

    expect(guidePoints.length).toBe(0) // still suppressed, per the ARMED (false) snapshot
    expect(onGuideCreated).not.toHaveBeenCalled()
    expect((tool as unknown as { createGuides: boolean }).createGuides).toBe(true) // reset by _resetToIdle()
  })

  it('Escape while pending behaves like Cancel (drops the guide, no rescale)', () => {
    const { scene, guidePoints } = makeWasmScene()
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint' }), RAY)
    tool.onPointerMove(makeSnap({ x: 2, y: 0, z: 0, kind: 'endpoint' }), RAY)
    typeAndEnter(tool, '3')

    tool.onKey({ key: 'Escape' } as KeyboardEvent)

    expect(scene.rescale_document).not.toHaveBeenCalled()
    expect(guidePoints.length).toBe(1)
    expect(tool.capturingInput()).toBe(false)
  })

  it('does not arm when either endpoint is in empty space — commits the guide normally', () => {
    const { scene, guidePoints } = makeWasmScene()
    const { tool, onRescaleArmed } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'ground' }), RAY)
    tool.onPointerMove(makeSnap({ x: 2, y: 0, z: 0, kind: 'ground' }), RAY)
    typeAndEnter(tool, '3')

    expect(onRescaleArmed).not.toHaveBeenCalled()
    expect(guidePoints.length).toBe(1)
  })

  it('does not arm for a non-positive typed length', () => {
    const { scene, guidePoints } = makeWasmScene()
    const { tool, onRescaleArmed } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint' }), RAY)
    tool.onPointerMove(makeSnap({ x: 2, y: 0, z: 0, kind: 'endpoint' }), RAY)
    typeAndEnter(tool, '-3')

    expect(onRescaleArmed).not.toHaveBeenCalled()
    expect(guidePoints.length).toBe(1) // the normal (recessed-direction) commit still ran
  })

  it('confirmRescale/cancelRescale are safe no-ops when nothing is pending', () => {
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)

    expect(() => tool.confirmRescale()).not.toThrow()
    expect(() => tool.cancelRescale()).not.toThrow()
    expect(scene.rescale_document).not.toHaveBeenCalled()
  })

  it('a refused rescale_document toasts and still returns to idle', () => {
    const { scene } = makeWasmScene()
    ;(scene as unknown as { rescale_document: ReturnType<typeof vi.fn> }).rescale_document.mockImplementation(() => {
      throw new Error('InvalidRescaleFactor: rescale factor must be a positive, finite number')
    })
    const { tool, onToast } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint' }), RAY)
    tool.onPointerMove(makeSnap({ x: 2, y: 0, z: 0, kind: 'endpoint' }), RAY)
    typeAndEnter(tool, '3')
    tool.confirmRescale()

    expect(onToast).toHaveBeenCalledTimes(1)
    expect(tool.capturingInput()).toBe(false)
  })
})

// Scoped-rescale arm gate (adversarial-review finding 6, group-session.md):
// inference/snapping stays unscoped while a session is open, so a
// measurement's two points can land on geometry OUTSIDE the open session —
// arming a scoped resize on one would translate the session's contents by a
// wrong anchor while the measured distance itself never changes. `setSessionScope`
// (the Viewport's push, mirrored here directly) gates the arm on BOTH
// endpoints' own `object`/`instance`/`sketch` attribution.
describe('TapeMeasureTool — scoped-rescale arm gate (adversarial-review finding 6)', () => {
  it('arms normally when both endpoints are on in-scope OBJECT geometry', () => {
    const { scene } = makeWasmScene()
    const { tool, onRescaleArmed, onToast } = makeTool(scene)
    tool.setSessionScope({ objectIds: new Set([7n]), instanceIds: new Set(), sketchIds: new Set() })

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint', object: 7n }), RAY)
    tool.onPointerMove(makeSnap({ x: 2, y: 0, z: 0, kind: 'endpoint', object: 7n }), RAY)
    typeAndEnter(tool, '3')

    expect(onRescaleArmed).toHaveBeenCalledTimes(1)
    expect(onToast).not.toHaveBeenCalled()
  })

  it('declines the arm — toast, no dialog, falls through to the ordinary guide commit — when the SECOND endpoint is on OUT-of-scope geometry', () => {
    const { scene, guidePoints } = makeWasmScene()
    const { tool, onRescaleArmed, onToast } = makeTool(scene)
    tool.setSessionScope({ objectIds: new Set([7n]), instanceIds: new Set(), sketchIds: new Set() })

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint', object: 7n }), RAY) // in scope
    tool.onPointerMove(makeSnap({ x: 2, y: 0, z: 0, kind: 'endpoint', object: 42n }), RAY) // NOT in scope
    typeAndEnter(tool, '3')

    expect(onRescaleArmed).not.toHaveBeenCalled()
    expect(onToast).toHaveBeenCalledTimes(1)
    expect(onToast.mock.calls[0][0]).toMatch(/outside the open group or component/i)
    // Declined arms fall through exactly like an empty-space endpoint would:
    // the ordinary typed-exact guide-point commit still runs.
    expect(guidePoints.length).toBe(1)
    expect(scene.rescale_document).not.toHaveBeenCalled()
    expect(scene.rescale_session).not.toHaveBeenCalled()
  })

  it('declines the arm when the FIRST endpoint is on out-of-scope geometry, even though the second is in scope', () => {
    const { scene } = makeWasmScene()
    const { tool, onRescaleArmed, onToast } = makeTool(scene)
    tool.setSessionScope({ objectIds: new Set([7n]), instanceIds: new Set(), sketchIds: new Set() })

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint', object: 42n }), RAY) // NOT in scope
    tool.onPointerMove(makeSnap({ x: 2, y: 0, z: 0, kind: 'endpoint', object: 7n }), RAY) // in scope
    typeAndEnter(tool, '3')

    expect(onRescaleArmed).not.toHaveBeenCalled()
    expect(onToast).toHaveBeenCalledTimes(1)
  })

  it('an INSTANCE-attributed endpoint (object reached through a placed component) is gated on the instance id, not the definition-local object id', () => {
    const { scene } = makeWasmScene()
    const { tool, onRescaleArmed } = makeTool(scene)
    tool.setSessionScope({ objectIds: new Set(), instanceIds: new Set([50n]), sketchIds: new Set() })

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint', object: 3n, instance: 50n }), RAY)
    tool.onPointerMove(makeSnap({ x: 2, y: 0, z: 0, kind: 'endpoint', object: 3n, instance: 50n }), RAY)
    typeAndEnter(tool, '3')

    expect(onRescaleArmed).toHaveBeenCalledTimes(1)
  })

  it('the same instance-attributed endpoint declines when the PLACEMENT (not the definition object) is out of scope', () => {
    const { scene } = makeWasmScene()
    const { tool, onRescaleArmed, onToast } = makeTool(scene)
    // The definition-local object id (3n) happens to coincide with something
    // in `objectIds` — the gate must still key off `instance`, not `object`,
    // once a snap carries both.
    tool.setSessionScope({ objectIds: new Set([3n]), instanceIds: new Set([50n]), sketchIds: new Set() })

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint', object: 3n, instance: 50n }), RAY)
    tool.onPointerMove(makeSnap({ x: 2, y: 0, z: 0, kind: 'endpoint', object: 3n, instance: 99n }), RAY) // different placement
    typeAndEnter(tool, '3')

    expect(onRescaleArmed).not.toHaveBeenCalled()
    expect(onToast).toHaveBeenCalledTimes(1)
  })

  it('a SKETCH-attributed endpoint is gated on sketchIds', () => {
    const { scene } = makeWasmScene()
    const { tool, onRescaleArmed, onToast } = makeTool(scene)
    tool.setSessionScope({ objectIds: new Set(), instanceIds: new Set(), sketchIds: new Set([8n]) })

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint', elementKind: 'sketch-edge', sketch: 8n }), RAY)
    tool.onPointerMove(makeSnap({ x: 2, y: 0, z: 0, kind: 'endpoint', elementKind: 'sketch-edge', sketch: 9n }), RAY) // different sketch
    typeAndEnter(tool, '3')

    expect(onRescaleArmed).not.toHaveBeenCalled()
    expect(onToast).toHaveBeenCalledTimes(1)
  })

  it('imposes no restriction at all when no session is open (setSessionScope(null), the default)', () => {
    const { scene } = makeWasmScene()
    const { tool, onRescaleArmed } = makeTool(scene)
    // Never called `setSessionScope` — `_sessionScope` starts `null`.

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint', object: 7n }), RAY)
    tool.onPointerMove(makeSnap({ x: 2, y: 0, z: 0, kind: 'endpoint', object: 42n }), RAY)
    typeAndEnter(tool, '3')

    expect(onRescaleArmed).toHaveBeenCalledTimes(1)
  })

  it('an out-of-scope RECALLED arm (idle "5" + Enter) declines too, restoring the frozen reading and leaving the recall intact', () => {
    const { scene } = makeWasmScene()
    const { tool, onRescaleArmed, onToast, onMeasurement } = makeTool(scene)
    tool.setSessionScope({ objectIds: new Set([7n]), instanceIds: new Set(), sketchIds: new Set() })

    // A real two-click measurement, both ends on IN-scope geometry, so the
    // recall is recorded.
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint', object: 7n }), RAY)
    tool.onPointerMove(makeSnap({ x: 2, y: 0, z: 0, kind: 'endpoint', object: 7n }), RAY)
    tool.onPointerDown(makeSnap({ x: 2, y: 0, z: 0, kind: 'endpoint', object: 7n }), RAY)

    // The scope then narrows (e.g. a DIFFERENT, smaller session opened) so
    // the SAME recalled measurement no longer qualifies.
    tool.setSessionScope({ objectIds: new Set([99n]), instanceIds: new Set(), sketchIds: new Set() })

    pressKey(tool, '5')
    pressKey(tool, 'Enter')

    expect(onRescaleArmed).not.toHaveBeenCalled()
    expect(onToast).toHaveBeenCalledTimes(1)
    const last = onMeasurement.mock.calls[onMeasurement.mock.calls.length - 1]
    expect(last).toEqual([formatLength(2), true]) // frozen original reading restored

    // The recall itself survives — a fresh in-scope measurement (or a
    // widened scope) could still use it; this mirrors `cancelRescale`'s own
    // fromRecall contract.
    expect(tool.capturesKey('5')).toBe(true)
  })
})

// Preview rendering — the guide-preview line + offset connector's colors
// (tape-measure-rework WP-6): the long guide line always reads neutral
// (GUIDE_COLOR), while the connector (parallel mode) and the measure segment
// (measure mode) pick up the axis-color rule shared with LineTool/
// ProtractorTool/SliceTool's own axis previews.
describe('TapeMeasureTool — preview connector/segment color (WP-6)', () => {
  it('the long guide-preview line is always the neutral GUIDE_COLOR, regardless of offset direction', () => {
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)

    tool.onPointerDown(
      makeSnap({ x: 1, y: 0, z: 0, kind: 'on-edge', elementKind: 'edge', object: 7n, element: 3n }),
      RAY,
    )
    tool.onPointerMove(makeSnap({ x: 1, y: 1, z: 0 }), RAY) // pull straight +Y — an axis-aligned offset

    const { previewLine } = previewInternals(tool)
    expect(previewLine).not.toBeNull()
    const mat = previewLine!.material as THREE.LineBasicMaterial
    expect(mat.color.getHex()).toBe(new THREE.LineBasicMaterial({ color: GUIDE_COLOR }).color.getHex())
  })

  it('an axis-aligned offset (parallel mode) colors the connector by the matching drawing axis', () => {
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)

    // Edge is +X (0,0,0)-(2,0,0) per the edge_endpoints stub; edgePoint is
    // the (1,0,0) pick. Pulling the cursor straight to (1,1,0) offsets
    // exactly along +Y — within 3 degrees of the green drawing axis.
    tool.onPointerDown(
      makeSnap({ x: 1, y: 0, z: 0, kind: 'on-edge', elementKind: 'edge', object: 7n, element: 3n }),
      RAY,
    )
    tool.onPointerMove(makeSnap({ x: 1, y: 1, z: 0 }), RAY)

    const { previewConnector } = previewInternals(tool)
    expect(previewConnector).not.toBeNull()
    const mat = previewConnector!.material as THREE.LineBasicMaterial
    expect(mat.color.getHex()).toBe(expectedPreviewColorHex([0, 1, 0]))
  })

  it('a 45-degree-off-axis offset colors the connector with the neutral GUIDE_COLOR fallback', () => {
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)

    // Offset direction (0, 1, 1)/sqrt(2) sits exactly 45 degrees off both the
    // green (Y) and blue (Z) axes — comfortably outside the 3-degree tolerance.
    tool.onPointerDown(
      makeSnap({ x: 1, y: 0, z: 0, kind: 'on-edge', elementKind: 'edge', object: 7n, element: 3n }),
      RAY,
    )
    tool.onPointerMove(makeSnap({ x: 1, y: 1, z: 1 }), RAY)

    const { previewConnector } = previewInternals(tool)
    expect(previewConnector).not.toBeNull()
    const mat = previewConnector!.material as THREE.LineBasicMaterial
    expect(mat.color.getHex()).toBe(new THREE.LineBasicMaterial({ color: GUIDE_COLOR }).color.getHex())
    // Sanity: this really is the neutral fallback, not a coincidentally
    // matching axis color.
    expect(mat.color.getHex()).not.toBe(expectedPreviewColorHex([0, 1, 0]))
    expect(mat.color.getHex()).not.toBe(expectedPreviewColorHex([0, 0, 1]))
  })

  it("the connector's endpoints are exactly edgePoint and origin", () => {
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)

    tool.onPointerDown(
      makeSnap({ x: 1, y: 0, z: 0, kind: 'on-edge', elementKind: 'edge', object: 7n, element: 3n }),
      RAY,
    )
    tool.onPointerMove(makeSnap({ x: 1, y: 0.4, z: 0.3 }), RAY)

    const { previewConnector } = previewInternals(tool)
    expect(previewConnector).not.toBeNull()
    const pos = Array.from(previewConnector!.geometry.attributes.position.array)
    // edgePoint (1,0,0) -> origin (1, 0.4, 0.3) — the offset is already
    // exactly perpendicular to the +X edge, so origin equals the pulled-to
    // cursor position unchanged. Float32 storage, so compare with tolerance.
    const expected = [1, 0, 0, 1, 0.4, 0.3]
    for (let i = 0; i < expected.length; i++) {
      expect(pos[i]).toBeCloseTo(expected[i], 6)
    }
  })

  it('no connector is built when edgePoint and origin coincide (cursor still on the edge)', () => {
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)

    tool.onPointerDown(
      makeSnap({ x: 1, y: 0, z: 0, kind: 'on-edge', elementKind: 'edge', object: 7n, element: 3n }),
      RAY,
    )
    // Collinear with the edge — zero perpendicular offset.
    tool.onPointerMove(makeSnap({ x: 1.5, y: 0, z: 0 }), RAY)

    const { previewLine, previewConnector } = previewInternals(tool)
    expect(previewConnector).toBeNull()
    expect(previewLine).not.toBeNull() // the long guide line still draws
  })

  it('the measure-stage p0->p1 segment picks up the same axis-color rule', () => {
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'ground' }), RAY)
    tool.onPointerMove(makeSnap({ x: 0, y: 3, z: 0, kind: 'ground' }), RAY) // +Y, axis-aligned

    const { previewLine } = previewInternals(tool)
    expect(previewLine).not.toBeNull()
    const mat = previewLine!.material as THREE.LineBasicMaterial
    expect(mat.color.getHex()).toBe(expectedPreviewColorHex([0, 1, 0]))
  })

  it('disposing (cancel) tears down every geometry and material the preview created — no leaked THREE resources', () => {
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)

    tool.onPointerDown(
      makeSnap({ x: 1, y: 0, z: 0, kind: 'on-edge', elementKind: 'edge', object: 7n, element: 3n }),
      RAY,
    )
    tool.onPointerMove(makeSnap({ x: 1, y: 1, z: 0 }), RAY) // both guide line + connector present

    const { previewLine, previewConnector } = previewInternals(tool)
    expect(previewLine).not.toBeNull()
    expect(previewConnector).not.toBeNull()

    const preview = (tool as unknown as { preview: THREE.Group }).preview
    expect(preview.children.length).toBe(2)

    const lineGeoSpy = vi.spyOn(previewLine!.geometry, 'dispose')
    const lineMatSpy = vi.spyOn(previewLine!.material as THREE.Material, 'dispose')
    const connGeoSpy = vi.spyOn(previewConnector!.geometry, 'dispose')
    const connMatSpy = vi.spyOn(previewConnector!.material as THREE.Material, 'dispose')

    tool.cancel()

    expect(lineGeoSpy).toHaveBeenCalledTimes(1)
    expect(lineMatSpy).toHaveBeenCalledTimes(1)
    expect(connGeoSpy).toHaveBeenCalledTimes(1)
    expect(connMatSpy).toHaveBeenCalledTimes(1)
    expect(preview.children.length).toBe(0)

    const after = previewInternals(tool)
    expect(after.previewLine).toBeNull()
    expect(after.previewConnector).toBeNull()
  })
})

// Ctrl/Cmd measure-only mode (tape-measure-rework WP-7 item 1): holding
// Ctrl (Windows/Linux) or Cmd (macOS) during a gesture takes the measurement
// WITHOUT creating a guide, same as SketchUp.
describe('TapeMeasureTool — Ctrl/Cmd measure-only mode (WP-7 item 1)', () => {
  it('a suppressed measure-mode gesture completes the readout but creates neither a guide line nor a guide point', () => {
    const { scene, guideLines, guidePoints } = makeWasmScene()
    const { tool, onMeasurement, onGuideCreated } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'ground' }), RAY)
    tool.setGuideCreationSuppressed(true) // Ctrl/Cmd held mid-gesture
    tool.onPointerMove(makeSnap({ x: 3, y: 0, z: 0, kind: 'ground' }), RAY)
    tool.onPointerDown(makeSnap({ x: 3, y: 0, z: 0, kind: 'ground' }), RAY) // commit

    expect(guideLines.length).toBe(0)
    expect(guidePoints.length).toBe(0)
    expect(onGuideCreated).not.toHaveBeenCalled()
    // The readout still happened at some point in the gesture (the final
    // call, from `_resetToIdle`'s default 'freeze' mode, re-pushes the last
    // reading with `frozen: true` rather than clearing it — check the history).
    const calls = onMeasurement.mock.calls.map((c) => c[0] as string)
    expect(calls.some((s) => /\d/.test(s))).toBe(true)
    expect(tool.capturingInput()).toBe(false) // the gesture still completed
  })

  it('suppresses a parallel-guide commit too, while the offset readout still updates', () => {
    const { scene, guideLines } = makeWasmScene()
    const { tool, onGuideCreated, onMeasurement } = makeTool(scene)

    tool.onPointerDown(
      makeSnap({ x: 1, y: 0, z: 0, kind: 'on-edge', elementKind: 'edge', object: 7n, element: 3n }),
      RAY,
    )
    tool.setGuideCreationSuppressed(true)
    tool.onPointerMove(makeSnap({ x: 1, y: 0.5, z: 0 }), RAY)
    tool.onPointerDown(makeSnap({ x: 1, y: 0.5, z: 0 }), RAY) // commit

    expect(guideLines.length).toBe(0)
    expect(onGuideCreated).not.toHaveBeenCalled()
    expect(tool.capturingInput()).toBe(false)
    const calls = onMeasurement.mock.calls.map((c) => c[0] as string)
    expect(calls.some((s) => /\d/.test(s))).toBe(true) // offset readout still ran
  })

  it('statusHint() reflects suppression while held, and stops reflecting it once released', () => {
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'ground' }), RAY)
    expect(tool.statusHint()).not.toContain('measuring only')

    tool.setGuideCreationSuppressed(true)
    expect(tool.statusHint()).toContain('measuring only')

    tool.setGuideCreationSuppressed(false)
    expect(tool.statusHint()).not.toContain('measuring only')
  })

  it('releasing the modifier mid-gesture restores guide creation for the eventual commit', () => {
    const { scene, guideLines, guidePoints } = makeWasmScene()
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'ground' }), RAY)
    tool.setGuideCreationSuppressed(true)
    tool.onPointerMove(makeSnap({ x: 3, y: 0, z: 0, kind: 'ground' }), RAY)
    tool.setGuideCreationSuppressed(false) // released before the commit click
    tool.onPointerDown(makeSnap({ x: 3, y: 0, z: 0, kind: 'ground' }), RAY) // commit

    expect(guideLines.length).toBe(1)
    expect(guidePoints.length).toBe(1)
  })

  it('createGuides resets to true after cancel()/_resetToIdle()', () => {
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'ground' }), RAY)
    tool.setGuideCreationSuppressed(true)
    tool.cancel()

    expect((tool as unknown as { createGuides: boolean }).createGuides).toBe(true)
  })

  it('calling setGuideCreationSuppressed while idle does not throw and does not leak into the next gesture', () => {
    const { scene, guideLines, guidePoints } = makeWasmScene()
    const { tool } = makeTool(scene)

    expect(() => tool.setGuideCreationSuppressed(true)).not.toThrow()
    // Nothing is armed, so this held state does nothing observable — the
    // next click's own `setGuideCreationSuppressed` calls are what matter
    // in practice; here nothing else toggles it, so it simply carries
    // forward (consistent with "Ctrl already held before the first click").
    tool.setGuideCreationSuppressed(false)
    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'ground' }), RAY)
    tool.onPointerDown(makeSnap({ x: 2, y: 0, z: 0, kind: 'ground' }), RAY)

    expect(guideLines.length).toBe(1)
    expect(guidePoints.length).toBe(1)
  })
})

// Empty-space measure commit also drops a guide LINE, not just a guide
// point (tape-measure-rework WP-7 item 3) — mirrors SketchUp: measuring
// between two points in empty space leaves both the infinite guide line
// through them and a guide point at the second click.
describe('TapeMeasureTool — empty-space measure commit also drops a guide line (WP-7 item 3)', () => {
  it('a well-separated two-point commit calls add_guide_line with p0/the unit direction, and add_guide_point with p1', () => {
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'ground' }), RAY)
    tool.onPointerMove(makeSnap({ x: 0, y: 4, z: 3, kind: 'ground' }), RAY) // dist 5, dir (0, 0.8, 0.6)
    tool.onPointerDown(makeSnap({ x: 0, y: 4, z: 3, kind: 'ground' }), RAY) // commit

    const wasm = scene as unknown as {
      add_guide_line: ReturnType<typeof vi.fn>
      add_guide_point: ReturnType<typeof vi.fn>
    }
    expect(wasm.add_guide_line).toHaveBeenCalledTimes(1)
    const [ox, oy, oz, dx, dy, dz] = wasm.add_guide_line.mock.calls[0] as number[]
    expect([ox, oy, oz]).toEqual([0, 0, 0])
    expect(dx).toBeCloseTo(0, 12)
    expect(dy).toBeCloseTo(0.8, 12)
    expect(dz).toBeCloseTo(0.6, 12)

    expect(wasm.add_guide_point).toHaveBeenCalledTimes(1)
    expect(wasm.add_guide_point).toHaveBeenCalledWith(0, 4, 3)
  })

  it('a degenerate two-point commit (p1 === p0) skips the guide line but still drops the point', () => {
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 1, kind: 'ground' }), RAY)
    tool.onPointerDown(makeSnap({ x: 1, y: 1, z: 1, kind: 'ground' }), RAY) // commit, p1 === p0

    const wasm = scene as unknown as {
      add_guide_line: ReturnType<typeof vi.fn>
      add_guide_point: ReturnType<typeof vi.fn>
    }
    expect(wasm.add_guide_line).not.toHaveBeenCalled()
    expect(wasm.add_guide_point).toHaveBeenCalledTimes(1)
  })

  it('createGuides === false (WP-7 item 1) skips BOTH the line and the point together', () => {
    const { scene, guideLines, guidePoints } = makeWasmScene()
    const { tool, onGuideCreated } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'ground' }), RAY)
    tool.setGuideCreationSuppressed(true)
    tool.onPointerMove(makeSnap({ x: 3, y: 4, z: 0, kind: 'ground' }), RAY)
    tool.onPointerDown(makeSnap({ x: 3, y: 4, z: 0, kind: 'ground' }), RAY) // commit

    expect(guideLines.length).toBe(0)
    expect(guidePoints.length).toBe(0)
    expect(onGuideCreated).not.toHaveBeenCalled()
  })
})

// Persistent frozen readout (tape-measure-rework part 1): the corner widget
// keeps the last measurement on screen until the tool switches, instead of
// clearing on every commit/abort.
describe('TapeMeasureTool — persistent frozen readout', () => {
  it('a real two-click measure commit ends with the readout frozen at the final distance', () => {
    const { scene } = makeWasmScene()
    const { tool, onMeasurement } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint' }), RAY)
    tool.onPointerMove(makeSnap({ x: 2, y: 0, z: 0, kind: 'endpoint' }), RAY)
    tool.onPointerDown(makeSnap({ x: 2, y: 0, z: 0, kind: 'endpoint' }), RAY) // commit

    const last = onMeasurement.mock.calls[onMeasurement.mock.calls.length - 1]
    expect(last).toEqual([formatLength(2), true])
  })

  it('cancel() after a frozen commit clears the readout', () => {
    const { scene } = makeWasmScene()
    const { tool, onMeasurement } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint' }), RAY)
    tool.onPointerMove(makeSnap({ x: 2, y: 0, z: 0, kind: 'endpoint' }), RAY)
    tool.onPointerDown(makeSnap({ x: 2, y: 0, z: 0, kind: 'endpoint' }), RAY) // commit

    tool.cancel()

    const last = onMeasurement.mock.calls[onMeasurement.mock.calls.length - 1]
    expect(last).toEqual(['', false])
  })
})

// Retroactive rescale (tape-measure-rework part 2): typing a length AFTER a
// real two-click measurement (both ends on real geometry) re-arms the same
// "resize the model?" confirmation the mid-gesture typed path already had,
// with no need to have mouse and keyboard "in play" at the same time.
describe('TapeMeasureTool — retroactive rescale recall', () => {
  /** Click two real (on-geometry) points 2m apart, completing an ordinary
   *  eligible measurement, and return the tool + spies. */
  function measureEligible(scene: WasmScene) {
    const made = makeTool(scene)
    made.tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint' }), RAY)
    made.tool.onPointerMove(makeSnap({ x: 2, y: 0, z: 0, kind: 'endpoint' }), RAY)
    made.tool.onPointerDown(makeSnap({ x: 2, y: 0, z: 0, kind: 'endpoint' }), RAY)
    return made
  }

  it('a digit at idle after an ELIGIBLE commit opens the buffer with a live (non-frozen) echo', () => {
    const { scene } = makeWasmScene()
    const { tool, onMeasurement } = measureEligible(scene)

    expect(tool.capturesKey('5')).toBe(true)
    pressKey(tool, '5')

    const last = onMeasurement.mock.calls[onMeasurement.mock.calls.length - 1]
    expect(last[1]).toBe(false)
    expect(last[0]).toContain('5')
  })

  it('a digit at idle after an INELIGIBLE commit (one end in empty space) is not captured', () => {
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'ground' }), RAY) // empty space
    tool.onPointerMove(makeSnap({ x: 2, y: 0, z: 0, kind: 'endpoint' }), RAY)
    tool.onPointerDown(makeSnap({ x: 2, y: 0, z: 0, kind: 'endpoint' }), RAY) // commit, one end not on geometry

    expect(tool.capturesKey('5')).toBe(false)
    pressKey(tool, '5')
    expect((tool as unknown as { typed: string }).typed).toBe('')
  })

  it('a bare letter cannot OPEN the buffer, but is accepted once a digit has opened it', () => {
    const { scene } = makeWasmScene()
    const { tool } = measureEligible(scene)

    expect(tool.capturesKey('m')).toBe(false)
    pressKey(tool, '5')
    expect(tool.capturesKey('m')).toBe(true)
  })

  it('idle "5" then Enter after a real 2m measurement arms the rescale confirmation from cold idle', () => {
    const { scene } = makeWasmScene()
    const { tool, onRescaleArmed } = measureEligible(scene)

    pressKey(tool, '5')
    pressKey(tool, 'Enter')

    expect(onRescaleArmed).toHaveBeenCalledWith({
      currentDistance: 2,
      typedDistance: 5,
      factor: 2.5,
      anchor: [0, 0, 0],
    })
    const stage = (tool as unknown as { stage: { kind: string; fromRecall?: boolean } }).stage
    expect(stage.kind).toBe('pendingRescale')
    expect(stage.fromRecall).toBe(true)
  })

  it('cancelRescale on a recalled arm creates nothing, restores the frozen reading, and keeps the recall usable', () => {
    const { scene, guideLines, guidePoints } = makeWasmScene()
    const { tool, onGuideCreated, onMeasurement, onRescaleArmed } = measureEligible(scene)

    pressKey(tool, '5')
    pressKey(tool, 'Enter')
    tool.cancelRescale()

    expect(guideLines.length).toBe(0)
    expect(guidePoints.length).toBe(0)
    expect(onGuideCreated).not.toHaveBeenCalled()
    const last = onMeasurement.mock.calls[onMeasurement.mock.calls.length - 1]
    expect(last).toEqual([formatLength(2), true])

    // The recall survives — typing and confirming again re-arms it.
    pressKey(tool, '5')
    pressKey(tool, 'Enter')
    expect(onRescaleArmed).toHaveBeenCalledTimes(2)
  })

  it('confirmRescale on a recalled arm applies the factor and clears the recall', () => {
    const { scene } = makeWasmScene()
    const { tool } = measureEligible(scene)

    pressKey(tool, '5')
    pressKey(tool, 'Enter')
    tool.confirmRescale()

    expect(scene.rescale_document).toHaveBeenCalledWith(2.5)
    expect(tool.capturesKey('5')).toBe(false)
  })

  it('forgetRecall clears the recall and blanks the readout', () => {
    const { scene } = makeWasmScene()
    const { tool, onMeasurement } = measureEligible(scene)

    tool.forgetRecall()

    expect(tool.capturesKey('5')).toBe(false)
    const last = onMeasurement.mock.calls[onMeasurement.mock.calls.length - 1]
    expect(last).toEqual(['', false])
  })

  it('starting a new gesture clears the old recall — aborting the new one does not resurrect it', () => {
    const { scene } = makeWasmScene()
    const { tool } = measureEligible(scene)

    tool.onPointerDown(makeSnap({ x: 5, y: 5, z: 0, kind: 'ground' }), RAY) // a fresh first click
    pressKey(tool, 'Escape') // abort the new gesture

    expect(tool.capturesKey('5')).toBe(false)
  })

  it('idle Escape with an open typed buffer clears the buffer but keeps the recall; a second Escape proceeds normally', () => {
    const { scene } = makeWasmScene()
    const { tool, onMeasurement } = measureEligible(scene)

    pressKey(tool, '5')
    pressKey(tool, 'Escape')

    const afterFirstEscape = onMeasurement.mock.calls[onMeasurement.mock.calls.length - 1]
    expect(afterFirstEscape).toEqual([formatLength(2), true])
    expect(tool.capturesKey('5')).toBe(true) // recall still there

    pressKey(tool, 'Escape') // buffer now empty — ordinary idle Escape (no plane lock set)
    const afterSecondEscape = onMeasurement.mock.calls[onMeasurement.mock.calls.length - 1]
    expect(afterSecondEscape).toEqual(['', false])
    expect(tool.capturesKey('5')).toBe(false) // the plain cancel() dropped the recall too
  })

  it('capturesKey is true for every non-idle stage regardless of key (regression guard)', () => {
    const { scene } = makeWasmScene()
    const { tool } = makeTool(scene)

    tool.onPointerDown(
      makeSnap({ x: 1, y: 0, z: 0, kind: 'on-edge', elementKind: 'edge', object: 7n, element: 3n }),
      RAY,
    )
    expect(tool.capturesKey('m')).toBe(true) // parallel
    tool.cancel()

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint' }), RAY)
    expect(tool.capturesKey('m')).toBe(true) // measure
    tool.onPointerMove(makeSnap({ x: 2, y: 0, z: 0, kind: 'endpoint' }), RAY)
    typeAndEnter(tool, '3') // live-arm, both ends on geometry

    expect(tool.capturesKey('m')).toBe(true) // pendingRescale
  })

  it('the existing live-arm path still sets fromRecall: false, and cancelRescale still drops the fallback guide point (regression guard)', () => {
    const { scene, guidePoints } = makeWasmScene()
    const { tool, onGuideCreated } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint' }), RAY)
    tool.onPointerMove(makeSnap({ x: 2, y: 0, z: 0, kind: 'endpoint' }), RAY)
    typeAndEnter(tool, '3')

    const stage = (tool as unknown as { stage: { kind: string; fromRecall?: boolean } }).stage
    expect(stage.kind).toBe('pendingRescale')
    expect(stage.fromRecall).toBe(false)

    tool.cancelRescale()

    expect(guidePoints.length).toBe(1)
    expect(onGuideCreated).toHaveBeenCalledTimes(1)
  })

  // Fix 1: forgetRecall() must not act while a rescale confirmation is
  // already armed — an Undo/Redo/explicit-delete keystroke doesn't route
  // through the dialog's own Escape handling, so it can reach the tool while
  // "resize the model?" is still showing. Without the guard, this would blank
  // the widget out from under the still-open dialog, and (fromRecall
  // specifically) leave Cancel unable to restore the frozen original reading
  // since `_recall` was already cleared.
  it('forgetRecall is a no-op while a RECALLED rescale confirmation is armed (Fix 1) — cancelRescale afterward still restores the frozen original reading', () => {
    const { scene } = makeWasmScene()
    const { tool, onMeasurement } = measureEligible(scene)

    pressKey(tool, '5')
    pressKey(tool, 'Enter') // arms pendingRescale, fromRecall: true
    const callsBefore = onMeasurement.mock.calls.length

    tool.forgetRecall()

    const stage = (tool as unknown as { stage: { kind: string; fromRecall?: boolean } }).stage
    expect(stage.kind).toBe('pendingRescale') // untouched by the stray call
    expect(stage.fromRecall).toBe(true)
    expect(onMeasurement.mock.calls.length).toBe(callsBefore) // no readout change
    expect(tool.capturesKey('m')).toBe(true) // still fully armed

    // Without the fix, `_recall` would already be null here, and Cancel's
    // fromRecall branch would read `null` and skip restoring the reading —
    // leaving the widget permanently blank instead of showing 2 m again.
    tool.cancelRescale()
    const last = onMeasurement.mock.calls[onMeasurement.mock.calls.length - 1]
    expect(last).toEqual([formatLength(2), true])
  })

  it('forgetRecall is a no-op while a LIVE-gesture rescale confirmation is armed (Fix 1)', () => {
    const { scene, guidePoints } = makeWasmScene()
    const { tool, onGuideCreated } = makeTool(scene)

    tool.onPointerDown(makeSnap({ x: 0, y: 0, z: 0, kind: 'endpoint' }), RAY)
    tool.onPointerMove(makeSnap({ x: 2, y: 0, z: 0, kind: 'endpoint' }), RAY)
    typeAndEnter(tool, '3') // live arm, fromRecall: false

    tool.forgetRecall()

    const stage = (tool as unknown as { stage: { kind: string; fromRecall?: boolean } }).stage
    expect(stage.kind).toBe('pendingRescale') // untouched
    expect(stage.fromRecall).toBe(false)

    // The fallback commit (createGuidesAtArm etc.) still works unaffected.
    tool.cancelRescale()
    expect(guidePoints.length).toBe(1)
    expect(onGuideCreated).toHaveBeenCalledTimes(1)
  })

  // Fix 2: idle arrow keys must not silently interleave with an open
  // retroactive-rescale typed buffer — the buffer owns Enter/digit/unit-
  // suffix/Backspace exclusively while it's open, and an arrow key is a
  // harmless no-op instead of toggling `idlePlaneLock` underneath it.
  it('an arrow key while a typed buffer is open is a harmless no-op (Fix 2) — buffer and idlePlaneLock both unchanged', () => {
    const { scene } = makeWasmScene()
    const { tool } = measureEligible(scene)

    pressKey(tool, '5') // opens the buffer
    pressKey(tool, 'ArrowRight')

    expect((tool as unknown as { typed: string }).typed).toBe('5')
    expect((tool as unknown as { idlePlaneLock: 0 | 1 | 2 | null }).idlePlaneLock).toBeNull()
  })

  it('regression: an arrow key at idle with NO typed buffer open still locks idlePlaneLock normally (Fix 2)', () => {
    const { scene } = makeWasmScene()
    const { tool } = measureEligible(scene) // recall present, but buffer NOT open

    pressKey(tool, 'ArrowRight')

    expect((tool as unknown as { idlePlaneLock: 0 | 1 | 2 | null }).idlePlaneLock).toBe(0)
    expect((tool as unknown as { typed: string }).typed).toBe('')
  })
})

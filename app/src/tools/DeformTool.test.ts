/**
 * DeformTool — object and face picking, the three drags (twist angle, bend
 * sideways, taper radius), kernel limits, typed amounts, retype, and Bend to
 * Path.
 */
import { describe, it, expect, vi } from 'vitest'
import { i18n } from '@lingui/core'
import * as THREE from 'three'
import { DeformTool, DEFORM_BUSY_REFUSED, DEFORM_PREVIEW_SKIPPED_HINT, limitInRefusal, sidedAmount, typedEcho } from './DeformTool'
import { nextPaint } from '../paint'
import type { Scene as WasmScene } from '../wasm/loader'
import type { Ray } from '../viewport/math'
import type { NodeRef } from '../panels/treeModel'
import type { Snap } from './types'
import type { DeformMode } from './deformMode'

const OBJECT = 5n
const selection: NodeRef[] = [{ kind: 'object', id: OBJECT }]

const meshStub = () => ({
  positions: () => new Float32Array([0, 0, 0, 1, 0, 0, 0, 1, 0]),
  normals: () => new Float32Array([0, 0, 1, 0, 0, 1, 0, 0, 1]),
  indices: () => new Uint32Array([0, 1, 2]),
  edge_positions: () => new Float32Array([0, 0, 0, 1, 0, 0]),
  free: vi.fn(),
})

/** A 0.1 x 0.1 x 1 bar standing on the ground, as the renderer names it. */
function objectsGroup(): THREE.Group {
  const root = new THREE.Group()
  const g = new THREE.Group()
  g.name = `Object_${OBJECT}`
  const mesh = new THREE.Mesh(new THREE.BoxGeometry(0.1, 0.1, 1), new THREE.MeshBasicMaterial())
  mesh.position.set(0, 0, 0.5)
  g.add(mesh)
  root.add(g)
  root.updateMatrixWorld(true)
  return root
}

function key(k: string): KeyboardEvent {
  return { key: k, ctrlKey: false, metaKey: false } as KeyboardEvent
}
function typeText(tool: DeformTool, text: string): void {
  for (const c of text) tool.onKey(key(c))
}
const down = (x: number, y: number): Ray => ({ origin: [x, y, 5], direction: [0, 0, -1] })
const lastJson = (fn: ReturnType<typeof vi.fn>) => JSON.parse(fn.mock.calls.at(-1)![1] as string)

function setup(mode: DeformMode, over: Record<string, unknown> = {}, sel: NodeRef[] = selection) {
  let gen = 1n
  const scene = {
    deform_preview: vi.fn(() => meshStub()),
    deform: vi.fn(() => { gen += 1n; return { free: vi.fn() } }),
    deform_limit: vi.fn(() => 1),
    deform_preview_limit: vi.fn(() => 1),
    history_generation: vi.fn(() => gen),
    scene_undo: vi.fn(() => { gen += 1n; return { free: vi.fn() } }),
    scene_redo: vi.fn(() => { gen += 1n; return { free: vi.fn() } }),
    pick_face: vi.fn(),
    pick_sketch_edge: vi.fn(),
    sketch_edge_endpoints: vi.fn(),
    ...over,
  }
  const preview = new THREE.Group()
  const onCommit = vi.fn()
  const onToast = vi.fn()
  const onMeasurement = vi.fn()
  const tool = new DeformTool(
    scene as unknown as WasmScene, preview, objectsGroup(), sel, onCommit, onToast, onMeasurement, mode,
  )
  return { scene, tool, preview, onCommit, onToast, onMeasurement }
}

/** Click the top face of the bar at its center-ish (off the axis for angles). */
function grabTop(t: ReturnType<typeof setup>, x = 0.04, y = 0) {
  t.tool.onPointerMove(null, down(x, y))
  t.tool.onPointerDown(null, down(x, y))
}

describe('DeformTool — picking', () => {
  it('shows the box gizmo for a selected solid and hints at the first click', () => {
    const t = setup('twist')
    expect(t.preview.children.length).toBeGreaterThan(0)
    expect(t.tool.statusHint()).toMatch(/Click the end of the box to twist/)
  })

  it('with nothing selected, a click acquires the solid and shows its box', () => {
    const t = setup('twist', {}, [])
    expect(t.tool.statusHint()).toBe('Click a solid to twist.')
    t.tool.setSelectionAcquirer(() => selection)
    t.tool.onPointerDown(null, down(0, 0))
    expect(t.tool.statusHint()).toMatch(/Click the end of the box/)
  })

  it('a click on a group is refused in plain words', () => {
    const t = setup('twist', {}, [])
    t.tool.setSelectionAcquirer(() => [{ kind: 'group', id: 3n }])
    t.tool.onPointerDown(null, down(0, 0))
    expect(t.onToast.mock.calls[0][0]).toMatch(/one solid at a time/)
  })

  it('hovering a box face outlines it', () => {
    const t = setup('twist')
    const before = t.preview.children.length
    t.tool.onPointerMove(null, down(0, 0))
    expect(t.preview.children.length).toBe(before + 1)
    t.tool.onPointerMove(null, down(3, 3)) // off the box
    expect(t.preview.children.length).toBe(before)
  })
})

describe('DeformTool — Twist', () => {
  it('follows the angle swept around the axis and commits the frame from the box', () => {
    const t = setup('twist')
    grabTop(t, 0.04, 0)
    expect(t.tool.statusHint()).toMatch(/Move to twist/)
    // Sweep a quarter turn counterclockwise about +Z.
    t.tool.onPointerMove(null, down(0.0283, 0.0283))
    t.tool.onPointerMove(null, down(0, 0.04))
    expect(lastJson(t.scene.deform_preview).mode).toBe('twist')
    expect(lastJson(t.scene.deform_preview).angle).toBeCloseTo(Math.PI / 2, 2)
    t.tool.onPointerDown(null, down(0, 0.04)) // click to finish
    expect(t.scene.deform).toHaveBeenCalledTimes(1)
    const spec = lastJson(t.scene.deform)
    expect(spec.frame.origin).toEqual([0, 0, 0])
    expect(spec.frame.axis).toEqual([0, 0, 1])
    expect(spec.frame.length).toBeCloseTo(1, 6)
    expect(spec.angle).toBeCloseTo(Math.PI / 2, 2)
    expect(t.onCommit).toHaveBeenCalledWith(OBJECT)
  })

  it('typed 25, 25° and 25deg commit that angle; 8s sets stations first', () => {
    for (const text of ['25', '25°', '25deg']) {
      const t = setup('twist')
      grabTop(t)
      typeText(t.tool, text)
      t.tool.onKey(key('Enter'))
      expect(lastJson(t.scene.deform).angle).toBeCloseTo((25 * Math.PI) / 180, 12)
    }
    const t = setup('twist')
    grabTop(t)
    typeText(t.tool, '8s')
    t.tool.onKey(key('Enter'))
    expect(t.scene.deform).not.toHaveBeenCalled()
    typeText(t.tool, '90')
    t.tool.onKey(key('Enter'))
    expect(lastJson(t.scene.deform).stations).toBe(8)
  })

  it('a typed amount the shape cannot take is refused and names the limit', () => {
    const deform = vi.fn((_o: bigint, spec: string) => {
      if (Math.abs(JSON.parse(spec).angle) > 1) throw new Error('SelfIntersecting: pass through itself')
      return { free: vi.fn() }
    })
    const t = setup('bend', { deform: deform, deform_limit: vi.fn(() => 1) })
    grabTop(t)
    typeText(t.tool, '80')
    t.tool.onKey(key('Enter'))
    expect(t.onCommit).not.toHaveBeenCalled()
    expect(t.onToast).toHaveBeenLastCalledWith(
      'The result would pass through itself. Angle limited to 57.3°.',
      'SelfIntersecting',
    )
    // The limit was asked for the amount typed, not the identity.
    const asked = (t.scene.deform_limit as unknown as ReturnType<typeof vi.fn>).mock.calls[0][1] as string
    expect(JSON.parse(asked).angle).toBeCloseTo((80 * Math.PI) / 180, 9)
  })

  it('an out-of-range typed twist or bend toasts the range instead of being ignored', () => {
    const twist = setup('twist')
    grabTop(twist)
    typeText(twist.tool, '900')
    twist.tool.onKey(key('Enter'))
    expect(twist.scene.deform).not.toHaveBeenCalled()
    expect(twist.onToast).toHaveBeenLastCalledWith('Twist takes up to 720°.')

    const bend = setup('bend')
    grabTop(bend)
    typeText(bend.tool, '400')
    bend.tool.onKey(key('Enter'))
    expect(bend.scene.deform).not.toHaveBeenCalled()
    expect(bend.onToast).toHaveBeenLastCalledWith('Bend takes up to 360°.')

    // The edge of the range is still accepted.
    const edge = setup('twist')
    grabTop(edge)
    typeText(edge.tool, '720')
    edge.tool.onKey(key('Enter'))
    expect(edge.scene.deform).toHaveBeenCalledTimes(1)
  })

  it('a typed amount within 1% of the limit commits at exactly the limit', () => {
    const deform = vi.fn((_o: bigint, spec: string) => {
      if (Math.abs(JSON.parse(spec).angle) > 1) throw new Error('SelfIntersecting: pass through itself')
      return { free: vi.fn() }
    })
    // 57.4 degrees is 1.0019 rad: 0.19% over the 1 rad limit.
    const t = setup('bend', { deform, deform_limit: vi.fn(() => 1) })
    grabTop(t)
    typeText(t.tool, '57.4')
    t.tool.onKey(key('Enter'))
    expect(t.onToast).not.toHaveBeenCalled()
    expect(t.onCommit).toHaveBeenCalledWith(OBJECT)
    expect(lastJson(t.scene.deform).angle).toBe(1)
  })

  it('a typed amount more than 1% past the limit is still refused', () => {
    const deform = vi.fn((_o: bigint, spec: string) => {
      if (Math.abs(JSON.parse(spec).angle) > 1) throw new Error('SelfIntersecting: pass through itself')
      return { free: vi.fn() }
    })
    // 58.5 degrees is 1.021 rad: 2.1% over.
    const t = setup('bend', { deform, deform_limit: vi.fn(() => 1) })
    grabTop(t)
    typeText(t.tool, '58.5')
    t.tool.onKey(key('Enter'))
    expect(t.onCommit).not.toHaveBeenCalled()
    expect(t.onToast).toHaveBeenLastCalledWith(
      'The result would pass through itself. Angle limited to 57.3°.',
      'SelfIntersecting',
    )
  })

  it('clamps to the kernel limit once per drag and says so', () => {
    const preview = vi.fn((_o: bigint, spec: string) => {
      if (Math.abs(JSON.parse(spec).angle) > 1) throw new Error('SelfIntersecting: pass through itself')
      return meshStub()
    })
    const t = setup('twist', { deform_preview: preview, deform_preview_limit: vi.fn(() => 1) })
    grabTop(t, 0.04, 0)
    t.tool.onPointerMove(null, down(0, 0.04)) // 90 degrees: 1.57 rad > limit 1
    t.tool.onPointerMove(null, down(-0.04, 0)) // 180 degrees
    t.tool.onPointerMove(null, down(0, -0.04)) // 270 degrees
    // The drag asks the preview's capped search, once; never the commit's.
    expect(t.scene.deform_preview_limit).toHaveBeenCalledTimes(1)
    expect(t.scene.deform_limit).not.toHaveBeenCalled()
    expect(t.tool.statusHint()).toMatch(/The result would pass through itself\. Angle limited to 57\.3°\./)
    // The ghost is the clamped one.
    const calls = (preview as unknown as ReturnType<typeof vi.fn>).mock.calls
    const accepted = calls.map((c) => JSON.parse(c[1] as string).angle as number).filter((a) => a <= 1)
    expect(accepted.at(-1)).toBeLessThan(1)
    expect(accepted.at(-1)).toBeCloseTo(1, 5)
    t.tool.onPointerDown(null, down(0, -0.04))
    expect(lastJson(t.scene.deform).angle).toBeLessThan(1)
  })

  it('Esc backs up to the face stage without committing, then cancels', () => {
    const t = setup('twist')
    grabTop(t)
    t.tool.onKey(key('Escape'))
    expect(t.tool.statusHint()).toMatch(/Click the end of the box/)
    expect(t.scene.deform).not.toHaveBeenCalled()
    t.tool.onKey(key('Escape'))
    expect(t.tool.statusHint()).toMatch(/Click a solid to twist/)
  })

  it('a refusal that is not "too far" explains itself and backs up', () => {
    const t = setup('twist', { deform_preview: vi.fn(() => { throw new Error('ShapeNeedsSolid: gaps') }) })
    grabTop(t, 0.04, 0)
    t.tool.onPointerMove(null, down(0, 0.04))
    expect(t.onToast).toHaveBeenCalledTimes(1)
    expect(t.onToast.mock.calls[0][1]).toBe('ShapeNeedsSolid')
    expect(t.tool.statusHint()).toMatch(/Click the end of the box/)
  })
})

describe('DeformTool — Taper', () => {
  it('the ratio is the pointer radius over the grab radius', () => {
    const t = setup('taper')
    grabTop(t, 0.04, 0)
    t.tool.onPointerMove(null, down(0.02, 0))
    expect(lastJson(t.scene.deform_preview).mode).toBe('taper')
    expect(lastJson(t.scene.deform_preview).ratio).toBeCloseTo(0.5, 6)
    t.tool.onPointerDown(null, down(0.02, 0))
    expect(lastJson(t.scene.deform).ratio).toBeCloseTo(0.5, 6)
  })

  it('typed 60% and a bare 60 both mean a ratio of 0.6', () => {
    for (const text of ['60%', '60']) {
      const t = setup('taper')
      grabTop(t)
      typeText(t.tool, text)
      t.tool.onKey(key('Enter'))
      expect(lastJson(t.scene.deform).ratio).toBeCloseTo(0.6, 12)
    }
  })

  it('a percentage typed into a twist, or an angle into a taper, names what the mode takes', () => {
    const twist = setup('twist')
    grabTop(twist)
    typeText(twist.tool, '60%')
    twist.tool.onKey(key('Enter'))
    expect(twist.onToast).toHaveBeenCalledWith('Only a taper takes a percentage. Type an angle like 25°.')
    expect(twist.scene.deform).not.toHaveBeenCalled()
    const taper = setup('taper')
    grabTop(taper)
    typeText(taper.tool, '25°')
    taper.tool.onKey(key('Enter'))
    expect(taper.onToast).toHaveBeenCalledWith('Taper takes a percentage, like 60%.')
    expect(taper.scene.deform).not.toHaveBeenCalled()
  })

  it('names the limit as a percentage', () => {
    const preview = vi.fn((_o: bigint, spec: string) => {
      if (JSON.parse(spec).ratio < 0.3) throw new Error('SelfIntersecting: x')
      return meshStub()
    })
    const t = setup('taper', { deform_preview: preview, deform_preview_limit: vi.fn(() => 0.3) })
    grabTop(t, 0.04, 0)
    t.tool.onPointerMove(null, down(0.004, 0))
    expect(t.tool.statusHint()).toMatch(/Taper limited to 30%\./)
  })
})

describe('DeformTool — Bend', () => {
  it('picks the cross axis the drag follows and bends by twice the drag over the length', () => {
    const t = setup('bend')
    // The camera looks down -Z, so the view plane is z = 1 (the grab height).
    grabTop(t, 0, 0)
    t.tool.onPointerMove(null, down(0.2, 0.01)) // mostly along +X
    const spec = lastJson(t.scene.deform_preview)
    expect(spec.mode).toBe('bend')
    expect(spec.frame.u).toEqual([1, 0, 0])
    expect(spec.angle).toBeCloseTo(0.4, 6) // 2 * 0.2 / 1
    t.tool.onPointerMove(null, down(-0.1, 0.01))
    expect(lastJson(t.scene.deform_preview).angle).toBeCloseTo(-0.2, 6)
    t.tool.onPointerDown(null, down(-0.1, 0.01))
    expect(lastJson(t.scene.deform).angle).toBeCloseTo(-0.2, 6)
  })

  it('bends toward Y when the drag goes along Y', () => {
    const t = setup('bend')
    grabTop(t, 0, 0)
    t.tool.onPointerMove(null, down(0.01, 0.3))
    expect(lastJson(t.scene.deform_preview).frame.u).toEqual([0, 1, 0])
  })

  it('shows the angle in the measurements box', () => {
    const t = setup('bend')
    grabTop(t, 0, 0)
    t.tool.onPointerMove(null, down(0.25, 0))
    expect(t.onMeasurement).toHaveBeenLastCalledWith('28.6°')
  })
})

describe('DeformTool — Bend after Twist at the limit (F-58)', () => {
  /** A twisted blade's bend: the shape takes 1.83° toward -u and 1.2°
   *  toward +u (the twist leans it), as `deform` and the previews say. */
  function twistedBlade(over: Record<string, unknown> = {}) {
    const limitDeg = (angle: number) => (angle < 0 ? -1.83 : 1.2)
    const fits = (spec: string) => {
      const a = JSON.parse(spec).angle as number
      return Math.abs(a) <= Math.abs((limitDeg(a) * Math.PI) / 180) + 1e-12
    }
    const refusal = (spec: string) =>
      new Error(`SelfIntersecting: the result would pass through itself (limit ${(limitDeg(JSON.parse(spec).angle) * Math.PI) / 180})`)
    return setup('bend', {
      deform_preview: vi.fn((_o: bigint, spec: string) => {
        if (!fits(spec)) throw new Error('SelfIntersecting: the result would pass through itself')
        return meshStub()
      }),
      deform: vi.fn((_o: bigint, spec: string) => {
        if (!fits(spec)) throw refusal(spec)
        return { free: vi.fn() }
      }),
      deform_preview_limit: vi.fn((_o: bigint, spec: string) => (limitDeg(JSON.parse(spec).angle) * Math.PI) / 180),
      deform_limit: vi.fn((_o: bigint, spec: string) => (limitDeg(JSON.parse(spec).angle) * Math.PI) / 180),
      ...over,
    })
  }

  it('a typed amount bends to the side the drag is on, so the named limit commits', () => {
    const t = twistedBlade()
    grabTop(t, 0, 0)
    // Drag toward -X past the limit: the ghost stops at 1.8° and says so.
    t.tool.onPointerMove(null, down(-0.2, 0.01))
    expect(t.tool.statusHint()).toBe('The result would pass through itself. Angle limited to 1.8°.')
    expect(t.onMeasurement).toHaveBeenLastCalledWith('1.8°')
    // Typing the named limit commits it on the dragged side: before, the
    // typed 1.8 bent the other way, past that side's 1.2°, and nothing
    // committed.
    typeText(t.tool, '1.8')
    t.tool.onKey(key('Enter'))
    expect(t.onCommit).toHaveBeenCalledWith(OBJECT)
    const spec = lastJson(t.scene.deform)
    expect(spec.frame.u).toEqual([1, 0, 0])
    expect(spec.angle).toBeCloseTo((-1.8 * Math.PI) / 180, 12)
  })

  it('a click at the limit commits the clamped amount', () => {
    const t = twistedBlade()
    grabTop(t, 0, 0)
    t.tool.onPointerMove(null, down(-0.2, 0.01))
    t.tool.onPointerDown(null, down(-0.3, 0.01))
    expect(t.onCommit).toHaveBeenCalledWith(OBJECT)
    const a = lastJson(t.scene.deform).angle as number
    expect(a).toBeLessThan(0)
    expect(Math.abs(a)).toBeCloseTo((1.83 * Math.PI) / 180, 6)
  })

  it('an explicit minus flips the side; a typed amount past it names that side\'s limit', () => {
    const t = twistedBlade()
    grabTop(t, 0, 0)
    t.tool.onPointerMove(null, down(-0.05, 0.001))
    typeText(t.tool, '-5')
    t.tool.onKey(key('Enter'))
    // -5 on the -X side is +5°: past the +u side's 1.2°, refused with it,
    // using the limit the commit's refusal carried (no second search).
    expect(t.onCommit).not.toHaveBeenCalled()
    expect(t.onToast).toHaveBeenLastCalledWith(
      'The result would pass through itself. Angle limited to 1.2°.',
      'SelfIntersecting',
    )
    expect(t.scene.deform_limit).not.toHaveBeenCalled()
  })

  it('a retype redoes the bend on the side it was made', () => {
    const t = twistedBlade()
    grabTop(t, 0, 0)
    t.tool.onPointerMove(null, down(-0.04, 0.001))
    typeText(t.tool, '1.5')
    t.tool.onKey(key('Enter'))
    expect(lastJson(t.scene.deform).angle).toBeCloseTo((-1.5 * Math.PI) / 180, 12)
    typeText(t.tool, '1.7')
    t.tool.onKey(key('Enter'))
    expect(lastJson(t.scene.deform).angle).toBeCloseTo((-1.7 * Math.PI) / 180, 12)
  })

  it('the typed echo shows what the commit will do', () => {
    // Review round 9: on the −u side a typed 5 commits −5° but echoed 5°,
    // and a typed −5 commits +5° (the other way) but echoed −5°.
    const t = twistedBlade()
    grabTop(t, 0, 0)
    t.tool.onPointerMove(null, down(-0.05, 0.001))
    typeText(t.tool, '1.5')
    expect(t.onMeasurement).toHaveBeenLastCalledWith('1.5°')
    t.tool.onKey(key('Escape'))
    grabTop(t, 0, 0)
    t.tool.onPointerMove(null, down(-0.05, 0.001))
    typeText(t.tool, '-1')
    expect(t.onMeasurement).toHaveBeenLastCalledWith('1° the other way')
    expect(typedEcho('bend', '5', 0.1)).toBe('5°')
    expect(typedEcho('bend', '-5', 0.1)).toBe('5° the other way')
    expect(typedEcho('bend', '-5', 0)).toBe('-5°')
    expect(typedEcho('twist', '-5', -0.1)).toBe('-5°')
  })

  it('the typed echo follows the side the pointer moves to, as Enter commits', () => {
    const t = twistedBlade()
    grabTop(t, 0, 0)
    t.tool.onPointerMove(null, down(-0.05, 0.001))
    typeText(t.tool, '-1')
    expect(t.onMeasurement).toHaveBeenLastCalledWith('1° the other way')
    // Over to the +u side: the same −1 now bends toward −u, the drag's
    // "other way".
    t.tool.onPointerMove(null, down(0.05, 0.001))
    expect(t.onMeasurement).toHaveBeenLastCalledWith('1° the other way')
    t.tool.onPointerMove(null, down(-0.05, 0.001))
    typeText(t.tool, '')
    t.tool.onKey(key('Backspace'))
    t.tool.onKey(key('Backspace'))
    typeText(t.tool, '1')
    t.tool.onPointerMove(null, down(0.05, 0.001))
    expect(t.onMeasurement).toHaveBeenLastCalledWith('1°')
    t.tool.onKey(key('Enter'))
    expect(lastJson(t.scene.deform).angle).toBeCloseTo(Math.PI / 180, 12)
  })

  it('sidedAmount and limitInRefusal', () => {
    expect(sidedAmount('bend', 0.1, -0.02)).toBe(-0.1)
    expect(sidedAmount('bend', -0.1, -0.02)).toBe(0.1)
    expect(sidedAmount('bend', 0.1, 0)).toBe(0.1)
    expect(sidedAmount('twist', 0.1, -0.02)).toBe(0.1)
    expect(limitInRefusal(new Error('SelfIntersecting: the result would pass through itself (limit -0.031)'))).toBe(-0.031)
    expect(limitInRefusal(new Error('SelfIntersecting: the result would pass through itself'))).toBeNull()
  })
})

const DRAG_TWIST = 'Move to twist, click to finish, or type an angle (25°). Esc backs up.'

describe('DeformTool — an object too large to preview', () => {
  const tooLarge = () => { throw new Error('PreviewTooLarge: the object is too large to preview') }

  it('says the preview was skipped, stops asking, and the click still commits behind the busy card', () => {
    const t = setup('twist', { deform_preview: vi.fn(tooLarge) })
    const busy = vi.fn((_m: string, work: () => void) => work())
    t.tool.setBusyRunner(busy)
    grabTop(t, 0.04, 0)
    t.tool.onPointerMove(null, down(0, 0.04))
    expect(t.tool.statusHint()).toBe(`${DEFORM_PREVIEW_SKIPPED_HINT} ${DRAG_TWIST}`)
    expect(t.preview.getObjectByName('DeformGizmo')).toBeUndefined()
    const asked = (t.scene.deform_preview as unknown as ReturnType<typeof vi.fn>).mock.calls.length
    t.tool.onPointerMove(null, down(-0.04, 0))
    t.tool.onPointerMove(null, down(0, -0.04))
    // No preview, no limit search per move; the amount still follows.
    expect((t.scene.deform_preview as unknown as ReturnType<typeof vi.fn>).mock.calls.length).toBe(asked)
    expect(t.scene.deform_preview_limit).not.toHaveBeenCalled()
    expect(t.onMeasurement).toHaveBeenLastCalledWith('270°')
    expect(t.onToast).not.toHaveBeenCalled()
    t.tool.onPointerDown(null, down(0, -0.04))
    expect(busy).toHaveBeenCalledWith('Twisting…', expect.any(Function))
    expect(t.onCommit).toHaveBeenCalledWith(OBJECT)
    expect(lastJson(t.scene.deform).angle).toBeCloseTo((270 * Math.PI) / 180, 6)
  })

  it('a limit search that gives up for size skips the preview the same way', () => {
    const t = setup('bend', {
      deform_preview: vi.fn((_o: bigint, spec: string) => {
        if (Math.abs(JSON.parse(spec).angle) > 0.1) throw new Error('SelfIntersecting: the result would pass through itself')
        return meshStub()
      }),
      deform_preview_limit: vi.fn(tooLarge),
    })
    grabTop(t, 0, 0)
    t.tool.onPointerMove(null, down(0.2, 0.01))
    expect(t.tool.statusHint()).toBe(
      `${DEFORM_PREVIEW_SKIPPED_HINT} Move sideways to bend, click to finish, or type an angle (25°). Esc backs up.`,
    )
    expect(t.onToast).not.toHaveBeenCalled()
  })

  it('a typed Enter right after the face pick commits behind the busy card too', () => {
    // Review round 9: nothing had previewed, so nothing knew the object
    // was heavy, and the commit froze the app without a word.
    const t = setup('twist')
    const busy = vi.fn((_m: string, work: () => void) => work())
    t.tool.setBusyRunner(busy)
    grabTop(t)
    typeText(t.tool, '30')
    t.tool.onKey(key('Enter'))
    expect(busy).toHaveBeenCalledWith('Twisting…', expect.any(Function))
    expect(t.onCommit).toHaveBeenCalledWith(OBJECT)
    // A retype goes behind it as well.
    typeText(t.tool, '40')
    t.tool.onKey(key('Enter'))
    expect(busy).toHaveBeenCalledTimes(2)
  })

  it('forgets the skip when the object changes: undo, redo, an edit or a commit', () => {
    let heavy = true
    const preview = vi.fn(() => {
      if (heavy) throw new Error('PreviewTooLarge: the object is too large to preview')
      return meshStub()
    })
    const t = setup('twist', { deform_preview: preview })
    grabTop(t, 0.04, 0)
    t.tool.onPointerMove(null, down(0, 0.04))
    expect(t.tool.statusHint()).toContain(DEFORM_PREVIEW_SKIPPED_HINT)
    // Undo made it light again.
    heavy = false
    t.tool.onHistoryChanged()
    t.tool.onPointerMove(null, down(-0.04, 0))
    expect(t.tool.statusHint()).not.toContain(DEFORM_PREVIEW_SKIPPED_HINT)
    expect(t.preview.children.length).toBeGreaterThan(0)
    // After a commit the next drag asks again.
    heavy = true
    t.tool.onPointerMove(null, down(0, -0.04))
    expect(t.tool.statusHint()).toContain(DEFORM_PREVIEW_SKIPPED_HINT)
    t.tool.onPointerDown(null, down(0, -0.04))
    heavy = false
    const asked = preview.mock.calls.length
    grabTop(t, 0.04, 0)
    t.tool.onPointerMove(null, down(0, 0.04))
    expect(preview.mock.calls.length).toBeGreaterThan(asked)
    expect(t.tool.statusHint()).not.toContain(DEFORM_PREVIEW_SKIPPED_HINT)
  })

  it('commits what was asked at the click, through the real paint deferral', async () => {
    // Review round 10: the host's card waits two animation frames
    // (nextPaint) before the work runs; pointer moves in those frames move
    // the ghost but must not change what commits. Node has no frames, so
    // they are stood in for by timers; the deferral itself is the app's.
    vi.stubGlobal('requestAnimationFrame', (cb: FrameRequestCallback) => setTimeout(() => cb(0), 0))
    try {
      const t = setup('twist')
      t.tool.setBusyRunner((_m, work) => {
        void nextPaint().then(work)
        return true
      })
      grabTop(t, 0.04, 0)
      t.tool.onPointerMove(null, down(0, 0.04)) // 90°
      t.tool.onPointerDown(null, down(0, 0.04))
      expect(t.scene.deform).not.toHaveBeenCalled()
      t.tool.onPointerMove(null, down(-0.04, 0)) // 180°, while the card paints
      await nextPaint()
      await new Promise((r) => setTimeout(r, 0))
      expect(t.scene.deform).toHaveBeenCalledTimes(1)
      expect(lastJson(t.scene.deform).angle).toBeCloseTo(Math.PI / 2, 6)
      expect(t.onCommit).toHaveBeenCalledWith(OBJECT)
    } finally {
      vi.unstubAllGlobals()
    }
  })

  it('a commit the busy card cannot take is told, and keeps the typed amount for another Enter', () => {
    const t = setup('twist')
    let busyElsewhere = true
    t.tool.setBusyRunner((_m, work) => {
      if (busyElsewhere) return false
      work()
      return true
    })
    grabTop(t)
    typeText(t.tool, '30')
    t.tool.onKey(key('Enter'))
    expect(t.scene.deform).not.toHaveBeenCalled()
    expect(t.onToast).toHaveBeenLastCalledWith(i18n._(DEFORM_BUSY_REFUSED))
    expect(t.onMeasurement).toHaveBeenLastCalledWith('30°')
    busyElsewhere = false
    t.tool.onKey(key('Enter'))
    expect(lastJson(t.scene.deform).angle).toBeCloseTo(Math.PI / 6, 9)
  })

  it('a click the busy card cannot take keeps the typed buffer and the readout', () => {
    const t = setup('twist')
    let up = true
    t.tool.setBusyRunner((_m, work) => {
      if (up) return false
      work()
      return true
    })
    grabTop(t, 0.04, 0)
    t.tool.onPointerMove(null, down(0, 0.04))
    typeText(t.tool, '30')
    t.tool.onPointerDown(null, down(0, 0.04))
    expect(t.scene.deform).not.toHaveBeenCalled()
    expect(t.onToast).toHaveBeenLastCalledWith(i18n._(DEFORM_BUSY_REFUSED))
    expect(t.onMeasurement).toHaveBeenLastCalledWith('30°')
    up = false
    t.tool.onKey(key('Enter'))
    expect(lastJson(t.scene.deform).angle).toBeCloseTo(Math.PI / 6, 9)
  })

  it('forgets the skip when the document changes by any route', () => {
    // An edit from the API or another window moves the history without
    // telling the tool through onHistoryChanged.
    let heavy = true
    let gen = 1n
    const preview = vi.fn(() => {
      if (heavy) throw new Error('PreviewTooLarge: the object is too large to preview')
      return meshStub()
    })
    const t = setup('twist', { deform_preview: preview, history_generation: vi.fn(() => gen) })
    grabTop(t, 0.04, 0)
    t.tool.onPointerMove(null, down(0, 0.04))
    expect(t.tool.statusHint()).toContain(DEFORM_PREVIEW_SKIPPED_HINT)
    heavy = false
    gen += 1n
    t.tool.onPointerMove(null, down(-0.04, 0))
    expect(t.tool.statusHint()).not.toContain(DEFORM_PREVIEW_SKIPPED_HINT)
    expect(t.preview.children.length).toBeGreaterThan(0)
  })

  it('Bend to Path says so in the status and still applies', () => {
    const t = setup('bend-to-path', {
      pick_sketch_edge: vi.fn(() => ({ sketch: () => 9n, edge: () => 1n, free: vi.fn() })),
      sketch_edge_island: vi.fn(() => 5n),
      sketch_island_edges: vi.fn(() => new BigUint64Array([1n, 2n])),
      sketch_edge_endpoints: vi.fn((_s: bigint, e: bigint) =>
        e === 1n ? new Float64Array([0, 0, 0, 1, 0, 0]) : new Float64Array([1, 0, 0, 1, 1, 0])),
      deform_preview: vi.fn(tooLarge),
    })
    const busy = vi.fn((_m: string, work: () => void) => work())
    t.tool.setBusyRunner(busy)
    t.tool.onPointerDown({ x: 0, y: 0, z: 0, kind: 'endpoint' }, down(0, 0))
    t.tool.onPointerDown({ x: 0, y: 0, z: 1, kind: 'endpoint' }, down(0, 0))
    t.tool.onPointerDown(null, down(0.5, 0))
    expect(t.tool.statusHint()).toBe('This object is too large to preview. Click or press Enter to apply it anyway.')
    t.tool.onKey(key('Enter'))
    expect(busy).toHaveBeenCalledWith('Bending…', expect.any(Function))
    expect(t.onCommit).toHaveBeenCalledWith(OBJECT)
  })
})

describe('DeformTool — retype and modes', () => {
  it('a typed amount after the commit redoes the deform', () => {
    const t = setup('twist')
    grabTop(t)
    typeText(t.tool, '30')
    t.tool.onKey(key('Enter'))
    expect(t.tool.capturesKey('4')).toBe(true)
    expect(t.tool.capturesKey('m')).toBe(false)
    typeText(t.tool, '60')
    t.tool.onKey(key('Enter'))
    expect(t.scene.scene_undo).toHaveBeenCalledTimes(1)
    expect(lastJson(t.scene.deform).angle).toBeCloseTo((60 * Math.PI) / 180, 12)
    typeText(t.tool, '6s')
    t.tool.onKey(key('Enter'))
    expect(lastJson(t.scene.deform).stations).toBe(6)
    t.tool.disarmRetype()
    expect(t.tool.capturesKey('4')).toBe(false)
  })

  it('switching mode keeps the solid and restarts the gesture', () => {
    const t = setup('twist')
    grabTop(t)
    t.tool.setMode('taper')
    expect(t.tool.statusHint()).toMatch(/Click the end of the box to taper/)
    grabTop(t, 0.04, 0)
    t.tool.onPointerMove(null, down(0.02, 0))
    expect(lastJson(t.scene.deform_preview).mode).toBe('taper')
  })
})

describe('DeformTool — Bend to Path', () => {
  const edgeSnap = (a: number[]): Snap => ({ x: a[0], y: a[1], z: a[2], kind: 'on-edge', elementKind: 'sketch-edge', sketch: 8n, element: 7n })

  function pathScene() {
    return {
      pick_sketch_edge: vi.fn(() => ({ sketch: () => 9n, edge: () => 1n, free: vi.fn() })),
      sketch_edge_island: vi.fn(() => 5n),
      sketch_island_edges: vi.fn(() => new BigUint64Array([1n, 2n])),
      sketch_edge_endpoints: vi.fn((_s: bigint, e: bigint) =>
        e === 1n ? new Float64Array([0, 0, 0, 1, 0, 0]) : new Float64Array([1, 0, 0, 1, 1, 0])),
    }
  }

  it('walks base line, path, preview, commit', () => {
    const t = setup('bend-to-path', pathScene())
    expect(t.tool.statusHint()).toMatch(/Click an edge or two points/)
    t.tool.onPointerDown({ x: 0, y: 0, z: 0, kind: 'endpoint' }, down(0, 0))
    expect(t.tool.statusHint()).toMatch(/second point/)
    t.tool.onPointerDown({ x: 0, y: 0, z: 1, kind: 'endpoint' }, down(0, 0))
    expect(t.tool.statusHint()).toMatch(/Click the path/)
    t.tool.onPointerDown(null, down(0.5, 0))
    expect(t.tool.statusHint()).toMatch(/Click or press Enter/)
    const spec = lastJson(t.scene.deform_preview)
    expect(spec.mode).toBe('bend_to_path')
    expect(spec.base_from).toEqual([0, 0, 0])
    expect(spec.base_to).toEqual([0, 0, 1])
    expect(spec.path).toEqual([[0, 0, 0], [1, 0, 0], [1, 1, 0]])
    expect(spec.closed).toBe(false)
    t.tool.onKey(key('Enter'))
    expect(t.scene.deform).toHaveBeenCalledTimes(1)
    expect(t.onCommit).toHaveBeenCalledWith(OBJECT)
  })

  it('takes a base line from an edge, and runs the path from the end nearer the base start', () => {
    const t = setup('bend-to-path', { ...pathScene(), edge_endpoints: vi.fn() })
    // The base starts near the path's far end (1,1,0).
    ;(t.scene.sketch_edge_endpoints as ReturnType<typeof vi.fn>).mockImplementation((_s: bigint, e: bigint) => {
      if (e === 7n) return new Float64Array([1, 1, 0.01, 1, 1, 1])
      return e === 1n ? new Float64Array([0, 0, 0, 1, 0, 0]) : new Float64Array([1, 0, 0, 1, 1, 0])
    })
    t.tool.onPointerDown(edgeSnap([1, 1, 0.5]), down(0, 0))
    t.tool.onPointerDown(null, down(0.5, 0))
    const spec = lastJson(t.scene.deform_preview)
    expect(spec.path).toEqual([[1, 1, 0], [1, 0, 0], [0, 0, 0]])
  })

  it('a click that finds no path says what to aim at; Esc steps back', () => {
    const t = setup('bend-to-path', { ...pathScene(), pick_sketch_edge: vi.fn(() => undefined) })
    t.tool.onPointerDown({ x: 0, y: 0, z: 0, kind: 'endpoint' }, down(0, 0))
    t.tool.onPointerDown({ x: 0, y: 0, z: 1, kind: 'endpoint' }, down(0, 0))
    t.tool.onPointerDown(null, down(5, 5))
    expect(t.onToast).toHaveBeenCalledWith('Click a drawn line or curve for the path.')
    t.tool.onKey(key('Escape'))
    expect(t.tool.statusHint()).toMatch(/Click an edge or two points/)
  })

  it('a path that forks is refused naming the fork, on every click (playtest 4.1.4)', () => {
    // Three edges meet at (1, 0, 0): the path and a spur.
    const t = setup('bend-to-path', {
      ...pathScene(),
      sketch_island_edges: vi.fn(() => new BigUint64Array([1n, 2n, 3n])),
      sketch_edge_endpoints: vi.fn((_s: bigint, e: bigint) =>
        e === 1n
          ? new Float64Array([0, 0, 0, 1, 0, 0])
          : e === 2n
            ? new Float64Array([1, 0, 0, 1, 1, 0])
            : new Float64Array([1, 0, 0, 2, 0, 0])),
    })
    t.tool.onPointerDown({ x: 0, y: 0, z: 0, kind: 'endpoint' }, down(0, 0))
    t.tool.onPointerDown({ x: 0, y: 0, z: 1, kind: 'endpoint' }, down(0, 0))
    const fork = 'The path forks. Bend to Path follows one run of lines with no branches: erase the extra line, or draw the path on its own.'
    t.tool.onPointerDown(null, down(0.5, 0))
    t.tool.onPointerDown(null, down(0.5, 0))
    expect(t.onToast).toHaveBeenCalledTimes(2)
    expect(t.onToast).toHaveBeenLastCalledWith(fork)
    expect(t.tool.statusHint()).toMatch(/Click the path/)
    expect(t.scene.deform_preview).not.toHaveBeenCalled()
  })

  it('a refused preview explains why in the status line', () => {
    const t = setup('bend-to-path', {
      ...pathScene(),
      deform_preview: vi.fn(() => { throw new Error('ShapeNeedsSolid: gaps') }),
    })
    t.tool.onPointerDown({ x: 0, y: 0, z: 0, kind: 'endpoint' }, down(0, 0))
    t.tool.onPointerDown({ x: 0, y: 0, z: 1, kind: 'endpoint' }, down(0, 0))
    t.tool.onPointerDown(null, down(0.5, 0))
    expect(t.tool.statusHint()).toMatch(/solid/i)
  })
})

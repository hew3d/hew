import { describe, it, expect, vi } from 'vitest'
import * as THREE from 'three'
import { LineTool } from './LineTool'
import { RectangleTool } from './RectangleTool'
import { CircleTool } from './CircleTool'
import { PolygonTool } from './PolygonTool'
import { ArcTool } from './ArcTool'
import { draftMock } from './draftMock'
import { DetachedMode } from './detachedMode'
import { cursorFor } from './toolIcons'
import type { LineMaterial } from 'three/examples/jsm/lines/LineMaterial.js'
import { PREVIEW_DRAW_JOINED_FALLBACK, PREVIEW_DETACHED_FALLBACK } from './previewTint'
import { DETACH_MODIFIER_LABEL } from '../platform'
import type { Snap, Tool } from './types'
import type { Scene as WasmScene } from '../wasm/loader'
import type { Ray } from '../viewport/math'

// The Detached toggle on the five draw tools (separate-shapes design §6.1):
// readout prefix, status hint, commit routing, retype, Line's chain latch.

const RAY: Ray = { origin: [0, 0, 5], direction: [0, 0, -1] }
const snap = (x: number, y: number, z = 0, kind: Snap['kind'] = 'ground'): Snap => ({ x, y, z, kind })
const key = (k: string): KeyboardEvent => ({ key: k, repeat: false, preventDefault: () => {} }) as unknown as KeyboardEvent

function makePick(object: bigint, face: bigint) {
  return { object: () => object, face: () => face, instance: () => undefined, depth: () => 1, free: vi.fn() }
}

function makeScene(opts: { pick?: ReturnType<typeof makePick>; failSegmentAt?: (n: number) => Error | null } = {}) {
  let gen = 1n
  const dm = draftMock({ failSegmentAt: opts.failSegmentAt })
  const baseEnd = dm.members.draft_end
  const scene = {
    history_generation: vi.fn(() => gen),
    ...dm.members,
    draft_end: vi.fn((d: bigint) => { const r = baseEnd(d); gen += 1n; return r }),
    scene_undo: vi.fn(() => { gen += 1n; return { free: vi.fn() } }),
    scene_redo: vi.fn(() => { gen += 1n; return { free: vi.fn() } }),
    sketch_plane: vi.fn(() => new Float64Array([0, 0, 0, 0, 0, 1])),
    pick_face: vi.fn(() => opts.pick),
    pick_sketch: vi.fn(() => undefined),
    node_parent: vi.fn(() => undefined),
    face_normal: vi.fn(() => new Float64Array([0, 0, 1])),
    face_plane: vi.fn(() => new Float64Array([0, 0, 1, 0, 0, 1])),
    instance_pose: vi.fn(() => new Float64Array([1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0])),
    axes: vi.fn(() => new Float64Array([0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1])),
    clear_transient_segments: vi.fn(),
    add_transient_segment: vi.fn(),
  }
  return { scene: scene as unknown as WasmScene, raw: scene, log: dm.log }
}

interface Rig {
  tool: Tool & { toggleDetached(): void; detachedOn: boolean }
  /** The tool's rubber-band group. */
  preview: THREE.Group
  onMeasurement: ReturnType<typeof vi.fn>
  onDetachedChange: ReturnType<typeof vi.fn>
  onFaceImprint: ReturnType<typeof vi.fn>
  /** Clicks that commit one shape of this tool on `at`. */
  draw(at: number): void
  /** A click sequence that has the tool mid-gesture (a readout showing). */
  startGesture(): void
}

const noop = () => { /* no-op */ }

function rig(name: string, scene: WasmScene): Rig {
  const preview = new THREE.Group()
  const onCommit = vi.fn()
  const onToast = vi.fn()
  const onFaceImprint = vi.fn()
  const onMeasurement = vi.fn()
  const onDetachedChange = vi.fn()
  const click = (tool: Tool, x: number, y: number) => tool.onPointerDown(snap(x, y), RAY)
  switch (name) {
    case 'Circle': {
      const tool = new CircleTool(scene, preview, onCommit, onToast, onFaceImprint, onMeasurement, onDetachedChange)
      return { tool, preview, onMeasurement, onDetachedChange, onFaceImprint, draw: (at) => { click(tool, at, 0); click(tool, at + 2, 0) }, startGesture: () => { click(tool, 0, 0); tool.onPointerMove(snap(1, 0), RAY) } }
    }
    case 'Rectangle': {
      const tool = new RectangleTool(scene, preview, onCommit, onToast, onFaceImprint, onMeasurement, onDetachedChange)
      return { tool, preview, onMeasurement, onDetachedChange, onFaceImprint, draw: (at) => { click(tool, at, 0); click(tool, at + 2, 1) }, startGesture: () => { click(tool, 0, 0); tool.onPointerMove(snap(2, 1), RAY) } }
    }
    case 'Polygon': {
      const tool = new PolygonTool(scene, preview, onCommit, onToast, onFaceImprint, onMeasurement, noop, onDetachedChange)
      return { tool, preview, onMeasurement, onDetachedChange, onFaceImprint, draw: (at) => { click(tool, at, 0); click(tool, at + 2, 0) }, startGesture: () => { click(tool, 0, 0); tool.onPointerMove(snap(1, 0), RAY) } }
    }
    case 'Arc': {
      const tool = new ArcTool(scene, preview, onCommit, onToast, onFaceImprint, onMeasurement, onDetachedChange)
      return { tool, preview, onMeasurement, onDetachedChange, onFaceImprint, draw: (at) => { click(tool, at, 0); click(tool, at + 4, 0); tool.onPointerMove(snap(at + 2, 1), RAY); click(tool, at + 2, 1) }, startGesture: () => { click(tool, 0, 0); tool.onPointerMove(snap(2, 0), RAY) } }
    }
    default: {
      const tool = new LineTool(scene, preview, onCommit, onToast, onFaceImprint, onMeasurement, onDetachedChange)
      return { tool, preview, onMeasurement, onDetachedChange, onFaceImprint, draw: (at) => { click(tool, at, 0); click(tool, at + 2, 0) }, startGesture: () => { click(tool, 0, 0); tool.onPointerMove(snap(2, 0), RAY) } }
    }
  }
}

const TOOLS = ['Line', 'Rectangle', 'Circle', 'Polygon', 'Arc']

describe.each(TOOLS)('%s — Detached toggle', (name) => {
  it('toggleDetached flips the mode, reports it, and prefixes the readout', () => {
    const { scene } = makeScene()
    const r = rig(name, scene)
    r.startGesture()
    const plain = r.onMeasurement.mock.calls.map((c) => c[0] as string).filter((s) => s !== '').pop()
    expect(plain).toBeDefined()
    expect(plain).not.toMatch(/^Detached/)

    r.tool.toggleDetached()
    expect(r.tool.detachedOn).toBe(true)
    expect(r.onDetachedChange).toHaveBeenLastCalledWith(true)
    // The readout is re-reported at once with the prefix.
    expect(r.onMeasurement).toHaveBeenLastCalledWith(`Detached · ${plain}`)

    r.tool.toggleDetached()
    expect(r.tool.detachedOn).toBe(false)
    expect(r.onDetachedChange).toHaveBeenLastCalledWith(false)
    expect(r.onMeasurement).toHaveBeenLastCalledWith(plain)
  })

  it('the status hint offers the tap when off and says what it means when on', () => {
    const { scene } = makeScene()
    const { tool } = rig(name, scene)
    expect(tool.statusHint?.()).toContain(`Tap ${DETACH_MODIFIER_LABEL} to detach the shape from the face.`)
    tool.toggleDetached()
    const on = tool.statusHint?.() ?? ''
    expect(on).toContain('Detach is on')
    expect(on).not.toContain('Tap ')
  })

  it('a commit while Detach is on opens draft_begin_detached on the plane, never imprints, and starts off again after a reset', () => {
    const { scene, raw } = makeScene()
    const r = rig(name, scene)
    r.tool.toggleDetached()
    r.draw(0)
    expect(raw.draft_begin_detached).toHaveBeenCalledWith('{"plane":[0,0,0,0,0,1]}', expect.any(String))
    expect(raw.draft_begin_plane).not.toHaveBeenCalled()
    expect(r.onFaceImprint).not.toHaveBeenCalled()
    ;(r.tool as unknown as { onDocumentReset(): void }).onDocumentReset()
    expect(r.tool.detachedOn).toBe(false)
    expect(r.onDetachedChange).toHaveBeenLastCalledWith(false)
  })

  it('without the toggle a commit opens the ordinary plane draft (the default split is untouched)', () => {
    const { scene, raw } = makeScene()
    const r = rig(name, scene)
    r.draw(0)
    expect(raw.draft_begin_plane).toHaveBeenCalledTimes(1)
    expect(raw.draft_begin_detached).not.toHaveBeenCalled()
  })

  it('a face commit while Detach is on opens draft_begin_detached on the face', () => {
    const { scene, raw } = makeScene({ pick: makePick(7n, 3n) })
    const r = rig(name, scene)
    r.tool.toggleDetached()
    const click = (x: number, y: number) => r.tool.onPointerDown(snap(x, y, 1, 'face'), RAY)
    if (name === 'Arc') {
      click(0, 0); click(4, 0); r.tool.onPointerMove(snap(2, 1, 1, 'face'), RAY); click(2, 1)
    } else if (name === 'Rectangle') {
      click(0, 0); click(2, 1)
    } else {
      click(0, 0); click(2, 0)
    }
    expect(raw.draft_begin_detached).toHaveBeenCalledWith('{"face":{"object":7,"face":3}}', expect.any(String))
    expect(raw.draft_begin_face).not.toHaveBeenCalled()
    expect(r.onFaceImprint).not.toHaveBeenCalled()
  })
})

describe('retype snapshot — a retyped value re-commits with the mode the shape was drawn with', () => {
  it.each(['Circle', 'Rectangle'])('%s keeps its mode when a size is typed after a tap flips it', (name) => {
    const { scene, raw } = makeScene()
    const r = rig(name, scene)
    r.tool.toggleDetached()
    r.draw(0)
    expect(raw.draft_begin_detached).toHaveBeenCalledTimes(1)
    r.tool.toggleDetached() // off again, window still open
    const typed = name === 'Circle' ? '3' : '3,2'
    for (const ch of typed) r.tool.onKey?.(key(ch))
    r.tool.onKey?.(key('Enter'))
    expect(raw.scene_undo).toHaveBeenCalledTimes(1)
    // The re-commit is detached again, not the (now off) live mode.
    expect(raw.draft_begin_detached).toHaveBeenCalledTimes(2)
    expect(raw.draft_begin_plane).not.toHaveBeenCalled()
  })
})

describe('LineTool — a chain is one detached sketch (D12)', () => {
  const line = (scene: WasmScene) => rig('Line', scene)

  it('latches Detached at the first segment and continues into the sketch it created', () => {
    const { scene, raw } = makeScene()
    const r = line(scene)
    r.tool.toggleDetached()
    r.tool.onPointerDown(snap(0, 0), RAY)
    r.tool.onPointerDown(snap(2, 0), RAY)
    r.tool.onPointerDown(snap(2, 2), RAY)
    r.tool.onPointerDown(snap(4, 2), RAY)
    expect(raw.draft_begin_detached).toHaveBeenCalledTimes(1)
    // Later segments continue into the new sketch (the mock's first minted id, 901).
    const ctxCalls = (raw.draft_begin_ctx as ReturnType<typeof vi.fn>).mock.calls
    expect(ctxCalls.map((c) => c[0])).toEqual(['{"sketch":901}', '{"sketch":901}'])
    expect(raw.draft_begin_plane).not.toHaveBeenCalled()
  })

  it('a tap mid-chain changes the readout and hint but not the chain; the next chain uses it', () => {
    const { scene, raw } = makeScene()
    const r = line(scene)
    r.tool.onPointerDown(snap(0, 0), RAY)
    r.tool.onPointerDown(snap(2, 0), RAY) // first segment: latched OFF
    expect(raw.draft_begin_plane).toHaveBeenCalledTimes(1)
    r.tool.toggleDetached()
    expect(r.tool.statusHint?.()).toContain('applies to the next line')
    r.tool.onPointerDown(snap(2, 2), RAY)
    expect(raw.draft_begin_detached).not.toHaveBeenCalled()
    expect(raw.draft_begin_plane).toHaveBeenCalledTimes(2)
    // End the chain; the next one is detached.
    r.tool.onKey?.(key('Escape'))
    r.tool.onPointerDown(snap(8, 0), RAY)
    r.tool.onPointerDown(snap(10, 0), RAY)
    expect(raw.draft_begin_detached).toHaveBeenCalledTimes(1)
  })

  it('a refused re-homed segment keeps the chain on its old plane AND in its old detached sketch', () => {
    const { scene, raw } = makeScene({ failSegmentAt: (n) => (n === 2 ? new Error('DegenerateSegment: too short') : null) })
    const r = line(scene)
    r.tool.toggleDetached()
    r.tool.onPointerDown(snap(0, 0), RAY)
    r.tool.onPointerDown(snap(2, 0), RAY)
    r.tool.onPointerDown(snap(2, 2), RAY)
    expect(raw.draft_begin_detached).toHaveBeenCalledTimes(1)
    const sketchOf = () => (r.tool as unknown as { chainSketch: bigint | null }).chainSketch
    expect(sketchOf()).toBe(901n)
    // Off the frozen plane: re-homes, and the kernel refuses that segment.
    r.tool.onPointerDown(snap(2, 2, 3), RAY)
    expect(sketchOf()).toBe(901n)
    // The refused attempt itself opened (and dropped) one draft; the next
    // on-plane click must continue into the old sketch, not open a third.
    const opened = (raw.draft_begin_detached as ReturnType<typeof vi.fn>).mock.calls.length
    r.tool.onPointerDown(snap(4, 2), RAY)
    expect(raw.draft_begin_detached).toHaveBeenCalledTimes(opened)
    const last = (raw.draft_begin_ctx as ReturnType<typeof vi.fn>).mock.calls.at(-1)
    expect(last?.[0]).toBe('{"sketch":901}')
  })

  it('retyping the first segment of a detached chain re-opens a new detached sketch (its sketch died with the undo)', () => {
    const { scene, raw } = makeScene()
    const r = line(scene)
    r.tool.toggleDetached()
    r.tool.onPointerDown(snap(0, 0), RAY)
    r.tool.onPointerDown(snap(2, 0), RAY)
    expect(raw.draft_begin_detached).toHaveBeenCalledTimes(1)
    for (const ch of '3') r.tool.onKey?.(key(ch))
    r.tool.onKey?.(key('Enter'))
    expect(raw.scene_undo).toHaveBeenCalledTimes(1)
    expect(raw.draft_begin_detached).toHaveBeenCalledTimes(2)
    expect(raw.draft_begin_ctx).not.toHaveBeenCalled()
  })
})

describe('DetachedMode', () => {
  it('decorates only non-empty readouts and re-reports on toggle', () => {
    const sink = vi.fn()
    const change = vi.fn()
    const m = new DetachedMode(sink, change)
    m.report('1 m')
    expect(sink).toHaveBeenLastCalledWith('1 m')
    m.toggle()
    expect(sink).toHaveBeenLastCalledWith('Detached · 1 m')
    m.report('')
    expect(sink).toHaveBeenLastCalledWith('')
    m.reset()
    expect(change).toHaveBeenLastCalledWith(false)
    expect(m.on).toBe(false)
  })
})

describe('Detached cursor badge and preview tint', () => {
  it.each(TOOLS)('%s: the mode callback drives a + badged cursor, and plain again when off or reset', (name) => {
    const { scene } = makeScene()
    const r = rig(name, scene)
    // The Viewport maps the callback to cursorFor(name, on).
    const cursors: string[] = []
    r.onDetachedChange.mockImplementation((on: boolean) => { cursors.push(cursorFor(name, on)) })
    r.tool.toggleDetached()
    expect(cursors[0]).toBe(cursorFor(name, true))
    expect(cursors[0]).not.toBe(cursorFor(name))
    r.tool.toggleDetached()
    expect(cursors[1]).toBe(cursorFor(name))
    r.tool.toggleDetached()
    ;(r.tool as unknown as { onDocumentReset(): void }).onDocumentReset()
    expect(cursors[3]).toBe(cursorFor(name))
  })

  it('the preview is orange only when the shape joins a face and Detached is off; blue otherwise', () => {
    const mode = new DetachedMode(() => { /* no-op */ }, () => { /* no-op */ })
    expect(mode.previewStyle(true).color).toBe(PREVIEW_DRAW_JOINED_FALLBACK)
    // Ground / void / any non-face anchor: blue even with Detached off.
    expect(mode.previewStyle(false).color).toBe(PREVIEW_DETACHED_FALLBACK)
    mode.toggle()
    expect(mode.previewStyle(true).color).toBe(PREVIEW_DETACHED_FALLBACK)
    expect(mode.previewStyle(false).color).toBe(PREVIEW_DETACHED_FALLBACK)
    expect(PREVIEW_DRAW_JOINED_FALLBACK).not.toBe(PREVIEW_DETACHED_FALLBACK)
  })
})

/** The colour of the rubber band currently in `preview` (every fat-line child
 *  shares one style), or null when nothing is drawn. */
function bandColor(preview: THREE.Group): number | null {
  const kids = preview.children.filter((c) => (c as { material?: unknown }).material !== undefined)
  if (kids.length === 0) return null
  return ((kids[0] as unknown as { material: LineMaterial }).material).color.getHex()
}

describe.each(TOOLS)('%s — rubber band colour at the call site', (name) => {
  const ORANGE = new THREE.Color(PREVIEW_DRAW_JOINED_FALLBACK).getHex()
  const BLUE = new THREE.Color(PREVIEW_DETACHED_FALLBACK).getHex()

  /** Anchor, then hover so the tool has a rubber band; `kind` picks face vs plane mode. */
  function hover(r: Rig, face: boolean): number | null {
    const z = face ? 1 : 0
    const k: Snap['kind'] = face ? 'face' : 'ground'
    const at = (x: number, y: number) => snap(x, y, z, k)
    r.tool.onPointerDown(at(0, 0), RAY)
    if (name === 'Arc') {
      r.tool.onPointerMove(at(4, 0), RAY)
    } else {
      r.tool.onPointerMove(at(name === 'Rectangle' ? 2 : 1, name === 'Rectangle' ? 1 : 0), RAY)
    }
    return bandColor(r.preview)
  }

  it('face mode, Detached off: the orange joined tint', () => {
    const { scene } = makeScene({ pick: makePick(7n, 3n) })
    expect(hover(rig(name, scene), true)).toBe(ORANGE)
  })

  it('face mode, Detached on: blue', () => {
    const { scene } = makeScene({ pick: makePick(7n, 3n) })
    const r = rig(name, scene)
    r.tool.toggleDetached()
    expect(hover(r, true)).toBe(BLUE)
  })

  it('plane / ground mode: blue with Detached off and on', () => {
    const off = rig(name, makeScene().scene)
    expect(hover(off, false)).toBe(BLUE)
    const on = rig(name, makeScene().scene)
    on.tool.toggleDetached()
    expect(hover(on, false)).toBe(BLUE)
  })
})

describe('LineTool — the rubber band follows the mode latched for the chain', () => {
  const ORANGE = new THREE.Color(PREVIEW_DRAW_JOINED_FALLBACK).getHex()
  const BLUE = new THREE.Color(PREVIEW_DETACHED_FALLBACK).getHex()
  const face = (x: number, y: number) => snap(x, y, 1, 'face')

  it('a chain started attached stays orange when Detached is tapped mid-chain', () => {
    const r = rig('Line', makeScene({ pick: makePick(7n, 3n) }).scene)
    r.tool.onPointerDown(face(0, 0), RAY)
    r.tool.onPointerDown(face(2, 0), RAY) // first segment: latched attached
    r.tool.onPointerMove(face(2, 2), RAY)
    expect(bandColor(r.preview)).toBe(ORANGE)
    r.tool.toggleDetached()
    r.tool.onPointerMove(face(3, 2), RAY)
    expect(bandColor(r.preview)).toBe(ORANGE)
  })

  it('a chain started detached stays blue when Detached is tapped off mid-chain', () => {
    const r = rig('Line', makeScene({ pick: makePick(7n, 3n) }).scene)
    r.tool.toggleDetached()
    r.tool.onPointerDown(face(0, 0), RAY)
    r.tool.onPointerDown(face(2, 0), RAY) // first segment: latched detached
    r.tool.onPointerMove(face(2, 2), RAY)
    expect(bandColor(r.preview)).toBe(BLUE)
    r.tool.toggleDetached()
    r.tool.onPointerMove(face(3, 2), RAY)
    expect(bandColor(r.preview)).toBe(BLUE)
  })
})

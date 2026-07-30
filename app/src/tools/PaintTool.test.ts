/**
 * PaintTool unit tests — plain paint/whole-object (pre-existing behavior,
 * pinned here for the first time), plus the eyedropper (Alt-sample) and
 * Shift/Ctrl+Shift "replace everywhere" states added by the paint-tool
 * design (§1–2). Mirrors the fake-WasmScene pattern used by
 * PushPullTool.test.ts/CircleTool.test.ts.
 */
import { describe, it, expect, vi } from 'vitest'
import { PaintTool, MATERIAL_SENTINEL } from './PaintTool'
import type { Snap } from './types'
import type { Scene as WasmScene } from '../wasm/loader'
import type { Ray } from '../viewport/math'

const RAY: Ray = { origin: [0, 0, 5], direction: [0, 0, -1] }

function makeSnap(overrides: Partial<Snap> = {}): Snap {
  return { x: 0, y: 0, z: 0, kind: 'ground', ...overrides }
}

/** A fake `FacePickJs` returning the seeded handles. */
function makeFacePick(object: bigint, face: bigint) {
  return {
    object: () => object,
    face: () => face,
    instance: () => undefined,
    free: vi.fn(),
  }
}

/** A fake `FaceMaterialJs` returning the seeded pair. */
function makeFaceMaterial(face: bigint, objectDefault: bigint) {
  return {
    face: () => face,
    object_default: () => objectDefault,
    free: vi.fn(),
  }
}

function makeWasmScene(opts: {
  facePick?: ReturnType<typeof makeFacePick>
  /** `face_material(object, face)` result — `undefined` simulates a stale
   *  pick (the object/face vanished between hover and click). */
  faceMaterial?: ReturnType<typeof makeFaceMaterial> | undefined
  paintFaceThrows?: string
  replaceMaterialThrows?: string
} = {}): WasmScene {
  return {
    pick_face: vi.fn(() => opts.facePick),
    face_material: vi.fn(() => opts.faceMaterial),
    paint_face: vi.fn(() => {
      if (opts.paintFaceThrows !== undefined) throw new Error(opts.paintFaceThrows)
    }),
    set_object_material: vi.fn(),
    replace_material: vi.fn(() => {
      if (opts.replaceMaterialThrows !== undefined) throw new Error(opts.replaceMaterialThrows)
    }),
  } as unknown as WasmScene
}

function makeTool(scene: WasmScene) {
  const onCommit = vi.fn()
  const onToast = vi.fn()
  const onSample = vi.fn()
  const onReplace = vi.fn()
  const tool = new PaintTool(scene, onCommit, onToast, onSample, onReplace)
  return { tool, onCommit, onToast, onSample, onReplace }
}

describe('PaintTool — plain paint and whole-object (pre-existing behavior)', () => {
  it('a plain click paints the picked face with the current material', () => {
    const scene = makeWasmScene({ facePick: makeFacePick(3n, 4n) })
    const { tool, onCommit } = makeTool(scene)
    tool.setCurrentMaterial(7n)

    tool.onPointerDown(makeSnap(), RAY)

    expect(scene.paint_face).toHaveBeenCalledWith(3n, 4n, 7n)
    expect(scene.set_object_material).not.toHaveBeenCalled()
    expect(scene.replace_material).not.toHaveBeenCalled()
    expect(onCommit).toHaveBeenCalledWith(3n)
  })

  it('setWholeObject(true) fills the object base material instead, then auto-resets', () => {
    const scene = makeWasmScene({ facePick: makeFacePick(3n, 4n) })
    const { tool, onCommit } = makeTool(scene)
    tool.setCurrentMaterial(7n)
    tool.setWholeObject(true)

    tool.onPointerDown(makeSnap(), RAY)
    expect(scene.set_object_material).toHaveBeenCalledWith(3n, 7n)
    expect(scene.paint_face).not.toHaveBeenCalled()
    expect(onCommit).toHaveBeenCalledWith(3n)

    // Auto-reset: a second click without re-arming paints the face again.
    tool.onPointerDown(makeSnap(), RAY)
    expect(scene.paint_face).toHaveBeenCalledTimes(1)
  })

  it('a stale pick (pick_face misses) is a silent no-op', () => {
    const scene = makeWasmScene({ facePick: undefined })
    const { tool, onCommit, onToast } = makeTool(scene)

    tool.onPointerDown(makeSnap(), RAY)

    expect(scene.paint_face).not.toHaveBeenCalled()
    expect(onCommit).not.toHaveBeenCalled()
    expect(onToast).not.toHaveBeenCalled()
  })

  it('a refused paint_face reports through onToast, not a thrown exception', () => {
    const scene = makeWasmScene({
      facePick: makeFacePick(3n, 4n),
      paintFaceThrows: 'UnknownMaterial: material handle is not in the palette',
    })
    const { tool, onCommit, onToast } = makeTool(scene)

    tool.onPointerDown(makeSnap(), RAY)

    expect(onCommit).not.toHaveBeenCalled()
    expect(onToast).toHaveBeenCalledTimes(1)
    expect(String((onToast as ReturnType<typeof vi.fn>).mock.calls[0][0])).toContain('palette')
  })
})

describe('PaintTool — eyedropper (Alt-sample, design §1)', () => {
  it('samples the face\'s own material and makes it current', () => {
    const scene = makeWasmScene({
      facePick: makeFacePick(3n, 4n),
      faceMaterial: makeFaceMaterial(9n, 5n),
    })
    const { tool, onSample, onCommit } = makeTool(scene)
    tool.setCurrentMaterial(1n)
    tool.setEyedropper(true)

    tool.onPointerDown(makeSnap(), RAY)

    expect(scene.face_material).toHaveBeenCalledWith(3n, 4n)
    expect(tool.getCurrentMaterial()).toBe(9n)
    expect(onSample).toHaveBeenCalledWith(9n)
    // Sampling never paints/commits a document mutation.
    expect(scene.paint_face).not.toHaveBeenCalled()
    expect(scene.set_object_material).not.toHaveBeenCalled()
    expect(onCommit).not.toHaveBeenCalled()
  })

  it('falls back to the object default when the face carries none', () => {
    const scene = makeWasmScene({
      facePick: makeFacePick(3n, 4n),
      faceMaterial: makeFaceMaterial(MATERIAL_SENTINEL, 5n),
    })
    const { tool, onSample } = makeTool(scene)
    tool.setEyedropper(true)

    tool.onPointerDown(makeSnap(), RAY)

    expect(tool.getCurrentMaterial()).toBe(5n)
    expect(onSample).toHaveBeenCalledWith(5n)
  })

  it('an unpainted face (both sentinels) samples the Default swatch', () => {
    const scene = makeWasmScene({
      facePick: makeFacePick(3n, 4n),
      faceMaterial: makeFaceMaterial(MATERIAL_SENTINEL, MATERIAL_SENTINEL),
    })
    const { tool, onSample } = makeTool(scene)
    tool.setEyedropper(true)

    tool.onPointerDown(makeSnap(), RAY)

    expect(tool.getCurrentMaterial()).toBe(MATERIAL_SENTINEL)
    expect(onSample).toHaveBeenCalledWith(MATERIAL_SENTINEL)
  })

  it('takes priority over a wholeObject/replaceScope modifier held at the same click', () => {
    const scene = makeWasmScene({
      facePick: makeFacePick(3n, 4n),
      faceMaterial: makeFaceMaterial(9n, MATERIAL_SENTINEL),
    })
    const { tool, onSample } = makeTool(scene)
    tool.setEyedropper(true)
    tool.setWholeObject(true)
    tool.setReplaceScope('document')

    tool.onPointerDown(makeSnap(), RAY)

    expect(onSample).toHaveBeenCalledWith(9n)
    expect(scene.set_object_material).not.toHaveBeenCalled()
    expect(scene.replace_material).not.toHaveBeenCalled()
  })

  it('a stale pick (face_material misses) samples nothing', () => {
    const scene = makeWasmScene({
      facePick: makeFacePick(3n, 4n),
      faceMaterial: undefined,
    })
    const { tool, onSample } = makeTool(scene)
    const before = tool.getCurrentMaterial()
    tool.setEyedropper(true)

    tool.onPointerDown(makeSnap(), RAY)

    expect(onSample).not.toHaveBeenCalled()
    expect(tool.getCurrentMaterial()).toBe(before)
  })

  it('statusHint reflects the live eyedropper state', () => {
    const scene = makeWasmScene()
    const { tool } = makeTool(scene)
    expect(tool.statusHint()).not.toContain('Alt-click a face to sample')
    tool.setEyedropper(true)
    expect(tool.statusHint()).toContain('Alt-click a face to sample')
    tool.setEyedropper(false)
    expect(tool.statusHint()).not.toContain('Alt-click a face to sample')
  })
})

describe('PaintTool — replace everywhere (Shift/Ctrl+Shift, design §2)', () => {
  it('Shift-click replaces document-wide using the clicked face\'s effective material as `from`', () => {
    const scene = makeWasmScene({
      facePick: makeFacePick(3n, 4n),
      faceMaterial: makeFaceMaterial(9n, MATERIAL_SENTINEL),
    })
    const { tool, onReplace, onCommit } = makeTool(scene)
    tool.setCurrentMaterial(2n)
    tool.setReplaceScope('document')

    tool.onPointerDown(makeSnap(), RAY)

    expect(scene.replace_material).toHaveBeenCalledWith(true, 3n, 9n, 2n)
    expect(onReplace).toHaveBeenCalledWith('document', 3n)
    expect(scene.paint_face).not.toHaveBeenCalled()
    expect(onCommit).not.toHaveBeenCalled()
  })

  it('Ctrl/Cmd+Shift-click replaces confined to the clicked object', () => {
    const scene = makeWasmScene({
      facePick: makeFacePick(3n, 4n),
      faceMaterial: makeFaceMaterial(9n, MATERIAL_SENTINEL),
    })
    const { tool, onReplace } = makeTool(scene)
    tool.setCurrentMaterial(2n)
    tool.setReplaceScope('object')

    tool.onPointerDown(makeSnap(), RAY)

    expect(scene.replace_material).toHaveBeenCalledWith(false, 3n, 9n, 2n)
    expect(onReplace).toHaveBeenCalledWith('object', 3n)
  })

  it('an unpainted clicked face replaces using the sentinel as `from`', () => {
    const scene = makeWasmScene({
      facePick: makeFacePick(3n, 4n),
      faceMaterial: makeFaceMaterial(MATERIAL_SENTINEL, MATERIAL_SENTINEL),
    })
    const { tool } = makeTool(scene)
    tool.setCurrentMaterial(2n)
    tool.setReplaceScope('document')

    tool.onPointerDown(makeSnap(), RAY)

    expect(scene.replace_material).toHaveBeenCalledWith(true, 3n, MATERIAL_SENTINEL, 2n)
  })

  it('auto-resets after one click — a follow-up click paints normally', () => {
    const scene = makeWasmScene({
      facePick: makeFacePick(3n, 4n),
      faceMaterial: makeFaceMaterial(9n, MATERIAL_SENTINEL),
    })
    const { tool } = makeTool(scene)
    tool.setReplaceScope('document')

    tool.onPointerDown(makeSnap(), RAY)
    expect(scene.replace_material).toHaveBeenCalledTimes(1)

    tool.onPointerDown(makeSnap(), RAY)
    expect(scene.replace_material).toHaveBeenCalledTimes(1)
    expect(scene.paint_face).toHaveBeenCalledTimes(1)
  })

  it('a refused replace_material reports through onToast, not a thrown exception', () => {
    const scene = makeWasmScene({
      facePick: makeFacePick(3n, 4n),
      faceMaterial: makeFaceMaterial(9n, MATERIAL_SENTINEL),
      replaceMaterialThrows: 'UnknownObject: stale or hidden object handle',
    })
    const { tool, onReplace, onToast } = makeTool(scene)
    tool.setReplaceScope('object')

    tool.onPointerDown(makeSnap(), RAY)

    expect(onReplace).not.toHaveBeenCalled()
    expect(onToast).toHaveBeenCalledTimes(1)
  })

  it('a stale pick (face_material misses) replaces nothing', () => {
    const scene = makeWasmScene({
      facePick: makeFacePick(3n, 4n),
      faceMaterial: undefined,
    })
    const { tool, onReplace } = makeTool(scene)
    tool.setReplaceScope('document')

    tool.onPointerDown(makeSnap(), RAY)

    expect(scene.replace_material).not.toHaveBeenCalled()
    expect(onReplace).not.toHaveBeenCalled()
  })
})

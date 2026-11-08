/**
 * PaintTool unit tests — plain paint/whole-object (pre-existing behavior,
 * pinned here for the first time), plus the eyedropper (Alt-sample) and
 * Shift/Ctrl+Shift "replace everywhere" states added by the paint-tool
 * design (§1–2). Mirrors the fake-WasmScene pattern used by
 * PushPullTool.test.ts/CircleTool.test.ts.
 */
import { describe, it, expect, vi } from 'vitest'
import { PaintTool, MATERIAL_SENTINEL, resetInstancePaintExplained } from './PaintTool'
import type { NodeRef } from '../panels/treeModel'
import type { Snap } from './types'
import type { Scene as WasmScene } from '../wasm/loader'
import type { Ray } from '../viewport/math'

const RAY: Ray = { origin: [0, 0, 5], direction: [0, 0, -1] }

function makeSnap(overrides: Partial<Snap> = {}): Snap {
  return { x: 0, y: 0, z: 0, kind: 'ground', ...overrides }
}

/** A fake `FacePickJs` returning the seeded handles (`instance`: the
 *  placement the pick went through, if any). */
function makeFacePick(object: bigint, face: bigint, instance?: bigint) {
  return {
    object: () => object,
    face: () => face,
    depth: () => 1,
    instance: () => instance,
    free: vi.fn(),
  }
}

/** A fake `FaceMaterialJs` returning the seeded pair. */
function makeFaceMaterial(face: bigint, objectDefault: bigint) {
  return {
    face: () => face,
    depth: () => 1,
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
  /** `instance_expanded_members`/`instance_expanded_inherited` of the
   *  picked placement; omitted, nothing inherited. */
  expanded?: { members: bigint[]; inherited: bigint[] }
  /** What `set_instance_material` reports as redrawn; omitted, `[]`. */
  touched?: bigint[]
  /** The placement's linked set definition; omitted, not a linked copy. */
  linkedDef?: bigint
} = {}): WasmScene {
  return {
    pick_face: vi.fn(() => opts.facePick),
    face_material: vi.fn(() => opts.faceMaterial),
    paint_face: vi.fn(() => {
      if (opts.paintFaceThrows !== undefined) throw new Error(opts.paintFaceThrows)
    }),
    set_object_material: vi.fn(),
    set_instance_material: vi.fn(() => BigUint64Array.from(opts.touched ?? [])),
    is_linked: vi.fn(() => opts.linkedDef !== undefined),
    instance_def: vi.fn(() => opts.linkedDef),
    instance_expanded_members: vi.fn(() => BigUint64Array.from(opts.expanded?.members ?? [])),
    instance_expanded_inherited: vi.fn(() => BigUint64Array.from(opts.expanded?.inherited ?? [])),
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

describe('PaintTool — component placements', () => {
  function makeInstanceTool(scene: WasmScene, stack: NodeRef[] = []) {
    const onCommit = vi.fn()
    const onToast = vi.fn()
    const onSample = vi.fn()
    const onReplace = vi.fn()
    const onInstanceCommit = vi.fn()
    const onHover = vi.fn()
    const tool = new PaintTool(scene, onCommit, onToast, onSample, onReplace, () => stack, onInstanceCommit, onHover)
    return { tool, onCommit, onToast, onSample, onInstanceCommit, onHover }
  }

  it('a click on an instance pick at the top level paints the placement', () => {
    resetInstancePaintExplained()
    const scene = makeWasmScene({ facePick: makeFacePick(3n, 4n, 9n), touched: [9n] })
    const { tool, onInstanceCommit, onToast } = makeInstanceTool(scene)
    tool.setCurrentMaterial(7n)
    tool.onPointerDown(makeSnap(), RAY)
    expect(scene.set_instance_material).toHaveBeenCalledWith(9n, 7n)
    expect(scene.paint_face).not.toHaveBeenCalled()
    expect(onInstanceCommit).toHaveBeenCalledWith([9n], [])
    expect(onToast).toHaveBeenCalledWith(
      'Painting a component colors this placement. Edit the component to paint its faces.',
    )
    // The first one explains itself, once per session.
    expect(onToast).toHaveBeenCalledTimes(1)
    tool.onPointerDown(makeSnap(), RAY)
    expect(onToast).toHaveBeenCalledTimes(1)
  })

  it('painting a linked copy refreshes its set and explains it as a copy', () => {
    // The kernel may move the shared solid's base onto the other copies
    // before the paint, so every copy re-draws and the set's member
    // geometry is re-pulled.
    resetInstancePaintExplained()
    const scene = makeWasmScene({ facePick: makeFacePick(3n, 4n, 9n), touched: [9n, 10n, 11n], linkedDef: 5n })
    const { tool, onInstanceCommit, onToast } = makeInstanceTool(scene)
    tool.setCurrentMaterial(7n)
    tool.onPointerDown(makeSnap(), RAY)
    expect(scene.set_instance_material).toHaveBeenCalledWith(9n, 7n)
    expect(onInstanceCommit).toHaveBeenCalledWith([9n, 10n, 11n], [5n])
    expect(onToast).toHaveBeenCalledTimes(1)
    expect(onToast).toHaveBeenCalledWith(
      'Painting a linked copy colors just that copy. Edit the copy to paint faces on every copy.',
    )
    tool.onPointerDown(makeSnap(), RAY)
    expect(onToast).toHaveBeenCalledTimes(1)
  })

  it('a whole-object (Ctrl/Cmd) click on an instance also paints the placement', () => {
    const scene = makeWasmScene({ facePick: makeFacePick(3n, 4n, 9n) })
    const { tool } = makeInstanceTool(scene)
    tool.setCurrentMaterial(7n)
    tool.setWholeObject(true)
    tool.onPointerDown(makeSnap(), RAY)
    expect(scene.set_instance_material).toHaveBeenCalledWith(9n, 7n)
    expect(scene.set_object_material).not.toHaveBeenCalled()
  })

  it('inside the instance editing context a click paints the face', () => {
    const scene = makeWasmScene({ facePick: makeFacePick(3n, 4n, 9n) })
    const { tool, onCommit } = makeInstanceTool(scene, [{ kind: 'instance', id: 9n }])
    tool.setCurrentMaterial(7n)
    tool.onPointerDown(makeSnap(), RAY)
    expect(scene.paint_face).toHaveBeenCalledWith(3n, 4n, 7n)
    expect(scene.set_instance_material).not.toHaveBeenCalled()
    expect(onCommit).toHaveBeenCalledWith(3n)
  })

  it('painting a nested placement refreshes every world placement that draws it', () => {
    // Inside an outer placement's editing session, a surfaced nested
    // placement is the pick's instance; the kernel names the world
    // placements whose drawing changed.
    const scene = makeWasmScene({ facePick: makeFacePick(3n, 4n, 9n), touched: [20n, 21n] })
    const { tool, onInstanceCommit } = makeInstanceTool(scene, [{ kind: 'instance', id: 5n }])
    tool.setCurrentMaterial(7n)
    tool.onPointerDown(makeSnap(), RAY)
    expect(onInstanceCommit).toHaveBeenCalledWith([20n, 21n], [])
  })

  it('the eyedropper samples the nearest painted placement on the member path', () => {
    // Member 3n draws under an unpainted outer placement and a painted
    // nested one: the kernel's nearest-wins inherited material is 12n.
    const scene = makeWasmScene({
      facePick: makeFacePick(3n, 4n, 9n),
      expanded: { members: [1n, 3n], inherited: [MATERIAL_SENTINEL, 12n] },
      faceMaterial: makeFaceMaterial(5n, 6n),
    })
    const { tool, onSample } = makeInstanceTool(scene)
    tool.setEyedropper(true)
    tool.onPointerDown(makeSnap(), RAY)
    expect(onSample).toHaveBeenCalledWith(12n)
    expect(tool.getCurrentMaterial()).toBe(12n)
    expect(scene.face_material).not.toHaveBeenCalled()
  })

  it('the eyedropper falls back to the face when the placement has no material', () => {
    const scene = makeWasmScene({
      facePick: makeFacePick(3n, 4n, 9n),
      faceMaterial: makeFaceMaterial(MATERIAL_SENTINEL, 6n),
    })
    const { tool, onSample } = makeInstanceTool(scene)
    tool.setEyedropper(true)
    tool.onPointerDown(makeSnap(), RAY)
    expect(onSample).toHaveBeenCalledWith(6n)
  })

  it('Shift-replace on a placement whose member shows its color uses that color as `from`', () => {
    // A linked copy: the member has no face paint and no base, so the face
    // shows the placement's own material (12n), and that is what replaces.
    const scene = makeWasmScene({
      facePick: makeFacePick(3n, 4n, 9n),
      expanded: { members: [3n], inherited: [12n] },
      faceMaterial: makeFaceMaterial(MATERIAL_SENTINEL, MATERIAL_SENTINEL),
    })
    const { tool } = makeInstanceTool(scene)
    tool.setCurrentMaterial(7n)
    tool.setReplaceScope('document')
    tool.onPointerDown(makeSnap(), RAY)
    expect(scene.replace_material).toHaveBeenCalledWith(true, 3n, 12n, 7n)
    expect(scene.set_instance_material).not.toHaveBeenCalled()
  })

  it('Shift-replace on a placement keeps the face or base it shows over the placement', () => {
    // The render rule: a face's own paint, else its Object's base, and only
    // then the placement's material.
    for (const [face, base, from] of [
      [5n, 6n, 5n],
      [MATERIAL_SENTINEL, 6n, 6n],
    ] as const) {
      const scene = makeWasmScene({
        facePick: makeFacePick(3n, 4n, 9n),
        expanded: { members: [3n], inherited: [12n] },
        faceMaterial: makeFaceMaterial(face, base),
      })
      const { tool } = makeInstanceTool(scene)
      tool.setCurrentMaterial(7n)
      tool.setReplaceScope('document')
      tool.onPointerDown(makeSnap(), RAY)
      expect(scene.replace_material).toHaveBeenCalledWith(true, 3n, from, 7n)
    }
  })

  it('hovering reports the placement a click would paint, and clears on cancel', () => {
    const scene = makeWasmScene({ facePick: makeFacePick(3n, 4n, 9n) })
    const { tool, onHover } = makeInstanceTool(scene)
    tool.onPointerMove(makeSnap(), RAY)
    expect(tool.hoveredInstance).toBe(9n)
    expect(onHover).toHaveBeenLastCalledWith(9n)
    tool.cancel()
    expect(onHover).toHaveBeenLastCalledWith(null)
  })
})


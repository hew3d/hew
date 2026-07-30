/**
 * annotationEdit tests — read-then-recommit round trip for all three
 * annotation kinds, and the text_override set/clear semantics
 * (docs/design/dimensions-text.md's simplified SketchUp `<>`).
 */
import { describe, it, expect, vi } from 'vitest'
import { readAnnotation, initialEditorText, commitAnnotationText } from './annotationEdit'
import type { Scene as WasmScene } from '../wasm/loader'

function makeLinearFakeScene(textOverride: string | undefined) {
  const updateCalls: unknown[][] = []
  const scene = {
    annotation_kind: vi.fn(() => 'linear'),
    annotation_anchor_node_kind: vi.fn((_id: bigint, which: number) => (which === 0 ? 0 : -1)),
    annotation_anchor_node_id: vi.fn((_id: bigint, which: number) => (which === 0 ? 5n : 0n)),
    annotation_anchor_point: vi.fn((_id: bigint, which: number) =>
      which === 0 ? new Float64Array([0, 0, 0]) : new Float64Array([2, 0, 0])),
    annotation_offset: vi.fn(() => new Float64Array([0, 1, 0])),
    annotation_plane: vi.fn(() => new Float64Array([0, 0, 0, 0, 0, 1])),
    annotation_text_override: vi.fn(() => textOverride),
    update_linear_dimension: vi.fn((...args: unknown[]) => {
      updateCalls.push(args)
    }),
  }
  return { scene: scene as unknown as WasmScene, updateCalls }
}

function makeRadialFakeScene() {
  const updateCalls: unknown[][] = []
  const scene = {
    annotation_kind: vi.fn(() => 'radial'),
    annotation_anchor_node_kind: vi.fn(() => -1),
    annotation_anchor_node_id: vi.fn(() => 0n),
    annotation_anchor_point: vi.fn(() => new Float64Array([2, 0, 0])),
    annotation_radial_kind: vi.fn(() => 'radius'),
    annotation_curve: vi.fn(() => new Float64Array([0, 0, 0, 2, 0, 0, 0, 0, 0, 1])),
    annotation_leader_dir: vi.fn(() => new Float64Array([1, 0, 0])),
    annotation_text_override: vi.fn(() => undefined),
    update_radial_dimension: vi.fn((...args: unknown[]) => {
      updateCalls.push(args)
    }),
  }
  return { scene: scene as unknown as WasmScene, updateCalls }
}

function makeLeaderFakeScene() {
  const updateCalls: unknown[][] = []
  const scene = {
    annotation_kind: vi.fn(() => 'leader'),
    annotation_anchor_node_kind: vi.fn(() => -1),
    annotation_anchor_node_id: vi.fn(() => 0n),
    annotation_anchor_point: vi.fn(() => new Float64Array([1, 1, 0])),
    annotation_offset: vi.fn(() => new Float64Array([0.5, 0.5, 0])),
    annotation_text: vi.fn(() => 'Original note'),
    update_leader_text: vi.fn((...args: unknown[]) => {
      updateCalls.push(args)
    }),
  }
  return { scene: scene as unknown as WasmScene, updateCalls }
}

describe('readAnnotation', () => {
  it('reads a linear dimension\'s full anchors/offset/plane/override', () => {
    const { scene } = makeLinearFakeScene('3m even')
    const snap = readAnnotation(scene, 1n)
    expect(snap).not.toBeNull()
    if (snap?.kind !== 'linear') throw new Error('expected linear')
    expect(snap.a).toEqual({ nodeKind: 0, nodeId: 5n, point: [0, 0, 0] })
    expect(snap.b).toEqual({ nodeKind: -1, nodeId: 0n, point: [2, 0, 0] })
    expect(snap.offset).toEqual([0, 1, 0])
    expect(snap.textOverride).toBe('3m even')
  })

  it('reads a radial dimension\'s curve/kind/leader', () => {
    const { scene } = makeRadialFakeScene()
    const snap = readAnnotation(scene, 2n)
    if (snap?.kind !== 'radial') throw new Error('expected radial')
    expect(snap.curveRadius).toBe(2)
    expect(snap.radialKind).toBe('radius')
    expect(snap.leaderDir).toEqual([1, 0, 0])
  })

  it('reads a leader-text annotation\'s content', () => {
    const { scene } = makeLeaderFakeScene()
    const snap = readAnnotation(scene, 3n)
    if (snap?.kind !== 'leader') throw new Error('expected leader')
    expect(snap.text).toBe('Original note')
  })

  it('returns null for a stale/hidden id', () => {
    const scene = { annotation_kind: vi.fn(() => undefined) } as unknown as WasmScene
    expect(readAnnotation(scene, 99n)).toBeNull()
  })
})

describe('initialEditorText', () => {
  it('shows the override for a linear dimension when set', () => {
    const { scene } = makeLinearFakeScene('3m even')
    const snap = readAnnotation(scene, 1n)!
    expect(initialEditorText(snap)).toBe('3m even')
  })

  it('shows empty (not the computed value) when no override is set', () => {
    const { scene } = makeLinearFakeScene(undefined)
    const snap = readAnnotation(scene, 1n)!
    expect(initialEditorText(snap)).toBe('')
  })

  it('shows the real content for a leader', () => {
    const { scene } = makeLeaderFakeScene()
    const snap = readAnnotation(scene, 3n)!
    expect(initialEditorText(snap)).toBe('Original note')
  })
})

describe('commitAnnotationText', () => {
  it('sets a linear dimension\'s text_override, keeping anchors/offset/plane unchanged', () => {
    const { scene, updateCalls } = makeLinearFakeScene(undefined)
    const snap = readAnnotation(scene, 1n)!
    commitAnnotationText(scene, 1n, snap, '3m even')
    expect(updateCalls.length).toBe(1)
    const [id, aKind, aId, aPoint, bKind, bId, bPoint, offset, plane, override] = updateCalls[0] as [
      bigint, number, bigint, Float64Array, number, bigint, Float64Array, Float64Array, Float64Array, string | undefined,
    ]
    expect(id).toBe(1n)
    expect(aKind).toBe(0)
    expect(aId).toBe(5n)
    expect(Array.from(aPoint)).toEqual([0, 0, 0])
    expect(bKind).toBe(-1)
    expect(Array.from(bPoint)).toEqual([2, 0, 0])
    expect(Array.from(offset)).toEqual([0, 1, 0])
    expect(Array.from(plane)).toEqual([0, 0, 0, 0, 0, 1])
    expect(override).toBe('3m even')
  })

  it('an empty commit CLEARS the override (restores the computed value)', () => {
    const { scene, updateCalls } = makeLinearFakeScene('3m even')
    const snap = readAnnotation(scene, 1n)!
    commitAnnotationText(scene, 1n, snap, '')
    const override = updateCalls[0][9]
    expect(override).toBeUndefined()
  })

  it('a whitespace-only commit also clears the override', () => {
    const { scene, updateCalls } = makeLinearFakeScene('3m even')
    const snap = readAnnotation(scene, 1n)!
    commitAnnotationText(scene, 1n, snap, '   ')
    expect(updateCalls[0][9]).toBeUndefined()
  })

  it('sets a radial dimension\'s override via update_radial_dimension', () => {
    const { scene, updateCalls } = makeRadialFakeScene()
    const snap = readAnnotation(scene, 2n)!
    commitAnnotationText(scene, 2n, snap, 'R 2m exactly')
    expect(updateCalls.length).toBe(1)
    const kind = updateCalls[0][4]
    const override = updateCalls[0][9]
    expect(kind).toBe('radius')
    expect(override).toBe('R 2m exactly')
  })

  it('sets a leader\'s text content via update_leader_text', () => {
    const { scene, updateCalls } = makeLeaderFakeScene()
    const snap = readAnnotation(scene, 3n)!
    commitAnnotationText(scene, 3n, snap, 'Edited note')
    expect(updateCalls.length).toBe(1)
    const [id, , , , , text] = updateCalls[0] as [bigint, number, bigint, Float64Array, Float64Array, string]
    expect(id).toBe(3n)
    expect(text).toBe('Edited note')
  })
})

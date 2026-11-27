/**
 * Automatic solid formation seen from the app (wireframe faces §4.2, §5,
 * §8): the draft report's `faces_closed` / `cells` end Line's chain, raise
 * the toasts, and select the new Object.
 */
import { describe, it, expect, vi, afterEach } from 'vitest'
import * as THREE from 'three'
import { LineTool } from './LineTool'
import { RectangleTool } from './RectangleTool'
import { draftMock } from './draftMock'
import { setDraftSolidSelector, cellOutcomeMessage, solidToSelect, type DraftReport } from './draftCommit'
import type { Snap } from './types'
import type { Scene as WasmScene } from '../wasm/loader'
import type { Ray } from '../viewport/math'

const RAY: Ray = { origin: [0, 0, 5], direction: [0, 0, -1] }
const snap = (x: number, y: number, z = 0): Snap => ({ x, y, z, kind: 'ground' })

function scene(extras: (n: number) => Record<string, unknown>): WasmScene {
  const dm = draftMock({ reportExtras: extras })
  return {
    axes: vi.fn(() => new Float64Array([0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1])),
    history_generation: vi.fn(() => 1n),
    pick_face: vi.fn(() => undefined),
    pick_sketch: vi.fn(() => undefined),
    sketch_plane: vi.fn(() => new Float64Array([0, 0, 0, 0, 0, 1])),
    node_parent: vi.fn(() => undefined),
    clear_transient_segments: vi.fn(),
    add_transient_segment: vi.fn(),
    ...dm.members,
  } as unknown as WasmScene
}

function line(s: WasmScene) {
  const onToast = vi.fn()
  const tool = new LineTool(s, new THREE.Group(), vi.fn(), onToast, vi.fn(), vi.fn())
  return { tool, onToast }
}

function rect(s: WasmScene) {
  const onToast = vi.fn()
  const tool = new RectangleTool(s, new THREE.Group(), vi.fn(), onToast, vi.fn(), vi.fn())
  return { tool, onToast }
}

afterEach(() => setDraftSolidSelector(null))

describe('Line — a face closed on another plane ends the chain', () => {
  it('faces_closed on the commit ends the chain: the next click starts a new line, not a segment from the old end', () => {
    const s = scene((n) => (n === 1 ? { faces_closed: [77] } : {}))
    const { tool, onToast } = line(s)
    tool.onPointerDown(snap(0, 0), RAY)
    tool.onPointerDown(snap(1, 0), RAY)
    tool.onPointerDown(snap(1, 1), RAY) // draft 2 reports a closed face
    expect(onToast).not.toHaveBeenCalled()
    tool.onPointerDown(snap(5, 5), RAY) // anchors a fresh chain
    expect(s.draft_add_segment).toHaveBeenCalledTimes(2)
  })

  it('without faces_closed the chain continues (control)', () => {
    const s = scene(() => ({}))
    const { tool } = line(s)
    tool.onPointerDown(snap(0, 0), RAY)
    tool.onPointerDown(snap(1, 0), RAY)
    tool.onPointerDown(snap(1, 1), RAY)
    tool.onPointerDown(snap(5, 5), RAY)
    expect(s.draft_add_segment).toHaveBeenCalledTimes(3)
  })

  it('a created cell on the closing segment ends the chain and toasts', () => {
    const s = scene((n) => (n === 1 ? { cells: [{ created: 12 }] } : {}))
    const { tool, onToast } = line(s)
    tool.onPointerDown(snap(0, 0), RAY)
    tool.onPointerDown(snap(1, 0), RAY)
    tool.onPointerDown(snap(1, 1), RAY)
    tool.onPointerDown(snap(5, 5), RAY)
    expect(s.draft_add_segment).toHaveBeenCalledTimes(2)
    expect(onToast).toHaveBeenCalledWith('The closed shape is now a solid.')
  })
})

describe('cell outcome toasts and selection', () => {
  const cases: Array<[string, Record<string, unknown>, string]> = [
    ['created', { created: 12 }, 'The closed shape is now a solid.'],
    ['grew', { grew: { from: 3, to: 4 } }, 'The closed shape joined the solid it was drawn on.'],
    [
      'unjoined',
      { unjoined: { object: 12, with: 3 } },
      'The closed shape is now a solid of its own. It could not be joined to the one it touches.',
    ],
  ]
  for (const [name, cell, text] of cases) {
    it(`a rectangle commit reporting ${name} toasts "${text}"`, () => {
      const { tool, onToast } = rect(scene(() => ({ cells: [cell] })))
      tool.onPointerDown(snap(0, 0), RAY)
      tool.onPointerDown(snap(1, 2), RAY)
      expect(onToast).toHaveBeenCalledTimes(1)
      expect(onToast).toHaveBeenCalledWith(text)
    })
  }

  it('declined or empty cells raise no toast', () => {
    const { tool, onToast } = rect(scene(() => ({ cells: [{ declined: { reason: 'not_closed', open_edges: 2, non_planar_loops: 0 } }] })))
    tool.onPointerDown(snap(0, 0), RAY)
    tool.onPointerDown(snap(1, 2), RAY)
    expect(onToast).not.toHaveBeenCalled()
  })

  it('created selects the new Object; grew selects the grown result; unjoined selects nothing', () => {
    const select = vi.fn()
    setDraftSolidSelector(select)
    const mk = (cells: unknown[]) => {
      const { tool } = rect(scene(() => ({ cells })))
      tool.onPointerDown(snap(0, 0), RAY)
      tool.onPointerDown(snap(1, 2), RAY)
    }
    mk([{ created: 12 }])
    expect(select).toHaveBeenLastCalledWith(12n)
    mk([{ grew: { from: 3, to: 4 } }])
    expect(select).toHaveBeenLastCalledWith(4n)
    select.mockClear()
    mk([{ unjoined: { object: 12, with: 3 } }])
    expect(select).not.toHaveBeenCalled()
  })
})

describe('report helpers', () => {
  const base: DraftReport = { objects: [], sketches: [], keptAsSketch: [], facesClosed: [], cells: [] }
  it('prefers the refused join over successes', () => {
    const r: DraftReport = {
      ...base,
      cells: [{ kind: 'created', object: 1n }, { kind: 'unjoined', object: 2n, with: 3n }],
    }
    expect(cellOutcomeMessage(r)).toMatch(/solid of its own/)
    expect(solidToSelect(r)).toBe(1n)
  })
  it('is silent with no cells', () => {
    expect(cellOutcomeMessage(base)).toBeNull()
    expect(solidToSelect(base)).toBeNull()
  })
})

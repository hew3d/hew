import { describe, it, expect } from 'vitest'
import { DRAG_MOVE_THRESHOLD_PX, exceedsDragThreshold, dragMoveGuides, dragMoveTargets } from './dragMove'
import type { NodeRef } from '../panels/treeModel'
import { ctxGround, draftRef } from '../tools/planeUnits'

const obj = (id: bigint): NodeRef => ({ kind: 'object', id })
const group = (id: bigint): NodeRef => ({ kind: 'group', id })
const CTX = ctxGround()
const hex = (n: bigint): string => n.toString(16).padStart(16, '0')
const island = (id: bigint, ctx: string = CTX): NodeRef => draftRef('draft-island', ctx, hex(id))
const OTHER_CTX = JSON.stringify({ plane: [0, 0, 1, 0, 0, 1] })

describe('exceedsDragThreshold (click ≠ drag)', () => {
  it('a sub-threshold wiggle is still a click', () => {
    expect(exceedsDragThreshold(100, 100, 100, 100)).toBe(false)
    expect(exceedsDragThreshold(100, 100, 103, 102)).toBe(false)
  })

  it('crossing the threshold in any direction is a drag', () => {
    expect(exceedsDragThreshold(100, 100, 100 + DRAG_MOVE_THRESHOLD_PX, 100)).toBe(true)
    expect(exceedsDragThreshold(100, 100, 100, 100 - DRAG_MOVE_THRESHOLD_PX)).toBe(true)
    // Diagonal distance counts, not per-axis deltas.
    expect(exceedsDragThreshold(0, 0, 4, 4)).toBe(true)
  })

  it('matches the marquee threshold so "click" means one thing everywhere', () => {
    expect(DRAG_MOVE_THRESHOLD_PX).toBe(5)
  })
})

describe('dragMoveTargets (what a drag moves)', () => {
  it('dragging an unselected node moves just that node', () => {
    expect(dragMoveTargets(obj(1n), [])).toEqual([obj(1n)])
    expect(dragMoveTargets(obj(1n), [obj(2n), group(3n)])).toEqual([obj(1n)])
  })

  it('dragging a member of the current selection moves the WHOLE selection (OS convention)', () => {
    const selection = [obj(1n), group(3n), island(4n)]
    expect(dragMoveTargets(obj(1n), selection)).toEqual(selection)
    expect(dragMoveTargets(group(3n), selection)).toEqual(selection)
  })

  it('drawn sub-entities compare by plane context AND key (nodeKey), not key alone', () => {
    const selection = [island(4n)]
    // Same unit key under a DIFFERENT plane context is a different node.
    expect(dragMoveTargets(island(4n, OTHER_CTX), selection)).toEqual([island(4n, OTHER_CTX)])
    expect(dragMoveTargets(island(4n), selection)).toEqual(selection)
  })

  it('returns a copy, never the live selection array', () => {
    const selection = [obj(1n), obj(2n)]
    const out = dragMoveTargets(obj(1n), selection)
    expect(out).not.toBe(selection)
    expect(out).toEqual(selection)
  })
})

describe('dragMoveGuides (the guides a drag carries)', () => {
  it('a drag on a selected node takes the whole selection, its carried guides included', () => {
    expect(dragMoveGuides(obj(1n), [obj(1n), group(3n)], [7n, 8n])).toEqual([7n, 8n])
  })

  it('a drag on an unselected node moves that node alone, no guide', () => {
    expect(dragMoveGuides(obj(2n), [obj(1n)], [7n])).toEqual([])
    expect(dragMoveGuides(obj(2n), [], [7n])).toEqual([])
  })
})

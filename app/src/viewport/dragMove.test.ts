import { describe, it, expect } from 'vitest'
import { DRAG_MOVE_THRESHOLD_PX, exceedsDragThreshold, dragMoveTargets } from './dragMove'
import type { NodeRef } from '../panels/treeModel'

const obj = (id: bigint): NodeRef => ({ kind: 'object', id })
const group = (id: bigint): NodeRef => ({ kind: 'group', id })
const island = (id: bigint, sketch: bigint): NodeRef => ({ kind: 'sketch-island', id, sketch })

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
    const selection = [obj(1n), group(3n), island(4n, 9n)]
    expect(dragMoveTargets(obj(1n), selection)).toEqual(selection)
    expect(dragMoveTargets(group(3n), selection)).toEqual(selection)
  })

  it('sketch sub-entities compare by sketch AND id (nodeKey), not id alone', () => {
    const selection = [island(4n, 9n)]
    // Same island id under a DIFFERENT sketch is a different node.
    expect(dragMoveTargets(island(4n, 8n), selection)).toEqual([island(4n, 8n)])
    expect(dragMoveTargets(island(4n, 9n), selection)).toEqual(selection)
  })

  it('returns a copy, never the live selection array', () => {
    const selection = [obj(1n), obj(2n)]
    const out = dragMoveTargets(obj(1n), selection)
    expect(out).not.toBe(selection)
    expect(out).toEqual(selection)
  })
})

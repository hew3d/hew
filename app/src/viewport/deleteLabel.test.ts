import { describe, expect, it } from 'vitest'
import { deleteStepLabel } from './deleteLabel'
import type { SelItem } from '../panels/treeModel'

const obj = (id: bigint): SelItem => ({ kind: 'object', id })
const grp = (id: bigint): SelItem => ({ kind: 'group', id })
const inst = (id: bigint): SelItem => ({ kind: 'instance', id })
const guide = (id: bigint): SelItem => ({ kind: 'guide', id })
const dim = (id: bigint): SelItem => ({ kind: 'annotation', id })
const edge: SelItem = { kind: 'draft-edge', id: 1n, ctx: '{"plane":[0,0,0,0,0,1]}', key: '0000000000000001' }

describe('deleteStepLabel', () => {
  it('counts one kind by its own noun', () => {
    expect(deleteStepLabel([obj(1n), obj(2n)])).toBe('Delete 2 objects')
    expect(deleteStepLabel([grp(1n), grp(2n), grp(3n)])).toBe('Delete 3 groups')
    expect(deleteStepLabel([inst(1n), inst(2n)])).toBe('Delete 2 instances')
    expect(deleteStepLabel([guide(1n), guide(2n)])).toBe('Delete 2 guides')
    expect(deleteStepLabel([dim(1n), dim(2n)])).toBe('Delete 2 annotations')
  })

  it('counts drawn units and sketches as sketches', () => {
    expect(deleteStepLabel([edge, { kind: 'sketch', id: 4n }])).toBe('Delete 2 sketches')
  })

  it('counts a mix of kinds as items', () => {
    expect(deleteStepLabel([obj(1n), guide(2n), dim(3n)])).toBe('Delete 3 items')
    expect(deleteStepLabel([obj(1n), grp(2n)])).toBe('Delete 2 items')
  })

  it('keeps the plain name for a single entry', () => {
    expect(deleteStepLabel([edge])).toBe('Delete')
  })
})

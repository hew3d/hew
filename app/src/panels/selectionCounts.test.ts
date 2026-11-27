import { describe, expect, it } from 'vitest'
import { selectionCountLine, selNoun } from './selectionCounts'
import type { SelItem } from './treeModel'

const edge: SelItem = { kind: 'draft-edge', id: 1n, ctx: '{"plane":[0,0,0,0,0,1]}', key: '0000000000000001' }

describe('selectionCountLine', () => {
  it('counts each kind once, in a fixed order whatever the pick order', () => {
    const sel: SelItem[] = [
      { kind: 'annotation', id: 9n },
      { kind: 'guide', id: 7n },
      { kind: 'object', id: 1n },
      { kind: 'guide', id: 8n },
      { kind: 'group', id: 2n },
    ]
    expect(selectionCountLine(sel)).toBe('1 object · 1 group · 2 guides · 1 annotation')
  })

  it('counts drawn units and whole sketches together as sketches', () => {
    expect(selNoun(edge)).toBe('sketch')
    expect(selectionCountLine([edge, { kind: 'sketch', id: 3n }, { kind: 'instance', id: 4n }])).toBe('1 instance · 2 sketches')
  })
})

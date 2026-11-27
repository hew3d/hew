import { describe, expect, it } from 'vitest'
import { uiContextJson, uiIsolationJson, uiSelectionJson } from './viewMirror'

describe('uiSelectionJson', () => {
  it('mirrors nodes and sketches in pick order, ids as decimal strings', () => {
    expect(
      JSON.parse(
        uiSelectionJson(
          [
            { kind: 'instance', id: 9007199254740993n },
            { kind: 'object', id: 4n },
            { kind: 'sketch', id: 5n },
          ],
        ),
      ),
    ).toEqual([
      { kind: 'instance', id: '9007199254740993' },
      { kind: 'object', id: '4' },
      { kind: 'sketch', id: '5' },
    ])
  })

  it('leaves the drafting kinds out', () => {
    expect(
      uiSelectionJson(
        [{ kind: 'draft-edge', id: 1n, ctx: '{"plane":[0,0,0,0,0,1]}', key: '0000000000000001' }],
      ),
    ).toBe('[]')
  })

  it('mirrors a mixed selection in pick order, the primary first', () => {
    // A guide picked first stays the primary ahead of the box picked after it.
    expect(
      JSON.parse(
        uiSelectionJson([
          { kind: 'guide', id: 12n },
          { kind: 'object', id: 4n },
          { kind: 'annotation', id: 13n },
          { kind: 'guide', id: 14n },
        ]),
      ),
    ).toEqual([
      { kind: 'guide', id: '12' },
      { kind: 'object', id: '4' },
      { kind: 'annotation', id: '13' },
      { kind: 'guide', id: '14' },
    ])
  })
})

describe('uiIsolationJson', () => {
  it('is null when nothing is isolated', () => {
    expect(uiIsolationJson(null)).toBe('null')
  })

  it('mirrors nodes and members', () => {
    expect(
      JSON.parse(
        uiIsolationJson([
          { kind: 'group', id: 3n },
          { kind: 'member', object: 8n, instance: 9n },
        ]),
      ),
    ).toEqual([
      { kind: 'group', id: '3' },
      { kind: 'member', object: '8', instance: '9' },
    ])
  })
})

describe('uiContextJson', () => {
  it('mirrors object and instance entries outermost first, ids as decimal strings', () => {
    expect(
      JSON.parse(
        uiContextJson([
          { kind: 'instance', id: 9007199254740993n },
          { kind: 'object', id: 4n },
        ]),
      ),
    ).toEqual([
      { kind: 'instance', id: '9007199254740993' },
      { kind: 'object', id: '4' },
    ])
  })

  it('leaves out kinds the fallback cannot hold, and is empty at the top level', () => {
    expect(uiContextJson([{ kind: 'group', id: 2n }])).toBe('[]')
    expect(uiContextJson([])).toBe('[]')
  })
})

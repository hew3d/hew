import { describe, expect, it } from 'vitest'
import { uiIsolationJson, uiSelectionJson } from './viewMirror'

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
          null,
          null,
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
        null,
        null,
      ),
    ).toBe('[]')
  })

  it('mirrors a guide or an annotation', () => {
    expect(JSON.parse(uiSelectionJson([], 12n, null))).toEqual([{ kind: 'guide', id: '12' }])
    expect(JSON.parse(uiSelectionJson([], null, 13n))).toEqual([{ kind: 'annotation', id: '13' }])
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

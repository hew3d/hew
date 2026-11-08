import { describe, expect, it } from 'vitest'
import { mapSelection, parseLinkedResult } from './linkedWraps'

const JSON_RESULT = JSON.stringify({
  nodes: [{ kind: 'instance', id: '18446744073709551615' }],
  linked: [
    {
      source: { kind: 'object', id: '4294967297' },
      instance: '8589934593',
      definition: '12884901889',
    },
  ],
})

describe('linkedWraps', () => {
  it('parseLinkedResult_parses_decimal_string_handles', () => {
    const r = parseLinkedResult(JSON_RESULT)
    expect(r.nodes).toEqual([{ kind: 'instance', id: 18446744073709551615n }])
    expect(r.linked).toEqual([
      { source: { kind: 'object', id: 4294967297n }, instance: 8589934593n, definition: 12884901889n },
    ])
  })

  it('mapSelection_replaces_a_wrapped_source_with_its_instance', () => {
    const { linked } = parseLinkedResult(JSON_RESULT)
    const sel = [
      { kind: 'object' as const, id: 4294967297n },
      { kind: 'object' as const, id: 7n },
      { kind: 'group' as const, id: 4294967297n },
    ]
    expect(mapSelection(sel, linked)).toEqual([
      { kind: 'instance', id: 8589934593n },
      { kind: 'object', id: 7n },
      { kind: 'group', id: 4294967297n },
    ])
    expect(mapSelection(sel, [])).toEqual(sel)
  })
})

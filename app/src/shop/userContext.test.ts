import { describe, expect, it } from 'vitest'
import { shopEnterChain, shopSetPathDepth } from './userContext'
import { uiContextJson } from '../api/viewMirror'

/** `node_parent` over a tiny tree: instance 5 sits in group 2, which sits in group 1. */
const scene = {
  node_parent: (kind: number, id: bigint): bigint | undefined => {
    if (kind === 2 && id === 5n) return 2n
    if (kind === 1 && id === 2n) return 1n
    return undefined
  },
}

describe('Shop Mode user context', () => {
  it('an entry pushes the ancestor chain outermost first, then the target', () => {
    expect(shopEnterChain(scene, { kind: 'instance', id: 5n })).toEqual([
      { kind: 'group', id: 1n },
      { kind: 'group', id: 2n },
      { kind: 'instance', id: 5n },
    ])
  })

  it('a top-level target is the whole chain, and a sketch-scoped one enters nothing', () => {
    expect(shopEnterChain(scene, { kind: 'object', id: 9n })).toEqual([{ kind: 'object', id: 9n }])
    expect(shopEnterChain(scene, { kind: 'sketch', id: 3n })).toEqual([])
  })

  it('truncation keeps the first depth entries and never goes below none', () => {
    const chain = shopEnterChain(scene, { kind: 'instance', id: 5n })
    expect(shopSetPathDepth(chain, 1)).toEqual([{ kind: 'group', id: 1n }])
    expect(shopSetPathDepth(chain, 0)).toEqual([])
    expect(shopSetPathDepth(chain, -1)).toEqual([])
    expect(shopSetPathDepth(chain, 9)).toEqual(chain)
  })

  it('the Scene hears only the entries a fallback can hold', () => {
    const chain = shopEnterChain(scene, { kind: 'instance', id: 5n })
    expect(JSON.parse(uiContextJson(chain))).toEqual([{ kind: 'instance', id: '5' }])
  })
})

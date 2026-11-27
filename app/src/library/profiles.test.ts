import { describe, expect, it } from 'vitest'
import { chainFirstFix, orderChain, type ChainSegment } from './profiles'

const seg = (a: [number, number, number], b: [number, number, number]): ChainSegment => ({ a, b, curve: null })

describe('orderChain', () => {
  it('puts loose segments end to end from the free end of the first drawn one', () => {
    const chain = orderChain([seg([1, 0, 0], [2, 0, 0]), seg([0, 0, 0], [1, 0, 0]), seg([2, 0, 0], [2, 1, 0])])
    expect(chain).not.toBeNull()
    // The first listed segment's free end (its `a`, at x = 1, touches the
    // second) is not free, so the chain starts from the segment whose end
    // touches nothing: (0,0,0).
    const points = [chain![0].a, ...chain!.map((s) => s.b)]
    expect(points).toEqual([
      [0, 0, 0],
      [1, 0, 0],
      [2, 0, 0],
      [2, 1, 0],
    ])
  })

  it('flips segments drawn backwards and keeps their circles', () => {
    const curve = { center: [0, 0, 0] as [number, number, number], radius: 1 }
    const chain = orderChain([seg([0, 0, 0], [1, 0, 0]), { a: [2, 0, 0], b: [1, 0, 0], curve }])
    expect(chain![1].a).toEqual([1, 0, 0])
    expect(chain![1].b).toEqual([2, 0, 0])
    expect(chain![1].curve).toBe(curve)
  })

  it('refuses a closed loop, a branch, a gap and nothing', () => {
    expect(orderChain([])).toBeNull()
    expect(orderChain([seg([0, 0, 0], [1, 0, 0]), seg([1, 0, 0], [1, 1, 0]), seg([1, 1, 0], [0, 0, 0])])).toBeNull()
    expect(orderChain([seg([0, 0, 0], [1, 0, 0]), seg([1, 0, 0], [2, 0, 0]), seg([1, 0, 0], [1, 1, 0])])).toBeNull()
    expect(orderChain([seg([0, 0, 0], [1, 0, 0]), seg([3, 0, 0], [4, 0, 0])])).toBeNull()
  })
})

describe('chainFirstFix', () => {
  it('says nothing for one open chain and names what is wrong otherwise', () => {
    expect(chainFirstFix([seg([0, 0, 0], [1, 0, 0]), seg([1, 0, 0], [2, 1, 0])])).toBeNull()
    const rect = [
      seg([0, 0, 0], [2, 0, 0]),
      seg([2, 0, 0], [2, 1, 0]),
      seg([2, 1, 0], [0, 1, 0]),
      seg([0, 1, 0], [0, 0, 0]),
    ]
    expect(chainFirstFix(rect)).toMatch(/closed/)
    expect(chainFirstFix([seg([0, 0, 0], [1, 0, 0]), seg([1, 0, 0], [2, 0, 0]), seg([1, 0, 0], [1, 1, 0])])).toMatch(/branches/)
    expect(chainFirstFix([seg([0, 0, 0], [1, 0, 0]), seg([3, 0, 0], [4, 0, 0])])).toMatch(/joined/)
  })
})

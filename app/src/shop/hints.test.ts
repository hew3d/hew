import { describe, expect, it } from 'vitest'
import {
  HintEngine,
  HINT_STORAGE_KEYS,
  HINT_TEST_SUPPRESS_KEY,
  NO_ORBIT_HINT_DELAY_MS,
  ORBIT_HINT_PLAY_MS,
  largestVisiblePart,
  testHintsSuppressed,
} from './hints'
import type { PartsSheetSection } from './partsSheetModel'

/** A tiny in-memory `Storage` stub — every test gets a fresh one, so no
 *  hint flag ever leaks between tests the way real `localStorage` would
 *  without a `beforeEach` clear. */
function makeStorage(): Storage {
  const map = new Map<string, string>()
  return {
    getItem: (key: string) => map.get(key) ?? null,
    setItem: (key: string, value: string) => { map.set(key, value) },
    removeItem: (key: string) => { map.delete(key) },
    clear: () => map.clear(),
    key: () => null,
    get length() { return map.size },
  }
}

/** A steppable fake clock: `advance(ms)` moves it forward, `now()` reads
 *  the current value — the "injected clock" `HintEngine`'s module doc
 *  promises every timing rule is testable through. */
function makeClock(start = 0) {
  let t = start
  return { now: () => t, advance: (ms: number) => { t += ms } }
}

describe('HintEngine — hint (a) tap: first model open', () => {
  it('fires immediately on documentOpened when a target exists, and writes the flag right away', () => {
    const storage = makeStorage()
    const engine = new HintEngine({ storage })
    engine.documentOpened('obj:1')
    expect(engine.getActive()).toEqual({ name: 'tap', targetPartId: 'obj:1' })
    // "fires once" = the DISPLAY event, not the dismissal — the flag is
    // already written the moment it starts showing (module doc).
    expect(storage.getItem(HINT_STORAGE_KEYS.tap)).toBe('1')
  })

  it('does not fire with no target (empty/meshless document)', () => {
    const engine = new HintEngine({ storage: makeStorage() })
    engine.documentOpened(null)
    expect(engine.getActive()).toBeNull()
  })

  it('tapped() kills it instantly and satisfies the flag', () => {
    const storage = makeStorage()
    const engine = new HintEngine({ storage })
    engine.documentOpened('obj:1')
    engine.tapped()
    expect(engine.getActive()).toBeNull()
    expect(storage.getItem(HINT_STORAGE_KEYS.tap)).toBe('1')
  })

  it('never fires again in a later session once the flag is set — a fresh instance over the SAME storage stays satisfied', () => {
    const storage = makeStorage()
    const first = new HintEngine({ storage })
    first.documentOpened('obj:1') // fires, writes the flag
    const second = new HintEngine({ storage }) // simulates a reload
    second.documentOpened('obj:2') // a different document, still eligible by target
    expect(second.getActive()).toBeNull()
  })

  it('tapping BEFORE the hint ever had a document to fire on permanently satisfies it', () => {
    const storage = makeStorage()
    const engine = new HintEngine({ storage })
    engine.tapped() // no documentOpened yet at all
    engine.documentOpened('obj:1')
    expect(engine.getActive()).toBeNull()
    expect(storage.getItem(HINT_STORAGE_KEYS.tap)).toBe('1')
  })

  it('opening a second document after the first already showed the hint does not refire it — "fires once" already spent', () => {
    const engine = new HintEngine({ storage: makeStorage() })
    engine.documentOpened('obj:1')
    expect(engine.getActive()).toEqual({ name: 'tap', targetPartId: 'obj:1' })
    // The flag was written the moment (a) fired for doc 1 (module doc:
    // "fires once" = the display event) — doc 2 opening is a fresh
    // `documentOpened`, but the hint itself has already spent its one shot
    // for this install, tapped or not.
    engine.documentOpened('obj:2')
    expect(engine.getActive()).toBeNull()
  })
})

describe('HintEngine — hint (b) orbit: no orbit within 8s', () => {
  it('does not fire before 8s, fires at/after 8s via tick(), and auto-dismisses after its play window', () => {
    const clock = makeClock()
    const storage = makeStorage()
    const engine = new HintEngine({ storage, now: clock.now })
    engine.documentOpened(null) // no target — hint (a) never blocks this test

    clock.advance(NO_ORBIT_HINT_DELAY_MS - 1)
    engine.tick()
    expect(engine.getActive()).toBeNull()

    clock.advance(1)
    engine.tick()
    expect(engine.getActive()).toEqual({ name: 'orbit' })
    expect(storage.getItem(HINT_STORAGE_KEYS.orbit)).toBe('1')

    clock.advance(ORBIT_HINT_PLAY_MS - 1)
    engine.tick()
    expect(engine.getActive()).toEqual({ name: 'orbit' }) // still mid-play

    clock.advance(1)
    engine.tick()
    expect(engine.getActive()).toBeNull() // one-shot play-through ended on its own
  })

  it('orbiting mid-play kills it instantly, same as any other gesture-kill', () => {
    const clock = makeClock()
    const engine = new HintEngine({ storage: makeStorage(), now: clock.now })
    engine.documentOpened(null)
    clock.advance(NO_ORBIT_HINT_DELAY_MS)
    engine.tick()
    expect(engine.getActive()).toEqual({ name: 'orbit' })

    clock.advance(ORBIT_HINT_PLAY_MS / 2)
    engine.orbited()
    expect(engine.getActive()).toBeNull()
  })

  it('orbiting BEFORE the 8s window elapses pre-satisfies it — it never fires later', () => {
    const clock = makeClock()
    const storage = makeStorage()
    const engine = new HintEngine({ storage, now: clock.now })
    engine.documentOpened(null)
    clock.advance(1000)
    engine.orbited()
    expect(storage.getItem(HINT_STORAGE_KEYS.orbit)).toBe('1')

    clock.advance(NO_ORBIT_HINT_DELAY_MS) // well past the original deadline
    engine.tick()
    expect(engine.getActive()).toBeNull()
  })

  it('is blocked from firing while hint (a) is still showing (one hint at a time) — fires once (a) clears', () => {
    const clock = makeClock()
    const engine = new HintEngine({ storage: makeStorage(), now: clock.now })
    engine.documentOpened('obj:1') // fires hint (a) immediately
    clock.advance(NO_ORBIT_HINT_DELAY_MS + 100)
    engine.tick()
    expect(engine.getActive()).toEqual({ name: 'tap', targetPartId: 'obj:1' }) // (a) still holds the slot

    // tapped() both kills (a) AND re-evaluates in the same call (the 8s
    // condition was true all along, just blocked) — (b) takes the slot
    // immediately, with no extra tick() needed.
    engine.tapped()
    expect(engine.getActive()).toEqual({ name: 'orbit' })
  })
})

describe('HintEngine — only one hint on screen at a time (cross-hint)', () => {
  it('hint (b) never interrupts an active hint (a)', () => {
    const clock = makeClock()
    const engine = new HintEngine({ storage: makeStorage(), now: clock.now })
    engine.documentOpened('obj:1')
    clock.advance(NO_ORBIT_HINT_DELAY_MS + 1)
    engine.tick()
    // Hint (a) still owns the slot even though (b)'s own condition is
    // independently true by now.
    expect(engine.getActive()).toMatchObject({ name: 'tap' })
  })
})

describe('largestVisiblePart', () => {
  function row(id: bigint, extents: [number, number, number] | null, hidden = false) {
    return { node: { kind: 'object' as const, id }, label: `Object ${id}`, depth: 0, extentsM: extents, hidden }
  }

  it('picks the row with the largest AABB volume across all sections', () => {
    const sections: PartsSheetSection[] = [
      { key: 's1', label: 'S1', path: null, hidden: false, rows: [row(1n, [1, 1, 1]), row(2n, [2, 2, 2])] },
      { key: 's2', label: 'S2', path: null, hidden: false, rows: [row(3n, [0.5, 0.5, 0.5])] },
    ]
    expect(largestVisiblePart(sections)).toEqual({ kind: 'object', id: 2n })
  })

  it('skips hidden rows and rows with no resolvable extents', () => {
    const sections: PartsSheetSection[] = [
      { key: 's1', label: 'S1', path: null, hidden: false, rows: [row(1n, [10, 10, 10], true), row(2n, null), row(3n, [1, 1, 1])] },
    ]
    expect(largestVisiblePart(sections)).toEqual({ kind: 'object', id: 3n })
  })

  it('returns null when nothing is visible/resolvable', () => {
    const sections: PartsSheetSection[] = [
      { key: 's1', label: 'S1', path: null, hidden: false, rows: [row(1n, [1, 1, 1], true), row(2n, null)] },
    ]
    expect(largestVisiblePart(sections)).toBeNull()
  })

  it('returns null for no sections at all', () => {
    expect(largestVisiblePart([])).toBeNull()
  })
})

describe('testHintsSuppressed', () => {
  it('is false with no storage entry', () => {
    expect(testHintsSuppressed(makeStorage())).toBe(false)
  })

  it('is true only for the exact "1" value', () => {
    const storage = makeStorage()
    storage.setItem(HINT_TEST_SUPPRESS_KEY, '1')
    expect(testHintsSuppressed(storage)).toBe(true)
    storage.setItem(HINT_TEST_SUPPRESS_KEY, 'true')
    expect(testHintsSuppressed(storage)).toBe(false)
  })

  it('is false with no storage at all (off-DOM)', () => {
    expect(testHintsSuppressed(undefined)).toBe(false)
  })
})

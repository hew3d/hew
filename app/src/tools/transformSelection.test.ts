/**
 * transformSelection — sketch-selection routing for the transform tools.
 *
 * Pins the fixes for the "cannot rotate any shape up 90 degrees" defect
 * class: a selected edge/curve transforms its ISLAND (never a silent
 * no-op), islands covering a whole sketch fold into one handle-stable
 * whole-sketch bake, and a strict subset routes through the per-island
 * kernel op with an all-first validation pass.
 */
import { describe, it, expect, vi } from 'vitest'
import {
  commitSelectionTransform,
  duplicateSketchSelection,
  planSketchTransforms,
  resolveSketchIsland,
} from './transformSelection'
import type { Scene as WasmScene } from '../wasm/loader'
import type { NodeRef } from '../panels/treeModel'

const AFFINE = new Float64Array([1, 0, 0, 5, 0, 1, 0, 0, 0, 0, 1, 0])

interface FakeOpts {
  /** island ids per sketch handle (decimal string keys) */
  islands?: Record<string, bigint[]>
  /** edge id -> island id */
  edgeIslands?: Record<string, bigint>
  canTransformIsland?: boolean
}

function makeScene(opts: FakeOpts = {}) {
  const calls: { name: string; args: unknown[] }[] = []
  const scene = {
    sketch_island_ids: vi.fn((sketch: bigint) => opts.islands?.[String(sketch)] ?? []),
    sketch_edge_island: vi.fn(
      (_sketch: bigint, edge: bigint) => opts.edgeIslands?.[String(edge)],
    ),
    can_transform_sketch_island: vi.fn(() => opts.canTransformIsland ?? true),
    transform_sketch_island: vi.fn((...args: unknown[]) => {
      calls.push({ name: 'transform_sketch_island', args })
    }),
    transform_selection: vi.fn((...args: unknown[]) => {
      calls.push({ name: 'transform_selection', args })
    }),
  }
  return { scene: scene as unknown as WasmScene, raw: scene, calls }
}

describe('resolveSketchIsland', () => {
  it('maps an edge and a curve to their owning island', () => {
    const { scene } = makeScene({ edgeIslands: { '7': 40n } })
    const edge: NodeRef = { kind: 'sketch-edge', id: 7n, sketch: 3n }
    const curve: NodeRef = { kind: 'sketch-curve', id: 7n, sketch: 3n }
    expect(resolveSketchIsland(scene, edge)).toEqual({ sketch: 3n, island: 40n })
    expect(resolveSketchIsland(scene, curve)).toEqual({ sketch: 3n, island: 40n })
  })

  it('returns null for a stale edge (pruned selection)', () => {
    const { scene } = makeScene()
    expect(
      resolveSketchIsland(scene, { kind: 'sketch-edge', id: 9n, sketch: 3n }),
    ).toBeNull()
  })
})

describe('planSketchTransforms', () => {
  it('folds islands covering the whole sketch into a whole-sketch bake', () => {
    const { scene } = makeScene({ islands: { '3': [40n] } })
    const plan = planSketchTransforms(scene, [
      { kind: 'sketch-island', id: 40n, sketch: 3n },
    ])
    expect(plan.sketches).toEqual([3n])
    expect(plan.islands).toEqual([])
  })

  it('keeps a strict subset of islands at island granularity', () => {
    const { scene } = makeScene({ islands: { '3': [40n, 41n] } })
    const plan = planSketchTransforms(scene, [
      { kind: 'sketch-island', id: 40n, sketch: 3n },
    ])
    expect(plan.sketches).toEqual([])
    expect(plan.islands).toEqual([{ sketch: 3n, island: 40n }])
  })

  it('dedupes an edge selected alongside its own island', () => {
    const { scene } = makeScene({
      islands: { '3': [40n, 41n] },
      edgeIslands: { '7': 40n },
    })
    const plan = planSketchTransforms(scene, [
      { kind: 'sketch-island', id: 40n, sketch: 3n },
      { kind: 'sketch-edge', id: 7n, sketch: 3n },
    ])
    expect(plan.islands).toEqual([{ sketch: 3n, island: 40n }])
  })

  it('a whole-sketch selection absorbs its own islands', () => {
    const { scene } = makeScene({ islands: { '3': [40n, 41n] } })
    const plan = planSketchTransforms(scene, [
      { kind: 'sketch', id: 3n },
      { kind: 'sketch-island', id: 40n, sketch: 3n },
    ])
    expect(plan.sketches).toEqual([3n])
    expect(plan.islands).toEqual([])
  })
})

describe('commitSelectionTransform', () => {
  it('transforms the island of a selected open-chain edge (no silent no-op)', () => {
    const { scene, raw } = makeScene({
      islands: { '3': [40n, 41n] },
      edgeIslands: { '7': 40n },
    })
    commitSelectionTransform(scene, [{ kind: 'sketch-edge', id: 7n, sketch: 3n }], AFFINE)
    expect(raw.transform_sketch_island).toHaveBeenCalledWith(3n, 40n, AFFINE)
    expect(raw.transform_selection).not.toHaveBeenCalled()
  })

  it('routes a sole-island selection through the whole-sketch path', () => {
    const { scene, raw } = makeScene({ islands: { '3': [40n] } })
    commitSelectionTransform(
      scene,
      [{ kind: 'sketch-island', id: 40n, sketch: 3n }],
      AFFINE,
    )
    expect(raw.transform_sketch_island).not.toHaveBeenCalled()
    expect(raw.transform_selection).toHaveBeenCalledTimes(1)
    const sketches = raw.transform_selection.mock.calls[0][2] as BigUint64Array
    expect([...sketches]).toEqual([3n])
  })

  it('validates every subset island before committing any', () => {
    const { scene, raw } = makeScene({
      islands: { '3': [40n, 41n, 42n] },
      canTransformIsland: false,
    })
    expect(() =>
      commitSelectionTransform(
        scene,
        [
          { kind: 'sketch-island', id: 40n, sketch: 3n },
          { kind: 'sketch-island', id: 41n, sketch: 3n },
        ],
        AFFINE,
      ),
    ).toThrow(/WouldRetopologize/)
    expect(raw.transform_sketch_island).not.toHaveBeenCalled()
    expect(raw.transform_selection).not.toHaveBeenCalled()
  })

  it('commits objects and sketch geometry together', () => {
    const { scene, raw } = makeScene({ islands: { '3': [40n, 41n] } })
    commitSelectionTransform(
      scene,
      [
        { kind: 'object', id: 9n },
        { kind: 'sketch-island', id: 40n, sketch: 3n },
      ],
      AFFINE,
    )
    expect(raw.transform_sketch_island).toHaveBeenCalledWith(3n, 40n, AFFINE)
    expect(raw.transform_selection).toHaveBeenCalledTimes(1)
    const ids = raw.transform_selection.mock.calls[0][1] as BigUint64Array
    expect([...ids]).toEqual([9n])
  })

  it('skips a stale edge like any pruned handle instead of throwing', () => {
    const { scene, raw } = makeScene({ islands: { '3': [40n] } })
    commitSelectionTransform(scene, [{ kind: 'sketch-edge', id: 99n, sketch: 3n }], AFFINE)
    expect(raw.transform_sketch_island).not.toHaveBeenCalled()
    expect(raw.transform_selection).not.toHaveBeenCalled()
  })
})

// ---------------------------------------------------------------------------
// duplicateSketchSelection — Move+Alt's sketch copy path (playtest: "you can
// Move a Sketch but you can't Copy a Sketch"). The copy is a translated
// replay through the ordinary drawing surface: one gesture per sketch (one
// undo step), curve chains re-bracketed with their translated analytic
// definition so a copied circle stays a true circle.
//
// The fake below models the kernel's REAL gesture semantics rather than just
// logging calls, because the interesting failures here are all about WHAT THE
// DOCUMENT IS LEFT HOLDING when a replay throws partway.
// ---------------------------------------------------------------------------

interface ReplayFake {
  scene: WasmScene
  raw: Record<string, ReturnType<typeof vi.fn>>
  log: string[]
  /** The live sketch content, serialized — the fake's byte-identity oracle. */
  content(): string
  /** How many steps the undo stack holds. */
  undoDepth(): number
  /** Push an unrelated recorded action (something the user did BEFORE the
   * copy) onto the undo stack, so a stray retracting undo is detectable. */
  pushUnrelatedAction(): void
}

/**
 * A fake Scene modelling the kernel's ACTUAL sketch-gesture semantics — the
 * contract `duplicateSketchSelection` has to be correct against. Every clause
 * here is checked against `crates/kernel/src/document.rs`:
 *
 * - `sketch_add_segment` mutates the live sketch IMMEDIATELY: the kernel
 *   reaches it through `Document::sketch_mut` and each add is its own
 *   committed clone-validate-swap, never routed through the undo log. So a
 *   LATER add throwing does not un-apply the earlier ones.
 * - The gesture bracket only decides what the UNDO LOG gets:
 *   `end_sketch_gesture` diffs the sketch against the gesture's `before`
 *   snapshot, pushing exactly ONE step if it changed and nothing at all if it
 *   didn't — and closing the bracket either way, including on its Err paths.
 * - `cancel_sketch_gesture` DROPS the snapshot WITHOUT restoring it ("Any
 *   mutations made inside the abandoned bracket stay in the sketch but out of
 *   the undo log; cancel-before-mutate is the caller's contract"). It is a
 *   cancel-BEFORE-mutate primitive, not a rollback.
 * - `history_generation` moves by exactly one on every push and every undo,
 *   and on nothing else.
 *
 * Two sketches, both on the ground plane: 3 (island 40 = a 2-edge curve
 * chain, circle center (1,2,0) r 0.5; island 41 = one plain edge) and 4
 * (island 50 = one plain edge).
 *
 * `failAfterAdds: n` lets the first n `sketch_add_segment` calls land and
 * throws on the next — `n > 0` is the case a first-call-only failure test
 * structurally cannot see.
 */
function makeReplayScene(opts: { failAfterAdds?: number } = {}): ReplayFake {
  const log: string[] = []
  let nextEdge = 500n
  let adds = 0
  // Out-of-plane copies land on fresh sketches; model each with its own id
  // and island list so `sketch_island_ids` can report the copy back.
  let nextCopySketch = 70n
  let nextCopyIsland = 80n
  const copyIslands = new Map<bigint, bigint[]>()

  // Live sketch content, keyed "<sketch>/<edge>". Seeded with the sources.
  let live = new Map<string, number[]>([
    ['3/10', [1.5, 2, 0, 1, 2.5, 0]],
    ['3/11', [1, 2.5, 0, 0.5, 2, 0]],
    ['3/20', [5, 5, 0, 6, 5, 0]],
    ['4/30', [0, 8, 0, 1, 8, 0]],
  ])
  const undoStack: { sketch: bigint; before: Map<string, number[]> }[] = []
  let pending: { sketch: bigint; before: Map<string, number[]> } | null = null
  let generation = 0n

  const ser = (m: Map<string, number[]>): string =>
    [...m.entries()]
      .sort((x, y) => (x[0] < y[0] ? -1 : x[0] > y[0] ? 1 : 0))
      .map(([k, v]) => `${k}=${v.join(',')}`)
      .join('|')
  /** Just one sketch's edges — what a gesture snapshots and diffs. */
  const ofSketch = (s: bigint): Map<string, number[]> =>
    new Map([...live].filter(([k]) => k.startsWith(`${s}/`)))

  const raw = {
    sketch_island_ids: vi.fn((s: bigint) =>
      copyIslands.get(s) ?? (s === 3n ? [40n, 41n] : s === 4n ? [50n] : []),
    ),
    sketch_island_edges: vi.fn((_s: bigint, island: bigint) =>
      island === 40n ? [10n, 11n] : island === 41n ? [20n] : island === 50n ? [30n] : [],
    ),
    sketch_edge_island: vi.fn((s: bigint, edge: bigint) =>
      edge >= 500n
        ? s === 3n
          ? 60n
          : 61n
        : edge === 20n
          ? 41n
          : edge === 30n
            ? 50n
            : 40n,
    ),
    sketch_edge_endpoints: vi.fn((s: bigint, e: bigint) => live.get(`${s}/${e}`)),
    sketch_edge_curve: vi.fn((s: bigint, e: bigint) =>
      s === 3n && (e === 10n || e === 11n) ? 7n : undefined,
    ),
    sketch_curve_geom: vi.fn(() => [1, 2, 0, 0.5]),
    sketch_plane: vi.fn(() => [0, 0, 0, 0, 0, 1]),
    history_generation: vi.fn(() => generation),
    sketch_begin_gesture: vi.fn((s: bigint) => {
      log.push('begin_gesture')
      pending = { sketch: s, before: ofSketch(s) }
    }),
    sketch_end_gesture: vi.fn((s: bigint) => {
      log.push('end_gesture')
      if (pending === null || pending.sketch !== s) throw new Error('SketchGestureNotOpen')
      const { before } = pending
      pending = null // the bracket closes either way
      if (ser(before) === ser(ofSketch(s))) return // unchanged: records nothing
      undoStack.push({ sketch: s, before })
      generation += 1n
    }),
    sketch_cancel_gesture: vi.fn(() => {
      log.push('cancel_gesture')
      pending = null // drops the snapshot; whatever landed STAYS live
    }),
    scene_undo: vi.fn(() => {
      log.push('scene_undo')
      const step = undoStack.pop()
      if (step === undefined) throw new Error('NothingToUndo')
      for (const k of [...live.keys()]) if (k.startsWith(`${step.sketch}/`)) live.delete(k)
      for (const [k, v] of step.before) live.set(k, v)
      generation += 1n
      return { free: () => { /* no-op */ } }
    }),
    sketch_begin_curve: vi.fn(() => { log.push('begin_curve'); return 8n }),
    sketch_begin_curve_with: vi.fn((_s: bigint, cx: number, cy: number, cz: number, r: number) => {
      log.push(`begin_curve_with(${cx},${cy},${cz},${r})`)
      return 8n
    }),
    sketch_end_curve: vi.fn(() => log.push('end_curve')),
    sketch_add_segment: vi.fn((s: bigint, ax: number, ay: number, az: number, bx: number, by: number, bz: number) => {
      if (opts.failAfterAdds !== undefined && adds >= opts.failAfterAdds) {
        throw new Error('WouldRetopologize: nope')
      }
      adds += 1
      log.push(`add(${ax},${ay},${az} -> ${bx},${by},${bz})`)
      const id = nextEdge++
      live.set(`${s}/${id}`, [ax, ay, az, bx, by, bz]) // lands NOW, not at end_gesture
      return { new_edges: () => [id], free: () => { /* no-op */ } }
    }),
    // Out-of-plane copy: the kernel builds ONE NEW sketch holding ALL the
    // given islands' geometry with the affine baked in, source untouched, in
    // ONE atomic undo step. The fake mirrors that — a fresh sketch id, every
    // island's edges added lifted by the affine's Z translation, one recorded
    // step so a retraction can remove it, and nothing changed on the source.
    copy_sketch_islands: vi.fn((s: bigint, islands: BigUint64Array, affine: Float64Array) => {
      const islandList = [...islands]
      log.push(`copy_sketch_islands(${s},[${islandList.join(',')}])`)
      const copySketch = nextCopySketch++
      const before = ofSketch(copySketch) // empty — brand-new sketch
      const dz = affine[11]
      const copyIslandIds: bigint[] = []
      for (const island of islandList) {
        copyIslandIds.push(nextCopyIsland++)
        for (const e of raw.sketch_island_edges(s, island)) {
          const seg = live.get(`${s}/${e}`)
          if (seg === undefined) continue
          live.set(`${copySketch}/${nextEdge++}`, [
            seg[0], seg[1], seg[2] + dz, seg[3], seg[4], seg[5] + dz,
          ])
        }
      }
      copyIslands.set(copySketch, copyIslandIds)
      undoStack.push({ sketch: copySketch, before }) // hide-the-copy undo
      generation += 1n
      return copySketch
    }),
  }
  return {
    scene: raw as unknown as WasmScene,
    raw,
    log,
    content: () => ser(live),
    undoDepth: () => undoStack.length,
    pushUnrelatedAction: () => {
      undoStack.push({ sketch: 9n, before: ofSketch(9n) })
      live.set('9/900', [0, 0, 0, 1, 0, 0])
      generation += 1n
    },
  }
}

describe('duplicateSketchSelection', () => {
  it('replays a subset island translated, inside one gesture, with the curve re-bracketed', () => {
    const { scene, raw, log } = makeReplayScene()
    const copies = duplicateSketchSelection(
      scene,
      [{ kind: 'sketch-island', id: 40n, sketch: 3n }],
      [0.08, 0, 0],
    )
    // One gesture; the curve bracket carries the TRANSLATED center and the
    // SAME radius; both edges replay translated; the plain island 41 (not
    // selected) is untouched.
    expect(log).toEqual([
      'begin_gesture',
      'begin_curve_with(1.08,2,0,0.5)',
      'add(1.58,2,0 -> 1.08,2.5,0)',
      'add(1.08,2.5,0 -> 0.58,2,0)',
      'end_curve',
      'end_gesture',
    ])
    expect(raw.sketch_cancel_gesture).not.toHaveBeenCalled()
    expect(raw.scene_undo).not.toHaveBeenCalled() // a clean copy retracts nothing
    // The copy comes back as the new island the replayed edges landed in.
    expect(copies).toEqual([{ kind: 'sketch-island', id: 60n, sketch: 3n }])
  })

  it('a whole-sketch selection replays every island', () => {
    const { scene, log } = makeReplayScene()
    duplicateSketchSelection(scene, [{ kind: 'sketch', id: 3n }], [0, 1, 0])
    expect(log.filter((l) => l.startsWith('add('))).toHaveLength(3)
    expect(log.filter((l) => l === 'begin_gesture')).toHaveLength(1)
    expect(log.filter((l) => l === 'end_gesture')).toHaveLength(1)
  })

  it('copies an out-of-plane offset onto a NEW sketch via copy_sketch_islands', () => {
    // A ground island copied straight up Z leaves its plane, so it detaches
    // onto its own new sketch (the kernel path) instead of the same-sketch
    // gesture replay — no PointOffPlane refusal any more.
    const { scene, raw } = makeReplayScene()
    const copies = duplicateSketchSelection(
      scene,
      [{ kind: 'sketch-island', id: 40n, sketch: 3n }],
      [0, 0, 0.5],
    )
    // The replay machinery is NOT used; the kernel copy op is.
    expect(raw.sketch_begin_gesture).not.toHaveBeenCalled()
    expect(raw.sketch_add_segment).not.toHaveBeenCalled()
    expect(raw.copy_sketch_islands).toHaveBeenCalledTimes(1)
    const [s, islands, affine] = raw.copy_sketch_islands.mock.calls[0]
    expect(s).toBe(3n)
    expect([...islands]).toEqual([40n])
    expect(affine[11]).toBeCloseTo(0.5) // the Z translation is baked in
    // The copy comes back as the island of the new sketch (id 70/80 here).
    expect(copies).toEqual([{ kind: 'sketch-island', id: 80n, sketch: 70n }])
  })

  it('a whole-sketch out-of-plane copy sends ALL the sketch islands in ONE call', () => {
    // Grouping matters: a region's hole boundary is its own island, so a
    // donut's islands must be copied together (one copy sketch), not one per
    // island. The whole-sketch selection therefore makes ONE copy call
    // carrying every island, and the copy comes back as that new sketch's
    // islands.
    const { scene, raw } = makeReplayScene()
    const copies = duplicateSketchSelection(scene, [{ kind: 'sketch', id: 3n }], [0, 0, 0.5])
    expect(raw.copy_sketch_islands).toHaveBeenCalledTimes(1)
    const [s, islands] = raw.copy_sketch_islands.mock.calls[0]
    expect(s).toBe(3n)
    expect([...islands]).toEqual([40n, 41n]) // both islands, together
    // Two islands on the one new sketch (70), reselected.
    expect(copies).toEqual([
      { kind: 'sketch-island', id: 80n, sketch: 70n },
      { kind: 'sketch-island', id: 81n, sketch: 70n },
    ])
  })

  it('islands on DIFFERENT source sketches each get their own copy sketch', () => {
    // Grouping is per source sketch: a selection spanning two sketches makes
    // one copy call per source, each landing on its own new sketch.
    const { scene, raw } = makeReplayScene()
    const copies = duplicateSketchSelection(
      scene,
      [
        { kind: 'sketch', id: 3n },
        { kind: 'sketch', id: 4n },
      ],
      [0, 0, 0.5],
    )
    expect(raw.copy_sketch_islands).toHaveBeenCalledTimes(2)
    expect([...raw.copy_sketch_islands.mock.calls[0][1]]).toEqual([40n, 41n])
    expect([...raw.copy_sketch_islands.mock.calls[1][1]]).toEqual([50n])
    // Distinct copy sketches (70 then 71), all reselected.
    expect(copies).toEqual([
      { kind: 'sketch-island', id: 80n, sketch: 70n },
      { kind: 'sketch-island', id: 81n, sketch: 70n },
      { kind: 'sketch-island', id: 82n, sketch: 71n },
    ])
  })

  it('an out-of-plane copy is retracted whole when a LATER source-sketch copy throws', () => {
    // A selection spanning two source sketches makes one copy call each; the
    // second throws. Move+Alt is ONE user action, so the first copy must be
    // retracted too — the caller never receives a stranded copy to reselect.
    const { scene, raw, content, undoDepth } = makeReplayScene()
    const realImpl = raw.copy_sketch_islands.getMockImplementation()!
    let calls = 0
    raw.copy_sketch_islands.mockImplementation((s: bigint, islands: BigUint64Array, a: Float64Array) => {
      calls += 1
      if (calls === 2) throw new Error('WouldRetopologize: nope')
      return realImpl(s, islands, a)
    })
    const before = content()
    expect(() =>
      duplicateSketchSelection(
        scene,
        [
          { kind: 'sketch', id: 3n },
          { kind: 'sketch', id: 4n },
        ],
        [0, 0, 0.5],
      ),
    ).toThrow(/WouldRetopologize/)
    expect(content()).toBe(before) // byte-identical: the first copy is gone too
    expect(undoDepth()).toBe(0)
  })

  // --- failure semantics: a refused copy leaves the document untouched ----
  //
  // `sketch_cancel_gesture` is NEVER the right tool here: the adds have
  // already mutated the live sketch by the time a later one throws, and
  // cancelling only drops the undo record of them. Every failure below has to
  // come back to the pre-copy content byte-for-byte.

  it('a failure on a LATER replay call leaves nothing behind (the first-call test cannot see this)', () => {
    // Island 40's 2-edge curve chain replays fine; the third add — island
    // 41's plain edge — throws. Two edges are ALREADY live in the sketch when
    // the failure arrives, which is exactly the case a fail-on-the-first-add
    // test never reaches.
    const { scene, raw, content, undoDepth } = makeReplayScene({ failAfterAdds: 2 })
    const before = content()
    expect(() =>
      duplicateSketchSelection(scene, [{ kind: 'sketch', id: 3n }], [0.08, 0, 0]),
    ).toThrow(/WouldRetopologize/)
    expect(content()).toBe(before) // byte-identical: no orphaned half-copy
    expect(undoDepth()).toBe(0) // and nothing left for Ctrl+Z to have to reach
    expect(raw.sketch_cancel_gesture).not.toHaveBeenCalled()
  })

  it('a failed sketch also retracts the copies earlier sketches already committed', () => {
    // Sketch 3 replays fully (3 adds, one recorded step); sketch 4's only add
    // throws. Move+Alt is ONE user action, so a refusal may not leave sketch
    // 3's copy stranded — the caller never receives those copies to reselect.
    const { scene, content, undoDepth } = makeReplayScene({ failAfterAdds: 3 })
    const before = content()
    expect(() =>
      duplicateSketchSelection(
        scene,
        [
          { kind: 'sketch', id: 3n },
          { kind: 'sketch', id: 4n },
        ],
        [0.08, 0, 0],
      ),
    ).toThrow(/WouldRetopologize/)
    expect(content()).toBe(before)
    expect(undoDepth()).toBe(0)
  })

  it('a first-call failure records nothing and never pops an unrelated action', () => {
    // The gesture mutated nothing, so it recorded nothing — and the
    // retraction must therefore NOT fire, or it would pop whatever the user
    // did BEFORE the copy. (The history-generation guard is what proves our
    // own step is on top; an unguarded undo here is a known-bad regression.)
    const { scene, raw, content, undoDepth, pushUnrelatedAction } = makeReplayScene({
      failAfterAdds: 0,
    })
    pushUnrelatedAction()
    const before = content()
    expect(() =>
      duplicateSketchSelection(
        scene,
        [{ kind: 'sketch-island', id: 40n, sketch: 3n }],
        [0.08, 0, 0],
      ),
    ).toThrow(/WouldRetopologize/)
    expect(content()).toBe(before)
    expect(raw.scene_undo).not.toHaveBeenCalled()
    expect(undoDepth()).toBe(1) // the unrelated action is untouched
    // The bracket is closed with end_gesture (which records nothing for an
    // unchanged sketch), never with the non-restoring cancel.
    expect(raw.sketch_end_gesture).toHaveBeenCalledTimes(1)
    expect(raw.sketch_cancel_gesture).not.toHaveBeenCalled()
  })
})

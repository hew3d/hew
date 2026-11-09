/**
 * Link identical copies — grouping by signature, the poses it asks for, one
 * undo step, and skipping what the API or the kernel refuses. The scene is a
 * mock; `hew.query.signature` is answered through `api_dispatch` the way
 * healthBatch's bus calls are.
 */
import { describe, expect, it, vi } from 'vitest'
import type { Scene as WasmScene } from './wasm/loader'
import { canLinkSimilar, candidatePoses, linkSimilar, linkSimilarMessage, linkSimilarScope, findLookAlikes, type PartSignature } from './linkSimilar'
import { tagged } from './test/taggedDispatch'

/** A 2 by 1 by 0.5 part whose frame origin (its minimum corner) sits at `x`. */
const EXTENTS: [number, number, number] = [2, 1, 0.5]
const frameAt = (x: number): PartSignature['frame'] => ({ origin: [x, 0, 0], x: [1, 0, 0], y: [0, 1, 0], z: [0, 0, 1], extents: EXTENTS })
const sig = (shape: string, mirror: string, at: number): PartSignature => ({ shape, mirror, symmetric: shape === mirror, frame: frameAt(at) })

interface Mock {
  scene: WasmScene
  calls: string[]
  joins: Array<{ set: bigint; joiner: bigint; pose: number[] }>
}

function mockScene(
  sigs: Record<string, PartSignature | 'not_an_object'>,
  opts: {
    refuse?: (joiner: bigint, attempt: number) => boolean
    report?: Record<string, unknown>
    /** Joiners whose join throws `UnknownObject` (stale). */
    stale?: bigint[]
    /** Objects whose wrap throws `UnknownObject` (stale). */
    staleFirst?: bigint[]
  } = {},
): Mock {
  const calls: string[] = []
  const joins: Mock['joins'] = []
  const attempts = new Map<bigint, number>()
  const ids = Object.keys(sigs).map((k) => BigInt(k))
  const scene = {
    object_ids: () => BigUint64Array.from(ids),
    node_is_live: () => true,
    node_parent: () => undefined,
    object_public_ids: (a: BigUint64Array) => Array.from(a).map((n) => `obj_${n}`),
    api_connection_open: () => 1,
    api_connection_close: () => calls.push('close'),
    api_dispatch: (_c: number, raw: string) => {
      const req = JSON.parse(raw) as { id: number; method: string; params?: { id?: string } }
      if (req.method !== 'hew.query.signature') return tagged(JSON.stringify({ jsonrpc: '2.0', id: req.id, result: {} }))
      const s = sigs[String(req.params?.id).slice(4)]
      return tagged(
        JSON.stringify(
          s === 'not_an_object'
            ? { jsonrpc: '2.0', id: req.id, error: { code: -32000, message: 'no', data: { refusal: 'not_an_object' } } }
            : { jsonrpc: '2.0', id: req.id, result: s },
        ),
      )
    },
    history_bracket_open: () => calls.push('open'),
    history_bracket_commit: (label: string) => calls.push(`commit:${label}`),
    history_bracket_abort: () => calls.push('abort'),
    wrap_linked: (_k: number, id: bigint) => {
      if (opts.staleFirst?.includes(id)) throw new Error('UnknownObject: That object no longer exists')
      calls.push(`wrap:${id}`)
      return 900n
    },
    join_linked_report: (set: bigint, joiner: bigint, pose: Float64Array) => {
      const n = (attempts.get(joiner) ?? 0) + 1
      attempts.set(joiner, n)
      if (opts.stale?.includes(joiner)) throw new Error('UnknownObject: That object no longer exists')
      if (opts.refuse?.(joiner, n)) throw new Error('NotIdentical: These aren’t the same shape')
      joins.push({ set, joiner, pose: Array.from(pose) })
      return JSON.stringify({ instance: '1000', dropped_face_paint: false, frame_differed: false, ...opts.report })
    },
  } as unknown as WasmScene
  return { scene, calls, joins }
}

describe('linkSimilar', () => {
  it('four_identical_boards_become_one_set_of_four', () => {
    const m = mockScene({ 1: sig('A', 'B', 0), 2: sig('A', 'B', 10), 3: sig('A', 'B', 20), 4: sig('A', 'B', 30) })
    const groups = findLookAlikes(m.scene, null)
    expect(groups).toHaveLength(1)
    const r = linkSimilar(m.scene, groups)
    expect(r).toMatchObject({ sets: 1, linked: 3, skipped: 0 })
    expect(m.calls.filter((c) => c.startsWith('wrap:'))).toEqual(['wrap:1'])
    expect(m.joins.map((j) => j.joiner)).toEqual([2n, 3n, 4n])
    // frame_joiner ∘ frame_first⁻¹ with identical axes is the translation between origins
    expect(m.joins[1].pose).toEqual([1, 0, 0, 20, 0, 1, 0, 0, 0, 0, 1, 0])
    expect(linkSimilarMessage(r)).toBe('Linked 4 copies')
  })

  it('a_mirror_image_joins_with_a_mirrored_pose', () => {
    const m = mockScene({ 1: sig('A', 'B', 0), 2: sig('B', 'A', 10) })
    linkSimilar(m.scene, findLookAlikes(m.scene, null))
    const pose = m.joins[0].pose
    expect(pose[0] * pose[5] * pose[10]).toBeLessThan(0)
    // Reflected about the frame's center, so the body stays inside the
    // joiner's frame: x -> -(x - 1) + 1 + 10.
    expect(pose[3]).toBe(EXTENTS[0] + 10)
  })

  it('every_candidate_pose_maps_the_firsts_center_onto_the_joiners', () => {
    const apply = (m: number[], p: [number, number, number]) => [
      m[0] * p[0] + m[1] * p[1] + m[2] * p[2] + m[3],
      m[4] * p[0] + m[5] * p[1] + m[6] * p[2] + m[7],
      m[8] * p[0] + m[9] * p[1] + m[10] * p[2] + m[11],
    ]
    const center = (x: number): [number, number, number] => [x + EXTENTS[0] / 2, EXTENTS[1] / 2, EXTENTS[2] / 2]
    const corner = (x: number): [number, number, number] => [x, 0, 0]
    for (const [first, joiner] of [
      [sig('A', 'B', 0), sig('A', 'B', 10)],
      [sig('A', 'B', 0), sig('B', 'A', 10)],
    ] as const) {
      const poses = candidatePoses(first, joiner)
      expect(poses).toHaveLength(4)
      for (const pose of poses) {
        // The center is fixed by every symmetry; the minimum corner lands on
        // one of the joiner frame's eight corners, never outside the frame.
        expect(apply(pose, center(0)).map((v) => +v.toFixed(12))).toEqual(center(10))
        const c = apply(pose, corner(0))
        expect([10, 10 + EXTENTS[0]]).toContain(+c[0].toFixed(12))
        expect([0, EXTENTS[1]]).toContain(+c[1].toFixed(12))
        expect([0, EXTENTS[2]]).toContain(+c[2].toFixed(12))
      }
      // The identity (or the plain reflection) comes first.
      const p0 = poses[0]
      expect(Math.abs(p0[5])).toBe(1)
      expect(Math.abs(p0[10])).toBe(1)
    }
  })

  it('a_pose_a_symmetry_off_is_retried_after_NotIdentical', () => {
    const m = mockScene({ 1: sig('A', 'B', 0), 2: sig('A', 'B', 10) }, { refuse: (_j, n) => n === 1 })
    const r = linkSimilar(m.scene, findLookAlikes(m.scene, null))
    expect(r.linked).toBe(1)
    expect(m.joins[0].pose).not.toEqual(candidatePoses(sig('A', 'B', 0), sig('A', 'B', 10))[0])
  })

  it('the_joiners_attributes_move_to_their_placements_and_the_firsts_stay', () => {
    // The kernel moves them (join_linked); the app joins every other Object and wraps only the first.
    const m = mockScene({ 1: sig('A', 'B', 0), 2: sig('A', 'B', 10), 3: sig('A', 'B', 20) })
    linkSimilar(m.scene, findLookAlikes(m.scene, null))
    expect(m.calls.filter((c) => c.startsWith('wrap:'))).toEqual(['wrap:1'])
    expect(m.joins.every((j) => j.joiner !== 1n && j.set === 900n)).toBe(true)
  })

  it('it_is_one_undo_entry_and_never_automatic', () => {
    const m = mockScene({ 1: sig('A', 'B', 0), 2: sig('A', 'B', 10), 3: sig('A', 'B', 20) })
    const groups = findLookAlikes(m.scene, null)
    // Finding and the gate change nothing.
    expect(canLinkSimilar(m.scene, null)).toBe(true)
    expect(m.calls.filter((c) => c === 'open' || c.startsWith('commit'))).toEqual([])
    linkSimilar(m.scene, groups)
    expect(m.calls.filter((c) => c === 'open')).toHaveLength(1)
    expect(m.calls.filter((c) => c.startsWith('commit'))).toEqual(['commit:Link identical copies'])
  })

  it('a_refused_signature_skips_the_node', () => {
    const m = mockScene({ 1: sig('A', 'B', 0), 2: 'not_an_object', 3: sig('A', 'B', 20) })
    const r = linkSimilar(m.scene, findLookAlikes(m.scene, null))
    expect(r.linked).toBe(1)
    expect(m.joins.map((j) => j.joiner)).toEqual([3n])
  })

  it('a_lone_object_or_all_different_objects_offer_nothing', () => {
    const m = mockScene({ 1: sig('A', 'B', 0), 2: sig('C', 'D', 10) })
    expect(canLinkSimilar(m.scene, null)).toBe(false)
  })

  it('a_joiner_with_no_accepted_pose_is_skipped_and_counted', () => {
    const m = mockScene(
      { 1: sig('A', 'B', 0), 2: sig('A', 'B', 10), 3: sig('A', 'B', 20) },
      { refuse: (j) => j === 2n },
    )
    const r = linkSimilar(m.scene, findLookAlikes(m.scene, null))
    expect(r).toMatchObject({ sets: 1, linked: 1, skipped: 1 })
    expect(linkSimilarMessage(r)).toBe('Linked 2 copies; 1 object could not be linked')
  })

  it('a_group_whose_every_joiner_is_refused_is_rolled_back_and_not_counted', () => {
    // Group A links; group C's only joiner is refused at every pose, so its
    // first Object must not stay wrapped: the step is redone without it.
    const m = mockScene(
      { 1: sig('A', 'B', 0), 2: sig('A', 'B', 10), 3: sig('A', 'B', 20), 4: sig('C', 'D', 30), 5: sig('C', 'D', 40) },
      { refuse: (j) => j === 5n },
    )
    const r = linkSimilar(m.scene, findLookAlikes(m.scene, null))
    expect(r).toEqual({ sets: 1, linked: 2, skipped: 2, droppedFacePaint: 0, frameDiffered: 0 })
    const abort = m.calls.lastIndexOf('abort')
    expect(abort).toBeGreaterThan(-1)
    const kept = m.calls.slice(abort + 1)
    expect(kept.filter((c) => c.startsWith('wrap:'))).toEqual(['wrap:1'])
    expect(kept.filter((c) => c === 'open')).toHaveLength(1)
    expect(kept.filter((c) => c.startsWith('commit'))).toEqual(['commit:Link identical copies'])
    expect(m.calls.filter((c) => c.startsWith('commit'))).toHaveLength(1)
    expect(linkSimilarMessage(r)).toBe('Linked 3 copies; 2 objects could not be linked')
  })

  it('when_no_group_links_nothing_changes_and_the_message_says_why', () => {
    const m = mockScene({ 1: sig('A', 'B', 0), 2: sig('A', 'B', 10) }, { refuse: () => true })
    const r = linkSimilar(m.scene, findLookAlikes(m.scene, null))
    expect(r).toMatchObject({ sets: 0, linked: 0, skipped: 2 })
    expect(m.calls.filter((c) => c.startsWith('commit'))).toEqual([])
    expect(m.calls.at(-1)).toBe('abort')
    expect(linkSimilarMessage(r)).toBe('Nothing was linked: 2 objects looked alike but were not the same shape.')
    expect(linkSimilarMessage({ sets: 0, linked: 0, skipped: 0, droppedFacePaint: 0, frameDiffered: 0 })).toBe(
      'Nothing was linked: no two objects were the same shape.',
    )
  })

  it('a_stale_joiner_or_a_stale_first_object_is_skipped_not_fatal', () => {
    const m = mockScene(
      { 1: sig('A', 'B', 0), 2: sig('A', 'B', 10), 3: sig('A', 'B', 20), 4: sig('C', 'D', 30), 5: sig('C', 'D', 40), 6: sig('E', 'F', 50), 7: sig('E', 'F', 60) },
      { stale: [2n], staleFirst: [4n] },
    )
    const r = linkSimilar(m.scene, findLookAlikes(m.scene, null))
    expect(r).toMatchObject({ sets: 2, linked: 2, skipped: 3 })
    expect(m.calls.filter((c) => c.startsWith('commit'))).toEqual(['commit:Link identical copies'])
    expect(m.joins.map((j) => j.joiner)).toEqual([3n, 7n])
  })

  it('the_announcement_says_when_face_paint_was_dropped_or_a_frame_differed', () => {
    const m = mockScene(
      { 1: sig('A', 'B', 0), 2: sig('A', 'B', 10), 3: sig('A', 'B', 20) },
      { report: { dropped_face_paint: true, frame_differed: true } },
    )
    const msg = linkSimilarMessage(linkSimilar(m.scene, findLookAlikes(m.scene, null)))
    expect(msg).toContain('Linked 3 copies')
    expect(msg).toContain('paint on the faces of 2 copies was dropped')
    expect(msg).toContain('2 copies had their own axes')
    expect(msg).not.toMatch(/material/i)
  })

  it('an_unreachable_signature_command_offers_nothing', () => {
    const m = mockScene({ 1: sig('A', 'B', 0), 2: sig('A', 'B', 10) })
    ;(m.scene as unknown as { api_dispatch: unknown }).api_dispatch = vi.fn(() =>
      tagged(JSON.stringify({ jsonrpc: '2.0', id: 1, error: { code: -32601, message: 'no such method' } })),
    )
    expect(canLinkSimilar(m.scene, null)).toBe(false)
  })
})

describe('linkSimilarScope', () => {
  const picked = [
    { kind: 'object' as const, id: 1n },
    { kind: 'object' as const, id: 2n },
  ]

  it('the Edit menu acts on the selection; the Model Health button on the whole model it counted', () => {
    expect(linkSimilarScope('menu', picked)).toBe(picked)
    expect(linkSimilarScope('model-health', picked)).toBeNull()
  })

  it('the Model Health button links every look-alike group even with two of them selected', () => {
    const m = mockScene({ 1: sig('A', 'B', 0), 2: sig('A', 'B', 10), 3: sig('C', 'D', 20), 4: sig('C', 'D', 30) })
    expect(findLookAlikes(m.scene, linkSimilarScope('menu', picked))).toHaveLength(1)
    const r = linkSimilar(m.scene, findLookAlikes(m.scene, linkSimilarScope('model-health', picked)))
    expect(r).toMatchObject({ sets: 2, linked: 2 })
  })
})

import { describe, it, expect, vi } from 'vitest'
import { runDraft, addDraftSegment } from './draftCommit'
import { groundDrawPlane } from './drawPlane'
import type { Scene as WasmScene } from '../wasm/loader'

const REPORT = JSON.stringify({ objects: [5], sketches: [9, 10], kept_as_sketch: [[[0, 0, 0], [1, 0, 0]]] })

function mockScene(over: Record<string, unknown> = {}) {
  const calls: string[] = []
  const rec = <T>(name: string, ret: T) => vi.fn((..._a: unknown[]) => { calls.push(name); return ret })
  const scene = {
    draft_begin_plane: rec('begin_plane', 1n),
    draft_begin_plane_in_instance: rec('begin_plane_in_instance', 1n),
    draft_begin_face: rec('begin_face', 1n),
    draft_begin_face_in_instance: rec('begin_face_in_instance', 1n),
    draft_begin_sketch: rec('begin_sketch', 1n),
    draft_begin_detached: rec('begin_detached', 1n),
    draft_end: rec('end', REPORT),
    draft_cancel: rec('cancel', true),
    sketch_plane: vi.fn(() => new Float64Array([0, 0, 0, 0, 0, 1])),
    instance_pose: vi.fn(() => new Float64Array([1, 0, 0, 5, 0, 1, 0, 6, 0, 0, 1, 7])),
    ...over,
  }
  return { scene: scene as unknown as WasmScene, calls, raw: scene }
}

const PLANE = { kind: 'plane', plane: groundDrawPlane(), instance: null } as const

describe('runDraft', () => {
  it('opens, runs the body with the handle, ends, and parses the report', () => {
    const { scene, calls } = mockScene()
    const { result, report } = runDraft(scene, PLANE, 'Rectangle', (d) => {
      calls.push('body')
      expect(d).toBe(1n)
      return 'r'
    })
    expect(result).toBe('r')
    expect(calls).toEqual(['begin_plane', 'body', 'end'])
    expect(report.objects).toEqual([5n])
    expect(report.sketches).toEqual([9n, 10n])
    expect(report.keptAsSketch).toHaveLength(1)
  })

  it('cancels and rethrows unchanged when the body throws', () => {
    const { scene, calls } = mockScene()
    const err = new Error('PointOffPlane: nope')
    expect(() => runDraft(scene, PLANE, 'x', () => { throw err })).toThrow(err)
    expect(calls).toEqual(['begin_plane', 'cancel'])
  })

  it('cancels and rethrows unchanged when draft_end refuses', () => {
    const err = new Error('SomeRefusal: at end')
    const { scene, calls } = mockScene({ draft_end: vi.fn(() => { throw err }) })
    expect(() => runDraft(scene, PLANE, 'x', () => 1)).toThrow(err)
    expect(calls).toEqual(['begin_plane', 'cancel'])
  })

  it('does not cancel when begin itself refuses', () => {
    const { scene, calls } = mockScene({ draft_begin_plane: vi.fn(() => { throw new Error('DraftAlreadyOpen') }) })
    expect(() => runDraft(scene, PLANE, 'x', () => 1)).toThrow('DraftAlreadyOpen')
    expect(calls).toEqual([])
  })

  it('routes each target kind to its begin call', () => {
    let m = mockScene()
    runDraft(m.scene, { kind: 'face', object: 2n, face: 3n, instance: null }, 'L', () => 0)
    expect(m.raw.draft_begin_face).toHaveBeenCalledWith(2n, 3n, 'L')

    m = mockScene()
    runDraft(m.scene, { kind: 'face', object: 2n, face: 3n, instance: 8n }, 'L', () => 0)
    expect(m.raw.draft_begin_face_in_instance).toHaveBeenCalledWith(8n, 2n, 3n, 'L')

    m = mockScene()
    runDraft(m.scene, { kind: 'plane', plane: groundDrawPlane(), instance: 8n }, 'L', () => 0)
    expect(m.raw.draft_begin_plane_in_instance).toHaveBeenCalledWith(8n, 0, 0, 0, 0, 0, 1, 'L')

    m = mockScene()
    runDraft(m.scene, { kind: 'sketch', handle: 4n, instance: null }, 'L', () => 0)
    expect(m.raw.draft_begin_sketch).toHaveBeenCalledWith(4n, 'L')
  })

  it('routes a detached plane or face target to draft_begin_detached with the exact context JSON', () => {
    let m = mockScene()
    runDraft(m.scene, { kind: 'plane', plane: groundDrawPlane(), instance: null, detached: true }, 'C', () => 0)
    expect(m.raw.draft_begin_detached).toHaveBeenCalledWith('{"plane":[0,0,0,0,0,1]}', 'C')
    expect(m.raw.draft_begin_plane).not.toHaveBeenCalled()

    m = mockScene()
    runDraft(m.scene, { kind: 'plane', plane: groundDrawPlane(), instance: 8n, detached: true }, 'C', () => 0)
    expect(m.raw.draft_begin_detached).toHaveBeenCalledWith('{"plane":[0,0,0,0,0,1],"instance":8}', 'C')

    m = mockScene()
    runDraft(m.scene, { kind: 'face', object: 2n, face: 3n, instance: null, detached: true }, 'C', () => 0)
    expect(m.raw.draft_begin_detached).toHaveBeenCalledWith('{"face":{"object":2,"face":3}}', 'C')
    expect(m.raw.draft_begin_face).not.toHaveBeenCalled()

    m = mockScene()
    runDraft(m.scene, { kind: 'face', object: 2n, face: 3n, instance: 8n, detached: true }, 'C', () => 0)
    expect(m.raw.draft_begin_detached).toHaveBeenCalledWith('{"face":{"object":2,"face":3},"instance":8}', 'C')
  })

  it('opens a definition-owned sketch as its world plane inside the instance', () => {
    const m = mockScene()
    runDraft(m.scene, { kind: 'sketch', handle: 4n, instance: 8n }, 'L', () => 0)
    expect(m.raw.draft_begin_plane_in_instance).toHaveBeenCalledWith(8n, 5, 6, 7, 0, 0, 1, 'L')
  })

  it('refuses a vanished hover-adopted sketch without opening a draft', () => {
    const m = mockScene({ sketch_plane: vi.fn(() => undefined) })
    expect(() => runDraft(m.scene, { kind: 'sketch', handle: 4n, instance: null }, 'L', () => 0)).toThrow(/^UnknownSketch/)
    expect(m.calls).toEqual([])
  })
})

describe('addDraftSegment', () => {
  it('returns the created regions and frees the report', () => {
    const free = vi.fn()
    const m = mockScene({
      draft_add_segment: vi.fn(() => ({ regions_created: () => new BigUint64Array([7n]), free })),
    })
    expect(addDraftSegment(m.scene, 1n, [0, 0, 0], [1, 0, 0])).toEqual([7n])
    expect(free).toHaveBeenCalled()
  })
})

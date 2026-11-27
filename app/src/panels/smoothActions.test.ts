/**
 * smoothActions: the Smooth level, Bake and Soften edits Object Info and the
 * palette share. A fake scene records the calls and the history brackets.
 */
import { describe, it, expect, vi } from 'vitest'
import {
  SMOOTH_BOOLEAN_NOTE,
  bakeSmooth,
  operandsHaveSmooth,
  parseSoftenDegrees,
  setSmoothLevel,
  smoothDenseToasts,
  smoothLevels,
  smoothRefusal,
  softenEdges,
  softenToast,
} from './smoothActions'
import type { Scene as WasmScene } from '../wasm/loader'

function makeScene(levels: Record<string, number> = {}, opts: { failOn?: bigint } = {}) {
  const log: string[] = []
  const free = vi.fn()
  const scene = {
    smooth_level: (id: bigint) => levels[String(id)] ?? 0,
    set_smooth_level: vi.fn((id: bigint, level: number) => {
      if (opts.failOn === id) throw new Error('AmountOutOfRange: too dense')
      log.push(`set ${id}=${level}`)
      return { free }
    }),
    bake: vi.fn((id: bigint) => {
      log.push(`bake ${id}`)
      return { free }
    }),
    soften_by_angle: vi.fn((id: bigint, rad: number) => {
      log.push(`soften ${id}@${Math.round((rad * 180) / Math.PI)}`)
      return Number(id)
    }),
    history_bracket_open: vi.fn(() => log.push('open')),
    history_bracket_commit: vi.fn((label: string) => {
      log.push(`commit ${label}`)
      return true
    }),
    history_bracket_abort: vi.fn(() => log.push('abort')),
    group_members: (id: bigint) => (id === 50n ? [{ kind: 'object', id: 3n }, { kind: 'object', id: 4n }] : []),
  }
  return { scene: scene as unknown as WasmScene, log }
}

describe('smoothActions', () => {
  it('reads each object\'s level', () => {
    const { scene } = makeScene({ '1': 2, '2': 0 })
    expect(smoothLevels(scene, [1n, 2n])).toEqual([2, 0])
  })

  it('one object needs no bracket; several are one undo step', () => {
    const one = makeScene()
    setSmoothLevel(one.scene, [1n], 2)
    expect(one.log).toEqual(['set 1=2'])
    const many = makeScene()
    setSmoothLevel(many.scene, [1n, 2n], 3)
    expect(many.log).toEqual(['open', 'set 1=3', 'set 2=3', 'commit Set Smooth'])
  })

  it('a refusal on one object undoes the whole bracket', () => {
    const { scene, log } = makeScene({}, { failOn: 2n })
    expect(() => setSmoothLevel(scene, [1n, 2n], 3)).toThrow()
    expect(log).toEqual(['open', 'set 1=3', 'abort'])
  })

  it('explains an over-dense cage in plain words', () => {
    const r = smoothRefusal(new Error('AmountOutOfRange: too dense'), 3)
    expect(r.message).toBe('This cage is too dense for Smooth level 3.')
    expect(r.code).toBe('AmountOutOfRange')
  })

  it('bakes only the smoothed objects', () => {
    const { scene, log } = makeScene({ '1': 2, '2': 0, '3': 1 })
    expect(bakeSmooth(scene, [1n, 2n, 3n])).toBe(2)
    expect(log).toEqual(['open', 'bake 1', 'bake 3', 'commit Bake Smooth'])
    expect(bakeSmooth(scene, [2n])).toBe(0)
  })

  it('softens by degrees and adds up the edges', () => {
    const { scene, log } = makeScene()
    expect(softenEdges(scene, [4n, 5n], 20)).toBe(9)
    expect(log).toEqual(['open', 'soften 4@20', 'soften 5@20', 'commit Soften Edges'])
  })

  it('words the Soften result', () => {
    expect(softenToast(12)).toBe('Softened 12 edges.')
    expect(softenToast(1)).toBe('Softened 1 edge.')
    expect(softenToast(0)).toBe('No edges to soften.')
  })

  it('reads the angle field', () => {
    expect(parseSoftenDegrees('20')).toBe(20)
    expect(parseSoftenDegrees(' 12.5° ')).toBe(12.5)
    expect(parseSoftenDegrees('0')).toBeNull()
    expect(parseSoftenDegrees('180')).toBeNull()
    expect(parseSoftenDegrees('abc')).toBeNull()
    expect(parseSoftenDegrees('')).toBeNull()
  })

  it('notices a smoothed operand, also inside a group', () => {
    const { scene } = makeScene({ '4': 1 })
    expect(operandsHaveSmooth(scene, [{ kind: 'object', id: 4n }, { kind: 'object', id: 9n }])).toBe(true)
    expect(operandsHaveSmooth(scene, [{ kind: 'group', id: 50n }])).toBe(true)
    expect(operandsHaveSmooth(scene, [{ kind: 'object', id: 9n }])).toBe(false)
    expect(SMOOTH_BOOLEAN_NOTE).toBe('Combining smooth objects uses their cages. Bake first for an exact cut.')
  })
})

describe('smoothDenseToasts', () => {
  const sceneWith = (levels: Record<string, number>, dense: Set<bigint>, names: Record<string, string> = {}) =>
    ({
      smooth_level: (id: bigint) => levels[String(id)] ?? 0,
      smooth_surface_available: (id: bigint) => !dense.has(id),
      object_name: (id: bigint) => names[String(id)],
    }) as unknown as WasmScene

  it('speaks once per object, with the plain copy and code', () => {
    const dense = new Set([1n])
    const scene = sceneWith({ '1': 2 }, dense, { '1': 'Vase' })
    const warned = new Set<bigint>()
    expect(smoothDenseToasts(scene, [1n], warned)).toEqual([
      {
        message: 'Smooth 2 on Vase is off for now: the shape grew too dense for it. Lower the level or simplify the shape.',
        code: 'SmoothTooDense',
      },
    ])
    expect(smoothDenseToasts(scene, [1n], warned)).toEqual([])
    // Recovering and then relapsing speaks up again.
    dense.delete(1n)
    expect(smoothDenseToasts(scene, [1n], warned)).toEqual([])
    dense.add(1n)
    expect(smoothDenseToasts(scene, [1n], warned)).toHaveLength(1)
  })

  it('forgets a warned object a full refresh no longer lists, so a relapse speaks again', () => {
    const dense = new Set([1n])
    const scene = sceneWith({ '1': 2 }, dense)
    const warned = new Set<bigint>()
    expect(smoothDenseToasts(scene, [1n], warned)).toHaveLength(1)
    // A refresh naming touched objects leaves the memory alone.
    expect(smoothDenseToasts(scene, [2n], warned)).toEqual([])
    expect(warned.has(1n)).toBe(true)
    // A full refresh (every smoothed object) without it: the object went to
    // Off or away, and is forgotten.
    expect(smoothDenseToasts(scene, [], warned, true)).toEqual([])
    expect(warned.has(1n)).toBe(false)
    // Smoothed again and still too dense: it speaks once more.
    expect(smoothDenseToasts(scene, [1n], warned, true)).toHaveLength(1)
  })

  it('reads only the objects it is given, and skips objects at Off', () => {
    const scene = sceneWith({ '2': 0, '3': 1 }, new Set([2n]))
    const spy = vi.spyOn(scene as unknown as { smooth_level: (id: bigint) => number }, 'smooth_level')
    expect(smoothDenseToasts(scene, [2n], new Set())).toEqual([])
    expect(spy.mock.calls.every(([id]) => id === 2n)).toBe(true)
  })
})

describe('smoothRefusal for SmoothTooDense', () => {
  it('shows the kernel copy and code', () => {
    expect(smoothRefusal(new Error('SmoothTooDense: cap'), 3)).toEqual({
      message: 'This shape is too dense for that Smooth level. Try a lower level, or simplify the shape first.',
      code: 'SmoothTooDense',
    })
  })
})

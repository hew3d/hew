/**
 * PathArrayTool unit tests: selection gate, path pick, the typed count and
 * ramp, Alt for align, the retype window, and kernel refusals. A fake scene
 * stands in for wasm, in the pattern of FollowMeTool.test.ts.
 */
import { describe, it, expect, vi } from 'vitest'
import * as THREE from 'three'
import { PathArrayTool } from './PathArrayTool'
import type { NodeRef } from '../panels/treeModel'
import type { Scene as WasmScene } from '../wasm/loader'
import type { Ray } from '../viewport/math'

const RAY: Ray = { origin: [0, 0, 5], direction: [0, 0, -1] }
const BOX: NodeRef = { kind: 'object', id: 7n }

function makeScene(opts: { arrayThrows?: string; boxLengthX?: number; spacingX?: number } = {}) {
  let generation = 0n
  const calls: unknown[][] = []
  const scene = {
    pick_sketch_edge: () => ({ sketch: () => 3n, edge: () => 11n, free: vi.fn() }),
    pick_face: () => undefined,
    sketch_edge_island: () => 1n,
    sketch_island_edges: () => new BigUint64Array([11n, 12n]),
    sketch_edge_endpoints: () => new Float64Array([0, 0, 0, 1, 0, 0]),
    face_boundary: () => new Float64Array(),
    max_array_count: () => 1000,
    history_generation: () => generation,
    undo_depth: () => 1,
    scene_undo: vi.fn(() => {
      generation += 1n
      return { free: vi.fn() }
    }),
    scene_redo: vi.fn(() => ({ free: vi.fn() })),
    array_along_path_preview_transforms: vi.fn((_p: string, count: number) => {
      const out = new Float64Array(12 * (count - 1))
      // With a spacing given: identity placements marching along +x.
      if (opts.spacingX !== undefined) {
        for (let i = 0; i < count - 1; i += 1) {
          out.set([1, 0, 0, opts.spacingX * (i + 1), 0, 1, 0, 0, 0, 0, 1, 0], i * 12)
        }
      }
      return out
    }),
    ...(opts.boxLengthX !== undefined
      ? { object_bounds: () => new Float64Array([0, 0, 0, opts.boxLengthX ?? 0, 0.1, 0.1]) }
      : {}),
    array_along_path: vi.fn((...args: unknown[]) => {
      calls.push(args)
      if (opts.arrayThrows !== undefined) throw new Error(opts.arrayThrows)
      generation += 1n
      return [{ kind: 'object', id: 20n }]
    }),
  }
  return { scene, calls }
}

function makeTool(selection: NodeRef[] = [BOX], opts: { arrayThrows?: string; boxLengthX?: number; spacingX?: number } = {}) {
  const { scene, calls } = makeScene(opts)
  const onCommit = vi.fn()
  const onToast = vi.fn()
  const tool = new PathArrayTool(
    scene as unknown as WasmScene,
    new THREE.Group(),
    null,
    selection,
    onCommit,
    onToast,
  )
  tool.setFaceEligibility(() => true)
  return { tool, scene, calls, onCommit, onToast }
}

const key = (k: string) => ({ key: k, repeat: false }) as KeyboardEvent
const type = (tool: PathArrayTool, text: string) => {
  for (const k of text) tool.onKey(key(k))
}
const enter = (tool: PathArrayTool) => tool.onKey(key('Enter'))
const alt = (tool: PathArrayTool) => tool.onKey(key('Alt'))

describe('PathArrayTool', () => {
  it('asks for a selection first, then a path', () => {
    const bare = makeTool([])
    expect(bare.tool.statusHint()).toBe('Select what to copy first.')
    bare.tool.onPointerDown(null, RAY)
    expect(bare.onToast).toHaveBeenCalledWith('Select what to copy first.')

    const { tool } = makeTool()
    expect(tool.statusHint()).toBe(
      'Click a path. Type a count, optionally a scale ramp like 100%→40%, then Enter. Copies turn to follow the path (Alt: face one way).',
    )
  })

  it('takes digits only once a path is picked', () => {
    const { tool } = makeTool()
    expect(tool.capturesKey('6')).toBe(false)
    tool.onPointerDown(null, RAY)
    expect(tool.capturesKey('6')).toBe(true)
    expect(tool.capturesKey(' ')).toBe(false)
  })

  it('previews, then commits on a typed count', () => {
    const { tool, scene, calls, onCommit } = makeTool()
    tool.onPointerDown(null, RAY)
    expect(scene.array_along_path_preview_transforms).toHaveBeenCalledWith(
      expect.stringContaining('"kind":"sketch_edges"'), 5, 1, 1, true,
    )
    type(tool, '6')
    enter(tool)
    expect(calls).toHaveLength(1)
    const [kinds, ids, path, count, from, to, align] = calls[0]
    expect(Array.from(kinds as Uint8Array)).toEqual([0])
    expect(Array.from(ids as BigUint64Array)).toEqual([7n])
    expect(JSON.parse(path as string)).toEqual({ kind: 'sketch_edges', sketch: '3', edges: ['11', '12'] })
    expect([count, from, to, align]).toEqual([6, 1, 1, true])
    expect(onCommit).toHaveBeenCalledWith([{ kind: 'object', id: 20n }])
  })

  it('notes when the copies overlap at this count, and says nothing when they clear', () => {
    // A 0.5 m long part, copies 0.25 m apart along the path's first direction.
    const tight = makeTool([BOX], { boxLengthX: 0.5, spacingX: 0.25 })
    tight.tool.onPointerDown(null, RAY)
    expect(tight.tool.statusHint()).not.toContain('overlap')
    type(tight.tool, '4')
    enter(tight.tool)
    expect(tight.onCommit).toHaveBeenCalled() // a note, never a refusal
    expect(tight.onToast).not.toHaveBeenCalled()
    expect(tight.tool.statusHint()).toBe('Copies overlap at this count.')
    // A smaller count is no help here, but a typed count re-evaluates: 0.25
    // spacing against the same part still overlaps.
    type(tight.tool, '3')
    enter(tight.tool)
    expect(tight.tool.statusHint()).toBe('Copies overlap at this count.')

    const clear = makeTool([BOX], { boxLengthX: 0.5, spacingX: 0.75 })
    clear.tool.onPointerDown(null, RAY)
    type(clear.tool, '4')
    enter(clear.tool)
    expect(clear.tool.statusHint()).not.toContain('overlap')
  })

  it('judges overlap with the scale ramp: shrinking copies clear what full-size ones would not', () => {
    // A 0.5 m part, copies 0.45 m apart: full size, the first copy overlaps
    // the original (0.45 < 0.5).
    const plain = makeTool([BOX], { boxLengthX: 0.5, spacingX: 0.45 })
    plain.tool.onPointerDown(null, RAY)
    type(plain.tool, '4')
    enter(plain.tool)
    expect(plain.tool.statusHint()).toBe('Copies overlap at this count.')
    // Scaled to 20%, each copy reaches 0.1 m: original to copy 1 needs
    // (0.5 + 0.1) / 2 = 0.3 m, and copy to copy 0.1 m; 0.45 m clears both.
    const ramped = makeTool([BOX], { boxLengthX: 0.5, spacingX: 0.45 })
    ramped.tool.onPointerDown(null, RAY)
    type(ramped.tool, '20%->20%')
    enter(ramped.tool)
    type(ramped.tool, '4')
    enter(ramped.tool)
    expect(ramped.onCommit).toHaveBeenCalled()
    expect(ramped.tool.statusHint()).not.toContain('overlap')
  })

  it('applies a ramp first, then the count commits with it', () => {
    const { tool, calls } = makeTool()
    tool.onPointerDown(null, RAY)
    type(tool, '100%->40%')
    enter(tool)
    expect(calls).toHaveLength(0)
    type(tool, '8x')
    enter(tool)
    expect(calls[0].slice(3)).toEqual([8, 1, 0.4, true])
  })

  it('flips align with Alt', () => {
    const { tool, calls } = makeTool()
    tool.onPointerDown(null, RAY)
    alt(tool)
    expect(tool.statusHint()).toContain('Copies all face one way (Alt: follow the path).')
    type(tool, '4')
    enter(tool)
    expect(calls[0][6]).toBe(false)
  })

  it('re-commits when a new count is typed after the commit', () => {
    const { tool, scene, calls } = makeTool()
    tool.onPointerDown(null, RAY)
    type(tool, '4')
    enter(tool)
    type(tool, '9')
    enter(tool)
    expect(scene.scene_undo).toHaveBeenCalledTimes(1)
    expect(calls).toHaveLength(2)
    expect(calls[1][3]).toBe(9)
    // A ramp after the commit retypes too, keeping the count.
    type(tool, '100%->50%')
    enter(tool)
    expect(calls[2].slice(3)).toEqual([9, 1, 0.5, true])
  })

  it('toasts the kernel copy for a refusal and keeps the path', () => {
    const { tool, onToast, onCommit } = makeTool([BOX], { arrayThrows: 'PathTooShortForCount: path is short' })
    tool.onPointerDown(null, RAY)
    type(tool, '4')
    enter(tool)
    expect(onToast).toHaveBeenCalledWith('The path is too short for that many copies.', 'PathTooShortForCount')
    expect(onCommit).not.toHaveBeenCalled()
    expect(tool.hasArmedGesture()).toBe(true)
  })

  it('refuses counts below 2 and above the cap', () => {
    const { tool, calls, onToast } = makeTool()
    tool.onPointerDown(null, RAY)
    type(tool, '1')
    enter(tool)
    type(tool, '5000')
    enter(tool)
    expect(calls).toHaveLength(0)
    expect(onToast).toHaveBeenCalledTimes(2)
  })
})

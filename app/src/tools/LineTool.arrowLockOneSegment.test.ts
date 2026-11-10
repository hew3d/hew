/**
 * R19: an arrow-key axis lock covers ONE segment of a Line chain, as in
 * SketchUp. Once a click commits the locked segment the lock releases and
 * the hint returns to the plain "Click the next point". A Shift-held lock
 * is the held key's own and stays; the idle plane locks of the other draw
 * tools are not touched.
 */
import { describe, it, expect, vi } from 'vitest'
import * as THREE from 'three'
import { LineTool } from './LineTool'
import type { Snap } from './types'
import type { Scene as WasmScene } from '../wasm/loader'
import type { Ray } from '../viewport/math'
import { draftMock } from './draftMock'

const RAY: Ray = { origin: [0, 0, 5], direction: [0, 0, -1] }

const snapAt = (p: readonly [number, number, number]): Snap => ({ x: p[0], y: p[1], z: p[2], kind: 'ground' })
const key = (k: string): KeyboardEvent =>
  ({ key: k, repeat: false, preventDefault: () => { /* no-op */ } }) as unknown as KeyboardEvent

function makeTool() {
  const dm = draftMock()
  const scene = {
    axes: vi.fn(() => new Float64Array([0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1])),
    history_generation: vi.fn(() => 1n),
    pick_face: vi.fn(() => undefined),
    pick_sketch: vi.fn(() => undefined),
    ...dm.members,
    clear_transient_segments: vi.fn(),
    add_transient_segment: vi.fn(),
  } as unknown as WasmScene
  const tool = new LineTool(scene, new THREE.Group(), vi.fn(), vi.fn(), vi.fn(), vi.fn())
  return { tool, log: dm.log }
}

const lockOf = (tool: LineTool): unknown => (tool as unknown as { lockAxis: unknown }).lockAxis

describe('LineTool — an arrow lock covers one segment (R19)', () => {
  it('the lock releases once the locked segment commits, and the hint returns to the plain one', () => {
    const { tool, log } = makeTool()
    tool.onPointerDown(snapAt([2, 3, 0]), RAY)
    const plain = tool.statusHint()
    tool.onKey(key('ArrowUp'))
    expect(lockOf(tool)).toBe(2)
    tool.onPointerDown(snapAt([2, 3, 4]), RAY)
    expect(log.segments).toHaveLength(1)
    expect(lockOf(tool)).toBeNull()
    // The retype window a commit opens ("Type a length to resize…") closes
    // once the pointer moves on; close it directly in the mock-free harness.
    ;(tool as unknown as { retype: { close(): void } }).retype.close()
    expect(tool.statusHint()).toBe(plain)
    expect(tool.statusHint()).toContain('Click the next point')
  })

  it('the next segment is free: a click off the old axis commits where it was clicked', () => {
    const { tool, log } = makeTool()
    tool.onPointerDown(snapAt([2, 3, 0]), RAY)
    tool.onKey(key('ArrowUp'))
    tool.onPointerDown(snapAt([2, 3, 4]), RAY)
    tool.onPointerMove(snapAt([2, 3, 4]), RAY)
    tool.onPointerDown(snapAt([6, 3, 4]), RAY)
    expect(log.segments).toHaveLength(2)
    expect(log.segments[1]).toEqual([2, 3, 4, 6, 3, 4])
    expect(lockOf(tool)).toBeNull()
  })

  it('a second vertical needs its own arrow press', () => {
    const { tool, log } = makeTool()
    tool.onPointerDown(snapAt([0, 0, 0]), RAY)
    tool.onKey(key('ArrowUp'))
    tool.onPointerDown(snapAt([0, 0, 2]), RAY)
    tool.onKey(key('ArrowUp'))
    expect(lockOf(tool)).toBe(2)
    tool.onPointerDown(snapAt([0, 0, 5]), RAY)
    expect(log.segments.map((s) => s.slice(0, 6))).toEqual([[0, 0, 0, 0, 0, 2], [0, 0, 2, 0, 0, 5]])
    expect(lockOf(tool)).toBeNull()
  })

  it('a typed length commits the locked segment and releases the lock the same way', () => {
    const { tool, log } = makeTool()
    tool.onPointerDown(snapAt([0, 0, 0]), RAY)
    tool.onKey(key('ArrowUp'))
    tool.onPointerMove(snapAt([0, 0, 1]), RAY)
    for (const c of '3') tool.onKey(key(c))
    tool.onKey(key('Enter'))
    expect(log.segments).toHaveLength(1)
    expect(lockOf(tool)).toBeNull()
  })

  it('a Shift-held lock stays for the held key', () => {
    const { tool } = makeTool()
    tool.onPointerDown(snapAt([0, 0, 0]), RAY)
    tool.onPointerMove(snapAt([4, 0.1, 0]), RAY)
    tool.setShiftHeld(true)
    expect(lockOf(tool)).toBe(0)
    tool.onPointerDown(snapAt([4, 0, 0]), RAY)
    expect(lockOf(tool)).toBe(0)
    tool.setShiftHeld(false)
    expect(lockOf(tool)).toBeNull()
  })
})

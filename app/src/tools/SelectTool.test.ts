/**
 * SelectTool unit tests — the tool now only FORWARDS the resolved hover snap
 * and the ray to `onSelect`; the host runs the shared `resolveSelectableRef`
 * (covered by snapSelection.test.ts). So these pin the forwarding contract and
 * Escape handling; selection resolution is tested at the resolver.
 */
import { describe, it, expect, vi } from 'vitest'
import { SelectTool } from './SelectTool'
import type { Snap } from './types'
import type { Ray } from '../viewport/math'
import type { Scene as WasmScene } from '../wasm/loader'

const RAY: Ray = { origin: [0, 0, 5], direction: [0, 0, -1] }
const scene = {} as unknown as WasmScene

describe('SelectTool — forwards snap + ray to the host resolver', () => {
  it('forwards a resolved snap and the ray on pointer down', () => {
    const onSelect = vi.fn()
    const snap: Snap = { x: 1, y: 2, z: 3, kind: 'on-face', object: 7n }
    new SelectTool(scene, onSelect).onPointerDown(snap, RAY)
    expect(onSelect).toHaveBeenCalledWith(snap, RAY)
  })

  it('forwards a null snap (a genuine empty click still resolves the ray)', () => {
    const onSelect = vi.fn()
    new SelectTool(scene, onSelect).onPointerDown(null, RAY)
    expect(onSelect).toHaveBeenCalledWith(null, RAY)
  })
})

describe('SelectTool — cancel', () => {
  it('Escape clears the last-seen hover snap', () => {
    const onSelect = vi.fn()
    const tool = new SelectTool(scene, onSelect)
    tool.onPointerMove({ x: 0, y: 0, z: 0, kind: 'ground' }, RAY)
    expect(tool.lastSnap).not.toBeNull()
    tool.onKey({ key: 'Escape', preventDefault: () => {} } as unknown as KeyboardEvent)
    expect(tool.lastSnap).toBeNull()
  })
})

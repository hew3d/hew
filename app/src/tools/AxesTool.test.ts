import { describe, it, expect, vi } from 'vitest'
import * as THREE from 'three'
import { AxesTool } from './AxesTool'
import type { Snap } from './types'
import type { Ray } from '../viewport/math'

/** A ray straight down (−Z) through world (x, y) — AxesTool ignores it
 *  (every gesture step consumes only the resolved snap). */
function rayThrough(x: number, y: number): Ray {
  return { origin: [x, y, 5], direction: [0, 0, -1] }
}

function makeSnap(x: number, y: number, z: number): Snap {
  return { x, y, z, kind: 'ground' }
}

/** Minimal KeyboardEvent-shaped fake — onKey only reads .key. */
function makeKeyEvent(key: string): KeyboardEvent {
  return { key, repeat: false, preventDefault: () => { /* no-op */ } } as unknown as KeyboardEvent
}

/** Minimal WasmScene stub — only the member AxesTool calls. */
function makeWasmScene() {
  return { set_axes: vi.fn() }
}

function makeTool() {
  const preview = new THREE.Group()
  const onCommit = vi.fn()
  const onToast = vi.fn()
  const wasmScene = makeWasmScene()
  const tool = new AxesTool(wasmScene as never, preview, onCommit, onToast)
  return { tool, preview, onCommit, onToast, wasmScene }
}

describe('AxesTool — three-click gesture', () => {
  it('axis-aligned picks commit set_axes with the literal world frame', () => {
    const { tool, onCommit, onToast, wasmScene } = makeTool()
    expect(tool.statusHint()).toContain('new origin')

    tool.onPointerDown(makeSnap(0, 0, 0), rayThrough(0, 0))
    expect(tool.statusHint()).toContain('red (X)')

    tool.onPointerDown(makeSnap(1, 0, 0), rayThrough(1, 0))
    expect(tool.statusHint()).toContain('green (Y)')

    tool.onPointerDown(makeSnap(0, 1, 0), rayThrough(0, 1))

    expect(wasmScene.set_axes).toHaveBeenCalledTimes(1)
    const [ox, oy, oz, xx, xy, xz, yx, yy, yz] = wasmScene.set_axes.mock.calls[0]
    expect([ox, oy, oz]).toEqual([0, 0, 0])
    expect([xx, xy, xz]).toEqual([1, 0, 0])
    expect([yx, yy, yz]).toEqual([0, 1, 0])
    expect(onCommit).toHaveBeenCalledTimes(1)
    expect(onToast).not.toHaveBeenCalled()
    // Back to idle after a commit.
    expect(tool.statusHint()).toContain('new origin')
  })

  it('a non-axis-aligned Y pick still resolves to a unit y ⊥ x via Gram-Schmidt', () => {
    const { tool, wasmScene } = makeTool()
    tool.onPointerDown(makeSnap(0, 0, 0), rayThrough(0, 0))
    tool.onPointerDown(makeSnap(1, 0, 0), rayThrough(1, 0))
    // Not perpendicular to X, and out of the XY plane too.
    tool.onPointerDown(makeSnap(0.5, 1, 0.25), rayThrough(0.5, 1))

    expect(wasmScene.set_axes).toHaveBeenCalledTimes(1)
    const [, , , xx, xy, xz, yx, yy, yz] = wasmScene.set_axes.mock.calls[0]
    const x: [number, number, number] = [xx, xy, xz]
    const y: [number, number, number] = [yx, yy, yz]

    const yLen = Math.sqrt(y[0] ** 2 + y[1] ** 2 + y[2] ** 2)
    expect(yLen).toBeCloseTo(1, 9)
    const dot = x[0] * y[0] + x[1] * y[1] + x[2] * y[2]
    expect(dot).toBeCloseTo(0, 9)
  })

  it('ignores a degenerate 2nd click (coincident with the origin) — stage does not advance', () => {
    const { tool, wasmScene } = makeTool()
    tool.onPointerDown(makeSnap(0, 0, 0), rayThrough(0, 0))
    tool.onPointerDown(makeSnap(0, 0, 0), rayThrough(0, 0)) // same point as origin
    expect(tool.statusHint()).toContain('red (X)') // still origin-picked

    tool.onPointerDown(makeSnap(1, 0, 0), rayThrough(1, 0)) // now a real X pick
    expect(tool.statusHint()).toContain('green (Y)')
    tool.onPointerDown(makeSnap(0, 1, 0), rayThrough(0, 1))
    expect(wasmScene.set_axes).toHaveBeenCalledTimes(1)
  })

  it('ignores a degenerate 3rd click (colinear with origin/X) — stage does not advance', () => {
    const { tool, wasmScene } = makeTool()
    tool.onPointerDown(makeSnap(0, 0, 0), rayThrough(0, 0))
    tool.onPointerDown(makeSnap(1, 0, 0), rayThrough(1, 0))
    tool.onPointerDown(makeSnap(2, 0, 0), rayThrough(2, 0)) // colinear with origin→X
    expect(tool.statusHint()).toContain('green (Y)') // still x-picked, not committed
    expect(wasmScene.set_axes).not.toHaveBeenCalled()

    tool.onPointerDown(makeSnap(0, 1, 0), rayThrough(0, 1)) // now a real Y pick
    expect(wasmScene.set_axes).toHaveBeenCalledTimes(1)
  })

  it('toasts and resets to idle if set_axes throws', () => {
    const { tool, onCommit, onToast, wasmScene } = makeTool()
    wasmScene.set_axes.mockImplementation(() => { throw new Error('NonOrthonormal: bad frame') })
    tool.onPointerDown(makeSnap(0, 0, 0), rayThrough(0, 0))
    tool.onPointerDown(makeSnap(1, 0, 0), rayThrough(1, 0))
    tool.onPointerDown(makeSnap(0, 1, 0), rayThrough(0, 1))

    expect(onToast).toHaveBeenCalledTimes(1)
    expect(onToast.mock.calls[0][0]).toContain('NonOrthonormal')
    expect(onCommit).not.toHaveBeenCalled()
    expect(tool.statusHint()).toContain('new origin')
  })
})

describe('AxesTool — Escape steps back one stage', () => {
  it('origin-picked → idle on Escape', () => {
    const { tool } = makeTool()
    tool.onPointerDown(makeSnap(0, 0, 0), rayThrough(0, 0))
    expect(tool.statusHint()).toContain('red (X)')
    tool.onKey(makeKeyEvent('Escape'))
    expect(tool.statusHint()).toContain('new origin')
  })

  it('x-picked → origin-picked → idle on two separate Escapes (never a full jump from one)', () => {
    const { tool } = makeTool()
    tool.onPointerDown(makeSnap(0, 0, 0), rayThrough(0, 0))
    tool.onPointerDown(makeSnap(1, 0, 0), rayThrough(1, 0))
    expect(tool.statusHint()).toContain('green (Y)')

    tool.onKey(makeKeyEvent('Escape'))
    expect(tool.statusHint()).toContain('red (X)') // stepped back to origin-picked only

    tool.onKey(makeKeyEvent('Escape'))
    expect(tool.statusHint()).toContain('new origin') // now idle
  })

  it('cancel() fully resets from any stage', () => {
    const { tool } = makeTool()
    tool.onPointerDown(makeSnap(0, 0, 0), rayThrough(0, 0))
    tool.onPointerDown(makeSnap(1, 0, 0), rayThrough(1, 0))
    tool.cancel()
    expect(tool.statusHint()).toContain('new origin')
  })
})

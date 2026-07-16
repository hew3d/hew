import { describe, it, expect, vi } from 'vitest'
import * as THREE from 'three'
import { ScaleTool } from './ScaleTool'
import type { Snap } from './types'
import type { Ray } from '../viewport/math'
import type { NodeRef } from '../panels/treeModel'

function rayThrough(x: number, y: number): Ray {
  return { origin: [x, y, 5], direction: [0, 0, -1] }
}

function makeSnap(overrides: Partial<Snap> = {}): Snap {
  return { x: 0, y: 0, z: 0, kind: 'ground', ...overrides }
}

function makeWasmScene() {
  return {
    transform_selection: vi.fn(),
  }
}

function makeTool(selection: NodeRef[] = []) {
  const preview = new THREE.Group()
  const onCommit = vi.fn()
  const onToast = vi.fn()
  const onMeasurement = vi.fn()
  const wasmScene = makeWasmScene()
  const tool = new ScaleTool(
    wasmScene as never,
    preview,
    null, // objectsGroup — no ghost mesh in logic tests
    selection,
    onCommit,
    onToast,
    null,
    onMeasurement,
  )
  return { tool, onCommit, onToast, onMeasurement, wasmScene }
}

describe('ScaleTool — auto-select on click', () => {
  // Deliberate contract change (selection-UX overhaul): see MoveTool.test.ts.
  it('empty selection: the first click acquires the node under the cursor and starts the drag', () => {
    const { tool, onToast } = makeTool([])
    const acquire = vi.fn(() => [{ kind: 'object', id: 9n } as NodeRef])
    tool.setSelectionAcquirer(acquire)

    tool.onPointerDown(makeSnap({ x: 1, y: 0, z: 0 }), rayThrough(1, 0))

    expect(acquire).toHaveBeenCalledTimes(1)
    expect(onToast).not.toHaveBeenCalled()
    expect(tool.capturingInput()).toBe(true) // dragging — the scale began
  })

  it('a genuine miss (acquirer returns null) toasts and stays idle', () => {
    const { tool, onToast } = makeTool([])
    tool.setSelectionAcquirer(() => null)
    tool.onPointerDown(makeSnap(), rayThrough(0, 0))
    expect(onToast).toHaveBeenCalledWith('Click an object to scale it')
    expect(tool.capturingInput()).toBe(false)
  })

  it('idle status hint matches the selection state (empty → "click the object")', () => {
    expect(makeTool([]).tool.statusHint()).toBe('Click the object you want to scale.')
    expect(makeTool([{ kind: 'object', id: 1n }]).tool.statusHint())
      .toBe('Click a base point to scale the selection about its center.')
  })
})

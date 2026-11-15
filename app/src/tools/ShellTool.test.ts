/**
 * ShellTool unit tests: picking an object by clicking a face, toggling open
 * faces, the default and typed wall thickness, the ghost, commit, and the
 * kernel's refusals. A fake scene stands in for wasm.
 */
import { describe, it, expect, vi } from 'vitest'
import * as THREE from 'three'
import { ShellTool, shellSpecJson } from './ShellTool'
import type { NodeRef } from '../panels/treeModel'
import type { Scene as WasmScene } from '../wasm/loader'
import type { Ray } from '../viewport/math'
import { formatLength } from '../settings/units'

const RAY: Ray = { origin: [0, 0, 5], direction: [0, 0, -1] }
const BIG = 18446744073709551000n // above 2^53: must survive the JSON spec

/** A 1 m cube's vertex bounds, enough for the default wall. */
const CUBE = new Float32Array([0, 0, 0, 1, 1, 1])
const RING = new Float32Array([0, 0, 1, 1, 0, 1, 1, 1, 1, 0, 1, 1])

function previewMesh() {
  return {
    positions: () => new Float32Array([0, 0, 0, 1, 0, 0, 0, 1, 0]),
    normals: () => new Float32Array([0, 0, 1, 0, 0, 1, 0, 0, 1]),
    indices: () => new Uint32Array([0, 1, 2]),
    edge_positions: () => new Float32Array(),
    free: vi.fn(),
  }
}

function makeScene(opts: { hit?: { object: bigint; face: bigint; instance?: bigint } | null; previewThrows?: string; shellThrows?: string } = {}) {
  const hit = opts.hit === undefined ? { object: 7n, face: BIG } : opts.hit
  const shellCalls: unknown[][] = []
  const previewCalls: unknown[][] = []
  const scene = {
    pick_face: () =>
      hit === null
        ? undefined
        : { object: () => hit.object, face: () => hit.face, instance: () => hit.instance, free: vi.fn() },
    object_mesh: () => ({ positions: () => CUBE, free: vi.fn() }),
    face_boundary: () => RING,
    node_parent: () => undefined,
    shell_preview: vi.fn((...args: unknown[]) => {
      previewCalls.push(args)
      if (opts.previewThrows !== undefined) throw new Error(opts.previewThrows)
      return previewMesh()
    }),
    shell: vi.fn((...args: unknown[]) => {
      shellCalls.push(args)
      if (opts.shellThrows !== undefined) throw new Error(opts.shellThrows)
      return {
        objects_touched: () => new BigUint64Array([7n]),
        components_touched: () => new BigUint64Array(),
        instances_touched: () => new BigUint64Array(),
        free: vi.fn(),
      }
    }),
  }
  return { scene, shellCalls, previewCalls }
}

function makeTool(selection: NodeRef[] = [], opts: Parameters<typeof makeScene>[0] = {}) {
  const { scene, shellCalls, previewCalls } = makeScene(opts)
  const preview = new THREE.Group()
  const onCommit = vi.fn()
  const onToast = vi.fn()
  const onMeasurement = vi.fn()
  const tool = new ShellTool(scene as unknown as WasmScene, preview, selection, onCommit, onToast, onMeasurement)
  return { tool, scene, preview, shellCalls, previewCalls, onCommit, onToast, onMeasurement }
}

const key = (k: string) => ({ key: k, repeat: false }) as KeyboardEvent
const type = (tool: ShellTool, text: string) => {
  for (const k of text) tool.onKey(key(k))
}

describe('shellSpecJson', () => {
  it('writes face handles above 2^53 exactly', () => {
    expect(shellSpecJson([BIG, 3n], 0.01)).toBe(`{"open":[${BIG},3],"thickness":0.01}`)
    expect(shellSpecJson([], -0.5)).toBe('{"open":[],"thickness":-0.5}')
  })
})

describe('ShellTool', () => {
  it('starts with a hint, takes no digits until an object is chosen', () => {
    const { tool } = makeTool()
    expect(tool.statusHint()).toContain('Click a face')
    expect(tool.capturesKey('5')).toBe(false)
    expect(tool.hasArmedGesture()).toBe(false)
  })

  it('a click on a face chooses its object, opens the face, and previews at a default wall', () => {
    const { tool, previewCalls, preview } = makeTool()
    tool.onPointerDown(null, RAY)
    expect(previewCalls).toHaveLength(1)
    // A 1 m cube gets a 5 cm wall (a twentieth, rounded to a plain number).
    expect(previewCalls[0][0]).toBe(7n)
    expect(previewCalls[0][1]).toBe(`{"open":[${BIG}],"thickness":0.05}`)
    expect(preview.children.length).toBeGreaterThan(0)
    expect(tool.capturesKey('5')).toBe(true)
    expect(tool.capturesKey(' ')).toBe(false)
    expect(tool.statusHint()).toContain(`Wall ${formatLength(0.05)}`)
  })

  it('clicking an open face again closes it', () => {
    const { tool, previewCalls } = makeTool()
    tool.onPointerDown(null, RAY)
    tool.onPointerDown(null, RAY)
    expect(previewCalls[1][1]).toBe('{"open":[],"thickness":0.05}')
  })

  it('a selected object starts closed, with the ghost already up', () => {
    const { tool, previewCalls } = makeTool([{ kind: 'object', id: 7n }])
    expect(previewCalls[0][1]).toBe('{"open":[],"thickness":0.05}')
    expect(tool.hasArmedGesture()).toBe(true)
  })

  it('does not take up the object it just hollowed again when the selection comes back (playtest 4.2.7)', () => {
    // The app re-sends the selection after a commit; the shelled object is
    // still selected. Taking it up again previewed a second shell of the
    // hollow solid and left its refusal in the status bar.
    const { tool, previewCalls, shellCalls } = makeTool([{ kind: 'object', id: 7n }])
    type(tool, '10mm')
    tool.onKey(key('Enter'))
    expect(shellCalls).toHaveLength(1)
    const before = previewCalls.length
    tool.setSelection([{ kind: 'object', id: 7n }])
    expect(previewCalls).toHaveLength(before)
    expect(tool.hasArmedGesture()).toBe(false)
    expect(tool.statusHint()).toBe('Click a face to hollow out its object. Faces you click are left open.')
    // Choosing it again by hand still works, and another selection adopts.
    tool.setSelection([])
    tool.setSelection([{ kind: 'object', id: 7n }])
    expect(tool.hasArmedGesture()).toBe(true)
  })

  it('a fresh tool does not adopt an already hollow selection and show a stale refusal', () => {
    // The tool is made again on each activation; the hollowed object is
    // still selected, and the kernel refuses to shell it a second time.
    const { tool } = makeTool([{ kind: 'object', id: 7n }], { previewThrows: 'WallPassesThroughItself: wall' })
    expect(tool.hasArmedGesture()).toBe(false)
    expect(tool.statusHint()).toBe('Click a face to hollow out its object. Faces you click are left open.')
  })

  it('undo or redo lets the selection be taken up again', () => {
    const { tool } = makeTool([{ kind: 'object', id: 7n }])
    type(tool, '10mm')
    tool.onKey(key('Enter'))
    tool.setSelection([{ kind: 'object', id: 7n }])
    expect(tool.hasArmedGesture()).toBe(false)
    tool.onHistoryChanged()
    tool.setSelection([{ kind: 'object', id: 7n }])
    expect(tool.hasArmedGesture()).toBe(true)
  })

  it('a typed thickness updates the ghost, and Enter commits it', () => {
    const { tool, previewCalls, shellCalls, onCommit } = makeTool()
    tool.onPointerDown(null, RAY)
    type(tool, '2cm')
    expect(previewCalls[previewCalls.length - 1][1]).toBe(`{"open":[${BIG}],"thickness":0.02}`)
    tool.onKey(key('Enter'))
    expect(shellCalls).toHaveLength(1)
    expect(shellCalls[0]).toEqual([7n, `{"open":[${BIG}],"thickness":0.02}`])
    expect(onCommit).toHaveBeenCalledWith({ objectIds: [7n], componentIds: [], instanceIds: [] })
    // The tool is ready for the next object.
    expect(tool.hasArmedGesture()).toBe(false)
  })

  it('a negative thickness puts the wall outside', () => {
    const { tool, shellCalls } = makeTool()
    tool.onPointerDown(null, RAY)
    type(tool, '-1cm')
    expect(tool.statusHint()).toContain('(outside)')
    tool.onKey(key('Enter'))
    expect(shellCalls[0][1]).toContain('"thickness":-0.01')
  })

  it('Enter with nothing typed commits at the default wall', () => {
    const { tool, shellCalls } = makeTool()
    tool.onPointerDown(null, RAY)
    tool.onKey(key('Enter'))
    expect(shellCalls[0][1]).toBe(`{"open":[${BIG}],"thickness":0.05}`)
  })

  it('shows the kernel refusal in the status bar and no ghost', () => {
    const { tool, preview } = makeTool([], { previewThrows: 'WallTooThick: no room' })
    tool.onPointerDown(null, RAY)
    expect(tool.statusHint()).toContain(`A ${formatLength(0.05)} wall leaves no room here.`)
    expect(preview.children.filter((c) => c.name === 'MeshPreview')).toHaveLength(0)
  })

  it('toasts a refused commit and keeps the choice', () => {
    const { tool, onToast, onCommit } = makeTool([], { shellThrows: 'WallTooThick: no room' })
    tool.onPointerDown(null, RAY)
    tool.onKey(key('Enter'))
    expect(onToast).toHaveBeenCalledWith(`A ${formatLength(0.05)} wall leaves no room here.`, 'WallTooThick')
    expect(onCommit).not.toHaveBeenCalled()
    expect(tool.hasArmedGesture()).toBe(true)
  })

  it('refuses a face that belongs to a component at the top level', () => {
    const { tool, onToast, previewCalls } = makeTool([], { hit: { object: 7n, face: 4n, instance: 9n } })
    tool.onPointerDown(null, RAY)
    expect(onToast).toHaveBeenCalledOnce()
    expect(previewCalls).toHaveLength(0)
  })

  it('Escape clears a half-typed value first, then the whole choice', () => {
    const { tool } = makeTool()
    tool.onPointerDown(null, RAY)
    type(tool, '3')
    tool.onKey(key('Escape'))
    expect(tool.hasArmedGesture()).toBe(true)
    tool.onKey(key('Escape'))
    expect(tool.hasArmedGesture()).toBe(false)
  })

  it('activate() redraws the ghost an outgoing tool cancel() wiped', () => {
    const { tool, preview } = makeTool([{ kind: 'object', id: 7n }])
    expect(preview.children.length).toBeGreaterThan(0)
    preview.clear() // the outgoing tool's clearPreview
    tool.activate()
    expect(preview.children.length).toBeGreaterThan(0)
  })

  it('a click that hits nothing explains once', () => {
    const { tool, onToast } = makeTool([], { hit: null })
    tool.onPointerDown(null, RAY)
    tool.onPointerDown(null, RAY)
    expect(onToast).toHaveBeenCalledTimes(1)
  })
})

/**
 * A stateful fake for the multi-click cases: the object and face under the
 * ray can change between clicks, and an undo re-mints every handle the way
 * the kernel does (so a retype that reuses a stale handle fails loudly).
 */
function makeLiveScene() {
  const state = { epoch: 0n, other: false, historyGen: 0n, depth: 0, shellThrows: null as string | null }
  const commits: Array<{ object: bigint; spec: { open: number[]; thickness: number } }> = []
  const previews: Array<{ object: bigint; spec: { open: number[]; thickness: number } }> = []
  const faceOf = (ray: number) => (state.other ? 50n : 1n + 10n * state.epoch) + BigInt(ray)
  const objectNow = () => (state.other ? 9n : 7n + 100n * state.epoch)
  const parse = (s: string) => JSON.parse(s.replace(/(\d{16,})/g, '"$1"')) as { open: string[]; thickness: number }
  const scene = {
    // The first click lands on face 0, any later one on face 1.
    pick_face: (_ox: number, _oy: number, oz: number) => {
      const index = oz > 6 ? 1 : 0
      return { object: () => objectNow(), face: () => faceOf(index), instance: () => undefined, free: vi.fn() }
    },
    object_mesh: () => ({ positions: () => CUBE, free: vi.fn() }),
    face_boundary: () => RING,
    node_parent: () => undefined,
    shell_preview: vi.fn((object: bigint, spec: string) => {
      const p = parse(spec)
      previews.push({ object, spec: { open: p.open.map(Number), thickness: p.thickness } })
      return previewMesh()
    }),
    shell: vi.fn((object: bigint, spec: string) => {
      const p = parse(spec)
      const open = p.open.map(Number)
      for (const f of open) {
        const live = state.other ? f >= 50 && f < 56 : f >= 1 + 10 * Number(state.epoch) && f < 7 + 10 * Number(state.epoch)
        if (!live) throw new Error(`StaleHandle: face ${f} is gone`)
      }
      if (state.shellThrows !== null) throw new Error(state.shellThrows)
      commits.push({ object, spec: { open, thickness: p.thickness } })
      state.historyGen += 1n
      state.depth += 1
      return {
        objects_touched: () => new BigUint64Array([object]),
        components_touched: () => new BigUint64Array(),
        instances_touched: () => new BigUint64Array(),
        free: vi.fn(),
      }
    }),
    history_generation: () => state.historyGen,
    undo_depth: () => state.depth,
    scene_undo: vi.fn(() => {
      state.historyGen += 1n
      state.depth -= 1
      state.epoch += 1n
      return { free: vi.fn() }
    }),
    scene_redo: vi.fn(() => {
      state.historyGen += 1n
      state.depth += 1
      return { free: vi.fn() }
    }),
  }
  const preview = new THREE.Group()
  const onCommit = vi.fn()
  const onToast = vi.fn()
  const tool = new ShellTool(scene as unknown as WasmScene, preview, [], onCommit, onToast, vi.fn())
  const high: Ray = { origin: [0, 0, 9], direction: [0, 0, -1] }
  return { tool, scene, state, commits, previews, onCommit, onToast, low: RAY, high }
}

describe('ShellTool — switching solids and retyping', () => {
  it('clicking a face of another solid resets the open set to that face alone', () => {
    const t = makeLiveScene()
    t.tool.onPointerDown(null, t.low) // object 7, face 1 opens
    t.tool.onPointerDown(null, t.high) // object 7, face 2 opens too
    expect(t.previews.at(-1)!.spec.open).toEqual([1, 2])
    t.state.other = true
    t.tool.onPointerDown(null, t.high) // a face of object 9
    const last = t.previews.at(-1)!
    expect(last.object).toBe(9n)
    expect(last.spec.open).toEqual([51])
    type(t.tool, '1cm')
    t.tool.onKey(key('Enter'))
    expect(t.commits).toEqual([{ object: 9n, spec: { open: [51], thickness: 0.01 } }])
  })
})

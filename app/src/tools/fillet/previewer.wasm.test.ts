/**
 * The blend previewer contract: one code path consumes a direct answer and
 * an asynchronous one, latest wins, a cancelled answer is dropped, and a
 * tool fed by an asynchronous previewer makes the same commit as one fed by
 * the synchronous previewer. Driven through the real compiled kernel.
 */

import { readFile } from 'node:fs/promises'
import { resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import * as THREE from 'three'
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest'
import init, { Scene } from '../../wasm/pkg/wasm_api.js'
import { FilletTool, type FilletHooks } from '../FilletTool'
import { resetFilletSettingsForTest } from '../filletSettings'
import { setLengthUnit } from '../../settings/units'
import type { Ray } from '../../viewport/math'
import {
  SyncBlendPreviewer,
  whenReady,
  type BlendPreviewRequest,
  type BlendPreviewResult,
  type BlendPreviewer,
} from './previewer'

const __dirname = fileURLToPath(new URL('.', import.meta.url))

beforeAll(async () => {
  const wasmBytes = await readFile(resolve(__dirname, '../../wasm/pkg/wasm_api_bg.wasm'))
  await init({ module_or_path: wasmBytes })
})

beforeEach(() => {
  resetFilletSettingsForTest()
  setLengthUnit('mm')
})

afterEach(() => {
  setLengthUnit('m')
})

function makeBox(scene: Scene, w: number, d: number, h: number): bigint {
  const sketch = scene.begin_sketch_on_plane(0, 0, 0, 0, 0, 1)
  const c: Array<[number, number]> = [
    [0, 0],
    [w, 0],
    [w, d],
    [0, d],
  ]
  for (let i = 0; i < 4; i++) {
    const a = c[i]
    const b = c[(i + 1) % 4]
    scene.sketch_add_segment(sketch, a[0], a[1], 0, b[0], b[1], 0)
  }
  const regions = Array.from(scene.sketch_regions(sketch))
  return scene.extrude_region(sketch, regions[0], h)
}

const down = (x: number, y: number): Ray => ({ origin: [x, y, 10], direction: [0, 0, -1] })
const key = (k: string): KeyboardEvent => ({ key: k }) as KeyboardEvent

/** Answers like the synchronous previewer, but later: each answer waits for
 *  `settle`. `cooperative` previewers resolve what is in flight to
 *  `'cancelled'` on `cancel()`; the others leave stale answers to arrive. */
class DeferredPreviewer implements BlendPreviewer {
  readonly pending: Array<{ answer: BlendPreviewResult; resolve: (r: BlendPreviewResult | 'cancelled') => void }> = []
  cancels = 0
  private readonly inner: SyncBlendPreviewer
  constructor(
    scene: Scene,
    private readonly cooperative: boolean,
  ) {
    this.inner = new SyncBlendPreviewer(scene, () => 0)
  }
  request(req: BlendPreviewRequest): Promise<BlendPreviewResult | 'cancelled'> {
    const answer = this.inner.request(req)
    return new Promise((res) => this.pending.push({ answer, resolve: res }))
  }
  cancel(): void {
    this.cancels++
    if (!this.cooperative) return
    for (const p of this.pending.splice(0)) p.resolve('cancelled')
  }
  dispose(): void {}
  /** Answers request number `i` (in request order) and lets the tool see it. */
  async settle(i: number): Promise<void> {
    this.pending[i].resolve(this.pending[i].answer)
    await Promise.resolve()
    await Promise.resolve()
  }
  async settleAll(): Promise<void> {
    for (let i = 0; i < this.pending.length; i++) await this.settle(i)
    this.pending.length = 0
  }
}

function setup(previewerFor?: (scene: Scene) => BlendPreviewer) {
  const scene = new Scene()
  const group = new THREE.Group()
  const hooks = {
    onCommit: vi.fn(),
    onSketchChange: vi.fn(),
    onToast: vi.fn(),
    onMeasurement: vi.fn(),
    requestRender: vi.fn(),
    setFaceMask: vi.fn(() => true),
    clearFaceMask: vi.fn(),
  }
  const previewer = previewerFor?.(scene)
  const tool = new FilletTool(scene, group, hooks as unknown as FilletHooks, { schedule: (cb) => cb(), previewer })
  const box = makeBox(scene, 1, 1, 0.5)
  return { scene, group, hooks, tool, box, previewer }
}
type H = ReturnType<typeof setup>

function arm(h: H): void {
  h.tool.onPointerScreenMove(100, 100)
  h.tool.onPointerMove(null, down(0.5, 0.004))
  h.tool.onPointerDown(null, down(0.5, 0.004))
  h.tool.onPointerUp(null, down(0.5, 0.004))
}

function handleRay(h: H, size: number): Ray {
  const f = h.tool.gizmoHandle()
  if (f === null) throw new Error('no radius handle')
  return down(f.base[0] + f.axis[0] * size, f.base[1] + f.axis[1] * size)
}

function pressHandle(h: H): void {
  const m = h.tool.gizmoMarker({ kind: 'radius' })
  if (m === null) throw new Error('no radius marker')
  h.tool.onPointerScreenMove(100, 100)
  h.tool.onPointerMove(null, down(m[0], m[1]))
  h.tool.onPointerDown(null, down(m[0], m[1]))
}

function moveHandle(h: H, size: number, y: number): void {
  h.tool.onPointerScreenMove(100, y)
  h.tool.onPointerMove(null, handleRay(h, size))
}

function patchPositions(h: H): number[] | null {
  const mesh = h.group.getObjectByName('FilletPatch') as THREE.Mesh | undefined
  return mesh === undefined ? null : Array.from(mesh.geometry.getAttribute('position').array as Float32Array)
}

function patchCount(h: H): number {
  let n = 0
  h.group.traverse((o) => {
    if (o.name === 'FilletPatch') n++
  })
  return n
}

const result = (n: number): BlendPreviewResult => ({
  kind: 'ok',
  positions: new Float32Array([n]),
  normals: new Float32Array(),
  indices: new Uint32Array(),
  rails: new Float32Array(),
  replacedFaces: [],
  clamped: false,
  maxSize: 0,
  limitedBy: '',
  threeWayCorners: 0,
  elapsedMs: 0,
})

describe('whenReady', () => {
  it('applies a direct answer at once and returns what apply returns', () => {
    const apply = vi.fn((r: BlendPreviewResult) => (r.kind === 'ok' ? r.positions[0] : -1))
    const out = whenReady(result(7), () => false, apply)
    expect(out).toBe(7)
    expect(apply).toHaveBeenCalledTimes(1)
  })

  it('applies a promised answer only while its request is still the latest', async () => {
    let latest = 1
    const apply = vi.fn((r: BlendPreviewResult) => (r.kind === 'ok' ? r.positions[0] : -1))
    const first = whenReady(Promise.resolve(result(1)), () => latest === 1, apply)
    latest = 2
    const second = whenReady(Promise.resolve(result(2)), () => latest === 2, apply)
    expect(await first).toBe('cancelled')
    expect(await second).toBe(2)
    expect(apply).toHaveBeenCalledTimes(1)
  })

  it('treats a cancelled answer as a no-op', async () => {
    const apply = vi.fn(() => 1)
    expect(await whenReady(Promise.resolve<'cancelled'>('cancelled'), () => true, apply)).toBe('cancelled')
    expect(apply).not.toHaveBeenCalled()
  })
})

describe('SyncBlendPreviewer', () => {
  it('answers with plain data, no kernel handles', () => {
    const h = setup()
    const sync = new SyncBlendPreviewer(h.scene, () => 0)
    const edges = h.scene.object_sharp_edges(h.box)
    const spec = JSON.stringify({ edges: [String(edges[0])], profile: { kind: 'round', radius: 0.05, segments: 8 }, corners: 'default' })
    const r = sync.request({ kind: 'create', object: h.box, spec, typed: false })
    if (r.kind !== 'ok') throw new Error(`refused: ${r.message}`)
    expect(r.positions).toBeInstanceOf(Float32Array)
    expect(r.indices).toBeInstanceOf(Uint32Array)
    expect(r.rails.length).toBeGreaterThan(0)
    expect(r.replacedFaces.length).toBeGreaterThan(0)
    expect(structuredClone(r)).toEqual(r)
  })

  it('reports a refusal as text and code, with the elapsed time', () => {
    const h = setup()
    let t = 0
    const sync = new SyncBlendPreviewer(h.scene, () => (t += 5))
    const r = sync.request({ kind: 'create', object: h.box, spec: '{"edges":["999999"]}', typed: false })
    expect(r.kind).toBe('refused')
    if (r.kind === 'refused') {
      expect(typeof r.message).toBe('string')
      expect(r.elapsedMs).toBe(5)
    }
  })
})

describe('the tool with an asynchronous previewer', () => {
  it('draws nothing until the answer arrives, then draws what the synchronous tool draws', async () => {
    const sync = setup()
    arm(sync)
    const want = patchPositions(sync)
    expect(want).not.toBeNull()

    const h = setup((s) => new DeferredPreviewer(s, true))
    const p = h.previewer as DeferredPreviewer
    arm(h)
    expect(patchPositions(h)).toBeNull()
    await p.settleAll()
    expect(patchPositions(h)).toEqual(want)
    expect(h.hooks.requestRender).toHaveBeenCalled()
  })

  it('applies only the latest answer, even when an older one arrives last', async () => {
    const sync = setup()
    arm(sync)
    pressHandle(sync)
    moveHandle(sync, 0.04, 140)
    moveHandle(sync, 0.08, 160)
    const want = patchPositions(sync)

    // A previewer that does not cancel on its own: the tool's own check drops
    // the stale answers.
    const h = setup((s) => new DeferredPreviewer(s, false))
    const p = h.previewer as DeferredPreviewer
    arm(h)
    pressHandle(h)
    moveHandle(h, 0.04, 140)
    moveHandle(h, 0.08, 160)
    expect(p.pending.length).toBe(3)
    await p.settle(2)
    expect(patchPositions(h)).toEqual(want)
    await p.settle(1)
    await p.settle(0)
    expect(patchPositions(h)).toEqual(want)
    expect(patchCount(h)).toBe(1)
  })

  it('drops an answer that is in flight when the tool is cancelled', async () => {
    const h = setup((s) => new DeferredPreviewer(s, true))
    const p = h.previewer as DeferredPreviewer
    arm(h)
    const cancelsBefore = p.cancels
    h.tool.onKey(key('Escape'))
    expect(p.cancels).toBeGreaterThan(cancelsBefore)
    await p.settleAll()
    expect(patchPositions(h)).toBeNull()
    expect(h.hooks.setFaceMask).not.toHaveBeenCalled()
  })

  it('drops an answer that arrives after the tool was cancelled, even from an uncooperative previewer', async () => {
    const h = setup((s) => new DeferredPreviewer(s, false))
    const p = h.previewer as DeferredPreviewer
    arm(h)
    h.tool.onKey(key('Escape'))
    await p.settleAll()
    expect(patchPositions(h)).toBeNull()
    expect(h.hooks.setFaceMask).not.toHaveBeenCalled()
  })

  const gestures: Array<[string, (h: H) => Promise<void>]> = [
    [
      'a dragged size',
      async (h) => {
        arm(h)
        pressHandle(h)
        moveHandle(h, 0.05, 140)
        h.tool.onPointerUp(null, handleRay(h, 0.05))
        await (h.previewer as DeferredPreviewer | undefined)?.settleAll()
        h.tool.onKey(key('Enter'))
      },
    ],
    [
      'a typed size',
      async (h) => {
        arm(h)
        for (const k of ['2', '0']) h.tool.onKey(key(k))
        await (h.previewer as DeferredPreviewer | undefined)?.settleAll()
        h.tool.onKey(key('Enter'))
      },
    ],
  ]
  for (const [name, gesture] of gestures) {
    it(`commits ${name} exactly as the synchronous tool does`, async () => {
      const a = setup()
      const aEdges = vi.spyOn(a.scene, 'blend_edges')
      await gesture(a)
      expect(aEdges).toHaveBeenCalledTimes(1)

      const b = setup((s) => new DeferredPreviewer(s, true))
      const bEdges = vi.spyOn(b.scene, 'blend_edges')
      await gesture(b)
      expect(bEdges).toHaveBeenCalledTimes(1)
      expect(bEdges.mock.calls[0]).toEqual(aEdges.mock.calls[0])
      expect(b.hooks.onCommit).toHaveBeenCalledWith([b.box])
    })
  }
})

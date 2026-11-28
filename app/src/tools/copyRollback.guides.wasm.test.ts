/**
 * A Move or Rotate copy that carries guides and is rolled back, against the
 * REAL compiled wasm kernel: the history bracket takes the guide copies back
 * with everything else, so the tool must not select them.
 */

import { readFile } from 'node:fs/promises'
import { resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import * as THREE from 'three'
import { beforeAll, describe, expect, it, vi } from 'vitest'
import init, { Scene } from '../wasm/pkg/wasm_api.js'
import { MoveTool } from './MoveTool'
import { RotateTool } from './RotateTool'
import type { Snap } from './types'
import type { Ray } from '../viewport/math'
import type { NodeRef } from '../panels/treeModel'

vi.mock('../platform', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../platform')>()
  return { ...actual, isMac: true, COPY_MODIFIER_KEY: 'Alt', COPY_MODIFIER_LABEL: 'Option' }
})

const __dirname = fileURLToPath(new URL('.', import.meta.url))

beforeAll(async () => {
  const wasmBytes = await readFile(resolve(__dirname, '../wasm/pkg/wasm_api_bg.wasm'))
  await init({ module_or_path: wasmBytes })
})

const rayAt = (x: number, y = 0): Ray => ({ origin: [x, y, 5], direction: [0, 0, -1] })
const snapAt = (x: number, y = 0): Snap => ({ x, y, z: 0, kind: 'ground' })
const altKey = { key: 'Alt', repeat: false, preventDefault: () => { /* no-op */ } } as unknown as KeyboardEvent

/** A whole drawn square, so the copy runs inside a history bracket, and a
 *  guide beside it. The bracket's commit refuses. */
function refusingScene() {
  const scene = new Scene()
  const sketch = scene.begin_sketch_on_plane(0, 0, 0, 0, 0, 1)
  const c: Array<[number, number]> = [[0, 0], [1, 0], [1, 1], [0, 1]]
  for (let i = 0; i < 4; i++) {
    const a = c[i]
    const b = c[(i + 1) % 4]
    scene.sketch_add_segment(sketch, a[0], a[1], 0, b[0], b[1], 0)
  }
  const guide = scene.add_guide_point(3, 0, 0)
  const nodes: NodeRef[] = [{ kind: 'sketch', id: sketch }]
  scene.history_bracket_commit = () => {
    throw new Error('InvalidTransform: refused')
  }
  return { scene, nodes, guide, before: scene.state_hash(), depth: scene.undo_depth() }
}

describe('a rolled-back copy carrying guides', () => {
  it('Move lands nothing: no commit, so the selection is left as it was', () => {
    const s = refusingScene()
    const onCommit = vi.fn()
    const onToast = vi.fn()
    const tool = new MoveTool(s.scene, new THREE.Group(), null, s.nodes, onCommit, onToast)
    tool.setCarriedGuides([s.guide])
    tool.onKey(altKey)
    tool.onPointerDown(snapAt(0), rayAt(0))
    tool.onPointerMove(snapAt(2), rayAt(2))
    tool.onPointerDown(snapAt(2), rayAt(2))
    expect(onToast).toHaveBeenCalledTimes(1)
    expect(onCommit).not.toHaveBeenCalled()
    expect(s.scene.state_hash()).toBe(s.before)
    expect(s.scene.undo_depth()).toBe(s.depth)
  })

  it('Rotate lands nothing either', () => {
    const s = refusingScene()
    const onCommit = vi.fn()
    const onToast = vi.fn()
    const tool = new RotateTool(s.scene, new THREE.Group(), null, s.nodes, onCommit, onToast)
    tool.setCarriedGuides([s.guide])
    tool.onKey(altKey)
    tool.onPointerDown(snapAt(0), rayAt(0)) // pivot
    tool.onPointerDown(snapAt(1), rayAt(1)) // reference arm
    tool.onPointerMove(snapAt(0, 1), rayAt(0, 1))
    tool.onPointerDown(snapAt(0, 1), rayAt(0, 1))
    expect(onToast).toHaveBeenCalledTimes(1)
    expect(onCommit).not.toHaveBeenCalled()
    expect(s.scene.state_hash()).toBe(s.before)
    expect(s.scene.undo_depth()).toBe(s.depth)
  })
})

/**
 * Wireframe faces R20 against the REAL compiled wasm kernel: a sketch Move
 * (and Copy) that closes an open cube reports the new solid in the draw
 * report shape, and the tool-side commit path surfaces one toast and selects
 * the solid. Needs a rebuilt `app/src/wasm/pkg` (the calls returned nothing
 * or a handle before R20).
 */

import { readFile } from 'node:fs/promises'
import { resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { afterEach, beforeAll, describe, expect, it } from 'vitest'
import init, { Scene } from '../wasm/pkg/wasm_api.js'
import type { Scene as WasmScene } from '../wasm/loader'
import { setDraftSolidSelector, surfaceSketchTransformReports } from './draftCommit'
import { commitSelectionTransform, duplicateSketchSelectionByAffineArray } from './transformSelection'
import { affineToFloat64, translationAffine } from './transformMath'
import { ctxOfPlane, ctxOfSketchOnly, draftRef, invalidatePlaneUnits, readPlaneUnits } from './planeUnits'
import type { NodeRef } from '../panels/treeModel'

const __dirname = fileURLToPath(new URL('.', import.meta.url))

beforeAll(async () => {
  const wasmBytes = await readFile(resolve(__dirname, '../wasm/pkg/wasm_api_bg.wasm'))
  await init({ module_or_path: wasmBytes })
})

afterEach(() => setDraftSolidSelector(null))

type V3 = [number, number, number]

function rawFace(scene: Scene, corners: V3[]): bigint {
  const [a, b, c] = corners
  const u: V3 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]]
  const v: V3 = [c[0] - a[0], c[1] - a[1], c[2] - a[2]]
  const n: V3 = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]]
  const s = scene.begin_sketch_on_plane(...a, ...n)
  scene.sketch_begin_gesture(s)
  for (let i = 0; i < corners.length; i++) {
    scene.sketch_add_segment(s, ...corners[i], ...corners[(i + 1) % corners.length])
  }
  scene.sketch_end_gesture(s)
  return s
}

/** Four walls and a floor of a unit cube, top open. */
function openCube(scene: Scene): void {
  const [a, b, c, d]: V3[] = [[0, 0, 0], [1, 0, 0], [1, 1, 0], [0, 1, 0]]
  const [a1, b1, c1, d1]: V3[] = [[0, 0, 1], [1, 0, 1], [1, 1, 1], [0, 1, 1]]
  rawFace(scene, [a, b, c, d])
  rawFace(scene, [a, b, b1, a1])
  rawFace(scene, [b, c, c1, b1])
  rawFace(scene, [c, d, d1, c1])
  rawFace(scene, [d, a, a1, d1])
}

const TOP_HIGH: V3[] = [[0, 0, 3], [1, 0, 3], [1, 1, 3], [0, 1, 3]]

/** The drawn-shape refs of a sketch, the way the selection holds them. */
function islandRefs(scene: Scene, sketch: bigint): NodeRef[] {
  const ctx = ctxOfSketchOnly(sketch)
  const units = readPlaneUnits(scene as unknown as WasmScene, ctx)?.units ?? []
  // Only the square at the origin: a second, far one stays behind so the
  // move is an island move (a lone island folds to a whole-sketch move).
  const mine = units.filter((u) => u.edges.some((e) => Math.abs(e.a[0]) < 1e-9 && Math.abs(e.a[1]) < 1e-9))
  expect(mine.length).toBeGreaterThan(0)
  return mine.map((u) => draftRef('draft-island', ctx, u.key))
}

/** The raised top square plus a second one far away on the same sketch. */
function topWithCompanion(scene: Scene): bigint {
  const top = rawFace(scene, TOP_HIGH)
  const far: V3[] = [[5, 5, 3], [6, 5, 3], [6, 6, 3], [5, 6, 3]]
  for (let i = 0; i < 4; i++) scene.sketch_add_segment(top, ...far[i], ...far[(i + 1) % 4])
  return top
}

describe('sketch transforms report like a draw (R20)', () => {
  it('a Move that closes the cube toasts once and selects the solid', () => {
    const scene = new Scene()
    openCube(scene)
    const top = topWithCompanion(scene)
    const toasts: string[] = []
    const selected: bigint[] = []
    setDraftSolidSelector((o) => selected.push(o))
    commitSelectionTransform(scene as unknown as WasmScene, islandRefs(scene, top), affineToFloat64(translationAffine(0, 0, -2)))
    surfaceSketchTransformReports((m) => toasts.push(m))
    expect(toasts).toEqual(['The closed shape is now a solid.'])
    expect(selected).toHaveLength(1)
  })

  it('a Move that closes nothing stays silent', () => {
    const scene = new Scene()
    openCube(scene)
    const top = topWithCompanion(scene)
    const toasts: string[] = []
    commitSelectionTransform(scene as unknown as WasmScene, islandRefs(scene, top), affineToFloat64(translationAffine(0, 0, 1)))
    surfaceSketchTransformReports((m) => toasts.push(m))
    expect(toasts).toEqual([])
  })

  it('a Copy that closes the cube toasts once and selects the solid', () => {
    const scene = new Scene()
    openCube(scene)
    const top = topWithCompanion(scene)
    const toasts: string[] = []
    const selected: bigint[] = []
    setDraftSolidSelector((o) => selected.push(o))
    duplicateSketchSelectionByAffineArray(
      scene as unknown as WasmScene,
      islandRefs(scene, top),
      [affineToFloat64(translationAffine(0, 0, -2))],
    )
    surfaceSketchTransformReports((m) => toasts.push(m))
    expect(toasts).toEqual(['The closed shape is now a solid.'])
    expect(selected).toHaveLength(1)
  })

  it('an in-plane Move by drafts that closes the cube toasts once and selects the solid', () => {
    const scene = new Scene()
    const w = scene as unknown as WasmScene
    // Floor and three walls drawn as drafts; the fourth wall is a closed
    // square on the plane x = 0, three meters away along y.
    const drawSquare = (plane: V3[], origin: V3, normal: V3) => {
      const d = scene.draft_begin_plane(...origin, ...normal, 'Draw')
      for (let i = 0; i < 4; i++) {
        scene.draft_add_segment(d, ...plane[i], ...plane[(i + 1) % 4]).free()
      }
      scene.draft_end(d)
    }
    drawSquare([[0, 0, 0], [1, 0, 0], [1, 1, 0], [0, 1, 0]], [0, 0, 0], [0, 0, 1])
    drawSquare([[0, 0, 0], [1, 0, 0], [1, 0, 1], [0, 0, 1]], [0, 0, 0], [0, 1, 0])
    drawSquare([[1, 0, 0], [1, 1, 0], [1, 1, 1], [1, 0, 1]], [1, 0, 0], [1, 0, 0])
    drawSquare([[0, 1, 0], [1, 1, 0], [1, 1, 1], [0, 1, 1]], [0, 1, 0], [0, 1, 0])
    drawSquare([[0, 3, 0], [0, 4, 0], [0, 4, 1], [0, 3, 1]], [0, 3, 0], [1, 0, 0])
    expect(scene.object_ids().length).toBe(0)
    invalidatePlaneUnits()
    const ctx = ctxOfPlane([0, 0, 0], [1, 0, 0]) as string
    const units = (readPlaneUnits(w, ctx)?.units ?? []).filter((u) => u.edges.some((e) => e.a[1] > 2.5 && e.b[1] > 2.5))
    expect(units.length).toBe(1)
    const toasts: string[] = []
    const selected: bigint[] = []
    setDraftSolidSelector((o) => selected.push(o))
    commitSelectionTransform(w, [draftRef('draft-island', ctx, units[0].key)], affineToFloat64(translationAffine(0, -3, 0)))
    surfaceSketchTransformReports((m) => toasts.push(m))
    expect(toasts).toEqual(['The closed shape is now a solid.'])
    expect(selected).toHaveLength(1)
  })
})

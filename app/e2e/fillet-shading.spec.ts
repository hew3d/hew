import { test, expect, type Page, type TestInfo } from '@playwright/test'
import { settleFrame } from './helpers/render'
import { requiresPreview } from './helpers/preview'

// Drives a Preview feature: skipped in the production-shape run (e2e/helpers/preview.ts).
requiresPreview('fillet')

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

/**
 * Smooth shading across a round's strips and corner ridges. The geometry is
 * unchanged: only the vertex normals the tessellator emits are. Each case
 * is built through the real kernel (the harness's blend call) and shot
 * close up; set HEW_E2E_SHOTS to keep the screenshots.
 */

async function setup(page: Page): Promise<void> {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 60_000 })
  await page.evaluate(() => window.__hew_test!.setLengthUnit('mm'))
  await page.waitForTimeout(100)
}

async function shot(page: Page, info: TestInfo, name: string): Promise<void> {
  await settleFrame(page)
  const dir = process.env.HEW_E2E_SHOTS
  await page.screenshot({ path: dir ? `${dir}/${name}.png` : info.outputPath(`${name}.png`) })
}

type Cam = { position: [number, number, number]; target: [number, number, number] }

async function look(page: Page, cam: Cam): Promise<void> {
  await page.evaluate(
    (c) => window.__hew_test!.setCamera({ ...c, up: [0, 0, 1], fovDeg: 40 }),
    cam,
  )
  await settleFrame(page)
}

test('Shading: the pipe tee fillet strip shades smooth, not in facets', async ({ page }, info) => {
  test.setTimeout(120_000)
  await setup(page)
  const id = await page.evaluate(() => {
    const t = window.__hew_test!
    const a = t.drawCircle([0.5, -0.5, 0], 0.1, 32)
    const pipe = t.extrudeRegion(a.sketch, a.region, 0.8)
    const b = t.drawCircle([0.5, 0.4, 0], 0.07, 32)
    const branch = t.extrudeRegion(b.sketch, b.region, 0.5)
    t.rotateObject(branch, 90, [1, 0, 0])
    const tee = t.boolean(0, pipe, branch)
    t.filletEdges(tee, [[0.533, -0.4056, 0.4617]], '{"kind":"round","radius":0.03,"segments":6}')
    return tee
  })
  expect((await page.evaluate((i) => window.__hew_test!.blendFeatures(i), id)).length).toBeGreaterThan(0)
  // Real assertions on the emitted vertex normals. Along the saddle strip's
  // rails (positions on the main pipe's wall, a radius-0.1 cylinder about
  // the axis through (0.5, -0.5)) the strip and the wall both report a
  // vertex, and they must report the same normal. A strip left to its flat
  // per-facet normals (the blend shading returning nothing) disagrees.
  const verts = await page.evaluate((i) => window.__hew_test!.getObjectVertexNormals(i), id)
  const byPos = new Map<string, Array<[number, number, number]>>()
  let offWall = 0
  for (const { p, n } of verts) {
    expect(Math.hypot(n[0], n[1], n[2])).toBeCloseTo(1, 3)
    const r = Math.hypot(p[0] - 0.5, p[1] + 0.5)
    if (Math.abs(r - 0.1) < 2e-4 && p[2] > 1e-3 && p[2] < 0.8 - 1e-3) {
      const k = p.map((c) => c.toFixed(5)).join(',')
      const g = byPos.get(k)
      if (g === undefined) byPos.set(k, [n])
      else g.push(n)
    } else if (r < 0.1 - 2e-4 && p[1] > -0.45 && Math.abs(n[2]) < 0.999) {
      offWall++
    }
  }
  let shared = 0
  for (const [k, ns] of byPos) {
    if (ns.length < 2) continue
    shared++
    for (const n of ns) {
      const dot = n[0] * ns[0][0] + n[1] * ns[0][1] + n[2] * ns[0][2]
      expect(Math.acos(Math.min(1, dot)) * (180 / Math.PI), `normals disagree at ${k}`).toBeLessThan(3)
    }
  }
  expect(shared).toBeGreaterThan(20)
  // The strip's interior joints exist and are shaded off the axes.
  expect(offWall).toBeGreaterThan(0)
  await look(page, { position: [0.78, -0.12, 0.72], target: [0.5, -0.4, 0.46] })
  await shot(page, info, 'shading-tee-close')
  await look(page, { position: [0.5, 0.9, 0.9], target: [0.5, -0.4, 0.4] })
  await shot(page, info, 'shading-tee-wide')
})

test('Shading: a round on a box rim shades smooth over the corner ridge, and the box stays crisp', async ({ page }, info) => {
  test.setTimeout(120_000)
  await setup(page)
  const boxId = await page.evaluate(() => {
    const t = window.__hew_test!
    const box = t.drawBox([0, 0, 0], [2, 2, 0], 1)
    t.filletEdges(
      box,
      [[1, 0, 1], [2, 1, 1], [1, 2, 1], [0, 1, 1]],
      '{"kind":"round","radius":0.25,"segments":6}',
    )
    return box
  })
  // The box's walls stay axis-aligned: a vertex on exactly one wall plane
  // carries that plane's normal; strip joints off every plane are not.
  const verts = await page.evaluate((i) => window.__hew_test!.getObjectVertexNormals(i), boxId)
  let flat = 0
  let smoothed = 0
  for (const { p, n } of verts) {
    expect(Math.hypot(n[0], n[1], n[2])).toBeCloseTo(1, 3)
    const planes: Array<[number, number]> = []
    for (let k = 0; k < 3; k++) {
      const hi = k === 2 ? 1 : 2
      if (Math.abs(p[k]) < 1e-5) planes.push([k, -1])
      if (Math.abs(p[k] - hi) < 1e-5) planes.push([k, 1])
    }
    if (planes.length === 1) {
      const [k, sign] = planes[0]
      // The top plane's rail vertices sit at z = 1 too: same rule.
      expect(n[k] * sign).toBeGreaterThan(0.999)
      flat++
    } else if (planes.length === 0 && n.every((c) => Math.abs(c) < 0.999)) {
      smoothed++
    }
  }
  expect(flat).toBeGreaterThan(8)
  expect(smoothed).toBeGreaterThan(0)
  await look(page, { position: [3.3, -1.3, 2.2], target: [1.7, 0.2, 0.9] })
  await shot(page, info, 'shading-rim-corner')
})

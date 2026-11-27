import { test, expect, type Page } from '@playwright/test'

/**
 * Hidden guides and annotations are not pickable through solids (issue #104).
 *
 * Guides, dimension lines and leaders are drawn depth-tested, so a solid in
 * front of one hides it; clicking the solid must take the solid, not the
 * hidden thing. The rule is the one the frame draws (viewport/
 * drawnVisibility.ts): see-through faces, X-ray/Wireframe, coplanar spots,
 * thin walls and cut-away section parts are covered here with REAL pointer
 * events. Each row asserts both directions: the solid wins while the thing is
 * hidden, the thing wins when it shows.
 *
 * Frame used throughout: a 2 x 2 x 2 box at the origin (x, y in -1..1, z in
 * 0..2). Seen from the front (camera on -y) a guide at y = 2, z = 1 is hidden
 * behind the box; seen from the back (camera on +y) it is in plain view.
 */

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

type V3 = [number, number, number]

const FRONT = { position: [0, -8, 1] as V3, target: [0, 0, 1] as V3, fovDeg: 50 }
const BACK = { position: [0, 8, 1] as V3, target: [0, 0, 1] as V3, fovDeg: 50 }

async function ready(page: Page): Promise<void> {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
}

async function camera(page: Page, pose: { position: V3; target: V3; fovDeg: number }): Promise<void> {
  await page.evaluate((p) => window.__hew_test!.setCamera(p), pose)
  await page.waitForTimeout(250)
}

/** Page pixel of a world point at the current camera. */
async function at(page: Page, w: V3): Promise<{ x: number; y: number }> {
  const s = await page.evaluate((p) => window.__hew_test!.worldToScreen(p), w)
  const box = await page.locator('canvas').first().boundingBox()
  if (box === null) throw new Error('no canvas')
  return { x: box.x + s.x, y: box.y + s.y }
}

async function clickWorld(page: Page, w: V3): Promise<void> {
  const p = await at(page, w)
  await page.mouse.move(p.x, p.y)
  await page.mouse.click(p.x, p.y)
  await page.waitForTimeout(200)
}

interface Picked { guide: string | null; annotation: string | null; nodes: string[] }
async function picked(page: Page): Promise<Picked> {
  return page.evaluate(() => {
    const h = window.__hew_test!
    return {
      guide: h.getSelectedGuide(),
      annotation: h.getSelectedAnnotation(),
      nodes: h.getSelection().map((n) => `${n.kind}:${n.id}`),
    }
  })
}

async function makeBox(page: Page): Promise<string> {
  return page.evaluate(() => {
    const h = window.__hew_test!
    h.setAxesVisible(false)
    h.setGridVisible(false)
    return h.drawBox([-1, -1, 0], [1, 1, 0], 2)
  })
}

/** The color at a world point's pixel, as "r,g,b" (the frame is rendered first). */
async function colorAt(page: Page, w: V3): Promise<string> {
  const c = await page.evaluate((p) => window.__hew_test!.pixelColorAt(p), w)
  if (c === null) throw new Error('point is off screen')
  return `${c.r},${c.g},${c.b}`
}

/** Row guard: box B (x 1..3) is still DRAWN, and drawn differently from the lit box A
 *  (x -3..-1) and from the empty background in the gap between them. Fails if the
 *  dimmed/faded box were hidden outright. */
async function expectFadedNotHidden(page: Page): Promise<void> {
  const bg = await colorAt(page, [0, -1, 1])
  const lit = await colorAt(page, [-2, -1, 1])
  const faded = await colorAt(page, [2, -1, 1])
  expect(faded).not.toBe(bg)
  expect(faded).not.toBe(lit)
}

async function clearSelection(page: Page): Promise<void> {
  // A click on empty canvas (top-left corner) clears node, guide and annotation selection.
  const box = await page.locator('canvas').first().boundingBox()
  if (box === null) throw new Error('no canvas')
  await page.mouse.click(box.x + 6, box.y + 6)
  await page.waitForTimeout(150)
  const p = await picked(page)
  expect(p.guide === null && p.annotation === null && p.nodes.length === 0, 'selection cleared').toBe(true)
}

test.beforeEach(async ({ page }) => {
  await ready(page)
})

test.describe('row 1-2: a hidden guide or annotation does not take the click', () => {
  test('line guide', async ({ page }) => {
    const box = await makeBox(page)
    const guide = await page.evaluate(() => window.__hew_test!.addGuideLine(0, 2, 1, 1, 0, 0))
    await camera(page, FRONT)
    await clickWorld(page, [0.3, -1, 1])
    let p = await picked(page)
    expect(p.guide).toBeNull()
    expect(p.nodes).toEqual([`object:${box}`])
    await clearSelection(page)
    await camera(page, BACK)
    await clickWorld(page, [0.3, 2, 1])
    p = await picked(page)
    expect(p.guide).toBe(guide)
  })

  test('point guide', async ({ page }) => {
    const box = await makeBox(page)
    const guide = await page.evaluate(() => window.__hew_test!.addGuidePoint(0, 2, 1))
    await camera(page, FRONT)
    await clickWorld(page, [0, -1, 1])
    let p = await picked(page)
    expect(p.guide).toBeNull()
    expect(p.nodes).toEqual([`object:${box}`])
    await clearSelection(page)
    await camera(page, BACK)
    await clickWorld(page, [0, 2, 1])
    p = await picked(page)
    expect(p.guide).toBe(guide)
  })

  test('linear dimension, and its text editor does not open through the box', async ({ page }) => {
    const box = await makeBox(page)
    const dim = await page.evaluate(() =>
      window.__hew_test!.addLinearDimension([-0.8, 2, 1], [0.8, 2, 1], [0, 0.5, 0], [0, 2, 1, 0, -1, 0]))
    await camera(page, FRONT)
    await clickWorld(page, [0.3, -1, 1])
    let p = await picked(page)
    expect(p.annotation).toBeNull()
    expect(p.nodes).toEqual([`object:${box}`])
    const front = await at(page, [0.3, -1, 1])
    await page.mouse.dblclick(front.x, front.y)
    await page.waitForTimeout(250)
    await expect(page.getByTestId('annotation-editor')).toHaveCount(0)
    await page.keyboard.press('Escape')
    await clearSelection(page)
    await camera(page, BACK)
    await clickWorld(page, [0.3, 2.5, 1])
    p = await picked(page)
    expect(p.annotation).toBe(dim)
    const back = await at(page, [0.3, 2.5, 1])
    await page.mouse.dblclick(back.x, back.y)
    await expect(page.getByTestId('annotation-editor')).toHaveCount(1)
    await page.keyboard.press('Escape')
  })

  test('leader (radial dimension)', async ({ page }) => {
    const box = await makeBox(page)
    const dim = await page.evaluate(() =>
      window.__hew_test!.addRadialDimension([0, 2, 1], 'radius', [0, 2, 1.5], 0.5, [0, 2, 1, 0, 1, 0], [0.8, 0, 0]))
    await camera(page, FRONT)
    await clickWorld(page, [0.4, -1, 1])
    let p = await picked(page)
    expect(p.annotation).toBeNull()
    expect(p.nodes).toEqual([`object:${box}`])
    await clearSelection(page)
    await camera(page, BACK)
    await clickWorld(page, [0.4, 2, 1])
    p = await picked(page)
    expect(p.annotation).toBe(dim)
  })
})

test('row 3: a press on the box over a hidden guide drags the box', async ({ page }) => {
  const box = await makeBox(page)
  await page.evaluate(() => window.__hew_test!.addGuideLine(0, 2, 1, 1, 0, 0))
  await camera(page, FRONT)
  await clickWorld(page, [0.3, -1.0, 1.5]) // select the box on a spot clear of the guide's screen line
  const before = await page.evaluate((id) => window.__hew_test!.getObjectBounds(id), box)
  const a = await at(page, [0.3, -1, 1])
  await page.mouse.move(a.x, a.y)
  await page.mouse.down()
  await page.mouse.move(a.x + 60, a.y, { steps: 8 })
  await page.mouse.move(a.x + 120, a.y, { steps: 8 })
  await page.mouse.up()
  await page.waitForTimeout(200)
  const after = await page.evaluate((id) => window.__hew_test!.getObjectBounds(id), box)
  expect(after[0]).toBeGreaterThan(before[0] + 0.05)
  expect((await picked(page)).guide).toBeNull()
})

test.describe('row 4: Move, Rotate and Scale take the box over a hidden guide', () => {
  for (const [name, key] of [['Move', 'm'], ['Rotate', 'q']] as const) {
    test(`${name}: hover highlights the box; the guide when it shows`, async ({ page }) => {
      const box = await makeBox(page)
      const guide = await page.evaluate(() => window.__hew_test!.addGuideLine(0, 2, 1, 1, 0, 0))
      await camera(page, FRONT)
      await page.keyboard.press(key)
      const f = await at(page, [0.3, -1, 1])
      await page.mouse.move(f.x, f.y)
      await page.mouse.move(f.x + 1, f.y)
      await page.waitForTimeout(250)
      let hl = await page.evaluate(() => window.__hew_test!.getHoverHighlight())
      expect(hl?.guide ?? null).toBeNull()
      expect(hl?.objects).toContain(box)
      await camera(page, BACK)
      const b = await at(page, [0.3, 2, 1])
      await page.mouse.move(b.x, b.y)
      await page.mouse.move(b.x + 1, b.y)
      await page.waitForTimeout(250)
      hl = await page.evaluate(() => window.__hew_test!.getHoverHighlight())
      expect(hl?.guide).toBe(guide)
    })
  }

  test('Move: click-move moves the box', async ({ page }) => {
    const box = await makeBox(page)
    await page.evaluate(() => window.__hew_test!.addGuideLine(0, 2, 1, 1, 0, 0))
    await camera(page, FRONT)
    const before = await page.evaluate((id) => window.__hew_test!.getObjectBounds(id), box)
    await page.keyboard.press('m')
    const a = await at(page, [0.3, -1, 1])
    await page.mouse.move(a.x, a.y)
    await page.mouse.down()
    await page.mouse.up()
    await page.mouse.move(a.x + 80, a.y, { steps: 10 })
    await page.mouse.down()
    await page.mouse.up()
    await page.waitForTimeout(200)
    const after = await page.evaluate((id) => window.__hew_test!.getObjectBounds(id), box)
    expect(Math.abs(after[0] - before[0]) + Math.abs(after[1] - before[1]) + Math.abs(after[2] - before[2]))
      .toBeGreaterThan(0.05)
  })

  test('Scale: takes the box', async ({ page }) => {
    const box = await makeBox(page)
    await page.evaluate(() => window.__hew_test!.addGuideLine(0, 2, 1, 1, 0, 0))
    await camera(page, FRONT)
    await page.keyboard.press('s')
    await clickWorld(page, [0.3, -1, 1])
    const p = await picked(page)
    expect(p.guide).toBeNull()
    expect(p.nodes).toEqual([`object:${box}`])
  })
})

test('row 5: a glass box hides nothing, the guide behind it is picked', async ({ page }) => {
  const box = await makeBox(page)
  const guide = await page.evaluate((id) => {
    const h = window.__hew_test!
    const m = h.addMaterial('glass', 180, 200, 220, 90)
    h.paintObject(id, m)
    return h.addGuideLine(0, 2, 1, 1, 0, 0)
  }, box)
  await camera(page, FRONT)
  await clickWorld(page, [0.3, -1, 1])
  expect((await picked(page)).guide).toBe(guide)
})

test('row 6: edit session dims the context; its dimmed box hides nothing, the lit one does', async ({ page }) => {
  const setup = await page.evaluate(() => {
    const h = window.__hew_test!
    h.setAxesVisible(false)
    h.setGridVisible(false)
    const a = h.drawBox([-3, -1, 0], [-1, 1, 0], 2)
    const b = h.drawBox([1, -1, 0], [3, 1, 0], 2)
    const ga = h.groupNodes([{ kind: 'object', id: a }])
    h.groupNodes([{ kind: 'object', id: b }])
    return {
      a, b, ga,
      guideA: h.addGuidePoint(-2, 2, 1), // behind the lit box A
      guideB: h.addGuidePoint(2, 2, 1), // behind the dimmed box B
    }
  })
  await camera(page, { position: [0, -12, 1], target: [0, 0, 1], fovDeg: 50 })
  const p = await at(page, [-2, -1, 1])
  await page.mouse.move(p.x, p.y)
  await page.mouse.click(p.x, p.y, { clickCount: 2 })
  await page.waitForTimeout(300)
  await page.keyboard.press(' ')
  await clearSelection(page)
  await expectFadedNotHidden(page)
  await clickWorld(page, [2, 2, 1]) // the guide's own pixel, behind box B
  expect((await picked(page)).guide).toBe(setup.guideB)
  await clearSelection(page)
  await clickWorld(page, [-2, 2, 1]) // the guide's own pixel, behind box A
  expect((await picked(page)).guide).toBeNull()
})

test('row 7: isolation fades the other box; the guide behind the faded box is picked', async ({ page }) => {
  const setup = await page.evaluate(() => {
    const h = window.__hew_test!
    h.setAxesVisible(false)
    h.setGridVisible(false)
    const a = h.drawBox([-3, -1, 0], [-1, 1, 0], 2)
    const b = h.drawBox([1, -1, 0], [3, 1, 0], 2)
    // Both boxes in one group: isolating A fades its group-mate B (objects
    // outside the group would be hidden, not faded).
    h.groupNodes([{ kind: 'object', id: a }, { kind: 'object', id: b }])
    h.isolate([{ kind: 'object', id: a }])
    return { a, b, guideB: h.addGuidePoint(2, 2, 1), guideA: h.addGuidePoint(-2, 2, 1) }
  })
  await camera(page, { position: [0, -12, 1], target: [0, 0, 1], fovDeg: 50 })
  // B must still be DRAWN (faded), not hidden: the pixel check is the guard.
  await expectFadedNotHidden(page)
  await clickWorld(page, [2, 2, 1]) // the guide's own pixel, behind box B
  expect((await picked(page)).guide).toBe(setup.guideB)
  await clearSelection(page)
  await clickWorld(page, [-2, 2, 1]) // the guide's own pixel, behind box A
  expect((await picked(page)).guide).toBeNull()
})

test('row 8: a section plane: the cut-away half hides nothing, the kept half does', async ({ page }) => {
  await makeBox(page)
  const g = await page.evaluate(() => {
    const h = window.__hew_test!
    h.setSectionPlane({ origin: [0, 0, 0], normal: [1, 0, 0], active: true }) // removes x > 0
    return { cut: h.addGuidePoint(0.6, 2, 1), kept: h.addGuidePoint(-0.6, 2, 1) }
  })
  await camera(page, FRONT)
  await clickWorld(page, [0.6, 2, 1])
  expect((await picked(page)).guide).toBe(g.cut)
  await clearSelection(page)
  await clickWorld(page, [-0.6, 2, 1])
  expect((await picked(page)).guide).toBeNull()
})

test.describe('row 9: a thin wall hides the guide just behind it at any model scale', () => {
  // wall thickness vs the pixel-fraction tolerance: 0.1 mm at 1 mm and 1 m
  // models; 1 mm at 100 m (a 0.1 mm wall is below a thousandth of a pixel there).
  for (const [label, s, t] of [['1 mm', 1e-3, 1e-4], ['1 m', 1, 1e-4], ['100 m', 100, 1e-3]] as const) {
    test(`${label} model`, async ({ page }) => {
      const id = await page.evaluate(([s, t]) => {
        const h = window.__hew_test!
        h.setAxesVisible(false)
        h.setGridVisible(false)
        return h.drawBox([-s / 2, 0, 0], [s / 2, t, 0], s)
      }, [s, t])
      const guide = await page.evaluate(([s, t]) => window.__hew_test!.addGuidePoint(0, t * 1.5, s / 2), [s, t])
      await camera(page, { position: [0, -s * 1.3, s / 2], target: [0, 0, s / 2], fovDeg: 50 })
      await clickWorld(page, [0, 0, s / 2])
      const p = await picked(page)
      expect(p.guide, 'the guide behind the wall is not taken').toBeNull()
      expect(p.nodes).toEqual([`object:${id}`])
    })
  }
})

test.describe('row 10: coplanar guides and dimensions on a face stay pickable', () => {
  const HEAD_ON = FRONT
  const GRAZING = { position: [8, -1.15, 1] as V3, target: [0, -1, 1] as V3, fovDeg: 50 }
  for (const [label, pose] of [['head-on', HEAD_ON], ['near edge-on', GRAZING]] as const) {
    test(`guide on the face, ${label}`, async ({ page }) => {
      await makeBox(page)
      const guide = await page.evaluate(() => window.__hew_test!.addGuideLine(0, -1, 1, 1, 0, 0))
      await camera(page, pose)
      await clickWorld(page, [0.4, -1, 1])
      expect((await picked(page)).guide).toBe(guide)
    })
    test(`dimension on the face, ${label}`, async ({ page }) => {
      await makeBox(page)
      const dim = await page.evaluate(() =>
        window.__hew_test!.addLinearDimension([-0.8, -1, 1.5], [0.8, -1, 1.5], [0, 0, -0.5], [0, -1, 1, 0, -1, 0]))
      await camera(page, pose)
      await clickWorld(page, [0.2, -1, 1])
      expect((await picked(page)).annotation).toBe(dim)
    })
  }
})

test.describe('row 11: X-ray and Wireframe hide nothing', () => {
  for (const mode of ['xray', 'wireframe'] as const) {
    test(mode, async ({ page }) => {
      await makeBox(page)
      const guide = await page.evaluate(() => window.__hew_test!.addGuideLine(0, 2, 1, 1, 0, 0))
      await page.evaluate((m) => window.__hew_test!.setRenderMode(m), mode)
      await camera(page, FRONT)
      await clickWorld(page, [0.3, -1, 1])
      expect((await picked(page)).guide).toBe(guide)
    })
  }
})

test('row 12: a guide along the view direction, passing behind the camera', async ({ page }) => {
  await page.evaluate(() => {
    const h = window.__hew_test!
    h.setAxesVisible(false)
    h.setGridVisible(false)
    h.drawBox([-0.6, 2, 0], [0.6, 3, 0], 2)
  })
  const guide = await page.evaluate(() => window.__hew_test!.addGuideLine(0.3, 0, 1, 0, 1, 0))
  await camera(page, FRONT)
  // The near stretch (y = -6) is in plain view; the far stretch (y = 30) is
  // behind the box.
  await clickWorld(page, [0.3, -6, 1])
  expect((await picked(page)).guide).toBe(guide)
  await clearSelection(page)
  await clickWorld(page, [0.3, 30, 1])
  expect((await picked(page)).guide).toBeNull()
})

test.describe('row 13: parallel projection', () => {
  test('hidden guide and coplanar guide', async ({ page }) => {
    const box = await makeBox(page)
    const hidden = await page.evaluate(() => window.__hew_test!.addGuideLine(0, 2, 1.6, 1, 0, 0))
    const onFace = await page.evaluate(() => window.__hew_test!.addGuideLine(0, -1, 0.4, 1, 0, 0))
    await page.evaluate(() => window.__hew_test!.toggleProjection())
    expect(await page.evaluate(() => window.__hew_test!.getProjection())).toBe('parallel')
    await camera(page, FRONT)
    // The parallel view's pose differs a little from the perspective one, so
    // aim at each guide's own pixel (the box face is under it).
    await clickWorld(page, [0.3, 2, 1.6])
    let p = await picked(page)
    expect(p.guide).toBeNull()
    expect(p.nodes).toEqual([`object:${box}`])
    await clearSelection(page)
    await clickWorld(page, [0.3, -1, 0.4])
    p = await picked(page)
    expect(p.guide).toBe(onFace)
    await clearSelection(page)
    await camera(page, BACK)
    await clickWorld(page, [0.3, 2, 1.6])
    expect((await picked(page)).guide).toBe(hidden)
  })
})

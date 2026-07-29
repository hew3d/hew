import { test, expect } from '@playwright/test'

/**
 * Dimension + Text tools (docs/design/dimensions-text.md) — end-to-end,
 * driven with real mouse clicks (not harness one-shot calls) for the actual
 * tool gestures, matching the pattern in tools.spec.ts/follow-me.spec.ts.
 *
 * One sequential scenario (camera/box setup shared): linear dimension via
 * real clicks on a box edge, moving the box (the dimension follows via
 * kernel re-anchoring), a unit-setting change relabeling it, a radius
 * dimension on a drawn circle, a leader-text create-then-edit round trip
 * through the in-viewport editor, and a save/load persistence round trip.
 */

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

test.beforeEach(async ({ page }) => {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, {
    timeout: 15_000,
  })
})

test('Dimension + Text: create, move-follows, unit relabel, radius dimension, leader edit round trip, persistence', async ({ page }) => {
  const boxId = await page.evaluate(() => {
    const h = window.__hew_test!
    h.setCamera({ position: [6, -8, 6], target: [1, 1, 0.5], fovDeg: 45 })
    return h.drawBox([0, 0, 0], [2, 2, 0], 1)
  })

  const canvas = await page.locator('canvas').first().boundingBox()
  if (canvas === null) throw new Error('no canvas')
  const toPage = async (world: [number, number, number]) => {
    const p = await page.evaluate(
      (w) => window.__hew_test!.worldToScreen(w as [number, number, number]),
      world,
    )
    return { x: canvas.x + p.x, y: canvas.y + p.y }
  }
  const click = async (pt: { x: number; y: number }) => {
    await page.mouse.move(pt.x, pt.y)
    await page.mouse.down()
    await page.mouse.up()
  }

  // -------------------------------------------------------------------------
  // Linear dimension: click two bottom corners, drag out the offset, click
  // to place — real clicks on the box edge from (0,0,0) to (2,0,0).
  // -------------------------------------------------------------------------
  await page.getByRole('radio', { name: 'Dimension' }).click()
  await expect(page.getByText('Click a point to dimension from', { exact: false })).toBeVisible()

  await click(await toPage([0, 0, 0]))
  await expect(page.getByText('Click the second point.')).toBeVisible()

  await click(await toPage([2, 0, 0]))
  await expect(page.getByText('Drag out the dimension line', { exact: false })).toBeVisible()

  await click(await toPage([1, -1, 0]))

  const linearId = await page.evaluate(() => window.__hew_test!.getAnnotationIds()[0])
  expect(linearId).toBeTruthy()
  expect(await page.evaluate((id) => window.__hew_test!.getAnnotationKind(id), linearId)).toBe('linear')
  expect(await page.evaluate((id) => window.__hew_test!.getAnnotationDetached(id), linearId)).toBe(false)
  const label0 = await page.evaluate((id) => window.__hew_test!.getAnnotationLabel(id), linearId)
  expect(label0).toContain('2')

  // -------------------------------------------------------------------------
  // Move the box: the dimension follows (geometric re-anchoring) — stays
  // attached and reports the SAME length (a pure translation).
  // -------------------------------------------------------------------------
  await page.evaluate((id) => window.__hew_test!.moveObject(id, 3, 0, 0), boxId)
  expect(await page.evaluate((id) => window.__hew_test!.getAnnotationDetached(id), linearId)).toBe(false)
  expect(await page.evaluate((id) => window.__hew_test!.getAnnotationLabel(id), linearId)).toBe(label0)

  // -------------------------------------------------------------------------
  // Change units: every dimension re-labels, like SketchUp.
  // -------------------------------------------------------------------------
  await page.evaluate(() => window.__hew_test!.setLengthUnit('cm'))
  const label1 = await page.evaluate((id) => window.__hew_test!.getAnnotationLabel(id), linearId)
  expect(label1).toContain('cm')
  expect(label1).not.toBe(label0)
  await page.evaluate(() => window.__hew_test!.setLengthUnit('m'))

  // -------------------------------------------------------------------------
  // Radial dimension: click a drawn circle's rim, drag the leader out into
  // free space, click to place — dimensions-playtest2.md §4's "one-entity
  // flow" row of the second-click disambiguation table. The drawn circle is
  // a full (closed) circle, so the default kind is Diameter, matching
  // SketchUp — this is intentionally NOT the quadrant-point-along-+X-then-
  // drag-further-along-+X gesture (that shape is exercised precisely, both
  // click orders, both kinds, and the antipodal-tolerance boundary, by
  // DimensionTool.test.ts's disambiguation-table suite). Tab toggling is
  // also covered there.
  // -------------------------------------------------------------------------
  await page.evaluate(() => window.__hew_test!.drawCircle([8, 2, 0], 1.5))
  await page.evaluate(() => window.__hew_test!.setCamera({ position: [8, -6, 6], target: [8, 2, 0], fovDeg: 45 }))
  await page.getByRole('radio', { name: 'Dimension' }).click()

  await click(await toPage([9.5, 2, 0])) // center (8,2,0) + radius along +X
  await expect(page.getByText('Drag out the leader', { exact: false })).toBeVisible()
  await click(await toPage([11, 2, 0]))

  const idsAfterRadial = await page.evaluate(() => window.__hew_test!.getAnnotationIds())
  expect(idsAfterRadial.length).toBe(2)
  const radialId = idsAfterRadial.find((i) => i !== linearId)
  expect(radialId).toBeTruthy()
  expect(await page.evaluate((id) => window.__hew_test!.getAnnotationKind(id), radialId!)).toBe('radial')
  const radialLabel = await page.evaluate((id) => window.__hew_test!.getAnnotationLabel(id), radialId!)
  expect(radialLabel).toContain('Ø')
  expect(radialLabel).toContain('3')

  // -------------------------------------------------------------------------
  // Leader text: click an anchor, drag the leader, type content in the
  // in-viewport editor (Command palette activates Text — it's a menu/
  // palette-only tool, no rail slot, per toolRegistry.ts).
  // -------------------------------------------------------------------------
  // A ground-plane point (z=0, no extrusion-direction ambiguity to reason
  // about) just past the moved box — clearly in frame, unambiguous ray/
  // ground-plane resolution.
  const leaderAnchorWorld: [number, number, number] = [4, -1, 0]
  await page.evaluate(
    (t) => window.__hew_test!.setCamera({ position: [4, -5, 3], target: t, fovDeg: 45 }),
    leaderAnchorWorld,
  )
  await page.keyboard.press('Control+k')
  const palette = page.getByRole('dialog', { name: 'Command palette' })
  await expect(palette).toBeVisible()
  await palette.getByRole('textbox', { name: 'Search' }).fill('Text')
  // The option's accessible name concatenates the label + description
  // ("Text Place a leader-text annotation…") — match the label prefix.
  await palette.getByRole('option', { name: /^Text\b/ }).first().click()
  await expect(page.getByText('Click a face, edge, or point to anchor the leader text.')).toBeVisible()

  const anchorPt = await toPage(leaderAnchorWorld)
  await click(anchorPt)
  await expect(page.getByText('Drag out the leader, then click to place and type.')).toBeVisible()
  // Drag out the leader by a pure screen-space offset — the exact world
  // point the ray/ground-plane resolves to doesn't matter (any offset is
  // valid for a leader), avoiding any assumption about scene depth here.
  const leaderPt = { x: anchorPt.x, y: anchorPt.y - 90 }
  await click(leaderPt)

  const editor = page.getByTestId('annotation-editor')
  await expect(editor).toBeVisible()
  await editor.fill('Ships loose')
  await page.keyboard.press('Enter')
  await expect(editor).not.toBeVisible()

  const idsAfterLeader = await page.evaluate(() => window.__hew_test!.getAnnotationIds())
  expect(idsAfterLeader.length).toBe(3)
  const leaderId = idsAfterLeader.find((i) => i !== linearId && i !== radialId)
  expect(leaderId).toBeTruthy()
  expect(await page.evaluate((id) => window.__hew_test!.getAnnotationKind(id), leaderId!)).toBe('leader')
  expect(await page.evaluate((id) => window.__hew_test!.getAnnotationLabel(id), leaderId!)).toBe('Ships loose')

  // -------------------------------------------------------------------------
  // Double-click-to-edit round trip: Select tool, double-click the leader's
  // label, change its text.
  // -------------------------------------------------------------------------
  await page.getByRole('radio', { name: 'Select' }).click()
  await page.mouse.dblclick(leaderPt.x, leaderPt.y)

  const editor2 = page.getByTestId('annotation-editor')
  await expect(editor2).toBeVisible()
  await expect(editor2).toHaveValue('Ships loose')
  await editor2.fill('Ships loose — handle with care')
  await page.keyboard.press('Enter')
  await expect(editor2).not.toBeVisible()
  expect(await page.evaluate((id) => window.__hew_test!.getAnnotationLabel(id), leaderId!)).toBe(
    'Ships loose — handle with care',
  )

  // -------------------------------------------------------------------------
  // Save/load persistence: all three annotations survive a round trip.
  // -------------------------------------------------------------------------
  const bytes = await page.evaluate(() => Array.from(window.__hew_test!.save()))
  await page.evaluate((b) => window.__hew_test!.load(b), bytes)
  await page.waitForFunction(() => window.__hew_test?.isReady() === true)

  const idsAfterLoad = await page.evaluate(() => window.__hew_test!.getAnnotationIds())
  expect(idsAfterLoad.length).toBe(3)
  const kindsAfterLoad = await Promise.all(
    idsAfterLoad.map((id) => page.evaluate((i) => window.__hew_test!.getAnnotationKind(i), id)),
  )
  expect(kindsAfterLoad.sort()).toEqual(['leader', 'linear', 'radial'])
  expect(
    await page.evaluate((id) => window.__hew_test!.getAnnotationLabel(id), leaderId!),
  ).toBe('Ships loose — handle with care')
})

import { test, expect } from '@playwright/test'

/**
 * Texture positioning (paint-tool design §3) — the Position Texture tool
 * driven with REAL mouse drags (mouse.move / mouse.down / several mouse.move
 * / mouse.up), not the harness's logic-only `setFaceUvFrame`, since the
 * gesture itself (drag-to-translate, commit-on-Enter, undo) is what's under
 * test. `getFaceUvFrame`/`setFaceUvFrame`/`addTextureMaterial` are
 * readback/fixture-setup helpers only (per the design's "assert the
 * committed frame via save() or a readback" allowance) — the actual
 * positioning gesture always goes through the real canvas.
 */

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

test.beforeEach(async ({ page }) => {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
})

/** A minimal valid 1x1 PNG (transparent pixel) — pixel content is
 *  irrelevant, only `has_texture()` needs to be true. */
const TINY_PNG = [
  0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
  0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4,
  0x89, 0x00, 0x00, 0x00, 0x0a, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0x00, 0x01, 0x00, 0x00,
  0x05, 0x00, 0x01, 0x0d, 0x0a, 0x2d, 0xb4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae,
  0x42, 0x60, 0x82,
]

/** Builds a 2x2x1 box, paints its top face with a fresh 1x1m textured
 *  material, and returns the object/face/canvas handles the gesture needs. */
async function setupTexturedBox(page: import('@playwright/test').Page) {
  await page.evaluate(() => {
    const h = window.__hew_test!
    h.setCamera({ position: [1, 1, 10], target: [1, 1, 0], up: [0, 1, 0], fovDeg: 45 })
  })
  const boxId = await page.evaluate(() => window.__hew_test!.drawBox([0, 0, 0], [2, 2, 0], 1))
  const matId = await page.evaluate(
    (image) => window.__hew_test!.addTextureMaterial('Tile', 255, 255, 255, 255, image, 0, 1, 1),
    TINY_PNG,
  )
  const topFace = await page.evaluate(
    (box) => window.__hew_test!.pickFace([1, 1, 10], [0, 0, -1]),
    boxId,
  )
  expect(topFace).not.toBeNull()
  expect(topFace!.object).toBe(boxId)
  await page.evaluate(
    ({ box, face, mat }) => window.__hew_test!.paintFace(box, face, mat),
    { box: boxId, face: topFace!.face, mat: matId },
  )
  await page.evaluate((id) => window.__hew_test!.selectObjects([id]), boxId)
  await page.waitForFunction(() => window.__hew_test!.getSelection().length === 1)

  const canvas = await page.locator('canvas').first().boundingBox()
  if (canvas === null) throw new Error('no canvas')
  return { boxId, faceId: topFace!.face, canvas }
}

function dock(page: import('@playwright/test').Page) {
  return page.locator('.hew-dock')
}

test('Position Texture appears on the dock for a selected object and activates the tool', async ({ page }) => {
  const { boxId } = await setupTexturedBox(page)
  await expect(dock(page)).toContainText('OBJECT')
  await dock(page).getByRole('button', { name: 'Position Texture' }).click()
  await expect(page.getByText(/Click a textured face to position its texture/i)).toBeVisible()
  // Sanity: the box is still there, unpainted state untouched by activation alone.
  expect(await page.evaluate((id) => window.__hew_test!.getFaceMaterial(id, id), boxId)).toBeDefined()
})

test('a real drag translates the texture and Enter commits it as one undo step', async ({ page }) => {
  const { boxId, faceId, canvas } = await setupTexturedBox(page)

  const before = await page.evaluate(
    ({ box, face }) => window.__hew_test!.getFaceUvFrame(box, face),
    { box: boxId, face: faceId },
  )
  expect(before).toEqual([]) // planar default, no explicit frame yet

  await dock(page).getByRole('button', { name: 'Position Texture' }).click()

  const start = await page.evaluate(() => window.__hew_test!.worldToScreen([1, 1, 1]))
  const end = await page.evaluate(() => window.__hew_test!.worldToScreen([1.6, 1, 1]))
  const sx = canvas.x + start.x
  const sy = canvas.y + start.y
  const ex = canvas.x + end.x
  const ey = canvas.y + end.y

  await page.mouse.move(sx, sy)
  await page.mouse.down() // enters positioning + begins the first (translate) drag
  await page.mouse.move((sx + ex) / 2, (sy + ey) / 2)
  await page.mouse.move(ex, ey)
  await page.mouse.up()

  // Nothing is committed to the document until Enter/click-away — the
  // drag-in-progress is a renderer-local preview only.
  const midGesture = await page.evaluate(
    ({ box, face }) => window.__hew_test!.getFaceUvFrame(box, face),
    { box: boxId, face: faceId },
  )
  expect(midGesture).toEqual([])

  await page.keyboard.press('Enter')

  const committed = await page.evaluate(
    ({ box, face }) => window.__hew_test!.getFaceUvFrame(box, face),
    { box: boxId, face: faceId },
  )
  expect(committed).not.toBeNull()
  expect(committed).toHaveLength(8)
  // An actual translate happened: the committed frame differs from an
  // untouched planar default's u0/v0 (both would be exactly 0,0 unmoved).
  expect(committed![6] !== 0 || committed![7] !== 0).toBe(true)

  // One undo step for the WHOLE drag session, regardless of the
  // intermediate mouse.move samples above.
  await page.evaluate(() => window.__hew_test!.undo())
  const afterUndo = await page.evaluate(
    ({ box, face }) => window.__hew_test!.getFaceUvFrame(box, face),
    { box: boxId, face: faceId },
  )
  expect(afterUndo).toEqual([])
})

test('Esc reverts the drag without committing anything', async ({ page }) => {
  const { boxId, faceId, canvas } = await setupTexturedBox(page)

  await dock(page).getByRole('button', { name: 'Position Texture' }).click()

  const start = await page.evaluate(() => window.__hew_test!.worldToScreen([1, 1, 1]))
  const end = await page.evaluate(() => window.__hew_test!.worldToScreen([0.4, 1.6, 1]))
  await page.mouse.move(canvas.x + start.x, canvas.y + start.y)
  await page.mouse.down()
  await page.mouse.move(canvas.x + end.x, canvas.y + end.y)
  await page.mouse.up()

  await page.keyboard.press('Escape')

  const frame = await page.evaluate(
    ({ box, face }) => window.__hew_test!.getFaceUvFrame(box, face),
    { box: boxId, face: faceId },
  )
  expect(frame).toEqual([])

  // No undo entry was created — undo() should now be a no-op / unrelated to
  // texture positioning (canUndo may still be true from the paint/box-draw
  // steps in setup, but the FRAME must already read back untouched above,
  // which is the actual contract this test pins).
})

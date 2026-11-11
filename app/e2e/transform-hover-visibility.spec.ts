import { test, expect, type Page } from '@playwright/test'

/**
 * The transform tools' hover highlight (Move/Rotate/Scale with nothing
 * selected) shows what a click would take, so it follows visibility changes
 * under a STILL cursor: hiding or showing the hovered object, or hiding the
 * guides, re-picks, and what is no longer drawn (so no longer clickable) is
 * no longer highlighted. (Isolation takes the same path, but it also frames
 * the isolated set, which moves the camera: that re-pick is the camera's.)
 */

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

async function hover(page: Page): Promise<{ objects: string[]; guide: string | null } | null> {
  return page.evaluate(() => {
    const h = window.__hew_test!.getHoverHighlight()
    return h === null ? null : { objects: h.objects, guide: h.guide }
  })
}

/** A box under the canvas center, Move active with the cursor parked on it. */
async function setup(page: Page): Promise<{ box: string }> {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
  const ids = await page.evaluate(() => ({ box: window.__hew_test!.drawBox([-1, -1, 0], [1, 1, 0], 1) }))
  await page.evaluate(() =>
    window.__hew_test!.setCamera({ position: [0, -1, 9], target: [0, 0, 1], up: [0, 0, 1], fovDeg: 45 } as never),
  )
  const canvas = (await page.locator('canvas').first().boundingBox())!
  const cx = canvas.x + canvas.width / 2
  const cy = canvas.y + canvas.height / 2
  await page.mouse.move(cx - 8, cy + 8)
  await page.mouse.move(cx, cy, { steps: 3 })
  await page.keyboard.press('m')
  await page.mouse.move(cx + 1, cy)
  await expect.poll(() => hover(page)).toEqual({ objects: [ids.box], guide: null })
  return ids
}

test('hiding the hovered object drops its highlight; showing it brings it back', async ({ page }) => {
  const { box } = await setup(page)
  await page.evaluate((id) => window.__hew_test!.toggleNodeHidden({ kind: 'object', id }), box)
  await expect.poll(() => hover(page)).toBeNull()
  await page.evaluate((id) => window.__hew_test!.toggleNodeHidden({ kind: 'object', id }), box)
  await expect.poll(() => hover(page)).toEqual({ objects: [box], guide: null })
})

test('hiding the guides drops a hovered guide\'s highlight', async ({ page }) => {
  const { box } = await setup(page)
  // A guide across the top of the box, right under the cursor: Move takes a
  // guide before the object beneath it.
  const guide = await page.evaluate(() => window.__hew_test!.addGuideLine(-3, 0, 1, 1, 0, 0))
  const canvas = (await page.locator('canvas').first().boundingBox())!
  await page.mouse.move(canvas.x + canvas.width / 2, canvas.y + canvas.height / 2 + 1)
  await expect.poll(() => hover(page)).toEqual({ objects: [], guide })

  await page.evaluate(() => window.__hew_test!.setGuidesVisible(false))
  // The guide is gone, so the click (and the highlight) falls to the box.
  await expect.poll(() => hover(page)).toEqual({ objects: [box], guide: null })
})

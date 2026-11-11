import { test, expect, type Page } from '@playwright/test'

/**
 * The transform tools' hover highlight (Move/Rotate/Scale with nothing
 * selected) shows what a click would take, so it follows the camera under a
 * STILL cursor: a wheel zoom, a Scene tween or Fly change what lies under
 * it with no pointer move. The re-pick runs on the camera's own change
 * event, on a ray rebuilt for the camera as it now stands, not the ray of
 * the last pointer move.
 *
 * `__hew_test.setCamera` moves the camera through the same OrbitControls
 * change event a wheel zoom fires, without moving the pointer.
 */

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

type V3 = [number, number, number]

/** Look straight down (slightly tilted) at `at` from 6 m up. */
function lookingDownAt(at: V3) {
  return { position: [at[0], at[1] - 1, at[2] + 6] as V3, target: at, up: [0, 0, 1] as V3, fovDeg: 45 }
}

async function hovered(page: Page): Promise<string[] | null> {
  return page.evaluate(() => window.__hew_test!.getHoverHighlight()?.objects ?? null)
}

test('the hover highlight follows the camera under a still cursor', async ({ page }) => {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
  const { a, b } = await page.evaluate(() => {
    const h = window.__hew_test!
    return { a: h.drawBox([0, 0, 0], [1, 1, 0], 1), b: h.drawBox([4, 0, 0], [5, 1, 0], 1) }
  })
  await page.evaluate((pose) => window.__hew_test!.setCamera(pose as never), lookingDownAt([0.5, 0.5, 1]))

  // Park the cursor on the canvas center (over A) with Move active.
  const canvas = (await page.locator('canvas').first().boundingBox())!
  const cx = canvas.x + canvas.width / 2
  const cy = canvas.y + canvas.height / 2
  await page.mouse.move(cx - 8, cy + 8)
  await page.mouse.move(cx, cy, { steps: 3 })
  await page.keyboard.press('m')
  await page.mouse.move(cx + 1, cy)
  await expect.poll(() => hovered(page)).toEqual([a])

  // The camera moves to B; the pointer does not.
  await page.evaluate((pose) => window.__hew_test!.setCamera(pose as never), lookingDownAt([4.5, 0.5, 1]))
  await expect.poll(() => hovered(page)).toEqual([b])

  // And to empty ground: nothing to take, nothing highlighted.
  await page.evaluate((pose) => window.__hew_test!.setCamera(pose as never), lookingDownAt([2.5, 5, 0]))
  await expect.poll(() => hovered(page)).toBeNull()
})

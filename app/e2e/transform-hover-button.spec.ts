import { test, expect, type Page } from '@playwright/test'

/**
 * The transform tools' hover highlight (Move/Rotate/Scale with nothing
 * selected) shows what a click would take, so while a button is held (a
 * middle-button orbit, a right-button press) nothing is about to be
 * clicked and nothing is highlighted: not on the press, and not as the
 * orbit moves the camera under the cursor (a camera move re-picks the
 * highlight). It comes back once every button is up.
 */

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

async function hovered(page: Page): Promise<string[] | null> {
  return page.evaluate(() => window.__hew_test!.getHoverHighlight()?.objects ?? null)
}

test('no hover highlight while a middle-button orbit holds the pointer', async ({ page }) => {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
  const box = await page.evaluate(() => window.__hew_test!.drawBox([-2, -2, 0], [2, 2, 0], 1))
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
  await expect.poll(() => hovered(page)).toEqual([box])

  // Press: nothing highlighted.
  await page.mouse.down({ button: 'middle' })
  await expect.poll(() => hovered(page)).toBeNull()
  // Orbit: the camera moves under the cursor on every step; still nothing.
  const seen: (string[] | null)[] = []
  for (let k = 1; k <= 8; k++) {
    await page.mouse.move(cx + 1 + k * 6, cy)
    seen.push(await hovered(page))
  }
  expect(seen.every((h) => h === null)).toBe(true)

  // Release: what a click would take is highlighted again.
  await page.mouse.up({ button: 'middle' })
  await page.mouse.move(cx + 50, cy + 1)
  await expect.poll(() => hovered(page)).toEqual([box])
})

import { test, expect, type Page } from '@playwright/test'

/**
 * The transform tools' hover highlight (Move/Rotate/Scale with nothing
 * selected) shows what a click would take, so it goes when the pointer
 * moves off the canvas onto UI that sits INSIDE the canvas's bounds (a
 * dock, a panel): the pointer is still within the canvas rectangle, but
 * what it rests on is not the canvas.
 *
 * The dock is a plain element laid over part of the canvas, standing in
 * for any panel there.
 */

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

async function hovered(page: Page): Promise<string[] | null> {
  return page.evaluate(() => window.__hew_test!.getHoverHighlight()?.objects ?? null)
}

test('the hover highlight goes when the pointer moves onto a dock over the canvas', async ({ page }) => {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
  const box = await page.evaluate(() => window.__hew_test!.drawBox([-1, -1, 0], [1, 1, 0], 1))
  await page.evaluate(() =>
    window.__hew_test!.setCamera({ position: [0, -1, 7], target: [0, 0, 1], up: [0, 0, 1], fovDeg: 45 } as never),
  )

  const canvas = (await page.locator('canvas').first().boundingBox())!
  const cx = canvas.x + canvas.width / 2
  const cy = canvas.y + canvas.height / 2
  // A dock over the right half of the box, well inside the canvas's bounds.
  await page.evaluate(([x, y]) => {
    const dock = document.createElement('div')
    dock.id = 'test-dock'
    Object.assign(dock.style, {
      position: 'fixed', left: `${x + 20}px`, top: `${y - 40}px`, width: '60px', height: '80px',
      background: '#888', zIndex: '1000',
    })
    document.body.appendChild(dock)
  }, [cx, cy])

  // Move active, the cursor on the box (left of the dock): lit.
  await page.mouse.move(cx - 30, cy + 10)
  await page.mouse.move(cx, cy, { steps: 3 })
  await page.keyboard.press('m')
  await page.mouse.move(cx + 1, cy)
  await expect.poll(() => hovered(page)).toEqual([box])

  // Onto the dock, over the same box: not the canvas, nothing to click.
  await page.mouse.move(cx + 50, cy, { steps: 4 })
  await expect.poll(() => hovered(page)).toBeNull()

  // Back onto the canvas: lit again.
  await page.mouse.move(cx, cy, { steps: 4 })
  await expect.poll(() => hovered(page)).toEqual([box])
})

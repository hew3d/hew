import { test, expect } from '@playwright/test'
import { activateFromToolsMenu, clickWorld, expectStatus, setup, shot, volume } from './helpers/offset'
import { requiresPreview } from './helpers/preview'

// Drives a Preview feature: skipped in the production-shape run (e2e/helpers/preview.ts).
requiresPreview('shell')

/**
 * Shell, driven with real pointer and keyboard events: click a face to leave
 * it open, type a wall thickness, Enter. The volume read back through the
 * harness checks the kernel's answer (WP16: top open, 1 cm wall on a 1 m
 * cube leaves 1 − 0.98·0.98·0.99 of material).
 */

const CAMERA = { position: [3, -3, 3.2] as [number, number, number], target: [0.5, 0.5, 0.5] as [number, number, number] }

test('Shell: click the top face, type 1cm, Enter hollows the box and leaves the top open; undo restores it', async ({ page }) => {
  await setup(page, CAMERA)
  const box = await page.evaluate(() => window.__hew_test!.drawBox([0, 0, 0], [1, 1, 0], 1))
  expect(await volume(page, box)).toBeCloseTo(1, 6)

  await activateFromToolsMenu(page, 'Shell')
  await expectStatus(page, /Click a face to hollow out its object/)
  await clickWorld(page, [0.5, 0.5, 1])
  // A ghost is up at once, at a wall that suits the object.
  await expectStatus(page, /Type a wall thickness and press Enter/)
  await shot(page, 'shell-ghost')

  await page.keyboard.type('1cm')
  await page.keyboard.press('Enter')
  await page.waitForTimeout(150)
  expect(await volume(page, box)).toBeCloseTo(1 - 0.98 * 0.98 * 0.99, 6)
  await shot(page, 'shell-committed')

  await page.evaluate(() => window.__hew_test!.undo())
  await page.waitForTimeout(100)
  expect(await volume(page, box)).toBeCloseTo(1, 6)
})

test('Shell: a selected box with no face clicked becomes a closed hollow body', async ({ page }) => {
  await setup(page, CAMERA)
  const box = await page.evaluate(() => window.__hew_test!.drawBox([0, 0, 0], [1, 1, 0], 1))
  await page.evaluate((id) => window.__hew_test!.selectObjects([id]), box)
  await activateFromToolsMenu(page, 'Shell')
  await page.keyboard.type('1cm')
  await page.keyboard.press('Enter')
  await page.waitForTimeout(150)
  expect(await volume(page, box)).toBeCloseTo(1 - 0.98 ** 3, 6)
})

test('Shell: a negative thickness adds the wall outside; a wall too thick explains itself and changes nothing', async ({ page }) => {
  await setup(page, CAMERA)
  const box = await page.evaluate(() => window.__hew_test!.drawBox([0, 0, 0], [1, 1, 0], 1))
  await activateFromToolsMenu(page, 'Shell')
  await clickWorld(page, [0.5, 0.5, 1])

  // Too thick: the ghost goes, the status says why, Enter toasts it and leaves the box alone.
  await page.keyboard.type('60cm')
  await expectStatus(page, /wall leaves no room here/)
  await page.keyboard.press('Enter')
  await page.waitForTimeout(100)
  expect(await volume(page, box)).toBeCloseTo(1, 6)
  await shot(page, 'shell-too-thick')

  // Back to a sane value, outward this time: the original size stays as the inside.
  await page.keyboard.type('-1cm')
  await page.keyboard.press('Enter')
  await page.waitForTimeout(150)
  const bounds = await page.evaluate((id) => window.__hew_test!.getObjectBounds(id), box)
  expect(bounds[0]).toBeCloseTo(-0.01, 6)
  expect(bounds[3]).toBeCloseTo(1.01, 6)
})

test('Shell: entering from Move shows the same ghost as entering from Select', async ({ page }) => {
  await setup(page, CAMERA)
  const box = await page.evaluate(() => window.__hew_test!.drawBox([0, 0, 0], [1, 1, 0], 1))
  await page.evaluate((id) => window.__hew_test!.selectObjects([id]), box)
  // The dock floats over the canvas and its Shell chip is mid-highlight
  // when the second shot lands, so it is masked out: the comparison is the
  // ghost in the canvas, not the chrome.
  const canvasShot = async (): Promise<Buffer> => {
    await page.mouse.move(5, 5)
    await page.waitForTimeout(250)
    return page.locator('canvas').first().screenshot({ mask: [page.getByTestId('contextual-dock')] })
  }

  await activateFromToolsMenu(page, 'Shell')
  const fromSelect = await canvasShot()

  // Back to Select, then through Move, whose cancel() clears the preview group.
  await page.keyboard.press('Escape')
  await page.keyboard.press(' ')
  await page.keyboard.press('m')
  await page.waitForTimeout(150)
  await activateFromToolsMenu(page, 'Shell')
  const fromMove = await canvasShot()
  expect(fromMove.equals(fromSelect)).toBe(true)

  // And the ghost is really there: typing a value would otherwise be the
  // first thing to draw it.
  await page.keyboard.press('Escape')
  await page.keyboard.press(' ')
  const bare = await canvasShot()
  expect(bare.equals(fromSelect)).toBe(false)
})

import { test, expect, type Page } from '@playwright/test'
import { activateFromToolsMenu } from './helpers/offset'
import { requiresPreview } from './helpers/preview'

// Drives a Preview feature: skipped in the production-shape run (e2e/helpers/preview.ts).
requiresPreview('path-array')

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

/**
 * Array along Path, driven with real pointer and keyboard events: select a
 * box, pick Array along Path from the Tools menu, click a drawn arc, type a
 * count, Enter. Then retype the count (one undo step throughout) and undo.
 * The scene is built through the harness; only the tool interaction is
 * synthesized input.
 */

const CAMERA = {
  position: [6, -6, 7] as [number, number, number],
  target: [1.5, 2, 0] as [number, number, number],
  up: [0, 0, 1] as [number, number, number],
  fovDeg: 45,
}

async function setup(page: Page): Promise<void> {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, {
    timeout: 60_000,
  })
  await page.evaluate((cam) => window.__hew_test!.setCamera(cam), CAMERA)
  await page.waitForTimeout(100)
}

async function pagePoint(page: Page, w: [number, number, number]): Promise<{ x: number; y: number }> {
  const box = await page.locator('canvas').first().boundingBox()
  if (box === null) throw new Error('no canvas')
  const p = await page.evaluate((w) => window.__hew_test!.worldToScreen(w), w)
  if (p.behind) throw new Error(`world point ${w.join(',')} is behind the camera`)
  return { x: box.x + p.x, y: box.y + p.y }
}

async function moveToWorld(page: Page, w: [number, number, number]): Promise<void> {
  const p = await pagePoint(page, w)
  await page.mouse.move(p.x, p.y)
}

async function clickWorld(page: Page, w: [number, number, number]): Promise<void> {
  await moveToWorld(page, w)
  await page.mouse.down()
  await page.mouse.up()
}

async function objectCount(page: Page): Promise<number> {
  return (await page.evaluate(() => window.__hew_test!.getObjectIds())).length
}

test('Array along Path: click an arc, type a count, get that many objects; retype and undo', async ({
  page,
}) => {
  await setup(page)
  const box = await page.evaluate(() => window.__hew_test!.drawBox([0, 0, 0], [0.5, 0.5, 0], 0.5))
  await page.evaluate(() => window.__hew_test!.drawArc([0, 3, 0], [3, 3, 0], 0.8))
  await page.evaluate((id) => window.__hew_test!.selectObjects([id]), box)
  expect(await objectCount(page)).toBe(1)

  await activateFromToolsMenu(page, 'Array along Path')
  // Hover first, as a real hand does, then click the arc at its start point.
  await moveToWorld(page, [0, 3, 0])
  await page.waitForTimeout(100)
  await page.mouse.down()
  await page.mouse.up()
  await page.waitForTimeout(100)
  expect(await objectCount(page)).toBe(1) // picking a path alone copies nothing

  await page.keyboard.type('6')
  await page.keyboard.press('Enter')
  await page.waitForTimeout(150)
  expect(await objectCount(page)).toBe(6)

  // Retype: a new count replaces the array in place (still one undo step).
  await page.keyboard.type('4')
  await page.keyboard.press('Enter')
  await page.waitForTimeout(150)
  expect(await objectCount(page)).toBe(4)

  await page.evaluate(() => window.__hew_test!.undo())
  await page.waitForTimeout(100)
  expect(await objectCount(page)).toBe(1)
})

test('Array along Path: a ramp then a count scales the copies; a miss and an empty selection explain themselves', async ({
  page,
}) => {
  await setup(page)
  const box = await page.evaluate(() => window.__hew_test!.drawBox([0, 0, 0], [0.5, 0.5, 0], 0.5))
  await page.evaluate(() => window.__hew_test!.drawArc([0, 3, 0], [3, 3, 0], 0.8))

  await activateFromToolsMenu(page, 'Array along Path')
  // Nothing selected: the click says what to do first.
  await clickWorld(page, [0, 3, 0])
  await expect(page.getByText('Select what to copy first.').first()).toBeVisible()

  await page.evaluate((id) => window.__hew_test!.selectObjects([id]), box)
  // The tool takes the selection in a React effect, and its status hint
  // turning to the click prompt is the observable that it has. Clicking
  // earlier is a miss, and the typed digits then switch tools (4 -> Move).
  await expect(page.getByText(/^Click a path\. Type a count/)).toBeVisible()
  await clickWorld(page, [0, 3, 0])
  await page.keyboard.type('100%->40%')
  await page.keyboard.press('Enter')
  await page.keyboard.type('4')
  await page.keyboard.press('Enter')
  // The copies land asynchronously; under load 150 ms was not always enough.
  await expect
    .poll(() => page.evaluate(() => window.__hew_test!.getObjectIds().length), { timeout: 10_000 })
    .toBe(4)

  const ids = await page.evaluate(() => window.__hew_test!.getObjectIds())
  expect(ids).toHaveLength(4)
  const sizes: number[] = []
  for (const id of ids) {
    const b = await page.evaluate((id) => window.__hew_test!.getObjectBounds(id), id)
    sizes.push(b[5] - b[2]) // height
  }
  sizes.sort((a, b) => b - a)
  // Original 0.5 tall; copies ramp from 100% to 40% of it.
  expect(sizes[0]).toBeCloseTo(0.5, 5)
  expect(sizes[sizes.length - 1]).toBeCloseTo(0.5 * 0.4, 5)
})

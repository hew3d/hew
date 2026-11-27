import { test, expect } from '@playwright/test'
import { clickWorld, setup, shot, volume } from './helpers/offset'
import { requiresPreview } from './helpers/preview'

// Drives a Preview feature: skipped in the production-shape run (e2e/helpers/preview.ts).
requiresPreview('smooth')

/**
 * Smooth in the real UI: the Object Info Smooth row, the cage wire while a
 * tool other than Select is active, Bake, the boolean note, and the Export
 * dialog's "Export cages instead".
 */

const CAMERA = { position: [3, -3, 2.6] as [number, number, number], target: [0.5, 0.5, 0.5] as [number, number, number] }

test('Smooth: the Object Info row rounds the box, shows its cage under other tools, and Bake keeps the shape', async ({ page }) => {
  await setup(page, CAMERA)
  const box = await page.evaluate(() => window.__hew_test!.drawBox([0, 0, 0], [1, 1, 0], 1))
  await page.evaluate((id) => window.__hew_test!.selectObjects([id]), box)

  const row = page.getByRole('combobox', { name: 'Smooth' })
  await expect(row).toHaveValue('0')
  expect(await page.evaluate(() => window.__hew_test!.getCageOverlayInfo())).toEqual({ objects: 0, visible: 0 })

  await row.selectOption('2')
  await page.waitForTimeout(200)
  expect(await page.evaluate((id) => window.__hew_test!.getSmoothLevel(id), box)).toBe(2)
  // Rounded corners take volume off the cage.
  const smooth = await volume(page, box)
  expect(smooth).toBeLessThan(0.98)
  expect(smooth).toBeGreaterThan(0.2)
  await shot(page, 'smooth-level-2-select')

  // Select shows the smooth shape alone; any other tool adds the cage wire.
  expect(await page.evaluate(() => window.__hew_test!.getCageOverlayInfo())).toEqual({ objects: 1, visible: 0 })
  await page.keyboard.press('p')
  await page.waitForTimeout(100)
  expect(await page.evaluate(() => window.__hew_test!.getCageOverlayInfo())).toEqual({ objects: 1, visible: 1 })
  await shot(page, 'smooth-level-2-cage')
  await page.keyboard.press('Space')
  await page.waitForTimeout(100)
  expect(await page.evaluate(() => window.__hew_test!.getCageOverlayInfo())).toEqual({ objects: 1, visible: 0 })

  // Bake: the level goes away, the rounded shape stays as real geometry.
  await page.getByRole('button', { name: 'Bake' }).click()
  await page.waitForTimeout(200)
  expect(await page.evaluate((id) => window.__hew_test!.getSmoothLevel(id), box)).toBe(0)
  expect(await volume(page, box)).toBeCloseTo(smooth, 6)
  await expect(page.getByRole('combobox', { name: 'Smooth' })).toHaveValue('0')

  // One undo brings the cage and the level back.
  await page.evaluate(() => window.__hew_test!.undo())
  await page.waitForTimeout(150)
  expect(await page.evaluate((id) => window.__hew_test!.getSmoothLevel(id), box)).toBe(2)
})

test('Smooth: combining a smooth object says it uses the cage', async ({ page }) => {
  await setup(page, CAMERA)
  const a = await page.evaluate(() => window.__hew_test!.drawBox([0, 0, 0], [1, 1, 0], 1))
  const b = await page.evaluate(() => window.__hew_test!.drawBox([0.5, 0.5, 0], [1.5, 1.5, 0], 1))
  await page.evaluate(([id]) => window.__hew_test!.setSmoothLevel(id, 1), [a])
  await page.evaluate(([x, y]) => window.__hew_test!.selectObjects([x, y]), [a, b])
  const menuBar = page.getByTestId('menu-bar')
  await menuBar.getByRole('button', { name: 'Object' }).click()
  await menuBar.getByText('Union', { exact: true }).click()
  await expect(page.getByText('Combining smooth objects uses their cages. Bake first for an exact cut.')).toBeVisible()
})

test('Smooth: the Export dialog offers cages only when the model has a smooth object', async ({ page }) => {
  await setup(page, CAMERA)
  const box = await page.evaluate(() => window.__hew_test!.drawBox([0, 0, 0], [1, 1, 0], 1))
  const openExport = async () => {
    const menuBar = page.getByTestId('menu-bar')
    await menuBar.getByRole('button', { name: 'File' }).click()
    await menuBar.getByText('Export…', { exact: true }).click()
  }
  await openExport()
  await expect(page.getByRole('dialog', { name: 'Export' })).toBeVisible()
  await expect(page.getByLabel('Export cages instead')).toHaveCount(0)
  await page.keyboard.press('Escape')

  await page.evaluate((id) => window.__hew_test!.setSmoothLevel(id, 1), box)
  await openExport()
  await expect(page.getByLabel('Export cages instead')).toBeVisible()
  await shot(page, 'smooth-export-cages')
})

test('Smooth: Push/Pull on the top of a smooth box keeps the Smooth level', async ({ page }) => {
  await setup(page, CAMERA)
  const box = await page.evaluate(() => window.__hew_test!.drawBox([0, 0, 0], [1, 1, 0], 1))
  await page.evaluate((id) => window.__hew_test!.setSmoothLevel(id, 2), box)
  const before = await volume(page, box)

  // The real Push/Pull tool: click the top, type the distance.
  await page.keyboard.press('p')
  await clickWorld(page, [0.5, 0.5, 1])
  await page.keyboard.type('0.2')
  await page.keyboard.press('Enter')
  await page.waitForTimeout(250)

  // The push may replace the object's id; the one object left still carries Smooth 2.
  const ids = await page.evaluate(() => window.__hew_test!.getObjectIds())
  expect(ids).toHaveLength(1)
  expect(await page.evaluate((id) => window.__hew_test!.getSmoothLevel(id), ids[0])).toBe(2)
  expect(await volume(page, ids[0])).toBeGreaterThan(before + 0.05)

  // One undo takes the push back and the level is still 2.
  await page.evaluate(() => window.__hew_test!.undo())
  await page.waitForTimeout(150)
  const back = await page.evaluate(() => window.__hew_test!.getObjectIds())
  expect(back).toHaveLength(1)
  expect(await page.evaluate((id) => window.__hew_test!.getSmoothLevel(id), back[0])).toBe(2)
  expect(await volume(page, back[0])).toBeCloseTo(before, 6)
})

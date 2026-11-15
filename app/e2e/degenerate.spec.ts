import { test, expect, type Page } from '@playwright/test'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { setup } from './helpers/offset'
import { requiresPreview } from './helpers/preview'

// Drives a Preview feature: skipped in the production-shape run (e2e/helpers/preview.ts).
requiresPreview('remove-degenerate-faces')

/**
 * Zero-area faces in the real UI: a closed cube carrying a sliver face (made
 * by the kernel from polygons; no tool can draw one) is listed in Model
 * Health, its "Remove degenerate faces" button clears it.
 */

const fixture = (name: string): number[] => Array.from(readFileSync(resolve(process.cwd(), `e2e/fixtures/${name}`)))
const CAMERA = { position: [3, -3, 2.5] as [number, number, number], target: [0.5, 0.5, 0.5] as [number, number, number] }
const healthWindow = (page: Page) => page.getByRole('dialog', { name: 'Model Health' })

async function openHealth(page: Page): Promise<void> {
  await page.getByTestId('menu-bar').getByRole('button', { name: 'Object' }).click()
  await page.getByTestId('menu-bar').getByText('Model Health…', { exact: true }).click()
  await expect(healthWindow(page)).toBeVisible()
}

test('Model Health: a closed cube with a sliver face is listed and Remove degenerate faces clears it', async ({ page }) => {
  await setup(page, CAMERA)
  const ok = await page.evaluate((b) => window.__hew_test!.load(b), fixture('sliver-cube.hew'))
  expect(ok).not.toBe(false)
  await openHealth(page)
  const group = healthWindow(page).locator('[data-health-group="zeroArea"]')
  await expect(group).toContainText('Has zero-area faces (1)')
  await expect(group).toContainText('1 zero-area face')
  await group.getByText('Statue').click()
  await page.screenshot({ path: `${process.env.HEW_SHOTS_DIR ?? '/dev/null'}/health-zero-area.png` }).catch(() => undefined)
  await group.getByRole('button', { name: 'Remove degenerate faces' }).first().click()
  await expect(page.getByText('Removed 1 zero-area face.')).toBeVisible()
  await expect(healthWindow(page).locator('[data-health-group="zeroArea"]')).toHaveCount(0)
  // One undo brings the sliver back.
  await page.evaluate(() => window.__hew_test!.undo())
  await expect(healthWindow(page).locator('[data-health-group="zeroArea"]')).toHaveCount(1)
})

import { test, expect } from '@playwright/test'

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

test('narrow rail: survives a reload and every tool still activates by icon', async ({ page }) => {
  const rail = page.getByRole('radiogroup', { name: 'Tools' })
  const search = page.getByRole('button', { name: 'Search tools, actions, help' })
  await expect(search).toBeVisible()
  const wideBox = await rail.boundingBox()

  await page.getByRole('button', { name: 'Collapse tool rail' }).click()
  await expect(search).toBeHidden()
  const narrowBox = await rail.boundingBox()
  expect(narrowBox!.width).toBeLessThan(wideBox!.width / 2)

  await page.reload()
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, {
    timeout: 15_000,
  })
  await expect(page.getByRole('button', { name: 'Expand tool rail' })).toBeVisible()
  await expect(search).toBeHidden()

  const pushPull = rail.getByRole('radio', { name: 'Push/Pull' })
  await pushPull.click()
  await expect(pushPull).toHaveAttribute('aria-checked', 'true')

  await page.getByRole('button', { name: 'Expand tool rail' }).click()
  await expect(search).toBeVisible()
  await page.reload()
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, {
    timeout: 15_000,
  })
  await expect(page.getByRole('button', { name: 'Collapse tool rail' })).toBeVisible()
})

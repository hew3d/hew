import { test, expect, type Page } from '@playwright/test'

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

async function ready(page: Page) {
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, {
    timeout: 15_000,
  })
}

test.beforeEach(async ({ page }) => {
  await page.goto('/')
  await ready(page)
})

test('tray: puts away to nothing, survives a reload, and comes back at its width', async ({ page }) => {
  const canvas = page.locator('canvas').first()
  const handle = page.getByRole('separator', { name: 'Resize panels' })
  const tray = page.getByRole('complementary', { name: 'Tray' })
  const objectInfo = page.getByRole('button', { name: 'Object Info', exact: true })

  // Give the tray a non-default width so "restored" means something.
  const h = (await handle.boundingBox())!
  await page.mouse.move(h.x + h.width / 2, h.y + h.height / 2)
  await page.mouse.down()
  await page.mouse.move(h.x + h.width / 2 - 80, h.y + h.height / 2, { steps: 5 })
  await page.mouse.up()
  const trayWidth = (await tray.boundingBox())!.width
  expect(trayWidth).toBeGreaterThan(330)
  const openCanvas = (await canvas.boundingBox())!

  await page.getByRole('button', { name: 'Collapse tray' }).click()
  await expect(tray).toBeHidden()
  await expect(handle).toBeHidden()
  await expect.poll(async () => (await canvas.boundingBox())!.width).toBeGreaterThan(openCanvas.width + trayWidth / 2)

  await page.reload()
  await ready(page)
  await expect(objectInfo).toBeHidden()

  await page.getByRole('button', { name: 'Expand tray' }).click()
  await expect(objectInfo).toBeVisible()
  await expect(page.getByRole('button', { name: 'Expand tray' })).toBeHidden()
  await expect.poll(async () => (await tray.boundingBox())!.width).toBeCloseTo(trayWidth, 0)
  await expect.poll(async () => (await canvas.boundingBox())!.width).toBeCloseTo(openCanvas.width, 0)
})

test('tray: a section shortcut brings a put-away tray back with that section open', async ({ page }) => {
  const objectInfo = page.getByRole('button', { name: 'Object Info', exact: true })
  await page.getByRole('button', { name: 'Collapse tray' }).click()
  await expect(objectInfo).toBeHidden()

  await page.keyboard.press('Control+Shift+O')
  await expect(objectInfo).toBeVisible()
  await expect(objectInfo).toHaveAttribute('aria-expanded', 'true')

  // Tray open again: the same shortcut goes back to flipping the section.
  await page.keyboard.press('Control+Shift+O')
  await expect(objectInfo).toHaveAttribute('aria-expanded', 'false')
})

test('tray: View > Scenes > Add Scene brings a put-away tray back with the rename focused', async ({ page }) => {
  await page.getByRole('button', { name: 'Collapse tray' }).click()
  await expect(page.getByRole('complementary', { name: 'Tray' })).toBeHidden()

  await page.getByRole('button', { name: 'View' }).click()
  await page.getByText('Scenes', { exact: true }).last().hover()
  await page.getByText('Add Scene', { exact: true }).last().click()

  const nameInput = page.getByRole('textbox', { name: 'Scene name' })
  await expect(nameInput).toBeFocused()
  await expect(nameInput).toHaveValue('Scene 1')
  await page.keyboard.press('Escape')
  await expect(nameInput).toBeHidden()
})

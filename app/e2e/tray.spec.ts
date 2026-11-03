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

test('tray: hides to nothing, survives a reload, and comes back at its width', async ({ page }) => {
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
  // The canvas resizes on the viewport's ResizeObserver, a frame or more
  // behind the tray: wait until it ends at the handle before measuring it,
  // or a loaded machine records a mid-drag width.
  const handleLeft = (await handle.boundingBox())!.x
  await expect
    .poll(async () => {
      const c = (await canvas.boundingBox())!
      return Math.abs(c.x + c.width - handleLeft)
    })
    .toBeLessThanOrEqual(1)
  const openCanvas = (await canvas.boundingBox())!

  await page.getByRole('button', { name: 'Hide tray' }).click()
  await expect(tray).toBeHidden()
  await expect(handle).toBeHidden()
  await expect.poll(async () => (await canvas.boundingBox())!.width).toBeGreaterThan(openCanvas.width + trayWidth / 2)

  await page.reload()
  await ready(page)
  await expect(objectInfo).toBeHidden()

  await page.getByRole('button', { name: 'Show tray' }).click()
  await expect(objectInfo).toBeVisible()
  await expect(page.getByRole('button', { name: 'Show tray' })).toBeHidden()
  await expect.poll(async () => (await tray.boundingBox())!.width).toBeCloseTo(trayWidth, 0)
  await expect.poll(async () => (await canvas.boundingBox())!.width).toBeCloseTo(openCanvas.width, 0)
})

test('tray: a section shortcut brings a hidden tray back with that section open', async ({ page }) => {
  const objectInfo = page.getByRole('button', { name: 'Object Info', exact: true })
  await page.getByRole('button', { name: 'Hide tray' }).click()
  await expect(objectInfo).toBeHidden()

  await page.keyboard.press('Control+Shift+O')
  await expect(objectInfo).toBeVisible()
  await expect(objectInfo).toHaveAttribute('aria-expanded', 'true')

  // Tray shown again: the same shortcut goes back to flipping the section.
  await page.keyboard.press('Control+Shift+O')
  await expect(objectInfo).toHaveAttribute('aria-expanded', 'false')
})

test('tray: View > Scenes > Add Scene brings a hidden tray back with the rename focused', async ({ page }) => {
  await page.getByRole('button', { name: 'Hide tray' }).click()
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

test('tray: View > Tray toggles it and the menu check mark follows', async ({ page }) => {
  const tray = page.getByRole('complementary', { name: 'Tray' })
  const trayItem = async () => {
    await page.getByRole('button', { name: 'View' }).click()
    // The label shares its span with the check mark, so match either state.
    return page.getByTestId('menu-bar').getByText(/^✓?Tray$/)
  }

  const item = await trayItem()
  await expect(item).toContainText('✓')
  await item.click()
  await expect(tray).toBeHidden()

  const again = await trayItem()
  await expect(again).not.toContainText('✓')
  await again.click()
  await expect(tray).toBeVisible()
})

test('dock: View > Contextual Dock turns it off, and it stays off across a reload', async ({ page }) => {
  const dock = page.locator('[data-dock-context]')
  await expect(dock).toBeVisible()
  const dockItem = async () => {
    await page.getByRole('button', { name: 'View' }).click()
    return page.getByTestId('menu-bar').getByText(/^✓?Contextual Dock$/)
  }

  await (await dockItem()).click()
  await expect(dock).toHaveCount(0)
  await page.reload()
  await ready(page)
  await expect(dock).toHaveCount(0)
  await expect(await dockItem()).not.toContainText('✓')

  await page.getByTestId('menu-bar').getByText(/^✓?Contextual Dock$/).click()
  await expect(dock).toBeVisible()
})

test('tray: keyboard focus follows the hide and show controls', async ({ page }) => {
  const hide = page.getByRole('button', { name: 'Hide tray' })
  await hide.focus()
  await page.keyboard.press('Enter')
  const show = page.getByRole('button', { name: 'Show tray' })
  await expect(show).toBeFocused()
  await page.keyboard.press('Enter')
  await expect(page.getByRole('complementary', { name: 'Tray' })).toBeVisible()
  await expect(hide).toBeFocused()
})

test('dock: with it off, Object > Save Selection to Library… still saves a selection', async ({ page }) => {
  await page.getByRole('button', { name: 'View' }).click()
  await page.getByTestId('menu-bar').getByText(/^✓?Contextual Dock$/).click()
  await expect(page.locator('[data-dock-context]')).toHaveCount(0)

  await page.evaluate(() => {
    const h = window.__hew_test!
    const box = h.drawBox([0, 0, 0], [1, 1, 0], 1)
    h.selectNodes([{ kind: 'object', id: box }])
  })
  await page.getByRole('button', { name: 'Object', exact: true }).click()
  await page.getByTestId('menu-bar').getByText('Save Selection to Library…').click()
  await expect(page.getByRole('dialog', { name: 'Save to Library' })).toBeVisible()
})

test('tray: section heights drag independently and overflowing sections remain reachable', async ({ page }) => {
  const tray = page.getByRole('complementary', { name: 'Tray' })
  const outliner = page.getByRole('button', { name: 'Outliner', exact: true })
  if (await outliner.getAttribute('aria-expanded') === 'false') await outliner.click()
  const handle = page.getByRole('separator', { name: 'Outliner', exact: true })
  const section = handle.locator('..')
  const before = (await section.boundingBox())!.height
  await handle.scrollIntoViewIfNeeded()
  const box = (await handle.boundingBox())!
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2)
  await page.mouse.down()
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2 + 100, { steps: 5 })
  await page.mouse.up()
  const chosen = (await section.boundingBox())!.height
  expect(chosen).toBeCloseTo(before + 100, 0)
  await outliner.click()
  await outliner.click()
  expect((await section.boundingBox())!.height).toBeCloseTo(chosen, 0)

  // Grow beyond the entire tray with the accessible keyboard interaction.
  await handle.focus()
  for (let i = 0; i < 40; i++) await page.keyboard.press('ArrowDown')
  await expect.poll(async () => tray.evaluate((el) => el.scrollHeight > el.clientHeight)).toBe(true)
  const changes = page.getByRole('button', { name: 'Changes', exact: true })
  await changes.scrollIntoViewIfNeeded()
  const last = (await changes.boundingBox())!
  const bounds = (await tray.boundingBox())!
  expect(last.y).toBeGreaterThanOrEqual(bounds.y)
  expect(last.y + last.height).toBeLessThanOrEqual(bounds.y + bounds.height + 1)
  expect(await tray.evaluate((el) => el.scrollTop)).toBeGreaterThan(0)

  // Another section gets its own size without taking the Outliner's choice.
  const info = page.getByRole('button', { name: 'Object Info', exact: true })
  if (await info.getAttribute('aria-expanded') === 'false') await info.click()
  const infoHandle = page.getByRole('separator', { name: 'Object Info', exact: true })
  await infoHandle.focus()
  await page.keyboard.press('ArrowDown')
  expect((await section.boundingBox())!.height).toBeCloseTo(chosen + 800, 0)
})

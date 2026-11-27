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

test('compact rail: survives a reload and every tool still activates by icon', async ({ page }) => {
  const rail = page.getByRole('complementary', { name: 'Tool rail' })
  const search = page.getByRole('button', { name: 'Search tools, actions, help' })
  await expect(search).toContainText('Search…')
  const wideBox = await rail.boundingBox()

  await page.getByRole('button', { name: 'Compact tool rail' }).click()
  await expect(search).not.toContainText('Search…')
  const narrowBox = await rail.boundingBox()
  expect(narrowBox!.width).toBeLessThan(wideBox!.width / 2)

  await page.reload()
  await ready(page)
  await expect(page.getByRole('button', { name: 'Expand tool rail' })).toBeVisible()
  await expect(search).not.toContainText('Search…')

  const pushPull = page.getByRole('radiogroup', { name: 'Tools' }).getByRole('radio', { name: 'Push/Pull' })
  await pushPull.click()
  await expect(pushPull).toHaveAttribute('aria-checked', 'true')

  await page.getByRole('button', { name: 'Expand tool rail' }).click()
  await expect(search).toContainText('Search…')
  await page.reload()
  await ready(page)
  await expect(page.getByRole('button', { name: 'Compact tool rail' })).toBeVisible()
})

test('compact rail: icons sit centered with no horizontal scroll, and search still opens the palette', async ({ page }) => {
  await page.getByRole('button', { name: 'Compact tool rail' }).click()
  const rail = page.getByRole('complementary', { name: 'Tool rail' })
  const geometry = await rail.evaluate((el) => {
    const railBox = el.getBoundingClientRect()
    const icons = [...el.querySelectorAll('[role=radio] > span, button[aria-label="Search tools, actions, help"] > span')]
    const centers = icons.map((i) => {
      const b = i.getBoundingClientRect()
      return b.left + b.width / 2
    })
    return {
      overflow: el.scrollWidth - el.clientWidth,
      railCenter: railBox.left + (railBox.width - 1) / 2, // less the hairline border-right
      centers,
    }
  })
  expect(geometry.overflow).toBe(0)
  expect(geometry.centers.length).toBeGreaterThan(5)
  for (const c of geometry.centers) expect(Math.abs(c - geometry.railCenter)).toBeLessThanOrEqual(1)

  await page.getByRole('button', { name: 'Search tools, actions, help' }).click()
  await expect(page.getByPlaceholder('Search tools, actions, help…')).toBeFocused()
})

test('compact rail: View > Compact Tool Rail toggles it with a check mark', async ({ page }) => {
  const openItem = async () => {
    await page.getByTestId('menu-bar').getByRole('button', { name: 'View' }).click()
    // The label shares its span with the check mark, so match either state.
    return page.getByTestId('menu-bar').getByText(/^✓?Compact Tool Rail$/)
  }
  const item = await openItem()
  await expect(item).not.toContainText('✓')
  await item.click()
  await expect(page.getByRole('button', { name: 'Expand tool rail' })).toBeVisible()
  await expect(await openItem()).toContainText('✓')
})

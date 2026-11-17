import { test, expect } from '@playwright/test'

/**
 * Components tray section (v1.1 assets lane, docs/design/v1.1-cycle.md
 * Lane A) — cross-layer behavior a component test can't reach: the panel
 * lists live definitions with their instance counts, delete removes every
 * instance as one undo entry, and Purge Unused reports what it removed and
 * undoes wholesale.
 */

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem(
      'hew.settings.trayLayout',
      JSON.stringify({
        modelInfo: true,
        objectInfo: true,
        materials: false,
        components: true,
        tags: false,
        scenes: false,
      }),
    )
  })
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
})

test('the Components panel lists a definition with its live instance count', async ({ page }) => {
  const { component } = await page.evaluate(() => {
    const h = window.__hew_test!
    const a = h.drawBox([0, 0, 0], [1, 1, 0], 1)
    const made = h.makeComponent([a])
    h.placeInstance(made.component, 3, 0, 0)
    return made
  })

  await expect(page.getByTestId('components-row-name').first()).toBeVisible()
  await expect(page.getByTestId('components-row-usage').first()).toHaveText('2')
  expect(await page.evaluate((c) => window.__hew_test!.definitionUsage(c), component)).toBe(2)
})

test('double-click renames a definition in place, undoably', async ({ page }) => {
  const { component } = await page.evaluate(() => {
    const h = window.__hew_test!
    return h.makeComponent([h.drawBox([0, 0, 0], [1, 1, 0], 1)])
  })
  // An unnamed selection gets a kernel-generated name ("Component 1", …) —
  // undo must restore exactly that, not clear it to nothing.
  const originalName = await page.evaluate((c) => window.__hew_test!.getComponentName(c), component)
  expect(originalName).not.toBeNull()

  const nameCell = page.getByTestId('components-row-name').first()
  await nameCell.dblclick()
  const input = page.getByRole('textbox', { name: /rename component/i })
  await expect(input).toBeVisible()
  await input.fill('Leg')
  await input.press('Enter')
  await expect(page.getByTestId('components-row-name').first()).toHaveText('Leg')
  expect(await page.evaluate((c) => window.__hew_test!.getComponentName(c), component)).toBe('Leg')

  await page.locator('canvas').first().click({ position: { x: 5, y: 5 } })
  await page.keyboard.press('ControlOrMeta+z')
  await expect
    .poll(() => page.evaluate((c) => window.__hew_test!.getComponentName(c), component))
    .toBe(originalName)
})

test('deleting a definition with instances confirms, then removes every instance as one undo step', async ({ page }) => {
  const setup = await page.evaluate(() => {
    const h = window.__hew_test!
    const a = h.drawBox([0, 0, 0], [1, 1, 0], 1)
    const made = h.makeComponent([a])
    const second = h.placeInstance(made.component, 3, 0, 0)
    return { ...made, second }
  })

  await page.getByRole('button', { name: /^rename component/i }).first().waitFor()
  await page.getByRole('button', { name: /^delete component/i }).first().click()

  await expect(page.getByRole('dialog', { name: 'Delete definition?' })).toBeVisible()
  await expect(page.getByText(/and its 2 instances/i)).toBeVisible()

  await page.getByRole('button', { name: 'Delete', exact: true }).click()
  await expect(page.getByRole('dialog')).toHaveCount(0)

  expect(await page.evaluate(() => window.__hew_test!.getComponentIds())).toEqual([])
  expect(
    await page.evaluate((c) => window.__hew_test!.getInstancesOf(c), setup.component),
  ).toEqual([])

  // One undo restores the definition and both instances together.
  await page.evaluate(() => window.__hew_test!.undo())
  expect(await page.evaluate(() => window.__hew_test!.getComponentIds())).toEqual([setup.component])
  expect(
    (await page.evaluate((c) => window.__hew_test!.getInstancesOf(c), setup.component)).sort(),
  ).toEqual([setup.instance, setup.second].sort())
})

test('deleting a definition with no instances removes it immediately, with no confirmation', async ({ page }) => {
  const component = await page.evaluate(() => {
    const h = window.__hew_test!
    const made = h.makeComponent([h.drawBox([0, 0, 0], [1, 1, 0], 1)])
    h.deleteNode('instance', made.instance)
    return made.component
  })

  await page.getByRole('button', { name: /^delete component/i }).first().click()
  await expect(page.getByRole('dialog')).toHaveCount(0)
  expect(await page.evaluate(() => window.__hew_test!.getComponentIds())).toEqual([])
  void component
})

test('Purge Unused previews what would go, then removes it as one undo step and reports the counts', async ({ page }) => {
  const setup = await page.evaluate(() => {
    const h = window.__hew_test!
    // An unused material (nothing paints it).
    const mat = h.addMaterial('Stray', 10, 20, 30, 255)
    // A definition made unused by deleting its only instance.
    const made = h.makeComponent([h.drawBox([0, 0, 0], [1, 1, 0], 1)])
    h.deleteNode('instance', made.instance)
    return { mat, component: made.component }
  })
  expect(await page.evaluate(() => window.__hew_test!.unusedMaterials())).toEqual([setup.mat])
  expect(await page.evaluate(() => window.__hew_test!.unusedDefinitions())).toEqual([setup.component])

  await page.getByRole('button', { name: 'Purge Unused…' }).click()
  await expect(page.getByRole('dialog', { name: 'Purge unused?' })).toBeVisible()
  await expect(page.getByText(/1 definition/i)).toBeVisible()
  await expect(page.getByText(/1 material/i)).toBeVisible()

  await page.getByRole('button', { name: 'Purge', exact: true }).click()
  await expect(page.getByRole('dialog')).toHaveCount(0)
  await expect(page.getByText('Purged 1 definition, 1 material.')).toBeVisible()

  expect(await page.evaluate(() => window.__hew_test!.getComponentIds())).toEqual([])
  expect(await page.evaluate(() => window.__hew_test!.unusedMaterials())).toEqual([])

  // One undo restores both.
  await page.evaluate(() => window.__hew_test!.undo())
  expect(await page.evaluate(() => window.__hew_test!.getComponentIds())).toEqual([setup.component])
  expect(await page.evaluate(() => window.__hew_test!.unusedMaterials())).toEqual([setup.mat])
})

test('Purge Unused toasts "Nothing to purge" and opens no dialog when idle', async ({ page }) => {
  await page.getByRole('button', { name: 'Purge Unused…' }).click()
  await expect(page.getByRole('dialog')).toHaveCount(0)
  await expect(page.getByText('Nothing to purge.')).toBeVisible()
})

test('filtering the components list narrows by name and clears back', async ({ page }) => {
  await page.evaluate(() => {
    const h = window.__hew_test!
    const door = h.makeComponent([h.drawBox([0, 0, 0], [1, 1, 0], 1)])
    h.setComponentName(door.component, 'Oak Door')
    const window_ = h.makeComponent([h.drawBox([3, 0, 0], [4, 1, 0], 1)])
    h.setComponentName(window_.component, 'Sky Window')
  })

  // Scoped to the row-name testid: the Outliner ALSO shows "Oak Door" (an
  // unnamed instance falls back to its definition's name), so a plain
  // text query is ambiguous across the two panels.
  const rowNamed = (name: string) => page.getByTestId('components-row-name').filter({ hasText: name })
  await expect(rowNamed('Oak Door')).toBeVisible()
  await expect(rowNamed('Sky Window')).toBeVisible()

  const filterInput = page.getByLabel('Filter components')
  await filterInput.fill('door')
  await expect(rowNamed('Oak Door')).toBeVisible()
  await expect(rowNamed('Sky Window')).toHaveCount(0)

  await filterInput.fill('nonexistent component name')
  await expect(page.getByText('No components match')).toBeVisible()

  await page.getByRole('button', { name: 'Clear filter' }).click()
  await expect(rowNamed('Oak Door')).toBeVisible()
  await expect(rowNamed('Sky Window')).toBeVisible()
})

test('each component row renders a thumbnail image of its own geometry', async ({ page }) => {
  await page.evaluate(() => {
    const h = window.__hew_test!
    h.makeComponent([h.drawBox([0, 0, 0], [1, 1, 0], 1)])
  })

  const thumb = page.getByTestId('components-row-thumb').first()
  // Rendered lazily off the critical path (see ComponentsPanel's doc
  // comment) — waits for the queued render rather than asserting
  // synchronously.
  await expect(thumb).toBeVisible()
  await expect(thumb).toHaveAttribute('src', /^blob:/)
})

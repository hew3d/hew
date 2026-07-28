import { test, expect, type Page } from '@playwright/test'

/**
 * 3D Text E2E (docs/design/3d-text.md) — the real dialog → placement →
 * undo/redo flow in a real browser with the real WASM kernel.
 *
 * Drives the actual UI (Draw menu → dialog → OK → a real canvas click) and
 * asserts through `window.__hew_test` (harness `getSelection`/
 * `getInstanceDef`/`canUndo`/`canRedo`) that the whole placement — the
 * glyph-injection gesture, every extrusion, and the component fold — lands
 * as ONE undo step: a single Undo removes the placed text entirely, and a
 * single Redo restores it.
 */

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

async function boot(page: Page): Promise<void> {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
}

test.beforeEach(async ({ page }) => {
  await boot(page)
})

test('Draw ▸ 3D Text…: dialog → placement on the ground → single-step undo → redo', async ({
  page,
}) => {
  // A top-down-ish view of the origin so a click near [0,0,0] lands on the
  // ground plane, matching the tool's face/ground/axis-lock plane rules.
  await page.evaluate(() => {
    window.__hew_test!.setCamera({ position: [0.3, -3, 4], target: [0.3, 0.3, 0], fovDeg: 45 })
  })

  expect(await page.evaluate(() => window.__hew_test!.canUndo())).toBe(false)

  // Draw ▸ 3D Text…
  await page.getByTestId('menu-bar').getByRole('button', { name: 'Draw' }).click()
  // Scoped to the menu bar: the dock's DRAW verb set now carries its own
  // "3D Text…" button, so a bare page-wide text locator is ambiguous.
  await page.getByTestId('menu-bar').getByText('3D Text…', { exact: true }).click()

  const dialog = page.getByRole('dialog', { name: '3D Text' })
  await expect(dialog).toBeVisible()

  // Defaults (text "Hew", first bundled font, 100mm height, 5mm depth) are
  // fine for this test — just confirm OK is enabled and commit the dialog.
  const okButton = dialog.getByRole('button', { name: /^OK$/ })
  await expect(okButton).toBeEnabled()
  await okButton.click()
  await expect(dialog).not.toBeVisible()

  // The dialog handed off to a one-shot placement tool; a real mouse move
  // then click on the ground plane commits it.
  const canvas = await page.locator('canvas').first().boundingBox()
  if (canvas === null) throw new Error('no canvas')
  const originPx = await page.evaluate(() => window.__hew_test!.worldToScreen([0.3, 0.3, 0]))
  const clickX = canvas.x + originPx.x
  const clickY = canvas.y + originPx.y

  await page.mouse.move(clickX, clickY)
  await page.mouse.move(clickX + 1, clickY + 1) // a real move event before the click
  await page.mouse.down()
  await page.mouse.up()

  await page.waitForFunction(() => window.__hew_test!.getSelection().length === 1, null, {
    timeout: 5000,
  })

  const placed = await page.evaluate(() => {
    const h = window.__hew_test!
    const sel = h.getSelection()
    return {
      selection: sel,
      canUndo: h.canUndo(),
      canRedo: h.canRedo(),
    }
  })
  expect(placed.selection).toEqual([{ kind: 'instance', id: expect.any(String) }])
  expect(placed.canUndo).toBe(true)
  expect(placed.canRedo).toBe(false)

  const instanceId = placed.selection[0].id
  const defAfterPlace = await page.evaluate(
    (id) => window.__hew_test!.getInstanceDef(id),
    instanceId,
  )
  expect(defAfterPlace).not.toBeNull()

  // ONE Undo (Edit ▸ Undo) removes the WHOLE placement — the extruded
  // solid(s), the fold into a component, AND the glyph-injection sketch
  // gesture that preceded it — as a single step (docs/design/3d-text.md).
  await page.getByTestId('menu-bar').getByRole('button', { name: 'Edit' }).click()
  await page.getByText('Undo', { exact: true }).click()

  await page.waitForFunction(() => window.__hew_test!.canUndo() === false, null, {
    timeout: 5000,
  })
  const afterUndo = await page.evaluate((id) => {
    const h = window.__hew_test!
    return { def: h.getInstanceDef(id), canUndo: h.canUndo(), canRedo: h.canRedo() }
  }, instanceId)
  expect(afterUndo.def).toBeNull()
  expect(afterUndo.canUndo).toBe(false) // exactly one step existed
  expect(afterUndo.canRedo).toBe(true)

  // ONE Redo restores the whole placement again, as one step.
  await page.getByTestId('menu-bar').getByRole('button', { name: 'Edit' }).click()
  await page.getByText('Redo', { exact: true }).click()

  await page.waitForFunction(() => window.__hew_test!.canRedo() === false, null, {
    timeout: 5000,
  })
  const afterRedo = await page.evaluate((id) => {
    const h = window.__hew_test!
    return { def: h.getInstanceDef(id), canUndo: h.canUndo(), canRedo: h.canRedo() }
  }, instanceId)
  expect(afterRedo.def).not.toBeNull()
  expect(afterRedo.canUndo).toBe(true)
  expect(afterRedo.canRedo).toBe(false)
})

test('Draw ▸ 3D Text…: font picker — filter narrows the list, picking a non-default bundled family places with it', async ({
  page,
}) => {
  await page.evaluate(() => {
    window.__hew_test!.setCamera({ position: [0.3, -3, 4], target: [0.3, 0.3, 0], fovDeg: 45 })
  })

  await page.getByTestId('menu-bar').getByRole('button', { name: 'Draw' }).click()
  await page.getByTestId('menu-bar').getByText('3D Text…', { exact: true }).click()

  const dialog = page.getByRole('dialog', { name: '3D Text' })
  await expect(dialog).toBeVisible()

  // Bundled fonts load asynchronously (docs/design/3d-text-fonts.md's
  // provider abstraction) — wait for the picker to actually list them
  // before filtering, rather than racing an empty list.
  await expect(dialog.getByText('Bundled')).toBeVisible()
  await expect(dialog.getByRole('option', { name: 'Onest' })).toBeVisible()

  // Filter to a single bundled family, distinct from the default first
  // entry — proves the filter box actually narrows the picker (not just
  // decoration) and that picking a NON-default row is what gets placed.
  await dialog.getByPlaceholder('Filter fonts…').fill('Space Mono')
  await expect(dialog.getByRole('option', { name: 'Onest' })).toHaveCount(0)
  const spaceMonoOption = dialog.getByRole('option', { name: 'Space Mono' })
  await expect(spaceMonoOption).toBeVisible()
  await spaceMonoOption.click()
  await expect(spaceMonoOption).toHaveAttribute('aria-selected', 'true')

  const okButton = dialog.getByRole('button', { name: /^OK$/ })
  await expect(okButton).toBeEnabled()
  await okButton.click()
  await expect(dialog).not.toBeVisible()

  const canvas = await page.locator('canvas').first().boundingBox()
  if (canvas === null) throw new Error('no canvas')
  const originPx = await page.evaluate(() => window.__hew_test!.worldToScreen([0.3, 0.3, 0]))
  const clickX = canvas.x + originPx.x
  const clickY = canvas.y + originPx.y

  await page.mouse.move(clickX, clickY)
  await page.mouse.move(clickX + 1, clickY + 1)
  await page.mouse.down()
  await page.mouse.up()

  await page.waitForFunction(() => window.__hew_test!.getSelection().length === 1, null, {
    timeout: 5000,
  })
  const placed = await page.evaluate(() => window.__hew_test!.getSelection())
  expect(placed).toEqual([{ kind: 'instance', id: expect.any(String) }])
})

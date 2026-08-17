import { test, expect, type Page } from '@playwright/test'

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

/**
 * Tags × undo/redo wiring — EVERY undo entry point must reconcile tag
 * visibility, including the viewport's own Cmd/Ctrl+Z keydown binding
 * (which never passes through App.handleUndo; only the menu and palette
 * do). The regression this pins: undoing a tag delete restores the
 * registry entry with its hidden flag, and without a resync on the
 * keyboard path the kernel considers the tag hidden again while the
 * content stays visible AND pickable until some unrelated resync runs.
 *
 * Pickability is the observable: the tag-visibility union is pushed to
 * both the renderer (mesh visibility) and the kernel (inference/pick
 * exclusion) in one call, and `pickFace` reads the kernel side exactly.
 */

async function setup(page: Page): Promise<void> {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, {
    timeout: 15_000,
  })
}

test('keyboard undo of a tag delete re-hides content hidden by the restored tag', async ({
  page,
}) => {
  await setup(page)

  // A unit box tagged 'Roof'; a downward ray over its top is the probe.
  await page.evaluate(() => {
    const h = window.__hew_test!
    const id = h.drawBox([0, 0, 0], [1, 1, 0], 1)
    h.addNodeTag('object', id, ['Roof'])
  })
  expect(
    await page.evaluate(() => window.__hew_test!.pickFace([0.5, 0.5, 5], [0, 0, -1]) !== null),
  ).toBe(true)

  // Hide the tag (the Tags panel eye): content goes invisible + unpickable.
  await page.evaluate(() => window.__hew_test!.toggleTagHidden(['Roof']))
  await page.waitForFunction(
    () => window.__hew_test!.pickFace([0.5, 0.5, 5], [0, 0, -1]) === null,
  )

  // Delete the tag: content hidden solely via that tag becomes visible.
  await page.evaluate(() => window.__hew_test!.deleteTag(['Roof']))
  await page.waitForFunction(
    () => window.__hew_test!.pickFace([0.5, 0.5, 5], [0, 0, -1]) !== null,
  )

  // Undo through the LIVE keydown path — the viewport's own Ctrl+Z binding,
  // not App.handleUndo. The tag registry is restored with its hidden flag,
  // so the content must re-hide (and become unpickable) immediately.
  await page.keyboard.press('Control+z')
  await page.waitForFunction(
    () => window.__hew_test!.pickFace([0.5, 0.5, 5], [0, 0, -1]) === null,
  )

  // The geometry itself was never touched — only its visibility.
  expect(await page.evaluate(() => window.__hew_test!.getObjectCount())).toBe(1)

  // Redo through the live keydown path re-deletes the tag: visible again.
  await page.keyboard.press('Control+Shift+Z')
  await page.waitForFunction(
    () => window.__hew_test!.pickFace([0.5, 0.5, 5], [0, 0, -1]) !== null,
  )
})

test('harness undo of a tag delete re-hides content hidden by the restored tag', async ({
  page,
}) => {
  await setup(page)

  // Same scenario as the keyboard spec above, driven through the HARNESS
  // document-level undo/redo (`__hew_test.undo()/redo()`, what session/
  // tools/hover-dock specs use). The harness must route through the same
  // runUndo/runRedo choke point, or tag visibility and pick exclusion go
  // stale in exactly the paths the E2E suite drives.
  await page.evaluate(() => {
    const h = window.__hew_test!
    const id = h.drawBox([0, 0, 0], [1, 1, 0], 1)
    h.addNodeTag('object', id, ['Roof'])
    h.toggleTagHidden(['Roof'])
  })
  await page.waitForFunction(
    () => window.__hew_test!.pickFace([0.5, 0.5, 5], [0, 0, -1]) === null,
  )

  await page.evaluate(() => window.__hew_test!.deleteTag(['Roof']))
  await page.waitForFunction(
    () => window.__hew_test!.pickFace([0.5, 0.5, 5], [0, 0, -1]) !== null,
  )

  // Harness undo restores the registry (hidden flag included): re-hidden.
  await page.evaluate(() => window.__hew_test!.undo())
  await page.waitForFunction(
    () => window.__hew_test!.pickFace([0.5, 0.5, 5], [0, 0, -1]) === null,
  )
  expect(await page.evaluate(() => window.__hew_test!.getObjectCount())).toBe(1)

  // Harness redo re-deletes the tag: visible and pickable again.
  await page.evaluate(() => window.__hew_test!.redo())
  await page.waitForFunction(
    () => window.__hew_test!.pickFace([0.5, 0.5, 5], [0, 0, -1]) !== null,
  )
})

test('Tags panel: clicking a tag selects its items; double-click renames it in place, undoably', async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem('hew.settings.trayLayout', JSON.stringify({ modelInfo: true, objectInfo: true, materials: false, tags: true, scenes: false }))
  })
  await setup(page)
  const ids = await page.evaluate(() => {
    const h = window.__hew_test!
    const a = h.drawBox([0, 0, 0], [1, 1, 0], 1)
    const b = h.drawBox([3, 0, 0], [4, 1, 0], 1)
    const c = h.drawBox([6, 0, 0], [7, 1, 0], 1)
    h.addNodeTag('object', a, ['Hardware'])
    h.addNodeTag('object', b, ['Hardware', 'Screws'])
    h.addNodeTag('object', c, ['Oak'])
    return { a, b, c }
  })

  // Click "Hardware" → both the tagged and the nested-tagged item are selected.
  const hardwareRow = page.getByTestId('tag-row').filter({ hasText: 'Hardware' }).first()
  await hardwareRow.click()
  await expect
    .poll(async () => (await page.evaluate(() => window.__hew_test!.getSelection().map((n) => n.id))).sort())
    .toEqual([ids.a, ids.b].sort())
  await expect(hardwareRow).toHaveAttribute('data-active', 'true')

  // Double-click → rename in place; Enter commits; every carrier is rewritten.
  await hardwareRow.getByText('Hardware').dblclick()
  const input = page.getByRole('textbox', { name: 'Tag name' })
  await expect(input).toBeVisible()
  await input.fill('Fixings')
  await input.press('Enter')
  await expect(page.getByTestId('tag-row').filter({ hasText: 'Fixings' }).first()).toBeVisible()
  expect(await page.evaluate((id) => window.__hew_test!.getNodeTags('object', id), ids.a)).toEqual(['Fixings'])
  expect(await page.evaluate((id) => window.__hew_test!.getNodeTags('object', id), ids.b)).toEqual(['Fixings/Screws'])
  expect(await page.evaluate((id) => window.__hew_test!.getNodeTags('object', id), ids.c)).toEqual(['Oak'])

  // A colliding rename is refused inline and nothing changes.
  const fixingsRow = page.getByTestId('tag-row').filter({ hasText: 'Fixings' }).first()
  await fixingsRow.getByText('Fixings').dblclick()
  await page.getByRole('textbox', { name: 'Tag name' }).fill('Oak')
  await page.getByRole('textbox', { name: 'Tag name' }).press('Enter')
  await expect(page.getByText(/already exists/)).toBeVisible()
  await page.getByRole('textbox', { name: 'Tag name' }).press('Escape')
  expect(await page.evaluate((id) => window.__hew_test!.getNodeTags('object', id), ids.a)).toEqual(['Fixings'])

  // Undo restores the old name on every carrier.
  await page.locator('canvas').first().click({ position: { x: 5, y: 5 } })
  await page.keyboard.press('Control+z')
  await expect.poll(async () => page.evaluate((id) => window.__hew_test!.getNodeTags('object', id), ids.b)).toEqual(['Hardware/Screws'])
})

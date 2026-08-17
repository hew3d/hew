import { test, expect, type Page } from '@playwright/test'

/**
 * Scenes E2E (docs/design/scenes.md §5, §8) — the editor half, driven the
 * way a user does: the tray's ⊕ Add Scene button, real row clicks, real
 * keys (Page Down / Page Up, Enter/Escape in the delete dialog), with the
 * kernel/viewport observed through `window.__hew_test`.
 *
 * Covers: Add captures the view and activates; orbiting away + hiding a
 * tag marks the active row drifted; clicking the row restores camera AND
 * tag visibility (the kernel-owned union path); Page Down/Up cycle;
 * delete asks and Escape dismisses; the section plane persists across
 * save/reload and a Scene restores it.
 */

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

const HOME = { position: [8, 6, 8] as [number, number, number], target: [1, 1, 1] as [number, number, number], up: [0, 0, 1] as [number, number, number], fovDeg: 45 }
const AWAY = { position: [-6, -9, 4] as [number, number, number], target: [0, 0, 0] as [number, number, number], up: [0, 0, 1] as [number, number, number], fovDeg: 45 }

async function boot(page: Page): Promise<void> {
  // The tray's Scenes section is open by default (trayLayout.scenes = true);
  // pin it (and transitions OFF, so camera assertions don't race a tween).
  await page.addInitScript(() => {
    localStorage.setItem('hew.settings.trayLayout', JSON.stringify({ modelInfo: true, objectInfo: true, materials: false, tags: true, scenes: true }))
    localStorage.setItem('hew.settings.sceneTransitions', 'false')
    localStorage.setItem('hew:shellMode', 'editor')
  })
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
  await page.evaluate(() => {
    const h = window.__hew_test!
    const a = h.drawBox([0, 0, 0], [1, 1, 0], 1)
    h.drawBox([3, 0, 0], [4, 1, 0], 1)
    h.addNodeTag('object', a, ['Hardware'])
  })
  await page.evaluate((cam) => window.__hew_test!.setCamera(cam), HOME)
}

function rows(page: Page) {
  return page.getByTestId('scene-row')
}

async function eyeDistance(page: Page, to: [number, number, number]): Promise<number> {
  return page.evaluate((target) => {
    const c = window.__hew_test!.getCameraState()
    return Math.hypot(c.eye[0] - target[0], c.eye[1] - target[1], c.eye[2] - target[2])
  }, to)
}

test('Add Scene captures the view; drift shows; clicking the row restores camera and tag visibility', async ({ page }) => {
  await boot(page)
  await expect(page.getByText('No Scenes yet', { exact: false })).toBeVisible()

  await page.getByRole('button', { name: 'Add Scene' }).first().click()
  // Add opens inline rename with the auto-name selected; Escape keeps it.
  const nameInput = page.getByRole('textbox', { name: 'Scene name' })
  await expect(nameInput).toBeVisible()
  await expect(nameInput).toHaveValue('Scene 1')
  await page.keyboard.press('Escape')
  await expect(rows(page)).toHaveCount(1)
  await expect(rows(page).first()).toHaveAttribute('data-active', 'true')
  await expect(rows(page).first()).toHaveAttribute('data-drifted', 'false')

  // Orbit away and hide the tag → the active row reads drifted.
  await page.evaluate((cam) => window.__hew_test!.setCamera(cam), AWAY)
  await page.evaluate(() => window.__hew_test!.toggleTagHidden(['Hardware']))
  await expect(rows(page).first()).toHaveAttribute('data-drifted', 'true', { timeout: 5_000 })
  await expect(page.getByRole('button', { name: /^Update Scene/ })).toBeVisible()
  expect(await page.evaluate(() => window.__hew_test!.isTagHidden(['Hardware']))).toBe(true)

  // Activate → camera back home, tag visible again, drift cleared.
  await rows(page).first().click()
  await expect
    .poll(async () => eyeDistance(page, HOME.position), { timeout: 5_000 })
    .toBeLessThan(1e-3)
  expect(await page.evaluate(() => window.__hew_test!.isTagHidden(['Hardware']))).toBe(false)
  await expect(rows(page).first()).toHaveAttribute('data-drifted', 'false', { timeout: 5_000 })

  // Update re-captures: hide the tag, orbit, Update → activating later lands there.
  await page.evaluate(() => window.__hew_test!.toggleTagHidden(['Hardware']))
  await page.evaluate((cam) => window.__hew_test!.setCamera(cam), AWAY)
  await expect(rows(page).first()).toHaveAttribute('data-drifted', 'true', { timeout: 5_000 })
  await page.getByRole('button', { name: /^Update Scene/ }).click()
  await expect(rows(page).first()).toHaveAttribute('data-drifted', 'false', { timeout: 5_000 })
  await page.evaluate((cam) => window.__hew_test!.setCamera(cam), HOME)
  await page.evaluate(() => window.__hew_test!.toggleTagHidden(['Hardware']))
  await rows(page).first().click()
  await expect
    .poll(async () => eyeDistance(page, AWAY.position), { timeout: 5_000 })
    .toBeLessThan(1e-3)
  expect(await page.evaluate(() => window.__hew_test!.isTagHidden(['Hardware']))).toBe(true)
})

test('Page Down / Page Up cycle Scenes with wrap; delete asks and Escape dismisses', async ({ page }) => {
  await boot(page)
  await page.getByRole('button', { name: 'Add Scene' }).first().click()
  await page.keyboard.press('Escape')
  await page.evaluate((cam) => window.__hew_test!.setCamera(cam), AWAY)
  await page.getByRole('button', { name: 'Add Scene' }).first().click()
  await page.keyboard.press('Escape')
  await expect(rows(page)).toHaveCount(2)
  await expect(rows(page).nth(1)).toHaveAttribute('data-active', 'true')

  // Blur the tray so the global shortcut handler sees the key.
  await page.locator('canvas').first().click({ position: { x: 5, y: 5 } })
  await page.keyboard.press('PageDown')
  await expect(rows(page).nth(0)).toHaveAttribute('data-active', 'true', { timeout: 5_000 })
  await expect
    .poll(async () => eyeDistance(page, HOME.position), { timeout: 5_000 })
    .toBeLessThan(1e-3)
  await page.keyboard.press('PageUp')
  await expect(rows(page).nth(1)).toHaveAttribute('data-active', 'true', { timeout: 5_000 })

  // Expand the second row's details, ask to delete, Escape keeps it.
  await rows(page).nth(1).getByRole('button', { name: 'Expand Scene details' }).click()
  await page.getByRole('button', { name: 'Delete', exact: true }).click()
  const dialog = page.getByRole('dialog', { name: /^Delete "Scene 2"\?$/ })
  await expect(dialog).toBeVisible()
  await expect(dialog.getByRole('button', { name: 'Cancel' })).toBeFocused()
  await page.keyboard.press('Escape')
  await expect(dialog).not.toBeVisible()
  await expect(rows(page)).toHaveCount(2)
  // Confirm for real.
  await page.getByRole('button', { name: 'Delete', exact: true }).click()
  await page.getByRole('dialog').getByRole('button', { name: 'Delete', exact: true }).click()
  await expect(rows(page)).toHaveCount(1)
})

test('the section plane persists across save/reload and a Scene restores it', async ({ page }) => {
  await boot(page)
  // Drive the real tool: activate Section Plane and click the ground.
  await page.getByRole('radiogroup', { name: 'Tools' }).getByRole('radio', { name: 'Section Plane' }).click()
  const box = await page.locator('canvas').first().boundingBox()
  if (box === null) throw new Error('viewport canvas has no bounding box')
  await page.mouse.move(box.x + box.width * 0.5, box.y + box.height * 0.8)
  await page.mouse.down()
  await page.mouse.up()
  await page.waitForFunction(() => window.__hew_test!.getSectionState() !== null)
  const before = await page.evaluate(() => window.__hew_test!.getSectionState())

  // Capture a Scene with the plane; then delete the plane; activate → back.
  await page.getByRole('button', { name: 'Add Scene' }).first().click()
  await page.keyboard.press('Escape')
  await page.keyboard.press('Space') // back to Select
  await page.getByRole('radiogroup', { name: 'Tools' }).getByRole('radio', { name: 'Section Plane' }).click()
  await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur())
  await page.keyboard.press('Delete')
  await page.waitForFunction(() => window.__hew_test!.getSectionState() === null)
  await rows(page).first().click()
  await page.waitForFunction(() => window.__hew_test!.getSectionState() !== null)
  const restored = await page.evaluate(() => window.__hew_test!.getSectionState())
  expect(restored?.active).toBe(before?.active)
  expect(restored?.origin.map((v) => Math.round(v * 1e6) / 1e6)).toEqual(before?.origin.map((v) => Math.round(v * 1e6) / 1e6))

  // Save/reload round trip keeps the plane and the Scene.
  const bytes = await page.evaluate(() => window.__hew_test!.save())
  await page.evaluate((b) => window.__hew_test!.load(b), bytes)
  await page.waitForFunction(() => window.__hew_test!.getSectionState() !== null)
  await expect(rows(page)).toHaveCount(1)
})

import { test, expect, type Page } from '@playwright/test'
import type { CameraParams } from './helpers/projectWorldToScreen'
import { aim, clickAt, moveTo, shot } from './helpers/shapes'
import { offeredTools, requiresPreview } from './helpers/preview'

/**
 * Where the True Curves tools live (docs/design/true-curves-ui-organization.md):
 * the lean rail, the Tools menu's Shape and Deform submenus, the Object menu's
 * Smooth entries, and the contextual dock's rows. Every interaction is a real
 * pointer click; scenes are built through the harness. With HEW_SHOTS_DIR set,
 * each surface is also saved as a screenshot.
 */

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

const CAMERA: CameraParams = {
  position: { x: 4, y: -6, z: 5 },
  target: { x: -0.2, y: 0.5, z: 0 },
  up: { x: 0, y: 0, z: 1 },
  fovDeg: 45,
  near: 0.1,
  far: 1000,
}

const menuBar = (page: Page) => page.getByTestId('menu-bar')
const dock = (page: Page) => page.getByTestId('contextual-dock')
const dockLabels = (page: Page) =>
  dock(page).getByRole('button').evaluateAll((els) => els.map((e) => e.getAttribute('title')))

async function openMenu(page: Page, name: RegExp): Promise<void> {
  await menuBar(page).getByRole('button', { name }).click()
}

test('rail: Modify is Push/Pull, Follow Me, Offset, Fillet, Loft, Move, Rotate, Scale, Flip, Eraser; Revolve and Deform are off it', async ({ page }) => {
  await aim(page, CAMERA)
  const names = await page
    .getByRole('radiogroup', { name: 'Tools' })
    .getByRole('radio')
    .evaluateAll((els) => els.map((e) => e.getAttribute('aria-label')))
  // A production-shape build has neither Fillet nor Loft; the Eraser is in every build.
  const expected = offeredTools(['Push/Pull', 'Follow Me', 'Offset', 'Fillet', 'Loft', 'Move', 'Rotate', 'Scale', 'Flip', 'Eraser'])
  const modify = names.slice(names.indexOf('Push/Pull'), names.indexOf(expected[expected.length - 1]) + 1)
  expect(modify).toEqual(expected)
  expect(names).not.toContain('Revolve')
  expect(names).not.toContain('Deform')
  await shot(page, 'rail')
})

test('Tools > Shape > Revolve from the menu bar starts Revolve', async ({ page }) => {
  requiresPreview('revolve')
  await aim(page, CAMERA)
  await openMenu(page, /^tools$/i)
  await shot(page, 'menu-tools')
  // The family is not on the flat list; hovering the Shape row opens it.
  await expect(menuBar(page).getByText('Revolve', { exact: true })).toHaveCount(0)
  await menuBar(page).getByText('Shape', { exact: true }).hover()
  await expect(menuBar(page).getByText('Revolve', { exact: true })).toBeVisible()
  for (const name of ['Loft', 'Shell', 'Project', 'Array along Path']) {
    await expect(menuBar(page).getByText(name, { exact: true })).toBeVisible()
  }
  await shot(page, 'menu-tools-shape')
  await menuBar(page).getByText('Revolve', { exact: true }).click()
  await expect(page.getByText('Click a drawn shape or a solid face to revolve.')).toBeVisible()
})

test('Tools > Deform opens Twist, Bend, Taper, Bend to Path', async ({ page }) => {
  requiresPreview('deform')
  await aim(page, CAMERA)
  await openMenu(page, /^tools$/i)
  await menuBar(page).getByText('Deform', { exact: true }).hover()
  for (const name of ['Twist', 'Bend', 'Taper', 'Bend to Path']) {
    await expect(menuBar(page).getByText(name, { exact: true })).toBeVisible()
  }
  await shot(page, 'menu-tools-deform')
})

test('Object > Smooth: the selection\'s level is checked, picking a level sets it, Bake and Soften are reachable', async ({ page }) => {
  requiresPreview('smooth', 'soften')
  await aim(page, CAMERA)
  const id = await page.evaluate(() => {
    const h = window.__hew_test!
    const id = h.drawBox([0, 0, 0], [1, 1, 0], 1)
    h.selectObjects([id])
    return id
  })
  await openMenu(page, /^object$/i)
  await menuBar(page).getByText('Smooth', { exact: true }).hover()
  await expect(menuBar(page).getByText(/^✓?Off$/)).toHaveText('✓Off')
  await shot(page, 'menu-object-smooth')
  await menuBar(page).getByText(/^✓?1$/).click()
  await expect.poll(() => page.evaluate((id) => window.__hew_test!.getSmoothLevel(id), id)).toBe(1)

  await openMenu(page, /^object$/i)
  await menuBar(page).getByText('Smooth', { exact: true }).hover()
  await expect(menuBar(page).getByText(/^✓?1$/)).toHaveText('✓1')

  // Bake Smooth keeps the rounded surface as real faces and drops the level
  // to Off; one undo step brings the level back.
  await menuBar(page).getByText('Bake Smooth', { exact: true }).click()
  await expect.poll(() => page.evaluate((id) => window.__hew_test!.getSmoothLevel(id), id)).toBe(0)
  await page.evaluate(() => window.__hew_test!.undo())
  await expect.poll(() => page.evaluate((id) => window.__hew_test!.getSmoothLevel(id), id)).toBe(1)

  // Soften Edges… brings up the angle field in Object Info and focuses it.
  await openMenu(page, /^object$/i)
  await menuBar(page).getByText('Soften Edges…', { exact: true }).click()
  await expect(page.locator('[data-soften-angle]')).toBeFocused()
})

test('Object > Smooth does nothing with nothing selected', async ({ page }) => {
  requiresPreview('smooth')
  await aim(page, CAMERA)
  const id = await page.evaluate(() => {
    const h = window.__hew_test!
    const id = h.drawBox([0, 0, 0], [1, 1, 0], 1)
    h.setSmoothLevel(id, 2)
    return id
  })
  await openMenu(page, /^object$/i)
  await menuBar(page).getByText('Smooth', { exact: true }).hover()
  await menuBar(page).getByText(/^✓?3$/).click()
  await page.waitForTimeout(200)
  expect(await page.evaluate((id) => window.__hew_test!.getSmoothLevel(id), id)).toBe(2)
})

for (const [width, height] of [[1024, 700], [1280, 720], [1600, 1000]]) {
  test(`dock is centered in the viewport and on one row at ${width}x${height}`, async ({ page }) => {
    await page.setViewportSize({ width, height })
    await aim(page, CAMERA)
    await page.evaluate(() => {
      const h = window.__hew_test!
      h.selectObjects([h.drawBox([0, 0, 0], [1, 1, 0], 1)])
    })
    await expect(dock(page)).toHaveAttribute('data-dock-context', 'object')
    const g = await dock(page).evaluate((el) => {
      const parent = (el as HTMLElement).offsetParent!.getBoundingClientRect()
      const r = el.getBoundingClientRect()
      return {
        centerDelta: r.left + r.width / 2 - (parent.left + parent.width / 2),
        rows: new Set(Array.from(el.children).map((c) => Math.round(c.getBoundingClientRect().top))).size,
      }
    })
    expect(Math.abs(g.centerDelta)).toBeLessThanOrEqual(1)
    expect(g.rows).toBe(1)
  })
}

test('dock, object row: Push/Pull, Fillet, Move, Paint, Shell, Make Component, Save to Library, Lock, Erase', async ({ page }) => {
  await aim(page, CAMERA)
  await page.evaluate(() => {
    const h = window.__hew_test!
    h.selectObjects([h.drawBox([0, 0, 0], [1, 1, 0], 1)])
  })
  await expect(dock(page)).toHaveAttribute('data-dock-context', 'object')
  const labels = await dockLabels(page)
  // The full nine-chip row. Save to Library needs a library backend, which
  // Chromium provides through OPFS.
  expect(labels).toEqual(offeredTools([
    'Push/Pull', 'Fillet', 'Move', 'Paint', 'Shell', 'Make Component', 'Save to Library', 'Lock', 'Erase',
  ]))
  await shot(page, 'dock-object')
})

test('dock, multi row: Move, Union, Subtract, Intersect, Group, Lock, Erase for two solids', async ({ page }) => {
  await aim(page, CAMERA)
  await page.evaluate(() => {
    const h = window.__hew_test!
    const a = h.drawBox([0, 0, 0], [1, 1, 0], 1)
    const b = h.drawBox([0.5, 0.5, 0], [1.5, 1.5, 0], 1)
    h.selectObjects([a, b])
  })
  await expect(dock(page)).toHaveAttribute('data-dock-context', 'multi')
  expect(await dockLabels(page)).toEqual(['Move', 'Union', 'Subtract', 'Intersect', 'Group', 'Lock', 'Erase'])
  await shot(page, 'dock-multi')
  await dock(page).getByRole('button', { name: 'Union' }).click()
  await expect.poll(() => page.evaluate(() => window.__hew_test!.getObjectCount())).toBe(1)
})

test('dock, sketch row: a selected drawn rectangle offers Push/Pull, Follow Me, Move, Rotate, Erase (and Revolve, Loft where offered)', async ({ page }) => {
  const ctx = await aim(page, CAMERA)
  await buildSketchScene(page)
  await page.getByRole('radio', { name: 'Select' }).click()
  await clickAt(page, ctx, 1.5, 0.5, 0)
  await expect(dock(page)).toHaveAttribute('data-dock-context', 'sketch')
  expect(await dockLabels(page)).toEqual(offeredTools(['Push/Pull', 'Revolve', 'Loft', 'Follow Me', 'Move', 'Rotate', 'Erase']))
  await shot(page, 'dock-sketch')
})

test('dock, sketch row: Revolve goes straight to the axis', async ({ page }) => {
  requiresPreview('revolve')
  const ctx = await aim(page, CAMERA)
  await buildSketchScene(page)
  await page.getByRole('radio', { name: 'Select' }).click()
  await clickAt(page, ctx, 1.5, 0.5, 0)
  await expect(dock(page)).toHaveAttribute('data-dock-context', 'sketch')

  await dock(page).getByRole('button', { name: 'Revolve' }).click()
  // The selected rectangle is already the profile: no profile click.
  await expect(page.getByText('Click an edge or two points for the axis.')).toBeVisible()
  await shot(page, 'revolve-preselected-axis-stage')

  // Finish it for real: pick the drawn line as the axis, full turn.
  await moveTo(page, ctx, -0.5, 1.2, 0)
  await clickAt(page, ctx, -0.5, 1.2, 0)
  await expect(page.getByText('Move to set the angle, click to finish, Enter for a full turn.')).toBeVisible()
  await page.keyboard.press('Enter')
  await page.waitForFunction(() => window.__hew_test!.getObjectCount() === 1)
})

test('dock, hover preview: with nothing selected, hovering a drawn shape enables Push/Pull, Revolve, and Loft only', async ({ page }) => {
  const ctx = await aim(page, CAMERA)
  await buildSketchScene(page)
  await page.getByRole('radio', { name: 'Select' }).click()
  await moveTo(page, ctx, 1.5, 0.5, 0)
  await page.waitForTimeout(200)
  await moveTo(page, ctx, 1.4, 0.5, 0)
  await expect(dock(page)).toHaveAttribute('data-dock-context', 'sketch')
  const enabled = await dock(page).getByRole('button').evaluateAll((els) =>
    els.filter((e) => !(e as HTMLButtonElement).disabled).map((e) => e.getAttribute('title')),
  )
  expect(enabled).toEqual(offeredTools(['Push/Pull', 'Revolve', 'Loft']))
  await shot(page, 'dock-hover-preview')
})

async function buildSketchScene(page: Page): Promise<void> {
  await page.evaluate(() => {
    const h = window.__hew_test!
    h.drawRectangle([1, 0, 0], [2, 1, 0])
    h.drawLineChain([[-0.5, -0.5, 0], [-0.5, 1.5, 0]])
  })
}

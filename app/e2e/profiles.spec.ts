import { test, expect, type Page, type TestInfo } from '@playwright/test'
import { settleFrame } from './helpers/render'
import { requiresPreview } from './helpers/preview'

// Drives a Preview feature: skipped in the production-shape run (e2e/helpers/preview.ts).
requiresPreview('fillet')

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

/**
 * Profiles in the Library and the Fillet tool's options card, driven with real
 * pointer and keyboard events: pick a built-in Ogee in the card and cut a box
 * edge with it; save a drawn arc-and-line chain as a profile; find it in the
 * card and in Library > Profiles; use it from the browser.
 *
 * Set HEW_E2E_SHOTS to a directory to keep the screenshots there.
 */

const CAMERA = {
  position: [7, -7, 6] as [number, number, number],
  target: [1, 1, 0.5] as [number, number, number],
  up: [0, 0, 1] as [number, number, number],
  fovDeg: 45,
}

async function pagePoint(page: Page, w: [number, number, number]): Promise<{ x: number; y: number }> {
  const box = await page.locator('canvas').first().boundingBox()
  if (box === null) throw new Error('no canvas')
  const p = await page.evaluate((w) => window.__hew_test!.worldToScreen(w), w)
  return { x: box.x + p.x, y: box.y + p.y }
}

async function clickWorld(page: Page, w: [number, number, number]): Promise<void> {
  const p = await pagePoint(page, w)
  await page.mouse.move(p.x, p.y, { steps: 3 })
  await page.mouse.down()
  await page.mouse.up()
}

async function shot(page: Page, info: TestInfo, name: string): Promise<void> {
  await settleFrame(page)
  const dir = process.env.HEW_E2E_SHOTS
  await page.screenshot({ path: dir ? `${dir}/${name}.png` : info.outputPath(`${name}.png`) })
}

/** The card's More shapes trigger; it names the active custom shape beside the label. */
const moreShapes = (page: Page) => page.getByTestId('fillet-shape-options').getByRole('button', { name: /^More shapes/ })

test('profiles: the Ogee in the card, a saved profile in the card and the Library', async ({ page }, info) => {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
  const id = await page.evaluate((cam) => {
    const h = window.__hew_test!
    h.setLengthUnit('mm')
    const box = h.drawBox([0, 0, 0], [2, 2, 0], 1)
    h.setCamera(cam)
    return box
  }, CAMERA)
  await page.waitForTimeout(150)

  // 1. Fillet, then Ogee from the card.
  await page.keyboard.press('Shift+F')
  await moreShapes(page).click()
  await page.getByRole('menuitemradio', { name: 'Ogee', exact: true }).click()
  await expect(moreShapes(page)).toContainText('Ogee')
  // The menu stays open after a choice; close it with its trigger before picking an edge.
  await expect(page.getByRole('menuitemradio', { name: 'Ogee', exact: true })).toHaveAttribute('aria-checked', 'true')
  await moreShapes(page).click()
  await expect(page.getByRole('menu')).toHaveCount(0)

  // 2. Click a box edge, type 10, Enter: an ogee cut (a profile blend, 10 mm wide).
  await clickWorld(page, [1, 0.03, 1])
  await page.keyboard.type('10')
  await page.keyboard.press('Enter')
  await page.waitForTimeout(200)
  const rows = await page.evaluate((id) => window.__hew_test!.blendFeatures(id), id)
  expect(rows).toHaveLength(1)
  expect(rows[0].kind).toBe('profile')
  expect(rows[0].size!).toBeCloseTo(0.01, 6)
  await shot(page, info, 'profiles-1-ogee')

  // 3. A drawn arc-and-line chain, selected, saved from the card.
  const sketch = await page.evaluate(() => {
    const h = window.__hew_test!
    const a = h.drawArc([0, 0.1, 0], [-0.06, 0.04, 0], 0.012)
    h.drawLineChain([[-0.06, 0.04, 0], [-0.1, 0, 0]])
    return a.sketch
  })
  await page.evaluate((sketch) => window.__hew_test!.selectNodes([{ kind: 'sketch', id: sketch }]), sketch)
  await moreShapes(page).click()
  await page.getByRole('menuitem', { name: 'Save profile…' }).click()
  const prompt = page.getByRole('dialog', { name: 'Save profile' })
  await expect(prompt).toBeVisible()
  await expect(prompt.getByText('Detected width')).toBeVisible()
  await prompt.getByRole('textbox', { name: 'Item name' }).fill('Quarter edge')
  await shot(page, info, 'profiles-3-save')
  await prompt.getByRole('button', { name: 'Save', exact: true }).click()
  await expect(page.getByText('Saved profile "Quarter edge"')).toBeVisible()

  // 4. It is in the card, and under Library > Profiles.
  await moreShapes(page).click()
  await expect(page.getByRole('menuitemradio', { name: /Quarter edge/ })).toBeVisible()
  await page.keyboard.press('Escape')
  await page.getByRole('button', { name: 'Library' }).click()
  const dialog = page.getByRole('dialog', { name: 'Library' })
  await dialog.getByRole('tab', { name: /Profiles/ }).click()
  await expect(dialog.getByRole('option', { name: /Quarter edge/ })).toBeVisible()
  await expect(dialog.getByRole('option', { name: /^Use Ogee/ })).toBeVisible()
  await shot(page, info, 'profiles-4-library')

  // 5. Use it from the browser: the dialog closes and the card names it.
  await dialog.getByRole('option', { name: /Quarter edge/ }).dblclick()
  await expect(dialog).toBeHidden()
  await expect(moreShapes(page)).toContainText('Quarter edge')
  await shot(page, info, 'profiles-5-used')
})

test('profiles: a closed shape says what to select first; a drawing that is not a profile is never saved', async ({ page }, info) => {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
  await page.evaluate((cam) => {
    const h = window.__hew_test!
    h.setLengthUnit('mm')
    h.setCamera(cam)
  }, CAMERA)
  await page.keyboard.press('Shift+F')

  // A closed rectangle, selected: Save profile… says what to pick instead.
  const rect = await page.evaluate(() => window.__hew_test!.drawRectangle([0, 0, 0], [0.2, 0.1, 0]))
  await page.evaluate((s) => window.__hew_test!.selectNodes([{ kind: 'sketch', id: s }]), rect.sketch)
  await moreShapes(page).click()
  await page.getByRole('menuitem', { name: 'Save profile…' }).click()
  await expect(page.getByText('That shape is closed. Select an open line or arc')).toBeVisible()
  await shot(page, info, 'profiles-closed-shape')

  // A chain that turns back on itself cannot be built into a profile.
  const chain = await page.evaluate(() =>
    window.__hew_test!.drawLineChain([[0.5, 0.5, 0], [0.44, 0.53, 0], [0.4, 0.6, 0], [0.47, 0.57, 0]]),
  )
  await page.evaluate((s) => window.__hew_test!.selectNodes([{ kind: 'sketch', id: s }]), chain.sketch)
  await moreShapes(page).click()
  await page.getByRole('menuitem', { name: 'Save profile…' }).click()
  await expect(page.getByText(/isn't a profile Hew can use\. Draw one open line or arc that runs from one side to the other without doubling back\./)).toBeVisible()
  await expect(page.getByRole('dialog', { name: 'Save profile' })).toHaveCount(0)

  // The same drawing as a whole document, File > Save to Library…: the
  // dialog may open (the file is one open chain), but Save writes nothing.
  await page.reload()
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
  const profileCount = async (): Promise<number> => {
    await page.getByRole('button', { name: 'Library' }).click()
    const lib = page.getByRole('dialog', { name: 'Library' })
    await lib.getByRole('tab', { name: /Profiles/ }).click()
    await page.waitForTimeout(400)
    const n = await lib.getByRole('option').count()
    await page.keyboard.press('Escape')
    await expect(lib).toBeHidden()
    return n
  }
  const before = await profileCount()
  await page.evaluate(() =>
    window.__hew_test!.drawLineChain([[0.5, 0.5, 0], [0.44, 0.53, 0], [0.4, 0.6, 0], [0.47, 0.57, 0]]),
  )
  await page.getByTestId('menu-bar').getByRole('button', { name: 'File' }).click()
  await page.getByText('Save to Library…', { exact: true }).click()
  const prompt = page.getByRole('dialog', { name: /Save/ })
  await expect(prompt).toBeVisible()
  await shot(page, info, 'profiles-bad-chain-dialog')
  await prompt.getByRole('button', { name: 'Save', exact: true }).click()
  await shot(page, info, 'profiles-bad-chain')
  await expect(page.getByText(/isn't a profile Hew can use/)).toBeVisible()
  // Nothing was written: the Library holds the same profiles as before.
  expect(await profileCount()).toBe(before)
})

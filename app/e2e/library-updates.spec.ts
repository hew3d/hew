import { test, expect, type Page } from '@playwright/test'

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

/**
 * Library updates (v1.5 OW-2), driven with REAL pointer and keyboard events
 * against the web Library on OPFS: save a part to the Library, change it
 * inside the model and push the change back with "Update in Library", then
 * open the part as it was before (so the Library is newer than the model),
 * see the notice bar, Review, update, and undo with Cmd/Ctrl+Z, which puts
 * everything back in ONE step. The harness only reads the model back and
 * stands in for "another document" (it reopens the earlier bytes).
 */

const CAMERA = {
  position: [4, -4, 3] as [number, number, number],
  target: [0.5, 0.5, 0.5] as [number, number, number],
  up: [0, 0, 1] as [number, number, number],
  fovDeg: 45,
}

async function setup(page: Page): Promise<void> {
  await page.addInitScript(() => {
    // The Components tray section open, so the part's marker is on screen.
    localStorage.setItem(
      'hew.settings.trayLayout',
      JSON.stringify({ modelInfo: false, objectInfo: false, materials: false, components: true, tags: false, scenes: false }),
    )
  })
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
  await page.evaluate((cam) => window.__hew_test!.setCamera(cam), CAMERA)
  await page.waitForTimeout(100)
}

async function pagePoint(page: Page, w: [number, number, number]): Promise<{ x: number; y: number }> {
  const box = await page.locator('canvas').first().boundingBox()
  if (box === null) throw new Error('no canvas')
  const p = await page.evaluate((w) => window.__hew_test!.worldToScreen(w), w)
  if (p.behind) throw new Error(`world point ${w.join(',')} is behind the camera`)
  return { x: box.x + p.x, y: box.y + p.y }
}

async function clickWorld(page: Page, w: [number, number, number]): Promise<void> {
  const p = await pagePoint(page, w)
  await page.mouse.move(p.x, p.y)
  await page.mouse.down()
  await page.mouse.up()
}

/** Push/Pull's two clicks: arm on the face, then commit where the drag ends. */
async function dragWorld(page: Page, from: [number, number, number], to: [number, number, number]): Promise<void> {
  const a = await pagePoint(page, from)
  const b = await pagePoint(page, to)
  await page.mouse.move(a.x, a.y)
  await page.mouse.down()
  await page.mouse.up()
  await page.mouse.move(b.x, b.y, { steps: 10 })
  await page.mouse.down()
  await page.mouse.up()
  await page.waitForTimeout(150)
}

test('a newer Library part: notice bar, Review, update, and one undo puts it back', async ({ page }) => {
  // A long real-click flow; the gate runs it beside other specs.
  test.slow()
  await setup(page)

  // A 1 x 1 x 1 box made a component, saved to the Library as "Leg".
  const made = await page.evaluate(() => {
    const h = window.__hew_test!
    const m = h.makeComponent([h.drawBox([0, 0, 0], [1, 1, 0], 1)])
    h.selectNodes([{ kind: 'instance', id: m.instance }])
    return m
  })
  await page.getByRole('button', { name: 'Save to Library' }).click()
  const prompt = page.getByRole('dialog', { name: 'Save to Library' })
  await prompt.getByRole('textbox').fill('Leg')
  await prompt.getByRole('button', { name: 'Save', exact: true }).click()
  await expect(page.getByText('Saved "Leg" to library')).toBeVisible()
  // The part, as it stands in step with the Library: the "other document".
  const inStep = await page.evaluate(() => window.__hew_test!.save())
  void made

  // Change the part inside the model: double-click opens it, Push/Pull its top.
  const top = await pagePoint(page, [0.5, 0.5, 1])
  await page.mouse.move(top.x, top.y)
  await page.mouse.click(top.x, top.y, { clickCount: 2 })
  await page.waitForTimeout(200)
  await page.keyboard.press('p')
  await dragWorld(page, [0.5, 0.5, 1], [0.5, 0.5, 2])
  await page.keyboard.press('Escape')
  await page.waitForTimeout(200)

  // The Components row now says "changed here".
  await expect(page.getByTestId('library-mark')).toHaveAttribute('data-state', 'edited_here', { timeout: 10_000 })

  // Back to the Select tool, then select the part and push the change to the Library with Update in Library.
  await page.getByRole('radio', { name: 'Select' }).click()
  await clickWorld(page, [0.5, 0, 0.5])
  await page.getByRole('button', { name: 'Save to Library' }).click()
  const popover = page.getByRole('dialog', { name: 'Save to Library' })
  await expect(popover.getByText('Other models that use it will be offered the update.')).toBeVisible()
  await popover.getByRole('button', { name: /^Update “Leg” in the Library$/ }).click()
  await expect(page.getByText('Updated "Leg" in the Library')).toBeVisible()

  // Open the part as it was: the Library now holds a newer version than this model.
  await page.evaluate((bytes) => window.__hew_test!.load(bytes), inStep)
  const bar = page.getByTestId('library-update-bar')
  await expect(bar).toBeVisible({ timeout: 10_000 })
  await expect(bar).toContainText('1 part has a newer version in the Library.')
  await expect(page.getByTestId('library-mark')).toHaveAttribute('data-state', 'library_newer')

  // Review lists it, checked, in plain words.
  await bar.getByRole('button', { name: 'Review…' }).click()
  const review = page.getByRole('dialog', { name: 'Library updates' })
  const row = review.getByTestId('library-review-row')
  await expect(row).toContainText('Leg')
  await expect(row).toContainText('Newer in the Library')
  await expect(row.getByRole('checkbox')).toBeChecked()

  const hashBefore = await page.evaluate(() => window.__hew_test!.getStateHash())
  const depthBefore = await page.evaluate(() => window.__hew_test!.getUndoDepth())
  await review.getByRole('button', { name: 'Update Selected' }).click()
  await expect(review.getByTestId('library-update-outcome')).toContainText('Updated 1 part from the Library.')
  await review.getByRole('button', { name: 'Done' }).click()
  await expect(review).not.toBeVisible()
  await expect(bar).not.toBeVisible({ timeout: 10_000 })

  // The model changed, as one undo step.
  const hashAfter = await page.evaluate(() => window.__hew_test!.getStateHash())
  expect(hashAfter).not.toBe(hashBefore)
  expect(await page.evaluate(() => window.__hew_test!.getUndoDepth())).toBe(depthBefore + 1)

  // Cmd/Ctrl+Z puts it all back at once, and the Library is newer again.
  await page.locator('canvas').first().click({ position: { x: 5, y: 5 } })
  await page.keyboard.press('ControlOrMeta+z')
  await expect.poll(() => page.evaluate(() => window.__hew_test!.getStateHash())).toBe(hashBefore)
  expect(await page.evaluate(() => window.__hew_test!.getUndoDepth())).toBe(depthBefore)
  await expect(bar).toBeVisible({ timeout: 10_000 })
})

/** Save a 1 m box part to the Library as "Leg"; returns the model bytes as they stand in step. */
async function saveLeg(page: Page): Promise<Uint8Array> {
  await page.evaluate(() => {
    const h = window.__hew_test!
    const m = h.makeComponent([h.drawBox([0, 0, 0], [1, 1, 0], 1)])
    h.selectNodes([{ kind: 'instance', id: m.instance }])
  })
  await page.getByRole('button', { name: 'Save to Library' }).click()
  const prompt = page.getByRole('dialog', { name: 'Save to Library' })
  await prompt.getByRole('textbox').fill('Leg')
  await prompt.getByRole('button', { name: 'Save', exact: true }).click()
  await expect(page.getByText('Saved "Leg" to library')).toBeVisible()
  return page.evaluate(() => window.__hew_test!.save())
}

/** Open the part, pull its top up 1 m, close it again. */
async function pullLegTop(page: Page): Promise<void> {
  const top = await pagePoint(page, [0.5, 0.5, 1])
  await page.mouse.move(top.x, top.y)
  await page.mouse.click(top.x, top.y, { clickCount: 2 })
  await page.waitForTimeout(200)
  await page.keyboard.press('p')
  await dragWorld(page, [0.5, 0.5, 1], [0.5, 0.5, 2])
  await page.keyboard.press('Escape')
  await page.waitForTimeout(200)
  await page.getByRole('radio', { name: 'Select' }).click()
}

test('a part open for editing keeps the notice bar and markers, and Update All names it in Review', async ({ page }) => {
  test.slow()
  await setup(page)
  const inStep = await saveLeg(page)
  await pullLegTop(page)
  await clickWorld(page, [0.5, 0, 0.5])
  await page.getByRole('button', { name: 'Save to Library' }).click()
  const popover = page.getByRole('dialog', { name: 'Save to Library' })
  await popover.getByRole('button', { name: /^Update “Leg” in the Library$/ }).click()
  await expect(page.getByText('Updated "Leg" in the Library')).toBeVisible()
  await page.evaluate((bytes) => window.__hew_test!.load(bytes), inStep)
  const bar = page.getByTestId('library-update-bar')
  await expect(bar).toBeVisible({ timeout: 10_000 })

  // Open the part (a real double-click): the bar and the marker stay, well past the check's quiet time.
  const p = await pagePoint(page, [0.5, 0.5, 1])
  await page.mouse.move(p.x, p.y)
  await page.mouse.click(p.x, p.y, { clickCount: 2 })
  await expect(page.getByText('editing', { exact: true }).first()).toBeVisible()
  await page.waitForTimeout(2500)
  await expect(bar).toBeVisible()
  await expect(page.getByTestId('library-mark').first()).toHaveAttribute('data-state', 'library_newer')

  // Update All reaches the kernel, which refuses the open part; Review says so.
  const hashBefore = await page.evaluate(() => window.__hew_test!.getStateHash())
  await bar.getByTestId('library-update-all').click()
  const skipped = page.getByTestId('library-update-skipped')
  await expect(skipped).toContainText('“Component 1” is open for editing and was not updated. Close the open group or component first.')
  expect(await page.evaluate(() => window.__hew_test!.getStateHash())).toBe(hashBefore)
})

test('with the check turned off there is no bar, but the Save popover still offers Update in Library', async ({ page }) => {
  test.slow()
  await page.addInitScript(() => localStorage.setItem('hew.settings.checkLibraryUpdates', 'false'))
  await setup(page)
  const inStep = await saveLeg(page)
  await pullLegTop(page)
  await clickWorld(page, [0.5, 0, 0.5])
  await page.getByRole('button', { name: 'Save to Library' }).click()
  const popover = page.getByRole('dialog', { name: 'Save to Library' })
  await expect(popover.getByRole('button', { name: /^Update “Leg” in the Library$/ })).toBeVisible({ timeout: 10_000 })
  await popover.getByRole('button', { name: /^Update “Leg” in the Library$/ }).click()
  await expect(page.getByText('Updated "Leg" in the Library')).toBeVisible()

  // The older model opens with the Library newer: nothing nags.
  await page.evaluate((bytes) => window.__hew_test!.load(bytes), inStep)
  await page.waitForTimeout(3000)
  await expect(page.getByTestId('library-update-bar')).toHaveCount(0)
})

test('a part shows changed here while it is still open, and the Library shows it too', async ({ page }) => {
  test.slow()
  await setup(page)
  await saveLeg(page)
  const savedHash = await page.evaluate(() => window.__hew_test!.getStateHash())

  const top = await pagePoint(page, [0.5, 0.5, 1])
  await page.mouse.move(top.x, top.y)
  await page.mouse.click(top.x, top.y, { clickCount: 2 })
  await page.waitForTimeout(200)
  await page.keyboard.press('p')
  await dragWorld(page, [0.5, 0.5, 1], [0.5, 0.5, 2])
  // Still inside the part: the mark follows the edit.
  await expect(page.getByTestId('library-mark')).toHaveAttribute('data-state', 'edited_here', { timeout: 10_000 })
  await expect(page.getByText(/editing/i).first()).toBeVisible()

  await page.keyboard.press('Shift+L')
  const state = page.getByTestId('library-item-state')
  await expect(state).toHaveAttribute('data-state', 'edited_here', { timeout: 10_000 })
  await expect(state).toContainText('You changed this part here.')
  await page.keyboard.press('Escape')
  await expect(state).not.toBeVisible()

  // Undo back to the saved shape: the mark goes away.
  const mod = (await page.evaluate(() => /Mac/.test(navigator.platform))) ? 'Meta' : 'Control'
  for (let i = 0; i < 6; i++) {
    if ((await page.evaluate(() => window.__hew_test!.getStateHash())) === savedHash) break
    await page.keyboard.press(`${mod}+z`)
    await page.waitForTimeout(100)
  }
  await expect(page.getByTestId('library-mark')).toHaveCount(0, { timeout: 10_000 })
})

test('a linked copy saved from a saved document is tracked', async ({ page }) => {
  test.slow()
  await setup(page)
  const board = await page.evaluate(() => window.__hew_test!.drawBox([0, 0, 0], [2, 2, 0], 1))
  await page.evaluate((id) => window.__hew_test!.selectObjects([id]), board)
  await page.waitForFunction(() => window.__hew_test!.getSelection().length === 1)
  await page.keyboard.press('m')
  const mac = await page.evaluate(() => /Mac|iPod|iPhone|iPad/.test(navigator.platform))
  await page.keyboard.press(mac ? 'Alt' : 'Control')
  await clickWorld(page, [1, 1, 0])
  await page.keyboard.press('ArrowRight')
  const far = await pagePoint(page, [5, 1, 0])
  await page.mouse.move(far.x, far.y)
  await page.keyboard.type('4')
  await page.keyboard.press('Enter')
  await page.waitForTimeout(200)
  // A saved document: the title has no unsaved bullet.
  await page.evaluate(() => window.__hew_test!.saveWithCameraState())
  await page.waitForTimeout(200)
  const sel = await page.evaluate(() => window.__hew_test!.getSelection())
  expect(sel).toHaveLength(1)

  await page.getByRole('button', { name: 'Save to Library' }).click()
  const prompt = page.getByRole('dialog', { name: 'Save to Library' })
  await prompt.getByRole('textbox').fill('Board')
  await prompt.getByRole('button', { name: 'Save', exact: true }).click()
  await expect(page.getByText('Saved "Board" to library')).toBeVisible()

  await page.keyboard.press('Shift+L')
  await expect(page.getByText('2 instances in this model').first()).toBeVisible({ timeout: 10_000 })
})

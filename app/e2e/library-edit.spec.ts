import { test, expect, type Page } from '@playwright/test'

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

/**
 * Library ▸ Edit (web build, OPFS Library): the item's own file opens for
 * editing, Save writes it back through the Library, and another document
 * that uses the part hears about it without a reload. Real pointer and
 * keyboard events drive the Library, the tools and Save.
 */

const CAMERA = {
  position: [4, -4, 3] as [number, number, number],
  target: [0.5, 0.5, 0.5] as [number, number, number],
  up: [0, 0, 1] as [number, number, number],
  fovDeg: 45,
}

async function ready(page: Page): Promise<void> {
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
  await page.evaluate((cam) => window.__hew_test!.setCamera(cam), CAMERA)
  await page.waitForTimeout(100)
}

async function setup(page: Page): Promise<void> {
  await page.goto('/')
  await ready(page)
}

async function pagePoint(page: Page, w: [number, number, number]): Promise<{ x: number; y: number }> {
  const box = await page.locator('canvas').first().boundingBox()
  if (box === null) throw new Error('no canvas')
  const p = await page.evaluate((w) => window.__hew_test!.worldToScreen(w), w)
  if (p.behind) throw new Error(`world point ${w.join(',')} is behind the camera`)
  return { x: box.x + p.x, y: box.y + p.y }
}

async function saveLeg(page: Page): Promise<void> {
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
}

/**
 * Open the part and pull its top face up 1 m. The part is selected from the
 * Outliner and opened with the dock's Edit button (real clicks); the pull is
 * the harness's kernel push/pull, because a document loaded from a file frames
 * itself with a view offset the harness' projection does not know about.
 */
async function pullTop(page: Page): Promise<void> {
  await page.getByText('Leg', { exact: true }).first().click()
  await page.getByRole('button', { name: 'Edit', exact: true }).last().click()
  await page.waitForTimeout(300)
  const depth = await page.evaluate(() => window.__hew_test!.getUndoDepth())
  await page.evaluate(() => {
    const h = window.__hew_test!
    // Rays straight down over a grid: the first thing any of them hits is the part's top.
    for (let x = -3; x <= 3; x += 0.25) {
      for (let y = -3; y <= 3; y += 0.25) {
        const hit = h.pickFace([x, y, 9], [0, 0, -1])
        if (hit !== null) {
          h.pushPull(hit.object, hit.face, 1)
          return
        }
      }
    }
    throw new Error('no face under the part')
  })
  await page.waitForTimeout(150)
  // The pull really changed the part.
  expect(await page.evaluate(() => window.__hew_test!.getUndoDepth())).toBeGreaterThan(depth)
}

async function save(page: Page): Promise<void> {
  const mod = (await page.evaluate(() => /Mac/.test(navigator.platform))) ? 'Meta' : 'Control'
  await page.keyboard.press(`${mod}+s`)
}

async function selectLegInLibrary(page: Page): Promise<string> {
  await page.keyboard.press('Shift+L')
  const tile = page.locator('[data-filename]').filter({ hasText: 'Leg' }).first()
  await expect(tile).toBeVisible({ timeout: 10_000 })
  await tile.click()
  return (await tile.getAttribute('data-filename'))!
}

test('Edit opens the Library item in place and Save updates it', async ({ page }) => {
  test.slow()
  await setup(page)
  await saveLeg(page)
  await page.reload()
  await ready(page)

  await page.keyboard.press('Shift+L')
  const tile = page.locator('[data-filename]').filter({ hasText: 'Leg' }).first()
  await expect(tile).toBeVisible({ timeout: 10_000 })
  const thumbBefore = await tile.locator('img').first().getAttribute('src')
  await tile.click()
  await page.getByTestId('library-edit').click()
  await expect(page.getByText('Editing "Leg" from the Library. Saving updates the Library.')).toBeVisible()
  await expect(page).toHaveTitle('Leg (Library item) — Hew')

  await pullTop(page)
  await save(page)
  await expect(page.getByText('Updated "Leg" in the Library')).toBeVisible({ timeout: 10_000 })
  await expect(page).toHaveTitle('Leg (Library item) — Hew')

  await page.keyboard.press('Shift+L')
  const after = page.locator('[data-filename]').filter({ hasText: 'Leg' }).first()
  await expect(after).toBeVisible()
  await expect.poll(async () => after.locator('img').first().getAttribute('src'), { timeout: 10_000 }).not.toBe(thumbBefore)
})

test('another document hears about the saved edit without a reload', async ({ page, context }) => {
  test.slow()
  await setup(page)
  await saveLeg(page)
  await selectLegInLibrary(page)
  // This tab holds work, so the real Edit button opens the item in a new tab.
  const opened = context.waitForEvent('page')
  await page.getByTestId('library-edit').click()
  const editor = await opened
  await editor.bringToFront()
  await expect(editor).toHaveTitle('Leg (Library item) — Hew', { timeout: 15_000 })
  await pullTop(editor)
  await save(editor)
  await expect(editor.getByText('Updated "Leg" in the Library')).toBeVisible({ timeout: 10_000 })
  // Back to the model's tab: the bar is raised without a reload or a click.
  await page.bringToFront()

  await expect(page.getByTestId('library-update-bar')).toBeVisible({ timeout: 10_000 })
  await expect(page.getByTestId('library-update-bar')).toContainText('1 part has a newer version in the Library.')
})

test('a rename and a keyword made in the Library while editing survive the save', async ({ page, context }) => {
  test.slow()
  await setup(page)
  await saveLeg(page)
  await selectLegInLibrary(page)
  const opened = context.waitForEvent('page')
  await page.getByTestId('library-edit').click()
  const editor = await opened
  await editor.bringToFront()
  await expect(editor).toHaveTitle('Leg (Library item) — Hew', { timeout: 15_000 })

  // Meanwhile, in the model's tab, the Library renames the item and adds a keyword.
  await page.bringToFront()
  const dialog = page.getByRole('dialog', { name: 'Library' })
  const name = dialog.getByRole('textbox', { name: 'Item name' })
  await name.fill('Oak leg')
  await name.press('Enter')
  await dialog.getByRole('button', { name: '+ keyword' }).click()
  const keyword = dialog.getByRole('textbox', { name: 'New keyword' })
  await keyword.fill('oak')
  await keyword.press('Enter')
  await expect(dialog.locator('.hwlib__keyword-chip').filter({ hasText: 'oak' })).toBeVisible()

  // The editor window, still holding the old name, changes the part and saves.
  await editor.bringToFront()
  await pullTop(editor)
  await save(editor)
  // The toast and the title name the item as the Library has it now.
  await expect(editor.getByText('Updated "Oak leg" in the Library')).toBeVisible({ timeout: 10_000 })
  await expect(editor).toHaveTitle('Oak leg (Library item) — Hew')

  await page.bringToFront()
  await page.keyboard.press('Escape')
  await page.keyboard.press('Shift+L')
  const tile = page.locator('[data-filename]').filter({ hasText: 'Oak leg' }).first()
  await expect(tile).toBeVisible({ timeout: 10_000 })
  await tile.click()
  const after = page.getByRole('dialog', { name: 'Library' })
  await expect(after.getByRole('textbox', { name: 'Item name' })).toHaveValue('Oak leg')
  await expect(after.locator('.hwlib__keyword-chip').filter({ hasText: 'oak' })).toBeVisible()
  await expect(page.locator('[data-filename]').filter({ hasText: /^Leg$/ })).toHaveCount(0)
})

test('Edit from a tab that holds work opens a new tab and leaves the model alone', async ({ page, context }) => {
  test.slow()
  await setup(page)
  await saveLeg(page)
  await selectLegInLibrary(page)
  const hash = await page.evaluate(() => window.__hew_test!.getModelStateHash())
  const opened = context.waitForEvent('page')
  await page.getByTestId('library-edit').click()
  const tab = await opened
  await expect(tab).toHaveTitle('Leg (Library item) — Hew', { timeout: 15_000 })
  expect(await page.evaluate(() => window.__hew_test!.getModelStateHash())).toBe(hash)
  expect(new URL(tab.url()).searchParams.has('libraryEdit')).toBe(false)
})

test('a second tab on an item that is already open for editing says so and loads nothing', async ({ page, context }) => {
  test.slow()
  await setup(page)
  await saveLeg(page)
  const fileName = await selectLegInLibrary(page)
  await page.keyboard.press('Escape')

  // Both tabs boot through the URL a tab opened by Edit carries: the second
  // must reach the same lock without this tab's Library dialog in between.
  const first = await context.newPage()
  await first.bringToFront()
  await first.goto(`/?libraryEdit=${encodeURIComponent(fileName)}`)
  await expect(first).toHaveTitle('Leg (Library item) — Hew', { timeout: 15_000 })

  const second = await context.newPage()
  await second.bringToFront()
  await second.goto(`/?libraryEdit=${encodeURIComponent(fileName)}`)
  await expect(second.getByText('"Leg" is already open for editing in another tab.')).toBeVisible({ timeout: 15_000 })
  await expect(second).toHaveTitle(/^Untitled/)
})

test('saving an edit whose item was deleted asks first and writes nothing on cancel', async ({ page, context }) => {
  test.slow()
  await setup(page)
  await saveLeg(page)
  await selectLegInLibrary(page)
  const opened = context.waitForEvent('page')
  await page.getByTestId('library-edit').click()
  const editor = await opened
  await expect(editor).toHaveTitle('Leg (Library item) — Hew', { timeout: 15_000 })

  // Delete the item from the model's tab, with real clicks.
  await page.bringToFront()
  const dialog = page.getByRole('dialog', { name: 'Library' })
  await dialog.getByRole('button', { name: 'Delete from library…' }).click()
  await dialog.getByRole('button', { name: /^Delete$/ }).click()
  await expect(dialog.getByText('Leg', { exact: true })).toHaveCount(0)

  await editor.bringToFront()
  await save(editor)
  const ask = editor.getByRole('dialog', { name: 'This Library item is gone' })
  await expect(ask).toBeVisible({ timeout: 10_000 })
  await ask.getByRole('button', { name: 'Cancel' }).click()
  await expect(editor.getByText('Updated "Leg" in the Library')).toHaveCount(0)

  // Still gone from the Library.
  await page.bringToFront()
  await page.keyboard.press('Escape')
  await page.keyboard.press('Shift+L')
  await expect(page.locator('[data-filename]').filter({ hasText: 'Leg' })).toHaveCount(0)
})

test('Insert is refused in a window that is editing a Library item, and the Components row keeps its count', async ({ page }) => {
  test.slow()
  await setup(page)
  await saveLeg(page)
  await page.reload()
  await ready(page)
  await selectLegInLibrary(page)
  await page.getByTestId('library-edit').click()
  await expect(page.getByText('Editing "Leg" from the Library. Saving updates the Library.')).toBeVisible()
  await pullTop(page)

  // Inside the part the Components row still counts the part once.
  await page.getByText('Components', { exact: true }).click()
  const row = page.getByTestId('components-row-usage').first()
  await expect(row).toHaveText('1')
  await expect(page.getByTestId('components-row-thumb').first()).toBeVisible({ timeout: 10_000 })

  const before = await page.evaluate(() => window.__hew_test!.getUndoDepth())
  await selectLegInLibrary(page)
  await page.getByRole('dialog', { name: 'Library' }).getByRole('button', { name: 'Insert', exact: true }).first().click()
  await expect(
    page.getByText('This window is editing a Library item. Switch to a model window to insert it.'),
  ).toBeVisible()
  // Nothing was armed or placed: the document is as it was.
  expect(await page.evaluate(() => window.__hew_test!.getUndoDepth())).toBe(before)
})

test('the Library count is model-wide while a part is entered in a model window', async ({ page }) => {
  test.slow()
  await setup(page)
  await saveLeg(page)
  await page.evaluate(() => {
    const h = window.__hew_test!
    const def = h.getComponentIds()[0]
    if (def === undefined) throw new Error('no definition for the saved part')
    h.placeInstance(def, 3, 0, 0)
  })
  const tile = await selectLegInLibrary(page)
  expect(tile).not.toBe('')
  await expect(page.getByText('2 instances in this model').first()).toBeVisible({ timeout: 10_000 })
  await page.keyboard.press('Escape')

  // Enter one placement: the other stays, and the count reads the same.
  await page.getByText('Component 1', { exact: true }).last().click()
  await page.getByRole('button', { name: 'Edit', exact: true }).last().click()
  await page.waitForTimeout(300)
  await selectLegInLibrary(page)
  await expect(page.getByText('2 instances in this model').first()).toBeVisible({ timeout: 10_000 })
})

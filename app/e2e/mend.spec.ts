import { test, expect, type Page } from '@playwright/test'

/**
 * Model Health's Mend, end to end with the real UI and the real wasm
 * kernel: a broken mesh comes in through File ▸ Import…, Object ▸ Model
 * Health… lists it as needing mending, the row's preview says what Mend
 * will do, the chooser's steps change that, and the Mend button makes the
 * object a solid in one step that Edit ▸ Undo and Redo take back and redo.
 */

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

/**
 * A binary STL (millimeters) of a 10 × 10 × 50 box standing on its base,
 * its top left out and one top corner raised 5 mm: an opening that does
 * not lie flat, small against the box, so one flat face can't close it
 * and Mend fills it.
 */
function warpedBottleStl(): number[] {
  const v: [number, number, number][] = [
    [0, 0, 0],
    [10, 0, 0],
    [10, 10, 0],
    [0, 10, 0],
    [0, 0, 50],
    [10, 0, 50],
    [10, 10, 55],
    [0, 10, 50],
  ]
  const quads = [
    [0, 3, 2, 1],
    [0, 1, 5, 4],
    [1, 2, 6, 5],
    [2, 3, 7, 6],
    [3, 0, 4, 7],
  ]
  const tris: [number, number, number][] = []
  for (const q of quads) {
    tris.push([q[0], q[1], q[2]])
    tris.push([q[0], q[2], q[3]])
  }
  const buf = new ArrayBuffer(84 + tris.length * 50)
  const view = new DataView(buf)
  view.setUint32(80, tris.length, true)
  let at = 84
  for (const t of tris) {
    at += 12 // the normal, left zero
    for (const i of t) {
      for (const c of v[i]) {
        view.setFloat32(at, c, true)
        at += 4
      }
    }
    at += 2 // attribute byte count
  }
  return Array.from(new Uint8Array(buf))
}

/** A stub File System Access picker (the same arming as model-health.spec.ts). */
async function armFilePicker(page: Page, name: string, bytes: number[]): Promise<void> {
  await page.addInitScript(
    ([n, b]) => {
      const buf = new Uint8Array(b as number[])
      // @ts-expect-error test override of the File System Access API
      window.showOpenFilePicker = async () => [
        { getFile: async () => ({ name: n as string, arrayBuffer: async () => buf.buffer }) },
      ]
    },
    [name, bytes] as [string, number[]],
  )
  await page.reload()
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
}

const healthWindow = (page: Page) => page.getByRole('dialog', { name: 'Model Health' })

async function menu(page: Page, top: string, item: string | RegExp): Promise<void> {
  await page.getByTestId('menu-bar').getByRole('button', { name: top, exact: true }).click()
  await page.getByTestId('menu-bar').getByText(item, { exact: typeof item === 'string' }).first().click()
}

/** Import the warped bottle, open Model Health from the Object menu, select its row. */
async function reviewBottle(page: Page) {
  await armFilePicker(page, 'warped_bottle.stl', warpedBottleStl())
  await menu(page, 'File', 'Import…')
  await page.getByRole('dialog', { name: /stl import units/i }).getByRole('button', { name: /^import$/i }).click()
  await page.getByRole('dialog', { name: /import report/i }).getByRole('button', { name: 'OK' }).click()
  await menu(page, 'Object', 'Model Health…')
  await expect(healthWindow(page)).toBeVisible()
  await expect(healthWindow(page)).toContainText('Needs mending')
  const row = page.locator('[data-health-row]')
  await expect(row).toHaveCount(1)
  await row.getByText('warped_bottle', { exact: true }).click()
  await expect(row).toHaveAttribute('aria-pressed', 'true')
  return row
}

const objectIds = (page: Page) => page.evaluate(() => window.__hew_test!.getObjectIds())
const solid = (page: Page, id: string) => page.evaluate((id) => window.__hew_test!.isObjectSolid(id), id)

test.beforeEach(async ({ page }) => {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
})

test('Mend makes an imported mesh solid in one step; Edit ▸ Undo and Redo take it back and redo it', async ({ page }) => {
  const row = await reviewBottle(page)
  const [id] = await objectIds(page)
  expect(await solid(page, id)).toBe(false)
  await expect(page.locator('[data-health-badge="open"]')).toHaveText('1 object · 1 not solid')

  // The preview says what the click will do, in plain words.
  await expect(row.locator('[data-health-mend-preview]')).toHaveText('Mend will fill 1 small gap (1 curved).')
  const mend = row.getByRole('button', { name: 'Mend', exact: true })
  await expect(mend).toBeEnabled()
  // Its opening is warped, so no one face can close it: the row offers Mend
  // and Keep as is, and no Close (nor a Close all over the list).
  await expect(row.getByRole('button', { name: 'Keep as is', exact: true })).toBeVisible()
  await expect(row.getByRole('button', { name: 'Close', exact: true })).toHaveCount(0)
  await expect(healthWindow(page).getByRole('button', { name: /^Close all/ })).toHaveCount(0)
  await mend.hover()
  await mend.click()

  await expect(page.locator('[data-health-row]')).toHaveCount(0)
  await expect(page.getByText('1 object ✓ solid')).toBeVisible()
  expect(await objectIds(page)).toEqual([id])
  expect(await solid(page, id)).toBe(true)
  expect(await page.evaluate(() => window.__hew_test!.getLastError())).toBeNull()

  // One Undo restores the open object and its row.
  await menu(page, 'Edit', /^Undo\b/)
  await expect(page.locator('[data-health-row]')).toHaveCount(1)
  expect(await solid(page, id)).toBe(false)
  await expect(page.locator('[data-health-badge="open"]')).toHaveText('1 object · 1 not solid')

  // One Redo mends it again.
  await menu(page, 'Edit', /^Redo\b/)
  await expect(page.locator('[data-health-row]')).toHaveCount(0)
  expect(await solid(page, id)).toBe(true)
})

test('Choose what to mend: turning off the step that has work leaves nothing to do', async ({ page }) => {
  const row = await reviewBottle(page)
  const [id] = await objectIds(page)
  await row.getByRole('button', { name: /Choose what to mend/ }).click()
  const fill = row.getByRole('checkbox', { name: 'Fill small gaps' })
  await expect(fill).toBeChecked()
  // The merge distance shows the object's default, in the length units.
  await expect(row.getByRole('textbox', { name: 'Closer than' })).toHaveValue(/ mm$/)

  await fill.uncheck()
  await expect(row.locator('[data-health-mend-preview]')).toHaveText('Nothing to merge, separate or remove here. Tick another step.')
  await expect(row.getByRole('button', { name: 'Mend', exact: true })).toBeDisabled()

  // A distance outside the object's range is refused before any click.
  await fill.check()
  await row.getByRole('textbox', { name: 'Closer than' }).fill('5 m')
  await expect(row.getByRole('alert')).toContainText('Enter a distance between')
  await expect(row.getByRole('button', { name: 'Mend', exact: true })).toBeDisabled()
  expect(await solid(page, id)).toBe(false)
})

test('Mend all on two warped bottles: one busy card, both solid, one Undo reopens both', async ({ page }) => {
  await reviewBottle(page)
  const [first] = await objectIds(page)
  await page.evaluate((id) => window.__hew_test!.copyObject(id, 0.03, 0, 0), first)
  const ids = await objectIds(page)
  expect(ids).toHaveLength(2)
  await expect(page.locator('[data-health-row]')).toHaveCount(2)
  const mendAll = healthWindow(page).getByRole('button', { name: 'Mend all', exact: true })
  await expect(mendAll).toBeEnabled()

  // Count the busy cards the click puts up.
  await page.evaluate(() => {
    const w = window as unknown as { __busyCards: number }
    w.__busyCards = 0
    new MutationObserver((records) => {
      for (const r of records) {
        r.addedNodes.forEach((n) => {
          if (n instanceof HTMLElement && n.querySelector('[aria-busy="true"]')) w.__busyCards++
        })
      }
    }).observe(document.body, { childList: true, subtree: true })
  })
  await mendAll.click()

  await expect(page.locator('[data-health-row]')).toHaveCount(0)
  await expect(page.getByText('2 objects ✓ solid')).toBeVisible()
  expect(await page.evaluate(() => (window as unknown as { __busyCards: number }).__busyCards)).toBe(1)
  for (const id of ids) expect(await solid(page, id)).toBe(true)
  expect(await page.evaluate(() => window.__hew_test!.getLastError())).toBeNull()

  // One Undo reopens both.
  await menu(page, 'Edit', /^Undo\b/)
  await expect(page.locator('[data-health-row]')).toHaveCount(2)
  for (const id of ids) expect(await solid(page, id)).toBe(false)
})

test('Make Solid wizard: Use recommended choices mends the bottle, and one Undo reopens it', async ({ page }) => {
  await armFilePicker(page, 'warped_bottle.stl', warpedBottleStl())
  await menu(page, 'File', 'Import…')
  await page.getByRole('dialog', { name: /stl import units/i }).getByRole('button', { name: /^import$/i }).click()
  await page.getByRole('dialog', { name: /import report/i }).getByRole('button', { name: 'OK' }).click()
  const [id] = await objectIds(page)
  expect(await solid(page, id)).toBe(false)

  await menu(page, 'Object', 'Make Solid…')
  const wizard = page.getByRole('dialog', { name: 'Make Solid' })
  await expect(wizard).toBeVisible()
  await wizard.getByRole('button', { name: 'Use recommended choices' }).click()
  await expect(wizard.getByRole('heading', { name: 'Done' })).toBeVisible()
  expect(await solid(page, id)).toBe(true)
  expect(await page.evaluate(() => window.__hew_test!.getLastError())).toBeNull()
  await wizard.getByRole('button', { name: 'Done' }).click()
  await expect(wizard).toHaveCount(0)

  await menu(page, 'Edit', /^Undo\b/)
  expect(await solid(page, id)).toBe(false)
  await expect(page.locator('[data-health-badge="open"]')).toHaveText('1 object · 1 not solid')
})

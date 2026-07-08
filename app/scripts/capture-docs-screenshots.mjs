/**
 * capture-docs-screenshots — regenerates the user-guide screenshots under
 * site/public/docs/ by driving the dev app through the semantic test harness
 * (`window.__hew_test`, dev builds only) plus DOM clicks for chrome.
 *
 * Usage:
 *   pnpm exec vite --host 127.0.0.1 --port 4173 --strictPort   # in app/
 *   pnpm exec node scripts/capture-docs-screenshots.mjs [outDir]
 *
 * Shots are 1440×900 @2x with a fixed camera per scene, so reruns after a UI
 * change produce comparable images. Keep scene setups deterministic (no
 * Date/random) for the same reason.
 */
import { chromium } from '@playwright/test'
import { mkdirSync } from 'node:fs'
import { resolve, dirname } from 'node:path'
import { fileURLToPath } from 'node:url'

const APP_DIR = dirname(dirname(fileURLToPath(import.meta.url)))
const OUT = resolve(process.argv[2] ?? resolve(APP_DIR, '../site/public/docs'))
mkdirSync(OUT, { recursive: true })

const BASE = 'http://127.0.0.1:4173/'
const VIEWPORT = { width: 1440, height: 900 }

// One standard "hero" camera used by most scenes, matching the e2e goldens'
// convention (position, target, Z-up, 45° fov).
const CAM = { position: [8, 6, 8], target: [1, 1, 1], up: [0, 0, 1], fovDeg: 45 }

const browser = await chromium.launch()

/** Fresh page with the app booted and the harness ready. */
async function freshPage() {
  const page = await browser.newPage({ viewport: VIEWPORT, deviceScaleFactor: 2 })
  await page.goto(BASE)
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 30_000 })
  await page.waitForTimeout(400)
  return page
}

async function settle(page, ms = 600) {
  await page.waitForTimeout(ms)
}

async function shot(page, name, opts = {}) {
  await settle(page)
  await page.screenshot({ path: `${OUT}/${name}.png`, ...opts })
  console.log(`captured ${name}.png`)
}

// ---------------------------------------------------------------------------
// 1. Default interface, empty document
// ---------------------------------------------------------------------------
{
  const page = await freshPage()
  await shot(page, 'ui-default')
  await page.close()
}

// ---------------------------------------------------------------------------
// 2. Getting started: rectangle on the ground, then the push/pulled box
// ---------------------------------------------------------------------------
{
  const page = await freshPage()
  await page.evaluate((cam) => {
    const h = window.__hew_test
    h.setCamera(cam)
    h.drawRectangle([0, 0, 0], [2, 2, 0])
  }, CAM)
  await shot(page, 'first-rectangle')

  await page.evaluate(() => {
    const h = window.__hew_test
    // Redo the sketch as a box: clear and rebuild deterministically.
    h.undo()
    h.drawBox([0, 0, 0], [2, 2, 0], 1.2)
  })
  await shot(page, 'first-box')

  // Select the box so Entity Info shows its solid status + the dock its verbs.
  await page.evaluate(() => {
    const h = window.__hew_test
    h.selectObjects(h.getObjectIds())
  })
  await shot(page, 'box-selected')
  await page.close()
}

// ---------------------------------------------------------------------------
// 3. A richer scene: several solids, materials, a through-cut notch
// ---------------------------------------------------------------------------
{
  const page = await freshPage()
  await page.evaluate((cam) => {
    const h = window.__hew_test
    h.setCamera(cam)
    // A small "bracket": base slab + upright, with a notch subtracted.
    // Ground sketches live at z=0, so draw everything on the ground and
    // lift the upper solids into place with moveObject.
    const base = h.drawBox([0, 0, 0], [3, 2, 0], 0.4)
    const upright = h.drawBox([0, 0, 0], [0.5, 2, 0], 1.6)
    h.moveObject(upright, 0, 0, 0.4)
    const cutter = h.drawBox([-0.2, 0.7, 0], [0.8, 1.3, 0], 1.2)
    h.moveObject(cutter, 0, 0, 1.2)
    const terracotta = h.addMaterial('Terracotta', 193, 104, 79, 255)
    const slate = h.addMaterial('Slate', 90, 103, 118, 255)
    h.paintObject(base, slate)
    h.paintObject(upright, terracotta)
    // boolean op codes: 0 union, 1 subtract, 2 intersect (kernel convention)
    h.boolean(1, upright, cutter)
  }, CAM)
  await shot(page, 'bracket-scene')
  await page.close()
}

// ---------------------------------------------------------------------------
// 4. Booleans: two overlapping solids selected, dock showing combine verbs
// ---------------------------------------------------------------------------
{
  const page = await freshPage()
  await page.evaluate((cam) => {
    const h = window.__hew_test
    h.setCamera(cam)
    h.drawBox([0, 0, 0], [2, 2, 0], 1)
    h.drawBox([1.2, 1.2, 0], [3, 3, 0], 1.6)
    h.selectObjects(h.getObjectIds())
  }, CAM)
  await shot(page, 'boolean-selection')

  await page.evaluate(() => {
    const h = window.__hew_test
    const [a, b] = h.getObjectIds()
    h.boolean(0, a, b) // union
    h.selectObjects(h.getObjectIds())
  })
  await shot(page, 'boolean-union')
  await page.close()
}

// ---------------------------------------------------------------------------
// 5. Slice: one box sliced by a tilted plane, halves moved apart
// ---------------------------------------------------------------------------
{
  const page = await freshPage()
  await page.evaluate((cam) => {
    const h = window.__hew_test
    h.setCamera(cam)
    const box = h.drawBox([0, 0, 0], [2.4, 1.6, 0], 1.4)
    const [pos] = h.sliceObject(box, [1.2, 0.8, 0.7, 1, 0, 0.35])
    h.moveObject(pos, 0.9, 0, 0.25)
  }, CAM)
  await shot(page, 'slice-halves')
  await page.close()
}

// ---------------------------------------------------------------------------
// 6. Guides: construction guide lines/points around a box
// ---------------------------------------------------------------------------
{
  const page = await freshPage()
  await page.evaluate(() => {
    const h = window.__hew_test
    h.setCamera({ position: [5, 3.6, 4.4], target: [1, 1, 0.5], up: [0, 0, 1], fovDeg: 45 })
    h.drawBox([0, 0, 0], [2, 2, 0], 1)
    // Guides on the box's top plane read clearly against the sky.
    h.addGuideLine(0, 2.4, 1, 1, 0, 0)
    h.addGuideLine(2.4, 0, 1, 0, 1, 0)
    h.addGuidePoint(2.4, 2.4, 1)
  })
  await shot(page, 'guides')
  await page.close()
}

// ---------------------------------------------------------------------------
// 6b. Materials panel expanded next to the painted bracket scene
// ---------------------------------------------------------------------------
{
  const page = await freshPage()
  await page.evaluate((cam) => {
    const h = window.__hew_test
    h.setCamera(cam)
    const base = h.drawBox([0, 0, 0], [3, 2, 0], 0.4)
    const upright = h.drawBox([0, 0, 0], [0.5, 2, 0], 1.6)
    h.moveObject(upright, 0, 0, 0.4)
    const terracotta = h.addMaterial('Terracotta', 193, 104, 79, 255)
    const slate = h.addMaterial('Slate', 122, 138, 153, 255)
    h.paintObject(base, slate)
    h.paintObject(upright, terracotta)
  }, CAM)
  await page.getByRole('button', { name: /materials/i }).click()
  await shot(page, 'materials-panel')
  await page.close()
}

// ---------------------------------------------------------------------------
// 6c. Organization: a named group, Entity Info rename + tag, Tags panel
// ---------------------------------------------------------------------------
{
  const page = await freshPage()
  await page.evaluate((cam) => {
    const h = window.__hew_test
    h.setCamera(cam)
    h.drawBox([0, 0, 0], [1.4, 1.4, 0], 1)
    const lid = h.drawBox([1.8, 0, 0], [3.2, 1.4, 0], 1)
    h.selectObjects(h.getObjectIds())
  }, CAM)
  await page.getByRole('button', { name: 'Edit', exact: true }).click()
  await settle(page, 200)
  await page.getByText('Group', { exact: true }).click()
  await settle(page, 400)
  // Rename the group in Entity Info (name input's placeholder is the default label).
  const nameInput = page.getByPlaceholder(/^Group /)
  await nameInput.fill('Enclosure')
  await nameInput.press('Enter')
  await settle(page, 200)
  // Tag it from Entity Info's add-tag field.
  await page.getByRole('button', { name: 'Add tag' }).click()
  const tagInput = page.getByPlaceholder('Structure/Roof')
  await tagInput.fill('Structure/Base')
  await tagInput.press('Enter')
  await settle(page, 200)
  // Expand the Tags tray section so the derived tag tree is visible.
  await page.getByRole('button', { name: /tags/i }).click()
  await shot(page, 'organization')
  await page.close()
}

// ---------------------------------------------------------------------------
// 7. Command palette open with a query
// ---------------------------------------------------------------------------
{
  const page = await freshPage()
  await page.evaluate((cam) => {
    const h = window.__hew_test
    h.setCamera(cam)
    h.drawBox([0, 0, 0], [2, 2, 0], 1.2)
  }, CAM)
  await page.keyboard.press(process.platform === 'darwin' ? 'Meta+k' : 'Control+k')
  await settle(page, 300)
  await page.keyboard.type('push')
  await shot(page, 'command-palette')
  await page.close()
}

// ---------------------------------------------------------------------------
// 8. Export dialog (File > Export…)
// ---------------------------------------------------------------------------
{
  const page = await freshPage()
  await page.evaluate((cam) => {
    const h = window.__hew_test
    h.setCamera(cam)
    h.drawBox([0, 0, 0], [2, 2, 0], 1.2)
  }, CAM)
  await page.getByRole('button', { name: 'File' }).click()
  await settle(page, 200)
  await page.getByText('Export…').click()
  await shot(page, 'export-dialog')
  await page.close()
}

// ---------------------------------------------------------------------------
// 9. Settings window (units pane)
// ---------------------------------------------------------------------------
{
  const page = await freshPage()
  await page.keyboard.press(process.platform === 'darwin' ? 'Meta+k' : 'Control+k')
  await settle(page, 200)
  await page.keyboard.type('settings')
  await page.keyboard.press('Enter')
  await shot(page, 'settings')
  await page.close()
}

await browser.close()
console.log(`\nDone. Screenshots in ${OUT}`)

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
 *
 * The whole manual is captured in **dark mode** (seeded via localStorage before
 * the app boots — themes both the CSS chrome and the WebGL clear color), since
 * that is how most people run Hew. Modeling scenes hide the origin axes so the
 * red/green/blue lines don't read on top of the solids; the interface tour
 * (ui-default) keeps them, being a faithful shot of the default window.
 *
 * NOT every shot under site/public/docs/ comes from here. The getting-started
 * chapter's are taken by hand through `capture-live-shot.mjs`, which lands the
 * same 2880×1800 format from a session someone drives themselves — a live
 * cursor, a gesture caught mid-drag, and inference cues this harness can't
 * stage. Anything captured that way must NOT get a scene here, or the next run
 * silently overwrites it; that is why the desk-organizer scenes and the export
 * dialog are gone.
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

/** Fresh page with the app booted, in dark mode, welcome dialog suppressed
 * (pass `{ welcome: true }` to keep the welcome overlay up, as on a first
 * launch). */
async function freshPage({ welcome = false } = {}) {
  const page = await browser.newPage({ viewport: VIEWPORT, deviceScaleFactor: 2 })
  await page.addInitScript((showWelcome) => {
    // Seed before any app module loads: dark theme + no welcome overlay.
    localStorage.setItem('hew.settings.theme', 'dark')
    localStorage.setItem('hew.settings.showWelcome', String(showWelcome))
  }, welcome)
  await page.goto(BASE)
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 30_000 })
  await page.waitForTimeout(400)
  // A GPU-less environment (headless capture on CI or a VM) surfaces the
  // software-GL notice; dismiss it so it never lands in a published shot.
  // dispatchEvent, not click(): the welcome overlay may cover the button.
  // No-op on a hardware-accelerated machine.
  const dismiss = page.getByRole('button', { name: 'Dismiss the graphics-acceleration notice' })
  if (await dismiss.count()) await dismiss.dispatchEvent('click')
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
// 0. Welcome screen, as on a first launch, with Centimeters chosen in the
//    Units dropdown — the state the getting-started chapter asks for.
// ---------------------------------------------------------------------------
{
  const page = await freshPage({ welcome: true })
  await page.getByRole('dialog', { name: 'Welcome to Hew' }).waitFor()
  await page.getByRole('combobox', { name: 'Units' }).selectOption('cm')
  await shot(page, 'welcome-screen')
  await page.close()
}

// ---------------------------------------------------------------------------
// 1. Default interface, empty document (keeps the axes — this is the tour shot)
// ---------------------------------------------------------------------------
{
  const page = await freshPage()
  await shot(page, 'ui-default')
  await page.close()
}

// ---------------------------------------------------------------------------
// 2. A plain box: the extrusion Push/Pull shows, and the same box selected for
//    Core concepts' "one closed extrusion is one Object".
// ---------------------------------------------------------------------------
{
  const page = await freshPage()
  await page.evaluate((cam) => {
    const h = window.__hew_test
    h.setCamera(cam)
    h.setAxesVisible(false)
    h.drawBox([0, 0, 0], [2, 2, 0], 1.2)
  }, CAM)
  await shot(page, 'first-box')

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
    h.setAxesVisible(false)
    const base = h.drawBox([0, 0, 0], [3, 2, 0], 0.4)
    const upright = h.drawBox([0, 0, 0], [0.5, 2, 0], 1.6)
    h.moveObject(upright, 0, 0, 0.4)
    const cutter = h.drawBox([-0.2, 0.7, 0], [0.8, 1.3, 0], 1.2)
    h.moveObject(cutter, 0, 0, 1.2)
    const terracotta = h.addMaterial('Terracotta', 193, 104, 79, 255)
    const slate = h.addMaterial('Slate', 90, 103, 118, 255)
    h.paintObject(base, slate)
    h.paintObject(upright, terracotta)
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
    h.setAxesVisible(false)
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
    h.setAxesVisible(false)
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
    h.setAxesVisible(false)
    h.drawBox([0, 0, 0], [2, 2, 0], 1)
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
    h.setAxesVisible(false)
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

  // Select Slate and drag its opacity down through the real slider, so the
  // opacity screenshot shows genuine UI interaction, not a scripted value.
  await page.getByTitle('Slate', { exact: true }).click()
  await settle(page, 150)
  const slider = page.getByRole('slider')
  const sliderBox = await slider.boundingBox()
  await page.mouse.move(sliderBox.x + sliderBox.width - 2, sliderBox.y + sliderBox.height / 2)
  await page.mouse.down()
  await page.mouse.move(sliderBox.x + sliderBox.width * 0.65, sliderBox.y + sliderBox.height / 2, {
    steps: 10,
  })
  await page.mouse.up()
  await shot(page, 'materials-opacity')
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
    h.setAxesVisible(false)
    h.drawBox([0, 0, 0], [1.4, 1.4, 0], 1)
    h.drawBox([1.8, 0, 0], [3.2, 1.4, 0], 1)
    h.selectObjects(h.getObjectIds())
  }, CAM)
  await page.getByRole('button', { name: 'Edit', exact: true }).click()
  await settle(page, 200)
  await page.getByTestId('menu-bar').getByText('Group', { exact: true }).click()
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
    h.setAxesVisible(false)
    h.drawBox([0, 0, 0], [2, 2, 0], 1.2)
  }, CAM)
  await page.keyboard.press(process.platform === 'darwin' ? 'Meta+/' : 'Control+/')
  await settle(page, 300)
  await page.keyboard.type('push')
  await shot(page, 'command-palette')
  await page.close()
}

// 8. Settings window (units pane)
// ---------------------------------------------------------------------------
{
  const page = await freshPage()
  await page.keyboard.press(process.platform === 'darwin' ? 'Meta+/' : 'Control+/')
  await settle(page, 200)
  await page.keyboard.type('settings')
  await page.keyboard.press('Enter')
  await shot(page, 'settings')
  await page.close()
}

await browser.close()
console.log(`\nDone. Screenshots in ${OUT}`)

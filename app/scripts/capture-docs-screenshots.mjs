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

// The desk-organizer project (getting-started chapter) is a bigger scene; this
// frames the whole 14×7.5 tray with headroom for the pen cup.
// Units == centimeters here, matching the exact dimensions the chapter quotes
// (24x14 tray, etc.), so the shots are dimensionally honest.
const ORG_CAM = { position: [42, 28, 22], target: [12, 6.5, 2.0], up: [0, 0, 1], fovDeg: 34 }

const browser = await chromium.launch()

/** Fresh page with the app booted, in dark mode, welcome dialog suppressed. */
async function freshPage() {
  const page = await browser.newPage({ viewport: VIEWPORT, deviceScaleFactor: 2 })
  await page.addInitScript(() => {
    // Seed before any app module loads: dark theme + no welcome overlay.
    localStorage.setItem('hew.settings.theme', 'dark')
    localStorage.setItem('hew.settings.showWelcome', 'false')
  })
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
// 1. Default interface, empty document (keeps the axes — this is the tour shot)
// ---------------------------------------------------------------------------
{
  const page = await freshPage()
  await shot(page, 'ui-default')
  await page.close()
}

// ---------------------------------------------------------------------------
// 2. Getting-started primitives reused by other chapters: rectangle, box.
//    (The getting-started chapter itself uses the desk-organizer shots below.)
// ---------------------------------------------------------------------------
{
  const page = await freshPage()
  await page.evaluate((cam) => {
    const h = window.__hew_test
    h.setCamera(cam)
    h.setAxesVisible(false)
    h.drawRectangle([0, 0, 0], [2, 2, 0])
  }, CAM)
  await shot(page, 'first-rectangle')

  await page.evaluate(() => {
    const h = window.__hew_test
    h.undo()
    h.drawBox([0, 0, 0], [2, 2, 0], 1.2)
  })
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
    h.setAxesVisible(false)
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
    h.setAxesVisible(false)
    h.drawBox([0, 0, 0], [2, 2, 0], 1.2)
  }, CAM)
  await page.getByRole('button', { name: 'File' }).click()
  await settle(page, 200)
  await page.getByText('Export…').click()
  // Show the STL branch: it carries the per-format Curve resolution select
  // the guide describes.
  await page.locator('#export-format-select').selectOption('stl')
  await settle(page, 200)
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

// ---------------------------------------------------------------------------
// 10. Getting started: the desk-organizer project, built stage by stage on a
//     single page so the set grows shot to shot. Every solid is a discrete,
//     watertight Object; combining (the bin's scoop) is always explicit.
// ---------------------------------------------------------------------------
{
  const page = await freshPage()
  await page.evaluate((cam) => {
    const h = window.__hew_test
    h.setCamera(cam)
    h.setAxesVisible(false)
  }, ORG_CAM)

  // Draw the tray footprint (a closed rectangle sketch region).
  await page.evaluate(() => {
    window.__org = {}
    window.__org.rect = window.__hew_test.drawRectangle([0, 0, 0], [24, 14, 0])
  })
  await shot(page, 'organizer-sketch')

  // Push/pull it into a solid board (1.5 cm); select it so Object Info reads Solid.
  await page.evaluate(() => {
    const h = window.__hew_test
    const r = window.__org.rect
    window.__org.tray = h.extrudeRegion(r.sketch, r.region, 1.5)
    h.selectObjects([window.__org.tray])
  })
  await shot(page, 'organizer-tray')
  await page.evaluate(() => window.__hew_test.selectObjects([]))

  // Pen cup: a cylinder (r3 -> 6 cm across, 9 cm tall) hollowed from the top.
  await page.evaluate(() => {
    const h = window.__hew_test
    const T = 1.5
    const cupR = 3, cupH = 9, cx = 4.5, cy = 7
    const circle = h.drawCircle([cx, cy, 0], cupR)
    const cup = h.extrudeRegion(circle.sketch, circle.region, cupH)
    const top = h.pickFace([cx, cy, 40], [0, 0, -1])
    const inner = h.imprintCircleOnFace(cup, top.face, [cx, cy, cupH], 2.4)
    h.pushPull(cup, inner, -(cupH - 1.5)) // hollow to a 1.5 cm floor
    h.moveObject(cup, 0, 0, T)
    window.__org.cup = cup
  })
  await shot(page, 'organizer-cup')

  // Bin (7 x 5 x 6 cm): hollow it first (0.7 cm walls, 1 cm floor), then scoop
  // the front with a cylinder — matching the chapter's order. The scoop
  // position is chosen to avoid exact facet/face coincidences, which the
  // kernel still (correctly) refuses as degenerate contact.
  await page.evaluate(() => {
    const h = window.__hew_test
    const T = 1.5, binH = 6
    let bin = h.drawBox([9, 4.5, 0], [16, 9.5, 0], binH)
    const cutter = h.drawBox([9.7, 5.2, 0], [15.3, 8.8, 0], binH + 0.5)
    h.moveObject(cutter, 0, 0, 1)
    bin = h.boolean(1, bin, cutter)
    const scoop = h.drawCircle([0, 0, 0], 2.0)
    const scCyl = h.extrudeRegion(scoop.sketch, scoop.region, 9)
    h.rotateObject(scCyl, 90, [0, 1, 0]) // axis Z -> axis X
    h.moveObject(scCyl, 8.0, 9.5, binH + 0.5)
    bin = h.boolean(1, bin, scCyl)
    h.moveObject(bin, 0, 0, T)
    window.__org.bin = bin
  })
  await shot(page, 'organizer-bin')

  // Phone stand, step 1: the profile as the chapter teaches it — a 6 x 8
  // rectangle, a guide line 1 cm above its bottom edge, and a diagonal from
  // the guide/edge intersection to the opposite top corner, splitting the
  // rectangle into two regions. Drawn as one Euler-path chain so the whole
  // profile shares a sketch (as the shared ground sketch would in real use).
  // Grid off so the dashed guide reads clearly.
  await page.evaluate(() => {
    const h = window.__hew_test
    const P = (x, y) => [27 + x, 3 + y, 0]
    h.drawLineChain([P(0, 8), P(0, 0), P(6, 0), P(6, 1), P(6, 8), P(0, 8), P(6, 1)])
    h.addGuideLine(27, 4, 0, 1, 0, 0)
    h.setGridVisible(false)
    h.setCamera({ position: [30, 1, 18], target: [30, 7.5, 0], up: [0, 0, 1], fovDeg: 40 })
  })
  await shot(page, 'organizer-stand-profile')

  // Phone stand, step 2: rebuild the trimmed wedge (the state after the two
  // excess lines are deleted), extrude 5 cm, tip upright, move onto the tray.
  // Undo x2 clears the guide and the teaching chain (and its emptied sketch).
  await page.evaluate((cam) => {
    const h = window.__hew_test
    h.undo() // the guide
    h.undo() // the profile chain (one gesture)
    h.setGridVisible(true)
    const P = (x, y) => [27 + x, 3 + y, 0]
    const wedge = h.drawLineChain([P(0, 0), P(0, 8), P(6, 1), P(6, 0), P(0, 0)])
    const stand = h.extrudeRegion(wedge.sketch, wedge.regions[0], 5)
    h.rotateObject(stand, 90, [1, 0, 0])
    h.moveObject(stand, -10, 9.5, -1.5)
    h.setCamera(cam)
    window.__org.stand = stand
  }, ORG_CAM)
  await shot(page, 'organizer-set')

  // Materials: paint each part, then reveal the Materials palette.
  await page.evaluate(() => {
    const h = window.__hew_test
    const o = window.__org
    const oak = h.addMaterial('Oak', 198, 161, 110, 255)
    const teal = h.addMaterial('Teal', 74, 138, 138, 255)
    const terracotta = h.addMaterial('Terracotta', 193, 104, 79, 255)
    const slate = h.addMaterial('Slate', 90, 103, 118, 255)
    h.paintObject(o.tray, oak)
    h.paintObject(o.cup, teal)
    h.paintObject(o.bin, terracotta)
    h.paintObject(o.stand, slate)
  })
  await page.getByRole('button', { name: /materials/i }).click()
  await shot(page, 'organizer-materials')

  // Organize: rename each part in Object Info, group the set, tag it.
  const names = [
    ['tray', 'Tray'],
    ['cup', 'Pen cup'],
    ['bin', 'Bin'],
    ['stand', 'Phone stand'],
  ]
  for (const [key, label] of names) {
    await page.evaluate((k) => window.__hew_test.selectObjects([window.__org[k]]), key)
    await settle(page, 150)
    const input = page.getByPlaceholder(/^Object /)
    await input.fill(label)
    await input.press('Enter')
    await settle(page, 120)
  }
  await page.evaluate(() => window.__hew_test.selectAll())
  await settle(page, 150)
  await page.getByRole('button', { name: 'Edit', exact: true }).click()
  await settle(page, 200)
  await page.getByText('Group', { exact: true }).click()
  await settle(page, 400)
  const groupName = page.getByPlaceholder(/^Group /)
  await groupName.fill('Desk organizer')
  await groupName.press('Enter')
  await settle(page, 200)
  await page.getByRole('button', { name: 'Add tag' }).click()
  const tagInput = page.getByPlaceholder('Structure/Roof')
  await tagInput.fill('Desk/Set')
  await tagInput.press('Enter')
  await settle(page, 200)
  // Expand the group in the Outliner so the renamed parts (Tray, Pen cup, …) show.
  // Relies on exactly one collapsed group ("Desk organizer") being present and the
  // Tags tray still collapsed at this point, so the sole '▸' caret is this group's.
  // If this scene grows a second group, scope this to the Outliner row instead.
  await page.getByRole('button', { name: '▸' }).click()
  await settle(page, 200)
  await page.getByRole('button', { name: /tags/i }).click()
  await shot(page, 'organizer-organized')
  await page.close()
}

await browser.close()
console.log(`\nDone. Screenshots in ${OUT}`)

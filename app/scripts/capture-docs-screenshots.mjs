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
import { mkdirSync, rmSync } from 'node:fs'
import { spawnSync } from 'node:child_process'
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

// Published shots are WebP: at quality 90 a 2880x1800 UI capture lands around
// 100-600 KB against multi-MB PNGs, with no visible loss on line art or text.
// Playwright doesn't encode WebP, so capture PNG and hand it to cwebp.
if (spawnSync('cwebp', ['-version']).error)
  throw new Error('cwebp is required (brew install webp)')

async function shot(page, name, opts = {}) {
  await settle(page)
  const tmp = `${OUT}/.${name}.capture.png`
  await page.screenshot({ path: tmp, ...opts })
  const res = spawnSync('cwebp', ['-q', '90', '-m', '6', tmp, '-o', `${OUT}/${name}.webp`])
  rmSync(tmp, { force: true })
  if (res.status !== 0) throw new Error(`cwebp failed for ${name}: ${res.stderr}`)
  console.log(`captured ${name}.webp`)
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


// ---------------------------------------------------------------------------
// 9. Follow Me — the profile-and-path setup, then the classic results:
//    picture frame, crown molding, sphere, goblet. Profiles are drawn flat
//    (or stood upright with rotateSketch, matching the tool's auto-stand)
//    exactly as the follow-me chapter teaches.
// ---------------------------------------------------------------------------
{
  const page = await freshPage()
  await page.evaluate(() => {
    const h = window.__hew_test
    h.setCamera({ position: [3.0, 2.4, 2.0], target: [0.4, 0, 0.3], up: [0, 0, 1], fovDeg: 45 })
    h.setAxesVisible(false)
    h.drawCircle([0, 0, 0], 1)
    const pts = [
      [0, 0, 0], [0.5, 0, 0], [0.48, 0.08, 0], [0.12, 0.14, 0], [0.09, 0.75, 0],
      [0.4, 0.95, 0], [0.52, 1.25, 0], [0.55, 1.6, 0], [0.47, 1.6, 0], [0.44, 1.28, 0],
      [0.33, 1.02, 0], [0, 0.88, 0], [0, 0, 0],
    ].map(([x, y]) => [x + 1.4, y - 0.8, 0])
    h.drawLineChain(pts)
  })
  await shot(page, 'follow-me-setup')
  await page.close()
}

{
  const page = await freshPage()
  await page.evaluate(() => {
    const h = window.__hew_test
    h.setCamera({ position: [3.4, 2.8, 2.6], target: [1, 0.75, 0], up: [0, 0, 1], fovDeg: 45 })
    h.setAxesVisible(false)
    const path = h.drawRectangle([0, 0, 0], [2, 1.5, 0])
    const pts = [
      [0, 0.6, 0], [-0.14, 0.6, 0], [-0.14, 0.72, 0], [-0.04, 0.78, 0], [0, 0.74, 0], [0, 0.6, 0],
    ]
    const prof = h.drawLineChain(pts)
    const obj = h.followMeAlongEdges(prof.sketch, prof.regions[0], path.sketch, h.getSketchEdgeIds(path.sketch))
    h.paintObject(obj, h.addMaterial('Walnut', 121, 92, 61, 255))
  })
  await shot(page, 'follow-me-frame')
  await page.close()
}

{
  const page = await freshPage()
  await page.evaluate(() => {
    const h = window.__hew_test
    h.setCamera({ position: [6.5, 5, 4.6], target: [1.5, 1, 1], up: [0, 0, 1], fovDeg: 45 })
    h.setAxesVisible(false)
    const box = h.drawBox([0, 0, 0], [3, 2, 0], 1.8)
    h.paintObject(box, h.addMaterial('Slate', 90, 103, 118, 255))
    const cs = [
      [0, 1.8], [0, 1.55], [-0.06, 1.58], [-0.1, 1.66], [-0.12, 1.78], [-0.12, 1.8], [0, 1.8],
    ]
    const prof = h.drawLineChain(cs.map(([a, zt]) => [1.5 + a, zt - 1.8, 0]))
    h.rotateSketch(prof.sketch, 90, [1, 0, 0], [0, -0.9, 0.9])
    h.rotateSketch(prof.sketch, 90, [0, 0, 1], [1.5, 0, 0])
    const top = h.pickFace([1.5, 1, 3], [0, 0, -1])
    const obj = h.followMeAroundFace(prof.sketch, prof.regions[0], top.object, top.face)
    h.paintObject(obj, h.addMaterial('Terracotta', 193, 104, 79, 255))
  })
  await shot(page, 'follow-me-molding')
  await page.close()
}

{
  const page = await freshPage()
  await page.evaluate(() => {
    const h = window.__hew_test
    h.setCamera({ position: [3.4, 2.6, 2.2], target: [0, 0, 0.2], up: [0, 0, 1], fovDeg: 45 })
    h.setAxesVisible(false)
    const path = h.drawCircle([0, 0, 0], 1)
    const prof = h.drawCircle([0, 0, 0], 1)
    h.rotateSketch(prof.sketch, 90, [1, 0, 0], [0, 0, 0])
    const obj = h.followMeAlongEdges(prof.sketch, prof.region, path.sketch, h.getSketchEdgeIds(path.sketch))
    h.paintObject(obj, h.addMaterial('Terracotta', 193, 104, 79, 255))
  })
  await shot(page, 'follow-me-sphere')
  await page.close()
}

{
  const page = await freshPage()
  await page.evaluate(() => {
    const h = window.__hew_test
    h.setCamera({ position: [3.2, 2.4, 2.4], target: [0, 0, 0.8], up: [0, 0, 1], fovDeg: 45 })
    h.setAxesVisible(false)
    const path = h.drawCircle([0, 0, 0], 0.6)
    const pts = [
      [0, 0, 0], [0.5, 0, 0], [0.48, 0.08, 0], [0.12, 0.14, 0], [0.09, 0.75, 0],
      [0.4, 0.95, 0], [0.52, 1.25, 0], [0.55, 1.6, 0], [0.47, 1.6, 0], [0.44, 1.28, 0],
      [0.33, 1.02, 0], [0, 0.88, 0], [0, 0, 0],
    ]
    const prof = h.drawLineChain(pts)
    h.rotateSketch(prof.sketch, 90, [1, 0, 0], [0, 0, 0])
    const obj = h.followMeAlongEdges(prof.sketch, prof.regions[0], path.sketch, h.getSketchEdgeIds(path.sketch))
    h.paintObject(obj, h.addMaterial('Brass', 181, 148, 92, 255))
  })
  await shot(page, 'follow-me-goblet')
  await page.close()
}

// ---------------------------------------------------------------------------
// 10. Drawing: the five shapes as filled regions, and drawing on a face
//     (a boss pulled out, a recess pushed in), and a through-cut.
// ---------------------------------------------------------------------------
{
  const page = await freshPage()
  await page.evaluate(() => {
    const h = window.__hew_test
    h.setCamera({ position: [3.6, -3.2, 4.4], target: [1.7, 1.1, 0], up: [0, 0, 1], fovDeg: 45 })
    h.setAxesVisible(false)
    h.drawRectangle([0, 0, 0], [1.4, 1, 0])
    h.drawCircle([2.4, 0.5, 0], 0.55)
    const hex = []
    for (let i = 0; i <= 6; i++) { const a = Math.PI / 3 * i; hex.push([0.7 + 0.55 * Math.cos(a), 2.3 + 0.55 * Math.sin(a), 0]) }
    h.drawLineChain(hex)
    h.drawArc([1.9, 2.0, 0], [3.0, 2.0, 0], 0.5, true)
  })
  await shot(page, 'drawing-shapes')
  await page.close()
}

{
  const page = await freshPage()
  await page.evaluate(() => {
    const h = window.__hew_test
    h.setCamera({ position: [2.6, -3.4, 2.6], target: [1.1, 0.6, 0.7], up: [0, 0, 1], fovDeg: 45 })
    h.setAxesVisible(false)
    const box = h.drawBox([0, 0, 0], [2.2, 1.2, 0], 1.4)
    const front = h.pickFace([1.1, -1, 0.7], [0, 1, 0])
    const boss = h.imprintCircleOnFace(front.object, front.face, [0.6, 0, 0.8], 0.3)
    h.pushPull(front.object, boss, 0.3)
    const front2 = h.pickFace([1.6, -1, 0.7], [0, 1, 0])
    const recess = h.imprintCircleOnFace(front2.object, front2.face, [1.6, 0, 0.8], 0.3)
    h.pushPull(front2.object, recess, -0.3)
  })
  await shot(page, 'drawing-on-face')
  await page.close()
}

{
  const page = await freshPage()
  await page.evaluate(() => {
    const h = window.__hew_test
    h.setCamera({ position: [2.6, -3.0, 1.9], target: [1.1, 0.45, 0.55], up: [0, 0, 1], fovDeg: 45 })
    h.setAxesVisible(false)
    const box = h.drawBox([0, 0, 0], [2.2, 0.9, 0], 1.2)
    const front = h.pickFace([1.1, -1, 0.6], [0, 1, 0])
    const hole = h.imprintCircleOnFace(front.object, front.face, [1.1, 0, 0.6], 0.35)
    h.pushPull(front.object, hole, -0.9)
  })
  await shot(page, 'pushpull-throughcut')
  await page.close()
}

// ---------------------------------------------------------------------------
// 11. Core concepts: two flush boxes stay two Objects — one selected.
// ---------------------------------------------------------------------------
{
  const page = await freshPage()
  await page.evaluate(() => {
    const h = window.__hew_test
    h.setCamera({ position: [5.4, 4.2, 3.6], target: [1.6, 1, 0.6], up: [0, 0, 1], fovDeg: 45 })
    h.setAxesVisible(false)
    h.drawBox([0, 0, 0], [1.6, 1.6, 0], 1.1)
    const b = h.drawBox([1.6, 0, 0], [3.2, 1.6, 0], 1.1)
    h.selectObjects([b])
  })
  await shot(page, 'core-touching')
  await page.close()
}

// ---------------------------------------------------------------------------
// 12. Transforms: a copied fence (Move's array gesture), and the Scale grips.
// ---------------------------------------------------------------------------
{
  const page = await freshPage()
  await page.evaluate(() => {
    const h = window.__hew_test
    h.setCamera({ position: [6.4, -4.4, 3.4], target: [2.6, 0.6, 0.8], up: [0, 0, 1], fovDeg: 45 })
    h.setAxesVisible(false)
    const post = h.drawBox([0, 0, 0], [0.16, 0.16, 0], 1.5)
    for (let i = 1; i <= 5; i++) h.copyNode('object', post, i * 1.0, 0, 0)
    const rail = h.drawBox([-0.1, 0.02, 0], [5.26, 0.14, 0], 0.12)
    h.moveObject(rail, 0, 0, 1.26)
    h.copyNode('object', rail, 0, 0, -0.55)
    const cedar = h.addMaterial('Cedar', 156, 106, 70, 255)
    for (const id of h.getObjectIds()) h.paintObject(id, cedar)
  })
  await shot(page, 'transform-array')
  await page.close()
}

{
  const page = await freshPage()
  await page.evaluate(() => {
    const h = window.__hew_test
    h.setCamera({ position: [5.2, 4, 3.6], target: [1, 1, 0.7], up: [0, 0, 1], fovDeg: 45 })
    h.setAxesVisible(false)
    const box = h.drawBox([0, 0, 0], [2, 1.4, 0], 1.2)
    h.selectObjects([box])
  })
  await page.locator('button:has-text("Scale")').first().click()
  await shot(page, 'scale-grips')
  await page.close()
}

// ---------------------------------------------------------------------------
// 13. Components: one Shelf Bracket definition, four instances, one selected
//     so Object Info shows the sibling count.
// ---------------------------------------------------------------------------
{
  const page = await freshPage()
  await page.evaluate(() => {
    const h = window.__hew_test
    h.setCamera({ position: [6.2, -4.6, 3.8], target: [2.4, 0.8, 0.5], up: [0, 0, 1], fovDeg: 45 })
    h.setAxesVisible(false)
    const base = h.drawBox([0, 0, 0], [0.9, 0.6, 0], 0.18)
    const up = h.drawBox([0, 0, 0], [0.18, 0.6, 0], 1.0)
    const u = h.boolean(0, base, up)
    const { instance, component } = h.makeComponent([u])
    h.setComponentName(component, 'Shelf Bracket')
    for (let i = 1; i <= 3; i++) h.placeInstance(component, i * 1.4, 0, 0)
    h.selectNodes([{ kind: 'instance', id: instance }])
  })
  await shot(page, 'components-instances')
  await page.close()
}

// ---------------------------------------------------------------------------
// 14. Dimensions: linear dimensions on a box, a radial one on a cylinder.
// ---------------------------------------------------------------------------
{
  const page = await freshPage()
  await page.evaluate(() => {
    const h = window.__hew_test
    h.setCamera({ position: [4.6, -3.8, 3.2], target: [1.6, 0.7, 0.5], up: [0, 0, 1], fovDeg: 45 })
    h.setAxesVisible(false)
    h.drawBox([0, 0, 0], [1.8, 1.2, 0], 0.9)
    const c = h.drawCircle([3.1, 0.6, 0], 0.5)
    h.extrudeRegion(c.sketch, c.region, 0.5)
    h.addLinearDimension([0, 0, 0], [1.8, 0, 0], [0, -0.45, 0], [0, 0, 0, 0, 0, 1])
    h.addLinearDimension([1.8, 0, 0], [1.8, 0, 0.9], [0.4, -0.25, 0], [1.8, 0, 0, 0, 1, 0])
    h.addRadialDimension([3.1, 0.1, 0.5], 'radius', [3.1, 0.6, 0.5], 0.5, [3.1, 0.6, 0.5, 0, 0, 1], [0.5, -1, 0])
  })
  await shot(page, 'dimensions')
  await page.close()
}

// ---------------------------------------------------------------------------
// 15. Section plane: a hollow chest with a shelf, cut open through the real
//     tool — axis-locked plane through an interior snap point.
// ---------------------------------------------------------------------------
{
  const page = await freshPage()
  await page.evaluate(() => {
    const h = window.__hew_test
    h.setCamera({ position: [4.8, -4.2, 3.4], target: [1.1, 0.8, 0.7], up: [0, 0, 1], fovDeg: 45 })
    h.setAxesVisible(false)
    const box = h.drawBox([0, 0, 0], [2.2, 1.6, 0], 1.4)
    const top = h.pickFace([1.1, 0.8, 3], [0, 0, -1])
    const inner = h.offsetFace(top.object, top.face, -0.15)
    h.pushPull(top.object, inner, -1.1)
    const shelf = h.drawBox([0.45, 0.35, 0], [1.75, 1.25, 0], 0.08)
    h.moveObject(shelf, 0, 0, 0.55)
  })
  await page.locator('button:has-text("Section Plane")').first().click()
  await settle(page, 300)
  await page.mouse.move(400, 750)
  await settle(page, 200)
  await page.keyboard.press('ArrowLeft')
  await settle(page, 200)
  const pt = await page.evaluate(() => window.__hew_test.worldToScreen([1.1, 0.8, 0.63]))
  await page.mouse.move(pt.x, pt.y, { steps: 8 })
  await settle(page, 300)
  await page.mouse.click(pt.x, pt.y)
  await page.mouse.move(1300, 200)
  await shot(page, 'section-cut')
  await page.close()
}

// ---------------------------------------------------------------------------
// 16. 3D Text: the real dialog flow — type, OK, place on the ground, select.
// ---------------------------------------------------------------------------
{
  const page = await freshPage()
  await page.evaluate(() => {
    const h = window.__hew_test
    h.setCamera({ position: [0.75, -0.55, 0.5], target: [0.22, 0.24, 0.03], up: [0, 0, 1], fovDeg: 45 })
    h.setAxesVisible(false)
  })
  await page.locator('button:has-text("3D Text")').first().click()
  await settle(page, 400)
  await page.locator('textarea').first().fill('HEW')
  await page.getByRole('button', { name: 'OK', exact: true }).click()
  await settle(page, 400)
  const pt = await page.evaluate(() => window.__hew_test.worldToScreen([0.1, 0.2, 0]))
  await page.mouse.move(pt.x, pt.y, { steps: 5 })
  await settle(page, 300)
  await page.mouse.click(pt.x, pt.y)
  await settle(page, 500)
  await page.keyboard.press('Escape')
  await page.keyboard.press('Space')
  await settle(page, 300)
  await page.mouse.move(1300, 250)
  await shot(page, 'text-3d')
  await page.close()
}

// ---------------------------------------------------------------------------
// 17. Viewing: the bracket scene through the real Top and Front buttons.
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
    h.paintObject(base, h.addMaterial('Slate', 90, 103, 118, 255))
    h.paintObject(upright, h.addMaterial('Terracotta', 193, 104, 79, 255))
  }, CAM)
  await page.getByRole('button', { name: 'Top', exact: true }).click()
  await shot(page, 'views-top')
  await page.getByRole('button', { name: 'Front', exact: true }).click()
  await shot(page, 'views-front')
  await page.close()
}

// ---------------------------------------------------------------------------
// 18. Shop Mode landing, phone-framed (390x844 @3x, touch).
// ---------------------------------------------------------------------------
{
  const page = await browser.newPage({
    viewport: { width: 390, height: 844 },
    deviceScaleFactor: 3,
    hasTouch: true,
    isMobile: true,
  })
  await page.goto(BASE)
  await page.waitForTimeout(4000)
  await shot(page, 'shop-mode-landing')
  await page.close()
}

await browser.close()
console.log(`\nDone. Screenshots in ${OUT}`)

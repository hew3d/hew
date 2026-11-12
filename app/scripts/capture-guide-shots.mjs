/**
 * capture-guide-shots — regenerates the getting-started / groups / scenes /
 * printing / phone chapter shots that show a live session: a cursor, a gesture
 * caught mid-drag, an inference tooltip, a typed value. They were hand-captured
 * through capture-live-shot.mjs; this drives the same dev app with REAL mouse
 * and keyboard events (Playwright) and reuses that script's cursor overlay, so
 * a rerun after a UI change gives the same kind of image without a person.
 *
 * Usage:
 *   pnpm exec vite --host 127.0.0.1 --port 4173 --strictPort   # in app/
 *   pnpm exec node scripts/capture-guide-shots.mjs [outDir] [shot ...]
 *
 * With no shot names every scene runs. Output is 1440x900 @2x, dark mode,
 * WebP via cwebp, the same as capture-docs-screenshots.mjs.
 */
import { chromium } from '@playwright/test'
import { mkdirSync, rmSync, readFileSync } from 'node:fs'
import { spawnSync } from 'node:child_process'
import { resolve, dirname } from 'node:path'
import { fileURLToPath } from 'node:url'

const APP_DIR = dirname(dirname(fileURLToPath(import.meta.url)))
const args = process.argv.slice(2)
const OUT = resolve(args[0] ?? resolve(APP_DIR, '../site/public/docs'))
const ONLY = args.slice(1)
mkdirSync(OUT, { recursive: true })

const BASE = process.env.HEW_BASE ?? 'http://127.0.0.1:4173/'
const VIEWPORT = { width: 1440, height: 900 }
if (spawnSync('cwebp', ['-version']).error) throw new Error('cwebp is required (brew install webp)')

// The cursor overlay lives in capture-live-shot.mjs; lift its source rather
// than keep a second copy (that script runs on import, so it can't be imported).
const liveSrc = readFileSync(resolve(APP_DIR, 'scripts/capture-live-shot.mjs'), 'utf8')
const start = liveSrc.indexOf('function installCursorOverlay()')
const end = liveSrc.indexOf('// ----', start)
const overlaySrc = liveSrc.slice(start, end)

const browser = await chromium.launch()

async function boot({ light = false, cursor = true, welcome = false } = {}) {
  const page = await browser.newPage({ viewport: VIEWPORT, deviceScaleFactor: 2 })
  await page.addInitScript(
    ([theme, showWelcome]) => {
      localStorage.setItem('hew.settings.theme', theme)
      localStorage.setItem('hew.settings.showWelcome', showWelcome)
    },
    [light ? 'light' : 'dark', String(welcome)],
  )
  // Force the hidden <input type=file> path so File > Open is scriptable.
  await page.addInitScript(() => {
    delete window.showOpenFilePicker
    delete window.showSaveFilePicker
  })
  await page.addInitScript(`${overlaySrc}\ninstallCursorOverlay()`)
  await page.goto(BASE)
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 30_000 })
  await page.waitForTimeout(400)
  const dismiss = page.getByRole('button', { name: 'Dismiss the graphics-acceleration notice' })
  if (await dismiss.count()) await dismiss.dispatchEvent('click')
  await page.evaluate(() => window.__hew_test.setLengthUnit('cm'))
  if (cursor) await page.evaluate(() => window.__shotCursor.enable())
  return page
}

const wait = (page, ms = 300) => page.waitForTimeout(ms)
const h = (page, fn, arg) => page.evaluate(fn, arg)

/** World point -> page CSS pixels. */
async function w2s(page, p) {
  const s = await page.evaluate((p) => window.__hew_test.worldToScreen(p), p)
  const box = await page.locator('canvas').first().boundingBox()
  return { x: box.x + s.x, y: box.y + s.y }
}
async function moveTo(page, p, steps = 8) {
  const s = Array.isArray(p) ? await w2s(page, p) : p
  await page.mouse.move(s.x, s.y, { steps })
  await wait(page, 150)
  return s
}
async function click(page, p) {
  const s = await moveTo(page, p)
  await page.mouse.down()
  await page.mouse.up()
  await wait(page, 150)
  return s
}
async function key(page, k) {
  await page.keyboard.press(k)
  await wait(page, 120)
}
async function type(page, text) {
  await page.keyboard.type(text, { delay: 30 })
  await wait(page, 150)
}

async function shot(page, name) {
  await page.evaluate(() => window.__shotCursor?.refresh())
  await wait(page, 500)
  const tmp = `${OUT}/.${name}.capture.png`
  await page.screenshot({ path: tmp })
  const res = spawnSync('cwebp', ['-q', '90', '-m', '6', tmp, '-o', `${OUT}/${name}.webp`])
  rmSync(tmp, { force: true })
  if (res.status !== 0) throw new Error(`cwebp failed for ${name}: ${res.stderr}`)
  console.log(`captured ${name}.webp`)
}

/** Look at `target` from the default three-quarter direction at `dist` meters. */
async function cam(page, target, dist, dir = [3.5, -3, 2.5]) {
  const n = Math.hypot(...dir)
  const position = target.map((c, i) => c + (dir[i] / n) * dist)
  await h(page, (pose) => window.__hew_test.setCamera(pose), { position, target, up: [0, 0, 1], fovDeg: 45 })
  await wait(page, 300)
}

const scenes = {}

// --- shared scene pieces (meters; the app displays centimeters) -------------
const TRAY = { a: [0, 0, 0], b: [0.24, 0.14, 0] }
async function makeTray(page) {
  return h(page, ({ a, b }) => window.__hew_test.drawBox(a, b, 0.015), TRAY)
}
/** Pen cup r=3 x 9 with a 2.4 bore pushed 7.5 down, at (cx, cy). */
async function makeCup(page, cx, cy) {
  return h(
    page,
    ([cx, cy]) => {
      const t = window.__hew_test
      const c = t.drawCircle([cx, cy, 0], 0.03)
      const obj = t.extrudeRegion(c.sketch, c.region, 0.09)
      const top = t.pickFace([cx, cy, 1], [0, 0, -1])
      const bore = t.imprintCircleOnFace(top.object, top.face, [cx, cy, 0.09], 0.024)
      const top2 = t.pickFace([cx, cy, 1], [0, 0, -1])
      t.pushPull(top2.object, bore?.face ?? bore, -0.075)
      return obj
    },
    [cx, cy],
  )
}

scenes['default-hew-layout'] = async () => {
  const page = await boot()
  await moveTo(page, [0.9, 0, 0]) // a point on the red axis
  await wait(page, 300)
  await shot(page, 'default-hew-layout')
  await page.close()
}

scenes['tray-sketch'] = async () => {
  const page = await boot()
  await cam(page, [0.12, 0.07, 0], 0.62)
  await key(page, 'r')
  await click(page, [0, 0, 0])
  await moveTo(page, [0.24, 0.14, 0])
  await type(page, '24cm,14cm')
  await shot(page, 'tray-sketch')
  await page.close()
}

scenes['tray-pushpull'] = async () => {
  const page = await boot()
  await cam(page, [0.12, 0.07, 0], 0.62)
  await h(page, (t) => window.__hew_test.drawRectangle(...t), [[0, 0, 0], [0.24, 0.14, 0]])
  await key(page, 'p')
  await moveTo(page, [0.12, 0.07, 0])
  await page.mouse.down(); await page.mouse.up(); await wait(page, 200)
  const s = await w2s(page, [0.12, 0.07, 0.015])
  await moveTo(page, s)
  await type(page, '1.5cm')
  await shot(page, 'tray-pushpull')
  await page.close()
}


// --- the desk organizer, built from harness commands -------------------------
const BIN = { x0: 0.085, y0: 0.045, w: 0.07, d: 0.05, hgt: 0.06 }
const CUP_AT = [0.04, 0.09]

async function makeBin(page, x0, y0, z = 0) {
  return h(
    page,
    ([x0, y0, z, B]) => {
      const t = window.__hew_test
      const id = t.drawBox([x0, y0, z], [x0 + B.w, y0 + B.d, z], B.hgt)
      const top = t.pickFace([x0 + B.w / 2, y0 + B.d / 2, z + 1], [0, 0, -1])
      const inner = t.offsetFace(top.object, top.face, -0.007)
      t.pushPull(top.object, inner, -0.05)
      return id
    },
    [x0, y0, z, BIN],
  )
}
/** The cutter: r=2 cylinder along X on the bin's top edge at its low-Y side. */
async function makeCutter(page, x0, y0, z = 0) {
  return h(
    page,
    ([x0, y0, z, B]) => {
      const t = window.__hew_test
      const c = t.drawCircle([0, 0, 0], 0.02)
      const id = t.extrudeRegion(c.sketch, c.region, B.w)
      t.rotateObject(id, 90, [0, 1, 0])
      t.moveObject(id, x0, y0, z + B.hgt)
      return id
    },
    [x0, y0, z, BIN],
  )
}
/** Rotate/move helpers keep ids as strings; select one object. */
const select = (page, ids) => h(page, (ids) => window.__hew_test.selectObjects(ids), ids)
const park = async (page, x = 1439, y = 300) => {
  await page.mouse.move(x, y)
  await wait(page, 200)
}

scenes['tray-with-cup'] = async () => {
  const page = await boot()
  await cam(page, [0.1, -0.02, 0.03], 0.62)
  await makeTray(page)
  const cup = await makeCup(page, 0.04, -0.1)
  await select(page, [cup])
  await key(page, 'm')
  await click(page, [0.04, -0.07, 0]) // grab the cup's base rim
  await moveTo(page, [0.07, 0.06, 0.015])
  await moveTo(page, [0.075, 0.065, 0.015])
  await shot(page, 'tray-with-cup')
  await page.close()
}

scenes['bin-with-cutter'] = async () => {
  const page = await boot()
  await cam(page, [-0.06, 0.06, 0.03], 0.42, [4, -2, 2])
  await makeTray(page)
  await makeCup(page, 0.12, 0.07)
  const bx = -0.14
  const by = 0.04
  await makeBin(page, bx, by)
  await key(page, 'c')
  await moveTo(page, [bx + BIN.w, by + 0.01, BIN.hgt - 0.01])
  await key(page, 'Control')
  await click(page, [bx + BIN.w, by, BIN.hgt]) // upper corner of the right face
  await type(page, '2cm')
  await key(page, 'Enter')
  await key(page, 'p')
  await click(page, [bx + BIN.w, by + 0.003, BIN.hgt - 0.003])
  await moveTo(page, [bx + BIN.w - 0.03, by, BIN.hgt])
  await type(page, '-7cm')
  await shot(page, 'bin-with-cutter')
  await page.close()
}

scenes['bin-with-cutout'] = async () => {
  const page = await boot()
  await cam(page, [0.0, 0.0, 0.03], 0.62)
  await makeTray(page)
  await makeCup(page, 0.12, 0.07)
  const bin = await makeBin(page, 0.0, -0.1)
  const cutter = await makeCutter(page, 0.0, -0.1)
  const res = await h(page, ([a, b]) => window.__hew_test.boolean(1, a, b), [bin, cutter])
  await select(page, [res])
  await park(page, 700, 900)
  await shot(page, 'bin-with-cutout')
  await page.close()
}

scenes['command-palette-sub'] = async () => {
  const page = await boot()
  await cam(page, [0.0, 0.0, 0.03], 0.62)
  await makeTray(page)
  const bin = await makeBin(page, 0.0, -0.1)
  const cutter = await makeCutter(page, 0.0, -0.1)
  await select(page, [bin, cutter])
  await key(page, 'Meta+/')
  await type(page, 'sub')
  const row = page.getByText('Subtract', { exact: true }).first()
  const b = await row.boundingBox()
  await page.mouse.move(b.x + 400, b.y + 6, { steps: 5 })
  await shot(page, 'command-palette-sub')
  await page.close()
}

async function organizer(page, { named = false, camTarget = [0.12, 0.07, 0.03] } = {}) {
  await cam(page, camTarget, 0.52)
  const tray = await makeTray(page)
  const cup = await makeCup(page, CUP_AT[0], CUP_AT[1])
  await h(page, ([c]) => window.__hew_test.moveObject(c, 0, 0, 0.015), [cup])
  const bin0 = await makeBin(page, BIN.x0, BIN.y0)
  const cutter = await makeCutter(page, BIN.x0, BIN.y0)
  const bin = await h(page, ([a, b]) => window.__hew_test.boolean(1, a, b), [bin0, cutter])
  await h(page, ([c]) => window.__hew_test.moveObject(c, 0, 0, 0.015), [bin])
  // the stand: wedge profile on the ground, extruded 5, stood up
  const stand = await h(page, () => {
    const t = window.__hew_test
    const x0 = 0.17, y0 = 0.045
    const pts = [[x0, y0 + 0.0, 0], [x0 + 0.06, y0, 0], [x0 + 0.06, y0 + 0.01, 0], [x0 + 0.05, y0 + 0.01, 0], [x0, y0 + 0.08, 0], [x0, y0, 0]]
    const r = t.drawLineChain(pts)
    const id = t.extrudeRegion(r.sketch, r.regions[0], 0.05)
    t.rotateObject(id, 90, [1, 0, 0])
    t.moveObject(id, 0, 0.095, -0.045 + 0.015)
    return id
  })
  return { tray, cup, bin, stand }
}

scenes['organizer-set'] = async () => {
  const page = await boot()
  const o = await organizer(page)
  await park(page, 700, 899)
  await shot(page, 'organizer-set')
  await page.close()
}


// --- the phone stand profile (6 x 8 rectangle beside the tray) ---------------
const ST = { x0: 0.12, y0: -0.15 }
async function standBase(page, { sketch = true, dir = [0.3, -3, 2.5], dist = 0.34 } = {}) {
  await cam(page, [0.15, -0.09, 0.0], dist, dir)
  await makeTray(page)
  await makeCup(page, 0.04, 0.09)
  await h(page, () => {
    const t = window.__hew_test
    const objs = t.getObjectIds()
    t.moveObject(objs[1], 0, 0, 0.015)
  })
  const bin = await makeBin(page, BIN.x0, BIN.y0)
  const cutter = await makeCutter(page, BIN.x0, BIN.y0)
  const res = await h(page, ([a, b]) => window.__hew_test.boolean(1, a, b), [bin, cutter])
  await h(page, ([c]) => window.__hew_test.moveObject(c, 0, 0, 0.015), [res])
  if (sketch) await h(page, ({ x0, y0 }) => {
    const t = window.__hew_test
    t.drawRectangle([x0, y0, 0], [x0 + 0.06, y0 + 0.08, 0])
    t.addGuideLine(x0, y0 + 0.01, 0, 1, 0, 0)
    t.addGuideLine(x0 + 0.05, y0, 0, 0, 1, 0)
  }, ST)
}
const P = (dx, dy) => [ST.x0 + dx, ST.y0 + dy, 0]

async function drawProfileLines(page) {
  await key(page, 'l')
  await click(page, P(0, 0.08)) // back top corner
  await click(page, P(0.05, 0.01)) // guide intersection
  await moveTo(page, P(0.06, 0.01))
}

scenes['stand-sketch-with-lines'] = async () => {
  const page = await boot()
  await standBase(page)
  await drawProfileLines(page)
  await type(page, '1cm')
  await shot(page, 'stand-sketch-with-lines')
  await page.close()
}

scenes['stand-sketch-editing'] = async () => {
  const page = await boot()
  await standBase(page)
  await drawProfileLines(page)
  await click(page, P(0.06, 0.01))
  await key(page, 'Escape')
  await key(page, 'Space')
  await click(page, P(0.03, 0.08)) // top line
  await key(page, 'Delete')
  await click(page, P(0.06, 0.045)) // short right piece above the diagonal
  await moveTo(page, P(0.06, 0.047))
  await shot(page, 'stand-sketch-editing')
  await page.close()
}

scenes['stand-solid-rotating'] = async () => {
  const page = await boot()
  await standBase(page, { sketch: false, dir: [-2.5, -3, 2.2], dist: 0.4 })
  await h(page, ({ x0, y0 }) => {
    const t = window.__hew_test
    const pts = [[x0, y0, 0], [x0 + 0.06, y0, 0], [x0 + 0.06, y0 + 0.01, 0], [x0 + 0.05, y0 + 0.01, 0], [x0, y0 + 0.08, 0], [x0, y0, 0]]
    const r = t.drawLineChain(pts)
    const id = t.extrudeRegion(r.sketch, r.regions[0], 0.05)
    t.selectObjects([id])
  }, ST)
  await key(page, 'q')
  await moveTo(page, [ST.x0, ST.y0 + 0.04, 0.025]) // hover the left face
  await key(page, 'Shift') // lock the rotation to the red axis
  await click(page, [ST.x0, ST.y0 + 0.08, 0]) // pivot: lower corner of the left face
  await click(page, [ST.x0, ST.y0 + 0.08, 0.05]) // reference: top of the same face
  await moveTo(page, [ST.x0, ST.y0 + 0.04, 0.045])
  await type(page, '-90')
  await shot(page, 'stand-solid-rotating')
  await page.close()
}


// --- opening a saved file through File > Open (so the title bar shows its name)
async function openFile(page, path) {
  const chooser = page.waitForEvent('filechooser', { timeout: 8000 })
  await page.getByTestId('menu-bar').getByText('File', { exact: true }).click()
  await page.getByText('Open…', { exact: true }).first().click()
  const fc = await chooser
  await fc.setFiles(path)
  await page.waitForFunction(() => window.__hew_test.getObjectCount() > 0, null, { timeout: 15000 })
  await wait(page, 600)
}
const DESK = resolve(process.env.HOME, 'Documents/Hew/desk-set.hew')
const CAFE = resolve(APP_DIR, 'public/samples/cafe-table.hew')


/** Same as File > Open but without a title: load saved bytes straight in. */
async function loadSample(page, path) {
  const bytes = [...readFileSync(path)]
  await h(page, (b) => window.__hew_test.load(b), bytes)
  await wait(page, 800)
}


/** Open desk-set.hew, make the title read "Edited", and pick the whole group with a real click. */
async function deskSession(page, { expand = true } = {}) {
  await openFile(page, DESK)
  await h(page, () => {
    const t = window.__hew_test
    for (const id of t.getObjectIds()) if (t.getNodeName('object', id) === 'Pen stand') t.setNodeName('object', id, 'Phone stand')
    const id = t.getObjectIds()[0]
    const parent = t.getNodeParent('object', id)
    if (parent) {
      const name = t.getNodeName('group', parent)
      t.setNodeName('group', parent, 'tmp')
      t.setNodeName('group', parent, name)
    }
  })
  await click(page, [0.105, 0.07, 0.075]) // a click on the bin picks the whole group
  if (expand) {
    await page.mouse.click(1161, 400) // the Outliner disclosure triangle on the group row
    await wait(page, 300)
  }
  await moveTo(page, { x: 100, y: 800 })
}

scenes['organizer-organized'] = async () => {
  const page = await boot()
  await deskSession(page)
  await shot(page, 'organizer-organized')
  await page.close()
}

scenes['export-dialog'] = async () => {
  const page = await boot()
  await deskSession(page)
  await page.getByTestId('menu-bar').getByText('File', { exact: true }).click()
  await page.getByText('Export…', { exact: true }).first().click()
  await wait(page, 500)
  await page.getByRole('combobox').first().selectOption({ label: 'STL binary (.stl) — Z-up, millimeters, for 3D printing' })
  await wait(page, 300)
  await park(page, 1439, 150)
  await shot(page, 'export-dialog')
  await page.close()
}



async function clickDieFace(page, id) {
  const b = await page.locator(`[data-testid="view-die-svg"] [data-face="${id}"]`).boundingBox()
  await page.mouse.move(b.x + b.width / 2, b.y + b.height / 2, { steps: 3 })
  await page.mouse.down()
  await page.mouse.up()
  await page.mouse.move(700, 890)
  await wait(page, 1200)
}
async function addSceneNamed(page, name) {
  await page.getByRole('button', { name: /Add Scene/ }).first().click()
  await wait(page, 300)
  await page.keyboard.type(name)
  await key(page, 'Enter')
  await wait(page, 400)
}

scenes['scenes-tray'] = async () => {
  const page = await boot({ light: true })
  await loadSample(page, CAFE)
  const home = await h(page, () => window.__hew_test.getCameraState())
  await addSceneNamed(page, 'Assembled')
  await clickDieFace(page, 'top')
  await addSceneNamed(page, 'Top-down')
  await h(page, () =>
    window.__hew_test.setCamera({ position: [0.55, -0.55, 0.3], target: [0, 0, 0.3], up: [0, 0, 1], fovDeg: 45 }),
  )
  await wait(page, 500)
  await addSceneNamed(page, 'Leg joint')
  await page.getByRole('button', { name: /Expand Scene details/ }).last().click()
  await wait(page, 400)
  await page.getByPlaceholder(/Add a description/).fill('Close-up on the stretcher joint under the top.')
  await page.getByText(/^object info$/i).click() // collapse it, as in the original shot
  await page.getByText(/^outliner$/i).click() // and the Outliner, so the whole Scene card fits
  await wait(page, 400)
  await h(page, (c) => window.__hew_test.setCamera({ position: c.eye, target: c.target, up: c.up, fovDeg: c.fovDeg }), home)
  await wait(page, 600)
  await park(page, 700, 899)
  await shot(page, 'scenes-tray')
  await page.close()
}


// --- the kitchen for the selection-path chapter (meters) ---------------------
async function buildKitchen(page) {
  await h(page, () => {
    const t = window.__hew_test
    const mat = (n, r, g, b) => t.addMaterial(n, r, g, b, 255)
    const wall = mat('Wall', 128, 128, 120)
    const floorM = mat('Floor', 112, 84, 52)
    const doorM = mat('Door', 74, 90, 62)
    const body = mat('Body', 140, 138, 130)
    const named = (kind, id, name) => (t.setNodeName(kind, id, name), id)
    const box = (a, b, hgt, name, m) => {
      const id = t.drawBox([a[0], a[1], 0], [b[0], b[1], 0], hgt)
      if (a[2]) t.moveObject(id, 0, 0, a[2])
      if (m) t.paintObject(id, m)
      return named('object', id, name)
    }
    const group = (kind, members, name) =>
      named('group', t.groupNodes(members.map((id) => ({ kind, id }))), name)
    const refs = (arr) => arr.map((g) => ({ kind: 'group', id: g }))

    box([-0.2, -0.1, 0], [4.2, 3.2, 0], 0.05, 'Floor', floorM)
    box([-0.2, -0.1, 0.05], [4.2, 0, 0], 2.5, 'Back Wall', wall)

    const cabinets = []
    for (let i = 0; i < 3; i++) {
      const x = 0.2 + i * 0.62
      const b = box([x, 0, 0.05], [x + 0.6, 0.6, 0.05], 0.75, 'Body', body)
      const d = box([x + 0.03, 0.6, 0.1], [x + 0.57, 0.62, 0.1], 0.66, 'Door', doorM)
      cabinets.push(group('object', [b, d].map((o) => o), `Cabinet ${i + 1}`))
    }
    const top = box([0.15, -0.0, 0.8], [2.1, 0.65, 0.8], 0.05, 'Countertop', body)
    const baseNodes = [...refs(cabinets), { kind: 'object', id: top }]
    const base = named('group', t.groupNodes(baseNodes), 'Base Cabinets')

    const uppers = []
    for (let i = 0; i < 2; i++) {
      const x = 0.3 + i * 0.85
      const b = box([x, 0, 1.55], [x + 0.8, 0.35, 1.55], 0.7, 'Body', body)
      const d = box([x + 0.03, 0.35, 1.6], [x + 0.77, 0.37, 1.6], 0.6, 'Door', doorM)
      uppers.push(group('object', [b, d], `Upper ${i + 1}`))
    }
    const upper = group('group', uppers, 'Upper Cabinets')
    const kitchen = named('group', t.groupNodes([{ kind: 'group', id: base }, { kind: 'group', id: upper }]), 'Kitchen')

    const ib = box([2.7, 1.5, 0.05], [3.6, 2.1, 0.05], 0.85, 'Island Base', body)
    const it = box([2.65, 1.45, 0.9], [3.65, 2.15, 0.9], 0.05, 'Island Top', body)
    const id = box([2.75, 1.5 - 0.02, 0.1], [3.55, 1.5, 0.1], 0.7, 'Island Door', doorM)
    group('object', [ib, it, id], 'Island')
    return kitchen
  })
}

scenes['selection-path'] = async () => {
  const page = await boot()
  await buildKitchen(page)
  await h(page, () =>
    window.__hew_test.setCamera({ position: [5.4, 7.0, 3.8], target: [1.9, 0.9, 0.8], up: [0, 0, 1], fovDeg: 45 }),
  )
  await wait(page, 400)
  await click(page, [1.1, 0.62, 0.45]) // one click on Cabinet 2's door
  await moveTo(page, { x: 100, y: 800 })
  await shot(page, 'selection-path')
  await page.close()
}


scenes['print-layout'] = async () => {
  const page = await boot({ welcome: true })
  await page.getByRole('dialog', { name: 'Welcome to Hew' }).waitFor()
  await page.getByText(/Table/).first().click()
  await wait(page, 1500)
  await h(page, () => window.__hew_test.setLengthUnit('mm')) // the original session ran in millimeters
  await page.getByTestId('menu-bar').getByText('File', { exact: true }).click()
  await page.getByText('Print…', { exact: true }).first().click()
  await wait(page, 1500)
  await page.getByRole('button', { name: 'Scaled', exact: true }).click()
  await wait(page, 500)
  const selects = page.getByRole('dialog').getByRole('combobox')
  await selects.nth(0).selectOption({ label: 'Top' })
  await wait(page, 400)
  await selects.nth(1).selectOption({ label: '1:2 — 1 mm = 2 mm' })
  await page.getByRole('button', { name: 'Line art', exact: true }).click()
  await wait(page, 1500)
  await park(page, 700, 899)
  await shot(page, 'print-layout')
  await page.close()
}

const names = Object.keys(scenes).filter((n) => !ONLY.length || ONLY.includes(n))
for (const n of names) {
  console.log(`-- ${n}`)
  await scenes[n]()
}
await browser.close()

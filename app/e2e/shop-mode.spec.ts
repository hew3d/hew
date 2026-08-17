import { test, expect, type Page } from '@playwright/test'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { buildViewProjection, worldToPagePixel, PINNED_CAMERA, type CameraParams } from './helpers/projectWorldToScreen'
import { settleFrame } from './helpers/render'
import { HINT_STORAGE_KEYS, HINT_TEST_SUPPRESS_KEY } from '../src/shop/hints'
import { encrypt, generateKey, toBase64Url } from '../src/io/shareCrypto'

/**
 * Shop Mode E2E — `#shop`'s boot, tap-to-inspect, the Parts sheet, and
 * long-press isolate, under touch emulation on a phone-sized viewport (the
 * device shape Shop Mode is actually built for).
 *
 * Fixture: rather than a static `.hew` file, the editor's own harness
 * (`window.__hew_test`) draws three boxes and `save()`s them in-process —
 * `web-smoke.spec.ts`'s exact drawBox+save pattern, reused instead of
 * adding a new binary fixture under `e2e/fixtures/`. Those bytes are then
 * handed to Shop Mode's OWN harness (`window.__hew_shop_test`,
 * `src/shop/testHarness.ts`) via its `load()` — the harness's load
 * mechanism the task calls for, deliberately a tiny sibling of the editor
 * harness rather than a reuse of it (see that module's doc comment for why:
 * Shop Mode issues zero kernel transactions, so most of the editor
 * harness's surface would be reachable here but not from Shop Mode's own
 * UI).
 *
 * Touch note: Playwright's touch APIs (`Locator.tap`/`page.touchscreen`)
 * cover single taps but have no press-and-HOLD primitive, so the long-press
 * isolate step (part d) falls back to `page.mouse` down/wait/up — real
 * `PointerEvent`s, just with `pointerType: 'mouse'` rather than 'touch'.
 * `ShopApp`/`PartsSheet`'s own long-press timers don't discriminate by
 * pointer type, so this exercises the identical code path a real touch
 * hold would. Every other interaction below uses genuine touch dispatch
 * (`hasTouch: true` + `Locator.tap()`/`page.touchscreen.tap()`).
 *
 * No new visual goldens — every assertion here is DOM/state (row text,
 * `aria-label`s, element visibility), matching the "assert via DOM when no
 * scene-state hook exists" fallback.
 */

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
    __hew_shop_test?: import('../src/shop/testHarness').ShopTestHarness
  }
}

type CameraPose = { position: [number, number, number]; target: [number, number, number]; fovDeg: number }

/**
 * Asserts two camera poses are the SAME, up to floating-point noise —
 * NOT `toEqual`. `OrbitControls.update()` recomputes `camera.position`
 * from its internal spherical state every frame it runs (whenever
 * `controls.enabled` is true, which it legitimately is for a chunk of the
 * Tape Measure loupe's own gesture — the `armed` phase, before a hold
 * engages, and again after release), and that recomputation is not
 * perfectly bit-stable call to call — empirically, a static hold with zero
 * pointer movement still drifts by ~1e-13 in each component. A REAL orbit
 * (a drag of tens of pixels) moves the camera by many orders of magnitude
 * more, so a loose tolerance here still catches the regression this guards
 * against without failing on ambient float noise.
 */
function expectCameraUnchanged(actual: CameraPose, expected: CameraPose): void {
  for (let i = 0; i < 3; i++) {
    expect(actual.position[i]).toBeCloseTo(expected.position[i], 6)
    expect(actual.target[i]).toBeCloseTo(expected.target[i], 6)
  }
  expect(actual.fovDeg).toBeCloseTo(expected.fovDeg, 6)
}

// Phone-sized viewport + real touch dispatch — the device shape Shop Mode's
// auto-detect heuristic (shellMode.ts) targets, and what makes the Parts
// sheet's drag-to-detent gesture worth exercising for real.
test.use({
  viewport: { width: 390, height: 844 },
  hasTouch: true,
  isMobile: true,
})

/** Build the three-box fixture through the EDITOR's harness (a separate
 *  page load — the editor and Shop Mode are two different render roots,
 *  main.tsx's module doc), and return its saved `.hew` bytes. Object 1 is
 *  centered near the world origin so `PINNED_CAMERA` (which targets the
 *  origin) frames it reliably for the canvas-tap step. */
async function buildFixtureBytes(page: Page): Promise<number[]> {
  // Force the EDITOR shell for THIS navigation — main.tsx's own auto-detect
  // heuristic (shellMode.ts: coarse pointer + a <600px viewport dimension)
  // would otherwise route this phone-emulated context straight into Shop
  // Mode, which has no editor harness to draw fixture geometry with. The
  // override is read once at boot (main.tsx), so it must be in localStorage
  // BEFORE the page's own scripts run — addInitScript, not evaluate after
  // goto. `#shop`'s hash check (bootShopModeWith below) outranks this
  // override regardless, so leaving it set doesn't affect that navigation.
  await page.addInitScript(() => localStorage.setItem('hew:shellMode', 'editor'))
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
  return page.evaluate(() => {
    const h = window.__hew_test!
    // Object 1 sits alone near the origin, closest to PINNED_CAMERA along
    // its approach from (8,6,8) — unoccluded. Objects 2/3 are well BEHIND
    // it (negative X/Y, further from the camera) rather than further along
    // +X: the camera's own position is offset in +X/+Y, so +X-spread boxes
    // would line up almost directly along its line of sight to Object 1
    // and occlude/alias it (the bug this comment is here to prevent
    // reintroducing — the first version of this fixture did exactly that
    // and the canvas tap hit the wrong object).
    h.drawBox([0, 0, 0], [1, 1, 0], 1) // Object 1 — near the origin
    h.drawBox([-5, -4, 0], [-4, -3, 0], 1) // Object 2 — well behind it
    h.drawBox([-10, -4, 0], [-9, -3, 0], 1) // Object 3 — further still
    return h.save()
  })
}

/** Single-box fixture that also returns the object's own kernel id (a
 *  string) — `buildFixtureBytes` above only returns bytes, which is enough
 *  for every DOM-observable assertion, but the read-only-contract
 *  regression test below needs the id to query `getObjectBounds` before and
 *  after the drag. Same near-origin placement as Object 1 in the 3-box
 *  fixture, for the same reason (unoccluded from `PINNED_CAMERA`). */
async function buildSingleBoxFixture(page: Page): Promise<{ bytes: number[]; objectId: string }> {
  await page.addInitScript(() => localStorage.setItem('hew:shellMode', 'editor'))
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
  return page.evaluate(() => {
    const h = window.__hew_test!
    const objectId = h.drawBox([0, 0, 0], [1, 1, 0], 1)
    return { bytes: h.save(), objectId }
  })
}

/** Single-box-inside-a-GROUP fixture, for the double-tap-opens-a-session
 *  regression test below (adversarial-review finding 2) — same near-origin
 *  placement/camera framing as `buildSingleBoxFixture`, just wrapped in one
 *  `groupNodes` call so a tap on it resolves to `kind: 'group'`
 *  (`Viewport.tsx`'s `onDoubleClick`), the branch that opens a group EDIT
 *  SESSION in the editor. */
async function buildGroupFixture(page: Page): Promise<{ bytes: number[] }> {
  await page.addInitScript(() => localStorage.setItem('hew:shellMode', 'editor'))
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
  return page.evaluate(() => {
    const h = window.__hew_test!
    const box = h.drawBox([0, 0, 0], [1, 1, 0], 1)
    h.groupNodes([{ kind: 'object', id: box }])
    return { bytes: h.save() }
  })
}

/** Drag the Parts sheet's grab handle by `deltaY` page pixels (negative =
 *  up = taller). Real touch has no drag primitive in Playwright (module
 *  doc), so this uses `page.mouse` — still real `PointerEvent`s;
 *  `PartsSheet`'s drag handler doesn't discriminate by `pointerType`. */
async function dragSheetHandle(page: Page, deltaY: number): Promise<void> {
  const handle = page.getByTestId('parts-sheet-handle')
  const handleBox = await handle.boundingBox()
  if (handleBox === null) throw new Error('parts sheet handle has no bounding box')
  const startX = handleBox.x + handleBox.width / 2
  const startY = handleBox.y + handleBox.height / 2
  await page.mouse.move(startX, startY)
  await page.mouse.down()
  await page.mouse.move(startX, startY + deltaY, { steps: 10 })
  await page.mouse.up()
}

/** Boot `#shop`, wait for its harness, and load `bytes` through it.
 *
 * `about:blank` in between is load-bearing: a browser treats a URL that
 * differs ONLY by fragment from the CURRENT document as a same-document
 * history navigation (a plain `hashchange`, same as clicking an in-page
 * anchor) rather than a real reload — so going straight from `/`
 * (`buildFixtureBytes` above) to `/#shop` would never re-run `main.tsx`,
 * leaving the EDITOR mounted with `#shop` merely sitting unread in the
 * address bar. Detouring through a different document first forces a
 * genuine navigation.
 *
 * `suppressHints` (default `true`): sets `hints.ts`'s
 * `HINT_TEST_SUPPRESS_KEY` before navigation, so a fresh, flag-free
 * install (which every spec in this file boots into) never races hint (a)
 * — a pulsing dot + "Tap a part for its size" tag — over these specs'
 * canvas taps and DOM assertions. It's `pointer-events: none` so it can
 * never actually SWALLOW a tap, but a couple of specs below assert on
 * broader visible content a floating tag could still perturb, so every
 * spec EXCEPT the "gesture hints" ones below leaves this at its default.
 * Those are the one place `suppressHints: false` is passed explicitly —
 * matching a real fresh install (hints ON, this key simply never written)
 * rather than a special test-only "hints enabled" mode. */
async function bootShopModeWith(page: Page, bytes: number[], opts: { suppressHints?: boolean } = {}): Promise<void> {
  const { suppressHints = true } = opts
  // Pin Meters — Shop Mode's own locale-based default (`localeUnits.ts`)
  // would otherwise seed Architectural for a US-locale test runner (CI's
  // default Chromium locale, unset here), making every dims/format
  // assertion below environment-dependent. Registered before the
  // navigations below so it's in place before ShopApp's boot effect runs.
  await page.addInitScript(() => localStorage.setItem('hew.settings.lengthUnit', 'm'))
  if (suppressHints) {
    await page.addInitScript((key) => localStorage.setItem(key, '1'), HINT_TEST_SUPPRESS_KEY)
  }
  await page.goto('about:blank')
  await page.goto('/#shop')
  await page.waitForFunction(() => window.__hew_shop_test?.isReady() === true, null, { timeout: 15_000 })
  const loaded = await page.evaluate((arr) => window.__hew_shop_test!.load(arr), bytes)
  expect(loaded).toBe(true)
}

test('#shop boots into Shop Mode and loads the fixture', async ({ page }) => {
  const bytes = await buildFixtureBytes(page)
  await bootShopModeWith(page, bytes)

  // The top strip reflects the loaded document (the empty state's "Open a
  // model…" button is gone) — proof the harness's load ran through
  // ShopApp's real open-apply path (applyOpenedBytes), not a bare
  // scene.load with no UI reset.
  await expect(page.getByText('Fixture.hew')).toBeVisible()
  await expect(page.getByRole('button', { name: /^open a model…$/i })).not.toBeVisible()
  // Shop Mode's 3-tool switcher (Select/Orbit/Tape Measure) is the chrome
  // tell that this is the Shop Mode shell, not the full editor.
  await expect(page.getByRole('button', { name: /^orbit$/i })).toBeVisible()
})

test('tapping a part shows the inspect card with its dimensions', async ({ page }) => {
  const bytes = await buildFixtureBytes(page)
  await bootShopModeWith(page, bytes)

  // Regression (playtest round 1): with a document open, the Parts sheet's
  // full-width peek bar sits at a higher z-index and used to swallow both
  // HUD clusters whole — only Zoom Extents poked above it, and the tool
  // switcher and AR chip were unreachable. A real .click() (not just
  // toBeVisible, which ignores occlusion) proves the chips now clear the
  // peek: Playwright's actionability check fails if the sheet intercepts
  // the pointer. Round-trips back to Select so the tap below still inspects.
  await page.getByRole('button', { name: 'Orbit' }).click()
  await page.getByRole('button', { name: 'Select', exact: true }).click()

  // Pin the camera (Shop Mode's own harness `setCamera`, delegating to the
  // SAME ViewportApi.setCamera the editor harness uses) so a known world
  // point projects to a reliable canvas pixel — docs/DEVELOPMENT.md's
  // "pixel interaction" strategy.
  await page.evaluate((cam) => window.__hew_shop_test!.setCamera(cam), {
    position: [PINNED_CAMERA.position.x, PINNED_CAMERA.position.y, PINNED_CAMERA.position.z],
    target: [PINNED_CAMERA.target.x, PINNED_CAMERA.target.y, PINNED_CAMERA.target.z],
    up: [PINNED_CAMERA.up.x, PINNED_CAMERA.up.y, PINNED_CAMERA.up.z],
    fovDeg: PINNED_CAMERA.fovDeg,
  })
  await settleFrame(page)

  const canvasBox = await page.locator('canvas').first().boundingBox()
  if (canvasBox === null) throw new Error('viewport canvas has no bounding box')
  const rect = { left: canvasBox.x, top: canvasBox.y, width: canvasBox.width, height: canvasBox.height }
  const vp = buildViewProjection(PINNED_CAMERA, rect.width / rect.height)
  // Object 1's top-face center ([0,1]x[0,1] at z=1) — on its rendered
  // geometry and unoccluded from the pinned iso-ish angle.
  const px = worldToPagePixel({ x: 0.5, y: 0.5, z: 1 }, vp, rect)
  if (px === null) throw new Error('Object 1 does not project onto the canvas under PINNED_CAMERA')

  // A genuine touch tap (hasTouch: true from test.use above).
  await page.touchscreen.tap(px.x, px.y)

  const card = page.getByRole('status')
  await expect(card).toBeVisible()
  await expect(card).toContainText('Object 1')
  // The compact dimensions readout — axis labels are literal in InspectCard,
  // so their presence proves a dimensions line rendered (exact formatted
  // length is units-setting-dependent, not asserted here). L/W/H, not X/Y/Z
  // (design_handoff_shop_mode/README.md §3 — Shop Mode's last X/Y/Z
  // remnant, completed this wave).
  await expect(card).toContainText('L')
  await expect(card).toContainText('W')
  await expect(card).toContainText('H')
})

test('tapping a part\'s edge shows the edge card with the owning part\'s name and its length', async ({ page }) => {
  const bytes = await buildFixtureBytes(page)
  await bootShopModeWith(page, bytes)

  await page.getByRole('button', { name: 'Orbit' }).click()
  await page.getByRole('button', { name: 'Select', exact: true }).click()

  await page.evaluate((cam) => window.__hew_shop_test!.setCamera(cam), {
    position: [PINNED_CAMERA.position.x, PINNED_CAMERA.position.y, PINNED_CAMERA.position.z],
    target: [PINNED_CAMERA.target.x, PINNED_CAMERA.target.y, PINNED_CAMERA.target.z],
    up: [PINNED_CAMERA.up.x, PINNED_CAMERA.up.y, PINNED_CAMERA.up.z],
    fovDeg: PINNED_CAMERA.fovDeg,
  })
  await settleFrame(page)

  const canvasBox = await page.locator('canvas').first().boundingBox()
  if (canvasBox === null) throw new Error('viewport canvas has no bounding box')
  const rect = { left: canvasBox.x, top: canvasBox.y, width: canvasBox.width, height: canvasBox.height }
  const vp = buildViewProjection(PINNED_CAMERA, rect.width / rect.height)
  // The midpoint of Object 1's top-face edge running from (0,0,1) to (1,0,1)
  // — right on the boundary the widened touch aperture still resolves as an
  // EDGE hit rather than the face behind it (inspect.ts's module doc: a
  // near-edge tap keeps edge precedence).
  const px = worldToPagePixel({ x: 0.5, y: 0, z: 1 }, vp, rect)
  if (px === null) throw new Error("Object 1's edge does not project onto the canvas under PINNED_CAMERA")

  await page.touchscreen.tap(px.x, px.y)

  const card = page.getByRole('status')
  await expect(card).toBeVisible()
  // Owning part's name + the "edge" chip — never a bare "Edge" title
  // (design_handoff_shop_mode/README.md §3).
  await expect(card).toContainText('Object 1')
  await expect(card).toContainText('edge')
})

// Shop-mode playtest finding 1 (CRITICAL, contract violation): the editor
// Select tool's press-drag-on-a-part gesture is drag-to-MOVE. Shop Mode is
// read-only end to end (module doc) — `Viewport`'s `readOnly` prop must
// make that gesture never arm, so a press-drag on a part behaves like a
// press-drag on empty space (this file's own module doc explains the touch
// mechanics — camera orbit, not a rubber-band marquee, since OrbitControls'
// own `touches.ONE` binding for Select is never nulled). Asserted against
// real kernel state (`getObjectBounds`, testHarness.ts's own doc on why
// this one query exists), not pixels or DOM text — a translation wouldn't
// change InspectCard's L/W/H dims either way, so only the object's actual
// world-space bounds prove nothing moved.
test('CONTRACT: a press-drag on a part never moves it (readOnly disarms drag-to-move)', async ({ page }) => {
  const { bytes, objectId } = await buildSingleBoxFixture(page)
  await bootShopModeWith(page, bytes)

  await page.evaluate((cam) => window.__hew_shop_test!.setCamera(cam), {
    position: [PINNED_CAMERA.position.x, PINNED_CAMERA.position.y, PINNED_CAMERA.position.z],
    target: [PINNED_CAMERA.target.x, PINNED_CAMERA.target.y, PINNED_CAMERA.target.z],
    up: [PINNED_CAMERA.up.x, PINNED_CAMERA.up.y, PINNED_CAMERA.up.z],
    fovDeg: PINNED_CAMERA.fovDeg,
  })
  await settleFrame(page)

  const canvasBox = await page.locator('canvas').first().boundingBox()
  if (canvasBox === null) throw new Error('viewport canvas has no bounding box')
  const rect = { left: canvasBox.x, top: canvasBox.y, width: canvasBox.width, height: canvasBox.height }
  const vp = buildViewProjection(PINNED_CAMERA, rect.width / rect.height)
  const px = worldToPagePixel({ x: 0.5, y: 0.5, z: 1 }, vp, rect)
  if (px === null) throw new Error('the fixture object does not project onto the canvas under PINNED_CAMERA')

  const before = await page.evaluate((id) => window.__hew_shop_test!.getObjectBounds(id), objectId)

  // A real press-drag ON the part, well past any click/drag threshold
  // (`dragMove.ts`'s `DRAG_MOVE_THRESHOLD_PX` is 5) — `page.mouse` (real
  // PointerEvents; Viewport's drag-arm logic doesn't discriminate by
  // pointerType, this file's own `dragSheetHandle` doc comment notes the
  // same for the Parts-sheet handle) rather than `page.touchscreen` (no
  // drag primitive in Playwright's touch API — module doc).
  await page.mouse.move(px.x, px.y)
  await page.mouse.down()
  await page.mouse.move(px.x + 60, px.y + 45, { steps: 12 })
  await page.mouse.up()
  await settleFrame(page)

  const after = await page.evaluate((id) => window.__hew_shop_test!.getObjectBounds(id), objectId)
  expect(after).toEqual(before)
})

// Adversarial-review finding 1 (CRITICAL): the prior read-only fix above
// only gated the Select tool's OWN drag-to-move arm — it left the keyboard
// tool-switch shortcuts (`onKeyDown`) and `switchToolRef`'s switch statement
// itself reachable, so pressing a tool-switch key (e.g. 'm' for Move) could
// still swap the active tool out from under Shop Mode's 3-tool chrome and
// arm a REAL drag-to-move on the next press. Fixed at the choke point
// (`switchToolRef`'s own top-of-function allowlist refusal) rather than
// re-gating each caller — this test presses the shortcut THEN repeats the
// exact drag the CONTRACT test above proves is inert, so a regression in
// either the keyboard path or the drag-arm gate itself would fail it.
test('CONTRACT: a keyboard tool-switch shortcut is refused under readOnly — Move never arms, the UI stays on Select', async ({ page }) => {
  const { bytes, objectId } = await buildSingleBoxFixture(page)
  await bootShopModeWith(page, bytes)

  await page.evaluate((cam) => window.__hew_shop_test!.setCamera(cam), {
    position: [PINNED_CAMERA.position.x, PINNED_CAMERA.position.y, PINNED_CAMERA.position.z],
    target: [PINNED_CAMERA.target.x, PINNED_CAMERA.target.y, PINNED_CAMERA.target.z],
    up: [PINNED_CAMERA.up.x, PINNED_CAMERA.up.y, PINNED_CAMERA.up.z],
    fovDeg: PINNED_CAMERA.fovDeg,
  })
  await settleFrame(page)

  const selectButton = page.getByRole('button', { name: 'Select', exact: true })
  const selectBgBefore = await selectButton.evaluate((el) => getComputedStyle(el).backgroundColor)

  // 'm' is Move's shortcut in the editor (Viewport.tsx's onKeyDown) — Shop
  // Mode's 3-tool registry has no Move segment at all, so this can only
  // reach the kernel through the keyboard path this test exists to close.
  await page.locator('canvas').first().focus()
  await page.keyboard.press('m')
  await settleFrame(page)

  // The dock's tool switcher still shows Select as active — proof the
  // keyboard shortcut never reached `onInternalToolChange`/`setActiveTool`
  // (ShopApp.tsx's own defense-in-depth allowlist would also refuse it even
  // if it had).
  const selectBgAfter = await selectButton.evaluate((el) => getComputedStyle(el).backgroundColor)
  expect(selectBgAfter).toBe(selectBgBefore)
  expect(selectBgAfter).not.toBe('rgba(0, 0, 0, 0)')

  const canvasBox = await page.locator('canvas').first().boundingBox()
  if (canvasBox === null) throw new Error('viewport canvas has no bounding box')
  const rect = { left: canvasBox.x, top: canvasBox.y, width: canvasBox.width, height: canvasBox.height }
  const vp = buildViewProjection(PINNED_CAMERA, rect.width / rect.height)
  const px = worldToPagePixel({ x: 0.5, y: 0.5, z: 1 }, vp, rect)
  if (px === null) throw new Error('the fixture object does not project onto the canvas under PINNED_CAMERA')

  const before = await page.evaluate((id) => window.__hew_shop_test!.getObjectBounds(id), objectId)

  // The SAME press-drag the CONTRACT test above uses — if 'm' had silently
  // armed Move (or the drag-arm gate itself regressed), this moves the box.
  await page.mouse.move(px.x, px.y)
  await page.mouse.down()
  await page.mouse.move(px.x + 60, px.y + 45, { steps: 12 })
  await page.mouse.up()
  await settleFrame(page)

  const after = await page.evaluate((id) => window.__hew_shop_test!.getObjectBounds(id), objectId)
  expect(after).toEqual(before)
})

// Round-3 playtest finding 1: the CONTRACT test above proves a press-drag on
// a part never MOVES it, but a prior fix routed that drag into the Select
// marquee's own arm-and-defer path — meaningless without multi-select, and
// (this test's own point) a rubber-band overlay is not what a real drag
// should show at all under readOnly. The fix replaces that arm with a
// lighter `deferredTapDrag` (Viewport.tsx) that never shows the marquee and
// never computes a selection; the drag itself reaches OrbitControls
// untouched (this file's own module doc has the touch mechanics), which
// this test proves by comparing the camera pose before/after.
//
// A REAL touch drag (CDP `Input.dispatchTouchEvent`, the exact pattern
// `camera-playtest2.spec.ts` established), not `page.mouse` — Select
// (like every non-draw tool) nulls `controls.mouseButtons.LEFT`, so a
// MOUSE drag can never orbit regardless of this fix; only OrbitControls'
// separate `touches.ONE` binding (never nulled for Select) does, and only
// a genuine `pointerType: 'touch'` event exercises that path. Chromium-only
// (CDP), same precedent.
test('CONTRACT: a press-drag orbits the camera with no marquee overlay ever shown (round-3 finding 1)', async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'chromium', 'CDP touch dispatch is Chromium-only')
  const { bytes } = await buildSingleBoxFixture(page)
  await bootShopModeWith(page, bytes)

  await page.evaluate((cam) => window.__hew_shop_test!.setCamera(cam), {
    position: [PINNED_CAMERA.position.x, PINNED_CAMERA.position.y, PINNED_CAMERA.position.z],
    target: [PINNED_CAMERA.target.x, PINNED_CAMERA.target.y, PINNED_CAMERA.target.z],
    up: [PINNED_CAMERA.up.x, PINNED_CAMERA.up.y, PINNED_CAMERA.up.z],
    fovDeg: PINNED_CAMERA.fovDeg,
  })
  await settleFrame(page)

  const canvasBox = await page.locator('canvas').first().boundingBox()
  if (canvasBox === null) throw new Error('viewport canvas has no bounding box')
  const rect = { left: canvasBox.x, top: canvasBox.y, width: canvasBox.width, height: canvasBox.height }
  const vp = buildViewProjection(PINNED_CAMERA, rect.width / rect.height)
  const px = worldToPagePixel({ x: 0.5, y: 0.5, z: 1 }, vp, rect)
  if (px === null) throw new Error('the fixture object does not project onto the canvas under PINNED_CAMERA')

  const cameraBefore = await page.evaluate(() => window.__hew_shop_test!.getCameraPose())
  const marquee = page.getByTestId('viewport-marquee-overlay')
  await expect(marquee).toBeHidden()

  const client = await page.context().newCDPSession(page)
  await client.send('Input.dispatchTouchEvent', { type: 'touchStart', touchPoints: [{ x: px.x, y: px.y }] })
  await client.send('Input.dispatchTouchEvent', { type: 'touchMove', touchPoints: [{ x: px.x + 70, y: px.y + 10 }] })
  await page.waitForTimeout(50)
  // Still mid-drag — the marquee must never have appeared, not even
  // transiently, over the course of the drag.
  await expect(marquee).toBeHidden()
  await client.send('Input.dispatchTouchEvent', { type: 'touchEnd', touchPoints: [] })
  await settleFrame(page)

  await expect(marquee).toBeHidden()
  const cameraAfter = await page.evaluate(() => window.__hew_shop_test!.getCameraPose())
  expect(cameraAfter).not.toEqual(cameraBefore)
})

// Round-3 playtest finding 1 continued: a PLAIN tap (no drag) on a part
// still has to resolve tap-to-inspect through the SAME `deferredTapDrag`
// path the drag test above exercises — the existing "tapping a part shows
// the inspect card" test above already covers this end-to-end (it hasn't
// changed), so this only re-asserts it survives a drag that immediately
// precedes it, proving the deferred-pick state resets cleanly between
// gestures rather than only working on the FIRST tap after boot.
test('a plain tap still resolves tap-to-inspect right after a press-drag orbit (deferredTapDrag resets cleanly)', async ({ page }) => {
  const { bytes } = await buildSingleBoxFixture(page)
  await bootShopModeWith(page, bytes)

  await page.evaluate((cam) => window.__hew_shop_test!.setCamera(cam), {
    position: [PINNED_CAMERA.position.x, PINNED_CAMERA.position.y, PINNED_CAMERA.position.z],
    target: [PINNED_CAMERA.target.x, PINNED_CAMERA.target.y, PINNED_CAMERA.target.z],
    up: [PINNED_CAMERA.up.x, PINNED_CAMERA.up.y, PINNED_CAMERA.up.z],
    fovDeg: PINNED_CAMERA.fovDeg,
  })
  await settleFrame(page)

  const canvasBox = await page.locator('canvas').first().boundingBox()
  if (canvasBox === null) throw new Error('viewport canvas has no bounding box')
  const rect = { left: canvasBox.x, top: canvasBox.y, width: canvasBox.width, height: canvasBox.height }
  const vp = buildViewProjection(PINNED_CAMERA, rect.width / rect.height)
  const px = worldToPagePixel({ x: 0.5, y: 0.5, z: 1 }, vp, rect)
  if (px === null) throw new Error('the fixture object does not project onto the canvas under PINNED_CAMERA')

  // A drag first (orbits, resolves nothing) — then re-pin the camera so the
  // object projects to the SAME pixel again for the plain tap below.
  await page.mouse.move(px.x, px.y)
  await page.mouse.down()
  await page.mouse.move(px.x + 50, px.y + 5, { steps: 8 })
  await page.mouse.up()
  await page.evaluate((cam) => window.__hew_shop_test!.setCamera(cam), {
    position: [PINNED_CAMERA.position.x, PINNED_CAMERA.position.y, PINNED_CAMERA.position.z],
    target: [PINNED_CAMERA.target.x, PINNED_CAMERA.target.y, PINNED_CAMERA.target.z],
    up: [PINNED_CAMERA.up.x, PINNED_CAMERA.up.y, PINNED_CAMERA.up.z],
    fovDeg: PINNED_CAMERA.fovDeg,
  })
  await settleFrame(page)

  await page.mouse.move(px.x, px.y)
  await page.mouse.down()
  await page.mouse.up()

  const card = page.getByRole('status')
  await expect(card).toBeVisible()
  await expect(card).toContainText('Object 1')
})

// Adversarial-review finding 2 (CRITICAL): `onDoubleClick` had zero readOnly
// checks — a double-tap on a group/component instance opens a group/
// component EDIT SESSION (`runOpenGroupSession`/`openExplodeSessionOrFallback`
// → `wasmScene.open_group_session`), a real kernel mutation with its own
// history entry, not a view-state toggle. `sessionDepth()` (testHarness.ts)
// is a QUERY added specifically to prove this — nothing in Shop Mode's DOM
// reflects an open session (ShopApp renders no breadcrumb chrome for one).
test('CONTRACT: a double-tap on a group never opens an edit session', async ({ page }) => {
  const { bytes } = await buildGroupFixture(page)
  await bootShopModeWith(page, bytes)

  await page.evaluate((cam) => window.__hew_shop_test!.setCamera(cam), {
    position: [PINNED_CAMERA.position.x, PINNED_CAMERA.position.y, PINNED_CAMERA.position.z],
    target: [PINNED_CAMERA.target.x, PINNED_CAMERA.target.y, PINNED_CAMERA.target.z],
    up: [PINNED_CAMERA.up.x, PINNED_CAMERA.up.y, PINNED_CAMERA.up.z],
    fovDeg: PINNED_CAMERA.fovDeg,
  })
  await settleFrame(page)

  const canvasBox = await page.locator('canvas').first().boundingBox()
  if (canvasBox === null) throw new Error('viewport canvas has no bounding box')
  const rect = { left: canvasBox.x, top: canvasBox.y, width: canvasBox.width, height: canvasBox.height }
  const vp = buildViewProjection(PINNED_CAMERA, rect.width / rect.height)
  const px = worldToPagePixel({ x: 0.5, y: 0.5, z: 1 }, vp, rect)
  if (px === null) throw new Error('the grouped fixture object does not project onto the canvas under PINNED_CAMERA')

  expect(await page.evaluate(() => window.__hew_shop_test!.sessionDepth())).toBe(0)

  // A real double-click (Viewport's `dblclick` listener, not a synthetic
  // double-tap — Playwright's touch APIs have no double-tap primitive, same
  // gap this file's module doc notes for long-press; a plain `pointerType:
  // 'mouse'` double-click exercises the identical `onDoubleClick` handler a
  // real double-tap would).
  await page.mouse.dblclick(px.x, px.y)
  await settleFrame(page)

  expect(await page.evaluate(() => window.__hew_shop_test!.sessionDepth())).toBe(0)
})

test('the Parts sheet opens on drag, toggling a row\'s eye hides it, and long-press isolate + Show all restores', async ({ page }) => {
  const bytes = await buildFixtureBytes(page)
  await bootShopModeWith(page, bytes)

  // A 600px pull comfortably clears the "full" detent threshold for an
  // 844px-tall viewport.
  await dragSheetHandle(page, -600)

  const sheet = page.getByTestId('parts-sheet')
  await expect(sheet.getByText('Object 1', { exact: true })).toBeVisible()
  await expect(sheet.getByText('Object 2', { exact: true })).toBeVisible()
  await expect(sheet.getByText('Object 3', { exact: true })).toBeVisible()
  // The header's "N of M shown" pill (design_handoff_shop_mode/README.md
  // §1) — lives in the drag-handle row, so it's visible at every detent,
  // not just full.
  await expect(sheet.getByText('3 of 3 shown')).toBeVisible()

  // (c) Toggling a row's eye hides the part — a renderer-level Shop Mode
  // view-state change (ShopApp.tsx's toggleHiddenNode), asserted via the
  // eye button's own aria-label flip (Hide → Show), the DOM fallback the
  // task calls for when no scene-state hook exists.
  const eyeObject2 = sheet.getByRole('button', { name: 'Hide Object 2' })
  await eyeObject2.tap()
  await expect(sheet.getByRole('button', { name: 'Show Object 2' })).toBeVisible()

  // (d) Long-press Object 1 → isolate. Mouse-based hold (module doc) since
  // Playwright's touch APIs can't hold; LONG_PRESS_MS is 500ms server-side,
  // so 700ms of hold comfortably clears it.
  const row1 = sheet.getByText('Object 1', { exact: true })
  await row1.hover()
  await page.mouse.down()
  await page.waitForTimeout(700)
  await page.mouse.up()

  // Isolating Object 1 hides Object 3 too (it was visible until now) — the
  // Parts sheet's own row eyes reflect isolate, not just the viewport
  // (partsSheetModel.ts's isolate-union). Object 2 stays hidden — it was
  // ALREADY hidden by its own eye toggle above, not by isolate.
  const showAllChip = page.getByRole('button', { name: /show all/i })
  await expect(showAllChip).toBeVisible()
  await expect(sheet.getByRole('button', { name: 'Show Object 3' })).toBeVisible()
  await expect(sheet.getByRole('button', { name: 'Show Object 2' })).toBeVisible()
  await expect(sheet.getByRole('button', { name: 'Hide Object 1' })).toBeVisible() // still visible itself

  // The sheet at "full" height overlaps the HUD's bottom-left corner (where
  // "Show all" lives) — real iOS-sheet behavior (it covers whatever's
  // beneath at that detent), not a bug this spec works around by tapping
  // through a stacking accident. Collapse to peek first, exactly what a
  // real user would do to reach the chip — `.tap()` below fails outright if
  // the chip is still covered, so this doubles as the collapse assertion.
  await dragSheetHandle(page, 600)

  // Show all: undoes ONLY the isolate — Object 3 comes back, but Object 2
  // (the sheet-driven hide from (c)) stays hidden. This is the design's
  // coherence requirement between long-press isolate and sheet-driven hides.
  await showAllChip.tap()

  await dragSheetHandle(page, -600)
  await expect(sheet.getByRole('button', { name: 'Hide Object 3' })).toBeVisible()
  await expect(sheet.getByRole('button', { name: 'Show Object 2' })).toBeVisible()
  await expect(showAllChip).not.toBeVisible()
})

test('switching units via the header chip\'s picker changes a row\'s rendered dims format live', async ({ page }) => {
  const bytes = await buildFixtureBytes(page)
  await bootShopModeWith(page, bytes)
  await dragSheetHandle(page, -600)

  const sheet = page.getByTestId('parts-sheet')
  // Object 1 is a 1×1×0 box (buildFixtureBytes) — metric mode (the pinned
  // default, bootShopModeWith's own doc comment) renders its dims as three
  // separate mono columns reading "1 m".
  await expect(sheet.getByText('1 m').first()).toBeVisible()

  await sheet.getByRole('button', { name: /^units: meters$/i }).tap()
  const picker = page.getByRole('dialog', { name: 'Units' })
  await expect(picker).toBeVisible()
  await picker.getByRole('button', { name: /^decimal inches$/i }).tap()
  await expect(picker).not.toBeVisible()

  // The picked format persists (units.ts's shared singleton — README
  // "Decisions": "persisted per device") and every row re-renders in it
  // immediately: metric's three columns give way to imperial's single
  // stacked L/W/H line, with NO reload and no re-fetch of the document.
  // Object 1 is a 1×1×1 box (buildFixtureBytes), so all three read 39.37"
  // (1m rounded to 3 decimal inches and trimmed by formatLengthIn).
  await expect(sheet.getByRole('button', { name: /^units: decimal inches$/i })).toBeVisible()
  await expect(sheet.getByText('1 m')).not.toBeVisible()
  // All 3 fixture objects are identical 1×1×1 boxes — `.first()` avoids a
  // strict-mode violation over the 3 identical matches.
  await expect(sheet.getByText(/^L 39\.37" · W 39\.37" · H 39\.37"$/).first()).toBeVisible()
})

// ---------------------------------------------------------------------------
// "Open on Phone" — the E2E-encrypted relay handoff's receive path
// (workers/share-relay/README.md, `src/io/shareCrypto.ts`'s module doc).
// The desktop's upload side (`PhoneShareDialog.tsx`) and the relay Worker
// itself are exercised elsewhere (`dialogs.test.tsx`, `workers/share-relay/
// src/handlers.test.ts`) — what a Playwright E2E CAN and SHOULD cover is
// the PHONE side: does booting with (or in-app-scanning) a `#recv=…`
// handoff actually fetch the relay, decrypt with the key riding the
// fragment, load the bytes, and clean up after itself. `page.route`
// intercepts the same-origin `/relay/drop/<token>` with REAL ciphertext — built
// in-test via `shareCrypto.encrypt`, the exact function the desktop itself
// calls — so this needs no real desktop, no real network relay, and no
// fake/simplified crypto standing in for the real wire format.
// ---------------------------------------------------------------------------

const HANDOFF_FIXTURE = readFileSync(resolve(process.cwd(), 'e2e/fixtures/follow-me-2.hew'))

/** 128-bit base64url token (share-relay's own shape) and 256-bit
 *  base64url key (shareCrypto's own shape) — fixed rather than randomly
 *  generated per test so route patterns below can hardcode the URL. */
const HANDOFF_TOKEN = 'a'.repeat(22)

/** Reads every recorded name out of the web recents store (`io/recents.ts`:
 *  db `hew-recents`, store `recents`) directly via IndexedDB — the same
 *  "seed/read the real store from `page.evaluate`" pattern
 *  `session.spec.ts` uses for the separate `hew-recovery` store. */
async function readRecentNames(page: Page): Promise<string[]> {
  return page.evaluate(async () => {
    const db: IDBDatabase = await new Promise((resolveDb, rejectDb) => {
      // No explicit version — the app owns the recents schema version (bumped
      // to 2 for the content-hash dedupe), and pinning an older number here
      // races the app's own upgrade and throws VersionError. Opening without a
      // version attaches to whatever the app created.
      const req = indexedDB.open('hew-recents')
      req.onsuccess = () => resolveDb(req.result)
      req.onerror = () => rejectDb(req.error)
    })
    const names = await new Promise<string[]>((resolveNames, rejectNames) => {
      const tx = db.transaction('recents', 'readonly')
      const all: string[] = []
      const cursorReq = tx.objectStore('recents').openCursor()
      cursorReq.onsuccess = () => {
        const cursor = cursorReq.result
        if (cursor === null) {
          resolveNames(all)
        } else {
          all.push((cursor.value as { name: string }).name)
          cursor.continue()
        }
      }
      cursorReq.onerror = () => rejectNames(cursorReq.error)
    })
    db.close()
    return names
  })
}

// Adversarial-review finding 1 (CRITICAL): a LINK-ARRIVED `#recv=…` must
// clear an explicit "Open shared model?" confirmation gate
// (`ReceiveConfirmSheet.tsx`) before anything is fetched/decrypted/loaded —
// share-relay's `/drop` PUT has no auth, so a forged link is otherwise
// indistinguishable from a real QR scan by the time it reaches the phone.

test('QR handoff (#recv=): shows a confirmation gate before fetching; Open completes the load and records to recents', async ({ page }) => {
  const key = generateKey()
  const ciphertext = await encrypt(key, HANDOFF_FIXTURE)
  let fetchCount = 0
  await page.route(`**/relay/drop/${HANDOFF_TOKEN}`, async (route) => {
    fetchCount++
    await route.fulfill({ status: 200, contentType: 'application/octet-stream', body: Buffer.from(ciphertext) })
  })

  // Detour through about:blank first — see bootShopModeWith's doc comment
  // above for why a URL that differs only by fragment from the CURRENT
  // document is a same-document hash navigation (main.tsx never re-runs),
  // not a real reload. This test's very first navigation would normally be
  // a genuine one regardless, but the detour is cheap insurance against
  // Playwright ever reusing a warm about:blank-adjacent page across runs.
  //
  // Deliberately `#shop&`-less (a real camera-app scan lands Safari on
  // exactly this bare `#recv=…` shape on the canonical origin — the task
  // this handoff exists for) — the auto-detect heuristic
  // (`shellMode.ts`'s `resolveShellMode`) routes this phone-sized,
  // coarse-pointer Playwright session into Shop Mode regardless (and now
  // also force-routes on the bare `#recv=` prefix itself — finding 2).
  await page.goto('about:blank')
  await page.goto(`/#recv=${HANDOFF_TOKEN}.${toBase64Url(key)}.Bench`)
  await page.waitForFunction(() => window.__hew_shop_test?.isReady() === true, null, { timeout: 15_000 })

  // The confirm gate shows the untrusted shared name ("Bench") and nothing
  // has been fetched yet.
  const confirm = page.getByRole('dialog', { name: 'Open shared model?' })
  await expect(confirm).toBeVisible()
  await expect(confirm.getByText('Bench')).toBeVisible()
  expect(fetchCount).toBe(0)

  // The hash is stripped bare (not to `#shop` — that's only preserved when
  // it was ALREADY part of the incoming hash, which it wasn't here) —
  // immediately, independent of the confirm/fetch/load sequence still
  // pending (adversarial-review finding 8) — so a reload can never
  // re-request an already one-shot-consumed token, whether or not the user
  // has decided yet.
  await expect.poll(() => page.evaluate(() => window.location.hash)).toBe('')

  await confirm.getByRole('button', { name: /^open$/i }).click()
  await expect(confirm).not.toBeVisible()

  // The name carried on the fragment is what ShopApp's top strip shows —
  // proof the fetch, decrypt, parse, and real open-apply path
  // (applyOpenedBytes) all ran, not just that SOME document loaded.
  await expect(page.getByText('Bench')).toBeVisible()
  expect(fetchCount).toBe(1)

  await expect.poll(() => readRecentNames(page)).toContain('Bench')
})

test('QR handoff (#recv=): Cancel never fetches, strips the hash, and leaves the empty state intact (no re-prompt on reload)', async ({ page }) => {
  const key = generateKey()
  let fetchCount = 0
  await page.route(`**/relay/drop/${HANDOFF_TOKEN}`, async (route) => {
    fetchCount++
    await route.fulfill({ status: 200, contentType: 'application/octet-stream', body: Buffer.from(await encrypt(key, HANDOFF_FIXTURE)) })
  })

  await page.goto('about:blank')
  await page.goto(`/#recv=${HANDOFF_TOKEN}.${toBase64Url(key)}.Bench`)
  await page.waitForFunction(() => window.__hew_shop_test?.isReady() === true, null, { timeout: 15_000 })

  const confirm = page.getByRole('dialog', { name: 'Open shared model?' })
  await expect(confirm).toBeVisible()
  await confirm.getByRole('button', { name: /^cancel$/i }).click()
  await expect(confirm).not.toBeVisible()

  expect(fetchCount).toBe(0)
  await expect(page.getByRole('button', { name: /^open a model…$/i })).toBeVisible()
  await expect.poll(() => page.evaluate(() => window.location.hash)).toBe('')

  // A reload lands on the plain empty state, not a re-prompt — the hash is
  // already gone, so there's nothing left for the boot effect to parse.
  await page.reload()
  await page.waitForFunction(() => window.__hew_shop_test?.isReady() === true, null, { timeout: 15_000 })
  await expect(page.getByRole('dialog', { name: 'Open shared model?' })).not.toBeVisible()
  await expect(page.getByRole('button', { name: /^open a model…$/i })).toBeVisible()
  expect(fetchCount).toBe(0)
})

test('QR handoff (#recv=): a 404 (expired/consumed token) after confirming shows a toast and leaves the empty state intact', async ({ page }) => {
  const key = generateKey()
  await page.route(`**/relay/drop/${HANDOFF_TOKEN}`, async (route) => {
    await route.fulfill({ status: 404, body: '' })
  })

  await page.goto('about:blank')
  await page.goto(`/#recv=${HANDOFF_TOKEN}.${toBase64Url(key)}.Bench`)
  await page.waitForFunction(() => window.__hew_shop_test?.isReady() === true, null, { timeout: 15_000 })

  const confirm = page.getByRole('dialog', { name: 'Open shared model?' })
  await expect(confirm).toBeVisible()
  await confirm.getByRole('button', { name: /^open$/i }).click()

  await expect(page.getByText(/expired/i)).toBeVisible()
  // Landed on the normal empty state — the fetch failure never left the
  // shell in some half-loaded limbo. ("Shop Mode" text is ambiguous here —
  // it appears both in the empty state's own title AND the top strip's
  // doc-name placeholder — so the empty state's unique "Open a model…"
  // button is the tell instead.)
  await expect(page.getByRole('button', { name: /^open a model…$/i })).toBeVisible()

  // Failed just as thoroughly here: the hash is still stripped (already
  // gone well before this point — finding 8's mount-time strip).
  await expect.poll(() => page.evaluate(() => window.location.hash)).toBe('')
})

// ---------------------------------------------------------------------------
// In-app QR scanner (`ScanSheet.tsx`) — CI has no real camera, so this
// fakes the WHOLE media pipeline rather than just the decode step: a
// `getUserMedia` override (installed before ShopApp ever mounts, via
// `addInitScript`) hands back a live `MediaStream` off an offscreen
// `<canvas>.captureStream()` — enough for the scan loop's own readiness
// gate (`video.readyState`/`videoWidth`/`videoHeight`) to pass on a REAL
// `<video>` element, no permission prompt or OS camera involved — and the
// decode step itself comes from `window.__hew_shop_test.setFakeQrDecode`
// (`testHarness.ts`), the same debug/test-build hook every other Shop Mode
// E2E harness call in this file uses.
// ---------------------------------------------------------------------------

/** Overrides `getUserMedia` with a fake camera stream sourced from an
 *  offscreen `<canvas>.captureStream()`, continuously redrawn (a static,
 *  never-repainted canvas never reaches a "ready" video frame in WebKit —
 *  its `<video>` stays at `readyState 0`/`videoWidth 0` forever, which
 *  would otherwise stall the scan loop's own `video.play()` await
 *  indefinitely; Chromium is more lenient but this works identically on
 *  both). Must run via `addInitScript` (before ShopApp's own scripts, so
 *  the override is in place before anything calls `getUserMedia`), and
 *  before the FAKE decode value is queued (below) — `createQrEngine()`
 *  reads whichever decode engine is live at scan-start, not per-frame. */
async function installFakeCamera(page: Page): Promise<void> {
  await page.addInitScript(() => {
    const canvas = document.createElement('canvas')
    canvas.width = 64
    canvas.height = 64
    const ctx = canvas.getContext('2d')
    let hue = 0
    setInterval(() => {
      if (ctx === null) return
      hue = (hue + 10) % 360
      ctx.fillStyle = `hsl(${hue}, 50%, 50%)`
      ctx.fillRect(0, 0, 64, 64)
    }, 50)
    const fakeStream = (canvas as HTMLCanvasElement & { captureStream: (fps?: number) => MediaStream }).captureStream(10)
    Object.defineProperty(navigator, 'mediaDevices', {
      configurable: true,
      value: { getUserMedia: async () => fakeStream },
    })
  })
}

test('scanner: decoding a valid handoff in-app closes the sheet and loads the model', async ({ page }) => {
  const key = generateKey()
  const ciphertext = await encrypt(key, HANDOFF_FIXTURE)
  await page.route(`**/relay/drop/${HANDOFF_TOKEN}`, async (route) => {
    await route.fulfill({ status: 200, contentType: 'application/octet-stream', body: Buffer.from(ciphertext) })
  })

  await installFakeCamera(page)
  await page.addInitScript(() => localStorage.setItem('hew.settings.lengthUnit', 'm'))
  await page.addInitScript((key) => localStorage.setItem(key, '1'), HINT_TEST_SUPPRESS_KEY)

  await page.goto('about:blank')
  await page.goto('/#shop')
  await page.waitForFunction(() => window.__hew_shop_test?.isReady() === true, null, { timeout: 15_000 })

  // Queued BEFORE the sheet ever opens — `qrEngine.ts`'s `createQrEngine`
  // runs once, at scan-start (right after the fake camera stream above
  // resolves), and picks whichever decode engine is live AT THAT MOMENT.
  // Setting this after the sheet's already scanning would be too late: the
  // real (jsQR, since this sandboxed Chromium has no `BarcodeDetector`)
  // engine would already be the one running, and it has no idea about this
  // fake value — it only ever sees the actual (blank) canvas frames.
  // The QR names THIS page's origin (a desktop pointed at the same server
  // the phone is bookmarked on) — a code for a different origin is not
  // fetched from here; the sheet offers to open it there instead
  // (`ScanSheet.tsx`'s foreign-origin state, unit-tested in
  // ScanSheet.test.tsx).
  await page.evaluate(
    ({ token, key, name }) => window.__hew_shop_test!.setFakeQrDecode(`${window.location.origin}/#recv=${token}.${key}.${name}`),
    { token: HANDOFF_TOKEN, key: toBase64Url(key), name: 'Bench' },
  )

  const scanButton = page.getByRole('button', { name: /^from your desktop…$/i })
  await scanButton.tap()

  const sheet = page.getByRole('dialog', { name: 'Scan from desktop' })
  await expect(sheet).toBeVisible()

  await expect(sheet).not.toBeVisible()
  await expect(page.getByText('Bench')).toBeVisible()
})

test('scanner: an unrecognized QR shows a "not a Hew code" hint and keeps scanning', async ({ page }) => {
  await installFakeCamera(page)
  await page.addInitScript(() => localStorage.setItem('hew.settings.lengthUnit', 'm'))
  await page.addInitScript((key) => localStorage.setItem(key, '1'), HINT_TEST_SUPPRESS_KEY)

  await page.goto('about:blank')
  await page.goto('/#shop')
  await page.waitForFunction(() => window.__hew_shop_test?.isReady() === true, null, { timeout: 15_000 })

  // Queued BEFORE the sheet opens — see the previous test's comment on
  // `createQrEngine`'s scan-start-only engine selection for why.
  await page.evaluate(() => window.__hew_shop_test!.setFakeQrDecode('https://example.com/not-a-hew-code'))

  await page.getByRole('button', { name: /^from your desktop…$/i }).tap()
  const sheet = page.getByRole('dialog', { name: 'Scan from desktop' })
  await expect(page.getByTestId('shop-scan-viewfinder')).toBeVisible()
  await expect(page.getByTestId('shop-scan-not-hew')).toBeVisible()
  // Still up — an unrecognized code never closes the sheet.
  await expect(sheet).toBeVisible()

  // Camera cleanup even from THIS path: closing via Cancel must still stop
  // the fake stream's own track(s) — asserted indirectly by the sheet
  // actually closing (a leaked track wouldn't prevent that, but a thrown
  // error mid-teardown would leave it stuck open).
  await page.getByRole('button', { name: /^cancel$/i }).tap()
  await expect(sheet).not.toBeVisible()
})

// ---------------------------------------------------------------------------
// Gesture-discoverability hints (design_handoff_shop_mode's "Gesture
// discoverability" section; state machine in `src/shop/hints.ts`). ONE spec
// covers the end-to-end contract for hint (a) — a genuinely fresh install
// (no flags at all, including `bootShopModeWith`'s own default hint
// suppression, explicitly opted OUT of below) shows it on first open, and a
// real touch tap kills it AND persists the flag. Every other rule (hint
// (b)'s 8s timer, hint (c)'s 3rd-inspect streak, "only one hint at a time",
// "pre-satisfied by doing the gesture first") is `hints.test.ts`'s job —
// exercising an 8s real-time delay here would make this file slow for
// coverage `hints.test.ts` already gets deterministically with a fake clock.
// ---------------------------------------------------------------------------

test('gesture hint (a): a fresh install shows "Tap a part for its size" on first open, and a tap kills it + persists the flag', async ({ page }) => {
  const bytes = await buildFixtureBytes(page)
  // The one boot in this file that does NOT suppress hints — a real fresh
  // install has no `hew.shop.hint.*`/suppression flags at all.
  await bootShopModeWith(page, bytes, { suppressHints: false })

  const hintTag = page.getByText('Tap a part for its size')
  await expect(hintTag).toBeVisible()

  // The SAME Orbit→Select round-trip + pinned-camera projection + real
  // touch tap the "tapping a part shows the inspect card" test above uses
  // (see its own comment: a real `.click()` here is what proved, in the
  // original regression this guards, that the pick layer is actually live
  // before the canvas tap below — not just cosmetic tool-switching) —
  // reused verbatim rather than tapping straight off the tool's default
  // 'Select' state, for the exact same reliability this file's other tap
  // tests already lean on.
  await page.getByRole('button', { name: 'Orbit' }).click()
  await page.getByRole('button', { name: 'Select', exact: true }).click()

  // This is a genuine tap, not a synthetic click, so the hint's
  // `pointer-events: none` overlay never stands in its way.
  await page.evaluate((cam) => window.__hew_shop_test!.setCamera(cam), {
    position: [PINNED_CAMERA.position.x, PINNED_CAMERA.position.y, PINNED_CAMERA.position.z],
    target: [PINNED_CAMERA.target.x, PINNED_CAMERA.target.y, PINNED_CAMERA.target.z],
    up: [PINNED_CAMERA.up.x, PINNED_CAMERA.up.y, PINNED_CAMERA.up.z],
    fovDeg: PINNED_CAMERA.fovDeg,
  })
  await settleFrame(page)

  const canvasBox = await page.locator('canvas').first().boundingBox()
  if (canvasBox === null) throw new Error('viewport canvas has no bounding box')
  const rect = { left: canvasBox.x, top: canvasBox.y, width: canvasBox.width, height: canvasBox.height }
  const vp = buildViewProjection(PINNED_CAMERA, rect.width / rect.height)
  const px = worldToPagePixel({ x: 0.5, y: 0.5, z: 1 }, vp, rect) // Object 1's top face
  if (px === null) throw new Error('Object 1 does not project onto the canvas under PINNED_CAMERA')

  await page.touchscreen.tap(px.x, px.y)

  await expect(hintTag).not.toBeVisible()
  await expect
    .poll(() => page.evaluate((key) => localStorage.getItem(key), HINT_STORAGE_KEYS.tap))
    .toBe('1')
})

// ---------------------------------------------------------------------------
// Playtest corrections: translucent chrome (item 2), the ⋯ button's iOS-
// convention swap for its own open menu (item 3), and the empty state's
// honest "From your desktop…" explainer (item 4, replacing a button that
// used to just toast a one-line hint).
// ---------------------------------------------------------------------------

/** Parses a `rgb(...)`/`rgba(...)` computed-style string into its alpha
 *  channel (1 for a bare `rgb()`, which has none). */
function cssAlpha(color: string): number {
  // Chromium resolves `color-mix(...)` to `color(srgb r g b / a)` (CSS
  // Color 4's slash syntax) — checked first. A plain opaque `rgb(r, g, b)`
  // has no alpha at all, and must NOT be mistaken for legacy `rgba(...)`:
  // a naive "last comma-separated number" regex would grab the BLUE
  // channel instead (caught by this test itself — --shop-dock light's
  // #fbf7f0 = rgb(251, 247, 240) read back a bogus alpha of 240). Only the
  // real 4-argument `rgba(...)` form is trusted for its 4th value.
  const slash = /\/\s*([\d.]+)\s*\)\s*$/.exec(color)
  if (slash !== null) return parseFloat(slash[1])
  const rgba = /^rgba\(([^)]+)\)$/.exec(color)
  if (rgba !== null) {
    const parts = rgba[1].split(',').map((s) => s.trim())
    if (parts.length === 4) return parseFloat(parts[3])
  }
  return 1
}

test('the title pill and the ⋯ button are translucent; the portrait dock stays opaque', async ({ page }) => {
  const bytes = await buildFixtureBytes(page)
  await bootShopModeWith(page, bytes)

  const pillBg = await page.getByText('Fixture.hew').evaluate((el) => getComputedStyle(el.parentElement!).backgroundColor)
  const ellipsisBg = await page.getByRole('button', { name: 'Settings' }).evaluate((el) => getComputedStyle(el).backgroundColor)
  // ~0.85-0.9 alpha (design correction item 2) — neither fully opaque (1)
  // nor so faint it's unreadable.
  expect(cssAlpha(pillBg)).toBeGreaterThan(0.8)
  expect(cssAlpha(pillBg)).toBeLessThan(0.95)
  expect(cssAlpha(ellipsisBg)).toBeGreaterThan(0.8)
  expect(cssAlpha(ellipsisBg)).toBeLessThan(0.95)

  // The portrait workbench dock's own toolbar row is explicitly EXCLUDED
  // from translucency (item 2: "portrait dock stays opaque") — its
  // Zoom Extents button's row background resolves to a bare `rgb()` (alpha
  // 1), not the pill/⋯'s `rgba()`.
  const dockRowBg = await page.getByRole('button', { name: 'Zoom Extents' }).evaluate((el) => getComputedStyle(el.parentElement!).backgroundColor)
  expect(cssAlpha(dockRowBg)).toBe(1)
})

// Maintainer-approved "Idea 2" split of the old combined overflow menu: the
// pill (document actions, `DocumentMenu`) and the ⋯ button (settings,
// `SettingsMenu`) each independently hide while their OWN panel is open,
// mirroring the round-3 iOS corner-occupying convention onto both top
// corners of the strip instead of just the right one.
test('the document pill hides while its own menu is open, and reappears on close (iOS convention)', async ({ page }) => {
  const bytes = await buildFixtureBytes(page)
  await bootShopModeWith(page, bytes)

  const pill = page.getByRole('button', { name: /^document menu/i })
  await expect(pill).toBeVisible()

  await pill.tap()
  await expect(page.getByText('Use full editor')).toBeVisible()
  await expect(pill).not.toBeVisible()

  // Tapping the scrim closes the menu — the pill is restored. A specific
  // corner position, not Playwright's default (the scrim element's own
  // bounding-box center): the scrim spans the FULL screen, but the
  // top-left-anchored panel is tall enough in portrait to cover that
  // center point with its own rows — tapping there would hit the panel,
  // not dismiss it. Bottom-right is clear of both the panel and the
  // (lower-z-index, so harmless either way) dock.
  await page.getByTestId('shop-document-scrim').tap({ position: { x: 370, y: 800 } })
  await expect(page.getByText('Use full editor')).not.toBeVisible()
  await expect(pill).toBeVisible()
})

test('the ⋯ button hides while its own settings menu is open, and reappears on close (iOS convention)', async ({ page }) => {
  const bytes = await buildFixtureBytes(page)
  await bootShopModeWith(page, bytes)

  const ellipsis = page.getByRole('button', { name: 'Settings' })
  await expect(ellipsis).toBeVisible()

  await ellipsis.tap()
  await expect(page.getByText('GESTURES')).toBeVisible()
  await expect(ellipsis).not.toBeVisible()

  // Tapping the scrim closes the menu — the ⋯ button is restored. A
  // specific corner position, not Playwright's default (the scrim
  // element's own bounding-box center): the scrim spans the FULL screen,
  // but the menu panel itself (a higher-z-index sibling) is tall enough in
  // portrait to cover that center point with its own content (GESTURES
  // section) — tapping there would hit the panel, not dismiss it, same as
  // a real user tapping ON the menu rather than outside it. Bottom-left is
  // clear of both the top-right-anchored panel and the (lower-z-index, so
  // harmless either way) dock.
  await page.getByTestId('shop-settings-scrim').tap({ position: { x: 20, y: 800 } })
  await expect(page.getByText('GESTURES')).not.toBeVisible()
  await expect(ellipsis).toBeVisible()
})

// Round-3 playtest finding 3: the panel used to render BELOW the ⋯ button
// rather than occupying its corner. The maintainer-approved "Idea 2" split
// mirrors this convention onto the pill's OWN top-left corner too. Captures
// each button's own bounding box BEFORE opening (it hides once its menu is
// open — the tests above's own point), then asserts the matching panel's
// corner lands there exactly — the modern Apple toolbar-popover convention
// this fix targets.
test('the document panel occupies the pill\'s own top-left corner (round-3 finding 3, mirrored)', async ({ page }) => {
  const bytes = await buildFixtureBytes(page)
  await bootShopModeWith(page, bytes)

  const pill = page.getByRole('button', { name: /^document menu/i })
  const pillBox = await pill.boundingBox()
  if (pillBox === null) throw new Error('pill has no bounding box')

  await pill.tap()
  const panel = page.getByTestId('shop-document-panel')
  await expect(panel).toBeVisible()
  const panelBox = await panel.boundingBox()
  if (panelBox === null) throw new Error('document panel has no bounding box')

  // Top edges flush, left edges flush — "the panel's top-left corner sits
  // exactly where the pill's top-left corner was."
  expect(panelBox.y).toBeCloseTo(pillBox.y, 0)
  expect(panelBox.x).toBeCloseTo(pillBox.x, 0)
})

test('the settings panel occupies the ⋯ button\'s own top-right corner (round-3 finding 3)', async ({ page }) => {
  const bytes = await buildFixtureBytes(page)
  await bootShopModeWith(page, bytes)

  const ellipsis = page.getByRole('button', { name: 'Settings' })
  const buttonBox = await ellipsis.boundingBox()
  if (buttonBox === null) throw new Error('⋯ button has no bounding box')

  await ellipsis.tap()
  const panel = page.getByTestId('shop-settings-panel')
  await expect(panel).toBeVisible()
  const panelBox = await panel.boundingBox()
  if (panelBox === null) throw new Error('settings panel has no bounding box')

  // Top edges flush, right edges flush — "the panel's top-right corner
  // sits exactly where the button's top-right corner was."
  expect(panelBox.y).toBeCloseTo(buttonBox.y, 0)
  expect(panelBox.x + panelBox.width).toBeCloseTo(buttonBox.x + buttonBox.width, 0)
})

// Empty state's "From your desktop…" opening the in-app scanner (rather
// than a fake-action toast or a "point your own camera app" explainer) is
// covered by the "scanner:" tests above, which need the fake-camera
// `addInitScript` those carry — this test only needs the NO-fake-camera
// branch (CI's real headless browser, no fake stream installed), proving
// SOME honest denied/no-camera copy renders instead of pretending a scan
// started. Whether headless CI's real getUserMedia lands on 'denied'
// (permission never granted) or 'unavailable' (no hardware/device found)
// is a CI-environment detail this test deliberately doesn't pin down —
// ScanSheet.test.tsx's unit tests already cover each state precisely with
// a controlled rejection reason; both states share the SAME fallback line,
// asserted here.
test('empty state: "From your desktop…" with no real camera shows an honest fallback message', async ({ page }) => {
  // A bare boot with nothing loaded (unlike bootShopModeWith, which always
  // loads a fixture) — this test needs the actual empty state.
  await page.addInitScript(() => localStorage.setItem('hew.settings.lengthUnit', 'm'))
  await page.addInitScript((key) => localStorage.setItem(key, '1'), HINT_TEST_SUPPRESS_KEY)
  await page.goto('about:blank')
  await page.goto('/#shop')
  await page.waitForFunction(() => window.__hew_shop_test?.isReady() === true, null, { timeout: 15_000 })

  const ghostButton = page.getByRole('button', { name: /^from your desktop…$/i })
  await expect(ghostButton).toBeVisible()
  await ghostButton.tap()

  const sheet = page.getByRole('dialog', { name: 'Scan from desktop' })
  await expect(sheet).toBeVisible()
  await expect(sheet).toContainText(/camera app instead/i)

  await page.getByRole('button', { name: /^cancel$/i }).tap()
  await expect(sheet).not.toBeVisible()
})

// ---------------------------------------------------------------------------
// Tape Measure loupe (round-3 playtest finding 4): press-and-hold a Tape
// Measure point instead of tapping it straight down, for fine positioning on
// thin parts / clustered endpoints a fingertip can't reliably hit. Real
// touch (CDP `Input.dispatchTouchEvent`, `camera-playtest2.spec.ts`'s own
// precedent, chromium-only) for the camera-orbit-suppression claims — a
// `page.mouse` hold would be VACUOUS there (Tape Measure already nulls
// `controls.mouseButtons.LEFT`, so a mouse drag can't orbit regardless of
// this fix; only OrbitControls' separate `touches.ONE` binding can, and
// only a genuine `pointerType: 'touch'` event exercises it). The
// commit/marker and quick-tap/drag-reject outcomes don't depend on
// pointerType (`Viewport.tsx`'s loupe wiring never branches on it), so
// those use `page.mouse` per this file's own long-press-isolate precedent.
// ---------------------------------------------------------------------------

test.describe('Tape Measure loupe', () => {
  /** Switch to Tape Measure, pin `cam` (default `PINNED_CAMERA`, the shared
   *  framing every other test in this block uses), and return the
   *  page-pixel `world` projects to. Shared by `setUpTapeMeasure` (the
   *  fixed near-top corner) and the finding-6 test below, which pins its
   *  OWN much closer camera — under `PINNED_CAMERA`'s wide iso framing the
   *  unit box's whole top face spans barely ~10px on screen (adjacent edge
   *  midpoints measured ~11px apart), leaving no room between the widened
   *  touch-aperture's sticky pull (up to `LOUPE_SLOP_PX`-and-then-some) and
   *  the face's own edges for a "plain interior, no snap candidate nearby"
   *  point to exist at all. */
  async function setUpTapeMeasureAt(
    page: Page,
    world: { x: number; y: number; z: number },
    cam: CameraParams = PINNED_CAMERA,
  ): Promise<{ x: number; y: number }> {
    const { bytes } = await buildSingleBoxFixture(page)
    await bootShopModeWith(page, bytes)
    await page.getByRole('button', { name: /^tape measure$/i }).click()
    await page.evaluate((c) => window.__hew_shop_test!.setCamera(c), {
      position: [cam.position.x, cam.position.y, cam.position.z],
      target: [cam.target.x, cam.target.y, cam.target.z],
      up: [cam.up.x, cam.up.y, cam.up.z],
      fovDeg: cam.fovDeg,
    })
    await settleFrame(page)
    const canvasBox = await page.locator('canvas').first().boundingBox()
    if (canvasBox === null) throw new Error('viewport canvas has no bounding box')
    const rect = { left: canvasBox.x, top: canvasBox.y, width: canvasBox.width, height: canvasBox.height }
    const vp = buildViewProjection(cam, rect.width / rect.height)
    const px = worldToPagePixel(world, vp, rect)
    if (px === null) throw new Error('the requested world point does not project onto the canvas under the pinned camera')
    return px
  }

  /** Switch to Tape Measure, pin the camera, and return a page-pixel target
   *  on the fixture box's near-top-corner-ish face — thin/precise enough to
   *  stand in for the "clustered endpoints" case the loupe exists for. */
  async function setUpTapeMeasure(page: Page): Promise<{ x: number; y: number }> {
    // The box's near-top corner (1,1,1) — a real vertex, standing in for a
    // "clustered endpoint" fat-finger target.
    return setUpTapeMeasureAt(page, { x: 1, y: 1, z: 1 })
  }

  test('a held touch engages the loupe without orbiting the camera; release commits a tape anchor and the copied canvas is non-blank', async ({ page }, testInfo) => {
    test.skip(testInfo.project.name !== 'chromium', 'CDP touch dispatch is Chromium-only')
    const px = await setUpTapeMeasure(page)
    const cameraBefore = await page.evaluate(() => window.__hew_shop_test!.getCameraPose())
    const loupe = page.getByTestId('tape-loupe')
    await expect(loupe).toBeHidden()

    const client = await page.context().newCDPSession(page)
    await client.send('Input.dispatchTouchEvent', { type: 'touchStart', touchPoints: [{ x: px.x, y: px.y }] })
    // Past LOUPE_HOLD_MS (300ms) with the finger held still.
    await page.waitForTimeout(450)

    await expect(loupe).toBeVisible()
    // Held perfectly still, past the hold threshold — OrbitControls must
    // not have moved the camera at all (`controls.enabled = false` for the
    // gesture's duration, engaged the instant the timer fired).
    const cameraDuringHold = await page.evaluate(() => window.__hew_shop_test!.getCameraPose())
    expectCameraUnchanged(cameraDuringHold, cameraBefore)

    // Evidence for the drawImage copy technique: sample the loupe's OWN 2D
    // canvas for non-blank content (a real WebGL frame was copied in, not
    // an empty/cleared buffer) — at least one pixel with nonzero alpha and
    // some color variation, ruling out a canvas that was merely filled with
    // one flat, uninformative color.
    const loupeHasContent = await page.evaluate(() => {
      const overlay = document.querySelector('[data-testid="tape-loupe"]')
      const canvas = overlay?.querySelector('canvas') as HTMLCanvasElement | null | undefined
      const ctx = canvas?.getContext('2d')
      if (ctx == null || canvas == null) return false
      const { data } = ctx.getImageData(0, 0, canvas.width, canvas.height)
      let firstOpaque: number | null = null
      for (let i = 0; i < data.length; i += 4) {
        if (data[i + 3] === 0) continue
        if (firstOpaque === null) { firstOpaque = i; continue }
        if (data[i] !== data[firstOpaque] || data[i + 1] !== data[firstOpaque + 1] || data[i + 2] !== data[firstOpaque + 2]) {
          return true // two differently-colored opaque pixels — real image content
        }
      }
      return false
    })
    expect(loupeHasContent).toBe(true)

    // Fine-position slightly, then release — commits at the probed point.
    await client.send('Input.dispatchTouchEvent', { type: 'touchMove', touchPoints: [{ x: px.x + 4, y: px.y - 3 }] })
    await page.waitForTimeout(50)
    await client.send('Input.dispatchTouchEvent', { type: 'touchEnd', touchPoints: [] })
    await settleFrame(page)

    await expect(loupe).toBeHidden()
    const cameraAfter = await page.evaluate(() => window.__hew_shop_test!.getCameraPose())
    expectCameraUnchanged(cameraAfter, cameraBefore) // never orbited, start to finish
    await expect(page.getByTestId('tape-anchor-marker')).toHaveCount(1)
  })

  test('pointercancel mid-hold disengages with no commit and restores the camera to normal orbit', async ({ page }, testInfo) => {
    test.skip(testInfo.project.name !== 'chromium', 'CDP touch dispatch is Chromium-only')
    const px = await setUpTapeMeasure(page)
    const loupe = page.getByTestId('tape-loupe')

    const client = await page.context().newCDPSession(page)
    await client.send('Input.dispatchTouchEvent', { type: 'touchStart', touchPoints: [{ x: px.x, y: px.y }] })
    await page.waitForTimeout(450)
    await expect(loupe).toBeVisible()

    await client.send('Input.dispatchTouchEvent', { type: 'touchCancel', touchPoints: [] })
    await settleFrame(page)

    await expect(loupe).toBeHidden()
    await expect(page.getByTestId('tape-anchor-marker')).toHaveCount(0)

    // Controls provably restored: a FRESH touch drag on Select orbits again
    // (reusing the CONTRACT test's own proof technique) rather than staying
    // stuck disabled.
    await page.getByRole('button', { name: 'Select', exact: true }).click()
    const cameraBefore = await page.evaluate(() => window.__hew_shop_test!.getCameraPose())
    await client.send('Input.dispatchTouchEvent', { type: 'touchStart', touchPoints: [{ x: px.x, y: px.y }] })
    await client.send('Input.dispatchTouchEvent', { type: 'touchMove', touchPoints: [{ x: px.x + 70, y: px.y + 10 }] })
    await page.waitForTimeout(50)
    await client.send('Input.dispatchTouchEvent', { type: 'touchEnd', touchPoints: [] })
    await settleFrame(page)
    const cameraAfter = await page.evaluate(() => window.__hew_shop_test!.getCameraPose())
    expect(cameraAfter).not.toEqual(cameraBefore)
  })

  test('a quick tap (released before the hold engages) commits immediately — no loupe, unchanged from a plain tap', async ({ page }) => {
    const px = await setUpTapeMeasure(page)

    // page.mouse: this outcome doesn't depend on pointerType at all (module
    // doc) — the long-press-isolate precedent for a non-camera assertion.
    await page.mouse.move(px.x, px.y)
    await page.mouse.down()
    await page.mouse.up()
    await settleFrame(page)

    await expect(page.getByTestId('tape-loupe')).toBeHidden()
    await expect(page.getByTestId('tape-anchor-marker')).toHaveCount(1)
  })

  // Adversarial-review finding 6: a quick tap (armed, released before the
  // hold engages) must commit at the FROZEN press-time snap, not a live
  // probe that ordinary touch jitter can drift onto a different point —
  // `onPointerMove` overwrites the live probe on every move regardless of
  // phase, so pre-fix, a sub-slop jitter during a quick tap silently
  // committed wherever the finger happened to be at release, not where it
  // first touched down. Proven on the plain INTERIOR of the top face (no
  // vertex/edge/midpoint candidate nearby to snap onto and mask the
  // difference — `SnapKind::OnFace` is exact, unsnapped ray-plane
  // intersection in the kernel), so the committed screen position tracks
  // the ray pixel-for-pixel and the press-vs-jitter outcomes are cleanly
  // distinguishable.
  /** A close-up camera looking down at the fixture box's top face — under
   *  `PINNED_CAMERA`'s own wide iso framing (shared by every other test in
   *  this block), the unit box's whole top face spans barely ~10 screen
   *  px, leaving no room between the (touch-widened) snap aperture's sticky
   *  pull and the face's own edges for an unsnapped "plain interior" point
   *  to exist. This one is close enough that the face fills a comfortable
   *  fraction of the canvas instead. */
  const CLOSE_FACE_CAMERA: CameraParams = {
    position: { x: 0.5, y: -1, z: 2.5 },
    target: { x: 0.5, y: 0.5, z: 1 },
    up: { x: 0, y: 0, z: 1 }, // Hew world-up is +Z, same as PINNED_CAMERA
    fovDeg: 50,
    near: 0.1,
    far: 1000,
  }

  test('a quick tap with a few pixels of jitter commits at the ORIGINAL press position, not the live-jittered probe', async ({ page }) => {
    // Comfortably inland from every edge of the top face ([0,1]x[0,1] at
    // z=1, world units) — even under the coarse-pointer-doubled aperture,
    // nowhere near an on-edge/on-vertex snap that would pull the commit
    // onto a fixed feature regardless of this fix (which would make the
    // press and jittered outcomes indistinguishable no matter which one the
    // code actually used).
    const px = await setUpTapeMeasureAt(page, { x: 0.35, y: 0.65, z: 1 }, CLOSE_FACE_CAMERA)

    /** One quick tap at `px`, optionally drifting `jitterPx` STRAIGHT ALONG
     *  X (screen px — a single-axis drift so its total Euclidean distance
     *  from the press point IS `jitterPx`, kept under `LOUPE_SLOP_PX` so
     *  the hold stays `armed`, never `rejected` — a diagonal `(j, j)` drift
     *  would instead travel `j*sqrt(2)`, silently crossing the slop
     *  threshold) before releasing — returns the committed marker's screen
     *  position. Switches to Select and back to Tape Measure first, which
     *  (`switchToolRef`) constructs a brand-new `TapeMeasureTool` instance,
     *  giving each rep a clean p0 slot rather than accumulating a
     *  two-point measurement across reps. */
    async function tapAndReadMarker(jitterPx: number): Promise<{ x: number; y: number }> {
      await page.getByRole('button', { name: 'Select', exact: true }).click()
      await page.getByRole('button', { name: /^tape measure$/i }).click()
      await page.mouse.move(px.x, px.y)
      await page.mouse.down()
      if (jitterPx !== 0) await page.mouse.move(px.x + jitterPx, px.y, { steps: 4 })
      await page.mouse.up()
      await settleFrame(page)
      const marker = page.getByTestId('tape-anchor-marker')
      await expect(marker).toHaveCount(1)
      return {
        x: Number(await marker.getAttribute('cx')),
        y: Number(await marker.getAttribute('cy')),
      }
    }

    const clean = await tapAndReadMarker(0)
    const jittered = await tapAndReadMarker(7) // < LOUPE_SLOP_PX (10)

    // Both taps pressed at the exact SAME pixel (`px`) — a jitter well
    // under the slop threshold during an `armed` (never `engaged`) hold
    // must commit at that same frozen press-time point either way. Pre-fix,
    // the jittered rep instead committed the LIVE probe (last resolved at
    // the drifted pixel on every intervening pointermove), landing several
    // pixels away from the clean rep's own marker — a real, measurable
    // divergence on this snap-free interior-of-face target (no vertex/edge
    // candidate to mask it by pulling both onto the same point).
    expect(Math.hypot(jittered.x - clean.x, jittered.y - clean.y)).toBeLessThan(1)
  })

  test('a drag that breaks the hold before engaging orbits the camera and commits NO point — a drag is a camera gesture in Tape mode', async ({ page }) => {
    const px = await setUpTapeMeasure(page)
    const cameraBefore = await page.evaluate(() => window.__hew_shop_test!.getCameraPose())

    await page.mouse.move(px.x, px.y)
    await page.mouse.down()
    // Well past LOUPE_SLOP_PX (10px), and fast enough that LOUPE_HOLD_MS
    // (300ms) can't have elapsed first — a real drag, not a hold.
    await page.mouse.move(px.x + 60, px.y + 40, { steps: 10 })
    await page.mouse.up()
    await settleFrame(page)

    await expect(page.getByTestId('tape-loupe')).toBeHidden()
    // Shop-mode playtest: a drag is purely a CAMERA gesture in Tape mode — it
    // orbits and drops NO point. The reject-commit this used to assert (a drag
    // committing a point at the press position) was removed, since it made the
    // camera unusable while measuring.
    await expect(page.getByTestId('tape-anchor-marker')).toHaveCount(0)
    const cameraAfter = await page.evaluate(() => window.__hew_shop_test!.getCameraPose())
    expect(cameraAfter).not.toEqual(cameraBefore)
  })

  // Adversarial-review finding 4: a second, independent pointer must never
  // touch a loupe gesture it didn't arm — mirrors camera-playtest2.spec.ts's
  // own "second concurrent pointer" precedent (a real mouse press for the
  // arming pointer, CDP-dispatched touch for the wholly independent one).
  test('a second, independent pointer cannot hijack an in-progress loupe — only the arming pointer can commit or cancel it', async ({ page }, testInfo) => {
    test.skip(testInfo.project.name !== 'chromium', 'CDP touch dispatch is Chromium-only')
    const px = await setUpTapeMeasure(page)
    const loupe = page.getByTestId('tape-loupe')

    // Arm + engage the loupe with a real mouse press — pointerType doesn't
    // matter for engagement itself (module doc above).
    await page.mouse.move(px.x, px.y)
    await page.mouse.down()
    await page.waitForTimeout(450) // past LOUPE_HOLD_MS (300ms)
    await expect(loupe).toBeVisible()

    // A SECOND, wholly independent contact presses, moves, and releases
    // entirely elsewhere on the canvas WHILE the first pointer is still
    // held. Pre-fix, this stray pointer's own down would reach
    // `armTapeLoupe`'s defensive `cancelTapeLoupe()` reset (silently
    // cancelling the first pointer's still-live gesture and re-arming on
    // the SECOND pointer's own position), and its own up would resolve the
    // commit path unconditionally.
    const client = await page.context().newCDPSession(page)
    await client.send('Input.dispatchTouchEvent', { type: 'touchStart', touchPoints: [{ x: px.x + 80, y: px.y + 80 }] })
    await client.send('Input.dispatchTouchEvent', { type: 'touchMove', touchPoints: [{ x: px.x + 120, y: px.y + 120 }] })
    await client.send('Input.dispatchTouchEvent', { type: 'touchEnd', touchPoints: [] })
    await page.waitForTimeout(30)

    // The FIRST pointer's loupe is still engaged, completely untouched by
    // the stray contact's entire down/move/up cycle.
    await expect(loupe).toBeVisible()
    await expect(page.getByTestId('tape-anchor-marker')).toHaveCount(0)

    // The first pointer's own release still commits normally.
    await page.mouse.up()
    await settleFrame(page)
    await expect(loupe).toBeHidden()
    await expect(page.getByTestId('tape-anchor-marker')).toHaveCount(1)
  })

  // Adversarial-review finding 5: a document swap landing mid-loupe (the QR
  // receive path's real trigger — it resolves on a background fetch) must
  // cancel the gesture and restore camera controls, not strand them.
  test('loading a new document mid-loupe cancels the gesture and restores camera controls', async ({ page, context }, testInfo) => {
    test.skip(testInfo.project.name !== 'chromium', 'CDP touch dispatch is Chromium-only')

    // A SECOND document's bytes, built on a THROWAWAY page — the primary
    // page below must stay parked on Shop Mode with the loupe gesture
    // still live; `buildSingleBoxFixture` navigates through the EDITOR
    // shell to draw its fixture, which would tear down that gesture (and
    // the whole Shop Mode session) rather than merely handing it a second
    // document to load.
    const scratchPage = await context.newPage()
    const { bytes: secondBytes } = await buildSingleBoxFixture(scratchPage)
    await scratchPage.close()

    const px = await setUpTapeMeasure(page)
    const loupe = page.getByTestId('tape-loupe')

    const client = await page.context().newCDPSession(page)
    await client.send('Input.dispatchTouchEvent', { type: 'touchStart', touchPoints: [{ x: px.x, y: px.y }] })
    await page.waitForTimeout(450)
    await expect(loupe).toBeVisible()

    // The document swap lands mid-hold — the harness's own `load` runs
    // through the SAME `applyOpenedBytes`/`notifyLoaded` path the real QR
    // receive flow does.
    await page.evaluate((arr) => window.__hew_shop_test!.load(arr), secondBytes)

    await expect(loupe).toBeHidden()
    await client.send('Input.dispatchTouchEvent', { type: 'touchEnd', touchPoints: [] })

    // Controls provably restored: a fresh touch drag on Select orbits again
    // (the pointercancel test's own proof technique) rather than staying
    // stuck disabled.
    await page.getByRole('button', { name: 'Select', exact: true }).click()
    const cameraBefore = await page.evaluate(() => window.__hew_shop_test!.getCameraPose())
    await client.send('Input.dispatchTouchEvent', { type: 'touchStart', touchPoints: [{ x: px.x, y: px.y }] })
    await client.send('Input.dispatchTouchEvent', { type: 'touchMove', touchPoints: [{ x: px.x + 70, y: px.y + 10 }] })
    await page.waitForTimeout(50)
    await client.send('Input.dispatchTouchEvent', { type: 'touchEnd', touchPoints: [] })
    await settleFrame(page)
    const cameraAfter = await page.evaluate(() => window.__hew_shop_test!.getCameraPose())
    expect(cameraAfter).not.toEqual(cameraBefore)
  })

  // Adversarial-review finding 9: a second finger reaching the dock's tool
  // buttons while the first still holds the canvas mid-loupe must not
  // strand the gesture.
  test('switching tools (a second finger reaching the dock) while the loupe is engaged cancels it and restores camera controls', async ({ page }, testInfo) => {
    test.skip(testInfo.project.name !== 'chromium', 'CDP touch dispatch is Chromium-only')
    const px = await setUpTapeMeasure(page)
    const loupe = page.getByTestId('tape-loupe')

    const client = await page.context().newCDPSession(page)
    await client.send('Input.dispatchTouchEvent', { type: 'touchStart', touchPoints: [{ x: px.x, y: px.y }] })
    await page.waitForTimeout(450)
    await expect(loupe).toBeVisible()

    // A second contact reaches the dock and taps Select — a genuinely
    // separate pointer/click, exactly like a real second finger would —
    // WHILE the first still holds the still-engaged loupe.
    await page.getByRole('button', { name: 'Select', exact: true }).click()

    await expect(loupe).toBeHidden()
    await client.send('Input.dispatchTouchEvent', { type: 'touchEnd', touchPoints: [] })

    // Controls provably restored, same proof technique as above.
    const cameraBefore = await page.evaluate(() => window.__hew_shop_test!.getCameraPose())
    await client.send('Input.dispatchTouchEvent', { type: 'touchStart', touchPoints: [{ x: px.x, y: px.y }] })
    await client.send('Input.dispatchTouchEvent', { type: 'touchMove', touchPoints: [{ x: px.x + 70, y: px.y + 10 }] })
    await page.waitForTimeout(50)
    await client.send('Input.dispatchTouchEvent', { type: 'touchEnd', touchPoints: [] })
    await settleFrame(page)
    const cameraAfter = await page.evaluate(() => window.__hew_shop_test!.getCameraPose())
    expect(cameraAfter).not.toEqual(cameraBefore)
  })
})

// ---------------------------------------------------------------------------
// Scenes (docs/design/scenes.md §6, SPEC.md §2) — the pill, the Views
// sheet's SCENES section, chevron cycling, and the SAME read-only contract
// (bounds unchanged, sessionDepth 0) the CONTRACT tests above already prove
// for the rest of Shop Mode, now proven to hold across a Scene activation
// too. Fixture built through the EDITOR harness's `addScene` (`harness.ts`)
// — captures the CURRENT (pinned) camera under the full property bitmask.
// ---------------------------------------------------------------------------

/** Two boxes, two Scenes ("Cut layout" then "Tenon section"), camera pinned
 *  before each capture — the editor harness's `addScene` reads whatever the
 *  viewport's CURRENT camera is (mirrors `useScenesController.ts`'s own
 *  `add`), so this fixture pins a DIFFERENT camera per Scene rather than
 *  capturing the same pose twice, which would make an activation's own
 *  camera tween unobservable. Returns the saved bytes and Object 1's own
 *  kernel id, for the read-only-contract bounds check below. */
async function buildSceneFixture(page: Page): Promise<{ bytes: number[]; objectId: string }> {
  await page.addInitScript(() => localStorage.setItem('hew:shellMode', 'editor'))
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
  return page.evaluate(() => {
    const h = window.__hew_test!
    const objectId = h.drawBox([0, 0, 0], [1, 1, 0], 1)
    h.setCamera({ position: [8, 6, 8], target: [0, 0, 0], up: [0, 0, 1], fovDeg: 45 })
    h.addScene('Cut layout')
    h.setCamera({ position: [-8, 6, 8], target: [0, 0, 0], up: [0, 0, 1], fovDeg: 35 })
    h.addScene('Tenon section')
    return { bytes: h.save(), objectId }
  })
}

test('Scenes: the Views sheet lists them, activating shows the pill and tweens the camera, chevrons cycle with wrap, and the read-only contract still holds', async ({ page }) => {
  const { bytes, objectId } = await buildSceneFixture(page)
  await bootShopModeWith(page, bytes)

  const boundsBefore = await page.evaluate((id) => window.__hew_shop_test!.getObjectBounds(id), objectId)

  // A document with Scenes opens on its FIRST Scene (playtest round 1,
  // matching the desktop): the pill is already up naming "Cut layout" and
  // the camera sits at that Scene's pinned pose — instantly, no tween.
  const pill = page.getByTestId('scene-pill')
  await expect(pill).toBeVisible()
  await expect(pill).toContainText('Cut layout')
  await expect(async () => {
    const cam = await page.evaluate(() => window.__hew_shop_test!.getCameraPose())
    expect(cam.position[0]).toBeCloseTo(8, 1)
    expect(cam.position[1]).toBeCloseTo(6, 1)
    expect(cam.position[2]).toBeCloseTo(8, 1)
  }).toPass({ timeout: 2000 })
  const cameraBefore = await page.evaluate(() => window.__hew_shop_test!.getCameraPose())

  // The Views sheet's SCENES section lists both, above the standard views.
  await page.getByRole('button', { name: /^views$/i }).click()
  const dialog = page.getByRole('dialog', { name: 'Views' })
  await expect(dialog).toBeVisible()
  await expect(dialog.getByText('Cut layout')).toBeVisible()
  await expect(dialog.getByText('Tenon section')).toBeVisible()

  // Activating "Tenon section" act-and-closes the sheet, the pill follows,
  // and the camera tweens away from the first Scene's pose.
  await dialog.getByText('Tenon section').click()
  await expect(dialog).not.toBeVisible()
  await expect(pill).toContainText('Tenon section')
  await expect(async () => {
    const cam = await page.evaluate(() => window.__hew_shop_test!.getCameraPose())
    expect(Math.hypot(cam.position[0] - 8, cam.position[1] - 6, cam.position[2] - 8)).toBeGreaterThan(0.5)
  }).toPass({ timeout: 2000 })

  // Chevrons cycle with wrap (SPEC.md §2 "Active-Scene pill").
  await page.getByRole('button', { name: 'Next Scene' }).click() // wraps back
  await expect(pill).toContainText('Cut layout')
  await page.getByRole('button', { name: 'Next Scene' }).click()
  await expect(pill).toContainText('Tenon section')
  await page.getByRole('button', { name: 'Previous Scene' }).click()
  await expect(pill).toContainText('Cut layout')

  // Tapping the pill's own name reopens the Views sheet; tapping a row
  // there activates too (not just the chevrons).
  await pill.getByText('Cut layout').click()
  await expect(dialog).toBeVisible()
  await dialog.getByText('Tenon section').click()
  await expect(dialog).not.toBeVisible()
  await expect(pill).toContainText('Tenon section')

  // CONTRACT (same proof technique as the CONTRACT tests above): Shop Mode
  // issues zero kernel transactions, so a Scene activation — a renderer/
  // camera-only affair (`resolve_scene`, never `apply_scene`) — must not
  // have moved the object or opened a session, even though the CAMERA
  // itself legitimately changed.
  const boundsAfter = await page.evaluate((id) => window.__hew_shop_test!.getObjectBounds(id), objectId)
  expect(boundsAfter).toEqual(boundsBefore)
  expect(await page.evaluate(() => window.__hew_shop_test!.sessionDepth())).toBe(0)

  // The camera itself DID change (proof this was a real activation, not a
  // no-op) — contrasted against the pre-activation pose captured above.
  const cameraAfter = await page.evaluate(() => window.__hew_shop_test!.getCameraPose())
  expect(cameraAfter).not.toEqual(cameraBefore)
})

// ---------------------------------------------------------------------------
// Landscape (design_handoff_shop_mode/README.md §5, corrected by Kurt's
// on-device playtest — item 8): the right rail keeps its own composition,
// but the old left-edge side sheet (tab <-> 340px panel) is gone entirely.
// Landscape now uses the SAME bottom sheet as portrait — `dragSheetHandle`
// (module doc above) drives it exactly like the portrait tests already do,
// no separate tab-toggle helper needed anymore. `test.describe`'s own
// `test.use` below overrides ONLY `viewport` from the file-level `test.use`
// at the top — `hasTouch`/`isMobile` (and every other real-touch-dispatch
// note in this file's module doc) still apply here too.
// ---------------------------------------------------------------------------

test.describe('landscape', () => {
  test.use({ viewport: { width: 844, height: 390 } })

  test('boots into the right rail + a centered bottom sheet at peek height, not the old side sheet', async ({ page }) => {
    const bytes = await buildFixtureBytes(page)
    await bootShopModeWith(page, bytes)

    // The right rail's tools (icon-only in landscape) share the SAME
    // accessible names as the portrait dock's — the E2E's own role queries
    // must keep working across a rotation without changing selector.
    await expect(page.getByRole('button', { name: 'Zoom Extents' })).toBeVisible()
    await expect(page.getByRole('button', { name: 'Select' })).toBeVisible()
    await expect(page.getByRole('button', { name: 'Orbit' })).toBeVisible()
    await expect(page.getByRole('button', { name: /^tape measure$/i })).toBeVisible()

    // The bottom sheet's own drag handle is present (unlike the old side
    // sheet, which had none — a tab instead), at its 64px peek height.
    // (Not asserting row-text invisibility here: Playwright's `toBeVisible`
    // doesn't account for an ANCESTOR's `overflow:hidden` clip — the peek
    // height visually clips the row list in a real browser, but the row
    // elements still have a nonzero, in-viewport bounding box, so Playwright
    // reports them "visible" regardless. The "opens on drag" test below is
    // the real, non-vacuous proof that a drag is what reveals them.)
    await expect(page.getByTestId('parts-sheet-handle')).toBeVisible()
    await expect(page.getByTestId('parts-sheet')).toHaveCSS('height', '64px')

    // The old side sheet's tab is gone entirely — dead code, not just
    // hidden (item 8: "remove dead code, don't strand it").
    await expect(page.getByTestId('shop-side-sheet-tab')).toHaveCount(0)
    await expect(page.getByRole('button', { name: /cutlist/i })).toHaveCount(0)
  })

  test('dragging the handle opens the bottom sheet, and a row\'s eye toggle hides it there', async ({ page }) => {
    const bytes = await buildFixtureBytes(page)
    await bootShopModeWith(page, bytes)

    // A 300px pull comfortably clears "full" for a 390px-tall landscape
    // viewport (portrait's own equivalent test pulls 600px against an
    // 844px-tall one — the same fractions, scaled to the shorter height).
    await dragSheetHandle(page, -300)

    const sheet = page.getByTestId('parts-sheet')
    await expect(sheet.getByText('Object 2', { exact: true })).toBeVisible()

    const eyeObject2 = sheet.getByRole('button', { name: 'Hide Object 2' })
    await eyeObject2.tap()
    await expect(sheet.getByRole('button', { name: 'Show Object 2' })).toBeVisible()
  })

  test('rotating back to portrait restores the dock/bottom sheet and preserves the hide', async ({ page }) => {
    const bytes = await buildFixtureBytes(page)
    await bootShopModeWith(page, bytes)

    await dragSheetHandle(page, -300)
    const sheet = page.getByTestId('parts-sheet')
    await sheet.getByRole('button', { name: 'Hide Object 2' }).tap()
    await expect(sheet.getByRole('button', { name: 'Show Object 2' })).toBeVisible()

    // The one rotation this file exercises via a live viewport resize
    // (task instructions) — `orientation.ts`'s `resize` listener re-decides
    // portrait/landscape from the new dimensions.
    await page.setViewportSize({ width: 390, height: 844 })

    // Portrait dock is back — exactly one "Zoom Extents" button (the
    // landscape rail's own copy is gone, not just covered).
    await expect(page.getByRole('button', { name: 'Zoom Extents' })).toHaveCount(1)

    // The detent carried over as OPEN (the lifted `detent` state —
    // `ShopApp.tsx`), so the portrait sheet already shows its rows without
    // a drag.
    const portraitSheet = page.getByTestId('parts-sheet')
    await expect(portraitSheet.getByText('Object 1', { exact: true })).toBeVisible()
    // The hide from landscape survived the rotation.
    await expect(portraitSheet.getByRole('button', { name: 'Show Object 2' })).toBeVisible()
  })

  test('the unit picker renders as a centered card', async ({ page }) => {
    const bytes = await buildFixtureBytes(page)
    await bootShopModeWith(page, bytes)

    // The header (title + unit chip) sits within the sheet's fixed peek
    // height, unclipped at ANY detent — no need to open the sheet first.
    await page.getByRole('button', { name: /^units: meters$/i }).tap()

    const picker = page.getByRole('dialog', { name: 'Units' })
    await expect(picker).toBeVisible()
    // The landscape "centered 360px card" (design §7) — a fixed width,
    // unlike the portrait bottom sheet's full-bleed `left/right: 0`.
    await expect(picker).toHaveCSS('width', '360px')
  })

  // Round-3 playtest finding 3, landscape half — same corner-anchor contract
  // as the portrait tests above, at landscape's own edge offsets
  // (`LANDSCAPE_LEFT_OFFSET_CSS`/`LANDSCAPE_RIGHT_OFFSET_CSS`, shared by the
  // pill/⋯ button's own row and their respective panels).
  test('the document panel occupies the pill\'s own top-left corner in landscape too', async ({ page }) => {
    const bytes = await buildFixtureBytes(page)
    await bootShopModeWith(page, bytes)

    const pill = page.getByRole('button', { name: /^document menu/i })
    const pillBox = await pill.boundingBox()
    if (pillBox === null) throw new Error('pill has no bounding box')

    await pill.tap()
    const panel = page.getByTestId('shop-document-panel')
    await expect(panel).toBeVisible()
    const panelBox = await panel.boundingBox()
    if (panelBox === null) throw new Error('document panel has no bounding box')

    expect(panelBox.y).toBeCloseTo(pillBox.y, 0)
    expect(panelBox.x).toBeCloseTo(pillBox.x, 0)
  })

  test('the settings panel occupies the ⋯ button\'s own top-right corner in landscape too', async ({ page }) => {
    const bytes = await buildFixtureBytes(page)
    await bootShopModeWith(page, bytes)

    const ellipsis = page.getByRole('button', { name: 'Settings' })
    const buttonBox = await ellipsis.boundingBox()
    if (buttonBox === null) throw new Error('⋯ button has no bounding box')

    await ellipsis.tap()
    const panel = page.getByTestId('shop-settings-panel')
    await expect(panel).toBeVisible()
    const panelBox = await panel.boundingBox()
    if (panelBox === null) throw new Error('settings panel has no bounding box')

    expect(panelBox.y).toBeCloseTo(buttonBox.y, 0)
    expect(panelBox.x + panelBox.width).toBeCloseTo(buttonBox.x + buttonBox.width, 0)
  })

  test('the right rail is translucent and hugs the safe-area-inset-right edge', async ({ page }) => {
    const bytes = await buildFixtureBytes(page)
    await bootShopModeWith(page, bytes)

    const rail = page.getByRole('button', { name: 'Zoom Extents' }).locator('xpath=..')
    const railBg = await rail.evaluate((el) => getComputedStyle(el).backgroundColor)
    expect(cssAlpha(railBg)).toBeGreaterThan(0.8)
    expect(cssAlpha(railBg)).toBeLessThan(0.95)

    // No device-frame cutout under Playwright, so
    // env(safe-area-inset-right) is 0 — the rail's right edge should sit
    // exactly 16px off the true viewport edge (item 9's hardened
    // `calc(env(safe-area-inset-right, 0px) + 16px)`; a real notched
    // device's nonzero inset isn't reproducible under Playwright, so that
    // half of the fix isn't independently verified here).
    const railBox = await rail.boundingBox()
    if (railBox === null) throw new Error('rail has no bounding box')
    const viewportWidth = page.viewportSize()!.width
    expect(viewportWidth - (railBox.x + railBox.width)).toBeCloseTo(16, 0)
  })
})

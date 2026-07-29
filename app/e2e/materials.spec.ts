import { test, expect } from '@playwright/test'

/**
 * Materials pane wiring — the cross-layer behavior a component test can't reach
 * (App → Viewport → ToolController → PaintTool).
 *
 * Contract: picking a material in the palette makes it current AND activates
 * the Paint tool, so the next click paints with it (Ctrl/Cmd-click fills the
 * whole object). This replaced the old "Fill selected object" button, which
 * must no longer exist.
 *
 * Also covers the eyedropper (Alt-sample) and Shift-click "replace
 * everywhere" gestures (paint-tool design §1–2), driven with real mouse
 * clicks and held keyboard modifiers (`page.keyboard.down`/`up` around
 * `page.mouse.click` — Playwright reflects a held key on the synthetic mouse
 * event's `altKey`/`shiftKey`, exactly like a real chord) rather than the
 * harness's logic-only `paintFace`/`pickFace`, since the modifiers
 * themselves are what's under test.
 */

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

test.beforeEach(async ({ page }) => {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
})

test('selecting a material activates the Paint tool; no Fill button remains', async ({ page }) => {
  // Reveal the Materials tray if collapsed.
  const defaultSwatch = page.getByTitle('Default (unpainted)')
  if (!(await defaultSwatch.isVisible().catch(() => false))) {
    await page.getByText('Materials', { exact: true }).first().click()
  }
  await expect(defaultSwatch).toBeVisible()

  // The removed button must not be present.
  await expect(page.getByRole('button', { name: /fill selected object/i })).toHaveCount(0)

  // Picking a swatch activates Paint — proven by the Paint tool's own status
  // hint (only shown when PaintTool is the active tool) appearing.
  await defaultSwatch.click()
  await expect(page.getByText(/Click a face to paint it with the current material/i)).toBeVisible()
})

/** Reveals the Materials tray if it's collapsed (same toggle the first test
 *  uses), so every test below can rely on the swatches being visible. */
async function revealMaterialsTray(page: import('@playwright/test').Page): Promise<void> {
  const defaultSwatch = page.getByTitle('Default (unpainted)')
  if (!(await defaultSwatch.isVisible().catch(() => false))) {
    await page.getByText('Materials', { exact: true }).first().click()
  }
  await expect(defaultSwatch).toBeVisible()
}

test('filtering the materials list narrows by name, keeps the Default row, and clears back', async ({ page }) => {
  await revealMaterialsTray(page)
  const defaultSwatch = page.getByTitle('Default (unpainted)')

  // Add two colors through the Add-color flow so there is something to filter.
  await page.getByRole('button', { name: 'Add color', exact: true }).click()
  await page.getByLabel('Choose color').fill('#ff0000')
  await page.getByPlaceholder('Name…').first().fill('Red Paint')
  await page.getByRole('button', { name: '+ Add color' }).click()

  await page.getByLabel('Choose color').fill('#0000ff')
  await page.getByPlaceholder('Name…').first().fill('Sky Blue')
  await page.getByRole('button', { name: '+ Add color' }).click()

  await expect(page.getByTitle('Red Paint')).toBeVisible()
  await expect(page.getByTitle('Sky Blue')).toBeVisible()

  const filterInput = page.getByLabel('Filter materials')
  await filterInput.fill('blue')
  await expect(page.getByTitle('Sky Blue')).toBeVisible()
  await expect(page.getByTitle('Red Paint')).toHaveCount(0)
  await expect(defaultSwatch).toBeVisible()

  await filterInput.fill('nonexistent material name')
  await expect(page.getByText('No materials match')).toBeVisible()

  await page.getByRole('button', { name: 'Clear filter' }).click()
  await expect(page.getByTitle('Red Paint')).toBeVisible()
  await expect(page.getByTitle('Sky Blue')).toBeVisible()
})

test('Add color sub-pane starts collapsed and requires a chosen color before it can be added', async ({ page }) => {
  await revealMaterialsTray(page)

  const colorHeader = page.getByRole('button', { name: 'Add color', exact: true })
  await expect(colorHeader).toHaveAttribute('aria-expanded', 'false')
  await expect(page.getByRole('button', { name: '+ Add color' })).toHaveCount(0)

  await colorHeader.click()
  await expect(colorHeader).toHaveAttribute('aria-expanded', 'true')
  const addButton = page.getByRole('button', { name: '+ Add color' })
  await expect(addButton).toBeVisible()
  await expect(addButton).toBeDisabled()

  await page.getByLabel('Choose color').fill('#33cc99')
  await expect(addButton).toBeEnabled()
  await page.getByPlaceholder('Name…').first().fill('Fresh Mint')
  await addButton.click()
  await expect(page.getByTitle('Fresh Mint')).toBeVisible()
})

test('Alt-click samples a face\'s material and makes it current (palette follows)', async ({ page }) => {
  await page.evaluate(() => {
    const h = window.__hew_test!
    h.setCamera({ position: [8, 6, 8], target: [1, 1, 1], up: [0, 0, 1], fovDeg: 45 })
  })
  const boxId = await page.evaluate(() => window.__hew_test!.drawBox([0, 0, 0], [2, 2, 0], 2))
  const redId = await page.evaluate(() => window.__hew_test!.addMaterial('Red', 220, 30, 30, 255))
  await page.evaluate(() => window.__hew_test!.addMaterial('Blue', 30, 30, 220, 255))
  const topFace = await page.evaluate(
    (box) => window.__hew_test!.pickFace([1, 1, 10], [0, 0, -1]),
    boxId,
  )
  expect(topFace).not.toBeNull()
  expect(topFace!.object).toBe(boxId)
  await page.evaluate(
    ({ box, face, red }) => window.__hew_test!.paintFace(box, face, red),
    { box: boxId, face: topFace!.face, red: redId },
  )

  await revealMaterialsTray(page)
  // Pick Blue as current (activates Paint), so the eyedropper's "makes it
  // current" is a REAL change, not a no-op that happened to already match.
  await page.getByTitle('Blue').click()
  await expect(page.getByLabel('Opacity for Blue')).toBeVisible()

  const canvas = await page.locator('canvas').first().boundingBox()
  if (canvas === null) throw new Error('no canvas')
  const topScreen = await page.evaluate(
    () => window.__hew_test!.worldToScreen([1, 1, 2]),
  )
  const x = canvas.x + topScreen.x
  const y = canvas.y + topScreen.y

  await page.keyboard.down('Alt')
  await page.mouse.click(x, y)
  await page.keyboard.up('Alt')

  // The palette selection followed the sample: the Opacity label (only
  // shown for the CURRENT material) now reads Red, not Blue.
  await expect(page.getByLabel('Opacity for Red')).toBeVisible()
  await expect(page.getByLabel('Opacity for Blue')).toHaveCount(0)

  // Sampling never mutates the document — the face is still Red, unchanged.
  const stillRed = await page.evaluate(
    ({ box, face }) => window.__hew_test!.getFaceMaterial(box, face),
    { box: boxId, face: topFace!.face },
  )
  expect(stillRed?.face).toBe(redId)
})

/** Losing window focus while Alt is held (Cmd-Tab, devtools, another window)
 *  swallows the keyup that would otherwise release the eyedropper, so a
 *  dedicated `blur` handler mirrors the existing precision-snap one
 *  (`onWindowBlurClearsPrecision`) to clear it. Without that handler the
 *  status line (and cursor) keep promising "Alt-click a face to sample" —
 *  the observable symptom — even though `pointerdown` already re-reads the
 *  live `ev.altKey` and would correctly paint once Alt is actually
 *  released, since the modifier state, not the stuck cursor, drives the
 *  click. This is exercised through a real `window` `blur` event dispatched
 *  in the browser (Viewport.tsx has no component-mount test harness — it
 *  owns a live WebGLRenderer/three.js scene jsdom can't construct — so this
 *  is the closest real-event equivalent to a unit test available here). */
test('losing window focus while Alt is held clears the stuck eyedropper status', async ({ page }) => {
  // Root cause of this test's flake reputation: NOT a timing race — the
  // palette never had a material named "Blue" to click in the first place,
  // so `page.getByTitle('Blue').click()` below deterministically hung on
  // an element that would never appear, until the fixed 30s Playwright
  // test timeout killed it. Every other test in this file adds its own
  // fixture material(s) before referencing them; this one was missing that
  // setup line entirely.
  await page.evaluate(() => window.__hew_test!.addMaterial('Blue', 30, 30, 220, 255))
  await revealMaterialsTray(page)
  await page.getByTitle('Blue').click()
  await expect(page.getByLabel('Opacity for Blue')).toBeVisible()
  await expect(page.getByText(/Click a face to paint it with the current material/i)).toBeVisible()

  await page.keyboard.down('Alt')
  await expect(page.getByText(/Alt-click a face to sample/i)).toBeVisible()

  // The blur swallows the keyup a real release would fire — dispatched
  // directly rather than via a real OS focus switch, which Playwright
  // cannot drive, but it is the same `window` `blur` event the app's own
  // listener reacts to.
  await page.evaluate(() => window.dispatchEvent(new Event('blur')))
  await expect(page.getByText(/Click a face to paint it with the current material/i)).toBeVisible()
  await expect(page.getByText(/Alt-click a face to sample/i)).toHaveCount(0)

  // Alt releasing later (the physical key coming up once the user is back,
  // as it would after a real Cmd-Tab) must not re-arm anything.
  await page.keyboard.up('Alt')
  await expect(page.getByText(/Click a face to paint it with the current material/i)).toBeVisible()
})

test('Shift-click replaces every matching assignment document-wide; one undo restores it all', async ({ page }) => {
  await page.evaluate(() => {
    const h = window.__hew_test!
    // Framed wide enough to comfortably see and click both boxes (x=[0,1]
    // and x=[3,4]) — the default boot camera isn't guaranteed to.
    h.setCamera({ position: [10, -10, 10], target: [2, 0.5, 0.5], up: [0, 0, 1], fovDeg: 55 })
  })
  const [boxA, boxB] = await page.evaluate(() => {
    const h = window.__hew_test!
    return [h.drawBox([0, 0, 0], [1, 1, 0], 1), h.drawBox([3, 0, 0], [4, 1, 0], 1)]
  })
  const redId = await page.evaluate(() => window.__hew_test!.addMaterial('Red', 220, 30, 30, 255))
  await page.evaluate(() => window.__hew_test!.addMaterial('Blue', 30, 30, 220, 255))

  // Paint box A's top face Red explicitly; leave everything else (A's other
  // faces, and all of box B) unpainted.
  const topA = (await page.evaluate((box) => window.__hew_test!.pickFace([0.5, 0.5, 10], [0, 0, -1]), boxA))!
  expect(topA.object).toBe(boxA)
  await page.evaluate(
    ({ box, face, red }) => window.__hew_test!.paintFace(box, face, red),
    { box: boxA, face: topA.face, red: redId },
  )
  // A side face of box A — stays unpainted, and must stay untouched by the
  // replace below (it never carried Red).
  const sideA = (await page.evaluate((box) => window.__hew_test!.pickFace([-10, 0.5, 0.5], [1, 0, 0]), boxA))!
  expect(sideA.object).toBe(boxA)
  const topB = (await page.evaluate((box) => window.__hew_test!.pickFace([3.5, 0.5, 10], [0, 0, -1]), boxB))!
  expect(topB.object).toBe(boxB)

  await revealMaterialsTray(page)
  await page.getByTitle('Blue').click()
  await expect(page.getByLabel('Opacity for Blue')).toBeVisible()

  // Shift-click box B's (unpainted) top face: fills every genuinely-unpainted
  // face/object-default in the document with Blue.
  const canvas = await page.locator('canvas').first().boundingBox()
  if (canvas === null) throw new Error('no canvas')
  const bTopScreen = await page.evaluate(() => window.__hew_test!.worldToScreen([3.5, 0.5, 1]))
  const x = canvas.x + bTopScreen.x
  const y = canvas.y + bTopScreen.y

  await page.keyboard.down('Shift')
  await page.mouse.click(x, y)
  await page.keyboard.up('Shift')

  const afterReplace = await page.evaluate(
    ({ box, faceA, faceSide, faceB }) => ({
      topA: window.__hew_test!.getFaceMaterial(box.a, faceA),
      sideA: window.__hew_test!.getFaceMaterial(box.a, faceSide),
      topB: window.__hew_test!.getFaceMaterial(box.b, faceB),
    }),
    { box: { a: boxA, b: boxB }, faceA: topA.face, faceSide: sideA.face, faceB: topB.face },
  )
  expect(afterReplace.topA?.face, 'the explicit Red override is untouched').toBe(redId)
  expect(afterReplace.sideA?.objectDefault, 'box A\'s base (genuinely unpainted) is filled with Blue')
    .not.toBeNull()
  expect(afterReplace.topB?.objectDefault, 'box B\'s base (the clicked object) is filled with Blue')
    .not.toBeNull()

  // One undo restores every touched assignment atomically.
  await page.evaluate(() => window.__hew_test!.undo())
  const afterUndo = await page.evaluate(
    ({ box, faceSide, faceB }) => ({
      sideA: window.__hew_test!.getFaceMaterial(box.a, faceSide),
      topB: window.__hew_test!.getFaceMaterial(box.b, faceB),
    }),
    { box: { a: boxA, b: boxB }, faceSide: sideA.face, faceB: topB.face },
  )
  expect(afterUndo.sideA?.objectDefault, 'undo restores box A\'s base to unpainted').toBeNull()
  expect(afterUndo.topB?.objectDefault, 'undo restores box B\'s base to unpainted').toBeNull()
})

/** Ctrl/Cmd+Shift-click confines "replace everywhere" to the clicked
 *  object — proven with two objects sharing the SAME assignment, so a
 *  document-wide leak (the plain Shift-click behavior) would be visible as
 *  the untouched object changing too. Runs the gesture with BOTH `Control`
 *  and `Meta` held (the tool reads `ev.metaKey || ev.ctrlKey`, treating
 *  either as "the modifier" rather than gating on platform), each on its
 *  own fresh pair of objects. */
test('Ctrl/Cmd+Shift-click replaces only the clicked object; one undo restores it', async ({ page }) => {
  await page.evaluate(() => {
    const h = window.__hew_test!
    h.setCamera({ position: [10, -10, 10], target: [2, 0.5, 0.5], up: [0, 0, 1], fovDeg: 55 })
  })
  const redId = await page.evaluate(() => window.__hew_test!.addMaterial('Red', 220, 30, 30, 255))
  const blueId = await page.evaluate(() => window.__hew_test!.addMaterial('Blue', 30, 30, 220, 255))
  await revealMaterialsTray(page)

  const canvas = await page.locator('canvas').first().boundingBox()
  if (canvas === null) throw new Error('no canvas')

  for (const [modifier, xOffset] of [['Control', 0], ['Meta', 6]] as const) {
    // Two objects, both painted Red on their top face — an identical
    // assignment on each, so a leak past object scope is observable.
    const [boxA, boxB] = await page.evaluate(
      (x0) => {
        const h = window.__hew_test!
        return [h.drawBox([x0, 0, 0], [x0 + 1, 1, 0], 1), h.drawBox([x0 + 3, 0, 0], [x0 + 4, 1, 0], 1)]
      },
      xOffset,
    )
    const topA = (await page.evaluate(
      (x0) => window.__hew_test!.pickFace([x0 + 0.5, 0.5, 10], [0, 0, -1]),
      xOffset,
    ))!
    expect(topA.object).toBe(boxA)
    const topB = (await page.evaluate(
      (x0) => window.__hew_test!.pickFace([x0 + 3.5, 0.5, 10], [0, 0, -1]),
      xOffset,
    ))!
    expect(topB.object).toBe(boxB)
    await page.evaluate(
      ({ box, face, red }) => window.__hew_test!.paintFace(box, face, red),
      { box: boxA, face: topA.face, red: redId },
    )
    await page.evaluate(
      ({ box, face, red }) => window.__hew_test!.paintFace(box, face, red),
      { box: boxB, face: topB.face, red: redId },
    )

    await page.getByTitle('Blue').click()
    await expect(page.getByLabel('Opacity for Blue')).toBeVisible()

    const aScreen = await page.evaluate(
      (x) => window.__hew_test!.worldToScreen([x + 0.5, 0.5, 1]),
      xOffset,
    )
    await page.keyboard.down(modifier)
    await page.keyboard.down('Shift')
    await page.mouse.click(canvas.x + aScreen.x, canvas.y + aScreen.y)
    await page.keyboard.up('Shift')
    await page.keyboard.up(modifier)

    const after = await page.evaluate(
      ({ box, faceA, faceB }) => ({
        topA: window.__hew_test!.getFaceMaterial(box.a, faceA),
        topB: window.__hew_test!.getFaceMaterial(box.b, faceB),
      }),
      { box: { a: boxA, b: boxB }, faceA: topA.face, faceB: topB.face },
    )
    expect(after.topA?.face, `${modifier}+Shift: the clicked object's face changed`).toBe(blueId)
    expect(after.topB?.face, `${modifier}+Shift: the OTHER object's identical assignment is untouched`)
      .toBe(redId)

    await page.evaluate(() => window.__hew_test!.undo())
    const afterUndo = await page.evaluate(
      ({ box, faceA }) => window.__hew_test!.getFaceMaterial(box, faceA),
      { box: boxA, faceA: topA.face },
    )
    expect(afterUndo?.face, `${modifier}+Shift: one undo restores the clicked object's face`).toBe(redId)
  }
})

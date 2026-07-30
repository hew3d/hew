import { test, expect, type Page } from '@playwright/test'

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

/**
 * Camera playtest round 2 (docs/design/camera-playtest2.md — internal,
 * gitignored; not shipped): millimetre FOV entry, Shift+Zoom fov drag/
 * wheel, and the removal of the standalone Field of View menu entry —
 * driven through the REAL Camera menu and real pointer/keyboard events,
 * per the design's own "In-app verification" section.
 */

async function setup(page: Page): Promise<void> {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, {
    timeout: 15_000,
  })
}

async function openCameraMenu(page: Page): Promise<void> {
  await page.getByRole('button', { name: 'Camera' }).click()
}

async function activateZoom(page: Page): Promise<void> {
  await openCameraMenu(page)
  await page.getByText('Zoom', { exact: true }).click()
}

/** The MeasurementBox's own text for the "Field of View" VCB ("" when not
 * showing) — read by DOM structure (label span + its sibling value span),
 * not by CSS, since the box has no test id (follow-me-partial-sweep.spec.ts's
 * own `vcbText` helper, same pattern). */
async function fovVcbText(page: Page): Promise<string> {
  return page.evaluate(() => {
    const label = Array.from(document.querySelectorAll('span')).find(
      (el) => el.textContent === 'Field of View',
    )
    return label?.parentElement?.textContent ?? ''
  })
}

async function canvasCenter(page: Page): Promise<{ x: number; y: number }> {
  const canvas = await page.locator('canvas').first().boundingBox()
  if (canvas === null) throw new Error('no canvas')
  return { x: canvas.x + canvas.width / 2, y: canvas.y + canvas.height / 2 }
}

async function pinCamera(page: Page, fovDeg = 45): Promise<void> {
  await page.evaluate((fov) => {
    window.__hew_test!.setCamera({ position: [0, -10, 0], target: [0, 0, 0], up: [0, 0, 1], fovDeg: fov })
  }, fovDeg)
  await page.waitForTimeout(100)
}

test.describe('FOV readout and typed millimetre entry (camera-playtest2.md §1)', () => {
  test('the readout shows both units the instant Zoom mode is active', async ({ page }) => {
    await setup(page)
    await pinCamera(page, 45)
    await activateZoom(page)

    const text = await fovVcbText(page)
    expect(text).toContain('Field of View')
    expect(text).toMatch(/45\.0°/)
    expect(text).toMatch(/mm/)
  })

  test('typing "50mm" and pressing Enter sets the fov via the focal-length law, shows both units, and does not move the eye', async ({ page }) => {
    await setup(page)
    await pinCamera(page, 45)
    await activateZoom(page)

    const before = await page.evaluate(() => window.__hew_test!.getCamera())

    await page.keyboard.type('50mm')
    await expect.poll(() => fovVcbText(page)).toContain('50mm')
    await page.keyboard.press('Enter')
    await page.waitForTimeout(50)

    const text = await fovVcbText(page)
    expect(text).toMatch(/39\.6°/)
    expect(text).toMatch(/50\.0 mm/)

    const after = await page.evaluate(() => window.__hew_test!.getCamera())
    expect(after.fovDeg).toBeCloseTo(39.6, 1)
    for (let i = 0; i < 3; i++) {
      expect(after.position[i]).toBeCloseTo(before.position[i], 9)
    }
  })
})

test.describe('Shift+Zoom fov drag (camera-playtest2.md §3)', () => {
  test('a Shift+drag up narrows the fov live in the readout and leaves the eye bit-identical', async ({ page }) => {
    await setup(page)
    await pinCamera(page, 45)
    await activateZoom(page)

    const before = await page.evaluate(() => window.__hew_test!.getCamera())
    const { x: cx, y: cy } = await canvasCenter(page)

    await page.mouse.move(cx, cy)
    await page.keyboard.down('Shift')
    await page.mouse.down()
    await page.mouse.move(cx, cy - 150, { steps: 10 })
    await page.waitForTimeout(50)

    const mid = await page.evaluate(() => window.__hew_test!.getCamera())
    expect(mid.fovDeg).toBeLessThan(before.fovDeg) // up narrows (ground truth: dragging up dollies IN)

    await page.mouse.up()
    await page.keyboard.up('Shift')
    await page.waitForTimeout(50)

    const after = await page.evaluate(() => window.__hew_test!.getCamera())
    expect(after.fovDeg).toBeLessThan(before.fovDeg)
    for (let i = 0; i < 3; i++) {
      expect(after.position[i]).toBeCloseTo(before.position[i], 6)
    }
  })

  test('a plain drag (no Shift) still dollies, and leaves the fov untouched', async ({ page }) => {
    await setup(page)
    await pinCamera(page, 45)
    await activateZoom(page)

    const before = await page.evaluate(() => window.__hew_test!.getCamera())
    const beforeDist = Math.hypot(
      before.position[0] - before.target[0],
      before.position[1] - before.target[1],
      before.position[2] - before.target[2],
    )
    const { x: cx, y: cy } = await canvasCenter(page)

    await page.mouse.move(cx, cy)
    await page.mouse.down()
    await page.mouse.move(cx, cy - 150, { steps: 10 })
    await page.mouse.up()
    await page.waitForTimeout(50)

    const after = await page.evaluate(() => window.__hew_test!.getCamera())
    const afterDist = Math.hypot(
      after.position[0] - after.target[0],
      after.position[1] - after.target[1],
      after.position[2] - after.target[2],
    )
    expect(afterDist).not.toBeCloseTo(beforeDist, 3) // the dolly moved the eye
    expect(after.fovDeg).toBe(before.fovDeg)
  })

  test('Shift pressed mid-dolly changes nothing about the fov — the mode is fixed at the press, not re-evaluated live', async ({ page }) => {
    await setup(page)
    await pinCamera(page, 45)
    await activateZoom(page)

    const before = await page.evaluate(() => window.__hew_test!.getCamera())
    const { x: cx, y: cy } = await canvasCenter(page)

    // Press WITHOUT Shift — this drag is a dolly, fixed at press.
    await page.mouse.move(cx, cy)
    await page.mouse.down()
    await page.mouse.move(cx, cy - 60, { steps: 5 })
    await page.waitForTimeout(30)
    const midDollyFov = (await page.evaluate(() => window.__hew_test!.getCamera())).fovDeg

    // NOW hold Shift mid-drag — must NOT retroactively turn this into a fov
    // adjustment (camera-playtest2.md §3's deliberate deviation from
    // SketchUp; OrbitControls' own MOUSE.DOLLY state, already armed at
    // press, cannot adopt a live gesture).
    await page.keyboard.down('Shift')
    await page.mouse.move(cx, cy - 150, { steps: 10 })
    await page.waitForTimeout(30)
    const afterShiftFov = (await page.evaluate(() => window.__hew_test!.getCamera())).fovDeg

    await page.mouse.up()
    await page.keyboard.up('Shift')
    await page.waitForTimeout(50)

    const final = await page.evaluate(() => window.__hew_test!.getCamera())
    expect(midDollyFov).toBe(before.fovDeg)
    expect(afterShiftFov).toBe(before.fovDeg)
    expect(final.fovDeg).toBe(before.fovDeg)
    // But the dolly itself DID keep moving the eye through the Shift press —
    // proving OrbitControls stayed in control of the gesture throughout.
    expect(
      Math.hypot(
        final.position[0] - before.position[0],
        final.position[1] - before.position[1],
        final.position[2] - before.position[2],
      ),
    ).toBeGreaterThan(0.01)
  })

  test('under Parallel Projection, a Shift-drag still dollies (no lens to adjust) and the fov stays untouched', async ({ page }) => {
    await setup(page)
    await pinCamera(page, 45)
    await openCameraMenu(page)
    await page.getByText('Parallel Projection').click()
    await page.waitForTimeout(100)
    await activateZoom(page)

    // OrbitControls' own dolly moves POSITION under perspective but scales
    // `.zoom` under orthographic (three.js's own branch — see
    // `_dollyIn`/`_dollyOut`), so eye-distance isn't the right ground truth
    // here. A parallel camera's `.zoom` scales the visible world directly:
    // project two laterally-separated points and use the screen-pixel gap
    // between them as the proxy — it changes if and only if `.zoom` did
    // (matches `depthPairScreenXGap`'s reasoning in camera.spec.ts, applied
    // to a lateral rather than depth-varying pair).
    async function screenGap(): Promise<number> {
      const [a, b] = await page.evaluate(() => {
        const h = window.__hew_test!
        return [h.worldToScreen([2, 0, 0]), h.worldToScreen([-2, 0, 0])]
      })
      return Math.abs(a.x - b.x)
    }

    const before = await page.evaluate(() => window.__hew_test!.getCamera())
    const beforeGap = await screenGap()
    const { x: cx, y: cy } = await canvasCenter(page)

    await page.keyboard.down('Shift')
    await page.mouse.move(cx, cy)
    await page.mouse.down()
    await page.mouse.move(cx, cy - 150, { steps: 10 })
    await page.mouse.up()
    await page.keyboard.up('Shift')
    await page.waitForTimeout(50)

    const after = await page.evaluate(() => window.__hew_test!.getCamera())
    const afterGap = await screenGap()
    expect(afterGap).not.toBeCloseTo(beforeGap, 0) // still an ordinary dolly (zoom changed)
    expect(after.fovDeg).toBe(before.fovDeg) // fov is perspective-only, untouched
  })
})

test.describe('Shift+Zoom fov drag — adversarial review fixes', () => {
  test('starting a fov drag with an uncommitted typed VCB entry abandons the buffer — the readout never shows a value the camera does not have (finding 1)', async ({ page }) => {
    await setup(page)
    await pinCamera(page, 45)
    await activateZoom(page)

    // Open a typed buffer with a value that DIFFERS from the live fov, and
    // deliberately leave it uncommitted (no Enter) — the review's exact repro.
    await page.keyboard.type('60')
    await expect.poll(() => fovVcbText(page)).toContain('60')

    const { x: cx, y: cy } = await canvasCenter(page)
    await page.mouse.move(cx, cy)
    await page.keyboard.down('Shift')
    await page.mouse.down()
    await page.mouse.move(cx, cy - 150, { steps: 10 })
    await page.waitForTimeout(50)

    // Mid-drag: the readout must track the ACTUAL camera fov, never the
    // stale typed "60" — updateFovDrag is genuinely moving the camera here.
    const midCamera = await page.evaluate(() => window.__hew_test!.getCamera())
    const midText = await fovVcbText(page)
    expect(midText).not.toContain('60')
    const midMatch = midText.match(/([\d.]+)°/)
    expect(midMatch).not.toBeNull()
    expect(Number(midMatch![1])).toBeCloseTo(midCamera.fovDeg, 1)

    await page.mouse.up()
    await page.keyboard.up('Shift')
    await page.waitForTimeout(50)

    const afterDragCamera = await page.evaluate(() => window.__hew_test!.getCamera())
    expect(afterDragCamera.fovDeg).toBeLessThan(45) // drag up narrowed it
    const afterDragText = await fovVcbText(page)
    const afterMatch = afterDragText.match(/([\d.]+)°/)
    expect(Number(afterMatch![1])).toBeCloseTo(afterDragCamera.fovDeg, 1)

    // A later Enter must NOT resurrect the stale typed "60" and silently
    // revert the completed drag — the buffer was abandoned when the drag
    // began, so Enter here is a no-op.
    await page.keyboard.press('Enter')
    await page.waitForTimeout(50)
    const finalCamera = await page.evaluate(() => window.__hew_test!.getCamera())
    expect(finalCamera.fovDeg).toBeCloseTo(afterDragCamera.fovDeg, 6)
  })

  test('a Shift-held single-finger touch drag adjusts fov without OrbitControls\' native touch-rotate running concurrently — the eye stays fixed (finding 2)', async ({ page }, testInfo) => {
    test.skip(testInfo.project.name !== 'chromium', 'CDP touch dispatch is Chromium-only')
    await setup(page)
    await pinCamera(page, 45)
    await activateZoom(page)

    const before = await page.evaluate(() => window.__hew_test!.getCamera())
    const { x: cx, y: cy } = await canvasCenter(page)

    // CDP's Input.dispatchTouchEvent does NOT inherit modifier state from a
    // separate Input.dispatchKeyEvent call (unlike page.mouse, which reads
    // Playwright's own live keyboard-state tracking) — each dispatched touch
    // event needs Shift passed explicitly via `modifiers` (CDP's bitmask:
    // Alt=1, Ctrl=2, Meta=4, Shift=8), or the resulting PointerEvent's
    // `shiftKey` reads false and the press never arms the fov drag at all.
    const CDP_SHIFT = 8
    const client = await page.context().newCDPSession(page)
    await page.keyboard.down('Shift')
    await client.send('Input.dispatchTouchEvent', {
      type: 'touchStart',
      touchPoints: [{ x: cx, y: cy }],
      modifiers: CDP_SHIFT,
    })
    await client.send('Input.dispatchTouchEvent', {
      type: 'touchMove',
      touchPoints: [{ x: cx, y: cy - 150 }],
      modifiers: CDP_SHIFT,
    })
    await page.waitForTimeout(50)
    await client.send('Input.dispatchTouchEvent', {
      type: 'touchEnd',
      touchPoints: [],
      modifiers: CDP_SHIFT,
    })
    await page.keyboard.up('Shift')
    await page.waitForTimeout(50)

    const after = await page.evaluate(() => window.__hew_test!.getCamera())

    // The eye is the actual contract — not just "fov changed" (a concurrent
    // native touch-rotate would move it while ALSO changing fov).
    for (let i = 0; i < 3; i++) {
      expect(after.position[i]).toBeCloseTo(before.position[i], 6)
    }
    expect(after.fovDeg).toBeLessThan(before.fovDeg) // the touch drag itself did work
  })

  test('pressing Shift mid-dolly does not swap the cursor to the fov cursor — the mode is fixed at press (finding 3)', async ({ page }) => {
    await setup(page)
    await pinCamera(page, 45)
    await activateZoom(page)

    const canvas = page.locator('canvas').first()
    const { x: cx, y: cy } = await canvasCenter(page)

    // Press WITHOUT Shift — a plain dolly, fixed at press.
    await page.mouse.move(cx, cy)
    await page.mouse.down()
    await page.mouse.move(cx, cy - 60, { steps: 5 })
    await page.waitForTimeout(30)
    const dollyCursor = await canvas.evaluate((el) => (el as HTMLElement).style.cursor)

    // Holding Shift mid-drag must not claim a mode change that will not
    // happen — the cursor should stay exactly what it was.
    await page.keyboard.down('Shift')
    await page.waitForTimeout(30)
    const midShiftCursor = await canvas.evaluate((el) => (el as HTMLElement).style.cursor)

    await page.mouse.up()
    await page.keyboard.up('Shift')
    await page.waitForTimeout(30)

    expect(midShiftCursor).toBe(dollyCursor)
  })

  test('a second concurrent pointer cannot drive an in-progress fov drag — only the pointer that armed it does (finding 4)', async ({ page }, testInfo) => {
    test.skip(testInfo.project.name !== 'chromium', 'CDP touch dispatch is Chromium-only')
    await setup(page)
    await pinCamera(page, 45)
    await activateZoom(page)

    const { x: cx, y: cy } = await canvasCenter(page)
    const client = await page.context().newCDPSession(page)

    // Arm the fov drag with a real Shift+mouse press.
    await page.mouse.move(cx, cy)
    await page.keyboard.down('Shift')
    await page.mouse.down()
    await page.mouse.move(cx, cy - 60, { steps: 5 })
    await page.waitForTimeout(30)
    const midMouseOnly = (await page.evaluate(() => window.__hew_test!.getCamera())).fovDeg

    // A SECOND, independent contact (a different pointerId) moves through
    // the same canvas while the mouse's drag is still armed — a large
    // downward travel, which would visibly WIDEN the fov if it were allowed
    // to drive the drag.
    await client.send('Input.dispatchTouchEvent', {
      type: 'touchStart',
      touchPoints: [{ x: cx + 40, y: cy }],
    })
    await client.send('Input.dispatchTouchEvent', {
      type: 'touchMove',
      touchPoints: [{ x: cx + 40, y: cy + 400 }],
    })
    await page.waitForTimeout(30)
    const midWithSecondPointer = (await page.evaluate(() => window.__hew_test!.getCamera())).fovDeg
    await client.send('Input.dispatchTouchEvent', { type: 'touchEnd', touchPoints: [] })

    expect(midWithSecondPointer).toBe(midMouseOnly)

    await page.mouse.up()
    await page.keyboard.up('Shift')
    await page.waitForTimeout(30)
  })
})

test.describe('Shift+Zoom fov drag — DELTA review fixes', () => {
  test('a Shift+wheel tick abandons an uncommitted typed VCB entry — the readout never shows a value the camera does not have (finding 3)', async ({ page }) => {
    await setup(page)
    await pinCamera(page, 45)
    await activateZoom(page)

    // Same finding-1 repro, but through the wheel path instead of the drag
    // path: open a typed buffer with a value that DIFFERS from the live
    // fov, and deliberately leave it uncommitted (no Enter).
    await page.keyboard.type('60')
    await expect.poll(() => fovVcbText(page)).toContain('60')

    const { x: cx, y: cy } = await canvasCenter(page)
    await page.mouse.move(cx, cy)
    await page.keyboard.down('Shift')
    await page.mouse.wheel(0, -100)
    await page.keyboard.up('Shift')
    await page.waitForTimeout(50)

    // The wheel tick must have applied to the ACTUAL camera fov and the
    // readout must track it — never the stale typed "60" left behind by
    // `cancelFovEntry`'s absence.
    const camera = await page.evaluate(() => window.__hew_test!.getCamera())
    expect(camera.fovDeg).not.toBeCloseTo(45, 1)
    const text = await fovVcbText(page)
    expect(text).not.toContain('60')
    const match = text.match(/([\d.]+)°/)
    expect(match).not.toBeNull()
    expect(Number(match![1])).toBeCloseTo(camera.fovDeg, 1)

    // A later Enter must NOT resurrect the stale typed "60" and silently
    // override the wheel's own change.
    await page.keyboard.press('Enter')
    await page.waitForTimeout(50)
    const finalCamera = await page.evaluate(() => window.__hew_test!.getCamera())
    expect(finalCamera.fovDeg).toBeCloseTo(camera.fovDeg, 6)
  })

  test('an unrelated pointer\'s release does not end another pointer\'s armed fov drag (findings 1+2)', async ({ page }, testInfo) => {
    test.skip(testInfo.project.name !== 'chromium', 'CDP touch dispatch is Chromium-only')
    await setup(page)
    await pinCamera(page, 45)
    await activateZoom(page)

    const before = await page.evaluate(() => window.__hew_test!.getCamera())
    const { x: cx, y: cy } = await canvasCenter(page)
    const client = await page.context().newCDPSession(page)

    // Arm the fov drag with a real Shift+mouse press — this is the pointer
    // whose gesture must survive the unrelated pointer below.
    await page.mouse.move(cx, cy)
    await page.keyboard.down('Shift')
    await page.mouse.down()
    await page.mouse.move(cx, cy - 60, { steps: 5 })
    await page.waitForTimeout(30)
    const midFirst = (await page.evaluate(() => window.__hew_test!.getCamera())).fovDeg
    expect(midFirst).toBeLessThan(before.fovDeg)

    // A SECOND, wholly independent contact presses and releases entirely on
    // its own (no Shift, a different pointerId) — it never arms anything
    // (onFovDragPointerDownCapture bails immediately since a drag is already
    // armed), but touch-derived pointer events report `button === 0` just
    // like the real mouse, so its own release must not be mistaken for the
    // armed pointer's own release.
    await client.send('Input.dispatchTouchEvent', {
      type: 'touchStart',
      touchPoints: [{ x: cx + 40, y: cy }],
    })
    await client.send('Input.dispatchTouchEvent', { type: 'touchEnd', touchPoints: [] })
    await page.waitForTimeout(30)

    // The FIRST drag must still be armed: further mouse movement keeps
    // changing the fov. Pre-fix, the second pointer's release called
    // `finishFovDrag()` unconditionally, ending the mouse's still-held
    // drag — the mouse stayed captured (mouseButtons.LEFT left null) but
    // produced no further fov change: the gesture froze rather than ending
    // cleanly or handing off to OrbitControls.
    await page.mouse.move(cx, cy - 150, { steps: 10 })
    await page.waitForTimeout(30)
    const midSecond = (await page.evaluate(() => window.__hew_test!.getCamera())).fovDeg
    expect(midSecond).toBeLessThan(midFirst)

    await page.mouse.up()
    await page.keyboard.up('Shift')
    await page.waitForTimeout(30)

    const after = await page.evaluate(() => window.__hew_test!.getCamera())
    expect(after.fovDeg).toBeLessThan(midFirst)
    for (let i = 0; i < 3; i++) {
      expect(after.position[i]).toBeCloseTo(before.position[i], 6)
    }
  })

  test('an unrelated pointer\'s cancellation does not end another pointer\'s armed fov drag (findings 1+2, onPointerCancel)', async ({ page }, testInfo) => {
    test.skip(testInfo.project.name !== 'chromium', 'CDP touch dispatch is Chromium-only')
    await setup(page)
    await pinCamera(page, 45)
    await activateZoom(page)

    const before = await page.evaluate(() => window.__hew_test!.getCamera())
    const { x: cx, y: cy } = await canvasCenter(page)
    const client = await page.context().newCDPSession(page)

    await page.mouse.move(cx, cy)
    await page.keyboard.down('Shift')
    await page.mouse.down()
    await page.mouse.move(cx, cy - 60, { steps: 5 })
    await page.waitForTimeout(30)
    const midFirst = (await page.evaluate(() => window.__hew_test!.getCamera())).fovDeg
    expect(midFirst).toBeLessThan(before.fovDeg)

    // A SECOND, wholly independent contact this time gets CANCELLED (the
    // `pointercancel` path — an OS gesture interruption, unrelated to the
    // armed drag) rather than released normally.
    await client.send('Input.dispatchTouchEvent', {
      type: 'touchStart',
      touchPoints: [{ x: cx + 40, y: cy }],
    })
    await client.send('Input.dispatchTouchEvent', { type: 'touchCancel', touchPoints: [] })
    await page.waitForTimeout(30)

    // Same assertion as the release variant: the mouse's own drag must
    // still be alive. Pre-fix, `onPointerCancel` called `abortFovDrag()`
    // unconditionally for ANY pointer's cancellation, freezing the armed
    // drag the same way.
    await page.mouse.move(cx, cy - 150, { steps: 10 })
    await page.waitForTimeout(30)
    const midSecond = (await page.evaluate(() => window.__hew_test!.getCamera())).fovDeg
    expect(midSecond).toBeLessThan(midFirst)

    await page.mouse.up()
    await page.keyboard.up('Shift')
    await page.waitForTimeout(30)

    const after = await page.evaluate(() => window.__hew_test!.getCamera())
    expect(after.fovDeg).toBeLessThan(midFirst)
    for (let i = 0; i < 3; i++) {
      expect(after.position[i]).toBeCloseTo(before.position[i], 6)
    }
  })
})

test.describe('Field of View has no standalone entry (camera-playtest2.md §2)', () => {
  test('neither the Camera menu nor the command palette offers a Field of View item', async ({ page }) => {
    await setup(page)

    await openCameraMenu(page)
    await expect(page.getByText('Zoom Window')).toBeVisible() // the menu IS open
    await expect(page.getByText('Field of View', { exact: true })).not.toBeVisible()
    // Close the menu before opening the palette.
    await page.keyboard.press('Escape')
    await expect(page.getByText('Zoom Window')).not.toBeVisible()

    const isMac = await page.evaluate(() => navigator.platform.toLowerCase().includes('mac'))
    await page.keyboard.press(isMac ? 'Meta+k' : 'Control+k')
    // Wait for the palette itself (not a fixed timeout) — its search input
    // grabs focus via requestAnimationFrame once mounted (CommandPalette.tsx).
    const search = page.getByRole('textbox', { name: 'Search' })
    await expect(search).toBeFocused()

    // `.fill()`, not `.type()` — sets the controlled input's value directly
    // through one React onChange, sidestepping any per-keystroke timing
    // sensitivity under load; the search itself is synchronous (registry.ts
    // is a plain in-memory filter, no debounce to wait out).
    //
    // `exact: true` below is load-bearing, not decoration: the palette's
    // own empty-state message echoes the typed query verbatim (`No matches
    // for "field of view".`), and Playwright's default text match is a
    // case-insensitive SUBSTRING — a non-exact `getByText('Field of View')`
    // would spuriously match that message itself once the query is this
    // exact phrase, independent of whether any real entry exists.
    await search.fill('field of view')
    await expect(page.getByText('No matches for')).toBeVisible()
    await expect(page.getByText('Field of View', { exact: true })).toHaveCount(0)

    await search.fill('fov')
    await expect(page.getByText('No matches for')).toBeVisible()
    await expect(page.getByText('Field of View', { exact: true })).toHaveCount(0)
  })
})

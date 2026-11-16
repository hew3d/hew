import { test, expect, type Page } from '@playwright/test'

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

/**
 * Fly (docs/design/fly-navigation.md §5 "Tests"): hold-to-fly under any
 * tool with a REAL right-button hold, real `keyboard.down`, and real frames
 * — forward flight along the view direction, E along world Z with the look
 * unchanged, the wheel speed ladder, a Line gesture that survives (and is
 * completed by a chorded click during) the hold, Q during a hold not
 * switching to Rotate, a hold under parallel ending in perspective, blur
 * stopping motion — plus the latched trackpad mode (§10): Camera ▸ Fly,
 * scroll-to-look, pinch-for-speed, Escape to exit — and the cursor lock
 * (§10 "Cursor lock"), driven through a stubbed Pointer Lock API because
 * headless browsers either deny the real one (Chromium on macOS, WebKit) or
 * grant it with movement no mouse produces (Chromium on Linux). A headed
 * Chromium grants it, freezes `clientX` and reports raw deltas.
 */

type Vec3 = [number, number, number]

async function setup(page: Page): Promise<void> {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, {
    timeout: 15_000,
  })
}

/** {@link setup}, with the real Pointer Lock API denied. The hold tests
 * drive the unlocked path. Headless Chromium on macOS denies
 * the lock by itself, but on Linux it grants it, and then Playwright's moves
 * arrive as absolute jumps with a jump back between them, not a real mouse's
 * deltas. The locked path has its own tests below, on a stubbed lock. */
async function setupUnlocked(page: Page): Promise<void> {
  await setup(page)
  await stubPointerLock(page, { denyAll: true })
}

async function canvasCenter(page: Page): Promise<{ x: number; y: number }> {
  const canvas = await page.locator('canvas').first().boundingBox()
  if (canvas === null) throw new Error('no canvas')
  return { x: canvas.x + canvas.width / 2, y: canvas.y + canvas.height / 2 }
}

async function readCameraState(page: Page) {
  return page.evaluate(() => window.__hew_test!.getCameraState())
}

/** A unit look direction from a camera state (target − eye, normalized). */
function lookDir(state: { eye: number[]; target: number[] }): Vec3 {
  const d: Vec3 = [state.target[0] - state.eye[0], state.target[1] - state.eye[1], state.target[2] - state.eye[2]]
  const len = Math.hypot(...d)
  return [d[0] / len, d[1] / len, d[2] / len]
}

function sub(a: number[], b: number[]): Vec3 {
  return [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

function dot(a: Vec3, b: Vec3): number {
  return a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/** A pose a few meters from the origin looking horizontally along +X, so
 * forward flight has an unambiguous axis to measure against. */
async function poseLookingAlongX(page: Page): Promise<void> {
  await page.evaluate(() => {
    window.__hew_test!.setCamera({ position: [-6, 0, 1.6], target: [0, 0, 1.6], up: [0, 0, 1], fovDeg: 45 })
  })
  await page.waitForTimeout(100)
}

const flyHint = (page: Page) => page.getByTestId('fly-hint')

test.describe('Fly ▸ hold the right mouse button (docs/design/fly-navigation.md §3)', () => {
  test('W flies the eye forward along the view direction; releasing the button ends the flight', async ({ page }) => {
    await setupUnlocked(page)
    await poseLookingAlongX(page)
    const { x, y } = await canvasCenter(page)
    const before = await readCameraState(page)
    await expect(flyHint(page)).toHaveCount(0)

    await page.mouse.move(x, y)
    await page.mouse.down({ button: 'right' })
    // The hint appears on the press itself, before any key (§4).
    await expect(flyHint(page)).toBeVisible()
    await page.keyboard.down('w')
    await page.waitForTimeout(1000)
    await page.keyboard.up('w')
    await page.mouse.up({ button: 'right' })
    await expect(flyHint(page)).toHaveCount(0)
    // Release ends the flight at once (§3.1): the pose read right after
    // release is final.
    await page.waitForTimeout(100)
    const flown = await readCameraState(page)

    const moved = sub(flown.eye, before.eye)
    const dist = Math.hypot(...moved)
    // ~1 s at 2 m/s minus the ramp: a clear move, not a nudge. The floor is
    // generous because headless software-GL frames run at ~4 fps and the
    // per-frame delta is clamped (FLY_MAX_DT_MS).
    expect(dist).toBeGreaterThan(0.3)
    expect(dist).toBeLessThan(2.5)
    // Along the view direction (+X here), not sideways or up.
    expect(dot([moved[0] / dist, moved[1] / dist, moved[2] / dist], lookDir(before))).toBeGreaterThan(0.99)
    // The look direction itself did not change — the mouse never moved.
    expect(dot(lookDir(flown), lookDir(before))).toBeGreaterThan(0.9999)

    // After release, nothing moves any more.
    await page.waitForTimeout(500)
    const after = await readCameraState(page)
    expect(Math.hypot(...sub(after.eye, flown.eye))).toBeLessThan(1e-6)
    // OrbitControls got its target re-seeded ahead of the eye (§3.1), not
    // left behind at the pre-flight pivot.
    expect(dot(lookDir(after), lookDir(before))).toBeGreaterThan(0.9999)
    expect(Math.hypot(...sub(after.target, after.eye))).toBeGreaterThan(0.5)
  })

  test('E climbs along world Z with the look direction unchanged, even while pitched', async ({ page }) => {
    await setupUnlocked(page)
    // Pitched down ~20°: forward has a −Z component, so "along the view
    // direction" and "along world Z" are distinguishable.
    await page.evaluate(() => {
      window.__hew_test!.setCamera({ position: [-6, 0, 3], target: [0, 0, 0.8], up: [0, 0, 1], fovDeg: 45 })
    })
    await page.waitForTimeout(100)
    const { x, y } = await canvasCenter(page)
    const before = await readCameraState(page)

    await page.mouse.move(x, y)
    await page.mouse.down({ button: 'right' })
    await page.keyboard.down('e')
    await page.waitForTimeout(1000)
    await page.keyboard.up('e')
    await page.mouse.up({ button: 'right' })
    await page.waitForTimeout(100)
    const flown = await readCameraState(page)

    const moved = sub(flown.eye, before.eye)
    expect(moved[2]).toBeGreaterThan(0.25)
    expect(Math.abs(moved[0])).toBeLessThan(1e-3)
    expect(Math.abs(moved[1])).toBeLessThan(1e-3)
    expect(dot(lookDir(flown), lookDir(before))).toBeGreaterThan(0.9999)
  })

  test('moving the mouse while held looks around; the eye stays put; right-drag no longer pans', async ({ page }) => {
    await setupUnlocked(page)
    await poseLookingAlongX(page)
    const { x, y } = await canvasCenter(page)
    const before = await readCameraState(page)

    await page.mouse.move(x, y)
    await page.mouse.down({ button: 'right' })
    await page.mouse.move(x + 200, y, { steps: 10 })
    await page.waitForTimeout(100)
    const looked = await readCameraState(page)
    await page.mouse.up({ button: 'right' })
    await page.waitForTimeout(300)

    // A pan would have moved the eye; a look leaves it exactly in place.
    expect(Math.hypot(...sub(looked.eye, before.eye))).toBeLessThan(1e-6)
    // Dragging right turns the view right (toward −Y when facing +X).
    const dir = lookDir(looked)
    expect(dir[1]).toBeLessThan(-0.2)
    expect(Math.abs(dir[2])).toBeLessThan(1e-3)
  })

  test('a wheel notch while held changes the speed readout and the flight speed', async ({ page }) => {
    await setupUnlocked(page)
    await poseLookingAlongX(page)
    const { x, y } = await canvasCenter(page)

    await page.mouse.move(x, y)
    await page.mouse.down({ button: 'right' })
    await expect(page.getByText('Fly speed')).toBeVisible()
    await expect(page.getByText('2 m/s', { exact: true })).toBeVisible()
    // Wheel up = faster (§3.3): one 100 px notch → ×1.25.
    await page.mouse.wheel(0, -100)
    await expect(page.getByText('2.5 m/s', { exact: true })).toBeVisible()
    await page.mouse.wheel(0, 100)
    await expect(page.getByText('2 m/s', { exact: true })).toBeVisible()
    await page.mouse.wheel(0, -100)
    await page.mouse.wheel(0, -100)
    await expect(page.getByText('3.13 m/s', { exact: true })).toBeVisible()

    // The faster speed is real: the same hold covers more ground.
    const before = await readCameraState(page)
    await page.keyboard.down('w')
    await page.waitForTimeout(1000)
    await page.keyboard.up('w')
    await page.mouse.up({ button: 'right' })
    await page.waitForTimeout(100)
    const flown = await readCameraState(page)
    const dist = Math.hypot(...sub(flown.eye, before.eye))
    expect(dist).toBeGreaterThan(0.4)

    // The speed is session-persistent (§3.4): the next hold shows it again.
    await page.mouse.down({ button: 'right' })
    await expect(page.getByText('3.13 m/s', { exact: true })).toBeVisible()
    await page.mouse.up({ button: 'right' })
  })

  test('a Line gesture armed before the hold survives it, and a chorded left click during the hold completes it', async ({ page }) => {
    await setupUnlocked(page)
    await page.evaluate(() => {
      window.__hew_test!.setCamera({ position: [3, -3, 4], target: [0, 0, 0], up: [0, 0, 1], fovDeg: 45 })
    })
    await page.waitForTimeout(100)
    const { x, y } = await canvasCenter(page)
    const sketchesBefore = await page.evaluate(() => window.__hew_test!.getSketchIds().length)

    await page.keyboard.press('l')
    await page.mouse.move(x - 60, y)
    await page.mouse.down()
    await page.mouse.up()
    await page.waitForTimeout(100)

    // Engage Fly with the Line's first point armed; strafe a little.
    await page.mouse.down({ button: 'right' })
    await expect(flyHint(page)).toBeVisible()
    await page.keyboard.down('d')
    await page.waitForTimeout(200)
    await page.keyboard.up('d')
    await page.waitForTimeout(300)
    // The tool is still Line, still armed — Q did not become Rotate, nothing
    // reset it.
    await expect(page.getByRole('radio', { name: 'Line' })).toHaveAttribute('aria-checked', 'true')

    // A LEFT click while the right button is still down: the Pointer
    // Events spec delivers this as a chorded pointermove, which the Fly
    // block re-synthesizes into the tool's down/up (§3.6).
    await page.mouse.move(x + 60, y + 20, { steps: 4 })
    await page.mouse.down()
    await page.mouse.up()
    await page.waitForTimeout(100)
    await page.mouse.up({ button: 'right' })
    await page.waitForTimeout(100)
    // A two-point Line commits on the second click: one new sketch with
    // one segment.
    const sketches = await page.evaluate(() => window.__hew_test!.getSketchIds())
    expect(sketches.length).toBe(sketchesBefore + 1)
    const segs = await page.evaluate((id) => window.__hew_test!.getSketchSegmentsExact(id), sketches[sketches.length - 1])
    expect(segs.length).toBe(1)
  })

  test('Q and S during a hold move the camera and never switch to Rotate or Scale', async ({ page }) => {
    await setupUnlocked(page)
    await poseLookingAlongX(page)
    const { x, y } = await canvasCenter(page)
    const select = page.getByRole('radio', { name: 'Select' })
    await expect(select).toHaveAttribute('aria-checked', 'true')
    const before = await readCameraState(page)

    await page.mouse.move(x, y)
    await page.mouse.down({ button: 'right' })
    // Each key is held until the camera has gone far enough, not for a
    // fixed time: software-GL frame rates vary, and the per-frame step is
    // clamped, so a fixed hold covers less ground on a slow runner.
    await page.keyboard.down('q')
    await expect.poll(async () => (await readCameraState(page)).eye[2]).toBeLessThan(before.eye[2] - 0.15)
    await page.keyboard.up('q')
    await page.keyboard.down('s')
    await expect.poll(async () => (await readCameraState(page)).eye[0]).toBeLessThan(before.eye[0] - 0.15)
    await page.keyboard.up('s')
    await page.mouse.up({ button: 'right' })
    await page.waitForTimeout(100)
    const flown = await readCameraState(page)

    await expect(select).toHaveAttribute('aria-checked', 'true')
    await expect(page.getByRole('radio', { name: 'Rotate' })).toHaveAttribute('aria-checked', 'false')
    await expect(page.getByRole('radio', { name: 'Scale' })).toHaveAttribute('aria-checked', 'false')
    // Q descended, S backed up along −X.
    expect(flown.eye[2]).toBeLessThan(before.eye[2] - 0.15)
    expect(flown.eye[0]).toBeLessThan(before.eye[0] - 0.15)

    // With the button up, Q is Rotate again — nothing stuck.
    await page.keyboard.press('q')
    await expect(page.getByRole('radio', { name: 'Rotate' })).toHaveAttribute('aria-checked', 'true')
  })

  test('a right-click or a look-only hold under parallel stays parallel; the first movement key switches to perspective and leaves it there (§3.5)', async ({ page }) => {
    await setupUnlocked(page)
    await poseLookingAlongX(page)
    await page.getByRole('button', { name: 'Camera' }).click()
    await page.getByText('Parallel Projection', { exact: true }).click()
    await page.waitForTimeout(100)
    expect((await readCameraState(page)).projection).toBe('parallel')

    const { x, y } = await canvasCenter(page)
    await page.mouse.move(x, y)
    // A bare right-click never changes the projection.
    await page.mouse.down({ button: 'right' })
    await page.waitForTimeout(50)
    expect((await readCameraState(page)).projection).toBe('parallel')
    await page.mouse.up({ button: 'right' })
    await page.waitForTimeout(100)
    expect((await readCameraState(page)).projection).toBe('parallel')

    // Neither does a hold that only looks around.
    await page.mouse.down({ button: 'right' })
    await page.mouse.move(x + 30, y + 10, { steps: 4 })
    await page.waitForTimeout(50)
    expect((await readCameraState(page)).projection).toBe('parallel')

    // The first movement key switches, and the switch stays after release.
    await page.keyboard.down('w')
    await page.waitForTimeout(100)
    await page.keyboard.up('w')
    expect((await readCameraState(page)).projection).toBe('perspective')
    await page.mouse.up({ button: 'right' })
    await page.waitForTimeout(100)
    expect((await readCameraState(page)).projection).toBe('perspective')
  })

  test('a window blur during a hold stops the flight and releases the keys', async ({ page }) => {
    await setupUnlocked(page)
    await poseLookingAlongX(page)
    const { x, y } = await canvasCenter(page)

    await page.mouse.move(x, y)
    await page.mouse.down({ button: 'right' })
    await page.keyboard.down('w')
    await page.waitForTimeout(250)
    await page.evaluate(() => window.dispatchEvent(new Event('blur')))
    await expect(flyHint(page)).toHaveCount(0)
    const atBlur = await readCameraState(page)
    await page.waitForTimeout(400)
    const later = await readCameraState(page)
    expect(Math.hypot(...sub(later.eye, atBlur.eye))).toBeLessThan(1e-6)
    await page.keyboard.up('w')
    await page.mouse.up({ button: 'right' })
  })
})

test.describe('Fly ▸ latched from Camera ▸ Fly, for trackpads (docs/design/fly-navigation.md §10)', () => {
  test('Camera ▸ Fly latches flight on: keys move, scrolling looks, pinching sets speed, Esc exits', async ({ page }) => {
    await setup(page)
    await poseLookingAlongX(page)
    // The no-lock contract, made deterministic: headless browsers differ on
    // the Pointer Lock API, and the scroll-looks fallback must not depend
    // on what this one does.
    await stubPointerLock(page, { denyAll: true })
    const { x, y } = await canvasCenter(page)
    const before = await readCameraState(page)

    await page.getByRole('button', { name: 'Camera' }).click()
    await page.getByText('Fly', { exact: true }).click()
    await expect(flyHint(page)).toBeVisible()
    await expect(page.getByText('Esc exits')).toBeVisible()
    await expect(page.getByText('2 m/s', { exact: true })).toBeVisible()

    // No button held: W moves.
    await page.mouse.move(x, y)
    await page.keyboard.down('w')
    await page.waitForTimeout(1000)
    await page.keyboard.up('w')
    // Let the glide mostly rest before the look checks compare eyes (at
    // headless frame rates the tail is long; the checks below tolerate the
    // last millimeters of it).
    await page.waitForTimeout(1500)
    const flown = await readCameraState(page)
    const moved = sub(flown.eye, before.eye)
    expect(Math.hypot(...moved)).toBeGreaterThan(0.15)
    expect(dot([moved[0], moved[1], moved[2]], lookDir(before))).toBeGreaterThan(0.1)

    // A two-finger scroll (a plain wheel) looks instead of zooming: the eye
    // stays, the direction turns. Scrolling right (`deltaX > 0`) yaws right.
    await page.mouse.wheel(200, 0)
    await page.waitForTimeout(100)
    const looked = await readCameraState(page)
    expect(Math.hypot(...sub(looked.eye, flown.eye))).toBeLessThan(0.02)
    expect(lookDir(looked)[1]).toBeLessThan(-0.2)

    // A pinch (ctrl+wheel) sets the speed; a wheel never dollied the eye.
    await page.keyboard.down('Control')
    await page.mouse.wheel(0, -100)
    await page.keyboard.up('Control')
    await expect(page.getByText('2.5 m/s', { exact: true })).toBeVisible()
    // WebKit reports a pinch as gesture events with a cumulative scale (§10);
    // Playwright cannot synthesize one, so dispatch the shape the handler
    // reads: start at 1, then a change at 2× doubles the speed.
    await page.evaluate(() => {
      const canvas = document.querySelector('canvas')!
      for (const [type, scale] of [['gesturestart', 1], ['gesturechange', 2]] as const) {
        const ev = new Event(type, { bubbles: true, cancelable: true })
        Object.defineProperty(ev, 'scale', { value: scale })
        canvas.dispatchEvent(ev)
      }
    })
    await expect(page.getByText('5 m/s', { exact: true })).toBeVisible()
    const afterPinch = await readCameraState(page)
    expect(Math.hypot(...sub(afterPinch.eye, flown.eye))).toBeLessThan(0.02)

    // Escape latches off.
    await page.keyboard.press('Escape')
    await expect(flyHint(page)).toHaveCount(0)
    // Keys are plain shortcuts again.
    await page.keyboard.press('q')
    await expect(page.getByRole('radio', { name: 'Rotate' })).toHaveAttribute('aria-checked', 'true')
  })

  test('the Fly palette entry toggles the latched mode', async ({ page }) => {
    await setup(page)
    await stubPointerLock(page, { denyAll: true })
    await page.keyboard.press('ControlOrMeta+K')
    const palette = page.getByRole('dialog', { name: /command palette/i })
    await expect(palette).toBeVisible()
    await palette.getByPlaceholder(/search/i).fill('fly')
    await page.keyboard.press('Enter')
    await expect(flyHint(page)).toBeVisible()
    await page.keyboard.press('Escape')
    await expect(flyHint(page)).toHaveCount(0)
  })
})

/**
 * A Pointer Lock API double on the canvas: grants (asynchronously, as the
 * browsers do) or denies each request, and can take the lock away the way
 * a browser's own Escape does. `document.pointerLockElement` is shadowed
 * on the instance; the real API is never reached.
 */
async function stubPointerLock(page: Page, opts: { denyFirst?: boolean; denyAll?: boolean } = {}): Promise<void> {
  await page.evaluate(({ denyFirst, denyAll }) => {
    const canvas = document.querySelector('canvas')!
    let locked: Element | null = null
    const stub = {
      requests: 0,
      exits: 0,
      pointerId: -1,
      grant() {
        locked = canvas
        document.dispatchEvent(new Event('pointerlockchange'))
      },
      lose() {
        locked = null
        document.dispatchEvent(new Event('pointerlockchange'))
      },
    }
    Object.defineProperty(document, 'pointerLockElement', { get: () => locked, configurable: true })
    canvas.requestPointerLock = function () {
      stub.requests++
      const deny = denyAll === true || (denyFirst === true && stub.requests === 1)
      setTimeout(() => {
        if (deny) document.dispatchEvent(new Event('pointerlockerror'))
        else stub.grant()
      }, 0)
      return Promise.resolve()
    } as typeof canvas.requestPointerLock
    document.exitPointerLock = () => {
      stub.exits++
      setTimeout(() => stub.lose(), 0)
    }
    canvas.addEventListener('pointerdown', (e) => { stub.pointerId = e.pointerId }, true)
    ;(window as unknown as { __plock: typeof stub }).__plock = stub
  }, { denyFirst: opts.denyFirst ?? false, denyAll: opts.denyAll ?? false })
}

async function plock(page: Page) {
  return page.evaluate(() => {
    const s = (window as unknown as { __plock: { requests: number; exits: number; pointerId: number } }).__plock
    return { requests: s.requests, exits: s.exits, pointerId: s.pointerId }
  })
}

async function flyState(page: Page) {
  return page.evaluate(() => window.__hew_test!.getFlyState())
}

/** A pointer move carrying raw movement with the position frozen — what a
 * locked pointer delivers (Playwright's WebKit driver reports no movement
 * on its own moves, so the shape is dispatched directly). */
async function lockedMove(page: Page, movementX: number, movementY: number, pointerId: number, buttons: number): Promise<void> {
  const { x, y } = await canvasCenter(page)
  await page.evaluate(({ movementX, movementY, pointerId, buttons, x, y }) => {
    const canvas = document.querySelector('canvas')!
    canvas.dispatchEvent(new PointerEvent('pointermove', {
      movementX, movementY, clientX: x, clientY: y, screenX: x, screenY: y,
      button: -1, buttons, pointerId, pointerType: 'mouse', isPrimary: true, bubbles: true, cancelable: true,
    }))
  }, { movementX, movementY, pointerId, buttons, x, y })
}

test.describe('Fly ▸ cursor lock (docs/design/fly-navigation.md §10 "Cursor lock")', () => {
  test('a hold locks the cursor: look reads raw movement with the position frozen, and release gives it back', async ({ page }) => {
    await setup(page)
    await poseLookingAlongX(page)
    await stubPointerLock(page)
    const { x, y } = await canvasCenter(page)
    const before = await readCameraState(page)

    await page.mouse.move(x, y)
    await page.mouse.down({ button: 'right' })
    await expect.poll(async () => (await flyState(page)).locked).toBe(true)
    expect((await plock(page)).requests).toBe(1)

    // Raw movement, frozen position: the view turns right, the eye stays.
    const { pointerId } = await plock(page)
    for (let i = 0; i < 5; i++) await lockedMove(page, 40, 0, pointerId, 2)
    await page.waitForTimeout(100)
    const looked = await readCameraState(page)
    expect(Math.hypot(...sub(looked.eye, before.eye))).toBeLessThan(1e-6)
    expect(lookDir(looked)[1]).toBeLessThan(-0.2)

    await page.mouse.up({ button: 'right' })
    await expect(flyHint(page)).toHaveCount(0)
    const state = await flyState(page)
    expect(state.locked).toBe(false)
    expect((await plock(page)).exits).toBe(1)
  })

  test('Camera ▸ Fly locks the cursor: the mouse looks with nothing held, the wheel sets the speed, and the browser taking the lock away ends the flight', async ({ page }) => {
    await setup(page)
    await poseLookingAlongX(page)
    await stubPointerLock(page)
    const before = await readCameraState(page)

    await page.getByRole('button', { name: 'Camera' }).click()
    await page.getByText('Fly', { exact: true }).click()
    await expect(flyHint(page)).toBeVisible()
    await expect.poll(async () => (await flyState(page)).locked).toBe(true)
    await expect(page.getByText(/mouse or trackpad looks/)).toBeVisible()

    // No button at all: raw movement looks.
    for (let i = 0; i < 5; i++) await lockedMove(page, 0, 40, 1, 0)
    await page.waitForTimeout(100)
    const looked = await readCameraState(page)
    expect(Math.hypot(...sub(looked.eye, before.eye))).toBeLessThan(1e-6)
    // Moving "down" pitches down.
    expect(lookDir(looked)[2]).toBeLessThan(-0.2)

    // The wheel is the speed while locked (the mouse is the look), and
    // never a dolly nor a look. (The move that parks the pointer over the
    // canvas is itself a look in Chromium, which reports movement on its
    // own moves, so the look is read again after it.)
    const { x, y } = await canvasCenter(page)
    await page.mouse.move(x, y)
    await page.waitForTimeout(100)
    const beforeWheel = await readCameraState(page)
    await page.mouse.wheel(0, -100)
    await expect(page.getByText('2.5 m/s', { exact: true })).toBeVisible()
    const afterWheel = await readCameraState(page)
    expect(Math.hypot(...sub(afterWheel.eye, before.eye))).toBeLessThan(1e-6)
    expect(Math.abs(dot(lookDir(afterWheel), lookDir(beforeWheel)) - 1)).toBeLessThan(1e-6)

    // The browser's own Escape exit (Chromium never shows the page that
    // keydown): the lock goes with focus intact, and the flight ends.
    await page.evaluate(() => (window as unknown as { __plock: { lose(): void } }).__plock.lose())
    await expect(flyHint(page)).toHaveCount(0)
    expect((await plock(page)).exits).toBe(0)
  })

  test('a latched flight denied the lock keeps flying unlocked; a click on the view locks it and is never a pick; a blur unlocks without ending it', async ({ page }) => {
    await setup(page)
    await poseLookingAlongX(page)
    await stubPointerLock(page, { denyFirst: true })
    const { x, y } = await canvasCenter(page)
    const sketchesBefore = await page.evaluate(() => window.__hew_test!.getSketchIds().length)

    await page.getByRole('button', { name: 'Camera' }).click()
    await page.getByText('Fly', { exact: true }).click()
    await expect(flyHint(page)).toBeVisible()
    await expect(page.getByText(/click the view to look with the mouse/)).toBeVisible()
    await expect.poll(async () => (await plock(page)).requests).toBe(1)
    expect((await flyState(page)).locked).toBe(false)

    // Rectangle armed, then the lock click: it locks and is consumed —
    // the Rectangle does not get a first corner from it.
    await page.keyboard.press('r')
    await expect(page.getByRole('radio', { name: 'Rectangle' })).toHaveAttribute('aria-checked', 'true')
    await page.mouse.click(x, y)
    await expect.poll(async () => (await flyState(page)).locked).toBe(true)
    expect((await plock(page)).requests).toBe(2)
    await expect(page.getByText(/mouse or trackpad looks/)).toBeVisible()

    // Focus goes elsewhere: the cursor is given back, the flight stays on.
    // The blur comes first and the browser's lock loss follows it, as a
    // real focus change orders them.
    await page.evaluate(() => window.dispatchEvent(new Event('blur')))
    await page.evaluate(() => (window as unknown as { __plock: { lose(): void } }).__plock.lose())
    await expect(flyHint(page)).toBeVisible()
    await expect(page.getByText(/click the view to look with the mouse/)).toBeVisible()
    expect((await flyState(page)).locked).toBe(false)

    // Escape ends it; had the lock click armed the Rectangle, this next
    // click would commit one (count +1) — it only arms. The positive
    // control: one more click does commit, so the mechanism was live.
    await page.keyboard.press('Escape')
    await expect(flyHint(page)).toHaveCount(0)
    await page.mouse.click(x + 80, y + 40)
    await page.waitForTimeout(100)
    expect(await page.evaluate(() => window.__hew_test!.getSketchIds().length)).toBe(sketchesBefore)
    await page.mouse.click(x + 160, y + 100)
    await expect.poll(() => page.evaluate(() => window.__hew_test!.getSketchIds().length)).toBe(sketchesBefore + 1)
  })
})

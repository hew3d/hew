import { test, expect, type Page } from '@playwright/test'

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

/**
 * Orbiting from a scene's view stays level. A stored camera's `up` is its
 * pitched screen-up (an imported SketchUp scene writes it that way); left on
 * the camera it became OrbitControls' turning axis, so a level
 * right-to-left orbit rolled the horizon until Fly mode re-levelled it.
 * Real pointer events over a scene applied through the real API.
 */
async function setup(page: Page): Promise<void> {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 20_000 })
}

test('orbiting after applying a pitched-up scene camera turns about the vertical', async ({ page }) => {
  await setup(page)
  // A 3/4 view looking down at the origin, with `up` the pitched screen-up
  // (perpendicular to the view direction, in the vertical plane through it).
  const eye: [number, number, number] = [6, -5, 4]
  const dir = [-eye[0], -eye[1], -eye[2]]
  const dl = Math.hypot(dir[0], dir[1], dir[2])
  const d = dir.map((v) => v / dl)
  // up = Z - (Z . d) d, normalised.
  const k = d[2]
  const raw = [-k * d[0], -k * d[1], 1 - k * d[2]]
  const rl = Math.hypot(raw[0], raw[1], raw[2])
  const up: [number, number, number] = [raw[0] / rl, raw[1] / rl, raw[2] / rl]
  await page.evaluate(
    ([e, u]) => window.__hew_test!.setCamera({ position: e, target: [0, 0, 0], up: u, fovDeg: 35 }),
    [eye, up] as [typeof eye, typeof up],
  )
  await page.evaluate(() => window.__hew_test!.addScene('Pitched'))
  const sceneId = await page.evaluate(() => {
    const r = window.__hew_test!.apiDispatch({ method: 'hew.scenes.list' }) as unknown as {
      result: { scenes: { id: string; name: string }[] }
    }
    return r.result.scenes.find((s) => s.name === 'Pitched')!.id
  })
  // Park the camera elsewhere, then apply the scene.
  await page.evaluate(() =>
    window.__hew_test!.setCamera({ position: [0, -9, 1], target: [0, 0, 0], up: [0, 0, 1], fovDeg: 35 }),
  )
  // The scene took hold: the camera is at its eye, and reads back with the
  // pitched `up` the scene stores (so activating it is not a change to it).
  // (Applied again on every poll: a busy runner can drop the first one.)
  await expect
    .poll(
      async () =>
        page.evaluate((id) => {
          const h = window.__hew_test!
          const x = h.getCameraState().eye[0]
          if (Math.abs(x) < 1) h.apiDispatch({ method: 'hew.scenes.apply', params: { id } })
          return x
        }, sceneId),
      { timeout: 20_000 },
    )
    .toBeCloseTo(eye[0], 3)
  const applied = await page.evaluate(() => window.__hew_test!.getCameraState())
  for (let i = 0; i < 3; i++) expect(applied.up[i]).toBeCloseTo(up[i], 4)

  // Orbit right-to-left with the middle button.
  const box = (await page.locator('canvas').first().boundingBox())!
  const cx = box.x + box.width / 2
  const cy = box.y + box.height / 2
  await page.mouse.move(cx + 150, cy)
  await page.mouse.down({ button: 'middle' })
  for (let i = 1; i <= 15; i++) await page.mouse.move(cx + 150 - i * 20, cy)
  await page.mouse.up({ button: 'middle' })
  await page.waitForTimeout(300)

  const cam = await page.evaluate(() => window.__hew_test!.getCameraState())
  // The drag really orbited: the eye moved a long way round the target.
  expect(Math.hypot(cam.eye[0] - applied.eye[0], cam.eye[1] - applied.eye[1])).toBeGreaterThan(1)
  expect(cam.eye[2]).toBeCloseTo(applied.eye[2], 3)
  expect(cam.up[0]).toBeCloseTo(0, 6)
  expect(cam.up[1]).toBeCloseTo(0, 6)
  expect(cam.up[2]).toBeCloseTo(1, 6)

  // A vertical line in the world stays vertical on screen: no roll.
  const [lo, hi] = await page.evaluate(() => [
    window.__hew_test!.worldToScreen([0, 0, 0]),
    window.__hew_test!.worldToScreen([0, 0, 1]),
  ])
  expect(Math.abs(lo.x - hi.x)).toBeLessThan(1.5)
})

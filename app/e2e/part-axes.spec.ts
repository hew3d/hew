import { test, expect, type Page } from '@playwright/test'
import { buildViewProjection, worldToPagePixel, type CameraParams } from './helpers/projectWorldToScreen'

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

/**
 * View > Part Axes: a triad at the selected component's origin, nothing for a
 * loose Object with no frame override, and the choice survives a reload. The
 * menu is clicked for real; the harness builds the starting model and reads
 * the overlay back.
 */

async function setup(page: Page): Promise<void> {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
}

async function togglePartAxes(page: Page): Promise<void> {
  await page.getByRole('button', { name: 'View' }).click()
  await page.getByTestId('menu-bar').getByText(/^✓?Part Axes$/).click()
}

test('select_a_component_and_toggle_part_axes', async ({ page }) => {
  await setup(page)
  const { instance } = await page.evaluate(() => {
    const h = window.__hew_test!
    const box = h.drawBox([2, 3, 0], [4, 5, 0], 2)
    return h.makeComponent([box])
  })
  await page.evaluate((id) => window.__hew_test!.selectNodes([{ kind: 'instance', id }]), instance)

  // Off by default: nothing is drawn even with a component selected.
  let info = await page.evaluate(() => window.__hew_test!.getPartAxesInfo())
  expect(info).toEqual({ visible: false, origins: [] })

  await togglePartAxes(page)
  await expect.poll(() => page.evaluate(() => window.__hew_test!.getPartAxesInfo().origins.length)).toBe(1)
  const pose = await page.evaluate((id) => window.__hew_test!.getInstancePose(id), instance)
  info = await page.evaluate(() => window.__hew_test!.getPartAxesInfo())
  expect(info.visible).toBe(true)
  expect(info.origins[0][0]).toBeCloseTo(pose![3])
  expect(info.origins[0][1]).toBeCloseTo(pose![7])
  expect(info.origins[0][2]).toBeCloseTo(pose![11])

  // A loose Object with no override gets no triad.
  const loose = await page.evaluate(() => window.__hew_test!.drawBox([8, 8, 0], [9, 9, 0], 1))
  await page.evaluate((id) => window.__hew_test!.selectNodes([{ kind: 'object', id }]), loose)
  await expect.poll(() => page.evaluate(() => window.__hew_test!.getPartAxesInfo().origins.length)).toBe(0)

  // The setting persists across a reload; toggling again removes the overlay.
  await page.reload()
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
  expect(await page.evaluate(() => window.__hew_test!.getPartAxesInfo().visible)).toBe(true)
  await togglePartAxes(page)
  expect(await page.evaluate(() => window.__hew_test!.getPartAxesInfo().visible)).toBe(false)
})

const CAMERA: CameraParams = {
  position: { x: 6, y: -6, z: 6 },
  target: { x: 3, y: 0.5, z: 0.5 },
  up: { x: 0, y: 0, z: 1 },
  fovDeg: 50,
  near: 0.1,
  far: 1000,
}

test('the_triad_stays_on_the_entered_component_through_a_real_session', async ({ page }) => {
  await setup(page)
  await page.evaluate((cam) => window.__hew_test!.setCamera({
    position: [cam.position.x, cam.position.y, cam.position.z],
    target: [cam.target.x, cam.target.y, cam.target.z],
    up: [cam.up.x, cam.up.y, cam.up.z],
    fovDeg: cam.fovDeg,
  }), CAMERA)
  const box = await page.locator('canvas').first().boundingBox()
  if (box === null) throw new Error('no canvas')
  const rect = { left: box.x, top: box.y, width: box.width, height: box.height }
  const vp = buildViewProjection(CAMERA, rect.width / rect.height)
  const { instance } = await page.evaluate(() => {
    const h = window.__hew_test!
    const b = h.drawBox([0, 0, 0], [1, 1, 0], 1)
    const { component } = h.makeComponent([b])
    return { instance: h.placeInstance(component, 3, 0, 0) }
  })
  await page.evaluate((id) => window.__hew_test!.selectNodes([{ kind: 'instance', id }]), instance)
  await togglePartAxes(page)
  await expect.poll(() => page.evaluate(() => window.__hew_test!.getPartAxesInfo().origins.length)).toBe(1)
  const pose = (await page.evaluate((id) => window.__hew_test!.getInstancePose(id), instance))!

  // A real double-click into the component opens the explode session.
  const p = worldToPagePixel({ x: 3.5, y: 0.5, z: 1 }, vp, rect)!
  await page.mouse.move(p.x, p.y)
  await page.mouse.click(p.x, p.y, { clickCount: 2 })
  await expect.poll(() => page.evaluate(() => window.__hew_test!.getExplodeSessionInstance())).toBe(instance)
  // The kernel hides the entered instance, so its pose reads as gone.
  expect(await page.evaluate((id) => window.__hew_test!.getInstancePose(id), instance)).toBeNull()

  await expect.poll(() => page.evaluate(() => window.__hew_test!.getPartAxesInfo().origins.length)).toBeGreaterThan(0)
  const info = await page.evaluate(() => window.__hew_test!.getPartAxesInfo())
  expect(info.origins).toHaveLength(1)
  expect(info.origins[0][0]).toBeCloseTo(pose[3])
  expect(info.origins[0][1]).toBeCloseTo(pose[7])
  expect(info.origins[0][2]).toBeCloseTo(pose[11])

  // Leaving the session keeps the triad for what is selected.
  await page.keyboard.press('Escape')
  await expect.poll(() => page.evaluate(() => window.__hew_test!.getExplodeSessionInstance())).toBeNull()
  await page.evaluate((id) => window.__hew_test!.selectNodes([{ kind: 'instance', id }]), instance)
  await expect.poll(() => page.evaluate(() => window.__hew_test!.getPartAxesInfo().origins.length)).toBe(1)
})

test('hiding_the_selected_component_removes_its_triad', async ({ page }) => {
  await setup(page)
  const { instance } = await page.evaluate(() => {
    const h = window.__hew_test!
    return h.makeComponent([h.drawBox([2, 3, 0], [4, 5, 0], 2)])
  })
  await page.evaluate((id) => window.__hew_test!.selectNodes([{ kind: 'instance', id }]), instance)
  await togglePartAxes(page)
  await expect.poll(() => page.evaluate(() => window.__hew_test!.getPartAxesInfo().origins.length)).toBe(1)
  await page.evaluate((id) => window.__hew_test!.toggleNodeHidden({ kind: 'instance', id }), instance)
  await expect.poll(() => page.evaluate(() => window.__hew_test!.getPartAxesInfo().origins.length)).toBe(0)
  await page.evaluate((id) => window.__hew_test!.toggleNodeHidden({ kind: 'instance', id }), instance)
  await expect.poll(() => page.evaluate(() => window.__hew_test!.getPartAxesInfo().origins.length)).toBe(1)
})

async function selectedTriadCount(page: Page): Promise<number> {
  return page.evaluate(() => window.__hew_test!.getPartAxesInfo().origins.length)
}

test('a_component_hidden_through_its_group_or_a_tag_has_no_triad', async ({ page }) => {
  await setup(page)
  const { instance } = await page.evaluate(() => {
    const h = window.__hew_test!
    return h.makeComponent([h.drawBox([2, 3, 0], [4, 5, 0], 2)])
  })
  await page.evaluate((id) => window.__hew_test!.selectNodes([{ kind: 'instance', id }]), instance)
  await togglePartAxes(page)
  await expect.poll(() => selectedTriadCount(page)).toBe(1)

  // A group holding the board: hiding the group hides the board, not by its own flag.
  const group = await page.evaluate((id) => window.__hew_test!.groupNodes([{ kind: 'instance', id }]), instance)
  await page.evaluate((id) => window.__hew_test!.selectNodes([{ kind: 'instance', id }]), instance)
  await expect.poll(() => selectedTriadCount(page)).toBe(1)
  await page.evaluate((id) => window.__hew_test!.toggleNodeHidden({ kind: 'group', id }), group)
  expect(await page.evaluate((id) => window.__hew_test!.isNodeHidden({ kind: 'instance', id }), instance)).toBe(false)
  await expect.poll(() => selectedTriadCount(page)).toBe(0)
  await page.evaluate((id) => window.__hew_test!.toggleNodeHidden({ kind: 'group', id }), group)
  await expect.poll(() => selectedTriadCount(page)).toBe(1)

  // A hidden tag on the board hides it too.
  await page.evaluate((id) => {
    const h = window.__hew_test!
    h.addNodeTag('instance', id, ['Hardware'])
    h.toggleTagHidden(['Hardware'])
  }, instance)
  await expect.poll(() => selectedTriadCount(page)).toBe(0)
  await page.evaluate(() => window.__hew_test!.toggleTagHidden(['Hardware']))
  await expect.poll(() => selectedTriadCount(page)).toBe(1)

  // Isolating another object leaves the board undrawn, so its triad goes
  // too, and comes back when the isolation is cleared.
  const isolated = await page.evaluate(() => {
    const h = window.__hew_test!
    h.drawBox([8, 0, 0], [9, 1, 0], 1)
    const rows = (h.apiDispatch({ method: 'hew.query.occurrences', params: { kinds: ['object'] } }).result as {
      occurrences: { id: string; path: string[] }[]
    }).occurrences
    const other = rows.find((r) => r.path.length === 0)!
    return h.apiDispatch({ method: 'hew.view.isolate', params: { ids: [other.id] } })
  })
  expect(isolated.error).toBeUndefined()
  await expect.poll(() => selectedTriadCount(page)).toBe(0)
  const cleared = await page.evaluate(() => window.__hew_test!.apiDispatch({ method: 'hew.view.clear_isolation' }))
  expect(cleared.error).toBeUndefined()
  await expect.poll(() => selectedTriadCount(page)).toBe(1)
})

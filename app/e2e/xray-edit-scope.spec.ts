/**
 * Picking follows the editing scope in Shaded and X-ray (docs/design/xray-edit-scope-pick.md).
 *
 * The reporter's scene: a box (A) with a component (B) half buried in it.
 * Double-click B's visible part to edit it, choose Push/Pull and click B's
 * buried face through A. With a component open for editing, geometry outside
 * it takes no click (Shaded or X-ray; Wireframe picks as before): the pick
 * passes through A and lands on B's face.
 *
 * Real pointer and keyboard events throughout; `pickFace` gives the
 * pixel-free check of what the kernel's pick returns.
 */
import { test, expect, type Page } from '@playwright/test'

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

test.skip(({ browserName }) => browserName !== 'chromium', 'pointer-driven spec, chromium only')

type Vec3 = [number, number, number]

// A: [0,-1]..[2,3], height 2.   B: [1,0.5]..[3,1.5], height 1 (x 1..2 buried).
const EYE: Vec3 = [1.5, -6, 8]
/** B's visible top face, outside A. */
const B_VISIBLE: Vec3 = [2.6, 1, 1]
/** B's buried top face, inside A: the ray from EYE passes A's top first. */
const B_BURIED: Vec3 = [1.5, 1, 1]
/** A's top face where nothing of B lies behind along the view ray. */
const A_EMPTY: Vec3 = [0.4, 2.2, 2]

const dir = (from: Vec3, to: Vec3): Vec3 => [to[0] - from[0], to[1] - from[1], to[2] - from[2]]

async function px(page: Page, w: Vec3): Promise<{ x: number; y: number }> {
  const box = await page.locator('canvas').first().boundingBox()
  if (box === null) throw new Error('viewport canvas has no bounding box')
  const p = await page.evaluate((w) => window.__hew_test!.worldToScreen(w), w)
  return { x: box.x + p.x, y: box.y + p.y }
}

async function hover(page: Page, w: Vec3): Promise<{ x: number; y: number }> {
  const p = await px(page, w)
  await page.mouse.move(p.x - 6, p.y - 4)
  await page.waitForTimeout(50)
  await page.mouse.move(p.x, p.y)
  await page.waitForTimeout(200)
  return p
}

async function chooseView(page: Page, name: 'X-ray' | 'Wireframe'): Promise<void> {
  if (!(await page.getByText('Wireframe').isVisible())) {
    await page.getByRole('button', { name: /^view$/i }).click()
    await expect(page.getByText('Wireframe')).toBeVisible()
  }
  await page.getByText(name).click()
  await page.waitForTimeout(250)
}

async function tool(page: Page, name: string): Promise<void> {
  await page.getByRole('radio', { name }).click()
}

const REFUSAL = /isn.t part of what you are editing/
const hint = (page: Page) => page.getByTestId('inference-tip-hint').filter({ hasText: REFUSAL })

async function setup(page: Page) {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
  const ids = await page.evaluate(
    ({ eye }) => {
      const h = window.__hew_test!
      h.setGridVisible(false)
      h.setAxesVisible(false)
      const a = h.drawBox([0, -1, 0], [2, 3, 0], 2)
      const b = h.drawBox([1, 0.5, 0], [3, 1.5, 0], 1)
      const { instance, component } = h.makeComponent([b])
      const sibling = h.placeInstance(component, 0, 6, 0)
      h.setCamera({ position: eye, target: [1.5, 1, 1], up: [0, 0, 1], fovDeg: 50 })
      return { a, b, instance, component, sibling }
    },
    { eye: EYE },
  )
  return ids
}

async function openComponent(page: Page): Promise<void> {
  const p = await px(page, B_VISIBLE)
  await page.mouse.move(p.x, p.y)
  await page.mouse.click(p.x, p.y, { clickCount: 2 })
  await page.waitForTimeout(250)
}

const pick = (page: Page, at: Vec3) =>
  page.evaluate(({ eye, d }) => window.__hew_test!.pickFace(eye, d), { eye: EYE, d: dir(EYE, at) })

test('Shaded and X-ray with a component open: Push/Pull reaches its buried face through the enclosing box', async ({ page }) => {
  test.setTimeout(120_000)
  const ids = await setup(page)
  await openComponent(page)
  const member = (await page.evaluate((c) => window.__hew_test!.getComponentMemberObjects(c), ids.component))[0]
  expect(await page.evaluate(() => window.__hew_test!.getExplodeSessionInstance())).toBe(ids.instance)

  // Shaded: with the component open the pick passes through A and lands on
  // B's member face; no refusal hint.
  expect((await pick(page, B_BURIED))?.object).toBe(member)
  await tool(page, 'Push/Pull')
  await hover(page, B_BURIED)
  await expect(hint(page)).toBeHidden()

  // Wireframe has no face targets: the scope changes nothing there.
  await chooseView(page, 'Wireframe')
  expect(await pick(page, B_BURIED)).toBeNull()

  // X-ray: the same result as Shaded.
  await chooseView(page, 'X-ray')
  expect((await pick(page, B_BURIED))?.object).toBe(member)
  await hover(page, B_BURIED)
  await expect(hint(page)).toBeHidden()

  // Nothing of B behind this part of A: today's hint, no silent nothing.
  expect((await pick(page, A_EMPTY))?.object).toBe(ids.a)
  await hover(page, A_EMPTY)
  await expect(hint(page)).toBeVisible()

  // Push/Pull B's buried face with real clicks.
  const aBefore = await page.evaluate((id) => window.__hew_test!.getObjectBounds(id), ids.a)
  const bBefore = await page.evaluate((id) => window.__hew_test!.getObjectBounds(id), member)
  // The sibling placement sits 6 m along +Y: a horizontal ray at z = 1.3
  // misses B's old top (z = 1) there and meets it once B is pushed up.
  const siblingRay = () =>
    page.evaluate(() => window.__hew_test!.pickFace([2.6, 12, 1.3], [0, -1, 0]))
  expect(await siblingRay()).toBeNull()
  await hover(page, B_BURIED)
  await page.mouse.down()
  await page.mouse.up()
  const to = await px(page, [1.5, 1, 1.6])
  await page.mouse.move(to.x, to.y, { steps: 10 })
  await page.mouse.down()
  await page.mouse.up()
  await page.waitForTimeout(250)

  const bAfter = await page.evaluate((id) => window.__hew_test!.getObjectBounds(id), member)
  const aAfter = await page.evaluate((id) => window.__hew_test!.getObjectBounds(id), ids.a)
  expect(bAfter).not.toEqual(bBefore)
  expect(bAfter[5]).toBeGreaterThan(bBefore[5] + 0.1)
  expect(aAfter).toEqual(aBefore)
  // The sibling copy follows: its placement of B now reaches z = 1.3.
  const sibHit = await siblingRay()
  expect(sibHit).not.toBeNull()
  expect(sibHit?.object).toBe(member)

  // Closing the session keeps both placements of the edited definition.
  await page.keyboard.press('Escape')
  await page.waitForTimeout(250)
  const left = await page.evaluate((c) => window.__hew_test!.getInstancesOf(c), ids.component)
  expect(new Set(left)).toEqual(new Set([ids.instance, ids.sibling]))
})

test('Shaded with a component open: a Select click through the box selects the member', async ({ page }) => {
  test.setTimeout(120_000)
  const ids = await setup(page)
  await openComponent(page)
  const member = (await page.evaluate((c) => window.__hew_test!.getComponentMemberObjects(c), ids.component))[0]
  await tool(page, 'Select')
  await page.evaluate(() => window.__hew_test!.selectObjects([]))
  await hover(page, B_BURIED)
  await page.mouse.down()
  await page.mouse.up()
  await page.waitForTimeout(200)
  const sel = await page.evaluate(() => window.__hew_test!.getSelection().map((n) => ({ kind: n.kind, id: n.id })))
  expect(sel).toEqual([{ kind: 'object', id: member }])
})

test('top level (no scope) still picks the enclosing box, Shaded and X-ray', async ({ page }) => {
  test.setTimeout(120_000)
  const ids = await setup(page)
  expect((await pick(page, B_BURIED))?.object).toBe(ids.a)
  await chooseView(page, 'X-ray')
  expect((await pick(page, B_BURIED))?.object).toBe(ids.a)
  await tool(page, 'Select')
  await page.evaluate(() => window.__hew_test!.selectObjects([]))
  await hover(page, B_BURIED)
  await page.mouse.down()
  await page.mouse.up()
  await page.waitForTimeout(200)
  const sel = await page.evaluate(() => window.__hew_test!.getSelection().map((n) => ({ kind: n.kind, id: n.id })))
  expect(sel).toEqual([{ kind: 'object', id: ids.a }])
})

test('Shaded double-click through the box enters the in-scope plain object behind it', async ({ page }) => {
  test.setTimeout(120_000)
  const ids = await setup(page)
  // A plain solid inside the open component's session scope: draw it
  // mid-session (it folds into the session's world geometry).
  await openComponent(page)
  const extra = await page.evaluate(() => window.__hew_test!.drawBox([0.2, 0.6, 0], [0.8, 1.4, 0], 0.5))
  await page.evaluate((id) => window.__hew_test!.setNodeName('object', id, 'XrayExtraBox'), extra)
  const aim: Vec3 = [0.5, 1, 0.5]
  const hit = await pick(page, aim)
  expect(hit?.object).toBe(extra)
  const path = page.getByTestId('selection-path')
  const before = await path.textContent()

  const p = await px(page, aim)
  await page.mouse.move(p.x, p.y)
  await page.mouse.click(p.x, p.y, { clickCount: 2 })
  await page.waitForTimeout(250)
  // Entered: the extra box (by name) is now the editing context, and A is
  // untouched.
  expect(await path.textContent()).not.toBe(before)
  expect(await path.textContent()).toContain('XrayExtraBox')
  expect(await page.evaluate((id) => window.__hew_test!.getObjectBounds(id), ids.a)).toEqual([0, -1, 0, 2, 3, 2])
})

import { test, expect, type Page } from '@playwright/test'
import {
  buildViewProjection,
  worldToPagePixel,
  type CameraParams,
  type Mat4,
} from './helpers/projectWorldToScreen'

/**
 * `hew.view.enter` through the harness's bus dispatch (the live bridge's own
 * answered-dispatch path): the kernel accepts a target, the window then acts
 * on it, and the call answers what the window did. An entry the window
 * declines, here a component that nests another on an instance no session
 * can take (the toast the person sees), answers the named refusal and leaves
 * the window where it was; an ordinary entry answers `{}` and opens the
 * session.
 */

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

interface Ctx {
  vp: Mat4
  rect: { left: number; top: number; width: number; height: number }
}

type Reply = {
  result?: unknown
  error?: { code: number; message: string; data?: { refusal?: string; failed_method?: string } }
}

async function ready(page: Page): Promise<void> {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
}

async function aim(page: Page, camera: CameraParams): Promise<Ctx> {
  await page.evaluate(
    (cam) =>
      window.__hew_test!.setCamera({
        position: [cam.position.x, cam.position.y, cam.position.z],
        target: [cam.target.x, cam.target.y, cam.target.z],
        up: [cam.up.x, cam.up.y, cam.up.z],
        fovDeg: cam.fovDeg,
      }),
    camera,
  )
  const box = await page.locator('canvas').first().boundingBox()
  if (box === null) throw new Error('viewport canvas has no bounding box')
  const rect = { left: box.x, top: box.y, width: box.width, height: box.height }
  return { vp: buildViewProjection(camera, rect.width / rect.height), rect }
}

async function dblClickWorld(page: Page, ctx: Ctx, x: number, y: number, z: number): Promise<void> {
  const p = worldToPagePixel({ x, y, z }, ctx.vp, ctx.rect)
  if (p === null) throw new Error(`world (${x},${y},${z}) does not project onto the canvas`)
  await page.mouse.move(p.x, p.y)
  await page.mouse.click(p.x, p.y, { clickCount: 2 })
}

const dispatch = (page: Page, method: string, params: unknown = {}): Promise<Reply> =>
  page.evaluate(([m, p]) => window.__hew_test!.apiDispatch({ method: m as string, params: p }), [method, params]) as Promise<Reply>

/** `hew.query.context`: the open frames and in-context entries. */
async function context(page: Page): Promise<{ stack: unknown[]; fallback: unknown[] }> {
  const { stack, fallback } = (await dispatch(page, 'hew.query.context')).result as {
    stack: unknown[]
    fallback: unknown[]
  }
  return { stack, fallback }
}

/** The history the window holds: how many steps Undo and Redo would take. */
const depths = (page: Page): Promise<{ undo: number; redo: number }> =>
  page.evaluate(() => ({ undo: window.__hew_test!.getUndoDepth(), redo: window.__hew_test!.getRedoDepth() }))

/**
 * Draws a marker step, then measures what one Undo does from here (and puts
 * it back): the model hash one Undo gives, to compare with the same Undo
 * after a refused call.
 */
async function markerStep(page: Page): Promise<string> {
  return page.evaluate(() => {
    const h = window.__hew_test!
    h.drawBox([8, 0, 0], [9, 1, 0], 1)
    h.undo()
    const afterOneUndo = h.getModelStateHash()
    h.redo()
    return afterOneUndo
  })
}

// A 3/4 view covering the standalone part (x:[0,1]) and the assembly
// (x:[3,4]) at z:[0,1].
const CAMERA: CameraParams = {
  position: { x: 8, y: -8, z: 8 },
  target: { x: 2, y: 0.5, z: 0.5 },
  up: { x: 0, y: 0, z: 1 },
  fovDeg: 60,
  near: 0.1,
  far: 1000,
}

test('hew.view.enter answers the refusal when the window declines, and the window stays put', async ({ page }) => {
  const ctx = await ready(page).then(() => aim(page, CAMERA))

  // A part, and an assembly that nests a placement of it: the assembly's
  // session is opened with a real double click, a part placed inside, and
  // the session closed again (the way nested-component.spec.ts builds it).
  const setup = await page.evaluate(() => {
    const h = window.__hew_test!
    const partBox = h.drawBox([0, 0, 0], [1, 1, 0], 1)
    const { instance: partInstance, component: partComponent } = h.makeComponent([partBox])
    const assemblyBox = h.drawBox([3, 0, 0], [4, 1, 0], 1)
    const { instance: assemblyInstance } = h.makeComponent([assemblyBox])
    return { partInstance, partComponent, assemblyInstance }
  })
  await dblClickWorld(page, ctx, 3.5, 0.5, 1)
  await page.waitForTimeout(200)
  expect(await page.evaluate(() => window.__hew_test!.getExplodeSessionInstance())).toBe(setup.assemblyInstance)
  await page.evaluate((c) => window.__hew_test!.placeInstance(c, 3, 3, 0), setup.partComponent)
  await page.keyboard.press('Escape')
  await page.waitForTimeout(200)
  expect(await page.evaluate(() => window.__hew_test!.getExplodeSessionInstance())).toBeNull()

  // The bus names entities its own way: the assembly is the top-level
  // instance holding another, the part the one holding none.
  const rows = (
    (await dispatch(page, 'hew.query.occurrences', { kinds: ['instance'] })).result as {
      occurrences: { id: string; depth: number; members: { instances: string[] } }[]
    }
  ).occurrences.filter((r) => r.depth === 0)
  const assemblyId = rows.find((r) => r.members.instances.length > 0)!.id
  const partId = rows.find((r) => r.members.instances.length === 0)!.id

  // Stretch the assembly on one axis only: no session can take it, and its
  // definition nests a component, so the window refuses the in-context
  // fallback with a toast.
  const scaled = await dispatch(page, 'hew.entity.scale', {
    ids: [assemblyId],
    anchor: [3.5, 0.5, 0],
    factors: [1, 2, 1],
  })
  expect(scaled.error).toBeUndefined()

  const before = await context(page)
  expect(before).toEqual({ stack: [], fallback: [] })
  const hashAfterOneUndo = await markerStep(page)
  const historyBefore = await depths(page)

  // ---- the declined entry
  const refused = await dispatch(page, 'hew.view.enter', { id: assemblyId })
  expect(refused.result).toBeUndefined()
  expect(refused.error).toMatchObject({
    code: -32000,
    data: { refusal: 'nested_component_in_context', failed_method: 'hew.view.enter' },
  })
  expect(await context(page)).toEqual(before)
  expect(await page.evaluate(() => window.__hew_test!.getExplodeSessionInstance())).toBeNull()
  // A refused single step records nothing: history is exactly as it was,
  // and one Undo takes back the step before the call, as if it never came.
  expect(await depths(page)).toEqual(historyBefore)
  await page.evaluate(() => window.__hew_test!.undo())
  expect(await page.evaluate(() => window.__hew_test!.getModelStateHash())).toBe(hashAfterOneUndo)
  await page.evaluate(() => window.__hew_test!.redo())

  // ---- the ordinary entry: the standalone part opens a session
  const entered = await dispatch(page, 'hew.view.enter', { id: partId })
  expect(entered.error).toBeUndefined()
  expect(entered.result).toEqual({})
  expect(await page.evaluate(() => window.__hew_test!.getExplodeSessionInstance())).toBe(setup.partInstance)
  expect((await context(page)).stack).toHaveLength(1)
})

type Occurrence = { id: string; depth: number; path: string[]; definition_name: string }
const occurrences = async (page: Page): Promise<Occurrence[]> =>
  ((await dispatch(page, 'hew.query.occurrences', { kinds: ['instance'] })).result as { occurrences: Occurrence[] })
    .occurrences

test('hew.view.enter through a path undoes the steps it took before the one the window declines', async ({ page }) => {
  const ctx = await ready(page).then(() => aim(page, CAMERA))

  // A small part (Component 1); Q (Component 2) nesting a placement of it;
  // an assembly A (Component 3) nesting a placement of Q. Each nesting is
  // built the way a person does: open the outer session with a double
  // click, place the inner instance, close it.
  const part = await page.evaluate(() => {
    const h = window.__hew_test!
    return h.makeComponent([h.drawBox([-3, 0, 0], [-2, 1, 0], 1)])
  })
  const q = await page.evaluate(() => {
    const h = window.__hew_test!
    return h.makeComponent([h.drawBox([0, 0, 0], [1, 1, 0], 1)])
  })
  await dblClickWorld(page, ctx, 0.5, 0.5, 1)
  await page.waitForTimeout(200)
  expect(await page.evaluate(() => window.__hew_test!.getExplodeSessionInstance())).toBe(q.instance)
  await page.evaluate((c) => window.__hew_test!.placeInstance(c, -3, 3, 0), part.component)
  await page.keyboard.press('Escape')
  await page.waitForTimeout(200)
  const standaloneQ = (await occurrences(page)).find((r) => r.depth === 0 && r.definition_name === 'Component 2')!.id

  const a = await page.evaluate(() => {
    const h = window.__hew_test!
    return h.makeComponent([h.drawBox([3, 0, 0], [4, 1, 0], 1)])
  })
  await dblClickWorld(page, ctx, 3.5, 0.5, 1)
  await page.waitForTimeout(200)
  expect(await page.evaluate(() => window.__hew_test!.getExplodeSessionInstance())).toBe(a.instance)
  await page.evaluate((c) => window.__hew_test!.placeInstance(c, 3, 3, 0), q.component)
  // While the placement is a world node of the open session, stretch it on
  // one axis only: no session can take it afterward.
  const qInA = (await occurrences(page)).find(
    (r) => r.depth === 0 && r.definition_name === 'Component 2' && r.id !== standaloneQ,
  )!.id
  const scaled = await dispatch(page, 'hew.entity.scale', { ids: [qInA], anchor: [3.5, 3.5, 0], factors: [1, 2, 1] })
  expect(scaled.error).toBeUndefined()
  await page.keyboard.press('Escape')
  await page.waitForTimeout(200)
  expect(await page.evaluate(() => window.__hew_test!.getExplodeSessionInstance())).toBeNull()

  const rows = await occurrences(page)
  const assemblyId = rows.find((r) => r.depth === 0 && r.definition_name === 'Component 3')!.id
  const nestedQ = rows.find((r) => r.path.length === 1 && r.path[0] === assemblyId && r.definition_name === 'Component 2')!.id
  const before = await context(page)
  expect(before).toEqual({ stack: [], fallback: [] })
  const hashAfterOneUndo = await markerStep(page)
  const historyBefore = await depths(page)

  // The assembly takes a session; the stretched placement inside it is then
  // declined. The refusal says so, and the assembly's session is closed
  // again: nothing is left half entered.
  // The event sink is reached through the dev server's module graph, which a
  // built app does not serve: there the events are not counted, and
  // everything else below is still checked.
  const counting = await page.evaluate(async () => {
    let pump: { registerEventSink(sink: (conn: number, frame: string) => void): void }
    try {
      pump = await import(/* @vite-ignore */ '/src/api/eventPump.ts')
    } catch {
      return false
    }
    const seen: string[] = []
    ;(window as unknown as { __seen: string[] }).__seen = seen
    pump.registerEventSink((_conn: number, frame: string) => seen.push(frame))
    window.__hew_test!.apiDispatch({ method: 'hew.event.subscribe', params: { events: ['context_changed'] } })
    return true
  })
  const refused = await dispatch(page, 'hew.view.enter', { path: [assemblyId, nestedQ] })
  // The steps it opened and took back are each reported, and the last event
  // says where the window is: back at the top.
  if (counting) {
    const seen = (
      await page.evaluate(() => (window as unknown as { __seen: string[] }).__seen)
    ).map((f) => (JSON.parse(f) as { params: { stack: unknown[] } }).params.stack.length)
    expect(seen).toEqual([1, 0])
  }
  expect(refused.error).toMatchObject({
    code: -32000,
    data: { refusal: 'nested_component_in_context', failed_method: 'hew.view.enter' },
  })
  expect(await context(page)).toEqual(before)
  expect(await page.evaluate(() => window.__hew_test!.getExplodeSessionInstance())).toBeNull()
  expect(await page.evaluate(() => window.__hew_test!.getSelection())).toEqual([])
  // The session the walk opened was taken back with Undo, not closed by a
  // second step: the undo depth is what it was, and one Undo now takes back
  // the step before the call.
  const after = await depths(page)
  expect(after.undo).toBe(historyBefore.undo)
  // Undo has no way to drop the step it took back: Redo would reopen the
  // assembly, and any redo stack the person had before is gone.
  expect(after.redo).toBe(historyBefore.redo + 1)
  await page.evaluate(() => window.__hew_test!.undo())
  expect(await page.evaluate(() => window.__hew_test!.getModelStateHash())).toBe(hashAfterOneUndo)
})

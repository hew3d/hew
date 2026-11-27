/**
 * Real-pointer helpers for the shape tools' specs (Revolve, Deform, Follow Me
 * twist): pin a camera, project world points to page pixels, and move, click
 * and drag the mouse there. The scene is built through the harness (not what
 * is under test); only the tool interaction uses synthesized pointer events.
 */

import { expect, type Page } from '@playwright/test'
import {
  buildViewProjection,
  worldToPagePixel,
  type CameraParams,
  type Mat4,
} from './projectWorldToScreen'
import { activateFromToolsMenu } from './offset'

export interface Ctx {
  vp: Mat4
  rect: { left: number; top: number; width: number; height: number }
}

/** Wait for the harness, pin `camera`, and return the projection context. */
export async function aim(page: Page, camera: CameraParams): Promise<Ctx> {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
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

export function px(ctx: Ctx, x: number, y: number, z: number): { x: number; y: number } {
  const p = worldToPagePixel({ x, y, z }, ctx.vp, ctx.rect)
  if (p === null) throw new Error(`world (${x},${y},${z}) does not project onto the canvas`)
  return p
}

/** Move the mouse onto a world point (a fresh move, so it is not coalesced away). */
export async function moveTo(page: Page, ctx: Ctx, x: number, y: number, z: number): Promise<void> {
  const p = px(ctx, x, y, z)
  await page.mouse.move(p.x - 4, p.y - 4)
  await page.waitForTimeout(40)
  await page.mouse.move(p.x, p.y)
  await page.waitForTimeout(120)
}

export async function clickAt(page: Page, ctx: Ctx, x: number, y: number, z: number): Promise<void> {
  await moveTo(page, ctx, x, y, z)
  await page.mouse.down()
  await page.mouse.up()
  await page.waitForTimeout(150)
}

/** Move through `points` in small steps (a drag in the click-move-click sense). */
export async function sweep(
  page: Page,
  ctx: Ctx,
  points: [number, number, number][],
  stepsPerLeg = 6,
): Promise<void> {
  for (let i = 0; i < points.length; i++) {
    const to = px(ctx, ...points[i])
    await page.mouse.move(to.x, to.y, { steps: stepsPerLeg })
    await page.waitForTimeout(60)
  }
}

/** Pick a tool from the rail and wait for its first status hint. */
export async function activateFromRail(page: Page, name: string, firstHint: string | RegExp): Promise<void> {
  await page.getByRole('radio', { name }).click()
  await expect(page.getByText(firstHint)).toBeVisible()
}

/** Activate a tool that has no rail row (Revolve, Deform's modes) through
 *  the Tools menu with real clicks, then wait for the tool's first hint. */
export async function activateFromMenu(page: Page, label: string, firstHint: string | RegExp): Promise<void> {
  await activateFromToolsMenu(page, label)
  await expect(page.getByText(firstHint)).toBeVisible()
}

/** Save a screenshot of the viewport when HEW_SHOTS_DIR names a folder. */
export async function shot(page: Page, name: string, nudge = false): Promise<void> {
  const dir = process.env.HEW_SHOTS_DIR
  if (dir === undefined || dir === '') return
  // The viewport redraws on pointer activity: `nudge` jiggles the mouse so a
  // ghost the last CLICK just built is on screen (never during a drag, where
  // the pointer position is the input).
  if (nudge) {
    await page.mouse.move(1, 1)
    await page.mouse.move(2, 2)
  }
  await page.waitForTimeout(250)
  await page.screenshot({ path: `${dir}/${name}.png` })
}

export function near(a: number, b: number, tol = 1e-4): boolean {
  return Math.abs(a - b) <= tol
}

import { expect, type Page } from '@playwright/test'

/**
 * Shared steps for the offset-family specs (Shell, Project, Smooth, Eraser,
 * Model Health): boot the harness, point the camera, and drive the tools with
 * real pointer and keyboard events. The scene is built through the semantic
 * harness; only the tool interaction is synthesized input.
 */

declare global {
  interface Window {
    __hew_test?: import('../../src/test/harness').HewTestHarness
  }
}

export type Vec3 = [number, number, number]

export async function setup(page: Page, camera: { position: Vec3; target: Vec3; up?: Vec3; fovDeg?: number }): Promise<void> {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 60_000 })
  await page.evaluate((cam) => window.__hew_test!.setCamera({ up: [0, 0, 1], fovDeg: 45, ...cam }), camera)
  await page.waitForTimeout(100)
}

export async function pagePoint(page: Page, w: Vec3): Promise<{ x: number; y: number }> {
  const box = await page.locator('canvas').first().boundingBox()
  if (box === null) throw new Error('no canvas')
  const p = await page.evaluate((w) => window.__hew_test!.worldToScreen(w), w)
  if (p.behind) throw new Error(`world point ${w.join(',')} is behind the camera`)
  return { x: box.x + p.x, y: box.y + p.y }
}

export async function moveToWorld(page: Page, w: Vec3): Promise<void> {
  const p = await pagePoint(page, w)
  await page.mouse.move(p.x, p.y)
}

/** Hover first, as a real hand does, then press and release. */
export async function clickWorld(page: Page, w: Vec3): Promise<void> {
  await moveToWorld(page, w)
  await page.waitForTimeout(60)
  await page.mouse.down()
  await page.mouse.up()
  await page.waitForTimeout(60)
}

/** The Tools menu's two submenus and what they hold. */
const TOOLS_SUBMENUS: Record<string, readonly string[]> = {
  Shape: ['Revolve', 'Loft', 'Shell', 'Project', 'Array along Path'],
  Deform: ['Twist', 'Bend', 'Taper', 'Bend to Path'],
}

/** Tools with no rail slot live in the MenuBar's Tools dropdown (the Shape and
 *  Deform families inside their submenus, opened by hovering the row). */
export async function activateFromToolsMenu(page: Page, label: string): Promise<void> {
  const menuBar = page.getByTestId('menu-bar')
  await menuBar.getByRole('button', { name: /^tools$/i }).click()
  const submenu = Object.keys(TOOLS_SUBMENUS).find((name) => TOOLS_SUBMENUS[name].includes(label))
  if (submenu !== undefined) await menuBar.getByText(submenu, { exact: true }).hover()
  await menuBar.getByText(label, { exact: true }).click()
  await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur())
}

export async function volume(page: Page, id: string): Promise<number> {
  return page.evaluate((id) => window.__hew_test!.getObjectVolume(id), id)
}

/** Save a screenshot when HEW_SHOTS_DIR is set (the hand-off's playtest images). */
export async function shot(page: Page, name: string): Promise<void> {
  const dir = process.env.HEW_SHOTS_DIR
  if (dir === undefined || dir === '') return
  await page.screenshot({ path: `${dir}/${name}.png` })
}

export async function expectStatus(page: Page, text: string | RegExp): Promise<void> {
  await expect(page.getByText(text).first()).toBeVisible()
}

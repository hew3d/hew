import type { Page } from '@playwright/test'

/**
 * Wait for the viewport to present the frame produced by the last harness
 * mutation. Two RAFs: a `reconcile()` schedules the render on the first frame,
 * the compositor presents it on the next. Used before any screenshot/readback
 * so the canvas reflects current document state. Shared by the web smoke and the
 *  visual goldens.
 */
export async function settleFrame(page: Page): Promise<void> {
  await page.evaluate(
    () => new Promise<void>((r) => requestAnimationFrame(() => requestAnimationFrame(() => r()))),
  )
}

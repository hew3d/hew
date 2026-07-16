import { test, expect, type Page } from '@playwright/test'

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

/**
 * Tags × undo/redo wiring — EVERY undo entry point must reconcile tag
 * visibility, including the viewport's own Cmd/Ctrl+Z keydown binding
 * (which never passes through App.handleUndo; only the menu and palette
 * do). The regression this pins: undoing a tag delete restores the
 * registry entry with its hidden flag, and without a resync on the
 * keyboard path the kernel considers the tag hidden again while the
 * content stays visible AND pickable until some unrelated resync runs.
 *
 * Pickability is the observable: the tag-visibility union is pushed to
 * both the renderer (mesh visibility) and the kernel (inference/pick
 * exclusion) in one call, and `pickFace` reads the kernel side exactly.
 */

async function setup(page: Page): Promise<void> {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, {
    timeout: 15_000,
  })
}

test('keyboard undo of a tag delete re-hides content hidden by the restored tag', async ({
  page,
}) => {
  await setup(page)

  // A unit box tagged 'Roof'; a downward ray over its top is the probe.
  await page.evaluate(() => {
    const h = window.__hew_test!
    const id = h.drawBox([0, 0, 0], [1, 1, 0], 1)
    h.addNodeTag('object', id, ['Roof'])
  })
  expect(
    await page.evaluate(() => window.__hew_test!.pickFace([0.5, 0.5, 5], [0, 0, -1]) !== null),
  ).toBe(true)

  // Hide the tag (the Tags panel eye): content goes invisible + unpickable.
  await page.evaluate(() => window.__hew_test!.toggleTagHidden(['Roof']))
  await page.waitForFunction(
    () => window.__hew_test!.pickFace([0.5, 0.5, 5], [0, 0, -1]) === null,
  )

  // Delete the tag: content hidden solely via that tag becomes visible.
  await page.evaluate(() => window.__hew_test!.deleteTag(['Roof']))
  await page.waitForFunction(
    () => window.__hew_test!.pickFace([0.5, 0.5, 5], [0, 0, -1]) !== null,
  )

  // Undo through the LIVE keydown path — the viewport's own Ctrl+Z binding,
  // not App.handleUndo. The tag registry is restored with its hidden flag,
  // so the content must re-hide (and become unpickable) immediately.
  await page.keyboard.press('Control+z')
  await page.waitForFunction(
    () => window.__hew_test!.pickFace([0.5, 0.5, 5], [0, 0, -1]) === null,
  )

  // The geometry itself was never touched — only its visibility.
  expect(await page.evaluate(() => window.__hew_test!.getObjectCount())).toBe(1)

  // Redo through the live keydown path re-deletes the tag: visible again.
  await page.keyboard.press('Control+Shift+Z')
  await page.waitForFunction(
    () => window.__hew_test!.pickFace([0.5, 0.5, 5], [0, 0, -1]) !== null,
  )
})

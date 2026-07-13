import { test, expect } from '@playwright/test'
import { settleFrame } from './helpers/render'

/**
 *  — shell/session journey specs.
 *
 * Tests the full lifecycle of the app session beyond individual modeling ops:
 *
 *   1. Multi-object + guide save/load fidelity (through `FileHost` / `Scene.save`
 *      / the real Open path — harness.save() + harness.load()).
 *   2. Autosave → recovery round-trip: seed the WebRecoveryStore (IndexedDB)
 *      directly, reload the page, confirm the recovery dialog, click "Recover",
 *      and verify the restored document state.
 *   3. Panel presence: floating panels are mounted and accessible in the DOM.
 *   4. Unit format persistence: the active length unit survives a page reload
 *      (localStorage key `hew.settings.lengthUnit`).
 *   5. Undo multi-step + save: undo a series of ops, then save — the saved
 *      document matches the undone state, not the original.
 */

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

// ---------------------------------------------------------------------------
// Helper: wait for the harness to be ready after a (re)load.
// ---------------------------------------------------------------------------
async function waitForHarness(page: Parameters<typeof settleFrame>[0]): Promise<void> {
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, {
    timeout: 20_000,
  })
}

// ---------------------------------------------------------------------------
// 1. Multi-object + guide save/load fidelity
// ---------------------------------------------------------------------------

test('save/load: multi-object scene + guide round-trips faithfully', async ({ page }) => {
  await page.goto('/')
  await waitForHarness(page)

  const beforeReload = await page.evaluate(() => {
    const h = window.__hew_test!

    // Build three boxes at different positions.
    h.drawBox([0, 0, 0], [1, 1, 0], 1)
    h.drawBox([2, 0, 0], [3, 1, 0], 2)
    h.drawBox([0, 2, 0], [1, 3, 0], 1.5)

    // Add a construction guide line.
    h.addGuideLine(0, 0, 0, 1, 0, 0)

    const hash = h.getStateHash()
    const count = h.getObjectCount()
    const guideCount = h.getGuideIds().length
    const bytes = h.save()

    return { hash, count, guideCount, bytes }
  })

  expect(beforeReload.count).toBe(3)
  expect(beforeReload.guideCount).toBe(1)

  // Reload through the app's real Open path.
  const afterReload = await page.evaluate((bytes) => {
    const h = window.__hew_test!
    h.load(bytes)
    return {
      hash: h.getStateHash(),
      count: h.getObjectCount(),
      guideCount: h.getGuideIds().length,
    }
  }, beforeReload.bytes)

  expect(afterReload.count).toBe(3)
  expect(afterReload.guideCount).toBe(1)
  expect(afterReload.hash).toBe(beforeReload.hash)
})

// ---------------------------------------------------------------------------
// 2. Autosave → recovery round-trip
// ---------------------------------------------------------------------------

test('autosave recovery: seeded IndexedDB snapshot prompts recovery dialog', async ({ page }) => {
  await page.goto('/')
  await waitForHarness(page)

  // Build a document and capture its bytes.
  const { bytes, hash: savedHash } = await page.evaluate(() => {
    const h = window.__hew_test!
    h.drawBox([0, 0, 0], [2, 2, 0], 3)
    return { bytes: h.save(), hash: h.getStateHash() }
  })

  expect(bytes.length).toBeGreaterThan(0)

  // Seed the web recovery store (IndexedDB) directly, simulating what the
  // 12-second autosave interval would write.
  await page.evaluate(async (savedBytes) => {
    const DB_NAME = 'hew-recovery'
    const STORE_NAME = 'snapshot'
    const KEY = 'current'

    const db: IDBDatabase = await new Promise((resolve, reject) => {
      const req = indexedDB.open(DB_NAME, 1)
      req.onupgradeneeded = () => {
        if (!req.result.objectStoreNames.contains(STORE_NAME)) {
          req.result.createObjectStore(STORE_NAME)
        }
      }
      req.onsuccess = () => resolve(req.result)
      req.onerror = () => reject(req.error)
    })

    const meta = {
      version: 1,
      savedAt: Date.now(),
      name: 'Test Recovery',
      path: null,
    }
    const record = { bytes: new Uint8Array(savedBytes), meta }

    await new Promise<void>((resolve, reject) => {
      const tx = db.transaction(STORE_NAME, 'readwrite')
      tx.objectStore(STORE_NAME).put(record, KEY)
      tx.oncomplete = () => resolve()
      tx.onerror = () => reject(tx.error)
    })
    db.close()
  }, bytes)

  // Reload the page. The startup recovery check should detect the seeded
  // snapshot and prompt the recovery dialog.
  await page.reload()

  // Wait for the recovery dialog to appear (it shows a "Recover" button).
  const recoverButton = page.locator('button', { hasText: 'Recover' })
  await expect(recoverButton).toBeVisible({ timeout: 15_000 })

  // Click "Recover" to load the saved snapshot.
  await recoverButton.click()

  // The harness must become ready after the recovery load.
  await waitForHarness(page)
  await settleFrame(page)

  const afterRecovery = await page.evaluate(() => {
    const h = window.__hew_test!
    return { count: h.getObjectCount(), hash: h.getStateHash() }
  })

  // The recovered document must have the same state as what was seeded.
  expect(afterRecovery.count).toBe(1)
  expect(afterRecovery.hash).toBe(savedHash)
})

test('autosave recovery: clicking Discard closes dialog without loading data', async ({ page }) => {
  await page.goto('/')
  await waitForHarness(page)

  // Seed a recovery snapshot with 1 box.
  const { bytes } = await page.evaluate(() => {
    const h = window.__hew_test!
    h.drawBox([0, 0, 0], [1, 1, 0], 1)
    return { bytes: h.save() }
  })

  await page.evaluate(async (savedBytes) => {
    const db: IDBDatabase = await new Promise((resolve, reject) => {
      const req = indexedDB.open('hew-recovery', 1)
      req.onupgradeneeded = () => {
        if (!req.result.objectStoreNames.contains('snapshot')) {
          req.result.createObjectStore('snapshot')
        }
      }
      req.onsuccess = () => resolve(req.result)
      req.onerror = () => reject(req.error)
    })
    const meta = { version: 1, savedAt: Date.now(), name: 'Discard Test', path: null }
    await new Promise<void>((resolve, reject) => {
      const tx = db.transaction('snapshot', 'readwrite')
      tx.objectStore('snapshot').put({ bytes: new Uint8Array(savedBytes), meta }, 'current')
      tx.oncomplete = () => resolve()
      tx.onerror = () => reject(tx.error)
    })
    db.close()
  }, bytes)

  await page.reload()

  // Wait for the recovery dialog.
  const discardButton = page.locator('button', { hasText: 'Discard' })
  await expect(discardButton).toBeVisible({ timeout: 15_000 })

  // Click Discard — the dialog should close AND the document must stay empty
  // (the snapshot was NOT loaded, only the snapshot data had the box).
  await discardButton.click()
  await expect(discardButton).not.toBeVisible({ timeout: 5_000 })

  await waitForHarness(page)

  // The document should be empty — Discard does not load the recovery snapshot.
  const count = await page.evaluate(() => window.__hew_test!.getObjectCount())
  expect(count).toBe(0)

  // Confirm the IndexedDB snapshot was cleared (the clear is async but completes
  // before any observable state can be re-queried from inside page.evaluate).
  const snapshotAfterDiscard = await page.evaluate(async () => {
    const db: IDBDatabase = await new Promise((resolve, reject) => {
      const req = indexedDB.open('hew-recovery', 1)
      req.onsuccess = () => resolve(req.result)
      req.onerror = () => reject(req.error)
    })
    const tx = db.transaction('snapshot', 'readonly')
    const record = await new Promise<unknown>((resolve) => {
      const req = tx.objectStore('snapshot').get('current')
      req.onsuccess = () => resolve(req.result)
      req.onerror = () => resolve(undefined)
    })
    db.close()
    return record === undefined || record === null
  })
  // The snapshot store should be empty after Discard.
  expect(snapshotAfterDiscard).toBe(true)
})

// ---------------------------------------------------------------------------
// 3. Docked tray presence & persistence
//
// (Rewritten : the original two specs asserted the pre-
// FloatingPanel architecture — data-testid="floating-panel" + drag-positioned
// inline left/top — which deleted when the docked right tray replaced
// floating panels.  removed the FloatingPanel unit suite but missed
// these; stale ever since. They now assert the tray equivalent, plus the
//  persistence that made the tray layout survive a reload.)
// ---------------------------------------------------------------------------

test('panels: docked tray sections are mounted with the default layout', async ({ page }) => {
  await page.goto('/')
  await waitForHarness(page)

  // All four tray section headers exist; Object Info + Outliner start
  // expanded, Materials + Tags start collapsed ( defaults).
  for (const [title, expanded] of [
    ['Object Info', 'true'],
    ['Outliner', 'true'],
    ['Materials', 'false'],
    ['Tags', 'false'],
  ] as const) {
    const header = page.getByRole('button', { name: title })
    await expect(header).toBeVisible({ timeout: 10_000 })
    await expect(header).toHaveAttribute('aria-expanded', expanded)
  }
})

test('panels: tray section expanded state persists across reload', async ({ page }) => {
  await page.goto('/')
  await waitForHarness(page)

  // Flip a collapsed-by-default section open…
  const materials = page.getByRole('button', { name: 'Materials' })
  await expect(materials).toHaveAttribute('aria-expanded', 'false')
  await materials.click()
  await expect(materials).toHaveAttribute('aria-expanded', 'true')

  // …and it survives a full reload (same browser context → same localStorage).
  await page.reload()
  await waitForHarness(page)
  await expect(page.getByRole('button', { name: 'Materials' })).toHaveAttribute(
    'aria-expanded',
    'true',
  )
})

// ---------------------------------------------------------------------------
// 4. Unit format persistence (localStorage)
// ---------------------------------------------------------------------------

test('units: length format persists across a page reload', async ({ page }) => {
  await page.goto('/')
  await waitForHarness(page)

  // Switch to centimeters and verify it's active.
  await page.evaluate(() => window.__hew_test!.setLengthUnit('cm'))
  const unitBefore = await page.evaluate(() => window.__hew_test!.getLengthUnit())
  expect(unitBefore).toBe('cm')

  // Reload the page — the format should persist via localStorage.
  await page.reload()
  await waitForHarness(page)

  const unitAfter = await page.evaluate(() => window.__hew_test!.getLengthUnit())
  expect(unitAfter).toBe('cm')

  // Verify the formatter uses the persisted unit.
  const formatted = await page.evaluate(() => window.__hew_test!.formatLength(1.0))
  expect(formatted).toBe('100 cm')

  // Clean up: reset to meters so other tests see the default.
  await page.evaluate(() => window.__hew_test!.setLengthUnit('m'))
})

// ---------------------------------------------------------------------------
// 5. Undo multi-step + save: saved document reflects undo state
// ---------------------------------------------------------------------------

test('undo then save: saved bytes encode the undone state, not the original', async ({ page }) => {
  await page.goto('/')
  await waitForHarness(page)

  const result = await page.evaluate(() => {
    const h = window.__hew_test!

    // Draw three boxes.
    const a = h.drawBox([0, 0, 0], [1, 1, 0], 1)
    h.drawBox([2, 0, 0], [3, 1, 0], 1)
    h.drawBox([4, 0, 0], [5, 1, 0], 1)
    const hash3 = h.getStateHash()

    // Undo two boxes → back to one. A drawBox is TWO undo steps since sketch
    // gestures became undoable ("sketches are first-class interactable"):
    // the extrusion, then the drawing gesture (which also removes the sketch
    // it created).
    h.undo()
    h.undo()
    h.undo()
    h.undo()
    const hash1 = h.getStateHash()
    const count1 = h.getObjectCount()

    // Save the undone state.
    const bytes = h.save()

    // Load the saved bytes into a fresh state — it should match the undone
    // state (1 box), not the 3-box state.
    h.load(bytes)
    const hashLoaded = h.getStateHash()
    const countLoaded = h.getObjectCount()

    return { hash3, hash1, count1, hashLoaded, countLoaded, firstBox: a }
  })

  // After undoing two boxes, we should have 1.
  expect(result.count1).toBe(1)
  // The saved state differs from the 3-box state.
  expect(result.hash1).not.toBe(result.hash3)
  // The loaded state matches exactly what was saved (the undone state).
  expect(result.hashLoaded).toBe(result.hash1)
  expect(result.countLoaded).toBe(1)
})

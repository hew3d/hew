import { test, expect, type Page } from '@playwright/test'
import { outlinerEditingChips } from './helpers/outliner'
import { isMac } from './helpers/platform'

/**
 * Lane C — dirty-state E2E (docs/design/v1.1-cycle.md): the derived
 * `dirty` flag (`!scene.at_saved_mark() || nonUndoableDirty`, App.tsx's
 * `handleDocumentChanged`) end to end, through the real reconcile path
 * (window.__hew_test's `act`-backed methods) rather than a unit test's
 * hand-called `afterMutation`.
 *
 * The window/document TITLE is the signal under test: `deriveTitle`
 * (documentSession.ts) prepends "• " while dirty, and App.tsx's effect
 * pushes it straight to `document.title` — the web build's own tab title,
 * with no native title bar to intercept it. `toHaveTitle` polls, so no
 * manual waits are needed for the title effect to commit after a harness
 * mutation.
 */

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

async function waitForHarness(page: Page): Promise<void> {
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, {
    timeout: 20_000,
  })
}

test('a draw dirties the title; undo back to the start cleans it', async ({ page }) => {
  await page.goto('/')
  await waitForHarness(page)

  await expect(page).toHaveTitle(/^Untitled — Hew$/)

  await page.evaluate(() => {
    window.__hew_test!.drawBox([0, 0, 0], [1, 1, 0], 1)
  })
  await expect(page).toHaveTitle(/^• Untitled — Hew$/)

  // Undo the extrude, then the draw gesture — two undo entries for one box
  // (kernel: `saved_mark_specs.rs`'s `build_box`), back to the document's
  // ORIGINAL depth-0 state, which is clean by kernel default — no save
  // needed for this half of the round trip.
  await page.evaluate(() => window.__hew_test!.undo())
  await page.evaluate(() => window.__hew_test!.undo())
  await expect(page).toHaveTitle(/^Untitled — Hew$/)
})

test('save cleans the title; undo dirties it again; redo cleans it back up', async ({ page }) => {
  await page.goto('/')
  await waitForHarness(page)

  await page.evaluate(() => {
    window.__hew_test!.drawBox([0, 0, 0], [1, 1, 0], 1)
  })
  await expect(page).toHaveTitle(/^• Untitled — Hew$/)

  // saveWithCameraState mirrors App's real save flow: it marks the
  // document's CURRENT undo depth as the saved mark and reconciles, so the
  // title should clean up exactly as it would after a real File ▸ Save.
  await page.evaluate(() => window.__hew_test!.saveWithCameraState())
  await expect(page).toHaveTitle(/^Untitled — Hew$/)

  // Undo below the saved depth: dirty again (the "saved" file has the box,
  // the live document doesn't).
  await page.evaluate(() => window.__hew_test!.undo())
  await expect(page).toHaveTitle(/^• Untitled — Hew$/)

  // Redo back to the saved depth: clean again — this is the headline Lane C
  // behavior (undo-to-clean), not just "save clears dirty".
  await page.evaluate(() => window.__hew_test!.redo())
  await expect(page).toHaveTitle(/^Untitled — Hew$/)
})

// Session bookkeeping (Lane C follow-up, maintainer playtest): entering or
// leaving a group/component edit session pushes a real, undoable history
// entry, but the kernel's `content_depth` ignores it — it must never dirty
// the document or count as a "change" in the Changes panel. Enters via a
// real Outliner double-click on the group's own row (`DocumentTree.tsx`'s
// `onEnterContext`, the same entry point group-session.spec.ts's viewport
// double-click resolves to) — no camera/pixel math needed since the row is
// a plain DOM element, not a viewport pick.
test('entering and leaving a group edit session never dirties the title or the Changes panel', async ({ page }) => {
  await page.goto('/')
  await waitForHarness(page)

  const setup = await page.evaluate(() => {
    const h = window.__hew_test!
    const box = h.drawBox([0, 0, 0], [2, 1, 0], 1)
    const group = h.groupNodes([{ kind: 'object', id: box }])
    h.saveWithCameraState()
    return { box, group }
  })
  await expect(page).toHaveTitle(/^Untitled — Hew$/)

  // Expand the Changes tray section so its footer text is on screen.
  await page.getByRole('button', { name: 'Changes' }).click()
  await expect(page.getByText('No changes since last save.')).toBeVisible()

  // Double-click the group's own Outliner row — opens its edit session.
  const groupRow = page.getByText('Group 1').first()
  await expect(groupRow).toBeVisible()
  await groupRow.dblclick()
  await page.waitForTimeout(200)
  await expect(outlinerEditingChips(page)).toHaveCount(1)

  // Undoable bookkeeping, but not a change: no dirty dot, no reported changes.
  await expect(page).toHaveTitle(/^Untitled — Hew$/)
  await expect(page.getByText('No changes since last save.')).toBeVisible()

  // A real edit inside the session dirties normally...
  await page.evaluate((id) => window.__hew_test!.addNodeTag('object', id, ['tag']), setup.box)
  await expect(page).toHaveTitle(/^• Untitled — Hew$/)
  await expect(page.getByText('1 change since last save.')).toBeVisible()

  // ...undoing it returns to clean, still inside the session.
  await page.evaluate(() => window.__hew_test!.undo())
  await expect(page).toHaveTitle(/^Untitled — Hew$/)
  await expect(page.getByText('No changes since last save.')).toBeVisible()

  // Exiting the session (Escape) is itself more bookkeeping — still clean.
  await page.keyboard.press('Escape')
  await page.waitForTimeout(200)
  await expect(outlinerEditingChips(page)).toHaveCount(0)
  await expect(page).toHaveTitle(/^Untitled — Hew$/)
  await expect(page.getByText('No changes since last save.')).toBeVisible()
})

test('a further edit past the saved mark dirties again, and stays dirty across an unrelated undo/redo pair', async ({ page }) => {
  await page.goto('/')
  await waitForHarness(page)

  await page.evaluate(() => {
    window.__hew_test!.drawBox([0, 0, 0], [1, 1, 0], 1)
  })
  await page.evaluate(() => window.__hew_test!.saveWithCameraState())
  await expect(page).toHaveTitle(/^Untitled — Hew$/)

  // A second box, drawn AFTER the save: dirty, and — because this is a
  // brand-new action — it discards any redo branch (none here) and moves
  // the undo depth past the saved mark.
  await page.evaluate(() => {
    window.__hew_test!.drawBox([2, 0, 0], [3, 1, 0], 1)
  })
  await expect(page).toHaveTitle(/^• Untitled — Hew$/)

  await page.evaluate(() => window.__hew_test!.undo())
  await page.evaluate(() => window.__hew_test!.undo())
  await expect(page).toHaveTitle(/^Untitled — Hew$/)

  await page.evaluate(() => window.__hew_test!.redo())
  await page.evaluate(() => window.__hew_test!.redo())
  await expect(page).toHaveTitle(/^• Untitled — Hew$/)
})

// Registry writes (palette materials, tags, Scenes) push no kernel undo
// entry, so the kernel's saved mark cannot see them: every path that makes
// one has to dirty the document itself, or closing it loses the change
// without a prompt.

async function openSavedDocument(page: Page): Promise<void> {
  await page.goto('/')
  await waitForHarness(page)
  await page.evaluate(() => {
    window.__hew_test!.drawBox([0, 0, 0], [1, 1, 0], 1)
    window.__hew_test!.saveWithCameraState()
  })
  await expect(page).toHaveTitle(/^Untitled — Hew$/)
}

async function revealMaterialsTray(page: Page): Promise<void> {
  const defaultSwatch = page.getByTitle('Default (unpainted)')
  if (!(await defaultSwatch.isVisible().catch(() => false))) {
    await page.getByText('Materials', { exact: true }).first().click()
  }
  await expect(defaultSwatch).toBeVisible()
}

async function addColorFromPalette(page: Page, name: string): Promise<void> {
  await revealMaterialsTray(page)
  await page.getByRole('button', { name: 'Add color', exact: true }).click()
  await page.getByPlaceholder('Name…').first().fill(name)
  await page.locator('input[type="color"]').first().fill('#aa3366')
  await page.getByRole('button', { name: '+ Add color' }).click()
}

test('the palette\'s "Add color" dirties a saved document', async ({ page }) => {
  await openSavedDocument(page)
  await addColorFromPalette(page, 'Plum')
  await expect(page).toHaveTitle(/^• Untitled — Hew$/)
})

test('the palette\'s "Add texture" dirties a saved document', async ({ page }) => {
  await openSavedDocument(page)
  await revealMaterialsTray(page)
  await page.getByRole('button', { name: 'Add texture', exact: true }).click()
  await page.locator('input[type="file"][accept*="image/png"]').setInputFiles({
    name: 'tile.png',
    mimeType: 'image/png',
    buffer: Buffer.from(
      'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhWMjWwAAAABJRU5ErkJggg==',
      'base64',
    ),
  })
  await page.getByRole('button', { name: '+ Add texture' }).click()
  await expect(page).toHaveTitle(/^• Untitled — Hew$/)
})

for (const [method, params] of [
  ['hew.material.create', { name: 'Oak', color: [180, 140, 90] }],
  ['hew.tag.create', { path: ['Structure'] }],
  ['hew.tag.set_visible', { path: ['Structure'], visible: false }],
  ['hew.scenes.add', { name: 'Front', camera: null }],
] as const) {
  test(`a live ${method} dirties a saved document`, async ({ page }) => {
    await openSavedDocument(page)
    const reply = await page.evaluate(
      ([m, p]) => window.__hew_test!.apiDispatch({ method: m as string, params: p as Record<string, unknown> }),
      [method, method === 'hew.scenes.add' ? { name: 'Front', properties: { camera: false } } : params] as const,
    )
    expect(reply).not.toHaveProperty('error')
    await expect(page).toHaveTitle(/^• Untitled — Hew$/)
  })
}

test('adding a Library material from the command palette dirties the document, and only when it adds one', async ({ page }) => {
  await page.goto('/')
  await waitForHarness(page)
  await addColorFromPalette(page, 'Heather')
  await page.getByRole('button', { name: 'Save to Library' }).click()
  await expect(page.getByText('Saved "Heather" to library.')).toBeVisible()

  const addFromCommandPalette = async (): Promise<void> => {
    await page.keyboard.press(isMac ? 'Meta+/' : 'Control+k')
    const commands = page.getByRole('dialog', { name: /command palette/i })
    await commands.getByPlaceholder(/search/i).fill('Heather')
    await expect(commands.getByText('Add "Heather" to palette')).toBeVisible()
    await page.keyboard.press('Enter')
    await expect(page.getByText('Added "Heather" to palette.').last()).toBeVisible()
  }

  // A fresh document, which does not hold the material yet.
  await openSavedDocument(page)
  await addFromCommandPalette()
  await expect(page).toHaveTitle(/^• Untitled — Hew$/)

  // Already in the palette: nothing is added, so nothing becomes unsaved.
  await openSavedDocument(page)
  await page.evaluate(() => {
    window.__hew_test!.addMaterial('Heather', 0xaa, 0x33, 0x66, 255)
    window.__hew_test!.saveWithCameraState()
  })
  await expect(page).toHaveTitle(/^Untitled — Hew$/)
  await addFromCommandPalette()
  await expect(page).toHaveTitle(/^Untitled — Hew$/)
})

test('a live Library save that links a part, and a transaction that adds a material, stay unsaved', async ({ page }) => {
  await openSavedDocument(page)
  // The transaction's undo entry takes the shape back, not the material.
  const reply = await page.evaluate(() =>
    window.__hew_test!.apiDispatch({
      method: 'hew.doc.transact',
      params: {
        commands: [
          { method: 'hew.sketch.draw_rect', params: { plane: { ground: true }, corner_a: [3, 0, 0], corner_b: [4, 1, 0] } },
          { method: 'hew.material.create', params: { name: 'Oak', color: [180, 140, 90] } },
        ],
      },
    }),
  )
  expect(reply).not.toHaveProperty('error')
  await page.evaluate(() => window.__hew_test!.undo())
  await expect(page).toHaveTitle(/^• Untitled — Hew$/)

  await openSavedDocument(page)
  const saved = await page.evaluate(() => {
    const h = window.__hew_test!
    const scene = h.apiDispatch({ method: 'hew.query.scene' })
    const id = JSON.stringify(scene).match(/obj_[0-9a-z]+/)![0]
    return h.apiDispatch({ method: 'hew.library.save', params: { name: 'Box', selection: [id] } })
  })
  expect(saved).not.toHaveProperty('error')
  await expect(page).toHaveTitle(/^• Untitled — Hew$/)
})

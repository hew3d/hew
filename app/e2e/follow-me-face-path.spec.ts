import { test, expect, type Page } from '@playwright/test'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'

/**
 * Follow Me — SOLID-FACE path, grounded in the maintainer's follow-me-2.hew.
 *
 * The file is a thin tabletop solid (0.1 x 0.2 x 0.015 m) with a circle and a
 * rectangle profile standing perpendicular to its top rim, plus a THIRD
 * profile — a stray ground rectangle, lying FLAT and parallel to the top
 * face — the "crown molding around a tabletop" scenario (the follow-me
 * design §2, the guide's "Running around a face"). The playtest report was
 * that the SOLID path was flaky where the sketch-edge path worked; the root
 * cause was never the kernel sweep — it produces a correct watertight
 * molding around the tabletop's TOP face for every one of the three profiles
 * (pinned by the kernel spec `follow_me_molding_around_the_maintainer_tabletop`,
 * and — since auto-orientation, design §2c — the flat one too, folded upright
 * before it sweeps rather than refused) — but WHICH face becomes the path:
 * only the flat top (or bottom) face carries an upright profile, and a pick
 * ray through the standing profile lands on a side face the kernel correctly
 * refuses.
 *
 * This drives the SOLID-FACE commit through the real Open path + wasm binding,
 * grounded in the actual file: load it, then sweep all three profiles around
 * the tabletop's top face and assert a watertight molding commits with the
 * tabletop left untouched — deterministic (no synthetic pixel picking, which
 * is what flakes on this tiny scene; the pick side is covered by the
 * kernel/inference layers and the real-pointer follow-me.spec.ts).
 */

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

async function boot(page: Page): Promise<void> {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
  const bytes = Array.from(readFileSync(resolve(process.cwd(), 'e2e/fixtures/follow-me-2.hew')))
  const ok = await page.evaluate((b) => window.__hew_test!.load(b), bytes)
  expect(ok).not.toBe(false)
  await page.waitForFunction(() => window.__hew_test!.getObjectCount() === 1)
}

test('Follow Me: all three profiles sweep a watertight molding around the tabletop top face (maintainer file)', async ({
  page,
}) => {
  await boot(page)

  const result = await page.evaluate(() => {
    const h = window.__hew_test!
    // The tabletop's top face, via a straight-down pick on its surface.
    const top = h.pickFace([0.03, 0.05, 1.0], [0, 0, -1])
    if (top === null) return { error: 'no top face' as const }

    // Each of the file's three sketches (rectangle profile, circle profile,
    // a stray ground rectangle) has exactly one region, id 0 → the same FFI
    // handle. Sweep each around the top face on a fresh document (undo
    // between), recording which commit and which refuse.
    const REGION = '4294967297'
    const outcomes = h.getSketchIds().map((sketch) => {
      const before = h.getObjectCount()
      let solidId: string | null = null
      let refused = false
      try {
        solidId = h.followMeAroundFace(sketch, REGION, top.object, top.face)
      } catch {
        refused = true
      }
      const committed = h.getObjectCount() === before + 1
      const out = {
        committed,
        refused,
        solid: solidId ? h.isObjectSolid(solidId) : null,
        // The tabletop is a separate, untouched object after a commit.
        tabletopLives: h.getObjectIds().includes(top.object),
      }
      if (committed) h.undo()
      return out
    })
    return { outcomes }
  })

  expect('error' in result).toBe(false)
  if ('error' in result) return

  // All three profiles commit a watertight molding now — the two upright
  // ones exactly as before, and the flat ground rectangle auto-oriented
  // (design §2c) instead of refused.
  const commits = result.outcomes.filter((o) => o.committed)
  const refusals = result.outcomes.filter((o) => o.refused)
  expect(commits.length).toBe(3)
  expect(refusals.length).toBe(0)
  for (const o of commits) {
    expect(o.solid).toBe(true)
    expect(o.tabletopLives).toBe(true)
  }
})

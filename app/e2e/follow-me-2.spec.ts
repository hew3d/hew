import { test, expect, type Page } from '@playwright/test'

/**
 * Follow Me II — the batch-3 kernel capabilities, driven through the real
 * wasm bindings (harness-API style, like `follow-me-face-path.spec.ts`):
 * deterministic scene setup, then the ACTUAL follow-me call the tool would
 * make, asserting the real kernel's response. Auto-orientation and instanced
 * face PATHS already have real-POINTER coverage (`follow-me-start-cue.spec.ts`,
 * `follow-me-face-pointer.spec.ts`); this file covers the two capabilities
 * that need more elaborate scenes to set up meaningfully — a solid-face
 * PROFILE (design §3a) and the merge gesture (design §3b) — plus the
 * group-context birth (design §2f) and, below, a batch-4 playtest fix: K2's
 * signed partial-sweep stop. K2's actual DIRECTION MAPPING (which way a real
 * drag moved, and the sign it produces) is a pure-TS concern with thorough
 * coverage in `FollowMeTool.test.ts` (including a seam-crossing case); a
 * real-POINTER drag on this file's kind of scene is `follow-me-partial-
 * sweep.spec.ts`'s idiom, not this file's. What belongs here — and what a
 * unit test cannot prove — is that a NEGATIVE `stop_len` passed through
 * `followMeAlongEdges` (the literal wasm call `FollowMeTool._invokeFollowMe`
 * makes) produces real, watertight geometry on the genuinely OTHER side of
 * the seam via the real kernel, cross-checked the same way every other test
 * in this file cross-checks its own capability.
 */

declare global {
  interface Window {
    __hew_test?: import('../src/test/harness').HewTestHarness
  }
}

async function boot(page: Page): Promise<void> {
  await page.goto('/')
  await page.waitForFunction(() => window.__hew_test?.isReady() === true, null, { timeout: 15_000 })
}

test('Follow Me: a solid FACE as the profile sweeps a NEW object, source untouched (design §3a)', async ({
  page,
}) => {
  await boot(page)

  const result = await page.evaluate(() => {
    const h = window.__hew_test!
    // The profile SOURCE: a small cube well away from the path, so the sweep
    // and the source can never overlap or be confused for one another.
    const cube = h.drawBox([5, 5, 0], [5.2, 5.2, 0], 0.2)
    const top = h.pickFace([5.1, 5.1, 0.2], [0, 0, 1])
    if (top === null) return { error: 'no cube top face' as const }

    // The PATH: a straight ground line far from the cube.
    const { sketch: pathSketch } = h.drawLineChain([
      [0, 0, 0],
      [1, 0, 0],
    ])
    const edges = h.getSketchEdgeIds(pathSketch)

    const before = h.getObjectCount()
    const swept = h.followMeFaceAlongEdges(top.object, top.face, pathSketch, edges)
    return {
      before,
      after: h.getObjectCount(),
      sweptSolid: h.isObjectSolid(swept),
      cube,
      cubeSolid: h.isObjectSolid(cube),
      cubeStillPresent: h.getObjectIds().includes(cube),
    }
  })

  expect('error' in result).toBe(false)
  if ('error' in result) return
  // The sweep births a SEPARATE object — the cube (an unrelated solid) is
  // never consumed or merged; only the FaceLoop-path-and-same-object case
  // (design §3b) merges, and this path is not even a FaceLoop.
  expect(result.after).toBe(result.before + 1)
  expect(result.sweptSolid).toBe(true)
  expect(result.cubeStillPresent).toBe(true)
  expect(result.cubeSolid).toBe(true)
})

test('Follow Me: the merged commit fuses a sketch-region profile with the path solid, one object after (design §3b)', async ({
  page,
}) => {
  await boot(page)

  const result = await page.evaluate(() => {
    const h = window.__hew_test!
    // The BASE solid the molding merges into.
    const base = h.drawBox([0, 0, 0], [1, 1, 0], 0.2)
    const top = h.pickFace([0.5, 0.5, 0.2], [0, 0, 1])
    if (top === null) return { error: 'no base top face' as const }

    // A small profile sketch region near the base's rim — auto-orientation
    // (design §2c) stands it upright regardless of how it was drawn, so it
    // does not need to be pre-rotated.
    const prof = h.drawRectangle([0.9, 0.4, 0], [1.1, 0.6, 0])

    const before = h.getObjectCount()
    const merged = h.followMeMergedAroundFace(prof.sketch, prof.region, top.object, top.face)
    return {
      before,
      after: h.getObjectCount(),
      base,
      mergedIsNewHandle: merged !== base,
      baseStillListed: h.getObjectIds().includes(base),
      mergedSolid: h.isObjectSolid(merged),
    }
  })

  expect('error' in result).toBe(false)
  if ('error' in result) return
  // ONE undo step, one object after: the base is consumed into the merge
  // (its old handle is hidden, not left standing beside the result) — the
  // molding never births as a second, separate object the way a plain
  // (non-merged) face-loop sweep would.
  expect(result.before).toBe(1)
  expect(result.after).toBe(1)
  expect(result.mergedIsNewHandle).toBe(true)
  expect(result.baseStillListed).toBe(false)
  expect(result.mergedSolid).toBe(true)
})

test('Follow Me: a sweep committed while editing a group births inside that group (design §2f)', async ({
  page,
}) => {
  await boot(page)

  const result = await page.evaluate(() => {
    const h = window.__hew_test!
    // An unrelated object, grouped by itself — the group Follow Me will be
    // "editing" when the sweep commits.
    const anchor = h.drawBox([9, 9, 0], [9.2, 9.2, 0], 0.1)
    const group = h.groupNodes([{ kind: 'object', id: anchor }])

    const { sketch: pathSketch } = h.drawLineChain([
      [0, 0, 0],
      [1, 0, 0],
    ])
    const edges = h.getSketchEdgeIds(pathSketch)
    const prof = h.drawRectangle([0.4, -0.1, 0], [0.6, 0.1, 0])
    h.rotateSketch(prof.sketch, 90, [1, 0, 0], [0.5, 0, 0])

    const swept = h.followMeAlongEdges(prof.sketch, prof.region, pathSketch, edges, group)
    return {
      members: h.getGroupMembers(group),
      swept,
      sweptSolid: h.isObjectSolid(swept),
    }
  })

  // The new solid is a member of the group — not a top-level sibling.
  expect(result.members.map((m) => m.id)).toContain(result.swept)
  expect(result.sweptSolid).toBe(true)
})

test('Follow Me: a NEGATIVE partial-sweep stop (K2) sweeps the OTHER way from the seam, real geometry both ways', async ({
  page,
}) => {
  await boot(page)

  const result = await page.evaluate(() => {
    const h = window.__hew_test!
    // A closed 4×4 ground loop, (0,0,0) → (4,0,0) → (4,4,0) → (0,4,0) →
    // back to (0,0,0) — the last point re-closes it into one island.
    const path = h.drawLineChain([
      [0, 0, 0],
      [4, 0, 0],
      [4, 4, 0],
      [0, 4, 0],
      [0, 0, 0],
    ])
    const edges = h.getSketchEdgeIds(path.sketch)

    // Two IDENTICAL fresh profiles, each a small square standing exactly at
    // (2, 0, 0) — the midpoint of the bottom edge, a mid-EDGE (Split)
    // anchor, not a corner (corner seams refuse a reversed stop by design —
    // see FollowMeTool's `_refusalMessage` doc). Drawn on the ground
    // centered on x = 2, then stood up 90° about the Y axis through that
    // same point, landing the profile in the x = 2 plane — perpendicular to
    // the path's bottom-edge tangent there, exactly the Rotate-tool move a
    // user makes.
    const makeProfile = (): { sketch: string; region: string } => {
      const p = h.drawRectangle([1.9, -0.1, 0], [2.1, 0.1, 0])
      h.rotateSketch(p.sketch, 90, [0, 1, 0], [2, 0, 0])
      return p
    }

    const fwdProfile = makeProfile()
    const fwd = h.followMeAlongEdges(
      fwdProfile.sketch, fwdProfile.region, path.sketch, edges, undefined, 3,
    )
    const revProfile = makeProfile()
    const rev = h.followMeAlongEdges(
      revProfile.sketch, revProfile.region, path.sketch, edges, undefined, -3,
    )

    return {
      fwdSolid: h.isObjectSolid(fwd),
      revSolid: h.isObjectSolid(rev),
      fwdBounds: h.getObjectBounds(fwd),
      revBounds: h.getObjectBounds(rev),
      count: h.getObjectCount(),
    }
  })

  // Both partial sweeps are real, watertight, separate objects.
  expect(result.count).toBe(2)
  expect(result.fwdSolid).toBe(true)
  expect(result.revSolid).toBe(true)

  // FORWARD (stop = 3) leaves the seam at x = 2 toward (4,0,0) — 2 m to that
  // corner, then 1 m more up the RIGHT edge (x = 4). REVERSE (stop = -3)
  // leaves the same seam the other way, toward (0,0,0) — 2 m to THAT
  // corner, then 1 m more up the LEFT edge (x = 0). The two solids'
  // bounding boxes land on genuinely opposite sides of the seam.
  const [fwdMinX, , , fwdMaxX] = result.fwdBounds
  const [revMinX, , , revMaxX] = result.revBounds
  expect(fwdMaxX).toBeGreaterThan(3.5) // reaches well past x = 2, toward x = 4
  expect(fwdMinX).toBeGreaterThan(1.5) // never crosses back past the seam
  expect(revMinX).toBeLessThan(0.5) // reaches well past x = 2 the other way, toward x = 0
  expect(revMaxX).toBeLessThan(2.5) // never crosses back past the seam
})

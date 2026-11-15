/**
 * Marks a unit test (or, in a `beforeEach`, a whole `describe`) as one that
 * needs a Preview feature. Its subject cannot exist in a production build, so
 * the production run (`HEW_PREVIEW=0`) skips it, with a reason that starts
 * `preview-wall:`; the preview run runs it. The wall's own side of the same
 * feature is asserted in `src/preview/wall.test.ts`.
 *
 *   it('the dock offers Fillet', (ctx) => { requiresPreview(ctx, 'fillet'); ... })
 *   describe('SmoothSection', () => { beforeEach((ctx) => requiresPreview(ctx, 'smooth')) ... })
 *
 * Ids come from `preview-features.json`; an id it does not list throws (a
 * typo, or the feature graduated: take the mark out). Mark a test only when
 * what it asserts is gated; a test that fails in the production run for any
 * other reason is a defect to fix, not to mark. `app/scripts/check-unit-skips.mjs`
 * reads a JSON report of the run and fails any skip that is not marked here.
 */

import { readFileSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { previewBuild } from '../preview/build'

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '../../..')
const KNOWN = new Set(
  (JSON.parse(readFileSync(join(ROOT, 'preview-features.json'), 'utf8')) as { features: { id: string }[] }).features.map((f) => f.id),
)

/** What a vitest test context offers here. */
interface TestCtx {
  skip(note?: string): never
  task: { meta: object }
}

/** With no ids: the test is about the preview build itself (its own UI). */
export function requiresPreview(ctx: TestCtx, ...featureIds: string[]): void {
  const unknown = featureIds.filter((id) => !KNOWN.has(id))
  if (unknown.length > 0) {
    throw new Error(`requiresPreview: "${unknown.join('", "')}" is not in preview-features.json (a typo, or the feature graduated: drop the mark).`)
  }
  const reason = `preview-wall: needs ${featureIds.length === 0 ? 'a preview build' : `the Preview feature(s) ${featureIds.join(', ')}`}`
  const meta = ctx.task.meta as Record<string, unknown>
  meta.previewWall = reason
  if (!previewBuild) ctx.skip(reason)
}

/**
 * Which build the browser suite runs against, and the one way a spec says it
 * needs a Preview feature (docs/design/preview-wall-build.md section 4).
 *
 * The build under test is chosen in ONE place, here, from `HEW_PREVIEW`:
 *
 *   HEW_PREVIEW=1      the preview build: the Vite dev server with the flag on
 *                      (what `pnpm e2e` does with nothing set)
 *   HEW_PREVIEW=0      the production-shape build: `vite build` with the flag
 *                      off, served by `vite preview` (playwright.config.ts)
 *
 * Unset or empty means the dev server, whose default is on; any value but 1 or 0 is an error (src/preview/parseFlag.ts). The flag-off run is
 * therefore selected with `HEW_PREVIEW=0`, never by leaving it unset.
 *
 * A spec that drives a Preview feature starts with
 * `requiresPreview('fillet')`. It names a feature id from
 * `preview-features.json`; in the production-shape run the whole file skips,
 * saying which feature it needs. An id that is not in the registry throws, in
 * either run, so a typo or a graduated feature cannot leave a spec silently
 * skipped forever. (When a feature graduates, its entry is deleted, this call
 * throws, and the line comes out of the spec.)
 */

import { readFileSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { test } from '@playwright/test'
import { parsePreviewFlag } from '../../src/preview/parseFlag'

/** Every skip the wall makes starts with this, so a gate can tell them from
 *  any other skip (scripts/check-e2e-skips.mjs fails the others). */
export const WALL_SKIP = 'preview-wall: '

export interface RegistryFeature {
  id: string
  name: string
  tools: string[]
  actions: string[]
  dockChips: string[]
  apiCommands: string[]
  behaviors: string[]
  defaultOn?: boolean
  loader?: {
    roots: string[]
    panes: string[]
    shellCommands: string[]
    kernelExports: string[]
    cliCommands: string[]
    storage: string[]
    markers: string[]
  }
}

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '../../..')

/** The registry, read from the same file the app and the Rust side read. */
export function previewFeatures(): RegistryFeature[] {
  return (JSON.parse(readFileSync(join(ROOT, 'preview-features.json'), 'utf8')) as { features: RegistryFeature[] }).features
}

/** `'production'` for the flag-off, production-shape build; else `'preview'`. */
export const buildUnderTest: 'preview' | 'production' = parsePreviewFlag(process.env.HEW_PREVIEW, true) ? 'preview' : 'production'

export const isProductionShape = buildUnderTest === 'production'

/** Call at the top of a spec (or inside a `describe`): skip it in the
 *  production-shape run because it drives these Preview features. */
export function requiresPreview(...featureIds: string[]): void {
  const known = new Set(previewFeatures().map((f) => f.id))
  const unknown = featureIds.filter((id) => !known.has(id))
  if (unknown.length > 0) {
    throw new Error(
      `requiresPreview: "${unknown.join('", "')}" is not in preview-features.json. A typo, or the feature graduated: drop it from this spec.`,
    )
  }
  test.skip(isProductionShape, `${WALL_SKIP}needs the Preview feature(s) ${featureIds.join(', ')}; the production-shape build withholds them`)
}

/**
 * What the Preview wall leaves of a list of tool names (rail and dock labels
 * are tool names): all of them in the preview build, and in the
 * production-shape build every name the registry lists as a Preview tool is
 * dropped. For an assertion that holds in both builds once the gated names
 * are accounted for, so the spec need not be marked as a whole.
 */
export function offeredTools(names: string[]): string[] {
  if (!isProductionShape) return names
  const withheld = new Set(previewFeatures().flatMap((f) => f.tools))
  return names.filter((n) => !withheld.has(n))
}

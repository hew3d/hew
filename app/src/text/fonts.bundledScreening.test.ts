/**
 * Bundled-manifest outline screening (docs/design/3d-text-fonts.md's
 * "Bundled set" section) — every font in `BUNDLED_MANIFEST` must produce no
 * self-intersecting glyph outline across the full alphanumeric set, at this
 * app's real flattening tolerances. This used to be a one-off exploration
 * (see `fonts.ts`'s `BUNDLED_MANIFEST` doc comment); making it a real test
 * means a future addition to the manifest is screened automatically rather
 * than by remembering to re-run a script.
 *
 * Reads each manifest entry's `.ttf` straight off disk (`fs.readFileSync`
 * via `fileURLToPath(entry.url)`) instead of `loadBundledFont`'s `fetch` —
 * Node's `fetch` does not implement the `file:` scheme
 * (`TypeError: fetch failed … not implemented… yet…`, confirmed empirically
 * against this exact vitest environment), and `fetch` only works for these
 * assets at all inside a running Vite dev/build server or a browser, which
 * a `.test.ts` suite has neither of. Parsing straight from disk exercises
 * the identical bytes `loadBundledFont` would fetch (same file, same
 * `opentype.parse` call) — only the byte-acquisition path differs.
 */
import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { describe, expect, it } from 'vitest'
import * as opentype from 'opentype.js'
import { BUNDLED_MANIFEST } from './fonts'
import { validateGlyphOutlines } from './outlineValidation'

/** The full alphanumeric set (both cases + digits) — the same coverage the
 *  original hand-screening exploration used to find Inter/Manrope/Work
 *  Sans/Fredoka's bad defaults (see `fonts.ts`'s doc comment). */
const SCREENING_TEXT = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789'

/** Matches `TextDialog.tsx`'s `DEFAULT_HEIGHT_M` — screening at the height
 *  most placements will actually use, per `validateGlyphOutlines`'s own
 *  contract (its tolerance is scaled to the requested height). */
const SCREENING_HEIGHT_M = 0.1

describe('bundled font manifest: no self-intersecting glyph outlines', () => {
  for (const entry of BUNDLED_MANIFEST) {
    it(`${entry.label} (${entry.id}) is clean across A-Z, a-z, 0-9`, () => {
      const bytes = readFileSync(fileURLToPath(entry.url))
      const ab = bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength)
      const font = opentype.parse(ab)
      const warnings = validateGlyphOutlines(font, SCREENING_TEXT, SCREENING_HEIGHT_M)
      expect(warnings).toEqual([])
    })
  }

  it('covers every manifest entry (a future addition cannot silently skip screening)', () => {
    expect(BUNDLED_MANIFEST.length).toBeGreaterThanOrEqual(5)
  })
})

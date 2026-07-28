/**
 * fonts — font PARSING and session memory for 3D Text
 * (docs/design/3d-text.md, docs/design/3d-text-fonts.md).
 *
 * opentype.js parses real TTF/OTF outlines (an app-layer dependency — the
 * kernel never sees a font; only flattened sketch edges cross that
 * boundary). This module owns turning BYTES into a parsed, memoized
 * `LoadedFont` (`loadFontEntry`) and the session's "last picked font"
 * memory — `text/fontSources.ts` owns DISCOVERY (which fonts exist, and
 * how to fetch each one's bytes: bundled asset, desktop system
 * enumeration, `queryLocalFonts()`, or a user-picked file), so this module
 * never itself fetches, invokes Tauri, or reads a `File` — it only ever
 * sees raw bytes in and a parsed `Font` out, from whichever provider
 * fetched them. A loaded user/system font's parse is cached for the rest
 * of the SESSION only — the document stores geometry, never the font — so
 * re-opening the dialog later in the same session still offers it, but a
 * fresh launch does not.
 */

import * as opentype from 'opentype.js'
import type { FontFaceEntry } from './fontSources'

/** One PARSED, selectable font — a bundled family, a desktop/web system
 *  font, or a user-loaded file. Bold/italic are never SYNTHESIZED
 *  (docs/design/3d-text.md, docs/design/3d-text-fonts.md) — every entry
 *  offers exactly the face its own source file renders; a system font's
 *  picker row for "Bold" only exists because the OS genuinely has a Bold
 *  face installed (`fontSources.ts`'s desktop/web providers enumerate real
 *  faces, never synthesize a weight/style that isn't backed by its own
 *  outlines). */
export interface LoadedFont {
  /** Stable key for the session (bundled ids are fixed; system/user ids
   *  are derived from the enumerated face / generated per load — see
   *  `fontSources.ts`). */
  id: string
  /** Display label for the font picker: "Family" when the style is the
   *  face's own default ("Regular"), else "Family Style". */
  label: string
  /** Where this entry came from — bundled fonts are always available;
   *  "system" and "user" entries only exist for the current session
   *  (a system font because its provider was only queried this session;
   *  a user file because it was never persisted). */
  source: 'bundled' | 'system' | 'user'
  font: opentype.Font
}

/** The bundled OFL fonts (docs/design/3d-text.md, docs/design/3d-text-fonts.md):
 *  a clean geometric sans (Onest), a serif that reads well at print scale
 *  (Lora), a rounded display face that prints cleanly at small sizes
 *  (Varela Round), a monospace for label/dimension-style text (Space
 *  Mono), and a heavier slab/display face for signage (Alfa Slab One).
 *  Each ships as-is; opentype.js renders whatever instance the file itself
 *  stores — no bold/italic claim is made for any of these five (see
 *  `LoadedFont`'s doc comment).
 *
 * Font choice was screened, not just aesthetic: a font's variable-axis
 * DEFAULT instance is the RAW `glyf` outline with no `gvar` deltas applied
 * (this app never resolves `gvar` — it only reads the stored default), and
 * for several otherwise-popular current Google Fonts releases that stored
 * default is not a simple polygon for every glyph (observed: Inter's 'e'
 * and 'w', Manrope's 'a', Work Sans's 'e'/'a', Fredoka's 'M'/'6'/'9', and
 * more — see the exploration this screening came from). Onest and Lora are
 * variable fonts read at their default instance; Varela Round, Space Mono,
 * and Alfa Slab One are plain STATIC faces, chosen deliberately when the
 * bundled set grew (docs/design/3d-text-fonts.md) — a static file has no
 * stored-default problem at all, so it needs no `gvar` caveat to begin
 * with. Every entry here was verified clean (no self-intersecting glyph
 * outline) across the full alphanumeric set at this app's flattening
 * tolerances by `fonts.bundledScreening.test.ts`, which re-runs that check
 * over this exact manifest — a future addition here is screened by
 * construction, not by remembering to run a one-off script.
 * `text/flatten.ts`'s `classifyContourFill`/sliver-area filtering in
 * `tools/TextPlaceTool.ts` still guards a USER-loaded or SYSTEM font that
 * isn't as clean (surfaced as a warning — `text/outlineValidation.ts` — not
 * a block), but a bundled default should never need that guard to produce
 * a legible letter. License text for each lives alongside its .ttf in
 * `app/src/assets/fonts/<family>/OFL.txt`. */
export const BUNDLED_MANIFEST: ReadonlyArray<{ id: string; label: string; url: URL }> = [
  { id: 'bundled-onest', label: 'Onest', url: new URL('../assets/fonts/onest/Onest.ttf', import.meta.url) },
  { id: 'bundled-lora', label: 'Lora', url: new URL('../assets/fonts/lora/Lora.ttf', import.meta.url) },
  {
    id: 'bundled-varela-round',
    label: 'Varela Round',
    url: new URL('../assets/fonts/varela-round/VarelaRound-Regular.ttf', import.meta.url),
  },
  {
    id: 'bundled-space-mono',
    label: 'Space Mono',
    url: new URL('../assets/fonts/space-mono/SpaceMono-Regular.ttf', import.meta.url),
  },
  {
    id: 'bundled-alfa-slab-one',
    label: 'Alfa Slab One',
    url: new URL('../assets/fonts/alfa-slab-one/AlfaSlabOne-Regular.ttf', import.meta.url),
  },
]

/** Parsed fonts, memoized per `FontFaceEntry.id` (parsing a ~1MB variable
 *  font isn't free; the picker may re-list/reselect the same entry
 *  repeatedly in a session). Keyed by entry id rather than provider, so a
 *  bundled family, a system face, and a user file all share one cache with
 *  one shape. */
const entryFontCache = new Map<string, Promise<LoadedFont>>()

/** A human label for a font from its own `name` table: "Family Subfamily",
 *  dropping a redundant/default "Regular" subfamily (so a plain family
 *  reads as just "Inter", not "Inter Regular") and falling back to
 *  `fallbackFamily` (typically derived from the file name) when the font
 *  carries no usable name-table strings (some hand-built or stripped fonts
 *  omit them). Used by `fontSources.ts`'s user-file provider to refine a
 *  freshly-loaded file's PROVISIONAL family/style (filename-derived) into
 *  the font's own claimed name once bytes are actually parsed. */
export function familyStyleFromFont(
  font: opentype.Font,
  fallbackFamily: string,
): { family: string; style: string } {
  const family = font.names.fontFamily?.en ?? fallbackFamily
  const subfamily = font.names.fontSubfamily?.en
  const style = !subfamily || subfamily.toLowerCase() === 'regular' ? 'Regular' : subfamily
  return { family, style }
}

/** The picker's display label for a family/style pair: just the family
 *  when the style is the face's own default ("Regular"), else "Family
 *  Style" (so a genuine "MyFont-Bold.ttf" or a system "Menlo Bold" face
 *  reads as "MyFont Bold" / "Menlo Bold", not with a redundant "Regular"
 *  suffix on the common case). */
export function fontDisplayLabel(family: string, style: string): string {
  return style === '' || style.toLowerCase() === 'regular' ? family : `${family} ${style}`
}

/**
 * Loads (and memoizes by `entry.id`) the parsed font for ANY
 * `FontFaceEntry`, from ANY provider — bundled, system, or user.
 * `fetchBytes` is the caller's job (typically a `FontProvider.bytes(entry)`
 * call from `text/fontSources.ts`), so this module never itself fetches,
 * invokes Tauri, or reads a `File` — parsing and caching are ALL it does.
 * Rejects on bytes that don't parse as a font (opentype.js throws typed on
 * a malformed/unsupported table) — the dialog surfaces this as a toast,
 * never a silent fallback (DEVELOPMENT.md rule 4's spirit, applied
 * app-side); a failed parse is evicted from the cache rather than
 * poisoning future attempts (a transient failure — e.g. a network blip
 * fetching a bundled asset — shouldn't permanently block retrying the same
 * entry).
 */
export function loadFontEntry(entry: FontFaceEntry, fetchBytes: () => Promise<ArrayBuffer>): Promise<LoadedFont> {
  const cached = entryFontCache.get(entry.id)
  if (cached) return cached
  const promise = fetchBytes().then((buf) => {
    const font = opentype.parse(buf)
    return {
      id: entry.id,
      label: fontDisplayLabel(entry.family, entry.style),
      source: entry.source,
      font,
    }
  })
  entryFontCache.set(entry.id, promise)
  promise.catch(() => {
    if (entryFontCache.get(entry.id) === promise) entryFontCache.delete(entry.id)
  })
  return promise
}

/** The id of whichever font the dialog last placed (or last had picked) —
 *  module-level state, so re-opening the dialog later in the same session
 *  defaults back to it instead of resetting to the first bundled font
 *  every time (playtest finding 3: "remember the chosen custom font for
 *  the session"). `null` before anything has ever been picked. Never
 *  persisted with the document, matching every other bit of font state
 *  here. */
let lastSelectedFontId: string | null = null

/** The remembered font id, or `null` if nothing has been picked yet this
 *  session (or the remembered pick was a system/user font that's since
 *  become unavailable — callers fall back to the first bundled font). */
export function lastSelectedFont(): string | null {
  return lastSelectedFontId
}

/** Records `id` as the session's remembered font pick. */
export function rememberSelectedFont(id: string): void {
  lastSelectedFontId = id
}

/** Test-only: clears the parsed-font cache and the remembered selection, so
 *  tests don't leak state across cases. */
export function resetFontCachesForTests(): void {
  entryFontCache.clear()
  lastSelectedFontId = null
}

/**
 * Overwrites the memoized `LoadedFont` for `id` with `loaded` directly, no
 * fetch or re-parse. This is `loadUserFontFile`'s (in `fontSources.ts`)
 * escape hatch for a chicken-and-egg problem `loadFontEntry` alone can't
 * solve: `loadFontEntry` bakes its `label` from the `entry.family`/`style`
 * it's called with, which is correct for bundled/system entries (already
 * accurate at `list()` time) but only a filename-derived GUESS for a
 * freshly user-picked file — the real family/style come from the font's own
 * name table, knowable only AFTER parsing. Re-seeding the cache here (same
 * `id`, corrected `label`) is what keeps that corrected label from being
 * permanently shadowed by the guess `loadFontEntry`'s first call baked in
 * (adversarial-review finding 4 — this fixes a regression from the
 * pre-`fontSources.ts` `labelFromFont`, which computed the label directly
 * off the parsed font before constructing anything, with no such two-step
 * dance to get wrong).
 */
export function seedFontEntry(id: string, loaded: LoadedFont): void {
  entryFontCache.set(id, Promise.resolve(loaded))
}

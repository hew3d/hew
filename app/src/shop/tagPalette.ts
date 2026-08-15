/**
 * tagPalette — deterministic tag-section colors for Shop Mode's Parts sheet
 * (design_handoff_shop_mode/README.md §2 "Cutlist"): the prototype's tag dot
 * colors are per-fixture-example (`#9c5aa0` for "Accessories", `#a07d2e` for
 * "Unfiled") rather than a real palette — this module is the "small stable
 * palette ... derive per tag by stable hash of tag path" the handoff calls
 * for, so an arbitrary document's real tags (not just the two demo ones) get
 * a consistent, harmonious dot/tint color on every render, keyed purely off
 * the tag's own path text rather than render order (which a document
 * reload, or a tag rename, could reshuffle).
 *
 * The catch-all "Unfiled"/"Parts" section (`partsSheetModel.ts`'s synthetic
 * bucket for untagged nodes) is NOT a hashed tag — the handoff reserves it a
 * fixed color (`UNFILED_TAG_COLOR`, the prototype's own literal "Unfiled"
 * swatch) regardless of which of the two labels it renders with.
 */

/** 7 hues chosen to sit comfortably alongside the terracotta accent
 * (`--shop-accent` #c45d3c) and the charcoal dock without competing with
 * either — muted, mid-value, none reading as a status color (no pure red/
 * green). The first entry intentionally matches the prototype's own
 * "Accessories" example, so a document that happens to tag things the same
 * way the demo does renders identically to the handoff's screenshots.
 * `UNFILED_TAG_COLOR` is deliberately excluded from this list — it's
 * reserved, never a hash outcome. */
const TAG_PALETTE: readonly string[] = [
  '#9c5aa0', // violet
  '#3f7fa6', // steel blue
  '#4f8f5c', // moss green
  '#a3555f', // brick rose
  '#5f68a8', // indigo
  '#3f9490', // teal
  '#5c7a3f', // olive
]

/** The untagged catch-all's fixed color (README §2: "Unfiled `#a07d2e`") —
 * never hashed, since the catch-all isn't a real tag path. */
export const UNFILED_TAG_COLOR = '#a07d2e'

/** Small stable string hash (FNV-1a, 32-bit) — deterministic across runs and
 * platforms (unlike relying on iteration/insertion order), so the same tag
 * path always lands on the same palette entry. */
function hashString(s: string): number {
  let h = 0x811c9dc5
  for (let i = 0; i < s.length; i++) {
    h ^= s.charCodeAt(i)
    h = Math.imul(h, 0x01000193)
  }
  return h >>> 0
}

/** The stable palette color for a real tag section's full path (e.g.
 * `['Structure', 'Roof']`) — joined the same way `PartsSheetSection.label`
 * is (`path.join(' / ')`), so two sections that read identically on screen
 * can never hash to different colors. */
export function colorForTagPath(path: readonly string[]): string {
  const key = path.join(' / ')
  const idx = hashString(key) % TAG_PALETTE.length
  return TAG_PALETTE[idx]
}

/** Expand a `#rrggbb` hex color to an `rgba(...)` string at `alpha` (0-1) —
 * the tag dot color reused as a faint row-background tint (design: "row bg
 * = tag color at ~7-8%"). Only handles the 6-digit form `colorForTagPath`/
 * `UNFILED_TAG_COLOR` ever produce. */
export function hexToRgba(hex: string, alpha: number): string {
  const r = parseInt(hex.slice(1, 3), 16)
  const g = parseInt(hex.slice(3, 5), 16)
  const b = parseInt(hex.slice(5, 7), 16)
  return `rgba(${r}, ${g}, ${b}, ${alpha})`
}

/* oxlint-disable lingui/no-unlocalized-strings -- URLs and env names; no user-visible text */
// Where the catalog lives (docs/design/shared-library-sources.md 6.3). Kept
// apart from the real dependencies so UI code can name the catalog's host
// without pulling in the wasm wrappers.

/** The catalog's address. */
export const DEFAULT_CATALOG_URL = 'https://raw.githubusercontent.com/hew3d/catalog/data/catalog.json'

/** The catalog URL for this build: `VITE_HEW_CATALOG_URL` replaces it, and
 * an empty value hides "Browse the catalog" (self-hosters, tests). */
export function catalogUrlFor(env: { VITE_HEW_CATALOG_URL?: string | undefined }): string {
  const set = env.VITE_HEW_CATALOG_URL
  return set === undefined ? DEFAULT_CATALOG_URL : set.trim()
}

/** The configured catalog's host, for messages; null when it is hidden. */
export function catalogHost(): string | null {
  try {
    const url = catalogUrlFor(import.meta.env as { VITE_HEW_CATALOG_URL?: string })
    return url === '' ? null : new URL(url).host
  } catch {
    return null
  }
}

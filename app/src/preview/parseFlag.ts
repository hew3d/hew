/**
 * The one rule for reading the `HEW_PREVIEW` build variable, shared by
 * `vite.config.ts`, the vitest setup and the Playwright helper (none of them
 * can import app code that needs the flag itself, so this file has no
 * imports):
 *
 *   `1`              on
 *   `0`              off
 *   unset or empty   the command's default: on for the dev server and for
 *                    vitest, off for `vite build`
 *   anything else    a hard error (a typo must not silently pick a build)
 *
 * The Rust build reads the same variable and treats only exactly `1` as on;
 * it is never given anything but `1`, `0` or unset by the repository scripts.
 */
export function parsePreviewFlag(raw: string | undefined, defaultOn: boolean): boolean {
  if (raw === undefined || raw === '') return defaultOn
  if (raw === '1') return true
  if (raw === '0') return false
  throw new Error(`HEW_PREVIEW must be 1 (preview build), 0 (production build), or unset/empty (the command's default); got "${raw}".`)
}

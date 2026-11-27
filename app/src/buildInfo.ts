/**
 * Build-time identity as seen by the UI: the version string and whether this
 * is the beta web build (`HEW_BETA_SHA` set at build time — see
 * `buildVersion.ts` and `vite.config.ts`). Both are Vite `define`s that are
 * undefined outside a Vite build (plain `tsc`/vitest), so each falls back,
 * matching the convention in the other `__HEW_VERSION__` consumers. Read at
 * call time so tests can stub the globals.
 */

declare const __HEW_VERSION__: string | undefined
declare const __HEW_BETA__: boolean | undefined

/** True only in a build made with `HEW_BETA_SHA` set. */
export function isBetaBuild(): boolean {
  return typeof __HEW_BETA__ !== 'undefined' && __HEW_BETA__ === true
}

/** The full version string, including any `+beta.<sha>` stamp. */
export function appVersionString(): string {
  return typeof __HEW_VERSION__ !== 'undefined' ? __HEW_VERSION__ : '0.0.0'
}

/**
 * The app and the WASM kernel module are both built with or without the
 * Preview flag, and must agree. A production page on a preview module would
 * hand the live API's Preview commands to a regular user; a preview page on a
 * production module would refuse its own features. Either is a build mistake,
 * so the kernel loader checks at startup and stops (src/wasm/loader.ts).
 *
 * `kernelPreview` is the module's `preview_build()`; a module built before
 * that export existed has none (`undefined`), and there is nothing to compare.
 */

/* oxlint-disable lingui/no-unlocalized-strings -- a build-mistake message for the developer who made the build, never part of a working app */
export function kernelFlavorProblem(appPreview: boolean, kernelPreview: boolean | undefined): string | null {
  if (kernelPreview === undefined || kernelPreview === appPreview) return null
  return appPreview
    ? 'This preview build of Hew loaded a production kernel module. Rebuild the WASM package with HEW_PREVIEW=1.'
    : 'This production build of Hew loaded a preview kernel module. Rebuild the WASM package without HEW_PREVIEW.'
}
/* oxlint-enable lingui/no-unlocalized-strings */

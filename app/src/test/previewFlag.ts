/**
 * Vitest setup, listed before `setup.ts`: sets the Preview wall's build flag
 * for the run, the way `vite.config.ts` does for a build. `HEW_PREVIEW=0` is
 * the production run (flag off); `1`, unset or empty is the preview run; any other value throws (src/preview/parseFlag.ts) (flag on, as
 * the dev server defaults). Vitest does not apply Vite's `define` to code it
 * runs in Node, so the global is set here.
 *
 * `src/preview/wall.test.ts` still forces either flag per test with a module
 * mock of `src/preview/build.ts`; nothing at run time can do that.
 */
import { parsePreviewFlag } from '../preview/parseFlag'

const env = (globalThis as { process?: { env: Record<string, string | undefined> } }).process?.env
;(globalThis as { __HEW_PREVIEW__?: boolean }).__HEW_PREVIEW__ = parsePreviewFlag(env?.HEW_PREVIEW, true)

/**
 * The one place the build flag is read. `__HEW_PREVIEW__` is a literal
 * `true` or `false` that `vite.config.ts` (or `vitest.config.ts`) defines from
 * the build-time variable `HEW_PREVIEW`; nothing at run time can change it.
 *
 * Everything else asks the wall (`./wall.ts`, `./index.ts`). A test that needs
 * the flag off replaces THIS module (`vi.doMock`), never a run-time switch.
 * Code that exists only for preview builds is written
 * `if (previewBuild) { ... }` so the bundler drops it when the flag is off.
 */
export const previewBuild: boolean = __HEW_PREVIEW__

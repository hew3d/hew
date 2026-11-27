/**
 * The platform the browser under test runs on, for the few keys that differ
 * by platform. The app reads `navigator.platform` (src/platform.ts), and a
 * Playwright browser reports the host's, so the host decides.
 *
 * A command chord needs none of this: press `ControlOrMeta+Z` and Playwright
 * sends Cmd on a Mac and Ctrl elsewhere, which is the keymap's `Mod`. This
 * is for a binding that exists on one platform only, such as the Ctrl+K
 * palette alias and the Ctrl+Y redo that a Mac does not have.
 */
export const isMac = process.platform === 'darwin'

/**
 * Welcome-screen visibility — module-level singleton.
 *
 * One persisted flag: whether the welcome screen appears on a bare launch
 * (no file-association open, no recovery prompt, primary window). Persisted
 * to localStorage like the rest of settings/*; no cross-window sync — the
 * flag is only read once, at startup, by whichever window is deciding
 * whether to show the screen.
 */

const STORAGE_KEY = 'hew.settings.showWelcome'

/** Whether the welcome screen should show on a bare launch (default true). */
export function getShowWelcome(): boolean {
  try {
    return localStorage.getItem(STORAGE_KEY) !== 'false'
  } catch {
    return true
  }
}

/** Persist the "show on startup" choice. */
export function setShowWelcome(show: boolean): void {
  try {
    localStorage.setItem(STORAGE_KEY, show ? 'true' : 'false')
  } catch {
    /* storage unavailable — the screen simply shows next launch */
  }
}

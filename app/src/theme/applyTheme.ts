/**
 * Applies the resolved theme to the document as a `data-theme` attribute
 *, which `theme/tokens.css`'s `[data-theme="dark"|"light"]`
 * blocks key off of.
 *
 * A plain function rather than a React provider/context: the main app window
 * and the separate Settings webview window (Tauri) are each their own
 * top-level HTML document with no shared React tree, so a context provider
 * wouldn't reach both. `initThemeSync()` is called once, unconditionally,
 * from `main.tsx` before either root renders — it operates on
 * `document.documentElement` directly, which works identically for both.
 */

import { getResolvedTheme, subscribe } from '../settings/theme'

function apply(): void {
  document.documentElement.dataset.theme = getResolvedTheme()
}

/**
 * Set the initial `data-theme` attribute synchronously (must run before first
 * paint to avoid a flash of the wrong theme), then keep it in sync with the
 * persisted setting and, when the setting is 'auto', the OS-level
 * `prefers-color-scheme` media query. Returns an unsubscribe-all cleanup
 * (unused today — `main.tsx` never unmounts — but keeps this testable/leak-free).
 */
export function initThemeSync(): () => void {
  apply()

  const unsubscribeSetting = subscribe(apply)

  let unsubscribeMedia = () => {}
  if (typeof window !== 'undefined' && typeof window.matchMedia === 'function') {
    const media = window.matchMedia('(prefers-color-scheme: dark)')
    // Recomputing on every change is a no-op when the setting is explicit
    // (not 'auto') — cheaper than tracking whether 'auto' is active.
    media.addEventListener('change', apply)
    unsubscribeMedia = () => media.removeEventListener('change', apply)
  }

  return () => {
    unsubscribeSetting()
    unsubscribeMedia()
  }
}

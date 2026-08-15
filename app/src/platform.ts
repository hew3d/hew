/**
 * Platform detection for keyboard-shortcut *display*.
 *
 * Hew ships the same web bundle on macOS, Windows, and Linux (Tauri +
 * WebKitGTK). Key handling already accepts `metaKey || ctrlKey` everywhere, so
 * this only governs how shortcut hints are rendered: the ⌘ glyph reads as
 * "Command" and is wrong on Linux/Windows, where the modifier is Ctrl.
 *
 * `navigator.platform` is deprecated but still populated by every webview we
 * target (incl. WebKitGTK, where it reports "Linux x86_64"), and needs no async
 * `userAgentData` round-trip.
 */
export const isMac: boolean =
  typeof navigator !== 'undefined' && /Mac|iPod|iPhone|iPad/.test(navigator.platform)

/**
 * True on Linux (incl. the WebKitGTK desktop webview, where `navigator.platform`
 * reports "Linux x86_64"). Used to switch the desktop shell to custom window
 * chrome: KWin/WebKitGTK won't repaint the server-side titlebar after
 * `setTitle`, so on Linux we go borderless and draw our own title bar + in-app
 * menu (see `TitleBar.tsx`). Windows joins this treatment in a later
 * milestone; macOS keeps native decorations + menu.
 */
export const isLinux: boolean =
  typeof navigator !== 'undefined' &&
  /Linux|X11/.test(navigator.platform) &&
  !/Android/.test(navigator.userAgent)

/**
 * True on Windows (`navigator.platform` reports "Win32"/"Win64" in every
 * webview we target, incl. Tauri's WebView2). Added in alongside
 * the Windows/Linux/Web bare-letter keybinding split — see the global keydown
 * handler in `App.tsx`, which now runs on Windows too (previously Linux-only
 * under Tauri).
 */
export const isWindows: boolean =
  typeof navigator !== 'undefined' && /Win/.test(navigator.platform)

/** Modifier prefix shown in shortcut hints: '⌘' on macOS, 'Ctrl+' elsewhere. */
export const modLabel: string = isMac ? '⌘' : 'Ctrl+'

/**
 * True on a coarse-pointer input device — touch, with no precise mouse —
 * per the `(pointer: coarse)` media feature. Two independent, unrelated
 * consumers key off this ONE test rather than each rolling their own:
 * `shop/shellMode.ts`'s auto-detection heuristic (coarse pointer + a small
 * viewport → Shop Mode) and `viewport/snapService.ts`'s pick/snap aperture
 * (a fingertip is far less precise than a mouse cursor, so touch gets a
 * wider acquire radius).
 *
 * A function, not a top-level constant like `isMac`/`isLinux`/`isWindows`
 * above: those read `navigator.platform`, fixed for the process lifetime;
 * `matchMedia` needs `window`, which some test/SSR contexts lack, and a
 * function lets a caller re-check live (a hybrid laptop's pointer type can
 * change if a touchscreen is (dis)connected, unlike the OS) or inject a
 * stub `window.matchMedia` in tests without touching this module.
 */
export function isCoarsePointer(): boolean {
  if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') return false
  try {
    // The MediaQueryList is cached (keyed on the live `window.matchMedia`
    // identity, so a test swapping in a fresh stub — or an environment
    // replacing the global — invalidates it) because this runs on EVERY
    // snap resolve, i.e. per pointermove during a drag: re-parsing the
    // query each call is measurable churn there. Reading `.matches` off
    // the cached list stays live — the browser updates the list when a
    // hybrid laptop's pointer situation changes — which is exactly the
    // "re-check live" property the doc comment above promises.
    if (cachedPointerQuery === null || cachedPointerQueryFrom !== window.matchMedia) {
      cachedPointerQueryFrom = window.matchMedia
      cachedPointerQuery = window.matchMedia('(pointer: coarse)')
    }
    return cachedPointerQuery.matches
  } catch {
    return false
  }
}

let cachedPointerQuery: MediaQueryList | null = null
let cachedPointerQueryFrom: typeof window.matchMedia | null = null

/**
 * True when the OS/browser `(prefers-reduced-motion: reduce)` media feature
 * matches — the one JS-side check for motion that isn't already handled by a
 * CSS `@media (prefers-reduced-motion: reduce)` block (this file's own
 * `.hew-*`/`.shop-*` animation classes in `index.css` cover the DOM/CSS
 * side; this is for motion driven from TypeScript instead, e.g.
 * `SceneRenderer.setHiddenFaded`'s per-frame opacity tween, which has no CSS
 * animation to gate). Same caching/guard shape as `isCoarsePointer` above —
 * see its doc comment for why this is a function, not a top-level constant.
 */
export function prefersReducedMotion(): boolean {
  if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') return false
  try {
    if (cachedReducedMotionQuery === null || cachedReducedMotionQueryFrom !== window.matchMedia) {
      cachedReducedMotionQueryFrom = window.matchMedia
      cachedReducedMotionQuery = window.matchMedia('(prefers-reduced-motion: reduce)')
    }
    return cachedReducedMotionQuery.matches
  } catch {
    return false
  }
}

let cachedReducedMotionQuery: MediaQueryList | null = null
let cachedReducedMotionQueryFrom: typeof window.matchMedia | null = null

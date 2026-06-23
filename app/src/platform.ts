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
 * menu (see `TitleBar.tsx`). macOS/Windows keep native decorations + menu.
 */
export const isLinux: boolean =
  typeof navigator !== 'undefined' &&
  /Linux|X11/.test(navigator.platform) &&
  !/Android/.test(navigator.userAgent)

/** Modifier prefix shown in shortcut hints: '⌘' on macOS, 'Ctrl+' elsewhere. */
export const modLabel: string = isMac ? '⌘' : 'Ctrl+'

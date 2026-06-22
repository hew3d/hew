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

/** Modifier prefix shown in shortcut hints: '⌘' on macOS, 'Ctrl+' elsewhere. */
export const modLabel: string = isMac ? '⌘' : 'Ctrl+'

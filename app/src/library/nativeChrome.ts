// Native chrome for the desktop Library window: stock context menus and
// confirmation dialogs served by the shell (`library_popup_menu` /
// `library_confirm` in main.rs). The web build keeps its DOM fallbacks —
// these helpers are only ever called from the `'window'`-variant dialog,
// which exists only under Tauri.

export interface NativeMenuEntry {
  id: string
  label: string
  disabled?: boolean
  separator?: boolean
}

let menuListenerStarted = false
let pendingResolve: ((id: string | null) => void) | null = null

/** Pops a native context menu at the cursor. Resolves with the picked
 * entry's id, or null when dismissed — a dismissal produces no event (the
 * native-menu contract), so the dangling promise is resolved null when the
 * NEXT popup supersedes it; callers treat "never resolved" and "null" the
 * same way (do nothing). */
export async function popupNativeMenu(entries: NativeMenuEntry[]): Promise<string | null> {
  const { invoke } = await import('@tauri-apps/api/core')
  const { getCurrentWebviewWindow } = await import('@tauri-apps/api/webviewWindow')
  if (!menuListenerStarted) {
    menuListenerStarted = true
    try {
      await getCurrentWebviewWindow().listen<string>('library-menu', (event) => {
        pendingResolve?.(event.payload)
        pendingResolve = null
      })
    } catch (err) {
      // A failed registration must not permanently deaden every future
      // menu (adversarial review S6) — retry on the next popup.
      menuListenerStarted = false
      throw err
    }
  }
  pendingResolve?.(null)
  return new Promise((resolve) => {
    pendingResolve = resolve
    void invoke('library_popup_menu', { entries }).catch(() => {
      if (pendingResolve === resolve) pendingResolve = null
      resolve(null)
    })
  })
}

/** Native confirm on this window — OK button carries `actionLabel`. */
export async function nativeConfirm(
  title: string,
  message: string,
  actionLabel: string,
): Promise<boolean> {
  const { invoke } = await import('@tauri-apps/api/core')
  try {
    return await invoke<boolean>('library_confirm', { title, message, actionLabel })
  } catch {
    return false
  }
}

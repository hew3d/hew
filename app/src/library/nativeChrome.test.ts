/**
 * The native Library menu passes whatever entries JS provides to the shell's
 * `library_popup_menu` and returns the picked id from the `library-menu`
 * event. The shell renders any id/label (main.rs `LibraryMenuEntry`), so the
 * source rows ("Update from source", "Unlink from <source>") need no shell
 * change; which items get them is the dialog's call (LibrarySources.test.tsx).
 */
import { beforeEach, describe, expect, it, vi } from 'vitest'

const { mockInvoke, listeners } = vi.hoisted(() => ({ mockInvoke: vi.fn(), listeners: [] as Array<(e: { payload: string }) => void> }))
vi.mock('@tauri-apps/api/core', () => ({ invoke: mockInvoke }))
vi.mock('@tauri-apps/api/webviewWindow', () => ({
  getCurrentWebviewWindow: () => ({
    listen: async (_name: string, fn: (e: { payload: string }) => void) => {
      listeners.push(fn)
      return () => {}
    },
  }),
}))

import { popupNativeMenu } from './nativeChrome'

beforeEach(() => mockInvoke.mockReset())

describe('popupNativeMenu', () => {
  it('sends every entry through unchanged, source rows included, and resolves with the picked id', async () => {
    mockInvoke.mockResolvedValue(undefined)
    const entries = [
      { id: 'open', label: 'Open as Document' },
      { id: 'sepSource', label: '', separator: true },
      { id: 'updateSource', label: 'Update from source' },
      { id: 'unlinkSource', label: 'Unlink from Hinges', disabled: false },
    ]
    const picked = popupNativeMenu(entries)
    await vi.waitFor(() => expect(mockInvoke).toHaveBeenCalledWith('library_popup_menu', { entries }))
    listeners[0]({ payload: 'updateSource' })
    expect(await picked).toBe('updateSource')
  })
})

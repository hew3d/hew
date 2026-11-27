// @vitest-environment jsdom
//
// On the desktop the installer is the shell's commands and nothing else: no
// error path may fall back to the browser store, which would write plugin
// files into the webview's own storage.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { requiresPreview } from '../test/requiresPreview'

beforeEach((ctx) => requiresPreview(ctx, 'plugins'))
afterEach(() => {
  vi.unstubAllGlobals()
  vi.doUnmock('@tauri-apps/api/core')
  delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__
  vi.resetModules()
})

describe('the Tauri tier', () => {
  it('never touches browser storage, even when every command fails', async () => {
    const getDirectory = vi.fn(async () => {
      throw new Error('browser storage must not be used')
    })
    vi.stubGlobal('navigator', { ...navigator, storage: { getDirectory } })
    ;(window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {}
    vi.resetModules()
    vi.doMock('@tauri-apps/api/core', () => ({
      invoke: async () => {
        throw 'io: the shell is not answering'
      },
    }))
    const { pluginInstaller } = await import('./installer')
    const installer = pluginInstaller()
    expect(installer.tier).toBe('desktop')
    await expect(installer.list()).rejects.toMatchObject({ refusal: 'io' })
    await expect(installer.commit('t', true)).rejects.toBeTruthy()
    await expect(installer.setEnabled('com.example.one', true)).rejects.toBeTruthy()
    await expect(installer.uninstall('com.example.one')).rejects.toBeTruthy()
    await expect(installer.rollback('com.example.one')).rejects.toBeTruthy()
    await expect(installer.stageFile(new File([], 'x.hewplugin'))).rejects.toBeTruthy()
    expect(await installer.iconUrl('com.example.one', 'icon.png')).toBeNull()
    expect(installer.previous).toBeUndefined()
    expect(getDirectory).not.toHaveBeenCalled()
  })

  it('the web tier is chosen only without the Tauri runtime', async () => {
    vi.resetModules()
    const { pluginInstaller } = await import('./installer')
    expect(pluginInstaller().tier).toBe('web')
  })
})

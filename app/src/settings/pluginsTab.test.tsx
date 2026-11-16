import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { requiresPreview } from '../test/requiresPreview'

/**
 * Settings shows the Plugins tab only while the Plugins preview switch is on.
 * The wall reads the switch once when the app loads, so each case sets the
 * stored switch and loads Settings fresh.
 */

async function settings(on: boolean) {
  vi.resetModules()
  localStorage.setItem('hew.settings.preview', JSON.stringify({ off: [], on: on ? ['plugins'] : [] }))
  const { SettingsWindow } = await import('./SettingsWindow')
  return render(<SettingsWindow />)
}

afterEach(() => {
  cleanup()
  localStorage.removeItem('hew.settings.preview')
  vi.unstubAllGlobals()
})

describe('Settings > Plugins', () => {
  beforeEach((ctx) => requiresPreview(ctx, 'plugins'))

  it('has no Plugins tab while the switch is off', async () => {
    await settings(false)
    expect(screen.queryByRole('tab', { name: 'Plugins' })).toBeNull()
  })

  it('has the tab and its pane while the switch is on', async () => {
    // An empty browser file system: nothing installed.
    const root = {
      getDirectoryHandle: async () => {
        throw new Error('NotFoundError')
      },
    }
    vi.stubGlobal('navigator', { ...navigator, storage: { getDirectory: async () => root } })
    await settings(true)
    fireEvent.click(screen.getByRole('tab', { name: 'Plugins' }))
    expect(await screen.findByText('No plugins are installed.')).toBeInTheDocument()
    expect(screen.getByText(/do not run yet in this build/)).toBeInTheDocument()
  })
})

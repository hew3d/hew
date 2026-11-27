/**
 * Settings ▸ Advanced "Automatically download and install updates", in both
 * renderings (AdvancedPane and FluentSettingsPage) on the shared hook:
 * shown only where the updater exists and the shell says it can work, saved
 * the moment it is toggled.
 */
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { AdvancedPane } from './AdvancedPane'
import { FluentSettingsPage } from './FluentSettingsPage'

const m = vi.hoisted(() => ({
  available: true,
  auto: { enabled: false, supported: true },
  set: [] as boolean[],
}))

vi.mock('../updater/updateIpc', () => ({
  updaterAvailable: async () => m.available,
  updateAutoGet: async () => ({ ...m.auto }),
  updateAutoSet: async (enabled: boolean) => {
    m.set.push(enabled)
    m.auto = { ...m.auto, enabled }
    return { ...m.auto }
  },
}))

// The desktop build: the server setting exists, so the Advanced pane renders
// its full form (the browser build's read-only note has no updates row).
vi.mock('./server', async (importOriginal) => {
  const actual = await importOriginal<typeof import('./server')>()
  return {
    ...actual,
    serverSettingAvailable: () => true,
    getServerSetting: () => Promise.resolve({ mode: 'cloud', origin: 'https://app.hew3d.com', uploadKey: '' }),
    setServerSetting: async (next: unknown) => next,
  }
})

beforeEach(() => {
  m.available = true
  m.auto = { enabled: false, supported: true }
  m.set = []
})

const LABEL = 'Automatically download and install updates'

describe('AdvancedPane: automatic updates', () => {
  it('saves at once when toggled', async () => {
    render(<AdvancedPane />)
    const box = await screen.findByRole('checkbox', { name: LABEL })
    expect(box).not.toBeChecked()
    fireEvent.click(box)
    await waitFor(() => expect(m.set).toEqual([true]))
    await waitFor(() => expect(box).toBeChecked())
  })

  it('shows the stored value', async () => {
    m.auto = { enabled: true, supported: true }
    render(<AdvancedPane />)
    expect(await screen.findByRole('checkbox', { name: LABEL })).toBeChecked()
  })

  it('is absent when the copy cannot install by itself', async () => {
    m.auto = { enabled: false, supported: false }
    render(<AdvancedPane />)
    await new Promise((r) => setTimeout(r, 20))
    expect(screen.queryByRole('checkbox', { name: LABEL })).toBeNull()
  })

  it('is absent without an updater (web, preview)', async () => {
    m.available = false
    render(<AdvancedPane />)
    await new Promise((r) => setTimeout(r, 20))
    expect(screen.queryByRole('checkbox', { name: LABEL })).toBeNull()
  })
})

describe('FluentSettingsPage: automatic updates', () => {
  it('saves at once when the switch is toggled', async () => {
    render(<FluentSettingsPage onBack={() => {}} />)
    const sw = await screen.findByRole('switch', { name: LABEL })
    expect(sw).toHaveAttribute('aria-checked', 'false')
    fireEvent.click(sw)
    await waitFor(() => expect(m.set).toEqual([true]))
    await waitFor(() => expect(sw).toHaveAttribute('aria-checked', 'true'))
  })

  it('is absent where it cannot work', async () => {
    m.auto = { enabled: false, supported: false }
    render(<FluentSettingsPage onBack={() => {}} />)
    await screen.findByText('Debug')
    await new Promise((r) => setTimeout(r, 20))
    expect(screen.queryByRole('switch', { name: LABEL })).toBeNull()
  })
})

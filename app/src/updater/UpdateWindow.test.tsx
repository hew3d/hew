/**
 * UpdateWindow: each install row, the progress and restart steps, read-only
 * mode, failure and the cancelled password prompt (docs/design D3). The
 * shell contract (updateIpc.ts) is mocked; events are fired by hand.
 */
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { UpdateWindow } from './UpdateWindow'
import type { InstallResult, UpdateInfo } from './updateIpc'

const m = vi.hoisted(() => ({
  info: null as unknown,
  handlers: new Map<string, (p: unknown) => void>(),
  auto: { enabled: false, supported: true },
  calls: [] as string[],
  install: undefined as undefined | (() => Promise<unknown>),
  restart: undefined as undefined | (() => Promise<unknown>),
  copy: undefined as undefined | (() => Promise<void>),
  open: undefined as unknown,
  close: undefined as unknown,
}))
m.open = vi.fn()
m.close = vi.fn(async () => {})

vi.mock('../panels/ReportBugDialog', () => ({ openExternal: (u: string) => (m.open as (u: string) => void)(u) }))
vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: () => ({ close: m.close }) }))
vi.mock('./updateIpc', () => ({
  updaterAvailable: async () => true,
  updateInfo: async () => m.info,
  updateInstall: () => (m.install ?? (async () => ({ status: 'installed', version: '1.6.0', error: null, closesApp: false })))(),
  updateSkip: async () => void m.calls.push('skip'),
  updateRemind: async () => void m.calls.push('remind'),
  updateRestart: () => {
    m.calls.push('restart')
    return (m.restart ?? (() => new Promise(() => {})))()
  },
  updateCopyCommand: () => {
    m.calls.push('copy')
    return (m.copy ?? (async () => {}))()
  },
  updateAutoGet: async () => ({ ...m.auto }),
  updateAutoSet: async (enabled: boolean) => {
    m.calls.push(`auto:${enabled}`)
    m.auto = { ...m.auto, enabled }
    return { ...m.auto }
  },
  onEvent: async (name: string, handler: (p: unknown) => void) => {
    m.handlers.set(name, handler)
    return () => m.handlers.delete(name)
  },
}))

function info(over: Partial<UpdateInfo> = {}, plan: Partial<UpdateInfo['plan']> = {}): UpdateInfo {
  return {
    version: '1.6.0',
    currentVersion: '1.5.0',
    notes: '### Added\n\n- **Lock:** keep an object fixed.\n',
    date: '2026-12-01',
    plan: {
      mode: 'selfInstall',
      channel: 'macApp',
      command: null,
      helpUrl: 'https://hew3d.com/download/update-help/',
      closesApp: false,
      ...plan,
    },
    phase: 'offer',
    readOnly: false,
    autoInstall: false,
    autoSupported: true,
    changelogUrl: 'https://hew3d.com/changelog/',
    downloadUrl: 'https://hew3d.com/download/',
    error: null,
    ...over,
  }
}

async function open(i: UpdateInfo) {
  m.info = i
  render(<UpdateWindow />)
  await screen.findByTestId('update-heading')
}

const fire = (name: string, payload?: unknown) => act(() => m.handlers.get(name)?.(payload))

beforeEach(() => {
  m.handlers.clear()
  m.calls = []
  m.auto = { enabled: false, supported: true }
  m.install = undefined
  m.restart = undefined
  m.copy = undefined
  ;(m.open as ReturnType<typeof vi.fn>).mockClear()
  ;(m.close as ReturnType<typeof vi.fn>).mockClear()
  Object.defineProperty(window, '__TAURI_INTERNALS__', { value: {}, configurable: true })
})

describe('UpdateWindow: self-install row (Mac app, Windows installer, AppImage)', () => {
  it('shows the offer, notes, link, checkbox and the three buttons', async () => {
    await open(info())
    expect(screen.getByTestId('update-heading')).toHaveTextContent('A new version of Hew is available!')
    expect(screen.getByTestId('update-message')).toHaveTextContent(
      'Hew 1.6.0 is now available. You have 1.5.0. Would you like to update now?',
    )
    expect(screen.getByTestId('update-notes')).toHaveTextContent('Lock: keep an object fixed.')
    expect(screen.getByText('Release notes')).toBeInTheDocument()
    expect(await screen.findByTestId('update-auto')).not.toBeChecked()
    for (const id of ['update-skip', 'update-remind', 'update-install']) expect(screen.getByTestId(id)).toBeInTheDocument()
    expect(screen.queryByTestId('update-password-note')).toBeNull()
    expect(screen.queryByTestId('update-help')).toBeNull()
    fireEvent.click(screen.getByTestId('update-all-notes'))
    expect(m.open).toHaveBeenCalledWith('https://hew3d.com/changelog/')
  })

  it('saves the checkbox the moment it is toggled', async () => {
    await open(info())
    fireEvent.click(await screen.findByTestId('update-auto'))
    await waitFor(() => expect(m.calls).toContain('auto:true'))
    await waitFor(() => expect(screen.getByTestId('update-auto')).toBeChecked())
  })

  it('hides the checkbox when the shell says automatic updates cannot work', async () => {
    m.auto = { enabled: false, supported: false }
    await open(info({ autoSupported: false }))
    await new Promise((r) => setTimeout(r, 20))
    expect(screen.queryByTestId('update-auto')).toBeNull()
  })

  it('Skip and Remind call the shell', async () => {
    await open(info())
    fireEvent.click(screen.getByTestId('update-skip'))
    fireEvent.click(screen.getByTestId('update-remind'))
    await waitFor(() => expect(m.calls).toEqual(['skip', 'remind']))
  })

  it('Escape is Remind Me Later', async () => {
    await open(info())
    fireEvent.keyDown(window, { key: 'Escape' })
    await waitFor(() => expect(m.calls).toEqual(['remind']))
  })

  it('empty or missing notes read "Release notes are on the Hew releases page." with the link', async () => {
    await open(info({ notes: null }))
    expect(screen.getByTestId('update-notes')).toHaveTextContent('Release notes are on the Hew releases page.')
    expect(screen.getByTestId('update-all-notes')).toBeInTheDocument()
  })

  it('blank notes get the same fallback', async () => {
    await open(info({ notes: '  \n ' }))
    expect(screen.getByTestId('update-notes')).toHaveTextContent('Release notes are on the Hew releases page.')
  })
})

describe('UpdateWindow: install, progress and restart', () => {
  it('goes offer -> progress bar -> restart step', async () => {
    let finish: (r: InstallResult) => void = () => {}
    m.install = () => new Promise((res) => (finish = res as (r: InstallResult) => void))
    await open(info())
    fireEvent.click(screen.getByTestId('update-install'))
    expect(await screen.findByText('Downloading Hew 1.6.0…')).toBeInTheDocument()
    expect(screen.queryByTestId('update-install')).toBeNull()
    fire('update-progress', { phase: 'downloading', version: '1.6.0', downloaded: 20, total: 40 })
    expect(screen.getByRole('progressbar')).toHaveAttribute('value', '50')
    fire('update-progress', { phase: 'installing', version: '1.6.0', closesApp: false })
    expect(screen.getByTestId('update-installing')).toHaveTextContent('Installing Hew 1.6.0…')
    await act(async () => finish({ status: 'installed', version: '1.6.0', error: null, closesApp: false }))
    expect(screen.getByTestId('update-message')).toHaveTextContent('Hew 1.6.0 is installed. Restart now to use it?')
    // Restart waits on the shell (closing the documents) with Later
    // disabled; a Cancel in a save prompt brings Later back.
    let answer: (r: unknown) => void = () => {}
    m.restart = () => new Promise((res) => (answer = res))
    fireEvent.click(screen.getByTestId('update-restart'))
    await waitFor(() => expect(m.calls).toContain('restart'))
    expect(screen.getByTestId('update-later')).toBeDisabled()
    expect(screen.getByTestId('update-restart')).toBeDisabled()
    await act(async () => answer({ status: 'cancelled' }))
    await waitFor(() => expect(screen.getByTestId('update-later')).toBeEnabled())
    fireEvent.click(screen.getByTestId('update-later'))
    await waitFor(() => expect(m.close).toHaveBeenCalled())
  })

  it('an unknown total shows an indeterminate bar', async () => {
    m.install = () => new Promise(() => {})
    await open(info())
    fireEvent.click(screen.getByTestId('update-install'))
    fire('update-progress', { phase: 'downloading', version: '1.6.0', downloaded: 5, total: null })
    expect(screen.getByRole('progressbar')).not.toHaveAttribute('value')
  })

  it('on Windows the installing step says Hew will close', async () => {
    m.install = () => new Promise(() => {})
    await open(info({}, { channel: 'windowsInstaller', closesApp: true }))
    fireEvent.click(screen.getByTestId('update-install'))
    fire('update-progress', { phase: 'installing', version: '1.6.0', closesApp: true })
    expect(screen.getByTestId('update-installing')).toHaveTextContent('Hew will close to finish installing.')
  })

  it('opens on the restart step when already installed', async () => {
    await open(info({ phase: 'installed' }))
    expect(screen.getByTestId('update-heading')).toHaveTextContent('Hew 1.6.0 is installed')
    expect(screen.getByTestId('update-restart')).toHaveTextContent('Restart Now')
    expect(screen.queryByTestId('update-install')).toBeNull()
  })

  it('opens on the install step when downloaded but not installed (Windows)', async () => {
    await open(info({ phase: 'ready' }, { channel: 'windowsInstaller', closesApp: true }))
    expect(screen.getByTestId('update-message')).toHaveTextContent(
      'Hew 1.6.0 is ready to install. Hew will close to finish installing.',
    )
    expect(screen.getByTestId('update-restart')).toHaveTextContent('Install and Restart')
    expect(screen.getByTestId('update-later')).toHaveTextContent('Later')
  })

  it('on Windows, Install Update stops at the ready step: nothing closes Hew yet', async () => {
    m.install = async () => ({ status: 'ready', version: '1.6.0', error: null, closesApp: true })
    await open(info({}, { channel: 'windowsInstaller', closesApp: true }))
    fireEvent.click(screen.getByTestId('update-install'))
    expect(await screen.findByTestId('update-restart')).toHaveTextContent('Install and Restart')
    expect(screen.getByTestId('update-message')).toHaveTextContent(
      'Hew 1.6.0 is ready to install. Hew will close to finish installing.',
    )
    expect(m.calls).toEqual([])
  })

  it('a "ready" result delivered by event shows the same step', async () => {
    m.install = () => new Promise(() => {})
    await open(info({}, { channel: 'windowsInstaller', closesApp: true }))
    fireEvent.click(screen.getByTestId('update-install'))
    fire('update-result', { status: 'ready', version: '1.6.0', error: null, closesApp: true })
    expect(screen.getByTestId('update-restart')).toHaveTextContent('Install and Restart')
  })

  it('opens on the progress bar mid-download', async () => {
    await open(info({ phase: 'downloading' }))
    expect(screen.getByTestId('update-progress')).toBeInTheDocument()
    expect(screen.queryByTestId('update-skip')).toBeNull()
  })

  it('Escape does nothing while downloading', async () => {
    m.install = () => new Promise(() => {})
    await open(info())
    fireEvent.click(screen.getByTestId('update-install'))
    fireEvent.keyDown(window, { key: 'Escape' })
    await new Promise((r) => setTimeout(r, 10))
    expect(m.calls).toEqual([])
    expect(m.close).not.toHaveBeenCalled()
  })
})

describe('UpdateWindow: restarting closes the documents first', () => {
  it('while the documents close: buttons disabled and one line', async () => {
    await open(info({ phase: 'installed' }))
    fireEvent.click(screen.getByTestId('update-restart'))
    expect(await screen.findByTestId('update-restart-pending')).toHaveTextContent('Closing your documents…')
    expect(screen.getByTestId('update-restart')).toBeDisabled()
    expect(screen.getByTestId('update-later')).toBeDisabled()
    // A second click cannot start another restart, and Escape does nothing.
    fireEvent.click(screen.getByTestId('update-restart'))
    fireEvent.keyDown(window, { key: 'Escape' })
    await new Promise((r) => setTimeout(r, 10))
    expect(m.calls).toEqual(['restart'])
    expect(m.close).not.toHaveBeenCalled()
  })

  it('installed, cancelled from a save prompt: same step, one added line', async () => {
    m.restart = async () => ({ status: 'cancelled' })
    await open(info({ phase: 'installed' }))
    const message = screen.getByTestId('update-message').textContent
    fireEvent.click(screen.getByTestId('update-restart'))
    expect(await screen.findByTestId('update-restart-cancelled')).toHaveTextContent(
      'Restart cancelled. Hew 1.6.0 will open the next time you start Hew.',
    )
    expect(screen.getByTestId('update-message').textContent).toBe(message)
    expect(screen.queryByTestId('update-restart-pending')).toBeNull()
    expect(screen.getByTestId('update-restart')).toBeEnabled()
    expect(screen.getByTestId('update-restart')).toHaveTextContent('Restart Now')
    expect(screen.getByTestId('update-later')).toBeEnabled()
  })

  it('ready (Windows), cancelled from a save prompt: same step, one added line', async () => {
    m.restart = async () => ({ status: 'cancelled' })
    await open(info({ phase: 'ready' }, { channel: 'windowsInstaller', closesApp: true }))
    fireEvent.click(screen.getByTestId('update-restart'))
    expect(await screen.findByTestId('update-restart-cancelled')).toHaveTextContent(
      'Install cancelled. Hew 1.6.0 is downloaded and ready when you are.',
    )
    expect(screen.getByTestId('update-message')).toHaveTextContent(
      'Hew 1.6.0 is ready to install. Hew will close to finish installing.',
    )
    expect(screen.getByTestId('update-restart')).toBeEnabled()
    expect(screen.getByTestId('update-restart')).toHaveTextContent('Install and Restart')
  })

  it('a restart already running elsewhere: buttons come back with no cancelled line', async () => {
    m.restart = async () => ({ status: 'inProgress' })
    await open(info({ phase: 'installed' }))
    fireEvent.click(screen.getByTestId('update-restart'))
    await waitFor(() => expect(screen.getByTestId('update-restart')).toBeEnabled())
    expect(screen.queryByTestId('update-restart-cancelled')).toBeNull()
    expect(screen.queryByTestId('update-restart-pending')).toBeNull()
  })

  it('the installer failing after the documents closed shows Open Download Page', async () => {
    m.restart = () => new Promise(() => {})
    await open(info({ phase: 'ready' }, { channel: 'windowsInstaller', closesApp: true }))
    fireEvent.click(screen.getByTestId('update-restart'))
    fire('update-progress', { phase: 'installing', version: '1.6.0', closesApp: true })
    expect(screen.getByTestId('update-installing')).toHaveTextContent('Hew will close to finish installing.')
    fire('update-result', { status: 'failed', version: '1.6.0', error: 'bad package', closesApp: true })
    expect(screen.getByTestId('update-error')).toHaveTextContent('bad package')
    expect(screen.getByTestId('update-download')).toBeInTheDocument()
  })

  it('a window opened after that failure starts on the failure step', async () => {
    await open(info({ phase: 'offer', error: 'bad package' }, { channel: 'windowsInstaller', closesApp: true }))
    expect(screen.getByTestId('update-error')).toHaveTextContent('bad package')
    expect(screen.getByTestId('update-download')).toBeInTheDocument()
    expect(screen.queryByTestId('update-install')).toBeNull()
  })
})

describe('UpdateWindow: failure and a dismissed password prompt', () => {
  it('shows the error and Open Download Page', async () => {
    m.install = async () => ({ status: 'failed', version: '1.6.0', error: 'signature mismatch', closesApp: false })
    await open(info())
    fireEvent.click(screen.getByTestId('update-install'))
    expect(await screen.findByTestId('update-error')).toHaveTextContent('The update could not be installed.')
    expect(screen.getByTestId('update-error')).toHaveTextContent('signature mismatch')
    fireEvent.click(screen.getByTestId('update-download'))
    expect(m.open).toHaveBeenCalledWith('https://hew3d.com/download/')
  })

  it('a failure delivered by event shows the same thing', async () => {
    m.install = () => new Promise(() => {})
    await open(info())
    fireEvent.click(screen.getByTestId('update-install'))
    fire('update-result', { status: 'failed', version: '1.6.0', error: 'disk full', closesApp: false })
    expect(screen.getByTestId('update-error')).toHaveTextContent('disk full')
  })

  it('cancelled goes back to the offer with no error', async () => {
    m.install = async () => ({ status: 'cancelled', version: '1.6.0', error: null, closesApp: false })
    await open(info({}, { mode: 'selfInstallElevated', channel: 'deb' }))
    fireEvent.click(screen.getByTestId('update-install'))
    await waitFor(() => expect(screen.getByTestId('update-install')).toBeInTheDocument())
    expect(screen.queryByTestId('update-error')).toBeNull()
    expect(screen.queryByTestId('update-progress')).toBeNull()
  })
})

describe('UpdateWindow: elevated row (.deb, .rpm)', () => {
  it('shows the password line and no checkbox', async () => {
    await open(info({}, { mode: 'selfInstallElevated', channel: 'deb' }))
    await new Promise((r) => setTimeout(r, 20))
    expect(screen.getByTestId('update-password-note')).toHaveTextContent(
      'Your computer will ask for your password to install it.',
    )
    expect(screen.queryByTestId('update-auto')).toBeNull()
    expect(screen.getByTestId('update-install')).toBeInTheDocument()
  })
})

describe('UpdateWindow: tell-only row (Flatpak, pacman, Gentoo, other)', () => {
  it('Flatpak: sentence, command with Copy, Update Help, never Install', async () => {
    await open(info({ autoSupported: false }, { mode: 'notifyOnly', channel: 'flatpak', command: 'flatpak update com.hew3d.Hew' }))
    expect(screen.getByTestId('update-message')).toHaveTextContent('Hew 1.6.0 is available — you have 1.5.0.')
    expect(screen.getByTestId('update-channel')).toHaveTextContent(
      'This copy of Hew was installed with Flatpak, so Flatpak updates it. To update, run this in a terminal:',
    )
    expect(screen.getByTestId('update-command')).toHaveTextContent('flatpak update com.hew3d.Hew')
    expect(screen.queryByTestId('update-install')).toBeNull()
    expect(screen.queryByTestId('update-auto')).toBeNull()
    fireEvent.click(screen.getByTestId('update-copy'))
    await waitFor(() => expect(m.calls).toContain('copy'))
    expect(await screen.findByText('Copied')).toBeInTheDocument()
    expect(screen.queryByTestId('update-copy-failed')).toBeNull()
    fireEvent.click(screen.getByTestId('update-help'))
    expect(m.open).toHaveBeenCalledWith('https://hew3d.com/download/update-help/')
  })

  it('a failed copy says so instead of doing nothing', async () => {
    m.copy = async () => {
      throw new Error('no clipboard')
    }
    await open(info({ autoSupported: false }, { mode: 'notifyOnly', channel: 'flatpak', command: 'flatpak update com.hew3d.Hew' }))
    fireEvent.click(screen.getByTestId('update-copy'))
    expect(await screen.findByTestId('update-copy-failed')).toHaveTextContent(
      'Couldn’t copy the command. Select it above and copy it, or click Update Help.',
    )
    expect(screen.getByTestId('update-copy')).toHaveTextContent('Copy Command')
    expect(screen.getByTestId('update-help')).toBeInTheDocument()
  })

  it.each([
    ['pacman', 'pacman updates it'],
    ['gentoo', 'Portage updates it'],
  ] as const)('%s names its package manager', async (channel, text) => {
    await open(info({}, { mode: 'notifyOnly', channel, command: 'x' }))
    expect(screen.getByTestId('update-channel')).toHaveTextContent(text)
  })

  it('other managed copies have no command and no Copy button', async () => {
    await open(info({}, { mode: 'notifyOnly', channel: 'otherManaged', command: null }))
    expect(screen.getByTestId('update-channel')).toHaveTextContent('This copy of Hew can’t update itself.')
    expect(screen.queryByTestId('update-copy')).toBeNull()
    expect(screen.getByTestId('update-help')).toBeInTheDocument()
  })
})

describe('UpdateWindow: read-only What’s New', () => {
  it('shows the notes and Close only', async () => {
    await open(info({ readOnly: true, phase: 'installed' }))
    expect(screen.getByTestId('update-heading')).toHaveTextContent('What’s new in Hew 1.6.0')
    expect(screen.getByTestId('update-notes')).toHaveTextContent('Lock: keep an object fixed.')
    for (const id of ['update-skip', 'update-remind', 'update-install', 'update-restart', 'update-auto', 'update-help']) {
      expect(screen.queryByTestId(id)).toBeNull()
    }
    fireEvent.click(screen.getByTestId('update-close'))
    await waitFor(() => expect(m.close).toHaveBeenCalled())
    expect(m.calls).toEqual([])
  })

  it('Escape closes without a reminder', async () => {
    await open(info({ readOnly: true }))
    fireEvent.keyDown(window, { key: 'Escape' })
    await waitFor(() => expect(m.close).toHaveBeenCalled())
    expect(m.calls).toEqual([])
  })
})

describe('UpdateWindow: a newer check replaces the offer', () => {
  it('refetches on update-changed', async () => {
    await open(info())
    m.info = info({ version: '1.7.0' })
    fire('update-changed')
    await waitFor(() => expect(screen.getByTestId('update-message')).toHaveTextContent('Hew 1.7.0 is now available'))
  })
})

// `update-changed` comes from a Check for Updates or What's New while the
// window is open, and when an automatic download ends.
describe('UpdateWindow: update-changed keeps what only the window knows', () => {
  const tick = () => act(async () => new Promise((r) => setTimeout(r, 10)))

  it('a restart closing the documents stays held: buttons disabled, Escape ignored', async () => {
    await open(info({ phase: 'installed' }))
    fireEvent.click(screen.getByTestId('update-restart'))
    expect(await screen.findByTestId('update-restart-pending')).toBeInTheDocument()
    fire('update-changed')
    await tick()
    expect(screen.getByTestId('update-restart-pending')).toHaveTextContent('Closing your documents…')
    expect(screen.getByTestId('update-restart')).toBeDisabled()
    expect(screen.getByTestId('update-later')).toBeDisabled()
    fireEvent.keyDown(window, { key: 'Escape' })
    await tick()
    expect(m.close).not.toHaveBeenCalled()
    expect(m.calls).toEqual(['restart'])
  })

  it('the cancelled line survives a refetch', async () => {
    m.restart = async () => ({ status: 'cancelled' })
    await open(info({ phase: 'installed' }))
    fireEvent.click(screen.getByTestId('update-restart'))
    expect(await screen.findByTestId('update-restart-cancelled')).toBeInTheDocument()
    fire('update-changed')
    await tick()
    expect(screen.getByTestId('update-restart-cancelled')).toBeInTheDocument()
  })

  it('download progress survives a refetch that still says downloading', async () => {
    m.install = () => new Promise(() => {})
    await open(info())
    fireEvent.click(screen.getByTestId('update-install'))
    fire('update-progress', { phase: 'downloading', version: '1.6.0', downloaded: 30, total: 40 })
    m.info = info({ phase: 'downloading' })
    fire('update-changed')
    await tick()
    expect(screen.getByRole('progressbar')).toHaveAttribute('value', '75')
  })

  it('opened during an automatic download: progress, then the restart step', async () => {
    await open(info({ phase: 'downloading' }))
    fire('update-progress', { phase: 'downloading', version: '1.6.0', downloaded: 10, total: 40 })
    expect(screen.getByRole('progressbar')).toHaveAttribute('value', '25')
    m.info = info({ phase: 'installed' })
    fire('update-changed')
    expect(await screen.findByTestId('update-restart')).toHaveTextContent('Restart Now')
    expect(screen.queryByTestId('update-progress')).toBeNull()
  })

  it('opened during an automatic download that fails: the failure step', async () => {
    await open(info({ phase: 'downloading' }))
    m.info = info({ phase: 'offer', error: 'network down' })
    fire('update-changed')
    expect(await screen.findByTestId('update-error')).toHaveTextContent('network down')
    expect(screen.getByTestId('update-download')).toBeInTheDocument()
    expect(screen.queryByTestId('update-progress')).toBeNull()
  })

  it('a phase that moved on still replaces the step', async () => {
    m.restart = () => new Promise(() => {})
    await open(info({ phase: 'ready' }, { channel: 'windowsInstaller', closesApp: true }))
    fireEvent.click(screen.getByTestId('update-restart'))
    // The installer could not start: the shell reopens on the failure.
    m.info = info({ phase: 'offer', error: 'installer missing' }, { channel: 'windowsInstaller', closesApp: true })
    fire('update-changed')
    expect(await screen.findByTestId('update-error')).toHaveTextContent('installer missing')
  })
})

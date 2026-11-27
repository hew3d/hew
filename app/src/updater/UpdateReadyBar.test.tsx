/**
 * UpdateReadyBar: the two wordings, the buttons, Later, the on-mount poll
 * and the build gate (docs/design D3).
 */
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { UpdateReadyBar } from './UpdateReadyBar'
import type { UpdateReady } from './updateIpc'

const m = vi.hoisted(() => ({
  available: true,
  polled: null as unknown,
  handlers: new Map<string, (p: unknown) => void>(),
  calls: [] as string[],
  restart: undefined as undefined | (() => Promise<unknown>),
}))

vi.mock('./updateIpc', () => ({
  updaterAvailable: async () => m.available,
  updateReadyGet: async () => m.polled,
  updateRestart: () => {
    m.calls.push('restart')
    return (m.restart ?? (async () => ({ status: 'cancelled' })))()
  },
  updateShowNotes: async () => void m.calls.push('notes'),
  onEvent: async (name: string, h: (p: unknown) => void) => {
    m.handlers.set(name, h)
    return () => m.handlers.delete(name)
  },
}))

const ready = (over: Partial<UpdateReady> = {}): UpdateReady => ({
  version: '1.6.0',
  currentVersion: '1.5.0',
  installed: true,
  closesApp: false,
  ...over,
})

beforeEach(() => {
  m.available = true
  m.polled = null
  m.handlers.clear()
  m.calls = []
  m.restart = undefined
})

describe('UpdateReadyBar', () => {
  it('renders nothing when no update is ready', async () => {
    render(<UpdateReadyBar />)
    await waitFor(() => expect(m.handlers.has('update-ready')).toBe(true))
    expect(screen.queryByTestId('update-ready-bar')).toBeNull()
  })

  it('installed (Mac, AppImage): opens-next-start wording and Restart Now', async () => {
    render(<UpdateReadyBar />)
    await waitFor(() => expect(m.handlers.has('update-ready')).toBe(true))
    act(() => m.handlers.get('update-ready')?.(ready()))
    expect(screen.getByTestId('update-ready-bar')).toHaveTextContent(
      'Hew 1.6.0 is installed and will open the next time you start Hew.',
    )
    expect(screen.getByTestId('update-ready-restart')).toHaveTextContent('Restart Now')
    fireEvent.click(screen.getByTestId('update-ready-restart'))
    fireEvent.click(screen.getByTestId('update-ready-notes'))
    await waitFor(() => expect(m.calls).toEqual(['restart', 'notes']))
    expect(screen.getByTestId('update-ready-bar')).toBeInTheDocument()
  })

  it('not installed (Windows): ready-to-install wording and Install and Restart', async () => {
    render(<UpdateReadyBar />)
    await waitFor(() => expect(m.handlers.has('update-ready')).toBe(true))
    act(() => m.handlers.get('update-ready')?.(ready({ installed: false, closesApp: true })))
    expect(screen.getByTestId('update-ready-bar')).toHaveTextContent('Hew 1.6.0 is ready to install. Hew will close and reopen.')
    expect(screen.getByTestId('update-ready-restart')).toHaveTextContent('Install and Restart')
    expect(screen.getByTestId('update-ready-notes')).toHaveTextContent('What’s New')
  })

  it('a window created after the event picks it up by polling', async () => {
    m.polled = ready({ version: '1.7.0' })
    render(<UpdateReadyBar />)
    expect(await screen.findByTestId('update-ready-bar')).toHaveTextContent('Hew 1.7.0 is installed')
  })

  it('Later hides it for this launch', async () => {
    m.polled = ready()
    render(<UpdateReadyBar />)
    fireEvent.click(await screen.findByTestId('update-ready-later'))
    expect(screen.queryByTestId('update-ready-bar')).toBeNull()
    expect(m.calls).toEqual([])
  })

  it('Restart Now waits disabled while the documents close, then comes back unchanged on Cancel', async () => {
    let answer: (r: unknown) => void = () => {}
    m.restart = () => new Promise((res) => (answer = res))
    m.polled = ready()
    render(<UpdateReadyBar />)
    const button = await screen.findByTestId('update-ready-restart')
    const text = screen.getByTestId('update-ready-bar').textContent
    fireEvent.click(button)
    await waitFor(() => expect(button).toBeDisabled())
    fireEvent.click(button)
    expect(m.calls).toEqual(['restart'])
    await act(async () => answer({ status: 'cancelled' }))
    expect(button).toBeEnabled()
    expect(screen.getByTestId('update-ready-bar').textContent).toBe(text)
    expect(button).toHaveTextContent('Restart Now')
  })

  it('Install and Restart is disabled the same way', async () => {
    let answer: (r: unknown) => void = () => {}
    m.restart = () => new Promise((res) => (answer = res))
    m.polled = ready({ installed: false, closesApp: true })
    render(<UpdateReadyBar />)
    const button = await screen.findByTestId('update-ready-restart')
    fireEvent.click(button)
    await waitFor(() => expect(button).toBeDisabled())
    await act(async () => answer({ status: 'cancelled' }))
    expect(button).toBeEnabled()
    expect(screen.getByTestId('update-ready-bar')).toHaveTextContent('Hew 1.6.0 is ready to install. Hew will close and reopen.')
  })

  it('does nothing in a build without an updater', async () => {
    m.available = false
    m.polled = ready()
    render(<UpdateReadyBar />)
    await new Promise((r) => setTimeout(r, 20))
    expect(screen.queryByTestId('update-ready-bar')).toBeNull()
    expect(m.handlers.size).toBe(0)
  })
})

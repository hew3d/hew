/**
 * AdvancedPane — the Settings ▸ Advanced ▸ Server pane on its shared state
 * machine (serverForm.ts). The Rust-backed store (`./server`) and the relay
 * client (`../io/relayClient`) are mocked so the draft/persist/test flow can
 * be driven without a shell: cloud persists at once, self-hosted waits for
 * a committed origin, validation errors show inline, Test connection
 * commits first and words the answer.
 */
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { AdvancedPane } from './AdvancedPane'
import { RelayError } from '../io/relayClient'

const store = vi.hoisted(() => ({
  available: true,
  setting: { mode: 'cloud' as 'cloud' | 'self-hosted', origin: 'https://app.hew3d.com', uploadKey: '' },
  set: vi.fn(),
  listeners: new Set<() => void>(),
}))
vi.mock('./server', async (importOriginal) => {
  const actual = await importOriginal<typeof import('./server')>()
  return {
    ...actual,
    serverSettingAvailable: () => store.available,
    getServerSetting: () => Promise.resolve(store.setting),
    setServerSetting: (next: unknown) => store.set(next),
    subscribe: (l: () => void) => {
      store.listeners.add(l)
      return () => store.listeners.delete(l)
    },
  }
})
const relay = vi.hoisted(() => ({ identity: vi.fn() }))
vi.mock('../io/relayClient', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../io/relayClient')>()
  return { ...actual, relayIdentity: () => relay.identity() }
})

describe('AdvancedPane', () => {
  beforeEach(() => {
    store.available = true
    store.setting = { mode: 'cloud', origin: 'https://app.hew3d.com', uploadKey: '' }
    store.set.mockReset()
    // Default: Rust accepts and echoes a canonical origin.
    store.set.mockImplementation(async (next: { mode: string; origin: string; uploadKey: string }) => ({
      ...next,
      origin: next.mode === 'self-hosted' ? next.origin.replace(/\/+$/, '').toLowerCase() : next.origin,
    }))
    store.listeners.clear()
    relay.identity.mockReset()
  })

  it('renders read-only in the browser build', () => {
    store.available = false
    render(<AdvancedPane />)
    expect(screen.getByTestId('settings-server-readonly')).toHaveTextContent(window.location.origin)
    expect(screen.queryByRole('radio')).not.toBeInTheDocument()
  })

  it('shows the persisted setting: cloud selected, address disabled and empty', async () => {
    render(<AdvancedPane />)
    await waitFor(() => expect(screen.getByRole('radio', { name: /hew cloud/i })).toBeChecked())
    const address = screen.getByLabelText<HTMLInputElement>(/address/i)
    expect(address).toBeDisabled()
    expect(address.value).toBe('')
    expect(screen.queryByLabelText(/upload key/i)).not.toBeInTheDocument()
  })

  it('switching to Self-hosted enables the fields but persists nothing until the address commits', async () => {
    render(<AdvancedPane />)
    await waitFor(() => expect(screen.getByRole('radio', { name: /hew cloud/i })).toBeChecked())
    fireEvent.click(screen.getByRole('radio', { name: /self-hosted/i }))
    expect(screen.getByLabelText(/address/i)).toBeEnabled()
    expect(screen.getByLabelText(/upload key/i)).toBeInTheDocument()
    expect(store.set).not.toHaveBeenCalled()

    const address = screen.getByLabelText<HTMLInputElement>(/address/i)
    fireEvent.change(address, { target: { value: 'HTTPS://Hew.Example.org/' } })
    fireEvent.keyDown(address, { key: 'Enter' })
    await waitFor(() => expect(store.set).toHaveBeenCalledTimes(1))
    expect(store.set).toHaveBeenCalledWith({ mode: 'self-hosted', origin: 'HTTPS://Hew.Example.org/', uploadKey: '' })
    // The field echoes Rust's canonical form.
    await waitFor(() => expect(address.value).toBe('https://hew.example.org'))
    expect(screen.queryByTestId('settings-server-error')).not.toBeInTheDocument()
  })

  it('shows Rust’s validation message inline and keeps the draft when the address is rejected', async () => {
    store.set.mockRejectedValue({ kind: 'invalidOrigin', message: 'the address needs a host name' })
    render(<AdvancedPane />)
    await waitFor(() => expect(screen.getByRole('radio', { name: /hew cloud/i })).toBeChecked())
    fireEvent.click(screen.getByRole('radio', { name: /self-hosted/i }))
    const address = screen.getByLabelText<HTMLInputElement>(/address/i)
    fireEvent.change(address, { target: { value: 'https://' } })
    fireEvent.blur(address)
    expect(await screen.findByTestId('settings-server-error')).toHaveTextContent(/needs a host name/)
    expect(address.value).toBe('https://')
    expect(screen.getByRole('radio', { name: /self-hosted/i })).toBeChecked()
  })

  it('switching back to Hew cloud persists immediately', async () => {
    store.setting = { mode: 'self-hosted', origin: 'https://hew.example.org', uploadKey: 'k' }
    render(<AdvancedPane />)
    await waitFor(() => expect(screen.getByRole('radio', { name: /self-hosted/i })).toBeChecked())
    fireEvent.click(screen.getByRole('radio', { name: /hew cloud/i }))
    await waitFor(() => expect(store.set).toHaveBeenCalledWith({ mode: 'cloud', origin: 'https://hew.example.org', uploadKey: 'k' }))
  })

  it('Test connection commits the draft, then reports the identity', async () => {
    store.setting = { mode: 'self-hosted', origin: 'https://hew.example.org', uploadKey: '' }
    relay.identity.mockResolvedValue({
      origin: 'https://hew.example.org',
      service: 'hew-relay',
      contract: 1,
      maxBytes: 32 * 1024 * 1024,
      ttlMs: 600_000,
      auth: 'none',
    })
    render(<AdvancedPane />)
    await waitFor(() => expect(screen.getByRole('radio', { name: /self-hosted/i })).toBeChecked())
    fireEvent.click(screen.getByRole('button', { name: /test connection/i }))
    const ok = await screen.findByTestId('settings-server-test-ok')
    expect(ok).toHaveTextContent(/hew\.example\.org/)
    expect(ok).toHaveTextContent(/32 MB max/)
    expect(ok).toHaveTextContent(/10 min TTL/)
    expect(ok).toHaveTextContent(/open uploads/)
    expect(store.set).toHaveBeenCalledTimes(1)
    expect(relay.identity).toHaveBeenCalledTimes(1)
  })

  it('Test connection words a rejected key, a missing required key, and each failure kind', async () => {
    store.setting = { mode: 'self-hosted', origin: 'https://hew.example.org', uploadKey: 'wrong' }
    const base = { origin: 'https://hew.example.org', service: 'hew-relay', contract: 1, maxBytes: 1, ttlMs: 60_000 }
    relay.identity.mockResolvedValueOnce({ ...base, auth: 'bearer', keyAccepted: false })
    const { rerender } = render(<AdvancedPane />)
    await waitFor(() => expect(screen.getByRole('radio', { name: /self-hosted/i })).toBeChecked())
    fireEvent.click(screen.getByRole('button', { name: /test connection/i }))
    expect(await screen.findByTestId('settings-server-test-fail')).toHaveTextContent(/rejected the upload key/i)

    // Server wants a key, none configured.
    fireEvent.change(screen.getByLabelText(/upload key/i), { target: { value: '' } })
    relay.identity.mockResolvedValueOnce({ ...base, auth: 'bearer' })
    fireEvent.click(screen.getByRole('button', { name: /test connection/i }))
    expect(await screen.findByTestId('settings-server-test-fail')).toHaveTextContent(/requires an upload key/i)

    for (const [kind, pattern] of [
      ['unreachable', /could not reach hew\.example\.org/i],
      ['tls', /certificate isn't trusted/i],
      ['notARelay', /isn't serving a hew relay/i],
      ['status', /status 502/i],
    ] as const) {
      relay.identity.mockRejectedValueOnce(new RelayError(kind, kind, kind === 'status' ? 502 : undefined))
      rerender(<AdvancedPane />)
      fireEvent.click(screen.getByRole('button', { name: /test connection/i }))
      await waitFor(() => expect(screen.getByTestId('settings-server-test-fail')).toHaveTextContent(pattern))
    }
  })
})

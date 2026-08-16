/**
 * io/relayClient.ts — the typed wrappers over the Rust relay commands: what
 * they invoke (and with what), and how a Rust `RelayError` (a plain object
 * on the wire) or an IPC-level failure surfaces to callers.
 */
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { RelayError, relayDelete, relayIdentity, relayPeek, relayPut, toRelayError } from './relayClient'

const mockInvoke = vi.hoisted(() => vi.fn())
vi.mock('@tauri-apps/api/core', () => ({ invoke: mockInvoke }))

describe('toRelayError', () => {
  it('passes a RelayError through', () => {
    const e = new RelayError('full', 'x', 503)
    expect(toRelayError(e)).toBe(e)
  })

  it('lifts the serialized Rust error object into a RelayError with kind/message/status', () => {
    const e = toRelayError({ kind: 'unauthorized', message: 'the server rejected the upload key', status: 401 })
    expect(e).toBeInstanceOf(RelayError)
    expect(e.kind).toBe('unauthorized')
    expect(e.message).toBe('the server rejected the upload key')
    expect(e.status).toBe(401)
    expect(toRelayError({ kind: 'tls', message: 'no' }).status).toBeUndefined()
  })

  it('treats an unknown kind, a bare string, or an Error as unreachable', () => {
    expect(toRelayError({ kind: 'weird', message: 'm' }).kind).toBe('unreachable')
    expect(toRelayError('boom').kind).toBe('unreachable')
    expect(toRelayError('boom').message).toBe('boom')
    expect(toRelayError(new Error('ipc down')).message).toBe('ipc down')
  })
})

describe('command wrappers', () => {
  // `mockClear`, not `mockReset` (and `mockImplementation(async …)`, not
  // `mockResolvedValue`, plain awaits, not `.resolves`): each of those
  // Vitest 4 conveniences, used in an EARLIER test of this file, makes the
  // rejection test below report an "Unknown Error" for the plain-object
  // rejection (the wire shape of a Rust RelayError) that the wrapper
  // demonstrably catches. Every test sets its own implementation anyway.
  beforeEach(() => {
    mockInvoke.mockClear()
  })

  it('relayPut sends the bytes as the RAW invoke body (not a JSON array) and returns the token', async () => {
    mockInvoke.mockImplementation(async () => ({ token: 'a'.repeat(22) }))
    const bytes = new Uint8Array([1, 2, 3])
    expect(await relayPut(bytes)).toEqual({ token: 'a'.repeat(22) })
    expect(mockInvoke).toHaveBeenCalledWith('relay_put', bytes)
    expect(mockInvoke.mock.calls[0][1]).toBeInstanceOf(Uint8Array)
  })

  it('relayPeek / relayDelete pass the token by name; relayIdentity takes nothing', async () => {
    mockInvoke.mockImplementation(async () => 'present')
    expect(await relayPeek('t')).toBe('present')
    expect(mockInvoke).toHaveBeenCalledWith('relay_peek', { token: 't' })
    mockInvoke.mockImplementation(async () => undefined)
    await relayDelete('t')
    expect(mockInvoke).toHaveBeenCalledWith('relay_delete', { token: 't' })
    mockInvoke.mockImplementation(async () => ({ service: 'hew-relay' }))
    await relayIdentity()
    expect(mockInvoke).toHaveBeenCalledWith('relay_identity', undefined)
  })

  it('a rejected invoke surfaces as a RelayError', async () => {
    // A fresh rejection per call (not `mockRejectedValue`, whose single
    // eagerly-created rejected promise can trip the unhandled-rejection
    // detector before the wrapper gets to catch it).
    mockInvoke.mockImplementation(() => Promise.reject({ kind: 'notARelay', message: 'reachable, but not a Hew relay' }))
    let caught: unknown = null
    try {
      await relayIdentity()
    } catch (err) {
      caught = err
    }
    expect(caught).toBeInstanceOf(RelayError)
    expect((caught as RelayError).kind).toBe('notARelay')
    expect((caught as RelayError).message).toBe('reachable, but not a Hew relay')
  })
})

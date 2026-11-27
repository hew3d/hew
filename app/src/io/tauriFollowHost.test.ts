// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from 'vitest'

const mockInvoke = vi.hoisted(() => vi.fn())
const mockListen = vi.hoisted(() => vi.fn())
vi.mock('@tauri-apps/api/core', () => ({ invoke: mockInvoke }))
vi.mock('@tauri-apps/api/event', () => ({ listen: mockListen }))

import { createTauriFollowHost } from './tauriFollowHost'

beforeEach(() => {
  mockInvoke.mockReset()
  mockListen.mockReset()
})

describe('tauriFollowHost', () => {
  it('maps each method to its follow_* command', async () => {
    const h = createTauriFollowHost()
    mockInvoke.mockResolvedValue([])
    await h.list()
    await h.check('launch')
    await h.check('manual', 'HEW-A')
    await h.post('HEW-A', 'hi')
    await h.markSeen('HEW-A', 3)
    await h.forget('HEW-A')
    expect(mockInvoke.mock.calls.map((c) => [c[0], c[1]])).toEqual([
      ['follow_list', undefined],
      ['follow_check', { reason: 'launch' }],
      ['follow_check', { id: 'HEW-A', reason: 'manual' }],
      ['follow_post', { id: 'HEW-A', text: 'hi' }],
      ['follow_seen', { id: 'HEW-A', upTo: 3 }],
      ['follow_forget', { id: 'HEW-A' }],
    ])
  })
  it('attach sends raw bytes with the hew-follow-id header and returns following:false', async () => {
    const bytes = new Uint8Array([1, 2])
    mockInvoke.mockResolvedValue({ id: 'HEW-NEW', following: false })
    const r = await createTauriFollowHost().attach('HEW-A', bytes)
    expect(r).toEqual({ id: 'HEW-NEW', following: false })
    expect(mockInvoke).toHaveBeenCalledWith('follow_attach', bytes, { headers: { 'hew-follow-id': 'HEW-A' } })
  })
  it('maps rejections to ReportError, keeping 403 and retryAfter', async () => {
    const h = createTauriFollowHost()
    mockInvoke.mockRejectedValueOnce({ kind: 'status', message: 'x', status: 403 })
    await expect(h.post('a', 'b')).rejects.toMatchObject({ kind: 'status', status: 403 })
    mockInvoke.mockRejectedValueOnce({ kind: 'rateLimited', message: 'x', status: 429, retryAfter: 12 })
    await expect(h.post('a', 'b')).rejects.toMatchObject({ kind: 'rateLimited', retryAfter: 12 })
  })
  it('subscribes to follow-changed and unlistens with the last subscriber', async () => {
    const unlisten = vi.fn()
    let handler: () => void = () => {}
    mockListen.mockImplementation(async (_n: string, cb: () => void) => {
      handler = cb
      return unlisten
    })
    const h = createTauriFollowHost()
    const cb = vi.fn()
    const off = h.subscribe(cb)
    await vi.waitFor(() => expect(mockListen).toHaveBeenCalledWith('follow-changed', expect.any(Function)))
    handler()
    expect(cb).toHaveBeenCalledTimes(1)
    await Promise.resolve()
    off()
    expect(unlisten).toHaveBeenCalled()
  })
})

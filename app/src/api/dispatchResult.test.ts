import { afterEach, describe, expect, it, vi } from 'vitest'
import { parseDispatch } from './dispatchResult'

describe('parseDispatch', () => {
  afterEach(() => {
    vi.restoreAllMocks()
  })

  it('reads a dropped notification as none', () => {
    expect(parseDispatch(undefined)).toEqual({ kind: 'none' })
  })

  it('reads a tagged reply as its frame', () => {
    const frame = '{"jsonrpc":"2.0","id":1,"result":{}}'
    expect(parseDispatch(JSON.stringify({ reply: frame }))).toEqual({ kind: 'reply', frame })
  })

  it('reads a dry run reply as a reply marked dryRun', () => {
    const frame = '{"jsonrpc":"2.0","id":1,"result":{"results":[],"label":"x","dry_run":true}}'
    expect(parseDispatch(JSON.stringify({ reply: frame, dry_run: true }))).toEqual({
      kind: 'reply',
      frame,
      dryRun: true,
    })
  })

  it('reads a pending forward with its token and request', () => {
    const forward = {
      token: 3,
      plugin: 'com.example.lib',
      local: 'sum',
      method: 'com.example.lib.sum',
      request_id: 7,
      params: { a: 1 },
      mutating: false,
      label: null,
      timeout_ms: 50000,
      concurrency: 'parallel',
      caller: { identity: 'live:0', plugin: null, app: null, forward: null },
    }
    expect(parseDispatch(JSON.stringify({ pending: { token: 3, forward } }))).toEqual({
      kind: 'pending',
      token: 3,
      forward,
    })
  })

  it('logs and drops anything else', () => {
    const error = vi.spyOn(console, 'error').mockImplementation(() => {})
    for (const raw of ['not json', '{"jsonrpc":"2.0","id":1,"result":{}}', '{"pending":{"token":"x"}}', 'null']) {
      expect(parseDispatch(raw)).toEqual({ kind: 'none' })
    }
    expect(error).toHaveBeenCalledTimes(4)
  })
})

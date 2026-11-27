/**
 * sourceLock.test - the per-source job lock (design 7.3): one job per
 * source across tabs, never two at once, never queued behind a stranger.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { fakeLocks } from '../test/locksFake'
import { withSourceLock } from './sourceLock'

let locks: ReturnType<typeof fakeLocks>

beforeEach(() => {
  locks = fakeLocks()
  vi.stubGlobal('navigator', { locks })
})
afterEach(() => vi.unstubAllGlobals())

/** A promise whose resolution the test controls. */
function gate() {
  let open!: () => void
  const promise = new Promise<void>((r) => (open = r))
  return { promise, open }
}

describe('withSourceLock', () => {
  it('runs the job and returns its value', async () => {
    expect(await withSourceLock('github:a/a', async () => 42)).toEqual({ ran: true, value: 42 })
  })

  it('a second caller on the same source does not run while the first holds the lock', async () => {
    const g = gate()
    const first = withSourceLock('github:a/a', async () => {
      await g.promise
      return 'first'
    })
    await Promise.resolve()
    const second = vi.fn(async () => 'second')
    expect(await withSourceLock('github:a/a', second)).toEqual({ ran: false })
    expect(second).not.toHaveBeenCalled()
    g.open()
    expect(await first).toEqual({ ran: true, value: 'first' })
  })

  it('the lock is free again once the holder finishes', async () => {
    await withSourceLock('github:a/a', async () => 1)
    expect(await withSourceLock('github:a/a', async () => 2)).toEqual({ ran: true, value: 2 })
  })

  it('a job that throws releases the lock and rejects to its caller', async () => {
    await expect(
      withSourceLock('github:a/a', async () => {
        throw new Error('boom')
      }),
    ).rejects.toThrow('boom')
    expect(await withSourceLock('github:a/a', async () => 'after')).toEqual({ ran: true, value: 'after' })
  })

  it('different sources run at the same time', async () => {
    const g = gate()
    const a = withSourceLock('github:a/a', async () => {
      await g.promise
      return 'a'
    })
    await Promise.resolve()
    expect(await withSourceLock('github:b/b', async () => 'b')).toEqual({ ran: true, value: 'b' })
    g.open()
    expect(await a).toEqual({ ran: true, value: 'a' })
  })

  it('holds a Web Lock whose name carries the source reference, and holds nothing afterward', async () => {
    const g = gate()
    const job = withSourceLock('github:a/a', async () => g.promise)
    await Promise.resolve()
    const during = (await locks.query()).held.map((h) => h.name)
    expect(during).toHaveLength(1)
    expect(during[0]).toContain('github:a/a')
    expect(during[0]).not.toMatch(/^hew-library-edit:/)
    g.open()
    await job
    expect((await locks.query()).held).toEqual([])
  })
})

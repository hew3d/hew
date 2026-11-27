/* oxlint-disable lingui/no-unlocalized-strings -- test fixtures */
/** The stored lastCheck keeps the HTTP status of an http_status failure, so
 * the persistent banner says the same thing the fresh error did (6.5). */
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import type { CheckFailed } from './sources'
import { failureMessage, sourceErrorMessage } from './sourceMessages'
import { SRC, makeEnv, parts, type Env } from './sourcesTestKit'

let env: Env
beforeEach(() => {
  env = makeEnv()
})
afterEach(() => env.assertClean())

describe('lastCheck status', () => {
  it('a 503 is stored, and the reloaded entry words the error as the fresh failure did', async () => {
    await env.addSource(SRC, { name: 'Shop', items: parts(1) })
    env.clock.advance(3600_000)
    env.server.fail((u) => u.includes('/hew.json'), 'http_status', { status: 503 })
    const r = (await env.sources.check(SRC)) as CheckFailed
    expect(r).toMatchObject({ ok: false, code: 'http_status', status: 503 })
    const fresh = failureMessage(r, { host: 'example.test', web: false }).text
    const [status] = await env.sources.listSources()
    expect(status.entry.lastCheck).toMatchObject({ ok: false, code: 'http_status', status: 503 })
    const stored = status.entry.lastCheck!
    const reloaded = sourceErrorMessage(stored.code as 'http_status', { host: 'example.test', web: false, ...(stored.status !== undefined ? { status: stored.status } : {}) })
    expect(reloaded).toBe(fresh)
    expect(reloaded).toContain('503')
  })

  it('a failure that is not an http_status stores no status', async () => {
    await env.addSource(SRC, { name: 'Shop', items: parts(1) })
    env.server.fail((u) => u.includes('/hew.json'), 'network')
    await env.sources.check(SRC)
    expect(env.store.entryFor(SRC)?.lastCheck).not.toHaveProperty('status')
  })

  it('an entry stored before status existed still loads', async () => {
    await env.addSource(SRC, { name: 'Shop', items: parts(1) })
    const at = env.clock.iso()
    await env.store.writeSources(SRC, { ...env.store.entryFor(SRC)!, lastCheck: { at, ok: false, code: 'http_status' } })
    const [status] = await env.sources.listSources()
    expect(status.entry.lastCheck).toEqual({ at, ok: false, code: 'http_status' })
    expect(status.unreachable).toBe(true)
  })
})

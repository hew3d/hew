/**
 * A Forgejo source resolves like GitHub and GitLab: a resolve answering 404
 * is an error (`http_status`), and nothing is read at the ref without a
 * commit. Only a rate-limited resolve falls back to an unpinned read.
 */
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { fakeParseSourceRef, fakeSourceUrls, makeEnv, parts, type Env } from './sourcesTestKit'
import type { CheckFailed } from './sources'

let env: Env
beforeEach(() => {
  env = makeEnv()
})
afterEach(() => {
  env.assertClean()
})

const resolveUrl = (canonical: string): string => {
  const refs = fakeParseSourceRef(canonical)
  if ('error' in refs) throw new Error('bad ref')
  return fakeSourceUrls(refs[0], []).resolve!
}

describe('a Forgejo resolve answering 404', () => {
  for (const src of ['forgejo:codeberg.org/acme/hinges@v1', 'forgejo:gitea.com/acme/hinges@v1', 'forgejo:codeberg.org/acme/hinges']) {
    it(`${src} fails and never reads hew.json`, async () => {
      await env.addSource(src, { name: 'Hinges', items: parts(1) })
      env.server.files.delete(resolveUrl(src))
      env.fetcher.reset()
      const r = await env.sources.check(src)
      expect((r as CheckFailed).ok).toBe(false)
      expect((r as CheckFailed).code).toBe('http_status')
      expect(env.fetcher.urls().some((u) => u.endsWith('/hew.json'))).toBe(false)
    })
  }
})

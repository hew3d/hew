/* oxlint-disable lingui/no-unlocalized-strings -- test fixtures */
import { afterEach, describe, expect, it, vi } from 'vitest'
import { failureMessage, sourceErrorMessage, sourcesOffText } from './sourceMessages'
import type { Failure, SourceErrorCode } from './sources'

const ALL_CODES: SourceErrorCode[] = [
  'not_https', 'too_large', 'timeout', 'too_many_redirects', 'rate_limited', 'http_status', 'network', 'offline',
  'ref_invalid', 'index_invalid', 'index_too_new', 'index_too_large', 'no_source', 'unknown_source', 'item_not_found',
  'not_checked', 'busy', 'source_removed', 'hash_mismatch', 'listing_incorrect', 'file_invalid', 'name_clash',
  'unavailable_other_source', 'needs_confirm', 'already_in_library', 'not_updatable', 'not_linked', 'not_found',
  'open_for_editing', 'not_enough_space', 'write_failed', 'catalog_disabled', 'catalog_invalid',
]

// Words a SketchUp user should never see (design 7.5).
const FORBIDDEN = /repositor|\bindex\b|\bfetch|download/i

describe('sourceErrorMessage (design 6.6)', () => {
  afterEach(() => vi.unstubAllGlobals())

  it('gives every code its own non-empty sentence in plain words', () => {
    const seen = new Set<string>()
    for (const code of ALL_CODES) {
      const text = sourceErrorMessage(code, { host: 'example.org', status: 500, reason: 'because', needed: 10, free: 5, otherSource: 'Other' })
      expect(text, code).not.toBe('')
      expect(text, code).not.toMatch(FORBIDDEN)
      // 'busy' etc. are words too; only the underscored codes can leak by name.
      if (code.includes('_')) expect(text, `${code} must not leak the code`).not.toContain(code)
      seen.add(text)
    }
    // Several codes share a sentence on purpose (6.6 groups them); the rest are distinct.
    expect(seen.size).toBeGreaterThanOrEqual(24)
  })

  it('network: the web build names both causes and the desktop app', () => {
    expect(sourceErrorMessage('network', { web: true })).toBe(
      'Hew could not read this site from the browser. The site may be down, or it may not allow web apps to read it. The desktop app can read it either way.',
    )
  })

  it('network on the desktop, and timeout, name the host', () => {
    expect(sourceErrorMessage('network', { host: 'example.org' })).toBe('Hew could not reach example.org. Check your connection and try again.')
    expect(sourceErrorMessage('timeout', { host: 'example.org', web: true })).toBe('Hew could not reach example.org. Check your connection and try again.')
    expect(sourceErrorMessage('timeout')).toBe('Hew could not reach this site. Check your connection and try again.')
  })

  it('an offline browser says the Library still works', () => {
    expect(sourceErrorMessage('network', { offline: true, web: true })).toBe('You are offline. Your Library still works.')
    expect(sourceErrorMessage('offline')).toBe('You are offline. Your Library still works.')
  })

  it('the fixed sentences of 6.6', () => {
    expect(sourceErrorMessage('not_https')).toBe('Sources must use https.')
    expect(sourceErrorMessage('rate_limited', { host: 'api.example.org' })).toBe('api.example.org is limiting requests right now. Try again later.')
    expect(sourceErrorMessage('index_too_new')).toBe('This source needs a newer version of Hew.')
    for (const c of ['index_invalid', 'index_too_large', 'ref_invalid'] as const) {
      expect(sourceErrorMessage(c)).toBe('This address does not have a Hew source.')
    }
    expect(sourceErrorMessage('hash_mismatch')).toBe('This item changed while Hew was reading it. Try again in a few minutes.')
    expect(sourceErrorMessage('listing_incorrect')).toBe('The source lists this item incorrectly. Hew did not add it.')
    expect(sourceErrorMessage('name_clash')).toBe('An item with the same file name is already in your Library.')
    expect(sourceErrorMessage('open_for_editing')).toBe('Open for editing. Close it and update again.')
  })

  it('not enough web storage shows both sizes', () => {
    expect(sourceErrorMessage('not_enough_space', { needed: 5 * 1024 * 1024, free: 2 * 1024 * 1024 })).toBe(
      'This source needs 5.0 MB. Your browser has 2.0 MB free.',
    )
  })

  it('a removed source carries the catalog reason', () => {
    expect(sourceErrorMessage('source_removed', { reason: 'It republished other people’s work.' })).toBe(
      'The Hew catalog removed this source: It republished other people’s work.',
    )
  })

  it('no candidate parsed: a dead address and a wrong address read differently', () => {
    expect(sourceErrorMessage('no_source', { causes: ['index_invalid', 'index_invalid', 'network'] })).toBe('This address does not have a Hew source.')
    expect(sourceErrorMessage('no_source', { causes: ['network', 'network', 'network'], web: true })).toContain('from the browser')
    expect(sourceErrorMessage('no_source', { causes: ['network', 'index_too_new'] })).toBe('This source needs a newer version of Hew.')
  })

  it('an HTTP 404 means there is no source there; other statuses say the site answered with an error', () => {
    expect(sourceErrorMessage('http_status', { status: 404, host: 'example.org' })).toBe('This address does not have a Hew source.')
    expect(sourceErrorMessage('http_status', { status: 503, host: 'example.org' })).toBe('example.org answered with an error (503). Try again later.')
  })
})

describe('failureMessage', () => {
  afterEach(() => vi.unstubAllGlobals())

  it('turns a failure into text and keeps only an https catalog link', () => {
    const f: Failure = { ok: false, code: 'source_removed', reason: 'Gone.', link: 'https://example.org/why' }
    expect(failureMessage(f, { web: true })).toEqual({ text: 'The Hew catalog removed this source: Gone.', link: 'https://example.org/why' })
    expect(failureMessage({ ...f, link: 'javascript:alert(1)' }, { web: true }).link).toBeUndefined()
    expect(failureMessage({ ...f, link: 'http://example.org/why' }, { web: true }).link).toBeUndefined()
    expect(failureMessage({ ...f, link: 'not a url' }, { web: true }).link).toBeUndefined()
  })

  it('reads the browser offline state for a network failure', () => {
    vi.stubGlobal('navigator', { onLine: false })
    expect(failureMessage({ ok: false, code: 'network' }, { web: true }).text).toBe('You are offline. Your Library still works.')
  })

  it('passes the status, sizes and other source through', () => {
    vi.stubGlobal('navigator', { onLine: true })
    expect(failureMessage({ ok: false, code: 'http_status', status: 500 }, { web: false, host: 'h.example' }).text).toContain('(500)')
    expect(failureMessage({ ok: false, code: 'unavailable_other_source', otherSource: 'Slides' }, { web: false }).text).toBe(
      'This item is already in your Library from Slides.',
    )
  })
})

describe('sourcesOffText', () => {
  it('is the one line the Off state shows', () => {
    expect(sourcesOffText()).toBe('Sources are turned off on this server.')
  })
})

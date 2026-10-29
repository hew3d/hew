import { describe, it, expect } from 'vitest'
import { i18n } from '@lingui/core'
import { formatRecoveryTime, normalizeRecoveryMeta, shouldPromptRecovery } from './recoveryStore'
import type { RecoveryListing, RecoveryMeta } from './recoveryStore'
import { INITIAL_SESSION } from './documentSession'
import type { DocSessionState } from './documentSession'
import type { FileRef } from './fileHost'

const mockRef = (name: string): FileRef => ({ name, format: 'hew', handle: null })

const mockMeta = (savedAt: number): RecoveryMeta => ({
  version: 1,
  savedAt,
  name: 'Untitled',
  path: null,
  format: 'hew',
})

const mockListing = (slot = 'main', savedAt = 0): RecoveryListing => ({
  slot,
  meta: mockMeta(savedAt),
})

describe('formatRecoveryTime', () => {
  it('shows "just now" for under 45 seconds', () => {
    expect(formatRecoveryTime(1000, 1000)).toBe('just now')
    expect(formatRecoveryTime(1000, 1000 + 44_000)).toBe('just now')
  })

  it('shows minutes for 45s up to 60 minutes', () => {
    expect(formatRecoveryTime(0, 60_000)).toBe('1 minute ago')
    expect(formatRecoveryTime(0, 2 * 60_000)).toBe('2 minutes ago')
    expect(formatRecoveryTime(0, 59 * 60_000)).toBe('59 minutes ago')
  })

  it('is right at the 45s "just now" boundary', () => {
    expect(formatRecoveryTime(0, 44_000)).toBe('just now')
    expect(formatRecoveryTime(0, 45_000)).toBe('1 minute ago')
  })

  it('shows hours for 60 minutes up to 24 hours', () => {
    expect(formatRecoveryTime(0, 60 * 60_000)).toBe('1 hour ago')
    expect(formatRecoveryTime(0, 2 * 60 * 60_000)).toBe('2 hours ago')
    expect(formatRecoveryTime(0, 23 * 60 * 60_000)).toBe('23 hours ago')
  })

  it('falls back to a date string beyond ~24h, in the app locale rather than the system one', () => {
    const savedAt = 0
    const now = 25 * 60 * 60_000
    // test/setup.ts activates `en`, whatever the machine's LANG.
    expect(formatRecoveryTime(savedAt, now)).toBe(new Date(savedAt).toLocaleString('en'))
    try {
      i18n.load('de', {})
      i18n.activate('de')
      expect(formatRecoveryTime(savedAt, now)).toBe(new Date(savedAt).toLocaleString('de'))
    } finally {
      i18n.activate('en')
    }
  })

  it('treats negative deltas (clock skew) as "just now" rather than throwing', () => {
    expect(formatRecoveryTime(2000, 1000)).toBe('just now')
  })
})

describe('shouldPromptRecovery', () => {
  it('returns false when there are no snapshots', () => {
    expect(shouldPromptRecovery(INITIAL_SESSION, [])).toBe(false)
  })

  it('returns true when a snapshot exists and the session is clean with no ref', () => {
    expect(shouldPromptRecovery(INITIAL_SESSION, [mockListing()])).toBe(true)
  })

  it('returns true with several snapshots (multi-window crash)', () => {
    expect(
      shouldPromptRecovery(INITIAL_SESSION, [mockListing('main', 5), mockListing('main-2', 3)]),
    ).toBe(true)
  })

  it('returns false when the session already has a currentRef (e.g. cold-start open)', () => {
    const session: DocSessionState = { currentRef: mockRef('opened.hew'), dirty: false, lastEditAt: null, lastSavedAt: null, nonUndoableReasons: new Set() }
    expect(shouldPromptRecovery(session, [mockListing()])).toBe(false)
  })

  it('returns false when the session is already dirty', () => {
    const session: DocSessionState = { currentRef: null, dirty: true, lastEditAt: null, lastSavedAt: null, nonUndoableReasons: new Set() }
    expect(shouldPromptRecovery(session, [mockListing()])).toBe(false)
  })

  it('returns false when both currentRef is set and dirty is true', () => {
    const session: DocSessionState = { currentRef: mockRef('opened.hew'), dirty: true, lastEditAt: null, lastSavedAt: null, nonUndoableReasons: new Set() }
    expect(shouldPromptRecovery(session, [mockListing()])).toBe(false)
  })
})

describe('normalizeRecoveryMeta', () => {
  it('defaults a record written before the format field to hew', () => {
    const old = { version: 1 as const, savedAt: 5, name: 'house.hew', path: '/tmp/house.hew' }
    expect(normalizeRecoveryMeta(old).format).toBe('hew')
  })

  it('keeps a SketchUp format', () => {
    const meta = { version: 1 as const, savedAt: 5, name: 'a.skp', path: null, format: 'skp2017' }
    expect(normalizeRecoveryMeta(meta).format).toBe('skp2017')
    expect(normalizeRecoveryMeta({ ...meta, format: 'skp2026' }).format).toBe('skp2026')
  })

  it('falls back to hew for an unrecognized format value', () => {
    const meta = { version: 1 as const, savedAt: 5, name: 'a', path: null, format: 'skp2030' }
    expect(normalizeRecoveryMeta(meta).format).toBe('hew')
  })

  it('preserves every other field', () => {
    const meta = { version: 1 as const, savedAt: 42, name: 'x.skp', path: '/p/x.skp', format: 'skp2026' }
    expect(normalizeRecoveryMeta(meta)).toEqual(meta)
  })
})

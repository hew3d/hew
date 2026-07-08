import { describe, it, expect } from 'vitest'
import { formatRecoveryTime, shouldPromptRecovery } from './recoveryStore'
import type { RecoveryListing, RecoveryMeta } from './recoveryStore'
import { INITIAL_SESSION } from './documentSession'
import type { DocSessionState } from './documentSession'
import type { FileRef } from './fileHost'

const mockRef = (name: string): FileRef => ({ name, handle: null })

const mockMeta = (savedAt: number): RecoveryMeta => ({
  version: 1,
  savedAt,
  name: 'Untitled',
  path: null,
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

  it('falls back to a date string beyond ~24h', () => {
    const savedAt = 0
    const now = 25 * 60 * 60_000
    const result = formatRecoveryTime(savedAt, now)
    expect(result).toBe(new Date(savedAt).toLocaleString())
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
    const session: DocSessionState = { currentRef: mockRef('opened.hew'), dirty: false, lastEditAt: null, lastSavedAt: null }
    expect(shouldPromptRecovery(session, [mockListing()])).toBe(false)
  })

  it('returns false when the session is already dirty', () => {
    const session: DocSessionState = { currentRef: null, dirty: true, lastEditAt: null, lastSavedAt: null }
    expect(shouldPromptRecovery(session, [mockListing()])).toBe(false)
  })

  it('returns false when both currentRef is set and dirty is true', () => {
    const session: DocSessionState = { currentRef: mockRef('opened.hew'), dirty: true, lastEditAt: null, lastSavedAt: null }
    expect(shouldPromptRecovery(session, [mockListing()])).toBe(false)
  })
})

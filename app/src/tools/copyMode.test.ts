import { describe, expect, it } from 'vitest'
import { copyBadgeText, linkedFor, nextCopyState, type CopyState } from './copyMode'

describe('copy state cycle', () => {
  it('the_cycle_is_off_primary_alternate_off', () => {
    let s: CopyState = 'off'
    const seen: CopyState[] = []
    for (let i = 0; i < 4; i += 1) {
      s = nextCopyState(s)
      seen.push(s)
    }
    expect(seen).toEqual(['primary', 'alternate', 'off', 'primary'])
  })

  it('linkedFor_follows_the_preference', () => {
    expect(linkedFor('off', true)).toBe(false)
    expect(linkedFor('off', false)).toBe(false)
    expect(linkedFor('primary', true)).toBe(true)
    expect(linkedFor('alternate', true)).toBe(false)
    expect(linkedFor('primary', false)).toBe(false)
    expect(linkedFor('alternate', false)).toBe(true)
  })

  it('copyBadgeText_reads_plus_and_the_other_mode', () => {
    expect(copyBadgeText('off', true)).toBe('')
    expect(copyBadgeText('primary', true)).toBe('+')
    expect(copyBadgeText('alternate', true)).toBe('+ independent')
    expect(copyBadgeText('primary', false)).toBe('+')
    expect(copyBadgeText('alternate', false)).toBe('+ linked')
  })
})

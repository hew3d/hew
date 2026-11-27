import { describe, expect, it } from 'vitest'
import { LOOK_ALIKE } from './healthCopy'
import { BANNED_RE } from './bannedWords'

describe('look-alike notice copy', () => {
  it('the_notice_row_offers_link_identical_copies', () => {
    expect(LOOK_ALIKE.notice(4)).toBe('4 look-alike objects can be linked')
    expect(LOOK_ALIKE.notice(1)).toBe('1 look-alike object can be linked')
    expect(LOOK_ALIKE.action).toBe('Link identical copies')
    for (const s of [LOOK_ALIKE.notice(4), LOOK_ALIKE.action, LOOK_ALIKE.actionTitle]) {
      expect(s).not.toMatch(BANNED_RE)
      expect(s).not.toMatch(/definition|instance/i)
    }
  })
})

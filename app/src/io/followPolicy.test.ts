import { describe, expect, it } from 'vitest'
import {
  compareVersions,
  computeState,
  FOLLOW_MAX_AGE_MS,
  hasNews,
  LAUNCH_CHECK_INTERVAL_MS,
  shouldCheckAtLaunch,
  totalUnread,
  unreadCount,
  type FollowedReport,
  type ThreadMessage,
} from './followPolicy'

const msg = (seq: number, author: ThreadMessage['author'], kind: ThreadMessage['kind'] = 'text'): ThreadMessage => ({ seq, at: 0, author, kind, text: 't' })
const rep = (o: Partial<FollowedReport> = {}): FollowedReport => ({
  id: 'HEW-AAAA-BBBB', sentAt: 1_000_000, appVersion: '1.3.1', excerpt: 'x', fileName: null, status: 'open', statusVersion: null, statusAt: null,
  messages: [], latest: 0, seenUpTo: 0, lastCheckedAt: null, state: 'following', ...o,
})

describe('compareVersions', () => {
  it.each([
    ['1.3.1', '1.3.1', 0],
    ['1.3.1', '1.3.2', -1],
    ['1.10.0', '1.9.0', 1],
    ['1.4.0-pre.1', '1.4.0', -1],
    ['1.4.0', '1.4.0-pre.1', 1],
    ['1.4.0-pre.2', '1.4.0-pre.10', -1],
    ['1.4.0-alpha', '1.4.0-pre.1', -1],
    ['1.4', '1.4.0', 0],
    ['v1.4.0+build5', '1.4.0', 0],
    ['1.4.0-1', '1.4.0-a', -1],
  ])('%s vs %s', (a, b, want) => expect(compareVersions(a, b)).toBe(want))
})

describe('unread', () => {
  it('counts maintainer, system, and status messages past seenUpTo, not the reporter own', () => {
    const r = rep({ messages: [msg(1, 'reporter'), msg(2, 'maintainer'), msg(3, 'system'), msg(4, 'reporter', 'status'), msg(5, 'maintainer')], seenUpTo: 2, latest: 5 })
    expect(unreadCount(r)).toBe(3)
    expect(hasNews(r)).toBe(true)
    expect(totalUnread([r, rep()])).toBe(3)
  })
  it('skips gone reports', () => {
    expect(unreadCount(rep({ status: 'gone', messages: [msg(1, 'maintainer')] }))).toBe(0)
  })
})

describe('computeState', () => {
  it('is done only once the final status message is seen', () => {
    const base = { status: 'fixed' as const, messages: [msg(1, 'maintainer'), msg(2, 'system', 'status')], latest: 2 }
    expect(computeState(rep({ ...base, seenUpTo: 1 }))).toBe('following')
    expect(computeState(rep({ ...base, seenUpTo: 2 }))).toBe('done')
    expect(computeState(rep({ status: 'open', seenUpTo: 9 }))).toBe('following')
    expect(computeState(rep({ status: 'gone' }))).toBe('done')
  })
})

describe('shouldCheckAtLaunch', () => {
  const now = 10_000_000_000
  it('honors the 6 hour rule', () => {
    expect(shouldCheckAtLaunch(rep({ sentAt: now - 1000 }), now)).toBe(true)
    expect(shouldCheckAtLaunch(rep({ sentAt: now - 1000, lastCheckedAt: now - LAUNCH_CHECK_INTERVAL_MS + 1 }), now)).toBe(false)
    expect(shouldCheckAtLaunch(rep({ sentAt: now - 1000, lastCheckedAt: now - LAUNCH_CHECK_INTERVAL_MS }), now)).toBe(true)
  })
  it('treats a future lastCheckedAt (clock set back) as due', () => {
    expect(shouldCheckAtLaunch(rep({ sentAt: now - 1000, lastCheckedAt: now + 5 * 3600_000 }), now)).toBe(true)
  })
  it('skips done, gone, and 365-day-old reports', () => {
    expect(shouldCheckAtLaunch(rep({ sentAt: now - 1000, state: 'done' }), now)).toBe(false)
    expect(shouldCheckAtLaunch(rep({ sentAt: now - 1000, status: 'gone' }), now)).toBe(false)
    expect(shouldCheckAtLaunch(rep({ sentAt: now - FOLLOW_MAX_AGE_MS }), now)).toBe(false)
  })
})

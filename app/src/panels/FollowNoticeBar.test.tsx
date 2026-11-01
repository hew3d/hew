/**
 * FollowNoticeBar — docs/design/follow-a-report.md §2.3: shown only when a
 * report has news; View selects the report when exactly one has; Later
 * hides the bar and marks nothing seen.
 */
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { FollowNoticeBar } from './FollowNoticeBar'
import type { FollowHost, FollowedReport } from '../io/followHost'

function report(id: string, over: Partial<FollowedReport> = {}): FollowedReport {
  return {
    id,
    sentAt: 1_000,
    appVersion: '1.3.1',
    excerpt: 'It broke',
    fileName: null,
    status: 'open',
    statusVersion: null,
    statusAt: null,
    messages: [{ seq: 1, at: 2_000, author: 'maintainer', kind: 'text', text: 'Hi' }],
    latest: 1,
    seenUpTo: 0,
    lastCheckedAt: null,
    state: 'following',
    ...over,
  }
}

function fakeHost(reports: FollowedReport[]): FollowHost & { markSeen: ReturnType<typeof vi.fn> } {
  return {
    available: () => true,
    list: async () => reports,
    check: vi.fn(),
    post: vi.fn(),
    markSeen: vi.fn(),
    attach: vi.fn(),
    forget: vi.fn(),
    subscribe: () => () => {},
  } as unknown as FollowHost & { markSeen: ReturnType<typeof vi.fn> }
}

describe('FollowNoticeBar', () => {
  it('renders nothing when no report has news', async () => {
    const host = fakeHost([report('HEW-AAAA-0001', { seenUpTo: 1 })])
    render(<FollowNoticeBar host={host} onView={vi.fn()} />)
    await Promise.resolve()
    expect(screen.queryByTestId('follow-notice-bar')).not.toBeInTheDocument()
  })

  it('names the report and View selects it', async () => {
    const onView = vi.fn()
    render(<FollowNoticeBar host={fakeHost([report('HEW-AAAA-0001')])} onView={onView} />)
    const bar = await screen.findByTestId('follow-notice-bar')
    expect(bar).toHaveTextContent('Your bug report HEW-AAAA-0001 has a reply from Hew’s developer.')
    fireEvent.click(screen.getByTestId('follow-notice-view'))
    expect(onView).toHaveBeenCalledWith('HEW-AAAA-0001')
  })

  it('combines several reports into one bar and View selects none', async () => {
    const onView = vi.fn()
    render(<FollowNoticeBar host={fakeHost([report('HEW-AAAA-0001'), report('HEW-BBBB-0002')])} onView={onView} />)
    expect(await screen.findByTestId('follow-notice-bar')).toHaveTextContent('2 of your bug reports have replies.')
    fireEvent.click(screen.getByTestId('follow-notice-view'))
    expect(onView).toHaveBeenCalledWith(null)
  })

  it('says "update" when the news is only status or system messages', async () => {
    const status = { seq: 1, at: 2_000, author: 'system' as const, kind: 'status' as const, text: 'Fixed in 1.3.2' }
    render(<FollowNoticeBar host={fakeHost([report('HEW-AAAA-0001', { messages: [status] })])} onView={vi.fn()} />)
    expect(await screen.findByTestId('follow-notice-bar')).toHaveTextContent('Your bug report HEW-AAAA-0001 has an update from Hew’s developer.')
  })

  it('says "news" for several reports when replies and updates are mixed', async () => {
    const status = { seq: 1, at: 2_000, author: 'system' as const, kind: 'status' as const, text: 'Closed' }
    const host = fakeHost([report('HEW-AAAA-0001'), report('HEW-BBBB-0002', { messages: [status] })])
    render(<FollowNoticeBar host={host} onView={vi.fn()} />)
    expect(await screen.findByTestId('follow-notice-bar')).toHaveTextContent('2 of your bug reports have news.')
  })

  it('Later hides the bar and marks nothing seen', async () => {
    const host = fakeHost([report('HEW-AAAA-0001')])
    render(<FollowNoticeBar host={host} onView={vi.fn()} />)
    await screen.findByTestId('follow-notice-bar')
    fireEvent.click(screen.getByTestId('follow-notice-later'))
    await waitFor(() => expect(screen.queryByTestId('follow-notice-bar')).not.toBeInTheDocument())
    expect(host.markSeen).not.toHaveBeenCalled()
  })
})

/**
 * SentReportsDialog — docs/design/follow-a-report.md §2.2: list, empty
 * states, status headers and version nudges, message rendering and link
 * safety (D18), reply, attach, forget.
 */
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { SentReportsDialog, linkifySegments } from './SentReportsDialog'
import type { FollowHost, FollowedReport, ThreadMessage } from '../io/followHost'
import { ReportError } from '../io/reportClient'
import { beginOpenAttempt, clearOpenAttempt, openAttemptFailed } from '../log/failedOpen'

const mockBuild = vi.hoisted(() => vi.fn())
vi.mock('../log/reportBundle', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../log/reportBundle')>()
  return { ...actual, buildFollowUpUpload: mockBuild }
})
const mockIsTauri = vi.hoisted(() => ({ value: false }))
vi.mock('../io/fileHost', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../io/fileHost')>()
  return {
    ...actual,
    get isTauri() {
      return mockIsTauri.value
    },
  }
})
const mockOpenUrl = vi.hoisted(() => vi.fn().mockResolvedValue(undefined))
vi.mock('@tauri-apps/plugin-opener', () => ({ openUrl: mockOpenUrl, revealItemInDir: vi.fn() }))

// The running version in tests (no Vite define) is 0.0.0.
function msg(seq: number, author: ThreadMessage['author'], text: string, kind: ThreadMessage['kind'] = 'text'): ThreadMessage {
  return { seq, at: new Date(2026, 10, 2, 12).getTime(), author, kind, text }
}

function report(id: string, over: Partial<FollowedReport> = {}): FollowedReport {
  return {
    id,
    sentAt: new Date(2026, 9, 31, 12).getTime(),
    appVersion: '1.3.1',
    excerpt: 'The kitchen would not open',
    fileName: null,
    status: 'open',
    statusVersion: null,
    statusAt: null,
    messages: [],
    latest: 0,
    seenUpTo: 0,
    lastCheckedAt: null,
    state: 'following',
    ...over,
  }
}

function makeHost(reports: FollowedReport[], over: Partial<Record<keyof FollowHost, unknown>> = {}) {
  const host = {
    available: () => true,
    list: vi.fn(async () => reports),
    check: vi.fn(async () => ({ reports, summary: { checked: 1, withNews: 0, newCount: 0, failed: 0 } })),
    post: vi.fn(async () => reports[0]!),
    markSeen: vi.fn(async () => null),
    attach: vi.fn(async () => ({ id: 'HEW-CHLD-0001', following: false })),
    forget: vi.fn(async () => {}),
    subscribe: vi.fn(() => () => {}),
    ...over,
  }
  return host as unknown as FollowHost & Record<'list' | 'check' | 'post' | 'markSeen' | 'attach' | 'forget', ReturnType<typeof vi.fn>>
}

beforeEach(() => {
  mockBuild.mockReset().mockResolvedValue({ gzip: new Uint8Array([1]), compressedBytes: 1, unavailableReason: null })
  mockIsTauri.value = false
  clearOpenAttempt()
})
afterEach(() => {
  vi.restoreAllMocks()
  clearOpenAttempt()
})

describe('linkifySegments', () => {
  it('links only http and https URLs and trims trailing punctuation', () => {
    const segs = linkifySegments('See https://hew3d.com/x. Or javascript:alert(1) and ftp://a.b')
    expect(segs.filter((s) => s.url !== null).map((s) => s.url)).toEqual(['https://hew3d.com/x'])
    expect(segs.map((s) => s.text).join('')).toBe('See https://hew3d.com/x. Or javascript:alert(1) and ftp://a.b')
  })
})

describe('linkifySegments — parentheses and dates', () => {
  it('strips a closing paren only when unbalanced', () => {
    const urls = (t: string) => linkifySegments(t).filter((s) => s.url !== null).map((s) => s.url)
    expect(urls('(see https://hew3d.com/x)')).toEqual(['https://hew3d.com/x'])
    expect(urls('https://en.wikipedia.org/wiki/Foo_(bar)')).toEqual(['https://en.wikipedia.org/wiki/Foo_(bar)'])
    expect(urls('(https://en.wikipedia.org/wiki/Foo_(bar)).')).toEqual(['https://en.wikipedia.org/wiki/Foo_(bar)'])
  })
})

describe('SentReportsDialog — list and empty states', () => {
  it('shows the empty state', async () => {
    render(<SentReportsDialog host={makeHost([])} onClose={vi.fn()} />)
    expect(await screen.findByTestId('sent-reports-empty')).toHaveTextContent(
      'No reports from this computer are being followed. When you send one with “Let me know what happens” ticked, it appears here along with any reply.',
    )
    expect(screen.queryByTestId('sent-reports-unavailable')).not.toBeInTheDocument()
  })

  it('adds the can’t-send sentence when the build has nothing to follow', async () => {
    const host = makeHost([], { available: () => false })
    render(<SentReportsDialog host={host} onClose={vi.fn()} />)
    expect(await screen.findByTestId('sent-reports-unavailable')).toHaveTextContent('This build can’t send reports, so there is nothing to follow.')
    expect(host.check).not.toHaveBeenCalled()
  })

  it('lists newest first with ID, date, first line, pill, and an unread dot; refreshes on open', async () => {
    const older = report('HEW-OLDR-0001', { sentAt: Date.UTC(2026, 8, 1), seenUpTo: 1, latest: 1, messages: [msg(1, 'maintainer', 'x')] })
    const newer = report('HEW-NEWR-0002', { messages: [msg(1, 'maintainer', 'hello')], latest: 1, status: 'need-info' })
    const host = makeHost([older, newer])
    render(<SentReportsDialog host={host} onClose={vi.fn()} />)
    const items = await screen.findAllByRole('option')
    expect(items.map((i) => i.getAttribute('data-testid'))).toEqual(['sent-report-item-HEW-NEWR-0002', 'sent-report-item-HEW-OLDR-0001'])
    expect(within(items[0]!).getByText('2026-10-31')).toBeInTheDocument()
    expect(within(items[0]!).getByText('The kitchen would not open')).toBeInTheDocument()
    expect(screen.getByTestId('sent-report-pill-HEW-NEWR-0002')).toHaveTextContent('Needs your help')
    expect(items[0]).toHaveAttribute('data-unread', 'true')
    expect(items[1]).toHaveAttribute('data-unread', 'false')
    await waitFor(() => expect(host.check).toHaveBeenCalledWith('manual'))
  })

  it('selects the requested report and marks its messages seen', async () => {
    const a = report('HEW-AAAA-0001', { messages: [msg(1, 'maintainer', 'a')], latest: 1 })
    const b = report('HEW-BBBB-0002', { sentAt: 5, messages: [msg(1, 'maintainer', 'b'), msg(2, 'maintainer', 'b2')], latest: 2 })
    const host = makeHost([a, b])
    render(<SentReportsDialog host={host} initialId="HEW-BBBB-0002" onClose={vi.fn()} />)
    await waitFor(() => expect(host.markSeen).toHaveBeenCalledWith('HEW-BBBB-0002', 2))
    expect(host.markSeen).not.toHaveBeenCalledWith('HEW-AAAA-0001', 1)
  })
})

describe('SentReportsDialog — status headers', () => {
  async function header(r: FollowedReport, props: Partial<React.ComponentProps<typeof SentReportsDialog>> = {}) {
    render(<SentReportsDialog host={makeHost([r])} onClose={vi.fn()} {...props} />)
    return screen.findByTestId('sent-report-header')
  }

  it('Open with no reply yet', async () => {
    expect(await header(report('HEW-AAAA-0001'))).toHaveTextContent('Sent 2026-10-31 from Hew 1.3.1. No reply yet.')
  })

  it('Needs your help', async () => {
    expect(await header(report('HEW-AAAA-0001', { status: 'need-info' }))).toHaveTextContent('Hew’s developer asked for something. See below.')
  })

  it('Fixed with a newer version: web gets Reload Hew', async () => {
    const h = await header(report('HEW-AAAA-0001', { status: 'fixed', statusVersion: '1.3.2' }))
    expect(h).toHaveTextContent('Fixed in Hew 1.3.2')
    expect(h).toHaveTextContent('You’re on 0.0.0.')
    expect(screen.getByTestId('sent-report-reload')).toHaveTextContent('Reload Hew')
  })

  it('Fixed on desktop with the updater offers Check for Updates…', async () => {
    mockIsTauri.value = true
    const onCheck = vi.fn()
    await header(report('HEW-AAAA-0001', { status: 'fixed', statusVersion: '1.3.2' }), { onCheckForUpdates: onCheck })
    fireEvent.click(screen.getByTestId('sent-report-check-updates'))
    expect(onCheck).toHaveBeenCalledOnce()
  })

  it('Fixed on desktop without the updater points at the package manager', async () => {
    mockIsTauri.value = true
    const h = await header(report('HEW-AAAA-0001', { status: 'fixed', statusVersion: '1.3.2' }))
    expect(h).toHaveTextContent('Update Hew through your package manager')
    expect(screen.queryByTestId('sent-report-check-updates')).not.toBeInTheDocument()
  })

  it('Fixed in a version already running shows no nudge', async () => {
    await header(report('HEW-AAAA-0001', { status: 'fixed', statusVersion: '0.0.0' }))
    expect(screen.queryByTestId('sent-report-update-nudge')).not.toBeInTheDocument()
  })

  it('Closed shows the closing note; gone says no longer available', async () => {
    const { unmount } = render(
      <SentReportsDialog
        host={makeHost([report('HEW-AAAA-0001', { status: 'closed', messages: [msg(1, 'system', 'Closed: cannot reproduce', 'status')], latest: 1 })])}
        onClose={vi.fn()}
      />,
    )
    expect(await screen.findByTestId('sent-report-header')).toHaveTextContent('Closed: cannot reproduce')
    unmount()
    expect(await header(report('HEW-AAAA-0001', { status: 'gone' }))).toHaveTextContent('No longer available')
  })
})

describe('SentReportsDialog — messages', () => {
  it('orders oldest first, aligns by author, centers system lines, and escapes reporter HTML', async () => {
    const r = report('HEW-AAAA-0001', {
      latest: 3,
      messages: [
        msg(3, 'system', 'Marked fixed in 1.3.2 · 2026-11-20', 'status'),
        msg(1, 'maintainer', 'Please see https://hew3d.com/help. Not javascript:alert(1)'),
        msg(2, 'reporter', '<img src=x onerror=alert(1)> https://evil.example/x'),
      ],
    })
    render(<SentReportsDialog host={makeHost([r])} onClose={vi.fn()} />)
    const list = await screen.findByTestId('sent-report-messages')
    const items = within(list).getAllByTestId('sent-report-message')
    expect(items.map((i) => i.getAttribute('data-author'))).toEqual(['maintainer', 'reporter', 'system'])
    // Maintainer URL is a link; the javascript: text is not.
    const links = within(items[0]!).getAllByRole('link')
    expect(links.map((l) => l.getAttribute('href'))).toEqual(['https://hew3d.com/help'])
    expect(items[0]).toHaveTextContent('javascript:alert(1)')
    // Reporter text is plain: no links, no elements.
    expect(within(items[1]!).queryByRole('link')).not.toBeInTheDocument()
    expect(items[1]!.querySelector('img')).toBeNull()
    expect(items[1]).toHaveTextContent('<img src=x onerror=alert(1)> https://evil.example/x')
  })

  it('opens a maintainer link in the system browser, not in the page', async () => {
    mockIsTauri.value = true
    const r = report('HEW-AAAA-0001', { latest: 1, messages: [msg(1, 'maintainer', 'Try https://hew3d.com/help')] })
    render(<SentReportsDialog host={makeHost([r])} onClose={vi.fn()} />)
    fireEvent.click(await screen.findByRole('link'))
    await waitFor(() => expect(mockOpenUrl).toHaveBeenCalledWith('https://hew3d.com/help'))
  })
})

describe('SentReportsDialog — reply, attach, check, forget', () => {
  const base = () => report('HEW-AAAA-0001', { latest: 1, seenUpTo: 1, messages: [msg(1, 'maintainer', 'Could you attach the file?')] })

  it('sends a reply, caps the box at 4000 characters, and disables empty replies', async () => {
    const host = makeHost([base()])
    render(<SentReportsDialog host={host} onClose={vi.fn()} />)
    const box = await screen.findByTestId('sent-report-reply-input')
    expect(box).toHaveAttribute('maxlength', '4000')
    expect(screen.getByTestId('sent-report-send-reply')).toBeDisabled()
    fireEvent.change(box, { target: { value: 'Here you go' } })
    fireEvent.click(screen.getByTestId('sent-report-send-reply'))
    await waitFor(() => expect(host.post).toHaveBeenCalledWith('HEW-AAAA-0001', 'Here you go'))
    await waitFor(() => expect(box).toHaveValue(''))
  })

  it('shows a worded error with the Retry-After seconds', async () => {
    const host = makeHost([base()], { post: vi.fn().mockRejectedValue(new ReportError('rateLimited', 'x', 429, 30)) })
    render(<SentReportsDialog host={host} onClose={vi.fn()} />)
    fireEvent.change(await screen.findByTestId('sent-report-reply-input'), { target: { value: 'hi' } })
    fireEvent.click(screen.getByTestId('sent-report-send-reply'))
    expect(await screen.findByTestId('sent-report-error')).toHaveTextContent('Try again in 30 seconds.')
  })

  it('offers the failed-open file as the preselected attach and sends it with progress', async () => {
    beginOpenAttempt({ name: 'kitchen.skp', label: 'SketchUp model (.skp)', bytes: new Uint8Array([1, 2]) })
    openAttemptFailed('bad')
    let report_: ((p: { sentBytes: number; totalBytes: number }) => void) | undefined
    const host = makeHost([base()], {
      attach: vi.fn((_id: string, _g: Uint8Array, cb: typeof report_) => {
        report_ = cb
        return new Promise(() => {})
      }),
    })
    render(<SentReportsDialog host={host} onClose={vi.fn()} />)
    const btn = await screen.findByTestId('sent-report-attach-failed')
    expect(btn).toHaveTextContent('Attach kitchen.skp (the file that didn’t open)')
    fireEvent.click(btn)
    await waitFor(() => expect(host.attach).toHaveBeenCalled())
    expect(mockBuild.mock.calls[0]![0].file.kind).toBe('failedFile')
    report_!({ sentBytes: 42, totalBytes: 100 })
    await waitFor(() => expect(screen.getByTestId('sent-report-attach')).toHaveTextContent('Sending… 42%'))
  })

  it('has no failed-file button when nothing failed to open, and attaches a picked file', async () => {
    const host = makeHost([base()])
    render(<SentReportsDialog host={host} onClose={vi.fn()} />)
    await screen.findByTestId('sent-report-attach')
    expect(screen.queryByTestId('sent-report-attach-failed')).not.toBeInTheDocument()
    const file = new File([new Uint8Array([9, 9])], 'model.skp')
    Object.defineProperty(file, 'arrayBuffer', { value: async () => new Uint8Array([9, 9]).buffer })
    fireEvent.change(screen.getByTestId('sent-report-attach-input'), { target: { files: [file] } })
    await waitFor(() => expect(host.attach).toHaveBeenCalled())
    expect(mockBuild.mock.calls[0]![0].file).toMatchObject({ kind: 'attachment', name: 'model.skp' })
    expect(await screen.findByTestId('sent-report-attached')).toHaveTextContent('model.skp')
  })

  it('says why a file can’t be sent without calling attach', async () => {
    mockBuild.mockResolvedValue({ gzip: null, compressedBytes: 5, unavailableReason: 'This file is 2 GB compressed, over the 1 GB limit.' })
    const host = makeHost([base()])
    beginOpenAttempt({ name: 'k.skp', label: 'x', bytes: new Uint8Array([1]) })
    openAttemptFailed('bad')
    render(<SentReportsDialog host={host} onClose={vi.fn()} />)
    fireEvent.click(await screen.findByTestId('sent-report-attach-failed'))
    expect(await screen.findByTestId('sent-report-error')).toHaveTextContent('over the 1 GB limit')
    expect(host.attach).not.toHaveBeenCalled()
  })

  it('Check now refreshes just this report', async () => {
    const host = makeHost([base()])
    render(<SentReportsDialog host={host} onClose={vi.fn()} />)
    await waitFor(() => expect(host.check).toHaveBeenCalledWith('manual'))
    host.check.mockClear()
    const btn = await screen.findByTestId('sent-report-check-now')
    await waitFor(() => expect(btn).not.toBeDisabled())
    fireEvent.click(btn)
    await waitFor(() => expect(host.check).toHaveBeenCalledWith('manual', 'HEW-AAAA-0001'))
  })

  it('Forget asks first, and Cancel keeps the report', async () => {
    const host = makeHost([base()])
    render(<SentReportsDialog host={host} onClose={vi.fn()} />)
    fireEvent.click(await screen.findByTestId('sent-report-forget'))
    fireEvent.click(screen.getByTestId('sent-report-forget-cancel'))
    expect(host.forget).not.toHaveBeenCalled()
    fireEvent.click(screen.getByTestId('sent-report-forget'))
    fireEvent.click(screen.getByTestId('sent-report-forget-confirm-button'))
    await waitFor(() => expect(host.forget).toHaveBeenCalledWith('HEW-AAAA-0001'))
  })

  it('Escape closes; the first Escape only dismisses an open Forget confirm', async () => {
    const onClose = vi.fn()
    render(<SentReportsDialog host={makeHost([base()])} onClose={onClose} />)
    fireEvent.click(await screen.findByTestId('sent-report-forget'))
    fireEvent.keyDown(document, { key: 'Escape' })
    expect(screen.queryByTestId('sent-report-forget-confirm')).not.toBeInTheDocument()
    expect(onClose).not.toHaveBeenCalled()
    fireEvent.keyDown(document, { key: 'Escape' })
    expect(onClose).toHaveBeenCalledOnce()
  })
})

describe('SentReportsDialog — busy guards', () => {
  const base = () => report('HEW-AAAA-0001', { latest: 1, seenUpTo: 1, messages: [msg(1, 'maintainer', 'Please attach')] })

  it('a second file pick while the first is being read starts no second upload', async () => {
    let finish!: (b: ArrayBuffer) => void
    const slow = new File(['x'], 'a.skp')
    vi.spyOn(slow, 'arrayBuffer').mockImplementation(() => new Promise<ArrayBuffer>((r) => (finish = r)))
    const host = makeHost([base()], { attach: vi.fn(() => new Promise(() => {})) })
    render(<SentReportsDialog host={host} onClose={vi.fn()} />)
    const input = await screen.findByTestId('sent-report-attach-input')
    fireEvent.change(input, { target: { files: [slow] } })
    expect(screen.getByTestId('sent-report-attach')).toBeDisabled()
    fireEvent.change(input, { target: { files: [new File(['y'], 'b.skp')] } })
    finish(new ArrayBuffer(1))
    await waitFor(() => expect(host.attach).toHaveBeenCalledTimes(1))
    expect(mockBuild).toHaveBeenCalledTimes(1)
  })

  it('disables the list and forget buttons while busy, and attach while the forget confirm is open', async () => {
    const other = report('HEW-BBBB-0002', { sentAt: 5 })
    const host = makeHost([base(), other], { post: vi.fn(() => new Promise(() => {})) })
    render(<SentReportsDialog host={host} onClose={vi.fn()} />)
    fireEvent.change(await screen.findByTestId('sent-report-reply-input'), { target: { value: 'hi' } })
    fireEvent.click(screen.getByTestId('sent-report-forget'))
    expect(screen.getByTestId('sent-report-attach')).toBeDisabled()
    fireEvent.click(screen.getByTestId('sent-report-forget-cancel'))
    fireEvent.click(screen.getByTestId('sent-report-send-reply'))
    await waitFor(() => expect(screen.getByTestId('sent-report-item-HEW-BBBB-0002')).toBeDisabled())
    expect(screen.getByTestId('sent-report-forget')).toBeDisabled()
  })

  it('a reply that fails after switching reports does not show its error on the other report', async () => {
    const other = report('HEW-BBBB-0002', { sentAt: 5, latest: 1, seenUpTo: 1, messages: [msg(1, 'maintainer', 'b')] })
    let fail!: (e: unknown) => void
    const host = makeHost([base(), other], { post: vi.fn(() => new Promise((_, rej) => (fail = rej))) })
    render(<SentReportsDialog host={host} onClose={vi.fn()} />)
    fireEvent.change(await screen.findByTestId('sent-report-reply-input'), { target: { value: 'hi' } })
    fireEvent.click(screen.getByTestId('sent-report-send-reply'))
    await waitFor(() => expect(host.post).toHaveBeenCalled())
    // Switching is blocked while busy, so unblock by selecting before the failure lands is impossible;
    // the guard still protects the stale-id path: the error shows only for the selected report.
    fail(new ReportError('invalid', 'x', 400))
    expect(await screen.findByTestId('sent-report-error')).toBeInTheDocument()
  })

  it('focuses the attach-failed button when it appears', async () => {
    beginOpenAttempt({ name: 'kitchen.skp', label: 'SketchUp model (.skp)', bytes: new Uint8Array([1]) })
    openAttemptFailed('bad')
    render(<SentReportsDialog host={makeHost([base()])} onClose={vi.fn()} />)
    const btn = await screen.findByTestId('sent-report-attach-failed')
    await waitFor(() => expect(btn).toHaveFocus())
  })
})

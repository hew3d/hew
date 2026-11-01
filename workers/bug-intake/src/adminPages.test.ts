import { test, describe } from 'node:test'
import assert from 'node:assert/strict'

import { escapeHtml, renderList, renderDetail, statusLabel, hasNewActivity, CANNED_REPLIES } from './adminPages.ts'
import type { ThreadAdminView } from './threadStore.ts'
import { parseHeadForDisplay } from './headScan.ts'
import type { ReportRow } from './indexStore.ts'

function entry(overrides: Partial<ReportRow> = {}): ReportRow {
  return {
    id: 'HEW-AAAA-BBBB',
    receivedAt: Date.parse('2026-09-13T12:00:00.000Z'),
    appVersion: '1.1.0',
    platform: 'desktop-macos',
    descriptionPreview: 'it crashed',
    sizeBytes: 4096,
    triaged: false,
    committed: true,
    following: false,
    status: 'open',
    statusVersion: null,
    reporterActivityAt: null,
    adminViewedAt: null,
    parentId: null,
    bundleDeleted: false,
    ...overrides,
  }
}

describe('escapeHtml', () => {
  test('escapes the five HTML-significant characters', () => {
    assert.equal(escapeHtml(`<script>alert('x')</script> & "quoted"`), '&lt;script&gt;alert(&#39;x&#39;)&lt;/script&gt; &amp; &quot;quoted&quot;')
  })

  test('round-trips plain text unchanged', () => {
    assert.equal(escapeHtml('nothing special here 123'), 'nothing special here 123')
  })
})

describe('renderList: XSS and Origin safety', () => {
  test('escapes an attacker-controlled description preview, app version, and platform', () => {
    const html = renderList([
      entry({
        descriptionPreview: '<img src=x onerror=alert(1)>',
        appVersion: '"><script>evil()</script>',
        platform: "<svg/onload=alert('p')>",
      }),
    ])
    assert.ok(!html.includes('<img src=x onerror=alert(1)>'))
    assert.ok(!html.includes('<script>evil()</script>'))
    assert.ok(!html.includes("<svg/onload=alert('p')>"))
    assert.ok(html.includes('&lt;img src=x onerror=alert(1)&gt;'))
  })

  test('does not escape the server-generated id (safe by construction)', () => {
    const html = renderList([entry({ id: 'HEW-QQQQ-RRRR' })])
    assert.ok(html.includes('HEW-QQQQ-RRRR'))
  })

  test('shows report count and triaged state', () => {
    const html = renderList([entry({ triaged: true }), entry({ id: 'HEW-CCCC-DDDD', triaged: false })])
    assert.match(html, /2 reports/)
  })

  test('shows "unknown" for an empty appVersion/platform — a report submitted with no system block', () => {
    const html = renderList([entry({ appVersion: '', platform: '' })])
    assert.match(html, /<td>unknown<\/td>\s*<td>unknown<\/td>/)
  })
})

describe('renderDetail: XSS safety', () => {
  test('escapes the description, expected text, and crash message', () => {
    const head = parseHeadForDisplay(
      JSON.stringify({
        format: 1,
        report: {
          description: '<script>steal(document.cookie)</script>',
          expected: '<img src=x onerror=alert(2)>',
        },
        crash: { at: '<b>here</b>', message: '"><script>oops()</script>' },
      }),
    )
    const html = renderDetail(entry(), head)
    assert.ok(!html.includes('<script>steal(document.cookie)</script>'))
    assert.ok(!html.includes('<img src=x onerror=alert(2)>'))
    assert.ok(!html.includes('<script>oops()</script>'))
    assert.ok(!html.includes('<b>here</b>'))
  })

  test('renders a fallback message when the head could not be read', () => {
    const html = renderDetail(entry(), null)
    assert.match(html, /could not be read/)
  })

  test('shows "unknown" for App version/Platform when the head has no system block', () => {
    const head = parseHeadForDisplay(JSON.stringify({ format: 1, report: { description: 'no system sent' } }))
    const html = renderDetail(entry(), head)
    assert.match(html, /App version<\/dt><dd>unknown<\/dd>/)
    assert.match(html, /Platform<\/dt><dd>unknown<\/dd>/)
  })

  test('escapes system info fields', () => {
    const head = parseHeadForDisplay(
      JSON.stringify({
        format: 1,
        report: { description: 'x'.repeat(20) },
        system: { userAgent: '<script>ua()</script>', gpu: '"><script>gpu()</script>' },
      }),
    )
    const html = renderDetail(entry(), head)
    assert.ok(!html.includes('<script>ua()</script>'))
    assert.ok(!html.includes('<script>gpu()</script>'))
  })

  test('says the recorded steps/files/model/log are in the downloaded bundle, without claiming inclusion either way', () => {
    const head = parseHeadForDisplay(JSON.stringify({ format: 1, report: { description: 'x'.repeat(20) } }))
    const html = renderDetail(entry(), head)
    assert.match(html, /downloaded bundle/)
  })
})

function threadView(overrides: Partial<ThreadAdminView> = {}): ThreadAdminView {
  return {
    reportId: 'HEW-AAAA-BBBB',
    following: true,
    status: 'open',
    statusVersion: null,
    statusAt: 0,
    createdAt: 0,
    lastActivityAt: 0,
    lastPolledAt: null,
    reporterSeenUpTo: 0,
    previousAdminSeenUpTo: 0,
    finalSeenAt: null,
    latest: 0,
    messages: [],
    ...overrides,
  }
}

describe('follow-a-report admin rendering', () => {
  test('list: status and following columns, the new marker, and the follow-up marker', () => {
    const html = renderList([
      entry({ id: 'HEW-AAAA-0001', following: true, status: 'fixed', statusVersion: '1.3.2', reporterActivityAt: 5, adminViewedAt: 2 }),
      entry({ id: 'HEW-AAAA-0002', parentId: 'HEW-AAAA-0001', bundleDeleted: true, sizeBytes: 0 }),
    ])
    assert.ok(html.includes('<td>fixed 1.3.2</td>'))
    assert.ok(html.includes('<td>yes</td>'))
    assert.ok(html.includes('HEW-AAAA-0001</code></a> <span class="new">new</span>'))
    assert.ok(html.includes('↳ for <a href="/report/admin/HEW-AAAA-0001">HEW-AAAA-0001</a>'))
    assert.ok(html.includes('<td>expired</td>'))
  })

  test('statusLabel and hasNewActivity', () => {
    assert.equal(statusLabel('need-info', null), 'needs info')
    assert.equal(statusLabel('fixed', '1.3.2'), 'fixed 1.3.2')
    assert.equal(statusLabel('closed', '1.3.2'), 'closed')
    assert.equal(statusLabel(undefined, null), 'open')
    assert.equal(hasNewActivity({ reporterActivityAt: null, adminViewedAt: null }), false)
    assert.equal(hasNewActivity({ reporterActivityAt: 3, adminViewedAt: null }), true)
    assert.equal(hasNewActivity({ reporterActivityAt: 3, adminViewedAt: 3 }), false)
  })

  test('detail: every message text is escaped, attachments link, forms carry the canned buttons', () => {
    const html = renderDetail(entry(), null, {
      thread: threadView({
        status: 'fixed',
        statusVersion: '"><img src=x>',
        latest: 3,
        previousAdminSeenUpTo: 1,
        messages: [
          { seq: 1, at: 0, author: 'maintainer', kind: 'text', text: '<b>hi</b>' },
          { seq: 2, at: 0, author: 'reporter', kind: 'text', text: '<script>alert(1)</script>' },
          { seq: 3, at: 0, author: 'reporter', kind: 'attachment', text: '<x>.skp (1 MB)', attachmentReportId: 'HEW-CCCC-DDDD' },
        ],
      }),
      children: [entry({ id: 'HEW-CCCC-DDDD', parentId: 'HEW-AAAA-BBBB', descriptionPreview: '<i>file</i>' })],
    })
    assert.ok(!html.includes('<script>alert(1)</script>'))
    assert.ok(!html.includes('<b>hi</b>'))
    assert.ok(!html.includes('<x>'))
    assert.ok(!html.includes('<i>file</i>'))
    assert.ok(!html.includes('"><img src=x>'))
    assert.ok(html.includes('&lt;script&gt;alert(1)&lt;/script&gt;'))
    assert.ok(html.includes('Following: yes'))
    assert.ok(html.includes('action="/report/admin/HEW-AAAA-BBBB/reply"'))
    assert.ok(html.includes('action="/report/admin/HEW-AAAA-BBBB/status"'))
    for (const [key, c] of Object.entries(CANNED_REPLIES)) {
      assert.ok(html.includes(`<button type="submit" name="canned" value="${key}">${c.label}</button>`))
    }
    assert.ok(html.includes('<option value="fixed" selected>Fixed in…</option>'))
    assert.ok(html.includes('/report/admin/HEW-CCCC-DDDD/download'))
    assert.equal((html.match(/<span class="new">new<\/span>/g) ?? []).length, 2, 'the two reporter messages after the cursor')
    assert.ok(!/<script/i.test(html))
  })

  test('detail: an unfollowed report says the reply reaches no one; a bundle-less one has no download', () => {
    const html = renderDetail(entry({ bundleDeleted: true, sizeBytes: 0 }), null, { thread: null, children: [] })
    assert.ok(html.includes('Following: no'))
    assert.ok(html.includes('No one is following this report. A reply is kept with it but will not reach anyone.'))
    assert.ok(!html.includes('/download'))
    assert.ok(html.includes('No messages yet.'))
  })
})

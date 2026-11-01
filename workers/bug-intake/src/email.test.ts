import { test, describe } from 'node:test'
import assert from 'node:assert/strict'

import {
  buildNotificationMime,
  buildThreadEventMime,
  sanitizeHeaderValue,
  sendNotification,
  sendThreadNotification,
  type RawMailSender,
} from './email.ts'
import { IndexStore } from './indexStore.ts'
import { EMAIL_DAY_MAX, EMAIL_THREAD_RESERVE } from './constants.ts'
import { FakeDurableObjectNamespace } from './testSupport/fakeDurableObject.ts'
import type { BugIntakeEnv, ReportIndexStub } from './types.ts'

function makeEnv(overrides: Partial<BugIntakeEnv> = {}): BugIntakeEnv {
  const env = {} as BugIntakeEnv
  env.REPORT_INDEX = new FakeDurableObjectNamespace<ReportIndexStub>((state) => new IndexStore(state.storage))
  env.NOTIFY_TO = 'maintainer@hew3d.com'
  env.NOTIFY_FROM = 'notify@hew3d.com'
  Object.assign(env, overrides)
  return env
}

function makeReport(overrides: Partial<Parameters<typeof buildNotificationMime>[0]> = {}) {
  return {
    id: 'HEW-AAAA-BBBB',
    appVersion: '1.1.0',
    platform: 'desktop-macos',
    description: 'the model disappeared after undo',
    sizeBytes: 12345,
    ...overrides,
  }
}

describe('sanitizeHeaderValue', () => {
  test('strips CR and LF', () => {
    assert.equal(sanitizeHeaderValue('a\r\nBcc: attacker@evil.example'), 'a Bcc: attacker@evil.example')
    assert.equal(sanitizeHeaderValue('a\nb\rc\r\nd'), 'a b c d')
  })

  test('trims surrounding whitespace left by stripping', () => {
    assert.equal(sanitizeHeaderValue('  value  '), 'value')
  })

  test('leaves an ordinary value untouched', () => {
    assert.equal(sanitizeHeaderValue('desktop-macos'), 'desktop-macos')
  })
})

describe('buildNotificationMime', () => {
  test('produces a header block terminated by a blank line, then the body', () => {
    const mime = buildNotificationMime(makeReport(), 'notify@hew3d.com', 'maintainer@hew3d.com')
    const sep = mime.indexOf('\r\n\r\n')
    assert.ok(sep > 0)
    const headers = mime.slice(0, sep)
    assert.match(headers, /^From: /)
    assert.match(headers, /Subject: \[Hew bug report\] HEW-AAAA-BBBB/)
    assert.match(headers, /Content-Type: text\/plain; charset="utf-8"/)
  })

  test('strips CR/LF from attacker-controlled appVersion/platform before they reach the Subject header', () => {
    const mime = buildNotificationMime(
      makeReport({ appVersion: '1.1.0\r\nBcc: attacker@evil.example', platform: 'desktop-macos' }),
      'notify@hew3d.com',
      'maintainer@hew3d.com',
    )
    const headerBlock = mime.slice(0, mime.indexOf('\r\n\r\n'))
    const headerLines = headerBlock.split('\r\n')
    // No injected header line, and no header line contains a raw \r or \n
    // other than the CRLF terminators `split` already consumed.
    assert.ok(!headerLines.some((line) => /^Bcc:/i.test(line)))
    for (const line of headerLines) {
      assert.ok(!line.includes('\r'))
      assert.ok(!line.includes('\n'))
    }
  })

  test('shows "unknown" for empty appVersion/platform — a report submitted with no system block', () => {
    const mime = buildNotificationMime(
      makeReport({ appVersion: '', platform: '' }),
      'notify@hew3d.com',
      'maintainer@hew3d.com',
    )
    assert.match(mime, /Subject: \[Hew bug report\] HEW-AAAA-BBBB — unknown \/ unknown/)
    assert.match(mime, /App version: unknown/)
    assert.match(mime, /Platform: unknown/)
  })

  test('the description rides the body verbatim, CR/LF included, and cannot inject a header', () => {
    const malicious = 'line one\r\nX-Injected: evil\r\n\r\nFake-Second-Message: yes'
    const mime = buildNotificationMime(makeReport({ description: malicious }), 'notify@hew3d.com', 'maintainer@hew3d.com')
    const sep = mime.indexOf('\r\n\r\n')
    const headerBlock = mime.slice(0, sep)
    // The header/body separator found is the REAL one the function emits,
    // not one hidden inside the malicious description — meaning everything
    // from the description landed after it, in the body, not parsed as
    // additional headers.
    assert.ok(!headerBlock.includes('X-Injected'))
    assert.ok(mime.includes(malicious))
  })

  test('includes the admin link and size', () => {
    const mime = buildNotificationMime(makeReport({ sizeBytes: 2048 }), 'notify@hew3d.com', 'maintainer@hew3d.com')
    assert.ok(mime.includes('https://app.hew3d.com/report/admin/HEW-AAAA-BBBB'))
    assert.ok(mime.includes('2.0 KB'))
  })
})

describe('sendNotification', () => {
  test('skips and returns false when NOTIFY_TO is missing', async () => {
    const env = makeEnv({ NOTIFY_TO: undefined })
    let called = false
    const mailer: RawMailSender = { send: async () => { called = true } }
    const sent = await sendNotification(env, makeReport(), mailer)
    assert.equal(sent, false)
    assert.equal(called, false)
  })

  test('skips and returns false when NOTIFY_FROM is missing', async () => {
    const env = makeEnv({ NOTIFY_FROM: undefined })
    let called = false
    const mailer: RawMailSender = { send: async () => { called = true } }
    const sent = await sendNotification(env, makeReport(), mailer)
    assert.equal(sent, false)
    assert.equal(called, false)
  })

  test('sends when configured, via the injected mailer', async () => {
    const env = makeEnv()
    let captured: [string, string, string] | null = null
    const mailer: RawMailSender = {
      send: async (from, to, raw) => {
        captured = [from, to, raw]
      },
    }
    const sent = await sendNotification(env, makeReport(), mailer)
    assert.equal(sent, true)
    assert.ok(captured !== null)
    assert.equal(captured[0], env.NOTIFY_FROM)
    assert.equal(captured[1], env.NOTIFY_TO)
  })

  test('a mailer failure is caught and reported as false, never thrown', async () => {
    const env = makeEnv()
    const mailer: RawMailSender = {
      send: async () => {
        throw new Error('SMTP said no')
      },
    }
    const sent = await sendNotification(env, makeReport(), mailer)
    assert.equal(sent, false)
  })

  test('a reserveEmail (Durable Object) failure is caught and reported as false, never thrown', async () => {
    // Regression: the doc comment always claimed this function never
    // throws, but only `mailer.send` used to be guarded — `reserveEmail`
    // is a DO RPC exactly like `mailer.send` and can fail the same way.
    const env = makeEnv()
    env.REPORT_INDEX = new FakeDurableObjectNamespace<ReportIndexStub>(() => ({
      reserveReport: () => Promise.reject(new Error('unused in this test')),
      commitReport: () => Promise.reject(new Error('unused in this test')),
      listReports: () => Promise.reject(new Error('unused in this test')),
      getReport: () => Promise.reject(new Error('unused in this test')),
      deleteReport: () => Promise.reject(new Error('unused in this test')),
      setTriaged: () => Promise.reject(new Error('unused in this test')),
      reserveEmail: () => Promise.reject(new Error('simulated Durable Object failure')),
    }))
    let called = false
    const mailer: RawMailSender = {
      send: async () => {
        called = true
      },
    }
    const sent = await sendNotification(env, makeReport(), mailer)
    assert.equal(sent, false)
    assert.equal(called, false, 'a reserveEmail failure must not fall through to actually sending')
  })

  test('skips once the daily email cap is reached, without touching the mailer', async () => {
    const env = makeEnv()
    let sendCount = 0
    const mailer: RawMailSender = { send: async () => { sendCount++ } }
    for (let i = 0; i < EMAIL_DAY_MAX; i++) {
      assert.equal(await sendNotification(env, makeReport({ id: `HEW-000${i}-0000` }), mailer), true)
    }
    const overCap = await sendNotification(env, makeReport({ id: 'HEW-OVER-CAP0' }), mailer)
    assert.equal(overCap, false)
    assert.equal(sendCount, EMAIL_DAY_MAX)
  })
})

describe('thread event emails (follow-a-report §3.3)', () => {
  test('reply: the subject and the message text in the body', () => {
    const raw = buildThreadEventMime({ kind: 'reply', id: 'HEW-AAAA-BBBB', text: 'Still broken.\nSubject: injected', reopened: false }, 'f@x', 't@x')
    assert.match(raw, /^Subject: \[Hew bug report\] HEW-AAAA-BBBB — reply from the reporter\r$/m)
    assert.equal((raw.split('\r\n\r\n')[0].match(/^Subject:/gm) ?? []).length, 1, 'body text never becomes a header')
    assert.ok(raw.includes('Still broken.'))
    assert.ok(!raw.includes('reopened'))
    assert.ok(buildThreadEventMime({ kind: 'reply', id: 'HEW-AAAA-BBBB', text: 'x', reopened: true }, 'f@x', 't@x').includes('reopened'))
  })

  test('attachment: the size in the subject, the reporter-chosen name only in the body', () => {
    const raw = buildThreadEventMime(
      { kind: 'attachment', id: 'HEW-AAAA-BBBB', childId: 'HEW-CCCC-DDDD', label: 'evil\r\nBcc: x@y.skp (46.5 MB)', sizeBytes: 48_758_000 },
      'f@x',
      't@x',
    )
    assert.match(raw, /^Subject: \[Hew bug report\] HEW-AAAA-BBBB — file attached \(46\.5 MB\)\r$/m)
    const headers = raw.split('\r\n\r\n')[0]
    assert.ok(!headers.includes('Bcc'))
    assert.ok(raw.includes('https://app.hew3d.com/report/admin/HEW-CCCC-DDDD'))
  })

  test('sendThreadNotification stops short of the cap, leaving the reserve for new reports, and never throws', async () => {
    const env = {} as BugIntakeEnv
    env.REPORT_INDEX = new FakeDurableObjectNamespace<ReportIndexStub>((state) => new IndexStore(state.storage))
    env.NOTIFY_TO = 't@x'
    env.NOTIFY_FROM = 'f@x'
    let sent = 0
    const mailer: RawMailSender = { send: async () => { sent++ } }
    const threadMax = EMAIL_DAY_MAX - EMAIL_THREAD_RESERVE
    for (let i = 0; i < threadMax; i++) {
      assert.equal(await sendThreadNotification(env, { kind: 'reply', id: 'HEW-AAAA-BBBB', text: 'x', reopened: false }, mailer), true)
    }
    assert.equal(await sendThreadNotification(env, { kind: 'reply', id: 'HEW-AAAA-BBBB', text: 'x', reopened: false }, mailer), false)
    assert.equal(sent, threadMax)
    // The reserved slots still carry new-report emails, all the way to the cap.
    for (let i = 0; i < EMAIL_THREAD_RESERVE; i++) {
      assert.equal(await sendNotification(env, makeReport(), mailer), true)
    }
    assert.equal(await sendNotification(env, makeReport(), mailer), false)
    assert.equal(sent, EMAIL_DAY_MAX)
    const throwing: RawMailSender = { send: async () => { throw new Error('smtp down') } }
    const env2 = { ...env, REPORT_INDEX: new FakeDurableObjectNamespace<ReportIndexStub>((state) => new IndexStore(state.storage)) }
    assert.equal(await sendThreadNotification(env2, { kind: 'reply', id: 'HEW-AAAA-BBBB', text: 'x', reopened: false }, throwing), false)
    const unconfigured = { ...env2, NOTIFY_TO: undefined }
    assert.equal(await sendThreadNotification(unconfigured, { kind: 'reply', id: 'HEW-AAAA-BBBB', text: 'x', reopened: false }, mailer), false)
  })
})

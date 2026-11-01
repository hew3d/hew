/**
 * The follow-a-report routes end to end through `handleRequest`, on the
 * in-process Worker from `testSupport/testEnv.ts` (all three Durable Object
 * classes on the fake namespace): START with `Hew-Follow`, COMMIT creating
 * the thread, every thread route and status code (§5.4), follow-up
 * attachments, the email choice, the index display caches, the
 * bundle-alarm change (§5.2), and the admin reply/status forms.
 */

import { test, describe } from 'node:test'
import assert from 'node:assert/strict'

import { buildTestEnv, type TestEnv } from './testSupport/testEnv.ts'
import { attachmentLabel } from './handlers.ts'
import { hashToken } from './uploadToken.ts'
import { CANNED_REPLIES } from './adminPages.ts'
import {
  PIECE_BYTES,
  THREAD_IDLE_MS,
  THREAD_FINAL_SEEN_GRACE_MS,
  THREAD_MESSAGE_MAX_CHARS,
  THREAD_MAINTAINER_RESERVE,
  THREAD_MAX_MESSAGES,
  THREAD_RATE_WINDOW_MAX,
  RATE_LIMIT_WINDOW_MAX,
  RETENTION_MS,
} from './constants.ts'

const BASE = 'https://app.hew3d.com'

async function gzipJson(doc: unknown): Promise<Uint8Array> {
  const cs = new CompressionStream('gzip')
  const writer = cs.writable.getWriter()
  const out = new Response(cs.readable).arrayBuffer()
  await writer.write(new TextEncoder().encode(JSON.stringify(doc)))
  await writer.close()
  return new Uint8Array(await out)
}

function bundle(description = 'the model disappeared after undo, ten times reproducible') {
  return { format: 1, report: { description }, system: { appVersion: '1.3.1', platform: 'desktop-macos' } }
}

async function start(t: TestEnv, headers: Record<string, string> = {}, description?: string, ip = '203.0.113.7'): Promise<Response> {
  const body = await gzipJson(bundle(description))
  return t.handle(
    new Request(`${BASE}/report/`, {
      method: 'POST',
      body,
      headers: {
        'content-type': 'application/gzip',
        'content-length': String(body.byteLength),
        'hew-upload-length': String(body.byteLength),
        'cf-connecting-ip': ip,
        ...headers,
      },
    }),
  )
}

function commit(t: TestEnv, id: string, token: string): Promise<Response> {
  return t.handle(new Request(`${BASE}/report/${id}/commit`, { method: 'POST', headers: { 'hew-upload-token': token } }))
}

/** A whole single-piece report, optionally followed. */
async function send(t: TestEnv, follow = true): Promise<{ id: string; token: string; follow?: string }> {
  const res = await start(t, follow ? { 'hew-follow': '1' } : {})
  assert.equal(res.status, 201, await res.clone().text())
  const body = (await res.json()) as { id: string; token: string; follow?: string }
  const c = await commit(t, body.id, body.token)
  assert.equal(c.status, 201, await c.clone().text())
  return body
}

function thread(t: TestEnv, id: string, secret: string | null, init: { method?: string; body?: unknown; path?: string; ip?: string } = {}) {
  const headers: Record<string, string> = { 'cf-connecting-ip': init.ip ?? '198.51.100.1' }
  if (secret !== null) headers['hew-follow-secret'] = secret
  if (init.body !== undefined) headers['content-type'] = 'application/json'
  return t.handle(
    new Request(`${BASE}/report/${id}/thread${init.path ?? ''}`, {
      method: init.method ?? 'GET',
      headers,
      body: init.body === undefined ? undefined : typeof init.body === 'string' ? init.body : JSON.stringify(init.body),
    }),
  )
}

function adminForm(t: TestEnv, id: string, action: string, fields: Record<string, string>, extraHeaders: Record<string, string> = {}) {
  return t.handle(
    new Request(`${BASE}/report/admin/${id}/${action}`, {
      method: 'POST',
      headers: t.adminHeaders({ 'content-type': 'application/x-www-form-urlencoded', ...extraHeaders }),
      body: new URLSearchParams(fields).toString(),
    }),
  )
}

async function adminGet(t: TestEnv, path: string): Promise<Response> {
  return t.handle(new Request(`${BASE}/report/admin/${path}`, { headers: t.adminHeaders() }))
}

function tableCount(t: TestEnv, id: string): number {
  const s = t.threadStorage(id)
  if (!s) return 0
  return s.sql.exec<{ n: number }>("SELECT COUNT(*) AS n FROM sqlite_master WHERE type = 'table'").toArray()[0].n
}

describe('START and COMMIT with Hew-Follow', () => {
  test('Hew-Follow: 1 adds a 43-character follow secret to the 201; the drop keeps only its hash', async () => {
    const t = await buildTestEnv()
    const res = await start(t, { 'hew-follow': '1' })
    assert.equal(res.status, 201)
    const body = (await res.json()) as Record<string, unknown>
    assert.deepEqual(Object.keys(body).sort(), ['follow', 'id', 'pieceBytes', 'token'])
    assert.match(body.follow as string, /^[A-Za-z0-9_-]{43}$/)
    assert.equal(body.pieceBytes, PIECE_BYTES)
    const meta = t.dropStorage(body.id as string)!.sql.exec<{ followHash: string }>('SELECT followHash FROM meta').toArray()[0]
    assert.equal(meta.followHash, await hashToken(body.follow as string))
    assert.equal(tableCount(t, body.id as string), 0, 'no thread before COMMIT')
  })

  test('without the header (or with another value) there is no follow field and no thread', async () => {
    const t = await buildTestEnv()
    for (const headers of [{}, { 'hew-follow': '0' }, { 'hew-follow': 'yes' }] as Array<Record<string, string>>) {
      const res = await start(t, headers)
      const body = (await res.json()) as Record<string, unknown>
      assert.equal('follow' in body, false)
      const c = await commit(t, body.id as string, body.token as string)
      assert.equal(c.status, 201)
      assert.equal(tableCount(t, body.id as string), 0)
      assert.equal((await t.index().getReport(body.id as string))?.following, false)
    }
  })

  test('COMMIT creates the thread and marks the row following; a retried COMMIT changes nothing', async () => {
    const t = await buildTestEnv({ overrides: { NOTIFY_TO: 'me@example.com', NOTIFY_FROM: 'bot@example.com' } })
    const { id, token, follow } = await send(t)
    const row = await t.index().getReport(id)
    assert.equal(row?.following, true)
    assert.equal(row?.status, 'open')
    assert.equal(await t.thread(id).verifySecret(await hashToken(follow!)), true)
    assert.equal(await t.threadAlarmAt(id) !== null, true)
    const again = await commit(t, id, token)
    assert.equal(again.status, 201)
    assert.equal(await t.thread(id).verifySecret(await hashToken(follow!)), true)
    assert.equal(t.mailer.calls.length, 1, 'one new-report email')
    assert.match(t.mailer.subjects()[0], /^\[Hew bug report\] HEW-.{4}-.{4} — 1\.3\.1 \/ desktop-macos$/)
  })

  test('a COMMIT whose thread creation fails is a 503 that a retry completes', async () => {
    const t = await buildTestEnv()
    const res = await start(t, { 'hew-follow': '1' })
    const { id, token, follow } = (await res.json()) as { id: string; token: string; follow: string }
    const real = t.env.REPORT_THREAD
    t.env.REPORT_THREAD = { idFromName: real.idFromName.bind(real), get: () => ({ create: async () => { throw new Error('down') } }) as never }
    assert.equal((await commit(t, id, token)).status, 503)
    assert.equal((await t.index().getReport(id))?.committed, false)
    t.env.REPORT_THREAD = real
    assert.equal((await commit(t, id, token)).status, 201)
    assert.equal((await t.index().getReport(id))?.following, true)
    assert.equal(await t.thread(id).verifySecret(await hashToken(follow)), true)
  })
})

describe('GET /report/<id>/thread', () => {
  test('200 with the §5.4 shape, no-store, and messages after the cursor', async () => {
    const t = await buildTestEnv()
    const { id, follow } = await send(t)
    assert.equal((await adminForm(t, id, 'reply', { text: 'Could you attach the file?' })).status, 303)
    assert.equal((await adminForm(t, id, 'status', { status: 'fixed', version: '1.3.2', note: '' })).status, 303)
    const res = await thread(t, id, follow!)
    assert.equal(res.status, 200)
    assert.equal(res.headers.get('cache-control'), 'no-store')
    assert.equal(res.headers.get('access-control-allow-origin'), null)
    const view = (await res.json()) as Record<string, unknown>
    assert.deepEqual(Object.keys(view).sort(), ['format', 'latest', 'messages', 'now', 'status', 'statusAt', 'statusVersion'])
    assert.equal(view.format, 1)
    assert.equal(view.status, 'fixed')
    assert.equal(view.statusVersion, '1.3.2')
    assert.equal(view.latest, 2)
    const messages = view.messages as Array<Record<string, unknown>>
    assert.deepEqual(
      messages.map((m) => [m.seq, m.author, m.kind, m.text]),
      [
        [1, 'maintainer', 'text', 'Could you attach the file?'],
        [2, 'system', 'status', 'Fixed in 1.3.2'],
      ],
    )
    const after = (await (await thread(t, id, follow!, { path: '?after=1' })).json()) as { messages: unknown[] }
    assert.equal(after.messages.length, 1)
  })

  test('every failure is the same 403: no secret, wrong secret, unknown id, malformed id, unfollowed report', async () => {
    const t = await buildTestEnv()
    const { id, follow } = await send(t)
    const plain = await send(t, false)
    const cases = [
      await thread(t, id, null),
      await thread(t, id, 'wrong'),
      await thread(t, 'HEW-ZZZZ-ZZZZ', follow!),
      await thread(t, 'not-an-id', follow!),
      await thread(t, plain.id, follow!),
    ]
    for (const res of cases) {
      assert.equal(res.status, 403)
      assert.equal(res.headers.get('cache-control'), 'no-store')
      assert.deepEqual(await res.json(), { error: 'forbidden' })
    }
    assert.equal(tableCount(t, 'HEW-ZZZZ-ZZZZ'), 0, 'an unknown id leaves no storage')
    assert.equal(tableCount(t, plain.id), 0)
  })

  test('a malformed after is 400', async () => {
    const t = await buildTestEnv()
    const { id, follow } = await send(t)
    for (const q of ['?after=-1', '?after=abc', '?after=1.5']) {
      assert.equal((await thread(t, id, follow!, { path: q })).status, 400)
    }
  })

  test('wrong secrets never touch the index; accepted requests count toward the per-client limit', async () => {
    const t = await buildTestEnv()
    const { id, follow } = await send(t)
    const countRows = () =>
      t.index()['storage'].sql.exec<{ n: number }>("SELECT COUNT(*) AS n FROM rate_window WHERE clientHash LIKE 'thread:%'").toArray()[0].n
    for (let i = 0; i < 5; i++) assert.equal((await thread(t, id, 'wrong')).status, 403)
    assert.equal(countRows(), 0)
    const bucketBefore = Math.floor(Date.now() / (10 * 60 * 1000))
    for (let i = 0; i < THREAD_RATE_WINDOW_MAX; i++) {
      const res = await thread(t, id, follow!)
      // A request at the exact window edge can land in a fresh bucket; the
      // assertion below is what matters.
      assert.ok(res.status === 200, `request ${i}: ${res.status}`)
    }
    const limited = await thread(t, id, follow!)
    const sameBucket = Math.floor(Date.now() / (10 * 60 * 1000)) === bucketBefore
    if (sameBucket) assert.equal(limited.status, 429)
    if (limited.status === 429) {
      assert.deepEqual(await limited.json(), { error: 'rate-limited' })
      assert.ok(Number(limited.headers.get('retry-after')) >= 1)
    } else {
      assert.equal(limited.status, 200, 'only acceptable when the 10-minute bucket rolled over mid-test')
    }
    assert.equal((await thread(t, id, follow!, { ip: '192.0.2.99' })).status, 200, 'another client is unaffected')
  })
})

describe('POST /report/<id>/thread', () => {
  test('201 {"seq"}; emails the maintainer; updates the row caches', async () => {
    const t = await buildTestEnv({ overrides: { NOTIFY_TO: 'me@example.com', NOTIFY_FROM: 'bot@example.com' } })
    const { id, follow } = await send(t)
    const res = await thread(t, id, follow!, { method: 'POST', body: { text: 'It happens <b>every</b> time.' } })
    assert.equal(res.status, 201)
    assert.equal(res.headers.get('cache-control'), 'no-store')
    assert.deepEqual(await res.json(), { seq: 1 })
    const row = await t.index().getReport(id)
    assert.ok(row?.reporterActivityAt)
    assert.equal(t.mailer.calls.length, 2)
    assert.equal(t.mailer.subjects()[1], `[Hew bug report] ${id} — reply from the reporter`)
    assert.ok(t.mailer.calls[1].raw.includes('It happens <b>every</b> time.'))
    assert.ok(!t.mailer.calls[1].raw.includes(follow!), 'the secret never reaches an email')
  })

  test('400 for text outside 1–4000 characters or a body that is not that JSON', async () => {
    const t = await buildTestEnv()
    const { id, follow } = await send(t)
    for (const body of [{ text: '' }, { text: '   ' }, { text: 'x'.repeat(THREAD_MESSAGE_MAX_CHARS + 1) }, { text: 5 }, {}, 'not json', '[1]']) {
      const res = await thread(t, id, follow!, { method: 'POST', body })
      assert.equal(res.status, 400, JSON.stringify(body).slice(0, 40))
      assert.equal(((await res.json()) as { error: string }).error, 'invalid')
    }
    // 4000 code points, some of them astral, is still fine.
    const exact = '😀'.repeat(10) + 'x'.repeat(THREAD_MESSAGE_MAX_CHARS - 10)
    assert.equal((await thread(t, id, follow!, { method: 'POST', body: { text: exact } })).status, 201)
  })

  test('403 for a wrong secret, with nothing stored', async () => {
    const t = await buildTestEnv()
    const { id, follow } = await send(t)
    assert.equal((await thread(t, id, 'wrong', { method: 'POST', body: { text: 'hi' } })).status, 403)
    const view = (await (await thread(t, id, follow!)).json()) as { latest: number }
    assert.equal(view.latest, 0)
  })

  test('429 with Retry-After after 20 posts in a day', async () => {
    const t = await buildTestEnv()
    const { id, follow } = await send(t)
    for (let i = 0; i < 20; i++) {
      assert.equal((await thread(t, id, follow!, { method: 'POST', body: { text: `m${i}` }, ip: `10.0.0.${i}` })).status, 201)
    }
    const res = await thread(t, id, follow!, { method: 'POST', body: { text: 'more' }, ip: '10.0.1.1' })
    assert.equal(res.status, 429)
    assert.ok(res.headers.get('retry-after'))
  })

  test('409 full on a full thread', async () => {
    const t = await buildTestEnv()
    const { id, follow } = await send(t)
    for (let i = 0; i < 200; i++) await t.thread(id).postMaintainer(`a${i}`, Date.now())
    const res = await thread(t, id, follow!, { method: 'POST', body: { text: 'hi' } })
    assert.equal(res.status, 409)
    assert.deepEqual(await res.json(), { error: 'full' })
  })

  test('409 full once only the maintainer reserve is left; the maintainer can still reply', async () => {
    const t = await buildTestEnv()
    const { id, follow } = await send(t)
    for (let i = 0; i < THREAD_MAX_MESSAGES - THREAD_MAINTAINER_RESERVE; i++) await t.thread(id).postMaintainer(`a${i}`, Date.now())
    const res = await thread(t, id, follow!, { method: 'POST', body: { text: 'hi' } })
    assert.equal(res.status, 409)
    assert.deepEqual(await res.json(), { error: 'full' })
    assert.equal((await adminForm(t, id, 'reply', { text: 'still room for me' })).status, 303)
    assert.equal((await adminForm(t, id, 'status', { status: 'closed' })).status, 303)
  })

  test('a reply on a fixed thread reopens it, and the list cache follows', async () => {
    const t = await buildTestEnv({ overrides: { NOTIFY_TO: 'me@example.com', NOTIFY_FROM: 'bot@example.com' } })
    const { id, follow } = await send(t)
    await adminForm(t, id, 'status', { status: 'fixed', version: '1.3.2' })
    assert.equal((await t.index().getReport(id))?.status, 'fixed')
    assert.equal((await t.index().getReport(id))?.statusVersion, '1.3.2')
    assert.equal((await thread(t, id, follow!, { method: 'POST', body: { text: 'still broken' } })).status, 201)
    const row = await t.index().getReport(id)
    assert.equal(row?.status, 'open')
    assert.equal(row?.statusVersion, null)
    const view = (await (await thread(t, id, follow!)).json()) as { status: string; messages: Array<{ text: string; kind: string }> }
    assert.equal(view.status, 'open')
    assert.equal(view.messages.at(-1)?.kind, 'status')
    assert.ok(t.mailer.calls.at(-1)!.raw.includes('reopened'))
  })
})

describe('POST /report/<id>/thread/seen', () => {
  test('204; clamps past latest; seeing a final status arms the 30-day grace', async () => {
    const t = await buildTestEnv()
    const { id, follow } = await send(t)
    await adminForm(t, id, 'status', { status: 'closed', note: 'Duplicate of HEW-AAAA-BBBB.' })
    const res = await thread(t, id, follow!, { method: 'POST', path: '/seen', body: { upTo: 999 } })
    assert.equal(res.status, 204)
    assert.equal(res.headers.get('cache-control'), 'no-store')
    const admin = await t.thread(id).adminView(Date.now())
    assert.equal(admin?.reporterSeenUpTo, 1)
    assert.ok(admin?.finalSeenAt)
    assert.equal(await t.threadAlarmAt(id), admin!.finalSeenAt! + THREAD_FINAL_SEEN_GRACE_MS)
  })

  test('400 for a bad body, 403 for a wrong secret', async () => {
    const t = await buildTestEnv()
    const { id, follow } = await send(t)
    for (const body of [{}, { upTo: -1 }, { upTo: 1.5 }, { upTo: '3' }, 'nope']) {
      assert.equal((await thread(t, id, follow!, { method: 'POST', path: '/seen', body })).status, 400)
    }
    assert.equal((await thread(t, id, 'wrong', { method: 'POST', path: '/seen', body: { upTo: 0 } })).status, 403)
  })
})

describe('DELETE /report/<id>/thread (Forget)', () => {
  test('204, then 403; the thread is gone and the row stops showing following', async () => {
    const t = await buildTestEnv()
    const { id, follow } = await send(t)
    const res = await thread(t, id, follow!, { method: 'DELETE' })
    assert.equal(res.status, 204)
    assert.equal(tableCount(t, id), 0)
    const row = await t.index().getReport(id)
    assert.equal(row?.following, false)
    assert.ok(row, 'the report itself stays with the maintainer')
    assert.equal((await thread(t, id, follow!, { method: 'DELETE' })).status, 403)
    assert.equal((await thread(t, id, follow!)).status, 403)
  })

  test('403 for a wrong secret, and the thread survives', async () => {
    const t = await buildTestEnv()
    const { id, follow } = await send(t)
    assert.equal((await thread(t, id, 'wrong', { method: 'DELETE' })).status, 403)
    assert.equal((await thread(t, id, follow!)).status, 200)
  })
})

describe('thread lifetime against the index (§5.2, §5.5)', () => {
  test('the bundle alarm keeps a followed row (bundleDeleted, sizeBytes 0) and deletes an unfollowed one', async () => {
    const t = await buildTestEnv()
    const followed = await send(t)
    const plain = await send(t, false)
    assert.equal(await t.dropAlarmAt(followed.id) !== null, true)
    await t.runDropAlarm(followed.id)
    await t.runDropAlarm(plain.id)
    const row = await t.index().getReport(followed.id)
    assert.equal(row?.bundleDeleted, true)
    assert.equal(row?.sizeBytes, 0)
    assert.equal(await t.index().totalStoredBytes(), 0)
    assert.equal(await t.index().getReport(plain.id), null)
    // Detail page still renders, with no download link; download is 404.
    const detail = await adminGet(t, followed.id)
    assert.equal(detail.status, 200)
    const html = await detail.text()
    assert.ok(!html.includes('/download'))
    assert.ok(html.includes('bundle has expired'))
    assert.equal((await adminGet(t, `${followed.id}/download`)).status, 404)
    // The maintainer can still set "fixed in" after day 90, and the reporter reads it.
    assert.equal((await adminForm(t, followed.id, 'status', { status: 'fixed', version: '1.3.2' })).status, 303)
    const view = (await (await thread(t, followed.id, followed.follow!)).json()) as { status: string }
    assert.equal(view.status, 'fixed')
    // When the thread then ends, the bundle-less row goes too.
    await t.runThreadAlarm(followed.id, Date.now() + THREAD_IDLE_MS + 1)
    assert.equal(await t.index().getReport(followed.id), null)
    assert.equal(tableCount(t, followed.id), 0)
  })

  test('a thread ending while the bundle lives flips following to 0 and keeps the row', async () => {
    const t = await buildTestEnv()
    const { id } = await send(t)
    const due = await t.threadAlarmAt(id)
    assert.ok(due !== null && due <= Date.now() + THREAD_IDLE_MS + 1000)
    await t.runThreadAlarm(id, due!)
    const row = await t.index().getReport(id)
    assert.equal(row?.following, false)
    assert.equal(row?.bundleDeleted, false)
    // And a later bundle alarm then deletes it, as for any unfollowed report.
    await t.runDropAlarm(id)
    assert.equal(await t.index().getReport(id), null)
  })

  test('the bundle alarm on an unfollowed report also destroys the admin-only thread a reply created', async () => {
    const t = await buildTestEnv()
    const plain = await send(t, false)
    const untouched = await send(t, false)
    assert.equal((await adminForm(t, plain.id, 'reply', { text: 'note to self' })).status, 303)
    assert.ok(tableCount(t, plain.id) > 0)
    assert.ok((await t.threadAlarmAt(plain.id)) !== null)
    await t.runDropAlarm(plain.id)
    assert.equal(await t.index().getReport(plain.id), null)
    assert.equal(tableCount(t, plain.id), 0, 'no thread outlives its row')
    assert.equal(await t.threadAlarmAt(plain.id), null)
    // A report that never had a thread: the destroy writes nothing.
    await t.runDropAlarm(untouched.id)
    assert.equal(tableCount(t, untouched.id), 0)
    // A followed report keeps its row, so its thread stays.
    const followed = await send(t)
    await t.runDropAlarm(followed.id)
    assert.ok(tableCount(t, followed.id) > 0)
  })

  test('a failed thread destroy after the row went keeps the drop for the retried alarm', async () => {
    const t = await buildTestEnv()
    const plain = await send(t, false)
    await adminForm(t, plain.id, 'reply', { text: 'note to self' })
    const realDestroy = t.thread(plain.id).destroy
    t.thread(plain.id).destroy = async () => {
      throw new Error('thread DO down')
    }
    await assert.rejects(t.runDropAlarm(plain.id))
    assert.ok((await t.drop(plain.id).head()) !== null, 'bytes kept for the retry')
    t.thread(plain.id).destroy = realDestroy
    await t.runDropAlarm(plain.id)
    assert.equal(tableCount(t, plain.id), 0)
    assert.equal(await t.drop(plain.id).head(), null)
  })

  test('the drop retention alarm is still 90 days from receipt', async () => {
    const t = await buildTestEnv()
    const { id } = await send(t)
    const head = await t.drop(id).head()
    assert.equal(await t.dropAlarmAt(id), head!.receivedAt + RETENTION_MS)
  })
})

describe('follow-up attachments (§5.6)', () => {
  async function followUp(t: TestEnv, parentId: string, secret: string | null, extra: Record<string, string> = {}) {
    const headers: Record<string, string> = { 'hew-follow-up': parentId, ...extra }
    if (secret !== null) headers['hew-follow-secret'] = secret
    return start(t, headers, `File for report ${parentId}: kitchen.skp\nThe one from the 18th.`, '198.51.100.50')
  }

  test('admin delete of a parent deletes its follow-up attachments too', async () => {
    const t = await buildTestEnv()
    const { id, token: _token, follow } = await send(t)
    const res = await followUp(t, id, follow!)
    assert.equal(res.status, 201)
    const child = (await res.json()) as { id: string; token: string }
    assert.equal((await commit(t, child.id, child.token)).status, 201)
    assert.equal((await t.index().listChildren(id)).length, 1)
    const del = await t.handle(new Request(`${BASE}/report/admin/${id}/delete`, { method: 'POST', headers: t.adminHeaders() }))
    assert.equal(del.status, 303)
    assert.equal(await t.index().getReport(child.id), null)
    assert.equal(await t.drop(child.id).head(), null)
    const list = await (await adminGet(t, '')).text()
    assert.ok(!list.includes(child.id))
    assert.ok(!list.includes('for <'))
  })

  test('a follow-up START with a bad secret, no secret, or a malformed/unknown parent is 403 before any reservation', async () => {
    const t = await buildTestEnv()
    const { id, follow } = await send(t)
    const plain = await send(t, false)
    for (const res of [
      await followUp(t, id, 'wrong'),
      await followUp(t, id, null),
      await followUp(t, 'garbage', follow!),
      await followUp(t, 'HEW-ZZZZ-ZZZZ', follow!),
      await followUp(t, plain.id, follow!),
    ]) {
      assert.equal(res.status, 403)
      assert.deepEqual(await res.json(), { error: 'forbidden' })
    }
    const reports = await t.index().listReports()
    assert.equal(reports.length, 2, 'nothing new reserved')
    const rows = t.index()['storage'].sql.exec<{ n: number }>('SELECT COUNT(*) AS n FROM reports').toArray()[0].n
    assert.equal(rows, 2)
  })

  test('the full path: parentId recorded, Hew-Follow ignored, attachment message, need-info → open, attachment email only', async () => {
    const t = await buildTestEnv({ overrides: { NOTIFY_TO: 'me@example.com', NOTIFY_FROM: 'bot@example.com' } })
    const { id, follow } = await send(t)
    await adminForm(t, id, 'reply', { canned: 'ask-file' })
    await adminForm(t, id, 'status', { status: 'need-info' })
    // The maintainer opens the page, so nothing is "new" yet.
    await adminGet(t, id)
    const mailsBefore = t.mailer.calls.length

    const res = await followUp(t, id, follow!, { 'hew-follow': '1' })
    assert.equal(res.status, 201)
    const child = (await res.json()) as Record<string, string>
    assert.equal('follow' in child, false, 'a follow-up never gets its own thread')
    const childRow = await t.index().getReport(child.id)
    assert.equal(childRow?.parentId, id)
    assert.equal((await commit(t, child.id, child.token)).status, 201)
    assert.equal((await commit(t, child.id, child.token)).status, 201, 'a retried COMMIT is fine')
    assert.equal(tableCount(t, child.id), 0)

    const view = (await (await thread(t, id, follow!)).json()) as {
      status: string
      messages: Array<{ kind: string; author: string; text: string; attachmentReportId?: string }>
    }
    assert.equal(view.status, 'open')
    const attachments = view.messages.filter((m) => m.kind === 'attachment')
    assert.equal(attachments.length, 1, 'appended once despite the retry')
    assert.equal(attachments[0].author, 'reporter')
    assert.equal(attachments[0].attachmentReportId, child.id)
    assert.match(attachments[0].text, /^kitchen\.skp \(\d+ B\)$/)

    const newMails = t.mailer.subjects().slice(mailsBefore)
    assert.equal(newMails.length, 1, 'no new-report email for the follow-up, and none on the retry')
    assert.match(newMails[0], new RegExp(`^\\[Hew bug report\\] ${id} — file attached \\(\\d+ B\\)$`))

    const parentRow = await t.index().getReport(id)
    assert.equal(parentRow?.status, 'open')
    const list = await (await adminGet(t, '')).text()
    assert.ok(list.includes(`↳ for <a href="/report/admin/${id}">${id}</a>`))
    assert.match(list, new RegExp(`${id}</code></a> <span class="new">new</span>`))
    const detail = await (await adminGet(t, id)).text()
    assert.ok(detail.includes(`/report/admin/${child.id}/download`), 'listed under its parent')
    const childDetail = await (await adminGet(t, child.id)).text()
    assert.ok(childDetail.includes(`↳ for <a href="/report/admin/${id}">`))
    assert.ok(!childDetail.includes('Send reply'), 'an attachment has no thread of its own')
  })

  test('a follow-up START needs a committed parent row, not just a thread that verifies', async () => {
    const t = await buildTestEnv()
    const a = await send(t)
    const b = await send(t)
    // Row uncommitted (cannot happen through the routes, but the thread alone must not suffice).
    t.index()['storage'].sql.exec('UPDATE reports SET committed = 0 WHERE id = ?', a.id)
    // Row gone while the thread is still there.
    await t.index().deleteReport(b.id)
    assert.equal(await t.thread(a.id).verifySecret(await hashToken(a.follow!)), true)
    assert.equal(await t.thread(b.id).verifySecret(await hashToken(b.follow!)), true)
    for (const res of [await followUp(t, a.id, a.follow!), await followUp(t, b.id, b.follow!)]) {
      assert.equal(res.status, 403)
      assert.deepEqual(await res.json(), { error: 'forbidden' })
    }
    const rows = t.index()['storage'].sql.exec<{ n: number }>('SELECT COUNT(*) AS n FROM reports').toArray()[0].n
    assert.equal(rows, 1, 'nothing new reserved')
  })

  test('an attachment on a fixed thread keeps the status but stops the 30-day clock', async () => {
    const t = await buildTestEnv()
    const { id, follow } = await send(t)
    await adminForm(t, id, 'status', { status: 'fixed', version: '1.3.2' })
    await thread(t, id, follow!, { method: 'POST', path: '/seen', body: { upTo: 1 } })
    assert.ok((await t.thread(id).adminView(Date.now()))?.finalSeenAt !== null)
    const res = await followUp(t, id, follow!)
    const child = (await res.json()) as { id: string; token: string }
    assert.equal((await commit(t, child.id, child.token)).status, 201)
    const v = await t.thread(id).adminView(Date.now())
    assert.equal(v?.status, 'fixed')
    assert.equal(v?.finalSeenAt, null)
    assert.equal(await t.threadAlarmAt(id), v!.lastActivityAt + THREAD_IDLE_MS)
    assert.equal((await t.index().getReport(id))?.status, 'fixed')
  })

  test('follow-ups count against the submission rate limit', async () => {
    const t = await buildTestEnv()
    const { id, follow } = await send(t)
    let last = 0
    for (let i = 0; i <= RATE_LIMIT_WINDOW_MAX; i++) last = (await followUp(t, id, follow!)).status
    assert.equal(last, 429)
  })

  test('a follow-up whose parent thread ended before COMMIT still commits, as a plain child report', async () => {
    const t = await buildTestEnv()
    const { id, follow } = await send(t)
    const res = await followUp(t, id, follow!)
    const child = (await res.json()) as { id: string; token: string }
    await thread(t, id, follow!, { method: 'DELETE' })
    assert.equal((await commit(t, child.id, child.token)).status, 201)
    assert.equal((await t.index().getReport(child.id))?.parentId, id)
  })

  test('attachmentLabel reads the file name from the description, with a fallback', () => {
    assert.equal(attachmentLabel('File for report HEW-7K3F-Q9XB: kitchen.skp\nnote', 48_758_000), 'kitchen.skp (46.5 MB)')
    assert.equal(attachmentLabel('Something else entirely here', 2048), 'A file (2.0 KB)')
    assert.equal(attachmentLabel('file for report hew-7k3f-q9xb:   spaced name.skp   \r\nmore', 1024), 'spaced name.skp (1.0 KB)')
    assert.equal(attachmentLabel('File for report HEW-7K3F-Q9XB:    ', 1024), 'A file (1.0 KB)')
    assert.equal(attachmentLabel(`File for report HEW-7K3F-Q9XB: ${'x'.repeat(500)}`, 1), `${'x'.repeat(200)} (1 B)`)
  })

  test('attachmentLabel is linear on a long run of spaces', () => {
    const hostile = `File for report HEW-7K3F-Q9XB:${' '.repeat(10_000)}x${' '.repeat(10_000)}!`
    const started = performance.now()
    const label = attachmentLabel(hostile, 1)
    const elapsed = performance.now() - started
    assert.ok(elapsed < 50, `took ${elapsed.toFixed(1)} ms`)
    assert.ok(label.startsWith('x'))
    const startedPlain = performance.now()
    attachmentLabel(`${' '.repeat(10_000)}x`, 1)
    assert.ok(performance.now() - startedPlain < 50)
  })
})

describe('admin reply and status forms', () => {
  test('reply: free text, both canned replies, and refusals', async () => {
    const t = await buildTestEnv()
    const { id, follow } = await send(t)
    const r1 = await adminForm(t, id, 'reply', { text: 'Thanks — looking into it.' })
    assert.equal(r1.status, 303)
    assert.equal(r1.headers.get('location'), `https://app.hew3d.com/report/admin/${id}`)
    assert.equal(r1.headers.get('cache-control'), 'no-store')
    assert.equal((await adminForm(t, id, 'reply', { text: 'ignored', canned: 'ask-file' })).status, 303)
    assert.equal((await adminForm(t, id, 'reply', { canned: 'ask-steps' })).status, 303)
    assert.equal((await adminForm(t, id, 'reply', { text: '' })).status, 400)
    assert.equal((await adminForm(t, id, 'reply', { canned: 'toString' })).status, 400)
    assert.equal((await adminForm(t, id, 'reply', { text: 'x' }, { origin: 'https://evil.example' })).status, 403)
    assert.equal((await adminForm(t, 'HEW-ZZZZ-ZZZZ', 'reply', { text: 'x' })).status, 404)
    const view = (await (await thread(t, id, follow!)).json()) as { messages: Array<{ author: string; text: string }> }
    assert.deepEqual(
      view.messages.map((m) => m.text),
      ['Thanks — looking into it.', CANNED_REPLIES['ask-file'].text, CANNED_REPLIES['ask-steps'].text],
    )
    assert.ok(view.messages.every((m) => m.author === 'maintainer'))
    assert.ok(CANNED_REPLIES['ask-file'].text.startsWith("Could you attach the file that wouldn't open? In Hew, open Help ▸ Sent Reports"))
  })

  test('status: every transition, version required for fixed, list caches updated', async () => {
    const t = await buildTestEnv()
    const { id, follow } = await send(t)
    assert.equal((await adminForm(t, id, 'status', { status: 'need-info' })).status, 303)
    assert.equal((await t.index().getReport(id))?.status, 'need-info')
    assert.equal((await adminForm(t, id, 'status', { status: 'fixed', version: '' })).status, 400)
    assert.equal((await adminForm(t, id, 'status', { status: 'bogus' })).status, 400)
    assert.equal((await adminForm(t, id, 'status', { status: 'fixed', version: 'v'.repeat(65) })).status, 400)
    assert.equal((await adminForm(t, id, 'status', { status: 'fixed', version: '1.3.2' })).status, 303)
    assert.equal((await t.index().getReport(id))?.statusVersion, '1.3.2')
    assert.equal((await adminForm(t, id, 'status', { status: 'closed', version: '1.3.2', note: 'Thanks!' })).status, 303)
    assert.equal((await adminForm(t, id, 'status', { status: 'open' })).status, 303)
    const view = (await (await thread(t, id, follow!)).json()) as { status: string; statusVersion: unknown; messages: Array<{ text: string }> }
    assert.equal(view.status, 'open')
    assert.equal(view.statusVersion, null)
    assert.deepEqual(view.messages.map((m) => m.text), ['Needs more information', 'Fixed in 1.3.2', 'Thanks!', 'Marked open'])
    const list = await (await adminGet(t, '')).text()
    assert.ok(list.includes('<td>open</td>'))
  })

  test('a status change stands even when the list cache update fails: 303, one status message', async () => {
    const t = await buildTestEnv()
    const { id } = await send(t)
    const index = t.index()
    const real = index.setThreadCache
    index.setThreadCache = async () => {
      throw new Error('index down')
    }
    try {
      assert.equal((await adminForm(t, id, 'status', { status: 'closed' })).status, 303)
    } finally {
      index.setThreadCache = real
    }
    const v = await t.thread(id).adminView(Date.now())
    assert.equal(v?.status, 'closed')
    assert.equal(v?.messages.filter((m) => m.kind === 'status').length, 1)
    assert.equal((await t.index().getReport(id))?.status, 'open', 'the cache write really failed')
  })

  test('a thread ending resets the list status cache', async () => {
    const t = await buildTestEnv()
    const { id, follow } = await send(t)
    await adminForm(t, id, 'status', { status: 'fixed', version: '1.3.2' })
    assert.equal((await t.index().getReport(id))?.status, 'fixed')
    assert.equal((await thread(t, id, follow!, { method: 'DELETE' })).status, 204)
    const row = await t.index().getReport(id)
    assert.equal(row?.status, 'open')
    assert.equal(row?.statusVersion, null)
  })

  test('a report nobody follows still takes a reply and a status, kept for the maintainer only', async () => {
    const t = await buildTestEnv()
    const plain = await send(t, false)
    assert.equal((await adminForm(t, plain.id, 'reply', { text: 'note to self' })).status, 303)
    assert.equal((await adminForm(t, plain.id, 'status', { status: 'closed' })).status, 303)
    const html = await (await adminGet(t, plain.id)).text()
    assert.ok(html.includes('Following: no'))
    assert.ok(html.includes('No one is following this report.'))
    assert.ok(html.includes('note to self'))
    assert.equal((await t.index().getReport(plain.id))?.following, false)
    assert.equal((await t.index().getReport(plain.id))?.status, 'closed')
  })

  test('detail escapes reporter text and marks it new until viewed; list shows the new marker', async () => {
    const t = await buildTestEnv()
    const { id, follow } = await send(t)
    await thread(t, id, follow!, { method: 'POST', body: { text: '<script>alert(1)</script>' } })
    const list = await (await adminGet(t, '')).text()
    assert.ok(list.includes('<span class="new">new</span>'))
    assert.ok(list.includes('<th>Status</th><th>Following</th>'))
    const res = await adminGet(t, id)
    assert.equal(res.status, 200)
    const html = await res.text()
    assert.ok(!html.includes('<script>alert(1)</script>'))
    assert.ok(html.includes('&lt;script&gt;alert(1)&lt;/script&gt;'))
    assert.ok(html.includes('Following: yes'))
    assert.ok(html.includes('name="canned" value="ask-file"'))
    assert.ok(html.includes('name="canned" value="ask-steps"'))
    assert.ok(!/<script/i.test(html), 'no script on admin pages')
    const row = await t.index().getReport(id)
    assert.ok(row?.adminViewedAt && row.adminViewedAt >= row.reporterActivityAt!)
    const list2 = await (await adminGet(t, '')).text()
    assert.ok(!list2.includes('<span class="new">new</span>'))
    assert.equal((await t.thread(id).adminView(Date.now()))?.previousAdminSeenUpTo, 1)
  })

  test('admin delete destroys the thread too: the reporter key stops working at once', async () => {
    const t = await buildTestEnv()
    const { id, follow } = await send(t)
    const del = await t.handle(new Request(`${BASE}/report/admin/${id}/delete`, { method: 'POST', headers: t.adminHeaders() }))
    assert.equal(del.status, 303)
    assert.equal(tableCount(t, id), 0)
    assert.equal((await thread(t, id, follow!)).status, 403)
    assert.equal(await t.index().getReport(id), null)
  })

  test('the reply and status routes need Access like every admin route', async () => {
    const t = await buildTestEnv()
    const { id } = await send(t)
    const res = await t.handle(
      new Request(`${BASE}/report/admin/${id}/reply`, {
        method: 'POST',
        headers: { origin: 'https://app.hew3d.com', 'content-type': 'application/x-www-form-urlencoded' },
        body: 'text=hi',
      }),
    )
    assert.equal(res.status, 403)
  })
})

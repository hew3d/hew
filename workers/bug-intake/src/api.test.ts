/**
 * The read-only report API (`GET /report/admin/api/reports` and
 * `GET /report/admin/api/reports/<id>`), end to end through `handleRequest`
 * on the in-process Worker from `testSupport/testEnv.ts`, plus the store
 * behavior the routes depend on: `threadActivityAt` on the index and
 * `ThreadStore.peek`. Each test name starts with the row of the design's
 * acceptance table it covers.
 *
 * Handlers take their time from `Date.now()`, so "at a later time" means
 * replacing `Date.now` for one call (`at`). Access verification runs on the
 * harness's own fixed clock and is unaffected.
 *
 * "Writes nothing" is checked with SQLite's `total_changes()`, which also
 * counts an update that stores the value already there. Thread state is read
 * with SQL on the thread's storage, never through `ThreadStore.adminView`,
 * which writes.
 */

import { test, describe, type TestContext } from 'node:test'
import assert from 'node:assert/strict'

import { buildTestEnv, type TestEnv } from './testSupport/testEnv.ts'
import { FakeDurableObjectStorage } from './testSupport/fakeDurableObject.ts'
import { IndexStore } from './indexStore.ts'
import { THREAD_MAX_MESSAGES } from './constants.ts'

const PUBLIC_BASE = 'https://app.hew3d.com'
const ADMIN_BASE = 'https://reports.hew3d.com'
const ADMIN = `${ADMIN_BASE}/report/admin`
const LIST = `${ADMIN}/api/reports`
const UNKNOWN_ID = 'HEW-ZZZZ-ZZZZ'
const MINUTE = 60_000

/** The keys of one API row (decision 10). */
const ROW_KEYS = [
  'id',
  'receivedAt',
  'appVersion',
  'platform',
  'descriptionPreview',
  'sizeBytes',
  'triaged',
  'committed',
  'following',
  'status',
  'statusVersion',
  'reporterActivityAt',
  'adminViewedAt',
  'parentId',
  'bundleDeleted',
  'lastActivity',
]

/** The keys of one API message (decision 19). */
const MESSAGE_KEYS = ['seq', 'at', 'author', 'kind', 'text', 'attachmentReportId']

const DETAIL_KEYS = ['generatedAt', 'report', 'description', 'messages']

interface ApiRow {
  id: string
  receivedAt: number
  appVersion: string
  platform: string
  descriptionPreview: string
  sizeBytes: number
  triaged: boolean
  committed: boolean
  following: boolean
  status: string
  statusVersion: string | null
  reporterActivityAt: number | null
  adminViewedAt: number | null
  parentId: string | null
  bundleDeleted: boolean
  lastActivity: number
}

interface ApiMessage {
  seq: number
  at: number
  author: string
  kind: string
  text: string
  attachmentReportId: string | null
}

interface ApiList {
  generatedAt: number
  reports: ApiRow[]
}

interface ApiDetail {
  generatedAt: number
  report: ApiRow
  description: string | null
  messages: ApiMessage[]
}

// ---------------------------------------------------------------------------
// Clock
// ---------------------------------------------------------------------------

/** Runs `fn` with `Date.now()` answering `ms`. */
async function at<T>(ms: number, fn: () => Promise<T>): Promise<T> {
  const real = Date.now
  Date.now = () => ms
  try {
    return await fn()
  } finally {
    Date.now = real
  }
}

/** Runs `fn` with a clock that moves 1 ms on every read, so two separate
 *  reads of `Date.now()` can never agree by luck. */
async function ticking<T>(start: number, fn: () => Promise<T>): Promise<T> {
  const real = Date.now
  let n = start
  Date.now = () => n++
  try {
    return await fn()
  } finally {
    Date.now = real
  }
}

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

async function gzipBytes(text: string): Promise<Uint8Array> {
  const cs = new CompressionStream('gzip')
  const writer = cs.writable.getWriter()
  const out = new Response(cs.readable).arrayBuffer()
  await writer.write(new TextEncoder().encode(text))
  await writer.close()
  return new Uint8Array(await out)
}

function bundle(report: Record<string, unknown>, extra: Record<string, unknown> = {}) {
  return { format: 1, report, system: { appVersion: '1.5.0', platform: 'desktop-macos' }, ...extra }
}

const DESCRIPTION = 'the model disappeared after undo, reproducible'

let ipCounter = 0

async function startUpload(t: TestEnv, headers: Record<string, string> = {}, doc: unknown = bundle({ description: DESCRIPTION })) {
  const body = await gzipBytes(JSON.stringify(doc))
  ipCounter++
  const res = await t.handle(
    new Request(`${PUBLIC_BASE}/report/`, {
      method: 'POST',
      body,
      headers: {
        'content-type': 'application/gzip',
        'content-length': String(body.byteLength),
        'hew-upload-length': String(body.byteLength),
        'cf-connecting-ip': `198.51.100.${(ipCounter % 250) + 1}`,
        ...headers,
      },
    }),
  )
  assert.equal(res.status, 201, await res.clone().text())
  return (await res.json()) as { id: string; token: string; follow?: string }
}

async function commit(t: TestEnv, upload: { id: string; token: string }): Promise<void> {
  const res = await t.handle(
    new Request(`${PUBLIC_BASE}/report/${upload.id}/commit`, { method: 'POST', headers: { 'hew-upload-token': upload.token } }),
  )
  assert.equal(res.status, 201, await res.clone().text())
}

/** A committed, unfollowed report. */
async function plainReport(t: TestEnv, doc?: unknown) {
  const upload = await startUpload(t, {}, doc)
  await commit(t, upload)
  return upload
}

/** A committed, followed report. */
async function followedReport(t: TestEnv, doc?: unknown) {
  const upload = await startUpload(t, { 'hew-follow': '1' }, doc)
  await commit(t, upload)
  return upload as { id: string; token: string; follow: string }
}

/** A follow-up attachment, started and not yet committed. */
function startAttachment(t: TestEnv, parentId: string, secret: string) {
  return startUpload(
    t,
    { 'hew-follow-up': parentId, 'hew-follow-secret': secret },
    bundle({ description: `File for report ${parentId}: kitchen.skp` }),
  )
}

async function attachment(t: TestEnv, parentId: string, secret: string) {
  const upload = await startAttachment(t, parentId, secret)
  await commit(t, upload)
  return upload
}

/** One reporter thread request. */
function threadRequest(t: TestEnv, id: string, secret: string, method: string, path = '', body?: unknown) {
  const headers: Record<string, string> = { 'cf-connecting-ip': '192.0.2.10', 'hew-follow-secret': secret }
  if (body !== undefined) headers['content-type'] = 'application/json'
  return t.handle(
    new Request(`${PUBLIC_BASE}/report/${id}/thread${path}`, {
      method,
      headers,
      body: body === undefined ? undefined : JSON.stringify(body),
    }),
  )
}

async function reporterPost(t: TestEnv, id: string, secret: string, text: string): Promise<void> {
  const res = await threadRequest(t, id, secret, 'POST', '', { text })
  assert.equal(res.status, 201, await res.clone().text())
}

async function forget(t: TestEnv, id: string, secret: string): Promise<void> {
  assert.equal((await threadRequest(t, id, secret, 'DELETE')).status, 204)
}

function adminForm(t: TestEnv, path: string, fields: Record<string, string> = {}) {
  return t.handle(
    new Request(`${ADMIN}/${path}`, {
      method: 'POST',
      headers: t.adminHeaders({ 'content-type': 'application/x-www-form-urlencoded' }),
      body: new URLSearchParams(fields).toString(),
    }),
  )
}

async function adminPost(t: TestEnv, path: string, fields: Record<string, string> = {}): Promise<void> {
  const res = await adminForm(t, path, fields)
  assert.equal(res.status, 303, `${path}: ${await res.clone().text()}`)
}

function reply(t: TestEnv, id: string, text = 'Could you attach the file?') {
  return adminPost(t, `${id}/reply`, { text })
}

function setStatus(t: TestEnv, id: string, status: string, version = '') {
  return adminPost(t, `${id}/status`, { status, version })
}

async function adminDetailPage(t: TestEnv, id: string): Promise<string> {
  const res = await t.handle(new Request(`${ADMIN}/${id}`, { headers: t.adminHeaders() }))
  assert.equal(res.status, 200)
  return res.text()
}

// ---------------------------------------------------------------------------
// The API
// ---------------------------------------------------------------------------

function api(t: TestEnv, url: string, init: { method?: string; headers?: Record<string, string> } = {}) {
  return t.handle(new Request(url, { method: init.method ?? 'GET', headers: init.headers ?? t.adminHeaders() }))
}

/** A 200 with the headers `jsonResponse` gives, parsed. */
async function okJson<T>(res: Response): Promise<{ body: T; text: string }> {
  const text = await res.text()
  assert.equal(res.status, 200, `expected 200, got ${res.status}: ${text}`)
  assert.equal(res.headers.get('content-type'), 'application/json')
  assert.equal(res.headers.get('cache-control'), 'no-store')
  return { body: JSON.parse(text) as T, text }
}

async function getList(t: TestEnv): Promise<ApiList> {
  return (await okJson<ApiList>(await api(t, LIST))).body
}

async function getDetail(t: TestEnv, id: string): Promise<ApiDetail> {
  return (await okJson<ApiDetail>(await api(t, `${LIST}/${id}`))).body
}

async function listRow(t: TestEnv, id: string): Promise<ApiRow> {
  const row = (await getList(t)).reports.find((r) => r.id === id)
  assert.ok(row, `${id} is on the list`)
  return row
}

/** `lastActivity` as the list gives it, after checking the detail agrees. */
async function lastActivity(t: TestEnv, id: string): Promise<number> {
  const row = await listRow(t, id)
  assert.equal((await getDetail(t, id)).report.lastActivity, row.lastActivity, `detail and list agree for ${id}`)
  return row.lastActivity
}

async function assertJson404(res: Response, what: string): Promise<void> {
  const text = await res.text()
  assert.equal(res.status, 404, `${what}: ${text}`)
  assert.equal(res.headers.get('content-type'), 'application/json', `${what}: ${text}`)
  assert.equal(res.headers.get('cache-control'), 'no-store', what)
  assert.deepEqual(JSON.parse(text), { error: 'not-found' }, what)
}

// ---------------------------------------------------------------------------
// Storage inspection
// ---------------------------------------------------------------------------

function indexSql(t: TestEnv) {
  return t.index()['storage'].sql
}

function indexRow(t: TestEnv, id: string): Record<string, unknown> {
  return { ...indexSql(t).exec('SELECT * FROM reports WHERE id = ?', id).toArray()[0] }
}

/** The stored column itself (`undefined` when there is no such row). */
async function threadActivityAt(t: TestEnv, id: string): Promise<number | null | undefined> {
  const row = (await t.index().getReport(id)) as { threadActivityAt?: number | null } | null
  assert.ok(row, `${id} has a row`)
  return row.threadActivityAt
}

function threadSql(t: TestEnv, id: string) {
  const storage = t.threadStorage(id)
  assert.ok(storage, `${id} has thread storage`)
  return storage.sql
}

function threadMetaRow(t: TestEnv, id: string): Record<string, unknown> {
  return { ...threadSql(t, id).exec('SELECT * FROM meta').toArray()[0] }
}

function threadMessageRows(t: TestEnv, id: string) {
  return threadSql(t, id)
    .exec<{ seq: number; at: number; author: string; kind: string; text: string; attachmentReportId: string | null }>(
      'SELECT seq, at, author, kind, text, attachmentReportId FROM messages ORDER BY seq',
    )
    .toArray()
    .map((r) => ({ ...r }))
}

/** 0 for a thread that was never constructed, too. */
function threadTableCount(t: TestEnv, id: string): number {
  const storage = t.threadStorage(id)
  if (!storage) return 0
  return storage.sql.exec<{ n: number }>("SELECT COUNT(*) AS n FROM sqlite_master WHERE type = 'table'").toArray()[0].n
}

/** Rows inserted, updated or deleted on this storage since it was made. */
function totalChanges(storage: { sql: FakeDurableObjectStorage['sql'] }): number {
  return storage.sql.exec<{ n: number }>('SELECT total_changes() AS n').toArray()[0].n
}

// ---------------------------------------------------------------------------
// The list
// ---------------------------------------------------------------------------

describe('GET /report/admin/api/reports', () => {
  test('1: every committed report, newest first, the child included and the uncommitted one absent', async () => {
    const t = await buildTestEnv()
    const base = Date.now()
    const first = await at(base - 5 * MINUTE, () => plainReport(t))
    const parent = await at(base - 4 * MINUTE, () => followedReport(t))
    const uncommitted = await at(base - 3 * MINUTE, () => startUpload(t))
    const third = await at(base - 2 * MINUTE, () => plainReport(t))
    const child = await at(base - MINUTE, () => attachment(t, parent.id, parent.follow))
    // No filter: a triaged report and a fixed one stay listed.
    await adminPost(t, `${first.id}/triage`)
    await setStatus(t, third.id, 'fixed', '1.5.1')

    const { body } = await at(base, async () => okJson<ApiList>(await api(t, LIST)))
    assert.deepEqual(Object.keys(body).sort(), ['generatedAt', 'reports'])
    assert.equal(typeof body.generatedAt, 'number')
    assert.equal(body.generatedAt, base, 'epoch milliseconds')
    assert.equal(body.reports.length, 4)
    assert.deepEqual(
      body.reports.map((r) => r.id),
      [child.id, third.id, parent.id, first.id],
    )
    assert.ok(!body.reports.some((r) => r.id === uncommitted.id))
    assert.equal(body.reports[0].parentId, parent.id)
    assert.deepEqual(
      body.reports.slice(1).map((r) => r.parentId),
      [null, null, null],
    )
  })

  test('1 (decision 9): an empty index lists no reports', async () => {
    const t = await buildTestEnv()
    assert.deepEqual((await getList(t)).reports, [])
  })

  test('1 (decision 5): neither route checks Origin', async () => {
    const t = await buildTestEnv()
    const r = await plainReport(t)
    for (const origin of [undefined, 'https://evil.example', PUBLIC_BASE]) {
      const headers: Record<string, string> = { 'cf-access-jwt-assertion': t.access!.token }
      if (origin !== undefined) headers.origin = origin
      await okJson(await api(t, LIST, { headers }))
      await okJson(await api(t, `${LIST}/${r.id}`, { headers }))
    }
  })

  test('2: a list row has exactly the sixteen keys, with the values the index entry has', async () => {
    const t = await buildTestEnv()
    const base = Date.now()
    const parent = await at(base - 10 * MINUTE, () => followedReport(t))
    const child = await at(base - 9 * MINUTE, () => attachment(t, parent.id, parent.follow))
    const plain = await at(base - 8 * MINUTE, () => plainReport(t))
    // Non-default values in as many of the keys as can have one.
    await at(base - 7 * MINUTE, () => reporterPost(t, parent.id, parent.follow, 'still there'))
    await at(base - 6 * MINUTE, () => setStatus(t, parent.id, 'fixed', '1.5.1'))
    await at(base - 5 * MINUTE, () => adminDetailPage(t, parent.id))
    await adminPost(t, `${parent.id}/triage`)
    await t.runDropAlarm(parent.id)

    const list = await getList(t)
    assert.equal(list.reports.length, 3)
    for (const row of list.reports) {
      assert.deepEqual(Object.keys(row).sort(), [...ROW_KEYS].sort(), row.id)
      const entry = (await t.index().getReport(row.id)) as unknown as Record<string, unknown>
      for (const key of ROW_KEYS) {
        if (key === 'lastActivity') assert.equal(typeof row.lastActivity, 'number')
        else assert.equal(row[key as keyof ApiRow], entry[key], `${row.id}.${key}`)
      }
      assert.deepEqual((await getDetail(t, row.id)).report, row, 'both routes build the same row')
    }
    const p = list.reports.find((r) => r.id === parent.id)!
    assert.equal(p.following, true)
    assert.equal(p.status, 'fixed')
    assert.equal(p.statusVersion, '1.5.1')
    assert.equal(p.reporterActivityAt, base - 7 * MINUTE)
    assert.equal(p.adminViewedAt, base - 5 * MINUTE)
    assert.equal(p.triaged, true)
    assert.equal(p.bundleDeleted, true)
    assert.equal(p.sizeBytes, 0)
    assert.equal(p.committed, true)
    assert.equal(list.reports.find((r) => r.id === child.id)!.parentId, parent.id)
    assert.equal(list.reports.find((r) => r.id === plain.id)!.statusVersion, null)
  })

  test('2 (decision 10): a field added to the index entry does not appear in either route', async () => {
    const t = await buildTestEnv()
    const r = await plainReport(t)
    const index = t.index()
    const realList = index.listReports.bind(index)
    const realGet = index.getReport.bind(index)
    index.listReports = async () => (await realList()).map((row) => ({ ...row, futureColumn: 'leak-sentinel' }))
    index.getReport = async (id: string) => {
      const row = await realGet(id)
      return row === null ? null : { ...row, futureColumn: 'leak-sentinel' }
    }
    for (const url of [LIST, `${LIST}/${r.id}`]) {
      const { text } = await okJson(await api(t, url))
      assert.ok(!text.includes('leak-sentinel'), url)
      assert.ok(!text.includes('futureColumn'), url)
      assert.ok(!text.includes('threadActivityAt'), url)
    }
  })
})

// ---------------------------------------------------------------------------
// Reading never writes
// ---------------------------------------------------------------------------

describe('reading through the API writes nothing', () => {
  /** A followed report with unread state on both sides. */
  async function unreadReport(t: TestEnv, base: number) {
    const r = await at(base, () => followedReport(t))
    await at(base + MINUTE, () => reply(t, r.id))
    await at(base + 2 * MINUTE, async () => {
      assert.equal((await threadRequest(t, r.id, r.follow, 'GET')).status, 200)
    })
    await at(base + 3 * MINUTE, () => reporterPost(t, r.id, r.follow, 'here are the steps'))
    return r
  }

  test('3: list then detail leave the index row and the thread meta row as they were', async () => {
    const t = await buildTestEnv()
    const base = Date.now()
    const r = await unreadReport(t, base)
    const rowBefore = indexRow(t, r.id)
    const metaBefore = threadMetaRow(t, r.id)
    const messagesBefore = threadMessageRows(t, r.id)
    const alarmBefore = await t.threadAlarmAt(r.id)
    const indexChanges = totalChanges(t.index()['storage'])
    const threadChanges = totalChanges(t.threadStorage(r.id)!)

    await at(base + 10 * MINUTE, async () => {
      await getList(t)
      const detail = await getDetail(t, r.id)
      assert.equal(detail.messages.length, 2)
    })

    const rowAfter = indexRow(t, r.id)
    const metaAfter = threadMetaRow(t, r.id)
    for (const key of ['adminViewedAt', 'reporterActivityAt', 'threadActivityAt']) {
      assert.ok(key in rowAfter, `the index row has ${key}`)
      assert.equal(rowAfter[key], rowBefore[key], key)
    }
    assert.equal(rowAfter.adminViewedAt, null)
    assert.equal(rowAfter.reporterActivityAt, base + 3 * MINUTE)
    assert.equal(rowAfter.threadActivityAt, base + MINUTE)
    for (const key of ['adminSeenUpTo', 'reporterSeenUpTo', 'lastPolledAt', 'lastActivityAt']) {
      assert.equal(metaAfter[key], metaBefore[key], key)
    }
    assert.equal(metaAfter.adminSeenUpTo, 0)
    assert.equal(metaAfter.lastPolledAt, base + 2 * MINUTE)
    assert.deepEqual(rowAfter, rowBefore)
    assert.deepEqual(metaAfter, metaBefore)
    assert.deepEqual(threadMessageRows(t, r.id), messagesBefore)
    assert.equal(await t.threadAlarmAt(r.id), alarmBefore)
    assert.equal(totalChanges(t.index()['storage']), indexChanges, 'no row written in the index')
    assert.equal(totalChanges(t.threadStorage(r.id)!), threadChanges, 'no row written in the thread')
  })

  test('3 (decision 7): neither route calls a method that writes, or one that reads and writes', async () => {
    const t = await buildTestEnv()
    const base = Date.now()
    const r = await unreadReport(t, base)
    const called: string[] = []
    const spy = (target: object, names: string[]) => {
      const methods = target as Record<string, (...args: unknown[]) => unknown>
      for (const name of names) {
        const real = methods[name].bind(target)
        methods[name] = (...args: unknown[]) => {
          called.push(name)
          return real(...args)
        }
      }
    }
    spy(t.index(), ['markAdminViewed', 'setThreadCache', 'touchActivity'])
    spy(t.thread(r.id), ['adminView', 'read', 'markSeen', 'create'])
    await getList(t)
    await getDetail(t, r.id)
    assert.deepEqual(called, [])
  })

  test('3 (decision 8): peek returns every message in seq order and writes nothing', async () => {
    const t = await buildTestEnv()
    const base = Date.now()
    const r = await unreadReport(t, base)
    const metaBefore = threadMetaRow(t, r.id)
    const alarmBefore = await t.threadAlarmAt(r.id)
    const changes = totalChanges(t.threadStorage(r.id)!)
    const store = t.thread(r.id) as unknown as { peek(): Promise<unknown[]> }
    const messages = await store.peek()
    assert.deepEqual(messages, [
      { seq: 1, at: base + MINUTE, author: 'maintainer', kind: 'text', text: 'Could you attach the file?' },
      { seq: 2, at: base + 3 * MINUTE, author: 'reporter', kind: 'text', text: 'here are the steps' },
    ])
    assert.deepEqual(threadMetaRow(t, r.id), metaBefore)
    assert.equal(await t.threadAlarmAt(r.id), alarmBefore)
    assert.equal(totalChanges(t.threadStorage(r.id)!), changes)
  })

  test('3 (decision 8): peek and the detail still show a dying thread its messages', async () => {
    const t = await buildTestEnv()
    const r = await unreadReport(t, Date.now())
    threadSql(t, r.id).exec('UPDATE meta SET dying = 1 WHERE id = 0')
    const store = t.thread(r.id) as unknown as { peek(): Promise<unknown[]> }
    assert.equal((await store.peek()).length, 2)
    assert.equal((await getDetail(t, r.id)).messages.length, 2)
    assert.equal(threadMetaRow(t, r.id).dying, 1)
  })

  test('4: the detail of a report with no thread is 200 with no messages, and creates none', async () => {
    const t = await buildTestEnv()
    const r = await plainReport(t)
    assert.equal(threadTableCount(t, r.id), 0)
    const detail = await getDetail(t, r.id)
    assert.deepEqual(detail.messages, [])
    assert.equal(detail.report.following, false)
    assert.equal(threadTableCount(t, r.id), 0, 'the thread storage still has no tables')
    assert.equal(await t.threadAlarmAt(r.id), null)
  })

  test('4 (decision 8): peek on a thread with no schema is empty and does not migrate', async () => {
    const t = await buildTestEnv()
    const store = t.thread(UNKNOWN_ID) as unknown as { peek(): Promise<unknown[]> }
    assert.deepEqual(await store.peek(), [])
    assert.equal(threadTableCount(t, UNKNOWN_ID), 0)
  })
})

// ---------------------------------------------------------------------------
// The detail
// ---------------------------------------------------------------------------

describe('GET /report/admin/api/reports/<id>', () => {
  /** Replaces a stored single-piece bundle's bytes. */
  async function replaceHead(t: TestEnv, id: string, bytes: Uint8Array): Promise<void> {
    t.dropStorage(id)!.sql.exec('UPDATE piece SET data = ?, length = ? WHERE idx = 0', bytes, bytes.byteLength)
  }

  test('5: the contact appears nowhere, and the description is the whole one from the head', async () => {
    const t = await buildTestEnv()
    const description = `${'the roof turns inside out when I push it. '.repeat(12)}END-OF-DESCRIPTION`
    const doc = {
      format: 1,
      report: { description, expected: 'sentinel-expected', contact: 'sentinel-contact@example.com' },
      system: {
        appVersion: '1.5.0',
        platform: 'desktop-macos',
        os: 'sentinel-os',
        gpu: 'sentinel-gpu',
        userAgent: 'sentinel-ua',
        locale: 'sentinel-locale',
      },
      crash: { at: 'sentinel-crash-at', message: 'sentinel-crash-message' },
    }
    const r = await followedReport(t, doc)
    const { body, text } = await okJson<ApiDetail>(await api(t, `${LIST}/${r.id}`))
    assert.deepEqual(Object.keys(body).sort(), [...DETAIL_KEYS].sort())
    assert.ok(!text.includes('sentinel-contact'), 'the contact is never returned')
    assert.ok(!text.includes('contact'))
    assert.ok(!text.includes('sentinel-'), 'no other head field either')
    assert.equal(body.description, description)
    assert.ok(body.report.descriptionPreview.length <= 200)
    assert.ok(!(await okJson(await api(t, LIST))).text.includes('sentinel-'))
    // The head really holds the contact: the admin page shows it.
    assert.ok((await adminDetailPage(t, r.id)).includes('sentinel-contact@example.com'))
  })

  test("6: the thread's secretHash appears nowhere, nor anything else from its meta row", async () => {
    const t = await buildTestEnv()
    const base = Date.now()
    const r = await followedReport(t)
    await reporterPost(t, r.id, r.follow, 'any news?')
    const secretHash = threadMetaRow(t, r.id).secretHash as string
    assert.ok(secretHash.length > 20)
    const { body, text } = await at(base + MINUTE, async () => okJson<ApiDetail>(await api(t, `${LIST}/${r.id}`)))
    assert.ok(!text.includes(secretHash))
    assert.ok(!text.includes(r.follow))
    assert.ok(!text.includes(r.token))
    for (const key of ['secretHash', 'reporterSeenUpTo', 'adminSeenUpTo', 'previousAdminSeenUpTo', 'lastPolledAt', 'lastActivityAt', 'finalSeenAt', 'dying']) {
      assert.ok(!text.includes(`"${key}"`), key)
    }
    assert.deepEqual(Object.keys(body).sort(), [...DETAIL_KEYS].sort())
    assert.equal(body.generatedAt, base + MINUTE, 'epoch milliseconds')
    assert.ok(!(await okJson(await api(t, LIST))).text.includes(secretHash))
  })

  test('7: a deleted bundle with a thread: 200, description null, messages present', async () => {
    const t = await buildTestEnv()
    const r = await followedReport(t)
    await reply(t, r.id)
    await t.runDropAlarm(r.id)
    const detail = await getDetail(t, r.id)
    assert.equal(detail.report.bundleDeleted, true)
    assert.equal(detail.description, null)
    assert.equal(detail.messages.length, 1)
  })

  test('7 (decision 20): a row marked bundleDeleted has no description even while bytes remain', async () => {
    const t = await buildTestEnv()
    const r = await followedReport(t)
    await reply(t, r.id)
    // The row says the bundle is gone; the drop is left in place.
    indexSql(t).exec('UPDATE reports SET bundleDeleted = 1 WHERE id = ?', r.id)
    const detail = await getDetail(t, r.id)
    assert.equal(detail.description, null)
    assert.equal(detail.messages.length, 1)
  })

  test('7: a head read that throws: 200, description null, messages present, and the error logged', async (ctx: TestContext) => {
    const t = await buildTestEnv()
    for (const method of ['head', 'read'] as const) {
      const r = await followedReport(t)
      await reply(t, r.id)
      let thrown = 0
      t.drop(r.id)[method] = async () => {
        thrown++
        throw new Error('drop down')
      }
      const log = ctx.mock.method(console, 'error', () => {})
      const detail = await getDetail(t, r.id)
      assert.ok(thrown > 0, `${method} was called`)
      assert.equal(detail.description, null, method)
      assert.equal(detail.messages.length, 1, method)
      assert.ok(log.mock.callCount() > 0, 'caught and logged')
      log.mock.restore()
    }
  })

  test('7: a head with no description: 200, description null, messages present', async () => {
    const t = await buildTestEnv()
    const r = await followedReport(t)
    await reply(t, r.id)
    // START refuses such a head, so it is written straight into the drop.
    await replaceHead(t, r.id, await gzipBytes(JSON.stringify(bundle({ expected: 'no description here' }))))
    const detail = await getDetail(t, r.id)
    assert.equal(detail.description, null)
    assert.equal(detail.messages.length, 1)
  })

  test('7 (decision 20): a head that cannot be decompressed or parsed, or a missing drop, gives a null description', async (ctx: TestContext) => {
    ctx.mock.method(console, 'error', () => {})
    const t = await buildTestEnv()
    const damage: Array<[string, (id: string) => Promise<void>]> = [
      ['not gzip', (id) => replaceHead(t, id, new TextEncoder().encode('definitely not gzip'))],
      ['not a bundle', async (id) => replaceHead(t, id, await gzipBytes('[1,2,3]'))],
      ['no drop', (id) => t.drop(id).destroy()],
    ]
    for (const [what, apply] of damage) {
      const r = await followedReport(t)
      await reply(t, r.id)
      await apply(r.id)
      const detail = await getDetail(t, r.id)
      assert.equal(detail.description, null, what)
      assert.equal(detail.messages.length, 1, what)
    }
  })

  test('8: messages in seq order, each with exactly the six keys; attachmentReportId only on the attachment', async () => {
    const t = await buildTestEnv()
    const base = Date.now()
    const r = await at(base, () => followedReport(t))
    await at(base + MINUTE, () => reporterPost(t, r.id, r.follow, 'it happened again'))
    await at(base + 2 * MINUTE, () => reply(t, r.id, 'Which version?'))
    await at(base + 3 * MINUTE, () => setStatus(t, r.id, 'need-info'))
    const child = await at(base + 4 * MINUTE, () => attachment(t, r.id, r.follow))

    const detail = await getDetail(t, r.id)
    assert.equal(detail.messages.length, 4)
    for (const m of detail.messages) assert.deepEqual(Object.keys(m).sort(), [...MESSAGE_KEYS].sort(), `seq ${m.seq}`)
    assert.deepEqual(
      detail.messages.map((m) => [m.seq, m.at, m.author, m.kind, m.attachmentReportId]),
      [
        [1, base + MINUTE, 'reporter', 'text', null],
        [2, base + 2 * MINUTE, 'maintainer', 'text', null],
        [3, base + 3 * MINUTE, 'system', 'status', null],
        [4, base + 4 * MINUTE, 'reporter', 'attachment', child.id],
      ],
    )
    assert.deepEqual(
      detail.messages.map((m) => m.text),
      threadMessageRows(t, r.id).map((m) => m.text),
    )
    assert.deepEqual(detail.messages.slice(0, 3).map((m) => m.text), ['it happened again', 'Which version?', 'Needs more information'])
    assert.match(detail.messages[3].text, /^kitchen\.skp \(\d+ B\)$/)
  })

  test('8 (decision 19): a field added to a thread message does not appear', async () => {
    const t = await buildTestEnv()
    const r = await followedReport(t)
    const store = t.thread(r.id) as unknown as { peek: () => Promise<unknown[]> }
    store.peek = async () => [
      { seq: 1, at: 5, author: 'maintainer', kind: 'text', text: 'hi', attachmentReportId: 'HEW-AAAA-BBBB', extra: 'leak-sentinel' },
    ]
    const { body, text } = await okJson<ApiDetail>(await api(t, `${LIST}/${r.id}`))
    assert.ok(!text.includes('leak-sentinel'))
    assert.deepEqual(Object.keys(body.messages[0]).sort(), [...MESSAGE_KEYS].sort())
  })

  test('9: unknown, uncommitted and malformed ids are the JSON 404', async () => {
    const t = await buildTestEnv()
    await plainReport(t)
    const uncommitted = await startUpload(t, { 'hew-follow': '1' })
    for (const id of [UNKNOWN_ID, UNKNOWN_ID.toLowerCase(), uncommitted.id, 'not-an-id', 'HEW-IIII-LLLL', 'batch']) {
      await assertJson404(await api(t, `${LIST}/${id}`), id)
    }
    assert.equal(threadTableCount(t, UNKNOWN_ID), 0, 'no thread is created for an unknown id')
    assert.equal(threadTableCount(t, uncommitted.id), 0)
  })

  test('9: every other path in the api branch is the JSON 404', async () => {
    const t = await buildTestEnv()
    const r = await plainReport(t)
    for (const path of [
      'api',
      'api/',
      'api/other',
      `api/other/${r.id}`,
      'api/reports/',
      `api/reports/${r.id}/`,
      `api/reports/${r.id}/x`,
      `api/reports/${r.id}/download`,
      'api/Reports',
    ]) {
      await assertJson404(await api(t, `${ADMIN}/${path}`), path)
    }
  })

  test('9 (decision 4): API in capitals is not the branch and stays the plain 404', async () => {
    const t = await buildTestEnv()
    const res = await api(t, `${ADMIN}/API/reports`)
    assert.equal(res.status, 404)
    assert.equal(await res.text(), 'Not found')
  })

  test('10: POST to the list, a detail, and a malformed-id detail is 405', async () => {
    const t = await buildTestEnv()
    const r = await followedReport(t)
    const before = indexRow(t, r.id)
    for (const url of [LIST, `${LIST}/${r.id}`, `${LIST}/not-an-id`, `${LIST}/${UNKNOWN_ID}`]) {
      for (const method of ['POST', 'PUT', 'DELETE']) {
        const res = await api(t, url, { method })
        assert.equal(res.status, 405, `${method} ${url}`)
        assert.equal(res.headers.get('cache-control'), 'no-store')
        assert.equal(await res.text(), '')
      }
    }
    assert.deepEqual(indexRow(t, r.id), before)
    assert.equal(threadTableCount(t, UNKNOWN_ID), 0)
  })

  test("10 (decision 3): the path's shape is checked before the method", async () => {
    const t = await buildTestEnv()
    const r = await plainReport(t)
    for (const path of ['api', 'api/other', 'api/reports/', `api/reports/${r.id}/x`, `api/reports/${r.id}/delete`]) {
      await assertJson404(await api(t, `${ADMIN}/${path}`, { method: 'POST' }), `POST ${path}`)
    }
    assert.ok(await t.index().getReport(r.id))
  })

  test('11: no JWT is the plain 403 on both routes', async () => {
    const t = await buildTestEnv()
    const r = await plainReport(t)
    for (const url of [LIST, `${LIST}/${r.id}`]) {
      // With the JWT the route answers, so the refusals below are of a real route.
      await okJson(await api(t, url))
      for (const headers of [{}, { 'cf-access-jwt-assertion': 'garbage' }] as Array<Record<string, string>>) {
        for (const method of ['GET', 'POST']) {
          const res = await api(t, url, { method, headers })
          assert.equal(res.status, 403, `${method} ${url}`)
          assert.match(res.headers.get('content-type') ?? '', /^text\/plain/)
          assert.equal(res.headers.get('cache-control'), 'no-store')
          assert.equal(await res.text(), 'Forbidden')
        }
      }
    }
  })

  test('11: on a host that is not the admin host both routes are the plain 404', async () => {
    const t = await buildTestEnv()
    const r = await plainReport(t)
    for (const path of ['/report/admin/api/reports', `/report/admin/api/reports/${r.id}`]) {
      await okJson(await api(t, `${ADMIN_BASE}${path}`))
      for (const host of [PUBLIC_BASE, 'https://beta.hew3d.com', 'https://example.com']) {
        for (const headers of [t.adminHeaders(), {}]) {
          const res = await api(t, `${host}${path}`, { headers })
          assert.equal(res.status, 404, `${host}${path}`)
          assert.match(res.headers.get('content-type') ?? '', /^text\/plain/)
          assert.equal(res.headers.get('cache-control'), 'no-store')
          assert.equal(await res.text(), 'Not found')
        }
      }
    }
  })

  test('12: a lower-case id finds its report and answers with the upper-case id', async () => {
    const t = await buildTestEnv()
    const r = await followedReport(t)
    await reply(t, r.id)
    const detail = await getDetail(t, r.id.toLowerCase())
    assert.equal(detail.report.id, r.id)
    assert.equal(detail.report.id, detail.report.id.toUpperCase())
    assert.equal(detail.messages.length, 1, 'the thread is the upper-case one')
    assert.equal(detail.description, DESCRIPTION)
  })

  test('12: a child has a detail of its own, with its row and its own (empty) thread', async () => {
    const t = await buildTestEnv()
    const parent = await followedReport(t)
    await reply(t, parent.id)
    const child = await attachment(t, parent.id, parent.follow)
    const detail = await getDetail(t, child.id)
    assert.equal(detail.report.id, child.id)
    assert.equal(detail.report.parentId, parent.id)
    assert.deepEqual(detail.report, await listRow(t, child.id))
    assert.equal(detail.description, `File for report ${parent.id}: kitchen.skp`)
    assert.deepEqual(detail.messages, [])
    assert.equal(threadTableCount(t, child.id), 0)
  })

  test('13: a throwing peek is the plain 503, not an empty thread', async (ctx: TestContext) => {
    ctx.mock.method(console, 'error', () => {})
    const t = await buildTestEnv()
    const r = await followedReport(t)
    await reply(t, r.id)
    let calls = 0
    const store = t.thread(r.id) as unknown as { peek: () => Promise<unknown[]> }
    store.peek = async () => {
      calls++
      throw new Error('thread DO down')
    }
    const res = await api(t, `${LIST}/${r.id}`)
    assert.equal(res.status, 503)
    assert.equal(calls, 1, 'peek was called')
    assert.match(res.headers.get('content-type') ?? '', /^text\/plain/)
    assert.equal(res.headers.get('cache-control'), 'no-store')
    assert.equal(await res.text(), 'Service unavailable')
  })
})

// ---------------------------------------------------------------------------
// lastActivity
// ---------------------------------------------------------------------------

describe('lastActivity', () => {
  test('14: a report committed with nothing else has lastActivity equal to receivedAt', async () => {
    const t = await buildTestEnv()
    const base = Date.now()
    const upload = await at(base, () => startUpload(t, { 'hew-follow': '1' }))
    await at(base + MINUTE, () => commit(t, upload))
    // The index's own `lastActivity` column is upload activity, not this.
    indexSql(t).exec('UPDATE reports SET lastActivity = ? WHERE id = ?', base + 30_000, upload.id)
    const row = await listRow(t, upload.id)
    assert.equal(row.receivedAt, base)
    assert.equal(row.lastActivity, row.receivedAt)
    assert.equal((await getDetail(t, upload.id)).report.lastActivity, base)
    assert.equal(await threadActivityAt(t, upload.id), null)
  })

  test('15: a reply, a status change, a reporter message and an attachment each advance it', async () => {
    const t = await buildTestEnv()
    const base = Date.now()
    const r = await at(base, () => followedReport(t))
    assert.equal(await lastActivity(t, r.id), base)

    await at(base + MINUTE, () => reply(t, r.id))
    assert.equal(await lastActivity(t, r.id), base + MINUTE, 'maintainer reply')
    assert.equal(await threadActivityAt(t, r.id), base + MINUTE)

    await at(base + 2 * MINUTE, () => setStatus(t, r.id, 'need-info'))
    assert.equal(await lastActivity(t, r.id), base + 2 * MINUTE, 'maintainer status change')
    assert.equal(await threadActivityAt(t, r.id), base + 2 * MINUTE)

    await at(base + 3 * MINUTE, () => reporterPost(t, r.id, r.follow, 'attaching it now'))
    assert.equal(await lastActivity(t, r.id), base + 3 * MINUTE, 'reporter message')
    assert.equal(await threadActivityAt(t, r.id), base + 2 * MINUTE, 'the reporter side does not write the column')

    const upload = await at(base + 4 * MINUTE, () => startAttachment(t, r.id, r.follow))
    assert.equal(await lastActivity(t, r.id), base + 3 * MINUTE, 'an uncommitted attachment is nothing yet')
    await at(base + 5 * MINUTE, () => commit(t, upload))
    assert.equal(await lastActivity(t, r.id), base + 5 * MINUTE, 'follow-up attachment, on the parent')
    assert.equal(await threadActivityAt(t, r.id), base + 2 * MINUTE)
    const child = await listRow(t, upload.id)
    assert.equal(child.receivedAt, base + 4 * MINUTE)
    assert.equal(await lastActivity(t, upload.id), child.receivedAt, "the child's is its own receivedAt")
    assert.equal(await threadActivityAt(t, upload.id), null)
  })

  test('15 (state matrix): a reporter message that reopens the report advances it through reporterActivityAt', async () => {
    const t = await buildTestEnv()
    const base = Date.now()
    const r = await at(base, () => followedReport(t))
    await at(base + MINUTE, () => setStatus(t, r.id, 'fixed', '1.5.1'))
    await at(base + 2 * MINUTE, () => reporterPost(t, r.id, r.follow, 'still broken'))
    const row = await listRow(t, r.id)
    assert.equal(row.status, 'open')
    assert.equal(row.lastActivity, base + 2 * MINUTE)
    assert.equal(await threadActivityAt(t, r.id), base + MINUTE)
  })

  test('15 (state matrix): the largest of the three wins, whichever side it is from', async () => {
    const t = await buildTestEnv()
    const base = Date.now()
    const r = await at(base, () => followedReport(t))
    await at(base + MINUTE, () => reporterPost(t, r.id, r.follow, 'one'))
    await at(base + 2 * MINUTE, () => reply(t, r.id))
    const row = await listRow(t, r.id)
    assert.equal(row.reporterActivityAt, base + MINUTE)
    assert.equal(row.lastActivity, base + 2 * MINUTE)
  })

  test('15 (decision 13): the stored time is the one the reply and the status change used', async () => {
    const t = await buildTestEnv()
    const base = Date.now()
    const r = await at(base, () => followedReport(t))
    await ticking(base + MINUTE, () => reply(t, r.id))
    const replyAt = threadMessageRows(t, r.id).at(-1)!.at
    assert.equal(await threadActivityAt(t, r.id), replyAt)
    assert.equal(await lastActivity(t, r.id), replyAt)
    await ticking(base + 2 * MINUTE, () => setStatus(t, r.id, 'closed'))
    const statusAt = threadMessageRows(t, r.id).at(-1)!.at
    assert.equal(await threadActivityAt(t, r.id), statusAt)
    assert.equal(await lastActivity(t, r.id), statusAt)
  })

  test("15 (decisions 12, 13): a reply's cache patch carries only threadActivityAt; a status change adds it to its own", async () => {
    const t = await buildTestEnv()
    const base = Date.now()
    const r = await followedReport(t)
    const index = t.index()
    const real = index.setThreadCache.bind(index)
    const calls: unknown[] = []
    index.setThreadCache = async (id, patch) => {
      calls.push([id, patch])
      await real(id, patch)
    }
    await at(base + MINUTE, () => reply(t, r.id))
    assert.deepEqual(calls, [[r.id, { threadActivityAt: base + MINUTE }]])
    calls.length = 0
    await at(base + 2 * MINUTE, () => setStatus(t, r.id, 'fixed', '1.5.1'))
    assert.deepEqual(calls, [[r.id, { status: 'fixed', statusVersion: '1.5.1', threadActivityAt: base + 2 * MINUTE }]])
  })

  test('15 (state matrix): a reply or status on a report nobody follows advances it too', async () => {
    const t = await buildTestEnv()
    const base = Date.now()
    const r = await at(base, () => plainReport(t))
    await at(base + MINUTE, () => reply(t, r.id, 'note to self'))
    assert.equal(await lastActivity(t, r.id), base + MINUTE)
    await at(base + 2 * MINUTE, () => setStatus(t, r.id, 'closed'))
    assert.equal(await lastActivity(t, r.id), base + 2 * MINUTE)
  })

  test('15 (decision 13): a refused reply or status change does not advance it', async () => {
    const t = await buildTestEnv()
    const base = Date.now()
    const r = await at(base, () => followedReport(t))
    for (let i = 0; i < THREAD_MAX_MESSAGES; i++) await t.thread(r.id).postMaintainer(`m${i}`, base)
    await at(base + MINUTE, async () => {
      assert.equal((await adminForm(t, `${r.id}/reply`, { text: 'one too many' })).status, 409)
      assert.equal((await adminForm(t, `${r.id}/status`, { status: 'closed' })).status, 409)
      assert.equal((await adminForm(t, `${r.id}/reply`, { text: '' })).status, 400)
      assert.equal((await adminForm(t, `${r.id}/status`, { status: 'bogus' })).status, 400)
    })
    assert.equal(await lastActivity(t, r.id), base)
    assert.equal(await threadActivityAt(t, r.id), null)
  })

  test('16: poll, markSeen, admin page view, triage and a repeated attachment commit leave it alone', async () => {
    const t = await buildTestEnv()
    const base = Date.now()
    const r = await at(base, () => followedReport(t))
    const upload = await at(base + MINUTE, () => startAttachment(t, r.id, r.follow))
    await at(base + 2 * MINUTE, () => commit(t, upload))
    await at(base + 3 * MINUTE, () => setStatus(t, r.id, 'fixed', '1.5.1'))
    const expected = base + 3 * MINUTE
    const childExpected = base + MINUTE

    const unchanged = async (what: string) => {
      assert.equal(await lastActivity(t, r.id), expected, what)
      assert.equal(await lastActivity(t, upload.id), childExpected, `${what} (child)`)
      assert.equal(await threadActivityAt(t, r.id), base + 3 * MINUTE, what)
    }
    await unchanged('before')

    await at(base + 4 * MINUTE, async () => {
      assert.equal((await threadRequest(t, r.id, r.follow, 'GET')).status, 200)
    })
    assert.equal(threadMetaRow(t, r.id).lastPolledAt, base + 4 * MINUTE)
    await unchanged('reporter poll')

    await at(base + 5 * MINUTE, async () => {
      assert.equal((await threadRequest(t, r.id, r.follow, 'POST', '/seen', { upTo: 999 })).status, 204)
    })
    assert.equal(threadMetaRow(t, r.id).finalSeenAt, base + 5 * MINUTE)
    await unchanged('markSeen')

    await at(base + 6 * MINUTE, () => adminDetailPage(t, r.id))
    assert.equal((await listRow(t, r.id)).adminViewedAt, base + 6 * MINUTE, 'adminViewedAt does change')
    await unchanged('admin detail page view')

    await at(base + 7 * MINUTE, () => adminPost(t, `${r.id}/triage`))
    assert.equal((await listRow(t, r.id)).triaged, true)
    await unchanged('triage toggle')

    await at(base + 8 * MINUTE, () => adminPost(t, 'batch/triage', { id: r.id, value: '0' }))
    assert.equal((await listRow(t, r.id)).triaged, false)
    await unchanged('batch triage')

    await at(base + 9 * MINUTE, () => commit(t, upload))
    await unchanged('repeated commit of a committed follow-up')
  })

  test('16 (state matrix): a bundle expiring leaves it alone and shows in bundleDeleted', async () => {
    const t = await buildTestEnv()
    const base = Date.now()
    const r = await at(base, () => followedReport(t))
    await at(base + MINUTE, () => reply(t, r.id))
    await setStatus(t, r.id, 'need-info')
    const before = await listRow(t, r.id)
    const column = await threadActivityAt(t, r.id)
    await at(base + 5 * MINUTE, () => t.runDropAlarm(r.id))
    const after = await listRow(t, r.id)
    assert.equal(after.bundleDeleted, true)
    assert.equal(after.lastActivity, before.lastActivity)
    assert.equal(await threadActivityAt(t, r.id), column)
  })

  test('17: Forget on a fixed report reopens it and advances lastActivity; on an open one it does not', async () => {
    const t = await buildTestEnv()
    const base = Date.now()
    const fixed = await at(base, () => followedReport(t))
    const open = await at(base, () => followedReport(t))
    const quiet = await at(base, () => followedReport(t))
    await at(base + MINUTE, () => setStatus(t, fixed.id, 'fixed', '1.5.1'))
    await at(base + MINUTE, () => reply(t, open.id))
    assert.equal(await lastActivity(t, fixed.id), base + MINUTE)

    await at(base + 5 * MINUTE, () => forget(t, fixed.id, fixed.follow))
    const first = await listRow(t, fixed.id)
    assert.equal(first.status, 'open')
    assert.equal(first.statusVersion, null)
    assert.equal(first.lastActivity, base + 5 * MINUTE)
    assert.equal(await threadActivityAt(t, fixed.id), base + 5 * MINUTE)
    assert.equal(await lastActivity(t, fixed.id), base + 5 * MINUTE)

    await at(base + 5 * MINUTE, () => forget(t, open.id, open.follow))
    assert.equal((await listRow(t, open.id)).status, 'open')
    assert.equal(await lastActivity(t, open.id), base + MINUTE)
    assert.equal(await threadActivityAt(t, open.id), base + MINUTE)

    await at(base + 5 * MINUTE, () => forget(t, quiet.id, quiet.follow))
    assert.equal(await lastActivity(t, quiet.id), base)
    assert.equal(await threadActivityAt(t, quiet.id), null)
  })

  test('17 (state matrix): the idle alarm does the same, at the time of the write', async () => {
    const t = await buildTestEnv()
    const base = Date.now()
    const waiting = await at(base, () => followedReport(t))
    const open = await at(base, () => followedReport(t))
    await at(base + MINUTE, () => setStatus(t, waiting.id, 'need-info'))
    for (const r of [waiting, open]) {
      const due = (await t.threadAlarmAt(r.id))!
      // The clock the index reads, not the time the alarm was handed.
      await at(base + 5 * MINUTE, () => t.runThreadAlarm(r.id, due))
      assert.equal(threadTableCount(t, r.id), 0)
    }
    const row = await listRow(t, waiting.id)
    assert.equal(row.status, 'open')
    assert.equal(row.lastActivity, base + 5 * MINUTE)
    assert.equal(await lastActivity(t, open.id), base)
    assert.equal(await threadActivityAt(t, open.id), null)
  })

  test('17 (state matrix): threadEnded stamps a row only when it was not open, so a repeat does nothing', async () => {
    const t = await buildTestEnv()
    const base = Date.now()
    const r = await at(base, () => followedReport(t))
    await at(base + MINUTE, () => setStatus(t, r.id, 'closed'))
    await at(base + 2 * MINUTE, () => t.index().threadEnded(r.id))
    assert.equal(await threadActivityAt(t, r.id), base + 2 * MINUTE)
    await at(base + 3 * MINUTE, () => t.index().threadEnded(r.id))
    assert.equal(await threadActivityAt(t, r.id), base + 2 * MINUTE)
  })

  test('17 (state matrix): markDestroyed stamps the rows that were not open and leaves the open ones', async () => {
    const t = await buildTestEnv()
    const base = Date.now()
    const made: string[] = []
    for (let i = 0; i < 4; i++) made.push((await at(base, () => followedReport(t))).id)
    const [fixed, waiting, replied, quiet] = made
    await at(base + MINUTE, () => setStatus(t, fixed, 'fixed', '1.5.1'))
    await at(base + MINUTE, () => setStatus(t, waiting, 'need-info'))
    await at(base + MINUTE, () => reply(t, replied))

    await at(base + 5 * MINUTE, () => t.index().markDestroyed(made))
    for (const id of [fixed, waiting]) {
      assert.equal(await threadActivityAt(t, id), base + 5 * MINUTE)
      const row = await listRow(t, id)
      assert.equal(row.status, 'open')
      assert.equal(row.bundleDeleted, true)
      assert.equal(row.lastActivity, base + 5 * MINUTE)
    }
    assert.equal(await threadActivityAt(t, replied), base + MINUTE)
    assert.equal(await lastActivity(t, replied), base + MINUTE)
    assert.equal(await threadActivityAt(t, quiet), null)
    assert.equal(await lastActivity(t, quiet), base)

    // Idempotent: they are all open now.
    await at(base + 6 * MINUTE, () => t.index().markDestroyed(made))
    assert.equal(await threadActivityAt(t, fixed), base + 5 * MINUTE)
    assert.equal(await threadActivityAt(t, quiet), null)
  })

  test('17 (state matrix): a thread destroyed after the bundle is gone removes the report from the API', async () => {
    const t = await buildTestEnv()
    const r = await followedReport(t)
    await setStatus(t, r.id, 'fixed', '1.5.1')
    await t.runDropAlarm(r.id)
    assert.ok(await listRow(t, r.id))
    await forget(t, r.id, r.follow)
    assert.deepEqual((await getList(t)).reports, [])
    await assertJson404(await api(t, `${LIST}/${r.id}`), r.id)
  })

  test('17 (state matrix): a deleted report, single or batch, is off the list and its detail is 404', async () => {
    const t = await buildTestEnv()
    const single = await followedReport(t)
    const batched = await followedReport(t)
    const child = await attachment(t, batched.id, batched.follow)
    const kept = await plainReport(t)
    assert.equal((await getList(t)).reports.length, 4)

    await adminPost(t, `${single.id}/delete`)
    await adminPost(t, 'batch/delete', { id: batched.id, confirm: '1' })

    assert.deepEqual(
      (await getList(t)).reports.map((r) => r.id),
      [kept.id],
    )
    for (const id of [single.id, batched.id, child.id]) await assertJson404(await api(t, `${LIST}/${id}`), id)
  })
})

// ---------------------------------------------------------------------------
// Older rows, the cache write
// ---------------------------------------------------------------------------

describe('the threadActivityAt column and its cache write', () => {
  /** An index whose `reports` table was made before `threadActivityAt`,
   *  served by the Worker in `t`'s place of its own. */
  function oldIndex(t: TestEnv, rows: Array<{ id: string; receivedAt: number; reporterActivityAt: number | null }>) {
    const storage = new FakeDurableObjectStorage()
    storage.sql.exec(
      `CREATE TABLE reports (
         id TEXT PRIMARY KEY,
         receivedAt INTEGER NOT NULL,
         appVersion TEXT NOT NULL,
         platform TEXT NOT NULL,
         descriptionPreview TEXT NOT NULL,
         sizeBytes INTEGER NOT NULL,
         triaged INTEGER NOT NULL DEFAULT 0,
         committed INTEGER NOT NULL DEFAULT 0,
         lastActivity INTEGER NOT NULL DEFAULT 0,
         following INTEGER NOT NULL DEFAULT 0,
         status TEXT NOT NULL DEFAULT 'open',
         statusVersion TEXT,
         reporterActivityAt INTEGER,
         adminViewedAt INTEGER,
         parentId TEXT,
         bundleDeleted INTEGER NOT NULL DEFAULT 0
       )`,
    )
    for (const row of rows) {
      storage.sql.exec(
        `INSERT INTO reports (id, receivedAt, appVersion, platform, descriptionPreview, sizeBytes, committed, lastActivity, reporterActivityAt)
         VALUES (?, ?, '1.4.0', 'web', 'an old report', 10, 1, ?, ?)`,
        row.id,
        row.receivedAt,
        // Upload activity, deliberately larger than anything the API may use.
        row.receivedAt + 90 * MINUTE,
        row.reporterActivityAt,
      )
    }
    const store = new IndexStore(storage)
    t.env.REPORT_INDEX = { idFromName: (name: string) => ({ toString: () => name }), get: () => store }
    const columns = () => storage.sql.exec<{ name: string }>('PRAGMA table_info(reports)').toArray().map((c) => c.name)
    return { storage, store, columns }
  }

  test('18: an older table gains the column, and its row lists with lastActivity from receivedAt and reporterActivityAt', async () => {
    const t = await buildTestEnv()
    const base = Date.now() - 60 * MINUTE
    const { store, columns } = oldIndex(t, [
      { id: 'HEW-AAAA-0001', receivedAt: base, reporterActivityAt: base + 5 * MINUTE },
      { id: 'HEW-AAAA-0002', receivedAt: base - MINUTE, reporterActivityAt: null },
    ])
    assert.ok(columns().includes('threadActivityAt'), 'the column is added')
    const stored = (await store.getReport('HEW-AAAA-0001')) as unknown as { threadActivityAt?: number | null }
    assert.equal(stored.threadActivityAt, null)

    const list = await getList(t)
    assert.deepEqual(
      list.reports.map((r) => [r.id, r.lastActivity]),
      [
        ['HEW-AAAA-0001', base + 5 * MINUTE],
        ['HEW-AAAA-0002', base - MINUTE],
      ],
    )
    for (const row of list.reports) assert.deepEqual(Object.keys(row).sort(), [...ROW_KEYS].sort())
  })

  test('18: setThreadCache never lowers threadActivityAt', async () => {
    const t = await buildTestEnv()
    const base = Date.now()
    const r = await at(base, () => followedReport(t))
    const index = t.index()
    const patch = (p: Record<string, unknown>) => index.setThreadCache(r.id, p)

    await patch({ threadActivityAt: base + 5 * MINUTE })
    assert.equal(await threadActivityAt(t, r.id), base + 5 * MINUTE, 'set over a null')
    await patch({ threadActivityAt: base + 2 * MINUTE })
    assert.equal(await threadActivityAt(t, r.id), base + 5 * MINUTE, 'the later value is kept')
    await patch({ status: 'closed', statusVersion: null, threadActivityAt: base + 3 * MINUTE })
    assert.equal(await threadActivityAt(t, r.id), base + 5 * MINUTE)
    assert.equal((await index.getReport(r.id))?.status, 'closed', 'the rest of the patch still applies')
    await patch({ threadActivityAt: base + 7 * MINUTE })
    assert.equal(await threadActivityAt(t, r.id), base + 7 * MINUTE)
    await patch({ status: 'open' })
    await patch({ reporterActivityAt: base + MINUTE })
    assert.equal(await threadActivityAt(t, r.id), base + 7 * MINUTE, 'a patch without it leaves it')
    assert.equal(await lastActivity(t, r.id), base + 7 * MINUTE)
  })

  test('18: a failing cache write during a reply changes nothing about the reply', async (ctx: TestContext) => {
    const log = ctx.mock.method(console, 'error', () => {})
    const t = await buildTestEnv()
    const base = Date.now()
    const r = await at(base, () => followedReport(t))
    const index = t.index()
    const real = index.setThreadCache
    let calls = 0
    index.setThreadCache = async () => {
      calls++
      throw new Error('index down')
    }
    const res = await at(base + MINUTE, () => adminForm(t, `${r.id}/reply`, { text: 'stored all the same' }))
    index.setThreadCache = real
    assert.equal(calls, 1, 'the reply tried the cache write')
    assert.equal(res.status, 303)
    assert.equal(res.headers.get('location'), `${ADMIN_BASE}/report/admin/${r.id}`)
    assert.equal(res.headers.get('cache-control'), 'no-store')
    assert.ok(log.mock.callCount() > 0, 'logged')
    assert.deepEqual(
      threadMessageRows(t, r.id).map((m) => [m.author, m.text]),
      [['maintainer', 'stored all the same']],
    )
    assert.equal(await lastActivity(t, r.id), base, 'stale until the next activity')
    assert.equal((await getDetail(t, r.id)).messages.length, 1, 'the detail is still complete')
  })

  test('18 (state matrix): a failing cache write during a status change leaves lastActivity stale', async (ctx: TestContext) => {
    ctx.mock.method(console, 'error', () => {})
    const t = await buildTestEnv()
    const base = Date.now()
    const r = await at(base, () => followedReport(t))
    const index = t.index()
    const real = index.setThreadCache
    index.setThreadCache = async () => {
      throw new Error('index down')
    }
    await at(base + MINUTE, () => setStatus(t, r.id, 'closed'))
    index.setThreadCache = real
    assert.equal(await lastActivity(t, r.id), base)
    assert.equal((await getDetail(t, r.id)).messages.length, 1)
    await at(base + 2 * MINUTE, () => reply(t, r.id))
    assert.equal(await lastActivity(t, r.id), base + 2 * MINUTE)
  })
})

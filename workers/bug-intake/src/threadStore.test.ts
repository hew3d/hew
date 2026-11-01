import { test, describe } from 'node:test'
import assert from 'node:assert/strict'

import { ThreadStore, REOPEN_TEXT, defaultStatusText } from './threadStore.ts'
import { FakeDurableObjectStorage } from './testSupport/fakeDurableObject.ts'
import { hashToken } from './uploadToken.ts'
import {
  THREAD_FINAL_SEEN_GRACE_MS,
  THREAD_IDLE_MS,
  THREAD_MAINTAINER_RESERVE,
  THREAD_MAX_MESSAGES,
  THREAD_REPORTER_POSTS_DAY_MAX,
} from './constants.ts'

const ID = 'HEW-7K3F-Q9XB'
const T0 = Date.parse('2026-11-01T12:00:00.000Z')

function tableCount(storage: FakeDurableObjectStorage): number {
  return storage.sql.exec<{ n: number }>("SELECT COUNT(*) AS n FROM sqlite_master WHERE type = 'table'").toArray()[0].n
}

async function setup(): Promise<{ store: ThreadStore; storage: FakeDurableObjectStorage; secret: string; destroyed: string[] }> {
  const storage = new FakeDurableObjectStorage()
  const destroyed: string[] = []
  const store = new ThreadStore(storage, async (id) => {
    destroyed.push(id)
  })
  const secret = await hashToken('the-follow-secret')
  await store.create(ID, secret, T0)
  return { store, storage, secret, destroyed }
}

describe('ThreadStore: lazy schema', () => {
  test('every reporter and admin read on a thread that was never created persists nothing', async () => {
    const storage = new FakeDurableObjectStorage()
    const store = new ThreadStore(storage)
    const h = await hashToken('x')
    assert.equal(await store.verifySecret(h), false)
    assert.equal(await store.read(h, 0, T0), 'forbidden')
    assert.deepEqual(await store.postReporter(h, 'hello', T0), { ok: false, reason: 'forbidden' })
    assert.equal(await store.markSeen(h, 3, T0), 'forbidden')
    assert.equal(await store.revoke(h), 'forbidden')
    assert.deepEqual(await store.postMaintainer('hi', T0), { ok: false, reason: 'not-found' })
    assert.deepEqual(await store.setStatus('fixed', '1.3.2', null, T0), { ok: false, reason: 'not-found' })
    assert.deepEqual(await store.appendAttachment('HEW-2QW8-M4ZD', 'f (1 MB)', T0), { ok: false, reason: 'not-found' })
    assert.equal(await store.adminView(T0), null)
    await store.alarm(T0)
    assert.equal(tableCount(storage), 0)
    assert.equal(await storage.getAlarm(), null)
  })

  test('only create writes the schema; destroy wipes it', async () => {
    const { store, storage } = await setup()
    assert.ok(tableCount(storage) > 0)
    await store.destroy()
    assert.equal(tableCount(storage), 0)
    assert.equal(await storage.getAlarm(), null)
  })
})

describe('ThreadStore: create and secrets', () => {
  test('create is idempotent and never replaces the stored secret', async () => {
    const { store, secret } = await setup()
    await store.create(ID, await hashToken('another'), T0 + 5)
    assert.equal(await store.verifySecret(secret), true)
    assert.equal(await store.verifySecret(await hashToken('another')), false)
  })

  test('a new thread is open with no messages, and its alarm is a year out', async () => {
    const { store, storage, secret } = await setup()
    const view = await store.read(secret, 0, T0)
    assert.notEqual(view, 'forbidden')
    if (view === 'forbidden') return
    assert.deepEqual(view, { format: 1, status: 'open', statusVersion: null, statusAt: T0, latest: 0, messages: [], now: T0 })
    assert.equal(await storage.getAlarm(), T0 + THREAD_IDLE_MS)
  })

  test('a wrong or empty secret is refused everywhere', async () => {
    const { store } = await setup()
    const wrong = await hashToken('wrong')
    assert.equal(await store.verifySecret(wrong), false)
    assert.equal(await store.verifySecret(''), false)
    assert.equal(await store.read(wrong, 0, T0), 'forbidden')
    assert.deepEqual(await store.postReporter(wrong, 'x', T0), { ok: false, reason: 'forbidden' })
    assert.equal(await store.markSeen(wrong, 0, T0), 'forbidden')
    assert.equal(await store.revoke(wrong), 'forbidden')
  })

  test('an admin-only thread (empty secret) can never be opened by a reporter', async () => {
    const store = new ThreadStore(new FakeDurableObjectStorage())
    await store.create(ID, '', T0)
    assert.equal(await store.verifySecret(''), false)
    assert.equal(await store.read('', 0, T0), 'forbidden')
    const view = await store.adminView(T0)
    assert.equal(view?.following, false)
  })
})

describe('ThreadStore: messages and the timeline', () => {
  test('read returns messages after the cursor in seq order, with attachmentReportId only on attachments', async () => {
    const { store, secret } = await setup()
    await store.postMaintainer('Could you attach the file?', T0 + 1)
    await store.appendAttachment('HEW-2QW8-M4ZD', 'kitchen.skp (46.5 MB)', T0 + 2)
    await store.setStatus('fixed', '1.3.2', null, T0 + 3)
    const view = await store.read(secret, 1, T0 + 4)
    if (view === 'forbidden') throw new Error('forbidden')
    assert.equal(view.latest, 3)
    assert.deepEqual(view.messages, [
      { seq: 2, at: T0 + 2, author: 'reporter', kind: 'attachment', text: 'kitchen.skp (46.5 MB)', attachmentReportId: 'HEW-2QW8-M4ZD' },
      { seq: 3, at: T0 + 3, author: 'system', kind: 'status', text: 'Fixed in 1.3.2' },
    ])
    assert.equal(view.status, 'fixed')
    assert.equal(view.statusVersion, '1.3.2')
    assert.equal(view.statusAt, T0 + 3)
  })

  test('read records the poll as activity and re-arms the alarm', async () => {
    const { store, storage, secret } = await setup()
    await store.read(secret, 0, T0 + 1000)
    assert.equal(await storage.getAlarm(), T0 + 1000 + THREAD_IDLE_MS)
    const admin = await store.adminView(T0 + 2000)
    assert.equal(admin?.lastPolledAt, T0 + 1000)
  })

  test('postReporter appends a reporter text message', async () => {
    const { store, secret } = await setup()
    const r = await store.postReporter(secret, 'it still happens', T0 + 1)
    assert.deepEqual(r, { ok: true, seq: 1, reopened: false, status: 'open', statusVersion: null })
  })

  test('a reporter message on a fixed thread reopens it with a system line and clears finalSeenAt', async () => {
    const { store, secret, storage } = await setup()
    await store.setStatus('fixed', '1.3.2', null, T0 + 1)
    await store.markSeen(secret, 1, T0 + 2)
    assert.equal((await store.adminView(T0 + 3))?.finalSeenAt, T0 + 2)
    const r = await store.postReporter(secret, 'it still happens on 1.3.2', T0 + 4)
    assert.deepEqual(r, { ok: true, seq: 2, reopened: true, status: 'open', statusVersion: null })
    const view = await store.adminView(T0 + 5)
    assert.equal(view?.finalSeenAt, null)
    assert.deepEqual(view?.messages.at(-1), { seq: 3, at: T0 + 4, author: 'system', kind: 'status', text: REOPEN_TEXT })
    assert.equal(await storage.getAlarm(), T0 + 4 + THREAD_IDLE_MS)
  })

  test('a reporter message on a closed thread also reopens it', async () => {
    const { store, secret } = await setup()
    await store.setStatus('closed', null, 'Not a bug.', T0 + 1)
    const r = await store.postReporter(secret, 'but it is', T0 + 2)
    assert.equal(r.ok && r.reopened, true)
  })

  test('a reporter message on a need-info thread moves it to open without a system line', async () => {
    const { store, secret } = await setup()
    await store.setStatus('need-info', null, null, T0 + 1)
    const r = await store.postReporter(secret, 'here are the steps', T0 + 2)
    assert.deepEqual(r, { ok: true, seq: 2, reopened: false, status: 'open', statusVersion: null })
    assert.equal((await store.adminView(T0 + 3))?.latest, 2)
  })

  test(`at most ${THREAD_REPORTER_POSTS_DAY_MAX} reporter posts per UTC day, with a retry-after to midnight`, async () => {
    const { store, secret } = await setup()
    for (let i = 0; i < THREAD_REPORTER_POSTS_DAY_MAX; i++) {
      assert.equal((await store.postReporter(secret, `m${i}`, T0 + i)).ok, true)
    }
    const refused = await store.postReporter(secret, 'one more', T0 + 100)
    assert.equal(refused.ok, false)
    if (refused.ok || refused.reason !== 'too-many') throw new Error('expected too-many')
    assert.equal(refused.retryAfterMs, Date.parse('2026-11-02T00:00:00.000Z') - (T0 + 100))
    const nextDay = await store.postReporter(secret, 'tomorrow', Date.parse('2026-11-02T00:00:01.000Z'))
    assert.equal(nextDay.ok, true)
  })

  test(`a thread holds at most ${THREAD_MAX_MESSAGES} messages; a reopen needs room for two`, async () => {
    const { store, secret } = await setup()
    for (let i = 0; i < THREAD_MAX_MESSAGES - 2; i++) await store.postMaintainer(`m${i}`, T0 + i)
    await store.setStatus('fixed', '1.3.2', null, T0 + 1000) // 199 messages
    assert.deepEqual(await store.postReporter(secret, 'still broken', T0 + 1001), { ok: false, reason: 'full' })
    assert.equal((await store.postMaintainer('last one', T0 + 1002)).ok, true) // 200
    assert.deepEqual(await store.postMaintainer('over', T0 + 1003), { ok: false, reason: 'full' })
    assert.deepEqual(await store.setStatus('closed', null, null, T0 + 1004), { ok: false, reason: 'full' })
    assert.deepEqual(await store.appendAttachment('HEW-2QW8-M4ZD', 'x', T0 + 1005), { ok: false, reason: 'full' })
  })

  test(`the last ${THREAD_MAINTAINER_RESERVE} slots are the maintainer's: reporter posts and attachments stop short`, async () => {
    const { store, secret } = await setup()
    const cap = THREAD_MAX_MESSAGES - THREAD_MAINTAINER_RESERVE
    for (let i = 0; i < cap - 1; i++) await store.postMaintainer(`m${i}`, T0 + i)
    // One slot left for the reporter: a plain post fits, then nothing does.
    assert.equal((await store.postReporter(secret, 'one more', T0 + 1000)).ok, true) // cap
    assert.deepEqual(await store.postReporter(secret, 'and another', T0 + 1001), { ok: false, reason: 'full' })
    assert.deepEqual(await store.appendAttachment('HEW-2QW8-M4ZD', 'x', T0 + 1002), { ok: false, reason: 'full' })
    // The maintainer still has the whole reserve: replies and a status.
    for (let i = 0; i < THREAD_MAINTAINER_RESERVE - 1; i++) {
      assert.equal((await store.postMaintainer(`r${i}`, T0 + 2000 + i)).ok, true)
    }
    assert.equal((await store.setStatus('closed', null, null, T0 + 3000)).ok, true) // THREAD_MAX_MESSAGES
    assert.deepEqual(await store.postMaintainer('over', T0 + 3001), { ok: false, reason: 'full' })
  })

  test('a reopening reporter post needs two slots under the reporter cap', async () => {
    const { store, secret } = await setup()
    const cap = THREAD_MAX_MESSAGES - THREAD_MAINTAINER_RESERVE
    for (let i = 0; i < cap - 2; i++) await store.postMaintainer(`m${i}`, T0 + i)
    await store.setStatus('fixed', '1.3.2', null, T0 + 1000) // cap - 1
    assert.deepEqual(await store.postReporter(secret, 'still broken', T0 + 1001), { ok: false, reason: 'full' })
    assert.equal((await store.adminView(T0 + 1002))?.status, 'fixed')
  })

  test('an attachment on a fixed/closed thread keeps the status but stops the grace clock', async () => {
    const { store, storage, secret } = await setup()
    await store.setStatus('fixed', '1.3.2', null, T0 + 1)
    await store.markSeen(secret, 1, T0 + 2)
    assert.equal(await storage.getAlarm(), T0 + 2 + THREAD_FINAL_SEEN_GRACE_MS)
    const appended = await store.appendAttachment('HEW-2QW8-M4ZD', 'log.txt (1 KB)', T0 + 50)
    assert.equal(appended.ok && appended.status, 'fixed')
    const v = await store.adminView(T0 + 51)
    assert.equal(v?.status, 'fixed')
    assert.equal(v?.statusVersion, '1.3.2')
    assert.equal(v?.finalSeenAt, null)
    assert.equal(v?.lastActivityAt, T0 + 50)
    assert.equal(await storage.getAlarm(), T0 + 50 + THREAD_IDLE_MS)
  })

  test('setStatus writes the note as the status text when given, else the default line', async () => {
    const { store } = await setup()
    await store.setStatus('closed', '9.9', '  Works as intended.  ', T0 + 1)
    await store.setStatus('fixed', '1.3.2', '', T0 + 2)
    await store.setStatus('need-info', '1.3.2', null, T0 + 3)
    const v = await store.adminView(T0 + 4)
    assert.deepEqual(
      v?.messages.map((m) => [m.author, m.kind, m.text]),
      [
        ['system', 'status', '  Works as intended.  '],
        ['system', 'status', 'Fixed in 1.3.2'],
        ['system', 'status', 'Needs more information'],
      ],
    )
    assert.equal(v?.status, 'need-info')
    assert.equal(v?.statusVersion, null, 'the version is kept only for fixed')
    assert.equal(defaultStatusText('open', null), 'Marked open')
  })

  test('appendAttachment is idempotent per child id and flips need-info to open', async () => {
    const { store } = await setup()
    await store.setStatus('need-info', null, null, T0 + 1)
    const a = await store.appendAttachment('HEW-2QW8-M4ZD', 'kitchen.skp (46.5 MB)', T0 + 2)
    assert.deepEqual(a, { ok: true, seq: 2, appended: true, status: 'open', statusVersion: null })
    const again = await store.appendAttachment('HEW-2QW8-M4ZD', 'kitchen.skp (46.5 MB)', T0 + 3)
    assert.deepEqual(again, { ok: true, seq: 2, appended: false, status: 'open', statusVersion: null })
    assert.equal((await store.adminView(T0 + 4))?.latest, 2)
  })

  test('appendAttachment leaves fixed/closed status alone', async () => {
    const { store } = await setup()
    await store.setStatus('fixed', '1.3.2', null, T0 + 1)
    const a = await store.appendAttachment('HEW-2QW8-M4ZD', 'f', T0 + 2)
    assert.equal(a.ok && a.status, 'fixed')
  })

  test('adminView moves adminSeenUpTo and reports the previous cursor', async () => {
    const { store, secret } = await setup()
    await store.postReporter(secret, 'one', T0 + 1)
    const first = await store.adminView(T0 + 2)
    assert.equal(first?.previousAdminSeenUpTo, 0)
    assert.equal(first?.following, true)
    await store.postReporter(secret, 'two', T0 + 3)
    const second = await store.adminView(T0 + 4)
    assert.equal(second?.previousAdminSeenUpTo, 1)
    assert.equal(second?.latest, 2)
    assert.equal((await store.adminView(T0 + 5))?.previousAdminSeenUpTo, 2)
  })
})

describe('ThreadStore: markSeen and the alarm rule', () => {
  test('upTo past latest is clamped; the cursor never moves backward', async () => {
    const { store, secret } = await setup()
    await store.postMaintainer('a', T0 + 1)
    assert.equal(await store.markSeen(secret, 99, T0 + 2), 'ok')
    assert.equal((await store.adminView(T0 + 3))?.reporterSeenUpTo, 1)
    await store.markSeen(secret, 0, T0 + 4)
    assert.equal((await store.adminView(T0 + 5))?.reporterSeenUpTo, 1)
  })

  test('seeing a final status sets finalSeenAt once; the alarm becomes the 30-day grace', async () => {
    const { store, storage, secret } = await setup()
    await store.setStatus('fixed', '1.3.2', null, T0 + 1)
    await store.postMaintainer('thanks!', T0 + 2)
    await store.markSeen(secret, 0, T0 + 3)
    assert.equal((await store.adminView(T0 + 3))?.finalSeenAt, null, 'not seen through the status message yet')
    await store.markSeen(secret, 1, T0 + 10)
    assert.equal((await store.adminView(T0 + 11))?.finalSeenAt, T0 + 10)
    assert.equal(await storage.getAlarm(), T0 + 10 + THREAD_FINAL_SEEN_GRACE_MS)
    await store.markSeen(secret, 2, T0 + 20)
    assert.equal((await store.adminView(T0 + 21))?.finalSeenAt, T0 + 10, 'set once')
    // A later poll moves activity, but the grace bound stays the earlier one.
    await store.read(secret, 0, T0 + 30)
    assert.equal(await storage.getAlarm(), T0 + 10 + THREAD_FINAL_SEEN_GRACE_MS)
  })

  test('seeing an open thread never sets finalSeenAt', async () => {
    const { store, secret } = await setup()
    await store.setStatus('need-info', null, null, T0 + 1)
    await store.markSeen(secret, 1, T0 + 2)
    assert.equal((await store.adminView(T0 + 3))?.finalSeenAt, null)
  })

  test('a status change after finalSeenAt clears it (the new status must be seen again)', async () => {
    const { store, secret } = await setup()
    await store.setStatus('fixed', '1.3.2', null, T0 + 1)
    await store.markSeen(secret, 1, T0 + 2)
    await store.setStatus('closed', null, null, T0 + 3)
    assert.equal((await store.adminView(T0 + 4))?.finalSeenAt, null)
  })

  test('alarmTimeFor picks the earlier of idle and grace', () => {
    assert.equal(ThreadStore.alarmTimeFor({ lastActivityAt: T0, finalSeenAt: null }), T0 + THREAD_IDLE_MS)
    assert.equal(ThreadStore.alarmTimeFor({ lastActivityAt: T0, finalSeenAt: T0 }), T0 + THREAD_FINAL_SEEN_GRACE_MS)
    const longAgo = T0 - THREAD_IDLE_MS
    assert.equal(ThreadStore.alarmTimeFor({ lastActivityAt: longAgo, finalSeenAt: T0 }), longAgo + THREAD_IDLE_MS)
  })

  test('an early (stale) alarm re-arms to the current due time and destroys nothing', async () => {
    const { store, storage, destroyed } = await setup()
    await store.alarm(T0 + 1000)
    assert.equal(destroyed.length, 0)
    assert.equal(await storage.getAlarm(), T0 + THREAD_IDLE_MS)
    assert.ok(tableCount(storage) > 0)
  })

  test('a due alarm tells the index first, then wipes', async () => {
    const storage = new FakeDurableObjectStorage()
    const events: string[] = []
    const store = new ThreadStore(storage, async (id) => {
      events.push(`index:${id}:${tableCount(storage) > 0 ? 'thread-still-there' : 'already-wiped'}`)
    })
    await store.create(ID, await hashToken('s'), T0)
    await store.alarm(T0 + THREAD_IDLE_MS)
    assert.deepEqual(events, [`index:${ID}:thread-still-there`])
    assert.equal(tableCount(storage), 0)
    assert.equal(await storage.getAlarm(), null)
  })

  test('if the index call fails the thread survives for the retried alarm', async () => {
    const storage = new FakeDurableObjectStorage()
    let fail = true
    const store = new ThreadStore(storage, async () => {
      if (fail) throw new Error('index down')
    })
    await store.create(ID, await hashToken('s'), T0)
    await assert.rejects(store.alarm(T0 + THREAD_IDLE_MS))
    assert.ok(tableCount(storage) > 0)
    fail = false
    await store.alarm(T0 + THREAD_IDLE_MS)
    assert.equal(tableCount(storage), 0)
  })
})

describe('ThreadStore: revoke', () => {
  test('revoke tells the index then destroys; a second revoke is forbidden', async () => {
    const { store, storage, secret, destroyed } = await setup()
    assert.equal(await store.revoke(secret), 'ok')
    assert.deepEqual(destroyed, [ID])
    assert.equal(tableCount(storage), 0)
    assert.equal(await store.revoke(secret), 'forbidden')
    assert.equal(tableCount(storage), 0)
  })
})

describe('ThreadStore: destruction is atomic against interleaved calls', () => {
  /** A store whose `onDestroy` runs `during` while the destroying call is
   *  suspended on it — what another request reaching the DO mid-await
   *  looks like. */
  async function interleaved(during: (store: ThreadStore, secret: string) => Promise<void>, fail = false) {
    const storage = new FakeDurableObjectStorage()
    const secret = await hashToken('the-follow-secret')
    let failNext = fail
    const store: ThreadStore = new ThreadStore(storage, async () => {
      await during(store, secret)
      if (failNext) {
        failNext = false
        throw new Error('index down')
      }
    })
    await store.create(ID, secret, T0)
    await store.postMaintainer('hello', T0 + 1)
    return { store, storage, secret }
  }

  async function assertAllRefused(store: ThreadStore, secret: string): Promise<void> {
    assert.deepEqual(await store.postReporter(secret, 'sneaking in', T0 + 10), { ok: false, reason: 'forbidden' })
    assert.equal(await store.read(secret, 0, T0 + 10), 'forbidden')
    assert.equal(await store.markSeen(secret, 1, T0 + 10), 'forbidden')
    assert.equal(await store.verifySecret(secret), false)
    assert.deepEqual(await store.postMaintainer('late reply', T0 + 10), { ok: false, reason: 'not-found' })
    assert.deepEqual(await store.setStatus('closed', null, null, T0 + 10), { ok: false, reason: 'not-found' })
    assert.deepEqual(await store.appendAttachment('HEW-2QW8-M4ZD', 'x', T0 + 10), { ok: false, reason: 'not-found' })
  }

  test('a post arriving while revoke awaits the index is refused', async () => {
    const { store, storage, secret } = await interleaved(assertAllRefused)
    assert.equal(await store.revoke(secret), 'ok')
    assert.equal(tableCount(storage), 0)
  })

  test('a post arriving while the alarm awaits the index is refused', async () => {
    const { store, storage } = await interleaved(assertAllRefused)
    await store.alarm(T0 + 1 + THREAD_IDLE_MS)
    assert.equal(tableCount(storage), 0)
  })

  test('a failed onDestroy clears the mark: the thread works again and the alarm stays retryable', async () => {
    const { store, storage, secret } = await interleaved(async () => {}, true)
    await assert.rejects(store.alarm(T0 + 1 + THREAD_IDLE_MS))
    assert.equal((await store.postReporter(secret, 'still here', T0 + 20)).ok, true)
    assert.equal((await store.postMaintainer('me too', T0 + 21)).ok, true)
    await store.alarm(T0 + 21 + THREAD_IDLE_MS)
    assert.equal(tableCount(storage), 0)
  })

  test('a failed destroy overlapping a live one leaves the mark to its owner: no post slips in', async () => {
    // Two destroys in flight: the first (the alarm) finds the index down,
    // the second (a Forget) is still awaiting its own index call. The
    // first's failure must not clear the mark the second relies on.
    const storage = new FakeDurableObjectStorage()
    const secret = await hashToken('the-follow-secret')
    let calls = 0
    let releaseSecond: () => void = () => {}
    const secondPending = new Promise<void>((resolve) => {
      releaseSecond = resolve
    })
    const store = new ThreadStore(storage, async () => {
      calls += 1
      if (calls === 1) await secondPending // the Forget, parked on the index
      else throw new Error('index down') // the alarm's own index call
    })
    await store.create(ID, secret, T0)
    await store.postMaintainer('hello', T0 + 1)
    // The Forget sets the mark and parks on its index call.
    const forget = store.revoke(secret)
    await new Promise((r) => setImmediate(r))
    // The alarm fires meanwhile; its index call fails. It did not set the
    // mark, so it must not clear it.
    await assert.rejects(store.alarm(T0 + 2))
    assert.deepEqual(await store.postReporter(secret, 'sneaking in', T0 + 3), { ok: false, reason: 'forbidden' })
    releaseSecond()
    assert.equal(await forget, 'ok')
    assert.equal(tableCount(storage), 0)
  })

  test('a destroy that died mid-flight (mark left set) is due at once and Forget can finish it', async () => {
    const { store, storage, secret } = await interleaved(async () => {})
    storage.sql.exec('UPDATE meta SET dying = 1 WHERE id = 0')
    assert.equal(await store.verifySecret(secret), false)
    assert.equal(await store.revoke(secret), 'ok')
    assert.equal(tableCount(storage), 0)
    const again = await interleaved(async () => {})
    again.storage.sql.exec('UPDATE meta SET dying = 1 WHERE id = 0')
    await again.store.alarm(T0 + 2) // long before the idle bound
    assert.equal(tableCount(again.storage), 0)
  })
})

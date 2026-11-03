import { test } from 'node:test'
import assert from 'node:assert/strict'

import { UploadBudgetStore } from './uploadBudgetStore.ts'
import { FakeDurableObjectStorage } from './testSupport/fakeDurableObject.ts'

test('missing releases do not schedule alarm writes while another reservation is live', async (t) => {
  const storage = new FakeDurableObjectStorage()
  const budget = new UploadBudgetStore(storage)
  await budget.reserve('live', 'client', 1)
  const alarm = await storage.getAlarm()
  const schedule = t.mock.method(storage, 'setAlarm')
  for (let i = 0; i < 20; i++) await budget.release(`unknown-${i}`)
  assert.equal(schedule.mock.callCount(), 0)
  assert.equal(await storage.getAlarm(), alarm)
})

test('admitting a later reservation leaves an unchanged earliest alarm untouched', async (t) => {
  const storage = new FakeDurableObjectStorage()
  const budget = new UploadBudgetStore(storage)
  await budget.reserve('first', 'one-client', 1)
  const schedule = t.mock.method(storage, 'setAlarm')
  await budget.reserve('second', 'one-client', 1)
  assert.equal(schedule.mock.callCount(), 0)
})

test('commit renews a live reservation from persistence completion and refuses expired admissions', async (t) => {
  let now = Date.UTC(2026, 10, 3, 12)
  t.mock.method(Date, 'now', () => now)
  const budget = new UploadBudgetStore(new FakeDurableObjectStorage())
  await budget.reserve('live', 'one-client', 1)
  await budget.reserve('abandoned', 'another-client', 1)
  now += 19 * 60 * 1000
  assert.equal(await budget.commit('live'), true)
  now += 2 * 60 * 1000
  await budget.alarm()
  assert.equal(await budget.resize('live', 1), true)
  assert.equal(await budget.commit('abandoned'), false)
  assert.equal(await budget.commit('missing'), false)
})

test('concurrent reservations cannot spend the same aggregate capacity', async () => {
  const budget = new UploadBudgetStore(new FakeDurableObjectStorage())
  const results = await Promise.all(Array.from({ length: 9 }, (_, i) =>
    budget.reserve(`token-${i}`, `client-${i}`, 32 * 1024 * 1024)))
  assert.equal(results.filter((r) => r.ok).length, 8)
  assert.deepEqual(results[8], { ok: false, reason: 'full' })
  await budget.release('token-0')
  assert.deepEqual(await budget.reserve('replacement', 'client-new', 32 * 1024 * 1024), { ok: true })
})

test('small drops have a count ceiling independent of the byte ceiling', async () => {
  const budget = new UploadBudgetStore(new FakeDurableObjectStorage())
  for (let i = 0; i < 256; i++) assert.deepEqual(await budget.reserve(`t-${i}`, `c-${i}`, 1), { ok: true })
  assert.deepEqual(await budget.reserve('overflow', 'another', 1), { ok: false, reason: 'full' })
})

test('shrinking reservations makes unused capacity available without allowing growth', async () => {
  const budget = new UploadBudgetStore(new FakeDurableObjectStorage())
  for (let i = 0; i < 8; i++) await budget.reserve(`t-${i}`, `c-${i}`, 32 * 1024 * 1024)
  assert.equal(await budget.resize('t-0', 1), true)
  assert.equal(await budget.resize('t-0', 2), false)
  assert.equal(await budget.resize('missing', 1), false)
  assert.equal(await budget.resize('t-1', 0), false)
  assert.deepEqual(await budget.reserve('extra', 'another', 32 * 1024 * 1024 - 1), { ok: true })
})

test('rate limits survive release and a new store instance over persisted storage', async () => {
  const storage = new FakeDurableObjectStorage()
  const budget = new UploadBudgetStore(storage)
  for (let i = 0; i < 60; i++) {
    await budget.reserve(`t-${i}`, 'one-client', 1)
    await budget.release(`t-${i}`)
  }
  const restarted = new UploadBudgetStore(storage)
  const denied = await restarted.reserve('overflow', 'one-client', 1)
  assert.equal(denied.ok, false)
  if (!denied.ok) assert.equal(denied.reason, 'rate-limited')
  assert.deepEqual(await restarted.reserve('another', 'another-client', 1), { ok: true })
})

test('daily write admission is bounded even when every upload is immediately deleted', async () => {
  const storage = new FakeDurableObjectStorage()
  const budget = new UploadBudgetStore(storage)
  for (let i = 0; i < 1000; i++) {
    assert.deepEqual(await budget.reserve(`t-${i}`, `c-${i}`, 1), { ok: true })
    await budget.release(`t-${i}`)
  }
  assert.equal((await budget.reserve('overflow', 'another', 1)).ok, false)
  assert.equal(storage.sql.exec<{ n: number }>('SELECT COUNT(*) AS n FROM client').toArray()[0].n, 1000)
  // Rejected identities do not create rows in the limiter.
  for (let i = 0; i < 20; i++) await budget.reserve(`denied-${i}`, `denied-${i}`, 1)
  assert.equal(storage.sql.exec<{ n: number }>('SELECT COUNT(*) AS n FROM client').toArray()[0].n, 1000)
})

test('expiry releases abandoned reservations and rate state while rejecting stale resize', async (t) => {
  let now = Date.UTC(2026, 10, 3, 12)
  t.mock.method(Date, 'now', () => now)
  const storage = new FakeDurableObjectStorage()
  const budget = new UploadBudgetStore(storage)
  for (let i = 0; i < 8; i++) await budget.reserve(`t-${i}`, 'one-client', 32 * 1024 * 1024)
  assert.notEqual(await storage.getAlarm(), null)
  now += 20 * 60 * 1000
  assert.equal(await budget.resize('t-0', 1), false)
  await budget.alarm()
  assert.equal(await storage.getAlarm(), null)
  assert.equal(storage.sql.exec<{ n: number }>('SELECT COUNT(*) AS n FROM client').toArray()[0].n, 0)
  assert.deepEqual(await budget.reserve('fresh', 'one-client', 32 * 1024 * 1024), { ok: true })
})

test('daily allowance resets using service time at UTC midnight', async (t) => {
  let now = Date.UTC(2026, 10, 3, 23, 59)
  t.mock.method(Date, 'now', () => now)
  const storage = new FakeDurableObjectStorage()
  const budget = new UploadBudgetStore(storage)
  for (let i = 0; i < 1000; i++) {
    await budget.reserve(`t-${i}`, `c-${i}`, 1)
    await budget.release(`t-${i}`)
  }
  assert.equal((await budget.reserve('denied', 'new-client', 1)).ok, false)
  now += 60 * 1000
  assert.deepEqual(await budget.reserve('next-day', 'new-client', 1), { ok: true })
})

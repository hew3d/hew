/**
 * Unit tests for `dropStore.ts`'s `DropStore` — the framework-free SQLite
 * logic behind the `ShareDrop` Durable Object. Driven directly against
 * `testSupport/fakeDurableObject.ts`'s `node:sqlite`-backed storage, so a test
 * can reach past the RPC surface (e.g. backdate a stored row's `uploadedAt` to
 * prove TTL expiry). `handlers.test.ts` covers the HTTP layer over a fake DO
 * stub; the thin `ShareDrop` RPC wrapper (`shareDrop.ts`) is validated against
 * real `workerd` via `wrangler dev` — bare `node --test` cannot import
 * `cloudflare:workers` and so cannot exercise DO RPC itself.
 */

import { test, describe } from 'node:test'
import assert from 'node:assert/strict'

import { DropStore, TTL_MS } from './dropStore.ts'
import { FakeDurableObjectStorage } from './testSupport/fakeDurableObject.ts'

function makeDrop(): { drop: DropStore; storage: FakeDurableObjectStorage } {
  const storage = new FakeDurableObjectStorage()
  const drop = new DropStore(storage)
  return { drop, storage }
}

function chunksOf(bytes: Uint8Array, size: number): Uint8Array[] {
  const chunks: Uint8Array[] = []
  for (let offset = 0; offset < bytes.byteLength; offset += size) {
    chunks.push(bytes.subarray(offset, Math.min(offset + size, bytes.byteLength)))
  }
  return chunks.length > 0 ? chunks : [bytes]
}

describe('DropStore.store / consume', () => {
  test('round-trips name and bytes across several chunks', async () => {
    const { drop } = makeDrop()
    const payload = new Uint8Array([10, 20, 30, 40, 50, 60, 70])
    await drop.store('a-name', chunksOf(payload, 3))

    const result = await drop.consume()
    assert.ok(result !== null)
    assert.equal(result.name, 'a-name')
    assert.deepEqual(Array.from(result.bytes), Array.from(payload))
  })

  test('consume on an empty (never-stored) drop returns null', async () => {
    const { drop } = makeDrop()
    assert.equal(await drop.consume(), null)
  })

  test('consume is one-shot: a second call returns null', async () => {
    const { drop } = makeDrop()
    await drop.store('x', [new Uint8Array([1, 2, 3])])
    const first = await drop.consume()
    assert.notEqual(first, null)
    const second = await drop.consume()
    assert.equal(second, null)
  })

  test('two concurrent consume() calls: exactly one returns a result', async () => {
    const { drop } = makeDrop()
    await drop.store('x', [new Uint8Array([9, 9, 9])])
    const [a, b] = await Promise.all([drop.consume(), drop.consume()])
    const nonNull = [a, b].filter((r) => r !== null)
    assert.equal(nonNull.length, 1)
  })

  test('store rejects a second write to an already-populated drop', async () => {
    const { drop } = makeDrop()
    await drop.store('first', [new Uint8Array([1])])
    await assert.rejects(() => drop.store('second', [new Uint8Array([2])]))
  })

  test('store arms an alarm at uploadedAt + TTL_MS', async () => {
    const { drop, storage } = makeDrop()
    const before = Date.now()
    await drop.store('x', [new Uint8Array([1])])
    const alarm = await storage.getAlarm()
    assert.ok(alarm !== null)
    assert.ok(alarm >= before + TTL_MS)
    assert.ok(alarm <= Date.now() + TTL_MS)
  })
})

describe('DropStore TTL / expiry', () => {
  test('consume of a drop whose uploadedAt is backdated past TTL_MS returns null', async () => {
    const { drop, storage } = makeDrop()
    await drop.store('x', [new Uint8Array([1, 2, 3])])

    const backdated = Date.now() - TTL_MS - 1
    storage.sql.exec('UPDATE meta SET uploadedAt = ? WHERE id = 0', backdated)

    assert.equal(await drop.consume(), null)
  })

  test('a drop consumed just under TTL_MS still succeeds', async () => {
    const { drop, storage } = makeDrop()
    await drop.store('x', [new Uint8Array([1, 2, 3])])

    const justUnder = Date.now() - TTL_MS + 5_000
    storage.sql.exec('UPDATE meta SET uploadedAt = ? WHERE id = 0', justUnder)

    assert.notEqual(await drop.consume(), null)
  })

  // `ShareDrop.alarm()` (the DO wrapper) simply calls `DropStore.destroy()`;
  // these prove the wipe semantics the alarm relies on. The alarm firing
  // itself is exercised end to end against real `workerd`.
  test('destroy (what the alarm handler calls) wipes an unconsumed drop', async () => {
    const { drop } = makeDrop()
    await drop.store('x', [new Uint8Array([1, 2, 3])])

    await drop.destroy()

    assert.equal(await drop.consume(), null)
  })

  test('destroy against an already-consumed drop is harmless', async () => {
    const { drop } = makeDrop()
    await drop.store('x', [new Uint8Array([1])])
    await drop.consume()

    await assert.doesNotReject(() => drop.destroy())
  })
})

describe('DropStore.peek', () => {
  test('returns {exists:true} for a freshly stored drop', async () => {
    const { drop } = makeDrop()
    await drop.store('x', [new Uint8Array([1, 2, 3])])

    assert.deepEqual(await drop.peek(), { exists: true })
  })

  test('does not consume: a peek followed by consume still returns the bytes', async () => {
    const { drop } = makeDrop()
    const payload = new Uint8Array([1, 2, 3])
    await drop.store('x', [payload])

    assert.deepEqual(await drop.peek(), { exists: true })
    const result = await drop.consume()
    assert.ok(result !== null)
    assert.deepEqual(Array.from(result.bytes), Array.from(payload))
  })

  test('returns {exists:false} after the drop has been consumed', async () => {
    const { drop } = makeDrop()
    await drop.store('x', [new Uint8Array([1])])
    await drop.consume()

    assert.deepEqual(await drop.peek(), { exists: false })
  })

  test('returns {exists:false} for a drop backdated past TTL_MS', async () => {
    const { drop, storage } = makeDrop()
    await drop.store('x', [new Uint8Array([1])])

    const backdated = Date.now() - TTL_MS - 1
    storage.sql.exec('UPDATE meta SET uploadedAt = ? WHERE id = 0', backdated)

    assert.deepEqual(await drop.peek(), { exists: false })
  })

  test('returns {exists:true} for a drop just under TTL_MS', async () => {
    const { drop, storage } = makeDrop()
    await drop.store('x', [new Uint8Array([1])])

    const justUnder = Date.now() - TTL_MS + 5_000
    storage.sql.exec('UPDATE meta SET uploadedAt = ? WHERE id = 0', justUnder)

    assert.deepEqual(await drop.peek(), { exists: true })
  })

  test('returns {exists:false} on a never-stored drop, without throwing "no such table"', async () => {
    const { drop } = makeDrop()
    await assert.doesNotReject(async () => {
      assert.deepEqual(await drop.peek(), { exists: false })
    })
  })
})

describe('DropStore.destroy', () => {
  test('wipes a populated drop; consume() afterward returns null', async () => {
    const { drop } = makeDrop()
    await drop.store('x', [new Uint8Array([1, 2, 3])])

    await drop.destroy()

    assert.equal(await drop.consume(), null)
  })

  test('is idempotent: calling it twice, or on an empty drop, never throws', async () => {
    const { drop } = makeDrop()
    await assert.doesNotReject(() => drop.destroy())
    await assert.doesNotReject(() => drop.destroy())

    await drop.store('x', [new Uint8Array([1])])
    await drop.destroy()
    await assert.doesNotReject(() => drop.destroy())
  })

  test('cancels the pending alarm', async () => {
    const { drop, storage } = makeDrop()
    await drop.store('x', [new Uint8Array([1])])
    assert.notEqual(await storage.getAlarm(), null)

    await drop.destroy()

    assert.equal(await storage.getAlarm(), null)
  })
})

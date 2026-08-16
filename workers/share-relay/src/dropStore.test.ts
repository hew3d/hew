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

import { DropStore, RPC_BATCH_CHUNKS, TTL_MS } from './dropStore.ts'
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

/** Stores `chunks` as one complete drop through the batched write protocol
 *  (`store` for the first batch, `append` for the rest — `handlers.ts` does
 *  exactly this). */
async function storeAll(drop: DropStore, name: string, chunks: Uint8Array[]): Promise<void> {
  const totalBytes = chunks.reduce((n, c) => n + c.byteLength, 0)
  await drop.store(name, chunks.length, totalBytes, chunks.slice(0, RPC_BATCH_CHUNKS))
  for (let from = RPC_BATCH_CHUNKS; from < chunks.length; from += RPC_BATCH_CHUNKS) {
    await drop.append(chunks.slice(from, from + RPC_BATCH_CHUNKS))
  }
}

/** The batched read: `consume` (the claim) then `take` to the end. Returns
 *  `null` exactly when `consume` does. */
async function consumeAll(drop: DropStore): Promise<{ name: string; bytes: Uint8Array } | null> {
  const head = await drop.consume()
  if (head === null) return null
  const bytes = new Uint8Array(head.totalBytes)
  let offset = 0
  for (let from = 0; from < head.chunkCount; from += RPC_BATCH_CHUNKS) {
    for (const chunk of await drop.take(from, RPC_BATCH_CHUNKS)) {
      bytes.set(chunk, offset)
      offset += chunk.byteLength
    }
  }
  assert.equal(offset, head.totalBytes)
  return { name: head.name, bytes }
}

describe('DropStore.store / consume', () => {
  test('round-trips name and bytes across several chunks', async () => {
    const { drop } = makeDrop()
    const payload = new Uint8Array([10, 20, 30, 40, 50, 60, 70])
    await storeAll(drop, 'a-name', chunksOf(payload, 3))

    const result = await consumeAll(drop)
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
    await storeAll(drop, 'x', [new Uint8Array([1, 2, 3])])
    const first = await consumeAll(drop)
    assert.notEqual(first, null)
    const second = await drop.consume()
    assert.equal(second, null)
  })

  test('consume is one-shot even before take: the claim alone blocks a second consume', async () => {
    const { drop } = makeDrop()
    await storeAll(drop, 'x', [new Uint8Array([1, 2, 3])])
    const head = await drop.consume()
    assert.notEqual(head, null)
    assert.equal(await drop.consume(), null)
    // ...and peek reports gone the moment it's claimed, before any bytes move.
    assert.deepEqual(await drop.peek(), { exists: false })
  })

  test('take reads and deletes in batches; the last batch wipes the drop and its alarm', async () => {
    const { drop, storage } = makeDrop()
    const payload = new Uint8Array(RPC_BATCH_CHUNKS * 3 * 2 + 1)
    for (let i = 0; i < payload.length; i++) payload[i] = i & 0xff
    const chunks = chunksOf(payload, 3) // 2 full RPC batches + 1 chunk
    await storeAll(drop, 'big', chunks)
    const head = await drop.consume()
    assert.ok(head !== null)
    assert.equal(head.chunkCount, chunks.length)
    assert.equal(head.totalBytes, payload.byteLength)

    const first = await drop.take(0, RPC_BATCH_CHUNKS)
    assert.equal(first.length, RPC_BATCH_CHUNKS)
    assert.notEqual(await storage.getAlarm(), null) // not done yet
    const second = await drop.take(RPC_BATCH_CHUNKS, RPC_BATCH_CHUNKS)
    assert.equal(second.length, RPC_BATCH_CHUNKS)
    const third = await drop.take(RPC_BATCH_CHUNKS * 2, RPC_BATCH_CHUNKS)
    assert.equal(third.length, 1)
    assert.equal(await storage.getAlarm(), null) // wiped after the last row

    const out = new Uint8Array(payload.byteLength)
    let offset = 0
    for (const chunk of [...first, ...second, ...third]) {
      out.set(chunk, offset)
      offset += chunk.byteLength
    }
    assert.deepEqual(out, payload)
    assert.equal(await drop.consume(), null)
  })

  test('take on a stored-but-unclaimed drop throws (protocol error); on a vanished drop it answers []', async () => {
    const { drop } = makeDrop()
    // Never stored → nothing there → [] (the "vanished mid-download" answer).
    assert.deepEqual(await drop.take(0, RPC_BATCH_CHUNKS), [])
    await storeAll(drop, 'x', [new Uint8Array([1])])
    await assert.rejects(() => drop.take(0, RPC_BATCH_CHUNKS))
  })

  test('a drop destroyed between consume and take (TTL alarm / DELETE mid-download) reads as vanished, not as an error', async () => {
    const { drop } = makeDrop()
    const payload = new Uint8Array(RPC_BATCH_CHUNKS * 3 * 2)
    await storeAll(drop, 'x', chunksOf(payload, 3))
    const head = await drop.consume()
    assert.ok(head !== null)
    const first = await drop.take(0, RPC_BATCH_CHUNKS)
    assert.equal(first.length, RPC_BATCH_CHUNKS)
    await drop.destroy() // the alarm, or the desktop's DELETE
    assert.deepEqual(await drop.take(RPC_BATCH_CHUNKS, RPC_BATCH_CHUNKS), [])
    assert.deepEqual(await drop.peek(), { exists: false })
  })

  test('an incomplete upload (fewer chunk rows than declared) is invisible to consume and peek', async () => {
    const { drop } = makeDrop()
    // Declare 3 chunks, deliver 2 — the third `append` never comes.
    await drop.store('partial', 3, 9, [new Uint8Array([1, 2, 3]), new Uint8Array([4, 5, 6])])
    assert.deepEqual(await drop.peek(), { exists: false })
    assert.equal(await drop.consume(), null)
    // Completing it makes it present.
    await drop.append([new Uint8Array([7, 8, 9])])
    assert.deepEqual(await drop.peek(), { exists: true })
    const result = await consumeAll(drop)
    assert.ok(result !== null)
    assert.deepEqual(Array.from(result.bytes), [1, 2, 3, 4, 5, 6, 7, 8, 9])
  })

  test('append refuses more chunks than declared, and refuses when nothing was stored', async () => {
    const { drop } = makeDrop()
    await assert.rejects(() => drop.append([new Uint8Array([1])]))
    await drop.store('x', 1, 1, [new Uint8Array([1])])
    await assert.rejects(() => drop.append([new Uint8Array([2])]))
  })

  test('store refuses a batch larger than the declared count, or a zero count', async () => {
    const { drop } = makeDrop()
    await assert.rejects(() => drop.store('x', 1, 2, [new Uint8Array([1]), new Uint8Array([2])]))
    await assert.rejects(() => drop.store('x', 0, 0, []))
  })

  test('two concurrent consume() calls: exactly one returns a result', async () => {
    const { drop } = makeDrop()
    await storeAll(drop, 'x', [new Uint8Array([9, 9, 9])])
    const [a, b] = await Promise.all([consumeAll(drop), consumeAll(drop)])
    const nonNull = [a, b].filter((r) => r !== null)
    assert.equal(nonNull.length, 1)
  })

  test('store rejects a second write to an already-populated drop', async () => {
    const { drop } = makeDrop()
    await storeAll(drop, 'first', [new Uint8Array([1])])
    await assert.rejects(() => drop.store('second', 1, 1, [new Uint8Array([2])]))
  })

  test('store arms an alarm at uploadedAt + TTL_MS', async () => {
    const { drop, storage } = makeDrop()
    const before = Date.now()
    await storeAll(drop, 'x', [new Uint8Array([1])])
    const alarm = await storage.getAlarm()
    assert.ok(alarm !== null)
    assert.ok(alarm >= before + TTL_MS)
    assert.ok(alarm <= Date.now() + TTL_MS)
  })
})

describe('DropStore schema upgrade', () => {
  test('a pre-existing meta table without the claimed column is upgraded in place, keeping its rows', async () => {
    const storage = new FakeDurableObjectStorage()
    // The exact pre-batched-protocol shape, populated as the old code would.
    storage.sql.exec(
      `CREATE TABLE meta (
         id INTEGER PRIMARY KEY CHECK (id = 0),
         name TEXT NOT NULL,
         uploadedAt INTEGER NOT NULL,
         totalBytes INTEGER NOT NULL,
         chunkCount INTEGER NOT NULL
       )`,
    )
    storage.sql.exec('CREATE TABLE chunk (idx INTEGER PRIMARY KEY, data BLOB NOT NULL)')
    storage.sql.exec('INSERT INTO meta (id, name, uploadedAt, totalBytes, chunkCount) VALUES (0, ?, ?, ?, ?)', 'old', Date.now(), 3, 1)
    storage.sql.exec('INSERT INTO chunk (idx, data) VALUES (0, ?)', new Uint8Array([1, 2, 3]).buffer)

    const drop = new DropStore(storage)
    assert.deepEqual(await drop.peek(), { exists: true })
    const result = await consumeAll(drop)
    assert.ok(result !== null)
    assert.equal(result.name, 'old')
    assert.deepEqual(Array.from(result.bytes), [1, 2, 3])
    assert.equal(await drop.consume(), null)
  })

  test('an empty old-shape table (a peeked, never-stored token) is upgraded and stays empty', async () => {
    const storage = new FakeDurableObjectStorage()
    storage.sql.exec(
      'CREATE TABLE meta (id INTEGER PRIMARY KEY CHECK (id = 0), name TEXT NOT NULL, uploadedAt INTEGER NOT NULL, totalBytes INTEGER NOT NULL, chunkCount INTEGER NOT NULL)',
    )
    const drop = new DropStore(storage)
    assert.deepEqual(await drop.peek(), { exists: false })
    assert.equal(await drop.consume(), null)
    await storeAll(drop, 'new', [new Uint8Array([9])])
    assert.deepEqual(await drop.peek(), { exists: true })
  })
})

describe('DropStore TTL / expiry', () => {
  test('consume of a drop whose uploadedAt is backdated past TTL_MS returns null', async () => {
    const { drop, storage } = makeDrop()
    await storeAll(drop, 'x', [new Uint8Array([1, 2, 3])])

    const backdated = Date.now() - TTL_MS - 1
    storage.sql.exec('UPDATE meta SET uploadedAt = ? WHERE id = 0', backdated)

    assert.equal(await drop.consume(), null)
  })

  test('a drop consumed just under TTL_MS still succeeds', async () => {
    const { drop, storage } = makeDrop()
    await storeAll(drop, 'x', [new Uint8Array([1, 2, 3])])

    const justUnder = Date.now() - TTL_MS + 5_000
    storage.sql.exec('UPDATE meta SET uploadedAt = ? WHERE id = 0', justUnder)

    assert.notEqual(await drop.consume(), null)
  })

  // `ShareDrop.alarm()` (the DO wrapper) simply calls `DropStore.destroy()`;
  // these prove the wipe semantics the alarm relies on. The alarm firing
  // itself is exercised end to end against real `workerd`.
  test('destroy (what the alarm handler calls) wipes an unconsumed drop', async () => {
    const { drop } = makeDrop()
    await storeAll(drop, 'x', [new Uint8Array([1, 2, 3])])

    await drop.destroy()

    assert.equal(await drop.consume(), null)
  })

  test('destroy against an already-consumed drop is harmless', async () => {
    const { drop } = makeDrop()
    await storeAll(drop, 'x', [new Uint8Array([1])])
    await consumeAll(drop)

    await assert.doesNotReject(() => drop.destroy())
  })
})

describe('DropStore.peek', () => {
  test('returns {exists:true} for a freshly stored drop', async () => {
    const { drop } = makeDrop()
    await storeAll(drop, 'x', [new Uint8Array([1, 2, 3])])

    assert.deepEqual(await drop.peek(), { exists: true })
  })

  test('does not consume: a peek followed by consume still returns the bytes', async () => {
    const { drop } = makeDrop()
    const payload = new Uint8Array([1, 2, 3])
    await storeAll(drop, 'x', [payload])

    assert.deepEqual(await drop.peek(), { exists: true })
    const result = await consumeAll(drop)
    assert.ok(result !== null)
    assert.deepEqual(Array.from(result.bytes), Array.from(payload))
  })

  test('returns {exists:false} after the drop has been consumed', async () => {
    const { drop } = makeDrop()
    await storeAll(drop, 'x', [new Uint8Array([1])])
    await consumeAll(drop)

    assert.deepEqual(await drop.peek(), { exists: false })
  })

  test('returns {exists:false} for a drop backdated past TTL_MS', async () => {
    const { drop, storage } = makeDrop()
    await storeAll(drop, 'x', [new Uint8Array([1])])

    const backdated = Date.now() - TTL_MS - 1
    storage.sql.exec('UPDATE meta SET uploadedAt = ? WHERE id = 0', backdated)

    assert.deepEqual(await drop.peek(), { exists: false })
  })

  test('returns {exists:true} for a drop just under TTL_MS', async () => {
    const { drop, storage } = makeDrop()
    await storeAll(drop, 'x', [new Uint8Array([1])])

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
    await storeAll(drop, 'x', [new Uint8Array([1, 2, 3])])

    await drop.destroy()

    assert.equal(await drop.consume(), null)
  })

  test('is idempotent: calling it twice, or on an empty drop, never throws', async () => {
    const { drop } = makeDrop()
    await assert.doesNotReject(() => drop.destroy())
    await assert.doesNotReject(() => drop.destroy())

    await storeAll(drop, 'x', [new Uint8Array([1])])
    await drop.destroy()
    await assert.doesNotReject(() => drop.destroy())
  })

  test('cancels the pending alarm', async () => {
    const { drop, storage } = makeDrop()
    await storeAll(drop, 'x', [new Uint8Array([1])])
    assert.notEqual(await storage.getAlarm(), null)

    await drop.destroy()

    assert.equal(await storage.getAlarm(), null)
  })
})

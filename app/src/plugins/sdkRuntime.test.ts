/**
 * The SDK runtime (`sdk/plugin/src/core.ts`) against a fake port: what a
 * plugin's `hew` does, message by message.
 */

import { beforeEach, describe, expect, it, vi } from 'vitest'
import { requiresPreview } from '../test/requiresPreview'
import { createRuntime, HewRefusal } from '../../../sdk/plugin/src/core'
import type { HostToRealm, RealmToHost } from '../../../sdk/plugin/src/protocol'
import { checkRealmMessage } from './realm/validate'
import { readForwardReply } from './hostCore'

const HELLO: HostToRealm = {
  t: 'hello',
  plugin: 'com.example.hello',
  version: '0.1.0',
  realm: 'background',
  document: 'doc_1',
  locale: 'en',
  theme: 'dark',
  grant: '{"plugin":"com.example.hello"}',
  api: 1,
  activation: 'command',
  features: ['background.worker'],
}

function setup() {
  const sent: RealmToHost[] = []
  const spies = { log: vi.fn(), info: vi.fn(), warn: vi.fn(), error: vi.fn() }
  const consoleSink = { ...spies }
  let now = 0
  const rt = createRuntime({
    post: (m) => {
      // Every message the runtime sends passes the host's validator.
      const checked = checkRealmMessage(structuredClone(m))
      if (!checked.ok) throw new Error(`the runtime sent something the host refuses: ${checked.reason}`)
      sent.push(m)
    },
    console: consoleSink,
    now: () => now,
  })
  const flush = async () => {
    for (let i = 0; i < 5; i++) await Promise.resolve()
  }
  const last = <T extends RealmToHost['t']>(t: T) => [...sent].reverse().find((m) => m.t === t) as Extract<RealmToHost, { t: T }>
  const answerBus = (result: unknown) => {
    const bus = last('bus')
    const id = (JSON.parse(bus.frame) as { id: unknown }).id
    rt.receive({ t: 'bus', frame: JSON.stringify({ jsonrpc: '2.0', id, result }) })
  }
  return { rt, sent, consoleSink, spies, flush, last, answerBus, setNow: (n: number) => (now = n) }
}

describe('the SDK runtime', () => {
  beforeEach((ctx) => requiresPreview(ctx, 'plugins'))

  it('holds everything until hello, then acknowledges and sends it in order', async () => {
    const { rt, sent } = setup()
    rt.hew.commands.handle('greet', () => 'hi')
    ;(rt.hew as unknown as { api: { query: { scene(p: object): Promise<unknown> } } }).api.query.scene({})
    expect(sent).toEqual([])
    expect(rt.hew.plugin).toBeNull()
    rt.receive(HELLO)
    expect(sent[0]).toEqual({ t: 'hello_ack' })
    expect(sent[1]).toMatchObject({ t: 'bus' })
    expect(rt.hew.plugin).toMatchObject({ id: 'com.example.hello', document: 'doc_1', grant: { plugin: 'com.example.hello' } })
    expect(rt.hew.supports('background.worker')).toBe(true)
    expect(rt.hew.supports('surface.webgl')).toBe(false)
    await rt.hew.ready
  })

  it('answers a forward with the handler result, a refusal, or crashed for a thrown error', async () => {
    const { rt, flush, last, sent } = setup()
    rt.receive(HELLO)
    rt.hew.commands.handle('ok', (p) => ({ echo: p }))
    rt.hew.commands.handle('nothing', () => undefined)
    rt.hew.commands.handle('refuse', () => {
      throw new HewRefusal('too_big', 'That is too big.', { max: 3 })
    })
    rt.hew.commands.handle('bug', async () => {
      throw new TypeError('oops')
    })
    const forward = (id: number, local: string): void =>
      rt.receive({ t: 'forward', id, local, params: '{"name":"Kurt"}', caller: '{"identity":"live:1"}', timeout_ms: 30_000 })

    forward(1, 'ok')
    await flush()
    expect(JSON.parse(last('forward_reply').reply)).toEqual({ ok: { echo: { name: 'Kurt' } } })
    forward(2, 'nothing')
    await flush()
    expect(JSON.parse(last('forward_reply').reply)).toEqual({ ok: null })
    forward(3, 'refuse')
    await flush()
    expect(JSON.parse(last('forward_reply').reply)).toEqual({ refusal: { name: 'too_big', explanation: 'That is too big.', detail: { max: 3 } } })
    forward(4, 'bug')
    await flush()
    expect(last('forward_reply')).toEqual({ t: 'forward_reply', id: 4, reply: '{"failed":"crashed"}' })
    expect(last('error').message).toBe('TypeError: oops')
    forward(5, 'missing')
    expect(last('forward_reply')).toEqual({ t: 'forward_reply', id: 5, reply: '{"failed":"no_handler"}' })
    expect(sent.filter((m) => m.t === 'forward_reply').map((m) => (m as { id: number }).id)).toEqual([1, 2, 3, 4, 5])
  })

  it('refuses a refusal name that is not snake_case', () => {
    expect(() => new HewRefusal('Bad Name', 'x')).toThrow(TypeError)
  })

  it('stamps bus calls a handler makes with its forward, and aborts its signal on cancel without answering', async () => {
    const { rt, flush, last, sent } = setup()
    rt.receive(HELLO)
    let signal: AbortSignal | null = null
    rt.hew.commands.handle('count', async (_p, ctx) => {
      signal = ctx.signal
      ctx.progress(0.5, 'Counting')
      const scene = (await rt.hew.api.query.scene({})) as { tree: unknown[] }
      return { n: scene.tree.length }
    })
    rt.receive({ t: 'forward', id: 7, local: 'count', params: '{}', caller: '{}', timeout_ms: 1000 })
    await flush()
    expect(last('forward_progress')).toEqual({ t: 'forward_progress', id: 7, fraction: 0.5, label: 'Counting' })
    expect(last('bus').forward).toBe(7)
    rt.receive({ t: 'forward_cancel', id: 7 })
    expect(signal!.aborted).toBe(true)
    const before = sent.length
    const bus = last('bus')
    rt.receive({ t: 'bus', frame: JSON.stringify({ jsonrpc: '2.0', id: (JSON.parse(bus.frame) as { id: number }).id, result: { tree: [1, 2] } }) })
    await flush()
    // The host already answered a cancelled forward: nothing more is sent.
    expect(sent.slice(before).filter((m) => m.t === 'forward_reply')).toEqual([])
  })

  it('reports nothing when a handler fails after its caller gave up', async () => {
    const { rt, flush, sent } = setup()
    rt.receive(HELLO)
    rt.hew.commands.handle('wait', (_p, ctx) => new Promise((_r, reject) => ctx.signal.addEventListener('abort', () => reject(new Error('aborted')))))
    rt.receive({ t: 'forward', id: 3, local: 'wait', params: '{}', caller: '{}', timeout_ms: 1000 })
    await flush()
    const before = sent.length
    rt.receive({ t: 'forward_cancel', id: 3 })
    await flush()
    // Neither an answer nor an error: an error would stop the plugin.
    expect(sent.slice(before)).toEqual([])
  })

  it('transact sends one envelope and expects the generation of the last read', async () => {
    const { rt, flush, last, answerBus } = setup()
    rt.receive(HELLO)
    const read = rt.hew.api.query.scene({})
    answerBus({ tree: [], generation: 12 })
    await read
    void rt.hew.transact([{ method: 'hew.sketch.draw_rect', params: {} }], { label: 'Box' })
    await flush()
    const frame = JSON.parse(last('bus').frame) as { method: string; params: Record<string, unknown> }
    expect(frame.method).toBe('hew.doc.transact')
    expect(frame.params).toEqual({ commands: [{ method: 'hew.sketch.draw_rect', params: {} }], label: 'Box', expect_generation: 12 })
  })

  it('commit retries once when the document moved', async () => {
    const { rt, flush, last, answerBus } = setup()
    rt.receive(HELLO)
    const fn = vi.fn(() => [{ method: 'hew.tag.create', params: {} }])
    const done = rt.hew.commit(fn)
    for (const generation of [1, 2]) {
      await flush()
      answerBus({ tree: [], generation })
      await flush()
      const bus = last('bus')
      const id = (JSON.parse(bus.frame) as { id: unknown }).id
      expect((JSON.parse(bus.frame) as { params: { expect_generation: number } }).params.expect_generation).toBe(generation)
      rt.receive({
        t: 'bus',
        frame: JSON.stringify(
          generation === 1
            ? { jsonrpc: '2.0', id, error: { code: -32000, message: 'refused', data: { refusal: 'generation_moved', failed_index: 0, failed_method: 'hew.doc.transact', detail: null, explanation: 'moved' } } }
            : { jsonrpc: '2.0', id, result: { results: [{}], label: 'x' } },
        ),
      })
    }
    await expect(done).resolves.toEqual({ results: [{}], label: 'x' })
    expect(fn).toHaveBeenCalledTimes(2)
  })

  it('subscribes once per event name and delivers event frames', async () => {
    const { rt, sent } = setup()
    rt.receive(HELLO)
    const a = vi.fn()
    const b = vi.fn()
    rt.hew.events.on('document_changed', a)
    rt.hew.events.on('document_changed', b)
    const subscribes = sent.filter((m) => m.t === 'bus' && m.frame.includes('hew.event.subscribe'))
    expect(subscribes).toHaveLength(1)
    rt.receive({ t: 'bus', frame: '{"jsonrpc":"2.0","method":"hew.event.document_changed","params":{"generation":4}}' })
    expect(a).toHaveBeenCalledWith({ generation: 4 })
    expect(b).toHaveBeenCalledTimes(1)
  })

  it('asks for a file and resolves its bytes, or refuses a missing one', async () => {
    const { rt, last } = setup()
    rt.receive(HELLO)
    const p = rt.hew.assets.bytes('icon.svg')
    const ask = last('files')
    expect(ask.paths).toEqual(['icon.svg'])
    const bytes = new Uint8Array([1, 2]).buffer
    rt.receive({ t: 'files', id: ask.id, entries: [{ path: 'icon.svg', bytes }], missing: [] })
    expect(new Uint8Array(await p)).toEqual(new Uint8Array([1, 2]))
    const q = rt.hew.assets.bytes('nope.txt')
    rt.receive({ t: 'files', id: last('files').id, entries: [], missing: ['nope.txt'] })
    await expect(q).rejects.toMatchObject({ name: 'file_missing' })
  })

  it('round-trips storage as JSON text, and surfaces a quota refusal by name', async () => {
    const { rt, last } = setup()
    rt.receive(HELLO)
    const set = rt.hew.storage.set('count', { n: 2 })
    const s = last('storage')
    expect(s).toEqual({ t: 'storage', id: s.id, op: 'set', key: 'count', value: '{"n":2}' })
    rt.receive({ t: 'storage_reply', id: s.id, ok: true })
    await set
    const get = rt.hew.storage.get('count')
    rt.receive({ t: 'storage_reply', id: last('storage').id, ok: true, value: '{"n":2}' })
    await expect(get).resolves.toEqual({ n: 2 })
    const missing = rt.hew.storage.get('none')
    rt.receive({ t: 'storage_reply', id: last('storage').id, ok: true, value: null })
    await expect(missing).resolves.toBeUndefined()
    const full = rt.hew.storage.set('big', 'x')
    rt.receive({ t: 'storage_reply', id: last('storage').id, ok: false, error: 'storage_quota' })
    await expect(full).rejects.toBeInstanceOf(HewRefusal)
    await expect(full).rejects.toMatchObject({ name: 'storage_quota' })
    await expect(rt.hew.storage.set('u', undefined)).rejects.toThrow(TypeError)
    const changed = vi.fn()
    rt.hew.storage.onChange(changed)
    rt.receive({ t: 'storage_changed', scope: 'plugin', keys: ['count'] })
    expect(changed).toHaveBeenCalledWith(['count'])
  })

  it('mirrors the console, cut to size and rate-limited', () => {
    const { rt, sent, consoleSink, spies, setNow } = setup()
    rt.receive(HELLO)
    const original = spies.info
    ;(consoleSink as unknown as Console).info('started', { a: 1 }, 'x'.repeat(10_000))
    expect(original).toHaveBeenCalled()
    const first = sent.find((m) => m.t === 'console') as Extract<RealmToHost, { t: 'console' }>
    expect(first.level).toBe('info')
    expect(first.args[0]).toBe('started')
    expect(first.args[1]).toBe('{"a":1}')
    expect(first.args[2].length).toBeLessThanOrEqual(2_000)
    for (let i = 0; i < 500; i++) (consoleSink as unknown as Console).log(i)
    expect(sent.filter((m) => m.t === 'console').length).toBe(50)
    setNow(1_500)
    ;(consoleSink as unknown as Console).log('later')
    const tail = sent.filter((m) => m.t === 'console').slice(-2) as Extract<RealmToHost, { t: 'console' }>[]
    expect(tail[0].args[0]).toMatch(/dropped/)
    expect(tail[1].args[0]).toBe('later')
  })

  it('answers ping at once, and runs unload listeners before saying unloaded', () => {
    const { rt, sent } = setup()
    rt.receive({ t: 'ping', n: 3 })
    expect(sent).toEqual([{ t: 'pong', n: 3 }])
    rt.receive(HELLO)
    const fn = vi.fn()
    rt.hew.lifecycle.on('unload', fn)
    rt.receive({ t: 'unload', reason: 'disable' })
    expect(fn).toHaveBeenCalledWith('disable')
    expect(sent.at(-1)).toEqual({ t: 'unloaded' })
  })

  describe('what a well-behaved plugin can ask for never trips a host limit', () => {
    it('refuses a storage key over 256 UTF-8 bytes by name, before sending it', async () => {
      const { rt, sent } = setup()
      rt.receive(HELLO)
      const storageSent = () => sent.filter((m) => m.t === 'storage').length
      void rt.hew.storage.set('k'.repeat(256), 1)
      void rt.hew.storage.set('\u00e9'.repeat(128), 1)
      void rt.hew.storage.set('\u{1F600}'.repeat(64), 1)
      expect(storageSent()).toBe(3)
      for (const key of ['k'.repeat(257), '\u00e9'.repeat(129), '\u{1F600}'.repeat(65)]) {
        await expect(rt.hew.storage.set(key, 1)).rejects.toMatchObject({ name: 'storage_key_invalid' })
        await expect(rt.hew.storage.get(key)).rejects.toBeInstanceOf(HewRefusal)
        await expect(rt.hew.storage.delete(key)).rejects.toMatchObject({ name: 'storage_key_invalid' })
      }
      await expect(rt.hew.storage.set('', 1)).rejects.toThrow(TypeError)
      // A surrogate without its pair is no text a store can keep.
      for (const key of ['\ud800', 'a\udc00b', 'x\ud83d']) {
        await expect(rt.hew.storage.set(key, 1)).rejects.toMatchObject({ name: 'storage_key_invalid' })
        await expect(rt.hew.storage.get(key)).rejects.toMatchObject({ name: 'storage_key_invalid' })
      }
      expect(storageSent()).toBe(3)
    })

    it('refuses a stored value over 1,000,000 characters of JSON by name, before sending it', async () => {
      const { rt, sent } = setup()
      rt.receive(HELLO)
      // Two quote characters make it exactly 1,000,000.
      void rt.hew.storage.set('k'.repeat(256), 'x'.repeat(999_998))
      await expect(rt.hew.storage.set('k', 'x'.repeat(999_999))).rejects.toMatchObject({ name: 'storage_value_too_large' })
      await expect(rt.hew.storage.set('k', 'x'.repeat(1_100_000))).rejects.toBeInstanceOf(HewRefusal)
      // The one that went out fits the host's message limit (setup checks every message).
      expect(sent.filter((m) => m.t === 'storage')).toHaveLength(1)
    })

    it('keeps at most 32 storage requests and 8 file requests open, and sends the rest as answers come', () => {
      const { rt, sent, last } = setup()
      rt.receive(HELLO)
      for (let i = 0; i < 100; i++) void rt.hew.storage.set(`k${i}`, i)
      for (let i = 0; i < 20; i++) rt.hew.assets.bytes(`f${i}.txt`).catch(() => {})
      const storageMsgs = () => sent.filter((m) => m.t === 'storage') as Extract<RealmToHost, { t: 'storage' }>[]
      const fileMsgs = () => sent.filter((m) => m.t === 'files')
      expect(storageMsgs()).toHaveLength(32)
      expect(fileMsgs()).toHaveLength(8)
      rt.receive({ t: 'storage_reply', id: storageMsgs()[0].id, ok: true })
      rt.receive({ t: 'files', id: last('files').id, entries: [], missing: ['f7.txt'] })
      return new Promise<void>((done) =>
        setTimeout(() => {
          expect(storageMsgs()).toHaveLength(33)
          expect(storageMsgs().at(-1)!.key).toBe('k32')
          expect(fileMsgs()).toHaveLength(9)
          done()
        }, 0),
      )
    })

    it('refuses a package path longer than the host reads, before sending it', async () => {
      const { rt, sent } = setup()
      rt.receive(HELLO)
      await expect(rt.hew.assets.bytes('a/'.repeat(600))).rejects.toMatchObject({ name: 'file_missing' })
      expect(sent.filter((m) => m.t === 'files')).toEqual([])
    })

    it("cuts a refusal's explanation to what the host accepts", async () => {
      const { rt, flush, last } = setup()
      rt.receive(HELLO)
      rt.hew.commands.handle('long', () => {
        throw new HewRefusal('too_long', 'x'.repeat(10_000))
      })
      rt.receive({ t: 'forward', id: 1, local: 'long', params: '{}', caller: '{}', timeout_ms: 1000 })
      await flush()
      const reply = JSON.parse(last('forward_reply').reply) as { refusal: { explanation: string } }
      expect(reply.refusal.explanation.length).toBe(2_000)
      expect(readForwardReply(last('forward_reply').reply)).not.toBeNull()
    })

    it('rejects a bus request too large to send instead of sending it', async () => {
      const { rt, sent } = setup()
      rt.receive(HELLO)
      const before = sent.length
      await expect(rt.hew.transact([{ method: 'hew.tag.create', params: { path: 'x'.repeat(1_000_001) } }])).rejects.toThrow(RangeError)
      expect(sent.length).toBe(before)
    })
  })
})

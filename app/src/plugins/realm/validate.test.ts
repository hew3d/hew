import { beforeEach, describe, expect, it } from 'vitest'
import { requiresPreview } from '../../test/requiresPreview'
import { REALM_TO_HOST_TYPES, RUNTIME_LIMITS } from '../../../../sdk/plugin/src/protocol'
import { checkBootMessage, checkRealmMessage, LIMITS, RateMeter } from './validate'

const ok = (data: unknown) => {
  const r = checkRealmMessage(data)
  if (!r.ok) throw new Error(`refused: ${r.reason}`)
  return r
}
const refused = (data: unknown): string => {
  const r = checkRealmMessage(data)
  if (r.ok) throw new Error(`accepted ${JSON.stringify(data)}`)
  return r.reason
}

describe('checkRealmMessage', () => {
  beforeEach((ctx) => requiresPreview(ctx, 'plugins'))

  it('accepts every message the protocol defines, in its documented shape', () => {
    const samples: Record<(typeof REALM_TO_HOST_TYPES)[number], unknown> = {
      hello_ack: { t: 'hello_ack' },
      bus: { t: 'bus', forward: 3, frame: '{"jsonrpc":"2.0","id":1,"method":"hew.query.scene","params":{}}' },
      forward_reply: { t: 'forward_reply', id: 3, reply: '{"ok":{}}' },
      forward_progress: { t: 'forward_progress', id: 3, fraction: 0.5, label: 'Halfway' },
      storage: { t: 'storage', id: 1, op: 'set', key: 'k', value: '1' },
      files: { t: 'files', id: 2, paths: ['icon.svg'] },
      console: { t: 'console', level: 'info', args: ['started'], at: 1 },
      pong: { t: 'pong', n: 4 },
      unloaded: { t: 'unloaded' },
      error: { t: 'error', message: 'TypeError: x', stack: 'at y' },
    }
    for (const t of REALM_TO_HOST_TYPES) expect(ok(samples[t]).msg.t).toBe(t)
  })

  it('measures a message as the sum of its text fields', () => {
    expect(ok({ t: 'bus', frame: 'abcd' }).size).toBe(4)
    expect(ok({ t: 'storage', id: 1, op: 'set', key: 'ab', value: 'cde' }).size).toBe(5)
    expect(ok({ t: 'console', level: 'log', args: ['a', 'bc'], at: 0 }).size).toBe(3)
    expect(ok({ t: 'error', message: 'ab', stack: 'c' }).size).toBe(3)
  })

  it('refuses anything that is not a plain object with a known type', () => {
    for (const bad of [null, undefined, 1, 'bus', [], new ArrayBuffer(4), new Uint8Array(2), { t: 'nope' }, { t: 7 }, {}]) {
      expect(checkRealmMessage(bad).ok, String(bad)).toBe(false)
    }
    expect(refused({ t: 'toString' })).toBe('unknown message type')
    expect(refused({ t: '__proto__' })).toBe('unknown message type')
  })

  it('refuses extra fields, wrong types and out-of-range numbers', () => {
    expect(refused({ t: 'hello_ack', x: 1 })).toMatch(/too many fields|unknown field/)
    expect(refused({ t: 'bus', frame: 'x', extra: 1 })).toMatch(/unknown field/)
    expect(refused({ t: 'bus', frame: 1 })).toMatch(/frame is not a string/)
    expect(refused({ t: 'bus', frame: '' })).toMatch(/empty/)
    expect(refused({ t: 'bus', frame: 'x', forward: -1 })).toMatch(/non-negative integer/)
    expect(refused({ t: 'bus', frame: 'x', forward: 1.5 })).toMatch(/non-negative integer/)
    expect(refused({ t: 'bus', frame: 'x', forward: '1' })).toMatch(/non-negative integer/)
    expect(refused({ t: 'forward_reply', id: 1, reply: { ok: 1 } })).toMatch(/reply is not a string/)
    expect(refused({ t: 'forward_progress', id: 1, fraction: 2 })).toMatch(/fraction/)
    expect(refused({ t: 'forward_progress', id: 1, fraction: Number.NaN })).toMatch(/fraction/)
    expect(refused({ t: 'pong', n: Number.MAX_SAFE_INTEGER + 2 })).toMatch(/non-negative integer/)
    expect(refused({ t: 'console', level: 'debug', args: [], at: 0 })).toMatch(/console level/)
    expect(refused({ t: 'console', level: 'log', args: [{}], at: 0 })).toMatch(/short text/)
    expect(refused({ t: 'console', level: 'log', args: 'x', at: 0 })).toMatch(/not a list/)
    expect(refused({ t: 'console', level: 'log', args: [], at: Infinity })).toMatch(/time/)
  })

  it('requires a document epoch on document storage, an integer in range, and refuses one on the plugin\'s own', () => {
    expect(ok({ t: 'storage', id: 1, op: 'set', scope: 'document', key: 'k', value: '1', epoch: 7 }).msg).toMatchObject({ scope: 'document', epoch: 7 })
    expect(ok({ t: 'storage', id: 1, op: 'get', scope: 'document', key: 'k', epoch: Number.MAX_SAFE_INTEGER }).msg).toMatchObject({ epoch: Number.MAX_SAFE_INTEGER })
    expect(refused({ t: 'storage', id: 1, op: 'list', scope: 'document' })).toMatch(/needs epoch/)
    for (const epoch of [-1, 1.5, '1', null, Number.NaN, Number.MAX_SAFE_INTEGER + 2, [1]]) {
      expect(refused({ t: 'storage', id: 1, op: 'list', scope: 'document', epoch }), String(epoch)).toMatch(/epoch is not a non-negative integer/)
    }
    expect(refused({ t: 'storage', id: 1, op: 'list', scope: 'plugin', epoch: 0 })).toMatch(/takes no epoch/)
    expect(refused({ t: 'storage', id: 1, op: 'list', epoch: 0 })).toMatch(/takes no epoch/)
    // Seven fields at most: the epoch is counted among them.
    expect(ok({ t: 'storage', id: 1, op: 'set', scope: 'document', key: 'k', value: 'v', epoch: 0 }).size).toBe(2)
  })

  it('holds each storage operation to its own fields', () => {
    expect(ok({ t: 'storage', id: 1, op: 'get', key: 'k' }).msg).toEqual({ t: 'storage', id: 1, op: 'get', scope: 'plugin', key: 'k' })
    expect(ok({ t: 'storage', id: 1, op: 'list' }).msg).toEqual({ t: 'storage', id: 1, op: 'list', scope: 'plugin' })
    // A scope, when given, is one of the two; absent is the plugin's own.
    expect(ok({ t: 'storage', id: 1, op: 'list', scope: 'document', epoch: 0 }).msg).toEqual({ t: 'storage', id: 1, op: 'list', scope: 'document', epoch: 0 })
    expect(ok({ t: 'storage', id: 1, op: 'list', scope: 'plugin' }).msg).toEqual({ t: 'storage', id: 1, op: 'list', scope: 'plugin' })
    for (const scope of ['window', '', 1, null, ['document']]) {
      expect(refused({ t: 'storage', id: 1, op: 'list', scope })).toMatch(/unknown storage scope/)
    }
    expect(refused({ t: 'storage', id: 1, op: 'get' })).toMatch(/needs key/)
    expect(refused({ t: 'storage', id: 1, op: 'set', key: 'k' })).toMatch(/needs value/)
    expect(refused({ t: 'storage', id: 1, op: 'get', key: 'k', value: 'v' })).toMatch(/takes no value/)
    expect(refused({ t: 'storage', id: 1, op: 'list', key: 'k' })).toMatch(/takes no key/)
    expect(refused({ t: 'storage', id: 1, op: 'drop', key: 'k' })).toMatch(/unknown storage operation/)
    expect(refused({ t: 'storage', id: 1, op: 'get', key: '' })).toMatch(/empty/)
    expect(refused({ t: 'storage', id: 1, op: 'get', key: 'k'.repeat(LIMITS.storageKeyBytes + 1) })).toMatch(/longer/)
    // A surrogate without its pair, as the SDK refuses it before sending.
    expect(refused({ t: 'storage', id: 1, op: 'set', key: '\ud800', value: '1' })).toMatch(/not well-formed/)
    expect(refused({ t: 'storage', id: 1, op: 'get', key: 'a\udc00' })).toMatch(/not well-formed/)
    expect(ok({ t: 'storage', id: 1, op: 'get', key: 'a\u{1F600}' }).ok).toBe(true)
    // The cap is 256 UTF-8 bytes, however many characters that is.
    expect(ok({ t: 'storage', id: 1, op: 'get', key: 'k'.repeat(256) }).ok).toBe(true)
    expect(ok({ t: 'storage', id: 1, op: 'get', key: '\u00e9'.repeat(128) }).ok).toBe(true)
    expect(ok({ t: 'storage', id: 1, op: 'get', key: '\u{1F600}'.repeat(64) }).ok).toBe(true)
    expect(refused({ t: 'storage', id: 1, op: 'get', key: '\u00e9'.repeat(129) })).toMatch(/longer than 256 bytes/)
    expect(refused({ t: 'storage', id: 1, op: 'get', key: '\u20ac'.repeat(86) })).toMatch(/longer than 256 bytes/)
    expect(refused({ t: 'storage', id: 1, op: 'get', key: '\u{1F600}'.repeat(65) })).toMatch(/longer than 256 bytes/)
    // A stored value crosses as text: an object is refused.
    expect(refused({ t: 'storage', id: 1, op: 'set', key: 'k', value: { a: 1 } })).toMatch(/not a string/)
  })

  it('caps list lengths before reading their entries', () => {
    const args = new Array(LIMITS.consoleArgs + 1).fill('x')
    expect(refused({ t: 'console', level: 'log', args, at: 0 })).toMatch(/more than/)
    expect(refused({ t: 'files', id: 1, paths: [] })).toMatch(/empty/)
    expect(refused({ t: 'files', id: 1, paths: new Array(LIMITS.filesPerRequest + 1).fill('a') })).toMatch(/more than/)
    // A list with a huge declared length and nothing in it costs nothing.
    const sparse: string[] = []
    sparse.length = 1_000_000
    expect(refused({ t: 'console', level: 'log', args: sparse, at: 0 })).toMatch(/more than/)
  })

  it('refuses a message over 1 MiB of text, and a field over its own cap', () => {
    expect(ok({ t: 'bus', frame: 'x'.repeat(LIMITS.messageChars) }).size).toBe(LIMITS.messageChars)
    expect(refused({ t: 'bus', frame: 'x'.repeat(LIMITS.messageChars + 1) })).toMatch(/longer/)
    expect(refused({ t: 'storage', id: 1, op: 'set', key: 'kk', value: 'x'.repeat(LIMITS.messageChars - 1) })).toBe('message too large')
    expect(refused({ t: 'error', message: 'x'.repeat(LIMITS.errorMessageChars + 1) })).toMatch(/longer/)
  })

  it('never throws, whatever it is given', () => {
    const weird = Object.create(null) as Record<string, unknown>
    weird.t = 'bus'
    weird.frame = 'x'
    expect(() => checkRealmMessage(weird)).not.toThrow()
    const getter = { t: 'bus', get frame(): string { throw new Error('boom') } }
    expect(() => checkRealmMessage(getter)).not.toThrow()
    expect(checkRealmMessage(getter).ok).toBe(false)
  })

  it("keeps the runtime's own cuts inside the host's limits", () => {
    expect(RUNTIME_LIMITS.consoleArgs).toBeLessThanOrEqual(LIMITS.consoleArgs)
    expect(RUNTIME_LIMITS.consoleArgChars).toBeLessThanOrEqual(LIMITS.consoleArgChars)
    expect(RUNTIME_LIMITS.errorMessageChars).toBeLessThanOrEqual(LIMITS.errorMessageChars)
    expect(RUNTIME_LIMITS.errorStackChars).toBeLessThanOrEqual(LIMITS.errorStackChars)
    expect(RUNTIME_LIMITS.progressLabelChars).toBeLessThanOrEqual(LIMITS.progressLabelChars)
    expect(RUNTIME_LIMITS.frameChars).toBeLessThanOrEqual(LIMITS.messageChars)
    expect(RUNTIME_LIMITS.storageKeyBytes).toBe(LIMITS.storageKeyBytes)
    // A set carries a key (at most 256 characters) and a value: always inside one message.
    expect(RUNTIME_LIMITS.storageKeyBytes + RUNTIME_LIMITS.storageValueChars).toBeLessThanOrEqual(LIMITS.messageChars)
    expect(RUNTIME_LIMITS.pathChars).toBeLessThanOrEqual(LIMITS.pathChars)
    expect(RUNTIME_LIMITS.refusalExplanationChars).toBeLessThanOrEqual(2_000)
    expect(RUNTIME_LIMITS.consolePerSecond).toBeLessThan(LIMITS.messagesPerSecond)
  })
})

describe('checkBootMessage', () => {
  beforeEach((ctx) => requiresPreview(ctx, 'plugins'))

  it('accepts booted and a short error, and nothing else', () => {
    expect(checkBootMessage({ t: 'booted' }).ok).toBe(true)
    expect(checkBootMessage({ t: 'error', message: 'x' }).ok).toBe(true)
    expect(checkBootMessage({ t: 'booted', x: 1 }).ok).toBe(false)
    expect(checkBootMessage({ t: 'bus', frame: 'x' }).ok).toBe(false)
    expect(checkBootMessage({ t: 'error', message: 'x'.repeat(LIMITS.errorMessageChars + 1) }).ok).toBe(false)
  })
})

describe('RateMeter', () => {
  beforeEach((ctx) => requiresPreview(ctx, 'plugins'))

  it('allows 1,000 messages in a second and refuses the next one', () => {
    let now = 0
    const meter = new RateMeter(() => now)
    for (let i = 0; i < LIMITS.messagesPerSecond; i++) expect(meter.add(1)).toBeNull()
    expect(meter.add(1)).toMatch(/messages/)
  })

  it('counts over a rolling second, not a fixed one', () => {
    let now = 0
    const meter = new RateMeter(() => now, 10, 1_000_000)
    for (let i = 0; i < 10; i++) {
      now = 500 + i * 10
      expect(meter.add(1)).toBeNull()
    }
    now = 1_200 // the first ones are still inside the last second
    expect(meter.add(1)).toMatch(/messages/)
    now = 2_000
    expect(new RateMeter(() => now, 10, 1_000_000).add(1)).toBeNull()
  })

  it('lets a steady sender through for long', () => {
    let now = 0
    const meter = new RateMeter(() => now, 10, 1_000_000)
    for (let i = 0; i < 10_000; i++) {
      now += 150
      expect(meter.add(100)).toBeNull()
    }
  })

  it('refuses more than the text budget in a second', () => {
    let now = 0
    const meter = new RateMeter(() => now)
    expect(meter.add(LIMITS.charsPerSecond - 10)).toBeNull()
    now = 999
    expect(meter.add(11)).toMatch(/text/)
  })
})

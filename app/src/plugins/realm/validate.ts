/* oxlint-disable lingui/no-unlocalized-strings -- protocol words and violation reasons for the log; nothing here is shown as UI copy. */
/**
 * The one check every message from a realm passes before the host acts on
 * it. A realm is hostile: its messages are whatever a structured clone of
 * the plugin's choosing deserializes to. So:
 *
 *  - a message is a plain object with a known `t`, exactly the keys that
 *    type allows, and fields of the right type and range;
 *  - every field whose size the sender chooses is a string, so a
 *    message's size is the sum of its strings' lengths, read field by
 *    field; nothing is stringified, walked or copied to measure it, and
 *    the number of keys is counted only up to one past what is allowed;
 *  - a message may hold at most `messageChars` of text, and a realm may
 *    send at most `messagesPerSecond` messages and `charsPerSecond` of
 *    text in any rolling second.
 *
 * Anything else is a violation: the caller crashes the realm. Neither
 * function here throws.
 */

import type { BootToHost, ConsoleLevel, RealmToHost, StorageOp, StorageScope } from '../../../../sdk/plugin/src/protocol'

export const LIMITS = {
  /** Text in one message, in UTF-16 code units (1 MiB). */
  messageChars: 1 << 20,
  /** One transferred buffer. No message a realm sends carries one yet; the
   *  limit is here for the ones that will. */
  transferBytes: 256 << 20,
  messagesPerSecond: 1_000,
  /** Text a realm may send in any rolling second. */
  charsPerSecond: 16 << 20,
  consoleArgs: 32,
  consoleArgChars: 4_096,
  errorMessageChars: 4_096,
  errorStackChars: 16_384,
  progressLabelChars: 256,
  /** A storage key, in UTF-8 bytes, as the stores count it. */
  storageKeyBytes: 256,
  filesPerRequest: 16,
  pathChars: 1_024,
} as const

export type Checked<T> = { ok: true; msg: T; size: number } | { ok: false; reason: string }

const CONSOLE_LEVELS: readonly ConsoleLevel[] = ['log', 'info', 'warn', 'error']
const STORAGE_OPS: readonly StorageOp[] = ['get', 'set', 'delete', 'list']
const STORAGE_SCOPES: readonly StorageScope[] = ['plugin', 'document']

/** The keys each type allows; the first is always `t`. */
const KEYS: Record<RealmToHost['t'], readonly string[]> = {
  hello_ack: ['t'],
  bus: ['t', 'forward', 'frame'],
  forward_reply: ['t', 'id', 'reply'],
  forward_progress: ['t', 'id', 'fraction', 'label'],
  storage: ['t', 'id', 'op', 'scope', 'key', 'value', 'epoch'],
  files: ['t', 'id', 'paths'],
  console: ['t', 'level', 'args', 'at'],
  pong: ['t', 'n'],
  unloaded: ['t'],
  error: ['t', 'message', 'stack'],
}

const BOOT_KEYS: Record<BootToHost['t'], readonly string[]> = {
  booted: ['t'],
  error: ['t', 'message'],
}

class Violation extends Error {}

/** The UTF-8 length of a short string (a lone surrogate counts 3, as its
 *  replacement character would). */
function utf8Bytes(s: string): number {
  let n = 0
  for (let i = 0; i < s.length; i++) {
    const c = s.charCodeAt(i)
    if (c < 0x80) n += 1
    else if (c < 0x800) n += 2
    else if (c >= 0xd800 && c <= 0xdbff && i + 1 < s.length && (s.charCodeAt(i + 1) & 0xfc00) === 0xdc00) {
      n += 4
      i++
    } else n += 3
  }
  return n
}
/** Whether `s` is well-formed UTF-16: no surrogate without its pair. A
 *  store cannot keep such a key (the desktop's JSON refuses it), and the
 *  SDK refuses it `storage_key_invalid` before sending, as it does a key
 *  that is too long. */
export function wellFormed(s: string): boolean {
  for (let i = 0; i < s.length; i++) {
    const c = s.charCodeAt(i)
    if (c < 0xd800 || c > 0xdfff) continue
    if (c > 0xdbff || i + 1 >= s.length || (s.charCodeAt(i + 1) & 0xfc00) !== 0xdc00) return false
    i++
  }
  return true
}
const bad = (reason: string): never => {
  throw new Violation(reason)
}

const isRecord = (v: unknown): v is Record<string, unknown> =>
  typeof v === 'object' && v !== null && !Array.isArray(v) && !(v instanceof ArrayBuffer) && !ArrayBuffer.isView(v)

/** Refuses a key outside `allowed`, counting at most `allowed.length + 1`. */
function onlyKeys(m: Record<string, unknown>, allowed: readonly string[]): void {
  let n = 0
  for (const key in m) {
    if (++n > allowed.length) bad('too many fields')
    if (!allowed.includes(key)) bad(`unknown field "${key.slice(0, 32)}"`)
  }
}

const has = (m: Record<string, unknown>, key: string): boolean => Object.prototype.hasOwnProperty.call(m, key)

function id(m: Record<string, unknown>, key: string): number {
  const v = m[key]
  if (typeof v !== 'number' || !Number.isSafeInteger(v) || v < 0) bad(`${key} is not a non-negative integer`)
  return v as number
}

function text(m: Record<string, unknown>, key: string, max: number, min = 0): string {
  const v = m[key]
  if (typeof v !== 'string') bad(`${key} is not a string`)
  const s = v as string
  if (s.length > max) bad(`${key} is longer than ${max}`)
  if (s.length < min) bad(`${key} is empty`)
  return s
}

function strings(m: Record<string, unknown>, key: string, maxItems: number, maxChars: number, minItems = 0): string[] {
  const v = m[key]
  if (!Array.isArray(v)) bad(`${key} is not a list`)
  const list = v as unknown[]
  if (list.length > maxItems) bad(`${key} has more than ${maxItems} entries`)
  if (list.length < minItems) bad(`${key} is empty`)
  for (let i = 0; i < list.length; i++) {
    const s = list[i]
    if (typeof s !== 'string' || s.length > maxChars) bad(`${key} holds something other than short text`)
  }
  return list as string[]
}

function checkRealm(data: unknown): { msg: RealmToHost; size: number } {
  if (!isRecord(data)) bad('not an object')
  const m = data as Record<string, unknown>
  const t = m.t
  if (typeof t !== 'string' || !Object.prototype.hasOwnProperty.call(KEYS, t)) bad('unknown message type')
  onlyKeys(m, KEYS[t as RealmToHost['t']])
  const big = LIMITS.messageChars
  switch (t as RealmToHost['t']) {
    case 'hello_ack':
    case 'unloaded':
      return { msg: { t } as RealmToHost, size: 0 }
    case 'bus': {
      const frame = text(m, 'frame', big, 1)
      if (has(m, 'forward')) return { msg: { t: 'bus', forward: id(m, 'forward'), frame }, size: frame.length }
      return { msg: { t: 'bus', frame }, size: frame.length }
    }
    case 'forward_reply': {
      const reply = text(m, 'reply', big, 1)
      return { msg: { t: 'forward_reply', id: id(m, 'id'), reply }, size: reply.length }
    }
    case 'forward_progress': {
      const f = m.fraction
      if (f !== null && (typeof f !== 'number' || !Number.isFinite(f) || f < 0 || f > 1)) bad('fraction is not null or 0 to 1')
      const msg: RealmToHost = { t: 'forward_progress', id: id(m, 'id'), fraction: f as number | null }
      if (has(m, 'label')) msg.label = text(m, 'label', LIMITS.progressLabelChars)
      return { msg, size: msg.label?.length ?? 0 }
    }
    case 'storage': {
      const op = m.op
      if (typeof op !== 'string' || !STORAGE_OPS.includes(op as StorageOp)) bad('unknown storage operation')
      const needsKey = op !== 'list'
      const needsValue = op === 'set'
      if (has(m, 'key') !== needsKey) bad(`a storage ${op as string} ${needsKey ? 'needs' : 'takes no'} key`)
      if (has(m, 'value') !== needsValue) bad(`a storage ${op as string} ${needsValue ? 'needs' : 'takes no'} value`)
      // `scope` absent is the plugin's own storage, as before there was
      // a document scope; present, it must be one of the two.
      let scope: StorageScope = 'plugin'
      if (has(m, 'scope')) {
        if (typeof m.scope !== 'string' || !STORAGE_SCOPES.includes(m.scope as StorageScope)) bad('unknown storage scope')
        scope = m.scope as StorageScope
      }
      const msg: RealmToHost = { t: 'storage', id: id(m, 'id'), op: op as StorageOp, scope }
      // The document epoch the call was made under: required for the
      // document's storage (the host refuses a stale one), refused for the
      // plugin's own, which no document change touches.
      if (scope === 'document') {
        if (!has(m, 'epoch')) bad('a document storage message needs epoch')
        msg.epoch = id(m, 'epoch')
      } else if (has(m, 'epoch')) bad('a plugin storage message takes no epoch')
      if (needsKey) {
        // A key of 256 bytes is at most 256 characters; the bytes are
        // counted only for a key that short.
        const key = text(m, 'key', LIMITS.storageKeyBytes, 1)
        if (!wellFormed(key)) bad('key is not well-formed text')
        if (utf8Bytes(key) > LIMITS.storageKeyBytes) bad(`key is longer than ${LIMITS.storageKeyBytes} bytes`)
        msg.key = key
      }
      if (needsValue) msg.value = text(m, 'value', big)
      return { msg, size: (msg.key?.length ?? 0) + (msg.value?.length ?? 0) }
    }
    case 'files': {
      const paths = strings(m, 'paths', LIMITS.filesPerRequest, LIMITS.pathChars, 1)
      let size = 0
      for (const p of paths) size += p.length
      return { msg: { t: 'files', id: id(m, 'id'), paths: [...paths] }, size }
    }
    case 'console': {
      const level = m.level
      if (typeof level !== 'string' || !CONSOLE_LEVELS.includes(level as ConsoleLevel)) bad('unknown console level')
      const args = strings(m, 'args', LIMITS.consoleArgs, LIMITS.consoleArgChars)
      const at = m.at
      if (typeof at !== 'number' || !Number.isFinite(at)) bad('at is not a time')
      let size = 0
      for (const a of args) size += a.length
      return { msg: { t: 'console', level: level as ConsoleLevel, args: [...args], at: at as number }, size }
    }
    case 'pong':
      return { msg: { t: 'pong', n: id(m, 'n') }, size: 0 }
    case 'error': {
      const message = text(m, 'message', LIMITS.errorMessageChars)
      const msg: RealmToHost = { t: 'error', message }
      if (has(m, 'stack')) msg.stack = text(m, 'stack', LIMITS.errorStackChars)
      return { msg, size: message.length + (msg.stack?.length ?? 0) }
    }
  }
  return bad('unknown message type')
}

/** Checks one message from a realm's Worker. */
export function checkRealmMessage(data: unknown): Checked<RealmToHost> {
  try {
    const { msg, size } = checkRealm(data)
    if (size > LIMITS.messageChars) return { ok: false, reason: 'message too large' }
    return { ok: true, msg, size }
  } catch (e) {
    return { ok: false, reason: e instanceof Violation ? e.message : 'unreadable message' }
  }
}

/** Checks one message from a realm's boot script. */
export function checkBootMessage(data: unknown): Checked<BootToHost> {
  try {
    if (!isRecord(data)) bad('not an object')
    const m = data as Record<string, unknown>
    const t = m.t
    if (typeof t !== 'string' || !Object.prototype.hasOwnProperty.call(BOOT_KEYS, t)) bad('unknown message type')
    onlyKeys(m, BOOT_KEYS[t as BootToHost['t']])
    if (t === 'booted') return { ok: true, msg: { t: 'booted' }, size: 0 }
    const message = text(m, 'message', LIMITS.errorMessageChars)
    return { ok: true, msg: { t: 'error', message }, size: message.length }
  } catch (e) {
    return { ok: false, reason: e instanceof Violation ? e.message : 'unreadable message' }
  }
}

/**
 * Counts what a realm sends over any rolling second: at most
 * `messagesPerSecond` messages and `charsPerSecond` of text. Keeps one
 * entry per message in the window, so it never holds more than the limit
 * plus one.
 */
export class RateMeter {
  private times: number[] = []
  private sizes: number[] = []
  private head = 0
  private chars = 0

  constructor(
    private readonly now: () => number,
    private readonly maxMessages: number = LIMITS.messagesPerSecond,
    private readonly maxChars: number = LIMITS.charsPerSecond,
  ) {}

  /** Records a message of `size`; returns a reason when it breaks a limit. */
  add(size: number): string | null {
    const t = this.now()
    while (this.head < this.times.length && t - this.times[this.head] >= 1000) {
      this.chars -= this.sizes[this.head]
      this.head++
    }
    if (this.head > 1024) {
      this.times = this.times.slice(this.head)
      this.sizes = this.sizes.slice(this.head)
      this.head = 0
    }
    this.times.push(t)
    this.sizes.push(size)
    this.chars += size
    if (this.times.length - this.head > this.maxMessages) return 'more than the allowed messages in one second'
    if (this.chars > this.maxChars) return 'more than the allowed text in one second'
    return null
  }
}

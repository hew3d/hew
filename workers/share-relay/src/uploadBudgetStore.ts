import { TTL_MS } from './dropStore.ts'
import type { DurableObjectStorage, UploadAdmission } from './types.ts'

export const MAX_TOTAL_BYTES = 256 * 1024 * 1024
export const MAX_DROPS = 256
export const MAX_CLIENT_STARTS = 60
export const MAX_DAILY_STARTS = 1000
export const CLIENT_WINDOW_MS = 10 * 60 * 1000
const DAY_MS = 24 * 60 * 60 * 1000
// Conservative: a reservation outlives a stored drop's ten-minute alarm,
// allowing time to read and persist its body. Abandoned uploads still expire.
const RESERVATION_MS = 2 * TTL_MS

/** One global SQLite-backed admission ledger. All admission checks and writes
 *  run synchronously before the first await, so competing requests cannot
 *  both spend the same bytes, drop slot, or rate allowance. */
export class UploadBudgetStore {
  private readonly storage: DurableObjectStorage

  constructor(storage: DurableObjectStorage) {
    this.storage = storage
    storage.sql.exec('CREATE TABLE IF NOT EXISTS reservation (token TEXT PRIMARY KEY, bytes INTEGER NOT NULL, expiresAt INTEGER NOT NULL)')
    storage.sql.exec('CREATE TABLE IF NOT EXISTS client (hash TEXT PRIMARY KEY, starts INTEGER NOT NULL, expiresAt INTEGER NOT NULL)')
    storage.sql.exec('CREATE TABLE IF NOT EXISTS daily (id INTEGER PRIMARY KEY CHECK (id = 0), day INTEGER NOT NULL, starts INTEGER NOT NULL)')
  }

  private sweep(now: number): void {
    this.storage.sql.exec('DELETE FROM reservation WHERE expiresAt <= ?', now)
    this.storage.sql.exec('DELETE FROM client WHERE expiresAt <= ?', now)
  }

  private async arm(): Promise<void> {
    const next = this.storage.sql.exec<{ expiresAt: number | null }>(
      'SELECT MIN(expiresAt) AS expiresAt FROM (SELECT expiresAt FROM reservation UNION ALL SELECT expiresAt FROM client)',
    ).toArray()[0].expiresAt
    if (next === await this.storage.getAlarm()) return
    if (next === null) await this.storage.deleteAlarm()
    else await this.storage.setAlarm(next)
  }

  async reserve(token: string, clientHash: string, bytes: number): Promise<UploadAdmission> {
    const now = Date.now()
    if (!Number.isSafeInteger(bytes) || bytes < 1 || bytes > MAX_TOTAL_BYTES) return { ok: false, reason: 'full' }
    this.sweep(now)
    const usage = this.storage.sql.exec<{ bytes: number; drops: number }>(
      'SELECT COALESCE(SUM(bytes), 0) AS bytes, COUNT(*) AS drops FROM reservation',
    ).toArray()[0]
    const client = this.storage.sql.exec<{ starts: number; expiresAt: number }>(
      'SELECT starts, expiresAt FROM client WHERE hash = ?', clientHash,
    ).toArray()[0]
    const daily = this.storage.sql.exec<{ day: number; starts: number }>('SELECT day, starts FROM daily').toArray()[0]
    const day = Math.floor(now / DAY_MS)
    if (client && client.starts >= MAX_CLIENT_STARTS) {
      return { ok: false, reason: 'rate-limited', retryAfter: Math.max(1, Math.ceil((client.expiresAt - now) / 1000)) }
    }
    if (daily?.day === day && daily.starts >= MAX_DAILY_STARTS) {
      return { ok: false, reason: 'rate-limited', retryAfter: Math.max(1, Math.ceil(((day + 1) * DAY_MS - now) / 1000)) }
    }
    if (usage.bytes + bytes > MAX_TOTAL_BYTES || usage.drops >= MAX_DROPS) return { ok: false, reason: 'full' }

    this.storage.sql.exec('INSERT INTO reservation (token, bytes, expiresAt) VALUES (?, ?, ?)', token, bytes, now + RESERVATION_MS)
    this.storage.sql.exec('INSERT INTO client (hash, starts, expiresAt) VALUES (?, 1, ?) ON CONFLICT(hash) DO UPDATE SET starts = starts + 1',
      clientHash, now + CLIENT_WINDOW_MS)
    this.storage.sql.exec('INSERT INTO daily (id, day, starts) VALUES (0, ?, 1) ON CONFLICT(id) DO UPDATE SET starts = CASE WHEN day = excluded.day THEN starts + 1 ELSE 1 END, day = excluded.day', day)
    await this.arm()
    return { ok: true }
  }

  /** Only shrink: a missing/expired reservation can never authorize storage. */
  async resize(token: string, bytes: number): Promise<boolean> {
    const row = this.storage.sql.exec<{ bytes: number; expiresAt: number }>(
      'SELECT bytes, expiresAt FROM reservation WHERE token = ?', token,
    ).toArray()[0]
    if (!row || row.expiresAt <= Date.now() || !Number.isSafeInteger(bytes) || bytes < 1 || bytes > row.bytes) return false
    this.storage.sql.exec('UPDATE reservation SET bytes = ? WHERE token = ?', bytes, token)
    return true
  }

  async release(token: string): Promise<void> {
    if (this.storage.sql.exec('SELECT token FROM reservation WHERE token = ?', token).toArray().length === 0) return
    this.storage.sql.exec('DELETE FROM reservation WHERE token = ?', token)
    await this.arm()
  }

  /** Persistence must finish while the admission is live. Extend the lease
   *  from completion so stored bytes never inherit an almost-expired budget. */
  async commit(token: string): Promise<boolean> {
    const now = Date.now()
    const row = this.storage.sql.exec<{ expiresAt: number }>(
      'SELECT expiresAt FROM reservation WHERE token = ?', token,
    ).toArray()[0]
    if (!row || row.expiresAt <= now) return false
    this.storage.sql.exec('UPDATE reservation SET expiresAt = ? WHERE token = ?', now + RESERVATION_MS, token)
    await this.arm()
    return true
  }

  async alarm(): Promise<void> {
    this.sweep(Date.now())
    await this.arm()
  }
}

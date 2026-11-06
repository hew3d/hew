/**
 * ReportIndex — the single Durable Object instance (`INDEX_DO_NAME`) that
 * backs the admin list, the storage ceiling, and the per-client/per-day
 * counters. Thin RPC wrapper over `IndexStore` (`indexStore.ts`), same split
 * as `ReportDrop`/`ReportStore` and share-relay's `ShareDrop`/`DropStore` —
 * every method here just forwards to `IndexStore`, which already returns
 * promises (see that file's note on why), EXCEPT `reserveReport`, which
 * also drives the idle-upload backstop (below) — that needs `env` access to
 * destroy the `ReportDrop`s behind whatever `IndexStore` finds idle, which
 * the framework-free store doesn't have.
 */

import { DurableObject } from 'cloudflare:workers'

import {
  IndexStore,
  type ReportRow,
  type ReportReservation,
  type ReserveResult,
  type CommitReportOutcome,
  type ThreadCachePatch,
} from './indexStore.ts'
import { IDLE_PRUNE_TIMEOUT_MS } from './constants.ts'
import type { BugIntakeEnv, DurableObjectStorage } from './types.ts'

export class ReportIndex extends DurableObject<BugIntakeEnv> {
  private readonly store_: IndexStore

  constructor(ctx: DurableObjectState, env: BugIntakeEnv) {
    super(ctx, env)
    this.store_ = new IndexStore(ctx.storage as unknown as DurableObjectStorage)
  }

  /** Runs the idle-upload backstop before every reservation attempt — the
   *  same "opportunistic, on the way in" cadence share-relay's rate-limit
   *  tables and this class's own counters prune on. Best-effort: a
   *  `ReportDrop.destroy()` failure for one idle id is logged and doesn't
   *  block the others or the reservation that triggered this. */
  async reserveReport(clientHash: string, now: number, entry: ReportReservation): Promise<ReserveResult> {
    await this.pruneIdleUploads(now)
    return this.store_.reserveReport(clientHash, now, entry)
  }

  private async pruneIdleUploads(now: number): Promise<void> {
    const idleIds = await this.store_.findAndDeleteIdleUncommittedIds(now, IDLE_PRUNE_TIMEOUT_MS)
    for (const id of idleIds) {
      try {
        await this.env.REPORT_DROP.get(this.env.REPORT_DROP.idFromName(id)).destroy()
      } catch (err) {
        console.error(`bug-intake: failed to destroy idle-pruned drop ${id}:`, err)
      }
    }
  }

  commitReport(id: string, following?: boolean): Promise<CommitReportOutcome> {
    return this.store_.commitReport(id, following)
  }

  touchActivity(id: string, now: number): Promise<void> {
    return this.store_.touchActivity(id, now)
  }

  listReports(): Promise<ReportRow[]> {
    return this.store_.listReports()
  }

  getReport(id: string): Promise<ReportRow | null> {
    return this.store_.getReport(id)
  }

  deleteReport(id: string): Promise<void> {
    return this.store_.deleteReport(id)
  }

  setTriaged(id: string, triaged: boolean): Promise<void> {
    return this.store_.setTriaged(id, triaged)
  }

  reserveEmail(now: number, purpose: 'report' | 'thread' = 'report'): Promise<boolean> {
    return this.store_.reserveEmail(now, purpose)
  }

  bundleExpired(id: string): Promise<boolean> {
    return this.store_.bundleExpired(id)
  }

  threadEnded(id: string): Promise<void> {
    return this.store_.threadEnded(id)
  }

  setThreadCache(id: string, patch: ThreadCachePatch): Promise<void> {
    return this.store_.setThreadCache(id, patch)
  }

  markAdminViewed(id: string, now: number): Promise<void> {
    return this.store_.markAdminViewed(id, now)
  }

  listChildren(parentId: string): Promise<ReportRow[]> {
    return this.store_.listChildren(parentId)
  }

  countThreadRequest(clientHash: string, now: number): Promise<{ ok: true } | { ok: false; retryAfterMs: number }> {
    return this.store_.countThreadRequest(clientHash, now)
  }
}

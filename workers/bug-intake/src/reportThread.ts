/**
 * ReportThread — the Durable Object holding one followed report's thread
 * (docs/design/follow-a-report.md §5.1), one instance per report id
 * (`env.REPORT_THREAD.idFromName(id)`). A thin RPC wrapper over
 * `ThreadStore` (`threadStore.ts`), the same split as `ReportDrop` over
 * `ReportStore`: it must `extend DurableObject` for its methods to be RPC at
 * all, and everything worth testing lives in the store.
 *
 * The one thing this wrapper adds is the store's `onDestroy` hook, pointed
 * at `ReportIndex.threadEnded`: when the thread ends (its alarm, or a
 * reporter's Forget) the index row either goes away (its bundle already
 * expired) or stops showing "Following: yes". The store calls it BEFORE
 * wiping, so a failed index call leaves the thread for the retried alarm.
 */

import { DurableObject } from 'cloudflare:workers'

import {
  ThreadStore,
  type ThreadView,
  type ThreadAdminView,
  type ThreadStatus,
  type PostReporterResult,
  type AppendResult,
  type AppendAttachmentResult,
} from './threadStore.ts'
import { INDEX_DO_NAME } from './constants.ts'
import type { BugIntakeEnv, DurableObjectStorage } from './types.ts'

export class ReportThread extends DurableObject<BugIntakeEnv> {
  private readonly store_: ThreadStore

  constructor(ctx: DurableObjectState, env: BugIntakeEnv) {
    super(ctx, env)
    this.store_ = new ThreadStore(ctx.storage as unknown as DurableObjectStorage, async (reportId) => {
      await env.REPORT_INDEX.get(env.REPORT_INDEX.idFromName(INDEX_DO_NAME)).threadEnded(reportId)
    })
  }

  /** RPC — COMMIT (with a minted follow secret), or an admin action on a
   *  report nobody follows (empty secret). Idempotent. */
  create(reportId: string, secretHash: string, now: number): Promise<void> {
    return this.store_.create(reportId, secretHash, now)
  }

  /** RPC — follow-up START. */
  verifySecret(secretHash: string): Promise<boolean> {
    return this.store_.verifySecret(secretHash)
  }

  /** RPC — `GET /report/<id>/thread`. */
  read(secretHash: string, afterSeq: number, now: number): Promise<ThreadView | 'forbidden'> {
    return this.store_.read(secretHash, afterSeq, now)
  }

  /** RPC — `POST /report/<id>/thread`. */
  postReporter(secretHash: string, text: string, now: number): Promise<PostReporterResult> {
    return this.store_.postReporter(secretHash, text, now)
  }

  /** RPC — `POST /report/<id>/thread/seen`. */
  markSeen(secretHash: string, upTo: number, now: number): Promise<'ok' | 'forbidden'> {
    return this.store_.markSeen(secretHash, upTo, now)
  }

  /** RPC — `DELETE /report/<id>/thread` (Forget). */
  revoke(secretHash: string): Promise<'ok' | 'forbidden'> {
    return this.store_.revoke(secretHash)
  }

  /** RPC — admin reply. */
  postMaintainer(text: string, now: number): Promise<AppendResult> {
    return this.store_.postMaintainer(text, now)
  }

  /** RPC — admin status form. */
  setStatus(status: ThreadStatus, version: string | null, note: string | null, now: number): Promise<AppendResult> {
    return this.store_.setStatus(status, version, note, now)
  }

  /** RPC — follow-up COMMIT. */
  appendAttachment(childId: string, text: string, now: number): Promise<AppendAttachmentResult> {
    return this.store_.appendAttachment(childId, text, now)
  }

  /** RPC — admin detail page. */
  adminView(now: number): Promise<ThreadAdminView | null> {
    return this.store_.adminView(now)
  }

  /** RPC — admin delete. */
  destroy(): Promise<void> {
    return this.store_.destroy()
  }

  /** DO alarm — see `ThreadStore.alarm`. */
  alarm(): Promise<void> {
    return this.store_.alarm(Date.now())
  }
}

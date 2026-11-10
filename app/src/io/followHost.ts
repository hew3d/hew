/**
 * followHost — what the UI needs from "Follow a bug report"
 * (docs/design/follow-a-report.md §6.1), nothing about secrets. Desktop
 * (`tauriFollowHost.ts`) keeps the secret in Rust; web (`webFollowHost.ts`)
 * keeps it in localStorage. `getFollowHost()` picks one.
 */

import { isTauri } from './fileHost'
import { canSend, type ReportProgress, type ReportSubmitResult } from './reportClient'
import type { CheckReason, CheckResult, FollowedReport } from './followPolicy'
import { createTauriFollowHost } from './tauriFollowHost'
import { createWebFollowHost } from './webFollowHost'

export * from './followPolicy'

export interface FollowHost {
  /** False when this build has nowhere to send (the Sent Reports UI hides). */
  available(): boolean
  list(): Promise<FollowedReport[]>
  /** `launch` honors the 6-hour rule; `manual` refreshes now. No `id` = every `following` report. */
  check(reason: CheckReason, id?: string): Promise<CheckResult>
  /** Posts a reporter message; resolves with the refreshed report. Rejects with `ReportError`. */
  post(id: string, text: string): Promise<FollowedReport>
  markSeen(id: string, upTo: number): Promise<FollowedReport | null>
  /** Follow-up upload of a gzip bundle (see `buildFollowUpUpload`). */
  attach(id: string, bundleGzip: Uint8Array, onProgress?: (p: ReportProgress) => void): Promise<ReportSubmitResult>
  forget(id: string): Promise<void>
  /** Called whenever stored reports change (any window / tab). Returns unsubscribe. */
  subscribe(cb: () => void): () => void
}

const unavailable: FollowHost = {
  available: () => false,
  list: async () => [],
  check: async () => ({ reports: [], summary: { checked: 0, withNews: 0, newCount: 0, failed: 0 } }),
  post: () => Promise.reject(new Error('follow unavailable')),
  markSeen: async () => null,
  attach: () => Promise.reject(new Error('follow unavailable')),
  forget: async () => {},
  subscribe: () => () => {},
}

let cached: FollowHost | null = null

/** The host for this runtime: Tauri, web (only where `canSend()`), or an inert one. */
export function getFollowHost(): FollowHost {
  if (cached === null) cached = isTauri ? createTauriFollowHost() : canSend() ? createWebFollowHost() : unavailable
  return cached
}

/** Drops the cached host — tests only. */
export function resetFollowHostForTest(): void {
  cached = null
}

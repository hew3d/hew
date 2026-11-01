/**
 * tauriFollowHost — the desktop `FollowHost`: every method is an `invoke` of
 * a `follow_*` command (docs/design/follow-a-report.md §6.2). The secret
 * lives in Rust; the webview never sees it. `follow-changed` keeps every
 * window in step (the `settings-changed` idiom in `settings/server.ts`).
 */

import type { FollowHost } from './followHost'
import type { CheckResult, FollowedReport } from './followPolicy'
import { invokeUpload, toReportError, type ReportProgress, type ReportSubmitResult } from './reportClient'

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  const { invoke } = await import('@tauri-apps/api/core')
  try {
    return await invoke<T>(command, args)
  } catch (err) {
    throw toReportError(err)
  }
}

export function createTauriFollowHost(): FollowHost {
  const listeners = new Set<() => void>()
  let listening: Promise<void> | null = null
  let unlisten: (() => void) | undefined

  const ensureListening = () => {
    if (listening !== null) return
    listening = import('@tauri-apps/api/event')
      .then(({ listen }) => listen('follow-changed', () => listeners.forEach((l) => l())))
      .then((fn) => {
        unlisten = fn
      })
      .catch(() => {
        listening = null
      })
  }

  return {
    available: () => true,
    list: () => call<FollowedReport[]>('follow_list'),
    check: (reason, id) => call<CheckResult>('follow_check', id === undefined ? { reason } : { id, reason }),
    post: (id, text) => call<FollowedReport>('follow_post', { id, text }),
    markSeen: (id, upTo) => call<FollowedReport | null>('follow_seen', { id, upTo }),
    async attach(id, bundleGzip, onProgress?: (p: ReportProgress) => void) {
      const r = await invokeUpload<{ id: string }>('follow_attach', bundleGzip, { 'hew-follow-id': id }, onProgress)
      const out: ReportSubmitResult = { id: r.id, following: false }
      return out
    },
    async forget(id) {
      await call<void>('follow_forget', { id })
    },
    subscribe(cb) {
      listeners.add(cb)
      ensureListening()
      return () => {
        listeners.delete(cb)
        if (listeners.size === 0 && unlisten !== undefined) {
          unlisten()
          unlisten = undefined
          listening = null
        }
      }
    },
  }
}

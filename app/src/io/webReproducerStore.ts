/**
 * WebReproducerStore — browser implementation of ReproducerStore.
 *
 * There is no addressable filesystem in a browser tab, so `write` instead
 * triggers a Blob-URL download named `name` (mirrors downloadDiagnosticLog()
 * in ../log/diagnosticLog.ts) and resolves to null — there is no path to
 * report back.
 *
 * Guarded throughout: must never throw, even from a failure handler context
 * (no `document`, popup/download blocked, etc).
 */

import type { ReproducerStore } from './reproducerStore'

export class WebReproducerStore implements ReproducerStore {
  async write(name: string, json: string): Promise<string | null> {
    try {
      const blob = new Blob([json], { type: 'application/json' })
      const url = URL.createObjectURL(blob)
      try {
        const a = document.createElement('a')
        a.href = url
        a.download = name
        a.click()
      } finally {
        URL.revokeObjectURL(url)
      }
    } catch {
      // Best-effort — never throw.
    }
    return null
  }
}

/**
 * WebLogFileStore — browser stand-in for LogFileStore.
 *
 * There is no real rolling file in a browser tab, so `append`/`rotateIfNeeded`
 * are no-ops and `path()` resolves to null. The web "file" is instead the
 * in-memory ring buffer plus an on-demand download (docs/DEVELOPMENT.md) —
 * see `downloadDiagnosticLog()` in ../log/diagnosticLog.ts.
 *
 * This module must never throw.
 */

import type { LogFileStore } from './logFileStore'

export class WebLogFileStore implements LogFileStore {
  async append(_ndjson: string): Promise<void> {
    // No-op: web has no rolling file; the ring buffer is the source of truth.
  }

  async rotateIfNeeded(): Promise<void> {
    // No-op: nothing to rotate.
  }

  async path(): Promise<string | null> {
    return null
  }
}

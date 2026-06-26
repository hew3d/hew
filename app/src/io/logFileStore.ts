/**
 * LogFileStore — abstracts the platform's rolling diagnostic-log file.
 *
 * Mirrors the RecoveryStore seam (./recoveryStore.ts): WebLogFileStore is
 * statically imported (always needed for the web build, where there is no
 * real file — `append` is a no-op and the "file" is instead an on-demand
 * browser download built from the in-memory ring, see diagnosticLog.ts), while
 * under Tauri the TauriLogFileStore is imported dynamically so its Tauri-only
 * imports never enter the web bundle.
 *
 * Used by the diagnostic-log sink (../log/diagnosticLog.ts) to flush newly
 * ingested NDJSON lines to a rolling file when Debug-mode file logging is
 * enabled (docs/DEVELOPMENT.md).
 */

export interface LogFileStore {
  /** Append `ndjson` (one or more newline-terminated JSON lines) to the rolling file. */
  append(ndjson: string): Promise<void>
  /** Rotate the file if it has grown past the size cap. No-op where not applicable (web). */
  rotateIfNeeded(): Promise<void>
  /** Absolute path of the rolling file, or null where there is no real file (web). */
  path(): Promise<string | null>
}

// WebLogFileStore is statically imported — it's always needed for the web build.
import { WebLogFileStore } from './webLogFileStore'

/**
 * True when running inside the Tauri desktop shell.
 * Re-derived here (rather than imported) to keep this module self-contained —
 * matches the same check in recoveryStore.ts / fileHost.ts.
 */
const isTauri: boolean =
  typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window

/**
 * Return the appropriate LogFileStore for the current platform.
 *
 * Under Tauri, TauriLogFileStore is imported dynamically (lazy) so its
 * Tauri-only imports never appear in the web bundle. makeLogFileStore() is
 * called once by the caller; we return a synchronous shim that defers to the
 * dynamically-loaded store once resolved (mirrors makeRecoveryStore()).
 */
export function makeLogFileStore(): LogFileStore {
  if (isTauri) {
    const storePromise = import('./tauriLogFileStore').then(
      (m) => new m.TauriLogFileStore(),
    )
    return {
      append: (ndjson) => storePromise.then((s) => s.append(ndjson)),
      rotateIfNeeded: () => storePromise.then((s) => s.rotateIfNeeded()),
      path: () => storePromise.then((s) => s.path()),
    }
  }
  return new WebLogFileStore()
}

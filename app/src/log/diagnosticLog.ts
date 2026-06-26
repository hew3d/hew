/**
 * diagnosticLog — the structured diagnostic-log sink (docs/DEVELOPMENT.md,
 * milestone  — the TS half of the seam; the kernel half, is the
 * wasm-api log surface documented in docs/DEVELOPMENT.md and docs/DIAGNOSTICS.md).
 *
 * Merges two streams into one unified, time-ordered ring buffer:
 *   - kernel: one JSON LogRecord string per `tracing` event, delivered via
 *     the wasm `set_log_drain` callback (installKernelDrain()).
 *   - ui: the app's own existing LogStore (info/warn/error) bridged in
 *     (bridgeLogStore()), plus future raw instrumentation via logUi().
 *
 * Both streams share the kernel's `corr` (gesture correlation) id so the
 * merged timeline filters to one user gesture: beginGesture()/endGesture()
 * open/close the wasm-side scope AND remember the returned id as the sink's
 * "current corr", stamped onto every `ui` record logged while the gesture is
 * open.
 *
 * The ring buffer (bounded, drops oldest) and the NDJSON serialisation
 * (toNDJSON, for the web download) work regardless of file logging; the
 * Debug-mode rolling-file flush (setFileLogging) is an additional, opt-in
 * sink on top, via ../io/logFileStore.
 */

import * as LogStore from './LogStore'
import { makeLogFileStore, type LogFileStore } from '../io/logFileStore'

/** A single unified diagnostic record — kernel or UI in origin. */
export interface DiagRecord {
  /** Sink-assigned monotonic arrival order — the unified-timeline merge key. */
  id: number
  source: 'kernel' | 'ui'
  /** Kernel wasm seq (kernel records) or null (ui records). */
  seq: number | null
  /** Gesture correlation id, shared across both streams, or null. */
  corr: number | null
  level: string
  target: string
  fields: Record<string, unknown>
  /** Date.now() at ingestion. */
  t: number
}

/** Shape of one kernel LogRecord JSON string (docs/DIAGNOSTICS.md). */
interface KernelLogRecord {
  seq: number
  corr: number | null
  level: string
  target: string
  fields: Record<string, unknown>
}

const MAX_RECORDS = 50_000

let nextId = 1
let records: DiagRecord[] = []

/** The gesture correlation id shared with new `ui` records, or null outside a gesture. */
let currentCorr: number | null = null

/** Last LogStore entry id already ingested, so bridgeLogStore() never double-ingests. */
let lastBridgedLogId = 0
let logStoreUnsubscribe: (() => void) | null = null

let fileLoggingEnabled = false
let fileStore: LogFileStore | null = null
/** Records already flushed to the file store, as an exclusive lower bound on `id`. */
let lastFlushedId = 0

function push(record: DiagRecord): void {
  records.push(record)
  if (records.length > MAX_RECORDS) {
    records = records.slice(records.length - MAX_RECORDS)
  }
  maybeFlush()
}

/** Ingest one kernel LogRecord JSON string (the `set_log_drain` callback body). */
export function ingestKernel(json: string): void {
  let parsed: KernelLogRecord
  try {
    parsed = JSON.parse(json) as KernelLogRecord
  } catch {
    // Malformed record from the drain — drop rather than throw (drain
    // callbacks that throw are ignored by the kernel per docs/DEVELOPMENT.md,
    // but guard anyway since this is also called directly from tests).
    return
  }
  push({
    id: nextId++,
    source: 'kernel',
    seq: parsed.seq,
    corr: parsed.corr,
    level: parsed.level,
    target: parsed.target,
    fields: parsed.fields ?? {},
    t: Date.now(),
  })
}

/**
 * Public API for raw app-event instrumentation (pointer/camera/tool state
 * etc.). Not yet wired into Viewport/tools — exposed for future use so
 * callers can start instrumenting without sink changes.
 */
export function logUi(target: string, level: string, fields: Record<string, unknown>): void {
  push({
    id: nextId++,
    source: 'ui',
    seq: null,
    corr: currentCorr,
    level,
    target,
    fields,
    t: Date.now(),
  })
}

/** Map a LogStore level to the diagnostic record's level field. */
function levelFromLogStore(level: LogStore.LogLevel): string {
  return level.toUpperCase()
}

/**
 * Subscribe to the existing app LogStore (info/warn/error) and ingest NEW
 * entries as `ui` records. LogStore.subscribe() delivers the FULL array on
 * every notify, so we track the last-seen LogStore entry id and only ingest
 * entries past it — both on each notification and on the initial delivery.
 */
export function bridgeLogStore(): void {
  if (logStoreUnsubscribe !== null) return // idempotent
  logStoreUnsubscribe = LogStore.subscribe((entries) => {
    for (const entry of entries) {
      if (entry.id <= lastBridgedLogId) continue
      lastBridgedLogId = entry.id
      push({
        id: nextId++,
        source: 'ui',
        seq: null,
        corr: currentCorr,
        level: levelFromLogStore(entry.level),
        target: entry.source,
        fields: { message: entry.message },
        t: entry.timestamp.getTime(),
      })
    }
  })
}

/** Undo bridgeLogStore() — for tests; not part of the app-startup contract. */
export function unbridgeLogStore(): void {
  logStoreUnsubscribe?.()
  logStoreUnsubscribe = null
  lastBridgedLogId = 0
}

/**
 * Install the kernel drain: calls `init_logging('info')` then routes each
 * record through ingestKernel(). Accepts the wasm module's `init_logging`/
 * `set_log_drain` as parameters so tests can inject fakes without touching
 * wasm/loader.ts.
 */
export function installKernelDrain(
  initLogging: (level: string) => void,
  setLogDrain: (cb: (json: string) => void) => void,
): void {
  initLogging('info')
  setLogDrain(ingestKernel)
}

/**
 * Begin a gesture: calls the wasm `beginGesture()` (returns a bigint id),
 * remembers it as the sink's current corr (shared with new `ui` records),
 * and returns it.
 */
export function beginGesture(beginGestureFn: () => bigint): bigint {
  const id = beginGestureFn()
  currentCorr = Number(id)
  return id
}

/** End the current gesture: calls the wasm `endGesture()` and clears the shared corr. */
export function endGesture(endGestureFn: () => void): void {
  endGestureFn()
  currentCorr = null
}

/** Current gesture correlation id shared with new `ui` records, or null. */
export function getCurrentCorr(): number | null {
  return currentCorr
}

/** Snapshot of all records currently in the ring buffer, oldest first. */
export function getRecords(): readonly DiagRecord[] {
  return records as readonly DiagRecord[]
}

/** Clear the ring buffer (does not affect file-logging flush bookkeeping). */
export function clear(): void {
  records = []
  lastFlushedId = 0
}

/** Serialise the ring buffer as newline-delimited JSON, one record per line. */
export function toNDJSON(recs: readonly DiagRecord[] = records): string {
  return recs.map((r) => JSON.stringify(r)).join('\n')
}

/**
 * Enable/disable Debug-mode rolling-file logging (default OFF). When turned
 * on, lazily creates the platform LogFileStore and flushes any records
 * already in the ring; subsequent pushes flush incrementally.
 */
export function setFileLogging(enabled: boolean): void {
  fileLoggingEnabled = enabled
  if (enabled) {
    fileStore ??= makeLogFileStore()
    maybeFlush()
  }
}

export function isFileLoggingEnabled(): boolean {
  return fileLoggingEnabled
}

/** Flush any records not yet written to the file store, if file logging is on. */
function maybeFlush(): void {
  if (!fileLoggingEnabled || fileStore === null) return
  const pending = records.filter((r) => r.id > lastFlushedId)
  if (pending.length === 0) return
  const ndjson = toNDJSON(pending) + '\n'
  lastFlushedId = pending[pending.length - 1].id
  const store = fileStore
  store.rotateIfNeeded().then(
    () => store.append(ndjson),
    () => store.append(ndjson),
  )
}

/**
 * Build a Blob URL download of the current ring buffer as NDJSON and trigger
 * a browser download (the web "file" — see WebLogFileStore). Safe to call on
 * any platform; on Tauri it still works as a manual export, on top of the
 * rolling file.
 */
export function downloadDiagnosticLog(filename = 'hew-diagnostic.log'): void {
  const ndjson = toNDJSON()
  const blob = new Blob([ndjson], { type: 'application/x-ndjson' })
  const url = URL.createObjectURL(blob)
  try {
    const a = document.createElement('a')
    a.href = url
    a.download = filename
    a.click()
  } finally {
    URL.revokeObjectURL(url)
  }
}

/**
 * LogStore — append-only, observable log for the persistent error/info panel.
 *
 * Design:
 *   - Plain module with a singleton store (no framework).
 *   - Entries are append-only; older entries beyond MAX_ENTRIES are dropped
 *     from the front so memory stays bounded.
 *   - Subscribers receive the full array on each change.
 *   - subscribe() returns an unsubscribe function.
 */

export type LogLevel = 'info' | 'warn' | 'error'

export interface LogEntry {
  id: number
  timestamp: Date
  level: LogLevel
  source: string
  message: string
}

export type Subscriber = (entries: readonly LogEntry[]) => void

const MAX_ENTRIES = 500

let nextId = 1
let entries: LogEntry[] = []
const subscribers = new Set<Subscriber>()

function notify(): void {
  const snapshot = entries as readonly LogEntry[]
  for (const sub of subscribers) {
    sub(snapshot)
  }
}

/** Append a new log entry. */
export function append(level: LogLevel, source: string, message: string): void {
  const entry: LogEntry = {
    id: nextId++,
    timestamp: new Date(),
    level,
    source,
    message,
  }
  entries = [...entries, entry]
  if (entries.length > MAX_ENTRIES) {
    entries = entries.slice(entries.length - MAX_ENTRIES)
  }
  notify()
}

/** Subscribe to changes. Returns an unsubscribe function. */
export function subscribe(fn: Subscriber): () => void {
  subscribers.add(fn)
  // Immediately deliver current state
  fn(entries as readonly LogEntry[])
  return () => {
    subscribers.delete(fn)
  }
}

/** Get a snapshot of the current entries. */
export function getEntries(): readonly LogEntry[] {
  return entries as readonly LogEntry[]
}

/** Clear all entries. */
export function clear(): void {
  entries = []
  notify()
}

// Convenience helpers
export const log = {
  info: (source: string, message: string) => append('info', source, message),
  warn: (source: string, message: string) => append('warn', source, message),
  error: (source: string, message: string) => append('error', source, message),
}

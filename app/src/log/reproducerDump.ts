/**
 * reproducerDump — the auto-reproducer dump (docs/DEVELOPMENT.md, "the
 * highest-value feature"). On a failure, bundle {recorded command stream so
 * far + serialized .hew + diagnostic-log tail} to disk so "it broke" becomes
 * "here is a model + an input log that reproduces it".
 *
 * Depends on:
 *   -  recording (docs/DIAGNOSTICS.md): `scene.start_recording()` /
 *     `scene.take_recording()` — the typed Scene command stream.
 *   -  diagnosticLog: `getRecords()` / `toNDJSON()` — the unified
 *     kernel+UI log ring buffer.
 *
 * The registered Scene is the only wasm/Scene access point in this module —
 * everything else (manifest fields, base64, store write) is plain TS, so the
 * module is unit-testable with a fake scene (see reproducerDump.test.ts).
 */

import * as diagnosticLog from './diagnosticLog'
import { makeReproducerStore, type ReproducerStore } from '../io/reproducerStore'

/** The minimal Scene surface this module needs — see crates/wasm-api/src/lib.rs. */
export interface RecordableScene {
  start_recording(): void
  take_recording(): string
  save(): Uint8Array
  state_hash(): bigint
}

/** The assembled reproducer bundle written to disk / downloaded. */
export interface ReproducerBundle {
  manifest: {
    reason: string
    ts: number
    appVersion: string
    stateHash: string
    userAgent: string
  }
  /** The Recording JSON string (docs/DIAGNOSTICS.md), or null if unavailable. */
  recording: string | null
  /** The diagnostic-log tail as NDJSON. */
  log: string
  /** The .hew document bytes (scene.save()), base64-encoded, or null if unavailable. */
  hew: string | null
}

let registeredScene: RecordableScene | null = null
let store: ReproducerStore | null = null

/** Guards against dump-within-a-dump (e.g. a failure handler throwing during dump). */
let dumping = false

/** Rate limit: at most one dump per this many ms, so an error storm writes one file. */
const RATE_LIMIT_MS = 5_000
let lastDumpAt = 0

/** Cap on how many diagnostic-log records to include in the bundle. */
const LOG_TAIL_RECORDS = 2_000

let failureHandlersInstalled = false

/**
 * Register the current Scene as the dump source, and start recording its
 * committed command stream from now on. Call this for every newly-created
 * Scene (loader.ts's newScene()). Auto-recording is cheap — ops are
 * user-gesture frequency, not per-frame.
 */
export function registerScene(scene: RecordableScene): void {
  registeredScene = scene
  try {
    scene.start_recording()
  } catch {
    // Best-effort — a scene that can't start recording still gets registered
    // so save()/state_hash() remain available to a later dump.
  }
}

/** Test-only: clear all module state (registered scene, store, rate limit, guards). */
export function resetForTest(): void {
  registeredScene = null
  store = null
  dumping = false
  lastDumpAt = 0
  failureHandlersInstalled = false
}

/** Test-only: inject a fake ReproducerStore instead of the platform-derived one. */
export function setStoreForTest(fake: ReproducerStore): void {
  store = fake
}

function getStore(): ReproducerStore {
  store ??= makeReproducerStore()
  return store
}

function base64FromBytes(bytes: Uint8Array): string {
  // btoa requires a binary string; build it in chunks to avoid blowing the
  // call stack on large arrays (String.fromCharCode(...hugeArray) can throw
  // "too many arguments" well before Uint8Array sizes hew documents reach).
  const CHUNK = 0x8000
  let binary = ''
  for (let i = 0; i < bytes.length; i += CHUNK) {
    const chunk = bytes.subarray(i, i + CHUNK)
    binary += String.fromCharCode(...chunk)
  }
  return btoa(binary)
}

/**
 * Gather a reproducer bundle from the registered scene + diagnostic log and
 * write it via the reproducer store as `reproducer-<ISO-timestamp>.json`.
 *
 * Best-effort and re-entrancy-guarded: never throws (so it's safe to call
 * from an `error`/`unhandledrejection` handler), and a missing or throwing
 * scene still produces a bundle with `recording`/`hew` set to null rather
 * than aborting. Rate-limited so an error storm doesn't write hundreds of
 * files. Returns the path (Tauri) or null (web download / rate-limited /
 * re-entrant / failed).
 */
export async function dumpReproducer(reason: string): Promise<string | null> {
  if (dumping) return null
  const now = Date.now()
  if (now - lastDumpAt < RATE_LIMIT_MS) return null

  dumping = true
  lastDumpAt = now
  try {
    const scene = registeredScene

    let recording: string | null = null
    let hew: string | null = null
    let stateHash = '0'

    if (scene !== null) {
      try {
        recording = scene.take_recording()
      } catch {
        recording = null
      }
      try {
        hew = base64FromBytes(scene.save())
      } catch {
        hew = null
      }
      try {
        stateHash = scene.state_hash().toString()
      } catch {
        stateHash = '0'
      }
    }

    let log = ''
    try {
      const records = diagnosticLog.getRecords()
      const tail = records.slice(Math.max(0, records.length - LOG_TAIL_RECORDS))
      log = diagnosticLog.toNDJSON(tail)
    } catch {
      log = ''
    }

    const bundle: ReproducerBundle = {
      manifest: {
        reason,
        ts: now,
        appVersion: typeof __HEW_VERSION__ !== 'undefined' ? __HEW_VERSION__ : '0.0.0',
        stateHash,
        userAgent: typeof navigator !== 'undefined' ? navigator.userAgent : '',
      },
      recording,
      log,
      hew,
    }

    const name = `reproducer-${new Date(now).toISOString().replace(/[:.]/g, '-')}.json`
    try {
      return await getStore().write(name, JSON.stringify(bundle))
    } catch {
      return null
    }
  } catch {
    // Never throw out of the failure handler.
    return null
  } finally {
    dumping = false
  }
}

/**
 * Install `window.addEventListener('error'/'unhandledrejection', ...)`
 * handlers that auto-dump a reproducer bundle on uncaught errors/rejections
 * (incl. uncaught wasm panics/traps surfaced as JS errors) — without needing
 * an App/ErrorBoundary edit. Idempotent (installs at most once per session).
 */
export function installFailureHandlers(): void {
  if (failureHandlersInstalled) return
  if (typeof window === 'undefined') return
  failureHandlersInstalled = true

  window.addEventListener('error', (event) => {
    const message = event.error instanceof Error ? event.error.message : event.message
    void dumpReproducer(`uncaught-error: ${message}`)
  })

  window.addEventListener('unhandledrejection', (event) => {
    const reason = event.reason instanceof Error ? event.reason.message : String(event.reason)
    void dumpReproducer(`unhandledrejection: ${reason}`)
  })
}

// Optional build-time version string (not currently defined by the Vite
// config — falls back to '0.0.0' via the `typeof` guard above so this module
// never depends on a new build-config wire-up).
declare const __HEW_VERSION__: string | undefined

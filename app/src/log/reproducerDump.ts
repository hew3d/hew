/**
 * reproducerDump — the auto-reproducer dump (docs/dev/DEVELOPMENT.md, "the
 * highest-value feature"). On a failure, bundle {recorded command stream so
 * far + serialized .hew + diagnostic-log tail} to disk so "it broke" becomes
 * "here is a model + an input log that reproduces it".
 *
 * The crash screen's own bundle (ErrorBoundary's "Report this crash") goes
 * through ReportBugDialog/reportBundle.ts instead of this module — it shares
 * the panic capture (./panicCapture.ts) as its `recording` source, but builds
 * the report-bug wire format rather than this module's reproducer format.
 *
 * Depends on:
 *   -  recording (docs/dev/DIAGNOSTICS.md): `scene.start_recording()` /
 *     `scene.take_recording()` — the typed Scene command stream.
 *   -  diagnosticLog: `getRecords()` / `toNDJSON()` — the unified
 *     kernel+UI log ring buffer.
 *
 * The registered Scene is the only wasm/Scene access point in this module —
 * everything else (manifest fields, base64, store write) is plain TS, so the
 * module is unit-testable with a fake scene (see reproducerDump.test.ts).
 */

import { enabledFeatureIds, previewBuild } from '../preview'
import * as diagnosticLog from './diagnosticLog'
import { makeReproducerStore, type ReproducerStore } from '../io/reproducerStore'
import { getPanicCapture } from './panicCapture'

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
    /** A preview build's stamp (the Preview wall, src/preview): `true` and the
     *  features switched on. Absent in production. */
    preview?: true
    previewFeatures?: string[]
  }
  /** The Recording JSON string (docs/dev/DIAGNOSTICS.md), or null if unavailable. */
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

/** An assembled bundle, ready to hand to a `ReproducerStore.write()` call. */
interface AssembledBundle {
  name: string
  json: string
}

/**
 * Build a {@link ReproducerBundle} (manifest + recording/log/hew) and its
 * `<prefix>-<ISO-timestamp>.json` filename for `dumpReproducer`.
 */
function assembleBundle(
  prefix: string,
  reason: string,
  now: number,
  recording: string | null,
  hew: string | null,
  stateHash: string,
): AssembledBundle {
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
      ...(previewBuild ? { preview: true as const, previewFeatures: enabledFeatureIds() } : {}),
    },
    recording,
    log,
    hew,
  }

  const name = `${prefix}-${new Date(now).toISOString().replace(/[:.]/g, '-')}.json`
  return { name, json: JSON.stringify(bundle) }
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
 * re-entrant / failed / stood down).
 *
 * Stands down once a kernel panic has been captured: every later uncaught
 * error is the poisoned instance's "recursive use" symptom, and the crash
 * screen the panic raised owns saving a reproducer, so dumping here as well
 * would only add an unrequested second download on web.
 */
export async function dumpReproducer(reason: string): Promise<string | null> {
  if (dumping) return null
  if (getPanicCapture() !== null) return null
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

    const { name, json } = assembleBundle('reproducer', reason, now, recording, hew, stateHash)
    try {
      return await getStore().write(name, json)
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
 * True for an uncaught error the browser refused to describe: no Error object
 * and the muted cross-origin placeholder message. The spec calls this a
 * "muted error"; every engine reports it as some spelling of "Script error",
 * with `filename`/`lineno`/`colno` zeroed and `error` null.
 *
 * These carry no attribution whatsoever, so a bundle built from one names no
 * failing code and proves nothing about the document — and the error is just
 * as likely to belong to a browser extension, an injected analytics tag, or
 * the browser itself as to Hew. iOS Safari raises them spontaneously on pages
 * that contain no script at all, which made the resulting download (the web
 * store's only way to deliver a bundle) hijack the user's Add to Home Screen
 * flow and leave the PWA uninstallable. Record the sighting in the diagnostic
 * log and dump nothing.
 */
function isMutedError(event: ErrorEvent): boolean {
  if (event.error != null) return false
  const message = (event.message ?? '').trim()
  // Chromium/Firefox: "Script error."; WebKit historically omits the period.
  return message === '' || message === 'Script error' || message === 'Script error.'
}

/**
 * Install `window.addEventListener('error'/'unhandledrejection', ...)`
 * handlers that auto-dump a reproducer bundle on uncaught errors/rejections
 * (incl. uncaught wasm panics/traps surfaced as JS errors) — without needing
 * an App/ErrorBoundary edit. Idempotent (installs at most once per session).
 *
 * Muted cross-origin errors are logged but never dumped — see `isMutedError`.
 */
export function installFailureHandlers(): void {
  if (failureHandlersInstalled) return
  if (typeof window === 'undefined') return
  failureHandlersInstalled = true

  window.addEventListener('error', (event) => {
    if (isMutedError(event)) {
      try {
        // oxlint-disable-next-line lingui/no-unlocalized-strings -- log level identifier
        diagnosticLog.logUi('reproducer', 'WARN', {
          // oxlint-disable-next-line lingui/no-unlocalized-strings -- diagnostic log line for the maintainer
          message: 'ignored muted cross-origin error (no attribution, nothing to reproduce)',
        })
      } catch {
        // Best-effort — never throw out of the failure handler.
      }
      return
    }
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

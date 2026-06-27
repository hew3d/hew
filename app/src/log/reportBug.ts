/**
 * reportBug — the user-triggered "Report Bug" bundle (QUALITY
 *, "a force multiplier for the manual exercising phase"). Unlike
 * reproducerDump.ts (auto-triggered on an uncaught error/rejection), this is
 * fired explicitly from a menu action — there is no failure to react to, so
 * the bundle additionally carries OS/GPU/app-version and the input recorder's
 * buffer (peeked, not taken, so an ongoing recording is undisturbed).
 *
 * Structurally this mirrors reproducerDump.ts: a minimal Scene surface
 * (`ReportableScene`), best-effort try/catch around every scene call so a
 * throwing scene still produces a bundle, chunked base64 for `save()`, and a
 * store write via the same `../io/reproducerStore` seam (with the same
 * test-injection hooks).
 */

import * as diagnosticLog from './diagnosticLog'
import * as inputRecorder from '../recording/inputRecorder'
import { makeReproducerStore, type ReproducerStore } from '../io/reproducerStore'

/** The minimal Scene surface this module needs — see crates/wasm-api/src/lib.rs. */
export interface ReportableScene {
  save(): Uint8Array
  state_hash(): bigint
}

/**
 * Outcome of a Report Bug attempt, so the caller can give the user honest
 * feedback (Tauri saves a file; web triggers a download — both used to look
 * like "nothing happened"). `null` alone was ambiguous (web-success and
 * failure both returned it), so this distinguishes them.
 */
export interface BugReportResult {
  /** The bundle was assembled and handed to the store without error. */
  ok: boolean
  /** The saved file path (Tauri); `null` on web (downloaded — no path) or on failure. */
  path: string | null
}

/** The assembled bug-report bundle written to disk / downloaded. */
export interface BugReportBundle {
  manifest: {
    reason: string
    ts: number
    appVersion: string
    stateHash: string
    userAgent: string
    os: string
    gpu: string
  }
  /** The .hew document bytes (scene.save()), base64-encoded, or null if unavailable. */
  hew: string | null
  /** The diagnostic-log tail as NDJSON. */
  log: string
  /** The buffered low-level input events (peeked, not cleared). */
  input: inputRecorder.InputEvent[]
}

/** Cap on how many diagnostic-log records to include in the bundle. */
const LOG_TAIL_RECORDS = 2_000

let store: ReproducerStore | null = null

/** Test-only: inject a fake ReproducerStore instead of the platform-derived one. */
export function setStoreForTest(fake: ReproducerStore): void {
  store = fake
}

/** Test-only: clear injected test state. */
export function resetForTest(): void {
  store = null
}

function getStore(): ReproducerStore {
  store ??= makeReproducerStore()
  return store
}

function base64FromBytes(bytes: Uint8Array): string {
  // btoa requires a binary string; build it in chunks to avoid blowing the
  // call stack on large arrays (mirrors reproducerDump.ts's base64FromBytes).
  const CHUNK = 0x8000
  let binary = ''
  for (let i = 0; i < bytes.length; i += CHUNK) {
    const chunk = bytes.subarray(i, i + CHUNK)
    binary += String.fromCharCode(...chunk)
  }
  return btoa(binary)
}

function detectOs(): string {
  if (typeof navigator === 'undefined') return ''
  const uad = (navigator as Navigator & { userAgentData?: { platform?: string } }).userAgentData
  return uad?.platform ?? navigator.platform ?? ''
}

/**
 * Best-effort GPU renderer string via a throwaway WebGL context's
 * WEBGL_debug_renderer_info extension. Returns '' if unavailable (e.g.
 * headless test environments, or a browser that blocks the extension).
 */
function detectGpu(): string {
  try {
    const canvas = document.createElement('canvas')
    const gl = canvas.getContext('webgl')
    if (gl === null) return ''
    const ext = gl.getExtension('WEBGL_debug_renderer_info')
    if (ext === null) return ''
    const renderer = gl.getParameter(ext.UNMASKED_RENDERER_WEBGL)
    return typeof renderer === 'string' ? renderer : ''
  } catch {
    return ''
  }
}

/**
 * Assemble a bug-report bundle from `scene` + the diagnostic log + the
 * buffered input recording, and write it via the reproducer store as
 * `bug-report-<ISO-timestamp>.json`. Best-effort: never throws (mirrors
 * reproducerDump.dumpReproducer); a throwing scene still produces a bundle
 * with `hew`/`stateHash` set to null/'0' rather than aborting. Returns a
 * [`BugReportResult`] so the caller can report success (saved path / web
 * download) vs. failure to the user.
 */
export async function generateBugReport(
  scene: ReportableScene,
  reason = 'user-report',
): Promise<BugReportResult> {
  try {
    const now = Date.now()

    let hew: string | null = null
    let stateHash = '0'
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

    let log = ''
    try {
      const records = diagnosticLog.getRecords()
      const tail = records.slice(Math.max(0, records.length - LOG_TAIL_RECORDS))
      log = diagnosticLog.toNDJSON(tail)
    } catch {
      log = ''
    }

    let input: inputRecorder.InputEvent[] = []
    try {
      input = inputRecorder.peek()
    } catch {
      input = []
    }

    let userAgent = ''
    try {
      userAgent = typeof navigator !== 'undefined' ? navigator.userAgent : ''
    } catch {
      userAgent = ''
    }

    let os = ''
    try {
      os = detectOs()
    } catch {
      os = ''
    }

    let gpu = ''
    try {
      gpu = detectGpu()
    } catch {
      gpu = ''
    }

    const bundle: BugReportBundle = {
      manifest: {
        reason,
        ts: now,
        appVersion: typeof __HEW_VERSION__ !== 'undefined' ? __HEW_VERSION__ : '0.0.0',
        stateHash,
        userAgent,
        os,
        gpu,
      },
      hew,
      log,
      input,
    }

    const name = `bug-report-${new Date(now).toISOString().replace(/[:.]/g, '-')}.json`
    try {
      const path = await getStore().write(name, JSON.stringify(bundle))
      return { ok: true, path }
    } catch {
      return { ok: false, path: null }
    }
  } catch {
    // Never throw — this is fired from a menu action, not a failure handler,
    // but the contract still holds: a bug report must never itself crash the app.
    return { ok: false, path: null }
  }
}

// Optional build-time version string (mirrors reproducerDump.ts — not
// currently defined by the Vite config, falls back to '0.0.0').
declare const __HEW_VERSION__: string | undefined

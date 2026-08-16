/**
 * useServerSettingForm — the state machine behind the Advanced ▸ Server
 * pane, shared by the macOS/Linux pane (`AdvancedPane.tsx`) and the Windows
 * mirror (`FluentSettingsPage.tsx`) so both render the same behavior:
 *
 *   - the persisted setting comes from `settings/server.ts` (Rust-held);
 *   - the pane edits a DRAFT: the mode radio persists immediately when it
 *     is *cloud* (nothing to validate), while *self-hosted* only persists
 *     once the origin field commits (blur / Enter) and passes Rust's
 *     validation — an empty or half-typed address stays a draft with the
 *     error shown inline, and the persisted mode stays whatever it was;
 *   - *Test connection* commits the draft first (the Rust probe reads the
 *     PERSISTED setting), then calls `relay_identity` and reports the
 *     answer or the specific failure.
 *
 * Every relay request still happens in Rust; this hook only moves the
 * setting and words the results.
 */

import { useCallback, useEffect, useRef, useState } from 'react'
import { RelayError, relayIdentity, toRelayError, type RelayIdentity } from '../io/relayClient'
import {
  CLOUD_ORIGIN,
  DEFAULT_SERVER_SETTING,
  getServerSetting,
  serverSettingAvailable,
  setServerSetting,
  subscribe,
  type ServerMode,
  type ServerSetting,
} from './server'

export type TestState =
  | { kind: 'idle' }
  | { kind: 'testing' }
  | { kind: 'ok'; identity: RelayIdentity }
  | { kind: 'fail'; message: string }

export interface ServerSettingForm {
  /** False in the browser build — the pane renders its read-only note. */
  available: boolean
  /** The draft being edited (mirrors the persisted value until touched). */
  draft: ServerSetting
  /** Inline validation/persist error for the origin/key fields. */
  error: string | null
  test: TestState
  setMode: (mode: ServerMode) => void
  setOriginDraft: (origin: string) => void
  setUploadKeyDraft: (key: string) => void
  /** Persist the current draft (origin/key blur, Enter). Resolves to
   *  whether it was accepted. */
  commit: () => Promise<boolean>
  testConnection: () => Promise<void>
}

/** Words a failed `relay_identity` for the pane. */
export function describeTestFailure(err: unknown, origin: string): string {
  const e = err instanceof RelayError ? err : toRelayError(err)
  const host = (() => {
    try {
      return new URL(origin).host
    } catch {
      return origin
    }
  })()
  switch (e.kind) {
    case 'unreachable':
      return `Could not reach ${host} — check the address, your connection, and that the server is up.`
    case 'tls':
      return `Reachable, but the server's certificate isn't trusted by this computer — install its certificate authority in the system keychain.`
    case 'notARelay':
      return `Reachable, but ${host} isn't serving a Hew relay at /relay/.`
    case 'unauthorized':
      return 'The server rejected the upload key.'
    case 'status':
      return `Unexpected answer from ${host} (status ${e.status ?? '?'}).`
    case 'invalidOrigin':
    case 'io':
    case 'full':
    case 'tooLarge':
      return e.message
  }
}

/** Words a successful probe for the pane. */
export function describeIdentity(identity: RelayIdentity): string {
  const cap = `${Math.floor(identity.maxBytes / (1024 * 1024))} MB max`
  const ttl = `${Math.round(identity.ttlMs / 60_000)} min TTL`
  const auth = identity.auth === 'bearer' ? 'upload key required' : 'open uploads'
  return `${identity.service} v${identity.contract} · ${cap} · ${ttl} · ${auth}`
}

export function useServerSettingForm(): ServerSettingForm {
  const available = serverSettingAvailable()
  const [draft, setDraft] = useState<ServerSetting>(DEFAULT_SERVER_SETTING)
  const [error, setError] = useState<string | null>(null)
  const [test, setTest] = useState<TestState>({ kind: 'idle' })
  // The latest draft, readable from event handlers without a stale
  // closure — `setMode` builds its next value from it OUTSIDE the state
  // updater (React double-invokes updaters under StrictMode, so an
  // `invoke` from inside one would fire twice).
  const draftRef = useRef(draft)
  draftRef.current = draft
  // Several persists can be in flight at once (a blur-commit racing a radio
  // click). Two guards: they are QUEUED so Rust receives them strictly in
  // the order the user acted (the last one issued is the one that ends up
  // on disk), and only the LATEST one is allowed to write its answer back
  // into the draft, so an older write can never revert what the user did
  // afterwards.
  const persistGen = useRef(0)
  const persistQueue = useRef<Promise<unknown>>(Promise.resolve())

  // Load once, then follow external changes (the other window).
  useEffect(() => {
    if (!available) return
    let cancelled = false
    const refresh = (): void => {
      getServerSetting()
        .then((s) => {
          if (!cancelled) setDraft(s)
        })
        .catch(() => {
          /* leave the draft as is */
        })
    }
    refresh()
    const unsubscribe = subscribe(refresh)
    return () => {
      cancelled = true
      unsubscribe()
    }
  }, [available])

  const persist = useCallback(
    async (next: ServerSetting): Promise<boolean> => {
      const gen = ++persistGen.current
      const run = persistQueue.current.then(() => setServerSetting(next))
      persistQueue.current = run.catch(() => undefined)
      try {
        const saved = await run
        if (gen === persistGen.current) {
          setDraft(saved)
          setError(null)
        }
        return true
      } catch (err) {
        if (gen === persistGen.current) setError(toRelayError(err).message)
        return false
      }
    },
    [],
  )

  const setMode = useCallback(
    (mode: ServerMode) => {
      setTest({ kind: 'idle' })
      const d = draftRef.current
      // The stored default origin IS the cloud origin; a fresh self-hosted
      // draft should start empty, not pre-filled with app.hew3d.com.
      const origin = mode === 'self-hosted' && d.origin === CLOUD_ORIGIN ? '' : d.origin
      const next = { ...d, mode, origin }
      setDraft(next)
      // Cloud needs no validation — persist right away. Self-hosted waits
      // for the origin to commit (it may be empty right now).
      if (mode === 'cloud') void persist(next)
      else setError(null)
    },
    [persist],
  )

  const setOriginDraft = useCallback((origin: string) => {
    setTest({ kind: 'idle' })
    setDraft((d) => ({ ...d, origin }))
  }, [])

  const setUploadKeyDraft = useCallback((uploadKey: string) => {
    setTest({ kind: 'idle' })
    setDraft((d) => ({ ...d, uploadKey }))
  }, [])

  const commit = useCallback(async (): Promise<boolean> => {
    if (!available) return false
    return persist(draft)
  }, [available, draft, persist])

  const testConnection = useCallback(async () => {
    if (!available) return
    setTest({ kind: 'testing' })
    const ok = await persist(draft)
    if (!ok) {
      setTest({ kind: 'idle' })
      return
    }
    try {
      const identity = await relayIdentity()
      if (identity.keyAccepted === false) {
        setTest({ kind: 'fail', message: 'Reachable, but the server rejected the upload key.' })
        return
      }
      if (identity.auth === 'bearer' && draft.mode === 'self-hosted' && draft.uploadKey.trim() === '') {
        setTest({ kind: 'fail', message: 'Reachable, but this server requires an upload key — enter the one its admin gave you.' })
        return
      }
      setTest({ kind: 'ok', identity })
    } catch (err) {
      setTest({ kind: 'fail', message: describeTestFailure(err, draft.mode === 'cloud' ? DEFAULT_SERVER_SETTING.origin : draft.origin) })
    }
  }, [available, draft, persist])

  return { available, draft, error, test, setMode, setOriginDraft, setUploadKeyDraft, commit, testConnection }
}

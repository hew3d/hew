/* oxlint-disable lingui/no-unlocalized-strings -- identifiers and state names; no user-visible text */
// What the Library dialog knows about sources, kept current
// (docs/design/shared-library-sources.md 7.4, 7.5). A hook over the sources
// module and nothing else: it reads the no-network parts of the module
// (`listSources`, `newCounts`, `localStates`), re-reads when the sources file
// or the Library changes, and runs the automatic check once each time the
// dialog opens. It never makes a request itself except through `autoCheck`,
// which holds the rules of 7.4 (setting on, last check over 6 hours old, the
// catalog only when a source is added, and no request at all when Off).

import { useCallback, useEffect, useRef, useState } from 'react'
import type { SourcesBinding } from './sourcesDeps'
import type { LocalState, NewCounts, SourceStatus } from './sources'

export interface SourcesSnapshot {
  /** The first read has finished. */
  loaded: boolean
  statuses: SourceStatus[]
  /** Per FOLLOWED source (canonical reference): new and changed counts.
   * Absent for a removed source (no bar count for it). */
  counts: Record<string, NewCounts>
  /** Every Library item that carries a note, against its source's cache. */
  locals: LocalState[]
  /** Changes whenever a source's last-check time does ("Later" hides the
   * bar until it does). */
  stamp: string
}

const EMPTY: SourcesSnapshot = { loaded: false, statuses: [], counts: {}, locals: [], stamp: '' }

export interface UseSources {
  snapshot: SourcesSnapshot
  /** The automatic check is running. */
  checking: boolean
  /** Re-read everything that needs no network. */
  refresh(): Promise<void>
}

/** `binding` undefined means the dialog was given no sources (older call
 * sites and tests): nothing is read and nothing is requested. `libraryVersion`
 * changes whenever the dialog re-lists the Library. */
export function useSources(binding: SourcesBinding | undefined, open: boolean, libraryVersion: number): UseSources {
  const [snapshot, setSnapshot] = useState<SourcesSnapshot>(EMPTY)
  const [checking, setChecking] = useState(false)
  const seq = useRef(0)
  const bindingRef = useRef(binding)
  bindingRef.current = binding
  // Reading the Library's notes walks every item, so overlapping reads are
  // folded into one more read after the one in flight.
  const inflight = useRef<Promise<void> | null>(null)
  const again = useRef(false)

  const read = useCallback(async (): Promise<void> => {
    const b = bindingRef.current
    if (b === undefined) return
    const mine = ++seq.current
    try {
      const [statuses, locals] = await Promise.all([b.sources.listSources(), b.sources.localStates()])
      const counts: Record<string, NewCounts> = {}
      // Only a followed source's new and changed counts reach the bar; the
      // others are counted from the items' own states.
      for (const s of statuses) {
        if (s.removed !== null || !s.followed) continue
        counts[s.entry.source] = await b.sources.newCounts(s.entry.source)
      }
      if (mine !== seq.current) return
      setSnapshot({
        loaded: true,
        statuses,
        counts,
        locals,
        stamp: statuses.map((s) => `${s.entry.source}@${s.entry.lastCheck?.at ?? ''}`).join('|'),
      })
    } catch {
      // A store that cannot be read leaves the last snapshot in place; the
      // dialog's own listing shows the storage problem.
      if (mine === seq.current) setSnapshot((s) => ({ ...s, loaded: true }))
    }
  }, [])

  const refresh = useCallback((): Promise<void> => {
    if (inflight.current !== null) {
      again.current = true
      return inflight.current
    }
    const run = (async (): Promise<void> => {
      do {
        again.current = false
        await read()
      } while (again.current)
    })().finally(() => {
      inflight.current = null
    })
    inflight.current = run
    return run
  }, [read])

  // Each time the dialog opens: read, run the automatic check, read again.
  useEffect(() => {
    if (!open || binding === undefined) return
    let cancelled = false
    void (async () => {
      await refresh()
      if (cancelled || !binding.available()) return
      setChecking(true)
      try {
        await binding.sources.autoCheck()
      } catch {
        /* a check that cannot run leaves the cached listings as they are */
      }
      if (cancelled) return
      setChecking(false)
      await refresh()
    })()
    return () => {
      cancelled = true
      setChecking(false)
    }
  }, [open, binding, refresh])

  // The sources file or the Library changed (this window or another).
  useEffect(() => {
    if (!open || binding === undefined) return
    return binding.subscribe(() => {
      void refresh()
    })
  }, [open, binding, refresh])

  // The dialog re-listed the Library.
  useEffect(() => {
    if (open && binding !== undefined) void refresh()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [libraryVersion])

  return { snapshot, checking, refresh }
}

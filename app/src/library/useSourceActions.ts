/* oxlint-disable lingui/no-unlocalized-strings -- identifiers and state names; the user-visible sentences come from sourceMessages and the components */
// Everything the Library dialog does TO a source, as one hook
// (docs/design/shared-library-sources.md 5.2, 7.4, 7.5): add an item, insert
// one, update one, unlink, check, add all, follow, update all, remove a
// source. It calls the sources module and turns each outcome into the words
// of 6.6 or a question the user must answer (a `needs_confirm`). It holds no
// markup; the dialog draws `confirm`, `run`, `review` and `message`.

import { useCallback, useEffect, useRef, useState } from 'react'
import { isTauri } from '../io/fileHost'
import { failureMessage, removeFailedText, sourcesOffText, type SourceMessage } from './sourceMessages'
import { sourceHost } from './sourceModel'
import type { SourcesBinding } from './sourcesDeps'
import type { SourcesSnapshot } from './useSources'
import type {
  AddItemResult,
  CatalogSourceEntry,
  Failure,
  ItemState,
  RunProgress,
  RunSummary,
} from './sources'

/** The item an action is about. `file` is set when the item is already in
 * the Library; `fromCatalog` when the source is only listed in the catalog
 * and must be added first. */
export interface ItemTarget {
  source: string
  id: string
  name: string
  file?: string | null
  state?: ItemState
  fromCatalog?: CatalogSourceEntry
}

export type Confirm =
  | { kind: 'replace_item'; name: string; yes(): void; no(): void }
  | { kind: 'replace_changes'; name: string; replace(): void; keep(): void; cancel(): void }
  | { kind: 'remove_source'; name: string; confirm(): void; cancel(): void }

export interface RunState {
  kind: 'addAll' | 'follow'
  source: string
  name: string
  running: boolean
  progress: RunProgress | null
  summary: RunSummary | null
  /** The run could not start, or stopped early on a problem. */
  message: SourceMessage | null
}

export interface ReviewItem {
  key: string
  source: string
  sourceName: string
  id: string
  /** The name the item will take (the source's own). */
  name: string
  /** The name it has in the Library now, for a changed item. */
  localName: string | null
  kind: 'newer' | 'new'
  edited: boolean
}

export type ReviewResult =
  | { kind: 'updated' | 'added' | 'replaced' | 'kept' | 'needs_review'; /** The update landed but the old file could not be removed. */ note?: SourceMessage }
  | { kind: 'failed'; message: SourceMessage }

export interface ReviewState {
  items: ReviewItem[]
  results: Record<string, ReviewResult>
  busy: boolean
}

export interface ActionDeps {
  binding: SourcesBinding | undefined
  snapshot: SourcesSnapshot
  /** Re-read the no-network state. */
  refresh(): Promise<void>
  /** Ask the dialog to re-list the Library. */
  bumpLibrary(): void
  /** Arm placement for a file that is now in the Library. */
  insertFile(file: string): Promise<void>
  /** A catalog source was added: the dialog can show it. */
  onSourceAdded?(source: string): void
}

export type Outcome =
  | { ok: true; file?: string; /** The write landed but something needs saying (the old copy stayed). */ notice?: SourceMessage }
  | { ok: false; pending?: boolean; message?: SourceMessage }

/** What a landed write says besides "done": the old copy could not be removed. */
function noticeOf(res: { removeFailed?: boolean }): SourceMessage | undefined {
  return res.removeFailed === true ? { text: removeFailedText() } : undefined
}

export const reviewKey = (source: string, id: string): string => `${source}\n${id}`

function isOff(x: unknown): x is { off: true } {
  return typeof x === 'object' && x !== null && (x as { off?: unknown }).off === true
}
function isFail(x: unknown): x is Failure {
  return typeof x === 'object' && x !== null && (x as { ok?: unknown }).ok === false
}

export interface UseSourceActions {
  message: SourceMessage | null
  setMessage(m: SourceMessage | null): void
  confirm: Confirm | null
  run: RunState | null
  review: ReviewState | null
  busy: boolean
  checkSource(source: string): Promise<void>
  addItem(target: ItemTarget): Promise<Outcome>
  /** Add a source the catalog lists (no item yet). */
  addListedSource(entry: CatalogSourceEntry): Promise<Outcome>
  insertItem(target: ItemTarget): Promise<Outcome>
  /** `onLate` hears the outcome of the second call made after the user answers a question. */
  updateItem(target: ItemTarget, onLate?: (out: Outcome) => void): Promise<Outcome>
  unlink(source: string, id: string): Promise<Outcome>
  addAll(source: string, name: string): Promise<void>
  follow(source: string, name: string): Promise<void>
  unfollow(source: string): Promise<void>
  cancelRun(): void
  retryRun(): Promise<void>
  dismissRun(): void
  requestRemoveSource(source: string, name: string, then: () => void): void
  /** Update all from the bar: run, then show what each row did. */
  updateAllFromBar(): Promise<void>
  openReview(): Promise<void>
  closeReview(): void
  reviewUpdateSelected(keys: string[]): Promise<void>
  reviewDecide(key: string, decision: 'replace' | 'keep'): Promise<void>
}

export function useSourceActions(deps: ActionDeps): UseSourceActions {
  const depsRef = useRef(deps)
  depsRef.current = deps
  const [message, setMessage] = useState<SourceMessage | null>(null)
  const [confirm, setConfirm] = useState<Confirm | null>(null)
  const [run, setRun] = useState<RunState | null>(null)
  const [review, setReview] = useState<ReviewState | null>(null)
  const [busyCount, setBusyCount] = useState(0)
  const controller = useRef<AbortController | null>(null)
  const relistTimer = useRef<ReturnType<typeof setTimeout> | null>(null)
  const mounted = useRef(true)
  const updatingAll = useRef(false)
  useEffect(() => {
    mounted.current = true
    return () => {
      mounted.current = false
      if (relistTimer.current !== null) clearTimeout(relistTimer.current)
      controller.current?.abort()
    }
  }, [])

  const messageOf = useCallback((f: Failure, source: string): SourceMessage => {
    return failureMessage(f, { host: sourceHost(source), web: !isTauri })
  }, [])

  async function guarded<T>(fn: () => Promise<T>): Promise<T> {
    setBusyCount((n) => n + 1)
    try {
      return await fn()
    } finally {
      if (mounted.current) setBusyCount((n) => n - 1)
    }
  }

  const afterWrite = useCallback(async (): Promise<void> => {
    depsRef.current.bumpLibrary()
    await depsRef.current.refresh()
  }, [])

  // ------------------------------------------------------------ one item

  /** The source an item action runs against, adding a catalog source first. */
  async function ensureSource(target: ItemTarget): Promise<{ source: string } | { out: Outcome }> {
    const { binding } = depsRef.current
    if (binding === undefined) return { out: { ok: false } }
    if (target.fromCatalog === undefined) return { source: target.source }
    const r = await binding.sources.addSource(target.fromCatalog.source)
    if (isOff(r)) return { out: fail({ text: sourcesOffText() }) }
    if (isFail(r)) return { out: fail(messageOf(r, target.fromCatalog.source)) }
    await depsRef.current.refresh()
    depsRef.current.onSourceAdded?.(r.entry.source)
    return { source: r.entry.source }
  }

  function fail(m: SourceMessage): Outcome {
    setMessage(m)
    return { ok: false, message: m }
  }

  async function addResult(res: AddItemResult, source: string, target: ItemTarget, thenInsert: boolean): Promise<Outcome> {
    const { binding } = depsRef.current
    if (isOff(res)) return fail({ text: sourcesOffText() })
    if (isFail(res)) {
      if (res.code === 'needs_confirm' && res.confirm === 'replace_item' && binding !== undefined) {
        setConfirm({
          kind: 'replace_item',
          name: target.name,
          yes: () => {
            setConfirm(null)
            void guarded(async () => {
              const again = await binding.sources.addItem(source, target.id, { replace: true })
              await addResult(again, source, target, thenInsert)
            })
          },
          no: () => setConfirm(null),
        })
        return { ok: false, pending: true }
      }
      return fail(messageOf(res, source))
    }
    const notice = noticeOf(res)
    setMessage(notice ?? null)
    await afterWrite()
    if (thenInsert) await depsRef.current.insertFile(res.file)
    return { ok: true, file: res.file, ...(notice !== undefined ? { notice } : {}) }
  }

  async function addItem(target: ItemTarget, thenInsert = false): Promise<Outcome> {
    const { binding } = depsRef.current
    if (binding === undefined) return { ok: false }
    return guarded(async () => {
      const src = await ensureSource(target)
      if ('out' in src) return src.out
      const res = await binding.sources.addItem(src.source, target.id)
      return addResult(res, src.source, target, thenInsert)
    })
  }

  async function addListedSource(entry: CatalogSourceEntry): Promise<Outcome> {
    if (depsRef.current.binding === undefined) return { ok: false }
    return guarded(async () => {
      const src = await ensureSource({ source: entry.source, id: '', name: entry.name, fromCatalog: entry })
      if ('out' in src) return src.out
      setMessage(null)
      return { ok: true }
    })
  }

  async function insertItem(target: ItemTarget): Promise<Outcome> {
    // Already in the Library: place it. Otherwise add it first, then place it.
    if (target.file != null && target.state !== undefined && target.state !== 'not_in_library') {
      await depsRef.current.insertFile(target.file)
      return { ok: true, file: target.file }
    }
    return addItem(target, true)
  }

  async function updateItem(target: ItemTarget, onLate?: (out: Outcome) => void): Promise<Outcome> {
    const { binding } = depsRef.current
    if (binding === undefined) return { ok: false }
    const { source, id } = target
    return guarded(async () => {
      const res = await binding.sources.updateItem(source, id)
      if (isOff(res)) return fail({ text: sourcesOffText() })
      if (isFail(res)) {
        if (res.code === 'needs_confirm' && res.confirm === 'replace_changes') {
          setConfirm({
            kind: 'replace_changes',
            name: target.name,
            replace: () => {
              setConfirm(null)
              void guarded(async () => {
                const again = await binding.sources.updateItem(source, id, { decision: 'replace' })
                let out: Outcome
                if (isOff(again)) out = fail({ text: sourcesOffText() })
                else if (isFail(again)) out = fail(messageOf(again, source))
                else {
                  const notice = noticeOf(again)
                  setMessage(notice ?? null)
                  await afterWrite()
                  out = { ok: true, file: again.file, ...(notice !== undefined ? { notice } : {}) }
                }
                onLate?.(out)
              })
            },
            keep: () => {
              setConfirm(null)
              void guarded(async () => {
                const again = await binding.sources.updateItem(source, id, { decision: 'keep' })
                let out: Outcome
                if (isOff(again)) out = fail({ text: sourcesOffText() })
                else if (isFail(again)) out = fail(messageOf(again, source))
                else {
                  setMessage(null)
                  out = { ok: true, file: again.file }
                }
                await depsRef.current.refresh()
                onLate?.(out)
              })
            },
            cancel: () => setConfirm(null),
          })
          return { ok: false, pending: true }
        }
        return fail(messageOf(res, source))
      }
      const notice = noticeOf(res)
      setMessage(notice ?? null)
      await afterWrite()
      return { ok: true, file: res.file, ...(notice !== undefined ? { notice } : {}) }
    })
  }

  async function unlink(source: string, id: string): Promise<Outcome> {
    const { binding } = depsRef.current
    if (binding === undefined) return { ok: false }
    return guarded(async () => {
      const res = await binding.sources.unlink(source, id)
      if (isFail(res)) return fail(messageOf(res, source))
      setMessage(null)
      await afterWrite()
      return { ok: true }
    })
  }

  // -------------------------------------------------------------- checks

  async function checkSource(source: string): Promise<void> {
    const { binding } = depsRef.current
    if (binding === undefined) return
    await guarded(async () => {
      const res = await binding.sources.check(source)
      if (isOff(res)) setMessage({ text: sourcesOffText() })
      else if ('ok' in res && res.ok === false) setMessage(messageOf(res, source))
      else setMessage(null)
      await depsRef.current.refresh()
    })
  }

  // ---------------------------------------------------------------- runs

  function scheduleRelist(): void {
    if (relistTimer.current !== null) return
    relistTimer.current = setTimeout(() => {
      relistTimer.current = null
      if (mounted.current) depsRef.current.bumpLibrary()
    }, 1000)
  }

  async function startRun(kind: 'addAll' | 'follow', source: string, name: string, ids?: string[]): Promise<void> {
    const { binding } = depsRef.current
    if (binding === undefined) return
    const ctl = new AbortController()
    controller.current = ctl
    setRun({ kind, source, name, running: true, progress: null, summary: null, message: null })
    setMessage(null)
    await guarded(async () => {
      const opts = {
        signal: ctl.signal,
        onProgress: (p: RunProgress) => {
          if (!mounted.current) return
          setRun((r) => (r === null ? r : { ...r, progress: p }))
          scheduleRelist()
        },
        ...(ids !== undefined ? { ids } : {}),
      }
      const res = kind === 'follow' && ids === undefined ? await binding.sources.follow(source, opts) : await binding.sources.addAll(source, opts)
      if (!mounted.current) return
      if (relistTimer.current !== null) {
        clearTimeout(relistTimer.current)
        relistTimer.current = null
      }
      if (isOff(res)) setRun((r) => (r === null ? r : { ...r, running: false, message: { text: sourcesOffText() } }))
      else if (isFail(res)) setRun((r) => (r === null ? r : { ...r, running: false, message: messageOf(res, source) }))
      else {
        const stopped = res.summary.stopped
        const stopMessage: SourceMessage | null =
          stopped === 'rate_limited'
            ? failureMessage({ ok: false, code: 'rate_limited' }, { host: sourceHost(source), web: !isTauri })
            : stopped === 'network'
              ? failureMessage({ ok: false, code: 'network' }, { host: sourceHost(source), web: !isTauri })
              : null
        setRun((r) => (r === null ? r : { ...r, running: false, summary: res.summary, message: stopMessage }))
      }
      await afterWrite()
    })
  }

  async function addAll(source: string, name: string): Promise<void> {
    await startRun('addAll', source, name)
  }
  async function follow(source: string, name: string): Promise<void> {
    await startRun('follow', source, name)
  }
  async function unfollow(source: string): Promise<void> {
    const { binding } = depsRef.current
    if (binding === undefined) return
    await guarded(async () => {
      const res = await binding.sources.unfollow(source)
      if (isFail(res)) setMessage(messageOf(res, source))
      await depsRef.current.refresh()
    })
  }
  function cancelRun(): void {
    controller.current?.abort()
  }
  async function retryRun(): Promise<void> {
    const r = run
    if (r === null || r.summary === null || r.summary.retry.length === 0) return
    await startRun('addAll', r.source, r.name, r.summary.retry)
  }
  function dismissRun(): void {
    setRun(null)
  }

  function requestRemoveSource(source: string, name: string, then: () => void): void {
    const { binding } = depsRef.current
    if (binding === undefined) return
    setConfirm({
      kind: 'remove_source',
      name,
      confirm: () => {
        setConfirm(null)
        void guarded(async () => {
          const res = await binding.sources.removeSource(source)
          if (isFail(res)) setMessage(messageOf(res, source))
          else {
            setRun(null)
            then()
          }
          await afterWrite()
        })
      },
      cancel: () => setConfirm(null),
    })
  }

  // -------------------------------------------------------------- review

  async function buildReviewItems(): Promise<ReviewItem[]> {
    const { binding, snapshot } = depsRef.current
    if (binding === undefined) return []
    const items: ReviewItem[] = []
    for (const s of snapshot.statuses) {
      if (s.removed !== null) continue
      const states = await binding.sources.itemStates(s.entry.source)
      if (isFail(states)) continue
      for (const row of states.rows) {
        const base = { key: reviewKey(s.entry.source, row.id), source: s.entry.source, sourceName: s.entry.name, id: row.id, name: row.name }
        if (row.state === 'newer' && !row.kept) {
          const local = snapshot.locals.find((l) => l.id === row.id && l.note.source === s.entry.source)
          items.push({ ...base, localName: local?.name ?? null, kind: 'newer', edited: row.edited })
        } else if (s.followed && row.state === 'not_in_library' && !row.skipped && row.entry !== null) {
          items.push({ ...base, localName: null, kind: 'new', edited: false })
        }
      }
    }
    return items
  }

  async function openReview(): Promise<void> {
    const items = await buildReviewItems()
    if (mounted.current) setReview({ items, results: {}, busy: false })
  }

  function closeReview(): void {
    setReview(null)
  }

  function withNote(res: { removeFailed?: boolean }): { note?: SourceMessage } {
    const n = noticeOf(res)
    return n !== undefined ? { note: n } : {}
  }

  function putResults(patch: Record<string, ReviewResult>): void {
    setReview((r) => (r === null ? r : { ...r, results: { ...r.results, ...patch } }))
  }

  async function updateAllFromBar(): Promise<void> {
    const { binding } = depsRef.current
    if (binding === undefined || updatingAll.current) return
    // Set before the first await: the bar's busy flag only rises later.
    updatingAll.current = true
    try {
      await runUpdateAll(binding)
    } finally {
      updatingAll.current = false
    }
  }

  async function runUpdateAll(binding: SourcesBinding): Promise<void> {
    const items = await buildReviewItems()
    setReview({ items, results: {}, busy: true })
    const results: Record<string, ReviewResult> = {}
    const sourcesInvolved = Array.from(new Set(items.map((i) => i.source)))
    await guarded(async () => {
      for (const source of sourcesInvolved) {
        const mine = items.filter((i) => i.source === source)
        if (mine.some((i) => i.kind === 'newer')) {
          const res = await binding.sources.updateAll(source)
          if (isOff(res)) {
            for (const i of mine) results[i.key] = { kind: 'failed', message: { text: sourcesOffText() } }
          } else if (isFail(res)) {
            const m = messageOf(res, source)
            for (const i of mine.filter((x) => x.kind === 'newer')) results[i.key] = { kind: 'failed', message: m }
          } else {
            for (const row of res.rows) {
              const key = reviewKey(source, row.id)
              if (isOff(row.result)) results[key] = { kind: 'failed', message: { text: sourcesOffText() } }
              else if (isFail(row.result)) results[key] = { kind: 'failed', message: messageOf(row.result, source) }
              else results[key] = { kind: 'updated', ...withNote(row.result) }
            }
            for (const id of res.needsReview) results[reviewKey(source, id)] = { kind: 'needs_review' }
          }
        }
        const fresh = mine.filter((i) => i.kind === 'new')
        if (fresh.length > 0) {
          const res = await binding.sources.addAll(source, { ids: fresh.map((i) => i.id) })
          if (isOff(res)) for (const i of fresh) results[i.key] = { kind: 'failed', message: { text: sourcesOffText() } }
          else if (isFail(res)) {
            const m = messageOf(res, source)
            for (const i of fresh) results[i.key] = { kind: 'failed', message: m }
          } else {
            const failed = new Map(res.summary.failures.map((f) => [f.id, f.code]))
            const notTried = new Set(res.summary.notTried)
            for (const i of fresh) {
              const code = failed.get(i.id)
              if (code !== undefined) results[i.key] = { kind: 'failed', message: messageOf({ ok: false, code }, source) }
              else if (!notTried.has(i.id)) results[i.key] = { kind: 'added' }
            }
          }
        }
      }
      await afterWrite()
    })
    if (mounted.current) setReview({ items, results, busy: false })
  }

  async function reviewUpdateSelected(keys: string[]): Promise<void> {
    const { binding } = depsRef.current
    if (binding === undefined || review === null) return
    const chosen = review.items.filter((i) => keys.includes(i.key))
    setReview((r) => (r === null ? r : { ...r, busy: true }))
    await guarded(async () => {
      for (const item of chosen) {
        const res: AddItemResult =
          item.kind === 'new' ? await binding.sources.addItem(item.source, item.id) : await binding.sources.updateItem(item.source, item.id)
        if (isOff(res)) putResults({ [item.key]: { kind: 'failed', message: { text: sourcesOffText() } } })
        else if (isFail(res)) {
          // A question is answered with the item's own two buttons, never here.
          if (res.code === 'needs_confirm') putResults({ [item.key]: { kind: 'needs_review' } })
          else putResults({ [item.key]: { kind: 'failed', message: messageOf(res, item.source) } })
        } else putResults({ [item.key]: { kind: item.kind === 'new' ? 'added' : 'updated', ...withNote(res) } })
      }
      await afterWrite()
    })
    setReview((r) => (r === null ? r : { ...r, busy: false }))
  }

  async function reviewDecide(key: string, decision: 'replace' | 'keep'): Promise<void> {
    const { binding } = depsRef.current
    const item = review?.items.find((i) => i.key === key)
    if (binding === undefined || item === undefined) return
    setReview((r) => (r === null ? r : { ...r, busy: true }))
    await guarded(async () => {
      const res = await binding.sources.updateItem(item.source, item.id, { decision })
      if (isOff(res)) putResults({ [key]: { kind: 'failed', message: { text: sourcesOffText() } } })
      else if (isFail(res)) putResults({ [key]: { kind: 'failed', message: messageOf(res, item.source) } })
      else putResults({ [key]: { kind: decision === 'keep' ? 'kept' : 'replaced', ...withNote(res) } })
      await afterWrite()
    })
    setReview((r) => (r === null ? r : { ...r, busy: false }))
  }

  return {
    message,
    setMessage,
    confirm,
    run,
    review,
    busy: busyCount > 0,
    checkSource,
    addItem: (t) => addItem(t),
    addListedSource,
    insertItem,
    updateItem,
    unlink,
    addAll,
    follow,
    unfollow,
    cancelRun,
    retryRun,
    dismissRun,
    requestRemoveSource,
    updateAllFromBar,
    openReview,
    closeReview,
    reviewUpdateSelected,
    reviewDecide,
  }
}

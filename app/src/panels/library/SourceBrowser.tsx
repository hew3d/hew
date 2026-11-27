/**
 * SourceBrowser — the center and detail columns for one source
 * (docs/design/shared-library-sources.md 5.1, 7.5): the source's header (name,
 * author, license, homepage, last check, Refresh, Add all, Follow, Remove
 * source), its items in the Library's own tile and row components with their
 * state in words, and the detail pane with the license before either action.
 * It also serves a catalog PREVIEW: the same view from the catalog's snapshot,
 * for a source not added yet, with "Add this source" in place of the source's
 * own buttons. It only draws and reports intent; every action runs in
 * `useSourceActions`.
 */

import { useEffect, useMemo, useReducer, useRef, useState } from 'react'
import { Trans, useLingui } from '@lingui/react/macro'
import { isTauri } from '../../io/fileHost'
import { sortListItems, type ListSortColumn, type ListSortDirection } from '../../library/libraryModel'
import { failureMessage, sourceErrorMessage, type SourceMessage } from '../../library/sourceMessages'
import {
  checkedLine,
  filterRows,
  rowLibraryItem,
  rowStateLabel,
  sourceHost,
  type SourceViewTarget,
} from '../../library/sourceModel'
import type { SourcesBinding } from '../../library/sourcesDeps'
import type { IndexSummary, ItemRow, SourceErrorCode } from '../../library/sources'
import type { ItemTarget, UseSourceActions } from '../../library/useSourceActions'
import type { SourcesSnapshot } from '../../library/useSources'
import type { LibraryItem } from '../../library/types'
import { ExternalLink } from './ExternalLink'
import { GridViewGlyph, ListViewGlyph, SearchGlyph } from './icons'
import { LibraryListHead, type LibraryListColWidths } from './LibraryListHead'
import { LibraryListRow } from './LibraryListRow'
import { LibraryTile, tileElementId } from './LibraryTile'
import { SourceItemDetail } from './SourceItemDetail'

type BrowseTarget = Extract<SourceViewTarget, { kind: 'source' | 'preview' }>

export interface SourceBrowserProps {
  binding: SourcesBinding
  target: BrowseTarget
  snapshot: SourcesSnapshot
  actions: UseSourceActions
  /** Changes when the dialog re-lists the Library (item states move). */
  libraryVersion: number
  /** The rendered thumbnail of a Library file, when it has one. */
  localThumbFor(file: string): string | null
  view: 'grid' | 'list'
  onViewChange(view: 'grid' | 'list'): void
  listWidths: LibraryListColWidths
  onResizeListCol(column: keyof LibraryListColWidths, width: number): void
  detailWidth: number
  native: boolean
  nowMs: number
  onResizeDetail(e: React.PointerEvent): void
  /** The dialog should show this (now added) source. */
  onOpenSource(source: string): void
  /** The source was removed from the list. */
  onRemoved(): void
}

interface Loaded {
  rows: ItemRow[]
  lastGoodAt: string | null
  summary: IndexSummary | null
}

const MAX_THUMBS = 100

export function SourceBrowser(props: SourceBrowserProps) {
  const { binding, target, snapshot, actions, libraryVersion, view, detailWidth, native, nowMs } = props
  const { t } = useLingui()
  const sourceRef = target.kind === 'source' ? target.source : target.entry.source
  const isPreview = target.kind === 'preview'
  const status = snapshot.statuses.find((s) => s.entry.source === sourceRef)
  const removed = status?.removed ?? null
  const host = sourceHost(sourceRef)

  const [loaded, setLoaded] = useState<Loaded | null>(null)
  const [loadMessage, setLoadMessage] = useState<SourceMessage | null>(null)
  const [query, setQuery] = useState('')
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const [sortColumn, setSortColumn] = useState<ListSortColumn>('name')
  const [sortDirection, setSortDirection] = useState<ListSortDirection>('asc')
  const [header, setHeader] = useState<IndexSummary | null>(null)
  // An added source with no cached listing while the check setting is off.
  const [needsCheck, setNeedsCheck] = useState(false)
  const thumbs = useRef(new Map<string, string>())
  const [, bump] = useReducer((n: number) => n + 1, 0)

  // A different source starts clean.
  useEffect(() => {
    setLoaded(null)
    setLoadMessage(null)
    setHeader(null)
    setNeedsCheck(false)
    setQuery('')
    setSelectedId(null)
    actions.setMessage(null)
    return () => {
      for (const url of thumbs.current.values()) URL.revokeObjectURL(url)
      thumbs.current.clear()
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sourceRef])

  // The header's facts. `addSource` of an added source reads them from the
  // cached listing without a request; the preview gets them with its rows.
  // `addSource` ADDS a source it does not know, so a source that has left the
  // list (a removal changes the stamp while this page is still open) must not
  // be asked for: the read would put it straight back.
  const inList = status !== undefined
  useEffect(() => {
    if (isPreview || !inList) return
    let cancelled = false
    void binding.sources.addSource(sourceRef).then((r) => {
      if (cancelled || !('ok' in r) || !r.ok) return
      setNeedsCheck(r.needsCheck === true)
      // A summary of an unread source holds only its name and zero counts.
      setHeader(r.needsCheck === true ? null : r.summary)
    })
    return () => {
      cancelled = true
    }
    // A check changes the stamp: the header is read again.
  }, [binding, sourceRef, isPreview, inList, snapshot.stamp])

  // The rows: this source's items against the Library, or the catalog's snapshot.
  useEffect(() => {
    let cancelled = false
    void (async () => {
      if (target.kind === 'preview') {
        const r = await binding.sources.catalogPreview(target.entry)
        if (cancelled) return
        if ('off' in r) return
        if ('ok' in r && r.ok) {
          setLoaded({ rows: r.rows, lastGoodAt: null, summary: r.summary })
          setHeader(r.summary)
          setLoadMessage(null)
        } else if ('ok' in r) {
          setLoadMessage(failureMessage(r, { host, web: !isTauri }))
        }
        return
      }
      const r = await binding.sources.itemStates(target.source)
      if (cancelled) return
      if (r.ok) {
        setLoaded((prev) => ({ rows: r.rows, lastGoodAt: r.lastGoodAt, summary: prev?.summary ?? null }))
        setLoadMessage(null)
      } else {
        setLoadMessage(failureMessage(r, { host, web: !isTauri }))
      }
    })()
    return () => {
      cancelled = true
    }
    // `libraryVersion` and the snapshot's stamp are the re-read signals.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [binding, sourceRef, libraryVersion, snapshot.stamp, snapshot.statuses.length])

  const rows = loaded?.rows ?? []
  const shown = useMemo(() => filterRows(rows, query), [rows, query])
  const rowItems = useMemo(() => shown.map((r) => ({ row: r, item: rowLibraryItem(r) })), [shown])
  const byFileName = useMemo(() => new Map(rowItems.map((x) => [x.item.file.fileName, x])), [rowItems])
  const ordered: Array<{ row: ItemRow; item: LibraryItem }> = useMemo(() => {
    if (view !== 'list') return rowItems
    return sortListItems(rowItems.map((x) => x.item), sortColumn, sortDirection).map((i) => byFileName.get(i.file.fileName)!)
  }, [rowItems, byFileName, view, sortColumn, sortDirection])

  const selected = ordered.find((x) => x.row.id === selectedId) ?? ordered[0] ?? null
  const selectedRow = selected?.row ?? null

  // Thumbnails: the Library's own when the item is in it, else the source's
  // (fetched once, cached by the module, shown as a blob URL).
  useEffect(() => {
    if (isPreview) return
    let cancelled = false
    void (async () => {
      let fetched = 0
      for (const { row } of ordered) {
        if (cancelled || fetched >= MAX_THUMBS) return
        if (row.file !== null && props.localThumbFor(row.file) !== null) continue
        if (row.entry?.thumbnail == null || thumbs.current.has(row.id)) continue
        fetched++
        const r = await binding.sources.thumbnail(sourceRef, row.id)
        if (cancelled) return
        if ('ok' in r && r.ok && r.bytes !== null) {
          thumbs.current.set(row.id, URL.createObjectURL(new Blob([new Uint8Array(r.bytes)], { type: 'image/png' })))
          bump()
        }
      }
    })()
    return () => {
      cancelled = true
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [binding, sourceRef, isPreview, ordered.map((x) => x.row.id).join('|')])

  function thumbFor(row: ItemRow): string | null {
    if (row.file !== null) {
      const local = props.localThumbFor(row.file)
      if (local !== null) return local
    }
    return thumbs.current.get(row.id) ?? null
  }

  // ------------------------------------------------------------- actions

  function targetOf(row: ItemRow): ItemTarget {
    return {
      source: sourceRef,
      id: row.id,
      name: row.name,
      file: row.file,
      state: row.state,
      ...(target.kind === 'preview' ? { fromCatalog: target.entry } : {}),
    }
  }
  const afterAdd = (out: { ok: boolean }): void => {
    if (out.ok && target.kind === 'preview') props.onOpenSource(sourceRef)
  }
  function addRow(row: ItemRow): void {
    void actions.addItem(targetOf(row)).then(afterAdd)
  }
  function insertRow(row: ItemRow): void {
    void actions.insertItem(targetOf(row)).then(afterAdd)
  }
  function defaultAction(row: ItemRow): void {
    if (row.state === 'not_in_library') addRow(row)
    else if (row.file !== null && (row.category === 'Components' || row.category === 'Models') && row.state !== 'in_library_from_other') insertRow(row)
  }

  const running = actions.run !== null && actions.run.source === sourceRef && actions.run.running
  const lockedUi = actions.busy || running
  const run = actions.run !== null && actions.run.source === sourceRef ? actions.run : null

  const headName = header?.name ?? status?.entry.name ?? (target.kind === 'preview' ? target.entry.name : sourceRef)
  const license = header?.license ?? (target.kind === 'preview' ? (target.entry.license ?? null) : null)
  const unreachableText =
    status?.unreachable === true && status.entry.lastCheck !== null
      ? sourceErrorMessage((status.entry.lastCheck.code ?? 'network') as SourceErrorCode, {
          host,
          web: !isTauri,
          ...(status.entry.lastCheck.status !== undefined ? { status: status.entry.lastCheck.status } : {}),
        })
      : null
  const removedReason = removed?.reason ?? ''
  const removedText =
    removed !== null
      ? removedReason
        ? t`The Hew catalog removed this source: ${removedReason}`
        : t`The Hew catalog removed this source.`
      : null
  const author = header?.author ?? ''
  const homepage = header?.homepage ?? ''
  const description = header?.description ?? ''
  // A failed check overwrites lastCheck.at: the header shows the last GOOD check (5.2).
  const lastCheckEntry = status?.entry.lastCheck ?? null
  const lastGood = loaded?.lastGoodAt ?? status?.entry.lastGoodAt ?? (lastCheckEntry?.ok === true ? lastCheckEntry.at : null)

  function selectByOffset(delta: number): void {
    if (ordered.length === 0) return
    const idx = ordered.findIndex((x) => x.row.id === selectedRow?.id)
    const next = Math.min(ordered.length - 1, Math.max(0, (idx === -1 ? 0 : idx) + delta))
    setSelectedId(ordered[next].row.id)
  }

  const listLabel = t`Items in ${headName}`
  return (
    <>
      <div className="hwlib__center hwlib__src-center" data-testid="source-browser" data-source={sourceRef}>
        <div className="hwlib__src-head">
          <h2 className="hwlib__src-title" data-testid="source-title">{headName}</h2>
          {author !== '' && <div className="hwlib__src-by"><Trans>by {author}</Trans></div>}
          <div className="hwlib__src-meta">
            <span data-testid="source-license">{license !== null ? <Trans>License: {license}</Trans> : <Trans>No license stated</Trans>}</span>
            {homepage !== '' && (
              <span data-testid="source-homepage"><ExternalLink href={homepage}><Trans>Homepage</Trans></ExternalLink></span>
            )}
            {!isPreview && <span data-testid="source-checked">{checkedLine(lastGood, nowMs)}</span>}
          </div>
          {description !== '' && <p className="hwlib__src-desc">{description}</p>}

          {removedText !== null && (
            <div className="hwlib__src-warning" role="status" data-testid="source-removed">
              {removedText}
              {removed?.link !== undefined && (
                <>
                  {' '}
                  <ExternalLink href={removed.link}><Trans>Learn more</Trans></ExternalLink>
                </>
              )}
            </div>
          )}
          {unreachableText !== null && removedText === null && (
            <div className="hwlib__src-warning" role="status" data-testid="source-unreachable">
              {unreachableText} <Trans>The list below is from the last time Hew could read this source.</Trans>
            </div>
          )}
          {needsCheck && !isPreview && (
            <div className="hwlib__src-warning" role="status" data-testid="source-not-checked">
              {sourceErrorMessage('not_checked')}
            </div>
          )}
          {loadMessage !== null && !needsCheck && (
            <div className="hwlib__src-warning" role="alert" data-testid="source-load-error">{loadMessage.text}</div>
          )}

          <div className="hwlib__src-buttons">
            {isPreview ? (
              <button type="button" className="hwlib__btn-primary" disabled={lockedUi} onClick={() => void actions.addListedSource(target.entry).then(afterAdd)}>
                <Trans>Add this source</Trans>
              </button>
            ) : (
              <>
                <button type="button" className="hwlib__btn-secondary" disabled={lockedUi} data-testid="source-refresh" onClick={() => void actions.checkSource(sourceRef)}>
                  <Trans>Refresh</Trans>
                </button>
                <button type="button" className="hwlib__btn-secondary" disabled={lockedUi || removed !== null} data-testid="source-add-all" onClick={() => void actions.addAll(sourceRef, headName)}>
                  <Trans>Add all</Trans>
                </button>
                <button
                  type="button"
                  className={status?.followed ? 'hwlib__btn-primary' : 'hwlib__btn-secondary'}
                  aria-pressed={status?.followed === true}
                  disabled={lockedUi || (removed !== null && status?.followed !== true)}
                  data-testid="source-follow"
                  onClick={() => void (status?.followed ? actions.unfollow(sourceRef) : actions.follow(sourceRef, headName))}
                >
                  {status?.followed ? t`Following` : t`Follow`}
                </button>
                <button
                  type="button"
                  className="hwlib__btn-danger"
                  disabled={lockedUi}
                  data-testid="source-remove"
                  onClick={() => actions.requestRemoveSource(sourceRef, headName, props.onRemoved)}
                >
                  <Trans>Remove source</Trans>
                </button>
              </>
            )}
          </div>

          {run !== null && <RunLine actions={actions} run={run} />}
          {actions.message !== null && !isPreview && (
            <div className="hwlib__action-error" role="alert" data-testid="source-message">
              {actions.message.text}
              {actions.message.link !== undefined && (
                <>
                  {' '}
                  <ExternalLink href={actions.message.link}><Trans>Learn more</Trans></ExternalLink>
                </>
              )}
            </div>
          )}

          <div style={{ display: 'flex', gap: 8, alignItems: 'center' }} className="hwlib__src-search">
            <div className="hwlib__search" style={{ flex: 1, width: 'auto' }}>
              <SearchGlyph />
              <input
                type="text"
                placeholder={t`Search this source…`}
                aria-label={t`Search this source`}
                value={query}
                onChange={(e) => setQuery(e.target.value)}
              />
            </div>
            <div className="hwlib__view-toggle" role="group" aria-label={t`View`}>
              <button type="button" className={`hwlib__view-btn${view === 'grid' ? ' hwlib__view-btn--active' : ''}`} aria-label={t`Grid view`} aria-pressed={view === 'grid'} onClick={() => props.onViewChange('grid')}>
                <GridViewGlyph />
              </button>
              <button type="button" className={`hwlib__view-btn${view === 'list' ? ' hwlib__view-btn--active' : ''}`} aria-label={t`List view`} aria-pressed={view === 'list'} onClick={() => props.onViewChange('list')}>
                <ListViewGlyph />
              </button>
            </div>
          </div>
        </div>

        <div
          className="hwlib__src-grid"
          role={view === 'list' ? 'grid' : 'listbox'}
          aria-label={listLabel}
          tabIndex={0}
          aria-activedescendant={selected ? tileElementId(selected.item.file.fileName) : undefined}
          onKeyDown={(e) => {
            if (e.key === 'ArrowRight' || e.key === 'ArrowDown') {
              e.preventDefault()
              selectByOffset(1)
            } else if (e.key === 'ArrowLeft' || e.key === 'ArrowUp') {
              e.preventDefault()
              selectByOffset(-1)
            } else if (e.key === 'Enter' && selectedRow !== null) {
              e.preventDefault()
              defaultAction(selectedRow)
            }
          }}
        >
          {loaded === null && loadMessage === null ? (
            <div className="hwlib__src-empty"><Trans>Loading…</Trans></div>
          ) : needsCheck && ordered.length === 0 ? (
            <div className="hwlib__src-empty" />
          ) : ordered.length === 0 ? (
            <div className="hwlib__src-empty" data-testid="source-empty">
              {query !== '' ? <Trans>No items match your search.</Trans> : <Trans>This source has no items.</Trans>}
            </div>
          ) : view === 'list' ? (
            <>
              <LibraryListHead
                widths={props.listWidths}
                sortColumn={sortColumn}
                sortDirection={sortDirection}
                onSort={(c) => {
                  if (c === sortColumn) setSortDirection((d) => (d === 'asc' ? 'desc' : 'asc'))
                  else {
                    setSortColumn(c)
                    setSortDirection('asc')
                  }
                }}
                onResize={props.onResizeListCol}
              />
              <div className="hwlib__list">
                {ordered.map(({ row, item }) => (
                  <LibraryListRow
                    key={row.id}
                    item={item}
                    thumbUrl={thumbFor(row)}
                    textureUrl={null}
                    selected={row.id === selectedRow?.id}
                    placementCount={0}
                    inPalette={false}
                    widths={props.listWidths}
                    subText={rowStateLabel(row)}
                    noActions
                    onSelect={() => setSelectedId(row.id)}
                    onActivateDefault={() => defaultAction(row)}
                    onOpenMenu={() => {}}
                  />
                ))}
              </div>
            </>
          ) : (
            <div className={`hwlib__grid-inner hwlib__grid-inner--${selectedRow?.category === 'Materials' ? 'material' : 'component'}`}>
              {ordered.map(({ row, item }) => (
                <LibraryTile
                  key={row.id}
                  item={item}
                  thumbUrl={thumbFor(row)}
                  textureUrl={null}
                  selected={row.id === selectedRow?.id}
                  placementCount={0}
                  inPalette={false}
                  defaultActionLabel=""
                  subText={rowStateLabel(row)}
                  noActions
                  onSelect={() => setSelectedId(row.id)}
                  onActivateDefault={() => defaultAction(row)}
                  onOpenMenu={() => {}}
                />
              ))}
            </div>
          )}
        </div>
      </div>

      <span
        className="hwlib__col-resize"
        role="separator"
        aria-orientation="vertical"
        aria-label={t`Resize detail pane`}
        onPointerDown={props.onResizeDetail}
      />

      <SourceItemDetail
        row={selectedRow}
        summary={header}
        thumbUrl={selectedRow !== null ? thumbFor(selectedRow) : null}
        width={detailWidth}
        native={native}
        removed={removed !== null}
        removedText={removedText}
        busy={lockedUi}
        error={isPreview ? (actions.message?.text ?? null) : null}
        onAdd={() => selectedRow !== null && addRow(selectedRow)}
        onInsert={() => selectedRow !== null && insertRow(selectedRow)}
        onUpdate={() => selectedRow !== null && void actions.updateItem(targetOf(selectedRow))}
      />
    </>
  )
}

/** Progress while Add all or Follow runs, then the summary with Retry. */
function RunLine({ actions, run }: { actions: UseSourceActions; run: NonNullable<UseSourceActions['run']> }) {
  const { t } = useLingui()
  if (run.running) {
    const done = run.progress?.done ?? 0
    const total = run.progress?.total ?? 0
    return (
      <div className="hwlib__src-progress" role="status" data-testid="source-progress">
        <span>{t`Adding ${done} of ${total}…`}</span>
        <progress value={done} max={Math.max(total, 1)} aria-label={t`Progress`} />
        <button type="button" className="hwlib__btn-secondary" data-testid="source-cancel-run" onClick={actions.cancelRun}>
          <Trans>Cancel</Trans>
        </button>
      </div>
    )
  }
  const s = run.summary
  const added = s?.added ?? 0
  const failed = s?.failed ?? 0
  return (
    <div role="status" data-testid="source-run-summary">
      {s !== null && (
        <div className="hwlib__src-summary">
          <span>{t`${added} added, ${failed} failed`}</span>
          {s.cancelled && <span><Trans>Stopped.</Trans></span>}
          {s.retry.length > 0 && (
            <button type="button" className="hwlib__btn-secondary" data-testid="source-retry" onClick={() => void actions.retryRun()}>
              <Trans>Retry</Trans>
            </button>
          )}
          <button type="button" className="hwlib__btn-secondary" onClick={actions.dismissRun}>
            <Trans>Done</Trans>
          </button>
        </div>
      )}
      {run.message !== null && <div className="hwlib__action-error" role="alert" data-testid="source-run-message">{run.message.text}</div>}
      {s === null && (
        <button type="button" className="hwlib__btn-secondary" onClick={actions.dismissRun}>
          <Trans>Done</Trans>
        </button>
      )}
    </div>
  )
}

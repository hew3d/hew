/**
 * ComponentsPanel — tray section listing every component definition in the
 * document (v1.1 assets lane, docs/design/v1.1-cycle.md Lane A).
 *
 * Each row shows a small thumbnail of the definition's own geometry, its
 * display name (its own kernel name, or a positional "Component N"
 * fallback — the same convention an unnamed instance falls back to,
 * `treeModel.entityLabel`), and its live instance count
 * (`scene.definition_usage`, matching `hew.query.scene`'s API-side `usage`
 * field). Clicking a row selects every instance that places it
 * (`scene.instances_of`, the same broad set ObjectInfoPanel's "(N
 * instances)" click already selects) — a way to see everywhere a
 * definition is used. Double-click does nothing at 1.1, mirroring the
 * design's explicit call-out (no in-context edit entry point from here
 * yet).
 *
 * A Filter box mirrors MaterialPalette's exactly: `filter` state, a
 * case-insensitive substring match on the definition's display name,
 * identical input styling/placement, and filtering never touches selection
 * state (an empty filter shows every definition).
 *
 * Row actions mirror MaterialPalette's: Rename (✎ button, or double-click
 * the name — Finder-style, like TagsPanel) and Delete (× button; the
 * caller decides whether to confirm, based on `definition_usage`).
 *
 * Thumbnails: `scene.render_definition_thumbnail(id, size)` rasterizes a
 * definition's OWN geometry — the same fitted-isometric PNG renderer the
 * Library's item thumbnails use, but sourced straight from the live
 * document instead of standalone item bytes, so it works even for a
 * currently-unused definition (`extract_item` has nothing to select from
 * an unplaced definition — see the wasm-api doc comment). Rendering one
 * costs a real rasterization pass, so rows render their thumbnail LAZILY,
 * one at a time, off the critical path (a `useEffect` queue, not inline
 * during render) — a document with many definitions stays responsive.
 * Results are cached per definition handle in a mount-scoped ref (object
 * URLs revoked on unmount and on a document-load `docGeneration` bump, the
 * same convention as MaterialPalette's texture-thumbnail cache) and marked
 * with the `docRev` they were rendered at, so an edit to a definition's own
 * geometry (which bumps `docRev` like every other document change)
 * invalidates and lazily re-renders just that definition's thumbnail.
 *
 * `PurgeUnusedButton` (exported alongside, mirroring ScenesPanel's
 * `ScenesAddButton`) is the section header's "Purge Unused…" control,
 * wired by App.tsx via `TraySection`'s `headerRight`.
 */

import { useEffect, useRef, useState } from 'react'
import { Trans, useLingui } from '@lingui/react/macro'
import type { Scene as WasmScene } from '../wasm/loader'
import { entityLabel } from './treeModel'
import type { NodeRef } from './treeModel'
import { isComposingKey } from '../input/keyboard'
import { compareNames } from './nameSort'
import type { LibraryLink } from '../library/libraryUpdates'
import { LibraryMark } from './LibraryMark'

interface Props {
  scene: WasmScene
  /** Bumped by the parent on any document change to trigger a re-query
   *  (and, for thumbnails, to invalidate stale cache entries). */
  docRev: number
  /** Document-load generation from App — a change means a NEW document, so
   *  the handle-keyed thumbnail cache (whose keys collide across loads)
   *  must be revoked and rebuilt. Optional (defaults 0) for tests that
   *  don't exercise cross-load behavior — mirrors MaterialPalette's prop. */
  docGeneration?: number
  /** Click a row → select every instance of that definition. */
  onSelectNodes: (nodes: NodeRef[]) => void
  /** Rename a definition in place. Returns inline error text (a kernel
   *  refusal) or null on success — mirroring TagsPanel's onRenameTag. */
  onRenameComponent: (id: bigint, name: string) => string | null
  /** Request a definition delete. The caller decides whether to confirm
   *  first (checking `scene.definition_usage`) before actually deleting. */
  onDeleteComponent: (id: bigint) => void
  /** Where each definition stands against its Library item (definition
   *  handle → link); a row shows a marker when it is newer there or changed
   *  here. Absent: no markers. */
  libraryLinks?: ReadonlyMap<string, LibraryLink>
  /** A marker on a part with a newer Library version was clicked. */
  onReviewLibraryUpdates?: () => void
}

const ROW_STYLE: React.CSSProperties = {
  display: 'flex',
  alignItems: 'center',
  gap: '6px',
  padding: '3px 0',
  fontFamily: 'monospace',
  fontSize: '11px',
  color: 'var(--text-secondary, #ccc)',
}

const ICON_BUTTON_STYLE: React.CSSProperties = {
  background: 'none',
  border: 'none',
  color: 'var(--text-faint, #666)',
  cursor: 'pointer',
  padding: '0 2px',
  lineHeight: 1,
  flexShrink: 0,
}

// Same footprint as MaterialPalette's SWATCH_STYLE ("Size ~ the Materials
// swatch"). Rendered at 2x for a crisp thumbnail on HiDPI displays.
const THUMB_PX = 36
const THUMB_RENDER_PX = THUMB_PX * 2

const THUMB_STYLE: React.CSSProperties = {
  width: `${THUMB_PX}px`,
  height: `${THUMB_PX}px`,
  borderRadius: '3px',
  flexShrink: 0,
  objectFit: 'cover',
  background: 'var(--surface-input, #111)',
  border: '1px solid var(--border-strong, #444)',
}

export function ComponentsPanel({
  scene,
  docRev,
  docGeneration = 0,
  onSelectNodes,
  onRenameComponent,
  onDeleteComponent,
  libraryLinks,
  onReviewLibraryUpdates,
}: Props) {
  const { t } = useLingui()
  // Suppress the docRev-triggers-re-render lint — intentionally re-queries
  // component_ids/definition_usage/component_name from the scene on every
  // document change, the same convention MaterialPalette follows.
  void docRev
  const componentIds = Array.from(scene.component_ids())

  // --- Thumbnail cache -------------------------------------------------
  // Object URLs keyed by definition handle, held in a mount-scoped ref (not
  // module-global) so they can't outlive the panel and leak. Dropped
  // wholesale and revoked on a document LOAD (docGeneration change —
  // handles collide across loads the same way MaterialPalette's material
  // handles do).
  const thumbCacheRef = useRef<Map<string, string>>(new Map())
  const thumbGenRef = useRef(docGeneration)
  // The docRev each cache entry was rendered at, so a later edit to that
  // definition's own geometry (or anything else — docRev is document-wide,
  // not per-definition) marks it stale for a lazy re-render, without
  // discarding the still-valid thumbnail in the meantime.
  const thumbRevRef = useRef<Map<string, number>>(new Map())
  if (thumbGenRef.current !== docGeneration) {
    for (const url of thumbCacheRef.current.values()) URL.revokeObjectURL(url)
    thumbCacheRef.current.clear()
    thumbRevRef.current.clear()
    thumbGenRef.current = docGeneration
  }
  useEffect(() => {
    const cache = thumbCacheRef.current
    return () => {
      for (const url of cache.values()) URL.revokeObjectURL(url)
      cache.clear()
    }
  }, [])

  // A plain tick counter to force a re-render once a queued thumbnail
  // finishes — the cache itself lives outside React state so a render
  // never blocks on it.
  const [, setThumbTick] = useState(0)

  // Lazily render thumbnails ONE AT A TIME, off the critical render path: a
  // definition's geometry can be non-trivial to rasterize, and doing it
  // inline for every row (the way MaterialPalette's cheap
  // `material_texture_bytes` cache read can afford to) would jank a panel
  // with many definitions. Re-queues whenever the definition list or docRev
  // changes; `cancelled` stops a stale queue on unmount or once a newer
  // effect run supersedes it.
  useEffect(() => {
    let cancelled = false
    const pending = componentIds.filter((id) => thumbRevRef.current.get(id.toString()) !== docRev)
    let i = 0
    function renderNext() {
      if (cancelled || i >= pending.length) return
      const id = pending[i]
      i += 1
      const key = id.toString()
      const png = scene.render_definition_thumbnail(id, THUMB_RENDER_PX)
      if (!cancelled) {
        const prevUrl = thumbCacheRef.current.get(key)
        if (png !== undefined) {
          thumbCacheRef.current.set(key, URL.createObjectURL(new Blob([new Uint8Array(png)], { type: 'image/png' })))
          if (prevUrl !== undefined) URL.revokeObjectURL(prevUrl)
          thumbRevRef.current.set(key, docRev)
        } else if (prevUrl === undefined) {
          // Nothing to draw and nothing shown: an honest blank until the
          // document changes again.
          thumbRevRef.current.set(key, docRev)
        }
        // A render that failed while a thumbnail is already showing is
        // best-effort's other half: keep the last good image and leave the
        // row pending so the next document change retries it. The kernel
        // refuses previews for EVERY definition while any component is open
        // for editing, so treating that as "this definition is empty" would
        // blank the whole panel every time one component is double-clicked.
        setThumbTick((t) => t + 1)
      }
      // Yield between renders (a macrotask, not a microtask) so a long
      // definition list never blocks a single frame.
      setTimeout(renderNext, 0)
    }
    renderNext()
    return () => {
      cancelled = true
    }
    // componentIds is a fresh array every render (derived from docRev), so
    // it is deliberately not a dep — docRev (which it's derived from) is.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [scene, docRev])

  // --- Filter state ------------------------------------------------------
  // Live, case-insensitive substring match on the definition's display
  // name — the exact same mechanics as MaterialPalette's filter. Applied at
  // render time against the already-fresh rows; filtering never touches
  // the scene and never affects selection.
  const [filter, setFilter] = useState('')
  const normalizedFilter = filter.toLowerCase()
  const filterInputRef = useRef<HTMLInputElement>(null)

  const [editingId, setEditingId] = useState<bigint | null>(null)
  const [editingText, setEditingText] = useState('')
  const [editingError, setEditingError] = useState<string | null>(null)

  function startRename(id: bigint, currentName: string) {
    setEditingId(id)
    setEditingText(currentName)
    setEditingError(null)
  }
  function cancelRename() {
    setEditingId(null)
    setEditingText('')
    setEditingError(null)
  }
  function commitRename(id: bigint, currentName: string) {
    const next = editingText.trim()
    if (next.length === 0 || next === currentName) {
      cancelRename()
      return
    }
    const err = onRenameComponent(id, next)
    if (err !== null) {
      setEditingError(err)
      return
    }
    cancelRename()
  }

  if (componentIds.length === 0) {
    return (
      <div style={{ color: 'var(--text-faint, #888)', fontSize: '10px', fontFamily: 'monospace' }}>
        <Trans>No component definitions in this document.</Trans>
      </div>
    )
  }

  // Rows carry their ORIGINAL (unfiltered) index, so an unnamed
  // definition's positional "Component N" fallback numbering stays stable
  // as the filter narrows the visible set, instead of renumbering.
  const rows = componentIds.map((id, index) => {
    const rawName = scene.component_name(id)
    const name = rawName !== undefined && rawName.length > 0 ? rawName : entityLabel('instance', index)
    return { id, name }
  })
  const matchingRows = normalizedFilter === '' ? rows : rows.filter((r) => r.name.toLowerCase().includes(normalizedFilter))
  // Listed by name (case-insensitive, numeric); the numbers above stay put.
  const filteredRows = [...matchingRows].sort((a, b) => compareNames(a.name, b.name))

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: '2px' }}>
      {/* Filter — mirrors MaterialPalette's filter box exactly. */}
      <div style={{ position: 'relative', display: 'flex', alignItems: 'center', marginBottom: '4px' }}>
        <span
          aria-hidden="true"
          style={{
            position: 'absolute',
            left: '6px',
            color: 'var(--text-faint, #888)',
            fontSize: '11px',
            pointerEvents: 'none',
          }}
        >
          ⌕
        </span>
        <input
          ref={filterInputRef}
          type="text"
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          placeholder={t`Filter components…`}
          aria-label={t`Filter components`}
          style={{
            flex: 1,
            fontSize: '11px',
            fontFamily: 'monospace',
            background: 'var(--surface-input, #444)',
            color: 'var(--text-primary, #eee)',
            border: 'none',
            borderRadius: '3px',
            padding: '3px 20px',
            boxSizing: 'border-box',
          }}
        />
        {filter !== '' && (
          <button
            type="button"
            onClick={() => {
              setFilter('')
              // This button unmounts the instant the filter becomes empty
              // (it only renders while filter !== ''); without this, focus
              // would drop to <body> rather than staying in the filter flow.
              filterInputRef.current?.focus()
            }}
            aria-label={t`Clear filter`}
            style={{
              position: 'absolute',
              right: '4px',
              background: 'none',
              border: 'none',
              color: 'var(--text-faint, #888)',
              cursor: 'pointer',
              fontSize: '13px',
              lineHeight: 1,
              padding: '2px',
            }}
          >
            ×
          </button>
        )}
      </div>

      {filteredRows.map(({ id, name }) => {
        const usage = scene.definition_usage(id)
        const isEditing = editingId === id
        const thumbUrl = thumbCacheRef.current.get(id.toString())

        return (
          <div key={id.toString()}>
            <div style={ROW_STYLE}>
              {thumbUrl !== undefined ? (
                <img data-testid="components-row-thumb" src={thumbUrl} alt="" style={THUMB_STYLE} />
              ) : (
                <div data-testid="components-row-thumb-placeholder" aria-hidden="true" style={THUMB_STYLE} />
              )}
              {isEditing ? (
                <input
                  autoFocus
                  aria-label={t`Rename component ${name}`}
                  value={editingText}
                  onChange={(e) => setEditingText(e.target.value)}
                  onFocus={(e) => e.currentTarget.select()}
                  onKeyDown={(e) => {
                    if (isComposingKey(e)) return
                    if (e.key === 'Enter') {
                      e.preventDefault()
                      commitRename(id, name)
                    } else if (e.key === 'Escape') {
                      e.stopPropagation()
                      cancelRename()
                    }
                  }}
                  onBlur={() => commitRename(id, name)}
                  spellCheck={false}
                  style={{
                    flex: 1,
                    minWidth: 0,
                    background: 'var(--surface-input, #111)',
                    border: '1px solid var(--accent-border)',
                    borderRadius: '3px',
                    color: 'var(--text-primary, #eee)',
                    fontFamily: 'monospace',
                    fontSize: '11px',
                    padding: '1px 4px',
                    outline: 'none',
                  }}
                />
              ) : (
                <span
                  role="button"
                  tabIndex={0}
                  data-testid="components-row-name"
                  onClick={() => onSelectNodes(Array.from(scene.instances_of(id)).map((iid) => ({ kind: 'instance' as const, id: iid })))}
                  onDoubleClick={() => startRename(id, name)}
                  onKeyDown={(e) => {
                    if (e.key === 'Enter' || e.key === ' ') {
                      e.preventDefault()
                      onSelectNodes(Array.from(scene.instances_of(id)).map((iid) => ({ kind: 'instance' as const, id: iid })))
                    }
                  }}
                  style={{
                    flex: 1,
                    minWidth: 0,
                    overflow: 'hidden',
                    textOverflow: 'ellipsis',
                    whiteSpace: 'nowrap',
                    cursor: 'pointer',
                    // Names were unreadably light before this pass — the
                    // same primary text color/weight the Outliner uses for
                    // names, at the panel's own (monospace, 11px) size.
                    // Secondary info (the usage count below) stays faint.
                    color: 'var(--text-primary)',
                  }}
                >
                  {name}
                </span>
              )}
              {!isEditing && <LibraryMark link={libraryLinks?.get(id.toString())} onReview={onReviewLibraryUpdates} />}
              {!isEditing && (
                <span
                  data-testid="components-row-usage"
                  style={{ fontSize: '10px', color: 'var(--text-faint, #666)', minWidth: '16px', textAlign: 'right', flexShrink: 0 }}
                >
                  {usage}
                </span>
              )}
              {!isEditing && (
                <button
                  type="button"
                  onClick={() => startRename(id, name)}
                  aria-label={t`Rename component ${name}`}
                  style={{ ...ICON_BUTTON_STYLE, fontSize: '11px' }}
                >
                  ✎
                </button>
              )}
              {!isEditing && (
                <button
                  type="button"
                  onClick={() => onDeleteComponent(id)}
                  aria-label={t`Delete component ${name}`}
                  style={{ ...ICON_BUTTON_STYLE, fontSize: '12px' }}
                >
                  ×
                </button>
              )}
            </div>
            {isEditing && editingError !== null && (
              <div style={{ padding: '2px 0 4px 4px', fontSize: '10px', color: 'var(--scene-delete-text)' }}>
                {editingError}
              </div>
            )}
          </div>
        )
      })}
      {normalizedFilter !== '' && filteredRows.length === 0 && (
        <div style={{ color: 'var(--text-faint, #888)', fontSize: '10px', padding: '4px 0' }}>
          <Trans>No components match</Trans>
        </div>
      )}
    </div>
  )
}

const HOVER_BG = 'var(--surface-hover)'

/** Section-header "Purge Unused…" control — mirrors ScenesPanel's
 * ScenesAddButton binding shape (a small button passed through
 * TraySection's `headerRight`). Text, not an icon: this is a rarer,
 * consequential action (deletes palette content), so it earns a label
 * rather than a glyph a user might click by reflex. */
export function PurgeUnusedButton({ onClick }: { onClick: () => void }) {
  const { t } = useLingui()
  const [hovered, setHovered] = useState(false)
  return (
    <button
      type="button"
      aria-label={t`Purge Unused…`}
      title={t`Delete every unused material and component definition`}
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
      onClick={(e) => {
        e.stopPropagation()
        onClick()
      }}
      style={{
        padding: '2px 6px',
        borderRadius: 6,
        border: 'none',
        background: hovered ? HOVER_BG : 'transparent',
        color: 'var(--text-secondary)',
        cursor: 'pointer',
        fontSize: '10px',
        fontFamily: 'monospace',
        lineHeight: 1.4,
      }}
    >
      <Trans>Purge Unused…</Trans>
    </button>
  )
}

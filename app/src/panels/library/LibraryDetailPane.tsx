/**
 * LibraryDetailPane — the right-hand pane of the Library browser (frames
 * 1a/1b detail, 1f manage). Reads the manifest only — it never touches
 * geometry buffers, matching the design's "detail pane reads the manifest
 * only" note.
 *
 * Every `⋯` menu action also has a home here (settled decision: nothing
 * requires the menu). Name and keywords are edit-in-place — no separate
 * "edit mode" toggle, matching the mock's always-editable name field and
 * always-visible keyword chip remove/add affordances.
 */

import { useEffect, useLayoutEffect, useRef, useState } from 'react'
import { materialSubline, metadataLine, type CollectionNode, normalizeCollectionPath, savedLine } from '../../library/libraryModel'
import type { LibraryCategory, LibraryItem } from '../../library/types'
import { CubeGlyph, RerenderGlyph, WarningGlyph } from './icons'
import { revealLabel } from './LibraryMenu'

/** Sentinel `<option>` value for "New collection…" — a leading NUL makes it
 * impossible to collide with a real collection path (`normalizeCollectionPath`
 * only ever joins trimmed, printable segments with `/`). */
const NEW_COLLECTION_VALUE = '\u0000__new__'

function actionLabels(category: LibraryCategory): { primary: string; secondary: string } {
  if (category === 'component') return { primary: 'Insert', secondary: 'Open as document' }
  if (category === 'model') return { primary: 'Open as document', secondary: 'Insert' }
  return { primary: 'Paint', secondary: 'Add to palette' }
}

export interface LibraryDetailPaneProps {
  item: LibraryItem | null
  thumbUrl: string | null
  textureUrl: string | null
  /** The pane's user-resizable width in px (playtest round-4 finding #1),
   * owned and persisted by `LibraryDialog`. Applied as an inline style
   * override of `.hwlib__detail`'s CSS default — same convention as
   * `LibraryListHead`'s column widths. */
  width: number
  /** `true` in the desktop `'window'` variant: render the primary/secondary
   * action buttons and the manage-row (Reveal/Delete) buttons as unstyled
   * system `<button>`s instead of the custom-painted ones, so WKWebView's
   * native push-button chrome shows through (playtest round-3 finding #5).
   * The web/modal variant keeps the custom styling — there's no native look
   * worth chasing there. */
  native: boolean
  /** The full collection tree (existing paths + synthesized parents), in
   * pre-order with `depth` for indentation — see `collectionTreeFromPaths`. */
  collections: CollectionNode[]
  placementCount: number
  canReveal: boolean
  inPalette: boolean
  nowMs: number
  /** One-line message from the last failed manage mutation (rename/keyword/
   * collection/re-render/reveal/delete), or null. Cleared by the dialog on
   * the next successful mutation or selection change. */
  actionError: string | null
  /** True while this item is mid-delete-confirmation (armed by either the
   * bottom-row "Delete…" button or the tile's ⋯ menu). */
  deleteArmed: boolean
  onRename: (name: string) => void
  onAddKeyword: (keyword: string) => void
  onRemoveKeyword: (keyword: string) => void
  onChangeCollection: (collection: string | null) => void
  /** Scrubs `meta.sourceDoc` (the "Remove Source Info" action) — only ever
   * called while `item.meta.sourceDoc` is set (the row that calls it is
   * itself conditional on that). */
  onRemoveSourceInfo: () => void
  onPrimaryAction: () => void
  onSecondaryAction: () => void
  onOpenAsDocument: () => void
  onReRenderThumbnail: () => void
  onReveal: () => void
  onRequestDelete: () => void
  onConfirmDelete: () => void
  onCancelDelete: () => void
}

export function LibraryDetailPane({
  item,
  thumbUrl,
  textureUrl,
  width,
  native,
  collections,
  placementCount,
  canReveal,
  inPalette,
  nowMs,
  actionError,
  deleteArmed,
  onRename,
  onAddKeyword,
  onRemoveKeyword,
  onChangeCollection,
  onRemoveSourceInfo,
  onPrimaryAction,
  onSecondaryAction,
  onOpenAsDocument,
  onReRenderThumbnail,
  onReveal,
  onRequestDelete,
  onConfirmDelete,
  onCancelDelete,
}: LibraryDetailPaneProps) {
  const [nameDraft, setNameDraft] = useState(item?.displayName ?? '')
  const [addingKeyword, setAddingKeyword] = useState(false)
  const [keywordDraft, setKeywordDraft] = useState('')
  const [creatingCollection, setCreatingCollection] = useState(false)
  const [collectionDraft, setCollectionDraft] = useState('')
  const nameFieldRef = useRef<HTMLTextAreaElement>(null)

  /** Swaps a custom-painted button class for the unstyled native one in the
   * `'window'` variant (playtest round-3 finding #5). `.hwlib__native-btn`
   * still picks up its parent's layout (`.hwlib__manage-row
   * .hwlib__native-btn { flex: 1 1 0 }`, etc. — see LIBRARY_CSS) — only the
   * paint (background/border/radius/color) is gone, letting WKWebView's own
   * push-button chrome show through. */
  function btnClass(styled: string): string {
    return native ? 'hwlib__native-btn' : styled
  }

  // Reset the drafts whenever the shown item changes (a different selection,
  // or a fresh reload after this pane's own edit committed) — never fight a
  // still-in-progress edit with a stale prop otherwise, since the effect
  // only fires when the *identity* (file name) or the committed name changes.
  useEffect(() => {
    setNameDraft(item?.displayName ?? '')
    setAddingKeyword(false)
    setKeywordDraft('')
    setCreatingCollection(false)
    setCollectionDraft('')
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [item?.file.fileName, item?.displayName])

  // Auto-grow the name textarea to fit its (possibly multi-line) content —
  // the maintainer explicitly wants the FULL name visible, wrapped, never
  // truncated. Re-measured on every draft change, including the reset
  // above; layout-effect so the height lands BEFORE paint (no one-frame
  // clipped flash on selection change).
  useLayoutEffect(() => {
    const el = nameFieldRef.current
    if (!el) return
    el.style.height = 'auto'
    el.style.height = `${el.scrollHeight}px`
  }, [nameDraft])

  // Inline width override for `.hwlib__detail`'s resizable state (finding
  // #1) — `flex` too, so the flex-basis stays in lockstep with the plain
  // `width` (matching `LibraryListHead`'s own `widths[col.id]` styling).
  const widthStyle: React.CSSProperties = { flex: `0 0 ${width}px`, width }

  if (item === null) {
    return (
      <div className="hwlib__detail hwlib__detail--empty" style={widthStyle}>
        <p>Select an item to see its details.</p>
      </div>
    )
  }

  if (item.error) {
    return (
      <div className="hwlib__detail" style={widthStyle}>
        <div className="hwlib__detail-thumb hwlib__detail-thumb--error">
          <WarningGlyph />
        </div>
        <div className="hwlib__detail-name">{item.displayName}</div>
        <p className="hwlib__detail-error">{item.error}</p>
        {actionError !== null && <div className="hwlib__action-error">{actionError}</div>}
        <div className="hwlib__manage-row">
          <button type="button" className={btnClass('hwlib__btn-danger')} onClick={onRequestDelete}>
            Delete from library…
          </button>
        </div>
        {deleteArmed && (
          <DeleteConfirm onConfirm={onConfirmDelete} onCancel={onCancelDelete} name={item.displayName} />
        )}
      </div>
    )
  }

  const labels = actionLabels(item.category)
  const isMaterial = item.category === 'material'
  const materialEntry = item.summary.material_entries[0]

  /** Names are single-line by contract: pasted newlines (the one way the
   * wrapping textarea can still receive them) collapse to single spaces. */
  function singleLine(raw: string): string {
    return raw.replace(/\s*[\r\n]+\s*/g, ' ')
  }

  function commitName() {
    const trimmed = singleLine(nameDraft).trim()
    if (trimmed !== '' && item !== null && trimmed !== item.displayName) onRename(trimmed)
    else setNameDraft(item?.displayName ?? '')
  }

  function commitKeyword() {
    const trimmed = keywordDraft.trim()
    if (trimmed !== '') onAddKeyword(trimmed)
    setKeywordDraft('')
    setAddingKeyword(false)
  }

  function commitNewCollection() {
    const normalized = normalizeCollectionPath(collectionDraft)
    if (normalized !== '') onChangeCollection(normalized)
    setCreatingCollection(false)
    setCollectionDraft('')
  }

  return (
    <div className="hwlib__detail" style={widthStyle}>
      <div className="hwlib__detail-thumb">
        {isMaterial ? (
          <div className="hwlib__detail-swatch hwlib__checker">
            {materialEntry && materialEntry.texture_asset !== null && textureUrl !== null ? (
              <div
                className="hwlib__detail-swatch-fill"
                style={{
                  backgroundImage: `url(${textureUrl})`,
                  backgroundSize: 'cover',
                  opacity: materialEntry.color[3] / 255,
                }}
              />
            ) : materialEntry ? (
              <div
                className="hwlib__detail-swatch-fill"
                style={{
                  background: `rgba(${materialEntry.color[0]}, ${materialEntry.color[1]}, ${materialEntry.color[2]}, ${materialEntry.color[3] / 255})`,
                }}
              />
            ) : null}
          </div>
        ) : thumbUrl ? (
          <img src={thumbUrl} alt="" className="hwlib__detail-thumb-img" />
        ) : (
          <CubeGlyph />
        )}
        <button
          type="button"
          className="hwlib__detail-rerender"
          onClick={onReRenderThumbnail}
          aria-label="Re-render thumbnail"
        >
          <RerenderGlyph /> Re-render
        </button>
      </div>

      <textarea
        ref={nameFieldRef}
        className="hwlib__name-input"
        value={nameDraft}
        aria-label="Item name"
        rows={1}
        onChange={(e) => setNameDraft(singleLine(e.target.value))}
        onBlur={commitName}
        onKeyDown={(e) => {
          // Enter commits (matches the old single-line input's behavior) —
          // it must never insert a newline into the name. Enter DURING IME
          // composition belongs to the composer, not to us.
          if (e.key === 'Enter' && !e.nativeEvent.isComposing) {
            e.preventDefault()
            commitName()
            ;(e.target as HTMLTextAreaElement).blur()
          }
        }}
      />

      <div className="hwlib__keywords" aria-label="Keywords">
        {(item.meta.keywords ?? []).map((kw) => (
          <span key={kw} className="hwlib__keyword-chip">
            {kw}
            <button type="button" aria-label={`Remove keyword ${kw}`} onClick={() => onRemoveKeyword(kw)}>
              ×
            </button>
          </span>
        ))}
        {addingKeyword ? (
          <input
            className="hwlib__keyword-input"
            autoFocus
            value={keywordDraft}
            aria-label="New keyword"
            onChange={(e) => setKeywordDraft(e.target.value)}
            onBlur={commitKeyword}
            onKeyDown={(e) => {
              if (e.key === 'Enter' && !e.nativeEvent.isComposing) {
                e.preventDefault()
                commitKeyword()
              } else if (e.key === 'Escape') {
                e.preventDefault()
                e.stopPropagation()
                setKeywordDraft('')
                setAddingKeyword(false)
              }
            }}
          />
        ) : (
          <button type="button" className="hwlib__keyword-add" onClick={() => setAddingKeyword(true)}>
            + keyword
          </button>
        )}
      </div>

      <label className="hwlib__field-row">
        <span>Collection</span>
        {creatingCollection ? (
          <input
            className="hwlib__select"
            autoFocus
            aria-label="New collection path"
            placeholder="Hardware/Fasteners"
            value={collectionDraft}
            onChange={(e) => setCollectionDraft(e.target.value)}
            onBlur={commitNewCollection}
            onKeyDown={(e) => {
              if (e.key === 'Enter' && !e.nativeEvent.isComposing) {
                e.preventDefault()
                commitNewCollection()
              } else if (e.key === 'Escape') {
                e.preventDefault()
                e.stopPropagation()
                setCreatingCollection(false)
                setCollectionDraft('')
              }
            }}
          />
        ) : (
          <select
            className="hwlib__select"
            // Normalize for comparison: a hand-edited or legacy item may
            // store "Hardware/" or "//Fasteners"; the option values are
            // normalized paths, and an unnormalized stored value must
            // still select its own collection rather than reading "None".
            value={normalizeCollectionPath(item.meta.collection ?? '')}
            onChange={(e) => {
              if (e.target.value === NEW_COLLECTION_VALUE) {
                setCreatingCollection(true)
                setCollectionDraft('')
                return
              }
              onChangeCollection(e.target.value === '' ? null : e.target.value)
            }}
          >
            <option value="">None</option>
            {collections.map((c) => (
              <option key={c.path} value={c.path}>
                {'  '.repeat(c.depth)}
                {c.label}
              </option>
            ))}
            <option value={NEW_COLLECTION_VALUE}>+ New collection…</option>
          </select>
        )}
      </label>

      <div className="hwlib__meta">
        <div>{metadataLine(item)}</div>
        {isMaterial && materialEntry && <div>{materialSubline(materialEntry)}</div>}
        <div>{savedLine(item, nowMs)}</div>
        {/* Models are their own source, never extracted from another
            model — Source Info only ever applies to components/materials
            saved out of a document. */}
        {item.category !== 'model' && item.meta.sourceDoc !== undefined && (
          <div className="hwlib__source-row">
            <span>from {item.meta.sourceDoc}</span>
            <button type="button" className="hwlib__btn-link" onClick={onRemoveSourceInfo}>
              Remove Source Info
            </button>
          </div>
        )}
      </div>

      {placementCount > 0 && (
        <div className="hwlib__inmodel">
          {placementCount} {placementCount === 1 ? 'instance' : 'instances'} in this model
        </div>
      )}
      {/* Materials aren't "placed" as instances, so there's no count — same
          "in this model" wording as components, just without one. */}
      {isMaterial && inPalette && <div className="hwlib__inmodel">In this model</div>}
      {actionError !== null && <div className="hwlib__action-error">{actionError}</div>}

      <div className="hwlib__actions">
        <button type="button" className={btnClass('hwlib__btn-primary')} onClick={onPrimaryAction}>
          {labels.primary}
        </button>
        <button type="button" className={btnClass('hwlib__btn-secondary')} onClick={onSecondaryAction}>
          {labels.secondary}
        </button>
        {isMaterial && (
          <button type="button" className={btnClass('hwlib__btn-secondary')} onClick={onOpenAsDocument}>
            Open as document
          </button>
        )}
      </div>

      <div className="hwlib__manage-row">
        {canReveal && (
          <button type="button" className={btnClass('hwlib__btn-secondary')} onClick={onReveal}>
            {revealLabel}
          </button>
        )}
        <button type="button" className={btnClass('hwlib__btn-danger')} onClick={onRequestDelete}>
          Delete from library…
        </button>
      </div>
      {deleteArmed && <DeleteConfirm onConfirm={onConfirmDelete} onCancel={onCancelDelete} name={item.displayName} />}
    </div>
  )
}

function DeleteConfirm({ onConfirm, onCancel, name }: { onConfirm: () => void; onCancel: () => void; name: string }) {
  return (
    <div className="hwlib__delete-confirm" role="alertdialog" aria-label={`Delete ${name} from the library?`}>
      <p>Delete &ldquo;{name}&rdquo; from the library? This can&rsquo;t be undone.</p>
      <div className="hwlib__delete-confirm-actions">
        <button type="button" className="hwlib__btn-secondary" onClick={onCancel}>
          Cancel
        </button>
        <button type="button" className="hwlib__btn-danger" onClick={onConfirm} autoFocus>
          Delete
        </button>
      </div>
    </div>
  )
}

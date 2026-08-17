/**
 * ScenesPanel — the fifth tray section (docs/design/design-spec/scenes/
 * design_handoff_scenes/SPEC.md §1). Drives ONLY the `ScenesController`
 * (`scenes/useScenesController.ts`) passed in as a prop; never touches the
 * wasm handle directly. Renders as the body of a `TraySection` with the
 * header's ⊕ Add Scene icon button passed through `headerRight` — App.tsx
 * owns the collapsed/expanded flag and the `TraySection` wrapper itself,
 * matching every other tray section (Tags, Materials, …).
 *
 * Row idiom borrows from `TagsPanel.tsx` (eye/× buttons, per-row hover
 * state) and `LibraryDialog.tsx` (inline rename: autofocused + selected
 * input, Enter commits, Esc/blur reverts, duplicate error surfaced in
 * place). Exactly one Scene's details may be expanded at a time (SPEC.md
 * §1 decision 3).
 */

import { useEffect, useRef, useState } from 'react'
import {
  PROP_BIT,
  PROP_LABEL,
  SCENE_COPY,
  SCENE_PROPS,
  hasProp,
  nextSceneName,
  sceneIndex,
  type SceneEntry,
} from '../scenes/scenesModel'
import type { ScenesController } from '../scenes/useScenesController'

export interface ScenesPanelProps {
  scenes: ScenesController
  rename: SceneRenameState
}

const ROW_HEIGHT = 30
const HOVER_BG = 'rgba(255,255,255,0.04)'

/**
 * Inline-rename UI state (SPEC.md §1 "Name" + "Add flow"), shared between
 * the tray section's header ⊕ Add Scene button and its body: `TraySection`
 * renders the header (via `headerRight`) as a SIBLING of the collapsible
 * body, so the Add button and the row list are two separate React trees
 * that both need to read/drive the same "which Scene is being renamed"
 * state — hence a hook App.tsx calls once and threads to both, rather than
 * local state owned by either half alone.
 */
export interface SceneRenameState {
  editingSid: number | null
  editingText: string
  editingError: string | null
  inputRef: React.RefObject<HTMLInputElement | null>
  /** Add Scene: capture, auto-name, activate (`scenes.add`), then open this
   *  row's name for editing with the auto-name selected. */
  add: () => void
  start: (entry: SceneEntry) => void
  change: (text: string) => void
  commit: () => void
  cancel: () => void
}

export function useSceneRenameState(scenes: ScenesController): SceneRenameState {
  const [editingSid, setEditingSid] = useState<number | null>(null)
  const [editingText, setEditingText] = useState('')
  const [editingError, setEditingError] = useState<string | null>(null)
  const inputRef = useRef<HTMLInputElement>(null)

  useEffect(() => {
    if (editingSid === null) return
    inputRef.current?.focus()
    inputRef.current?.select()
  }, [editingSid])

  const start = (entry: SceneEntry) => {
    setEditingSid(entry.sid)
    setEditingText(entry.name)
    setEditingError(null)
  }

  const cancel = () => {
    setEditingSid(null)
    setEditingError(null)
  }

  const commit = () => {
    if (editingSid === null) return
    const err = scenes.rename(editingSid, editingText)
    if (err !== null) {
      setEditingError(err)
      return
    }
    setEditingSid(null)
    setEditingError(null)
  }

  const add = () => {
    // Predicted deterministically (mirrors the kernel's own `next_scene_name`
    // — `nextSceneName`'s own doc comment) so the rename input can open with
    // the right text selected in the SAME tick `add()` returns, rather than
    // waiting a render for `entries` to catch up.
    const predictedName = nextSceneName(scenes.entries)
    const sid = scenes.add()
    if (sid === null) return
    setEditingSid(sid)
    setEditingText(predictedName)
    setEditingError(null)
  }

  return { editingSid, editingText, editingError, inputRef, add, start, change: setEditingText, commit, cancel }
}

/** The Add Scene header icon button — rendered by App.tsx as `TraySection`'s
 *  `headerRight`, alongside (not inside) the collapse-toggle button. */
export function ScenesAddButton({ rename }: { rename: SceneRenameState }) {
  const [hovered, setHovered] = useState(false)
  return (
    <button
      type="button"
      aria-label="Add Scene"
      title="Add Scene"
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
      onClick={(e) => {
        e.stopPropagation()
        rename.add()
      }}
      style={{
        width: 22,
        height: 22,
        borderRadius: 6,
        border: 'none',
        background: hovered ? HOVER_BG : 'transparent',
        color: 'var(--text-secondary)',
        cursor: 'pointer',
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        fontSize: 15,
        lineHeight: 1,
        padding: 0,
      }}
    >
      ⊕
    </button>
  )
}

export function ScenesPanel({ scenes, rename }: ScenesPanelProps) {
  const { entries, activeSid, drift, activeDrifted, thumbnails } = scenes
  const { editingSid, editingText, editingError, inputRef: renameInputRef } = rename

  const [expandedSid, setExpandedSid] = useState<number | null>(null)
  const [confirmDeleteSid, setConfirmDeleteSid] = useState<number | null>(null)
  const [descDraft, setDescDraft] = useState('')

  // Seed the description textarea's draft whenever a DIFFERENT Scene's
  // details expand — not on every `entries` change (an in-progress edit
  // must never be stomped by an unrelated docRev bump).
  const expandedForDescRef = useRef<number | null>(null)
  useEffect(() => {
    if (expandedSid !== expandedForDescRef.current) {
      expandedForDescRef.current = expandedSid
      const entry = expandedSid === null ? undefined : entries.find((e) => e.sid === expandedSid)
      setDescDraft(entry?.description ?? '')
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [expandedSid])

  // The Scene the delete confirmation targets vanished from under it
  // (deleted elsewhere, or the document changed) — dismiss rather than
  // confirm against a stale sid.
  useEffect(() => {
    if (confirmDeleteSid !== null && sceneIndex(entries, confirmDeleteSid) < 0) {
      setConfirmDeleteSid(null)
    }
  }, [entries, confirmDeleteSid])

  const confirmDeleteEntry = confirmDeleteSid === null ? null : entries.find((e) => e.sid === confirmDeleteSid) ?? null

  if (entries.length === 0) {
    return (
      <div style={{ padding: '14px 11px', display: 'flex', flexDirection: 'column', gap: '10px' }}>
        <p style={{ margin: 0, fontSize: 12, color: 'var(--text-muted)', lineHeight: 1.4 }}>
          {SCENE_COPY.emptyState}
        </p>
        <button
          type="button"
          onClick={rename.add}
          style={{
            height: 26,
            padding: '0 12px',
            alignSelf: 'flex-start',
            borderRadius: 7,
            border: '1px solid var(--border-panel)',
            background: 'transparent',
            color: 'var(--text-primary)',
            fontSize: 12,
            fontFamily: 'var(--font-family-ui)',
            cursor: 'pointer',
          }}
        >
          Add Scene
        </button>
      </div>
    )
  }

  return (
    <div style={{ display: 'flex', flexDirection: 'column' }}>
      {entries.map((entry, index) => {
        const isActive = entry.sid === activeSid
        const isDrifted = isActive && activeDrifted
        const isExpanded = expandedSid === entry.sid
        const isEditing = editingSid === entry.sid
        return (
          <SceneRow
            key={entry.sid}
            entry={entry}
            index={index}
            count={entries.length}
            isActive={isActive}
            isDrifted={isDrifted}
            isExpanded={isExpanded}
            isEditing={isEditing}
            editingText={editingText}
            editingError={editingError}
            renameInputRef={renameInputRef}
            thumbnail={thumbnails.get(entry.sid)}
            drift={isActive ? drift : null}
            descDraft={isExpanded ? descDraft : ''}
            onSetDescDraft={setDescDraft}
            onActivate={() => scenes.activate(entry.sid)}
            onUpdate={() => scenes.update(entry.sid)}
            onToggleExpand={() => setExpandedSid((cur) => (cur === entry.sid ? null : entry.sid))}
            onStartRename={() => rename.start(entry)}
            onEditingTextChange={rename.change}
            onCommitRename={rename.commit}
            onCancelRename={rename.cancel}
            onCommitDescription={() => scenes.setDescription(entry.sid, descDraft)}
            onToggleProp={(prop) => {
              const bit = PROP_BIT[prop]
              const next = hasProp(entry, prop) ? entry.props & ~bit : entry.props | bit
              scenes.setProps(entry.sid, next)
            }}
            onMoveUp={() => scenes.moveUp(entry.sid)}
            onMoveDown={() => scenes.moveDown(entry.sid)}
            onRefreshThumbnail={() => scenes.refreshThumbnail(entry.sid)}
            onRequestDelete={() => setConfirmDeleteSid(entry.sid)}
          />
        )
      })}
      {confirmDeleteEntry !== null && (
        <SceneDeleteDialog
          name={confirmDeleteEntry.name}
          onCancel={() => setConfirmDeleteSid(null)}
          onConfirm={() => {
            const sid = confirmDeleteEntry.sid
            scenes.remove(sid)
            setConfirmDeleteSid(null)
            setExpandedSid((cur) => (cur === sid ? null : cur))
            if (editingSid === sid) rename.cancel()
          }}
        />
      )}
    </div>
  )
}

// ---------------------------------------------------------------------------
// SceneRow
// ---------------------------------------------------------------------------

interface SceneRowProps {
  entry: SceneEntry
  index: number
  count: number
  isActive: boolean
  isDrifted: boolean
  isExpanded: boolean
  isEditing: boolean
  editingText: string
  editingError: string | null
  renameInputRef: React.RefObject<HTMLInputElement | null>
  thumbnail: string | undefined
  drift: import('../scenes/scenesModel').SceneDrift | null
  descDraft: string
  onSetDescDraft: (text: string) => void
  onActivate: () => void
  onUpdate: () => void
  onToggleExpand: () => void
  onStartRename: () => void
  onEditingTextChange: (text: string) => void
  onCommitRename: () => void
  onCancelRename: () => void
  onCommitDescription: () => void
  onToggleProp: (prop: (typeof SCENE_PROPS)[number]) => void
  onMoveUp: () => void
  onMoveDown: () => void
  onRefreshThumbnail: () => void
  onRequestDelete: () => void
}

function SceneRow({
  entry,
  index,
  count,
  isActive,
  isDrifted,
  isExpanded,
  isEditing,
  editingText,
  editingError,
  renameInputRef,
  thumbnail,
  drift,
  descDraft,
  onSetDescDraft,
  onActivate,
  onUpdate,
  onToggleExpand,
  onStartRename,
  onEditingTextChange,
  onCommitRename,
  onCancelRename,
  onCommitDescription,
  onToggleProp,
  onMoveUp,
  onMoveDown,
  onRefreshThumbnail,
  onRequestDelete,
}: SceneRowProps) {
  const [hovered, setHovered] = useState(false)
  const [chevronHovered, setChevronHovered] = useState(false)
  const [updateHovered, setUpdateHovered] = useState(false)

  const rowBg = isActive ? 'var(--accent-tint-15)' : hovered ? HOVER_BG : 'transparent'

  return (
    <div style={{ display: 'flex', flexDirection: 'column' }}>
      <div
        data-testid="scene-row"
        data-sid={entry.sid}
        data-active={isActive ? 'true' : 'false'}
        data-drifted={isDrifted ? 'true' : 'false'}
        aria-current={isActive ? 'true' : undefined}
        role="button"
        tabIndex={isEditing ? -1 : 0}
        aria-label={`Scene ${entry.name}`}
        onKeyDown={(e) => {
          if (isEditing) return
          if (e.key === 'Enter' || e.key === ' ') {
            e.preventDefault()
            onActivate()
          } else if (e.key === 'F2') {
            e.preventDefault()
            onStartRename()
          }
        }}
        onMouseEnter={() => setHovered(true)}
        onMouseLeave={() => setHovered(false)}
        onClick={() => {
          if (!isEditing) onActivate()
        }}
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 8,
          height: ROW_HEIGHT,
          padding: '0 8px',
          margin: '0 3px',
          borderRadius: 9,
          background: rowBg,
          cursor: 'default',
        }}
      >
        {/* Thumbnail: 24x17 r4, 1px border-panel; camera glyph placeholder. */}
        <div
          style={{
            width: 24,
            height: 17,
            borderRadius: 4,
            border: '1px solid var(--border-panel)',
            flexShrink: 0,
            overflow: 'hidden',
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            backgroundImage: thumbnail !== undefined ? `url(${thumbnail})` : undefined,
            backgroundSize: 'cover',
            backgroundPosition: 'center',
          }}
        >
          {thumbnail === undefined && (
            <span aria-hidden="true" style={{ fontSize: 9, color: 'var(--text-muted)', opacity: 0.6 }}>
              ⛶
            </span>
          )}
        </div>

        {/* Name / inline rename */}
        {isEditing ? (
          <input
            ref={renameInputRef}
            value={editingText}
            aria-label="Scene name"
            onChange={(e) => onEditingTextChange(e.target.value)}
            onClick={(e) => e.stopPropagation()}
            onBlur={onCancelRename}
            onKeyDown={(e) => {
              if (e.key === 'Enter') {
                e.preventDefault()
                onCommitRename()
              } else if (e.key === 'Escape') {
                e.preventDefault()
                e.stopPropagation()
                onCancelRename()
              }
            }}
            style={{
              flex: 1,
              minWidth: 0,
              fontSize: 13,
              fontFamily: 'var(--font-family-ui)',
              color: 'var(--text-primary)',
              background: 'var(--surface-input)',
              border: '1px solid var(--accent-border)',
              borderRadius: 4,
              padding: '1px 4px',
            }}
          />
        ) : (
          <span
            onDoubleClick={(e) => {
              e.stopPropagation()
              onStartRename()
            }}
            style={{
              flex: 1,
              minWidth: 0,
              overflow: 'hidden',
              textOverflow: 'ellipsis',
              whiteSpace: 'nowrap',
              fontSize: 13,
              fontWeight: isActive ? 600 : 400,
              color: 'var(--text-primary)',
            }}
          >
            {entry.name}
          </span>
        )}

        {/* State dot: filled when active; hollow ring when active + drifted. */}
        <span
          aria-hidden="true"
          style={{
            width: 7,
            height: 7,
            borderRadius: '50%',
            flexShrink: 0,
            background: isActive && !isDrifted ? 'var(--accent-base)' : 'transparent',
            border: isDrifted ? '1.5px solid var(--accent-base)' : 'none',
            boxSizing: 'border-box',
          }}
        />

        {/* Update — drifted only. */}
        {isDrifted && (
          <button
            type="button"
            aria-label={SCENE_COPY.updateTooltip}
            title={SCENE_COPY.updateTooltip}
            onMouseEnter={() => setUpdateHovered(true)}
            onMouseLeave={() => setUpdateHovered(false)}
            onClick={(e) => {
              e.stopPropagation()
              onUpdate()
            }}
            style={{
              flexShrink: 0,
              width: 18,
              height: 18,
              borderRadius: 4,
              border: 'none',
              background: updateHovered ? HOVER_BG : 'transparent',
              color: 'var(--accent-base)',
              fontSize: 12,
              lineHeight: 1,
              cursor: 'pointer',
              display: 'flex',
              alignItems: 'center',
              justifyContent: 'center',
              padding: 0,
            }}
          >
            ↻
          </button>
        )}

        {/* Disclosure chevron — a real stroked caret (playtest: the 10px
            glyph at 35% opacity read as a dot, not a control). Points right
            when collapsed, rotates down when expanded; 60% idle, full on
            hover/expanded. */}
        <button
          type="button"
          aria-label={isExpanded ? 'Collapse Scene details' : 'Expand Scene details'}
          aria-expanded={isExpanded}
          onMouseEnter={() => setChevronHovered(true)}
          onMouseLeave={() => setChevronHovered(false)}
          onClick={(e) => {
            e.stopPropagation()
            onToggleExpand()
          }}
          style={{
            flexShrink: 0,
            width: 20,
            height: 20,
            border: 'none',
            borderRadius: 5,
            background: chevronHovered ? HOVER_BG : 'transparent',
            color: 'var(--text-secondary)',
            cursor: 'pointer',
            opacity: isExpanded || chevronHovered || hovered ? 1 : 0.6,
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            padding: 0,
          }}
        >
          <svg
            width="12"
            height="12"
            viewBox="0 0 12 12"
            aria-hidden="true"
            style={{
              transform: isExpanded ? 'rotate(90deg)' : 'none',
              transition: 'transform 120ms ease-out',
            }}
          >
            <path d="M4 2.5 L8 6 L4 9.5" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" />
          </svg>
        </button>
      </div>

      {isEditing && editingError !== null && (
        <div
          style={{
            padding: '2px 11px 4px',
            fontSize: 11.5,
            color: 'var(--scene-delete-text)',
          }}
        >
          {editingError}
        </div>
      )}

      {isExpanded && (
        <SceneDetails
          entry={entry}
          index={index}
          count={count}
          descDraft={descDraft}
          onSetDescDraft={onSetDescDraft}
          onCommitDescription={onCommitDescription}
          onToggleProp={onToggleProp}
          staleRefs={isActive ? drift?.staleRefs ?? 0 : 0}
          onMoveUp={onMoveUp}
          onMoveDown={onMoveDown}
          onRefreshThumbnail={onRefreshThumbnail}
          onRequestDelete={onRequestDelete}
        />
      )}
    </div>
  )
}

// ---------------------------------------------------------------------------
// SceneDetails — inline expansion under a row.
// ---------------------------------------------------------------------------

function SceneDetails({
  entry,
  index,
  count,
  descDraft,
  onSetDescDraft,
  onCommitDescription,
  onToggleProp,
  staleRefs,
  onMoveUp,
  onMoveDown,
  onRefreshThumbnail,
  onRequestDelete,
}: {
  entry: SceneEntry
  index: number
  count: number
  descDraft: string
  onSetDescDraft: (text: string) => void
  onCommitDescription: () => void
  onToggleProp: (prop: (typeof SCENE_PROPS)[number]) => void
  staleRefs: number
  onMoveUp: () => void
  onMoveDown: () => void
  onRefreshThumbnail: () => void
  onRequestDelete: () => void
}) {
  return (
    <div
      style={{
        padding: '8px 11px 13px',
        margin: '0 3px',
        borderTop: '1px solid var(--border-panel)',
        display: 'flex',
        flexDirection: 'column',
        gap: 8,
      }}
    >
      <textarea
        value={descDraft}
        placeholder={SCENE_COPY.descriptionPlaceholder}
        aria-label="Scene description"
        onChange={(e) => onSetDescDraft(e.target.value)}
        onBlur={onCommitDescription}
        rows={2}
        style={{
          resize: 'vertical',
          border: 'none',
          background: 'transparent',
          color: 'var(--text-secondary)',
          fontSize: 12,
          fontFamily: 'var(--font-family-ui)',
          padding: 0,
          outline: 'none',
        }}
      />

      <div>
        <div
          style={{
            fontSize: 9.5,
            textTransform: 'uppercase',
            letterSpacing: '0.08em',
            color: 'var(--text-muted)',
            marginBottom: 2,
          }}
        >
          Captured Properties
        </div>
        {SCENE_PROPS.map((prop) => (
          <label
            key={prop}
            style={{
              display: 'flex',
              alignItems: 'center',
              gap: 8,
              height: 24,
              fontSize: 12,
              color: 'var(--text-secondary)',
              cursor: 'pointer',
            }}
          >
            <input type="checkbox" checked={hasProp(entry, prop)} onChange={() => onToggleProp(prop)} />
            {PROP_LABEL[prop]}
          </label>
        ))}
      </div>

      {staleRefs > 0 && (
        <div style={{ fontSize: 11.5, color: 'var(--scene-stale-text)' }}>{SCENE_COPY.staleRefs(staleRefs)}</div>
      )}

      <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
        <DetailButton ariaLabel="Move Up" disabled={index === 0} onClick={onMoveUp}>
          ↑
        </DetailButton>
        <DetailButton ariaLabel="Move Down" disabled={index === count - 1} onClick={onMoveDown}>
          ↓
        </DetailButton>
        <div style={{ flex: 1 }} />
        <DetailButton ariaLabel="Refresh thumbnail" onClick={onRefreshThumbnail}>
          Refresh thumbnail
        </DetailButton>
        <DetailButton ariaLabel="Delete" onClick={onRequestDelete} colorVar="--scene-delete-text">
          Delete
        </DetailButton>
      </div>
    </div>
  )
}

function DetailButton({
  ariaLabel,
  onClick,
  disabled,
  colorVar,
  children,
}: {
  ariaLabel: string
  onClick: () => void
  disabled?: boolean
  colorVar?: string
  children: React.ReactNode
}) {
  const [hovered, setHovered] = useState(false)
  return (
    <button
      type="button"
      aria-label={ariaLabel}
      disabled={disabled === true}
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
      onClick={(e) => {
        e.stopPropagation()
        onClick()
      }}
      style={{
        height: 24,
        padding: '0 8px',
        borderRadius: 7,
        border: '1px solid var(--border-panel)',
        background: hovered && disabled !== true ? HOVER_BG : 'transparent',
        color: colorVar !== undefined ? `var(${colorVar})` : 'var(--text-secondary)',
        fontSize: 12,
        fontFamily: 'var(--font-family-ui)',
        whiteSpace: 'nowrap',
        cursor: disabled === true ? 'default' : 'pointer',
        opacity: disabled === true ? 0.3 : 1,
      }}
    >
      {children}
    </button>
  )
}

// ---------------------------------------------------------------------------
// SceneDeleteDialog — SPEC.md §1 "Delete confirmation".
// ---------------------------------------------------------------------------

function SceneDeleteDialog({
  name,
  onCancel,
  onConfirm,
}: {
  name: string
  onCancel: () => void
  onConfirm: () => void
}) {
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault()
        e.stopPropagation()
        onCancel()
      } else if (e.key === 'Enter') {
        e.preventDefault()
        e.stopPropagation()
        onCancel()
      } else if (e.key === 'Delete' && (e.metaKey || e.ctrlKey)) {
        e.preventDefault()
        e.stopPropagation()
        onConfirm()
      }
    }
    document.addEventListener('keydown', onKeyDown)
    return () => document.removeEventListener('keydown', onKeyDown)
  }, [onCancel, onConfirm])

  return (
    <div
      onClick={onCancel}
      style={{
        position: 'fixed',
        inset: 0,
        background: 'var(--backdrop-dim)',
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        zIndex: 2000,
      }}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-label={SCENE_COPY.deleteTitle(name)}
        onClick={(e) => e.stopPropagation()}
        style={{
          width: 300,
          borderRadius: 14,
          background: 'var(--surface-overlay)',
          border: '1px solid var(--border-strong)',
          boxShadow: 'var(--shadow-palette)',
          padding: '18px 20px',
          fontFamily: 'var(--font-family-ui)',
        }}
      >
        <div style={{ fontSize: 13, fontWeight: 600, color: 'var(--text-primary)', marginBottom: 8 }}>
          {SCENE_COPY.deleteTitle(name)}
        </div>
        <div style={{ fontSize: 12, color: 'var(--text-muted)', marginBottom: 16, lineHeight: 1.4 }}>
          {SCENE_COPY.deleteBody}
        </div>
        <div style={{ display: 'flex', justifyContent: 'flex-end', gap: 10 }}>
          <button
            type="button"
            autoFocus
            onClick={onCancel}
            style={{
              padding: '6px 16px',
              borderRadius: 7,
              border: '1px solid var(--border-panel)',
              background: 'transparent',
              color: 'var(--text-primary)',
              fontSize: 12.5,
              fontFamily: 'var(--font-family-ui)',
              cursor: 'pointer',
            }}
          >
            Cancel
          </button>
          <button
            type="button"
            onClick={onConfirm}
            style={{
              padding: '6px 16px',
              borderRadius: 7,
              border: 'none',
              background: 'var(--scene-delete-fill)',
              color: '#ffffff',
              fontSize: 12.5,
              fontFamily: 'var(--font-family-ui)',
              cursor: 'pointer',
            }}
          >
            Delete
          </button>
        </div>
      </div>
    </div>
  )
}

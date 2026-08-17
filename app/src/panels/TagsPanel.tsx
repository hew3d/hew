/**
 * TagsPanel — a closeable pane listing the nestable tag tree built from
 * @@HEWTAG@@-encoded node names (see tools/sketchup/hew_export_tags.rb).
 *
 * Each tag row has a show/hide eye toggle that hides every object/instance
 * tagged at or under that path. Hides compose with manual per-node hides
 * (DocumentTree eye toggles) — a node hidden by either stays hidden; unhiding
 * a tag does not un-hide a node that is also manually hidden.
 *
 * Each row also has a delete (×) button: deleting a tag unassigns it (and
 * its sub-tags) from every node and drops it from the registry — geometry is
 * never deleted, and the action is undoable (Scene.delete_tag).
 *
 * If the model has no encoded tags, the panel renders nothing — an empty
 * list is self-explanatory and boilerplate would just be noise.
 */

import { useEffect, useMemo, useRef, useState } from 'react'
import type { Scene as WasmScene } from '../wasm/loader'
import { buildTagTree, collectTagDescendantNodes, tagPathKey, type TagTreeNode } from './tagModel'
import { nodeRefFromJs, nodeKindToNumber, type NodeRef } from './treeModel'

/** A palette "jump to tag" request: which row to reveal (expand ancestors,
 * scroll into view, briefly highlight). `nonce` distinguishes repeat jumps
 * to the same tag so the scroll/flash re-fires. */
export interface TagReveal {
  key: string
  nonce: number
}

interface Props {
  scene: WasmScene
  /** Bumped on any document change to trigger a re-query. */
  docRev: number
  /** The set of tag path keys (JSON-serialised path arrays) currently hidden. */
  hiddenTagPaths: Set<string>
  /** Toggle hide/show for a tag (and all its descendants). */
  onToggleTagPath: (path: string[]) => void
  /** Delete a tag (and all its descendants): unassign everywhere, undoable. */
  onDeleteTag: (path: string[]) => void
  /** Reveal request from the command palette (null = none). */
  revealTag?: TagReveal | null
  /** Click a tag row → select every item carrying it (or a nested tag),
   *  the way clicking an Outliner row selects that object. Replaces the
   *  selection. */
  onSelectNodes?: (nodes: NodeRef[]) => void
  /** Rename a tag in place (its LAST segment; parents stay): a second
   *  click on the already-active row, or a double-click, opens the editor
   *  — Finder/Explorer style. Returns inline error text, or null on
   *  success (the kernel refuses a duplicate or empty name). */
  onRenameTag?: (path: string[], newSegment: string) => string | null
}

/** A second click on the ACTIVE row later than this opens rename (Finder's
 *  "click twice" — slower than a double-click, which the browser reports as
 *  `dblclick` and which also opens rename). */
export const RENAME_CLICK_TWICE_MIN_MS = 350

const ROW_BASE: React.CSSProperties = {
  display: 'flex',
  alignItems: 'center',
  gap: '6px',
  padding: '3px 8px',
  fontSize: '12px',
  fontFamily: 'var(--font-family-ui)',
  cursor: 'default',
  borderRadius: '3px',
  userSelect: 'none',
  minWidth: 0,
  color: 'var(--text-secondary, #ccc)',
}

export function TagsPanel({ scene, docRev, hiddenTagPaths, onToggleTagPath, onDeleteTag, revealTag, onSelectNodes, onRenameTag }: Props) {
  // The row last clicked (highlighted); a later click on it opens rename.
  const [activeKey, setActiveKey] = useState<string | null>(null)
  const [editingKey, setEditingKey] = useState<string | null>(null)
  const [editingText, setEditingText] = useState('')
  const [editingError, setEditingError] = useState<string | null>(null)
  const lastClickRef = useRef<{ key: string; at: number } | null>(null)

  const startRename = (node: TagTreeNode) => {
    if (onRenameTag === undefined) return
    setEditingKey(tagPathKey(node.path))
    setEditingText(node.segment)
    setEditingError(null)
  }
  const cancelRename = () => {
    setEditingKey(null)
    setEditingText('')
    setEditingError(null)
  }
  const commitRename = (node: TagTreeNode) => {
    if (onRenameTag === undefined) return
    const next = editingText.trim()
    if (next.length === 0 || next === node.segment) {
      cancelRename()
      return
    }
    const err = onRenameTag(node.path, next)
    if (err !== null) {
      setEditingError(err)
      return
    }
    // The renamed row re-keys; keep it active under its new path.
    setActiveKey(tagPathKey([...node.path.slice(0, -1), next]))
    cancelRename()
  }
  const clickRow = (node: TagTreeNode) => {
    const key = tagPathKey(node.path)
    const now = Date.now()
    const last = lastClickRef.current
    lastClickRef.current = { key, at: now }
    if (activeKey === key && last !== null && last.key === key && now - last.at >= RENAME_CLICK_TWICE_MIN_MS) {
      startRename(node)
      return
    }
    setActiveKey(key)
    onSelectNodes?.(collectTagDescendantNodes(node))
  }

  // Re-query the scene on every docRev bump.
  const tagTree = useMemo(() => {
    // Collect all nodes (objects, groups, instances) and parse their names.
    const allNodes = [
      ...Array.from(scene.object_ids()).map((id) => ({ kind: 'object' as const, id })),
      ...Array.from(scene.group_ids()).map((id) => ({ kind: 'group' as const, id })),
      ...Array.from(scene.instance_ids()).map((id) => ({ kind: 'instance' as const, id })),
    ]

    const tagged: { node: ReturnType<typeof nodeRefFromJs>; path: string[] }[] = []
    for (const raw of allNodes) {
      const node = nodeRefFromJs(raw)
      const kindNum = nodeKindToNumber(node.kind)
      // node_tags returns each tag path joined with "/"; split to recover segments.
      const rawTags = scene.node_tags(kindNum, node.id)
      for (const rawTag of rawTags) {
        const path = rawTag.split('/').map((s) => s.trim()).filter((s) => s.length > 0)
        if (path.length > 0) {
          tagged.push({ node, path })
        }
      }
    }

    // Union in every KNOWN tag path from the document's tag registry — this
    // includes tags no node currently carries (e.g. an imported .skp layer
    // list survives in full even for empty layers).
    const registryPaths = Array.from(scene.tag_meta_paths())
      .map((raw) => raw.split('/').map((s) => s.trim()).filter((s) => s.length > 0))
      .filter((path) => path.length > 0)

    return buildTagTree(tagged, registryPaths)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [scene, docRev])

  // Ancestor keys (every prefix, including the full path) of the reveal
  // target, so collapsed parents pop open on a palette jump.
  const revealExpandKeys = useMemo(() => {
    const keys = new Set<string>()
    if (revealTag == null) return keys
    try {
      const path = JSON.parse(revealTag.key) as unknown
      if (!Array.isArray(path)) return keys
      for (let len = 1; len <= path.length; len++) {
        keys.add(tagPathKey((path as string[]).slice(0, len)))
      }
    } catch {
      /* malformed key — no expansion */
    }
    return keys
  }, [revealTag])

  // No tags → render nothing (no boilerplate empty state).
  if (tagTree.length === 0) return null

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: '8px' }}>
      <div style={{ display: 'flex', flexDirection: 'column', gap: '1px' }}>
        {tagTree.map((node) => (
          <TagRow
            key={tagPathKey(node.path)}
            node={node}
            depth={0}
            hiddenTagPaths={hiddenTagPaths}
            onToggleTagPath={onToggleTagPath}
            onDeleteTag={onDeleteTag}
            revealTag={revealTag ?? null}
            revealExpandKeys={revealExpandKeys}
            activeKey={activeKey}
            editingKey={editingKey}
            editingText={editingText}
            editingError={editingError}
            onClickRow={clickRow}
            onStartRename={startRename}
            onEditingTextChange={setEditingText}
            onCommitRename={commitRename}
            onCancelRename={cancelRename}
            canRename={onRenameTag !== undefined}
          />
        ))}
      </div>
    </div>
  )
}

// ---------------------------------------------------------------------------
// TagRow — recursive tree row for a tag node
// ---------------------------------------------------------------------------

interface RowInteraction {
  activeKey: string | null
  editingKey: string | null
  editingText: string
  editingError: string | null
  onClickRow: (node: TagTreeNode) => void
  onStartRename: (node: TagTreeNode) => void
  onEditingTextChange: (text: string) => void
  onCommitRename: (node: TagTreeNode) => void
  onCancelRename: () => void
  canRename: boolean
}

function TagRow({
  node,
  depth,
  hiddenTagPaths,
  onToggleTagPath,
  onDeleteTag,
  revealTag,
  revealExpandKeys,
  ...rowInteraction
}: {
  node: TagTreeNode
  depth: number
  hiddenTagPaths: Set<string>
  onToggleTagPath: (path: string[]) => void
  onDeleteTag: (path: string[]) => void
  revealTag: TagReveal | null
  revealExpandKeys: Set<string>
} & RowInteraction) {
  const { activeKey, editingKey, editingText, editingError, onClickRow, onStartRename, onEditingTextChange, onCommitRename, onCancelRename, canRename } = rowInteraction
  const [expanded, setExpanded] = useState(true)
  const hasChildren = node.children.length > 0
  const key = tagPathKey(node.path)
  const hidden = hiddenTagPaths.has(key)
  const isActive = activeKey === key
  const isEditing = editingKey === key

  // Palette jump: pop open when on the reveal path, and scroll the revealed
  // row itself into view. Keyed on the nonce so a repeat jump re-fires.
  const isRevealed = revealTag !== null && revealTag.key === key
  const onRevealPath = revealTag !== null && revealExpandKeys.has(key)
  const rowRef = useRef<HTMLDivElement>(null)
  const revealNonce = revealTag?.nonce
  useEffect(() => {
    if (onRevealPath) setExpanded(true)
  }, [onRevealPath, revealNonce])
  useEffect(() => {
    if (isRevealed) rowRef.current?.scrollIntoView?.({ block: 'nearest' })
  }, [isRevealed, revealNonce])

  // Compute the count of directly-tagged nodes (not counting descendants).
  const directCount = node.nodes.length

  // A tag row is visually dimmed when it (or an ancestor) is hidden.
  // We check both the exact path and all ancestor paths.
  const isHiddenByAncestorOrSelf = isHiddenByAny(node.path, hiddenTagPaths)

  return (
    <>
      <div
        ref={rowRef}
        data-testid="tag-row"
        data-active={isActive ? 'true' : 'false'}
        aria-current={isActive ? 'true' : undefined}
        onClick={() => {
          if (!isEditing) onClickRow(node)
        }}
        onDoubleClick={() => {
          if (!isEditing && canRename) onStartRename(node)
        }}
        style={{
          ...ROW_BASE,
          paddingLeft: `${8 + depth * 16}px`,
          paddingRight: '4px',
          background: isRevealed || isActive ? 'var(--accent-tint-18)' : 'transparent',
        }}
      >
        {/* Expand/collapse button for tag folders with children */}
        {hasChildren ? (
          <button
            onClick={(e) => {
              e.stopPropagation()
              setExpanded((v) => !v)
            }}
            onDoubleClick={(e) => e.stopPropagation()}
            style={{
              background: 'none',
              border: 'none',
              color: 'var(--text-tertiary, #aaa)',
              cursor: 'pointer',
              padding: '0 2px',
              fontSize: '10px',
              lineHeight: 1,
            }}
          >
            {expanded ? '▾' : '▸'}
          </button>
        ) : (
          /* Spacer to keep text aligned when no expand button */
          <span style={{ width: '14px', flexShrink: 0 }} />
        )}

        {/* Tag folder/label icon */}
        <span style={{ fontSize: '11px', color: 'var(--tag-accent)', flexShrink: 0 }}>⬧</span>

        {/* Tag name — or, while renaming, the in-place editor (Enter commits,
            Escape cancels, blur commits a changed name / cancels an unchanged
            one; a kernel refusal shows below the row). */}
        {isEditing ? (
          <input
            autoFocus
            aria-label="Tag name"
            value={editingText}
            onChange={(e) => onEditingTextChange(e.target.value)}
            onFocus={(e) => e.currentTarget.select()}
            onClick={(e) => e.stopPropagation()}
            onDoubleClick={(e) => e.stopPropagation()}
            onKeyDown={(e) => {
              if (e.key === 'Enter') {
                e.preventDefault()
                onCommitRename(node)
              } else if (e.key === 'Escape') {
                e.stopPropagation()
                onCancelRename()
              }
            }}
            onBlur={() => onCommitRename(node)}
            spellCheck={false}
            style={{
              flex: 1,
              minWidth: 0,
              background: 'var(--surface-input, #111)',
              border: '1px solid var(--accent-border)',
              borderRadius: '3px',
              color: 'var(--text-primary, #eee)',
              fontFamily: 'var(--font-family-ui)',
              fontSize: '12px',
              padding: '1px 4px',
              outline: 'none',
            }}
          />
        ) : (
          <span
            style={{
              flex: 1,
              minWidth: 0,
              overflow: 'hidden',
              textOverflow: 'ellipsis',
              whiteSpace: 'nowrap',
              color: isHiddenByAncestorOrSelf ? 'var(--text-faint, #555)' : 'var(--text-secondary, #ccc)',
              fontSize: '12px',
            }}
          >
            {node.segment}
          </span>
        )}

        {/* Direct node count badge */}
        {directCount > 0 && (
          <span
            style={{
              fontSize: '10px',
              color: 'var(--text-faint, #666)',
              minWidth: '16px',
              textAlign: 'right',
              flexShrink: 0,
            }}
          >
            {directCount}
          </span>
        )}

        {/* Eye toggle */}
        <button
          onClick={(e) => {
            e.stopPropagation()
            onToggleTagPath(node.path)
          }}
          onDoubleClick={(e) => e.stopPropagation()}
          title={hidden ? 'Show tagged objects' : 'Hide tagged objects'}
          style={{
            background: 'none',
            border: 'none',
            color: hidden ? 'var(--text-faint, #555)' : 'var(--text-tertiary, #888)',
            cursor: 'pointer',
            padding: '0 2px',
            fontSize: '11px',
            lineHeight: 1,
            flexShrink: 0,
          }}
        >
          {hidden ? '○' : '●'}
        </button>

        {/* Delete tag — unassigns everywhere (undoable); never deletes geometry */}
        <button
          onClick={(e) => {
            e.stopPropagation()
            onDeleteTag(node.path)
          }}
          onDoubleClick={(e) => e.stopPropagation()}
          title={hasChildren ? 'Delete tag and sub-tags (objects are kept)' : 'Delete tag (objects are kept)'}
          aria-label={`Delete tag ${node.segment}`}
          style={{
            background: 'none',
            border: 'none',
            color: 'var(--text-faint, #666)',
            cursor: 'pointer',
            padding: '0 2px',
            fontSize: '12px',
            lineHeight: 1,
            flexShrink: 0,
          }}
        >
          ×
        </button>
      </div>
      {isEditing && editingError !== null && (
        <div style={{ padding: `2px 8px 4px ${8 + depth * 16 + 20}px`, fontSize: '11px', color: 'var(--scene-delete-text)' }}>
          {editingError}
        </div>
      )}

      {/* Children — shown when expanded */}
      {hasChildren && expanded && node.children.map((child) => (
        <TagRow
          key={tagPathKey(child.path)}
          node={child}
          depth={depth + 1}
          hiddenTagPaths={hiddenTagPaths}
          onToggleTagPath={onToggleTagPath}
          onDeleteTag={onDeleteTag}
          revealTag={revealTag}
          revealExpandKeys={revealExpandKeys}
          {...rowInteraction}
        />
      ))}
    </>
  )
}

/**
 * Return true if the given path is itself hidden OR if any ancestor path is
 * hidden (which also covers this tag's descendants in the renderer).
 */
function isHiddenByAny(path: string[], hiddenTagPaths: Set<string>): boolean {
  for (let len = 1; len <= path.length; len++) {
    if (hiddenTagPaths.has(tagPathKey(path.slice(0, len)))) return true
  }
  return false
}

/**
 * ObjectInfoPanel — a closeable pane showing info for the currently selected node.
 *
 * Displays the selected node's:
 *   - Name (editable — commits on Enter/blur → scene.set_node_name; empty
 *     clears the kernel name and the field falls back to showing the same
 *     default label the Outliner shows, via resolveLabel — never "(unnamed)")
 *   - Type (read-only: "Object", "Group", "Component", or "Sketch")
 *   - Solid / Leaky (only for Objects; calls scene.object_solid)
 *   - Tags (removable chips + a "+" affordance that reveals the add field)
 *
 * Empty states are quiet: nothing selected renders nothing at all; a
 * multi-selection shows only the count ("3 selected").
 *
 * StrictMode notes:
 *   - No impure setState updaters (all updaters are either functional or derive
 *     from explicit current state, not captured stale closures).
 *   - The useMemo for node info re-derives from scene + docRev on every doc change.
 *   - useEffect for name input sync derives only from the stable nameFromScene value
 *     computed inside the memo; the effect is idempotent (setting the same string
 *     twice is a no-op for the controlled input). StrictMode double-invocation is
 *     safe: the only mutation is `setLocalName(...)`.
 *   - No async listeners or cleanup races introduced here.
 */

import { useMemo, useState, useEffect, useRef, useCallback } from 'react'
import type { Scene as WasmScene } from '../wasm/loader'
import { entityLabel, resolveLabel, nodeKindToNumber, nodeKey, nodeRefFromJs, buildTreeIndexMap, type NodeRef } from './treeModel'

interface Props {
  scene: WasmScene
  /** Bumped on any document change to trigger re-query. */
  docRev: number
  /** The currently selected nodes (ordered; index 0 = primary). */
  selectedIds: NodeRef[]
  /**
   * Must be called after any mutation so the scene re-renders and other panels
   * update. This is the same handleDocumentChanged that all other mutations use.
   */
  onDocumentChanged: () => void
}

/** Human-readable type label for each node kind. */
function kindLabel(kind: NodeRef['kind']): string {
  if (kind === 'object') return 'Object'
  if (kind === 'group') return 'Group'
  if (kind === 'sketch' || kind === 'sketch-island') return 'Sketch'
  if (kind === 'sketch-curve') return 'Curve'
  if (kind === 'sketch-edge') return 'Sketch Line'
  return 'Component'
}

const PANEL_STYLE: React.CSSProperties = {
  display: 'flex',
  flexDirection: 'column',
  gap: '10px',
}

const LABEL_STYLE: React.CSSProperties = {
  fontSize: '10px',
  color: 'var(--text-faint, #888)',
  textTransform: 'uppercase',
  letterSpacing: '0.05em',
  marginBottom: '2px',
}

const VALUE_STYLE: React.CSSProperties = {
  fontSize: '12px',
  color: 'var(--text-secondary, #ccc)',
  fontFamily: 'var(--font-family-ui)',
}

const INPUT_STYLE: React.CSSProperties = {
  width: '100%',
  boxSizing: 'border-box',
  background: 'var(--surface-input, #1e1e1e)',
  border: '1px solid var(--border-strong, #444)',
  borderRadius: '3px',
  color: 'var(--text-primary, #eee)',
  fontFamily: 'var(--font-family-ui)',
  fontSize: '12px',
  padding: '3px 6px',
  outline: 'none',
}

export function ObjectInfoPanel({ scene, docRev, selectedIds, onDocumentChanged }: Props) {
  // --------------------------------------------------------------------------
  // Derive the node info from the scene whenever docRev or selectedIds changes.
  // --------------------------------------------------------------------------
  const nodeInfo = useMemo(() => {
    if (selectedIds.length !== 1) return null
    const node = selectedIds[0]
    const kind = node.kind
    const id = node.id

    // A sketch is a thin selectable (a drawn line) with no kernel NodeId, name,
    // tags, or solid state — show a minimal read-only entry and never call the
    // object/group/instance APIs (`nodeKindToNumber` yields the -1 sentinel,
    // which no node_id-keyed wasm call accepts). Its
    // "name" is the same positional label the tree/canvas show ("Sketch 2"),
    // derived from `sketch_ids()` order since sketches can't be renamed yet.
    if (kind !== 'object' && kind !== 'group' && kind !== 'instance') {
      const sketchId = node.sketch ?? id
      // Number by the ISLAND's position in the outliner's flattened
      // cross-sketch island list, so panel and tree agree ('Sketch 3' here
      // is 'Sketch 3' there). Resolve the owning island for sub-entities.
      let islandId: bigint | undefined
      if (kind === 'sketch-island') {
        islandId = id
      } else if (kind === 'sketch') {
        // Legacy whole-sketch ref: number by its first island's row.
        const islands = Array.from(scene.sketch_island_ids(sketchId))
        islandId = islands.length > 0 ? islands[0] : undefined
      } else if (kind === 'sketch-edge') {
        islandId = scene.sketch_edge_island(sketchId, id)
      } else if (kind === 'sketch-curve') {
        const edges = Array.from(scene.sketch_curve_edges(sketchId, id))
        islandId = edges.length > 0 ? scene.sketch_edge_island(sketchId, edges[0]) : undefined
      }
      const flat = Array.from(scene.sketch_ids()).flatMap((sid) =>
        Array.from(scene.sketch_island_ids(sid)).map((island) => ({ sid, island })),
      )
      const idx = flat.findIndex((f) => f.sid === sketchId && f.island === islandId)
      const sketchLabel = entityLabel('sketch', idx >= 0 ? idx : 0)
      return {
        node,
        kind,
        id,
        kindNum: null as number | null,
        nameFromScene: undefined as string | undefined,
        // Sub-entities have no identity of their own — label by the owner.
        defaultLabel:
          kind === 'sketch-edge'
            ? `Line of ${sketchLabel}`
            : kind === 'sketch-curve'
              ? `Curve of ${sketchLabel}`
              : sketchLabel,
        tags: [] as string[][],
        solid: null as boolean | null,
      }
    }

    const kindNum = nodeKindToNumber(kind)

    let nameFromScene: string | undefined
    let defName: string | undefined
    if (kind === 'object') {
      nameFromScene = scene.object_name(id)
    } else if (kind === 'group') {
      nameFromScene = scene.group_name(id)
    } else {
      nameFromScene = scene.instance_name(id)
      const def = scene.instance_def(id)
      defName = def !== undefined ? scene.component_name(def) : undefined
    }

    // The label the Outliner would show for this node when it carries no
    // kernel name — the panel falls back to exactly this, never "(unnamed)".
    // The positional index is the node's position within its parent container
    // (buildTreeIndexMap), NOT its position in the flat per-kind id list —
    // the Outliner numbers per container, and the two disagree once an
    // unnamed node sits inside a group.
    const treeIndex = buildTreeIndexMap(
      scene.top_level_nodes().map(nodeRefFromJs),
      (groupId) => scene.group_members(groupId).map(nodeRefFromJs),
    )
    const idx = treeIndex.get(nodeKey(node)) ?? 0
    const defaultLabel = resolveLabel(undefined, defName, kind, idx)

    // Tags: array of "Seg1/Seg2" strings from the kernel, split to string[][]
    const rawTags = scene.node_tags(kindNum, id)
    const tags: string[][] = rawTags.map((t) =>
      t.split('/').map((s) => s.trim()).filter((s) => s.length > 0)
    )

    // Solid/leaky only for objects
    let solid: boolean | null = null
    if (kind === 'object') {
      solid = scene.object_solid(id)
    }

    return { node, kind, kindNum, id, nameFromScene, defaultLabel, tags, solid }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [scene, docRev, selectedIds])

  // --------------------------------------------------------------------------
  // Local controlled state for the name input.
  // Sync with the kernel-derived name whenever nodeInfo changes (new selection
  // or doc mutation). This effect is idempotent — calling setLocalName with the
  // same value twice is harmless, so StrictMode double-invoke is safe.
  // --------------------------------------------------------------------------
  const [localName, setLocalName] = useState(nodeInfo?.nameFromScene ?? '')
  const prevNameFromSceneRef = useRef<string | undefined>(undefined)
  useEffect(() => {
    const next = nodeInfo?.nameFromScene
    if (next !== prevNameFromSceneRef.current) {
      prevNameFromSceneRef.current = next
      setLocalName(next ?? '')
    }
  }, [nodeInfo?.nameFromScene, nodeInfo?.node.id, nodeInfo?.node.kind])

  // --------------------------------------------------------------------------
  // Name commit: called on Enter or blur.
  // --------------------------------------------------------------------------
  const commitName = useCallback(() => {
    if (nodeInfo === null || nodeInfo.kindNum === null) return
    const trimmed = localName.trim()
    // Pass undefined to clear; pass the string to set. Clearing makes the
    // field fall back to the placeholder default label (Outliner parity).
    scene.set_node_name(nodeInfo.kindNum, nodeInfo.id, trimmed === '' ? undefined : trimmed)
    onDocumentChanged()
  }, [nodeInfo, localName, scene, onDocumentChanged])

  // --------------------------------------------------------------------------
  // Tag add state — hidden behind a "+" affordance (HIG-style disclosure).
  // The field auto-focuses when revealed; Enter commits, Esc cancels, blurring
  // with an empty field closes it again.
  // --------------------------------------------------------------------------
  const [addingTag, setAddingTag] = useState(false)
  const [tagInput, setTagInput] = useState('')

  // Close the add-tag field whenever the selected node changes.
  const selectedKeyForReset = nodeInfo !== null ? nodeKey(nodeInfo.node) : null
  const prevSelectedKeyRef = useRef<string | null>(selectedKeyForReset)
  useEffect(() => {
    if (prevSelectedKeyRef.current !== selectedKeyForReset) {
      prevSelectedKeyRef.current = selectedKeyForReset
      setAddingTag(false)
      setTagInput('')
    }
  }, [selectedKeyForReset])

  const handleAddTag = useCallback(() => {
    if (nodeInfo === null || nodeInfo.kindNum === null) return
    const segments = tagInput.split('/').map((s) => s.trim()).filter((s) => s.length > 0)
    if (segments.length === 0) return
    scene.add_node_tag(nodeInfo.kindNum, nodeInfo.id, segments)
    setTagInput('')
    setAddingTag(false)
    onDocumentChanged()
  }, [nodeInfo, tagInput, scene, onDocumentChanged])

  const handleRemoveTag = useCallback((path: string[]) => {
    if (nodeInfo === null || nodeInfo.kindNum === null) return
    scene.remove_node_tag(nodeInfo.kindNum, nodeInfo.id, path)
    onDocumentChanged()
  }, [nodeInfo, scene, onDocumentChanged])

  // --------------------------------------------------------------------------
  // Render
  // --------------------------------------------------------------------------

  // Nothing selected → an empty panel, no boilerplate.
  if (nodeInfo === null && selectedIds.length === 0) {
    return <div style={PANEL_STYLE} />
  }

  // Multi-selection → a single quiet count line (information, not boilerplate).
  if (nodeInfo === null) {
    return (
      <div style={PANEL_STYLE}>
        <div style={{ fontSize: '11px', color: 'var(--text-faint, #666)', padding: '4px 8px' }}>
          {selectedIds.length} selected
        </div>
      </div>
    )
  }

  return (
    <div style={PANEL_STYLE}>
      {/* Name — sketches can't be named yet; show the read-only default label instead. */}
      {nodeInfo.kind !== 'object' && nodeInfo.kind !== 'group' && nodeInfo.kind !== 'instance' ? (
        <div>
          <div style={LABEL_STYLE}>Name</div>
          <div style={VALUE_STYLE}>{nodeInfo.defaultLabel}</div>
        </div>
      ) : (
        <div>
          <div style={LABEL_STYLE}>Name</div>
          <input
            style={INPUT_STYLE}
            value={localName}
            onChange={(e) => setLocalName(e.target.value)}
            onBlur={commitName}
            onKeyDown={(e) => {
              if (e.key === 'Enter') {
                e.currentTarget.blur()
              }
            }}
            placeholder={nodeInfo.defaultLabel}
            spellCheck={false}
          />
        </div>
      )}

      {/* Type */}
      <div>
        <div style={LABEL_STYLE}>Type</div>
        <div style={VALUE_STYLE}>{kindLabel(nodeInfo.kind)}</div>
      </div>

      {/* Solid / Leaky — only for objects */}
      {nodeInfo.solid !== null && (
        <div>
          <div style={LABEL_STYLE}>Geometry</div>
          <div
            style={{
              ...VALUE_STYLE,
              color: nodeInfo.solid ? 'var(--status-solid)' : 'var(--status-leaky)',
              fontWeight: 'bold',
            }}
          >
            {nodeInfo.solid ? 'Solid' : 'Leaky'}
          </div>
        </div>
      )}

      {/* Tags — sketches can't be tagged yet. Empty state is just the "+"
       * button next to the label: no chips, no "No tags" text. */}
      {(nodeInfo.kind === 'object' || nodeInfo.kind === 'group' || nodeInfo.kind === 'instance') && (
      <div>
        <div style={{ display: 'flex', alignItems: 'center', gap: '6px', marginBottom: '2px' }}>
          <div style={{ ...LABEL_STYLE, marginBottom: 0 }}>Tags</div>
          {!addingTag && (
            <button
              onClick={() => setAddingTag(true)}
              title="Add tag"
              aria-label="Add tag"
              style={{
                background: 'none',
                border: '1px solid var(--border-strong, #444)',
                color: 'var(--text-muted, #999)',
                cursor: 'pointer',
                borderRadius: '3px',
                width: '16px',
                height: '16px',
                fontSize: '11px',
                lineHeight: 1,
                padding: 0,
                display: 'inline-flex',
                alignItems: 'center',
                justifyContent: 'center',
                flexShrink: 0,
              }}
            >
              +
            </button>
          )}
        </div>

        {nodeInfo.tags.length > 0 && (
          <div style={{ display: 'flex', flexDirection: 'column', gap: '4px' }}>
            {nodeInfo.tags.map((path) => (
              <TagChip
                key={path.join('/')}
                path={path}
                onRemove={() => handleRemoveTag(path)}
              />
            ))}
          </div>
        )}

        {/* Add-tag field — revealed by the "+" button. Auto-focused; Enter
         * commits, Esc cancels, blur with an empty field closes. */}
        {addingTag && (
          <input
            style={{ ...INPUT_STYLE, marginTop: nodeInfo.tags.length > 0 ? '6px' : '2px' }}
            autoFocus
            value={tagInput}
            onChange={(e) => setTagInput(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') {
                handleAddTag()
              } else if (e.key === 'Escape') {
                setTagInput('')
                setAddingTag(false)
              }
            }}
            onBlur={() => {
              if (tagInput.trim() === '') {
                setTagInput('')
                setAddingTag(false)
              } else {
                handleAddTag()
              }
            }}
            placeholder="Structure/Roof"
            spellCheck={false}
          />
        )}
      </div>
      )}
    </div>
  )
}

// ---------------------------------------------------------------------------
// TagChip — a single removable tag chip
// ---------------------------------------------------------------------------

function TagChip({ path, onRemove }: { path: string[]; onRemove: () => void }) {
  const display = path.join(' / ')
  return (
    <div
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: '4px',
        background: 'var(--accent-tint-18, #1e3a5e)',
        borderRadius: '3px',
        padding: '2px 6px',
        fontSize: '11px',
        color: 'var(--accent-text-on-tint, #aaccee)',
        fontFamily: 'var(--font-family-ui)',
      }}
    >
      <span style={{ flex: 1, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
        {display}
      </span>
      <button
        onClick={onRemove}
        title="Remove tag"
        style={{
          background: 'none',
          border: 'none',
          color: 'var(--accent-text-on-tint, #7799bb)',
          cursor: 'pointer',
          fontSize: '12px',
          lineHeight: 1,
          padding: '0 1px',
          flexShrink: 0,
        }}
      >
        ×
      </button>
    </div>
  )
}

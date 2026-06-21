/**
 * ObjectInfoPanel — a closeable pane showing info for the currently selected node.
 *
 * Displays the selected node's:
 *   - Name (editable — commits on Enter/blur → scene.set_node_name; empty clears)
 *   - Type (read-only: "Object", "Group", or "Component")
 *   - Solid / Leaky (only for Objects; calls scene.object_solid)
 *   - Tags (removable chips + add input)
 *
 * Modeled on TagsPanel for styling, close button, and pane mounting patterns.
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
import { nodeKindToNumber, type NodeRef } from './treeModel'

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
  /** Called when the user closes the panel. */
  onClose: () => void
}

/** Human-readable type label for each node kind. */
function kindLabel(kind: NodeRef['kind']): string {
  if (kind === 'object') return 'Object'
  if (kind === 'group') return 'Group'
  return 'Component'
}

const PANEL_STYLE: React.CSSProperties = {
  width: '220px',
  flexShrink: 0,
  background: '#2a2a2a',
  color: '#ddd',
  borderRadius: '4px',
  padding: '8px',
  overflowY: 'auto',
  display: 'flex',
  flexDirection: 'column',
  gap: '10px',
}

const LABEL_STYLE: React.CSSProperties = {
  fontSize: '10px',
  color: '#888',
  textTransform: 'uppercase',
  letterSpacing: '0.05em',
  marginBottom: '2px',
}

const VALUE_STYLE: React.CSSProperties = {
  fontSize: '12px',
  color: '#ccc',
  fontFamily: 'monospace',
}

const INPUT_STYLE: React.CSSProperties = {
  width: '100%',
  boxSizing: 'border-box',
  background: '#1e1e1e',
  border: '1px solid #444',
  borderRadius: '3px',
  color: '#eee',
  fontFamily: 'monospace',
  fontSize: '12px',
  padding: '3px 6px',
  outline: 'none',
}

export function ObjectInfoPanel({ scene, docRev, selectedIds, onDocumentChanged, onClose }: Props) {
  // --------------------------------------------------------------------------
  // Derive the node info from the scene whenever docRev or selectedIds changes.
  // --------------------------------------------------------------------------
  const nodeInfo = useMemo(() => {
    if (selectedIds.length !== 1) return null
    const node = selectedIds[0]
    const kind = node.kind
    const kindNum = nodeKindToNumber(kind)
    const id = node.id

    let nameFromScene: string | undefined
    if (kind === 'object') nameFromScene = scene.object_name(id)
    else if (kind === 'group') nameFromScene = scene.group_name(id)
    else nameFromScene = scene.instance_name(id)

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

    return { node, kind, kindNum, id, nameFromScene, tags, solid }
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
    if (nodeInfo === null) return
    const trimmed = localName.trim()
    // Pass undefined to clear; pass the string to set.
    scene.set_node_name(nodeInfo.kindNum, nodeInfo.id, trimmed === '' ? undefined : trimmed)
    onDocumentChanged()
  }, [nodeInfo, localName, scene, onDocumentChanged])

  // --------------------------------------------------------------------------
  // Tag add state
  // --------------------------------------------------------------------------
  const [tagInput, setTagInput] = useState('')

  const handleAddTag = useCallback(() => {
    if (nodeInfo === null) return
    const segments = tagInput.split('/').map((s) => s.trim()).filter((s) => s.length > 0)
    if (segments.length === 0) return
    scene.add_node_tag(nodeInfo.kindNum, nodeInfo.id, segments)
    setTagInput('')
    onDocumentChanged()
  }, [nodeInfo, tagInput, scene, onDocumentChanged])

  const handleRemoveTag = useCallback((path: string[]) => {
    if (nodeInfo === null) return
    scene.remove_node_tag(nodeInfo.kindNum, nodeInfo.id, path)
    onDocumentChanged()
  }, [nodeInfo, scene, onDocumentChanged])

  // --------------------------------------------------------------------------
  // Render
  // --------------------------------------------------------------------------
  return (
    <aside style={PANEL_STYLE}>
      {/* Panel header */}
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
        <span style={{ fontWeight: 'bold', fontSize: '12px', color: '#eee' }}>Object Info</span>
        <button
          onClick={onClose}
          title="Close panel"
          style={{
            background: 'none',
            border: 'none',
            color: '#888',
            cursor: 'pointer',
            fontSize: '14px',
            lineHeight: 1,
            padding: '0 2px',
          }}
        >
          ×
        </button>
      </div>

      {nodeInfo === null ? (
        <EmptyState multiSelect={selectedIds.length > 1} />
      ) : (
        <>
          {/* Name */}
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
              placeholder="(unnamed)"
              spellCheck={false}
            />
          </div>

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
                  color: nodeInfo.solid ? '#4caf50' : '#f44336',
                  fontWeight: 'bold',
                }}
              >
                {nodeInfo.solid ? 'Solid' : 'Leaky'}
              </div>
            </div>
          )}

          {/* Tags */}
          <div>
            <div style={LABEL_STYLE}>Tags</div>
            {nodeInfo.tags.length === 0 ? (
              <div style={{ fontSize: '11px', color: '#666', fontStyle: 'italic' }}>No tags</div>
            ) : (
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

            {/* Add tag input */}
            <div style={{ display: 'flex', gap: '4px', marginTop: '6px' }}>
              <input
                style={{ ...INPUT_STYLE, flex: 1 }}
                value={tagInput}
                onChange={(e) => setTagInput(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter') handleAddTag()
                }}
                placeholder="Structure/Roof"
                spellCheck={false}
              />
              <button
                onClick={handleAddTag}
                disabled={tagInput.trim() === ''}
                style={{
                  background: '#3a5e9e',
                  border: 'none',
                  color: '#eee',
                  cursor: tagInput.trim() === '' ? 'default' : 'pointer',
                  borderRadius: '3px',
                  fontSize: '11px',
                  padding: '3px 8px',
                  opacity: tagInput.trim() === '' ? 0.5 : 1,
                  flexShrink: 0,
                }}
              >
                Add
              </button>
            </div>
          </div>
        </>
      )}
    </aside>
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
        background: '#1e3a5e',
        borderRadius: '3px',
        padding: '2px 6px',
        fontSize: '11px',
        color: '#aaccee',
        fontFamily: 'monospace',
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
          color: '#7799bb',
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

// ---------------------------------------------------------------------------
// EmptyState
// ---------------------------------------------------------------------------

function EmptyState({ multiSelect }: { multiSelect: boolean }) {
  return (
    <div
      style={{
        fontSize: '11px',
        color: '#666',
        fontStyle: 'italic',
        padding: '4px 8px',
        lineHeight: 1.5,
      }}
    >
      {multiSelect ? 'Multiple nodes selected.' : 'Select an object.'}
    </div>
  )
}

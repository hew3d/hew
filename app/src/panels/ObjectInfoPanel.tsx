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
 * A component instance gets the component treatment instead of the plain
 * Name field:
 *   - Definition Name (editable → scene.set_component_name) — the shared
 *     label; renaming it renames every instance of the component.
 *   - Instance Name (editable → scene.set_node_name) — this placement's own
 *     override; the Outliner then shows "Instance Name (Definition Name)".
 *   - Type reads "Component (N instances)", and the count is a button that
 *     selects every instance of the definition — in the viewport and the
 *     Outliner at once (both render from the same selection state).
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
  /**
   * Replace the selection with `nodes` (the "(N instances)" click). Routed to
   * the same selection state the viewport and Outliner render from.
   */
  onSelectMany: (nodes: NodeRef[]) => void
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

export function ObjectInfoPanel({ scene, docRev, selectedIds, onDocumentChanged, onSelectMany }: Props) {
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
        // The ref's id is the chain's representative EDGE.
        islandId = scene.sketch_edge_island(sketchId, id)
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
        defId: null as bigint | null,
        defName: undefined as string | undefined,
        instanceIds: [] as bigint[],
      }
    }

    const kindNum = nodeKindToNumber(kind)

    let nameFromScene: string | undefined
    let defName: string | undefined
    let defId: bigint | null = null
    let instanceIds: bigint[] = []
    if (kind === 'object') {
      nameFromScene = scene.object_name(id)
    } else if (kind === 'group') {
      nameFromScene = scene.group_name(id)
    } else {
      nameFromScene = scene.instance_name(id)
      const def = scene.instance_def(id)
      if (def !== undefined) {
        defId = def
        defName = scene.component_name(def)
        // Every visible sibling placing the same definition, this one included
        // — the "(N instances)" count and the click-to-select set.
        instanceIds = Array.from(scene.instances_of(def))
      }
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

    return {
      node,
      kind,
      kindNum,
      id,
      nameFromScene,
      defaultLabel,
      tags,
      solid,
      defId,
      defName,
      instanceIds,
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [scene, docRev, selectedIds])

  // --------------------------------------------------------------------------
  // Local controlled state for the name input.
  // Sync with the kernel-derived name whenever nodeInfo changes (new selection
  // or doc mutation). This effect is idempotent — calling setLocalName with the
  // same value twice is harmless, so StrictMode double-invoke is safe.
  //
  // The reset is keyed on the selected node's IDENTITY as well as the value:
  // resetting only when the value changes would carry uncommitted typed text
  // across a selection change between two nodes whose kernel names happen to
  // be equal (e.g. both unnamed), and the next blur would silently commit
  // node A's typed name to node B.
  // --------------------------------------------------------------------------
  const syncKey = nodeInfo !== null ? nodeKey(nodeInfo.node) : null
  const [localName, setLocalName] = useState(nodeInfo?.nameFromScene ?? '')
  const prevNameSyncRef = useRef<{ key: string | null; name: string | undefined }>({
    key: null,
    name: undefined,
  })
  useEffect(() => {
    const next = nodeInfo?.nameFromScene
    const prev = prevNameSyncRef.current
    if (syncKey !== prev.key || next !== prev.name) {
      prevNameSyncRef.current = { key: syncKey, name: next }
      setLocalName(next ?? '')
    }
  }, [nodeInfo?.nameFromScene, syncKey])

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
  // Definition Name (instances only) — local controlled state + commit,
  // mirroring the name field's identity-keyed reset, but keyed on the
  // DEFINITION's identity, not the instance's: this field edits the shared
  // definition, so cycling between two instances of the same definition
  // keeps an in-progress edit alive (it commits to that one definition
  // either way), while moving to an instance of a different — possibly
  // same-valued — definition resets it, so typed text never leaks onto
  // another component.
  // --------------------------------------------------------------------------
  const [localDefName, setLocalDefName] = useState(nodeInfo?.defName ?? '')
  const defSyncKey = nodeInfo !== null && nodeInfo.defId !== null ? String(nodeInfo.defId) : null
  const prevDefSyncRef = useRef<{ key: string | null; name: string | undefined }>({
    key: null,
    name: undefined,
  })
  useEffect(() => {
    const next = nodeInfo?.defName
    const prev = prevDefSyncRef.current
    if (defSyncKey !== prev.key || next !== prev.name) {
      prevDefSyncRef.current = { key: defSyncKey, name: next }
      setLocalDefName(next ?? '')
    }
  }, [nodeInfo?.defName, defSyncKey])

  const commitDefName = useCallback(() => {
    if (nodeInfo === null || nodeInfo.defId === null) return
    const trimmed = localDefName.trim()
    scene.set_component_name(nodeInfo.defId, trimmed === '' ? undefined : trimmed)
    onDocumentChanged()
  }, [nodeInfo, localDefName, scene, onDocumentChanged])

  // --------------------------------------------------------------------------
  // "(N instances)" click: select every instance of the definition. The
  // viewport and the Outliner both render from this one selection state.
  // --------------------------------------------------------------------------
  const selectAllInstances = useCallback(() => {
    if (nodeInfo === null || nodeInfo.instanceIds.length === 0) return
    onSelectMany(nodeInfo.instanceIds.map((id) => ({ kind: 'instance' as const, id })))
  }, [nodeInfo, onSelectMany])

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
      {/* Name — sketches can't be named yet; show the read-only default label
       * instead. A component instance splits into Definition Name (the shared
       * label — renames every instance) and Instance Name (this placement's
       * own override; the Outliner then shows "Instance (Definition)"). */}
      {nodeInfo.kind !== 'object' && nodeInfo.kind !== 'group' && nodeInfo.kind !== 'instance' ? (
        <div>
          <div style={LABEL_STYLE}>Name</div>
          <div style={VALUE_STYLE}>{nodeInfo.defaultLabel}</div>
        </div>
      ) : nodeInfo.kind === 'instance' ? (
        <>
          <div>
            <div style={LABEL_STYLE}>Definition Name</div>
            <input
              style={INPUT_STYLE}
              aria-label="Definition Name"
              value={localDefName}
              onChange={(e) => setLocalDefName(e.target.value)}
              onBlur={commitDefName}
              onKeyDown={(e) => {
                if (e.key === 'Enter') {
                  e.currentTarget.blur()
                }
              }}
              title="Shared by every instance of this component"
              spellCheck={false}
            />
          </div>
          <div>
            <div style={LABEL_STYLE}>Instance Name</div>
            <input
              style={INPUT_STYLE}
              aria-label="Instance Name"
              value={localName}
              onChange={(e) => setLocalName(e.target.value)}
              onBlur={commitName}
              onKeyDown={(e) => {
                if (e.key === 'Enter') {
                  e.currentTarget.blur()
                }
              }}
              placeholder={nodeInfo.defaultLabel}
              title="This instance only — leave empty to show the definition name"
              spellCheck={false}
            />
          </div>
        </>
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

      {/* Type — an instance also shows how many placements share its
       * definition; the count selects them all (viewport + Outliner). */}
      <div>
        <div style={LABEL_STYLE}>Type</div>
        <div style={VALUE_STYLE}>
          <span>{kindLabel(nodeInfo.kind)}</span>
          {nodeInfo.kind === 'instance' && nodeInfo.instanceIds.length > 0 && (
            <>
              {' '}
              <button
                onClick={selectAllInstances}
                title="Select every instance of this component"
                style={{
                  background: 'none',
                  border: 'none',
                  padding: 0,
                  font: 'inherit',
                  fontFamily: 'var(--font-family-ui)',
                  color: 'var(--accent-base, #7aa7e0)',
                  cursor: 'pointer',
                }}
              >
                ({nodeInfo.instanceIds.length}{' '}
                {nodeInfo.instanceIds.length === 1 ? 'instance' : 'instances'})
              </button>
            </>
          )}
        </div>
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

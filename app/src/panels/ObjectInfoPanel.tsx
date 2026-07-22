/**
 * ObjectInfoPanel — a closeable pane showing info for the currently selected node.
 *
 * Displays the selected node's:
 *   - Name (editable — commits on Enter/blur → scene.set_node_name; empty
 *     clears the kernel name and the field falls back to showing the same
 *     default label the Outliner shows, via resolveLabel — never "(unnamed)")
 *   - Type (read-only: "Object", "Group", "Component", or "Sketch")
 *   - Solid / Leaky (only for Objects; calls scene.object_solid)
 *   - Bounding Box (world axis-aligned bounding-box extents, X × Y × Z, in the
 *     active length format — objectBounds.worldBoundsForSelection computes
 *     it client-side from the render meshes the app already holds; shown for
 *     Object/Group/Component/multi-selection, hidden when the selection has
 *     no mesh, e.g. only Sketches)
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
import { worldBoundsForSelection, boundsExtents, type Bounds } from './objectBounds'
import { formatLength } from '../settings/units'
import { parseKernelErrorCode, kernelErrorMessage } from '../kernelErrors'
import { MIN_SEGMENTS_PER_TURN } from '../tools/arcMath'

/**
 * The fewest facets a circle may have. Mirrors the kernel's
 * `MIN_CIRCLE_SEGMENTS` — the density floor below which a chord ring is
 * deliberately not stamped as a circle at all — via the draw tools' own copy
 * of it in `arcMath`, so this file introduces no third source of truth. The
 * kernel refuses anything below it (`SegmentsBelowFloor`); the panel says so
 * before the round trip.
 */
const MIN_CIRCLE_SEGMENTS = MIN_SEGMENTS_PER_TURN

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
   * Replace the selection with `nodes` — the "(N instances)" click, and the
   * re-point a re-facet needs once it has replaced the selected curve's
   * edges. Routed to the same selection state the viewport and Outliner
   * render from.
   */
  onSelectMany: (nodes: NodeRef[]) => void
  /**
   * Surface a refusal. Segments is the panel's first control the kernel can
   * refuse, and a refusal has to be visible — never a field that silently
   * springs back. Same signature every tool's toast uses.
   */
  onToast?: (message: string, code?: string) => void
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

export function ObjectInfoPanel({ scene, docRev, selectedIds, onDocumentChanged, onSelectMany, onToast }: Props) {
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

      // Segments — SketchUp's Entity Info "Segments", for a drawn CIRCLE.
      // Only a chain carrying an analytic circle has one: `sketch_curve_geom`
      // answers for circles only, so a polygon chain (whose sides are its
      // real geometry, not facets of anything) and a pre-analytic chain both
      // fall through to null and the row does not render. The count itself is
      // just how many facets the chain currently has — nothing stores it.
      let curveId: bigint | null = null
      let segments: number | null = null
      if (kind === 'sketch-curve') {
        const cid = scene.sketch_edge_curve(sketchId, id)
        if (cid !== undefined) {
          curveId = cid
          if (scene.sketch_curve_geom(sketchId, cid) !== undefined) {
            segments = scene.sketch_curve_edges(sketchId, cid).length
          }
        }
      }

      return {
        sketchId,
        curveId,
        segments,
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
      // Curve-only fields: an Object/Group/Instance never has a Segments row.
      sketchId: undefined as bigint | undefined,
      curveId: null as bigint | null,
      segments: null as number | null,
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
  // World bounding-box (Bounding Box row). Computed for ANY selection — single
  // node OR multi-selection — unlike nodeInfo above, which is single-node
  // only; this is why it's a separate memo rather than a field on nodeInfo.
  // Client-side (objectBounds.worldBoundsForSelection): unions the world AABB
  // of every leaf mesh reachable from the selection. Null for an empty
  // selection or one with no mesh (e.g. only Sketches) — the Bounding Box row
  // just doesn't render in that case.
  // --------------------------------------------------------------------------
  const bounds = useMemo(
    () => worldBoundsForSelection(scene, selectedIds),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [scene, docRev, selectedIds],
  )

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
  // Segments (drawn circles only) — SketchUp's Entity Info "Segments".
  //
  // Editing it RE-FACETS the existing circle: the chain keeps its handle and
  // its exact centre and radius, and its chords are rebuilt at the new
  // density. It is not a setting for the next circle drawn.
  //
  // Two things the commit has to get right:
  //   - It is bracketed as one sketch gesture, so the whole rebuild is a
  //     single undo step (the gesture snapshots the sketch).
  //   - It invalidates the selection. A `sketch-curve` selection carries a
  //     representative EDGE handle, and every one of the chain's edges is
  //     replaced — so the selection is re-pointed at the rebuilt chain's own
  //     representative before the panel re-renders, or the panel would go
  //     blank on a stale handle.
  // --------------------------------------------------------------------------
  const [localSegments, setLocalSegments] = useState('')
  // Keyed on SKETCH + curve, not the curve alone. Curve handles are slotmap
  // keys scoped per sketch, so the first curve drawn on one plane and the
  // first drawn on another share a handle — keying on it alone would carry
  // uncommitted text from one circle to another whose facet count happened to
  // match, and the next blur would commit the first circle's typed value onto
  // the second. This is the same composite identity `nodeKey` builds for the
  // Name field above, for the same reason.
  const segSyncKey =
    nodeInfo !== null && nodeInfo.curveId !== null
      ? `${nodeInfo.sketchId}:${nodeInfo.curveId}`
      : null
  const prevSegSyncRef = useRef<{ key: string | null; value: number | null }>({
    key: null,
    value: null,
  })
  useEffect(() => {
    const next = nodeInfo?.segments ?? null
    const prev = prevSegSyncRef.current
    if (segSyncKey !== prev.key || next !== prev.value) {
      prevSegSyncRef.current = { key: segSyncKey, value: next }
      setLocalSegments(next === null ? '' : String(next))
    }
  }, [nodeInfo?.segments, segSyncKey])

  const commitSegments = useCallback(() => {
    if (
      nodeInfo === null ||
      nodeInfo.curveId === null ||
      nodeInfo.sketchId === undefined ||
      nodeInfo.segments === null
    ) {
      return
    }
    const sketchId = nodeInfo.sketchId
    const curveId = nodeInfo.curveId
    const parsed = Number.parseInt(localSegments.trim(), 10)
    if (!Number.isFinite(parsed)) {
      setLocalSegments(String(nodeInfo.segments))
      return
    }
    if (parsed === nodeInfo.segments) return // nothing to do
    if (parsed < MIN_CIRCLE_SEGMENTS) {
      // Refused HERE rather than clamped silently: substituting 24 for what
      // was typed would build a different circle than the one asked for. The
      // kernel refuses this too (`SegmentsBelowFloor`) — this is the same
      // refusal, said before the round trip.
      onToast?.(kernelErrorMessage('SegmentsBelowFloor', ''), 'SegmentsBelowFloor')
      setLocalSegments(String(nodeInfo.segments))
      return
    }

    let repointTo: bigint | null = null
    try {
      scene.sketch_begin_gesture(sketchId)
      try {
        const report = scene.sketch_refacet_curve(sketchId, curveId, parsed)
        try {
          const edges = report.new_edges()
          if (edges.length > 0) {
            // Any member resolves to the chain's canonical representative.
            const chain = scene.sketch_curve_chain(sketchId, edges[0])
            repointTo = chain.length > 0 ? chain[0] : edges[0]
          }
        } finally {
          report.free()
        }
      } finally {
        scene.sketch_end_gesture(sketchId)
      }
    } catch (err) {
      const code = parseKernelErrorCode(err)
      const raw = err instanceof Error ? err.message : String(err)
      onToast?.(kernelErrorMessage(code ?? 'Unknown', raw), code ?? undefined)
      setLocalSegments(String(nodeInfo.segments))
      return
    }

    if (repointTo !== null) {
      onSelectMany([{ kind: 'sketch-curve', id: repointTo, sketch: sketchId }])
    }
    onDocumentChanged()
  }, [nodeInfo, localSegments, scene, onDocumentChanged, onSelectMany, onToast])

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

  // Multi-selection → a single quiet count line (information, not boilerplate),
  // plus the union Bounding Box of every selected node's mesh, if any.
  if (nodeInfo === null) {
    return (
      <div style={PANEL_STYLE}>
        <div style={{ fontSize: '11px', color: 'var(--text-faint, #666)', padding: '4px 8px' }}>
          {selectedIds.length} selected
        </div>
        {bounds !== null && <BoundingBoxRow bounds={bounds} />}
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

      {/* Segments — a drawn circle only. Editing it re-facets that circle in
       * place (one undo step); the circle itself, its centre and its radius
       * are unchanged. Commits on Enter or blur, like the name fields. */}
      {nodeInfo.segments !== null && (
        <div>
          <div style={LABEL_STYLE}>Segments</div>
          <input
            style={INPUT_STYLE}
            aria-label="Segments"
            type="number"
            min={MIN_CIRCLE_SEGMENTS}
            step={1}
            value={localSegments}
            onChange={(e) => setLocalSegments(e.target.value)}
            onBlur={commitSegments}
            onKeyDown={(e) => {
              if (e.key === 'Enter') {
                e.currentTarget.blur()
              } else if (e.key === 'Escape') {
                setLocalSegments(String(nodeInfo.segments))
                e.currentTarget.blur()
              }
            }}
            title={`How many straight facets this circle is drawn with (minimum ${MIN_CIRCLE_SEGMENTS}). Changing it rebuilds the circle.`}
            spellCheck={false}
          />
        </div>
      )}

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

      {/* Bounding Box — world AABB extents; hidden for a mesh-less selection
       * (e.g. a Sketch, or a stale instance). */}
      {bounds !== null && <BoundingBoxRow bounds={bounds} />}

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
// BoundingBoxRow — world AABB extents as "X 12mm × Y 34mm × Z 56mm", each
// axis letter colored to match the viewport's axis colors (X=red, Y=green,
// Z=blue — Z is height) and each extent formatted with the same length
// formatter Tape Measure / typed entry use, so it always reads in the
// active length format (docs/DEVELOPMENT.md rule 6 — display-layer
// formatting only, kernel lengths stay f64 meters throughout). Labeled
// "Bounding Box" (not "Dimensions") so it's clear the three numbers are the
// selection's axis-aligned extents, not e.g. a single object property.
// ---------------------------------------------------------------------------

function BoundingBoxRow({ bounds }: { bounds: Bounds }) {
  const [ex, ey, ez] = boundsExtents(bounds)
  return (
    <div>
      <div style={LABEL_STYLE}>Bounding Box</div>
      <div style={VALUE_STYLE}>
        <span style={{ color: 'var(--axis-red)' }}>X</span> {formatLength(ex)}
        <span style={{ color: 'var(--text-faint, #888)' }}> × </span>
        <span style={{ color: 'var(--axis-green)' }}>Y</span> {formatLength(ey)}
        <span style={{ color: 'var(--text-faint, #888)' }}> × </span>
        <span style={{ color: 'var(--axis-blue)' }}>Z</span> {formatLength(ez)}
      </div>
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

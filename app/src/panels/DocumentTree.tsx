/**
 * DocumentTree — the document outliner ( navigation).
 *
 * A recursive tree of the document's top-level nodes (Objects + Groups),
 * with expand/collapse for groups. Sketches remain a flat separate section.
 * Breadcrumb shows the active context path.
 *
 * Click to select; double-click to enter context. Group/Ungroup buttons added
 * alongside the existing boolean buttons.
 */

import { useMemo, useState } from 'react'
import type { Scene as WasmScene } from '../wasm/loader'
import {
  entityLabel,
  breadcrumb,
  isTreeRowDimmed,
  canGroup as canGroupHelper,
  canUngroup as canUngroupHelper,
  nodeRefFromJs,
  nodeKindToNumber,
  type NodeRef,
} from './treeModel'

interface Props {
  scene: WasmScene
  /** Bumped by the parent on any document change to trigger a re-query. */
  docRev: number
  /** Per-object watertight state, for the solid/leaky dot. */
  watertightMap: Map<bigint, boolean>
  /** Selected nodes (ordered; index 0 = primary). */
  selectedIds: NodeRef[]
  /** Active context path. Empty = top level. */
  activeContext: NodeRef[]
  /** `additive` = shift/ctrl-click (multi-select). */
  onSelect: (node: NodeRef, additive: boolean) => void
  onEnterContext: (node: NodeRef) => void
  /** Pop one level off the context path (breadcrumb root = exit to top). */
  onExitContext: () => void
  /** Truncate the context path to a given depth (crumb click). */
  onSetContextDepth: (depth: number) => void
  /** True when exactly two objects are selected at top level (boolean-ready). */
  canBoolean: boolean
  /** Run a boolean on the two selected objects (0=union,1=subtract,2=intersect). */
  onBoolean: (op: number) => void
  /** Group selected nodes. */
  onGroup: () => void
  /** Ungroup the single selected group. */
  onUngroup: () => void
  /** True when the selection can become a component. */
  canMakeComponent: boolean
  /** Fold the selection into a component + instance. */
  onMakeComponent: () => void
  /** True when exactly one instance is selected (can place a copy). */
  canPlaceInstance: boolean
  /** Place another instance of the selected instance's definition. */
  onPlaceInstance: () => void
  /** True when exactly one instance is selected (can explode). */
  canExplodeInstance: boolean
  /** Bake the instance's pose into independent world objects. */
  onExplodeInstance: () => void
  /** True when exactly one instance is selected (can make unique). */
  canMakeUnique: boolean
  /** Detach the instance onto a private copy of its definition. */
  onMakeUnique: () => void
  /** Called when the user closes the panel. */
  onClose: () => void
}

const ROW_BASE: React.CSSProperties = {
  display: 'flex',
  alignItems: 'center',
  gap: '6px',
  padding: '3px 8px',
  fontSize: '12px',
  fontFamily: 'monospace',
  cursor: 'pointer',
  borderRadius: '3px',
  userSelect: 'none',
}

export function DocumentTree({
  scene,
  docRev,
  watertightMap,
  selectedIds,
  activeContext,
  onSelect,
  onEnterContext,
  onExitContext,
  onSetContextDepth,
  canBoolean,
  onBoolean,
  onGroup,
  onUngroup,
  canMakeComponent,
  onMakeComponent,
  canPlaceInstance,
  onPlaceInstance,
  canExplodeInstance,
  onExplodeInstance,
  canMakeUnique,
  onMakeUnique,
  onClose,
}: Props) {
  // Re-query the entity lists whenever the document changes.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  const topNodes = useMemo(
    () => scene.top_level_nodes().map(nodeRefFromJs),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [scene, docRev],
  )
  // eslint-disable-next-line react-hooks/exhaustive-deps
  const sketches = useMemo(() => Array.from(scene.sketch_ids()), [scene, docRev])
  // eslint-disable-next-line react-hooks/exhaustive-deps
  const instanceIds = useMemo(() => Array.from(scene.instance_ids()), [scene, docRev])

  const selected = new Set(selectedIds.map((n) => `${n.kind}:${n.id}`))
  const isSelected = (n: NodeRef) => selected.has(`${n.kind}:${n.id}`)

  const deepestCtx = activeContext.length > 0 ? activeContext[activeContext.length - 1] : null

  // Compute canGroup / canUngroup for the button row
  const parentOf = (n: NodeRef) => scene.node_parent(nodeKindToNumber(n.kind), n.id)
  const canGroupNow = canGroupHelper(selectedIds, parentOf)
  const canUngroupNow = canUngroupHelper(selectedIds)

  // Label resolver for breadcrumbs — uses top_level_nodes ordering
  const labelFor = (node: NodeRef): string => {
    if (node.kind === 'group') {
      const groups = Array.from(scene.group_ids())
      const idx = groups.indexOf(node.id)
      return entityLabel('group', idx >= 0 ? idx : 0)
    } else if (node.kind === 'instance') {
      const instances = Array.from(scene.instance_ids())
      const idx = instances.indexOf(node.id)
      return entityLabel('instance', idx >= 0 ? idx : 0)
    } else {
      const objects = Array.from(scene.object_ids())
      const idx = objects.indexOf(node.id)
      return entityLabel('object', idx >= 0 ? idx : 0)
    }
  }

  const crumbs = breadcrumb(activeContext, labelFor)

  return (
    <aside
      style={{
        width: '240px',
        flexShrink: 0,
        background: '#2a2a2a',
        color: '#ddd',
        borderRadius: '4px',
        padding: '8px',
        overflowY: 'auto',
        display: 'flex',
        flexDirection: 'column',
        gap: '8px',
      }}
    >
      {/* Panel header */}
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
        <span style={{ fontWeight: 'bold', fontSize: '12px', color: '#eee' }}>Model Info</span>
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
      {/* Breadcrumb */}
      <div style={{ display: 'flex', alignItems: 'center', flexWrap: 'wrap', gap: '2px', fontSize: '12px', fontFamily: 'monospace' }}>
        {crumbs.map((c, i) => (
          <span key={i} style={{ display: 'flex', alignItems: 'center', gap: '2px' }}>
            {i > 0 && <span style={{ color: '#777' }}>›</span>}
            {i === crumbs.length - 1 ? (
              <span style={{ color: '#fff', fontWeight: 'bold' }}>{c.label}</span>
            ) : (
              <button
                onClick={() => {
                  if (c.depth === -1) {
                    // Root: exit to top
                    onExitContext()
                  } else {
                    // Truncate path to depth d+1
                    onSetContextDepth(c.depth + 1)
                  }
                }}
                style={{
                  background: 'none',
                  border: 'none',
                  color: '#7aa7e0',
                  cursor: 'pointer',
                  fontFamily: 'monospace',
                  fontSize: '12px',
                  padding: 0,
                }}
              >
                {c.label}
              </button>
            )}
          </span>
        ))}
      </div>

      {/* Action buttons row */}
      <div style={{ display: 'flex', flexDirection: 'column', gap: '4px' }}>
        {/* Boolean actions — active only with exactly two objects selected. */}
        {canBoolean && (
          <div style={{ display: 'flex', gap: '4px' }}>
            <BoolButton label="Union" title="Combine both (A ∪ B)" onClick={() => onBoolean(0)} />
            <BoolButton label="Subtract" title="First minus second (A − B)" onClick={() => onBoolean(1)} />
            <BoolButton label="Intersect" title="Overlap only (A ∩ B)" onClick={() => onBoolean(2)} />
          </div>
        )}
        {/* Group / Ungroup */}
        {(canGroupNow || canUngroupNow) && (
          <div style={{ display: 'flex', gap: '4px' }}>
            {canGroupNow && (
              <ActionButton
                label="Group"
                title="Group selected nodes into a merge group"
                onClick={onGroup}
              />
            )}
            {canUngroupNow && (
              <ActionButton
                label="Ungroup"
                title="Dissolve the selected group"
                onClick={onUngroup}
              />
            )}
          </div>
        )}
        {/* Component actions */}
        {(canMakeComponent || canPlaceInstance) && (
          <div style={{ display: 'flex', gap: '4px' }}>
            {canMakeComponent && (
              <ActionButton
                label="Make Component"
                title="Fold selection into a shared component definition"
                onClick={onMakeComponent}
              />
            )}
            {canPlaceInstance && (
              <ActionButton
                label="Place Copy"
                title="Stamp another instance of this component (offset by 0.5m)"
                onClick={onPlaceInstance}
              />
            )}
          </div>
        )}
        {(canExplodeInstance || canMakeUnique) && (
          <div style={{ display: 'flex', gap: '4px' }}>
            {canExplodeInstance && (
              <ActionButton
                label="Explode"
                title="Bake instance pose into independent world objects"
                onClick={onExplodeInstance}
              />
            )}
            {canMakeUnique && (
              <ActionButton
                label="Make Unique"
                title="Detach onto a private copy of this component's definition"
                onClick={onMakeUnique}
              />
            )}
          </div>
        )}
      </div>

      {/* Recursive node tree */}
      <Section title="Objects" empty="(no solids yet)">
        {topNodes.map((node, index) => (
          <NodeRow
            key={`${node.kind}:${node.id}`}
            node={node}
            index={index}
            depth={0}
            scene={scene}
            docRev={docRev}
            watertightMap={watertightMap}
            activeContext={activeContext}
            deepestCtx={deepestCtx}
            isSelected={isSelected}
            onSelect={onSelect}
            onEnterContext={onEnterContext}
          />
        ))}
      </Section>

      <Section title="Sketches" empty="(no sketches yet)">
        {sketches.map((id, index) => (
          <Row
            key={String(id)}
            label={entityLabel('sketch', index)}
            selected={false}
            active={false}
            dimmed={activeContext.length > 0}
            indent={0}
            onClick={() => { /* sketches are not selectable as nodes for now */ }}
          />
        ))}
      </Section>
    </aside>
  )
}

/** One tree row that may be an object or a group (with expand/collapse). */
function NodeRow({
  node,
  index,
  depth,
  scene,
  docRev,
  watertightMap,
  activeContext,
  deepestCtx,
  isSelected,
  onSelect,
  onEnterContext,
}: {
  node: NodeRef
  index: number
  depth: number
  scene: WasmScene
  docRev: number
  watertightMap: Map<bigint, boolean>
  activeContext: NodeRef[]
  deepestCtx: NodeRef | null
  isSelected: (n: NodeRef) => boolean
  onSelect: (n: NodeRef, additive: boolean) => void
  onEnterContext: (n: NodeRef) => void
}) {
  const [expanded, setExpanded] = useState(true)

  const selected = isSelected(node)
  const active = deepestCtx !== null &&
    deepestCtx.kind === node.kind && deepestCtx.id === node.id
  const dimmed = isTreeRowDimmed(activeContext, node, depth)

  if (node.kind === 'object') {
    const watertight = watertightMap.get(node.id) ?? true
    return (
      <Row
        label={entityLabel('object', index)}
        selected={selected}
        active={active}
        dimmed={dimmed}
        indent={depth}
        dot={watertight ? '#1a7a3a' : '#cc3322'}
        onClick={(additive) => onSelect(node, additive)}
        onDoubleClick={() => onEnterContext(node)}
      />
    )
  }

  if (node.kind === 'instance') {
    return (
      <Row
        label={entityLabel('instance', index)}
        selected={selected}
        active={active}
        dimmed={dimmed}
        indent={depth}
        isInstance
        onClick={(additive) => onSelect(node, additive)}
        onDoubleClick={() => onEnterContext(node)}
      />
    )
  }

  // Group: show folder + expand/collapse + children
  // eslint-disable-next-line react-hooks/exhaustive-deps
  const members = useMemo(
    () => scene.group_members(node.id).map(nodeRefFromJs),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [scene, node.id, docRev],
  )

  return (
    <>
      <Row
        label={entityLabel('group', index)}
        selected={selected}
        active={active}
        dimmed={dimmed}
        indent={depth}
        isGroup
        expanded={expanded}
        onToggleExpand={() => setExpanded((e) => !e)}
        onClick={(additive) => onSelect(node, additive)}
        onDoubleClick={() => onEnterContext(node)}
      />
      {expanded && members.map((child, childIdx) => (
        <NodeRow
          key={`${child.kind}:${child.id}`}
          node={child}
          index={childIdx}
          depth={depth + 1}
          scene={scene}
          docRev={docRev}
          watertightMap={watertightMap}
          activeContext={activeContext}
          deepestCtx={deepestCtx}
          isSelected={isSelected}
          onSelect={onSelect}
          onEnterContext={onEnterContext}
        />
      ))}
    </>
  )
}

function BoolButton({ label, title, onClick }: { label: string; title: string; onClick: () => void }) {
  return (
    <button
      onClick={onClick}
      title={title}
      style={{
        flex: 1,
        padding: '4px 2px',
        fontSize: '11px',
        fontFamily: 'monospace',
        cursor: 'pointer',
        background: '#46618c',
        color: '#eee',
        border: 'none',
        borderRadius: '3px',
      }}
    >
      {label}
    </button>
  )
}

function ActionButton({ label, title, onClick }: { label: string; title: string; onClick: () => void }) {
  return (
    <button
      onClick={onClick}
      title={title}
      style={{
        flex: 1,
        padding: '4px 2px',
        fontSize: '11px',
        fontFamily: 'monospace',
        cursor: 'pointer',
        background: '#4a6840',
        color: '#eee',
        border: 'none',
        borderRadius: '3px',
      }}
    >
      {label}
    </button>
  )
}

function Section({ title, empty, children }: { title: string; empty: string; children: React.ReactNode }) {
  const items = Array.isArray(children) ? children : [children]
  const isEmpty = items.flat().filter(Boolean).length === 0
  return (
    <div>
      <div style={{ fontSize: '10px', textTransform: 'uppercase', letterSpacing: '0.06em', color: '#888', marginBottom: '4px' }}>
        {title}
      </div>
      {isEmpty ? (
        <div style={{ fontSize: '11px', color: '#666', fontStyle: 'italic', padding: '2px 8px' }}>{empty}</div>
      ) : (
        children
      )}
    </div>
  )
}

function Row({
  label,
  selected,
  active,
  dimmed,
  indent,
  dot,
  isGroup,
  isInstance,
  expanded,
  onToggleExpand,
  onClick,
  onDoubleClick,
}: {
  label: string
  selected: boolean
  active: boolean
  dimmed: boolean
  indent: number
  dot?: string
  isGroup?: boolean
  isInstance?: boolean
  expanded?: boolean
  onToggleExpand?: () => void
  onClick: (additive: boolean) => void
  onDoubleClick?: () => void
}) {
  const background = active ? '#3a567f' : selected ? '#39507040' : 'transparent'
  return (
    <div
      onClick={(e) => onClick(e.shiftKey || e.ctrlKey || e.metaKey)}
      onDoubleClick={onDoubleClick}
      style={{
        ...ROW_BASE,
        paddingLeft: `${8 + indent * 16}px`,
        background,
        opacity: dimmed ? 0.5 : 1,
        fontWeight: active ? 'bold' : 'normal',
      }}
    >
      {isGroup === true && (
        <button
          onClick={(e) => {
            e.stopPropagation()
            onToggleExpand?.()
          }}
          style={{
            background: 'none',
            border: 'none',
            color: '#aaa',
            cursor: 'pointer',
            padding: '0 2px',
            fontSize: '10px',
            lineHeight: 1,
          }}
        >
          {expanded === true ? '▾' : '▸'}
        </button>
      )}
      {/* Folder icon for groups */}
      {isGroup === true && (
        <span style={{ fontSize: '11px', color: '#e8c84a' }}>▤</span>
      )}
      {/* Component/instance icon */}
      {isInstance === true && (
        <span style={{ fontSize: '11px', color: '#7acce8' }}>⬡</span>
      )}
      {dot !== undefined && (
        <span style={{ width: '7px', height: '7px', borderRadius: '50%', background: dot, flexShrink: 0 }} />
      )}
      <span>{label}</span>
      {active && <span style={{ marginLeft: 'auto', fontSize: '10px', color: '#cfe0f5' }}>editing</span>}
    </div>
  )
}

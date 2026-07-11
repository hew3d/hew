/**
 * DocumentTree — the document outliner ( navigation).
 *
 * One unified tree list: the document's top-level nodes (Objects, Groups,
 * Component instances — recursive, with expand/collapse for groups) followed
 * by free-standing sketches as ordinary rows in the same list. Breadcrumb
 * shows the active context path.
 *
 * Click to select; double-click to enter context. Structural actions
 * (booleans, group/ungroup, component ops) live in the menus/dock — this
 * panel is purely navigational. Node types are distinguished by small
 * stroke-based inline SVG icons tinted per type (see NodeIcon).
 */

import { useMemo, useState, useEffect, useRef } from 'react'
import type { Scene as WasmScene } from '../wasm/loader'
import {
  entityLabel,
  resolveLabel,
  breadcrumb,
  buildTreeIndexMap,
  isTreeRowDimmed,
  nodeRefFromJs,
  nodeKey,
  type NodeRef,
  type NodeKind,
} from './treeModel'

interface Props {
  scene: WasmScene
  /** Bumped by the parent on any document change to trigger a re-query. */
  docRev: number
  /** Per-object watertight state, for the solid/leaky icon state. */
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
  /** Set of nodeKey strings for nodes that are currently hidden. */
  hiddenKeys: Set<string>
  /** Toggle hide/show for a node (and its descendants if it's a group). */
  onToggleHidden: (node: NodeRef) => void
}

const ROW_BASE: React.CSSProperties = {
  display: 'flex',
  alignItems: 'center',
  gap: '6px',
  padding: '3px 8px',
  fontSize: '12px',
  fontFamily: 'var(--font-family-ui)',
  color: 'var(--text-secondary, #ccc)',
  cursor: 'pointer',
  borderRadius: '3px',
  userSelect: 'none',
  minWidth: 0,
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
  hiddenKeys,
  onToggleHidden,
}: Props) {
  // Re-query the entity lists whenever the document changes.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  const topNodes = useMemo(
    () => scene.top_level_nodes().map(nodeRefFromJs),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [scene, docRev],
  )
  // eslint-disable-next-line react-hooks/exhaustive-deps
  // One outliner row per ISLAND (connected shape), numbered across all
  // sketches — the user-facing unit; two shapes drawn apart get two rows.
  const sketches = useMemo(
    () =>
      Array.from(scene.sketch_ids()).flatMap((sid) =>
        Array.from(scene.sketch_island_ids(sid)).map((island) => ({ sketch: sid, island })),
      ),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [scene, docRev],
  )

  const selected = new Set(selectedIds.map((n) => nodeKey(n)))
  // A selected line/curve/island has no dedicated row beyond the island's —
  // light up the owning ISLAND's row so the outliner reflects the selection.
  for (const n of selectedIds) {
    if (n.sketch === undefined) continue
    if (n.kind === 'sketch-island') continue // has its own row key already
    let island: bigint | undefined
    if (n.kind === 'sketch-edge') {
      island = scene.sketch_edge_island(n.sketch, n.id)
    } else if (n.kind === 'sketch-curve') {
      // The ref's id is the chain's representative edge.
      island = scene.sketch_edge_island(n.sketch, n.id)
    }
    if (island !== undefined) {
      selected.add(nodeKey({ kind: 'sketch-island', id: island, sketch: n.sketch }))
    }
  }
  const isSelected = (n: NodeRef) => selected.has(nodeKey(n))

  // Primary selection for scroll-into-view: stable ref so the effect only
  // fires when the primary selection actually changes (not on docRev bumps).
  const primaryKey = selectedIds.length > 0 ? nodeKey(selectedIds[0]) : null
  const primaryKeyRef = useRef<string | null>(null)
  const selectedRowRef = useRef<HTMLDivElement>(null)
  useEffect(() => {
    if (primaryKey === primaryKeyRef.current) return
    // Only advance the ref once we've actually scrolled. If the selected node is
    // inside a collapsed group, its row isn't mounted on this pass; a child
    // NodeRow auto-expands and re-renders, and we scroll on the next pass.
    if (selectedRowRef.current !== null) {
      primaryKeyRef.current = primaryKey
      selectedRowRef.current.scrollIntoView({ block: 'nearest' })
    } else if (primaryKey === null) {
      primaryKeyRef.current = null
    }
  })

  // Compute the set of group ancestor keys for the primary selected node so
  // those groups can be auto-expanded when they're collapsed.
  const ancestorGroupKeys = useMemo(() => {
    const keys = new Set<string>()
    if (selectedIds.length === 0) return keys
    const primary = selectedIds[0]
    // Sketch-scoped selections are always top-level with no kernel NodeId —
    // no ancestors.
    if (
      primary.kind === 'sketch' ||
      primary.kind === 'sketch-island' ||
      primary.kind === 'sketch-curve' ||
      primary.kind === 'sketch-edge'
    ) {
      return keys
    }
    // Walk up the parent chain from the primary node.
    const kindNum = primary.kind === 'object' ? 0 : primary.kind === 'group' ? 1 : 2
    let parentId = scene.node_parent(kindNum, primary.id)
    while (parentId !== undefined) {
      keys.add(nodeKey({ kind: 'group', id: parentId }))
      parentId = scene.node_parent(1, parentId)
    }
    return keys
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selectedIds, scene, docRev])

  const deepestCtx = activeContext.length > 0 ? activeContext[activeContext.length - 1] : null

  // Positional indices for breadcrumb labels: position within the parent
  // container, so a crumb for an unnamed nested node reads exactly like its
  // tree row — the flat per-kind id lists disagree with that as soon as
  // containers nest. Memoized: building it is a full group_members traversal
  // across the WASM boundary, far too heavy to run once per crumb per render.
  const treeIndex = useMemo(
    () =>
      buildTreeIndexMap(topNodes, (groupId) =>
        scene.group_members(groupId).map(nodeRefFromJs),
      ),
    // docRev: membership changes on every mutation without changing identity.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [topNodes, scene, docRev],
  )

  // Label resolver for breadcrumbs.
  const labelFor = (node: NodeRef): string => {
    const idx = treeIndex.get(nodeKey(node)) ?? 0
    if (node.kind === 'group') {
      return resolveLabel(scene.group_name(node.id), undefined, 'group', idx)
    } else if (node.kind === 'instance') {
      const def = scene.instance_def(node.id)
      const defName = def !== undefined ? scene.component_name(def) : undefined
      return resolveLabel(scene.instance_name(node.id), defName, 'instance', idx)
    } else {
      return resolveLabel(scene.object_name(node.id), undefined, 'object', idx)
    }
  }

  const crumbs = breadcrumb(activeContext, labelFor)

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: '8px' }}>
      {/* Breadcrumb */}
      <div style={{ display: 'flex', alignItems: 'center', flexWrap: 'wrap', gap: '2px', fontSize: '12px', fontFamily: 'var(--font-family-ui)' }}>
        {crumbs.map((c, i) => (
          <span key={i} style={{ display: 'flex', alignItems: 'center', gap: '2px' }}>
            {i > 0 && <span style={{ color: 'var(--text-faint, #777)' }}>›</span>}
            {i === crumbs.length - 1 ? (
              <span style={{ color: 'var(--text-primary, #fff)', fontWeight: 'bold' }}>{c.label}</span>
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
                  color: 'var(--accent-base, #7aa7e0)',
                  cursor: 'pointer',
                  fontFamily: 'var(--font-family-ui)',
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

      {/* Unified node tree: top-level nodes first, then free-standing sketches.
          An empty document renders no rows at all — no placeholder text. */}
      <div>
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
            primaryKey={primaryKey}
            selectedRowRef={selectedRowRef}
            ancestorGroupKeys={ancestorGroupKeys}
            hiddenKeys={hiddenKeys}
            onToggleHidden={onToggleHidden}
            onSelect={onSelect}
            onEnterContext={onEnterContext}
          />
        ))}
        {sketches.map(({ sketch, island }, index) => {
          const node: NodeRef = { kind: 'sketch-island', id: island, sketch }
          return (
            <Row
              key={`${sketch}:${island}`}
              label={entityLabel('sketch', index)}
              icon={<NodeIcon kind="sketch" />}
              selected={isSelected(node)}
              isPrimary={primaryKey === nodeKey(node)}
              active={false}
              dimmed={activeContext.length > 0}
              indent={0}
              rowRef={primaryKey === nodeKey(node) ? selectedRowRef : undefined}
              onClick={(additive) => onSelect(node, additive)}
            />
          )
        })}
      </div>
    </div>
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
  primaryKey,
  selectedRowRef,
  ancestorGroupKeys,
  hiddenKeys,
  onToggleHidden,
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
  primaryKey: string | null
  selectedRowRef: React.RefObject<HTMLDivElement>
  ancestorGroupKeys: Set<string>
  hiddenKeys: Set<string>
  onToggleHidden: (node: NodeRef) => void
  onSelect: (n: NodeRef, additive: boolean) => void
  onEnterContext: (n: NodeRef) => void
}) {
  // Auto-expand when this group is an ancestor of the primary selected node.
  const isAncestor = node.kind === 'group' && ancestorGroupKeys.has(nodeKey(node))
  // Nested containers start COLLAPSED — an outliner full of pre-expanded
  // hierarchy is noise; the auto-expand effect below still opens the
  // ancestors of whatever is selected.
  const [expanded, setExpanded] = useState(false)
  // Force expand when this group is in the ancestor path of the primary selection.
  useEffect(() => {
    if (isAncestor) setExpanded(true)
  }, [isAncestor])

  const selected = isSelected(node)
  const isPrimary = primaryKey !== null && nodeKey(node) === primaryKey
  const active = deepestCtx !== null &&
    deepestCtx.kind === node.kind && deepestCtx.id === node.id
  const dimmed = isTreeRowDimmed(activeContext, node, depth)
  const hidden = hiddenKeys.has(nodeKey(node))

  if (node.kind === 'object') {
    const watertight = watertightMap.get(node.id) ?? true
    return (
      <Row
        label={resolveLabel(scene.object_name(node.id), undefined, 'object', index)}
        icon={<NodeIcon kind="object" solid={watertight} />}
        selected={selected}
        isPrimary={isPrimary}
        active={active}
        dimmed={dimmed}
        hidden={hidden}
        indent={depth}
        rowRef={isPrimary ? selectedRowRef : undefined}
        onClick={(additive) => onSelect(node, additive)}
        onDoubleClick={() => onEnterContext(node)}
        onToggleHidden={() => onToggleHidden(node)}
      />
    )
  }

  if (node.kind === 'instance') {
    const def = scene.instance_def(node.id)
    const defName = def !== undefined ? scene.component_name(def) : undefined
    return (
      <Row
        label={resolveLabel(scene.instance_name(node.id), defName, 'instance', index)}
        icon={<NodeIcon kind="instance" />}
        selected={selected}
        isPrimary={isPrimary}
        active={active}
        dimmed={dimmed}
        hidden={hidden}
        indent={depth}
        rowRef={isPrimary ? selectedRowRef : undefined}
        onClick={(additive) => onSelect(node, additive)}
        onDoubleClick={() => onEnterContext(node)}
        onToggleHidden={() => onToggleHidden(node)}
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
        label={resolveLabel(scene.group_name(node.id), undefined, 'group', index)}
        icon={<NodeIcon kind="group" />}
        selected={selected}
        isPrimary={isPrimary}
        active={active}
        dimmed={dimmed}
        hidden={hidden}
        indent={depth}
        isGroup
        expanded={expanded}
        onToggleExpand={() => setExpanded((e) => !e)}
        rowRef={isPrimary ? selectedRowRef : undefined}
        onClick={(additive) => onSelect(node, additive)}
        onDoubleClick={() => onEnterContext(node)}
        onToggleHidden={() => onToggleHidden(node)}
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
          primaryKey={primaryKey}
          selectedRowRef={selectedRowRef}
          ancestorGroupKeys={ancestorGroupKeys}
          hiddenKeys={hiddenKeys}
          onToggleHidden={onToggleHidden}
          onSelect={onSelect}
          onEnterContext={onEnterContext}
        />
      ))}
    </>
  )
}

// ---------------------------------------------------------------------------
// NodeIcon — 14px stroke-based inline SVG per node type.
//
// Matches the minimal line-icon language of the toolbar (thin, geometric,
// currentColor) while staying quiet: each type gets a subtle theme-aware tint
// via CSS vars, applied as the SVG's color so `currentColor` picks it up.
//   object   — isometric cube; solid = solid outline (--status-solid),
//              leaky = dashed outline (--status-leaky)
//   group    — folder outline (--glyph-group)
//   instance — hexagon with a center definition dot (--glyph-instance)
//   sketch   — pen curve (--glyph-sketch)
// ---------------------------------------------------------------------------

const ICON_SVG_PROPS = {
  width: 14,
  height: 14,
  viewBox: '0 0 14 14',
  fill: 'none',
  stroke: 'currentColor',
  strokeWidth: 1.2,
  strokeLinecap: 'round' as const,
  strokeLinejoin: 'round' as const,
  'aria-hidden': true,
  style: { flexShrink: 0, display: 'block' } as React.CSSProperties,
}

export function NodeIcon({ kind, solid }: { kind: NodeKind; solid?: boolean }) {
  if (kind === 'object') {
    const leaky = solid === false
    return (
      <svg
        {...ICON_SVG_PROPS}
        data-node-icon={leaky ? 'object-leaky' : 'object-solid'}
        style={{ ...ICON_SVG_PROPS.style, color: leaky ? 'var(--status-leaky)' : 'var(--status-solid)' }}
      >
        <path
          d="M7 1.4 12.1 4.2 12.1 9.8 7 12.6 1.9 9.8 1.9 4.2 Z"
          strokeDasharray={leaky ? '2 1.7' : undefined}
        />
        <path d="M1.9 4.2 7 7 12.1 4.2 M7 7 7 12.6" strokeDasharray={leaky ? '2 1.7' : undefined} />
      </svg>
    )
  }
  if (kind === 'group') {
    return (
      <svg
        {...ICON_SVG_PROPS}
        data-node-icon="group"
        style={{ ...ICON_SVG_PROPS.style, color: 'var(--glyph-group)' }}
      >
        <path d="M1.7 4.6v6a1 1 0 0 0 1 1h8.6a1 1 0 0 0 1-1V5.9a1 1 0 0 0-1-1H7.1L5.7 3.4H2.7a1 1 0 0 0-1 1Z" />
      </svg>
    )
  }
  if (kind === 'instance') {
    return (
      <svg
        {...ICON_SVG_PROPS}
        data-node-icon="instance"
        style={{ ...ICON_SVG_PROPS.style, color: 'var(--glyph-instance)' }}
      >
        <path d="M7 1.6 11.7 4.3 11.7 9.7 7 12.4 2.3 9.7 2.3 4.3 Z" />
        <circle cx="7" cy="7" r="1.3" fill="currentColor" stroke="none" />
      </svg>
    )
  }
  // sketch
  return (
    <svg
      {...ICON_SVG_PROPS}
      data-node-icon="sketch"
      style={{ ...ICON_SVG_PROPS.style, color: 'var(--glyph-sketch)' }}
    >
      <path d="M2 12c1.4-5.6 5.4-1.8 10-10" />
      <circle cx="2" cy="12" r="0.9" fill="currentColor" stroke="none" />
    </svg>
  )
}

function Row({
  label,
  icon,
  selected,
  isPrimary,
  active,
  dimmed,
  hidden,
  indent,
  isGroup,
  expanded,
  onToggleExpand,
  rowRef,
  onClick,
  onDoubleClick,
  onToggleHidden,
}: {
  label: string
  icon: React.ReactNode
  selected: boolean
  isPrimary?: boolean
  active: boolean
  dimmed: boolean
  hidden?: boolean
  indent: number
  isGroup?: boolean
  expanded?: boolean
  onToggleExpand?: () => void
  rowRef?: React.Ref<HTMLDivElement>
  onClick: (additive: boolean) => void
  onDoubleClick?: () => void
  onToggleHidden?: () => void
}) {
  // Selection highlight uses the theme accent tint (06_docked_panels.md: "the
  // selected node is highlighted with accent/tint background + accent text"),
  // not the old hardcoded blue bars that broke on the light theme. Three tiers:
  // the active (being-edited) row gets an inset accent rail; primary selection
  // the full tint; secondary selection a fainter tint.
  const anySelected = active || isPrimary === true || selected
  const background = active || isPrimary === true
    ? 'var(--accent-tint-18)'
    : selected
      ? 'var(--accent-tint-15)'
      : 'transparent'

  return (
    <div
      ref={rowRef}
      onClick={(e) => onClick(e.shiftKey || e.ctrlKey || e.metaKey)}
      onDoubleClick={onDoubleClick}
      style={{
        ...ROW_BASE,
        paddingLeft: `${8 + indent * 16}px`,
        paddingRight: '4px',
        background,
        boxShadow: active ? 'inset 2px 0 0 var(--accent-base)' : 'none',
        color: anySelected ? 'var(--accent-text-on-tint)' : undefined,
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
            color: 'var(--text-tertiary, #aaa)',
            cursor: 'pointer',
            padding: '0 2px',
            fontSize: '10px',
            lineHeight: 1,
          }}
        >
          {expanded === true ? '▾' : '▸'}
        </button>
      )}
      {icon}
      <span style={{ flex: 1, minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', color: hidden === true ? 'var(--text-faint, #666)' : undefined }}>{label}</span>
      {active && <span style={{ fontSize: '10px', color: 'var(--accent-text-on-tint)' }}>editing</span>}
      {/* Eye toggle — only visible on hover via CSS would require class, so always show */}
      {onToggleHidden !== undefined && (
        <button
          onClick={(e) => {
            e.stopPropagation()
            onToggleHidden()
          }}
          title={hidden === true ? 'Show' : 'Hide'}
          style={{
            background: 'none',
            border: 'none',
            color: hidden === true ? 'var(--text-section)' : 'var(--text-muted)',
            cursor: 'pointer',
            padding: '0 2px',
            fontSize: '11px',
            lineHeight: 1,
            flexShrink: 0,
          }}
        >
          {hidden === true ? '○' : '●'}
        </button>
      )}
    </div>
  )
}

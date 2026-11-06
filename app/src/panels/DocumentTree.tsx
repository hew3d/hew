/**
 * DocumentTree — the document outliner ( navigation).
 *
 * One unified tree list: a non-selectable root "Model" row, then the
 * document's top-level nodes (Objects, Groups, Component instances —
 * recursive, with expand/collapse for groups) followed by free-standing
 * sketches as ordinary rows in the same list. Breadcrumb shows the combined
 * path: Model → open session frames (outermost first) → object context
 * (docs/design/group-session.md).
 *
 * Click to select; double-click to enter context. Structural actions
 * (booleans, group/ungroup, component ops) live in the menus/dock — this
 * panel is purely navigational plus visibility (the eye toggles) and a text
 * filter. Node types are distinguished by small stroke-based inline SVG
 * icons tinted per type (see NodeIcon).
 */

import { Trans, useLingui } from '@lingui/react/macro'
import { useMemo, useState, useEffect, useRef, useCallback, memo } from 'react'
import type { Scene as WasmScene } from '../wasm/loader'
import {
  entityLabel,
  resolveLabel,
  makeLabelFor,
  breadcrumb,
  buildTreeIndexMap,
  isTreeRowDimmed,
  nodeRefFromJs,
  nodeKey,
  nodeKindToNumber,
  collectDescendants,
  filterTreeKeys,
  dropTargetFor,
  sortOutlinerLevel,
  type NodeRef,
  type NodeKind,
} from './treeModel'
import { draftRef, outlinerDrawing, resolveDraft, type OutlinerDrawing } from '../tools/planeUnits'
import { useIsRtl } from '../i18n/direction'
import { isSketchScopedKind } from './treeModel'
import type { HealthClass } from '../health/healthReport'
import { OUTLINER_TOOLTIP } from '../health/healthCopy'
import {
  getOutlinerSort,
  subscribeOutlinerSort,
  setOutlinerSort,
  type OutlinerSort,
} from '../settings/view'
import { OutlinerSortToggle } from './OutlinerSortToggle'
import { useViewSetting } from '../settings/useViewSetting'

interface Props {
  scene: WasmScene
  /** Bumped by the parent on any document change to trigger a re-query. */
  docRev: number
  /** Bumped by the parent on every File ▸ New / Open (App.tsx's
   *  `applyLoadedBytes`) — a NEW document, not just a mutation of this one.
   *  Resets `expandedMap` (below) when it changes: handles are dense and
   *  generational per-document, so a `kind:id` key left over from the
   *  PREVIOUS document can alias an unrelated node in the new one (the same
   *  hazard `hiddenKeys` is reset for on load) — a stale key surviving here
   *  would force-expand or misrender a node that just happens to reuse that
   *  slot. */
  docGeneration: number
  /** Per-object watertight state, the icon's fallback when `objectHealth`
   *  is not supplied. */
  watertightMap: Map<bigint, boolean>
  /** Per-object Model Health state for the icon: `surface` (a sheet drawn on
   *  purpose, or one marked as accepted) or `open` (needs attention). An
   *  object absent from the map is a solid. Comes from the health report, so
   *  component-definition members are covered too (the tessellation-derived
   *  `watertightMap` never sees them). */
  objectHealth?: Map<bigint, HealthClass>
  /** Selected nodes (ordered; index 0 = primary). */
  selectedIds: NodeRef[]
  /** Object-context path: app-only sticky editing of at most one plain
   *  object (or, via the K1/K2 fallback, a component instance), sitting
   *  logically inside the innermost open session frame if any. Empty =
   *  nothing pushed past the session stack. Combined with `sessionStack`
   *  below (session frames first, this tail last) for the breadcrumb/
   *  dimming/"editing"-chip path — see `fullPath` in the component body. */
  activeContext: NodeRef[]
  /** The open kernel session stack, outermost first — empty when nothing is
   *  open. Each frame's label is supplied by the caller (Viewport) since a
   *  session HIDES its own node (its name is unreadable through the
   *  ordinary `group_name`/`instance_name` queries once hidden — see
   *  `Viewport.tsx`'s `runOpenGroupSession`/`componentSessionLabel`). */
  sessionStack: { node: NodeRef; label: string }[]
  /** The innermost open session frame's current direct-member list (any
   *  node kind), or `null` when no session is open —
   *  `ViewportApi.sessionMembers()`, re-read by the parent whenever
   *  `docRev`/`sessionStack` change. While a session is open, its members
   *  are plain top-level nodes in the kernel (the ungroup posture /
   *  `Document::open_explode_session`'s bake alike) — left alone, they'd
   *  render as loose, unlabeled rows with no indication of what they are,
   *  while the group/instance row that used to explain them vanishes
   *  (hidden for the session's duration). Instead the tree nests them under
   *  the innermost frame's synthetic header row, generalizing the single-
   *  instance "explode session member list" it rendered before groups
   *  existed. */
  sessionMembers: NodeRef[] | null
  /** `additive` = shift/ctrl-click (multi-select). */
  onSelect: (node: NodeRef, additive: boolean) => void
  /** Entry convergence (docs/design/group-session.md): given ANY target
   *  node — including one buried behind session frames that aren't open
   *  yet — opens/closes exactly the frames needed and pushes an object
   *  context if the target is a plain object, converging on the same state
   *  shape a viewport double-click would reach one level at a time. */
  onEnterContext: (node: NodeRef) => void
  /** Root "Model" breadcrumb crumb: exit everything (every open session
   *  frame, innermost-first, plus the object-context tail). */
  onExitContext: () => void
  /** A non-root breadcrumb crumb click, at combined-path depth (session
   *  frames first, then the object-context tail): close/pop everything
   *  past it. */
  onSetContextDepth: (depth: number) => void
  /** Set of nodeKey strings for nodes that are currently hidden. */
  hiddenKeys: Set<string>
  /** Toggle hide/show for a single node (and its descendants if it's a group). */
  onToggleHidden: (node: NodeRef) => void
  /** Batch hide/show — the per-container "hide/show all children" control
   *  (every group row, plus the root Model row's own eye button, which has
   *  no single node of its own to toggle) and its E2E-facing counterpart.
   *  One Set mutation + one kernel push for the whole batch, not N
   *  individual toggles. */
  onSetHiddenMany: (nodes: NodeRef[], hidden: boolean) => void
  /** Outliner drag-and-drop: `nodes` (the dragged row, or the whole current
   *  selection when the dragged row was part of it) dropped onto `group` —
   *  a live group's id, or `undefined` for the Model root row (move to the
   *  top level). Only ever called with a drop `dropTargetFor` (treeModel.ts)
   *  already validated; the kernel's own refusal (a race with another
   *  mutation, say) still surfaces as a toast from the caller. */
  onReparent: (nodes: NodeRef[], group: bigint | undefined) => void
  /** A drag ended somewhere it could not drop: the reason, for a toast.
   *  Without it a refused drop (a group being edited, a group onto its own
   *  member, a sketch row) is indistinguishable from a drag that never
   *  registered. */
  onDropRefused?: (reason: string) => void
}

const EMPTY_KEYS: ReadonlySet<string> = new Set()

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

/**
 * Decide and run one "hide/show all children" click for a container row
 * (the root Model row, or a group row). State rule (design):
 * - If ANY descendant (or the container's own node, when it has one) is
 *   currently hidden, the action is "Show all": clear every descendant's
 *   hidden key plus the container's own.
 * - Otherwise the action is "Hide all": set every DIRECT child's own hidden
 *   key (descendants inherit visibility from their hidden direct ancestor —
 *   `unionHiddenLeafIds`'s recursive expansion already resolves that at
 *   push time, so only the direct children need their own key set).
 *
 * `ownNode` is `null` for the Model row (no NodeRef of its own to carry a
 * hidden key).
 */
function toggleContainerVisibility(
  ownNode: NodeRef | null,
  directChildren: NodeRef[],
  getGroupMembers: (groupId: bigint) => NodeRef[],
  hiddenKeys: Set<string>,
  onSetHiddenMany: (nodes: NodeRef[], hidden: boolean) => void,
): void {
  const descendants = collectDescendants(directChildren, getGroupMembers)
  const anyHidden =
    (ownNode !== null && hiddenKeys.has(nodeKey(ownNode))) ||
    descendants.some((d) => hiddenKeys.has(nodeKey(d)))
  if (anyHidden) {
    onSetHiddenMany(ownNode !== null ? [ownNode, ...descendants] : descendants, false)
  } else {
    onSetHiddenMany(directChildren, true)
  }
}

/** Whether `node` is hidden only because an ANCESTOR group is hidden (not
 *  its own key) — walks the parent chain via `node_parent`, mirroring the
 *  ancestor walk `DocumentTree`'s selection auto-expand already does. Used
 *  to render the eye as "hidden by parent" (dimmed ○) rather than the
 *  ordinary hidden ○, per the design's "eye rendering fix". */
function isHiddenByAncestor(node: NodeRef, scene: WasmScene, hiddenKeys: Set<string>): boolean {
  if (node.kind !== 'object' && node.kind !== 'group' && node.kind !== 'instance') return false
  const kindNum = nodeKindToNumber(node.kind)
  let parentId = scene.node_parent(kindNum, node.id)
  while (parentId !== undefined) {
    if (hiddenKeys.has(nodeKey({ kind: 'group', id: parentId }))) return true
    parentId = scene.node_parent(1, parentId)
  }
  return false
}

export function DocumentTree({
  scene,
  docRev,
  docGeneration,
  watertightMap,
  objectHealth,
  selectedIds,
  activeContext,
  sessionStack,
  sessionMembers,
  onSelect,
  onEnterContext,
  onExitContext,
  onSetContextDepth,
  hiddenKeys,
  onToggleHidden,
  onSetHiddenMany,
  onReparent,
  onDropRefused,
}: Props) {
  const { t } = useLingui()
  // Re-query the entity lists whenever the document changes.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  const topNodes = useMemo(
    () => scene.top_level_nodes().map(nodeRefFromJs),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [scene, docRev],
  )
  const getGroupMembers = (groupId: bigint): NodeRef[] =>
    scene.group_members(groupId).map(nodeRefFromJs)
  const sessionMemberKeySet = useMemo(
    () => (sessionMembers === null ? null : new Set(sessionMembers.map(nodeKey))),
    [sessionMembers],
  )
  // Top-level array index per node key — the positional-label index a
  // session member keeps when rendered nested under the innermost frame's
  // header (see the nested NodeRow's `index` prop for why).
  const topIndexByKey = useMemo(() => {
    const m = new Map<string, number>()
    topNodes.forEach((n, i) => m.set(nodeKey(n), i))
    return m
  }, [topNodes])
  // eslint-disable-next-line react-hooks/exhaustive-deps
  // Where every drawn shape lists: under its object when any of its edges is
  // face topology or an object-owned sketch, else (entirely over the void)
  // as a top-level sketch row, numbered across all sketches — the user-facing
  // unit; two shapes drawn apart get two rows. One kernel read per document
  // revision, shared by every row.
  const drawing = useMemo(
    () => outlinerDrawing(scene as unknown as Parameters<typeof outlinerDrawing>[0], Array.from(scene.object_ids())),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [scene, docRev],
  )
  const sketches = drawing.free
  // A row's number among the sketch rows, looked up rather than searched
  // for: a search per row is quadratic, and a model of tens of thousands
  // of stray lines has that many rows.
  const sketchIndex = useMemo(() => {
    const m = new Map<string, number>()
    sketches.forEach((s, i) => m.set(nodeKey(s.ref), i))
    return m
  }, [sketches])

  const selected = new Set(selectedIds.map((n) => nodeKey(n)))
  // A selected line/curve has no dedicated row beyond its shape's — light up
  // the owning SHAPE's row so the outliner reflects the selection.
  for (const n of selectedIds) {
    if (n.kind !== 'draft-edge' && n.kind !== 'draft-curve') continue
    const unit = resolveDraft(scene as unknown as Parameters<typeof resolveDraft>[0], n)?.unit
    if (unit !== undefined && unit !== null && n.ctx !== undefined) {
      selected.add(nodeKey(draftRef('draft-island', n.ctx, unit.key)))
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

  // Session-stack reveal: scroll the row for the frame that just opened (the
  // stack grew) or the node that just closed (the stack shrank) into view —
  // the same "stranded scroll position" bug as the primary-selection case
  // above, on the session-frame axis instead (playtest: entering a component
  // re-roots the tree far from wherever the scroll happened to sit; leaving
  // does the reverse, landing back on rows that no longer show anything
  // "editing"). This needs its OWN target rather than reusing
  // `primaryKey`/`selectedRowRef`: App.tsx's `handleSessionChange` clears
  // `selectedIds` right at a session boundary (opening or closing always
  // changes which frame is innermost), so by the time `sessionStack` here
  // reflects the change, `selectedIds` has already gone empty and carries no
  // information about which row to reveal.
  //
  // `sessionRevealKey` names the target row:
  //   - grow (open — including drilling ONE level deeper while already
  //     inside a session; same shape, one more frame appended): tagged
  //     `frame:<nodeKey>` and matched against the just-opened frame's HEADER
  //     row below (the synthetic "editing" row from `sessionStack.map`), not
  //     a first-member row. The header always renders immediately, even for
  //     a frame with no live members yet, and sits directly above its
  //     members, so revealing it brings the members into view too; a
  //     first-member target would need special-casing an empty frame.
  //   - shrink (close): tagged `node:<nodeKey>` and matched against the
  //     closed frame's own node once it reappears as an ordinary row — a
  //     `topNodes` entry, or a member of whichever frame is now innermost.
  //     Either way it renders unconditionally, never behind a COLLAPSED
  //     plain group's expand toggle: every node a session can be opened on
  //     got there with its whole ancestor chain already open, either as
  //     session frames themselves or as members of one (both this
  //     component's entry convergence and the viewport's own one-level-at-
  //     a-time double click only ever open a session whose parent chain is
  //     already open) — so no ancestor auto-expand is needed here the way
  //     `ancestorGroupKeys` below provides it for selection.
  // A stack change that is neither a clean grow-by-append nor a clean
  // shrink-by-truncation (an undo/redo resync can replace the whole stack at
  // once — see `App.tsx`'s `handleSessionChange`) doesn't name a single row
  // that "moved" — leave the previous target alone rather than guess wrong.
  //
  // Diffed here, during render, against the LAST stack this component
  // actually rendered (`prevSessionStackKeys` is STATE, not a ref — calling
  // `setState` conditionally, straight in the render body, is the React-
  // blessed way to derive state from a prop change: it lets the freshly
  // computed key take effect in the SAME render pass that first mounts the
  // target row, rather than one render late).
  const [prevSessionStackKeys, setPrevSessionStackKeys] = useState<string[]>([])
  const [sessionRevealKey, setSessionRevealKey] = useState<string | null>(null)
  const nextSessionStackKeys = sessionStack.map((f) => nodeKey(f.node))
  const sessionStackUnchanged =
    nextSessionStackKeys.length === prevSessionStackKeys.length &&
    nextSessionStackKeys.every((k, i) => k === prevSessionStackKeys[i])
  if (!sessionStackUnchanged) {
    const grew = nextSessionStackKeys.length > prevSessionStackKeys.length &&
      prevSessionStackKeys.every((k, i) => k === nextSessionStackKeys[i])
    const shrank = nextSessionStackKeys.length < prevSessionStackKeys.length &&
      nextSessionStackKeys.every((k, i) => k === prevSessionStackKeys[i])
    setPrevSessionStackKeys(nextSessionStackKeys)
    if (grew) {
      setSessionRevealKey(`frame:${nextSessionStackKeys[nextSessionStackKeys.length - 1]}`)
    } else if (shrank) {
      setSessionRevealKey(`node:${prevSessionStackKeys[nextSessionStackKeys.length]}`)
    }
  }
  const sessionRevealRowRef = useRef<HTMLDivElement>(null)
  useEffect(() => {
    if (sessionRevealKey === null) return
    // Same retry-until-mounted shape as the primary-selection effect above,
    // for the rare case the target row isn't mounted on this exact pass yet.
    const el = sessionRevealRowRef.current
    if (el === null) return
    // Optional call, not a plain invocation: this fires on every session
    // open/close, including in App-level tests that exercise the session
    // stack without caring about scroll position and so never polyfill
    // jsdom's missing `scrollIntoView` the way the panel's own tests do.
    el.scrollIntoView?.({ block: 'nearest' })
    setSessionRevealKey(null)
  })

  // Compute the union of group ancestor keys over EVERY selected node so
  // those groups can be auto-expanded when they're collapsed. Walking only
  // the primary's chain would leave the rest of a multi-selection (marquee,
  // Select All, Object Info's "(N instances)" click) hidden inside collapsed
  // groups. Cheap even for large selections: one parent-chain walk per node,
  // cut short as soon as it rejoins a chain already in the set.
  const ancestorGroupKeys = useMemo(() => {
    const keys = new Set<string>()
    for (const node of selectedIds) {
      // Sketch-scoped selections have no kernel NodeId — no ancestors.
      if (isSketchScopedKind(node.kind)) continue
      // Walk up the parent chain from this node.
      const kindNum = node.kind === 'object' ? 0 : node.kind === 'group' ? 1 : 2
      let parentId = scene.node_parent(kindNum, node.id)
      while (parentId !== undefined) {
        const key = nodeKey({ kind: 'group', id: parentId })
        if (keys.has(key)) break // rest of this chain is already in the set
        keys.add(key)
        parentId = scene.node_parent(1, parentId)
      }
    }
    return keys
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selectedIds, scene, docRev])

  // The combined path every breadcrumb/dimming/"editing"-chip computation
  // walks: open session frames outermost-first, then the object-context
  // tail (docs/design/group-session.md). `treeModel`'s `breadcrumb`/
  // `isTreeRowDimmed` are already generic over `NodeRef[]` — no changes
  // needed there, just feeding them this combined path instead of
  // `activeContext` alone.
  const fullPath = useMemo(
    () => [...sessionStack.map((f) => f.node), ...activeContext],
    [sessionStack, activeContext],
  )

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

  // A session frame's own node is HIDDEN in the kernel for the session's
  // duration (the ungroup posture / explode-session bake), so the ordinary
  // scene-query label resolution below answers nothing for it — the
  // caller-supplied label (captured before hiding) is the only source.
  const frameLabelByKey = useMemo(
    () => new Map(sessionStack.map((f) => [nodeKey(f.node), f.label])),
    [sessionStack],
  )

  // Label resolver for breadcrumbs AND the filter below (same text, so a
  // filter match is exactly what the row itself displays).
  const labelFor = useMemo(
    () => makeLabelFor(scene, treeIndex, frameLabelByKey, () => drawing),
    // docRev: names change without the scene identity changing.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [scene, treeIndex, frameLabelByKey, drawing, docRev],
  )

  // Row order at every level (a per-person setting, not saved in the file).
  // Display only: each row keeps the document-order index its generated
  // label ("Group 3") was numbered from, so sorting never renumbers.
  const sortMode: OutlinerSort = useViewSetting(getOutlinerSort, subscribeOutlinerSort)
  // Free sketches are top-level rows too: they sort with the rest (by the
  // "Sketch N" label they print; N is their stable position among sketches),
  // and in document order they follow the other top-level rows.
  const sortedTop = useMemo(
    () =>
      sortOutlinerLevel(
        [
          ...topNodes.map((node, index) => ({ node, index, free: false })),
          ...sketches.map((s, index) => ({ node: s.ref, index, free: true })),
        ],
        sortMode,
        (node) => {
          const at = sketchIndex.get(nodeKey(node))
          return at !== undefined ? entityLabel('sketch', at) : labelFor(node)
        },
      ),
    [topNodes, sketches, sketchIndex, sortMode, labelFor],
  )
  const sortedSessionMembers = useMemo(
    () =>
      sortOutlinerLevel(
        (sessionMembers ?? []).map((node, sessionIndex) => ({ node, sessionIndex })),
        sortMode,
        labelFor,
      ),
    [sessionMembers, sortMode, labelFor],
  )

  // ---------------------------------------------------------------------
  // Filter (MaterialPalette's filter pattern): case-insensitive substring
  // on the same label text the rows themselves render. While active, only
  // matches and their ancestors render; ancestors force-expand and dim.
  // ---------------------------------------------------------------------
  const [filter, setFilter] = useState('')
  const filterInputRef = useRef<HTMLInputElement>(null)
  const filterResult = useMemo(() => {
    // Drawn-shape rows filter by the same text they render (an object's
    // shapes are its children for this walk).
    const getChildren = (node: NodeRef): NodeRef[] =>
      node.kind === 'group'
        ? getGroupMembers(node.id)
        : node.kind === 'object'
          ? (drawing.byObject.get(node.id.toString()) ?? []).map((u) => u.ref)
          : []
    const labelForFiltered = (node: NodeRef): string => {
      if (isSketchScopedKind(node.kind)) return drawing.labels.get(nodeKey(node)) ?? ''
      return labelFor(node)
    }
    return filterTreeKeys(topNodes, getChildren, labelForFiltered, filter)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [topNodes, filter, scene, docRev, drawing])
  const filterActive = filterResult !== null
  const filterQuery = filter.trim().toLowerCase()
  // Sketches aren't part of the group-nested tree `filterTreeKeys` walks
  // (they're a flat, separately-numbered list) — filtered here directly
  // against the same "Sketch N" label each row renders.
  const visibleSketches = filterActive
    ? sketches.filter((_, index) => entityLabel('sketch', index).toLowerCase().includes(filterQuery))
    : sketches
  const filterEmpty =
    filterActive && (filterResult?.matches.size ?? 0) === 0 && visibleSketches.length === 0

  // Expand/collapse state, lifted out of each NodeRow's own local state
  // (design: a Map<key, boolean> here) so applying and then clearing a
  // filter doesn't reset whatever the user had manually expanded/collapsed
  // before it.
  const [expandedMap, setExpandedMap] = useState<Map<string, boolean>>(new Map())
  // Reset SYNCHRONOUSLY on a new document (docGeneration bump — File ▸ New
  // / Open), before this render reads `expandedMap`: handles are dense and
  // generational PER DOCUMENT, so a `kind:id` key left over from the
  // previous document (adversarial review finding 1) can alias an unrelated
  // node in the new one — force-expanding, or misapplying filter-ancestor
  // styling to, a node that just happens to reuse that slot. An effect-time
  // clear would run one render too late (the same reasoning DocumentTree's
  // own session-reveal-key derivation above uses, and MaterialPalette's
  // `thumbGenRef` docRev-generation guard uses for its thumbnail cache).
  const [expandedMapGeneration, setExpandedMapGeneration] = useState(docGeneration)
  if (expandedMapGeneration !== docGeneration) {
    setExpandedMapGeneration(docGeneration)
    setExpandedMap(new Map())
  }
  // Stable identity (setExpandedMap, the only external dep, is itself a
  // stable setState function) — part of NodeRow's React.memo payoff below:
  // a freshly-recreated callback prop on every render would invalidate
  // memo for every row regardless of whether anything that row actually
  // renders from changed.
  const setNodeExpanded = useCallback((key: string, value: boolean) => {
    setExpandedMap((prev) => {
      if (prev.get(key) === value) return prev
      const next = new Map(prev)
      next.set(key, value)
      return next
    })
  }, [])

  // -----------------------------------------------------------------------
  // Drag-and-drop reparenting: pointer events (this codebase's own drag
  // convention — LibraryDialog's column-resize drag is the precedent, not
  // the HTML5 DnD API), so a plain click/double-click keeps working
  // unmodified — dragging only engages past a small movement threshold.
  //
  // The hovered row is resolved via `elementFromPoint` + a `data-drop-
  // target` attribute (the row's own `nodeKey`, or `"root"` for the Model
  // row) rather than per-row pointer-enter tracking, so it keeps working
  // even though the dragged row never calls `setPointerCapture` (capturing
  // would suppress every OTHER row's pointer-enter/leave while dragging).
  // Only object/group/instance rows and the Model row carry the attribute;
  // sketch rows deliberately don't, so hovering one resolves to "no
  // target" — refused, with no drop highlight, exactly like an unmarked
  // gap in the list.
  // -----------------------------------------------------------------------
  const sessionOpen = sessionStack.length > 0
  const dragRef = useRef<{
    dragged: NodeRef[]
    startX: number
    startY: number
    active: boolean
    lastHighlightKey: string | null
    cleanup: () => void
  } | null>(null)
  const [dropHighlightKey, setDropHighlightKey] = useState<string | null>(null)
  /** The drag ghost that follows the pointer while a row drag is active —
   *  the dragged names, so it is visible that something is being carried —
   *  plus the source rows' keys, dimmed in place. */
  const [dragGhost, setDragGhost] = useState<{ x: number; y: number; labels: string[]; keys: Set<string> } | null>(null)
  const dragSourceKeys = dragGhost?.keys ?? EMPTY_KEYS

  const resolveDropTarget = (
    clientX: number,
    clientY: number,
  ): { key: string; node: NodeRef | 'root' } | null => {
    const el = document.elementFromPoint(clientX, clientY)
    const rowEl = el instanceof Element ? el.closest('[data-drop-target]') : null
    const key = rowEl?.getAttribute('data-drop-target') ?? null
    if (key === null) return null
    if (key === 'root') return { key, node: 'root' }
    const sep = key.indexOf(':')
    if (sep < 0) return null
    return { key, node: { kind: key.slice(0, sep) as NodeKind, id: BigInt(key.slice(sep + 1)) } }
  }

  const endDrag = () => {
    dragRef.current?.cleanup()
    dragRef.current = null
    setDropHighlightKey(null)
    setDragGhost(null)
    document.body.style.cursor = ''
  }

  const handlePointerMove = (e: PointerEvent) => {
    const st = dragRef.current
    if (st === null) return
    if (!st.active) {
      if (Math.hypot(e.clientX - st.startX, e.clientY - st.startY) < 4) return
      st.active = true
    }
    setDragGhost({
      x: e.clientX,
      y: e.clientY,
      labels: st.dragged.map(labelFor),
      keys: new Set(st.dragged.map((n) => `${n.kind}:${n.id}`)),
    })
    const resolved = resolveDropTarget(e.clientX, e.clientY)
    const valid =
      resolved !== null &&
      dropTargetFor(st.dragged, resolved.node, { getGroupMembers, sessionOpen }) !== null
    const nextKey = valid ? (resolved as { key: string }).key : null
    if (st.lastHighlightKey !== nextKey) {
      st.lastHighlightKey = nextKey
      setDropHighlightKey(nextKey)
    }
    document.body.style.cursor = valid ? 'grabbing' : 'not-allowed'
  }

  const handlePointerUp = (e: PointerEvent) => {
    const st = dragRef.current
    if (st !== null && st.active) {
      const resolved = resolveDropTarget(e.clientX, e.clientY)
      const result =
        resolved !== null
          ? dropTargetFor(st.dragged, resolved.node, { getGroupMembers, sessionOpen })
          : null
      if (result !== null) {
        onReparent(st.dragged, result.group)
      } else if (sessionOpen) {
        onDropRefused?.(t`Close the group you are editing before moving items between groups.`)
      } else if (resolved !== null) {
        onDropRefused?.(
          resolved.node === 'root'
            ? t`That is already at the top level.`
            : t`Drop onto a group row, or onto Model to move to the top level — not into itself or its own member.`,
        )
      }
    }
    endDrag()
  }

  const handlePointerCancel = () => endDrag()

  // Stable identity (no external deps besides `startRowDrag`'s own
  // closures, captured fresh per invocation) so this teardown only ever
  // targets whatever listeners the CURRENTLY in-flight drag actually added.
  // A collapsed TraySection unmounts its contents, so a drag in flight when
  // the Outliner section itself collapses mid-drag must tear down through
  // here rather than `endDrag` (never called — no more pointerup to reach
  // it) — including the cursor, or `document.body`'s cursor is left stuck
  // at 'grabbing'/'not-allowed' with nothing left to clear it.
  useEffect(
    () => () => {
      if (dragRef.current !== null) {
        dragRef.current.cleanup()
        dragRef.current = null
        document.body.style.cursor = ''
      }
    },
    [],
  )

  const startRowDrag = (node: NodeRef, e: React.PointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return
    // With a group open for editing the kernel refuses every reparent
    // (ExplodeSessionScope), so nothing can be a valid target — but the
    // drag still runs so the not-allowed cursor and the drop toast say so,
    // instead of the row simply not moving.
    if (nodeKindToNumber(node.kind) < 0) return // sketch-scoped: no kernel NodeId
    // A text selection or the row's own image would start WebKit's native
    // drag on mouse movement and cancel the pointer stream mid-drag.
    e.preventDefault()
    if (dragRef.current !== null) return
    const dragged =
      isSelected(node) && selectedIds.length > 1
        ? selectedIds.filter((n) => nodeKindToNumber(n.kind) >= 0)
        : [node]
    if (dragged.length === 0) return
    const onMove = (ev: PointerEvent) => handlePointerMove(ev)
    const onUp = (ev: PointerEvent) => handlePointerUp(ev)
    const onCancel = () => handlePointerCancel()
    window.addEventListener('pointermove', onMove)
    window.addEventListener('pointerup', onUp)
    window.addEventListener('pointercancel', onCancel)
    dragRef.current = {
      dragged,
      startX: e.clientX,
      startY: e.clientY,
      active: false,
      lastHighlightKey: null,
      cleanup: () => {
        window.removeEventListener('pointermove', onMove)
        window.removeEventListener('pointerup', onUp)
        window.removeEventListener('pointercancel', onCancel)
      },
    }
  }

  const crumbs = breadcrumb(fullPath, labelFor)

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: '8px' }}>
      {dragGhost !== null && (
        <div
          data-testid="outliner-drag-ghost"
          style={{
            position: 'fixed',
            left: dragGhost.x + 14,
            top: dragGhost.y + 10,
            pointerEvents: 'none',
            zIndex: 1000,
            padding: '3px 8px',
            borderRadius: '4px',
            background: 'var(--surface-overlay, #2a2a2e)',
            border: '1px solid var(--accent-base)',
            color: 'var(--text-primary, #eee)',
            fontSize: '12px',
            fontFamily: 'var(--font-family-ui)',
            boxShadow: '0 2px 8px rgba(0,0,0,0.35)',
            whiteSpace: 'nowrap',
            opacity: 0.95,
          }}
        >
          {dragGhost.labels.length === 1
            ? dragGhost.labels[0]
            : `${dragGhost.labels[0]} +${dragGhost.labels.length - 1}`}
        </div>
      )}
      {/* Breadcrumb — every entry on `fullPath` gets a crumb, including
          every open session frame (docs/design/group-session.md). */}
      <div style={{ display: 'flex', alignItems: 'center', flexWrap: 'wrap', gap: '2px', fontSize: '12px', fontFamily: 'var(--font-family-ui)' }}>
        {crumbs.map((c, i) => (
          <span key={i} style={{ display: 'flex', alignItems: 'center', gap: '2px' }}>
            {i > 0 && <span aria-hidden="true" className="hew-flip-icon" style={{ color: 'var(--text-faint, #777)' }}>›</span>}
            {i === crumbs.length - 1 ? (
              <span style={{ color: 'var(--text-primary, #fff)', fontWeight: 'bold' }}>{c.label}</span>
            ) : (
              <button
                onClick={() => {
                  if (c.depth === -1) {
                    // Root: exit to top — every open session frame plus the
                    // object-context tail.
                    onExitContext()
                  } else {
                    // Truncate the COMBINED path to depth d+1: closes
                    // session frames innermost-first down to that depth if
                    // the crumb sits in the session-stack region, or just
                    // truncates the object-context tail otherwise (parent
                    // dispatches on `sessionStack.length` — see
                    // `handleSetPathDepth`).
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

      {/* Filter — MaterialPalette's filter pattern (⌕ / × clear). Matches
          the same label text every row renders below. */}
      <div style={{ display: 'flex', alignItems: 'center', gap: '4px' }}>
      <div style={{ position: 'relative', display: 'flex', alignItems: 'center', flex: 1 }}>
        <span
          aria-hidden="true"
          style={{
            position: 'absolute',
            insetInlineStart: '6px',
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
          placeholder={t`Filter outliner…`}
          aria-label={t`Filter outliner`}
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
              insetInlineEnd: '4px',
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
      {/* Sort — applies at every level; display order only. */}
      <OutlinerSortToggle value={sortMode} onChange={setOutlinerSort} />
      </div>

      {/* Unified node tree: root Model row, top-level nodes, then
          free-standing sketches. An empty document renders no content rows
          at all (just the Model row) — no placeholder text; an empty FILTER
          result gets one, below. */}
      <div>
        <ModelRow
          hidden={false}
          onToggleAllHidden={() =>
            toggleContainerVisibility(null, topNodes, getGroupMembers, hiddenKeys, onSetHiddenMany)
          }
          anyChildHidden={collectDescendants(topNodes, getGroupMembers).some((d) => hiddenKeys.has(nodeKey(d)))}
          isDropTarget={dropHighlightKey === 'root'}
        />
        {sessionStack.map((frame, i) => (
          <Row
            key={`session-frame:${nodeKey(frame.node)}`}
            label={frame.label}
            icon={<NodeIcon kind={frame.node.kind === 'instance' ? 'instance' : 'group'} />}
            selected={false}
            active
            dimmed={false}
            indent={i}
            rowRef={sessionRevealKey === `frame:${nodeKey(frame.node)}` ? sessionRevealRowRef : undefined}
            onClick={() => {}}
          />
        ))}
        {sessionStack.length > 0 && sortedSessionMembers.map(({ node, sessionIndex }) => (
          <NodeRow
              drawing={drawing}
            key={`session-member:${nodeKey(node)}`}
            node={node}
            // Positional-label index: an unnamed member's fallback label
            // must be the SAME index it carries as a top-level row (session
            // members are genuinely top-level nodes right now, so they're
            // in `topNodes`) — a fresh 0-based sequence here would make a
            // nested member and a surviving top-level row share one label
            // (delta-review finding, carried over from the single-instance
            // explode session), and would relabel members across every
            // open/close. A node born mid-session that a render races ahead
            // of `topNodes` falls back past its end rather than colliding.
            index={topIndexByKey.get(nodeKey(node)) ?? topNodes.length + sessionIndex}
            depth={sessionStack.length}
            scene={scene}
            docRev={docRev}
            watertightMap={watertightMap}
            objectHealth={objectHealth}
            fullPath={fullPath}
            isSelected={isSelected}
            primaryKey={primaryKey}
            selectedRowRef={selectedRowRef}
            sessionRevealKey={sessionRevealKey}
            sessionRevealRowRef={sessionRevealRowRef}
            ancestorGroupKeys={ancestorGroupKeys}
            hiddenKeys={hiddenKeys}
            onToggleHidden={onToggleHidden}
            onSetHiddenMany={onSetHiddenMany}
            onSelect={onSelect}
            onEnterContext={onEnterContext}
            filterResult={null}
            sortMode={sortMode}
            labelOf={labelFor}
            expandedMap={expandedMap}
            setNodeExpanded={setNodeExpanded}
          dropHighlightKey={dropHighlightKey}
          dragSourceKeys={dragSourceKeys}
          onStartDrag={startRowDrag}
          />
        ))}
        {sortedTop.map(({ node, index, free }) => {
          if (free) {
            if (
              filterActive &&
              !entityLabel('sketch', index).toLowerCase().includes(filterQuery)
            ) {
              return null
            }
            return (
              <Row
                key={nodeKey(node)}
                label={entityLabel('sketch', index)}
                icon={<NodeIcon kind="sketch" />}
                selected={isSelected(node)}
                isPrimary={primaryKey === nodeKey(node)}
                active={false}
                dimmed={fullPath.length > 0}
                indent={0}
                rowRef={primaryKey === nodeKey(node) ? selectedRowRef : undefined}
                onClick={(additive) => onSelect(node, additive)}
              />
            )
          }
          // The innermost session frame's members are rendered nested under
          // its header above — skip their loose top-level row so they don't
          // appear twice. (Every OTHER open frame's own node is already
          // excluded from `topNodes` by the kernel's hidden filter — see
          // the session-frame header block above.)
          if (sessionMemberKeySet !== null && sessionMemberKeySet.has(nodeKey(node))) {
            return null
          }
          if (
            filterResult !== null &&
            !filterResult.matches.has(nodeKey(node)) &&
            !filterResult.ancestors.has(nodeKey(node))
          ) {
            return null
          }
          return (
            <NodeRow
              drawing={drawing}
              key={`${node.kind}:${node.id}`}
              node={node}
              index={index}
              depth={0}
              scene={scene}
              docRev={docRev}
              watertightMap={watertightMap}
              objectHealth={objectHealth}
              fullPath={fullPath}
              isSelected={isSelected}
              primaryKey={primaryKey}
              selectedRowRef={selectedRowRef}
              sessionRevealKey={sessionRevealKey}
              sessionRevealRowRef={sessionRevealRowRef}
              ancestorGroupKeys={ancestorGroupKeys}
              hiddenKeys={hiddenKeys}
              onToggleHidden={onToggleHidden}
              onSetHiddenMany={onSetHiddenMany}
              onSelect={onSelect}
              onEnterContext={onEnterContext}
              filterResult={filterResult}
              sortMode={sortMode}
              labelOf={labelFor}
              expandedMap={expandedMap}
              setNodeExpanded={setNodeExpanded}
          dropHighlightKey={dropHighlightKey}
          dragSourceKeys={dragSourceKeys}
          onStartDrag={startRowDrag}
            />
          )
        })}
        {filterEmpty && (
          <div
            style={{
              padding: '6px 8px',
              fontSize: '12px',
              fontFamily: 'var(--font-family-ui)',
              color: 'var(--text-faint, #888)',
              fontStyle: 'italic',
            }}
          >
            <Trans>No objects match</Trans>
          </div>
        )}
      </div>
    </div>
  )
}

/** The root "Model" row: always present, always expanded (no chevron — it
 *  is not collapsible), not selectable, not filterable-away. Its own eye
 *  button IS the whole-document "hide/show all" control — there is no
 *  individual per-node hidden flag for the Model row itself the way a
 *  group/object has one, so a single button covers what a container row
 *  splits into two (its own eye plus the "all children" control). */
function ModelRow({
  hidden,
  anyChildHidden,
  onToggleAllHidden,
  isDropTarget,
}: {
  hidden: boolean
  anyChildHidden: boolean
  onToggleAllHidden: () => void
  /** Highlighted as the current VALID drag-and-drop target (move to the
   *  top level) — see `DocumentTree`'s `dropHighlightKey`. */
  isDropTarget?: boolean
}) {
  const { t } = useLingui()
  return (
    <div
      data-drop-target="root"
      style={{
        ...ROW_BASE,
        // Tree indentation is an inline-start inset: it grows from the row's
        // leading edge, which is the right edge in a mirrored layout.
        paddingInlineStart: '8px',
        paddingInlineEnd: '4px',
        cursor: 'default',
        fontWeight: 'bold',
        outline: isDropTarget === true ? '2px solid var(--accent-base)' : 'none',
        outlineOffset: '-2px',
        background: isDropTarget === true ? 'var(--accent-tint-15)' : undefined,
      }}
    >
      <ModelIcon />
      <span style={{ flex: 1, minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', opacity: hidden ? 0.6 : 1 }}>
        {t({ context: 'root', comment: 'Top row of the outliner: the whole document', message: 'Model' })}
      </span>
      <button
        onClick={(e) => {
          e.stopPropagation()
          onToggleAllHidden()
        }}
        aria-label={anyChildHidden ? t`Show all children` : t`Hide all children`}
        title={anyChildHidden ? t`Show all children` : t`Hide all children`}
        style={{
          background: 'none',
          border: 'none',
          color: anyChildHidden ? 'var(--text-section)' : 'var(--text-muted)',
          cursor: 'pointer',
          padding: '0 2px',
          fontSize: '11px',
          lineHeight: 1,
          flexShrink: 0,
        }}
      >
        {anyChildHidden ? '○○' : '●●'}
      </button>
    </div>
  )
}

function ModelIcon() {
  return (
    <svg
      {...ICON_SVG_PROPS}
      data-node-icon="model"
      style={{ ...ICON_SVG_PROPS.style, color: 'var(--text-section, #9ab)' }}
    >
      <path d="M2 6.6 7 2.4 12 6.6 12 12 2 12 Z" />
      <path d="M5.4 12 5.4 8.4 8.6 8.4 8.6 12" />
    </svg>
  )
}

/** One tree row that may be an object or a group (with expand/collapse). */
// Wrapped in React.memo (adversarial review finding 5): DocumentTree can
// re-render often (docRev bumps, selection changes, an expand toggle
// anywhere in the tree), and a group with many members re-renders every
// child NodeRow along with it — memo skips a row whose own props are
// unchanged. The inner function is named `NodeRowInner`, NOT `NodeRow`, so
// the recursive `<NodeRow>` JSX below resolves to the memoized `const`
// (a same-named function expression's own name would shadow it and bypass
// memo for every nested row).
const NodeRow = memo(function NodeRowInner({
  node,
  index,
  depth,
  scene,
  docRev,
  watertightMap,
  objectHealth,
  fullPath,
  isSelected,
  primaryKey,
  selectedRowRef,
  sessionRevealKey,
  sessionRevealRowRef,
  ancestorGroupKeys,
  hiddenKeys,
  onToggleHidden,
  onSetHiddenMany,
  onSelect,
  onEnterContext,
  filterResult,
  sortMode,
  labelOf,
  expandedMap,
  setNodeExpanded,
  dropHighlightKey,
  dragSourceKeys,
  onStartDrag,
  drawing,
}: {
  node: NodeRef
  index: number
  depth: number
  scene: WasmScene
  docRev: number
  /** Where every drawn shape lists (planeUnits.ts `outlinerDrawing`). */
  drawing: OutlinerDrawing
  watertightMap: Map<bigint, boolean>
  objectHealth?: Map<bigint, HealthClass>
  /** The combined session-stack + object-context path (see the parent
   *  component's `fullPath`) — every entry on it gets the "editing"
   *  treatment (design: not just the deepest), via the same
   *  positional depth-match `isTreeRowDimmed` already used. */
  fullPath: NodeRef[]
  isSelected: (n: NodeRef) => boolean
  primaryKey: string | null
  selectedRowRef: React.RefObject<HTMLDivElement | null>
  /** Session-stack reveal target (see the parent component's comment where
   *  it's computed) — `` `node:${nodeKey(node)}` `` when THIS row is the
   *  node a just-closed session frame returned to. `null` when no reveal is
   *  pending. */
  sessionRevealKey: string | null
  sessionRevealRowRef: React.RefObject<HTMLDivElement | null>
  ancestorGroupKeys: Set<string>
  hiddenKeys: Set<string>
  onToggleHidden: (node: NodeRef) => void
  onSetHiddenMany: (nodes: NodeRef[], hidden: boolean) => void
  onSelect: (n: NodeRef, additive: boolean) => void
  onEnterContext: (n: NodeRef) => void
  /** Active text filter (`null` = no filter): drives force-expand of an
   *  ancestor group and the dimmed "ancestor, not a match itself" styling. */
  filterResult: { matches: Set<string>; ancestors: Set<string> } | null
  /** Row order at this and every nested level (display only). */
  sortMode: OutlinerSort
  /** The shared label resolver the Name and Kind sorts compare. */
  labelOf: (node: NodeRef) => string
  expandedMap: Map<string, boolean>
  setNodeExpanded: (key: string, value: boolean) => void
  /** The `nodeKey` currently highlighted as a VALID drag-and-drop target
   *  (`treeModel.ts`'s `dropTargetFor`), or `null` — an invalid hover
   *  target deliberately shows no highlight (design), only a cursor
   *  change, so this is never set for one. */
  dropHighlightKey: string | null
  /** Rows being dragged right now (`${kind}:${id}`), dimmed in place. */
  dragSourceKeys: ReadonlySet<string>
  /** Begins a row drag (DocumentTree's pointer-based DnD) for `node`. */
  onStartDrag: (node: NodeRef, e: React.PointerEvent<HTMLDivElement>) => void
}) {
  const { t } = useLingui()
  const key = nodeKey(node)
  // Auto-expand when this group is an ancestor of the primary selected node,
  // OR (filter active) an ancestor of a filter match — either reason forces
  // it open so the thing being revealed is actually visible.
  const isAncestor = node.kind === 'group' && ancestorGroupKeys.has(key)
  const isFilterAncestor = node.kind === 'group' && (filterResult?.ancestors.has(key) ?? false)
  const forceExpand = isAncestor || isFilterAncestor
  // Nested containers start COLLAPSED — an outliner full of pre-expanded
  // hierarchy is noise; the force-expand effect below still opens the
  // ancestors of whatever is selected/matched. Lifted into the parent's
  // `expandedMap` (design: filter/unfilter must not lose manual expand
  // state) rather than local useState.
  const expanded = expandedMap.get(key) ?? false
  useEffect(() => {
    if (forceExpand && !expanded) setNodeExpanded(key, true)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [forceExpand, expanded, key])

  const selected = isSelected(node)
  const isPrimary = primaryKey !== null && nodeKey(node) === primaryKey
  // This row is exactly the path entry at ITS OWN depth — the same
  // condition `isTreeRowDimmed` checks for "not dimmed" — so EVERY row on
  // the path gets the "editing" chip, not just the deepest (design).
  const pathEntry = fullPath[depth]
  const active = pathEntry !== undefined &&
    pathEntry.kind === node.kind && pathEntry.id === node.id
  // Dimmed either by the active editing context (existing behavior) or by
  // being a non-matching ANCESTOR of a filter match (design: "non-matching
  // ancestors render dimmed") — a plain filter match renders full-strength.
  const dimmedByFilter =
    filterResult !== null && !filterResult.matches.has(key) && filterResult.ancestors.has(key)
  const dimmed = isTreeRowDimmed(fullPath, node, depth) || dimmedByFilter
  const ownHidden = hiddenKeys.has(key)
  // "Hidden by parent" (eye rendering fix): a child of a hidden ancestor is
  // effectively hidden too (unionHiddenLeafIds's recursive expansion), so
  // its eye reflects that — dimmed ○, not the normal ●, and NOT because
  // its own key is set. Memoized (adversarial review finding 5): the walk
  // is O(depth) per row, redone on every render otherwise — keyed on the
  // node identity, `scene`/`docRev` (a group can be reparented across
  // mutations, changing its ancestor chain), and `hiddenKeys`.
  const hiddenByParent = useMemo(
    () => !ownHidden && isHiddenByAncestor(node, scene, hiddenKeys),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [node.kind, node.id, scene, docRev, hiddenKeys, ownHidden],
  )
  const hidden = ownHidden || hiddenByParent
  // Imprints (drawn-but-not-yet-pushed shapes on this object's faces —
  // imprints.ts) render below as indented, always-visible child rows; a
  // circle or arc other shapes cut into runs is one row, the curve drawn.
  // Memoized on the document revision: the read is a kernel topology walk
  // plus a JSON round-trip, and this row re-renders on every hover state
  // change — unconditional so the hook order never varies with `kind`.
  const imprints = node.kind === 'object' ? (drawing.byObject.get(node.id.toString()) ?? []) : []
  // Whether THIS row is the node a just-closed session frame returned to
  // (see the parent component's `sessionRevealKey` comment) — mutually
  // exclusive with `isPrimary` in practice (a session boundary always clears
  // the selection first), but `isPrimary` still wins the `rowRef` slot below
  // on the off chance both line up, matching how the two mechanisms are
  // otherwise independent.
  const isSessionRevealTarget = sessionRevealKey === `node:${nodeKey(node)}`
  const rowRef = isPrimary ? selectedRowRef : isSessionRevealTarget ? sessionRevealRowRef : undefined

  if (node.kind === 'object') {
    const health: 'solid' | HealthClass =
      objectHealth !== undefined
        ? (objectHealth.get(node.id) ?? 'solid')
        : watertightMap.get(node.id) === false
          ? 'open'
          : 'solid'
    // An object usually carries 0-2 imprints, and objects have no expand
    // chevron of their own, so there is no collapsed state to hide the
    // rows behind.
    return (
      <>
        <Row
          label={resolveLabel(scene.object_name(node.id), undefined, 'object', index)}
          icon={<NodeIcon kind="object" health={health} />}
          selected={selected}
          isPrimary={isPrimary}
          active={active}
          dimmed={dimmed}
          hidden={hidden}
          hiddenByParent={hiddenByParent}
          indent={depth}
          rowRef={rowRef}
          onClick={(additive) => onSelect(node, additive)}
          onDoubleClick={() => onEnterContext(node)}
          onToggleHidden={() => onToggleHidden(node)}
          dropTargetKey={key}
          isDragSource={dragSourceKeys.has(key)}
          isDropTarget={dropHighlightKey === key}
          onRowPointerDown={(e) => onStartDrag(node, e)}
        />
        {(sortMode === 'document'
          ? imprints
          : sortOutlinerLevel(imprints.map((unit) => ({ node: unit.ref, unit })), 'name', labelOf).map((e) => e.unit)
        ).map((unit) => {
          const ref = unit.ref
          const refKey = nodeKey(ref)
          return (
            <Row
              key={refKey}
              label={drawing.labels.get(refKey) ?? t({ context: 'drawn shape', message: 'Shape' })}
              icon={<NodeIcon kind="draft-island" />}
              selected={isSelected(ref)}
              isPrimary={primaryKey === refKey}
              active={false}
              dimmed={dimmed}
              indent={depth + 1}
              rowRef={primaryKey === refKey ? selectedRowRef : undefined}
              onClick={(additive) => onSelect(ref, additive)}
            />
          )
        })}
      </>
    )
  }

  if (node.kind === 'instance') {
    const def = scene.instance_def(node.id)
    const defName = def !== undefined ? scene.component_name(def) : undefined
    // No `onToggleAllHidden`/`anyChildHidden` here, unlike the group branch
    // below: an instance's members live in its SHARED definition, not in
    // this instance alone (every other instance of the same component
    // renders the same members) — a per-INSTANCE "hide all children"
    // control has no node-local set of children to represent; hiding the
    // whole instance (the primary eye above) is the only representable
    // granularity. This row also doesn't expand/nest its members in the
    // tree at all (unlike a group), so there is no visible child list a
    // second control would even be toggling.
    return (
      <Row
        label={resolveLabel(scene.instance_name(node.id), defName, 'instance', index)}
        icon={<NodeIcon kind="instance" />}
        selected={selected}
        isPrimary={isPrimary}
        active={active}
        dimmed={dimmed}
        hidden={hidden}
        hiddenByParent={hiddenByParent}
        indent={depth}
        rowRef={rowRef}
        onClick={(additive) => onSelect(node, additive)}
        onDoubleClick={() => onEnterContext(node)}
        onToggleHidden={() => onToggleHidden(node)}
        dropTargetKey={key}
        isDragSource={dragSourceKeys.has(key)}
        isDropTarget={dropHighlightKey === key}
        onRowPointerDown={(e) => onStartDrag(node, e)}
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
  const getGroupMembers = (groupId: bigint): NodeRef[] =>
    scene.group_members(groupId).map(nodeRefFromJs)
  // Memoized (adversarial review finding 5): a full recursive descendant
  // walk (every nested member, every level) on EVERY render otherwise —
  // keyed on `members` (already memoized above) and `hiddenKeys`, the only
  // two things that can change the answer.
  const anyChildHidden = useMemo(
    () => collectDescendants(members, getGroupMembers).some((d) => hiddenKeys.has(nodeKey(d))),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [members, hiddenKeys],
  )
  // Each child keeps its position among ALL members: an unnamed child's
  // "Group N" must not shift when a filter hides its siblings.
  const visibleMembers = sortOutlinerLevel(
    members.map((child, position) => ({ node: child, child, position })),
    sortMode,
    labelOf,
  )
    .filter(
      ({ child }) =>
        filterResult === null ||
        filterResult.matches.has(nodeKey(child)) ||
        filterResult.ancestors.has(nodeKey(child)),
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
        hiddenByParent={hiddenByParent}
        indent={depth}
        isGroup
        expanded={expanded}
        onToggleExpand={() => setNodeExpanded(key, !expanded)}
        rowRef={rowRef}
        onClick={(additive) => onSelect(node, additive)}
        onDoubleClick={() => onEnterContext(node)}
        onToggleHidden={() => onToggleHidden(node)}
        onToggleAllHidden={() =>
          toggleContainerVisibility(node, members, getGroupMembers, hiddenKeys, onSetHiddenMany)
        }
        anyChildHidden={anyChildHidden}
        dropTargetKey={key}
        isDragSource={dragSourceKeys.has(key)}
        isDropTarget={dropHighlightKey === key}
        onRowPointerDown={(e) => onStartDrag(node, e)}
      />
      {expanded && visibleMembers.map(({ child, position }) => (
        <NodeRow
              drawing={drawing}
          key={`${child.kind}:${child.id}`}
          node={child}
          index={position}
          depth={depth + 1}
          scene={scene}
          docRev={docRev}
          watertightMap={watertightMap}
          objectHealth={objectHealth}
          fullPath={fullPath}
          isSelected={isSelected}
          primaryKey={primaryKey}
          selectedRowRef={selectedRowRef}
          sessionRevealKey={sessionRevealKey}
          sessionRevealRowRef={sessionRevealRowRef}
          ancestorGroupKeys={ancestorGroupKeys}
          hiddenKeys={hiddenKeys}
          onToggleHidden={onToggleHidden}
          onSetHiddenMany={onSetHiddenMany}
          onSelect={onSelect}
          onEnterContext={onEnterContext}
          filterResult={filterResult}
          sortMode={sortMode}
          labelOf={labelOf}
          expandedMap={expandedMap}
          setNodeExpanded={setNodeExpanded}
          dropHighlightKey={dropHighlightKey}
          dragSourceKeys={dragSourceKeys}
          onStartDrag={onStartDrag}
        />
      ))}
    </>
  )
})

// ---------------------------------------------------------------------------
// NodeIcon — 14px stroke-based inline SVG per node type.
//
// Matches the minimal line-icon language of the toolbar (thin, geometric,
// currentColor) while staying quiet: each type gets a subtle theme-aware tint
// via CSS vars, applied as the SVG's color so `currentColor` picks it up.
//   object   — isometric cube; solid = solid outline (--status-solid),
//              surface = thin neutral outline, no dash (a sheet drawn on
//              purpose, or accepted), open = dashed outline (--status-leaky)
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

export function NodeIcon({ kind, health = 'solid' }: { kind: NodeKind; health?: 'solid' | HealthClass }) {
  if (kind === 'object' && health === 'surface') {
    return (
      <svg
        {...ICON_SVG_PROPS}
        data-node-icon="object-surface"
        style={{ ...ICON_SVG_PROPS.style, color: 'var(--text-muted)' }}
      >
        <title>{OUTLINER_TOOLTIP.panel}</title>
        <path d="M7 1.4 12.1 4.2 12.1 9.8 7 12.6 1.9 9.8 1.9 4.2 Z" strokeWidth={0.9} />
        <path d="M1.9 4.2 7 7 12.1 4.2 M7 7 7 12.6" strokeWidth={0.9} />
      </svg>
    )
  }
  if (kind === 'object') {
    const leaky = health === 'open'
    return (
      <svg
        {...ICON_SVG_PROPS}
        data-node-icon={leaky ? 'object-leaky' : 'object-solid'}
        style={{ ...ICON_SVG_PROPS.style, color: leaky ? 'var(--status-leaky)' : 'var(--status-solid)' }}
      >
        {leaky && <title>{OUTLINER_TOOLTIP.open}</title>}
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
  hiddenByParent,
  indent,
  isGroup,
  expanded,
  onToggleExpand,
  rowRef,
  onClick,
  onDoubleClick,
  onToggleHidden,
  onToggleAllHidden,
  anyChildHidden,
  dropTargetKey,
  isDragSource,
  isDropTarget,
  onRowPointerDown,
}: {
  label: string
  icon: React.ReactNode
  selected: boolean
  isPrimary?: boolean
  active: boolean
  dimmed: boolean
  hidden?: boolean
  /** Hidden only because an ancestor group is hidden, not this row's own
   *  key — renders the eye more faintly than a directly-hidden row. */
  hiddenByParent?: boolean
  indent: number
  isGroup?: boolean
  expanded?: boolean
  onToggleExpand?: () => void
  rowRef?: React.Ref<HTMLDivElement>
  onClick: (additive: boolean) => void
  onDoubleClick?: () => void
  onToggleHidden?: () => void
  /** Container "hide/show all children" control (design): a second,
   *  hover/focus-visible eye-stack button, present on group rows only. */
  onToggleAllHidden?: () => void
  anyChildHidden?: boolean
  /** This row's `nodeKey` — set on `data-drop-target` so a drag in
   *  progress can resolve "what row is under the pointer" via
   *  `elementFromPoint`. Only object/group/instance rows carry one; a
   *  sketch row omits it, so hovering one resolves to no target. */
  dropTargetKey?: string
  /** This row is being dragged: dimmed in place while its ghost travels. */
  isDragSource?: boolean
  /** Highlighted as the current VALID drag-and-drop target — see
   *  `DocumentTree`'s `dropHighlightKey`. An invalid hover target is
   *  never passed `true` here (design: no highlight, cursor only). */
  isDropTarget?: boolean
  /** Begins a row drag (DocumentTree's pointer-based DnD), when this row
   *  can be dragged. */
  onRowPointerDown?: (e: React.PointerEvent<HTMLDivElement>) => void
}) {
  const { t } = useLingui()
  const rtl = useIsRtl()
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

  // "Hide/show all children" is visible on hover/focus only (design) — kept
  // in the DOM at all times (never `display:none`) so keyboard Tab still
  // reaches it; visibility is purely `opacity`, driven by hovering the row
  // OR focusing the button itself.
  const [rowHovered, setRowHovered] = useState(false)
  const [allHiddenFocused, setAllHiddenFocused] = useState(false)
  const showAllHiddenControl = rowHovered || allHiddenFocused

  return (
    <div
      ref={rowRef}
      data-drop-target={dropTargetKey}
      onClick={(e) => onClick(e.shiftKey || e.ctrlKey || e.metaKey)}
      onDoubleClick={onDoubleClick}
      onPointerDown={onRowPointerDown}
      onMouseEnter={() => setRowHovered(true)}
      onMouseLeave={() => setRowHovered(false)}
      style={{
        ...ROW_BASE,
        // One indentation step per level, measured from the row's leading
        // (inline-start) edge — the tree grows rightward in English and
        // leftward in Arabic without a second layout rule.
        paddingInlineStart: `${8 + indent * 16}px`,
        paddingInlineEnd: '4px',
        background: isDropTarget === true ? 'var(--accent-tint-15)' : background,
        // The open-session marker sits on the leading edge; a box-shadow
        // cannot be logical, so its sign follows the current direction.
        boxShadow: active ? `inset ${rtl ? -2 : 2}px 0 0 var(--accent-base)` : 'none',
        outline: isDropTarget === true ? '2px solid var(--accent-base)' : 'none',
        outlineOffset: '-2px',
        color: anySelected ? 'var(--accent-text-on-tint)' : undefined,
        opacity: dimmed ? 0.5 : isDragSource === true ? 0.4 : 1,
        fontWeight: active ? 'bold' : 'normal',
      }}
    >
      {isGroup === true && (
        <button
          onClick={(e) => {
            e.stopPropagation()
            onToggleExpand?.()
          }}
          onPointerDown={(e) => e.stopPropagation()}
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
          {/* The collapse caret points along the reading direction, so a
              collapsed group's arrow faces the side its children will open
              on (index.css `.hew-flip-icon`). */}
          <span className="hew-flip-icon">{expanded === true ? '▾' : '▸'}</span>
        </button>
      )}
      {icon}
      <span data-testid="outliner-row-label" style={{ flex: 1, minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', color: hidden === true ? 'var(--text-faint, #666)' : undefined }}>{label}</span>
      {active && <span style={{ fontSize: '10px', color: 'var(--accent-text-on-tint)' }}><Trans>editing</Trans></span>}
      {onToggleAllHidden !== undefined && (
        <button
          onClick={(e) => {
            e.stopPropagation()
            onToggleAllHidden()
          }}
          onPointerDown={(e) => e.stopPropagation()}
          onFocus={() => setAllHiddenFocused(true)}
          onBlur={() => setAllHiddenFocused(false)}
          aria-label={anyChildHidden === true ? t`Show all children` : t`Hide all children`}
          title={anyChildHidden === true ? t`Show all children` : t`Hide all children`}
          style={{
            background: 'none',
            border: 'none',
            color: anyChildHidden === true ? 'var(--text-section)' : 'var(--text-muted)',
            cursor: 'pointer',
            padding: '0 2px',
            fontSize: '11px',
            lineHeight: 1,
            flexShrink: 0,
            opacity: showAllHiddenControl ? 1 : 0,
          }}
        >
          {anyChildHidden === true ? '○○' : '●●'}
        </button>
      )}
      {/* Primary eye toggle — always visible (no hover-only class
          infrastructure here), same as before this Lane. */}
      {onToggleHidden !== undefined && (
        <button
          onClick={(e) => {
            e.stopPropagation()
            onToggleHidden()
          }}
          onPointerDown={(e) => e.stopPropagation()}
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
            opacity: hiddenByParent === true ? 0.55 : 1,
          }}
        >
          {hidden === true ? '○' : '●'}
        </button>
      )}
    </div>
  )
}

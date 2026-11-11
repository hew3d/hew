/**
 * ModelHealthPanel — the body of the Model Health window, listing the
 * objects of a scope that aren't solid, grouped by kind (the same
 * classification the Make Solid wizard uses: `health/fixPlan.ts`). The
 * per-object detail view for power users: finding, framing, stepping through
 * openings, and fixing one object or one group.
 *
 * The report is `Scene.health_report_json(scope)` read inside a `useMemo`
 * keyed on `docRev` + scope (the raw-scene + `docRev` pattern ObjectInfoPanel
 * uses). Rows are grouped per kind ("Has openings", "Flat panels", ...) plus
 * "Split shapes" per family; panels and sheets are listed but never styled
 * as problems. Rows the person kept stay hidden unless "Show kept panels and
 * sheets" is on.
 *
 * Clicking a row selects the object (a definition member selects its first
 * placing instance — a member is not selectable on its own outside a
 * component edit) and frames it. ◀ ▶ (and Tab / Shift+Tab while a row has
 * focus) step through the selected row's openings, framing each; that
 * "current finding" is also what the viewport overlay draws brighter.
 *
 * The panel is mounted only while its window is open, so the overlay it
 * reports through `onOverlayChange` is cleared on unmount — the viewport
 * draws nothing for a closed window.
 */
import { useEffect, useMemo, useRef, useState } from 'react'
import { i18n } from '@lingui/core'
import { useLingui } from '@lingui/react/macro'
import type { Scene as WasmScene } from '../wasm/loader'
import {
  HEALTH_SCOPE_LABELS,
  KIND_ORDER,
  canCloseRow,
  capPreviewRims,
  classifyRow,
  filterRows,
  groupRows,
  groupedRows,
  readHealthReport,
  openReason,
  rowDetailText,
  splitFamilySet,
  rowDisplayName,
  rowPlacements,
  segmentBounds,
  type HealthOverlay,
  type HealthRow,
  type HealthScope,
} from '../health/healthReport'
import { ACTION, KIND_NAME, MEND, PANEL, REASON_MEND_LINE, REASON_ON_PURPOSE, REASON_SENTENCE, TOOLTIP, UNDO_LABEL, distinctLabels, type KindKey } from '../health/healthCopy'
import {
  batchLabel,
  batchSummary,
  capOp,
  closableRows,
  failureLine,
  familyCascade,
  familyRepairOps,
  groupBatchButtons,
  markOp,
  markableRows,
  runBatch,
  thickenOp,
  type BatchKind,
  type BatchOp,
  type BatchOptions,
} from '../health/healthBatch'
import {
  formatMergeDistance,
  hasWarpedHoles,
  mendCandidate,
  mendIdleLine,
  mendOp,
  mendOutcomes,
  mendPreviewLine,
  mendToast,
  mendableRows,
  mendableScan,
  parseMergeDistance,
  readDefaultWeld,
  readMendCensus,
  readMendCensusResult,
  DEFAULT_MEND_PLAN,
  concatPoints,
  readMendPoints,
  readWeldRange,
  skippedLine,
  type MendPlan,
} from '../health/mend'
import { BUSY_REFUSED } from '../busyCard'
import { ThickenDialog } from './ThickenDialog'
import { collectLeafIds, nodeRefFromJs, type NodeRef } from './treeModel'
import { worldBoundsForSelection } from './objectBounds'
import { kernelErrorMessage, parseKernelErrorCode } from '../kernelErrors'
import type { IsolationTarget } from '../viewport/isolation'
import { isComposingKey } from '../input/keyboard'

interface Props {
  scene: WasmScene
  /** Bumped by the parent on any document change to trigger a re-read. */
  docRev: number
  selectedIds: NodeRef[]
  scope: HealthScope
  onScopeChange: (scope: HealthScope) => void
  /** Object Info's "Show" link: list only this object. */
  objectFilter: bigint | null
  onClearObjectFilter: () => void
  /** Replace the selection with `node`. */
  onSelectNode: (node: NodeRef) => void
  /** Show only `targets` in the viewport (view-only isolation), replacing any
   *  isolation already on. The panel frames the row itself, so it asks the
   *  viewport not to. */
  onIsolate: (targets: IsolationTarget[], opts?: { frame?: boolean }) => void
  /** End the isolation this panel made, if it is still the one on screen
   *  (App ignores the call otherwise, so the wizard's isolation survives). */
  onClearIsolation: () => void
  /** Frame a world box in the viewport. */
  onFrameBounds: (min: [number, number, number], max: [number, number, number]) => void
  /** The rims the viewport should draw; null when the panel goes away. */
  onOverlayChange: (overlay: HealthOverlay | null) => void
  /** After a repair: re-tessellate and bump `docRev` (App's refreshAfterPanelMutation). */
  onGeometryChanged: () => void
  onToast?: (message: string, code?: string) => void
  /** App's busy card: show `message`, let it paint, run `work`. False when
   *  another long change is still running (the caller says so). */
  onBusy?: (message: string, work: () => void) => boolean
}

/** What "Choose what to mend" holds: the steps on or off. */
interface MendChoice {
  weld: boolean
  split: boolean
  fill: boolean
  folds: boolean
}
const MEND_ALL_ON: MendChoice = { weld: true, split: true, fill: true, folds: true }

/** What a hovered Mend button draws: the rims it fills and the points it merges / splits. */
interface MendPreview {
  rims: { id: bigint; rim: number }[]
  merge: Float32Array
  split: Float32Array
}

/** The search box waits this long after the last keystroke before it changes
 *  the listed set (and so the viewport overlay). */
const SEARCH_DEBOUNCE_MS = 150

/** The merge-distance field waits this long before the preview follows it. */
const DISTANCE_DEBOUNCE_MS = 300

/** Mend all's hover draws points for this many objects at most. */
const MEND_ALL_POINTS_ROWS = 20

/** Mend all reads this many fresh censuses while the group renders. */
const MEND_ALL_CENSUS_ROWS = 25

/** Groups with more rows than this start collapsed (keeps a 1000-row import
 *  from mounting a thousand rows at once). */
const AUTO_COLLAPSE_ROWS = 100

const CONTROL_STYLE: React.CSSProperties = {
  background: 'var(--surface-input, #1e1e1e)',
  border: '1px solid var(--border-strong, #444)',
  borderRadius: '3px',
  color: 'var(--text-primary, #eee)',
  fontFamily: 'var(--font-family-ui)',
  fontSize: '12px',
  padding: '3px 6px',
  outline: 'none',
  boxSizing: 'border-box',
}

const GROUP_HEADER_STYLE: React.CSSProperties = {
  display: 'flex',
  alignItems: 'center',
  gap: '6px',
  width: '100%',
  padding: '4px 0',
  background: 'none',
  border: 'none',
  cursor: 'pointer',
  fontFamily: 'var(--font-family-ui)',
  fontSize: '12px',
  fontWeight: 600,
  color: 'var(--text-secondary, #ccc)',
  textAlign: 'left',
}

const ROW_STYLE: React.CSSProperties = {
  display: 'flex',
  alignItems: 'center',
  gap: '6px',
  padding: '3px 8px',
  borderRadius: '3px',
  fontFamily: 'var(--font-family-ui)',
  fontSize: '12px',
  color: 'var(--text-secondary, #ccc)',
  cursor: 'pointer',
  minWidth: 0,
}

const STEP_BUTTON_STYLE: React.CSSProperties = {
  background: 'none',
  border: 'none',
  color: 'var(--text-secondary, #ccc)',
  cursor: 'pointer',
  padding: '0 3px',
  lineHeight: 1,
  flexShrink: 0,
}

const ACTION_BUTTON_STYLE: React.CSSProperties = {
  background: 'none',
  border: '1px solid var(--border-strong, #444)',
  borderRadius: '3px',
  color: 'var(--text-primary, #eee)',
  cursor: 'pointer',
  fontFamily: 'var(--font-family-ui)',
  fontSize: '11px',
  padding: '1px 6px',
  whiteSpace: 'nowrap',
}

/** The object ids a selection covers: world objects, group members, and the
 *  members of every selected instance. */
export function selectionObjectIds(scene: WasmScene, selectedIds: NodeRef[]): bigint[] {
  const out = new Set<bigint>()
  const members = (groupId: bigint): NodeRef[] => scene.group_members(groupId).map(nodeRefFromJs)
  for (const node of selectedIds) {
    const { objectIds, instanceIds } = collectLeafIds(node, members)
    for (const o of objectIds) out.add(o)
    for (const i of instanceIds) for (const m of scene.instance_expanded_members(i)) out.add(m)
  }
  return [...out]
}

/** The node a row selects: the object itself for a world object, else the
 *  first instance placing the definition member — or, when the viewport
 *  selection already is one of the row's instances, that one. */
export function rowNode(row: HealthRow, selected: readonly NodeRef[] = []): NodeRef {
  if (row.definition === null || row.instances.length === 0) return { kind: 'object', id: row.object }
  const chosen = selected.find((n) => n.kind === 'instance' && row.instances.includes(n.id))
  return { kind: 'instance', id: chosen !== undefined ? chosen.id : row.instances[0] }
}

export function ModelHealthPanel({
  scene,
  docRev,
  selectedIds,
  scope,
  onScopeChange,
  objectFilter,
  onClearObjectFilter,
  onSelectNode,
  onIsolate,
  onClearIsolation,
  onFrameBounds,
  onOverlayChange,
  onGeometryChanged,
  onToast,
  onBusy,
}: Props) {
  const { t } = useLingui()
  const [query, setQuery] = useState('')
  /** `query` after the debounce: the one the listing filters by. */
  const [debouncedQuery, setDebouncedQuery] = useState('')
  useEffect(() => {
    if (query === debouncedQuery) return
    const timer = setTimeout(() => setDebouncedQuery(query), SEARCH_DEBOUNCE_MS)
    return () => clearTimeout(timer)
  }, [query, debouncedQuery])
  const [showAccepted, setShowAccepted] = useState(false)
  const [collapsedGroups, setCollapsedGroups] = useState<Record<string, boolean>>({})
  /** The current finding: the selected row and which of its rims is framed. */
  const [current, setCurrent] = useState<{ object: bigint; rim: number } | null>(null)
  /** The rims a hovered / focused Close button will cap (viewport preview). */
  const [preview, setPreview] = useState<{ id: bigint; rim: number }[] | null>(null)
  /** "Choose what to mend" and the merge distance the person typed (null = the default). */
  const [mendChoice, setMendChoice] = useState<MendChoice>(MEND_ALL_ON)
  const [distanceText, setDistanceText] = useState<string | null>(null)
  /** `distanceText` once typing pauses: the census is not re-asked on every keystroke. */
  const [distanceSettled, setDistanceSettled] = useState<string | null>(null)
  useEffect(() => {
    if (distanceText === distanceSettled) return
    const t = setTimeout(() => setDistanceSettled(distanceText), DISTANCE_DEBOUNCE_MS)
    return () => clearTimeout(t)
  }, [distanceText, distanceSettled])
  const [mendOpen, setMendOpen] = useState(false)
  /** The points and rims a hovered / focused Mend button previews. */
  const [mendPreview, setMendPreview] = useState<MendPreview | null>(null)
  /** Which batch button is running (disables the rest). */
  const [busy, setBusy] = useState<string | null>(null)
  /** Per-row warning lines from the last batch's refusals. */
  const [warnings, setWarnings] = useState<Map<bigint, string>>(new Map())
  /** The Thicken… dialog's rows while it is open. */
  const [thickenRows, setThickenRows] = useState<HealthRow[] | null>(null)
  /** Seam families whose "Keep partition sheets as surfaces" box is ticked. */
  const [keepSheets, setKeepSheets] = useState<Record<number, boolean>>({})

  // Selection scope lists the objects the selection covered when the scope was
  // chosen (or last refreshed), NOT the live selection: a row click selects its
  // object, which must not collapse the list to that one row.
  const selectedIdsRef = useRef(selectedIds)
  selectedIdsRef.current = selectedIds
  const [selectionSnapshot, setSelectionSnapshot] = useState<bigint[] | null>(() =>
    scope === 'selection' ? selectionObjectIds(scene, selectedIds) : null,
  )
  const snapshotKeyRef = useRef({ scene, scope })
  useEffect(() => {
    const key = snapshotKeyRef.current
    if (key.scene === scene && key.scope === scope) return // the initial snapshot stands
    snapshotKeyRef.current = { scene, scope }
    setSelectionSnapshot(scope === 'selection' ? selectionObjectIds(scene, selectedIdsRef.current) : null)
  }, [scene, scope])

  const scopeIds = useMemo(() => {
    if (objectFilter !== null) return [objectFilter]
    if (scope === 'last') return Array.from(scene.last_import_object_ids())
    if (scope === 'selection') return selectionSnapshot ?? selectionObjectIds(scene, selectedIdsRef.current)
    return null
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [scene, docRev, scope, objectFilter, selectionSnapshot])
  const report = useMemo(
    () => readHealthReport(scene, docRev, scopeIds),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [scene, docRev, scopeIds],
  )

  const listed = useMemo(
    () => filterRows(report.rows, { query: debouncedQuery, showAccepted, object: objectFilter }),
    [report, debouncedQuery, showAccepted, objectFilter],
  )
  const grouped = useMemo(() => groupRows(report, listed), [report, listed])
  const splits = useMemo(() => splitFamilySet(report.rows), [report])
  const inSplit = (row: HealthRow): boolean => row.family !== null && splits.has(row.family)
  const ordered = useMemo(() => groupedRows(grouped), [grouped])

  const currentRow = current === null ? null : (ordered.find((r) => r.object === current.object) ?? null)
  const currentRim = currentRow !== null && current !== null ? current.rim : 0

  // The overlay follows the listed rows (scope / search / accepted filter)
  // and the current finding; the cleanup clears it when the section closes.
  // The listed ids are memoised apart from the current finding / preview, so
  // stepping a rim leaves the arrays (and the renderer's rim lines) untouched.
  const listedIds = useMemo(
    () => ({
      ids: ordered.filter((r) => r.verdict.kind !== 'non_manifold').map((r) => r.object),
      warnIds: ordered.filter((r) => r.verdict.kind === 'non_manifold').map((r) => r.object),
    }),
    [ordered],
  )
  const overlay = useMemo<HealthOverlay>(
    () => ({
      ids: listedIds.ids,
      warnIds: listedIds.warnIds,
      rev: docRev,
      current: currentRow !== null && currentRow.rims.length > 0 ? { id: currentRow.object, rim: currentRim } : null,
      capPreview: preview ?? mendPreview?.rims ?? null,
      mendPoints: mendPreview === null ? null : { merge: mendPreview.merge, split: mendPreview.split },
    }),
    [listedIds, docRev, currentRow, currentRim, preview, mendPreview],
  )
  useEffect(() => {
    onOverlayChange(overlay)
  }, [overlay, onOverlayChange])
  useEffect(() => () => onOverlayChange(null), [onOverlayChange])

  // The panel's isolation belongs to the open panel: closing its window
  // (unmount) or switching its scope ends it.
  const clearIsolationRef = useRef(onClearIsolation)
  clearIsolationRef.current = onClearIsolation
  useEffect(() => () => clearIsolationRef.current(), [])
  const firstScope = useRef(true)
  useEffect(() => {
    if (firstScope.current) {
      firstScope.current = false
      return
    }
    clearIsolationRef.current()
  }, [scope])

  // A repaired row leaves the report: drop its warning, and any preview of it.
  useEffect(() => {
    setWarnings((prev) => {
      if (prev.size === 0) return prev
      const listedNow = new Set(report.rows.map((r) => r.object))
      const next = new Map([...prev].filter(([id]) => listedNow.has(id)))
      return next.size === prev.size ? prev : next
    })
    setPreview(null)
    setMendPreview(null)
  }, [report])

  const frameRow = (row: HealthRow): void => {
    const bounds = worldBoundsForSelection(scene, [rowNode(row, selectedIds)])
    if (bounds !== null) onFrameBounds([...bounds.min], [...bounds.max])
  }
  const frameRim = (row: HealthRow, rim: number): void => {
    const box = segmentBounds(scene.health_rim_highlight(row.object, rim))
    if (box !== null) onFrameBounds(box.min, box.max)
  }

  const selectRow = (row: HealthRow): void => {
    // A typed merge distance belongs to the object it was typed for.
    if (currentRow !== row) {
      setDistanceText(null)
      setDistanceSettled(null)
    }
    setCurrent({ object: row.object, rim: 0 })
    const node = rowNode(row, selectedIds)
    onSelectNode(node)
    // Show just this object (a definition member through the instance the
    // row selected); the rest of its group or instance draws faded.
    onIsolate(
      [node.kind === 'instance' ? { kind: 'member', object: row.object, instance: node.id } : { kind: 'object', id: row.object }],
      { frame: false },
    )
    frameRow(row)
  }

  const stepRim = (row: HealthRow, delta: number): void => {
    const n = row.rims.length
    if (n === 0) return
    const from = currentRow === row ? currentRim : 0
    const rim = (((from + delta) % n) + n) % n
    setCurrent({ object: row.object, rim })
    frameRim(row, rim)
  }

  /** Run a kernel repair; a refusal toasts its plain-language copy and leaves
   *  the row alone. Returns the call's result, or null when refused. */
  const repair = <T,>(call: () => T): { value: T } | null => {
    try {
      const value = call()
      onGeometryChanged()
      return { value }
    } catch (err) {
      const code = parseKernelErrorCode(err)
      const raw = err instanceof Error ? err.message : String(err)
      onToast?.(kernelErrorMessage(code ?? 'Unknown', raw), code ?? undefined)
      return null
    }
  }

  /** Pointer / keyboard focus on a Close button previews what it will cap. */
  const previewProps = (rows: readonly HealthRow[], only?: readonly number[]) => ({
    onMouseEnter: () => setPreview(capPreviewRims(rows, only)),
    onMouseLeave: () => setPreview(null),
    onFocus: () => setPreview(capPreviewRims(rows, only)),
    onBlur: () => setPreview(null),
  })

  /** Run a batch as one transaction, showing refusals on their rows. */
  const execute = (kind: BatchKind, ops: BatchOp[], busyKey: string, options?: BatchOptions): void => {
    // A split shape's undo label counts shapes, not pieces.
    const label = kind === 'repair' ? () => UNDO_LABEL.rejoin(1) : (n: number) => batchLabel(kind, n)
    if (ops.length === 0 || busy !== null) return
    setBusy(busyKey)
    setWarnings(new Map())
    setPreview(null)
    // Let the busy state paint before the synchronous run.
    setTimeout(() => {
      try {
        const result = runBatch(scene, label, ops, options)
        setWarnings(new Map(result.failed.map((f) => [f.object, failureLine(kind, f.explanation)])))
        if (result.done.length > 0) {
          setCurrent(null)
          onGeometryChanged()
        }
        onToast?.(batchSummary(kind, result))
      } catch (err) {
        const code = parseKernelErrorCode(err)
        const raw = err instanceof Error ? err.message : String(err)
        onToast?.(code !== null ? kernelErrorMessage(code, raw) : raw, code ?? undefined)
      } finally {
        setBusy(null)
      }
    }, 0)
  }

  /** The plan the Mend button of `row` runs: the steps chosen, the typed
   *  distance checked against the object's range. */
  const mendPlanFor = (row: HealthRow, choice: MendChoice = mendChoice): { plan: MendPlan; error: string | null } => {
    const base = { folds: choice.folds, split: choice.split, fill: choice.fill }
    if (!choice.weld) return { plan: { ...base, weld: false }, error: null }
    if (distanceSettled === null) return { plan: { ...base, weld: true }, error: null }
    const parsed = parseMergeDistance(distanceSettled, readWeldRange(scene, row.object))
    return parsed.ok
      ? { plan: { ...base, weld: parsed.meters }, error: null }
      : { plan: { ...base, weld: true }, error: parsed.error }
  }

  /** Whether Mend is offered for `row` at all (before the census). */
  const mendEligible = (row: HealthRow): boolean =>
    !row.accepted && (mendCandidate(row, inSplit(row)) || hasWarpedHoles(row))

  /** Hovering a Mend button: dots for the points it merges and splits, and the rims it fills. */
  const showMendPreview = (rows: readonly HealthRow[], planOf: MendPlan | ((row: HealthRow) => MendPlan)): void => {
    const merge: Float32Array[] = []
    const split: Float32Array[] = []
    const rims: { id: bigint; rim: number }[] = []
    rows.forEach((r, n) => {
      const plan = typeof planOf === 'function' ? planOf(r) : planOf
      const dots = n < MEND_ALL_POINTS_ROWS
      if (dots && plan.weld !== false) merge.push(readMendPoints(scene, r.object, plan, 0, docRev))
      if (dots && plan.split) split.push(readMendPoints(scene, r.object, plan, 1, docRev))
      if (plan.fill) r.rims.forEach((rim, i) => { if (rim.role === 'hole' && rim.closed) rims.push({ id: r.object, rim: i }) })
    })
    setMendPreview({ rims, merge: concatPoints(merge), split: concatPoints(split) })
  }
  const mendPreviewProps = (rows: readonly HealthRow[], plan: MendPlan | ((row: HealthRow) => MendPlan)) => ({
    onMouseEnter: () => showMendPreview(rows, plan),
    onMouseLeave: () => setMendPreview(null),
    onFocus: () => showMendPreview(rows, plan),
    onBlur: () => setMendPreview(null),
  })

  /** Run Mend on `ops` as one undo step behind the busy card. */
  const executeMend = (opsOf: BatchOp[] | (() => BatchOp[]), busyKey: string): void => {
    if ((Array.isArray(opsOf) && opsOf.length === 0) || busy !== null) return
    setBusy(busyKey)
    setWarnings(new Map())
    setPreview(null)
    setMendPreview(null)
    const work = (): void => {
      try {
        // Mend all reads every census here, behind the busy card, not while rendering.
        const ops = Array.isArray(opsOf) ? opsOf : opsOf()
        if (ops.length === 0) {
          onToast?.(MEND.nothingToMend)
          return
        }
        const result = runBatch(scene, (n) => batchLabel('mend', n), ops)
        const lines = new Map<bigint, string>()
        for (const [op, outcome] of mendOutcomes(result)) {
          const note = skippedLine(outcome.skipped)
          if (note !== null) lines.set(op.object, note)
        }
        for (const f of result.failed) lines.set(f.object, failureLine('mend', f.explanation))
        setWarnings(lines)
        if (result.done.length > 0) {
          setCurrent(null)
          // A distance typed for one pass does not carry into the next.
          setDistanceText(null)
          setDistanceSettled(null)
          onGeometryChanged()
        }
        onToast?.(mendToast(result))
      } catch (err) {
        const code = parseKernelErrorCode(err)
        const raw = err instanceof Error ? err.message : String(err)
        onToast?.(code !== null ? kernelErrorMessage(code, raw) : raw, code ?? undefined)
      } finally {
        setBusy(null)
      }
    }
    if (onBusy === undefined) {
      setTimeout(work, 0)
    } else if (!onBusy(MEND.busy, work)) {
      setBusy(null)
      onToast?.(i18n._(BUSY_REFUSED))
    }
  }

  const closeRow = (row: HealthRow, rims: number[] | null): void => {
    setPreview(null)
    const done = repair(
      () => JSON.parse(scene.cap_rims(row.object, rims === null ? null : Uint32Array.from(rims))) as { closed: boolean },
    )
    if (done === null) return
    // A solid now: its row is gone. Otherwise rim indices were re-diagnosed.
    setCurrent(done.value.closed ? null : { object: row.object, rim: 0 })
  }

  const markRow = (row: HealthRow, accepted: boolean): void => {
    repair(() => scene.set_accepted_surface(row.object, accepted))
  }

  const onRowKeyDown = (e: React.KeyboardEvent, row: HealthRow): void => {
    // Keys from a child button (◀ ▶, the actions) keep their native behavior.
    if (e.target !== e.currentTarget) return
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault()
      selectRow(row)
    } else if (e.key === 'Tab' && currentRow === row && row.rims.length > 1) {
      // Solid Inspector style: Tab walks the selected row's rims, stopping at
      // the last (first, with Shift) so focus can then leave the row.
      const next = currentRim + (e.shiftKey ? -1 : 1)
      if (next < 0 || next >= row.rims.length) return
      e.preventDefault()
      stepRim(row, e.shiftKey ? -1 : 1)
    }
  }

  // Every row is a non-solid object, so no rows at all means every object in
  // scope is a solid.
  const noneAtAll = report.rows.length === 0 && objectFilter === null
  const emptyScope = objectFilter === null && scopeIds !== null && scopeIds.length === 0

  const isCollapsed = (key: string, rows: number): boolean => collapsedGroups[key] ?? rows > AUTO_COLLAPSE_ROWS
  const toggleGroup = (key: string, rows: number): void =>
    setCollapsedGroups((m) => ({ ...m, [key]: !isCollapsed(key, rows) }))

  const rowEl = (row: HealthRow, label: string): React.ReactNode => {
    const selected = currentRow === row
    const members = row.definition !== null && row.placements > 1
    const rim = row.rims[currentRim]
    const thisRimClosable = selected && canCloseRow(row) && rim !== undefined && rim.cappable
    const mendChoiceNow = selected && mendEligible(row) ? mendPlanFor(row) : null
    const censusRead =
      mendChoiceNow === null ? null : readMendCensusResult(scene, row.object, mendChoiceNow.plan, docRev)
    const mendCensus = censusRead?.census ?? null
    const censusError = censusRead?.error ?? null
    const mendLine = mendCensus === null || mendChoiceNow === null ? null : mendPreviewLine(mendCensus, mendChoiceNow.plan)
    // The default plan decides whether the row has anything to mend at all:
    // switching every step off must not hide the controls to switch them on.
    const mendDefault = mendChoiceNow === null ? null : readMendCensus(scene, row.object, DEFAULT_MEND_PLAN, docRev)
    const mendAvailable = mendDefault !== null && mendPreviewLine(mendDefault) !== null
    return (
      <div
        key={row.object.toString()}
        role="button"
        tabIndex={0}
        data-health-row={row.object.toString()}
        aria-pressed={selected}
        onClick={() => selectRow(row)}
        onKeyDown={(e) => onRowKeyDown(e, row)}
        style={{
          ...ROW_STYLE,
          flexWrap: selected || warnings.has(row.object) ? 'wrap' : undefined,
          background: selected ? 'var(--surface-row-selected, rgba(80,140,255,0.18))' : 'transparent',
        }}
      >
        <span
          style={{ color: 'var(--text-primary, #eee)', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}
        >
          {label}
        </span>
        {members && <span style={{ color: 'var(--text-faint, #888)', flexShrink: 0 }}>×{row.placements}</span>}
        <span
          style={{
            marginLeft: 'auto',
            color: 'var(--text-faint, #888)',
            overflow: 'hidden',
            textOverflow: 'ellipsis',
            whiteSpace: 'nowrap',
          }}
        >
          {rowDetailText(row, inSplit(row))}
        </span>
        {selected && (
          <span style={{ display: 'flex', alignItems: 'center', gap: '4px', flexShrink: 0 }} data-health-actions>
            {canCloseRow(row) && (
              <button type="button" title={TOOLTIP.close(rowPlacements(row))} style={ACTION_BUTTON_STYLE} {...previewProps([row])} onClick={(e) => { e.stopPropagation(); closeRow(row, null) }}>
                {ACTION.close}
              </button>
            )}
            {mendAvailable && mendChoiceNow !== null && (
              <button
                type="button"
                data-health-mend
                title={TOOLTIP.mend(rowPlacements(row))}
                style={ACTION_BUTTON_STYLE}
                disabled={busy !== null || mendChoiceNow.error !== null || mendLine === null || censusError !== null || distanceText !== distanceSettled}
                {...mendPreviewProps([row], mendChoiceNow.plan)}
                onClick={(e) => {
                  e.stopPropagation()
                  executeMend([mendOp(row, mendChoiceNow.plan)], `mend:${row.object}`)
                }}
              >
                {busy === `mend:${row.object}` ? PANEL.working : ACTION.mend}
              </button>
            )}
            {classifyRow(row, inSplit(row)) === 'flatPanels' && (
              <button
                type="button"
                title={TOOLTIP.addThickness}
                style={ACTION_BUTTON_STYLE}
                disabled={busy !== null}
                onClick={(e) => { e.stopPropagation(); setThickenRows([row]) }}
              >
                {ACTION.addThickness}
              </button>
            )}
            {thisRimClosable && (
              <button
                type="button"
                title={TOOLTIP.closeThis(rowPlacements(row))}
                style={ACTION_BUTTON_STYLE}
                {...previewProps([row], [currentRim])}
                onClick={(e) => { e.stopPropagation(); closeRow(row, [currentRim]) }}
              >
                {ACTION.closeThis}
              </button>
            )}
            <button
              type="button"
              title={row.accepted ? TOOLTIP.unkeep(rowPlacements(row)) : TOOLTIP.keep(rowPlacements(row))}
              style={ACTION_BUTTON_STYLE}
              onClick={(e) => { e.stopPropagation(); markRow(row, !row.accepted) }}
            >
              {row.accepted ? ACTION.unkeep : ACTION.keep}
            </button>
          </span>
        )}
        {selected && row.rims.length > 0 && (
          <span style={{ display: 'flex', alignItems: 'center', flexShrink: 0 }}>
            <button
              type="button"
              aria-label={ACTION.previousOpening}
              title={TOOLTIP.previousOpening}
              disabled={row.rims.length < 2}
              style={STEP_BUTTON_STYLE}
              onClick={(e) => {
                e.stopPropagation()
                stepRim(row, -1)
              }}
            >
              ◀
            </button>
            <span data-health-rim-position style={{ fontSize: '11px', color: 'var(--text-faint, #888)' }}>
              {currentRim + 1}/{row.rims.length}
            </span>
            <button
              type="button"
              aria-label={ACTION.nextOpening}
              title={TOOLTIP.nextOpening}
              disabled={row.rims.length < 2}
              style={STEP_BUTTON_STYLE}
              onClick={(e) => {
                e.stopPropagation()
                stepRim(row, 1)
              }}
            >
              ▶
            </button>
          </span>
        )}
        {selected && classifyRow(row, inSplit(row)) === 'closerLook' && (
          <span data-health-reason style={{ flexBasis: '100%', fontSize: '11px', whiteSpace: 'normal', color: 'var(--text-secondary, #ccc)' }}>
            {REASON_SENTENCE[openReason(row)]} {mendAvailable ? `${i18n._(REASON_MEND_LINE)} ` : ''}{i18n._(REASON_ON_PURPOSE)}
          </span>
        )}
        {selected && (mendAvailable || censusError !== null) && mendChoiceNow !== null && (
          <div
            data-health-mend-panel
            style={{ flexBasis: '100%', fontSize: '11px', whiteSpace: 'normal', color: 'var(--text-secondary, #ccc)' }}
            onClick={(e) => e.stopPropagation()}
          >
            <div data-health-mend-preview>{censusError ?? mendLine ?? mendIdleLine(mendChoiceNow.plan)}</div>
            {mendChoiceNow.error !== null && (
              <div role="alert" style={{ color: 'var(--status-warning, #e8c84a)' }}>{mendChoiceNow.error}</div>
            )}
            <button
              type="button"
              aria-expanded={mendOpen}
              data-health-mend-choose
              style={{ ...STEP_BUTTON_STYLE, textDecoration: 'underline', padding: 0, marginTop: '2px' }}
              onClick={() => setMendOpen((o) => !o)}
            >
              {mendOpen ? '▾' : '▸'} {MEND.choose}
            </button>
            {mendOpen && mendChooser(row)}
          </div>
        )}
        {warnings.has(row.object) && (
          <span
            role="status"
            data-health-warning
            style={{ flexBasis: '100%', fontSize: '11px', color: 'var(--status-warning, #e8c84a)', whiteSpace: 'normal' }}
          >
            {warnings.get(row.object)}
          </span>
        )}
      </div>
    )
  }

  /** The "Choose what to mend" checkboxes and the merge distance. */
  const mendChooser = (row: HealthRow): React.ReactNode => {
    const box = (key: keyof MendChoice, label: string): React.ReactNode => (
      <label style={{ display: 'flex', alignItems: 'center', gap: '4px' }}>
        <input
          type="checkbox"
          checked={mendChoice[key]}
          onChange={(e) => setMendChoice((c) => ({ ...c, [key]: e.target.checked }))}
        />
        {label}
      </label>
    )
    return (
      <div data-health-mend-options style={{ display: 'flex', flexDirection: 'column', gap: '3px', margin: '4px 0 2px 10px' }}>
        {box('weld', MEND.mergeLabel)}
        {mendChoice.weld && (
          <label style={{ display: 'flex', alignItems: 'center', gap: '6px', marginLeft: '18px' }}>
            {MEND.mergeDistance}
            <input
              type="text"
              aria-label={MEND.mergeDistance}
              value={distanceText ?? formatMergeDistance(readDefaultWeld(scene, row.object))}
              spellCheck={false}
              onChange={(e) => setDistanceText(e.target.value)}
              onFocus={(e) => e.currentTarget.select()}
              style={{ ...CONTROL_STYLE, width: '90px' }}
            />
          </label>
        )}
        {box('split', MEND.separateLabel)}
        {box('fill', MEND.fillLabel)}
        {box('folds', MEND.foldsLabel)}
      </div>
    )
  }

  const batchButton = (
    busyKey: string,
    label: string,
    title: string,
    onClick: () => void,
    extra: React.ButtonHTMLAttributes<HTMLButtonElement> = {},
  ): React.ReactNode => (
    <button
      {...extra}
      type="button"
      title={title}
      data-health-batch={busyKey}
      style={ACTION_BUTTON_STYLE}
      disabled={busy !== null || extra.disabled === true}
      onClick={onClick}
    >
      {busy === busyKey ? PANEL.working : label}
    </button>
  )

  /** Batch buttons for a kind group's header. */
  const kindActions = (kind: KindKey, key: string, rows: HealthRow[]): React.ReactNode => {
    const buttons = groupBatchButtons(kind)
    const closable = closableRows(rows)
    const markable = markableRows(rows)
    // Mend all takes the steps chosen, at each object's own default distance.
    const mendAllPlan: MendPlan = { weld: mendChoice.weld, folds: mendChoice.folds, split: mendChoice.split, fill: mendChoice.fill }
    // The census runs for each listed row once its group is open.
    // A distance typed in the chooser applies to each object it fits; the rest
    // keep their own default (the tooltip says so).
    const typedDistance = mendAllPlan.weld !== false && distanceSettled !== null
    // A typed distance that cannot be read, or that fits none of these
    // objects, stops Mend all with the words the row shows, rather than
    // mending everything at the default distance unasked.
    const distanceReads = typedDistance
      ? rows.filter(mendEligible).map((row) => parseMergeDistance(distanceSettled as string, readWeldRange(scene, row.object)))
      : []
    const firstRefusal = distanceReads.find((p) => !p.ok)
    const distanceError =
      firstRefusal !== undefined && !firstRefusal.ok && distanceReads.every((p) => !p.ok) ? firstRefusal.error : null
    const planWithDistance = (row: HealthRow): MendPlan => {
      if (!typedDistance) return mendAllPlan
      const parsed = parseMergeDistance(distanceSettled as string, readWeldRange(scene, row.object))
      return parsed.ok ? { ...mendAllPlan, weld: parsed.meters } : mendAllPlan
    }
    // The census runs for the first rows only (a budget, so opening a group of
    // hundreds of imports stays quick); the rest are read when the button is
    // pressed, behind the busy card. Reads made earlier cost nothing.
    const scan =
      kind === 'closerLook' && !isCollapsed(key, rows.length)
        ? mendableScan(scene, rows, inSplit, docRev, planWithDistance, MEND_ALL_CENSUS_ROWS)
        : { found: [], unread: 0 }
    const mendable = scan.found
    return (
      <>
        {(mendable.length > 0 || scan.unread > 0) &&
          batchButton(
            `mend:${key}`,
            ACTION.mendAll,
            distanceError ?? TOOLTIP.mendAll(mendable.length + scan.unread, typedDistance, scan.unread > 0),
            () => {
              if (distanceError !== null) return
              executeMend(
                () => mendableRows(scene, rows, inSplit, docRev, planWithDistance).map(({ row, plan }) => mendOp(row, plan)),
                `mend:${key}`,
              )
            },
            {
              ...mendPreviewProps(mendable.map(({ row }) => row), planWithDistance),
              disabled: distanceText !== distanceSettled || distanceError !== null,
            },
          )}
        {(mendable.length > 0 || scan.unread > 0) && distanceError !== null && (
          <span role="status" data-health-mend-all-error style={{ fontSize: '11px', color: 'var(--status-warning, #e8c84a)' }}>
            {distanceError}
          </span>
        )}
        {buttons.close && closable.length > 0 &&
          batchButton(
            `close:${key}`,
            ACTION.closeAll,
            TOOLTIP.closeAll(closable.length),
            () => execute('close', closable.map(capOp), `close:${key}`),
            previewProps(closable),
          )}
        {buttons.thicken !== null &&
          (buttons.thicken === 'enabled'
            ? batchButton(`thicken:${key}`, ACTION.addThickness, TOOLTIP.addThicknessAll, () => setThickenRows(rows))
            : batchButton(`thicken:${key}`, ACTION.addThickness, TOOLTIP.noCurvedThickness, () => undefined, { disabled: true }))}
        {buttons.mark && markable.length > 0 &&
          batchButton(
            `mark:${key}`,
            ACTION.keepAll,
            TOOLTIP.keepAll(markable.length),
            () => execute('mark', markable.map((r) => markOp(r)), `mark:${key}`),
          )}
      </>
    )
  }

  /** A split shape's Rejoin button and its inside-wall choice. */
  const familyActions = (family: number, rows: HealthRow[]): React.ReactNode => {
    const keep = keepSheets[family] === true
    const ops = familyRepairOps(rows, keep)
    if (ops.length === 0) return null
    return (
      <>
        <label
          style={{ display: 'flex', alignItems: 'center', gap: '4px', fontSize: '11px', color: 'var(--text-secondary, #ccc)', whiteSpace: 'nowrap' }}
        >
          <input
            type="checkbox"
            checked={keep}
            onChange={(e) => setKeepSheets((m) => ({ ...m, [family]: e.target.checked }))}
          />
          {ACTION.keepInsideWalls}
        </label>
        {batchButton(
          `repair:${family}`,
          ACTION.rejoin,
          keep ? TOOLTIP.rejoinKeepingWalls : TOOLTIP.rejoin,
          () => execute('repair', ops, `repair:${family}`, { cascade: familyCascade }),
          previewProps(rows.filter((r) => r.verdict.kind === 'cappable')),
        )}
      </>
    )
  }

  const groupEl = (
    key: string,
    label: string,
    rows: HealthRow[],
    nested = false,
    actions: React.ReactNode = null,
  ): React.ReactNode => {
    const collapsed = isCollapsed(key, rows.length)
    return (
      <div key={key} data-health-group={key} style={nested ? { marginLeft: '10px' } : undefined}>
        <div style={{ display: 'flex', alignItems: 'center', gap: '6px', flexWrap: 'wrap' }}>
          <button
            type="button"
            aria-expanded={!collapsed}
            onClick={() => toggleGroup(key, rows.length)}
            style={{ ...GROUP_HEADER_STYLE, width: 'auto', flex: '1 1 auto', minWidth: 0 }}
          >
            <span aria-hidden="true" style={{ color: 'var(--text-section)', width: '10px' }}>{collapsed ? '▸' : '▾'}</span>
            <span>{label} ({rows.length})</span>
          </button>
          {actions}
        </div>
        {!collapsed && distinctLabels(rows.map(rowDisplayName)).map((label, i) => rowEl(rows[i], label))}
      </div>
    )
  }

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: '6px' }} data-model-health>
      <div style={{ display: 'flex', gap: '6px', flexWrap: 'wrap' }}>
        <select
          aria-label={t`Scope`}
          value={scope}
          disabled={objectFilter !== null}
          onChange={(e) => onScopeChange(e.target.value as HealthScope)}
          style={{ ...CONTROL_STYLE, flex: '0 0 auto' }}
        >
          {(Object.keys(HEALTH_SCOPE_LABELS) as HealthScope[]).map((s) => (
            <option key={s} value={s}>{HEALTH_SCOPE_LABELS[s]}</option>
          ))}
        </select>
        {scope === 'selection' && objectFilter === null && (
          <button
            type="button"
            style={ACTION_BUTTON_STYLE}
            title={PANEL.useSelectionTitle}
            onClick={() => setSelectionSnapshot(selectionObjectIds(scene, selectedIds))}
          >
            {PANEL.useSelection}
          </button>
        )}
        <input
          type="search"
          aria-label={PANEL.search}
          placeholder={`${PANEL.search}…`}
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={(e) => {
            if (isComposingKey(e)) return
            if (e.key === 'Escape') e.stopPropagation()
          }}
          spellCheck={false}
          style={{ ...CONTROL_STYLE, flex: '1 1 80px', minWidth: 0 }}
        />
      </div>
      <label style={{ display: 'flex', alignItems: 'center', gap: '6px', fontSize: '12px', color: 'var(--text-secondary, #ccc)' }}>
        <input
          type="checkbox"
          checked={showAccepted}
          onChange={(e) => setShowAccepted(e.target.checked)}
        />
        {PANEL.showKept}
      </label>
      {objectFilter !== null && (
        <div style={{ display: 'flex', alignItems: 'center', gap: '6px', fontSize: '12px', color: 'var(--text-secondary, #ccc)' }}>
          <span>{PANEL.showingOne}</span>
          <button type="button" style={STEP_BUTTON_STYLE} onClick={onClearObjectFilter} aria-label={PANEL.showAllObjects} title={PANEL.showAllObjects}>
            ✕
          </button>
        </div>
      )}
      {emptyScope ? (
        <div style={{ fontSize: '12px', color: 'var(--text-muted, #888)', padding: '6px 0' }}>
          {scope === 'last' ? PANEL.nothingImported : PANEL.nothingSelected}
        </div>
      ) : noneAtAll ? (
        <div style={{ fontSize: '12px', color: 'var(--text-muted, #888)', padding: '6px 0' }}>
          {PANEL.allSolid}
        </div>
      ) : listed.length === 0 ? (
        <div style={{ fontSize: '12px', color: 'var(--text-muted, #888)', padding: '6px 0' }}>
          {objectFilter !== null ? PANEL.isSolid : PANEL.noMatches}
        </div>
      ) : (
        <div role="list" style={{ display: 'flex', flexDirection: 'column' }}>
          {KIND_ORDER.map((kind) => {
            if (kind === 'split') {
              return (
                grouped.families.length > 0 && (
                  <div key="split" data-health-group="split">
                    <div style={{ ...GROUP_HEADER_STYLE, cursor: 'default' }}>
                      {KIND_NAME.split} ({grouped.families.length})
                    </div>
                    {grouped.families.map((f) => groupEl(`family-${f.family}`, f.title, f.rows, true, familyActions(f.family, f.rows)))}
                  </div>
                )
              )
            }
            const g = grouped.groups.find((x) => x.key === kind)
            return g === undefined ? null : groupEl(g.key, g.label, g.rows, false, kindActions(g.key, g.key, g.rows))
          })}
        </div>
      )}
      {thickenRows !== null && (
        <ThickenDialog
          count={thickenRows.length}
          onCancel={() => setThickenRows(null)}
          onConfirm={(distance, centered) => {
            const rows = thickenRows
            setThickenRows(null)
            execute('thicken', rows.map((r) => thickenOp(r, distance, centered)), 'thicken')
          }}
        />
      )}
    </div>
  )
}

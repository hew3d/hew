/**
 * healthReport — the typed model behind Model Health (docs/design/
 * import-health.md): a parse of `Scene.health_report_json`, the words the UI
 * uses for each verdict, and the pure grouping / filtering the panel, the
 * Outliner icon, Object Info and the status badge all share.
 *
 * Words (design Revision 2): every user-facing string is in `healthCopy.ts`.
 * A flat panel or curved sheet is something SketchUp lets you draw on
 * purpose and never reads as a problem; only objects with openings, split
 * shapes and objects that need a closer look "need fixing".
 */
import { plural, t } from '@lingui/core/macro'
import type { Scene as WasmScene } from '../wasm/loader'
import { KIND_NAME, PANEL, rowDetail, type KindKey, type OpenReason } from './healthCopy'

export type VerdictKind =
  | 'flat_sheet'
  | 'bent_sheet'
  | 'cappable'
  | 'holes'
  | 'mixed'
  | 'non_manifold'

export interface HealthVerdict {
  kind: VerdictKind
  /** flat_sheet: holes punched through the sheet. */
  holes?: number
  /** holes: how many of the holes are planar / warped. */
  planar?: number
  warped?: number
}

export interface HealthRim {
  role: 'outer' | 'hole'
  planar: boolean
  closed: boolean
  /** A face can fill this rim: closed, planar, not the lip of a flat part of
   *  the surface, and not through a branching vertex. Decides every
   *  "can this be closed" question about a single rim. */
  cappable: boolean
  /** Planar, but runs along the edge of a flat part of the surface (a lip). */
  sheetEdge: boolean
  /** Another piece of the same split shape shares this rim: a seam. */
  shared: boolean
  perimeter: number
  extent: number
  edges: number
}

export interface HealthRow {
  object: bigint
  name: string | null
  /** Owning component definition when the object is a definition member. */
  definition: bigint | null
  definitionName: string | null
  placements: number
  /** Outermost instances placing a definition member; [] for a world object. */
  instances: bigint[]
  /** Marked as an accepted surface. */
  accepted: boolean
  /** Index into `HealthReport.families`. */
  family: number | null
  verdict: HealthVerdict
  rims: HealthRim[]
  branchingVertices: number
  islands: number
  /** How many faces the object has. */
  faces: number
}

export interface HealthFamily {
  members: bigint[]
  sharedRims: number
}

export interface HealthCounts {
  objects: number
  solids: number
  flat_sheets: number
  bent_sheets: number
  cappable: number
  holes: number
  mixed: number
  non_manifold: number
  seam_families: number
}

export interface HealthReport {
  rows: HealthRow[]
  families: HealthFamily[]
  counts: HealthCounts
}

export const EMPTY_COUNTS: HealthCounts = {
  objects: 0,
  solids: 0,
  flat_sheets: 0,
  bent_sheets: 0,
  cappable: 0,
  holes: 0,
  mixed: 0,
  non_manifold: 0,
  seam_families: 0,
}

export const EMPTY_REPORT: HealthReport = { rows: [], families: [], counts: EMPTY_COUNTS }

/* eslint-disable @typescript-eslint/no-explicit-any */
/** Parse `Scene.health_report_json`. Handles arrive as decimal strings. */
export function parseHealthReport(json: string): HealthReport {
  const raw = JSON.parse(json) as any
  const rows: HealthRow[] = (raw.rows ?? []).map((r: any): HealthRow => ({
    object: BigInt(r.object),
    name: r.name ?? null,
    definition: r.definition == null ? null : BigInt(r.definition),
    definitionName: r.definition_name ?? null,
    placements: r.placements ?? 1,
    instances: (r.instances ?? []).map((i: string) => BigInt(i)),
    accepted: r.accepted === true,
    family: r.family ?? null,
    verdict: r.verdict as HealthVerdict,
    rims: ((r.rims ?? []) as any[]).map(
      (rim): HealthRim => ({
        role: rim.role,
        planar: rim.planar === true,
        closed: rim.closed === true,
        cappable: rim.cappable === true,
        sheetEdge: rim.sheet_edge === true,
        shared: rim.shared === true,
        perimeter: rim.perimeter ?? 0,
        extent: rim.extent ?? 0,
        edges: rim.edges ?? 0,
      }),
    ),
    branchingVertices: r.branching_vertices ?? 0,
    islands: r.islands ?? 1,
    faces: r.faces ?? 0,
  }))
  const families: HealthFamily[] = (raw.families ?? []).map((f: any): HealthFamily => ({
    members: (f.members ?? []).map((m: string) => BigInt(m)),
    sharedRims: f.shared_rims ?? 0,
  }))
  return { rows, families, counts: { ...EMPTY_COUNTS, ...(raw.counts ?? {}) } }
}
/* eslint-enable @typescript-eslint/no-explicit-any */

/** Which objects a report covers. `last` = the last import's, `selection` =
 *  the current selection's, `document` = everything. */
export type HealthScope = 'last' | 'selection' | 'document'

export const HEALTH_SCOPE_LABELS: Record<HealthScope, string> = {
  get last() {
    return PANEL.scopeLastImport
  },
  get selection() {
    return PANEL.scopeSelection
  },
  get document() {
    return PANEL.scopeModel
  },
}

// ---------------------------------------------------------------------------
// Classification
// ---------------------------------------------------------------------------

/** A surface (a sheet drawn on purpose, or one the person accepted) versus
 *  an open object that needs attention. */
export type HealthClass = 'surface' | 'open'

export function verdictClass(kind: VerdictKind, accepted = false): HealthClass {
  if (accepted) return 'surface'
  return kind === 'flat_sheet' || kind === 'bent_sheet' ? 'surface' : 'open'
}

/** Stage 2 repairs: rows whose flat openings can take a face. Non-manifold
 *  rows are refused by the kernel and surfaces are Stage 3, so neither offers it. */
export function canCloseRow(row: HealthRow): boolean {
  const v = row.verdict
  return v.kind === 'cappable' || v.kind === 'mixed' || (v.kind === 'holes' && (v.planar ?? 0) > 0)
}

export function rowClass(row: HealthRow): HealthClass {
  return verdictClass(row.verdict.kind, row.accepted)
}

/** One open object of `Scene.health_summary_json`: just enough for its icon. */
export interface HealthOpenEntry {
  object: bigint
  kind: VerdictKind
  accepted: boolean
}

/** `Scene.health_summary_json`: the cheap whole-document read the status
 *  badge and the Outliner icons use (the full report is the panel's). */
export interface HealthSummaryData {
  counts: HealthCounts
  /** Every live object that is not a solid. */
  open: HealthOpenEntry[]
}

export const EMPTY_SUMMARY: HealthSummaryData = { counts: EMPTY_COUNTS, open: [] }

/* eslint-disable @typescript-eslint/no-explicit-any */
/** Parse `Scene.health_summary_json`: `{counts, open: [[handle, kind, accepted], …]}`. */
export function parseHealthSummary(json: string): HealthSummaryData {
  const raw = JSON.parse(json) as any
  const open: HealthOpenEntry[] = (raw.open ?? []).map((e: [string, VerdictKind, boolean]): HealthOpenEntry => ({
    object: BigInt(e[0]),
    kind: e[1],
    accepted: e[2] === true,
  }))
  return { counts: { ...EMPTY_COUNTS, ...(raw.counts ?? {}) }, open }
}
/* eslint-enable @typescript-eslint/no-explicit-any */

/** Read the whole-document summary (cheap: the scene caches per-object verdicts). */
export function readHealthSummary(scene: WasmScene): HealthSummaryData {
  return parseHealthSummary(scene.health_summary_json())
}

/** Per-object icon / badge state for every open object (solids are absent). */
export function objectHealthStates(summary: HealthSummaryData): Map<bigint, HealthClass> {
  const out = new Map<bigint, HealthClass>()
  for (const e of summary.open) out.set(e.object, verdictClass(e.kind, e.accepted))
  return out
}

export interface HealthSummary {
  objects: number
  solids: number
  surfaces: number
  /** Open objects needing attention (accepted surfaces excluded). */
  open: number
}

/** The status badge's numbers, from the whole-document summary. */
export function summarize(summary: HealthSummaryData): HealthSummary {
  let surfaces = 0
  let open = 0
  for (const e of summary.open) {
    if (verdictClass(e.kind, e.accepted) === 'surface') surfaces += 1
    else open += 1
  }
  return { objects: summary.counts.objects, solids: summary.counts.solids, surfaces, open }
}

// ---------------------------------------------------------------------------
// Words
// ---------------------------------------------------------------------------

/* oxlint-disable lingui/no-unlocalized-strings -- returns KindKey identifiers, not copy */
/**
 * The one classification the panel, the wizard and the badge's lists share:
 * split shapes first (any row with a seam family), then flat panels and
 * curved sheets, then objects whose openings can all be closed, and
 * everything else needs a closer look.
 */
export function classifyRow(row: HealthRow, inSplit: boolean = row.family !== null): KindKey {
  if (inSplit) return 'split'
  switch (row.verdict.kind) {
    case 'flat_sheet':
      return 'flatPanels'
    case 'bent_sheet':
      return 'curvedSheets'
    case 'non_manifold':
      return 'closerLook'
    default:
      return row.rims.length > 0 && row.rims.every((r) => r.cappable) ? 'openings' : 'closerLook'
  }
}
/* oxlint-enable lingui/no-unlocalized-strings */

/**
 * The seam families that really are split shapes: at least one piece, and
 * every piece (a member that is not a flat panel) is an open shape whose
 * rims can all take a face. Any other family's members are classified one by
 * one, as if they had no family: closing a piece that can't be closed, or
 * deleting a wall beside a piece that stays open, would only damage the model.
 */
export function splitFamilySet(rows: readonly HealthRow[]): Set<number> {
  const byFamily = new Map<number, HealthRow[]>()
  for (const row of rows) {
    if (row.family === null) continue
    const list = byFamily.get(row.family)
    if (list === undefined) byFamily.set(row.family, [row])
    else list.push(row)
  }
  const out = new Set<number>()
  for (const [family, members] of byFamily) {
    const pieces = members.filter((r) => r.verdict.kind !== 'flat_sheet')
    const closable = (r: HealthRow): boolean =>
      (r.verdict.kind === 'cappable' || r.verdict.kind === 'holes' || r.verdict.kind === 'mixed') &&
      r.rims.length > 0 &&
      r.rims.every((rim) => rim.cappable)
    if (pieces.length > 0 && pieces.every(closable)) out.add(family)
  }
  return out
}

/** How many copies a repair of this row reaches: a definition member is
 *  shared by every placement of its component. */
export function rowPlacements(row: HealthRow): number {
  return row.definition !== null ? row.placements : 1
}

/** The row's name: the definition's name for a member, else its own. */
export function rowDisplayName(row: HealthRow): string {
  return row.definition !== null
    ? (row.definitionName ?? row.name ?? t({ context: 'unnamed item', comment: 'Name shown for a component with no name', message: 'Component' }))
    : (row.name ?? t({ context: 'unnamed item', comment: 'Name shown for an object with no name', message: 'Object' }))
}

/* oxlint-disable lingui/no-unlocalized-strings -- returns OpenReason identifiers, not copy */
/**
 * Why Hew can't close a row with flat faces (design Revision 3, "Reasons"),
 * from the report's facts: open edges that branch or stop short first, then
 * a closed opening off any plane, then one along a flat lip. Meaningful for
 * rows `classifyRow` puts under "can't be closed".
 */
export function openReason(row: HealthRow): OpenReason {
  if (row.branchingVertices > 0 || row.rims.some((r) => !r.closed)) return 'branches'
  if (row.rims.some((r) => !r.planar)) return 'notFlat'
  if (row.rims.some((r) => r.sheetEdge)) return 'lip'
  return 'notFlat'
}
/* oxlint-enable lingui/no-unlocalized-strings */

/** One-line detail under a row's name ("2 openings", "Flat panel", "Open edges branch"). */
export function rowDetailText(row: HealthRow, inSplit?: boolean): string {
  return rowDetail(classifyRow(row, inSplit), row.rims.length, row.accepted, openReason(row))
}

/** Whether the row needs attention (the Outliner's "needs fixing" icon). */
export function needsAttention(row: HealthRow): boolean {
  return rowClass(row) === 'open'
}

// ---------------------------------------------------------------------------
// Filtering and grouping
// ---------------------------------------------------------------------------

export interface HealthFilter {
  query: string
  showAccepted: boolean
  /** Restrict to one object (Object Info's "Show" link). */
  object?: bigint | null
}

/** Rows that survive the panel's search / accepted toggle / object filter. */
export function filterRows(rows: readonly HealthRow[], filter: HealthFilter): HealthRow[] {
  const q = filter.query.trim().toLowerCase()
  return rows.filter((row) => {
    if (filter.object != null && row.object !== filter.object) return false
    if (row.accepted && !filter.showAccepted && filter.object == null) return false
    if (q === '') return true
    const hay = `${row.name ?? ''} ${row.definitionName ?? ''} ${rowDetailText(row)} ${
      KIND_NAME[classifyRow(row)]
    }`.toLowerCase()
    return hay.includes(q)
  })
}

export interface HealthGroup {
  /** The kind's key (`openings`, `flatPanels`, …). */
  key: KindKey
  label: string
  rows: HealthRow[]
}

export interface HealthFamilyGroup {
  /** Index into `HealthReport.families`. */
  family: number
  /** "<name> → N pieces". */
  title: string
  rows: HealthRow[]
}

export interface GroupedHealth {
  /** One group per kind except split shapes (empty groups dropped). */
  groups: HealthGroup[]
  /** Split shapes, one group per family. */
  families: HealthFamilyGroup[]
}

/** Display order of the kinds (`KIND_NAME`'s order). */
export const KIND_ORDER = Object.keys(KIND_NAME) as KindKey[]

/**
 * Group listed rows by kind (empty groups dropped), split shapes by family.
 * Every row is listed once.
 */
export function groupRows(report: HealthReport, rows: readonly HealthRow[]): GroupedHealth {
  const splits = splitFamilySet(report.rows)
  const inSplit = (r: HealthRow): boolean => r.family !== null && splits.has(r.family)
  const groups: HealthGroup[] = []
  for (const key of KIND_ORDER) {
    if (key === 'split') continue
    const members = rows.filter((r) => classifyRow(r, inSplit(r)) === key)
    if (members.length > 0) groups.push({ key, label: KIND_NAME[key], rows: members })
  }
  const byFamily = new Map<number, HealthRow[]>()
  for (const row of rows) {
    if (!inSplit(row)) continue
    const list = byFamily.get(row.family as number)
    if (list === undefined) byFamily.set(row.family as number, [row])
    else list.push(row)
  }
  const families: HealthFamilyGroup[] = [...byFamily.entries()]
    .sort((a, b) => a[0] - b[0])
    .map(([family, members]) => {
      const pieces = report.families[family]?.members.length ?? members.length
      const name = rowDisplayName(members[0])
      return {
        family,
        title: plural(pieces, { one: `${name} → # piece`, other: `${name} → # pieces` }),
        rows: members,
      }
    })
  return { groups, families }
}

/** Number of rows a grouping lists. */
export function groupedRowCount(g: GroupedHealth): number {
  return g.groups.reduce((n, x) => n + x.rows.length, 0) + g.families.reduce((n, x) => n + x.rows.length, 0)
}

/** Every listed row, in display order (the overlay and keyboard stepping). */
export function groupedRows(g: GroupedHealth): HealthRow[] {
  const out: HealthRow[] = []
  for (const key of KIND_ORDER) {
    if (key === 'split') for (const f of g.families) out.push(...f.rows)
    else for (const x of g.groups) if (x.key === key) out.push(...x.rows)
  }
  return out
}

// ---------------------------------------------------------------------------
// Reading from the scene
// ---------------------------------------------------------------------------

let wholeCache: { scene: WasmScene; docRev: number; report: HealthReport } | null = null

/**
 * Read a report from the scene. The whole-document read (the panel's Whole
 * document scope) is cached per `docRev`. Only the panel reads reports; the
 * status badge and the Outliner icons read `readHealthSummary`.
 */
export function readHealthReport(
  scene: WasmScene,
  docRev: number,
  scopeIds: readonly bigint[] | null,
): HealthReport {
  if (scopeIds === null) {
    if (wholeCache !== null && wholeCache.scene === scene && wholeCache.docRev === docRev) {
      return wholeCache.report
    }
    const report = parseHealthReport(scene.health_report_json(null))
    wholeCache = { scene, docRev, report }
    return report
  }
  return parseHealthReport(scene.health_report_json(BigUint64Array.from(scopeIds)))
}

/** Read a report straight from the scene, never from the cache (the wizard
 *  re-reads right after its own batch, before `docRev` has moved). */
export function readHealthReportFresh(scene: WasmScene, scopeIds: readonly bigint[] | null): HealthReport {
  return parseHealthReport(scene.health_report_json(scopeIds === null ? null : BigUint64Array.from(scopeIds)))
}

/** Bounds of a segment-pair list (`[ax,ay,az,bx,by,bz,…]`); null when empty. */
export function segmentBounds(
  segs: ArrayLike<number>,
): { min: [number, number, number]; max: [number, number, number] } | null {
  if (segs.length < 3) return null
  const min: [number, number, number] = [Infinity, Infinity, Infinity]
  const max: [number, number, number] = [-Infinity, -Infinity, -Infinity]
  for (let i = 0; i + 2 < segs.length; i += 3) {
    for (let k = 0; k < 3; k++) {
      const v = segs[i + k]
      if (v < min[k]) min[k] = v
      if (v > max[k]) max[k] = v
    }
  }
  return { min, max }
}

/** What the viewport draws while Model Health is open: the rims of every
 *  listed row (non-manifold ones in the warning color) and the current
 *  finding's rim. */
export interface HealthOverlay {
  ids: bigint[]
  warnIds: bigint[]
  current: { id: bigint; rim: number } | null
  /** The document revision the listed rims were read at: the renderer
   *  re-reads the rim geometry of the listed rows only when this, `ids` or
   *  `warnIds` change, not when only `current` / `capPreview` do. */
  rev: number
  /** While the pointer / focus is on a Close button: the rims it WILL cap,
   *  drawn in the solid color over the usual ones (design §5.3). */
  capPreview: { id: bigint; rim: number }[] | null
  /** Loose line segments to draw (six floats each, `ax ay az bx by bz`,
   *  world space), for drawing that has no Object to key them to: the open
   *  edges of faces Make Solid could not close (rim color) and the edges of
   *  closed loops that lie on no one plane (warn color). Wireframe faces §6. */
  segments?: { open: Float32Array; warn: Float32Array }
}

/** The rims a Close on `rows` would cap: those the kernel flags `cappable`
 *  (never a non-manifold row's). `only` restricts to rim indices. */
export function capPreviewRims(rows: readonly HealthRow[], only?: readonly number[]): { id: bigint; rim: number }[] {
  const out: { id: bigint; rim: number }[] = []
  for (const row of rows) {
    if (row.verdict.kind === 'non_manifold') continue
    row.rims.forEach((rim, i) => {
      if (rim.cappable && (only === undefined || only.includes(i))) out.push({ id: row.object, rim: i })
    })
  }
  return out
}

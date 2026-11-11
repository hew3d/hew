/**
 * mend — the app side of Model Health's Mend (the mesh-repair design,
 * section 7): the preview census a row shows before
 * the click, the plan the "Choose what to mend" disclosure builds, the
 * `hew.solid.mend` op the batch runs, and the sentences the result reads as.
 * Pure of React. Every string is in `healthCopy.ts`.
 */
import type { Scene as WasmScene } from '../wasm/loader'
import { describedErrorCodes, kernelErrorMessage, parseKernelErrorCode } from '../kernelErrors'
import { MEND } from './healthCopy'
import { getLengthSystem, parseLengthToMeters } from '../settings/units'
import { batchSummary, type BatchOp, type BatchResult } from './healthBatch'
import { canCloseRow, classifyRow, type HealthRow } from './healthReport'

/** What the person chose under "Choose what to mend". `weld` is a distance
 *  in meters, `true` for the object's default, `false` for none. */
export interface MendPlan {
  weld: number | boolean
  folds: boolean
  split: boolean
  fill: boolean
}

/** The one-click Mend: every step on, at the default distance. */
export const DEFAULT_MEND_PLAN: MendPlan = { weld: true, folds: true, split: true, fill: true }

/** The JSON `Scene.mend_census_json` and `Scene.mend` read (and the params
 *  of `hew.solid.mend`, minus `object`). */
export function mendPlanParams(plan: MendPlan): Record<string, unknown> {
  return { weld: plan.weld, folds: plan.folds, split: plan.split, fill: plan.fill ? 'holes' : false }
}

export const mendPlanJson = (plan: MendPlan): string => JSON.stringify(mendPlanParams(plan))

/** `Scene.mend_census_json`: what a Mend would do, counted by the kernel
 *  running the Mend on a copy, so each count is the result's. */
export interface MendCensus {
  /** The distance the plan resolved to (meters), or null with no merge. */
  weld: number | null
  weldVertices: number
  weldMaxGap: number
  folds: number
  branchingVertices: number
  islands: number
  /** New objects the Mend makes (loose parts, a pinch's split included). */
  pieces: number
  /** Gaps the fill closes. */
  fillRims: number
  /** Of `fillRims`, those closed flat (one face, or stitched shut) and curved. */
  fillFlat: number
  fillCurved: number
  /** Gaps the fill leaves open (too big, bent too far, or tangled). */
  skippedRims: number
}

function num(v: unknown): number {
  return typeof v === 'number' && Number.isFinite(v) ? v : 0
}

export function parseMendCensus(json: string): MendCensus {
  const r = JSON.parse(json) as Record<string, unknown>
  // `fill_rims` names the object's rims the fill reaches (one gap the merge
  // joins from two rims names both); `filled_rims` counts the gaps closed.
  const fillRims = num(r.filled_rims)
  const fillCurved = Math.min(num(r.fill_curved), fillRims)
  return {
    weld: typeof r.weld === 'number' ? r.weld : null,
    weldVertices: num(r.weld_vertices),
    weldMaxGap: num(r.weld_max_gap),
    folds: num(r.folds),
    branchingVertices: num(r.branching_vertices),
    islands: num(r.islands),
    pieces: num(r.pieces),
    fillRims,
    fillFlat: fillRims - fillCurved,
    fillCurved,
    skippedRims: num(r.skipped_rims),
  }
}

/** The counts a plan acts on: a step the person switched off counts zero. */
export interface MendCounts {
  merge: number
  folds: number
  pinched: number
  loose: number
  fill: number
  flat: number
  curved: number
}

export function mendCounts(c: MendCensus, plan: MendPlan = DEFAULT_MEND_PLAN): MendCounts {
  const merge = plan.weld !== false && c.weld !== null ? c.weldVertices : 0
  const pinched = plan.split ? c.branchingVertices : 0
  const fill = plan.fill ? c.fillRims : 0
  return {
    merge,
    folds: plan.folds ? c.folds : 0,
    pinched,
    // Every new object the commit makes, whether a pinch's split or a loose
    // part made it: the census ran the plan, so this is the result's count.
    loose: plan.split ? c.pieces : 0,
    fill,
    flat: fill > 0 ? c.fillFlat : 0,
    curved: fill > 0 ? c.fillCurved : 0,
  }
}

/** Whether a Mend with this plan would change anything. */
export const hasMendWork = (c: MendCensus, plan: MendPlan = DEFAULT_MEND_PLAN): boolean => {
  const n = mendCounts(c, plan)
  return n.merge + n.folds + n.pinched + n.loose + n.fill > 0
}

/** "a, b and c". */
function joinClauses(parts: string[]): string {
  if (parts.length <= 1) return parts.join('')
  return MEND.and(parts.slice(0, -1).join(', '), parts[parts.length - 1])
}

/** The clauses of the preview, zero steps left out. */
export function mendClauses(c: MendCensus, plan: MendPlan = DEFAULT_MEND_PLAN): string[] {
  const n = mendCounts(c, plan)
  const out: string[] = []
  if (n.merge > 0) out.push(MEND.will.merge(n.merge))
  if (n.pinched > 0) out.push(MEND.will.separatePinched(n.pinched))
  if (n.loose > 0) out.push(MEND.will.separateLoose(n.loose))
  if (n.fill > 0) {
    const fill = MEND.will.fill(n.fill)
    out.push(n.flat + n.curved === n.fill ? MEND.will.fillKinds(fill, n.flat, n.curved) : fill)
  }
  if (n.folds > 0) out.push(MEND.will.removeFolds(n.folds))
  return out
}

/** "Mend will merge about 312 nearby points, separate 2 spots where parts touch at a single point and
 *  fill 7 small gaps (3 flat, 4 curved)." Null when there is nothing to do. */
export function mendPreviewLine(c: MendCensus, plan: MendPlan = DEFAULT_MEND_PLAN): string | null {
  const clauses = mendClauses(c, plan)
  if (clauses.length === 0) return null
  return withStaysOpen(MEND.willLine(joinClauses(clauses)), c, plan)
}

/** Adds "2 gaps will stay open." when the fill is on and leaves gaps. */
function withStaysOpen(line: string, c: MendCensus, plan: MendPlan): string {
  return plan.fill && c.skippedRims > 0 ? `${line} ${MEND.willStayOpen(c.skippedRims)}` : line
}

/** The short form a wizard item carries: "merge about 312 nearby points, fill 7 small gaps". */
export function mendItemLine(c: MendCensus, plan: MendPlan = DEFAULT_MEND_PLAN): string {
  const line = joinClauses(mendClauses(c, plan))
  return line === '' ? line : withStaysOpen(line, c, plan)
}

// ------------------------------------------------------------------ census

const cache = new WeakMap<WasmScene, { generation: string; entries: Map<string, { census: MendCensus | null; error: string | null }> }>()

/** A cheap stand-in for the history generation when the scene (a test
 *  double) has none. */
function generationOf(scene: WasmScene, rev: number): string {
  const g = (scene as { history_generation?: () => bigint }).history_generation
  // The history generation restarts at 0 when another file loads, so the
  // document revision (which moves on a load) is part of the key too.
  return typeof g === 'function' ? `g${g.call(scene)}:r${rev}` : `r${rev}`
}

/**
 * The census of one object under `plan`, asked once per history generation
 * and plan. Null when the scene has no census, the object is gone, or the
 * kernel refuses to read the plan: the row simply offers no Mend.
 */
export function readMendCensus(
  scene: WasmScene,
  object: bigint,
  plan: MendPlan = DEFAULT_MEND_PLAN,
  rev = 0,
): MendCensus | null {
  return readMendCensusResult(scene, object, plan, rev).census
}

/** [`readMendCensus`] that also says why a read failed, in plain words: the
 *  kernel refuses a census the way it would refuse the Mend (a distance out
 *  of range, a hidden or stale target). A failure is cached like a success,
 *  until the history generation moves. */
export function readMendCensusResult(
  scene: WasmScene,
  object: bigint,
  plan: MendPlan = DEFAULT_MEND_PLAN,
  rev = 0,
): { census: MendCensus | null; error: string | null } {
  if (typeof (scene as { mend_census_json?: unknown }).mend_census_json !== 'function') return { census: null, error: null }
  const generation = generationOf(scene, rev)
  let slot = cache.get(scene)
  if (slot === undefined || slot.generation !== generation) {
    slot = { generation, entries: new Map() }
    cache.set(scene, slot)
  }
  const key = `${object}:${mendPlanJson(plan)}`
  const hit = slot.entries.get(key)
  if (hit !== undefined) return hit
  let read: { census: MendCensus | null; error: string | null }
  try {
    read = { census: parseMendCensus(scene.mend_census_json(object, mendPlanJson(plan))), error: null }
  } catch (err) {
    const code = parseKernelErrorCode(err)
    const raw = err instanceof Error ? err.message : String(err)
    read = { census: null, error: code !== null ? kernelErrorMessage(code, raw) : raw }
  }
  // A refusal is remembered too, until the generation moves: the panel
  // re-renders often and must not re-run the kernel for the same answer.
  slot.entries.set(key, read)
  return read
}

/** Whether a row can be offered Mend at all (before asking the census):
 *  an open object that is not a flat panel, a curved sheet or a split
 *  shape's piece, and that the Close button cannot finish alone. */
export function mendCandidate(row: HealthRow, inSplit: boolean): boolean {
  if (row.accepted) return false
  return classifyRow(row, inSplit) === 'closerLook'
}

/** Whether a row has warped holes Mend would fill (a "Has openings" row
 *  carrying both kinds is Close's, with Mend beside it). */
export const hasWarpedHoles = (row: HealthRow): boolean =>
  row.verdict.kind === 'mixed' || (row.verdict.kind === 'holes' && (row.verdict.warped ?? 0) > 0) ||
  (canCloseRow(row) && row.rims.some((r) => !r.planar))

/** The rows of `rows` a Mend button can act on, with their census. */
export function mendableRows(
  scene: WasmScene,
  rows: readonly HealthRow[],
  inSplit: (row: HealthRow) => boolean,
  rev = 0,
  plan: MendPlan | ((row: HealthRow) => MendPlan) = DEFAULT_MEND_PLAN,
  limit = Infinity,
): { row: HealthRow; census: MendCensus; plan: MendPlan }[] {
  return mendableScan(scene, rows, inSplit, rev, plan, limit).found
}

/** [`mendableRows`] with a budget: at most `limit` census reads, and how many
 *  candidate rows were left unread. */
export function mendableScan(
  scene: WasmScene,
  rows: readonly HealthRow[],
  inSplit: (row: HealthRow) => boolean,
  rev = 0,
  plan: MendPlan | ((row: HealthRow) => MendPlan) = DEFAULT_MEND_PLAN,
  limit = Infinity,
): { found: { row: HealthRow; census: MendCensus; plan: MendPlan }[]; unread: number } {
  const found: { row: HealthRow; census: MendCensus; plan: MendPlan }[] = []
  let reads = 0
  let unread = 0
  for (const row of rows) {
    if (!mendCandidate(row, inSplit(row)) && !(!row.accepted && hasWarpedHoles(row))) continue
    const rowPlan = typeof plan === 'function' ? plan(row) : plan
    // Reads already made cost nothing; only a fresh read counts against the budget.
    if (reads >= limit && !hasCensusRead(scene, row.object, rowPlan, rev)) {
      unread += 1
      continue
    }
    if (!hasCensusRead(scene, row.object, rowPlan, rev)) reads += 1
    const census = readMendCensus(scene, row.object, rowPlan, rev)
    if (census !== null && hasMendWork(census, rowPlan)) found.push({ row, census, plan: rowPlan })
  }
  return { found, unread }
}

function hasCensusRead(scene: WasmScene, object: bigint, plan: MendPlan, rev: number): boolean {
  const slot = cache.get(scene)
  return slot !== undefined && slot.generation === generationOf(scene, rev) && slot.entries.has(`${object}:${mendPlanJson(plan)}`)
}

/** The op a Mend button or Mend all runs. */
export const mendOp = (row: HealthRow, plan: MendPlan = DEFAULT_MEND_PLAN): BatchOp => ({
  object: row.object,
  method: 'hew.solid.mend',
  params: mendPlanParams(plan),
})

/** The weld distance range for an object: `[min, max]` meters, or null. */
export function readWeldRange(scene: WasmScene, object: bigint): [number, number] | null {
  const f = (scene as { weld_range?: (o: bigint) => Float64Array }).weld_range
  if (typeof f !== 'function') return null
  const r = f.call(scene, object)
  return r.length >= 2 ? [r[0], r[1]] : null
}

/** The object's default merge distance in meters, or 0. */
export function readDefaultWeld(scene: WasmScene, object: bigint): number {
  const f = (scene as { default_weld_distance?: (o: bigint) => number }).default_weld_distance
  return typeof f === 'function' ? f.call(scene, object) : 0
}

/** Points to draw while the pointer is on a Mend button: `kind` 0 would
 *  merge, 1 would split. Empty when the scene has no such call. */
export function readMendPoints(scene: WasmScene, object: bigint, plan: MendPlan, kind: 0 | 1, rev = 0): Float32Array {
  const f = (scene as { mend_census_points?: (o: bigint, p: string, k: number) => Float32Array }).mend_census_points
  if (typeof f !== 'function') return new Float32Array()
  const generation = generationOf(scene, rev)
  let slot = pointCache.get(scene)
  if (slot === undefined || slot.generation !== generation) {
    slot = { generation, entries: new Map() }
    pointCache.set(scene, slot)
  }
  const key = `${object}:${kind}:${mendPlanJson(plan)}`
  const hit = slot.entries.get(key)
  if (hit !== undefined) return hit
  let points: Float32Array = new Float32Array()
  try {
    points = f.call(scene, object, mendPlanJson(plan), kind)
  } catch {
    points = new Float32Array()
  }
  slot.entries.set(key, points)
  return points
}

const pointCache = new WeakMap<WasmScene, { generation: string; entries: Map<string, Float32Array> }>()

/** Join point arrays without spreading them into call arguments (a large
 *  import has more floats than an engine's argument limit). */
export function concatPoints(parts: readonly Float32Array[]): Float32Array {
  let total = 0
  for (const p of parts) total += p.length
  const out = new Float32Array(total)
  let at = 0
  for (const p of parts) {
    out.set(p, at)
    at += p.length
  }
  return out
}

// ------------------------------------------------------------------ result

/** What `hew.solid.mend` returned, the parts the sentences use. */
export interface MendOutcome {
  welded: number
  folds: number
  split: number
  /** Edges cut apart where more than two faces met. */
  splitEdges: number
  /** Loose parts that became objects of their own. */
  pieces: number
  filled: number
  closed: boolean
  skipped: string[]
}

/** The copy table's key for a skipped gap's reason: the API names it by its
 *  refusal code (`fill_crosses_faces`), the table by the kernel's variant
 *  (`FillCrossesFaces`). */
function reasonCode(reason: unknown): string {
  const m = typeof reason === 'string' ? /^[A-Za-z][A-Za-z0-9_]*/.exec(reason) : null
  // oxlint-disable-next-line lingui/no-unlocalized-strings -- an error code, not copy
  if (m === null) return 'Unknown'
  return m[0].includes('_') || /^[a-z]/.test(m[0])
    ? m[0].split('_').map((w) => w.charAt(0).toUpperCase() + w.slice(1)).join('')
    : m[0]
}

/** A skipped gap's reason in plain words (the app's copy for the code). */
export function skippedReason(reason: unknown): string {
  const code = reasonCode(reason)
  return describedErrorCodes().includes(code) ? kernelErrorMessage(code, '') : MEND.skippedFallback
}

export function parseMendOutcome(result: unknown): MendOutcome {
  const r = (result ?? {}) as Record<string, unknown>
  const skipped = Array.isArray(r.skipped_rims) ? (r.skipped_rims as { reason?: unknown }[]) : []
  return {
    welded: num(r.welded_vertices),
    folds: num(r.folds_removed),
    split: num(r.split_vertices),
    splitEdges: num(r.split_edges),
    // The new objects' ids.
    pieces: Array.isArray(r.pieces) ? r.pieces.length : num(r.pieces),
    filled: num(r.filled_rims),
    closed: r.closed === true,
    skipped: skipped.map((s) => skippedReason(s.reason)),
  }
}

/** The outcomes of a finished batch, per op that ran. */
export function mendOutcomes(result: BatchResult): Map<BatchOp, MendOutcome> {
  const out = new Map<BatchOp, MendOutcome>()
  for (const op of result.done) {
    if (op.method === 'hew.solid.mend') out.set(op, parseMendOutcome(result.results?.get(op)))
  }
  return out
}

/** "Merged 312 points, separated 2 loose parts, filled 7 gaps." (null when nothing changed). */
export function mendDoneClauses(
  o: Pick<MendOutcome, 'welded' | 'folds' | 'split' | 'splitEdges' | 'pieces' | 'filled'>,
): string | null {
  const parts: string[] = []
  if (o.welded > 0) parts.push(MEND.did.merged(o.welded))
  if (o.split > 0) parts.push(MEND.did.separatedPinched(o.split))
  else if (o.splitEdges > 0) parts.push(MEND.did.separatedEdges(o.splitEdges))
  if (o.pieces > 0) parts.push(MEND.did.separatedLoose(o.pieces))
  if (o.filled > 0) parts.push(MEND.did.filled(o.filled))
  if (o.folds > 0) parts.push(MEND.did.removedFolds(o.folds))
  if (parts.length === 0) return null
  return MEND.didLine(parts.join(', '))
}

/** The row's note under a Mend that left gaps open: "3 gaps stayed open. <reasons>". */
export function skippedLine(skipped: readonly string[]): string | null {
  if (skipped.length === 0) return null
  const reasons = [...new Set(skipped)]
  return `${MEND.skippedHead(skipped.length)} ${reasons.join(' ')}`
}

/** The totals of several outcomes. */
export function sumOutcomes(list: readonly MendOutcome[]): MendOutcome {
  const total: MendOutcome = { welded: 0, folds: 0, split: 0, splitEdges: 0, pieces: 0, filled: 0, closed: list.length > 0, skipped: [] }
  for (const o of list) {
    total.welded += o.welded
    total.folds += o.folds
    total.split += o.split
    total.splitEdges += o.splitEdges
    total.pieces += o.pieces
    total.filled += o.filled
    total.closed &&= o.closed
    total.skipped.push(...o.skipped)
  }
  return total
}

/** The detail sentence of the toast: counts, "It's solid now." for one
 *  object that closed, and a pointer to the list for gaps left open. */
export function mendResultSentence(list: readonly MendOutcome[]): string {
  const total = sumOutcomes(list)
  const parts: string[] = []
  const done = mendDoneClauses(total)
  if (done !== null) parts.push(done)
  if (list.length === 1 && total.closed) parts.push(MEND.solidNow)
  if (total.skipped.length > 0) parts.push(MEND.skippedToast(total.skipped.length))
  return parts.join(' ')
}

/** A Mend batch's toast: "Mended 3 objects. 1 couldn't be mended; see the
 *  list." and what changed ("Merged 312 points, ... It's solid now."). */
export function mendToast(result: BatchResult): string {
  const outcomes = [...mendOutcomes(result).values()]
  const detail = mendResultSentence(outcomes)
  const head = batchSummary('mend', result)
  if (outcomes.length === 1 && result.failed.length === 0 && detail !== '') return detail
  return detail === '' ? head : `${head} ${detail}`
}

// --------------------------------------------------------- merge distance

const trimZeros = (x: number): string => x.toFixed(9).replace(/0+$/, '').replace(/\.$/, '')

/** A merge distance for the field: millimeters (inches in imperial) with
 *  three significant digits, since the usual length display rounds a few
 *  microns to zero. */
export function formatMergeDistance(meters: number, round: 'nearest' | 'up' | 'down' = 'nearest'): string {
  const imperial = getLengthSystem() === 'imperial'
  const value = imperial ? meters / 0.0254 : meters * 1000
  let shown = Number(value.toPrecision(3))
  if (round !== 'nearest' && value > 0) {
    // A range bound rounds inward, so the number printed is always accepted.
    const step = 10 ** (Math.floor(Math.log10(value)) - 2)
    const steps = round === 'up' ? Math.ceil(value / step - 1e-9) : Math.floor(value / step + 1e-9)
    shown = Number((steps * step).toPrecision(3))
  }
  // oxlint-disable-next-line lingui/no-unlocalized-strings -- unit symbols, as the length fields show them
  return `${trimZeros(shown)} ${imperial ? 'in' : 'mm'}`
}

export type MergeDistance = { ok: true; meters: number } | { ok: false; error: string }

/** The typed merge distance, checked against the kernel's `weld_range`. A
 *  bare number is read in the unit the field shows (millimeters, or inches
 *  in imperial), whatever the model's length unit; a typed unit wins. */
export function parseMergeDistance(text: string, range: readonly [number, number] | null): MergeDistance {
  const meters = parseLengthToMeters(text, getLengthSystem() === 'imperial' ? 'dec_in' : 'mm')
  if (meters === null || !Number.isFinite(meters) || meters <= 0) return { ok: false, error: MEND.distanceInvalid }
  if (range !== null && (meters < range[0] * (1 - 1e-9) || meters > range[1] * (1 + 1e-9))) {
    return { ok: false, error: MEND.distanceRange(formatMergeDistance(range[0], 'up'), formatMergeDistance(range[1], 'down')) }
  }
  return { ok: true, meters }
}

/** Why Mend is idle for `plan`: no step ticked, or the ticked steps have nothing to do here. */
export function mendIdleLine(plan: MendPlan): string {
  const verbs = [
    plan.weld !== false ? MEND.verb.merge : '',
    plan.split ? MEND.verb.separate : '',
    plan.fill ? MEND.verb.fill : '',
    plan.folds ? MEND.verb.remove : '',
  ].filter((v) => v !== '')
  if (verbs.length === 0) return MEND.nothingChosen
  if (verbs.length === 4) return MEND.nothingToMend
  return MEND.nothingToDo(verbs.length === 1 ? verbs[0] : MEND.or(verbs.slice(0, -1).join(', '), verbs[verbs.length - 1]))
}

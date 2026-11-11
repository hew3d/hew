/**
 * healthBatch — Model Health's "fix many" (docs/design/import-health.md §5.4,
 * F10): a batch of per-object repairs run as ONE `hew.doc.transact` frame
 * through the Hew API (`Scene.api_dispatch`), so the whole batch is one undo
 * entry and anything the panel can do in bulk a script can do too.
 *
 * Refusals are never skipped silently: the transaction is all-or-nothing, so
 * when it refuses at `failed_index` the refused object is dropped from the
 * batch, its explanation is remembered for the row, and the rest re-runs —
 * repeating until the batch succeeds or is empty. Pure of React; the panel
 * and Object Info call `runBatch` and show the result.
 */
import { i18n } from '@lingui/core'
import { msg, t } from '@lingui/core/macro'
import type { Scene as WasmScene } from '../wasm/loader'
import { canCloseRow, type HealthFamilyGroup, type HealthGroup, type HealthRow } from './healthReport'
import { SPLIT_PIECE_LEFT, batchToast, rowWarning, UNDO_LABEL, type KindKey } from './healthCopy'
import { describedErrorCodes, kernelErrorMessage } from '../kernelErrors'

/** One repair: an API command on one object. */
export interface BatchOp {
  object: bigint
  /** `hew.solid.cap` | `hew.solid.thicken` | `hew.solid.mark_surface` | `hew.solid.delete_faces`
   *  | `hew.solid.mend`. */
  method: string
  /** The command's params besides `object`. */
  params?: Record<string, unknown>
}

export interface BatchFailure {
  object: bigint
  method: string
  explanation: string
}

export interface BatchResult {
  done: BatchOp[]
  failed: BatchFailure[]
  /** What each finished command returned (a Mend's report). */
  results?: Map<BatchOp, unknown>
}

export interface BatchOptions {
  /** When `failed` is refused, other ops that must not run without it, with
   *  the explanation to show on their rows. */
  cascade?: (failed: BatchOp, remaining: readonly BatchOp[]) => { ops: BatchOp[]; explanation: string } | null
}

export const STALE_OBJECT_EXPLANATION = msg`This object is no longer in the model.`

let frameId = 0

interface Reply {
  result?: { results?: unknown[] } & Record<string, unknown>
  error?: {
    code?: number
    message?: string
    data?: { failed_index?: number; explanation?: string; refusal?: string }
  }
}

/** `cap_would_be_coplanar` → `CapWouldBeCoplanar`. */
function pascalCase(snake: string): string {
  return snake
    .split('_')
    .map((w) => w.charAt(0).toUpperCase() + w.slice(1))
    .join('')
}

/** A refusal's reason in plain words: the app's copy for the kernel code
 *  when it has one, else the API's own explanation. */
export function refusalReason(data: { explanation?: string; refusal?: string } | undefined, fallback: string): string {
  const explanation = data?.explanation ?? fallback
  const code = data?.refusal === undefined ? null : pascalCase(data.refusal)
  return code !== null && describedErrorCodes().includes(code) ? kernelErrorMessage(code, explanation) : explanation
}

function send(scene: WasmScene, conn: number, method: string, params: unknown): Reply {
  const raw = scene.api_dispatch(conn, JSON.stringify({ jsonrpc: '2.0', id: ++frameId, method, params }))
  if (raw === undefined) throw new Error(`${method} gave no reply`)
  return JSON.parse(raw) as Reply
}

/**
 * Run `ops` as one transaction (labeled `label(n)` for the n ops that
 * finally ran), retrying past refusals. Throws only for an error that is not
 * a per-command refusal (a broken connection, a malformed frame).
 */
export function runBatch(
  scene: WasmScene,
  label: (n: number) => string,
  ops: readonly BatchOp[],
  options: BatchOptions = {},
): BatchResult {
  const done: BatchOp[] = []
  const failed: BatchFailure[] = []
  const results = new Map<BatchOp, unknown>()
  if (ops.length === 0) return { done, failed }
  const conn = scene.api_connection_open()
  try {
    for (const [method, params] of [
      ['hew.meta.hello', { protocol: 1 }],
      ['hew.doc.attach', {}],
    ] as const) {
      const r = send(scene, conn, method, params)
      if (r.error !== undefined) throw new Error(r.error.message ?? `${method} failed`)
    }
    let remaining = [...ops]
    // Each pass either finishes or removes at least one op.
    for (let pass = 0; pass <= ops.length && remaining.length > 0; pass++) {
      const ids = scene.object_public_ids(BigUint64Array.from(remaining.map((o) => o.object)))
      const idOf = new Map<BatchOp, string>()
      const live: BatchOp[] = []
      remaining.forEach((op, i) => {
        if (ids[i]) {
          live.push(op)
          idOf.set(op, ids[i])
        } else {
          failed.push({ object: op.object, method: op.method, explanation: i18n._(STALE_OBJECT_EXPLANATION) })
        }
      })
      remaining = live
      if (remaining.length === 0) break
      const reply = send(scene, conn, 'hew.doc.transact', {
        label: label(remaining.length),
        commands: remaining.map((op) => ({
          method: op.method,
          params: { object: idOf.get(op), ...(op.params ?? {}) },
        })),
      })
      if (reply.error === undefined) {
        const per = reply.result?.results
        remaining.forEach((op, i) => {
          if (Array.isArray(per)) results.set(op, per[i])
        })
        done.push(...remaining)
        remaining = []
        break
      }
      const index = reply.error.data?.failed_index
      const fallback = (): string => t`Hew couldn't make that change.`
      if (typeof index !== 'number' || index < 0 || index >= remaining.length) {
        throw new Error(reply.error.data?.explanation ?? reply.error.message ?? fallback())
      }
      const bad = remaining[index]
      const explanation = refusalReason(reply.error.data, reply.error.message ?? fallback())
      failed.push({ object: bad.object, method: bad.method, explanation })
      const dropped = new Set<BatchOp>([bad])
      const extra = options.cascade?.(bad, remaining)
      if (extra !== undefined && extra !== null) {
        for (const op of extra.ops) {
          if (dropped.has(op) || !remaining.includes(op)) continue
          dropped.add(op)
          failed.push({ object: op.object, method: op.method, explanation: extra.explanation })
        }
      }
      remaining = remaining.filter((op) => !dropped.has(op))
    }
  } finally {
    scene.api_connection_close(conn)
  }
  return results.size > 0 ? { done, failed, results } : { done, failed }
}

// ---------------------------------------------------------------------------
// Plans: which ops a header button runs
// ---------------------------------------------------------------------------

export type BatchKind = 'close' | 'mark' | 'thicken' | 'repair' | 'mend'

/** Undo-entry label for each batch kind. */
export function batchLabel(kind: BatchKind, n: number): string {
  switch (kind) {
    case 'close':
      return UNDO_LABEL.close(n)
    case 'mark':
      return UNDO_LABEL.keep(n)
    case 'thicken':
      return UNDO_LABEL.thicken(n)
    case 'repair':
      return UNDO_LABEL.rejoin(n)
    case 'mend':
      return UNDO_LABEL.mend(n)
  }
}

const ACTION_ROW: Record<BatchKind, 'close' | 'thicken' | 'keep' | 'mend'> = {
  close: 'close',
  mark: 'keep',
  thicken: 'thicken',
  repair: 'close',
  mend: 'mend',
}

/** The warning line under a row a batch refused. */
export function failureLine(kind: BatchKind, explanation: string): string {
  return rowWarning(ACTION_ROW[kind], explanation)
}

/** "Closed 11 objects. 1 couldn't be closed; see the list." (also the
 *  all-failed and nothing-failed forms). */
export function batchSummary(kind: BatchKind, result: BatchResult): string {
  return batchToast(kind, result.done.length, result.failed.length)
}

export const capOp = (row: HealthRow): BatchOp => ({ object: row.object, method: 'hew.solid.cap' })
export const markOp = (row: HealthRow, accepted = true): BatchOp => ({
  object: row.object,
  method: 'hew.solid.mark_surface',
  params: { accepted },
})
export const thickenOp = (row: HealthRow, distance: number, centered: boolean): BatchOp => ({
  object: row.object,
  method: 'hew.solid.thicken',
  params: centered ? { distance, centered: true } : { distance },
})
export const deleteFacesOp = (row: HealthRow): BatchOp => ({ object: row.object, method: 'hew.solid.delete_faces' })

/** Rows a group's **Close all** caps: those with a flat opening to fill. */
export const closableRows = (rows: readonly HealthRow[]): HealthRow[] => rows.filter(canCloseRow)

/** Rows **Keep all** keeps (those not already kept). */
export const markableRows = (rows: readonly HealthRow[]): HealthRow[] => rows.filter((r) => !r.accepted)

/** Which batch buttons a group carries. */
export function groupBatchButtons(kind: KindKey): { close: boolean; mark: boolean; thicken: 'enabled' | 'disabled' | null } {
  switch (kind) {
    case 'openings':
      return { close: true, mark: false, thicken: null }
    case 'flatPanels':
      return { close: false, mark: true, thicken: 'enabled' }
    case 'curvedSheets':
      return { close: false, mark: true, thicken: 'disabled' }
    case 'closerLook':
      return { close: false, mark: true, thicken: null }
    case 'split':
      return { close: false, mark: false, thicken: null }
  }
}

/** Convenience for a header's rows. */
export const groupRowsOf = (g: HealthGroup | HealthFamilyGroup): HealthRow[] => g.rows

/**
 * The split-shape repair: close every piece (a member that is not a flat
 * panel) and delete every flat-panel member (the inside wall), or keep it
 * when `keepSheets`.
 */
export function familyRepairOps(rows: readonly HealthRow[], keepSheets: boolean): BatchOp[] {
  const caps = rows.filter((r) => r.verdict.kind !== 'flat_sheet').map(capOp)
  const sheets = rows
    .filter((r) => r.verdict.kind === 'flat_sheet')
    .map((r) => (keepSheets ? markOp(r) : deleteFacesOp(r)))
  return [...caps, ...sheets]
}

/** An inside wall must not go when a piece it depends on couldn't be closed. */
export const familyCascade: NonNullable<BatchOptions['cascade']> = (failed, remaining) => {
  if (failed.method !== 'hew.solid.cap') return null
  const sheets = remaining.filter((op) => op.method === 'hew.solid.delete_faces')
  return sheets.length === 0
    ? null
    : { ops: sheets, explanation: i18n._(SPLIT_PIECE_LEFT) }
}

/**
 * fixPlan — the pure plan behind the Make Solid wizard (docs/design/
 * import-health.md, Revisions 2 and 3): sort a health report into the five
 * kinds the wizard and the Model Health panel both show, then turn the
 * person's per-page choices into ONE ordered command list for `runBatch`.
 *
 * Kinds (`KIND_NAME`): split shapes (rows with a seam family, one item per
 * family), objects with openings (every opening can take a flat face), flat
 * panels, curved sheets, and "can't be closed" (everything else open).
 * Rows the person kept in Model Health are listed too: keeping only hides a
 * row from that list, and Make Solid is where they come to change their mind.
 *
 * Each item is made solid or left alone: its page's choice decides, unless
 * the person ticked or unticked it under "Choose for each item". Leaving an
 * item writes nothing. Command order: split shapes first (close the seams
 * between the pieces, then delete or keep the inside walls), then openings
 * (close every opening), then flat panels (add thickness). Curved sheets and
 * "can't be closed" items never produce a command.
 */
import { i18n } from '@lingui/core'
import { classifyRow, openReason, rowClass, rowDisplayName, rowPlacements, splitFamilySet, type HealthReport, type HealthRow } from './healthReport'
import {
  capOp,
  deleteFacesOp,
  rejoinSolidOp,
  thickenOp,
  type BatchOp,
  type BatchOptions,
} from './healthBatch'
import { SPLIT_PIECE_LEFT, WIZARD, rowDetail, type KindKey, type WizardScope } from './healthCopy'
import { defaultThicknessText, parseThickness, type ThickenSide } from './thickenSettings'

export { classifyRow }

/** One line of a wizard list: an object, or a whole split shape. */
export interface FixItem {
  /** Stable key: the object's handle, or `family-<n>` for a split shape. */
  key: string
  kind: KindKey
  /** The rows the item stands for (one, or every piece of a split shape). */
  rows: HealthRow[]
  /** The name shown in the list. */
  name: string
  /** How many copies of a component the item reaches (1 for plain objects). */
  copies: number
  /** Index into `HealthReport.families` for a split shape. */
  family: number | null
}

/** The wizard's pages, in order (the overview and the ready/done pages aside). */
// oxlint-disable-next-line lingui/no-unlocalized-strings -- KindKey identifiers, not copy
export const PAGE_ORDER: KindKey[] = ['openings', 'split', 'flatPanels', 'curvedSheets', 'closerLook']

export interface FixPlan {
  items: Record<KindKey, FixItem[]>
  /** Objects the report covers. */
  objects: number
  solids: number
  /** Flat panels and curved sheets, kept or not. */
  panels: number
  /** Objects that need fixing (not solid, not a panel or sheet). */
  fix: number
}

/** Build the plan from a report. */
export function buildPlan(report: HealthReport): FixPlan {
  const items: Record<KindKey, FixItem[]> = { openings: [], split: [], flatPanels: [], curvedSheets: [], closerLook: [] }
  const families = new Map<number, HealthRow[]>()
  // A split shape needs a piece besides its walls; a family left with only
  // walls is judged wall by wall, like any flat panel.
  const splits = splitFamilySet(report.rows)
  const live = new Set<number>()
  for (const row of report.rows) {
    if (row.family !== null && splits.has(row.family) && row.verdict.kind !== 'flat_sheet') live.add(row.family)
  }
  let fix = 0
  for (const row of report.rows) {
    if (rowClass(row) === 'open') fix += 1
    const kind = classifyRow(row, row.family !== null && live.has(row.family))
    if (kind === 'split') {
      const list = families.get(row.family as number)
      if (list === undefined) families.set(row.family as number, [row])
      else list.push(row)
      continue
    }
    items[kind].push({
      key: row.object.toString(),
      kind,
      rows: [row],
      name: rowDisplayName(row),
      copies: rowPlacements(row),
      family: null,
    })
  }
  for (const [family, rows] of [...families.entries()].sort((a, b) => a[0] - b[0])) {
    items.split.push({
      key: `family-${family}`,
      kind: 'split',
      rows,
      name: rowDisplayName(rows[0]),
      copies: Math.max(...rows.map(rowPlacements)),
      family,
    })
  }
  const objects = report.counts.objects
  return {
    items,
    objects,
    solids: report.counts.solids,
    panels: Math.max(0, objects - report.counts.solids - fix),
    fix,
  }
}

/** The pages that have anything on them, in wizard order. */
export function pagesWithItems(plan: FixPlan): KindKey[] {
  return PAGE_ORDER.filter((k) => plan.items[k].length > 0)
}

/** The person's choice on every page. */
export interface FixChoices {
  openings: 'close' | 'leave'
  split: 'rejoin' | 'rejoinSolid' | 'keepWalls' | 'leave'
  flatPanels: 'keep' | 'thicken'
  thickness: { text: string; side: ThickenSide }
  /**
   * "Choose for each item": item key → make it solid (true) or leave it
   * (false), whatever its page's choice. Items not in the map follow the page.
   */
  perItem: ReadonlyMap<string, boolean>
}

/**
 * The recommended choices. Openings are usually deliberate (a duct, a pipe,
 * a box without a lid), so they stay open, except on a selection: the
 * person picked those objects to make them solid.
 */
export function defaultChoices(scope: WizardScope = 'model'): FixChoices {
  return {
    openings: scope === 'selection' ? 'close' : 'leave',
    split: 'rejoin',
    flatPanels: 'keep',
    thickness: { text: defaultThicknessText(), side: 'front' },
    perItem: new Map(),
  }
}

/** Whether `kind`'s page choice makes its items solid. */
export function pageActs(kind: KindKey, choices: FixChoices): boolean {
  switch (kind) {
    case 'openings':
      return choices.openings === 'close'
    case 'split':
      return choices.split !== 'leave'
    case 'flatPanels':
      return choices.flatPanels === 'thicken'
    case 'curvedSheets':
    case 'closerLook':
      return false
  }
}

/** Whether `item` will be made solid: its own tick, else its page's choice. */
export function itemActs(item: FixItem, choices: FixChoices): boolean {
  if (item.kind === 'curvedSheets' || item.kind === 'closerLook') return false
  return choices.perItem.get(item.key) ?? pageActs(item.kind, choices)
}

/**
 * Carry the person's per-item ticks across a re-read of the model: an item
 * keeps the tick of any object it stands for (split shape numbers can
 * change between reads, object handles don't).
 */
export function carryPerItem(before: FixPlan, after: FixPlan, perItem: ReadonlyMap<string, boolean>): Map<string, boolean> {
  // Keyed by object AND kind: ticking a split shape means "rejoin it", never
  // "thicken its wall" should the wall turn up on its own as a flat panel.
  const ticks = new Map<string, boolean>()
  const keyOf = (kind: KindKey, object: bigint): string => `${kind}:${object}`
  for (const kind of PAGE_ORDER) {
    for (const item of before.items[kind]) {
      const tick = perItem.get(item.key)
      if (tick !== undefined) for (const r of item.rows) ticks.set(keyOf(kind, r.object), tick)
    }
  }
  const out = new Map<string, boolean>()
  for (const kind of PAGE_ORDER) {
    for (const item of after.items[kind]) {
      const row = item.rows.find((r) => ticks.has(keyOf(kind, r.object)))
      if (row !== undefined) out.set(item.key, ticks.get(keyOf(kind, row.object)) as boolean)
    }
  }
  return out
}

const acting = (plan: FixPlan, choices: FixChoices, kind: KindKey): FixItem[] =>
  plan.items[kind].filter((i) => itemActs(i, choices))

/** The pages whose items the person chooses for (the rest can't be made solid). */
// oxlint-disable-next-line lingui/no-unlocalized-strings -- KindKey identifiers, not copy
const CHOOSABLE: KindKey[] = ['openings', 'split', 'flatPanels']

/** How many items the person's choices leave as they are (openings, split shapes, flat panels). */
export function leftByChoice(plan: FixPlan, choices: FixChoices): number {
  return CHOOSABLE.reduce((n, kind) => n + plan.items[kind].length - acting(plan, choices, kind).length, 0)
}

/** How many listed items stay as they are, by choice or because Hew can't make them solid. */
export function leftTotal(plan: FixPlan, choices: FixChoices): number {
  return PAGE_ORDER.reduce((n, kind) => n + plan.items[kind].length - acting(plan, choices, kind).length, 0)
}

/** An item's detail line: its kind in plain words (the short reason for one that can't be closed). */
export function itemDetail(item: FixItem): string {
  const row = item.rows[0]
  return rowDetail(item.kind, row.rims.length, row.accepted, openReason(row))
}

/** Close only a split piece's seams (the rims another piece shares). */
const capSeamsOp = (row: HealthRow, rims: number[]): BatchOp => ({
  object: row.object,
  method: 'hew.solid.cap',
  params: { rims },
})

export type FixCommands =
  | { ok: true; ops: BatchOp[]; options: BatchOptions }
  | { ok: false; error: string }

/**
 * The ordered command list for `choices`. Fails (with the plain message for
 * the thickness field) only when Add thickness is chosen for at least one
 * panel and the thickness doesn't parse.
 */
export function planCommands(plan: FixPlan, choices: FixChoices): FixCommands {
  const ops: BatchOp[] = []
  /** The family each cap op belongs to, and each inside-wall op, so a refused piece only keeps its own walls. */
  const capFamily = new Map<BatchOp, number>()
  const wallFamily = new Map<BatchOp, number>()

  const keepWalls = choices.split === 'keepWalls'
  const walls: BatchOp[] = []
  for (const item of acting(plan, choices, 'split')) {
    // One solid: a single command carries the whole family; the kernel caps
    // the seams, removes the walls, and unions in one step.
    if (choices.split === 'rejoinSolid') {
      ops.push(rejoinSolidOp(item.rows))
      continue
    }
    // A split shape's pieces are every member that is not a flat wall; each
    // closes only where it meets another piece, so openings the shape was
    // drawn with stay open.
    for (const row of item.rows) {
      if (row.verdict.kind === 'flat_sheet') continue
      const seams = row.rims.flatMap((rim, i) => (rim.shared ? [i] : []))
      if (seams.length === 0) continue
      const op = capSeamsOp(row, seams)
      ops.push(op)
      capFamily.set(op, item.family as number)
    }
    if (keepWalls) continue
    for (const row of item.rows) {
      if (row.verdict.kind === 'flat_sheet') {
        const op = deleteFacesOp(row)
        walls.push(op)
        wallFamily.set(op, item.family as number)
      }
    }
  }
  // Rejoins are addressed by family index, and one that runs may shift the
  // indices after it: run the highest index first so the rest still point at
  // their own shapes.
  if (choices.split === 'rejoinSolid') ops.sort((a, b) => (b.params?.family as number) - (a.params?.family as number))
  ops.push(...walls)

  for (const item of acting(plan, choices, 'openings')) ops.push(capOp(item.rows[0]))

  // oxlint-disable-next-line lingui/no-unlocalized-strings -- KindKey identifiers, not copy
  const panels = acting(plan, choices, 'flatPanels')
  if (panels.length > 0) {
    const parsed = parseThickness(choices.thickness.text, choices.thickness.side)
    if (!parsed.ok) return { ok: false, error: parsed.error }
    for (const item of panels) ops.push(thickenOp(item.rows[0], parsed.distance, parsed.centered))
  }

  const options: BatchOptions = {
    cascade: (failed, remaining) => {
      if (failed.method !== 'hew.solid.cap') return null
      const family = capFamily.get(failed)
      if (family === undefined) return null
      const walls = remaining.filter((op) => wallFamily.get(op) === family)
      return walls.length === 0 ? null : { ops: walls, explanation: i18n._(SPLIT_PIECE_LEFT) }
    },
  }
  return { ok: true, ops, options }
}

/**
 * One sentence per decision, for the Ready page, then how many items stay
 * as they are; `nothing` alone when no command would run.
 */
export function readySentences(plan: FixPlan, choices: FixChoices): string[] {
  const out: string[] = []
  const split = acting(plan, choices, 'split')
  const openings = acting(plan, choices, 'openings')
  // oxlint-disable-next-line lingui/no-unlocalized-strings -- KindKey identifiers, not copy
  const panels = acting(plan, choices, 'flatPanels')
  if (split.length > 0) {
    out.push(
      choices.split === 'rejoinSolid'
        ? WIZARD.ready.rejoinSolid(split.length)
        : WIZARD.ready.rejoin(split.length, choices.split === 'keepWalls'),
    )
  }
  if (openings.length > 0) out.push(WIZARD.ready.close(openings.length))
  if (panels.length > 0) out.push(WIZARD.ready.thicken(panels.length, choices.thickness.text.trim()))
  if (out.length === 0) return [WIZARD.ready.nothing]
  const left = leftTotal(plan, choices)
  if (left > 0) out.push(WIZARD.ready.leave(left))
  return out
}

/**
 * libraryUpdates — the pure logic behind "Update from Library" and
 * "Update in Library" (v1.5 OW-2): reading the kernel's link status, deciding
 * what the notice bar counts, what the Review list shows and starts checked,
 * which markers a part gets, and what to tell a person about every typed
 * refusal. No I/O, no React, so it runs on the node test environment.
 *
 * The kernel (`Scene.library_status_json`, `update_definitions_from_items`)
 * owns the comparison itself — three content prints per link — so nothing
 * here decides "newer"; it only presents the answer.
 */

import { plural, t } from '@lingui/core/macro'

/** Where one linked definition stands against its Library item. */
export type LinkState = 'in_step' | 'library_newer' | 'edited_here' | 'both_changed' | 'different' | 'item_missing'

const STATES: readonly LinkState[] = ['in_step', 'library_newer', 'edited_here', 'both_changed', 'different', 'item_missing']

/** One row of `library_status_json`'s `links`. */
export interface LibraryLink {
  /** Decimal definition handle. */
  definition: string
  name?: string
  /** The Library item's id. */
  item: string
  defSid?: string
  state: LinkState
  /** Placements of this definition in the model. */
  copies: number
  /** A lock freezes the definition: an update would skip it. */
  locked: boolean
  mine: string
  base?: string
  theirs?: string
}

export interface LibraryStatus {
  links: LibraryLink[]
  /** Prints the kernel computed from item bytes (pre-1.5 items), by item id. */
  itemPrints: Record<string, Record<string, string>>
}

/** Parse `library_status_json`; anything malformed reads as "no links". */
export function parseLibraryStatus(json: string): LibraryStatus {
  const empty: LibraryStatus = { links: [], itemPrints: {} }
  let raw: unknown
  try {
    raw = JSON.parse(json)
  } catch {
    return empty
  }
  if (raw === null || typeof raw !== 'object') return empty
  const obj = raw as { links?: unknown; itemPrints?: unknown }
  const links: LibraryLink[] = []
  if (Array.isArray(obj.links)) {
    for (const l of obj.links) {
      if (l === null || typeof l !== 'object') continue
      const r = l as Record<string, unknown>
      if (typeof r.definition !== 'string' || typeof r.item !== 'string') continue
      if (typeof r.state !== 'string' || !(STATES as readonly string[]).includes(r.state)) continue
      const link: LibraryLink = {
        definition: r.definition,
        item: r.item,
        state: r.state as LinkState,
        copies: typeof r.copies === 'number' ? r.copies : 0,
        locked: r.locked === true,
        mine: typeof r.mine === 'string' ? r.mine : '',
      }
      if (typeof r.name === 'string') link.name = r.name
      if (typeof r.defSid === 'string') link.defSid = r.defSid
      if (typeof r.base === 'string') link.base = r.base
      if (typeof r.theirs === 'string') link.theirs = r.theirs
      links.push(link)
    }
  }
  const itemPrints: Record<string, Record<string, string>> = {}
  if (obj.itemPrints !== null && typeof obj.itemPrints === 'object') {
    for (const [id, prints] of Object.entries(obj.itemPrints as Record<string, unknown>)) {
      if (prints === null || typeof prints !== 'object') continue
      const map: Record<string, string> = {}
      for (const [sid, p] of Object.entries(prints as Record<string, unknown>)) {
        if (typeof p === 'string') map[sid] = p
      }
      itemPrints[id] = map
    }
  }
  return { links, itemPrints }
}

// ---------------------------------------------------------------------------
// The notice bar: what is newer, what was dismissed
// ---------------------------------------------------------------------------

/** A dismissal is remembered per Library item and per print the Library holds
 *  now: the same newer version never nags twice in a session, a still newer
 *  one does. */
export function dismissKey(link: LibraryLink): string {
  return `${link.item}|${link.theirs ?? ''}`
}

/** Links the Library has a newer version of. */
export function newerLinks(links: readonly LibraryLink[]): LibraryLink[] {
  return links.filter((l) => l.state === 'library_newer')
}

/** Links the bar still counts: newer, and not yet dismissed. */
export function pendingNewer(links: readonly LibraryLink[], dismissed: ReadonlySet<string>): LibraryLink[] {
  return newerLinks(links).filter((l) => !dismissed.has(dismissKey(l)))
}

/**
 * How many updates a set of links stands for. A part nested in another item
 * carries its outer item's link, so with "update all" a Table and its Leg can
 * both read newer; they come from one Library item and are one update.
 */
export function updateCount(links: readonly LibraryLink[]): number {
  return new Set(links.map((l) => l.item)).size
}

// ---------------------------------------------------------------------------
// Markers (Components panel row, Outliner row)
// ---------------------------------------------------------------------------

/** States that earn a marker. In step, and an item missing from the Library,
 *  show nothing (a missing item is not announced: the part keeps working).
 *  `different` (Hew cannot tell which side is newer, as for a link made before
 *  1.5) gets a marker, the only way into Review for it, but never the bar. */
export type MarkedState = 'library_newer' | 'edited_here' | 'both_changed' | 'different'

export function markedState(state: LinkState): state is MarkedState {
  return state === 'library_newer' || state === 'edited_here' || state === 'both_changed' || state === 'different'
}

/** The marker's tooltip. */
export function markTooltip(state: MarkedState): string {
  switch (state) {
    case 'library_newer':
      return t`A newer version of this part is in the Library.`
    case 'edited_here':
      return t`You changed this part here. The Library still has the older version.`
    case 'both_changed':
      return t`This part changed here and in the Library.`
    case 'different':
      return t`This part differs from the Library, and Hew can’t tell which one is newer.`
  }
}

/** Short word beside a marker, for screen readers and the Review list. */
export function stateLabel(state: LinkState): string {
  switch (state) {
    case 'library_newer':
      return t`Newer in the Library`
    case 'edited_here':
      return t`Changed here`
    case 'both_changed':
      return t`Changed here and in the Library`
    case 'different':
      return t`Different from the Library`
    case 'in_step':
      return t`Matches the Library`
    case 'item_missing':
      return t`Not in the Library any more`
  }
}

/** The definition handle → link map the panels read their markers from. */
export function linksByDefinition(links: readonly LibraryLink[]): Map<string, LibraryLink> {
  const out = new Map<string, LibraryLink>()
  for (const l of links) out.set(l.definition, l)
  return out
}

// ---------------------------------------------------------------------------
// Update in Library (model to Library)
// ---------------------------------------------------------------------------

/** States in which the model's copy has something the Library does not. */
export function canUpdateLibraryFrom(state: LinkState): boolean {
  return state === 'edited_here' || state === 'both_changed' || state === 'different'
}

/** Whether replacing the item would throw away a change someone made in the
 *  Library, so the person must confirm first. */
export function needsLibraryConfirm(state: LinkState): boolean {
  return state === 'both_changed' || state === 'different'
}

/** Which question the confirm asks: the Library changed too (`both_changed`),
 *  or Hew cannot tell which side is newer (`different`). */
export function libraryConfirmKind(state: LinkState): 'both_changed' | 'different' | null {
  return state === 'both_changed' || state === 'different' ? state : null
}

/**
 * Whether a link is the ROOT of its Library item: the part a person saved or
 * inserted, not a part nested inside it that carries the outer item's id
 * (the Leg of a Table, once the Table is exploded). Replacing the item with
 * a nested part would overwrite the whole item with one piece of it.
 * `rootSid` is the stable id of the item's root definition; a link whose
 * own id is unknown, or an item whose root is unknown, is not offered.
 */
export function isItemRoot(link: LibraryLink, rootSid: string | undefined): boolean {
  return link.defSid !== undefined && rootSid !== undefined && link.defSid === rootSid
}

// ---------------------------------------------------------------------------
// The Review list
// ---------------------------------------------------------------------------

/** One row of the Review list. Parts of one Library item that read alike are
 *  one row, so a Table with its Leg is a single update. */
export interface ReviewRow {
  /** Stable key: the item id and the state. */
  key: string
  item: string
  state: 'library_newer' | 'both_changed' | 'different'
  /** Definition handles this row updates. */
  definitions: string[]
  /** Names of the parts, in the order the kernel listed them. */
  names: string[]
  /** The name the Library shows for the item, when it is known. */
  itemName?: string
  copies: number
  /** Frozen by a lock: shown, but disabled. */
  locked: boolean
}

function isReviewState(s: LinkState): s is ReviewRow['state'] {
  return s === 'library_newer' || s === 'both_changed' || s === 'different'
}

const STATE_ORDER: Record<ReviewRow['state'], number> = { library_newer: 0, both_changed: 1, different: 2 }

/** The rows the Review list shows: newer first, then the ones a careless
 *  update would hurt. Parts only changed here, in step or gone are not listed
 *  (there is nothing to pull). */
export function reviewRows(links: readonly LibraryLink[], itemName?: (item: string) => string | undefined): ReviewRow[] {
  const rows = new Map<string, ReviewRow>()
  for (const l of links) {
    if (!isReviewState(l.state)) continue
    const key = `${l.item}|${l.state}`
    const existing = rows.get(key)
    const name = l.name !== undefined && l.name !== '' ? l.name : ''
    if (existing === undefined) {
      rows.set(key, {
        key,
        item: l.item,
        state: l.state,
        definitions: [l.definition],
        names: name === '' ? [] : [name],
        copies: l.copies,
        locked: l.locked,
        ...(itemName?.(l.item) !== undefined ? { itemName: itemName(l.item) } : {}),
      })
    } else {
      existing.definitions.push(l.definition)
      if (name !== '') existing.names.push(name)
      existing.copies += l.copies
      // One locked part is enough to hold the whole row back.
      existing.locked = existing.locked || l.locked
    }
  }
  return [...rows.values()].sort((a, b) => STATE_ORDER[a.state] - STATE_ORDER[b.state] || rowName(a).localeCompare(rowName(b)))
}

/** The name a row shows: the Library's name for the item, which is the one a
 *  person chose when saving it, else the part's own name. */
export function rowName(row: ReviewRow): string {
  return row.itemName ?? row.names[0] ?? t`Library part`
}

/** Checked at first: what the Library changed, and nothing that would throw
 *  away a change made here. Locked rows start unchecked and cannot be checked. */
export function defaultChecked(row: ReviewRow): boolean {
  return row.state === 'library_newer' && !row.locked
}

/** The line under a row's state, in plain words. */
export function rowNote(row: ReviewRow): string | null {
  switch (row.state) {
    case 'library_newer':
      return null
    case 'both_changed':
      return t`Your changes here would be replaced.`
    case 'different':
      return t`Hew can’t tell which one is newer. Updating would replace what is here.`
  }
}

/** Why a row cannot be updated, or `null` when it can. */
export function rowBlockedReason(row: ReviewRow): string | null {
  return row.locked ? t`Locked. Unlock it first to update it.` : null
}

// ---------------------------------------------------------------------------
// The result of an update
// ---------------------------------------------------------------------------

export interface SkippedPart {
  definition: string
  name?: string
  /** `LockedNode`, `ExplodeSessionScope`, ... */
  code: string
  message?: string
}

export interface UpdateOutcome {
  updated: string[]
  alsoUpdated: string[]
  unchanged: string[]
  skipped: SkippedPart[]
  /** Parts left as they were because they were changed here or locked; the
   *  updated part got a fresh copy of the item's instead. */
  kept: string[]
  definitionsAdded: number
  materialsAdded: number
}

/** Parse `update_definitions_from_items`' answer; null when it is not one. */
export function parseUpdateOutcome(json: string): UpdateOutcome | null {
  let raw: unknown
  try {
    raw = JSON.parse(json)
  } catch {
    return null
  }
  if (raw === null || typeof raw !== 'object') return null
  const r = raw as Record<string, unknown>
  const strings = (v: unknown): string[] => (Array.isArray(v) ? v.filter((x): x is string => typeof x === 'string') : [])
  const skipped: SkippedPart[] = []
  if (Array.isArray(r.skipped)) {
    for (const s of r.skipped) {
      if (s === null || typeof s !== 'object') continue
      const o = s as Record<string, unknown>
      if (typeof o.definition !== 'string' || typeof o.code !== 'string') continue
      const part: SkippedPart = { definition: o.definition, code: o.code }
      if (typeof o.name === 'string') part.name = o.name
      if (typeof o.message === 'string') part.message = o.message
      skipped.push(part)
    }
  }
  return {
    updated: strings(r.updated),
    alsoUpdated: strings(r.alsoUpdated),
    unchanged: strings(r.unchanged),
    kept: strings(r.kept),
    skipped,
    definitionsAdded: typeof r.definitionsAdded === 'number' ? r.definitionsAdded : 0,
    materialsAdded: typeof r.materialsAdded === 'number' ? r.materialsAdded : 0,
  }
}

/** "Oak leg is locked and was not updated." — the plain-English line for one
 *  refusal. Every code the kernel can answer has its own words; an unknown
 *  code falls back to the kernel's own message. */
export function skippedCopy(part: SkippedPart): string {
  const name = part.name !== undefined && part.name !== '' ? part.name : t`This part`
  const reason = part.message ?? ''
  switch (part.code) {
    case 'LockedNode':
      return t`“${name}” is locked and was not updated. Unlock it first.`
    case 'LockedNodeViaSharedShape':
      return t`“${name}” shares its shape with a locked part and was not updated. Unlock that part first.`
    case 'ExplodeSessionScope':
      return t`“${name}” is open for editing and was not updated. Close the open group or component first.`
    case 'ComponentCycle':
      return t`“${name}” was not updated: the new version would contain itself.`
    case 'ComponentDepthExceeded':
      return t`“${name}” was not updated: the new version has parts nested too deeply.`
    case 'ComponentExpansionExceeded':
      return t`“${name}” was not updated: the new version is too large to bring in.`
    case 'UnknownLibraryItem':
      return t`“${name}” was not updated: its Library item is gone, or no longer has this part.`
    case 'LoadFailed':
      return t`“${name}” was not updated: Hew couldn’t read its Library item.`
    case 'UnknownComponent':
      return t`“${name}” was not updated: it is no longer in this model.`
    default:
      return reason !== '' ? t`“${name}” was not updated: ${reason}` : t`“${name}” was not updated.`
  }
}

/** How many Library items an outcome changed (a nested part counts with the
 *  item it came from, not again). */
export function updatedItemCount(outcome: UpdateOutcome, links: readonly LibraryLink[]): number {
  const changed = new Set([...outcome.updated, ...outcome.alsoUpdated])
  const items = new Set<string>()
  for (const l of links) if (changed.has(l.definition)) items.add(l.item)
  // A definition the status no longer lists still counts as one update.
  return items.size > 0 ? items.size : outcome.updated.length
}

/** The one-line toast after an update. */
export function outcomeSummary(outcome: UpdateOutcome, links: readonly LibraryLink[]): string {
  const n = updatedItemCount(outcome, links)
  const skipped = outcome.skipped.length
  if (n === 0 && skipped === 0) return t`Nothing needed updating.`
  const notUpdated = plural(skipped, { one: '# part was not updated.', other: '# parts were not updated.' })
  if (n === 0) return notUpdated
  const done = plural(n, { one: 'Updated # part from the Library.', other: 'Updated # parts from the Library.' })
  return skipped === 0 ? done : `${done} ${notUpdated}`
}

/** "“Leg” was left as it is because it was changed here or is locked; the
 *  updated “Table” got its own copy of the new “Leg”." One sentence per kept
 *  part (the kernel keeps a part for either reason and does not say which). */
export function keptCopy(outcome: UpdateOutcome, links: readonly LibraryLink[]): string[] {
  const byDef = new Map(links.map((l) => [l.definition, l]))
  const changed = new Set([...outcome.updated, ...outcome.alsoUpdated])
  return outcome.kept.map((def) => {
    const link = byDef.get(def)
    const name = link?.name !== undefined && link.name !== '' ? link.name : t`This part`
    const parent = links.find((l) => l.item === link?.item && l.definition !== def && changed.has(l.definition) && (l.name ?? '') !== '')
    const parentName = parent?.name
    return parentName !== undefined
      ? t`“${name}” was left as it is because it was changed here or is locked; the updated “${parentName}” got its own copy of the new “${name}”.`
      : t`“${name}” was left as it is because it was changed here or is locked; the updated part got its own copy of the new “${name}”.`
  })
}

/** The toast after an update: the summary, then what was kept. */
export function outcomeToast(outcome: UpdateOutcome, links: readonly LibraryLink[]): string {
  return [outcomeSummary(outcome, links), ...keptCopy(outcome, links)].join(' ')
}

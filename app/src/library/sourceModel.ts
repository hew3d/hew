/* oxlint-disable lingui/no-unlocalized-strings -- identifiers and state names; the user-visible sentences are all t``/plural`` below */
// The Library dialog's view of sources, as pure functions
// (docs/design/shared-library-sources.md 5.1, 7.5): which words an item's
// state gets, how a source row stands in for a Library item in the existing
// tile and list components, what the bar over the dialog counts, and what a
// local item's detail pane says about where it came from.

import { plural, t } from '@lingui/core/macro'
import { intlLocale } from '../i18n/format'
import { builtInProfileItem } from './libraryModel'
import type { CatalogSourceEntry, IndexCategory, IndexSummary, ItemRow, LocalState, NewCounts, SourceStatus } from './sources'
import type { SourcesSnapshot } from './useSources'
import type { LibraryCategory, LibraryItem } from './types'

/** What the center and detail columns show instead of the local Library. */
export type SourceViewTarget =
  | { kind: 'catalog' }
  | { kind: 'source'; source: string }
  | { kind: 'preview'; entry: CatalogSourceEntry }

const LIB_CATEGORY: Record<IndexCategory, LibraryCategory> = {
  Components: 'component',
  Materials: 'material',
  Models: 'model',
  Profiles: 'profile',
}

/** Prefix of a source row's pseudo file name (never a real file). */
export const SOURCE_ROW_PREFIX = 'source:'

export function isSourceRowItem(item: Pick<LibraryItem, 'file'>): boolean {
  return item.file.fileName.startsWith(SOURCE_ROW_PREFIX)
}

/** The state of one item in a source's view, in words (5.1). Each sentence is
 * whole, so a language can order its words as it needs to. */
export function rowStateLabel(row: ItemRow): string {
  const changed = row.edited && row.state !== 'not_in_library'
  const from = row.from ?? ''
  switch (row.state) {
    case 'not_in_library':
      return t`Not in your Library`
    case 'in_library':
    case 'in_step':
      return changed ? t`In your Library · changed here` : t`In your Library`
    case 'in_library_from_other':
      if (from === '') return changed ? t`In your Library from another source · changed here` : t`In your Library from another source`
      return changed ? t`In your Library from ${from} · changed here` : t`In your Library from ${from}`
    case 'newer':
      return changed ? t`Newer at the source · changed here` : t`Newer at the source`
    case 'gone':
      return changed ? t`The source no longer has this item · changed here` : t`The source no longer has this item`
    case 'orphan':
      return t`From a source that is not in your list`
  }
}

/** The license an item is under: its own, the source's, or the plain words. */
export function rowLicense(row: ItemRow, source: Pick<IndexSummary, 'license'> | null): string {
  return row.entry?.license ?? source?.license ?? t`No license stated`
}

/** A source row dressed as a Library item so the tile, list row and their
 * thumbnails work unchanged. The file name is a pseudo name keyed by the
 * item's id; nothing reads it as a path. */
export function rowLibraryItem(row: ItemRow): LibraryItem {
  const base = builtInProfileItem('x', 'x')
  const category = LIB_CATEGORY[row.category]
  return {
    file: { fileName: `${SOURCE_ROW_PREFIX}${row.id}`, size: row.entry?.bytes ?? 0, mtimeMs: 0 },
    summary: { ...base.summary, profile_chain_only: false },
    meta: {
      id: row.id,
      name: row.name,
      category,
      ...(row.entry?.keywords !== undefined ? { keywords: row.entry.keywords } : {}),
      ...(row.entry?.collection != null ? { collection: row.entry.collection } : {}),
    },
    displayName: row.name,
    category,
    contentHash: row.entry?.sha256 ?? row.id,
  }
}

/** A search over a source's items: name, keywords and collection. */
export function filterRows(rows: ItemRow[], query: string): ItemRow[] {
  const q = query.trim().toLowerCase()
  if (q === '') return rows
  return rows.filter((r) => {
    const hay = [r.name, ...(r.entry?.keywords ?? []), r.entry?.collection ?? ''].join('\n').toLowerCase()
    return hay.includes(q)
  })
}

/** "Checked …" for a source header, in the shape of the detail pane's saved line. */
export function checkedLine(iso: string | null, nowMs: number): string {
  const ts = iso === null ? NaN : Date.parse(iso)
  if (Number.isNaN(ts)) return t`Not checked yet`
  const diffMs = Math.max(0, nowMs - ts)
  const minutes = Math.floor(diffMs / 60_000)
  const hours = Math.floor(diffMs / 3_600_000)
  const days = Math.floor(diffMs / 86_400_000)
  if (minutes < 1) return t`Checked just now`
  if (hours < 1) return plural(minutes, { one: 'Checked # minute ago', other: 'Checked # minutes ago' })
  if (days < 1) return plural(hours, { one: 'Checked # hour ago', other: 'Checked # hours ago' })
  if (days === 1) return t`Checked yesterday`
  if (days <= 30) return plural(days, { one: 'Checked # day ago', other: 'Checked # days ago' })
  const date = new Date(ts).toLocaleDateString(intlLocale(), { year: 'numeric', month: 'short', day: 'numeric' })
  return t`Checked ${date}`
}

// ------------------------------------------------------------------ the bar

export interface FollowedLine {
  source: string
  name: string
  fresh: number
  changed: number
}

export interface BarModel {
  /** Items with a newer version at a source the user does not follow. */
  newer: number
  /** One line per followed source that has something new or changed. */
  followed: FollowedLine[]
}

/** What the bar over the dialog counts (7.5): newer items at sources the user
 * browses, and new and changed items at followed sources. A removed source has
 * no count; an item the user kept is not counted. */
export function barModel(snapshot: SourcesSnapshot): BarModel {
  const bySource = new Map(snapshot.statuses.map((s) => [s.entry.source, s]))
  let newer = 0
  for (const l of snapshot.locals) {
    if (l.state !== 'newer' || l.kept) continue
    const s = bySource.get(l.note.source)
    if (s === undefined || s.removed !== null || s.followed) continue
    newer++
  }
  const followed: FollowedLine[] = []
  for (const s of snapshot.statuses) {
    if (!s.followed || s.removed !== null) continue
    const c: NewCounts | undefined = snapshot.counts[s.entry.source]
    if (c === undefined || c.new + c.changed === 0) continue
    followed.push({ source: s.entry.source, name: s.entry.name, fresh: c.new, changed: c.changed })
  }
  return { newer, followed }
}

export function barIsEmpty(m: BarModel): boolean {
  return m.newer === 0 && m.followed.length === 0
}

export function newerLine(n: number): string {
  return plural(n, {
    one: '# item has a newer version at its source.',
    other: '# items have newer versions at their sources.',
  })
}

export function followedLine(l: FollowedLine): string {
  const name = l.name
  const { fresh, changed } = l
  if (fresh > 0 && changed > 0) return t`${name} has ${fresh} new and ${changed} changed items`
  if (l.fresh > 0) {
    const what = plural(l.fresh, { one: '# new item', other: '# new items' })
    return t`${name} has ${what}`
  }
  const what = plural(l.changed, { one: '# changed item', other: '# changed items' })
  return t`${name} has ${what}`
}

// ------------------------------------------------------- local item details

export interface LocalSourceInfo {
  source: string
  sourceName: string
  /** "From <source> · <license>". */
  fromLine: string
  /** The item's state line, then any warnings. */
  stateLines: string[]
  state: LocalState['state']
  /** The source is added, not removed, and sources are on: an update can be offered. */
  canUpdate: boolean
  edited: boolean
}

/** What a local item's detail pane says about its source. `off` is the Off
 * state: the "From" line shows, nothing else is offered (5.2). */
export function localSourceInfo(local: LocalState, status: SourceStatus | undefined, off: boolean): LocalSourceInfo {
  const name = local.note.name
  const license = local.note.license ?? t`No license stated`
  const lines: string[] = []
  switch (local.state) {
    case 'in_step':
      lines.push(t`It matches the source.`)
      break
    case 'newer':
      lines.push(t`Newer at the source`)
      break
    case 'gone':
      lines.push(t`The source no longer has this item.`)
      break
    case 'orphan':
      lines.push(t`This source is not in your list. Add it again to get updates.`)
      break
    case 'unchecked':
      lines.push(t`This source has not been checked yet.`)
      break
  }
  if (local.edited) lines.push(t`Changed here`)
  if (status?.removed != null) {
    const reason = status.removed.reason
    lines.push(reason ? t`The Hew catalog removed this source: ${reason}` : t`The Hew catalog removed this source.`)
  }
  if (off) lines.push(t`Sources are turned off on this server.`)
  const canUpdate = !off && status !== undefined && status.removed === null && local.state === 'newer'
  return {
    source: local.note.source,
    sourceName: name,
    fromLine: t`From ${name} · ${license}`,
    stateLines: lines,
    state: local.state,
    canUpdate,
    edited: local.edited,
  }
}

/** The host a source lives on, for messages ("Hew could not reach <host>").
 * Read from the canonical reference (6.2); null when it names none. */
export function sourceHost(canonical: string): string | null {
  const colon = canonical.indexOf(':')
  if (colon < 0) return null
  const kind = canonical.slice(0, colon)
  const rest = canonical.slice(colon + 1)
  if (kind === 'github') return 'github.com'
  if (kind === 'web' || rest.startsWith('https://')) {
    try {
      return new URL(rest.startsWith('https://') ? rest : `https://${rest}`).host
    } catch {
      return null
    }
  }
  if (kind === 'gitlab' || kind === 'forgejo') {
    const host = rest.split('/')[0]
    return host === '' ? null : host.toLowerCase()
  }
  return null
}

/* oxlint-disable lingui/no-unlocalized-strings -- test fixtures */
import { describe, expect, it } from 'vitest'
import {
  barIsEmpty,
  barModel,
  checkedLine,
  filterRows,
  followedLine,
  isSourceRowItem,
  localSourceInfo,
  newerLine,
  rowLibraryItem,
  rowLicense,
  rowStateLabel,
  sourceHost,
} from './sourceModel'
import type { ItemRow, LocalState, SourceEntry, SourceStatus } from './sources'
import type { SourcesSnapshot } from './useSources'
import type { LibraryRemoteNote } from './types'

const entry = (source: string, over: Partial<SourceEntry> = {}): SourceEntry => ({
  source,
  kind: 'github',
  name: source.split('/')[1] ?? source,
  followed: false,
  addedAt: '2026-11-23T00:00:00.000Z',
  lastCheck: { at: '2026-11-23T00:00:00.000Z', ok: true, code: null },
  skipped: [],
  kept: {},
  ...over,
})
const status = (source: string, over: Partial<SourceStatus> = {}, e: Partial<SourceEntry> = {}): SourceStatus => ({
  entry: entry(source, e),
  followed: e.followed === true,
  unreachable: false,
  removed: null,
  ...over,
})
const note = (source: string, over: Partial<LibraryRemoteNote> = {}): LibraryRemoteNote => ({
  source,
  name: 'Hinges',
  path: 'a.hew',
  sha256: 'a'.repeat(64),
  fetchedAt: '2026-11-23T00:00:00.000Z',
  edited: false,
  ...over,
})
const local = (source: string, over: Partial<LocalState> = {}): LocalState => ({
  id: 'i1',
  file: 'Components/a-000001.hew',
  name: 'A',
  note: note(source),
  state: 'in_step',
  edited: false,
  kept: false,
  ...over,
})
const row = (over: Partial<ItemRow> = {}): ItemRow => ({
  id: 'i1',
  name: 'Part',
  category: 'Components',
  state: 'not_in_library',
  edited: false,
  kept: false,
  skipped: false,
  file: null,
  entry: { id: 'i1', path: 'p.hew', name: 'Part', category: 'Components', sha256: 'b'.repeat(64), bytes: 2048, keywords: ['brass'], collection: 'Hinges', license: null },
  ...over,
})
const snap = (over: Partial<SourcesSnapshot>): SourcesSnapshot => ({ loaded: true, statuses: [], counts: {}, locals: [], stamp: '', ...over })

describe('rowStateLabel (5.1)', () => {
  it('words every state', () => {
    expect(rowStateLabel(row())).toBe('Not in your Library')
    expect(rowStateLabel(row({ state: 'in_library' }))).toBe('In your Library')
    expect(rowStateLabel(row({ state: 'in_step' }))).toBe('In your Library')
    expect(rowStateLabel(row({ state: 'in_library_from_other', from: 'Slides' }))).toBe('In your Library from Slides')
    expect(rowStateLabel(row({ state: 'newer' }))).toBe('Newer at the source')
    expect(rowStateLabel(row({ state: 'gone' }))).toBe('The source no longer has this item')
    expect(rowStateLabel(row({ state: 'orphan' }))).toBe('From a source that is not in your list')
  })

  it('"changed here" rides on any state that has a Library copy', () => {
    expect(rowStateLabel(row({ state: 'in_step', edited: true }))).toBe('In your Library · changed here')
    expect(rowStateLabel(row({ state: 'newer', edited: true }))).toBe('Newer at the source · changed here')
    expect(rowStateLabel(row({ state: 'not_in_library', edited: true }))).toBe('Not in your Library')
  })
})

describe('rows as Library items', () => {
  it('dresses a row so the tile and row components can draw it', () => {
    const item = rowLibraryItem(row())
    expect(isSourceRowItem(item)).toBe(true)
    expect(item.displayName).toBe('Part')
    expect(item.category).toBe('component')
    expect(item.file.size).toBe(2048)
    expect(item.meta.keywords).toEqual(['brass'])
    expect(item.contentHash).toBe('b'.repeat(64))
    expect(rowLibraryItem(row({ category: 'Materials' })).category).toBe('material')
  })

  it('a gone row has no entry and still draws', () => {
    const item = rowLibraryItem(row({ state: 'gone', entry: null }))
    expect(item.file.size).toBe(0)
    expect(item.contentHash).toBe('i1')
  })

  it('the license is the item\'s own, the source\'s, or the plain words', () => {
    const own = row({ entry: { ...row().entry!, license: 'CC0-1.0' } })
    expect(rowLicense(own, { license: 'MIT' })).toBe('CC0-1.0')
    expect(rowLicense(row(), { license: 'MIT' })).toBe('MIT')
    expect(rowLicense(row(), { license: null })).toBe('No license stated')
    expect(rowLicense(row(), null)).toBe('No license stated')
  })

  it('search matches name, keywords and collection, ignoring case', () => {
    const rows = [row({ name: 'Butt hinge' }), row({ id: 'i2', name: 'Slide', entry: { ...row().entry!, id: 'i2', keywords: [], collection: 'Drawers' } })]
    expect(filterRows(rows, '').length).toBe(2)
    expect(filterRows(rows, 'HINGE').map((r) => r.name)).toEqual(['Butt hinge'])
    expect(filterRows(rows, 'brass').map((r) => r.name)).toEqual(['Butt hinge'])
    expect(filterRows(rows, 'drawers').map((r) => r.name)).toEqual(['Slide'])
    expect(filterRows(rows, 'nothing')).toEqual([])
  })
})

describe('checkedLine', () => {
  const now = Date.parse('2026-11-23T12:00:00.000Z')
  it('reads like the saved line', () => {
    expect(checkedLine(null, now)).toBe('Not checked yet')
    expect(checkedLine('2026-11-23T11:59:50.000Z', now)).toBe('Checked just now')
    expect(checkedLine('2026-11-23T11:30:00.000Z', now)).toBe('Checked 30 minutes ago')
    expect(checkedLine('2026-11-23T09:00:00.000Z', now)).toBe('Checked 3 hours ago')
    expect(checkedLine('2026-11-22T09:00:00.000Z', now)).toBe('Checked yesterday')
    expect(checkedLine('2026-11-18T12:00:00.000Z', now)).toBe('Checked 5 days ago')
    expect(checkedLine('2026-01-02T12:00:00.000Z', now)).toMatch(/^Checked /)
  })
})

describe('the bar (7.5)', () => {
  const a = 'github:acme/hinges'
  const b = 'github:acme/slides'

  it('counts newer items at sources the user browses, not kept ones, not removed sources', () => {
    const m = barModel(
      snap({
        statuses: [status(a), status(b, { removed: { reason: 'x', severity: 'removed' } })],
        locals: [
          local(a, { id: '1', state: 'newer' }),
          local(a, { id: '2', state: 'newer', kept: true }),
          local(a, { id: '3', state: 'in_step' }),
          local(b, { id: '4', state: 'newer' }),
        ],
      }),
    )
    expect(m).toEqual({ newer: 1, followed: [] })
  })

  it('gives a followed source its own line and leaves its items out of the first count', () => {
    const m = barModel(
      snap({
        statuses: [status(a, {}, { followed: true, name: 'Hinges' })],
        counts: { [a]: { new: 2, changed: 1 } },
        locals: [local(a, { id: '1', state: 'newer' })],
      }),
    )
    expect(m.newer).toBe(0)
    expect(m.followed).toEqual([{ source: a, name: 'Hinges', fresh: 2, changed: 1 }])
    expect(barIsEmpty(m)).toBe(false)
  })

  it('is empty with nothing to say', () => {
    expect(barIsEmpty(barModel(snap({ statuses: [status(a, {}, { followed: true })], counts: { [a]: { new: 0, changed: 0 } } })))).toBe(true)
  })

  it('words the lines', () => {
    expect(newerLine(1)).toBe('1 item has a newer version at its source.')
    expect(newerLine(4)).toBe('4 items have newer versions at their sources.')
    expect(followedLine({ source: a, name: 'Hinges', fresh: 2, changed: 1 })).toBe('Hinges has 2 new and 1 changed items')
    expect(followedLine({ source: a, name: 'Hinges', fresh: 3, changed: 0 })).toBe('Hinges has 3 new items')
    expect(followedLine({ source: a, name: 'Hinges', fresh: 0, changed: 1 })).toBe('Hinges has 1 changed item')
  })
})

describe('localSourceInfo', () => {
  const a = 'github:acme/hinges'
  it('says where an item came from and where it stands', () => {
    const info = localSourceInfo(local(a, { note: note(a, { license: 'MIT' }) }), status(a), false)
    expect(info.fromLine).toBe('From Hinges · MIT')
    expect(info.stateLines).toEqual(['It matches the source.'])
    expect(info.canUpdate).toBe(false)
  })

  it('a newer item can be updated; "changed here" is added; no license reads plainly', () => {
    const info = localSourceInfo(local(a, { state: 'newer', edited: true }), status(a), false)
    expect(info.fromLine).toBe('From Hinges · No license stated')
    expect(info.stateLines).toEqual(['Newer at the source', 'Changed here'])
    expect(info.canUpdate).toBe(true)
  })

  it('gone, orphan and unchecked have their own lines and no update', () => {
    expect(localSourceInfo(local(a, { state: 'gone' }), status(a), false).stateLines[0]).toBe('The source no longer has this item.')
    const orphan = localSourceInfo(local(a, { state: 'orphan' }), undefined, false)
    expect(orphan.stateLines[0]).toBe('This source is not in your list. Add it again to get updates.')
    expect(orphan.canUpdate).toBe(false)
    expect(localSourceInfo(local(a, { state: 'unchecked' }), status(a), false).stateLines[0]).toBe('This source has not been checked yet.')
  })

  it('a removed source gives the reason and no update; Off gives its line and no update', () => {
    const removed = localSourceInfo(local(a, { state: 'newer' }), status(a, { removed: { reason: 'Copied.', severity: 'removed' } }), false)
    expect(removed.stateLines).toContain('The Hew catalog removed this source: Copied.')
    expect(removed.canUpdate).toBe(false)
    const off = localSourceInfo(local(a, { state: 'newer' }), status(a), true)
    expect(off.stateLines).toContain('Sources are turned off on this server.')
    expect(off.canUpdate).toBe(false)
    expect(off.fromLine).toBe('From Hinges · No license stated')
  })
})

describe('sourceHost', () => {
  it('names the host of each kind of reference', () => {
    expect(sourceHost('github:acme/hinges')).toBe('github.com')
    expect(sourceHost('gitlab:gitlab.com/group/proj')).toBe('gitlab.com')
    expect(sourceHost('forgejo:codeberg.org/o/r@v1')).toBe('codeberg.org')
    expect(sourceHost('forgejo:https://git.example.org/o/r')).toBe('git.example.org')
    expect(sourceHost('web:https://Shop.Example.org/parts/hew.json')).toBe('shop.example.org')
    expect(sourceHost('nonsense')).toBeNull()
  })
})

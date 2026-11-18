/**
 * libraryUpdates — the pure logic behind the notice bar, the Review list, the
 * markers and the refusal copy (v1.5 OW-2).
 */
import { describe, expect, it } from 'vitest'
import {
  canUpdateLibraryFrom,
  defaultChecked,
  dismissKey,
  linksByDefinition,
  markedState,
  markTooltip,
  needsLibraryConfirm,
  libraryConfirmKind,
  isItemRoot,
  keptCopy,
  outcomeToast,
  newerLinks,
  outcomeSummary,
  parseLibraryStatus,
  parseUpdateOutcome,
  pendingNewer,
  reviewRows,
  rowBlockedReason,
  rowNote,
  skippedCopy,
  stateLabel,
  updateCount,
  updatedItemCount,
  type LibraryLink,
  type LinkState,
} from './libraryUpdates'

function link(over: Partial<LibraryLink> & { definition: string }): LibraryLink {
  return { item: 'item-a', state: 'in_step', copies: 1, locked: false, mine: 'p1:aa', theirs: 'p1:bb', ...over }
}

describe('parseLibraryStatus', () => {
  it('reads links and cached item prints', () => {
    const json = JSON.stringify({
      links: [{ definition: '1', item: 'a', state: 'library_newer', copies: 2, locked: false, mine: 'p1:1', name: 'Leg', theirs: 'p1:2' }],
      itemPrints: { a: { '7': 'p1:2' } },
    })
    const s = parseLibraryStatus(json)
    expect(s.links).toEqual([
      { definition: '1', item: 'a', state: 'library_newer', copies: 2, locked: false, mine: 'p1:1', name: 'Leg', theirs: 'p1:2' },
    ])
    expect(s.itemPrints).toEqual({ a: { '7': 'p1:2' } })
  })

  it('drops rows with an unknown state or no item, and survives garbage', () => {
    const json = JSON.stringify({ links: [{ definition: '1', item: 'a', state: 'sideways' }, { definition: '2' }, 7] })
    expect(parseLibraryStatus(json).links).toEqual([])
    expect(parseLibraryStatus('not json')).toEqual({ links: [], itemPrints: {} })
    expect(parseLibraryStatus('null')).toEqual({ links: [], itemPrints: {} })
  })
})

describe('the notice bar', () => {
  const links = [
    link({ definition: '1', item: 'a', state: 'library_newer', theirs: 'p1:t1' }),
    link({ definition: '2', item: 'b', state: 'library_newer', theirs: 'p1:t2' }),
    link({ definition: '3', item: 'c', state: 'edited_here' }),
    link({ definition: '4', item: 'd', state: 'both_changed' }),
    link({ definition: '5', item: 'e', state: 'item_missing' }),
  ]

  it('counts only what the Library has newer', () => {
    expect(newerLinks(links).map((l) => l.definition)).toEqual(['1', '2'])
    expect(updateCount(pendingNewer(links, new Set()))).toBe(2)
  })

  it('a table and its leg from one item are one update', () => {
    const nested = [
      link({ definition: '1', item: 'table', state: 'library_newer', name: 'Table' }),
      link({ definition: '2', item: 'table', state: 'library_newer', name: 'Leg' }),
    ]
    expect(updateCount(pendingNewer(nested, new Set()))).toBe(1)
  })

  it('stays quiet after Later for the same print, speaks again for a newer one', () => {
    const dismissed = new Set(newerLinks(links).map(dismissKey))
    expect(pendingNewer(links, dismissed)).toEqual([])
    const newer = links.map((l) => (l.definition === '1' ? { ...l, theirs: 'p1:t3' } : l))
    expect(pendingNewer(newer, dismissed).map((l) => l.definition)).toEqual(['1'])
  })
})

describe('markers', () => {
  it('mark newer, changed here, changed on both sides and different; nothing else', () => {
    const states: LinkState[] = ['in_step', 'library_newer', 'edited_here', 'both_changed', 'different', 'item_missing']
    expect(states.filter(markedState)).toEqual(['library_newer', 'edited_here', 'both_changed', 'different'])
  })

  it('different never raises the bar: only newer counts', () => {
    const l = (state: LinkState): LibraryLink => ({ definition: '1', item: 'a', state, copies: 1, locked: false, mine: 'p1:a' })
    expect(pendingNewer([l('different')], new Set())).toEqual([])
  })

  it('every marker has its own words, and every state a label', () => {
    expect(new Set(['library_newer', 'edited_here', 'both_changed', 'different'].map((s) => markTooltip(s as 'library_newer'))).size).toBe(4)
    expect(markTooltip('different')).toBe('This part differs from the Library, and Hew can’t tell which one is newer.')
    expect(stateLabel('different')).toBe('Different from the Library')
    expect(stateLabel('library_newer')).toBe('Newer in the Library')
  })

  it('indexes by definition handle', () => {
    const m = linksByDefinition([link({ definition: '9' })])
    expect(m.get('9')?.item).toBe('item-a')
  })
})

describe('which part may stand for its Library item', () => {
  it('offers only the item root, never a part nested in it', () => {
    expect(isItemRoot(link({ definition: '1', defSid: '7' }), '7')).toBe(true)
    // The Leg of an exploded Table carries the Table's item id with its own definition id.
    expect(isItemRoot(link({ definition: '2', defSid: '8' }), '7')).toBe(false)
    expect(isItemRoot(link({ definition: '2' }), '7')).toBe(false)
    expect(isItemRoot(link({ definition: '2', defSid: '8' }), undefined)).toBe(false)
  })
})

describe('what was kept', () => {
  const links = [
    link({ definition: '1', item: 'table', name: 'Table' }),
    link({ definition: '2', item: 'table', name: 'Leg' }),
  ]
  const outcome = { updated: ['1'], alsoUpdated: [], unchanged: [], kept: ['2'], skipped: [], definitionsAdded: 1, materialsAdded: 0 }

  it('says which part was kept and that the updated one got its own copy', () => {
    expect(keptCopy(outcome, links)).toEqual(['“Leg” was left as it is because it was changed here or is locked; the updated “Table” got its own copy of the new “Leg”.'])
    expect(outcomeToast(outcome, links)).toBe(
      'Updated 1 part from the Library. “Leg” was left as it is because it was changed here or is locked; the updated “Table” got its own copy of the new “Leg”.',
    )
  })

  it('says nothing when nothing was kept', () => {
    expect(keptCopy({ ...outcome, kept: [] }, links)).toEqual([])
  })
})

describe('Update in Library', () => {
  it('is offered when the model has something the Library does not', () => {
    expect(canUpdateLibraryFrom('edited_here')).toBe(true)
    expect(canUpdateLibraryFrom('both_changed')).toBe(true)
    expect(canUpdateLibraryFrom('different')).toBe(true)
    expect(canUpdateLibraryFrom('in_step')).toBe(false)
    expect(canUpdateLibraryFrom('library_newer')).toBe(false)
    expect(canUpdateLibraryFrom('item_missing')).toBe(false)
  })

  it('asks first only when the Library changed too', () => {
    expect(needsLibraryConfirm('both_changed')).toBe(true)
    expect(needsLibraryConfirm('edited_here')).toBe(false)
    // Hew cannot tell which side is newer: replacing may lose the Library's version.
    expect(needsLibraryConfirm('different')).toBe(true)
    expect(libraryConfirmKind('different')).toBe('different')
    expect(libraryConfirmKind('both_changed')).toBe('both_changed')
    expect(libraryConfirmKind('edited_here')).toBeNull()
  })
})

describe('the Review list', () => {
  it('lists only what could be pulled, newer first', () => {
    const rows = reviewRows([
      link({ definition: '1', item: 'a', state: 'different', name: 'Zed' }),
      link({ definition: '2', item: 'b', state: 'edited_here', name: 'Mine' }),
      link({ definition: '3', item: 'c', state: 'library_newer', name: 'Leg' }),
      link({ definition: '4', item: 'd', state: 'both_changed', name: 'Top' }),
      link({ definition: '5', item: 'e', state: 'in_step' }),
    ])
    expect(rows.map((r) => r.state)).toEqual(['library_newer', 'both_changed', 'different'])
  })

  it('starts the Library-changed rows checked and the risky ones not', () => {
    const [newer, both, different] = reviewRows([
      link({ definition: '1', item: 'a', state: 'library_newer' }),
      link({ definition: '2', item: 'b', state: 'both_changed' }),
      link({ definition: '3', item: 'c', state: 'different' }),
    ])
    expect(defaultChecked(newer!)).toBe(true)
    expect(defaultChecked(both!)).toBe(false)
    expect(defaultChecked(different!)).toBe(false)
    expect(rowNote(newer!)).toBeNull()
    expect(rowNote(both!)).toBe('Your changes here would be replaced.')
    expect(rowNote(different!)).toMatch(/can’t tell which one is newer/)
  })

  it('merges parts of one item that read alike into one row, summing copies', () => {
    const rows = reviewRows([
      link({ definition: '1', item: 'table', state: 'library_newer', name: 'Table', copies: 1 }),
      link({ definition: '2', item: 'table', state: 'library_newer', name: 'Leg', copies: 4 }),
    ])
    expect(rows).toHaveLength(1)
    expect(rows[0]).toMatchObject({ definitions: ['1', '2'], names: ['Table', 'Leg'], copies: 5 })
  })

  it('shows a locked row disabled with the reason, and never checks it', () => {
    const [row] = reviewRows([link({ definition: '1', item: 'a', state: 'library_newer', locked: true })])
    expect(defaultChecked(row!)).toBe(false)
    expect(rowBlockedReason(row!)).toBe('Locked. Unlock it first to update it.')
  })
})

describe('what an update says', () => {
  it('has plain words for every code the kernel can refuse with', () => {
    const codes = [
      'LockedNode',
      'LockedNodeViaSharedShape',
      'ExplodeSessionScope',
      'ComponentCycle',
      'ComponentDepthExceeded',
      'ComponentExpansionExceeded',
      'UnknownLibraryItem',
      'LoadFailed',
      'UnknownComponent',
    ]
    const lines = codes.map((code) => skippedCopy({ definition: '1', name: 'Oak leg', code }))
    expect(new Set(lines).size).toBe(codes.length)
    for (const l of lines) expect(l).toContain('Oak leg')
    expect(lines[0]).toBe('“Oak leg” is locked and was not updated. Unlock it first.')
    expect(lines[2]).toMatch(/Close the open group or component first/)
  })

  it('falls back to the kernel message for a code it does not know, and names an unnamed part', () => {
    expect(skippedCopy({ definition: '1', code: 'Brand New', message: 'it broke' })).toBe('“This part” was not updated: it broke')
    expect(skippedCopy({ definition: '1', code: 'Brand New' })).toBe('“This part” was not updated.')
  })

  it('parses the kernel answer', () => {
    const out = parseUpdateOutcome(
      JSON.stringify({
        updated: ['1'],
        alsoUpdated: ['2'],
        unchanged: [],
        kept: ['4'],
        skipped: [{ definition: '3', name: 'Top', code: 'LockedNode', message: 'x', lockedBy: { kind: 'instance', id: '9' } }],
        definitionsAdded: 0,
        materialsAdded: 1,
      }),
    )
    expect(out).toMatchObject({ updated: ['1'], alsoUpdated: ['2'], kept: ['4'], materialsAdded: 1 })
    expect(out!.skipped).toEqual([{ definition: '3', name: 'Top', code: 'LockedNode', message: 'x' }])
    expect(parseUpdateOutcome('nope')).toBeNull()
  })

  it('counts a nested part with its item and writes one line', () => {
    const links = [
      link({ definition: '1', item: 'table' }),
      link({ definition: '2', item: 'table' }),
      link({ definition: '3', item: 'shelf' }),
    ]
    const outcome = { updated: ['1', '2'], alsoUpdated: [], unchanged: [], kept: [], skipped: [], definitionsAdded: 0, materialsAdded: 0 }
    expect(updatedItemCount(outcome, links)).toBe(1)
    expect(outcomeSummary(outcome, links)).toBe('Updated 1 part from the Library.')
    const two = { ...outcome, updated: ['1', '3'] }
    expect(outcomeSummary(two, links)).toBe('Updated 2 parts from the Library.')
    const withSkips = { ...outcome, skipped: [{ definition: '3', code: 'LockedNode' }] }
    expect(outcomeSummary(withSkips, links)).toBe('Updated 1 part from the Library. 1 part was not updated.')
    expect(outcomeSummary({ ...outcome, updated: [], skipped: [{ definition: '3', code: 'LockedNode' }] }, links)).toBe('1 part was not updated.')
    expect(outcomeSummary({ ...outcome, updated: [] }, links)).toBe('Nothing needed updating.')
  })
})

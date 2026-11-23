/* oxlint-disable lingui/no-unlocalized-strings -- test fixtures */
// Review round: the profile Replace metadata (F15).
import { describe, expect, it } from 'vitest'
import { metaForProfileReplace } from './libraryModel'

const fresh = { id: 'p1', name: 'Arch', category: 'profile', savedAt: '2026-11-23T00:00:00.000Z', sourceDoc: 'Doc' }
const note = { source: 'github:a/b', name: 'B', path: 'Profiles/arch.hew', sha256: 'a'.repeat(64), fetchedAt: '2026-11-01T00:00:00.000Z', edited: false }

describe('F15: a profile Replace writes the item as it is now, not as it was when opened', () => {
  it('carries no collection or keywords from anywhere', () => {
    const out = metaForProfileReplace(fresh, { collection: 'B', keywords: ['new'] })
    expect(out).toEqual(fresh)
  })

  it("keeps the current remote note and marks it edited; nothing else of the item's meta", () => {
    const out = metaForProfileReplace(fresh, { collection: 'B', keywords: ['new'], remote: note })
    expect(out).toEqual({ ...fresh, remote: { ...note, edited: true } })
  })

  it('an item the Library no longer has (nothing current) writes no note', () => {
    expect(metaForProfileReplace(fresh, undefined)).toEqual(fresh)
  })
})

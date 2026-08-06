import { describe, expect, it } from 'vitest'
import {
  buildLibraryItem,
  categoryTypeLabel,
  collectionMatchesSubtree,
  collectionsOf,
  collectionSegments,
  collectionTreeFromPaths,
  erroredItem,
  filterItems,
  formatBytes,
  materialSubline,
  metadataLine,
  normalizeCollectionPath,
  savedLine,
  sortListItems,
} from './libraryModel'
import type { LibraryFileEntry, LibraryItem, LibraryItemSummary, LibraryMaterialSummary } from './types'

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

function file(fileName: string, overrides: Partial<LibraryFileEntry> = {}): LibraryFileEntry {
  return { fileName, size: 4096, mtimeMs: 1_700_000_000_000, ...overrides }
}

function summary(overrides: Partial<LibraryItemSummary> = {}): LibraryItemSummary {
  return {
    format_version: 1,
    objects: 0,
    materials: 0,
    components: 0,
    instances: 0,
    groups: 0,
    world_sketches: 0,
    annotations: 0,
    guides: 0,
    first_component_name: null,
    first_component_sid: null,
    first_root_name: null,
    doc_attrs: {},
    material_entries: [],
    ...overrides,
  }
}

function withLibraryMeta(meta: Record<string, unknown>): LibraryItemSummary['doc_attrs'] {
  return { 'hew.library': meta }
}

// ---------------------------------------------------------------------------
// buildLibraryItem — displayName fallback chain
// ---------------------------------------------------------------------------

describe('buildLibraryItem — displayName', () => {
  it('prefers meta.name over everything else', () => {
    const s = summary({
      first_component_name: 'Def Name',
      first_root_name: 'Root Name',
      doc_attrs: withLibraryMeta({ name: 'Meta Name' }),
    })
    const item = buildLibraryItem(file('chair.hew'), s, 'hash1')
    expect(item.displayName).toBe('Meta Name')
  })

  it('falls back to first_component_name when meta has no name', () => {
    const s = summary({ first_component_name: 'Def Name', first_root_name: 'Root Name' })
    const item = buildLibraryItem(file('chair.hew'), s, 'hash1')
    expect(item.displayName).toBe('Def Name')
  })

  it('falls back to first_root_name when there is no component name either', () => {
    const s = summary({ first_root_name: 'Root Name' })
    const item = buildLibraryItem(file('chair.hew'), s, 'hash1')
    expect(item.displayName).toBe('Root Name')
  })

  it('falls back to the file stem (extension stripped) as a last resort', () => {
    const s = summary()
    const item = buildLibraryItem(file('theater-chair.hew'), s, 'hash1')
    expect(item.displayName).toBe('theater-chair')
  })

  it('keeps a name with no .hew extension as-is', () => {
    const s = summary()
    const item = buildLibraryItem(file('weird-name'), s, 'hash1')
    expect(item.displayName).toBe('weird-name')
  })

  it('strips the category subfolder from the file-stem fallback', () => {
    const s = summary()
    const item = buildLibraryItem(file('Components/theater-chair-3f2a.hew'), s, 'hash1')
    expect(item.displayName).toBe('theater-chair-3f2a')
  })
})

// ---------------------------------------------------------------------------
// buildLibraryItem — meta parsing (type guards over untrusted doc_attrs)
// ---------------------------------------------------------------------------

describe('buildLibraryItem — meta parsing', () => {
  it('parses a well-formed hew.library dictionary', () => {
    const s = summary({
      doc_attrs: withLibraryMeta({
        id: 'uuid-1',
        name: 'Theater Chair',
        category: 'component',
        keywords: ['chair', 'seating'],
        collection: 'Furniture',
        savedAt: '2026-08-02T00:00:00.000Z',
        sourceDoc: 'theater-test-4.hew',
      }),
    })
    const item = buildLibraryItem(file('chair.hew'), s, 'hash1')
    expect(item.meta).toEqual({
      id: 'uuid-1',
      name: 'Theater Chair',
      category: 'component',
      keywords: ['chair', 'seating'],
      collection: 'Furniture',
      savedAt: '2026-08-02T00:00:00.000Z',
      sourceDoc: 'theater-test-4.hew',
    })
  })

  it('never throws on a missing hew.library namespace', () => {
    const s = summary({ doc_attrs: {} })
    expect(() => buildLibraryItem(file('x.hew'), s, 'hash1')).not.toThrow()
    expect(buildLibraryItem(file('x.hew'), s, 'hash1').meta).toEqual({})
  })

  it('never throws on a non-object hew.library value', () => {
    const s = summary({ doc_attrs: { 'hew.library': 'not an object' } as unknown as LibraryItemSummary['doc_attrs'] })
    expect(() => buildLibraryItem(file('x.hew'), s, 'hash1')).not.toThrow()
    expect(buildLibraryItem(file('x.hew'), s, 'hash1').meta).toEqual({})
  })

  it('never throws on an array masquerading as hew.library', () => {
    const s = summary({ doc_attrs: { 'hew.library': ['nope'] } as unknown as LibraryItemSummary['doc_attrs'] })
    expect(buildLibraryItem(file('x.hew'), s, 'hash1').meta).toEqual({})
  })

  it('drops individually mistyped fields rather than rejecting the whole object', () => {
    const s = summary({
      doc_attrs: withLibraryMeta({
        id: 42, // wrong type — dropped
        name: 'Good Name', // right type — kept
        category: 'not-a-real-category', // invalid enum — dropped
        keywords: ['a', 2, 'b'], // mixed array — dropped whole (not a string[])
        collection: null, // wrong type — dropped
      }),
    })
    const item = buildLibraryItem(file('x.hew'), s, 'hash1')
    expect(item.meta).toEqual({ name: 'Good Name' })
  })
})

// ---------------------------------------------------------------------------
// buildLibraryItem — category derivation
// ---------------------------------------------------------------------------

describe('buildLibraryItem — category', () => {
  it('meta.category always wins over derivation', () => {
    // Shape looks like a plain model (many objects, many groups), but meta
    // insists it's a material.
    const s = summary({
      objects: 5,
      groups: 2,
      doc_attrs: withLibraryMeta({ category: 'material' }),
    })
    expect(buildLibraryItem(file('x.hew'), s, 'h').category).toBe('material')
  })

  it('derives "material" for a material-only file (no solids, no defs, at least one palette entry)', () => {
    const s = summary({ objects: 0, components: 0, materials: 1 })
    expect(buildLibraryItem(file('x.hew'), s, 'h').category).toBe('material')
  })

  it('does not derive "material" when materials is zero even with no objects/components', () => {
    const s = summary({ objects: 0, components: 0, materials: 0 })
    expect(buildLibraryItem(file('x.hew'), s, 'h').category).toBe('model')
  })

  it('derives "component" for a single-definition file with no top-level groups', () => {
    const s = summary({ objects: 1, components: 1, groups: 0, instances: 1 })
    expect(buildLibraryItem(file('x.hew'), s, 'h').category).toBe('component')
  })

  it('derives "model" when there is more than one component definition', () => {
    const s = summary({ components: 2, groups: 0 })
    expect(buildLibraryItem(file('x.hew'), s, 'h').category).toBe('model')
  })

  it('derives "model" when a single definition coexists with a top-level group', () => {
    const s = summary({ components: 1, groups: 1 })
    expect(buildLibraryItem(file('x.hew'), s, 'h').category).toBe('model')
  })

  it('derives "model" for a plain multi-object scene with no definitions', () => {
    const s = summary({ objects: 4, components: 0, groups: 0, materials: 2 })
    expect(buildLibraryItem(file('x.hew'), s, 'h').category).toBe('model')
  })
})

// ---------------------------------------------------------------------------
// erroredItem
// ---------------------------------------------------------------------------

describe('erroredItem', () => {
  it('carries the error string and a file-stem display name', () => {
    const item = erroredItem(file('corrupt-thing.hew'), '', 'MalformedManifest: unexpected EOF')
    expect(item.error).toBe('MalformedManifest: unexpected EOF')
    expect(item.displayName).toBe('corrupt-thing')
  })

  it('has an empty, valid summary shape (no counts leak through)', () => {
    const item = erroredItem(file('x.hew'), '', 'boom')
    expect(item.summary.objects).toBe(0)
    expect(item.summary.material_entries).toEqual([])
    expect(item.meta).toEqual({})
  })
})

// ---------------------------------------------------------------------------
// filterItems
// ---------------------------------------------------------------------------

const NOW = 1_700_000_000_000 // fixed reference instant

function componentItem(overrides: Partial<LibraryItem> = {}): LibraryItem {
  const s = summary({ objects: 1, components: 1, groups: 0, instances: 1 })
  const base = buildLibraryItem(file('chair.hew'), s, 'hash-chair')
  return { ...base, ...overrides, meta: { ...base.meta, ...overrides.meta } }
}

describe('filterItems', () => {
  const chair = componentItem({
    displayName: 'Theater Chair',
    meta: { id: 'src-chair', keywords: ['seating', 'furniture'] },
  })
  const hinge = componentItem({
    file: file('hinge.hew'),
    displayName: 'Door Hinge',
    meta: { id: 'src-hinge', keywords: ['hardware'] },
  })
  const table = { ...componentItem({ file: file('table.hew'), displayName: 'Café Table' }), category: 'model' as const }
  const items = [chair, hinge, table]

  it('always applies the category filter, even with an empty query', () => {
    const result = filterItems(items, {
      query: '',
      category: 'component',
      scope: 'all',
      collection: null,
      placements: {},
      nowMs: NOW,
    })
    expect(result.map((i) => i.displayName).sort()).toEqual(['Door Hinge', 'Theater Chair'])
  })

  it('ranks a name-prefix match above a name-substring match above a keyword match', () => {
    const result = filterItems(items, {
      query: 'chair',
      category: 'component',
      scope: 'all',
      collection: null,
      placements: {},
      nowMs: NOW,
    })
    // "Theater Chair" matches by substring ("chair" is not a prefix of
    // "Theater Chair"); it's the only match here.
    expect(result.map((i) => i.displayName)).toEqual(['Theater Chair'])
  })

  it('ranks name prefix over keyword hit', () => {
    const items2 = [
      componentItem({ displayName: 'Bolt', meta: { keywords: ['boltcutter'] } }),
      componentItem({ file: file('other.hew'), displayName: 'M3 Bolt', meta: { keywords: [] } }),
    ]
    const result = filterItems(items2, {
      query: 'bolt',
      category: 'component',
      scope: 'all',
      collection: null,
      placements: {},
      nowMs: NOW,
    })
    expect(result[0].displayName).toBe('Bolt') // prefix match ranks first
  })

  it('matches by keyword when the name does not match at all', () => {
    const result = filterItems(items, {
      query: 'hardware',
      category: 'component',
      scope: 'all',
      collection: null,
      placements: {},
      nowMs: NOW,
    })
    expect(result.map((i) => i.displayName)).toEqual(['Door Hinge'])
  })

  it('is case-insensitive', () => {
    const result = filterItems(items, {
      query: 'THEATER',
      category: 'component',
      scope: 'all',
      collection: null,
      placements: {},
      nowMs: NOW,
    })
    expect(result.map((i) => i.displayName)).toEqual(['Theater Chair'])
  })

  it('scope "in-model" keeps only items with a positive placement count', () => {
    const result = filterItems(items, {
      query: '',
      category: 'component',
      scope: 'in-model',
      collection: null,
      placements: { 'src-chair': 3, 'src-hinge': 0 },
      nowMs: NOW,
    })
    expect(result.map((i) => i.displayName)).toEqual(['Theater Chair'])
  })

  it('scope "in-model" excludes items with no meta.id at all', () => {
    const noId = componentItem({ file: file('no-id.hew'), displayName: 'No Id', meta: {} })
    const result = filterItems([noId], {
      query: '',
      category: 'component',
      scope: 'in-model',
      collection: null,
      placements: {},
      nowMs: NOW,
    })
    expect(result).toEqual([])
  })

  // --- Playtest round-4 finding #2: materials match Components' "in this
  // model" treatment — matched by palette membership, not a placement
  // count (materials aren't "placed" as instances). ------------------------
  describe('scope "in-model" and materials', () => {
    function materialItem(overrides: Partial<LibraryItem> = {}): LibraryItem {
      const s = summary({ materials: 1, material_entries: [{ name: 'Oak', color: [180, 140, 90, 255], texture_asset: null, texture_format: null, texture_world_size: null, content_hash: '1' }] })
      const base = buildLibraryItem(file('oak.hew'), s, 'hash-oak')
      return { ...base, ...overrides, meta: { ...base.meta, ...overrides.meta } }
    }

    it('includes a material whose file name is in `materialInPalette`, even with no meta.id and no placement', () => {
      const oak = materialItem({ displayName: 'Oak', meta: {} })
      const result = filterItems([oak], {
        query: '',
        category: 'material',
        scope: 'in-model',
        collection: null,
        placements: {},
        materialInPalette: { 'oak.hew': true },
        nowMs: NOW,
      })
      expect(result.map((i) => i.displayName)).toEqual(['Oak'])
    })

    it('excludes a material not present in `materialInPalette`', () => {
      const oak = materialItem({ displayName: 'Oak' })
      const result = filterItems([oak], {
        query: '',
        category: 'material',
        scope: 'in-model',
        collection: null,
        placements: {},
        materialInPalette: { 'oak.hew': false },
        nowMs: NOW,
      })
      expect(result).toEqual([])
    })

    it('excludes a material when `materialInPalette` is omitted entirely', () => {
      const oak = materialItem({ displayName: 'Oak' })
      const result = filterItems([oak], {
        query: '',
        category: 'material',
        scope: 'in-model',
        collection: null,
        placements: {},
        nowMs: NOW,
      })
      expect(result).toEqual([])
    })

    it('a placed component is unaffected by `materialInPalette` — placements still win for non-materials', () => {
      const result = filterItems(items, {
        query: '',
        category: 'component',
        scope: 'in-model',
        collection: null,
        placements: { 'src-chair': 1 },
        materialInPalette: { 'chair.hew': false },
        nowMs: NOW,
      })
      expect(result.map((i) => i.displayName)).toEqual(['Theater Chair'])
    })
  })

  it('scope "recent" keeps items within 14 days and sorts newest first', () => {
    const oneDayMs = 24 * 60 * 60 * 1000
    const recentA = componentItem({
      file: file('a.hew', { mtimeMs: NOW - 1 * oneDayMs }),
      displayName: 'Recent A',
    })
    const recentB = componentItem({
      file: file('b.hew', { mtimeMs: NOW - 10 * oneDayMs }),
      displayName: 'Recent B',
    })
    const old = componentItem({
      file: file('c.hew', { mtimeMs: NOW - 20 * oneDayMs }),
      displayName: 'Old',
    })
    const result = filterItems([old, recentB, recentA], {
      query: '',
      category: 'component',
      scope: 'recent',
      collection: null,
      placements: {},
      nowMs: NOW,
    })
    expect(result.map((i) => i.displayName)).toEqual(['Recent A', 'Recent B'])
  })

  it('scope "recent" prefers meta.savedAt over file mtime when present', () => {
    const item = componentItem({
      file: file('a.hew', { mtimeMs: NOW - 100 * 24 * 60 * 60 * 1000 }), // ancient mtime
      displayName: 'Saved Recently',
      meta: { savedAt: new Date(NOW - 24 * 60 * 60 * 1000).toISOString() }, // 1 day ago
    })
    const result = filterItems([item], {
      query: '',
      category: 'component',
      scope: 'recent',
      collection: null,
      placements: {},
      nowMs: NOW,
    })
    expect(result).toHaveLength(1)
  })

  it('scope "recent" excludes items with an unknown timestamp (mtimeMs 0, no savedAt)', () => {
    const item = componentItem({ file: file('unknown.hew', { mtimeMs: 0 }) })
    const result = filterItems([item], {
      query: '',
      category: 'component',
      scope: 'recent',
      collection: null,
      placements: {},
      nowMs: NOW,
    })
    expect(result).toEqual([])
  })

  it('filters by collection', () => {
    const furniture = componentItem({ file: file('a.hew'), displayName: 'A', meta: { collection: 'Furniture' } })
    const hardware = componentItem({ file: file('b.hew'), displayName: 'B', meta: { collection: 'Hardware' } })
    const result = filterItems([furniture, hardware], {
      query: '',
      category: 'component',
      scope: 'all',
      collection: 'Furniture',
      placements: {},
      nowMs: NOW,
    })
    expect(result.map((i) => i.displayName)).toEqual(['A'])
  })

  it('selecting a parent collection matches its whole subtree', () => {
    const parent = componentItem({ file: file('a.hew'), displayName: 'Parent', meta: { collection: 'Hardware' } })
    const child = componentItem({
      file: file('b.hew'),
      displayName: 'Child',
      meta: { collection: 'Hardware/Fasteners' },
    })
    const grandchild = componentItem({
      file: file('c.hew'),
      displayName: 'Grandchild',
      meta: { collection: 'Hardware/Fasteners/Metric' },
    })
    const result = filterItems([parent, child, grandchild], {
      query: '',
      category: 'component',
      scope: 'all',
      collection: 'Hardware',
      placements: {},
      nowMs: NOW,
    })
    expect(result.map((i) => i.displayName).sort()).toEqual(['Child', 'Grandchild', 'Parent'])
  })

  it('selecting a parent collection does NOT match a same-prefix sibling name', () => {
    const child = componentItem({
      file: file('a.hew'),
      displayName: 'Child',
      meta: { collection: 'Hardware/Fasteners' },
    })
    const sibling = componentItem({ file: file('b.hew'), displayName: 'Sibling', meta: { collection: 'HardwareX' } })
    const result = filterItems([child, sibling], {
      query: '',
      category: 'component',
      scope: 'all',
      collection: 'Hardware',
      placements: {},
      nowMs: NOW,
    })
    expect(result.map((i) => i.displayName)).toEqual(['Child'])
  })

  it('selecting a nested collection excludes its own parent', () => {
    const parent = componentItem({ file: file('a.hew'), displayName: 'Parent', meta: { collection: 'Hardware' } })
    const child = componentItem({
      file: file('b.hew'),
      displayName: 'Child',
      meta: { collection: 'Hardware/Fasteners' },
    })
    const result = filterItems([parent, child], {
      query: '',
      category: 'component',
      scope: 'all',
      collection: 'Hardware/Fasteners',
      placements: {},
      nowMs: NOW,
    })
    expect(result.map((i) => i.displayName)).toEqual(['Child'])
  })

  it('a null collection filter means "any collection" (no restriction)', () => {
    const result = filterItems(items, {
      query: '',
      category: 'component',
      scope: 'all',
      collection: null,
      placements: {},
      nowMs: NOW,
    })
    expect(result).toHaveLength(2)
  })

  // --- 'all' category (finding #4: the sidebar's All row) -------------------
  it('category "all" skips the category filter, mixing every category together', () => {
    const result = filterItems(items, {
      query: '',
      category: 'all',
      scope: 'all',
      collection: null,
      placements: {},
      nowMs: NOW,
    })
    expect(result.map((i) => i.displayName).sort()).toEqual(['Café Table', 'Door Hinge', 'Theater Chair'])
  })

  it('category "all" still composes with search/scope/collection filtering', () => {
    const result = filterItems(items, {
      query: 'chair',
      category: 'all',
      scope: 'all',
      collection: null,
      placements: {},
      nowMs: NOW,
    })
    expect(result.map((i) => i.displayName)).toEqual(['Theater Chair'])
  })
})

// ---------------------------------------------------------------------------
// categoryTypeLabel / sortListItems — the List view's Type column and its
// Name/Type/Size sort (finding #3)
// ---------------------------------------------------------------------------

describe('categoryTypeLabel', () => {
  it('capitalizes each category for the Type column', () => {
    expect(categoryTypeLabel('component')).toBe('Component')
    expect(categoryTypeLabel('material')).toBe('Material')
    expect(categoryTypeLabel('model')).toBe('Model')
  })
})

describe('sortListItems', () => {
  function listItem(displayName: string, category: LibraryItem['category'], size: number): LibraryItem {
    return { ...componentItem({ displayName, category, file: file(`${displayName}.hew`, { size }) }) }
  }

  const items = [listItem('Banana Crate', 'model', 500), listItem('apple bin', 'component', 5_000), listItem('Cherry Jar', 'material', 50)]

  it('sorts by name ascending by default order, case-insensitively', () => {
    const result = sortListItems(items, 'name', 'asc')
    expect(result.map((i) => i.displayName)).toEqual(['apple bin', 'Banana Crate', 'Cherry Jar'])
  })

  it('sorts by name descending', () => {
    const result = sortListItems(items, 'name', 'desc')
    expect(result.map((i) => i.displayName)).toEqual(['Cherry Jar', 'Banana Crate', 'apple bin'])
  })

  it('sorts by type (Component/Material/Model), alphabetically', () => {
    const result = sortListItems(items, 'type', 'asc')
    expect(result.map((i) => i.displayName)).toEqual(['apple bin', 'Cherry Jar', 'Banana Crate'])
  })

  it('sorts by size ascending and descending', () => {
    expect(sortListItems(items, 'size', 'asc').map((i) => i.displayName)).toEqual(['Cherry Jar', 'Banana Crate', 'apple bin'])
    expect(sortListItems(items, 'size', 'desc').map((i) => i.displayName)).toEqual(['apple bin', 'Banana Crate', 'Cherry Jar'])
  })

  it('breaks ties by display name so equal values never jitter', () => {
    const tied = [listItem('Zed', 'component', 100), listItem('Alpha', 'component', 100)]
    expect(sortListItems(tied, 'size', 'asc').map((i) => i.displayName)).toEqual(['Alpha', 'Zed'])
    expect(sortListItems(tied, 'size', 'desc').map((i) => i.displayName)).toEqual(['Alpha', 'Zed'])
  })

  it('never mutates its input array', () => {
    const original = [...items]
    sortListItems(items, 'name', 'desc')
    expect(items).toEqual(original)
  })
})

// ---------------------------------------------------------------------------
// collectionsOf
// ---------------------------------------------------------------------------

describe('collectionsOf', () => {
  it('returns sorted, deduplicated collection nodes at depth 0 for flat collections', () => {
    const items = [
      componentItem({ file: file('a.hew'), meta: { collection: 'Furniture' } }),
      componentItem({ file: file('b.hew'), meta: { collection: 'Hardware' } }),
      componentItem({ file: file('c.hew'), meta: { collection: 'Furniture' } }),
    ]
    expect(collectionsOf(items)).toEqual([
      { path: 'Furniture', label: 'Furniture', depth: 0 },
      { path: 'Hardware', label: 'Hardware', depth: 0 },
    ])
  })

  it('ignores items with no collection or a blank one', () => {
    const items = [
      componentItem({ file: file('a.hew'), meta: {} }),
      componentItem({ file: file('b.hew'), meta: { collection: '' } }),
      componentItem({ file: file('c.hew'), meta: { collection: '   ' } }),
    ]
    expect(collectionsOf(items)).toEqual([])
  })

  it('derives the tree from nested item collections, synthesizing parents', () => {
    const items = [
      componentItem({ file: file('a.hew'), meta: { collection: 'Hardware/Fasteners' } }),
      componentItem({ file: file('b.hew'), meta: { collection: 'Hardware/Screws' } }),
    ]
    expect(collectionsOf(items)).toEqual([
      { path: 'Hardware', label: 'Hardware', depth: 0 },
      { path: 'Hardware/Fasteners', label: 'Fasteners', depth: 1 },
      { path: 'Hardware/Screws', label: 'Screws', depth: 1 },
    ])
  })
})

// ---------------------------------------------------------------------------
// Collection paths — normalization, subtree matching, tree derivation
// ---------------------------------------------------------------------------

describe('collectionSegments / normalizeCollectionPath', () => {
  it('splits a plain path into segments', () => {
    expect(collectionSegments('Hardware/Fasteners')).toEqual(['Hardware', 'Fasteners'])
  })

  it('trims whitespace around each segment', () => {
    expect(collectionSegments(' Hardware / Fasteners ')).toEqual(['Hardware', 'Fasteners'])
  })

  it('drops empty segments from leading, trailing, and doubled slashes', () => {
    expect(collectionSegments('/Hardware//Fasteners/')).toEqual(['Hardware', 'Fasteners'])
  })

  it('reduces an all-slash/whitespace input to no segments at all', () => {
    expect(collectionSegments('  /  / ')).toEqual([])
  })

  it('normalizeCollectionPath rejoins segments canonically', () => {
    expect(normalizeCollectionPath('/Hardware//Fasteners/')).toBe('Hardware/Fasteners')
  })

  it('normalizeCollectionPath of an all-empty input is the empty string', () => {
    expect(normalizeCollectionPath('   ')).toBe('')
    expect(normalizeCollectionPath('///')).toBe('')
  })

  it('normalizeCollectionPath of a single segment is unchanged (trimmed)', () => {
    expect(normalizeCollectionPath('  Furniture  ')).toBe('Furniture')
  })
})

describe('collectionTreeFromPaths', () => {
  it('returns an empty tree for no paths', () => {
    expect(collectionTreeFromPaths([])).toEqual([])
  })

  it('returns flat depth-0 nodes for unrelated top-level paths, sorted', () => {
    expect(collectionTreeFromPaths(['Zebra', 'Anvil'])).toEqual([
      { path: 'Anvil', label: 'Anvil', depth: 0 },
      { path: 'Zebra', label: 'Zebra', depth: 0 },
    ])
  })

  it('synthesizes an intermediate parent that no item is directly collected under', () => {
    // Only "Hardware/Fasteners" is ever assigned — "Hardware" itself must
    // still appear as a depth-0 row to nest it under.
    expect(collectionTreeFromPaths(['Hardware/Fasteners'])).toEqual([
      { path: 'Hardware', label: 'Hardware', depth: 0 },
      { path: 'Hardware/Fasteners', label: 'Fasteners', depth: 1 },
    ])
  })

  it('dedupes a parent that appears both standalone and as a prefix', () => {
    expect(collectionTreeFromPaths(['Hardware', 'Hardware/Fasteners', 'Hardware'])).toEqual([
      { path: 'Hardware', label: 'Hardware', depth: 0 },
      { path: 'Hardware/Fasteners', label: 'Fasteners', depth: 1 },
    ])
  })

  it('orders siblings alphabetically at every depth (pre-order)', () => {
    const tree = collectionTreeFromPaths(['Hardware/Screws', 'Hardware/Fasteners', 'Software'])
    expect(tree).toEqual([
      { path: 'Hardware', label: 'Hardware', depth: 0 },
      { path: 'Hardware/Fasteners', label: 'Fasteners', depth: 1 },
      { path: 'Hardware/Screws', label: 'Screws', depth: 1 },
      { path: 'Software', label: 'Software', depth: 0 },
    ])
  })

  it('keeps "Hardware" and "HardwareX" as distinct top-level siblings, not parent/child', () => {
    const tree = collectionTreeFromPaths(['Hardware/Fasteners', 'HardwareX'])
    const paths = tree.map((n) => n.path)
    expect(paths).toEqual(['Hardware', 'Hardware/Fasteners', 'HardwareX'])
    expect(tree.find((n) => n.path === 'HardwareX')?.depth).toBe(0)
  })

  it('normalizes ragged slashes before building the tree', () => {
    expect(collectionTreeFromPaths(['/Hardware//Fasteners/'])).toEqual([
      { path: 'Hardware', label: 'Hardware', depth: 0 },
      { path: 'Hardware/Fasteners', label: 'Fasteners', depth: 1 },
    ])
  })

  it('ignores an all-empty path rather than adding a blank root', () => {
    expect(collectionTreeFromPaths(['', '   ', '///'])).toEqual([])
  })

  it('supports depth beyond one level', () => {
    const tree = collectionTreeFromPaths(['A/B/C'])
    expect(tree).toEqual([
      { path: 'A', label: 'A', depth: 0 },
      { path: 'A/B', label: 'B', depth: 1 },
      { path: 'A/B/C', label: 'C', depth: 2 },
    ])
  })
})

describe('collectionMatchesSubtree', () => {
  it('matches the exact same path', () => {
    expect(collectionMatchesSubtree('Hardware', 'Hardware')).toBe(true)
  })

  it('matches a child path under the filter', () => {
    expect(collectionMatchesSubtree('Hardware/Fasteners', 'Hardware')).toBe(true)
  })

  it('matches a grandchild path under the filter', () => {
    expect(collectionMatchesSubtree('Hardware/Fasteners/Metric', 'Hardware')).toBe(true)
  })

  it('does NOT match a same-prefix sibling that is not actually nested (segment boundary)', () => {
    expect(collectionMatchesSubtree('HardwareX', 'Hardware')).toBe(false)
  })

  it('does not match a parent when filtering by a more specific child path', () => {
    expect(collectionMatchesSubtree('Hardware', 'Hardware/Fasteners')).toBe(false)
  })

  it('does not match an unrelated collection', () => {
    expect(collectionMatchesSubtree('Furniture', 'Hardware')).toBe(false)
  })

  it('returns false for an item with no collection at all', () => {
    expect(collectionMatchesSubtree(undefined, 'Hardware')).toBe(false)
  })

  it('returns false for a blank/empty filter path', () => {
    expect(collectionMatchesSubtree('Hardware', '')).toBe(false)
    expect(collectionMatchesSubtree('Hardware', '   ')).toBe(false)
  })

  it('tolerates ragged slashes on either side', () => {
    expect(collectionMatchesSubtree('/Hardware/Fasteners/', 'Hardware/')).toBe(true)
  })
})

// ---------------------------------------------------------------------------
// Formatting helpers
// ---------------------------------------------------------------------------

describe('formatBytes', () => {
  it('shows raw bytes under 1 KB', () => {
    expect(formatBytes(512)).toBe('512 B')
  })

  it('shows KB with one decimal', () => {
    expect(formatBytes(2048)).toBe('2.0 KB')
  })

  it('shows MB with one decimal', () => {
    expect(formatBytes(1_258_291)).toBe('1.2 MB') // ~1.2 MB
  })
})

describe('metadataLine', () => {
  it('formats solids, materials, and size together', () => {
    const s = summary({ objects: 4, materials: 2 })
    const item = buildLibraryItem(file('x.hew', { size: 1_258_291 }), s, 'h')
    expect(metadataLine(item)).toBe('4 solids · 2 materials · 1.2 MB')
  })

  it('uses singular "solid"/"material" for a count of 1', () => {
    const s = summary({ objects: 1, materials: 1 })
    const item = buildLibraryItem(file('x.hew', { size: 100 }), s, 'h')
    expect(metadataLine(item)).toBe('1 solid · 1 material · 100 B')
  })

  it('omits zero-valued counts', () => {
    const s = summary({ objects: 0, materials: 3 })
    const item = buildLibraryItem(file('x.hew', { size: 100 }), s, 'h')
    expect(metadataLine(item)).toBe('3 materials · 100 B')
  })
})

describe('materialSubline', () => {
  function material(overrides: Partial<LibraryMaterialSummary> = {}): LibraryMaterialSummary {
    return {
      name: 'Oak',
      color: [180, 140, 90, 255],
      texture_asset: null,
      texture_format: null,
      texture_world_size: null,
      content_hash: '0',
      ...overrides,
    }
  }

  it('describes a flat opaque color', () => {
    expect(materialSubline(material())).toBe('color')
  })

  it('describes a translucent color with an opacity percentage', () => {
    expect(materialSubline(material({ color: [200, 50, 50, 102] }))).toBe('color · opacity 40%')
  })

  it('describes a texture with its tile size', () => {
    expect(
      materialSubline(
        material({ texture_asset: 'textures/oak.png', texture_format: 'png', texture_world_size: [0.6, 0.6] }),
      ),
    ).toBe('texture · 0.6 m tile')
  })

  it('describes a translucent texture with both tile size and opacity', () => {
    expect(
      materialSubline(
        material({
          texture_asset: 'textures/glass.png',
          texture_format: 'png',
          texture_world_size: [1, 1],
          color: [255, 255, 255, 102],
        }),
      ),
    ).toBe('texture · 1 m tile · opacity 40%')
  })
})

describe('savedLine', () => {
  const item = componentItem()

  it('reads "just now" for sub-minute ages', () => {
    const i = { ...item, meta: { savedAt: new Date(NOW - 5000).toISOString() } }
    expect(savedLine(i, NOW)).toBe('Saved just now')
  })

  it('reads minutes ago within the first hour', () => {
    const i = { ...item, meta: { savedAt: new Date(NOW - 5 * 60 * 1000).toISOString() } }
    expect(savedLine(i, NOW)).toBe('Saved 5 minutes ago')
  })

  it('reads hours ago within the first day', () => {
    const i = { ...item, meta: { savedAt: new Date(NOW - 3 * 60 * 60 * 1000).toISOString() } }
    expect(savedLine(i, NOW)).toBe('Saved 3 hours ago')
  })

  it('reads "yesterday" for exactly one day ago', () => {
    const i = { ...item, meta: { savedAt: new Date(NOW - 24 * 60 * 60 * 1000).toISOString() } }
    expect(savedLine(i, NOW)).toBe('Saved yesterday')
  })

  it('reads "N days ago" up to 30 days', () => {
    const i = { ...item, meta: { savedAt: new Date(NOW - 3 * 24 * 60 * 60 * 1000).toISOString() } }
    expect(savedLine(i, NOW)).toBe('Saved 3 days ago')
  })

  it('falls back to an absolute date past 30 days', () => {
    const i = { ...item, meta: { savedAt: new Date(NOW - 40 * 24 * 60 * 60 * 1000).toISOString() } }
    expect(savedLine(i, NOW)).toMatch(/Saved [A-Z][a-z]{2} \d{1,2}, \d{4}/)
  })

  it('reports an unknown date when neither savedAt nor mtimeMs is usable', () => {
    const i = { ...item, file: file('x.hew', { mtimeMs: 0 }), meta: {} }
    expect(savedLine(i, NOW)).toBe('Saved date unknown')
  })
})

/**
 * Unit tests for tagModel.ts pure helpers.
 *
 * NOTE: as of the first-class tags milestone, the Tags panel and Object Info
 * panel source tag paths from `scene.node_tags()` (first-class kernel data),
 * NOT from parsing node names.  `parseTag` is kept for back-compat and legacy
 * importer awareness, but it no longer drives any panel in the UI.
 *
 * Tests here cover:
 *   - `parseTag` — kept as a regression guard for the legacy function itself.
 *   - `buildTagTree` / `collectTagDescendantNodes` / `tagPathKey` / `isPathUnder`
 *     — unchanged; they operate on `{ node, path }` pairs regardless of source.
 */
import { describe, it, expect } from 'vitest'
import {
  parseTag,
  buildTagTree,
  collectTagDescendantNodes,
  tagPathKey,
  isPathUnder,
  type TagTreeNode,
} from './tagModel'
import type { NodeRef } from './treeModel'

// ---------------------------------------------------------------------------
// parseTag
// ---------------------------------------------------------------------------

describe('parseTag', () => {
  it('returns full name as display and null path for an untagged name', () => {
    expect(parseTag('Roof Truss A')).toEqual({ display: 'Roof Truss A', path: null })
    expect(parseTag('')).toEqual({ display: '', path: null })
  })

  it('parses a single flat tag', () => {
    expect(parseTag('Roof Truss A__HEWTAG__Structure')).toEqual({
      display: 'Roof Truss A',
      path: ['Structure'],
    })
  })

  it('parses a nested path joined by __HEWSEP__', () => {
    expect(parseTag('Beam__HEWTAG__Building__HEWSEP__Level1__HEWSEP__Framing')).toEqual({
      display: 'Beam',
      path: ['Building', 'Level1', 'Framing'],
    })
  })

  // SketchUp sanitizes node names to [A-Za-z0-9_]: `@@HEWTAG@@` arrives as a run
  // of underscores around HEWTAG, the display name is empty for unnamed groups,
  // and spaces in a tag name become underscores.
  it('tolerates the underscore-mangled delimiter SketchUp actually exports', () => {
    expect(parseTag('___HEWTAG__Exterior_Foundation')).toEqual({
      display: '',
      path: ['Exterior_Foundation'],
    })
    expect(parseTag('__HEWTAG__PolyIso')).toEqual({ display: '', path: ['PolyIso'] })
    expect(parseTag('Wall__HEWTAG__Roof_Framing')).toEqual({
      display: 'Wall',
      path: ['Roof_Framing'],
    })
  })

  it('tolerates underscore-mangled nesting separators', () => {
    expect(parseTag('___HEWTAG__Structure___HEWSEP__Roof')).toEqual({
      display: '',
      path: ['Structure', 'Roof'],
    })
  })

  it('only the first delimiter splits (rest is tag data)', () => {
    expect(parseTag('display__HEWTAG__seg1__HEWSEP__seg2')).toEqual({
      display: 'display',
      path: ['seg1', 'seg2'],
    })
  })

  it('a trailing delimiter with no tag yields a null path', () => {
    expect(parseTag('Thing__HEWTAG__')).toEqual({ display: 'Thing', path: null })
  })
})

// ---------------------------------------------------------------------------
// buildTagTree
// ---------------------------------------------------------------------------

const obj = (id: bigint): NodeRef => ({ kind: 'object', id })
const inst = (id: bigint): NodeRef => ({ kind: 'instance', id })

describe('buildTagTree', () => {
  it('returns an empty array for an empty input', () => {
    expect(buildTagTree([])).toEqual([])
  })

  it('creates a single root tag with one node', () => {
    const result = buildTagTree([{ node: obj(1n), path: ['Structure'] }])
    expect(result).toHaveLength(1)
    expect(result[0].segment).toBe('Structure')
    expect(result[0].path).toEqual(['Structure'])
    expect(result[0].nodes).toEqual([obj(1n)])
    expect(result[0].children).toHaveLength(0)
  })

  it('groups multiple nodes under the same root tag', () => {
    const result = buildTagTree([
      { node: obj(1n), path: ['Structure'] },
      { node: obj(2n), path: ['Structure'] },
    ])
    expect(result).toHaveLength(1)
    expect(result[0].nodes).toHaveLength(2)
  })

  it('creates separate root tags for different root segments', () => {
    const result = buildTagTree([
      { node: obj(1n), path: ['Exterior'] },
      { node: obj(2n), path: ['Interior'] },
    ])
    // Sorted alphabetically
    expect(result.map((r) => r.segment)).toEqual(['Exterior', 'Interior'])
  })

  it('creates a nested child tag', () => {
    const result = buildTagTree([
      { node: obj(1n), path: ['Structure', 'Roof'] },
    ])
    expect(result).toHaveLength(1)
    const structure = result[0]
    expect(structure.segment).toBe('Structure')
    // The node is tagged at ['Structure','Roof'], so the root Structure carries no direct nodes
    expect(structure.nodes).toHaveLength(0)
    expect(structure.children).toHaveLength(1)
    const roof = structure.children[0]
    expect(roof.segment).toBe('Roof')
    expect(roof.path).toEqual(['Structure', 'Roof'])
    expect(roof.nodes).toEqual([obj(1n)])
  })

  it('places nodes at the exact depth of their path', () => {
    const result = buildTagTree([
      { node: obj(1n), path: ['A'] },
      { node: obj(2n), path: ['A', 'B'] },
      { node: obj(3n), path: ['A', 'B', 'C'] },
    ])
    expect(result).toHaveLength(1)
    const a = result[0]
    expect(a.nodes).toEqual([obj(1n)])
    expect(a.children).toHaveLength(1)
    const b = a.children[0]
    expect(b.nodes).toEqual([obj(2n)])
    expect(b.children).toHaveLength(1)
    const c = b.children[0]
    expect(c.nodes).toEqual([obj(3n)])
    expect(c.children).toHaveLength(0)
  })

  it('skips entries with empty path arrays', () => {
    const result = buildTagTree([
      { node: obj(1n), path: [] },
      { node: obj(2n), path: ['Real'] },
    ])
    expect(result).toHaveLength(1)
    expect(result[0].segment).toBe('Real')
  })

  it('sorts siblings alphabetically', () => {
    const result = buildTagTree([
      { node: obj(1n), path: ['Z'] },
      { node: obj(2n), path: ['A'] },
      { node: obj(3n), path: ['M'] },
    ])
    expect(result.map((r) => r.segment)).toEqual(['A', 'M', 'Z'])
  })

  it('handles mixed node kinds', () => {
    const result = buildTagTree([
      { node: obj(1n), path: ['Tag'] },
      { node: inst(2n), path: ['Tag'] },
    ])
    expect(result[0].nodes).toHaveLength(2)
    expect(result[0].nodes).toContainEqual(obj(1n))
    expect(result[0].nodes).toContainEqual(inst(2n))
  })
})

// ---------------------------------------------------------------------------
// collectTagDescendantNodes
// ---------------------------------------------------------------------------

describe('collectTagDescendantNodes', () => {
  it('returns just the direct nodes when no children', () => {
    const tag: TagTreeNode = {
      segment: 'Roof',
      path: ['Roof'],
      nodes: [obj(1n), obj(2n)],
      children: [],
    }
    expect(collectTagDescendantNodes(tag)).toEqual([obj(1n), obj(2n)])
  })

  it('includes all descendant nodes recursively', () => {
    const grandchild: TagTreeNode = {
      segment: 'Detail',
      path: ['Structure', 'Roof', 'Detail'],
      nodes: [obj(3n)],
      children: [],
    }
    const child: TagTreeNode = {
      segment: 'Roof',
      path: ['Structure', 'Roof'],
      nodes: [obj(2n)],
      children: [grandchild],
    }
    const root: TagTreeNode = {
      segment: 'Structure',
      path: ['Structure'],
      nodes: [obj(1n)],
      children: [child],
    }
    const result = collectTagDescendantNodes(root)
    expect(result).toContainEqual(obj(1n))
    expect(result).toContainEqual(obj(2n))
    expect(result).toContainEqual(obj(3n))
    expect(result).toHaveLength(3)
  })

  it('returns empty for a tag with no nodes and no children', () => {
    const tag: TagTreeNode = { segment: 'Empty', path: ['Empty'], nodes: [], children: [] }
    expect(collectTagDescendantNodes(tag)).toEqual([])
  })
})

// ---------------------------------------------------------------------------
// tagPathKey
// ---------------------------------------------------------------------------

describe('tagPathKey', () => {
  it('round-trips single-segment paths', () => {
    expect(tagPathKey(['A'])).toBe(JSON.stringify(['A']))
  })

  it('is distinct for different paths', () => {
    expect(tagPathKey(['A', 'B'])).not.toBe(tagPathKey(['A']))
    expect(tagPathKey(['A', 'B'])).not.toBe(tagPathKey(['B', 'A']))
  })
})

// ---------------------------------------------------------------------------
// isPathUnder
// ---------------------------------------------------------------------------

describe('isPathUnder', () => {
  it('returns true for an exact match', () => {
    expect(isPathUnder(['A', 'B'], ['A', 'B'])).toBe(true)
  })

  it('returns true when candidate is a descendant of anchor', () => {
    expect(isPathUnder(['A', 'B', 'C'], ['A'])).toBe(true)
    expect(isPathUnder(['A', 'B', 'C'], ['A', 'B'])).toBe(true)
  })

  it('returns false when candidate is an ancestor of anchor', () => {
    expect(isPathUnder(['A'], ['A', 'B'])).toBe(false)
  })

  it('returns false when paths diverge', () => {
    expect(isPathUnder(['A', 'C'], ['A', 'B'])).toBe(false)
    expect(isPathUnder(['X', 'B'], ['A', 'B'])).toBe(false)
  })

  it('returns false for unrelated paths', () => {
    expect(isPathUnder(['X'], ['A'])).toBe(false)
  })

  it('handles empty anchor path (everything is under root)', () => {
    expect(isPathUnder(['A', 'B'], [])).toBe(true)
    expect(isPathUnder([], [])).toBe(true)
  })
})

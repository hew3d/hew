/**
 * The three Shared Library source exports through their TS wrappers, against
 * the REAL compiled kernel (docs/design/shared-library-sources.md 6.4). Mirrors
 * the Rust cases in crates/wasm-api/src/sources_api.rs. Needs the kernel with
 * the exports built: `HEW_PREVIEW=0 pnpm --dir app wasm` (production flavor;
 * the three exports are in both flavors, so a preview build also works).
 */

import { readFile } from 'node:fs/promises'
import { resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { beforeAll, describe, expect, it } from 'vitest'
import init from '../wasm/pkg/wasm_api.js'
import { SourceWasmError, commitFromResponse, parseIndex, parseSourceRef, sourceUrls, type SourceRef } from './sourcesWasm'

const __dirname = fileURLToPath(new URL('.', import.meta.url))

beforeAll(async () => {
  await init({ module_or_path: await readFile(resolve(__dirname, '../wasm/pkg/wasm_api_bg.wasm')) })
})

const SHA = '0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef'
const COMMIT = '0123456789abcdef0123456789abcdef01234567'

function item(n: number): Record<string, unknown> {
  return {
    id: `00000000-0000-4000-8000-${n.toString(16).padStart(12, '0')}`,
    path: `Components/item-${n}.hew`,
    name: `Item ${n}`,
    category: 'Components',
    keywords: ['a', 'b'],
    collection: 'Hinges',
    sha256: SHA,
    bytes: 100,
    thumbnail: `thumbnails/${SHA}.png`,
    license: 'CC0-1.0',
  }
}

function fullIndex(): Record<string, unknown> {
  return {
    hew: 1,
    name: 'Shop Hardware',
    description: 'Hinges.',
    author: 'Example Maker',
    homepage: 'https://example.com/parts',
    license: 'LicenseRef-Mine',
    licenseUrl: 'https://example.com/license',
    items: [item(1), item(2)],
    plugins: [],
  }
}

/** Runs `f`, which must throw a SourceWasmError; returns it. */
function refusal(f: () => unknown): SourceWasmError {
  try {
    f()
  } catch (e) {
    expect(e).toBeInstanceOf(SourceWasmError)
    const err = e as SourceWasmError
    expect(err.code).toBeTruthy()
    expect(err.reason).toBeTruthy()
    return err
  }
  throw new Error('expected a SourceWasmError, nothing was thrown')
}

function candidate(text: string): SourceRef {
  return parseSourceRef(text)[0]!
}

describe('parseIndex', () => {
  it('returns the parsed index, camelCase (6.1)', () => {
    const idx = parseIndex(JSON.stringify(fullIndex()))
    expect(idx.hew).toBe(1)
    expect(idx.name).toBe('Shop Hardware')
    expect(idx.licenseUrl).toBe('https://example.com/license')
    expect(idx.items).toHaveLength(2)
    expect(idx.items[0]!.category).toBe('Components')
    expect(idx.items[1]!.path).toBe('Components/item-2.hew')
    expect(idx.plugins).toEqual([])
  })

  it('its own answer is a valid index', () => {
    const once = parseIndex(JSON.stringify(fullIndex()))
    expect(parseIndex(JSON.stringify(once))).toEqual(once)
  })

  it('hew above the supported version is index_too_new', () => {
    expect(refusal(() => parseIndex(JSON.stringify({ hew: 2, items: 'who knows' }))).code).toBe('index_too_new')
  })

  it('an index over 4 MB is index_too_large', () => {
    const text = JSON.stringify({ hew: 1, name: 'x', items: [], pad: 'a'.repeat(4 * 1024 * 1024 + 1) })
    expect(refusal(() => parseIndex(text)).code).toBe('index_too_large')
  })

  it('index_invalid carries the 0-based item position', () => {
    const idx = fullIndex()
    idx.items = [item(1), { ...item(2), category: 'Gadgets' }]
    const err = refusal(() => parseIndex(JSON.stringify(idx)))
    expect(err.code).toBe('index_invalid')
    expect(err.item).toBe(1)
  })

  it('a top-level violation is index_invalid with no item', () => {
    const err = refusal(() => parseIndex(JSON.stringify({ hew: 1 })))
    expect(err.code).toBe('index_invalid')
    expect(err.item).toBeUndefined()
  })

  it('text that is not a JSON object is index_invalid, not bad_input', () => {
    for (const text of ['', 'not json', '[]', 'null', '42', '{']) {
      expect(refusal(() => parseIndex(text)).code, text).toBe('index_invalid')
    }
  })

  it('hostile items stay refused: traversal path, duplicate ids', () => {
    const idx = fullIndex()
    idx.items = [{ ...item(1), path: '../../etc/passwd.hew' }]
    expect(refusal(() => parseIndex(JSON.stringify(idx))).code).toBe('index_invalid')
    idx.items = [item(1), item(1)]
    expect(refusal(() => parseIndex(JSON.stringify(idx))).code).toBe('index_invalid')
  })
})

describe('parseSourceRef', () => {
  it('a known host is one candidate in canonical form', () => {
    const c = parseSourceRef('github:o/r//dir@main')
    expect(c).toEqual([
      { kind: 'github', host: 'github.com', repo: 'o/r', dir: 'dir', git_ref: 'main', canonical: 'github:o/r//dir@main' },
    ])
  })

  it('an unknown https host is web, forgejo, gitlab, in that order', () => {
    expect(parseSourceRef('https://example.com/owner/repo').map((r) => r.kind)).toEqual(['web', 'forgejo', 'gitlab'])
  })

  it('a one-segment https URL is a web candidate only', () => {
    expect(parseSourceRef('https://example.com/parts').map((r) => r.kind)).toEqual(['web'])
  })

  it('no @ref gives a null git_ref', () => {
    expect(candidate('github:o/r').git_ref).toBeNull()
  })

  it('refusals are typed', () => {
    expect(refusal(() => parseSourceRef('')).code).toBe('ref_empty')
    expect(refusal(() => parseSourceRef('   ')).code).toBe('ref_empty')
    expect(refusal(() => parseSourceRef('https://github.com/o/r?tab=x')).code).toBe('ref_query_refused')
    expect(refusal(() => parseSourceRef('ftp://example.com/x')).code).toBe('ref_invalid')
    expect(refusal(() => parseSourceRef('http://example.com/x')).code).toBe('ref_invalid')
    expect(refusal(() => parseSourceRef('github:onlyowner')).code).toBe('ref_invalid')
  })
})

describe('sourceUrls', () => {
  it('a git source pinned to a commit: resolve, index, one file URL per path in order', () => {
    const r = candidate('github:o/r//dir@main')
    const out = sourceUrls(r, ['Components/a.hew', 'thumbnails/x.png', 'b.hew'], COMMIT)
    expect(out.resolve).toBe('https://api.github.com/repos/o/r/commits/main')
    expect(out.index).toBe(`https://raw.githubusercontent.com/o/r/${COMMIT}/dir/hew.json`)
    expect(out.files).toEqual([
      `https://raw.githubusercontent.com/o/r/${COMMIT}/dir/Components/a.hew`,
      `https://raw.githubusercontent.com/o/r/${COMMIT}/dir/thumbnails/x.png`,
      `https://raw.githubusercontent.com/o/r/${COMMIT}/dir/b.hew`,
    ])
  })

  it('without a commit the reads use the ref', () => {
    const out = sourceUrls(candidate('github:o/r@main'), ['a.hew'])
    expect(out.index).toBe('https://raw.githubusercontent.com/o/r/main/hew.json')
    expect(out.files).toEqual(['https://raw.githubusercontent.com/o/r/main/a.hew'])
  })

  it('a web source has a null resolve', () => {
    const out = sourceUrls(candidate('web:https://example.com/parts/hew.json'), ['a.hew'])
    expect(out.resolve).toBeNull()
    expect(out.index).toBe('https://example.com/parts/hew.json')
  })

  it('no paths gives an empty files array', () => {
    const out = sourceUrls(candidate('github:o/r'), [])
    expect(out.files).toEqual([])
    expect(typeof out.index).toBe('string')
  })

  it('a path that escapes the folder is path_invalid', () => {
    const r = candidate('github:o/r//dir')
    for (const bad of ['../x.hew', 'a/../../x.hew', '/abs.hew', 'a\\b.hew', '%2e%2e/x.hew', 'a?x.hew', '.hidden/x.hew', '']) {
      expect(refusal(() => sourceUrls(r, ['ok.hew', bad], COMMIT)).code, bad).toBe('path_invalid')
    }
  })

  it('a hand-built ref whose fields do not follow from its canonical form is ref_invalid', () => {
    const hostile = [
      { kind: 'github', host: 'evil.example', repo: '../../x', dir: '../..', git_ref: null, canonical: 'github:o/r' },
      { kind: 'github', host: 'user:pw@evil.example', repo: 'o/r', dir: '', git_ref: 'a/../b', canonical: 'x' },
      { kind: 'gitlab', host: '127.0.0.1', repo: 'g/p', dir: '', git_ref: null, canonical: 'x' },
      { kind: 'forgejo', host: '169.254.169.254', repo: 'o/r', dir: '', git_ref: 'main', canonical: 'github:o/r' },
    ] as SourceRef[]
    for (const r of hostile) {
      expect(refusal(() => sourceUrls(r, ['a.hew'], COMMIT)).code).toBe('ref_invalid')
    }
  })

  it('each field must agree with the canonical form', () => {
    const good = candidate('github:o/r//dir@main')
    const variants: SourceRef[] = [
      { ...good, host: 'evil.example' },
      { ...good, repo: 'x/y' },
      { ...good, dir: 'other' },
      { ...good, git_ref: 'dev' },
      { ...good, git_ref: null },
      { ...good, kind: 'gitlab' },
      { ...good, canonical: 'not a reference' },
    ]
    for (const r of variants) expect(refusal(() => sourceUrls(r, ['a.hew'])).code).toBe('ref_invalid')
  })

  it('a canonical form with several candidates is matched by the fields', () => {
    const cs = parseSourceRef('https://example.com/owner/repo')
    expect(cs.map((c) => c.kind)).toEqual(['web', 'forgejo', 'gitlab'])
    const indexes = cs.map((c) => sourceUrls(c, []).index)
    expect(new Set(indexes).size).toBe(3)
    for (const c of cs) expect(() => sourceUrls({ ...c, canonical: 'https://example.com/owner/repo' }, [])).not.toThrow()
    expect(refusal(() => sourceUrls({ ...cs[1]!, canonical: 'https://example.com/owner/repo', host: 'other.example' }, [])).code).toBe('ref_invalid')
    expect(refusal(() => sourceUrls({ ...cs[0]!, kind: 'gitlab' }, [])).code).toBe('ref_invalid')
    expect(refusal(() => sourceUrls({ ...cs[2]!, repo: 'not/there' }, [])).code).toBe('ref_invalid')
  })

  it('does not refuse loopback or link-local hosts (is_public_host is not applied here)', () => {
    for (const [text, host] of [
      ['gitlab:https://127.0.0.1/g/p', '127.0.0.1'],
      ['forgejo:169.254.169.254/o/r@main', '169.254.169.254'],
      ['gitlab:https://localhost/g/p', 'localhost'],
    ] as const) {
      const out = sourceUrls(candidate(text), ['a.hew'])
      expect(new URL(out.index).hostname).toBe(host)
    }
  })

  it('a commit that is not 40 or 64 lowercase hex is commit_invalid', () => {
    const r = candidate('github:o/r')
    for (const bad of ['../../evil?x=1#', '', 'main', COMMIT.slice(0, 39), `${COMMIT}0`, COMMIT.toUpperCase(), `${COMMIT.slice(0, 39)}g`]) {
      expect(refusal(() => sourceUrls(r, ['a.hew'], bad)).code, bad).toBe('commit_invalid')
    }
    expect(() => sourceUrls(r, [], 'a'.repeat(64))).not.toThrow()
  })

  it('malformed or wrongly shaped input is bad_input, never a throw of another kind', () => {
    // The wrapper serializes its arguments, so wrong shapes arrive as JSON values.
    const good = candidate('github:o/r')
    for (const bad of [null, [], {}, { kind: 'nope' }, 'github:o/r']) {
      expect(refusal(() => sourceUrls(bad as unknown as SourceRef, ['a.hew'])).code).toBe('bad_input')
    }
    for (const bad of [null, {}, 'a.hew', [1, 2], [null], [['a']]]) {
      expect(refusal(() => sourceUrls(good, bad as unknown as string[])).code).toBe('bad_input')
    }
  })
})

describe('commitFromResponse', () => {
  const GH = '7fd1a60b01f91b314f59955a4e4d4e80d8edf11d'
  const GL = '13dd661da0a5d40364b42bad39af78686deaa073'
  const CB = '7d55578694c587995e8ac43b7e7e236ecda4e00a'
  const SHA256 = 'a'.repeat(64)

  it('reads the commit from each kind\'s resolve response', () => {
    expect(commitFromResponse('github', JSON.stringify({ sha: GH, node_id: 'x' }))).toBe(GH)
    expect(commitFromResponse('github', GH)).toBe(GH) // the bare vnd.github.sha body
    expect(commitFromResponse('gitlab', JSON.stringify({ id: GL, short_id: '13dd661d' }))).toBe(GL)
    expect(commitFromResponse('forgejo', JSON.stringify({ commit: { id: CB } }))).toBeNull() // old branches/{ref} shape
    expect(commitFromResponse('forgejo', JSON.stringify({ sha: CB }))).toBe(CB) // git/commits/{ref}
    expect(commitFromResponse('github', JSON.stringify({ sha: SHA256 }))).toBe(SHA256)
  })

  it('a web source has no commit', () => {
    expect(commitFromResponse('web', JSON.stringify({ sha: GH }))).toBeNull()
  })

  it('short, uppercase, spliced or unrecognized bodies are null, not errors', () => {
    const bad = [
      '', '{}', '[]', 'null', 'not json', '<html>rate limited</html>',
      JSON.stringify({ sha: GH.slice(0, 12) }),
      JSON.stringify({ sha: GH.toUpperCase() }),
      JSON.stringify({ sha: `${GH}/../x?y#` }),
      JSON.stringify({ sha: 5 }),
      JSON.stringify({ id: null }),
    ]
    for (const kind of ['github', 'gitlab', 'forgejo'] as const) {
      for (const body of bad) expect(commitFromResponse(kind, body), `${kind} ${body}`).toBeNull()
    }
  })

  it('an unknown kind is bad_input', () => {
    expect(refusal(() => commitFromResponse('svn' as never, '{}')).code).toBe('bad_input')
  })
})

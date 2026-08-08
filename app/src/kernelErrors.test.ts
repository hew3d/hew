/**
 * Plain-language error copy tests.
 *
 * The load-bearing one is the exhaustiveness check. Its inventory of every
 * code the wasm boundary can emit is NOT hand-maintained (a hand-copied list
 * silently drifts — eight annotation/FFI-validation codes were added to the
 * kernel and wasm-api without ever being copied into a hand list here, and
 * nothing caught it). Instead `DERIVED_KERNEL_ERROR_CODES` below is scraped
 * structurally from the checked-in Rust sources, mirroring the same
 * source-scrape approach `nativeMenuParity.test.ts` already uses for the
 * Rust/TS menu-id drift check:
 *
 *  - Every enum an error can be minted FROM (`DocumentError`'s own flat
 *    variants, plus each leaf error enum it delegates to per `doc_err` in
 *    crates/wasm-api/src/lib.rs: `SketchError`, `ExtrudeError`,
 *    `FollowMeError`, `PushPullError`, `StickyError`, `BooleanError`,
 *    `SliceError`, `TransformError`, `MathError`, `LoadError`) is parsed for
 *    its variant identifiers directly out of its `pub enum … { … }` body —
 *    adding a variant to any of THESE enums is caught with no test change.
 *  - Every literal `"CODE: message"` wasm-api mints inline (`stale("CODE",
 *    …)`, `ApiError("CODE: …")`, `ApiError::new("CODE", …)`) is scraped by
 *    regex from crates/wasm-api/src/lib.rs.
 *  - WHICH `DocumentError` variants delegate their CODE to one of those leaf
 *    enums, rather than being their own code, is itself parsed out of
 *    `doc_err`'s match arms (`extractDocErrDelegators` /
 *    `DOCUMENT_ERROR_DELEGATORS` below) instead of hand-copied a second
 *    time — hand-copying this set was itself a prior sync-point gap: a new
 *    delegating arm forgotten here would silently orphan a hand-listed
 *    requirement while hiding the delegated-to enum's real codes, all green.
 *
 * A whole NEW leaf error enum (not one of the ones listed above) would still
 * need a one-line addition to `LEAF_ENUMS` below — the same "if the file
 * moves, update the const" tradeoff `nativeMenuParity.test.ts` already
 * accepts over building real cross-language codegen for a short, stable
 * list. Parsing `doc_err`'s match arms closes drift on WHICH variants
 * delegate, but not on this: a fresh `DocumentError::Foo(inner) =>
 * api_err(inner, &e)` arm is correctly recognized as delegating (so `Foo`
 * itself stops contributing a bogus code), but `Foo`'s inner enum's own
 * variants still need that one-line `LEAF_ENUMS` addition to be scraped at
 * all. A build-artifact JSON emitted by a kernel test was the other option
 * considered; rejected because it couples this suite's pass/fail to
 * `cargo test` having already run and left a fresh file behind — a plain
 * `pnpm --dir app test` would either read a stale artifact or fail with a
 * confusing "file not found" instead of a real assertion failure. Parsing
 * the checked-in source has no such ordering dependency and needs no new
 * build step.
 */

import { readFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { describe, it, expect } from 'vitest'
import {
  parseKernelErrorCode,
  kernelErrorMessage,
  friendlyErrorText,
  describedErrorCodes,
  isErrorLevelCode,
  WRAPPER_CODES,
} from './kernelErrors'

const REPO_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '../..')

function readRust(relPath: string): string {
  return readFileSync(resolve(REPO_ROOT, relPath), 'utf8')
}

/**
 * Splits a Rust item list (an enum body, a match arm list, …) at top-level
 * commas only — commas nested inside `()`/`{}`/`[]` (a struct variant's
 * fields, a tuple variant's payload type) don't count as separators.
 */
function splitTopLevel(body: string): string[] {
  const chunks: string[] = []
  let depth = 0
  let cur = ''
  for (const ch of body) {
    if (ch === '(' || ch === '{' || ch === '[') depth++
    else if (ch === ')' || ch === '}' || ch === ']') depth--
    if (ch === ',' && depth === 0) {
      chunks.push(cur)
      cur = ''
    } else {
      cur += ch
    }
  }
  if (cur.trim() !== '') chunks.push(cur)
  return chunks
}

/**
 * Extracts every variant identifier from a `pub enum <enumName> { … }` block
 * in `source` — unit (`Ident,`), tuple (`Ident(Type),`), and struct
 * (`Ident { field: Type },`) variants alike, since only the identifier
 * BEFORE the first `,`/`(`/`{` at the enum's own top level is taken.
 */
function extractEnumVariants(source: string, enumName: string): string[] {
  // Strip `//`/`///`/`//!` comments FIRST, before the brace-depth walk below
  // ever runs: walking depth over raw source would count a brace mentioned
  // inside a doc comment (e.g. "/// unbalanced: `{`") as real enum
  // structure, corrupting where the walk thinks the body ends — silently
  // truncating or overrunning the variant list well below the sanity
  // floor's guard, rather than throwing. Doing this once, up front, also
  // means the header search itself can't accidentally match a mention of
  // the enum name left in a preceding comment.
  const stripped = source.replace(/\/\/.*$/gm, '')
  const header = new RegExp(`pub enum ${enumName}\\b[^{]*\\{`)
  const m = header.exec(stripped)
  if (m === null) {
    throw new Error(
      `kernelErrors.test.ts: enum ${enumName} not found — has it been renamed or moved? Update the LEAF_ENUMS/paths below.`,
    )
  }
  let i = m.index + m[0].length
  let depth = 1
  const start = i
  while (depth > 0) {
    if (stripped[i] === '{') depth++
    else if (stripped[i] === '}') depth--
    i++
  }
  const body = stripped.slice(start, i - 1)
  return splitTopLevel(body)
    .map((c) => c.trim())
    .filter((c) => c.length > 0)
    .map((c) => {
      const vm = /^([A-Za-z][A-Za-z0-9_]*)/.exec(c)
      if (vm === null) {
        throw new Error(`kernelErrors.test.ts: couldn't parse a variant name from "${c}" in enum ${enumName}`)
      }
      return vm[1]
    })
}

/**
 * Every literal `"CODE: message"` (or bare `"CODE"`) string wasm-api mints
 * directly at the FFI boundary — `stale("CODE", "what")`'s first argument,
 * `ApiError("CODE: msg")`/`ApiError(format!("CODE: msg", …))`'s leading code,
 * and `ApiError::new("CODE", …)` — none of these have a backing kernel enum
 * variant to scrape instead.
 */
function extractLiteralCodes(source: string): string[] {
  const codes = new Set<string>()
  for (const m of source.matchAll(/\bstale\(\s*"([A-Za-z][A-Za-z0-9_]*)"/g)) codes.add(m[1])
  for (const m of source.matchAll(/\bApiError\(\s*(?:format!\()?\s*"([A-Za-z][A-Za-z0-9_]*):/g)) {
    codes.add(m[1])
  }
  for (const m of source.matchAll(/\bApiError::new\(\s*"([A-Za-z][A-Za-z0-9_]*)"/g)) codes.add(m[1])
  return [...codes]
}

/**
 * Parses `doc_err`'s own match arms out of crates/wasm-api/src/lib.rs to
 * derive which `DocumentError` variants delegate their CODE to an inner
 * typed error, instead of hand-copying that set as a second sync point that
 * can silently drift from the function it's describing (a new delegating
 * arm — or a removed one — forgotten here would hide the affected variant's
 * real codes without failing anything). Isolates the function body with the
 * same brace-depth walk `extractEnumVariants` uses (comments stripped
 * first, for the same reason), then matches every `DocumentError::Name(` in
 * it; `Op`'s two arms (`KernelOpError::PushPull`/`Sticky`) both surface as
 * the single outer name `Op`, deduped via the `Set`. The `_ => api_err(&e,
 * &e)` fallback arm has no `DocumentError::Name(` to match, so it never
 * contributes here — exactly the "own code" set `documentErrorCodes` below
 * needs to exclude.
 */
function extractDocErrDelegators(source: string): string[] {
  const stripped = source.replace(/\/\/.*$/gm, '')
  const header = /fn doc_err\([^{]*\{/
  const m = header.exec(stripped)
  if (m === null) {
    throw new Error(
      'kernelErrors.test.ts: doc_err not found — has it been renamed or moved? Update DOCUMENT_ERROR_DELEGATORS\'s derivation below.',
    )
  }
  let i = m.index + m[0].length
  let depth = 1
  const start = i
  while (depth > 0) {
    if (stripped[i] === '{') depth++
    else if (stripped[i] === '}') depth--
    i++
  }
  const body = stripped.slice(start, i - 1)
  const delegators = new Set<string>()
  for (const dm of body.matchAll(/DocumentError::([A-Za-z_][A-Za-z0-9_]*)\(/g)) delegators.add(dm[1])
  return [...delegators]
}

const documentRs = readRust('crates/kernel/src/document.rs')
const sketchRs = readRust('crates/kernel/src/sketch.rs')
const opsRs = readRust('crates/kernel/src/ops.rs')
const transformRs = readRust('crates/kernel/src/transform.rs')
const mathRs = readRust('crates/kernel/src/math.rs')
const serializeRs = readRust('crates/kernel/src/serialize.rs')
const wasmApiRs = readRust('crates/wasm-api/src/lib.rs')

// `DocumentError` variants that delegate their CODE to an inner typed error
// instead of contributing their own field name — derived structurally by
// `extractDocErrDelegators` above, parsing `doc_err`'s own match arms in
// crates/wasm-api/src/lib.rs, rather than hand-copied a second time (a hand
// list here could silently go stale the same way the codes list itself
// used to: a new delegating arm added to `doc_err` without a matching
// addition here would hide that variant's real inner codes behind an
// orphan "own code" that's never actually emitted, with every test still
// green). Every OTHER `DocumentError` variant — including a tuple variant
// like `InverseFailed` — is its own code via `doc_err`'s `_ => api_err(&e,
// &e)` fallback, because Rust's derived `Debug` leads with the OUTER
// variant's name regardless of its payload.
//
// This closes drift on WHICH variants delegate, but not on WHAT they
// delegate TO: a whole new leaf error enum introduced by a fresh
// `DocumentError::Foo(inner) => api_err(inner, &e)` arm is correctly
// recognized as a delegator (so `Foo` stops contributing a bogus code of
// its own), but `Foo`'s inner enum still needs a one-line addition to
// LEAF_ENUMS below to actually have ITS variants scraped — the same "if the
// file moves, update the const" tradeoff LEAF_ENUMS's own comment already
// accepts, just not eliminable by parsing `doc_err` alone.
const DOCUMENT_ERROR_DELEGATORS = new Set(extractDocErrDelegators(wasmApiRs))

const documentErrorCodes = extractEnumVariants(documentRs, 'DocumentError').filter(
  (v) => !DOCUMENT_ERROR_DELEGATORS.has(v),
)

// The leaf error enums `DocumentError` (or a standalone wasm-api call site,
// for `MathError`/`LoadError`) delegates its CODE to — each contributes ALL
// of its own variants.
const LEAF_ENUMS: ReadonlyArray<readonly [string, string]> = [
  ['SketchError', sketchRs],
  ['ExtrudeError', opsRs],
  ['FollowMeError', opsRs],
  ['PushPullError', opsRs],
  ['StickyError', opsRs],
  ['BooleanError', opsRs],
  ['SliceError', opsRs],
  ['TransformError', transformRs],
  ['MathError', mathRs],
  ['LoadError', serializeRs],
]
const leafErrorCodes = LEAF_ENUMS.flatMap(([name, source]) => extractEnumVariants(source, name))

const literalCodes = extractLiteralCodes(wasmApiRs)

/** Every error code the wasm boundary can emit, derived structurally from
 * the checked-in Rust sources (see the module doc comment above). */
const DERIVED_KERNEL_ERROR_CODES = [...new Set([...documentErrorCodes, ...leafErrorCodes, ...literalCodes])]

describe('extractEnumVariants — robustness', () => {
  it('is unaffected by an unbalanced brace inside a doc comment', () => {
    // Without stripping comments BEFORE the brace-depth walk, the stray `}`
    // on the `Bar` doc comment below would close the walk right there —
    // reading the enum as containing zero variants instead of two (`Bar`
    // parses out of a truncated body, but nothing after it exists to walk
    // over). This is the same brace-in-comment shape a real doc comment
    // could plausibly contain (e.g. describing code with example braces).
    const fixture = `
pub enum Foo {
    /// unbalanced: }
    Bar,
    Baz(String),
}
`
    expect(extractEnumVariants(fixture, 'Foo')).toEqual(['Bar', 'Baz'])
  })
})

describe('DOCUMENT_ERROR_DELEGATORS — structural derivation', () => {
  it('parsed the expected delegating variants out of doc_err, not a false-empty derivation', () => {
    // A guard against `extractDocErrDelegators` silently regressing to
    // near-nothing (`doc_err` renamed/restructured in a way the regex stops
    // matching) — every `DocumentError` variant would then wrongly count as
    // its own code instead of delegating, which the exhaustiveness check
    // below would eventually catch but only obliquely.
    for (const name of ['Sketch', 'Extrude', 'FollowMe', 'Boolean', 'Slice', 'Transform', 'Op']) {
      expect(DOCUMENT_ERROR_DELEGATORS.has(name), name).toBe(true)
    }
  })
})

describe('parseKernelErrorCode', () => {
  it('parses a CODE: message format', () => {
    const code = parseKernelErrorCode(new Error('WouldVanish: face would be removed'))
    expect(code).toBe('WouldVanish')
  })

  it('parses multi-word codes', () => {
    expect(parseKernelErrorCode(new Error('NonManifoldResult: edge shared by 3+ faces'))).toBe(
      'NonManifoldResult',
    )
  })

  it('returns null when format does not match', () => {
    expect(parseKernelErrorCode(new Error('something went wrong'))).toBeNull()
    expect(parseKernelErrorCode(new Error(''))).toBeNull()
    expect(parseKernelErrorCode('plain string')).toBeNull()
  })

  it('handles non-Error objects', () => {
    expect(parseKernelErrorCode('WouldVanish: bad things')).toBe('WouldVanish')
  })
})

describe('kernelErrorMessage — coverage', () => {
  it('the structural derivation itself found a plausible-sized inventory', () => {
    // A guard against the extractor silently regressing to near-nothing (a
    // renamed enum, a moved file, a regex that stopped matching) and making
    // the "has copy for every code" check below vacuously pass. The real
    // inventory is ~100 codes; anything far below that means the scrape
    // broke, not that the kernel shrank.
    expect(DERIVED_KERNEL_ERROR_CODES.length).toBeGreaterThan(90)
  })

  it('has copy for every kernel error code the boundary can emit (derived structurally from the Rust sources)', () => {
    const described = new Set(describedErrorCodes())
    const missing = DERIVED_KERNEL_ERROR_CODES.filter(
      (c) => !described.has(c) && !WRAPPER_CODES.has(c),
    )
    expect(missing).toEqual([])
  })

  it('has no orphaned copy for codes the kernel no longer emits', () => {
    // The app itself emits a few refusals through the same "CODE: copy"
    // toast convention without a kernel enum behind them — enumerate them
    // here so the guard still catches genuinely stale kernel copy.
    const APP_ERROR_CODES = ['InvalidSelection', 'NestedComponentInContext']
    const known = new Set([...DERIVED_KERNEL_ERROR_CODES, ...APP_ERROR_CODES, ...WRAPPER_CODES])
    const orphans = describedErrorCodes().filter((c) => !known.has(c))
    expect(orphans).toEqual([])
  })

  it('every description is plain language: complete sentences, no raw code, no jargon', () => {
    for (const code of describedErrorCodes()) {
      const msg = kernelErrorMessage(code, 'raw detail')
      expect(msg, code).toMatch(/\.$/)
      expect(msg, code).not.toContain(code)
      expect(msg.toLowerCase(), code).not.toMatch(/\bmanifold\b/)
      expect(msg.toLowerCase(), code).not.toMatch(/\bcoplanar\b/)
      expect(msg.toLowerCase(), code).not.toMatch(/\btopology\b/)
      expect(msg, code).not.toContain('raw detail')
    }
  })

  it('refusals that need action carry a suggested next step (second sentence)', () => {
    // Spot the pattern on representative refusals across the op families.
    for (const code of [
      'WouldVanish', 'NonManifoldResult', 'RadiusVanishes', 'DistanceTooSmall',
      'EndpointNotOnBoundary', 'DegenerateContact', 'PlaneMissesSolid',
      'MixedParents', 'CannotExplodeReflected', 'WouldRetopologize',
      'RestoreConflicts', 'WallNeighborNonPlanar',
    ]) {
      const sentences = kernelErrorMessage(code, '').match(/[.!?](\s|$)/g) ?? []
      expect(sentences.length, code).toBeGreaterThanOrEqual(2)
    }
  })

  it('still degrades safely for a code the table has never seen', () => {
    const msg = kernelErrorMessage('SomeUnknownCode', 'raw detail')
    expect(msg).toContain('SomeUnknownCode')
    expect(msg).toContain('raw detail')
    expect(msg).toContain('Report Bug')
  })
})

describe('friendlyErrorText', () => {
  it('maps a kernel CODE: message error to its plain-language copy', () => {
    const text = friendlyErrorText(new Error('WouldVanish: face 42 would be removed'))
    expect(text).toBe(kernelErrorMessage('WouldVanish', 'face 42 would be removed'))
    expect(text).toContain('Push a shorter distance')
  })

  it('passes non-kernel errors through unchanged (host errors are already human text)', () => {
    expect(friendlyErrorText(new Error('permission denied: C:\\models\\a.hew'))).toBe(
      'permission denied: C:\\models\\a.hew',
    )
    expect(friendlyErrorText('disk full')).toBe('disk full')
  })

  it('unwraps importer format tags and shows their human payload as-is', () => {
    // The .skp importer's own message carries exact guidance — it must reach
    // the toast intact, not buried in the unknown-code fallback.
    const skp =
      'SKP: unsupported SketchUp version {26.2.0}: open it in SketchUp and ' +
      'File ▸ Save As ▸ SketchUp Version 2017, then import that'
    expect(friendlyErrorText(new Error(skp))).toBe(skp.slice('SKP: '.length))
    expect(friendlyErrorText(new Error('DAE: missing <library_geometries>'))).toBe(
      'missing <library_geometries>',
    )
    expect(friendlyErrorText(new Error('glTF: buffer 0 out of range'))).toBe(
      'buffer 0 out of range',
    )
  })

  it('carries copy for the app-side InvalidSelection refusal (structuralSelection boundary)', () => {
    const text = kernelErrorMessage('InvalidSelection', '')
    expect(text).toContain('group or component')
    expect(text).toContain('Sketches')
  })

  it('maps load failures through their typed variant codes', () => {
    // Scene::load emits LoadError variant codes ("NotAContainer: …"), the
    // same boundary convention as every other typed error.
    const text = friendlyErrorText(new Error('NotAContainer: bad magic'))
    expect(text).toBe(kernelErrorMessage('NotAContainer', 'bad magic'))
    expect(text).toContain('Hew document')
  })
})

describe('isErrorLevelCode', () => {
  it('classifies every boolean-operand refusal as an error, like its siblings', () => {
    // The three group-boolean refusals must not render one level softer than
    // OperandNotSolid (adversarial review, minor).
    for (const code of [
      'OperandNotSolid', 'DegenerateContact', 'EmptyResult',
      'BooleanOperandHasInstance', 'BooleanOperandNotSolid', 'BooleanOperandEmpty',
    ]) {
      expect(isErrorLevelCode(code), code).toBe(true)
    }
  })

  it('leaves ordinary refusals at warning level', () => {
    expect(isErrorLevelCode('DistanceTooSmall')).toBe(false)
    expect(isErrorLevelCode('GroupedOperand')).toBe(false)
  })

  it('only classifies codes that actually have copy', () => {
    const described = new Set(describedErrorCodes())
    for (const code of [
      'WouldVanish', 'NonManifoldResult', 'ObjectNotSolid', 'DegenerateGeometry',
      'OperandNotSolid', 'DegenerateContact', 'EmptyResult', 'SingularTransform',
      'BooleanOperandHasInstance', 'BooleanOperandNotSolid', 'BooleanOperandEmpty',
    ]) {
      expect(described.has(code), `${code} has copy`).toBe(true)
    }
  })
})

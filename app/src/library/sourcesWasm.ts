// Typed wrappers over the three Shared Library source exports of the kernel
// module (docs/design/shared-library-sources.md 6.4): `source_parse_index`,
// `source_parse_ref` and `source_urls`. The exports take and return JSON text
// and answer `{ "error": { "code", "reason" } }` on failure; the wrappers
// turn that into a thrown `SourceWasmError`.
//
// The wasm module must already be initialized by the caller (the loader, or
// `init` in a test). The tests in sourcesWasm.wasm.test.ts pin the behavior.

import {
  source_commit,
  source_parse_index,
  source_parse_ref,
  source_urls,
} from '../wasm/pkg/wasm_api.js'

export type SourceKind = 'github' | 'gitlab' | 'forgejo' | 'web'
export type IndexCategory = 'Components' | 'Materials' | 'Models' | 'Profiles'

/** One reference in canonical form, as `source_parse_ref` returns it. */
export interface SourceRef {
  kind: SourceKind
  host: string
  repo: string
  dir: string
  git_ref: string | null
  canonical: string
}

export interface IndexItem {
  id: string
  path: string
  name: string
  category: IndexCategory
  keywords: string[]
  collection: string | null
  sha256: string
  bytes: number
  thumbnail: string | null
  license: string | null
}

export interface PluginEntry {
  id: string
  version: string
  package: string
  sha256: string
  bytes: number
  hew: number
  signature: string
  key: string
}

/** A parsed, validated `hew.json` (6.1). */
export interface SourceIndex {
  hew: number
  name: string
  description: string | null
  author: string | null
  homepage: string | null
  license: string | null
  licenseUrl: string | null
  items: IndexItem[]
  plugins: PluginEntry[]
}

/** The requests for one source: resolve (null for `web`), index, one per path. */
export interface SourceUrls {
  resolve: string | null
  index: string
  files: string[]
}

export type SourceWasmErrorCode =
  | 'index_too_new'
  | 'index_too_large'
  | 'index_invalid'
  | 'ref_empty'
  | 'ref_query_refused'
  | 'ref_invalid'
  | 'path_invalid'
  | 'commit_invalid'
  | 'bad_input'

/** A refusal from a source export. `item` is the 0-based position for `index_invalid`. */
export class SourceWasmError extends Error {
  readonly code: SourceWasmErrorCode
  readonly reason: string
  readonly item?: number
  constructor(code: SourceWasmErrorCode, reason: string, item?: number) {
    super(`${code}: ${reason}`)
    this.name = 'SourceWasmError'
    this.code = code
    this.reason = reason
    this.item = item
  }
}

interface SourceWasmFailure {
  error: { code: SourceWasmErrorCode; reason: string; item?: number }
}

/** Throws `SourceWasmError` when a source export answered with `error`. */
function unwrap<T>(answer: string): T {
  const parsed = JSON.parse(answer) as T | SourceWasmFailure
  if (parsed !== null && typeof parsed === 'object' && 'error' in parsed) {
    const { code, reason, item } = parsed.error
    throw new SourceWasmError(code, reason, item)
  }
  return parsed as T
}

/** Parses and validates the text of a `hew.json`. */
export function parseIndex(json: string): SourceIndex {
  return unwrap<SourceIndex>(source_parse_index(json))
}

/** Parses a pasted reference into candidates, in probe order. */
export function parseSourceRef(text: string): SourceRef[] {
  return unwrap<SourceRef[]>(source_parse_ref(text))
}

/** The resolve, index and per-path file URLs of one candidate. */
export function sourceUrls(ref: SourceRef, paths: string[], commit?: string): SourceUrls {
  return unwrap<SourceUrls>(
    source_urls(JSON.stringify(ref), JSON.stringify(paths), commit ?? undefined),
  )
}

/**
 * The commit a resolve response names (`source_commit`), or null when the
 * body names none or is not a plain 40 or 64 hex commit. Only an unknown
 * `kind` throws (`bad_input`).
 */
export function commitFromResponse(kind: SourceKind, body: string): string | null {
  return unwrap<{ commit: string | null }>(source_commit(kind, body)).commit
}

/**
 * Generates `crates/api/src/refusal_copy.gen.rs` from this module's own
 * `DESCRIPTIONS` table (`./kernelErrors.ts` — the authoritative CODE →
 * user-copy table) and keeps it in sync.
 *
 * Mirrors the kernel's `REGENERATE_GOLDEN` pattern
 * (`crates/kernel/tests/golden_file.rs`): a plain `pnpm --dir app test` run
 * only ASSERTS the committed `.gen.rs` matches a fresh generation (drift
 * fails the build, pointing at the regen command below); setting
 * `REGENERATE_REFUSAL_COPY=1` WRITES the file instead.
 *
 * Regenerate with:
 *
 *   REGENERATE_REFUSAL_COPY=1 pnpm --dir app exec vitest run src/kernelErrorsDump.test.ts
 *
 * A plain node-executable generator script was considered instead (Node's
 * native TypeScript type-stripping runs `kernelErrors.ts` directly with no
 * flag on the Node version this repo develops against), but CI's runner
 * pins no Node version and the rest of the suite already goes through
 * Vitest's esbuild-backed TS transform — a test file has no dependency on
 * which Node happens to be on the runner's PATH.
 */

import { readFileSync, writeFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { spawnSync } from 'node:child_process'
import { describe, it, expect } from 'vitest'
import { describedErrorCodes, kernelErrorMessage } from './kernelErrors'

const REPO_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '../..')
const GEN_PATH = resolve(REPO_ROOT, 'crates/api/src/refusal_copy.gen.rs')
const REGEN_COMMAND =
  'REGENERATE_REFUSAL_COPY=1 pnpm --dir app exec vitest run src/kernelErrorsDump.test.ts'

/** Escapes a JS string as the body of a Rust double-quoted string literal. */
function rustEscape(s: string): string {
  return s
    .replace(/\\/g, '\\\\')
    .replace(/"/g, '\\"')
    .replace(/\r/g, '\\r')
    .replace(/\n/g, '\\n')
    .replace(/\t/g, '\\t')
}

/**
 * Renders `refusal_copy.gen.rs`: one match arm per code in
 * `describedErrorCodes()`, each mapped through `kernelErrorMessage` to its
 * FINAL string — helper-composed entries (`stale(...)`, the `${checkSolid}`
 * / `${internalRequest}` template interpolations, the `+`-concatenated
 * `DegenerateAnnotation` entry) are already resolved to plain strings by the
 * time `DESCRIPTIONS` is built, so there is nothing left to re-derive here.
 */
function generate(): string {
  const lines: string[] = []
  lines.push('// GENERATED from app/src/kernelErrors.ts — do not edit; regenerate with:')
  lines.push(`//   ${REGEN_COMMAND}`)
  lines.push('')
  lines.push("/// UI copy for a kernel error CODE, or `None` if the UI table (app/src/")
  lines.push('/// kernelErrors.ts) has none — the caller falls back to the kernel\'s own')
  lines.push('/// `Display` text for those.')
  lines.push('pub fn ui_copy(code: &str) -> Option<&\'static str> {')
  lines.push('    Some(match code {')
  for (const code of describedErrorCodes()) {
    const text = kernelErrorMessage(code, '')
    lines.push(`        "${code}" => "${rustEscape(text)}",`)
  }
  lines.push('        _ => return None,')
  lines.push('    })')
  lines.push('}')
  lines.push('')
  return lines.join('\n')
}

/**
 * Runs `rustfmt` over generated source so this test's notion of "fresh"
 * matches what actually ends up committed (the repo's `fmt` gate reformats
 * the long match-arm bodies rustfmt doesn't like on one line) — otherwise
 * this test and `cargo fmt --check` would fight over the file's formatting
 * on every regeneration.
 */
function rustfmt(source: string): string {
  const result = spawnSync('rustfmt', ['--edition', '2024', '--emit', 'stdout'], {
    input: source,
    encoding: 'utf8',
  })
  if (result.status !== 0) {
    throw new Error(`rustfmt failed (status ${result.status}):\n${result.stderr}`)
  }
  return result.stdout
}

describe('refusal_copy.gen.rs', () => {
  it('stays in sync with app/src/kernelErrors.ts', () => {
    const fresh = rustfmt(generate())
    if (process.env.REGENERATE_REFUSAL_COPY) {
      writeFileSync(GEN_PATH, fresh)
      return
    }
    let committed: string
    try {
      committed = readFileSync(GEN_PATH, 'utf8')
    } catch {
      throw new Error(
        `crates/api/src/refusal_copy.gen.rs is missing — generate it with: ${REGEN_COMMAND}`,
      )
    }
    expect(
      committed,
      `crates/api/src/refusal_copy.gen.rs is out of date with app/src/kernelErrors.ts — regenerate with: ${REGEN_COMMAND}`,
    ).toBe(fresh)
  })

  it('covers a large, plausible-sized inventory (guards against the scrape silently regressing)', () => {
    // Mirrors kernelErrors.test.ts's own floor on DERIVED_KERNEL_ERROR_CODES:
    // a renamed export or an emptied DESCRIPTIONS table should fail loudly
    // here, not generate a near-empty match statement that still compiles.
    expect(describedErrorCodes().length).toBeGreaterThan(90)
  })
})

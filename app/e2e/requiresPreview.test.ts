/**
 * Guard for the browser suite's two-way run (see e2e/helpers/preview.ts): a
 * spec that names a Preview feature's tool, command or API call must declare
 * `requiresPreview(...)`, or the production-shape run would try to drive a
 * feature that build withholds. Needles come from `preview-features.json`, so
 * a feature added later is covered without touching this file.
 *
 * (The production-shape run itself is the real check: a spec that drives a
 * withheld feature some way this scan cannot see fails there.)
 */

import { readFileSync, readdirSync, statSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { describe, expect, it } from 'vitest'
import { previewFeatures } from './helpers/preview'

const E2E = dirname(fileURLToPath(import.meta.url))

function specs(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const p = join(dir, name)
    if (statSync(p).isDirectory()) return name === 'fixtures' || name === 'plugin-realm' ? [] : specs(p)
    return name.endsWith('.spec.ts') ? [p] : []
  })
}

const features = previewFeatures()

/** What names a feature in a spec: its tools as quoted strings, its command ids, its API names, its name. */
function needlesOf(f: (typeof features)[number]): string[] {
  return [...f.tools.flatMap((t) => [`'${t}'`, `"${t}"`]), ...f.actions, ...f.apiCommands, ...(f.name.includes(' ') ? [f.name] : [])]
}

describe('browser specs that drive a Preview feature', () => {
  it('declare requiresPreview(...) naming a feature they use', () => {
    const missing: string[] = []
    for (const file of specs(E2E)) {
      const text = readFileSync(file, 'utf8')
      const used = features.filter((f) => needlesOf(f).some((n) => text.includes(n)))
      if (used.length === 0) continue
      if (file.endsWith('preview-wall.spec.ts')) continue
      const declared = [...text.matchAll(/requiresPreview\(([^)]*)\)/g)].flatMap((m) =>
        [...m[1].matchAll(/'([^']+)'/g)].map((x) => x[1]),
      )
      if (!used.some((f) => declared.includes(f.id))) {
        missing.push(`${file.slice(E2E.length + 1)}: uses ${used.map((f) => f.id).join(', ')}, declares ${declared.join(', ') || 'nothing'}`)
      }
    }
    expect(missing).toEqual([])
  })

  it('every requiresPreview id is a feature in the registry', () => {
    const known = new Set(features.map((f) => f.id))
    const bad: string[] = []
    for (const file of specs(E2E)) {
      for (const m of readFileSync(file, 'utf8').matchAll(/requiresPreview\(([^)]*)\)/g)) {
        for (const x of m[1].matchAll(/'([^']+)'/g)) if (!known.has(x[1])) bad.push(`${file.slice(E2E.length + 1)}: ${x[1]}`)
      }
    }
    expect(bad).toEqual([])
  })
})

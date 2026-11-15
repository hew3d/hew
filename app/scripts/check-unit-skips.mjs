#!/usr/bin/env node
// The unit suite's skip rule, for both builds (docs/dev/DEVELOPMENT.md
// "Preview features"). Reads a vitest JSON report
// (`vitest run --reporter=json --outputFile=<file>`) and fails when:
//   - a test was skipped without the Preview wall's mark (`requiresPreview` in
//     src/test/requiresPreview.ts records `meta.previewWall`, a reason that
//     starts "preview-wall:"), which would be a test that asserts nothing;
//   - the preview run skipped anything at all;
//   - the production run skipped as many tests as it ran (a run that silently
//     skips everything);
//   - any test failed, or nothing ran.
// usage: node scripts/check-unit-skips.mjs <report.json> <production|preview>

import { readFileSync } from 'node:fs'

const [reportPath, build] = process.argv.slice(2)
if (!reportPath || !['production', 'preview'].includes(build)) {
  console.error('usage: node scripts/check-unit-skips.mjs <report.json> <production|preview>')
  process.exit(2)
}
const report = JSON.parse(readFileSync(reportPath, 'utf8'))
const problems = []
let passed = 0
let failed = 0
let skipped = 0
let marked = 0
const byFile = new Map()
for (const file of report.testResults ?? []) {
  for (const t of file.assertionResults ?? []) {
    if (t.status === 'passed') passed += 1
    else if (t.status === 'failed') {
      failed += 1
      problems.push(`failed: ${file.name} > ${t.fullName}`)
    } else {
      skipped += 1
      const reason = t.meta?.previewWall
      if (typeof reason === 'string' && reason.startsWith('preview-wall:')) {
        marked += 1
        byFile.set(file.name, (byFile.get(file.name) ?? 0) + 1)
      } else problems.push(`skipped without the Preview wall's reason: ${file.name} > ${t.fullName}`)
    }
  }
  if (file.status === 'failed' && (file.assertionResults ?? []).length === 0) problems.push(`file failed to run: ${file.name} ${file.message ?? ''}`)
}
if (passed + failed === 0) problems.push('no test ran')
if (build === 'preview' && skipped > 0) problems.push(`the preview run skipped ${skipped} test(s)`)
if (build === 'production' && skipped >= passed) problems.push(`the production run skipped ${skipped} test(s) and ran ${passed}: most of the suite was skipped`)
if (problems.length > 0) {
  console.error(`check-unit-skips (${build}): FAILED`)
  for (const p of problems) console.error(`  ${p}`)
  process.exit(1)
}
console.log(`check-unit-skips (${build}): ${passed} passed, ${marked} skipped by the Preview wall, ${failed} failed`)
for (const [f, n] of [...byFile].sort()) console.log(`  ${n}  ${f.replace(/^.*\/app\//, '')}`)

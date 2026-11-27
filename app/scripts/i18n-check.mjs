// Coverage report for shipped catalogs (docs/LOCALIZATION.md).
//
// A shipped locale may lag behind the source: new and reworded messages
// arrive untranslated and show in English until a translator catches up,
// so an untranslated entry never fails the gate. This prints how much of
// each shipped catalog is translated, which is what a release looks at
// before asking translators for an update. The source locale is exempt
// (its msgstr is its msgid by definition).
//
//   node scripts/i18n-check.mjs          one coverage line per locale
//   node scripts/i18n-check.mjs --list   also every untranslated message
//
// Run from app/. A shipped locale whose catalog cannot be read or holds
// no messages still fails: that is a broken checkout, not a lagging one.
// The logic lives in po.mjs (`coverageReport`) so a test can drive it.

import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { dirname, resolve } from 'node:path'
import { SHIPPED_LOCALES, SOURCE_LOCALE } from '../src/i18n/locales.ts'
import { coverageReport } from './po.mjs'

const args = process.argv.slice(2)
const unknown = args.filter((a) => a !== '--list')
if (unknown.length > 0) {
  console.error(`i18n-check: unknown argument ${unknown.join(' ')} (the only flag is --list)`)
  process.exit(2)
}

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const report = coverageReport({
  locales: SHIPPED_LOCALES.filter((l) => l !== SOURCE_LOCALE),
  read: (locale) => readFileSync(resolve(root, 'src/locales', locale, 'messages.po'), 'utf8'),
  list: args.includes('--list'),
})
for (const line of report.lines) (line.error ? console.error : console.log)(line.text)
if (report.failed) process.exit(1)

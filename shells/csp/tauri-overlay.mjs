// Writes a Tauri `--config` overlay that replaces the desktop shell's content
// security policy with a named one from policies.json:
//
//   node shells/csp/tauri-overlay.mjs <preview|probe> <out-file>
//
// tauri.conf.json itself always holds the production policy, so a build
// that passes no overlay is a production build's. `production` is refused
// here for that reason: there is nothing to overlay.

import { mkdirSync, writeFileSync } from 'node:fs'
import { dirname } from 'node:path'
import { policy } from './policy.mjs'

const [name, out] = process.argv.slice(2)
if (!['preview', 'probe'].includes(name) || !out) {
  console.error('usage: node shells/csp/tauri-overlay.mjs <preview|probe> <out-file>')
  process.exit(2)
}
mkdirSync(dirname(out), { recursive: true })
writeFileSync(out, `${JSON.stringify({ app: { security: { csp: policy('desktop', name) } } }, null, 2)}\n`)
console.log(`tauri-overlay: wrote the desktop.${name} policy to ${out}`)

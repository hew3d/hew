#!/usr/bin/env node
// Whether a built desktop binary carries the content security policy its
// flavor needs, checked on the file the bundle ships.
//
//   node scripts/check-desktop-binary.mjs --want <preview|production> [--target <dir>]
//
// A preview desktop build serves the desktop.preview policy, which allows
// the plugin realm's boot script by its hash; a production build must not
// allow it. A build that was meant to be one and got the other (the policy
// overlay dropped or mangled on its way to the Tauri CLI) would otherwise
// leave the realm blocked, or the production wall open, with every other
// check green. The policy is configuration the Tauri build compiles into
// the binary, so the allowed-source text `'sha256-<hash>'` is looked for in
// the release binaries under --target (default shells/tauri/src-tauri/target).
//
// Fails when no binary is found: a scan of nothing proves nothing.

import { existsSync, lstatSync, readFileSync, readdirSync } from 'node:fs'
import { dirname, join, relative, resolve, sep } from 'node:path'
import { fileURLToPath } from 'node:url'

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const args = process.argv.slice(2)
const value = (name) => {
  const i = args.indexOf(name)
  return i === -1 ? undefined : args[i + 1]
}
const want = value('--want')
const target = resolve(value('--target') ?? join(ROOT, 'shells/tauri/src-tauri/target'))
if (want !== 'preview' && want !== 'production') {
  console.error('usage: node scripts/check-desktop-binary.mjs --want <preview|production> [--target <dir>]')
  process.exit(2)
}
if (!existsSync(target)) {
  console.error(`check-desktop-binary: ${target} does not exist`)
  process.exit(2)
}

const hash = readFileSync(join(ROOT, 'app/src/plugins/boot/boot-1.hash'), 'utf8').trim()
const allowed = `'sha256-${hash}'`

// The shell's own executable: `hew` (`hew.exe`) in a `release` folder, at
// any depth (a --target subfolder, or the macOS universal output).
const binaries = []
const walk = (dir) => {
  for (const name of readdirSync(dir)) {
    const path = join(dir, name)
    // Not followed: a link (dangling ones included) is not the build's own
    // output, and following one could loop.
    const info = lstatSync(path)
    if (info.isSymbolicLink()) continue
    if (info.isDirectory()) walk(path)
    else if ((name === 'hew' || name === 'hew.exe') && info.size > 1_000_000 && path.split(sep).includes('release')) binaries.push(path)
  }
}
walk(target)

if (binaries.length === 0) {
  console.error(`check-desktop-binary: no hew binary under ${relative(ROOT, target)}; nothing was checked`)
  process.exit(1)
}
let bad = 0
for (const file of binaries) {
  const has = readFileSync(file).includes(Buffer.from(allowed, 'utf8'))
  const ok = has === (want === 'preview')
  console.log(`check-desktop-binary: ${relative(ROOT, file)}: ${has ? 'allows' : 'does not allow'} the realm boot script (${want} build: ${ok ? 'ok' : 'WRONG'})`)
  if (!ok) bad++
}
process.exit(bad === 0 ? 0 : 1)

#!/usr/bin/env node
// Runs the Tauri CLI with the build flavor and the content security policy
// the app it builds needs:
//
//   node scripts/tauri-policy.mjs [--print-args] <dev|build> [tauri args...]
//
// One rule decides the flavor, for the web side and the Rust shell alike
// (app/vite.config.ts, app/scripts/build-wasm.mjs, build.rs):
// HEW_PREVIEW=1 is a preview build and 0 a production one; unset or empty,
// `tauri dev` (which runs `vite dev`) is a preview build and `tauri build`
// a production one. Anything else is an error. The wrapper resolves the
// rule once and exports the answer as HEW_PREVIEW (`1` or `0`) to the Tauri
// CLI, so the Rust shell (cfg(hew_preview)), the frontend build and the
// overlay below always agree, whatever the caller's environment held.
//
// tauri.conf.json always carries the production desktop policy. For a
// preview build, a `--config` overlay with the desktop.preview policy
// (shells/csp/tauri-overlay.mjs preview) is written under src-tauri/target/
// and passed after the caller's arguments; the Tauri CLI merges repeated
// `--config` values in the order given, so the overlay's policy wins and
// the rest of the configuration stands.
//
// The package scripts of this directory are the supported way to build:
// the Tauri CLI run directly applies no overlay and no flavor.
//
// `--print-args` writes what would run as one line of JSON and exits
// without writing or running anything. It is what the tests read.

import { spawnSync } from 'node:child_process'
import { readFileSync } from 'node:fs'
import { createRequire } from 'node:module'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { previewFlag } from '../../csp/policy.mjs'

const HERE = dirname(fileURLToPath(import.meta.url))
const SHELL = resolve(HERE, '..')
const OVERLAY_WRITER = join(SHELL, '..', 'csp', 'tauri-overlay.mjs')
export const OVERLAY = join(SHELL, 'src-tauri', 'target', 'csp-preview.conf.json')

/** What a run of `command` with `env` does: whether it is a preview build,
 *  the HEW_PREVIEW value the Tauri CLI gets, and the arguments after the
 *  CLI's own entry point. Throws on an unusable HEW_PREVIEW. */
export function plan(command, rest, env) {
  const preview = previewFlag(env.HEW_PREVIEW, command === 'dev')
  const tauriArgs = [command, ...rest]
  // The overlay is the CLI's own option: before a `--`, after which the
  // arguments belong to the program the CLI runs (cargo, or the app).
  if (preview) {
    const end = tauriArgs.indexOf('--')
    tauriArgs.splice(end < 0 ? tauriArgs.length : end, 0, '--config', OVERLAY)
  }
  return { command, preview, hewPreview: preview ? '1' : '0', overlay: preview ? OVERLAY : null, tauriArgs }
}

/** The Tauri CLI's JavaScript entry point, so that it runs under this very
 *  Node with an argument array on every platform (a `.cmd` shim needs a
 *  shell, and a shell splits a path that holds a space). */
function tauriEntry() {
  const require = createRequire(join(SHELL, 'package.json'))
  const manifest = require.resolve('@tauri-apps/cli/package.json')
  const bin = JSON.parse(readFileSync(manifest, 'utf8')).bin
  return join(dirname(manifest), typeof bin === 'string' ? bin : bin.tauri)
}

function main(argv) {
  const dry = argv[0] === '--print-args'
  const [command, ...rest] = dry ? argv.slice(1) : argv
  if (command !== 'dev' && command !== 'build') {
    console.error('usage: node scripts/tauri-policy.mjs [--print-args] <dev|build> [tauri args...]')
    process.exit(2)
  }
  let p
  try {
    p = plan(command, rest, process.env)
  } catch (e) {
    console.error(`tauri-policy: ${e.message}`)
    process.exit(2)
  }
  if (dry) {
    console.log(JSON.stringify({ ...p, overlayWriter: p.preview ? ['preview', OVERLAY] : null }))
    return
  }

  const run = (cmd, args, env) => {
    const r = spawnSync(cmd, args, { stdio: 'inherit', cwd: SHELL, shell: false, env })
    if (r.error) {
      console.error(`tauri-policy: ${cmd}: ${r.error.message}`)
      process.exit(1)
    }
    if (r.status !== 0) process.exit(r.status ?? 1)
  }
  if (p.preview) run(process.execPath, [OVERLAY_WRITER, 'preview', OVERLAY], process.env)
  run(process.execPath, [tauriEntry(), ...p.tauriArgs], { ...process.env, HEW_PREVIEW: p.hewPreview })
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) main(process.argv.slice(2))

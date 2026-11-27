#!/usr/bin/env node
// Builds the plugin realm's boot script: the SDK runtime (src/runtime.ts) as
// text, embedded in the frame's boot script (src/boot.ts), written to
//
//   app/src/plugins/boot/boot-1.js     the exact text between the frame's
//                                       <script> tags
//   app/src/plugins/boot/boot-1.hash   base64 SHA-256 of those bytes, the
//                                       value the content security policies pin
//
// Both are committed. The build is deterministic (one esbuild version from
// the lockfile, fixed options, no timestamps), so a rebuild of an unchanged
// tree gives the same bytes.
//
//   pnpm --dir sdk/plugin build           rewrite both files
//   pnpm --dir sdk/plugin build --check   exit 1 if either would change
//   pnpm --dir sdk/plugin build --print   print {"hash","bytes"} of a fresh build
//
// After a change, `--check` fails until the files are rewritten and the
// hash is pasted where shells/csp/policies.json (web.preview,
// desktop.preview) and preview-features.json (the plugins entry's markers)
// carry it; app/src/plugins/boot/bootArtifacts.test.ts holds all of them
// to each other.

import { createHash } from 'node:crypto'
import { existsSync, readFileSync, writeFileSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { build } from 'esbuild'

const HERE = dirname(fileURLToPath(import.meta.url))
const OUT_DIR = resolve(HERE, '../../app/src/plugins/boot')
const OUT_JS = join(OUT_DIR, 'boot-1.js')
const OUT_HASH = join(OUT_DIR, 'boot-1.hash')

const common = {
  absWorkingDir: HERE,
  bundle: true,
  write: false,
  format: 'iife',
  target: 'es2022',
  platform: 'browser',
  minify: false,
  charset: 'ascii',
  legalComments: 'none',
  sourcemap: false,
  logLevel: 'silent',
}

async function text(options) {
  const result = await build({ ...common, ...options })
  if (result.errors.length > 0) throw new Error(result.errors.map((e) => e.text).join('\n'))
  return result.outputFiles[0].text
}

/** The boot script's text and its hash, built in memory. */
export async function buildBoot() {
  const runtime = await text({ entryPoints: ['src/runtime.ts'] })
  const boot = await text({
    entryPoints: ['src/boot.ts'],
    define: { __HEW_RUNTIME__: JSON.stringify(runtime) },
  })
  // The script sits inside a srcdoc: the HTML parser would end it at the
  // first "</script", treat "<!--" specially, and normalize CR, which would
  // change the bytes the hash covers.
  if (/<\/script|<!--/i.test(boot)) throw new Error('the boot script contains "</script" or "<!--"')
  if (boot.includes('\r') || /[^\x09\x0a\x20-\x7e]/.test(boot)) {
    throw new Error('the boot script is not plain printable ASCII with LF line ends')
  }
  const hash = createHash('sha256').update(Buffer.from(boot, 'utf8')).digest('base64')
  return { boot, hash }
}

async function main() {
  const args = process.argv.slice(2)
  const { boot, hash } = await buildBoot()
  if (args.includes('--print')) {
    console.log(JSON.stringify({ hash, bytes: Buffer.byteLength(boot) }))
    return
  }
  if (args.includes('--check')) {
    const same =
      existsSync(OUT_JS) &&
      existsSync(OUT_HASH) &&
      readFileSync(OUT_JS, 'utf8') === boot &&
      readFileSync(OUT_HASH, 'utf8') === `${hash}\n`
    if (!same) {
      console.error('sdk/plugin: the committed boot script is stale; run `pnpm --dir sdk/plugin build` and paste the new hash (see build.mjs)')
      process.exit(1)
    }
    console.log(`sdk/plugin: boot-1.js is current (${hash})`)
    return
  }
  writeFileSync(OUT_JS, boot)
  writeFileSync(OUT_HASH, `${hash}\n`)
  console.log(`sdk/plugin: wrote boot-1.js (${Buffer.byteLength(boot)} bytes), sha256-${hash}`)
}

if (process.argv[1] !== undefined && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch((e) => {
    console.error(`sdk/plugin: ${e instanceof Error ? e.message : String(e)}`)
    process.exit(2)
  })
}

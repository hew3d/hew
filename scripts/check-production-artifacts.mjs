#!/usr/bin/env node
// What a production build must not contain, checked on the built artifacts
// (docs/dev/DEVELOPMENT.md "Preview features", "Code loading"). The Preview
// wall's tests prove the gates; this proves the result, on the files a
// regular user is served.
//
//   node scripts/check-production-artifacts.mjs [--dist <dir>] [--dts <file>] [--web-dist <dir>]
//
//   --dist   a production app build. Default app/dist, as `pnpm --dir app
//            build` leaves it with HEW_PREVIEW unset.
//   --dts    the production kernel module's generated types. Default
//            target/wasm-flavors/production/web/wasm_api.d.ts.
//   --web-dist  a built web shell (shells/web/dist), when there is one: its
//            page must carry the production policy. The release workflow
//            passes it for a production tag.
//
// Checks, each of which fails the run:
//
//   policy        Both shells' production content security policies equal
//                 their committed baselines; tauri.conf.json carries the
//                 desktop one and no platform overlay replaces it; the web
//                 injector puts the web one into the built page; no header
//                 file sets a cross-origin isolation header.
//   capabilities  The desktop shell's capability files equal their
//                 committed baselines, file for file.
//   bundle        No file of the app build holds a `loader.markers` string
//                 or a Preview feature's state line from
//                 preview-features.json. The kernel's .wasm is left out: it
//                 embeds the registry's text, which is why exports are
//                 checked by name instead.
//   exports       The production kernel module exports no
//                 `loader.kernelExports` name.
//
// A scan that looked at nothing is a failure too: every check has a
// positive control.

import { existsSync, readFileSync, readdirSync, statSync } from 'node:fs'
import { dirname, join, relative, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { cspMetaOf, policy, policyForBuild, withCspMeta } from '../shells/csp/policy.mjs'

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const at = (...p) => join(ROOT, ...p)

const args = process.argv.slice(2)
const OPTIONS = ['--dist', '--dts', '--web-dist']
function option(name, fallback) {
  const i = args.indexOf(name)
  if (i === -1) return fallback
  if (!args[i + 1] || args[i + 1].startsWith('--')) fail(`${name} needs a value`)
  return resolve(args[i + 1])
}

const problems = []
const fail = (message) => {
  console.error(`check-production-artifacts: ${message}`)
  process.exit(2)
}
const problem = (section, message) => problems.push(`[${section}] ${message}`)
const read = (path) => readFileSync(path, 'utf8')

for (let i = 0; i < args.length; i += 2) {
  if (!OPTIONS.includes(args[i])) fail(`unknown option ${args[i]}; known: ${OPTIONS.join(', ')}`)
}
const dist = option('--dist', at('app/dist'))
const dts = option('--dts', at('target/wasm-flavors/production/web/wasm_api.d.ts'))

const registry = JSON.parse(read(at('preview-features.json'))).features
const loaders = registry.filter((f) => f.loader)

// ---------------------------------------------------------------- policy
{
  const baseline = (shell) => read(at(`shells/csp/baseline/${shell}.production.csp`)).replace(/\n$/, '')
  for (const shell of ['web', 'desktop']) {
    const current = policy(shell, 'production')
    if (current !== baseline(shell)) {
      problem('policy', `${shell}.production in shells/csp/policies.json differs from shells/csp/baseline/${shell}.production.csp. A production policy change edits both, on purpose.`)
    }
    const script = current.split('; ').find((d) => d.startsWith('script-src ')) ?? ''
    for (const source of ['blob:', "'unsafe-eval'", "'unsafe-inline'", "'sha256-", 'data:', 'https:', 'http:']) {
      if (script.includes(source)) problem('policy', `${shell}.production script-src allows ${source}`)
    }
    if (!script.includes("'self'")) problem('policy', `${shell}.production has no script-src 'self' (the check read nothing)`)
  }

  const conf = JSON.parse(read(at('shells/tauri/src-tauri/tauri.conf.json')))
  if (conf.app?.security?.csp !== policy('desktop', 'production')) {
    problem('policy', 'shells/tauri/src-tauri/tauri.conf.json does not carry desktop.production. That file is what a build with no overlay ships.')
  }
  // Everything else under app.security widens what a page may do (inline
  // capabilities, remote access to the command bridge, the asset protocol),
  // so the object holds the policy and nothing more.
  const securityKeys = Object.keys(conf.app?.security ?? {})
  if (JSON.stringify(securityKeys) !== JSON.stringify(['csp'])) {
    problem('policy', `tauri.conf.json app.security holds ${securityKeys.join(', ') || 'nothing'}; the pinned shape is csp alone. Extend this check with the new key's baseline before adding one.`)
  }
  // A preview desktop build is given no overlay yet (the release workflow
  // and `tauri dev` both read tauri.conf.json), so a wider desktop.preview
  // would be served by nothing. Wire shells/csp/tauri-overlay.mjs into
  // every preview desktop build, then replace this check with one of that
  // wiring.
  if (policy('desktop', 'preview') !== policy('desktop', 'production')) {
    problem('policy', 'desktop.preview differs from desktop.production, but no preview desktop build applies it yet (see the comment at this check)')
  }
  for (const name of readdirSync(at('shells/tauri/src-tauri'))) {
    if (!/^tauri\..+\.conf\.json$/.test(name)) continue
    if (JSON.parse(read(at('shells/tauri/src-tauri', name))).app?.security !== undefined) {
      problem('policy', `shells/tauri/src-tauri/${name} sets app.security; only tauri.conf.json and a generated overlay may`)
    }
  }

  // The injector's own output for a production environment.
  const page = '<!doctype html><html><head>\n    <meta charset="UTF-8" />\n  </head><body></body></html>'
  for (const env of [{}, { HEW_PREVIEW: '' }, { HEW_PREVIEW: '0' }]) {
    if (cspMetaOf(withCspMeta(page, policyForBuild('web', env))) !== policy('web', 'production')) {
      problem('policy', `the web injector does not serve web.production for HEW_PREVIEW=${JSON.stringify(env.HEW_PREVIEW)}`)
    }
  }
  if (cspMetaOf(withCspMeta(page, policyForBuild('web', { HEW_PREVIEW: '1' }))) !== policy('web', 'preview')) {
    problem('policy', 'the web injector does not serve web.preview for HEW_PREVIEW=1')
  }

  const webDist = option('--web-dist', null)
  if (webDist !== null) {
    const index = join(webDist, 'index.html')
    if (!existsSync(index)) fail(`${relative(ROOT, webDist)} holds no built web shell`)
    if (cspMetaOf(read(index)) !== policy('web', 'production')) {
      problem('policy', `${relative(ROOT, index)} does not carry web.production`)
    }
  }

  const headerFiles = ['shells/web/_headers', 'shells/web/_headers.beta', 'shells/web/deploy/nginx.conf', 'shells/web/404.html', 'app/index.html']
  let headersRead = 0
  for (const file of headerFiles) {
    if (!existsSync(at(file))) {
      problem('policy', `${file} is missing; update the list in scripts/check-production-artifacts.mjs`)
      continue
    }
    headersRead++
    if (/cross-origin-(opener|embedder)-policy/i.test(read(at(file)))) problem('policy', `${file} sets a cross-origin isolation header`)
  }
  for (const dir of ['shells/web/deploy/hew.d']) {
    for (const name of existsSync(at(dir)) ? readdirSync(at(dir)) : []) {
      if (/cross-origin-(opener|embedder)-policy/i.test(read(at(dir, name)))) problem('policy', `${dir}/${name} sets a cross-origin isolation header`)
    }
  }
  if (headersRead === 0) problem('policy', 'no header file was read')
  if (/content-security-policy/i.test(read(at('app/index.html')))) {
    problem('policy', 'app/index.html carries a policy of its own; the shells supply it (shells/web/inject-csp.mjs, tauri.conf.json)')
  }
}

// ---------------------------------------------------------- capabilities
{
  const live = at('shells/tauri/src-tauri/capabilities')
  const pinned = at('shells/csp/baseline/capabilities')
  const names = (dir) => readdirSync(dir).sort()
  if (JSON.stringify(names(live)) !== JSON.stringify(names(pinned))) {
    problem('capabilities', `the capability files (${names(live).join(', ')}) are not the baseline's (${names(pinned).join(', ')}). A capability file for a Preview feature is generated for a preview build, never committed beside default.json.`)
  }
  for (const name of names(pinned)) {
    if (!existsSync(join(live, name))) continue
    const a = JSON.stringify(JSON.parse(read(join(live, name))))
    const b = JSON.stringify(JSON.parse(read(join(pinned, name))))
    if (a !== b) problem('capabilities', `capabilities/${name} differs from shells/csp/baseline/capabilities/${name}. A capability change edits both, on purpose.`)
  }
  if (names(pinned).length === 0) problem('capabilities', 'the baseline is empty')
}

// ---------------------------------------------------------------- bundle
{
  const markers = [...new Set(loaders.flatMap((f) => f.loader.markers))]
  const states = registry.map((f) => f.state)
  if (markers.length === 0) problem('bundle', 'no loader marker in the registry: nothing to scan for')
  if (!existsSync(join(dist, 'index.html'))) fail(`${relative(ROOT, dist)} holds no app build; run \`pnpm --dir app build\` (HEW_PREVIEW unset) first`)
  const files = []
  const walk = (dir) => {
    for (const name of readdirSync(dir)) {
      const path = join(dir, name)
      if (statSync(path).isDirectory()) walk(path)
      else if (!name.endsWith('.wasm')) files.push(path)
    }
  }
  walk(dist)
  let scripts = 0
  let control = false
  for (const file of files) {
    const text = readFileSync(file, 'latin1')
    if (file.endsWith('.js')) scripts++
    // Positive control: a string every app build holds, so a scan of the
    // wrong directory, or of nothing, cannot pass.
    if (text.includes('hew.settings.')) control = true
    for (const needle of markers) {
      if (text.includes(needle)) problem('bundle', `${relative(ROOT, file)} holds the loader marker "${needle}"`)
    }
    for (const needle of states) {
      if (text.includes(needle)) problem('bundle', `${relative(ROOT, file)} holds a Preview feature's state line ("${needle.slice(0, 40)}...")`)
    }
  }
  if (scripts === 0 || !control) problem('bundle', `${relative(ROOT, dist)} does not look like an app build (${scripts} scripts, control string ${control ? 'found' : 'missing'})`)
  console.log(`check-production-artifacts: bundle: ${files.length} files, ${markers.length} markers, ${states.length} state lines`)
}

// --------------------------------------------------------------- exports
{
  const names = [...new Set(loaders.flatMap((f) => f.loader.kernelExports))]
  if (!existsSync(dts)) fail(`${relative(ROOT, dts)} is missing; build the production kernel first (scripts/wasm-flavors.sh build production web)`)
  const text = read(dts)
  const exported = (name) => new RegExp(`\\b${name}\\s*\\(`).test(text)
  if (!exported('preview_build')) problem('exports', `${relative(ROOT, dts)} does not declare preview_build (the check read nothing)`)
  const stamp = join(dirname(dts), 'preview-build')
  if (existsSync(stamp) && read(stamp).trim() !== '0') problem('exports', `${relative(ROOT, dirname(dts))} is stamped as a preview module`)
  for (const name of names) {
    if (exported(name)) problem('exports', `the production kernel module exports ${name}`)
  }
  // Every name is real: the preview module, when it is built, has each one.
  const previewDts = at('target/wasm-flavors/preview/web/wasm_api.d.ts')
  if (existsSync(previewDts)) {
    const preview = read(previewDts)
    for (const name of names) {
      if (!new RegExp(`\\b${name}\\s*\\(`).test(preview)) problem('exports', `loader.kernelExports names ${name}, which the preview kernel module does not export`)
    }
  }
  console.log(`check-production-artifacts: exports: ${names.length} gated names`)
}

if (problems.length > 0) {
  console.error('check-production-artifacts: FAILED')
  for (const p of problems) console.error(`  ${p}`)
  process.exit(1)
}
console.log('check-production-artifacts: all clear')

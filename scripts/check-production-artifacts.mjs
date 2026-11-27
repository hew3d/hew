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
//            page must carry the production policy or the strict one
//            (HEW_WEB_SOURCES=off). The release workflow passes it for a
//            production tag.
//
// Checks, each of which fails the run:
//
//   policy        Both shells' production content security policies equal
//                 their committed baselines; tauri.conf.json carries the
//                 desktop one and no platform overlay replaces it; only a
//                 preview desktop build is given the desktop.preview
//                 overlay (the shell's scripts, the release workflow); the web
//                 injector puts the web one into the built page; no header
//                 file sets a cross-origin isolation header.
//   capabilities  The desktop shell's capability files equal their
//                 committed baselines, file for file.
//   markers       Every loader marker and shell command of the registry
//                 occurs in preview-only code (one listed exception keeps
//                 the older rule: anywhere in the code) — the code it marks (a marker that marks nothing
//                 would make the bundle scan vacuous). Needs no build.
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

import { spawnSync } from 'node:child_process'
import { existsSync, mkdtempSync, readFileSync, readdirSync, rmSync, statSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join, relative, resolve, sep } from 'node:path'
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
  const baseline = (name) => read(at(`shells/csp/baseline/${name}.csp`)).replace(/\n$/, '')
  // web.strict (HEW_WEB_SOURCES=off) is a second pinned web tier, against its own baseline.
  if (policy('web', 'strict') !== baseline('web.strict')) {
    problem('policy', 'web.strict in shells/csp/policies.json differs from shells/csp/baseline/web.strict.csp. A strict policy change edits both, on purpose.')
  }
  for (const shell of ['web', 'desktop']) {
    const current = policy(shell, 'production')
    if (current !== baseline(`${shell}.production`)) {
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
  // A preview desktop build serves desktop.preview through a `--config`
  // overlay that shells/csp/tauri-overlay.mjs writes: the shell's dev,
  // build and e2e:build scripts run the Tauri CLI only through
  // scripts/tauri-policy.mjs, which adds the overlay for a preview build,
  // and the release workflow writes and passes it for a -pre. tag alone.
  {
    const scripts = JSON.parse(read(at('shells/tauri/package.json'))).scripts ?? {}
    for (const [name, command] of [['dev', 'dev'], ['build', 'build'], ['e2e:build', 'build']]) {
      const line = scripts[name] ?? ''
      if (!line.includes(`node scripts/tauri-policy.mjs ${command}`)) {
        problem('policy', `shells/tauri's "${name}" script does not run the Tauri CLI through scripts/tauri-policy.mjs ${command}, so a preview build would serve the production policy`)
      }
      if (/(^|&&|\s)tauri (dev|build)\b/.test(line.replaceAll('node scripts/tauri-policy.mjs', ''))) {
        problem('policy', `shells/tauri's "${name}" script also runs the Tauri CLI directly`)
      }
    }
    // The wrapper is run, not read: its dry run prints the Tauri arguments
    // and the HEW_PREVIEW value it would hand the CLI. The overlay appears
    // exactly for a preview build, and only a preview build's overlay,
    // as the overlay writer produces it, carries the realm's boot hash.
    const wrapperPath = at('shells/tauri/scripts/tauri-policy.mjs')
    const dryRun = (command, hewPreview) => {
      const env = { ...process.env }
      delete env.HEW_PREVIEW
      if (hewPreview !== undefined) env.HEW_PREVIEW = hewPreview
      const r = spawnSync(process.execPath, [wrapperPath, '--print-args', command, '--debug'], { encoding: 'utf8', env })
      if (r.status !== 0) return { error: (r.stderr || r.error?.message || `exit ${r.status}`).trim() }
      try {
        return JSON.parse(r.stdout)
      } catch {
        return { error: `unreadable output: ${r.stdout.slice(0, 80)}` }
      }
    }
    // [HEW_PREVIEW, command, preview build?] (undefined is unset).
    const table = [
      [undefined, 'dev', true], ['', 'dev', true], ['1', 'dev', true], ['0', 'dev', false],
      [undefined, 'build', false], ['', 'build', false], ['1', 'build', true], ['0', 'build', false],
    ]
    let ran = 0
    for (const [value, command, expectPreview] of table) {
      const label = `tauri-policy.mjs ${command} with HEW_PREVIEW=${value === undefined ? '(unset)' : JSON.stringify(value)}`
      const got = dryRun(command, value)
      ran++
      if (got.error) {
        problem('policy', `${label} failed: ${got.error}`)
        continue
      }
      const configs = got.tauriArgs.flatMap((a, i) => (a === '--config' ? [got.tauriArgs[i + 1]] : []))
      if (expectPreview) {
        if (configs.length !== 1 || !String(configs[0]).endsWith('csp-preview.conf.json') || got.hewPreview !== '1') {
          problem('policy', `${label} must pass the desktop.preview overlay and export HEW_PREVIEW=1; it passed ${JSON.stringify(got.tauriArgs)} with HEW_PREVIEW=${got.hewPreview}`)
        }
      } else if (configs.length !== 0 || got.hewPreview !== '0' || got.overlay !== null) {
        problem('policy', `${label} must pass no overlay and export HEW_PREVIEW=0; it passed ${JSON.stringify(got.tauriArgs)} with HEW_PREVIEW=${got.hewPreview}`)
      }
    }
    for (const command of ['dev', 'build']) {
      ran++
      if (!dryRun(command, 'garbage').error) problem('policy', `tauri-policy.mjs ${command} accepts HEW_PREVIEW=garbage`)
    }
    if (ran === 0) problem('policy', 'no wrapper run was made')
    {
      const bootHash = read(at('app/src/plugins/boot/boot-1.hash')).trim()
      const dir = mkdtempSync(join(tmpdir(), 'hew-overlay-'))
      try {
        const out = join(dir, 'overlay.json')
        const w = spawnSync(process.execPath, [at('shells/csp/tauri-overlay.mjs'), 'preview', out], { encoding: 'utf8' })
        const csp = w.status === 0 ? JSON.parse(read(out)).app?.security?.csp ?? '' : ''
        if (!bootHash) problem('policy', 'app/src/plugins/boot/boot-1.hash is empty')
        else if (!csp.includes(`'sha256-${bootHash}'`)) problem('policy', `the desktop.preview overlay does not allow the realm boot script (boot-1.hash ${bootHash})`)
        if (csp !== policy('desktop', 'preview')) problem('policy', 'the overlay for a preview build is not desktop.preview')
      } finally {
        rmSync(dir, { recursive: true, force: true })
      }
      if (policy('desktop', 'production').includes(bootHash)) problem('policy', 'desktop.production allows the realm boot script')
      if (JSON.stringify(JSON.parse(read(at('shells/tauri/src-tauri/tauri.conf.json')))).includes(bootHash)) problem('policy', 'tauri.conf.json (what a build with no overlay ships) allows the realm boot script')
    }
    const release = read(at('.github/workflows/release.yml'))
    const overlayStep =
      /if \[\[ "\$TAG" == \*-pre\.\* \]\]; then\n\s+overlay="[^"\n]+"\n\s+node shells\/csp\/tauri-overlay\.mjs preview "\$overlay"\n\s+echo "PREVIEW_POLICY_CONFIG=--config \$overlay" >> "\$GITHUB_ENV"\n\s+else\n\s+echo "PREVIEW_POLICY_CONFIG=" >> "\$GITHUB_ENV"/
    if (!overlayStep.test(release) || !release.includes('/release-version.json ${{ env.PREVIEW_POLICY_CONFIG }}')) {
      problem('policy', '.github/workflows/release.yml no longer gives a -pre. tag, and only one, the desktop.preview overlay')
    }
    if (!release.includes('node scripts/check-desktop-binary.mjs --want "$want"') || !release.includes('want=production; [[ "$TAG" == *-pre.* ]] && want=preview')) {
      problem('policy', '.github/workflows/release.yml no longer checks the built desktop binary for the policy its tag needs (scripts/check-desktop-binary.mjs)')
    }
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
  if (cspMetaOf(withCspMeta(page, policyForBuild('web', { HEW_WEB_SOURCES: 'off' }))) !== policy('web', 'strict')) {
    problem('policy', 'the web injector does not serve web.strict for HEW_WEB_SOURCES=off')
  }
  if (cspMetaOf(withCspMeta(page, policyForBuild('web', { HEW_PREVIEW: '1' }))) !== policy('web', 'preview')) {
    problem('policy', 'the web injector does not serve web.preview for HEW_PREVIEW=1')
  }

  const webDist = option('--web-dist', null)
  if (webDist !== null) {
    const index = join(webDist, 'index.html')
    if (!existsSync(index)) fail(`${relative(ROOT, webDist)} holds no built web shell`)
    const built = cspMetaOf(read(index))
    if (built !== policy('web', 'production') && built !== policy('web', 'strict')) {
      problem('policy', `${relative(ROOT, index)} carries neither web.production nor web.strict`)
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

// --------------------------------------------------------------- markers
// A marker that marks nothing makes the bundle scan below vacuous: rename
// the string in the code and forget the registry, and the production scan
// looks for text no build holds. Every loader marker and shell command must
// occur in the code of the feature it names (not in a test, not in the
// registry itself, not in the kernel's embedded copy of it), and in
// preview-only code: the plugin modules the loader roots import
// (app/src/plugins), the realm's SDK (sdk/plugin, which every-build code
// takes only types from), the desktop shell's preview_only module, and the
// kernel's gated plugin exports. A marker found only in every-build code
// would mark code every build carries, so the bundle scan could never pass
// honestly. One marker cannot be placed there: the Settings window's tab
// icon attribute lives in the every-build Settings window, in a branch the
// build literal removes; it keeps the older rule (anywhere in the code).
{
  const PREVIEW_ONLY = [
    'app/src/plugins/',
    'sdk/plugin/',
    'shells/tauri/src-tauri/src/preview_only/',
    'crates/wasm-api/src/plugins_api.rs',
  ]
  const ANYWHERE = new Set(['data-plugins-tab-icon'])
  const roots = ['app/src', 'app/index.html', 'sdk/plugin', 'shells/csp', 'shells/tauri/src-tauri/src', 'crates']
  const skipDirs = new Set(['node_modules', 'target', 'dist', 'pkg', 'e2e', 'tests'])
  const sources = []
  const collect = (path) => {
    if (statSync(path).isDirectory()) {
      for (const name of readdirSync(path)) if (!skipDirs.has(name)) collect(join(path, name))
    } else if (!/\.test\.[cm]?[jt]sx?$/.test(path) && !/(^|\/)(preview\.rs|preview-features\.json)$/.test(path.split(sep).join('/')) && !/\.(png|svg|ico|wasm|woff2?|ttf)$/.test(path)) {
      sources.push(path)
    }
  }
  for (const root of roots) if (existsSync(at(root))) collect(at(root))
  const text = sources.map((f) => readFileSync(f, 'utf8')).join('\n')
  const named = [
    ...new Set(loaders.flatMap((f) => [...f.loader.markers, ...f.loader.shellCommands])),
  ]
  if (sources.length === 0 || !text.includes('hew.settings.')) problem('markers', 'the source scan read nothing (no hew.settings. in the code)')
  const previewText = sources
    .filter((f) => PREVIEW_ONLY.some((root) => relative(ROOT, f).split(sep).join('/').startsWith(root)))
    .map((f) => readFileSync(f, 'utf8'))
    .join('\n')
  if (previewText === '') problem('markers', 'the source scan read no preview-only code')
  for (const needle of named) {
    if (!text.includes(needle)) problem('markers', `the registry names "${needle}", which occurs in no code of the feature it marks; rename it in preview-features.json too, or the production scan checks for nothing`)
    else if (!ANYWHERE.has(needle) && !previewText.includes(needle)) {
      problem('markers', `the registry names "${needle}", which occurs only in code every build carries (not under ${PREVIEW_ONLY.join(', ')}); a marker there marks nothing a production build drops`)
    }
  }
  console.log(`check-production-artifacts: markers: ${named.length} names found in ${sources.length} source files`)
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

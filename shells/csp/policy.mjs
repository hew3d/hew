// The content security policies of both shells, read from one file
// (policies.json beside this one; docs/dev/DEVELOPMENT.md "Preview features",
// "Code loading").
//
//   <shell>.production   what a regular release serves. Pinned: it must equal
//                        the committed baseline (baseline/<shell>.production.csp),
//                        so changing it takes two visible edits, and
//                        scripts/check-production-artifacts.mjs fails the gate
//                        on any other difference.
//   <shell>.preview      what a preview build (HEW_PREVIEW=1) serves. A Preview
//                        feature that needs a wider policy widens this one, never
//                        the production one.
//   <shell>.probe        the policy the realm probe is measured under. No release
//                        of either kind serves it; only the probe's own test
//                        build and its browser spec apply it.
//
// No policy here, and no header file in the repository, sets a cross-origin
// isolation header (COOP or COEP): Hew does not isolate its host page.

import { readFileSync } from 'node:fs'

const POLICIES = JSON.parse(readFileSync(new URL('./policies.json', import.meta.url), 'utf8'))

/** The build flag, by the app's rule (app/src/preview/parseFlag.ts): `1` on,
 *  `0` off, unset or empty the caller's default, anything else an error. */
export function previewFlag(raw, defaultOn) {
  if (raw === undefined || raw === '') return defaultOn
  if (raw === '1') return true
  if (raw === '0') return false
  throw new Error(`HEW_PREVIEW must be 1 (preview build), 0 (production build), or unset/empty; got "${raw}".`)
}

/** The policy string for `shell` (`web` or `desktop`) and `name`
 *  (`production`, `preview` or `probe`). An unknown pair is an error. */
export function policy(shell, name) {
  const value = POLICIES[shell]?.[name]
  if (typeof value !== 'string' || value === '') throw new Error(`no content security policy "${shell}.${name}" in shells/csp/policies.json`)
  return value
}

/** The policy a build of `shell` serves, from the build environment. A
 *  build is a production build unless HEW_PREVIEW is exactly 1. */
export function policyForBuild(shell, env = process.env) {
  return policy(shell, previewFlag(env.HEW_PREVIEW, false) ? 'preview' : 'production')
}

/** `html` with the policy as a <meta> right after the charset meta, so it
 *  governs every later resource. Throws when the anchor is missing or a
 *  policy is already present: a build must never ship without one, or with
 *  two. */
export function withCspMeta(html, csp) {
  if (html.includes('http-equiv="Content-Security-Policy"')) throw new Error('a Content-Security-Policy <meta> is already present')
  const charset = html.match(/<meta\s+charset=["'][^"']*["']\s*\/?>/i)
  if (!charset) throw new Error('no <meta charset> to anchor the Content-Security-Policy <meta> after')
  return html.replace(charset[0], `${charset[0]}\n    <meta http-equiv="Content-Security-Policy" content="${csp}" />`)
}

/** The policy in a built page's <meta>, or null. */
export function cspMetaOf(html) {
  const m = html.match(/<meta http-equiv="Content-Security-Policy" content="([^"]*)" \/>/)
  return m ? m[1] : null
}

/**
 * The user guide must not describe a Preview feature as shipped
 * (docs/dev/DEVELOPMENT.md "Preview features").
 *
 * The public site is built from the repo at each production release, so a
 * Learn chapter about a Preview feature carries `preview: [<registry id>]` in
 * its frontmatter and `site/src/lib/published.ts` keeps it out of the build.
 * This test covers what the build cannot:
 *
 *  - every id a chapter names is in `preview-features.json` (the site build
 *    enforces this too; here it fails in the unit run, with the same advice);
 *  - a published chapter links to no held-back chapter;
 *  - a published chapter, the unreleased part of CHANGELOG.md, README.md and
 *    the shipped half of docs/ROADMAP.md mention no Preview tool or command by name, using the
 *    patterns below.
 *
 * What it CANNOT see: a gated feature described without its name (a Preview
 * behavior such as a new key on an existing tool, or the twist typed into
 * Follow Me), and the plain words "shell", "project", "smooth" and
 * "twist", which are ordinary English and would flag unrelated sentences.
 * Those get the narrower patterns in AMBIGUOUS (the bolded or capitalized
 * tool name, the menu path, the link); a sentence that dodges those needs a
 * reviewer. Pages outside Learn (compare, FAQ, landing) are not scanned: they
 * say "no fillet tool" in places on purpose. `docs/agents/*` and `docs/dev/*`
 * are developer docs and are not scanned either.
 */

import { readFileSync, readdirSync, statSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { describe, expect, it } from 'vitest'

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '../../..')
const LEARN = join(ROOT, 'site/src/content/learn')
const REGISTRY = (JSON.parse(readFileSync(join(ROOT, 'preview-features.json'), 'utf8')) as { features: { id: string }[] }).features
const KNOWN = new Set(REGISTRY.map((f) => f.id))

/** Tool and command names that mean one thing only. Matched as written. */
export const UNAMBIGUOUS: RegExp[] = [
  /\b[Ff]illet(s|ed|ing)?\b/,
  /\bChamfer\b/, // capitalized: a lower-case "chamfer" is the ordinary noun
  /\bLoft\b/,
  /\bRevolve\b/,
  /Array along Path/,
  /Soften [Ee]dges/,
  /\bDeform\b/,
  /Bend to Path/,
  /[Rr]emove degenerate faces/,
  /zero-area/,
  /Bake Smooth/,
  /Export cages/,
  // The plugin commands and the plugin pane's names: the bus commands, a
  // plugin asking for more, its console and activity.
  /\bhew\.plugin\.[a-z_]+/,
  /\bhew\.permissions\.request\b/,
  /\b[Oo]ptional permissions?\b/,
  /Allow more for this plugin/,
]

/** Everyday words that are also a tool name: matched only in the shapes the
 *  guide uses for the tool (bolded, a menu path, a link, "<Name> tool"). */
export const AMBIGUOUS: RegExp[] = [
  /\*\*(Shell|Project|Smooth)\*\*/,
  /\b(Shell|Project) tool\b/,
  /Object ▸ Smooth/,
  /\]\(\/learn\/smooth\//,
  /\bSmooth (Off|levels?|1|2|3)\b/,
  /Twist and end scale/i,
  // "plugin" alone is an ordinary word here (the license exception, the
  // roadmap); what is held back is loading one inside Hew.
  /Settings ▸ Plugins/,
  /Preview ▸ Plugins/,
  /\binstall(s|ed|ing)? (a|the|your) plugin\b/i,
]

interface Chapter {
  slug: string
  text: string
  preview: string[]
}

function walk(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const p = join(dir, name)
    return statSync(p).isDirectory() ? walk(p) : p.endsWith('.md') ? [p] : []
  })
}

function chapters(): Chapter[] {
  return walk(LEARN).map((file) => {
    const text = readFileSync(file, 'utf8')
    const front = /^---\n([\s\S]*?)\n---\n/.exec(text)?.[1] ?? ''
    const list = /^preview:\s*\[(.*)\]\s*$/m.exec(front)?.[1] ?? ''
    return {
      slug: file.slice(LEARN.length + 1).replace(/\.md$/, ''),
      text: text.slice(text.indexOf('\n---\n', 4) + 5),
      preview: list.split(',').map((s) => s.trim()).filter(Boolean),
    }
  })
}

function mentions(text: string): string[] {
  return [...UNAMBIGUOUS, ...AMBIGUOUS].flatMap((re) => {
    const hit = re.exec(text)
    return hit === null ? [] : [`${re}: "${text.slice(Math.max(0, hit.index - 40), hit.index + 60).replace(/\s+/g, ' ')}"`]
  })
}

/** The Unreleased section of CHANGELOG.md. A Preview feature has no entry
 *  there until it graduates, whatever the heading. */
function unreleased(): string {
  const changelog = readFileSync(join(ROOT, 'CHANGELOG.md'), 'utf8')
  const start = changelog.indexOf('## [Unreleased]')
  const end = changelog.indexOf('\n## [', start + 1)
  return changelog.slice(start, end)
}

/** docs/ROADMAP.md up to its "Where it's going" heading: what exists today. */
function roadmapShipped(): string {
  const roadmap = readFileSync(join(ROOT, 'docs/ROADMAP.md'), 'utf8')
  const end = roadmap.indexOf("## Where it's going")
  return end < 0 ? roadmap : roadmap.slice(0, end)
}

describe('user guide and the Preview wall', () => {
  const all = chapters()
  const published = all.filter((c) => c.preview.length === 0)

  it('finds the chapters (guards the scan against a moved folder)', () => {
    expect(all.length).toBeGreaterThan(20)
    expect(published.length).toBeGreaterThan(15)
  })

  it('every `preview:` id names a feature in preview-features.json', () => {
    const bad = all.flatMap((c) => c.preview.filter((id) => !KNOWN.has(id)).map((id) => `${c.slug}: ${id}`))
    expect(bad, 'a graduated feature: take the id off the chapter (it then publishes); for preview-additions/, move its sections into the chapters they name and delete the file').toEqual([])
  })

  it('held-back text sits only in chapters that name a feature', () => {
    // preview-additions/ is the holding area for text taken out of shared chapters.
    const stray = all.filter((c) => c.slug.startsWith('preview-additions/') && c.preview.length === 0).map((c) => c.slug)
    expect(stray).toEqual([])
  })

  it('a published chapter links to no held-back chapter', () => {
    const held = new Set(all.filter((c) => c.preview.length > 0).map((c) => c.slug))
    const bad = published.flatMap((c) =>
      [...c.text.matchAll(/\]\(\/learn\/([a-z0-9/-]+?)\/?(?:#[^)]*)?\)/g)]
        .filter((m) => held.has(m[1]))
        .map((m) => `${c.slug} -> ${m[1]}`),
    )
    expect(bad).toEqual([])
  })

  it('a published chapter names no Preview tool or command', () => {
    const bad = published.flatMap((c) => mentions(c.text).map((m) => `${c.slug} ${m}`))
    expect(bad).toEqual([])
  })

  it('the next release\'s changelog entries, README and the roadmap name none either', () => {
    const bad = [
      ...mentions(unreleased()).map((m) => `CHANGELOG.md [Unreleased] ${m}`),
      ...mentions(readFileSync(join(ROOT, 'README.md'), 'utf8')).map((m) => `README.md ${m}`),
      // The roadmap's "Where it's going" list may name what is planned.
      ...mentions(roadmapShipped()).map((m) => `docs/ROADMAP.md ${m}`),
    ]
    expect(bad).toEqual([])
  })

  it('the patterns do catch the held-back text (the scan is not blind)', () => {
    const gated = all.filter((c) => c.preview.length > 0)
    expect(gated.length).toBeGreaterThan(5)
    for (const c of gated) expect(mentions(c.text).length, c.slug).toBeGreaterThan(0)
  })
})

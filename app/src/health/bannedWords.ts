/* oxlint-disable lingui/no-unlocalized-strings -- test helper: a word list that polices rendered copy, plus DOM attribute names; never shown to a person */
/**
 * Test helper: the words that must never reach the screen from Model Health
 * and Make Solid (docs/design/import-health.md, Revisions 2 and 3, "Words"), and a
 * way to collect everything a person could read in a rendered tree: text,
 * plus title, aria-label and placeholder attributes.
 */
export const BANNED_WORDS = [
  'leaky',
  'rim',
  'rims',
  'planar',
  'manifold',
  'seam',
  'seams',
  'verdict',
  'cappable',
  'partition',
  'surface',
  'surfaces',
  'accepted',
  'need fixing',
  'needs fixing',
  'fix model',
  'closer look',
  'watertight',
  'weld',
  'welded',
  'welding',
  'vertex',
  'vertices',
  'triangulate',
  'triangulation',
] as const

/** Matches a banned word as a whole word, also inside "non-manifold". */
export const BANNED_RE = new RegExp(`\\b(${BANNED_WORDS.join('|')})\\b`, 'i')

/** Everything readable in `root`: its text and the text-bearing attributes. */
export function readableText(root: ParentNode): string {
  const parts: string[] = [(root as HTMLElement).textContent ?? '']
  for (const el of Array.from(root.querySelectorAll('[title],[aria-label],[placeholder]'))) {
    for (const attr of ['title', 'aria-label', 'placeholder']) {
      const v = el.getAttribute(attr)
      if (v !== null) parts.push(v)
    }
  }
  for (const title of Array.from(root.querySelectorAll('title'))) parts.push(title.textContent ?? '')
  return parts.join('\n')
}

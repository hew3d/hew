/**
 * palette/search — pure ranking for the command palette (
 * `04_command_palette.md`).
 *
 * Implements two of the spec's three ranking signals: **query match**
 * quality (prefix > word-prefix > substring, name over synonym over
 * description) and **recency** (a small tie-breaking bonus, not enough to
 * override a materially better text match). **Current-selection relevance**
 * is deliberately NOT implemented as a general ranking signal in this
 * milestone — it would need the same selection-type mapping 's
 * (rescoped) contextual dock defines, which hasn't landed yet. Instead, the
 * empty-query state falls back to recency + a small fixed set of
 * generally-useful suggestions, which the spec explicitly allows ("Empty
 * query -> show Recent + a few suggested actions").
 *
 * No React import — pure data in, pure data out, unit-testable directly.
 */

import type { PaletteEntry } from './registry'

/** Shown for an empty query, after Recent, when they aren't already in Recent. */
const DEFAULT_SUGGESTIONS = ['tool-select', 'tool-pushpull', 'save', 'undo']

const RECENCY_BONUS_MAX = 10

function normalize(s: string): string {
  return s.trim().toLowerCase()
}

/** Score a single entry against a normalized, non-empty query. Higher is
 * better; 0 means "no match" (caller filters these out). Tiers are spaced
 * far enough apart (20+) that the recency bonus (max 10) can only break ties
 * within a tier, never promote a worse text match over a better one. */
function scoreEntry(entry: PaletteEntry, query: string): number {
  const label = normalize(entry.label)
  if (label === query) return 100
  if (label.startsWith(query)) return 90
  // Word-prefix: query prefixes any word-boundary-delimited word in the
  // label (e.g. "pull" -> "Push/Pull", "extents" -> "Zoom Extents").
  if (label.split(/[^a-z0-9]+/).some((word) => word.startsWith(query))) return 80
  if (label.includes(query)) return 60

  const synonyms = (entry.synonyms ?? []).map(normalize)
  if (synonyms.some((s) => s.startsWith(query))) return 55
  if (synonyms.some((s) => s.includes(query))) return 40

  if (normalize(entry.description).includes(query)) return 20

  return 0
}

function recencyBonus(id: string, recentIds: string[]): number {
  const idx = recentIds.indexOf(id)
  if (idx === -1) return 0
  return Math.max(0, RECENCY_BONUS_MAX - idx)
}

/**
 * Rank `entries` for `query` (current-selection relevance intentionally
 * omitted — see module doc comment). Empty/whitespace-only query returns the
 * empty-state ordering (Recent, then a few fixed suggestions, then
 * everything else in registry order) rather than a text-ranked list.
 */
export function rankEntries(query: string, entries: PaletteEntry[], recentIds: string[]): PaletteEntry[] {
  const q = normalize(query)
  const byId = new Map(entries.map((e) => [e.id, e]))

  if (q === '') {
    const seen = new Set<string>()
    const ordered: PaletteEntry[] = []
    for (const id of recentIds) {
      const e = byId.get(id)
      if (e !== undefined && !seen.has(id)) {
        ordered.push(e)
        seen.add(id)
      }
    }
    for (const id of DEFAULT_SUGGESTIONS) {
      const e = byId.get(id)
      if (e !== undefined && !seen.has(id)) {
        ordered.push(e)
        seen.add(id)
      }
    }
    for (const e of entries) {
      if (!seen.has(e.id)) {
        ordered.push(e)
        seen.add(e.id)
      }
    }
    return ordered
  }

  return entries
    .map((entry) => ({ entry, score: scoreEntry(entry, q) }))
    .filter(({ score }) => score > 0)
    .map(({ entry, score }) => ({ entry, score: score + recencyBonus(entry.id, recentIds) }))
    .sort((a, b) => b.score - a.score)
    .map(({ entry }) => entry)
}

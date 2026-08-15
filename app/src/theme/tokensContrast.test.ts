/**
 * tokensContrast — a durable guard against WCAG contrast regressions in
 * Shop Mode's small-text token pairs (tokens.css's own "Shop Mode" block).
 * An adversarial design review found two pairs failing 4.5:1 (light
 * `--shop-text-faint` on `--surface-card`/`--surface-sheet`, ~2.75:1/
 * ~2.85:1; cream `--shop-on-accent` on plain `--shop-accent`, ~3.6:1 — the
 * latter fixed by adding `--shop-accent-fill` as the background wherever
 * cream sits on it as TEXT, tokens.css's own doc comment on that token).
 * This file parses the REAL tokens.css (not a hand-copied snapshot of the
 * values) so a future token edit that silently regresses one of these
 * pairs fails a test instead of shipping unnoticed.
 *
 * `--shop-dock`/`--shop-dock-text`/`--shop-dock-text-strong` became
 * theme-scoped (an on-device playtest finding: charcoal-in-both-themes dock
 * chrome read as jarring mixed-mode on a light phone) — their own pairs
 * below check EACH theme's values now, not one shared ratio.
 *
 * Deliberately narrow: only the Shop Mode pairs a review/playtest finding
 * named, not a full audit of the editor's own (already-reviewed) palette.
 */
import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { describe, expect, it } from 'vitest'

const TOKENS_CSS_PATH = fileURLToPath(new URL('./tokens.css', import.meta.url))

/** One `selector { --a: b; ... }` block's declarations, keyed by var name
 *  WITHOUT its leading `--`. Comments are stripped before parsing so
 *  doc-comment prose that happens to mention a token name (e.g. "not
 *  --shop-accent itself") is never mistaken for a declaration — real
 *  declarations always have a `:` immediately after the name; prose
 *  mentions never do. */
function parseCssBlocks(css: string): { selector: string; vars: Record<string, string> }[] {
  const uncommented = css.replace(/\/\*[\s\S]*?\*\//g, '')
  const blocks: { selector: string; vars: Record<string, string> }[] = []
  const blockRe = /([^{}]+)\{([^{}]*)\}/g
  let m: RegExpExecArray | null
  while ((m = blockRe.exec(uncommented)) !== null) {
    const selector = m[1].replace(/\s+/g, ' ').trim()
    const vars: Record<string, string> = {}
    const varRe = /--([a-zA-Z0-9-]+)\s*:\s*([^;]+);/g
    let vm: RegExpExecArray | null
    while ((vm = varRe.exec(m[2])) !== null) vars[vm[1]] = vm[2].trim()
    blocks.push({ selector, vars })
  }
  return blocks
}

const css = readFileSync(TOKENS_CSS_PATH, 'utf-8')
const blocks = parseCssBlocks(css)

const sharedVars = blocks.find((b) => b.selector === ':root')?.vars
const darkVars = blocks.find((b) => b.selector.replace(/'/g, '"') === ':root, [data-theme="dark"]')?.vars
const lightVars = blocks.find((b) => b.selector.replace(/'/g, '"') === '[data-theme="light"]')?.vars

if (sharedVars === undefined || darkVars === undefined || lightVars === undefined) {
  throw new Error('tokensContrast.test.ts: could not locate tokens.css\'s :root/dark/light blocks — selector text drifted, update this parser')
}

/** A theme's value for `name` — the theme-specific block if it redefines
 *  the token, else the shared `:root` block (mirrors the cascade: Shop
 *  Mode's accent/dock tokens deliberately live only in the shared block —
 *  tokens.css's own doc comment on why). */
function tokenValue(themeVars: Record<string, string>, name: string): string {
  const value = themeVars[name] ?? sharedVars![name]
  if (value === undefined) throw new Error(`tokensContrast.test.ts: token --${name} not found in tokens.css`)
  return value
}

function hexToRgb(hex: string): [number, number, number] {
  const trimmed = hex.trim().replace('#', '')
  const full = trimmed.length === 3 ? trimmed.split('').map((c) => c + c).join('') : trimmed
  const num = parseInt(full, 16)
  if (full.length !== 6 || Number.isNaN(num)) {
    throw new Error(`tokensContrast.test.ts: expected a plain #rrggbb/#rgb color, got "${hex}" — this guard only handles the flat hex colors the four Shop Mode pairs use`)
  }
  return [(num >> 16) & 255, (num >> 8) & 255, num & 255]
}

/** WCAG 2.x relative luminance (the sRGB piecewise-gamma formula in the
 *  spec's own "relative luminance" definition). */
function relativeLuminance([r, g, b]: [number, number, number]): number {
  const channel = (c: number) => {
    const s = c / 255
    return s <= 0.03928 ? s / 12.92 : Math.pow((s + 0.055) / 1.055, 2.4)
  }
  const [rl, gl, bl] = [channel(r), channel(g), channel(b)]
  return 0.2126 * rl + 0.7152 * gl + 0.0722 * bl
}

/** WCAG contrast ratio between two colors — order-independent (the spec's
 *  own formula: (lighter + 0.05) / (darker + 0.05)). */
function contrastRatio(hexA: string, hexB: string): number {
  const lumA = relativeLuminance(hexToRgb(hexA))
  const lumB = relativeLuminance(hexToRgb(hexB))
  const [lighter, darker] = lumA > lumB ? [lumA, lumB] : [lumB, lumA]
  return (lighter + 0.05) / (darker + 0.05)
}

const WCAG_AA_TEXT_MIN = 4.5

describe('Shop Mode token contrast (WCAG AA text, >=4.5:1)', () => {
  it.each(['dark', 'light'] as const)('%s: --shop-text-faint on --surface-sheet', (theme) => {
    const themeVars = theme === 'dark' ? darkVars! : lightVars!
    const ratio = contrastRatio(tokenValue(themeVars, 'shop-text-faint'), tokenValue(themeVars, 'surface-sheet'))
    expect(ratio).toBeGreaterThanOrEqual(WCAG_AA_TEXT_MIN)
  })

  it.each(['dark', 'light'] as const)('%s: --shop-text-faint on --surface-card', (theme) => {
    const themeVars = theme === 'dark' ? darkVars! : lightVars!
    const ratio = contrastRatio(tokenValue(themeVars, 'shop-text-faint'), tokenValue(themeVars, 'surface-card'))
    expect(ratio).toBeGreaterThanOrEqual(WCAG_AA_TEXT_MIN)
  })

  // Shared (non-themed) pair: --shop-accent-fill/--shop-on-accent both live
  // only in the shared :root block (tokens.css's own doc comment — the
  // working terracotta accent is deliberately the same in both themes), so
  // there is exactly one ratio to check, not one per theme.
  it('--shop-on-accent on --shop-accent-fill', () => {
    const ratio = contrastRatio(tokenValue(sharedVars!, 'shop-on-accent'), tokenValue(sharedVars!, 'shop-accent-fill'))
    expect(ratio).toBeGreaterThanOrEqual(WCAG_AA_TEXT_MIN)
  })

  // --shop-dock/--shop-dock-text/--shop-dock-text-strong are theme-scoped
  // (dark/light doc comments in tokens.css) — one ratio per theme each.
  it.each(['dark', 'light'] as const)('%s: --shop-dock-text on --shop-dock', (theme) => {
    const themeVars = theme === 'dark' ? darkVars! : lightVars!
    const ratio = contrastRatio(tokenValue(themeVars, 'shop-dock-text'), tokenValue(themeVars, 'shop-dock'))
    expect(ratio).toBeGreaterThanOrEqual(WCAG_AA_TEXT_MIN)
  })

  it.each(['dark', 'light'] as const)('%s: --shop-dock-text-strong on --shop-dock', (theme) => {
    const themeVars = theme === 'dark' ? darkVars! : lightVars!
    const ratio = contrastRatio(tokenValue(themeVars, 'shop-dock-text-strong'), tokenValue(themeVars, 'shop-dock'))
    expect(ratio).toBeGreaterThanOrEqual(WCAG_AA_TEXT_MIN)
  })
})

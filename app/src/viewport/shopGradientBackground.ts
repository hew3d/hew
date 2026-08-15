/**
 * shopGradientBackground — Shop Mode's warm/near-black viewport backdrop
 * (design_handoff_shop_mode/README.md Design Tokens: `--viewport-bg`), built
 * as a small vertical-gradient `THREE.CanvasTexture` set as `scene.background`
 * — NOT a transparent canvas over a CSS gradient (the task's explicit "no
 * renderer alpha/transparent-canvas" constraint: an alpha WebGL context
 * forces a slower compositing path on many phone GPUs, and every other
 * overlay in this app already assumes an opaque canvas). `Viewport.tsx`
 * rebuilds this texture on mount and on every live theme change, the same
 * way it rebuilds `originAxes`/`infiniteGrid`'s colors — see its own
 * `applyShopGradientBackground` call sites.
 *
 * `parseGradientStops` is the pure, unit-tested half: turns the CSS
 * `linear-gradient(...)` VALUE of the `--viewport-bg` token (read live off
 * `getComputedStyle`, so it honors whatever `tokens.css` currently says
 * without a second hard-coded copy of the colors) into an ordered list of
 * `{ color, offset }` stops a canvas gradient can consume directly.
 */
import * as THREE from 'three'

export interface GradientStop {
  /** 0–1 position along the gradient. */
  offset: number
  /** Any CSS color `CanvasRenderingContext2D.createLinearGradient`'s
   *  `addColorStop` accepts — the parser passes the token's own color
   *  literal through unchanged rather than re-parsing color syntax itself. */
  color: string
}

/**
 * Parse a CSS `linear-gradient(180deg, #rrggbb 0%, #rrggbb 60%, #rrggbb
 * 100%)` (or the angle-less `linear-gradient(#rrggbb, #rrggbb)` the dark
 * `--viewport-bg` token uses) into ordered stops. A stop with no explicit
 * `%` is spread evenly across `[0, 1]` (the dark token's two bare colors
 * become 0% and 100%) — both real tokens either give every stop a percent or
 * none at all, so this deliberately doesn't implement general CSS
 * gradient-stop interpolation (mixed percent/bare stops). Returns `[]` for
 * anything that isn't a `linear-gradient(...)` — the caller falls back to
 * the editor's plain clear-color path rather than throwing on a malformed or
 * future token value.
 */
export function parseGradientStops(cssValue: string): GradientStop[] {
  const match = /linear-gradient\(([^)]*)\)/i.exec(cssValue.trim())
  if (match === null) return []
  const parts = match[1].split(',').map((s) => s.trim()).filter((s) => s.length > 0)
  if (parts.length === 0) return []

  // Drop a leading angle/direction token ("180deg", "to bottom", …) — every
  // remaining part is "<color> [<percent>]".
  const first = parts[0]
  const isAngleOrKeyword = /^-?\d+(\.\d+)?deg$/i.test(first) || /^to\s+/i.test(first)
  const colorParts = isAngleOrKeyword ? parts.slice(1) : parts
  if (colorParts.length === 0) return []

  const parsed = colorParts.map((part) => {
    const m = /^(.+?)\s+(-?\d+(?:\.\d+)?)%$/.exec(part)
    return m !== null
      ? { color: m[1].trim(), offset: Number(m[2]) / 100 }
      : { color: part, offset: null as number | null }
  })

  const n = parsed.length
  return parsed.map((p, i) => ({
    color: p.color,
    offset: p.offset ?? (n === 1 ? 0 : i / (n - 1)),
  }))
}

/** Canvas height (px) for the built texture — narrow (2px wide, no
 *  horizontal variation) since it's stretched to fill the screen; linear
 *  texture filtering (three's default) smooths the vertical steps between
 *  this many sampled rows. */
const TEXTURE_HEIGHT_PX = 128
const TEXTURE_WIDTH_PX = 2

/** Build the vertical-gradient texture for `scene.background` from already-
 *  parsed stops. Returns `null` (rather than an all-white texture) when
 *  `stops` is empty or the environment has no 2D canvas context (headless
 *  test runners) — callers fall back to the editor's plain clear-color path
 *  in that case, same as a parse failure. */
export function buildShopGradientTexture(stops: GradientStop[]): THREE.CanvasTexture | null {
  if (stops.length === 0) return null
  const canvas = document.createElement('canvas')
  canvas.width = TEXTURE_WIDTH_PX
  canvas.height = TEXTURE_HEIGHT_PX
  const ctx = canvas.getContext('2d')
  if (ctx === null) return null

  const gradient = ctx.createLinearGradient(0, 0, 0, TEXTURE_HEIGHT_PX)
  for (const stop of stops) {
    gradient.addColorStop(Math.min(1, Math.max(0, stop.offset)), stop.color)
  }
  ctx.fillStyle = gradient
  ctx.fillRect(0, 0, TEXTURE_WIDTH_PX, TEXTURE_HEIGHT_PX)

  const texture = new THREE.CanvasTexture(canvas)
  texture.colorSpace = THREE.SRGBColorSpace
  texture.wrapS = THREE.ClampToEdgeWrapping
  texture.wrapT = THREE.ClampToEdgeWrapping
  texture.needsUpdate = true
  return texture
}

/**
 * Read the LIVE `--viewport-bg` custom property off `el` (default
 * `document.documentElement`, where `theme/applyTheme.ts` sets the
 * `data-theme` attribute the token's per-theme value keys off) and build the
 * gradient texture from it. `null` when running outside a DOM (SSR/unit
 * tests), the token isn't set, or it can't be parsed — every case a caller
 * should fall back to the editor's plain clear-color path for.
 */
export function buildShopGradientTextureFromToken(el: HTMLElement = document.documentElement): THREE.CanvasTexture | null {
  if (typeof window === 'undefined' || typeof window.getComputedStyle !== 'function') return null
  const raw = window.getComputedStyle(el).getPropertyValue('--viewport-bg')
  return buildShopGradientTexture(parseGradientStops(raw))
}

/**
 * Shop Mode chrome icons — inline SVG only (design_handoff_shop_mode's
 * Assets note: "no raster assets... the full set is drawn in
 * `Shop Mode Screens.dc.html` option 1p"). Paths are mined verbatim from
 * that component sheet (and the interactive prototype, which renders the
 * identical markup) rather than approximated, per the handoff's
 * pixel-perfect fidelity instruction.
 *
 * 24x24 viewBox, stroke=2, round joins/caps — matching the app's existing
 * Lucide-like icon language (`tools/toolIcons.ts`'s Material Symbols set is
 * a DIFFERENT, filled icon language used only for the desktop editor's
 * toolbar/cursors; Shop Mode's chrome never mixes the two).
 *
 * Every icon takes only `size` (px, both dimensions — the design's icons
 * are never non-square) — color comes from the caller's `color`/`currentColor`
 * context, never hardcoded here, so the same glyph works on the (now
 * theme-scoped) dock's own `--shop-dock-text` and a terracotta-active
 * segment (`--shop-on-accent`) alike, in either theme.
 */
import type { CSSProperties } from 'react'

export interface ShopIconProps {
  size?: number
  className?: string
  style?: CSSProperties
}

const BASE_PROPS = {
  viewBox: '0 0 24 24',
  fill: 'none',
  stroke: 'currentColor',
  strokeWidth: 2,
} as const

export function SelectIcon({ size = 24, className, style }: ShopIconProps) {
  return (
    <svg width={size} height={size} aria-hidden="true" className={className} style={style} {...BASE_PROPS} strokeLinejoin="round">
      <path d="M6 3l7 15 2-6 6-2z" />
    </svg>
  )
}

export function OrbitIcon({ size = 24, className, style }: ShopIconProps) {
  return (
    <svg width={size} height={size} aria-hidden="true" className={className} style={style} {...BASE_PROPS} strokeLinecap="round">
      <circle cx="12" cy="12" r="3.5" />
      <path d="M19.6 9.3a8 8 0 1 1-4.9-4.9" />
      <path d="M19 3v5h-5" />
    </svg>
  )
}

export function TapeIcon({ size = 24, className, style }: ShopIconProps) {
  return (
    <svg width={size} height={size} aria-hidden="true" className={className} style={style} {...BASE_PROPS} strokeLinecap="round">
      <rect x="3" y="8" width="18" height="8" rx="2" />
      <path d="M7.5 8v3M12 8v4M16.5 8v3" />
    </svg>
  )
}

export function ZoomExtentsIcon({ size = 24, className, style }: ShopIconProps) {
  return (
    <svg width={size} height={size} aria-hidden="true" className={className} style={style} {...BASE_PROPS} strokeLinecap="round">
      <path d="M4 9V4h5M20 9V4h-5M4 15v5h5M20 15v5h-5" />
    </svg>
  )
}

/** "Views" dock button (playtest finding 12) — a flat, front-on square
 *  divided into quadrants, reading as a "pick a view/face" grid rather than
 *  a solid isometric cube: deliberately NOT the same glyph as `ArCubeIcon`
 *  below (a different feature, and the design task's own instruction that
 *  this must not be confused with the phone camera it also isn't). */
export function ViewCubeIcon({ size = 24, className, style }: ShopIconProps) {
  return (
    <svg width={size} height={size} aria-hidden="true" className={className} style={style} {...BASE_PROPS} strokeLinejoin="round">
      <rect x="4" y="4" width="16" height="16" rx="2" />
      <path d="M4 12h16M12 4v16" />
    </svg>
  )
}

export function ArCubeIcon({ size = 24, className, style }: ShopIconProps) {
  return (
    <svg width={size} height={size} aria-hidden="true" className={className} style={style} {...BASE_PROPS} strokeLinejoin="round">
      <path d="M12 3l7 4v10l-7 4-7-4V7l7-4z" />
      <path d="M12 12l7-4M12 12L5 8M12 12v9" />
    </svg>
  )
}

export function EllipsisIcon({ size = 24, className, style }: ShopIconProps) {
  return (
    <svg width={size} height={size} aria-hidden="true" className={className} style={style} viewBox="0 0 24 24" fill="currentColor">
      <circle cx="5" cy="12" r="1.8" />
      <circle cx="12" cy="12" r="1.8" />
      <circle cx="19" cy="12" r="1.8" />
    </svg>
  )
}

/** "Open…" / "Open a model…" — same glyph in the overflow menu and the
 *  empty state's primary button (design: "upload icon"). */
export function UploadIcon({ size = 24, className, style }: ShopIconProps) {
  return (
    <svg width={size} height={size} aria-hidden="true" className={className} style={style} {...BASE_PROPS} strokeLinecap="round">
      <path d="M12 15V4M7 9l5-5 5 5" />
      <path d="M5 15v4h14v-4" />
    </svg>
  )
}

/** "Scan from desktop" ghost button (empty state) — a QR glyph; Shop Mode
 *  has no camera scan flow of its own (module doc on the button's handler
 *  in ShopApp.tsx), this only illustrates the desktop-initiated handoff. */
/** Document menu "Recent models…" row — a clock face. */
export function ClockIcon({ size = 24, className, style }: ShopIconProps) {
  return (
    <svg width={size} height={size} aria-hidden="true" className={className} style={style} {...BASE_PROPS} strokeLinecap="round">
      <circle cx="12" cy="12" r="8.5" />
      <path d="M12 7.5V12l3 2" />
    </svg>
  )
}

export function QrIcon({ size = 24, className, style }: ShopIconProps) {
  return (
    <svg width={size} height={size} aria-hidden="true" className={className} style={style} {...BASE_PROPS}>
      <rect x="4" y="4" width="6" height="6" rx="1" />
      <rect x="14" y="4" width="6" height="6" rx="1" />
      <rect x="4" y="14" width="6" height="6" rx="1" />
      <path d="M14 14h3v3M20 14v6h-6" />
    </svg>
  )
}

/** Recents row chevron. */
export function ChevronRightIcon({ size = 24, className, style }: ShopIconProps) {
  return (
    <svg width={size} height={size} aria-hidden="true" className={className} style={style} {...BASE_PROPS} strokeLinecap="round">
      <path d="M9 6l6 6-6 6" />
    </svg>
  )
}

/** The Hew brand mark (BRAND.md geometry, reproduced in the README): a
 *  hexagon + three spokes from the center to the top and two upper
 *  vertices. `strokeWidth` defaults to 7 — the README's called-out
 *  "7 at <=20px renders" weight for the small top-strip/empty-state sizes
 *  this component is used at; pass 4.6 (the base weight) at any larger size. */
export function HewMark({ size = 24, strokeWidth = 7, color = 'var(--shop-accent)', className, style }: ShopIconProps & { strokeWidth?: number; color?: string }) {
  return (
    <svg
      width={size}
      height={size}
      aria-hidden="true"
      className={className}
      style={style}
      viewBox="-50 -50 100 100"
      fill="none"
      stroke={color}
      strokeWidth={strokeWidth}
      strokeLinejoin="round"
      strokeLinecap="round"
    >
      <polygon points="0,-34 29.44,-17 29.44,17 0,34 -29.44,17 -29.44,-17" />
      <path d="M0 0V-34M0 0L29.44 -17M0 0L-29.44 -17" />
    </svg>
  )
}

/** Cutlist row eye toggle (shown/visible state) — design §2 "Part row":
 *  "20px stroke-2 eye/eye-off icon". Paths mined from the prototype's own
 *  eye glyph (default butt/miter joins — the prototype's `<svg>` sets no
 *  `stroke-linecap`/`stroke-linejoin`, unlike most of this file's icons). */
export function EyeIcon({ size = 24, className, style }: ShopIconProps) {
  return (
    <svg width={size} height={size} aria-hidden="true" className={className} style={style} {...BASE_PROPS}>
      <path d="M2 12s4-7 10-7 10 7 10 7-4 7-10 7-10-7-10-7z" />
      <circle cx="12" cy="12" r="3" />
    </svg>
  )
}

/** Cutlist row eye toggle (hidden state) — pairs with `EyeIcon` above. */
export function EyeOffIcon({ size = 24, className, style }: ShopIconProps) {
  return (
    <svg width={size} height={size} aria-hidden="true" className={className} style={style} {...BASE_PROPS} strokeLinecap="round">
      <path d="M3 3l18 18M10.6 5.2A10.8 10.8 0 0 1 22 12s-1.4 2.5-3.9 4.4M6.6 6.6C3.5 8.6 2 12 2 12s4 7 10 7c1.5 0 2.9-.4 4.1-1" />
    </svg>
  )
}

/** Unit chip / Settings "Units" row chevron — the cutlist header's unit chip
 *  (design §1) and the picker-opening affordance. `strokeWidth` is fixed at
 *  2.4 (not this file's usual 2) to match the design's own thinner chevron
 *  weight at this small a size. */
export function ChevronDownIcon({ size = 24, className, style }: ShopIconProps) {
  return (
    <svg width={size} height={size} aria-hidden="true" className={className} style={style} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2.4} strokeLinecap="round">
      <path d="M6 9l6 6 6-6" />
    </svg>
  )
}

/** Unit picker's active-row checkmark (design §7). */
export function CheckIcon({ size = 24, className, style }: ShopIconProps) {
  return (
    <svg width={size} height={size} aria-hidden="true" className={className} style={style} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2.6} strokeLinecap="round">
      <path d="M4 12.5l5 5L20 6.5" />
    </svg>
  )
}

/** Overflow menu's "Save a copy (.hew)" item (design §6) — two overlapping
 *  squares, matching the prototype exactly (no linecap/linejoin override,
 *  same as `EyeIcon`). Deliberately NOT reused for "Open…", which mines the
 *  identical glyph to `UploadIcon` above and just uses that component
 *  directly rather than duplicating its paths under a second name. */
export function SaveCopyIcon({ size = 24, className, style }: ShopIconProps) {
  return (
    <svg width={size} height={size} aria-hidden="true" className={className} style={style} {...BASE_PROPS}>
      <rect x="9" y="9" width="11" height="11" rx="2" />
      <rect x="4" y="4" width="11" height="11" rx="2" />
    </svg>
  )
}

/** Document menu's "Print…" item (docs/design/printing.md §9c) — a printer:
 *  paper in, body, paper out. */
export function PrinterIcon({ size = 24, className, style }: ShopIconProps) {
  return (
    <svg width={size} height={size} aria-hidden="true" className={className} style={style} {...BASE_PROPS} strokeLinecap="round" strokeLinejoin="round">
      <path d="M7 9V4h10v5" />
      <rect x="4" y="9" width="16" height="8" rx="1.5" />
      <path d="M7 14h10v6H7z" />
    </svg>
  )
}

/** Overflow menu's "Use full editor" item (design §6) — an external-link
 *  glyph. */
export function FullEditorIcon({ size = 24, className, style }: ShopIconProps) {
  return (
    <svg width={size} height={size} aria-hidden="true" className={className} style={style} {...BASE_PROPS} strokeLinecap="round">
      <path d="M14 5h5v5M19 5l-8 8" />
      <path d="M19 14v5H5V5h5" />
    </svg>
  )
}

/** AR-busy spinner — replaces the AR cube glyph in place while an export is
 *  in flight (design §9 "AR busy"). CSS-animated (`.shop-spin`, index.css)
 *  rather than JS, matching the app's existing spinner precedent
 *  (`.hew-dock`'s crossfade). */
export function ArSpinner({ size = 17 }: { size?: number }) {
  return (
    <span
      aria-hidden="true"
      className="shop-spin"
      style={{
        display: 'inline-block',
        width: `${size}px`,
        height: `${size}px`,
        borderRadius: '50%',
        border: '2px solid rgba(217,138,104,.3)',
        borderTopColor: 'var(--shop-accent)',
      }}
    />
  )
}

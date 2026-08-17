/**
 * ScenePill — Shop Mode's active-Scene indicator (SPEC.md §2 "Active-Scene
 * pill", docs/design/scenes.md §6 "a visible indicator of the active Scene
 * with next/previous affordance"). Placement (playtest round 1, overriding
 * SPEC.md's "12px below the top strip"): PORTRAIT — centered at the bottom,
 * directly above the workbench dock's tool row (`placement.kind ===
 * 'bottom'`, the caller passes the dock stack's live height); LANDSCAPE —
 * top-center, on the top strip's own row between the document pill and the
 * ⋯ menu (`placement.kind === 'top'`). Layout `[‹ 48×48][7px dot + name 15px 600, max 160px
 * ellipsis][› 48×48]`; chevrons cycle Scenes with wrap (`neighborScene`,
 * scenesModel.ts); tapping the name opens the Views sheet. Hidden entirely
 * when no Scene is active — the ONLY always-on proof a Scene is live, since
 * Shop Mode renders no tray/tab chrome of its own.
 *
 * Drift (SPEC.md §2 "Drift & Show all"): the dot becomes a 1.5px ring and
 * the name drops to 80% opacity — nothing moves or flashes. Re-activating
 * (tapping the name → Views sheet → the same row) snaps back, since
 * activation always re-resolves and clears drift.
 */
import { ChevronRightIcon } from './icons'
import type { SceneEntry } from '../scenes/scenesModel'

export interface ScenePillProps {
  /** The active Scene, or `null` when none is active — hides the pill. */
  entry: SceneEntry | null
  /** True when the active Scene has drifted (SPEC.md §2). */
  drifted: boolean
  onPrevious: () => void
  onNext: () => void
  /** Tapping the name opens the Views sheet (SPEC.md §2). */
  onOpenName: () => void
  /** Where the pill sits (module doc): portrait = `bottom` (px above the
   *  viewport's bottom edge that the dock/sheet stack currently occupies;
   *  the pill floats 12px above that), landscape = `top` (a CSS length for
   *  the top strip's own offset). */
  placement: { kind: 'bottom'; bottomPx: number } | { kind: 'top'; topCss: string }
}

const PILL_STYLE_ID = 'hew-scene-pill-style'

/**
 * Injects the pill's own theme-aware translucent background once (mirrors
 * `ImportingOverlay.tsx`'s `ensureSpinnerKeyframes` precedent for a
 * self-contained `<style>` injector). SPEC.md §2 gives TWO DIFFERENT alpha
 * values per theme (dark .88, light .92) — a single `color-mix()`
 * percentage against one token can't express two different alphas, and
 * `theme/tokens.css` is out of scope for this effort — so this reproduces
 * tokens.css's own `:root` (dark-first) / `[data-theme="light"]` selector
 * shape locally instead, reacting to the SAME `data-theme` attribute
 * `theme/applyTheme.ts` already keeps current, with no React-side theme
 * subscription needed. NO `backdrop-filter` (SPEC.md §2 — the WebGL
 * compositing constraint every other dock/toast/banner in this shell
 * already respects).
 */
function ensurePillStyle(): void {
  if (typeof document === 'undefined' || document.getElementById(PILL_STYLE_ID) !== null) return
  const style = document.createElement('style')
  style.id = PILL_STYLE_ID
  style.textContent =
    '.shop-scene-pill{background:rgba(27,26,23,.88)}' +
    '[data-theme="light"] .shop-scene-pill{background:rgba(251,247,240,.92)}'
  document.head.appendChild(style)
}

const chevronButtonStyle: React.CSSProperties = {
  width: '48px', height: '48px', flexShrink: 0, display: 'flex', alignItems: 'center', justifyContent: 'center',
  border: 'none', background: 'transparent', color: 'var(--shop-dock-text)', cursor: 'pointer', padding: 0,
}

const nameButtonStyle: React.CSSProperties = {
  display: 'flex', alignItems: 'center', gap: '7px', height: '48px', minWidth: 0, maxWidth: '181px',
  border: 'none', background: 'transparent', cursor: 'pointer', padding: '0 2px',
}

/** SPEC.md §2: "max 160px ellipsis" is the NAME text's own cap — the
 *  button's `nameButtonStyle` above is 21px wider (7px dot + 7px gap +
 *  2×2px padding + rounding slop) to hold the dot alongside it without
 *  ALSO shrinking the name's available width below 160px. */
const nameTextStyle: React.CSSProperties = {
  fontFamily: 'var(--font-family-ui)', fontSize: '15px', fontWeight: 600,
  color: 'var(--shop-dock-text-strong)',
  overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', minWidth: 0, maxWidth: '160px',
}

export function ScenePill({ entry, drifted, onPrevious, onNext, onOpenName, placement }: ScenePillProps) {
  ensurePillStyle()

  if (entry === null) return null

  const anchor: React.CSSProperties =
    placement.kind === 'bottom'
      ? { bottom: `${placement.bottomPx + 12}px` }
      : { top: placement.topCss }

  return (
    <div
      data-testid="scene-pill"
      data-placement={placement.kind}
      style={{
        position: 'absolute', ...anchor, left: 0, right: 0,
        display: 'flex', justifyContent: 'center', zIndex: 41, pointerEvents: 'none',
      }}
    >
      <div
        className="shop-scene-pill"
        style={{
          display: 'flex', alignItems: 'center', height: '48px', borderRadius: 'var(--radius-hud, 15px)',
          boxShadow: '0 6px 18px -8px rgba(27,26,23,.6)', pointerEvents: 'auto',
        }}
      >
        <button type="button" aria-label="Previous Scene" className="shop-press" onClick={onPrevious} style={chevronButtonStyle}>
          <ChevronRightIcon size={18} style={{ transform: 'rotate(180deg)' }} />
        </button>

        <button type="button" className="shop-press" onClick={onOpenName} style={nameButtonStyle}>
          <svg aria-hidden="true" width="7" height="7" viewBox="0 0 7 7" style={{ flexShrink: 0 }}>
            {drifted ? (
              <circle cx="3.5" cy="3.5" r="2.75" fill="none" stroke="var(--shop-accent)" strokeWidth="1.5" />
            ) : (
              <circle cx="3.5" cy="3.5" r="3.5" fill="var(--shop-accent)" />
            )}
          </svg>
          <span style={{ ...nameTextStyle, opacity: drifted ? 0.8 : 1 }}>
            {entry.name}
          </span>
        </button>

        <button type="button" aria-label="Next Scene" className="shop-press" onClick={onNext} style={chevronButtonStyle}>
          <ChevronRightIcon size={18} />
        </button>
      </div>
    </div>
  )
}

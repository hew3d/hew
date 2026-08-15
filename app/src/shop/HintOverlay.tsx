/**
 * HintOverlay — draws whichever of Shop Mode's two ghost-affordance
 * gesture hints `hints.ts`'s `HintEngine` currently has active (design
 * §"Gesture discoverability" / `Shop Mode Screens.dc.html`'s option 1q).
 * Purely presentational: `ShopApp` owns the engine, decides WHEN hints are
 * allowed to render at all (never in the empty state, during AR busy, or
 * while a menu/picker/sheet-half-or-full is open — `ShopApp.tsx`'s
 * `hintsAllowed`), and supplies the tap-hint's own screen-space anchor
 * below; this only draws.
 *
 * A third hint, 'hold' ("Hold to see it alone" after the 3rd inspect of the
 * same part), used to render here too — playtest finding 7 dropped it as
 * redundant with `InspectCard.tsx`'s own permanent "hold part to isolate"
 * caption, which already teaches the identical gesture every time a part
 * card is up, not just once per install. `hints.ts`'s `ActiveHint` no
 * longer has a 'hold' variant at all.
 *
 * Every element here is `pointer-events: none` (module doc's "never
 * intercept pointer events" requirement) so a hint can float directly over
 * live viewport content — including right over the part it's pointing at —
 * without stealing the very tap/drag/hold it exists to teach. Motion
 * (the pulse, the ghost-finger sweep) lives entirely in `index.css`'s
 * `.shop-hint-*` classes, which already fall back to a static look under
 * `prefers-reduced-motion` — nothing here needs to know about that itself.
 */
import type { ActiveHint } from './hints'

export interface HintOverlayProps {
  hint: ActiveHint
  /** (a) 'tap' only: the largest part's current screen position (`ShopApp`
   *  projects it via `ViewportApi.worldToScreen`, re-run on the same
   *  camera-change signal that dismisses the inspect card). `null` while
   *  off-screen or mid camera-drag — nothing renders for 'tap' without one,
   *  rather than pointing at a stale/wrong position. */
  dotScreen: { x: number; y: number } | null
  containerWidth: number
  containerHeight: number
}

const TAG_MARGIN_PX = 12
const TAG_HALF_WIDTH_ESTIMATE_PX = 84

function clamp(value: number, min: number, max: number): number {
  return Math.min(Math.max(value, min), Math.max(min, max))
}

const tagChromeStyle: React.CSSProperties = {
  // --shop-dock-text-strong, not --shop-on-accent (ShopApp.tsx's own dock
  // tokens doc comment): this tag's text sits directly on --shop-dock, not
  // on a terracotta accent-fill, so it needs the theme-aware "primary text
  // on dock" pair, not the fixed-cream one.
  background: 'var(--shop-dock)', color: 'var(--shop-dock-text-strong)',
  fontFamily: 'var(--font-family-ui)', fontSize: '11px', fontWeight: 500,
  borderRadius: '9px', padding: '7px 11px', whiteSpace: 'nowrap',
  boxShadow: '0 6px 16px -8px rgba(27,26,23,.5)',
}

function tagStyle(left: number, top: number): React.CSSProperties {
  return { ...tagChromeStyle, position: 'absolute', left: `${left}px`, top: `${top}px`, transform: 'translate(-50%, 0)' }
}

export function HintOverlay({ hint, dotScreen, containerWidth, containerHeight }: HintOverlayProps) {
  if (hint.name === 'tap') {
    if (dotScreen === null) return null
    const tagLeft = clamp(dotScreen.x, TAG_MARGIN_PX + TAG_HALF_WIDTH_ESTIMATE_PX, containerWidth - TAG_MARGIN_PX - TAG_HALF_WIDTH_ESTIMATE_PX)
    const tagTop = clamp(dotScreen.y + 30, TAG_MARGIN_PX, containerHeight - 36)
    return (
      <div aria-hidden="true" style={{ position: 'absolute', inset: 0, pointerEvents: 'none', zIndex: 27 }}>
        {/* Halo ring (design 1q mock: 34px, terracotta at ~18%/50% alpha) —
            the piece that pulses; the solid core dot below stays steady as
            the fixed anchor. */}
        <div
          className="shop-hint-tap-ring"
          style={{
            position: 'absolute', left: `${dotScreen.x}px`, top: `${dotScreen.y}px`,
            width: '34px', height: '34px', borderRadius: '50%',
            background: 'rgba(196, 93, 60, .18)', border: '2px solid rgba(196, 93, 60, .5)',
            transform: 'translate(-50%, -50%)',
          }}
        />
        <div
          style={{
            position: 'absolute', left: `${dotScreen.x}px`, top: `${dotScreen.y}px`,
            width: '12px', height: '12px', borderRadius: '50%',
            background: 'var(--shop-accent)', transform: 'translate(-50%, -50%)',
          }}
        />
        <div style={tagStyle(tagLeft, tagTop)}>Tap a part for its size</div>
      </div>
    )
  }

  // hint.name === 'orbit' — the only case left once 'tap' is ruled out above.
  return (
    <div
      aria-hidden="true"
      style={{
        position: 'absolute', inset: 0, pointerEvents: 'none', zIndex: 27,
        display: 'flex', alignItems: 'center', justifyContent: 'center',
      }}
    >
      <div style={{ position: 'relative', width: '120px', height: '90px' }}>
        <svg width="120" height="90" viewBox="0 0 120 90" aria-hidden="true">
          <path
            d="M10 66 Q60 4 110 66"
            fill="none"
            stroke="var(--shop-dock-text)"
            strokeWidth="2"
            strokeDasharray="4 5"
            strokeLinecap="round"
            opacity="0.45"
          />
        </svg>
        {/* Ghost fingertip — the sole animated element (index.css's
            `.shop-hint-orbit-finger`): a hand-keyed traverse along the
            SAME quadratic curve the arc above draws. */}
        <div
          className="shop-hint-orbit-finger"
          style={{
            position: 'absolute', width: '18px', height: '18px', marginLeft: '-9px', marginTop: '-9px',
            borderRadius: '50%', background: 'var(--shop-dock-text)',
          }}
        />
        <div style={{ ...tagStyle(60, 78) }}>Drag to look around</div>
      </div>
    </div>
  )
}

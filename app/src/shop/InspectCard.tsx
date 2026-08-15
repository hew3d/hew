/**
 * InspectCard — Shop Mode's tap-to-inspect anchored callout
 * (design_handoff_shop_mode/README.md §3 "Inspect card"): a leader dot at
 * the exact snap point + a vertical leader line up (or down) to a compact
 * card, showing exactly what `inspect.ts`'s `resolveInspect` decided the tap
 * landed on. Purely presentational — `ShopApp` owns when it's shown/
 * dismissed (tap-elsewhere, camera movement) and re-resolves its content on
 * every tap; it also owns the enter/exit choreography (`leaving` prop below)
 * since a React unmount can't animate itself — same "leaving" pattern
 * ShopApp's own toast already uses.
 *
 * Reuses `settings/units.formatLengthIn` with the LIVE current length format
 * (subscribed below), the same "re-render every dimension on any unit
 * change" contract `PartsSheet.tsx` has followed since Wave 2 — never the
 * app-wide singleton `formatLength`.
 *
 * Two shapes (design §3):
 *   - PART: name + tag chip, an `L … W … H …` mono dims line with
 *     axis-colored letters (the L/W/H relabel's last X/Y/Z remnant in Shop
 *     Mode), a static usage caption.
 *   - EDGE: the OWNING PART's name + a small "edge" chip (never bare
 *     "Edge") + the measurement, biggest text on the card. The prototype's
 *     caption for this shape ("rim, top face · double-tap zooms to part")
 *     bundles a per-edge kernel attribution Hew has no query for yet
 *     (flagged as a follow-on spec item — see the caption constant below)
 *     with a generic usage hint; only the generic half ships here.
 */
import { useEffect, useState } from 'react'
import { formatLengthIn, getLengthUnit, subscribe as subscribeLengthUnit, type LengthFormat } from '../settings/units'
import { colorForTagPath, hexToRgba } from './tagPalette'
import type { InspectResult } from './inspect'

export interface InspectCardProps {
  result: InspectResult
  /** Tap position in viewport-container-relative CSS px — the leader dot
   *  sits exactly here (the design's "centered on the snap point"); the card
   *  centers itself horizontally on this X and flips above/below it. */
  screenX: number
  screenY: number
  containerWidth: number
  containerHeight: number
  /** True while ShopApp is running the 100ms exit fade (Motion summary:
   *  "out 100ms ease-in") — camera movement instead unmounts this component
   *  directly (no `leaving` phase at all), matching the design's "camera
   *  movement dismisses instantly" rule. */
  leaving?: boolean
}

const CARD_MARGIN_PX = 12
/** Rough card footprint for the flip/clamp math below — the card's real
 *  size depends on its (variable-length) content, but a phone-viewport
 *  callout only needs to avoid gross overflow, not sub-pixel placement. */
const CARD_WIDTH_ESTIMATE_PX = 230
const CARD_HEIGHT_ESTIMATE_PX: Record<InspectResult['kind'], number> = { node: 112, edge: 96 }
const DOT_SIZE_PX = 14
const DOT_RADIUS_PX = DOT_SIZE_PX / 2
/** Gap between the dot's edge and the card's near edge the leader line fills. */
const LEADER_GAP_PX = 14

/** design §3's edge caption, minus the "rim, top face"-style per-edge
 *  attribution the task instructions flag as unbacked by any real kernel
 *  query today (`resolveInspect`'s edge arm reports length + owning-part
 *  label only) — kept as the follow-on spec note for whoever adds that
 *  query later. */
const EDGE_CAPTION = 'double-tap zooms to part'
const PART_CAPTION = 'tap away to dismiss · hold part to isolate'

function clamp(value: number, min: number, max: number): number {
  return Math.min(Math.max(value, min), Math.max(min, max))
}

export function InspectCard({ result, screenX, screenY, containerWidth, containerHeight, leaving = false }: InspectCardProps) {
  const [unit, setUnit] = useState<LengthFormat>(getLengthUnit)
  useEffect(() => subscribeLengthUnit(setUnit), [])

  const cardHeightEstimate = CARD_HEIGHT_ESTIMATE_PX[result.kind]

  const cardTopAbove = screenY - DOT_RADIUS_PX - LEADER_GAP_PX - cardHeightEstimate
  const fitsAbove = cardTopAbove >= CARD_MARGIN_PX
  const cardTop = fitsAbove
    ? cardTopAbove
    : clamp(screenY + DOT_RADIUS_PX + LEADER_GAP_PX, CARD_MARGIN_PX, containerHeight - cardHeightEstimate - CARD_MARGIN_PX)

  const idealLeft = screenX - CARD_WIDTH_ESTIMATE_PX / 2
  const cardLeft = clamp(idealLeft, CARD_MARGIN_PX, containerWidth - CARD_WIDTH_ESTIMATE_PX - CARD_MARGIN_PX)

  // The leader line runs from the dot's outer edge to the card's near edge —
  // whichever of the two is closer to the anchor, given the flip decided
  // above (`fitsAbove`: card above → line runs UP from the dot to the
  // card's bottom edge; card below → line runs DOWN from the dot to the
  // card's top edge).
  const leaderTop = fitsAbove ? cardTop + cardHeightEstimate : screenY + DOT_RADIUS_PX
  const leaderBottom = fitsAbove ? screenY - DOT_RADIUS_PX : cardTop
  const leaderHeight = Math.max(0, leaderBottom - leaderTop)

  const tagColor = result.kind === 'node' && result.tagLabel !== null ? colorForTagPath(result.tagLabel.split(' / ')) : null

  return (
    <div
      className={leaving ? 'shop-inspect-out' : 'shop-inspect-in'}
      style={{ position: 'absolute', inset: 0, pointerEvents: 'none', zIndex: 30 }}
    >
      <div
        aria-hidden="true"
        style={{
          position: 'absolute',
          left: `${screenX}px`,
          top: `${screenY}px`,
          width: `${DOT_SIZE_PX}px`,
          height: `${DOT_SIZE_PX}px`,
          borderRadius: '50%',
          background: 'var(--shop-accent)',
          border: '3px solid #fff',
          boxShadow: '0 1px 4px rgba(0,0,0,.3)',
          transform: 'translate(-50%, -50%)',
        }}
      />
      {leaderHeight > 0 && (
        <div
          aria-hidden="true"
          style={{
            position: 'absolute',
            left: `${screenX}px`,
            top: `${leaderTop}px`,
            width: '1.5px',
            height: `${leaderHeight}px`,
            background: 'var(--shop-accent)',
            transform: 'translateX(-50%)',
          }}
        />
      )}
      <div
        role="status"
        style={{
          position: 'absolute',
          left: `${cardLeft}px`,
          top: `${cardTop}px`,
          minWidth: '210px',
          background: 'var(--surface-card)',
          border: '1px solid var(--shop-hairline-2)',
          borderRadius: '15px',
          boxShadow: '0 10px 28px -10px rgba(60,50,35,.45)',
          padding: '12px 18px',
          fontFamily: 'var(--font-family-ui)',
          color: 'var(--shop-text)',
        }}
      >
        {result.kind === 'edge' ? (
          <>
            <div style={{ display: 'flex', alignItems: 'baseline', gap: '8px', marginBottom: '8px' }}>
              <span style={{ fontSize: '14px', fontWeight: 600, whiteSpace: 'nowrap' }}>{result.partLabel}</span>
              <span
                style={{
                  fontSize: '11px', fontWeight: 500, color: 'var(--shop-text-muted)',
                  background: 'var(--shop-eyebg)', borderRadius: '6px', padding: '2px 7px', whiteSpace: 'nowrap',
                }}
              >
                edge
              </span>
            </div>
            <div style={{ fontFamily: 'var(--font-family-mono)', fontSize: '26px', fontWeight: 600, whiteSpace: 'nowrap' }}>
              {formatLengthIn(result.lengthM, unit)}
            </div>
            <div style={{ fontSize: '11px', color: 'var(--shop-text-faint)', marginTop: '4px', whiteSpace: 'nowrap' }}>
              {EDGE_CAPTION}
            </div>
          </>
        ) : (
          <>
            <div style={{ display: 'flex', alignItems: 'baseline', gap: '8px', marginBottom: '6px' }}>
              <span style={{ fontSize: '14px', fontWeight: 600, whiteSpace: 'nowrap' }}>{result.label}</span>
              {result.tagLabel !== null && tagColor !== null && (
                <span
                  style={{
                    fontSize: '11px', fontWeight: 500, color: tagColor,
                    background: hexToRgba(tagColor, 0.12), borderRadius: '6px', padding: '2px 7px', whiteSpace: 'nowrap',
                  }}
                >
                  {result.tagLabel}
                </span>
              )}
            </div>
            {result.extentsM !== null && (
              <div style={{ fontFamily: 'var(--font-family-mono)', fontSize: '15px', fontWeight: 600, whiteSpace: 'nowrap' }}>
                <span style={{ color: 'var(--shop-axis-l)' }}>L</span> {formatLengthIn(result.extentsM[0], unit)}{' '}
                <span style={{ color: 'var(--shop-axis-w)' }}>W</span> {formatLengthIn(result.extentsM[1], unit)}{' '}
                <span style={{ color: 'var(--shop-axis-h)' }}>H</span> {formatLengthIn(result.extentsM[2], unit)}
              </div>
            )}
            <div style={{ fontSize: '11px', color: 'var(--shop-text-faint)', marginTop: '4px', whiteSpace: 'nowrap' }}>
              {PART_CAPTION}
            </div>
          </>
        )}
      </div>
    </div>
  )
}

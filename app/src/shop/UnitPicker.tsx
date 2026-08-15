/**
 * UnitPicker — Shop Mode's unit-format picker (design_handoff_shop_
 * mode/README.md §7 "Unit picker"): six rows, one per `LengthFormat`,
 * showing just its bare name and an active check. Originally also carried a
 * live per-row preview value + a "set from your region" footnote — Kurt's
 * on-device playtest called for dropping both (rows are now unit name +
 * active check only), so this component no longer takes a preview dimension
 * at all — see `ShopApp.tsx`/`PartsSheet.tsx` for the removal of the
 * `previewDimensionM`/`onPreviewDimensionChange` plumbing that used to feed it.
 *
 * Portrait: bottom sheet. Landscape (design §7): a centered 360px card —
 * `orientation` swaps just the outer container/scrim; `open`/`onClose` and
 * the six-row list below are shared verbatim between the two (container
 * swap only, per the design's own framing — Wave 2 structured this
 * component for exactly that split).
 *
 * Picking a row calls `setLengthUnit` (`settings/units.ts`) directly — the
 * SAME persisted singleton the desktop Settings window's own unit control
 * writes, by design (README "Decisions": "persisted per device" — shared
 * with the desktop deliberately, never a Shop-Mode-local preference).
 */
import { useEffect, useState } from 'react'
import {
  getLengthUnit,
  setLengthUnit,
  subscribe as subscribeLengthUnit,
  LENGTH_FORMATS_BY_SYSTEM,
  LENGTH_FORMAT_NAME,
  type LengthFormat,
} from '../settings/units'
import { CheckIcon } from './icons'
import type { ShopOrientation } from './orientation'

export interface UnitPickerProps {
  open: boolean
  /** Portrait: bottom sheet. Landscape: centered 360px card (design §7). */
  orientation: ShopOrientation
  onClose: () => void
}

/** Metric formats first, then imperial — `LENGTH_FORMATS_BY_SYSTEM`'s own
 *  per-system order, matching the design's row order (Meters/Centimeters/
 *  Millimeters/Architectural/Fractional/Decimal). */
const ALL_FORMATS: LengthFormat[] = [
  ...LENGTH_FORMATS_BY_SYSTEM.metric,
  ...LENGTH_FORMATS_BY_SYSTEM.imperial,
]

export function UnitPicker({ open, orientation, onClose }: UnitPickerProps) {
  const [unit, setUnit] = useState<LengthFormat>(getLengthUnit)
  useEffect(() => subscribeLengthUnit(setUnit), [])

  if (!open) return null

  const isLandscape = orientation === 'landscape'

  return (
    <>
      <div
        data-testid="shop-unit-picker-scrim"
        aria-hidden="true"
        onClick={onClose}
        style={{ position: 'absolute', inset: 0, background: 'rgba(27,26,23,.35)', zIndex: 55 }}
      />
      <div
        role="dialog"
        aria-label="Units"
        style={
          isLandscape
            ? {
                // Landscape (design §7): centered 360px card — top/bottom
                // 20px clear of the safe area rather than pinned to the
                // bottom edge, since a bottom sheet would collide with the
                // side sheet/rail either side of it.
                position: 'absolute', left: '50%', transform: 'translateX(-50%)',
                top: 'max(20px, env(safe-area-inset-top))', bottom: 'max(20px, env(safe-area-inset-bottom))',
                width: '360px', zIndex: 56,
                background: 'var(--surface-sheet)', borderRadius: '18px',
                padding: '12px 10px', boxShadow: '0 18px 48px -14px rgba(27,26,23,.5)',
                overflowY: 'auto',
              }
            : {
                position: 'absolute', left: 0, right: 0, bottom: 0, zIndex: 56,
                background: 'var(--surface-sheet)', borderRadius: '18px 18px 0 0',
                padding: '10px 10px max(20px, calc(env(safe-area-inset-bottom) + 10px))',
                boxShadow: '0 -14px 40px -12px rgba(27,26,23,.5)',
              }
        }
      >
        <div style={{ width: '40px', height: '5px', borderRadius: '3px', background: 'var(--shop-hairline)', margin: '0 auto 10px' }} />
        <div style={{ padding: '0 10px 6px' }}>
          <span style={{ fontFamily: 'var(--font-family-ui)', fontSize: '18px', fontWeight: 600, color: 'var(--shop-text)' }}>Units</span>
        </div>
        {ALL_FORMATS.map((format) => {
          const active = format === unit
          return (
            <button
              key={format}
              type="button"
              aria-label={LENGTH_FORMAT_NAME[format]}
              aria-pressed={active}
              onClick={() => { setLengthUnit(format); onClose() }}
              style={{
                display: 'flex', alignItems: 'center', width: '100%', height: '54px', padding: '0 14px',
                margin: '2px 0', borderRadius: '13px', cursor: 'pointer', textAlign: 'left',
                background: active ? 'var(--shop-picker-active-wash)' : 'transparent',
                border: active ? '1.5px solid var(--shop-accent)' : '1.5px solid transparent',
              }}
            >
              <span style={{ flex: 1, fontFamily: 'var(--font-family-ui)', fontSize: '15px', fontWeight: active ? 600 : 500, color: 'var(--shop-text)' }}>
                {LENGTH_FORMAT_NAME[format]}
              </span>
              {active && (
                <span aria-hidden="true" style={{ marginLeft: '12px', color: 'var(--shop-accent)', display: 'flex' }}>
                  <CheckIcon size={18} />
                </span>
              )}
            </button>
          )
        })}
      </div>
    </>
  )
}

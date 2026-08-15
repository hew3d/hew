/**
 * ViewsSheet — Shop Mode's camera-views picker (playtest finding 12): a
 * dedicated "Views" dock button (portrait toolbar + landscape rail,
 * `ShopApp.tsx`) opening this sheet, offering the seven standard views
 * (ISO/Front/Back/Left/Right/Top/Bottom) plus a Parallel⇄Perspective
 * projection toggle — all wired straight to `ViewportApi.setStandardView`/
 * `toggleProjection`/`getProjection`, which already exist on the api (no
 * `Viewport.tsx` changes needed for this).
 *
 * Same visual family as `UnitPicker.tsx` — portrait bottom sheet, landscape
 * centered 360px card, same scrim/handle/radius/shadow chrome, same
 * "tapping a row both acts AND closes the sheet" convention for the seven
 * view rows. The projection toggle is the one exception: it's a persistent
 * two-way control (SettingsMenu.tsx's Theme segmented control is the same
 * shape), not a one-shot pick, so tapping it toggles WITHOUT closing the
 * sheet — the user may want to compare a view under both projections before
 * dismissing.
 */
import type { StandardView } from '../viewport/Viewport'
import type { Projection } from '../viewport/cameraRig'
import type { ShopOrientation } from './orientation'

export interface ViewsSheetProps {
  open: boolean
  /** Portrait: bottom sheet. Landscape: centered 360px card (same fork
   *  `UnitPicker.tsx`/`ScanSheet.tsx` already use). */
  orientation: ShopOrientation
  onClose: () => void
  onSelectView: (view: StandardView) => void
  /** Current projection (`ShopApp.tsx` mirrors `ViewportApi.getProjection`/
   *  `Viewport`'s own `onProjectionChange` callback into React state) — read
   *  here purely to show which of the two toggle segments is active; this
   *  component never reads the viewport api directly. */
  projection: Projection
  onToggleProjection: () => void
}

/** The seven standard views (design task's own list) in the order they're
 *  listed there — ISO first (the default "orient yourself" view), then the
 *  six axis-aligned ones in opposite-pairs. */
const STANDARD_VIEWS: { view: StandardView; label: string }[] = [
  { view: 'iso', label: 'ISO' },
  { view: 'front', label: 'Front' },
  { view: 'back', label: 'Back' },
  { view: 'left', label: 'Left' },
  { view: 'right', label: 'Right' },
  { view: 'top', label: 'Top' },
  { view: 'bottom', label: 'Bottom' },
]

const PROJECTION_SEGMENTS: { value: Projection; label: string }[] = [
  { value: 'perspective', label: 'Perspective' },
  { value: 'parallel', label: 'Parallel' },
]

export function ViewsSheet({ open, orientation, onClose, onSelectView, projection, onToggleProjection }: ViewsSheetProps) {
  if (!open) return null

  const isLandscape = orientation === 'landscape'

  return (
    <>
      <div
        data-testid="shop-views-scrim"
        aria-hidden="true"
        onClick={onClose}
        style={{ position: 'absolute', inset: 0, background: 'rgba(27,26,23,.35)', zIndex: 55 }}
      />
      <div
        role="dialog"
        aria-label="Views"
        style={
          isLandscape
            ? {
                // Landscape (matches UnitPicker's own §7 fork): centered
                // 360px card, top/bottom clear of the safe area rather than
                // pinned to the bottom edge, since a bottom sheet would
                // collide with the rail/sheet either side of it.
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
          <span style={{ fontFamily: 'var(--font-family-ui)', fontSize: '18px', fontWeight: 600, color: 'var(--shop-text)' }}>Views</span>
        </div>

        {STANDARD_VIEWS.map(({ view, label }) => (
          <button
            key={view}
            type="button"
            aria-label={label}
            onClick={() => { onSelectView(view); onClose() }}
            style={{
              display: 'flex', alignItems: 'center', width: '100%', height: '54px', padding: '0 14px',
              margin: '2px 0', borderRadius: '13px', cursor: 'pointer', textAlign: 'left',
              background: 'transparent', border: '1.5px solid transparent',
            }}
          >
            <span style={{ flex: 1, fontFamily: 'var(--font-family-ui)', fontSize: '15px', fontWeight: 500, color: 'var(--shop-text)' }}>
              {label}
            </span>
          </button>
        ))}

        <div style={{ padding: '10px 14px 4px', display: 'flex', alignItems: 'center', justifyContent: 'space-between', borderTop: '1px solid var(--shop-hairline-2)', marginTop: '4px' }}>
          <span style={{ fontFamily: 'var(--font-family-ui)', fontSize: '14px', fontWeight: 500, color: 'var(--shop-text)' }}>Projection</span>
          <div style={{ display: 'flex', gap: '2px', background: 'var(--shop-eyebg)', borderRadius: '9px', padding: '3px' }}>
            {PROJECTION_SEGMENTS.map(({ value, label }) => {
              const active = projection === value
              return (
                <button
                  key={value}
                  type="button"
                  aria-pressed={active}
                  onClick={() => { if (!active) onToggleProjection() }}
                  style={{
                    fontFamily: 'var(--font-family-ui)', fontSize: '12px',
                    fontWeight: active ? 600 : 500,
                    // --shop-accent-fill/--shop-on-accent — the same "active
                    // segment" pairing SettingsMenu.tsx's Theme control and
                    // ShopApp.tsx's own tool segments use (verified ≥4.5:1
                    // by tokensContrast.test.ts), not the plain unthemed
                    // --shop-accent a text label at this size needs.
                    color: active ? 'var(--shop-on-accent)' : 'var(--shop-text-muted)',
                    background: active ? 'var(--shop-accent-fill)' : 'transparent',
                    border: 'none', borderRadius: '7px', padding: '7px 11px', cursor: 'pointer',
                  }}
                >
                  {label}
                </button>
              )
            })}
          </div>
        </div>
      </div>
    </>
  )
}

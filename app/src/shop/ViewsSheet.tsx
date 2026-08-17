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
 *
 * SPEC.md §2 "Scenes in the Views sheet": a "SCENES" section, FIRST, above
 * the standard views — only when the document has any (an additive prop
 * set, `scenes`/`activeSid`/`drifted`/`onSelectScene`, all optional so
 * every pre-existing caller/test that never heard of Scenes keeps working
 * unchanged). Rows act-and-close exactly like the standard-view rows below
 * them; no thumbnails (SPEC.md §2 "No thumbnails").
 */
import type { StandardView } from '../viewport/Viewport'
import type { Projection } from '../viewport/cameraRig'
import type { ShopOrientation } from './orientation'
import type { SceneEntry } from '../scenes/scenesModel'

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
  /** The document's Scenes, in tab order (SPEC.md §2) — the "SCENES"
   *  section renders only when this is non-empty. Omitted/empty for a
   *  document with none, so every existing caller is unaffected. */
  scenes?: SceneEntry[]
  /** The active Scene's sid, or `null`/absent when none is active — drives
   *  which row's state dot renders filled. */
  activeSid?: number | null
  /** Whether the ACTIVE Scene has drifted — the active row's dot renders as
   *  a ring instead of a filled dot (SPEC.md §2, mirroring the pill). */
  drifted?: boolean
  /** A Scene row's tap (act-and-close, same as a standard-view row). */
  onSelectScene?: (sid: number) => void
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

export function ViewsSheet({
  open,
  orientation,
  onClose,
  onSelectView,
  projection,
  onToggleProjection,
  scenes = [],
  activeSid = null,
  drifted = false,
  onSelectScene,
}: ViewsSheetProps) {
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

        {scenes.length > 0 && (
          <div style={{ padding: '4px 10px 6px', borderBottom: '1px solid var(--shop-hairline-2)', marginBottom: '4px' }}>
            <span
              style={{
                display: 'block', padding: '0 4px 4px', fontFamily: 'var(--font-family-ui)', fontSize: '12px',
                fontWeight: 700, letterSpacing: '0.08em', textTransform: 'uppercase', color: 'var(--shop-text-muted)',
              }}
            >
              Scenes
            </span>
            {scenes.map((entry) => {
              const active = entry.sid === activeSid
              const rowDrifted = active && drifted
              return (
                <button
                  key={entry.sid}
                  type="button"
                  onClick={() => { onSelectScene?.(entry.sid); onClose() }}
                  style={{
                    display: 'flex', alignItems: 'flex-start', width: '100%', minHeight: '54px', padding: '12px 4px',
                    gap: '10px', borderRadius: '13px', cursor: 'pointer', textAlign: 'left',
                    background: 'transparent', border: 'none',
                  }}
                >
                  <span
                    aria-hidden="true"
                    style={{
                      flexShrink: 0, width: '20px', height: '54px', display: 'flex', alignItems: 'center', justifyContent: 'center',
                    }}
                  >
                    <svg width="8" height="8" viewBox="0 0 8 8">
                      {rowDrifted ? (
                        <circle cx="4" cy="4" r="3.25" fill="none" stroke="var(--shop-accent)" strokeWidth="1.5" />
                      ) : (
                        <circle cx="4" cy="4" r="4" fill={active ? 'var(--shop-accent)' : 'transparent'} />
                      )}
                    </svg>
                  </span>
                  <span style={{ flex: 1, minWidth: 0, display: 'flex', flexDirection: 'column', gap: '2px' }}>
                    <span style={{ fontFamily: 'var(--font-family-ui)', fontSize: '16px', fontWeight: 600, color: 'var(--shop-text)' }}>
                      {entry.name}
                    </span>
                    {entry.description !== '' && (
                      <span
                        style={{
                          fontFamily: 'var(--font-family-ui)', fontSize: '13px', color: 'var(--shop-text-muted)',
                          display: '-webkit-box', WebkitLineClamp: 2, WebkitBoxOrient: 'vertical', overflow: 'hidden',
                        }}
                      >
                        {entry.description}
                      </span>
                    )}
                  </span>
                </button>
              )
            })}
          </div>
        )}

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

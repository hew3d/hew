/**
 * MeasurementBox — the docked VCB (value control box),
 * `07_inference_feedback.md`. SketchUp's type-a-number-to-set-dimension
 * field, docked top-right instead of an easily-missed corner field. Net new:
 * today's measurement text only ever reached the bottom status bar as plain
 * text (`onMeasurement`); this is the first real floating box for it.
 *
 * Reuses the *existing* `measurement` text `App.tsx` already receives via
 * `Viewport`'s `onMeasurement` callback — no new Viewport plumbing. The
 * label is derived from the active tool name (a small local map) since
 * `onMeasurement` only ever carried the formatted value, never a label.
 *
 * `frozen` (Tape Measure only, tape-measure-rework part 1): the blinking
 * caret's PRESENCE now specifically means "a typed buffer is live and Enter
 * will act on it"; its ABSENCE on a non-empty value means "a finished
 * reading, kept on screen for reference, not currently editable" — Tape
 * Measure's readout persists after a commit until the tool switches, rather
 * than clearing immediately, and the caret says whether typing right now
 * would do anything.
 */
const VCB_LABEL: Record<string, string> = {
  'Move': 'Distance',
  'Push/Pull': 'Push depth',
  'Rotate': 'Angle',
  'Scale': 'Factor',
  'Tape Measure': 'Distance',
  'Protractor': 'Angle',
  'Follow Me': 'Swept length',
  // Camera ▸ Field of View activates the same 'Zoom' camera mode as
  // Camera ▸ Zoom / the Z shortcut (docs/design/camera.md §2) — the label
  // only ever shows while a value is present (MeasurementBox returns null
  // otherwise), i.e. only while the typed-FOV buffer is actually open.
  'Zoom': 'Field of View',
}

export interface MeasurementBoxProps {
  toolName: string
  value: string
  /** True when `value` is a finished reading kept on screen for reference
   *  rather than a live typed buffer (Tape Measure only) — hides the caret.
   *  Defaults to false (every other tool). */
  frozen?: boolean
  /**
   * Shop-mode playtest finding 4: the editor's top-right docking sits
   * directly under Shop Mode's own ⋯ menu button, hiding the readout behind
   * it in both orientations. `'shop'` (opt-in, default `'editor'` so the
   * desktop editor — the only caller that omits this — is byte-identical)
   * moves the box to a top-CENTER position below Shop's top strip (the same
   * `top: strip-offset + 54px, left: 50%` spot Shop's own isolate banner
   * pill already uses, clear of both the title pill and the ⋯ button in
   * portrait AND landscape) and restyles it to the shop chrome's themed
   * dock pill family (`--shop-dock`/`--shop-dock-text-strong`, matching
   * `topStripPillStyle`) instead of the editor's control-surface chip.
   */
  variant?: 'editor' | 'shop'
  /**
   * Shop-variant, PORTRAIT placement only (playtest fix 5 — the maintainer's
   * own words: "default to the lower right in portrait mode"): px of dock +
   * Parts-sheet height to clear above the safe-area-inset-bottom, so the
   * chip never rides UNDER the fused dock+sheet object as it grows toward
   * 'full'. `ShopApp.tsx` passes `sheetHeightPx + DOCK_ROW_HEIGHT_PX` — the
   * exact live formula its own toast already tracks for the identical
   * reason (`DOCK_ROW_HEIGHT_PX`'s own doc comment). Ignored in LANDSCAPE
   * (its own bottom-left dock — see `orientation` below — sits clear of the
   * bottom-CENTER, width-capped Parts sheet without needing to track its
   * live height) and by the `'editor'` variant.
   */
  bottomOffsetPx?: number
  /**
   * Shop-variant placement fork (playtest fix 5, maintainer's own words:
   * "default to the lower right in portrait mode..."): PORTRAIT docks
   * lower-right, clear of the dock/sheet (`bottomOffsetPx`) — the prior
   * top-center spot could hide behind the centered magnifier loupe.
   * LANDSCAPE (task 5 — a later playtest finding: the original top-center
   * spot still blocked measurements taken above the screen centerline) docks
   * lower-LEFT instead — mirroring portrait's own corner, moved to the left
   * edge so it clears the right rail's tools and the bottom-center,
   * width-capped Parts sheet (`ShopApp.tsx`'s own landscape render). Defaults
   * to `'portrait'`; ignored by the `'editor'` variant. A plain string
   * union — not `ShopOrientation` imported from `shop/orientation.ts` — this
   * shared `viewport/` component doesn't take a dependency on the `shop/`
   * app shell for one small type.
   */
  orientation?: 'portrait' | 'landscape'
}

/** The shared chrome (background/radius/shadow/font) for the `'shop'`
 *  variant's pill, minus positioning — both the portrait (lower-right) and
 *  landscape (lower-left) placements below spread this and add ONLY the
 *  `position`/`top`/`bottom`/`left`/`right`/`transform` that differ. */
const shopChipChromeStyle: React.CSSProperties = {
  display: 'flex',
  alignItems: 'center',
  gap: '8px',
  padding: '11px 15px',
  background: 'var(--shop-dock)',
  borderRadius: 'var(--radius-hud, 15px)',
  boxShadow: '0 6px 18px -8px rgba(27,26,23,.6)',
  fontFamily: 'var(--font-family-ui)',
  whiteSpace: 'nowrap',
  zIndex: 36,
}

export function MeasurementBox({
  toolName,
  value,
  frozen = false,
  variant = 'editor',
  bottomOffsetPx = 0,
  orientation = 'portrait',
}: MeasurementBoxProps) {
  if (value === '') return null
  const label = VCB_LABEL[toolName] ?? 'Value'

  if (variant === 'shop') {
    const positionStyle: React.CSSProperties =
      orientation === 'landscape'
        ? {
            // Landscape (task 5): lower-LEFT — the prior top-center spot
            // (removed) blocked measurements taken above the screen
            // centerline. Moved to the left edge so it clears the right rail's
            // tools (design §5). It must ALSO clear the bottom-CENTER Parts
            // sheet: at narrow landscape widths the width-capped sheet's left
            // edge reaches close to this corner, so it is lifted by the sheet's
            // LIVE height (`bottomOffsetPx`, same mechanism portrait uses —
            // shop-mode playtest adversarial review found a flat offset
            // overlapped the sheet at common device widths) rather than a flat
            // clearance.
            position: 'absolute',
            left: 'calc(env(safe-area-inset-left, 0px) + 14px)',
            bottom: `calc(env(safe-area-inset-bottom, 0px) + ${bottomOffsetPx}px + 14px)`,
          }
        : {
            // Portrait (playtest fix 5): lower-right, clear of the fused
            // dock+Parts-sheet object via `bottomOffsetPx` and the
            // safe-area insets on both edges it's docked to — the prior
            // top-center spot could hide directly behind the centered
            // magnifier loupe.
            position: 'absolute',
            right: 'calc(env(safe-area-inset-right, 0px) + 14px)',
            bottom: `calc(env(safe-area-inset-bottom, 0px) + ${bottomOffsetPx}px + 14px)`,
          }
    return (
      <div style={{ ...shopChipChromeStyle, ...positionStyle }}>
        <span style={{ fontSize: '11px', fontWeight: 500, color: 'var(--shop-dock-text)', whiteSpace: 'nowrap' }}>{label}</span>
        <span
          style={{
            fontFamily: 'var(--font-family-mono)',
            fontSize: 'var(--font-size-measurement, 14px)',
            fontWeight: 600,
            // shop-mode adversarial-review finding 4: --shop-on-accent is
            // cream in BOTH themes (tokens.css's own doc comment — the
            // terracotta working accent stays unthemed on purpose), but this
            // text sits on the now-themed --shop-dock, which is ALSO cream
            // in light mode — cream-on-cream. --shop-dock-text-strong is
            // the token this container's own background is verified against
            // (tokensContrast.test.ts's "dock-text-strong-on-dock" pair, one
            // ratio per theme) — the same pairing `topStripPillStyle` and
            // the isolate banner's label already use on this exact surface.
            color: 'var(--shop-dock-text-strong)',
            whiteSpace: 'nowrap',
          }}
        >
          {value}
          {!frozen && (
            <span aria-hidden="true" className="hew-vcb-caret" style={{ color: 'var(--shop-accent)' }}>|</span>
          )}
        </span>
      </div>
    )
  }

  return (
    <div
      style={{
        position: 'absolute',
        top: '16px',
        right: '16px',
        display: 'flex',
        alignItems: 'center',
        gap: '8px',
        padding: '6px 12px',
        background: 'var(--surface-overlay)',
        border: '1px solid var(--border-strong)',
        borderRadius: 'var(--radius-control, 7px)',
        boxShadow: 'var(--shadow-chip, none)',
        fontFamily: 'var(--font-family-ui)',
        zIndex: 20,
      }}
    >
      <span style={{ fontSize: '11px', color: 'var(--text-faint)', whiteSpace: 'nowrap' }}>{label}</span>
      <span
        style={{
          fontFamily: 'var(--font-family-mono)',
          fontSize: 'var(--font-size-measurement, 14px)',
          fontWeight: 600,
          color: 'var(--text-primary)',
          whiteSpace: 'nowrap',
        }}
      >
        {value}
        {!frozen && (
          <span aria-hidden="true" className="hew-vcb-caret" style={{ color: 'var(--accent-base)' }}>|</span>
        )}
      </span>
    </div>
  )
}

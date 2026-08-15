/**
 * SettingsMenu — Shop Mode's ⋯ panel, anchored top-right below (in the same
 * corner-occupying sense `MenuPanel.tsx` describes) the ⋯ button: a Units
 * row (opens `UnitPicker`), a Theme segmented control (System/Light/Dark),
 * and the permanent GESTURES legend.
 *
 * Formerly `OverflowMenu.tsx`, which also carried the document-action rows
 * (Open…/Open from desktop…/Save a copy/Use full editor) mixed in with these
 * settings. The maintainer approved splitting that combined menu ("Idea 2"):
 * document actions now live in `DocumentMenu.tsx`, anchored to the OTHER top
 * corner (the mark/filename pill), matching the iOS document-app convention
 * (Pages/Files: a tappable title opens a document menu; a separate control
 * holds settings). This file keeps only the settings half.
 *
 * Theme reuses `settings/theme.ts` directly rather than a Shop-Mode-local
 * store — the SAME persisted `hew.settings.theme` singleton (+ cross-window
 * broadcast) the desktop Settings window's own Theme control writes.
 * `theme/applyTheme.ts`'s `initThemeSync()`, wired once in `main.tsx`
 * regardless of which shell mounted, already keeps `<html data-theme>` in
 * sync with whatever this sets — no separate wiring needed here.
 *
 * `ShopApp.tsx` owns `open`/positioning (this renders as a plain child of
 * its relatively-positioned root, not nested inside the small ⋯-button
 * wrapper, so `MenuPanel`'s scrim below can cover the full screen rather
 * than just that button's own box) and every action; this component is
 * otherwise self-contained for its OWN two subscribed settings (length unit
 * label, theme setting).
 */
import { useEffect, useState } from 'react'
import { getThemeSetting, setThemeSetting, subscribe as subscribeTheme, type ThemeSetting } from '../settings/theme'
import { getLengthUnit, subscribe as subscribeLengthUnit, LENGTH_FORMAT_NAME, type LengthFormat } from '../settings/units'
import { ChevronRightIcon } from './icons'
import type { ShopOrientation } from './orientation'
import { MenuPanel } from './MenuPanel'

export interface SettingsMenuProps {
  open: boolean
  /** Portrait: anchored top-right below the ⋯ button, right:12px, no height
   *  cap. Landscape (design §5): same anchor shape but right:16px (+
   *  inset-right, matching the top strip's own landscape edge offset) and
   *  height-capped + scrollable — a 390px-tall landscape viewport has much
   *  less room below the top strip than an 844px-tall portrait one. */
  orientation: ShopOrientation
  onClose: () => void
  onOpenUnitPicker: () => void
}

const THEME_SEGMENTS: { value: ThemeSetting; label: string }[] = [
  { value: 'auto', label: 'System' },
  { value: 'light', label: 'Light' },
  { value: 'dark', label: 'Dark' },
]

export function SettingsMenu({ open, orientation, onClose, onOpenUnitPicker }: SettingsMenuProps) {
  const [unit, setUnit] = useState<LengthFormat>(getLengthUnit)
  useEffect(() => subscribeLengthUnit(setUnit), [])
  const [themeSetting, setThemeSettingState] = useState<ThemeSetting>(getThemeSetting)
  useEffect(() => subscribeTheme(setThemeSettingState), [])

  return (
    <MenuPanel open={open} orientation={orientation} anchor="right" onClose={onClose} scrimTestId="shop-settings-scrim" panelTestId="shop-settings-panel">
      <div style={{ padding: '12px 16px', display: 'flex', flexDirection: 'column', gap: '8px', background: 'var(--shop-eyebg)' }}>
        <span style={sectionLabelStyle}>SETTINGS</span>
        <button
          type="button"
          aria-label="Units"
          onClick={onOpenUnitPicker}
          style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', height: '40px', width: '100%', background: 'none', border: 'none', cursor: 'pointer', padding: 0 }}
        >
          <span style={rowLabelStyle}>Units</span>
          <span style={{ display: 'flex', alignItems: 'center', gap: '6px', fontFamily: 'var(--font-family-mono)', fontSize: '13px', fontWeight: 600, color: 'var(--shop-accent-text)' }}>
            {LENGTH_FORMAT_NAME[unit]}
            <ChevronRightIcon size={13} />
          </span>
        </button>
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', height: '40px' }}>
          <span style={rowLabelStyle}>Theme</span>
          <div style={{ display: 'flex', gap: '2px', background: 'var(--shop-eyebg)', borderRadius: '9px', padding: '3px' }}>
            {THEME_SEGMENTS.map(({ value, label }) => {
              const active = themeSetting === value
              return (
                <button
                  key={value}
                  type="button"
                  aria-pressed={active}
                  onClick={() => setThemeSetting(value)}
                  style={{
                    fontFamily: 'var(--font-family-ui)', fontSize: '12px',
                    fontWeight: active ? 600 : 500,
                    // shop-mode adversarial-review finding 5: this used to
                    // pair cream text with a --shop-dock fill — fine back
                    // when --shop-dock was hardcoded charcoal in both
                    // themes, cream-on-cream once it became themed (light
                    // is near-white). --shop-accent-fill/--shop-on-accent
                    // is the SAME "active chip" pairing every other active
                    // segment in Shop Mode uses (ShopApp.tsx's
                    // `segmentStyle`/`railSegmentStyle`, the isolate
                    // banner's "Show all" button) — unthemed by design
                    // (tokens.css: one working terracotta accent in both
                    // themes) and already verified ≥4.5:1 by
                    // tokensContrast.test.ts's "--shop-on-accent on
                    // --shop-accent-fill" case.
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

      <div style={{ padding: '10px 16px 12px', borderTop: '1px solid var(--shop-hairline-2)' }}>
        <span style={sectionLabelStyle}>GESTURES</span>
        <div style={{ fontFamily: 'var(--font-family-ui)', fontSize: '12px', lineHeight: 1.7, color: 'var(--shop-text-muted)', marginTop: '4px' }}>
          Tap a part — its size · Double-tap — zoom · Hold — isolate<br />
          One finger orbits · Two fingers pan · Pinch zooms
        </div>
      </div>
    </MenuPanel>
  )
}

const sectionLabelStyle: React.CSSProperties = {
  fontFamily: 'var(--font-family-ui)', fontSize: '10px', fontWeight: 700,
  letterSpacing: '0.08em', color: 'var(--shop-text-faint)',
}

const rowLabelStyle: React.CSSProperties = {
  fontFamily: 'var(--font-family-ui)', fontSize: '14px', fontWeight: 500, color: 'var(--shop-text)',
}

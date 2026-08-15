/**
 * MenuPanel — the shared scrim + corner-anchored panel shell behind Shop
 * Mode's two top-strip menus (`DocumentMenu.tsx`, anchored to the pill's
 * top-left corner; `SettingsMenu.tsx`, anchored to the ⋯ button's top-right
 * corner — the maintainer-approved split of what used to be one combined
 * `OverflowMenu.tsx`). Both menus share the exact same corner-occupying
 * anchoring, scrim, and landscape height-cap/scroll behavior; this factors
 * that shell out once rather than duplicating it, so each menu file is left
 * with only its own header/rows.
 *
 * `anchor` picks which top corner the panel occupies and which direction it
 * expands from — 'left' anchors `left` (portrait 12px, landscape
 * `LANDSCAPE_LEFT_OFFSET_CSS`) and expands right; 'right' anchors `right`
 * (portrait 12px, landscape `LANDSCAPE_RIGHT_OFFSET_CSS`) and expands left —
 * mirroring the round-3 playtest's "panel occupies the button's own corner"
 * convention (macOS Notes/Calendar toolbar popovers) to the opposite side of
 * the top strip.
 */
import type { ShopOrientation } from './orientation'
import { TOP_STRIP_OFFSET_CSS, LANDSCAPE_LEFT_OFFSET_CSS, LANDSCAPE_RIGHT_OFFSET_CSS } from './ShopApp'

export interface MenuPanelProps {
  open: boolean
  orientation: ShopOrientation
  anchor: 'left' | 'right'
  onClose: () => void
  /** Distinguishes the two scrims in tests/E2E (`shop-document-scrim` vs
   *  `shop-settings-scrim`) — each menu passes its own. */
  scrimTestId: string
  /** Distinguishes the two panels themselves (`shop-document-panel` vs
   *  `shop-settings-panel`) — lets a test/E2E locate the panel's own
   *  bounding box directly (corner-anchoring assertions) without relying on
   *  a row's own DOM shape. */
  panelTestId: string
  children: React.ReactNode
}

export function MenuPanel({ open, orientation, anchor, onClose, scrimTestId, panelTestId, children }: MenuPanelProps) {
  if (!open) return null

  const isLandscape = orientation === 'landscape'
  const sideOffset = anchor === 'left'
    ? (isLandscape ? LANDSCAPE_LEFT_OFFSET_CSS : '12px')
    : (isLandscape ? LANDSCAPE_RIGHT_OFFSET_CSS : '12px')

  return (
    <>
      {/* Scrim: closes on outside tap — a full-screen sibling of the panel
          below, not nested inside it, so it covers the whole viewport
          rather than just the panel's own box. */}
      <div data-testid={scrimTestId} aria-hidden="true" onClick={onClose} style={{ position: 'absolute', inset: 0, background: 'rgba(27,26,23,.22)', zIndex: 50 }} />
      <div
        data-testid={panelTestId}
        style={{
          position: 'absolute', top: TOP_STRIP_OFFSET_CSS,
          [anchor]: sideOffset,
          width: '288px',
          background: 'var(--surface-card)', borderRadius: 'var(--radius-hud)',
          boxShadow: '0 18px 48px -14px rgba(27,26,23,.5)', border: '1px solid var(--shop-hairline-2)',
          zIndex: 51,
          overflowX: 'hidden',
          // Landscape's much shorter (390px) viewport has far less room
          // below the top strip than portrait's 844px one — cap the panel's
          // own height and let ITS content scroll instead of running off
          // the bottom of the screen.
          maxHeight: isLandscape ? '316px' : undefined,
          overflowY: isLandscape ? 'auto' : 'hidden',
        }}
      >
        {children}
      </div>
    </>
  )
}

/** A single icon + label row, shared by both menus' document-action-style
 *  rows (`DocumentMenu.tsx`'s Open…/Save a copy/etc., `SettingsMenu.tsx` no
 *  longer has any of these itself but keeps the type for consistency). */
export function MenuItem({ icon, label, onClick, last }: { icon: React.ReactNode; label: string; onClick: () => void; last?: boolean }) {
  return (
    <button
      type="button"
      onClick={onClick}
      style={{
        display: 'flex', alignItems: 'center', gap: '12px', width: '100%', height: '52px', padding: '0 16px',
        background: 'none', border: 'none', borderBottom: last ? 'none' : '1px solid var(--shop-hairline-2)',
        fontFamily: 'var(--font-family-ui)', fontSize: '15px', fontWeight: 500, color: 'var(--shop-text)',
        cursor: 'pointer', textAlign: 'left',
      }}
    >
      <span aria-hidden="true" style={{ color: 'var(--shop-text-muted)', display: 'flex', flexShrink: 0 }}>{icon}</span>
      {label}
    </button>
  )
}

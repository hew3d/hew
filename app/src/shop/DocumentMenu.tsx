/**
 * DocumentMenu — Shop Mode's document panel, anchored top-left to the mark/
 * filename pill (`MenuPanel.tsx`'s `anchor="left"`, mirroring `ShopApp.tsx`'s
 * ⋯/`SettingsMenu` top-right corner-occupying convention onto the pill's own
 * corner instead). Holds every document action: Open…, Open from desktop…,
 * Save a copy (.hew) (document-only), View in AR… (document-only, iOS Quick
 * Look candidates only), Use full editor — the half of the former combined
 * `OverflowMenu.tsx` this doesn't cover is `SettingsMenu.tsx` (Units/Theme/
 * Gestures), reachable from the separate ⋯ button.
 *
 * Matches the iOS document-app convention (Pages/Files): a tappable document
 * title opens a document menu, whose header shows the SAME name the pill
 * itself shows — the filename "lives" here while the panel is open, same as
 * the pill hides while `open` (`ShopApp.tsx`'s `!documentMenuOpen` gate on
 * the pill, mirroring the ⋯ button's existing `!settingsMenuOpen` one).
 *
 * `docName === null` (nothing loaded yet) degrades the header to "Shop
 * Mode" and hides the two document-only rows (Save a copy, View in AR…) —
 * Open… and Use full editor stay visible, since this pill/panel is the ONLY
 * way to reach either before a document exists (the empty state's own
 * inline buttons are a second seam onto Open…/scanner, but not onto "Use
 * full editor").
 *
 * "View in AR…" used to be its own dock/rail button (design §"View in AR
 * (Shop Mode, iOS)") — the toolbar reshuffle that put Views/Zoom Extents on
 * either side of the tool group dropped it from both bars, and this row is
 * its new and only home.
 */
import { ArCubeIcon, ClockIcon, FullEditorIcon, HewMark, PrinterIcon, QrIcon, SaveCopyIcon, UploadIcon } from './icons'
import type { ShopOrientation } from './orientation'
import { MenuPanel, MenuItem } from './MenuPanel'

export interface DocumentMenuProps {
  open: boolean
  /** `null` before any document is loaded — drives the header text AND
   *  gates the two document-only rows (Save a copy, View in AR…), same as
   *  the old `hasDocument` prop this replaces (`docName !== null`). */
  docName: string | null
  orientation: ShopOrientation
  onClose: () => void
  onOpen: () => void
  /** Opens `ScanSheet.tsx`'s in-app QR scanner — the second seam a user can
   *  reach "Open on Phone" from (the first is the empty state's own "From
   *  your desktop…" button; both call the exact same handler ShopApp.tsx
   *  wires to each). */
  onOpenScanner: () => void
  /** "Recent models…" — the offline recents list as a sheet, reachable
   *  while a document is open (playtest: getting back to a scanned model
   *  used to need a full PWA relaunch into the empty state). */
  onOpenRecents: () => void
  onSaveCopy: () => void
  /** "Print…" (document-only): the Print sheet — paper/PDF at a drawing
   *  scale (docs/design/printing.md §9c). */
  onPrint: () => void
  /** Gates the "View in AR…" row — `ShopApp.tsx` bundles its own document-
   *  loaded check together with `isArQuickLookCandidate()` (iOS Safari
   *  only, arQuickLook.ts) into this single flag rather than this component
   *  importing browser-sniffing logic directly. */
  showViewInAr: boolean
  /** True while `ShopApp.tsx`'s own `viewInAr()` is mid-export — its
   *  double-tap busy-guard already lives there (`if (arBusy) return`); this
   *  just relabels the row "Preparing…" so re-opening this now-closed menu
   *  during a pending export doesn't show a misleadingly idle label. */
  arBusy: boolean
  onViewInAr: () => void
  onUseFullEditor: () => void
}

export function DocumentMenu({ open, docName, orientation, onClose, onOpen, onOpenScanner, onOpenRecents, onSaveCopy, onPrint, showViewInAr, arBusy, onViewInAr, onUseFullEditor }: DocumentMenuProps) {
  return (
    <MenuPanel open={open} orientation={orientation} anchor="left" onClose={onClose} scrimTestId="shop-document-scrim" panelTestId="shop-document-panel">
      {/* Header: non-interactive title row — where the filename "lives"
          while the pill itself is hidden. */}
      <div style={{ display: 'flex', alignItems: 'center', gap: '8px', padding: '14px 16px', borderBottom: '1px solid var(--shop-hairline-2)' }}>
        <HewMark size={16} />
        <span style={{ fontFamily: 'var(--font-family-ui)', fontSize: '15px', fontWeight: 700, color: 'var(--shop-text)', overflowWrap: 'break-word', wordBreak: 'break-word' }}>
          {docName ?? 'Shop Mode'}
        </span>
      </div>

      <MenuItem icon={<UploadIcon size={19} />} label="Open…" onClick={onOpen} />
      <MenuItem icon={<QrIcon size={19} />} label="Open from desktop…" onClick={onOpenScanner} />
      <MenuItem icon={<ClockIcon size={19} />} label="Recent models…" onClick={onOpenRecents} />
      {docName !== null && <MenuItem icon={<SaveCopyIcon size={19} />} label="Save a copy (.hew)" onClick={onSaveCopy} />}
      {docName !== null && <MenuItem icon={<PrinterIcon size={19} />} label="Print…" onClick={onPrint} />}
      {showViewInAr && <MenuItem icon={<ArCubeIcon size={19} />} label={arBusy ? 'Preparing…' : 'View in AR…'} onClick={onViewInAr} />}
      <MenuItem icon={<FullEditorIcon size={19} />} label="Use full editor" onClick={onUseFullEditor} last />
    </MenuPanel>
  )
}

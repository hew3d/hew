/**
 * RecentsSheet — the offline recents list as a sheet reachable WHILE a
 * document is open (Document menu ▸ "Recent models…"). Same chrome fork as
 * `ViewsSheet`/`UnitPicker`: portrait bottom sheet, landscape centered 360px
 * card, scrim tap closes. Rows act-and-close through `onOpen`.
 */
import { RecentsList } from './RecentsList'
import type { RecentEntry } from '../io/recents'
import type { ShopOrientation } from './orientation'

export interface RecentsSheetProps {
  open: boolean
  orientation: ShopOrientation
  entries: RecentEntry[]
  onClose: () => void
  onOpen: (entry: RecentEntry) => void
}

export function RecentsSheet({ open, orientation, entries, onClose, onOpen }: RecentsSheetProps) {
  if (!open) return null
  const isLandscape = orientation === 'landscape'
  return (
    <>
      <div
        data-testid="shop-recents-scrim"
        aria-hidden="true"
        onClick={onClose}
        style={{ position: 'absolute', inset: 0, background: 'rgba(27,26,23,.35)', zIndex: 55 }}
      />
      <div
        role="dialog"
        aria-label="Recent models"
        style={
          isLandscape
            ? {
                position: 'absolute', left: '50%', transform: 'translateX(-50%)',
                top: 'max(20px, env(safe-area-inset-top))', bottom: 'max(20px, env(safe-area-inset-bottom))',
                width: '360px', zIndex: 56,
                background: 'var(--surface-sheet)', borderRadius: '18px',
                padding: '12px 10px', boxShadow: '0 18px 48px -14px rgba(27,26,23,.5)',
                overflowY: 'auto',
              }
            : {
                position: 'absolute', left: 0, right: 0, bottom: 0, zIndex: 56,
                maxHeight: '70%', overflowY: 'auto',
                background: 'var(--surface-sheet)', borderRadius: '18px 18px 0 0',
                padding: '10px 10px max(20px, calc(env(safe-area-inset-bottom) + 10px))',
                boxShadow: '0 -14px 40px -12px rgba(27,26,23,.5)',
              }
        }
      >
        <div style={{ width: '40px', height: '5px', borderRadius: '3px', background: 'var(--shop-hairline)', margin: '0 auto 10px' }} />
        <div style={{ padding: '0 10px 6px' }}>
          <span style={{ fontFamily: 'var(--font-family-ui)', fontSize: '18px', fontWeight: 600, color: 'var(--shop-text)' }}>Recent models</span>
        </div>
        {entries.length === 0 ? (
          <div style={{ padding: '10px 14px 14px', fontFamily: 'var(--font-family-ui)', fontSize: '14px', color: 'var(--shop-text-muted)' }}>
            No recent models yet — open one from Files or your desktop and it will be kept here for offline use.
          </div>
        ) : (
          <div style={{ display: 'flex', flexDirection: 'column', gap: '8px', padding: '0 4px' }}>
            <RecentsList entries={entries} onOpen={(entry) => { onOpen(entry); onClose() }} />
          </div>
        )}
      </div>
    </>
  )
}

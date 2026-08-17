/**
 * RecentsList — the offline-recents rows Shop Mode shows: a real model
 * thumbnail (or the striped placeholder for entries that predate capture),
 * name, relative time, and part count, chevron. One list, two homes: the
 * empty state (before any document is open) and `RecentsSheet` (playtest:
 * once a document is open — a scanned handoff, say — the only way back to
 * a recent model used to be killing the PWA and relaunching into the empty
 * state; the Document menu's "Recent models…" now opens this list in place).
 */
import type React from 'react'
import { ChevronRightIcon } from './icons'
import { formatRelativeTime } from '../io/relativeTime'
import type { RecentEntry } from '../io/recents'

export interface RecentsListProps {
  entries: RecentEntry[]
  onOpen: (entry: RecentEntry) => void
  /** Cap the rows shown (the empty state shows 5; the sheet shows all). */
  limit?: number
}

const recentRowStyle: React.CSSProperties = {
  display: 'flex', alignItems: 'center', gap: '12px', width: '100%',
  padding: '8px 12px', borderRadius: '13px', border: 'none', cursor: 'pointer',
  background: 'color-mix(in srgb, var(--shop-text) 5%, transparent)', textAlign: 'left', minWidth: 0, minHeight: 'var(--hit-min, 48px)',
}

/** Striped placeholder thumbnail (design §8: "striped thumbnail placeholder —
 *  real model thumbnails when available") — plain CSS, no raster asset.
 *  Fallback for any `RecentEntry` with no `thumbnail` yet. */
const recentThumbStyle: React.CSSProperties = {
  width: '46px', height: '46px', borderRadius: '8px', flexShrink: 0,
  background: 'repeating-linear-gradient(45deg, rgba(196,93,60,.22), rgba(196,93,60,.22) 4px, rgba(196,93,60,.08) 4px, rgba(196,93,60,.08) 8px)',
}

/** The real model thumbnail (playtest finding 4) — same box as
 *  `recentThumbStyle` so swapping between the two never reflows the row;
 *  `objectFit: 'cover'` as a second line of defense alongside the capture's
 *  own center-crop (the stored JPEG is already square). */
const recentThumbImageStyle: React.CSSProperties = {
  width: '46px', height: '46px', borderRadius: '8px', flexShrink: 0, objectFit: 'cover',
}

export function RecentsList({ entries, onOpen, limit }: RecentsListProps) {
  const shown = limit === undefined ? entries : entries.slice(0, limit)
  return (
    <>
      {shown.map((entry) => (
        <button key={entry.id} type="button" className="shop-press" onClick={() => onOpen(entry)} style={recentRowStyle}>
          {entry.thumbnail !== undefined ? (
            <img data-testid="shop-recent-thumb" src={entry.thumbnail} alt="" aria-hidden="true" style={recentThumbImageStyle} />
          ) : (
            <span aria-hidden="true" style={recentThumbStyle} />
          )}
          <span style={{ flex: 1, minWidth: 0, display: 'flex', flexDirection: 'column', gap: '2px' }}>
            <span style={{ fontSize: '15px', fontWeight: 600, color: 'var(--shop-text)', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
              {entry.name}
            </span>
            <span style={{ fontSize: '12px', color: 'var(--shop-text-muted)' }}>
              {formatRelativeTime(entry.timestamp, Date.now())}
              {entry.partCount !== undefined
                ? ` · ${entry.partCount} ${entry.partCount === 1 ? 'part' : 'parts'}`
                : ''}
            </span>
          </span>
          <ChevronRightIcon size={18} style={{ color: 'var(--shop-text-muted)', flexShrink: 0 }} />
        </button>
      ))}
    </>
  )
}

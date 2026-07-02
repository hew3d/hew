/**
 * formatRelativeTime — short relative-time string ("just now", "2 minutes
 * ago", "3 hours ago", falling back to a locale date string beyond ~24h).
 *
 * Extracted from recoveryStore.ts so documentSession.ts's
 * "Edited/Saved <relative time>" indicator (`02_app_shell.md`) can reuse the
 * exact same formatting without documentSession depending on recoveryStore
 * (a more specific feature module) or vice versa. recoveryStore.ts re-exports
 * this as `formatRecoveryTime` for its existing call sites.
 */
export function formatRelativeTime(atMs: number, now: number): string {
  const deltaMs = Math.max(0, now - atMs)
  const deltaSec = Math.floor(deltaMs / 1000)

  if (deltaSec < 45) return 'just now'

  // Round (rather than floor) so e.g. 45s reads as "1 minute ago" instead of
  // "0 minutes ago" — the 45s "just now" cutoff implies the next bucket up.
  const deltaMin = Math.round(deltaSec / 60)
  if (deltaMin < 60) {
    return deltaMin === 1 ? '1 minute ago' : `${deltaMin} minutes ago`
  }

  const deltaHour = Math.round(deltaSec / 3600)
  if (deltaHour < 24) {
    return deltaHour === 1 ? '1 hour ago' : `${deltaHour} hours ago`
  }

  return new Date(atMs).toLocaleString()
}

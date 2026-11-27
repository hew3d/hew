/**
 * UpdateReadyBar — the notice bar a document window shows when an automatic
 * launch check finished downloading (or installing) an update
 * (docs/design D3, in FollowNoticeBar's shape). Never a silent restart:
 *
 *   installed (Mac, AppImage)  "Hew X is installed and will open the next
 *                              time you start Hew."  [Restart Now]
 *   not installed (Windows)    "Hew X is ready to install. Hew will close
 *                              and reopen."  [Install and Restart]
 *
 * plus [What's New] (the update window, read-only) and [Later] (this launch,
 * client-only). Subscribes to the shell's `update-ready` event and asks
 * `update_ready_get` on mount for a window created after the event fired.
 *
 * The restart button asks the shell to close every document through its
 * save prompt first. It stays disabled while that runs; if someone presses
 * Cancel in a save prompt the shell answers "cancelled" and the bar is
 * simply live again, text unchanged.
 */

import { useEffect, useState } from 'react'
import { Trans, useLingui } from '@lingui/react/macro'
import {
  onEvent,
  updateReadyGet,
  updateRestart,
  updateShowNotes,
  updaterAvailable,
  type UpdateReady,
} from './updateIpc'

const BAR_STYLE: React.CSSProperties = {
  background: 'var(--surface-overlay, #2a2a2a)',
  borderBottom: '1px solid var(--border-strong, #4a4a4a)',
  color: 'var(--text-primary, #eee)',
  padding: '8px 16px',
  display: 'flex',
  alignItems: 'center',
  gap: '12px',
  fontFamily: 'var(--font-family-ui, system-ui, sans-serif)',
  fontSize: '13px',
  flexShrink: 0,
  zIndex: 200,
}

const BUTTON_STYLE: React.CSSProperties = {
  padding: '4px 14px',
  background: 'var(--surface-input, #444)',
  color: 'var(--text-primary, #eee)',
  border: '1px solid var(--border-strong, transparent)',
  borderRadius: 'var(--radius-control, 4px)',
  fontSize: '13px',
  fontFamily: 'inherit',
  cursor: 'pointer',
}

const PRIMARY_STYLE: React.CSSProperties = {
  ...BUTTON_STYLE,
  background: 'var(--accent-base, #3a5e9e)',
  color: 'var(--accent-text-strong, #fff)',
  border: 'none',
}

export function UpdateReadyBar() {
  const { t } = useLingui()
  const [ready, setReady] = useState<UpdateReady | null>(null)
  const [later, setLater] = useState(false)
  const [restarting, setRestarting] = useState(false)

  useEffect(() => {
    let cancelled = false
    let unsub: (() => void) | null = null
    void (async () => {
      if (!(await updaterAvailable()) || cancelled) return
      try {
        const fn = await onEvent<UpdateReady>('update-ready', (payload) => {
          if (!cancelled) setReady(payload)
        })
        if (cancelled) fn()
        else unsub = fn
      } catch {
        /* no event bridge: the poll below still works */
      }
      try {
        const current = await updateReadyGet()
        if (!cancelled && current) setReady((prev) => prev ?? current)
      } catch {
        /* nothing pending, or the command is absent */
      }
    })()
    return () => {
      cancelled = true
      unsub?.()
    }
  }, [])

  const restart = () => {
    setRestarting(true)
    // Returns only when the restart did not happen (cancelled, already
    // running, or refused): the bar comes back as it was.
    void updateRestart()
      .catch(() => {})
      .finally(() => setRestarting(false))
  }

  if (later || ready === null) return null
  const version = ready.version
  return (
    <div style={BAR_STYLE} role="status" data-testid="update-ready-bar">
      <span style={{ flex: 1 }}>
        {ready.installed ? (
          <Trans>Hew {version} is installed and will open the next time you start Hew.</Trans>
        ) : (
          <Trans>Hew {version} is ready to install. Hew will close and reopen.</Trans>
        )}
      </span>
      <button
        type="button"
        style={restarting ? { ...PRIMARY_STYLE, opacity: 0.6, cursor: 'default' } : PRIMARY_STYLE}
        data-testid="update-ready-restart"
        disabled={restarting}
        onClick={restart}
      >
        {ready.installed ? t`Restart Now` : t`Install and Restart`}
      </button>
      <button type="button" style={BUTTON_STYLE} data-testid="update-ready-notes" onClick={() => void updateShowNotes().catch(() => {})}>
        {t`What’s New`}
      </button>
      <button type="button" style={BUTTON_STYLE} data-testid="update-ready-later" onClick={() => setLater(true)}>
        {t`Later`}
      </button>
    </div>
  )
}

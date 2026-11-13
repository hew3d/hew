/**
 * The notice a long plugin command shows (docs/agents/PLUGINS.md
 * "Compounds"): nothing for the first quarter second, then a non-modal
 * strip at the top center naming who is working, with Cancel. Hidden as
 * soon as the command ends. While it is up, the input layer ignores the
 * person's gestures, so the strip is the explanation for that.
 */

import { useEffect, useState } from 'react'
import { Trans, useLingui } from '@lingui/react/macro'
import { useBusy } from '../api/busyState'

/** How long a command runs before the notice appears, in milliseconds. */
export const BUSY_NOTICE_DELAY_MS = 250

export function BusyNotice() {
  const busy = useBusy()
  const { t } = useLingui()
  const [shown, setShown] = useState(false)

  useEffect(() => {
    setShown(false)
    if (busy === null) return
    const wait = Math.max(0, BUSY_NOTICE_DELAY_MS - (performance.now() - busy.startedAt))
    const timer = setTimeout(() => setShown(true), wait)
    return () => clearTimeout(timer)
  }, [busy])

  if (busy === null || !shown) return null
  const name = busy.label ?? busy.plugin
  return (
    <div
      role="status"
      data-testid="busy-notice"
      style={{
        position: 'absolute',
        top: '12px',
        left: '50%',
        transform: 'translateX(-50%)',
        display: 'flex',
        alignItems: 'center',
        gap: '12px',
        padding: '8px 12px 8px 16px',
        background: 'var(--surface-overlay)',
        color: 'var(--text-primary)',
        border: '1px solid var(--border-strong)',
        borderRadius: '4px',
        boxShadow: '0 2px 8px rgba(0,0,0,0.4)',
        fontSize: '13px',
        zIndex: 101,
      }}
    >
      <span>{t`${name} is working…`}</span>
      <button type="button" onClick={() => busy.cancel()}>
        <Trans>Cancel</Trans>
      </button>
    </div>
  )
}

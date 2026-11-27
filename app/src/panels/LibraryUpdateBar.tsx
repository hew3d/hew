/**
 * LibraryUpdateBar — the notice under the menu bar when the Library holds a
 * newer version of parts this model uses (v1.5 OW-2). The shape of
 * FollowNoticeBar: not a dialog, blocks nothing. "Later" hides it for what is
 * newer now; a still newer version of the same part brings it back.
 */

import { plural } from '@lingui/core/macro'
import { useLingui } from '@lingui/react/macro'
import { BAR_STYLE, BUTTON_STYLE, PRIMARY_STYLE } from './FollowNoticeBar'

export interface LibraryUpdateBarProps {
  /** Library parts with a newer version (one per Library item). */
  count: number
  onUpdateAll: () => void
  onReview: () => void
  onLater: () => void
  /** An update is running: the buttons rest until it lands. */
  busy?: boolean
}

export function LibraryUpdateBar({ count, onUpdateAll, onReview, onLater, busy = false }: LibraryUpdateBarProps) {
  const { t } = useLingui()
  if (count <= 0) return null
  const message = plural(count, {
    one: '# part has a newer version in the Library.',
    other: '# parts have newer versions in the Library.',
  })
  return (
    <div style={BAR_STYLE} role="status" data-testid="library-update-bar">
      <span style={{ flex: 1 }}>{message}</span>
      <button type="button" style={PRIMARY_STYLE} disabled={busy} data-testid="library-update-all" onClick={onUpdateAll}>
        {t`Update All`}
      </button>
      <button type="button" style={BUTTON_STYLE} disabled={busy} data-testid="library-update-review" onClick={onReview}>
        {t`Review…`}
      </button>
      <button type="button" style={BUTTON_STYLE} data-testid="library-update-later" onClick={onLater}>
        {t`Later`}
      </button>
    </div>
  )
}

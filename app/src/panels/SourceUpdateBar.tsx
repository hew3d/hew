/**
 * SourceUpdateBar — the notice at the top of the Library dialog when a source
 * has something newer (docs/design/shared-library-sources.md 7.5). Modeled on
 * LibraryUpdateBar: not a dialog, blocks nothing. One line says how many
 * items have newer versions at their sources; a followed source gets a line of
 * its own ("<Source> has <N> new and <M> changed items"). Update all, Review…
 * and Later; Later hides the bar until the next check.
 */

import { useLingui } from '@lingui/react/macro'
import { barIsEmpty, followedLine, newerLine, type BarModel } from '../library/sourceModel'

export interface SourceUpdateBarProps {
  model: BarModel
  onUpdateAll: () => void
  onReview: () => void
  onLater: () => void
  /** An update is running: the buttons rest until it lands. */
  busy?: boolean
}

export function SourceUpdateBar({ model, onUpdateAll, onReview, onLater, busy = false }: SourceUpdateBarProps) {
  const { t } = useLingui()
  if (barIsEmpty(model)) return null
  return (
    <div className="hwlib__src-bar" role="status" data-testid="source-update-bar">
      <div className="hwlib__src-bar-lines">
        {model.newer > 0 && <span data-testid="source-update-newer">{newerLine(model.newer)}</span>}
        {model.followed.map((l) => (
          <span key={l.source} data-testid="source-update-followed">{followedLine(l)}</span>
        ))}
      </div>
      <button type="button" className="hwlib__btn-primary" disabled={busy} data-testid="source-update-all" onClick={onUpdateAll}>
        {t`Update all`}
      </button>
      <button type="button" className="hwlib__btn-secondary" disabled={busy} data-testid="source-update-review-button" onClick={onReview}>
        {t`Review…`}
      </button>
      <button type="button" className="hwlib__btn-secondary" data-testid="source-update-later" onClick={onLater}>
        {t`Later`}
      </button>
    </div>
  )
}

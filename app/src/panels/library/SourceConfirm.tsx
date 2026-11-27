/**
 * SourceConfirm — the questions a source action can ask
 * (docs/design/shared-library-sources.md 5.2): "Replace the item in your
 * Library?", "Replace your changes" / "Keep mine", and removing a source.
 * One in-page dialog for both the web modal and the desktop window; it takes
 * Escape itself and stops it, so the Library dialog underneath stays open.
 */

import { useEffect } from 'react'
import { Trans, useLingui } from '@lingui/react/macro'
import type { Confirm } from '../../library/useSourceActions'

export function SourceConfirm({ confirm }: { confirm: Confirm }) {
  const { t } = useLingui()
  const cancel = confirm.kind === 'replace_item' ? confirm.no : confirm.cancel
  useEffect(() => {
    function onKey(e: KeyboardEvent): void {
      if (e.key !== 'Escape') return
      e.preventDefault()
      e.stopPropagation()
      cancel()
    }
    // Capture, so the Library dialog's own Escape (on `document`) never sees it.
    document.addEventListener('keydown', onKey, true)
    return () => document.removeEventListener('keydown', onKey, true)
  }, [cancel])

  const name = confirm.name
  let heading: string
  let body: React.ReactNode
  let buttons: React.ReactNode
  if (confirm.kind === 'replace_item') {
    heading = t`Replace the item in your Library?`
    body = <Trans>Your Library already has “{name}”. Replacing it with the one from the source links the two.</Trans>
    buttons = (
      <>
        <button type="button" className="hwlib__btn-secondary" onClick={confirm.no}>
          <Trans>No</Trans>
        </button>
        <button type="button" className="hwlib__btn-primary" autoFocus onClick={confirm.yes}>
          <Trans>Yes</Trans>
        </button>
      </>
    )
  } else if (confirm.kind === 'replace_changes') {
    heading = t`You changed this item here`
    body = <Trans>“{name}” has a newer version at its source. Taking it replaces the changes you made.</Trans>
    buttons = (
      <>
        <button type="button" className="hwlib__btn-secondary" onClick={confirm.keep}>
          <Trans>Keep mine</Trans>
        </button>
        <button type="button" className="hwlib__btn-primary" autoFocus onClick={confirm.replace}>
          <Trans>Replace your changes</Trans>
        </button>
      </>
    )
  } else {
    heading = t`Remove this source?`
    body = <Trans>“{name}” leaves your list. Items you already added stay in your Library.</Trans>
    buttons = (
      <>
        <button type="button" className="hwlib__btn-secondary" onClick={confirm.cancel}>
          <Trans>Cancel</Trans>
        </button>
        <button type="button" className="hwlib__btn-danger" autoFocus onClick={confirm.confirm}>
          <Trans>Remove source</Trans>
        </button>
      </>
    )
  }
  return (
    <div className="hwlib__src-overlay" onClick={cancel}>
      <div className="hwlib__src-dialog" role="alertdialog" aria-modal="true" aria-label={heading} data-testid="source-confirm" data-kind={confirm.kind} onClick={(e) => e.stopPropagation()}>
        <h2>{heading}</h2>
        <p style={{ margin: 0 }}>{body}</p>
        <div className="hwlib__src-dialog-actions">{buttons}</div>
      </div>
    </div>
  )
}

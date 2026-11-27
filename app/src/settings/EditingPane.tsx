/**
 * EditingPane — the "Editing" settings pane.
 *
 * One preference so far: whether copies are linked. Changes apply instantly
 * — no OK/Cancel. Laid out on the shared form grid (SettingsForm.tsx).
 */

import type { CSSProperties } from 'react'
import { Trans, useLingui } from '@lingui/react/macro'
import { getCopiesLinked, setCopiesLinked, subscribeCopiesLinked } from './editing'
import { useViewSetting } from './useViewSetting'
import { SettingsForm, SettingsRow, SettingsNote } from './SettingsForm'

const choiceLabelStyle: CSSProperties = {
  display: 'flex',
  alignItems: 'center',
  gap: '7px',
  fontSize: '13px',
  color: 'var(--text-primary, #eee)',
  cursor: 'pointer',
}

export function EditingPane() {
  const { t } = useLingui()
  const linked = useViewSetting(getCopiesLinked, subscribeCopiesLinked)
  return (
    <SettingsForm>
      <SettingsRow label={t`Copies`} alignTop>
        <label style={choiceLabelStyle}>
          <input type="checkbox" checked={linked} onChange={(e) => setCopiesLinked(e.target.checked)} />
          <span><Trans>Copies are linked</Trans></span>
        </label>
      </SettingsRow>
      <SettingsNote>
        <Trans>Turn this off to make copies independent; you can still choose a linked copy each time.</Trans>
      </SettingsNote>
    </SettingsForm>
  )
}

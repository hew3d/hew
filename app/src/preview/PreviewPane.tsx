/**
 * PreviewPane — Settings > Preview, in preview builds only: one row per
 * Preview feature (name, state line, switch). A switch applies the next time
 * Hew starts (see `switches.ts`). Exempt from localization: the registry's
 * names and state lines are English only.
 */

import { featureEnabled, previewFeatures } from '.'
import { previewSwitches, switchReadsOn, withSwitch, type StoredSwitches } from './switches'
import { PREVIEW_SETTINGS } from './copy'
import { useViewSetting } from '../settings/useViewSetting'
import { SettingsForm, SettingsNote, SettingsRow } from '../settings/SettingsForm'

const NONE: StoredSwitches = { off: [], on: [] }
const NO_SWITCHES = { get: () => NONE, subscribe: () => () => undefined }

export function PreviewPane() {
  const store = previewSwitches ?? NO_SWITCHES
  const stored = useViewSetting<StoredSwitches>(store.get, store.subscribe)
  const features = previewFeatures()
  return (
    <SettingsForm>
      <SettingsNote>{PREVIEW_SETTINGS.intro}</SettingsNote>
      {features.map((f) => {
        const defaultOn = f.defaultOn !== false
        const on = switchReadsOn(stored, f.id, defaultOn)
        // What this window loaded with, against what is stored now.
        const pending = on !== featureEnabled(f.id)
        return (
          <FeatureRow key={f.id} id={f.id} name={f.name} state={f.state} on={on} defaultOn={defaultOn} pending={pending} />
        )
      })}
    </SettingsForm>
  )
}

function FeatureRow({
  id,
  name,
  state,
  on,
  defaultOn,
  pending,
}: {
  id: string
  name: string
  state: string
  on: boolean
  defaultOn: boolean
  pending: boolean
}) {
  const inputId = `preview-switch-${id}`
  return (
    <>
      <SettingsRow label={name} htmlFor={inputId} alignTop>
        <label style={{ display: 'flex', alignItems: 'center', gap: '7px', cursor: 'pointer' }}>
          <input
            id={inputId}
            type="checkbox"
            role="switch"
            data-preview-switch={id}
            checked={on}
            onChange={(e) => {
              previewSwitches?.set(withSwitch(previewSwitches.get(), id, e.target.checked, defaultOn))
            }}
          />
          <span>{on ? PREVIEW_SETTINGS.on : PREVIEW_SETTINGS.off}</span>
        </label>
      </SettingsRow>
      <SettingsNote>
        {state}
        {pending ? ` ${PREVIEW_SETTINGS.pending(on)}` : ''}
      </SettingsNote>
    </>
  )
}

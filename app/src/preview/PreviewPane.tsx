/**
 * PreviewPane — Settings > Preview, in preview builds only: one row per
 * Preview feature (name, state line, switch). A switch applies the next time
 * Hew starts (see `switches.ts`). Exempt from localization: the registry's
 * names and state lines are English only.
 */

import { featureEnabled, previewFeatures } from '.'
import { previewSwitches } from './switches'
import { PREVIEW_SETTINGS } from './copy'
import { useViewSetting } from '../settings/useViewSetting'
import { SettingsForm, SettingsNote, SettingsRow } from '../settings/SettingsForm'

const NO_SWITCHES = { get: () => [] as readonly string[], subscribe: () => () => undefined }

export function PreviewPane() {
  const store = previewSwitches ?? NO_SWITCHES
  const off = useViewSetting<readonly string[]>(store.get, store.subscribe)
  const features = previewFeatures()
  return (
    <SettingsForm>
      <SettingsNote>{PREVIEW_SETTINGS.intro}</SettingsNote>
      {features.map((f) => {
        const on = !off.includes(f.id)
        // What this window loaded with, against what is stored now.
        const pending = on !== featureEnabled(f.id)
        return (
          <FeatureRow key={f.id} id={f.id} name={f.name} state={f.state} on={on} pending={pending} />
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
  pending,
}: {
  id: string
  name: string
  state: string
  on: boolean
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
              const next = (previewSwitches?.get() ?? []).filter((x) => x !== id)
              previewSwitches?.set(e.target.checked ? next : [...next, id])
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

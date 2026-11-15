/* oxlint-disable lingui/no-unlocalized-strings -- True Curves strings are not yet wrapped for translation: this file is English-only until they are */
/**
 * CornersChip — how three-way corners are built, shown next to the profile
 * chip only while the armed pick has any.
 *
 * Round: "Rolled | Mitered". Chamfer: "Mitered | Flat". The choice is sticky
 * for the session and rides on the next blend as its corner style.
 */

import { useSyncExternalStore } from 'react'
import {
  getFilletSettings,
  getFilletThreeWayCorners,
  setFilletCorners,
  subscribeFilletSettings,
  subscribeFilletThreeWayCorners,
  type FilletCorners,
} from '../tools/filletSettings'

interface Choice {
  label: string
  value: FilletCorners
}

/** The two choices for a profile family. The first is always the default. */
export function cornerChoices(chamferFamily: boolean): [Choice, Choice] {
  return chamferFamily
    ? [
        { label: 'Mitered', value: 'default' },
        { label: 'Flat', value: 'flat' },
      ]
    : [
        { label: 'Rolled', value: 'default' },
        { label: 'Mitered', value: 'mitered' },
      ]
}

export function CornersChip() {
  const settings = useSyncExternalStore(subscribeFilletSettings, getFilletSettings, getFilletSettings)
  const count = useSyncExternalStore(subscribeFilletThreeWayCorners, getFilletThreeWayCorners, getFilletThreeWayCorners)
  if (count <= 0) return null
  const choices = cornerChoices(settings.profile.kind === 'chamfer')
  return (
    <div
      role="radiogroup"
      aria-label="Corners"
      data-testid="fillet-corners-chip"
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: 2,
        padding: '3px',
        background: 'var(--surface-overlay)',
        border: '1px solid var(--border-strong)',
        borderRadius: 'var(--radius-control, 7px)',
        boxShadow: 'var(--shadow-chip, none)',
        fontFamily: 'var(--font-family-ui)',
        fontSize: 12.5,
      }}
    >
      <span style={{ padding: '0 8px', fontSize: 11, color: 'var(--text-faint)' }}>Corners</span>
      {choices.map((c) => {
        const active = settings.corners === c.value
        return (
          <button
            key={c.value}
            type="button"
            role="radio"
            aria-checked={active}
            onClick={() => setFilletCorners(c.value)}
            style={{
              padding: '3px 10px',
              border: 'none',
              borderRadius: 5,
              background: active ? 'var(--accent-base)' : 'transparent',
              color: active ? 'var(--text-on-accent, #fff)' : 'var(--text-primary)',
              fontFamily: 'inherit',
              fontSize: 'inherit',
              cursor: 'pointer',
            }}
          >
            {c.label}
          </button>
        )
      })}
    </div>
  )
}

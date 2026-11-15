/* oxlint-disable lingui/no-unlocalized-strings -- True Curves strings are not yet wrapped for translation: this file is English-only until they are */
/**
 * ProfileChip — the Fillet tool's profile picker, floating at the top of the
 * viewport to the left of the measurement box while the tool is active.
 *
 * Two one-click pills, Round and Chamfer, sit left of the menu button, the
 * active one highlighted. The chip names the current profile; its menu lists Round, Chamfer, Cove and
 * Bullnose, then any further profile entries the caller passes in (the
 * built-in and Library profiles, and Save profile…). Picking one sets the
 * tool's session-sticky profile (`filletSettings`); Alt still swaps Round and
 * Chamfer only. Over a drawn corner, which takes only Round and Chamfer, the
 * other profiles are disabled.
 */

import { useEffect, useRef, useState, useSyncExternalStore } from 'react'
import { useLingui } from '@lingui/react/macro'
import {
  getFilletCornerOnly,
  getFilletSettings,
  profileSizeLabel,
  setFilletProfile,
  subscribeFilletCornerOnly,
  subscribeFilletSettings,
  type FilletProfile,
} from '../tools/filletSettings'

/** One row of the menu beyond the four basic kinds. */
export interface ProfileChipItem {
  /** Stable key. */
  id: string
  label: string
  /** A 32 px thumbnail (an object URL or data URL), when there is one. */
  thumbnail?: string
  /** Picked: the profile to arm. Absent for an action row. */
  profile?: FilletProfile
  /** An action row instead (Save profile…). */
  onSelect?: () => void
  disabled?: boolean
  /** Why a disabled row is disabled (its tooltip). */
  title?: string
  /** A small badge after the label ("Built-in"). */
  badge?: string
}

export interface ProfileChipSection {
  id: string
  heading?: string
  items: ProfileChipItem[]
  /** Shown in place of the rows when there are none ("No saved profiles
   *  yet."); a section without one is hidden while empty. */
  emptyHint?: string
}

export interface ProfileChipProps {
  /** Further sections below the four basics, each set off by a separator. */
  sections?: ProfileChipSection[]
}

const BASICS: Array<{ label: string; profile: FilletProfile }> = [
  { label: 'Round', profile: { kind: 'round' } },
  { label: 'Chamfer', profile: { kind: 'chamfer' } },
  { label: 'Cove', profile: { kind: 'cove' } },
  { label: 'Bullnose', profile: { kind: 'bullnose' } },
]

/** The two one-click pills left of the menu. They stay enabled over a drawn
 *  corner, which takes exactly these two. */
const QUICK_PILLS = BASICS.slice(0, 2)

const pillStyle = (active: boolean): React.CSSProperties => ({
  padding: '6px 10px',
  background: active ? 'var(--accent-soft, rgba(77,144,255,0.18))' : 'var(--surface-overlay)',
  border: `1px solid ${active ? 'var(--accent-base)' : 'var(--border-strong)'}`,
  borderRadius: 'var(--radius-control, 7px)',
  boxShadow: 'var(--shadow-chip, none)',
  color: 'var(--text-primary)',
  fontFamily: 'var(--font-family-ui)',
  fontSize: 12.5,
  fontWeight: active ? 600 : 400,
  cursor: 'pointer',
})

/** The chip's name for a profile. */
export function profileLabel(p: FilletProfile): string {
  switch (p.kind) {
    case 'round':
      return 'Round'
    case 'chamfer':
      return 'Chamfer'
    case 'cove':
      return 'Cove'
    case 'bullnose':
      return 'Bullnose'
    case 'shape':
      return p.name
  }
}

export { profileSizeLabel }

/** The tooltip on a profile a drawn corner can't take. */
const CORNER_ONLY_TITLE = 'Drawn corners can be rounded or chamfered.'

function cornerTakes(p: FilletProfile): boolean {
  return p.kind === 'round' || p.kind === 'chamfer'
}

const itemStyle = (active: boolean, disabled: boolean): React.CSSProperties => ({
  display: 'flex',
  alignItems: 'center',
  gap: 8,
  width: '100%',
  padding: '6px 12px',
  border: 'none',
  background: active ? 'var(--accent-soft, rgba(77,144,255,0.18))' : 'transparent',
  color: disabled ? 'var(--text-faint)' : 'var(--text-primary)',
  fontFamily: 'var(--font-family-ui)',
  fontSize: 12.5,
  textAlign: 'left',
  cursor: disabled ? 'default' : 'pointer',
})

export function ProfileChip({ sections = [] }: ProfileChipProps) {
  const { t } = useLingui()
  const settings = useSyncExternalStore(subscribeFilletSettings, getFilletSettings, getFilletSettings)
  const cornerOnly = useSyncExternalStore(subscribeFilletCornerOnly, getFilletCornerOnly, getFilletCornerOnly)
  const [open, setOpen] = useState(false)
  const rootRef = useRef<HTMLDivElement | null>(null)

  useEffect(() => {
    if (!open) return
    const onDown = (ev: PointerEvent): void => {
      if (rootRef.current !== null && !rootRef.current.contains(ev.target as Node)) setOpen(false)
    }
    const onKey = (ev: KeyboardEvent): void => {
      if (ev.key === 'Escape') setOpen(false)
    }
    window.addEventListener('pointerdown', onDown, true)
    window.addEventListener('keydown', onKey, true)
    return () => {
      window.removeEventListener('pointerdown', onDown, true)
      window.removeEventListener('keydown', onKey, true)
    }
  }, [open])

  const current = settings.profile
  const pick = (p: FilletProfile): void => {
    setFilletProfile(p)
    setOpen(false)
  }
  const isCurrent = (p: FilletProfile): boolean =>
    p.kind === current.kind && (p.kind !== 'shape' || (current.kind === 'shape' && current.id === p.id))

  return (
    <div
      ref={rootRef}
      style={{ position: 'relative', display: 'flex', alignItems: 'center', gap: 6 }}
      data-testid="fillet-profile-chip"
    >
      <div role="group" aria-label="Quick edge shape" style={{ display: 'flex', gap: 4 }}>
        {QUICK_PILLS.map((b) => {
          const active = current.kind === b.profile.kind
          return (
            <button
              key={b.label}
              type="button"
              aria-pressed={active}
              data-testid={`fillet-pill-${b.profile.kind}`}
              onClick={() => pick(b.profile)}
              style={pillStyle(active)}
            >
              {b.label}
            </button>
          )
        })}
      </div>
      <button
        type="button"
        aria-haspopup="menu"
        aria-expanded={open}
        title={t`Choose the shape of the edge`}
        onClick={() => setOpen((o) => !o)}
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 8,
          padding: '6px 12px',
          background: 'var(--surface-overlay)',
          border: '1px solid var(--border-strong)',
          borderRadius: 'var(--radius-control, 7px)',
          boxShadow: 'var(--shadow-chip, none)',
          color: 'var(--text-primary)',
          fontFamily: 'var(--font-family-ui)',
          fontSize: 12.5,
          cursor: 'pointer',
        }}
      >
        <span style={{ fontSize: 11, color: 'var(--text-faint)' }}>Edge</span>
        <span style={{ fontWeight: 600 }}>{profileLabel(current)}</span>
        <span aria-hidden="true" style={{ fontSize: 9, color: 'var(--text-faint)' }}>
          ▾
        </span>
      </button>
      {open && (
        <div
          role="menu"
          style={{
            position: 'absolute',
            top: 'calc(100% + 4px)',
            right: 0,
            minWidth: 200,
            maxHeight: '60vh',
            overflowY: 'auto',
            padding: '4px 0',
            background: 'var(--surface-overlay)',
            border: '1px solid var(--border-strong)',
            borderRadius: 'var(--radius-control, 7px)',
            boxShadow: 'var(--shadow-popover, 0 6px 20px rgba(0,0,0,0.25))',
            zIndex: 40,
          }}
        >
          {BASICS.map((b) => {
            const off = cornerOnly && !cornerTakes(b.profile)
            return (
              <button
                key={b.label}
                type="button"
                role="menuitemradio"
                aria-checked={isCurrent(b.profile)}
                disabled={off}
                title={off ? CORNER_ONLY_TITLE : undefined}
                onClick={() => {
                  if (!off) pick(b.profile)
                }}
                style={itemStyle(isCurrent(b.profile), off)}
              >
                <span style={{ flex: 1 }}>{b.label}</span>
              </button>
            )
          })}
          {sections
            .filter((s) => s.items.length > 0 || s.emptyHint !== undefined)
            .map((s) => (
              <div key={s.id} role="group" aria-label={s.heading}>
                <div style={{ height: 1, margin: '4px 0', background: 'var(--border-hairline)' }} />
                {s.heading !== undefined && (
                  <div style={{ padding: '2px 12px', fontSize: 10.5, color: 'var(--text-faint)' }}>{s.heading}</div>
                )}
                {s.items.length === 0 && (
                  <div style={{ padding: '4px 12px 6px', fontSize: 12, color: 'var(--text-faint)' }}>{s.emptyHint}</div>
                )}
                {s.items.map((it) => {
                  const active = it.profile !== undefined && isCurrent(it.profile)
                  const cornerOff = cornerOnly && it.profile !== undefined && !cornerTakes(it.profile)
                  const off = it.disabled === true || cornerOff
                  return (
                    <button
                      key={it.id}
                      type="button"
                      role={it.profile !== undefined ? 'menuitemradio' : 'menuitem'}
                      aria-checked={it.profile !== undefined ? active : undefined}
                      disabled={off}
                      title={cornerOff ? CORNER_ONLY_TITLE : it.title}
                      onClick={() => {
                        if (off) return
                        if (it.profile !== undefined) pick(it.profile)
                        else {
                          setOpen(false)
                          it.onSelect?.()
                        }
                      }}
                      style={itemStyle(active, off)}
                    >
                      {it.thumbnail !== undefined && (
                        <img src={it.thumbnail} width={32} height={32} alt="" style={{ borderRadius: 4 }} />
                      )}
                      <span style={{ flex: 1 }}>{it.label}</span>
                      {it.badge !== undefined && (
                        <span style={{ fontSize: 10, color: 'var(--text-faint)' }}>{it.badge}</span>
                      )}
                    </button>
                  )
                })}
              </div>
            ))}
        </div>
      )}
    </div>
  )
}

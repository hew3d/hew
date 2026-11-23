/* oxlint-disable lingui/no-unlocalized-strings -- True Curves strings are not yet wrapped for translation: this file is English-only until they are */
import { useContext, useEffect, useId, useLayoutEffect, useRef, useState, useSyncExternalStore, type ReactNode } from 'react'
import { useLingui } from '@lingui/react/macro'
import {
  getFilletCornerOnly,
  getFilletSettings,
  getFilletThreeWayCorners,
  setFilletCorners,
  setFilletProfile,
  setFilletSegmentsOverride,
  subscribeFilletCornerOnly,
  subscribeFilletSettings,
  subscribeFilletThreeWayCorners,
  type FilletCorners,
  type FilletProfile,
} from '../tools/filletSettings'
import {
  clampBlendSegments,
  clampCornerSegments,
  MIN_BLEND_SEGMENTS,
  MAX_BLEND_SEGMENTS,
  MIN_CORNER_SEGMENTS,
  MAX_CORNER_SEGMENTS,
} from '../tools/filletMath'
import { ProfileMenuIcon } from './ProfileMenuIcon'
import { isComposingKey } from '../input/keyboard'
import { ToolOptionsInfo, ToolOptionsExpandedContext } from './ToolOptionsCard'

/** A profile choice or action supplied by the existing library adapter. */
export interface ProfileOptionItem {
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

export interface ProfileOptionSection {
  /** Profile sections expand together; saving actions stay visible. */
  id: string
  heading?: string
  items: ProfileOptionItem[]
  /** Shown in place of the rows when there are none ("No saved profiles
   *  yet."); a section without one is hidden while empty. */
  emptyHint?: string
}

export interface FilletOptionsProps {
  /** Built-in menu, saved-profile and saving-action sections. */
  sections?: ProfileOptionSection[]
}

const BASICS: Array<{ label: string; profile: Exclude<FilletProfile, { kind: 'shape' }> }> = [
  { label: 'Round', profile: { kind: 'round' } },
  { label: 'Chamfer', profile: { kind: 'chamfer' } },
  { label: 'Cove', profile: { kind: 'cove' } },
  { label: 'Bullnose', profile: { kind: 'bullnose' } },
]

const pillStyle = (active: boolean): React.CSSProperties => ({
  padding: '5px 6px',
  background: active ? 'var(--accent-base)' : 'transparent',
  border: `1px solid ${active ? 'var(--accent-base)' : 'var(--border-strong)'}`,
  borderRadius: 'var(--radius-control, 7px)',
  boxShadow: 'var(--shadow-chip, none)',
  color: active ? 'var(--text-on-accent, #fff)' : 'var(--text-primary)',
  fontFamily: 'var(--font-family-ui)',
  fontSize: 'var(--font-size-menu-item)',
  fontWeight: active ? 600 : 400,
  cursor: 'pointer',
})

/** The name shown beside More shapes. */
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
      return p.id === 'builtin:roman-ogee' ? 'Roman Ogee' : p.name
  }
}
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
  background: active ? 'var(--accent-base)' : 'transparent',
  color: disabled ? 'var(--text-faint)' : active ? 'var(--text-on-accent, #fff)' : 'var(--text-primary)',
  fontFamily: 'var(--font-family-ui)',
  fontSize: 'var(--font-size-menu-item)',
  fontWeight: active ? 600 : 400,
  textAlign: 'left',
  cursor: disabled ? 'default' : 'pointer',
})

function ShapeOptions({ sections = [], children }: FilletOptionsProps & { children?: ReactNode }) {
  const { t } = useLingui()
  const settings = useSyncExternalStore(subscribeFilletSettings, getFilletSettings, getFilletSettings)
  const cornerOnly = useSyncExternalStore(subscribeFilletCornerOnly, getFilletCornerOnly, getFilletCornerOnly)
  const [open, setOpen] = useState(false)
  const expanded = useContext(ToolOptionsExpandedContext)
  useEffect(() => { if (!expanded) setOpen(false) }, [expanded])
  const triggerRef = useRef<HTMLButtonElement | null>(null)
  const shapesRef = useRef<HTMLDivElement | null>(null)
  // Leaving the menu by Escape or an action row hands the keyboard back to the
  // tool: parking focus on the trigger would make the card swallow the next
  // Enter, Space or digit meant for the viewport.
  const releaseFocus = (): void => {
    const focused = document.activeElement
    if (focused instanceof HTMLElement && shapesRef.current?.contains(focused)) focused.blur()
  }
  const menuRef = useRef<HTMLDivElement | null>(null)
  const pendingFocus = useRef<'first' | 'last' | null>(null)
  const focusItem = (last = false): boolean => {
    const items = menuRef.current?.querySelectorAll<HTMLButtonElement>('button:not(:disabled)')
    if (!items?.length) return false
    items[last ? items.length - 1 : 0].focus({ preventScroll: true })
    // Focusing while the opening grid is still 0fr must not scroll past its first row.
    if (menuRef.current !== null) menuRef.current.scrollTop = last ? menuRef.current.scrollHeight : 0
    return true
  }
  useLayoutEffect(() => {
    if (!open) pendingFocus.current = null
    else if (pendingFocus.current !== null && focusItem(pendingFocus.current === 'last')) pendingFocus.current = null
  }, [open, sections])
  const menuId = useId()

  useEffect(() => {
    if (!open) return
    const onDown = (ev: PointerEvent): void => {
      const target = ev.target as Node
      if (!menuRef.current?.contains(target) && !triggerRef.current?.contains(target)) setOpen(false)
    }
    const onKey = (ev: KeyboardEvent): void => {
      if (ev.key === 'Escape' && !isComposingKey(ev)) {
        setOpen(false)
        // A focused field owns its rollback even while the menu is open.
        if (ev.target instanceof Element && ev.target.closest('input, textarea, [contenteditable="true"]')) return
        releaseFocus()
        ev.preventDefault()
        ev.stopPropagation()
      }
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

  const renderSections = (profileSections: ProfileOptionSection[]) => (<>
          {profileSections
            .filter((s) => s.items.length > 0 || s.emptyHint !== undefined)
            .map((s) => (
              <div key={s.id} data-profile-section={s.id} role="group" aria-label={s.heading}>
                <div style={{ height: 1, margin: '4px 0', background: 'var(--border-hairline)' }} />
                {s.heading !== undefined && (
                  <div style={{ padding: '2px 12px', fontSize: 10.5, color: 'var(--text-faint)' }}>{s.heading}</div>
                )}
                {s.items.length === 0 && (
                  <div style={{ padding: '4px 12px 6px', fontSize: 'var(--font-size-body)', color: 'var(--text-faint)' }}>{s.emptyHint}</div>
                )}
                <div className={s.items.some(it => it.profile !== undefined) ? 'fillet-profile-choices' : undefined}>
                {s.items.map((it) => {
                  const active = it.profile !== undefined && isCurrent(it.profile)
                  const cornerOff = cornerOnly && it.profile !== undefined && !cornerTakes(it.profile)
                  const off = it.disabled === true || cornerOff
                  return (
                    <button
                      key={it.id}
                      className="fillet-profile-choice"
                      type="button"
                      tabIndex={-1}
                      role={it.profile !== undefined ? 'menuitemradio' : 'menuitem'}
                      aria-checked={it.profile !== undefined ? active : undefined}
                      disabled={off}
                      title={cornerOff ? CORNER_ONLY_TITLE : it.title}
                      onClick={() => {
                        if (off) return
                        if (it.profile !== undefined) {
                          setFilletProfile(it.profile)
                        }
                        else {
                          setOpen(false)
                          releaseFocus()
                          it.onSelect?.()
                        }
                      }}
                      style={{ ...itemStyle(active, off), minWidth: 0, padding: '6px 4px', gap: 5, borderRadius: 5, fontSize: 'var(--font-size-body)',
                        ...(it.profile === undefined ? { color: off ? 'var(--text-faint)' : 'var(--text-secondary)', fontSize: 11, padding: '6px 12px' } : {}) }}
                    >
                      {it.profile?.kind === 'shape' && <ProfileMenuIcon shape={it.profile.shape} thumbnail={it.thumbnail} />}
                      <span className="fillet-profile-label">{it.label}</span>
                      {it.badge !== undefined && it.badge !== s.heading && (
                        <span style={{ fontSize: 10, color: 'var(--text-faint)' }}>{it.badge}</span>
                      )}
                    </button>
                  )
                })}
                </div>
              </div>
            ))}
  </>)

  return (
    <div
      ref={shapesRef}
      className="fillet-shapes"
      style={{ position: 'relative', display: 'flex', flexDirection: 'column', gap: 6 }}
      data-testid="fillet-shape-options"
    >
      <button
        ref={triggerRef}
        type="button"
        aria-haspopup="menu"
        aria-controls={menuId}
        onKeyDown={event => {
          if (isComposingKey(event)) return
          if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
            event.preventDefault()
            event.stopPropagation()
            if (open) focusItem(event.key === 'ArrowUp')
            else { pendingFocus.current = event.key === 'ArrowUp' ? 'last' : 'first'; setOpen(true) }
          }
        }}
        aria-expanded={open}
        title={t`Choose the shape of the edge`}
        onClick={event => {
          if (!open && event.detail === 0) pendingFocus.current = 'first'
          setOpen(o => !o)
        }}
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 8,
          justifyContent: 'space-between',
          padding: '6px 12px',
          background: 'var(--surface-overlay)',
          border: '1px solid var(--border-strong)',
          borderRadius: 'var(--radius-control, 7px)',
          boxShadow: 'var(--shadow-chip, none)',
          color: 'var(--text-primary)',
          fontFamily: 'var(--font-family-ui)',
          fontSize: 'var(--font-size-menu-item)',
          cursor: 'pointer',
        }}
      >
        <span>More shapes</span>
        {current.kind === 'shape' && <span style={{ flex: 1, textAlign: 'right', fontSize: 11, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{profileLabel(current)}</span>}
        <svg aria-hidden="true" width="12" height="12" viewBox="0 0 12 12" style={{ flexShrink: 0, transform: open ? 'rotate(180deg)' : undefined }}>
          <path d="M2.5 4.5 6 8l3.5-3.5" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" />
        </svg>
      </button>
      <div className="fillet-profile-expansion" data-open={open} aria-hidden={!open} inert={!open}
        onTransitionEnd={event => {
          if (!open || event.target !== event.currentTarget || event.propertyName !== 'grid-template-rows') return
          const focused = document.activeElement
          if (focused instanceof HTMLElement && menuRef.current?.contains(focused)) focused.scrollIntoView({ block: 'nearest' })
        }}>
        <div
          id={menuId}
          ref={menuRef}
          className="fillet-profile-menu"
          role="menu"
          aria-label="Additional edge profiles"
          onKeyDown={event => {
            if (isComposingKey(event)) return
            const keys = ['ArrowDown', 'ArrowUp', 'ArrowLeft', 'ArrowRight', 'Home', 'End']
            if (keys.includes(event.key)) {
              event.preventDefault()
              event.stopPropagation()
              const items = Array.from(menuRef.current?.querySelectorAll<HTMLButtonElement>('button:not(:disabled)') ?? [])
              if (!items.length) return
              const index = items.indexOf(document.activeElement as HTMLButtonElement)
              const next = event.key === 'Home' ? 0 : event.key === 'End' ? items.length - 1
                : (index + (event.key === 'ArrowUp' || event.key === 'ArrowLeft' ? -1 : 1) + items.length) % items.length
              items[next].focus()
            } else if (event.key === 'Tab') {
              setOpen(false)
              triggerRef.current?.focus()
            }
          }}
        >
          {renderSections(sections)}
        </div>
      </div>
      <OptionHeading label="Shape" info="Choose the shape of the edge." marginBottom={0} />
      <div className="fillet-primary-choices" role="group" aria-label="Quick edge shape" style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 4 }}>
        {BASICS.map((b) => {
          const active = current.kind === b.profile.kind
          return (
            <button
              key={b.label}
              className="fillet-profile-choice"
              type="button"
              aria-pressed={active}
              disabled={cornerOnly && !cornerTakes(b.profile)}
              title={cornerOnly && !cornerTakes(b.profile) ? CORNER_ONLY_TITLE : undefined}
              onClick={() => pick(b.profile)}
              style={{ ...pillStyle(active), display: 'flex', alignItems: 'center', gap: 5, padding: '6px 4px', fontSize: 'var(--font-size-body)', minWidth: 0, textAlign: 'left', color: cornerOnly && !cornerTakes(b.profile) ? 'var(--text-faint)' : pillStyle(active).color }}
            >
              <ProfileMenuIcon kind={b.profile.kind} /><span className="fillet-profile-label">{b.label}</span>
            </button>
          )
        })}
      </div>
      {children}
    </div>
  )
}


function OptionHeading({ label, info, marginBottom = 5 }: { label: string; info: string; marginBottom?: number }) {
  return <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom, fontSize: 'var(--font-size-body)' }}>
    <span>{label}</span><ToolOptionsInfo label={label}>{info}</ToolOptionsInfo>
  </div>
}

const MODIFIER_KEYS = ['Shift', 'Alt', 'Control', 'Meta']

/** Facets owns a draft while focused; every accepted value still uses the tool's store. */
function FacetsField({ cornerOnly, value }: { cornerOnly: boolean; value: number | null }) {
  const [draft, setDraft] = useState<string | null>(null)
  const before = useRef<number | null>(value)
  const cancelled = useRef(false)
  const edited = useRef(false)
  const clamp = cornerOnly ? clampCornerSegments : clampBlendSegments
  const min = cornerOnly ? MIN_CORNER_SEGMENTS : MIN_BLEND_SEGMENTS
  const max = cornerOnly ? MAX_CORNER_SEGMENTS : MAX_BLEND_SEGMENTS
  // Shown for the current pick: a count stored over a drawn corner can exceed
  // what an ordinary edge takes. Display only; the store is not rewritten.
  const shown = value === null ? '' : String(clamp(value))
  const parse = (text: string): number | null | undefined => {
    if (text.trim() === '') return null
    if (!/^[+-]?\d+$/.test(text.trim())) return undefined
    const n = Number(text)
    return Number.isFinite(n) ? n : undefined
  }
  const finish = (): void => {
    if (!cancelled.current && edited.current && draft !== null) {
      const n = parse(draft)
      // An unparsable draft keeps the last valid count already applied live.
      if (n !== undefined) setFilletSegmentsOverride(n === null ? null : clamp(n))
    }
    cancelled.current = false
    edited.current = false
    setDraft(null)
  }
  return <input aria-label="Facets" type="text" inputMode="numeric" placeholder="Auto"
    value={draft ?? shown}
    // The draft starts at the first keystroke: until then the field keeps
    // following the pick, which can change under a focused field.
    onFocus={() => { before.current = value; cancelled.current = false; edited.current = false }}
    onChange={(ev) => {
      const text = ev.target.value
      edited.current = true
      setDraft(text)
      const n = parse(text)
      // Intermediate digits can be outside the range (1 on the way to 12).
      // Keep the preview steady until a valid count exists or the edit ends.
      if (n === null || (n !== undefined && n >= min && n <= max)) setFilletSegmentsOverride(n)
    }}
    onBlur={finish}
    onKeyDown={(ev) => {
      // A bare modifier must still reach the viewport's held-modifier tracking
      // (its keyup does); the tool ignores keys while a field has focus.
      if (MODIFIER_KEYS.includes(ev.key)) return
      ev.stopPropagation()
      if (isComposingKey(ev)) return
      if (ev.key === 'Escape') {
        ev.preventDefault()
        cancelled.current = true
        setFilletSegmentsOverride(before.current)
        ev.currentTarget.blur()
      } else if (ev.key === 'Enter') {
        ev.preventDefault()
        ev.currentTarget.blur()
      }
    }}
    style={{ width: '100%', boxSizing: 'border-box', padding: '5px 8px',
      background: 'var(--surface-input)', border: '1px solid var(--border-strong)',
      borderRadius: 'var(--radius-control, 7px)', color: 'var(--text-primary)', font: 'inherit', fontSize: 'var(--font-size-menu-item)' }} />
}

export function cornerChoices(chamferFamily: boolean): Array<{ label: string; value: FilletCorners }> {
  return chamferFamily
    ? [{ label: 'Mitered', value: 'default' }, { label: 'Flat', value: 'flat' }]
    : [{ label: 'Rolled', value: 'default' }, { label: 'Mitered', value: 'mitered' }]
}

export function FilletOptions({ sections = [] }: FilletOptionsProps) {
  const settings = useSyncExternalStore(subscribeFilletSettings, getFilletSettings, getFilletSettings)
  const cornerOnly = useSyncExternalStore(subscribeFilletCornerOnly, getFilletCornerOnly, getFilletCornerOnly)
  const corners = useSyncExternalStore(subscribeFilletThreeWayCorners, getFilletThreeWayCorners, getFilletThreeWayCorners)
  const chamfer = settings.profile.kind === 'chamfer'
  return <div className="fillet-options">
    <ShapeOptions sections={sections}>
      <div className="fillet-main-settings">
        {!chamfer && <div>
          <OptionHeading label="Facets" info="Choose how many flat faces make the curve, or leave it on Auto and Hew picks." />
          <FacetsField cornerOnly={cornerOnly} value={settings.segmentsOverride} />
        </div>}
        {corners > 0 && <div>
          <OptionHeading label="Corners" info="Choose how three rounded or chamfered edges meet." />
          <div role="radiogroup" aria-label="Corners" style={{ display: 'flex', gap: 4 }}>
            {cornerChoices(chamfer).map((c) => <button key={c.value} type="button" role="radio"
              aria-checked={settings.corners === c.value} onClick={() => setFilletCorners(c.value)}
              style={{ ...pillStyle(settings.corners === c.value), flex: 1 }}>{c.label}</button>)}
          </div>
        </div>}
      </div>
    </ShapeOptions>
  </div>
}

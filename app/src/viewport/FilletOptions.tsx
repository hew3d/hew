/* oxlint-disable lingui/no-unlocalized-strings -- True Curves strings are not yet wrapped for translation: this file is English-only until they are */
import { useContext, useEffect, useLayoutEffect, useRef, useState, useSyncExternalStore } from 'react'
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
import { isComposingKey } from '../input/keyboard'
import { ToolOptionsInfo, ToolOptionsExpandedContext } from './ToolOptionsCard'

/** One row of the menu beyond the four basic kinds. */
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
  id: string
  heading?: string
  items: ProfileOptionItem[]
  /** Shown in place of the rows when there are none ("No saved profiles
   *  yet."); a section without one is hidden while empty. */
  emptyHint?: string
}

export interface FilletOptionsProps {
  /** Further sections below the four basics, each set off by a separator. */
  sections?: ProfileOptionSection[]
}

const BASICS: Array<{ label: string; profile: FilletProfile }> = [
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
  fontSize: 12.5,
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
      return p.name
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
  fontSize: 12.5,
  textAlign: 'left',
  cursor: disabled ? 'default' : 'pointer',
})

function ShapeOptions({ sections = [] }: FilletOptionsProps) {
  const { t } = useLingui()
  const settings = useSyncExternalStore(subscribeFilletSettings, getFilletSettings, getFilletSettings)
  const cornerOnly = useSyncExternalStore(subscribeFilletCornerOnly, getFilletCornerOnly, getFilletCornerOnly)
  const [open, setOpen] = useState(false)
  const expanded = useContext(ToolOptionsExpandedContext)
  useEffect(() => { if (!expanded) setOpen(false) }, [expanded])
  const rootRef = useRef<HTMLDivElement | null>(null)
  const [menuBounds, setMenuBounds] = useState({ left: 0, bottom: 0, width: 200, maxHeight: 0 })
  useLayoutEffect(() => {
    if (!open) return
    const measure = (): void => {
      const root = rootRef.current
      if (root === null) return
      const r = root.getBoundingClientRect()
      const card = root.closest('[data-testid="tool-options-card"]') as HTMLElement | null
      const viewport = card?.offsetParent
      const top = viewport instanceof HTMLElement ? viewport.getBoundingClientRect().top : 0
      const next = { left: r.left, bottom: window.innerHeight - r.top + 4, width: r.width,
        maxHeight: Math.max(0, Math.min(window.innerHeight * 0.6, r.top - top - 20)) }
      setMenuBounds(previous => Object.keys(next).every(key => previous[key as keyof typeof next] === next[key as keyof typeof next]) ? previous : next)
    }
    measure()
    window.addEventListener('resize', measure)
    window.addEventListener('scroll', measure, true)
    const observer = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(measure)
    if (rootRef.current !== null) observer?.observe(rootRef.current)
    return () => {
      window.removeEventListener('resize', measure)
      window.removeEventListener('scroll', measure, true)
      observer?.disconnect()
    }
  }, [open])

  useEffect(() => {
    if (!open) return
    const onDown = (ev: PointerEvent): void => {
      if (rootRef.current !== null && !rootRef.current.contains(ev.target as Node)) setOpen(false)
    }
    const onKey = (ev: KeyboardEvent): void => {
      if (ev.key === 'Escape' && !isComposingKey(ev)) {
        setOpen(false)
        // A focused field owns its rollback even while the menu is open.
        if (ev.target instanceof Element && ev.target.closest('input, textarea, [contenteditable="true"]')) return
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

  return (
    <div
      ref={rootRef}
      style={{ position: 'relative', display: 'flex', flexDirection: 'column', gap: 6 }}
      data-testid="fillet-shape-options"
    >
      <div role="group" aria-label="Quick edge shape" style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 4 }}>
        {BASICS.map((b) => {
          const active = current.kind === b.profile.kind
          return (
            <button
              key={b.label}
              type="button"
              aria-pressed={active}
              disabled={cornerOnly && !cornerTakes(b.profile)}
              title={cornerOnly && !cornerTakes(b.profile) ? CORNER_ONLY_TITLE : undefined}
              onClick={() => pick(b.profile)}
              style={{ ...pillStyle(active), color: cornerOnly && !cornerTakes(b.profile) ? 'var(--text-faint)' : pillStyle(active).color }}
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
        <span style={{ fontSize: 11, color: 'var(--text-faint)' }}>More shapes</span>
        <span style={{ fontWeight: 600 }}>{profileLabel(current)}</span>
        <span aria-hidden="true" style={{ fontSize: 9, color: 'var(--text-faint)' }}>
          ▾
        </span>
      </button>
      {open && (
        <div
          role="menu"
          style={{
            // Fixed positioning escapes the card's scrolling body without a new window.
            position: 'fixed',
            bottom: menuBounds.bottom,
            left: menuBounds.left,
            width: menuBounds.width,
            boxSizing: 'border-box',
            maxHeight: menuBounds.maxHeight,
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


function OptionHeading({ label, info }: { label: string; info: string }) {
  return <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 5, fontSize: 12 }}>
    <span>{label}</span><ToolOptionsInfo label={label}>{info}</ToolOptionsInfo>
  </div>
}

/** Facets owns a draft while focused; every accepted value still uses the tool's store. */
function FacetsField({ cornerOnly, value }: { cornerOnly: boolean; value: number | null }) {
  const [draft, setDraft] = useState<string | null>(null)
  const before = useRef<number | null>(value)
  const cancelled = useRef(false)
  const clamp = cornerOnly ? clampCornerSegments : clampBlendSegments
  const min = cornerOnly ? MIN_CORNER_SEGMENTS : MIN_BLEND_SEGMENTS
  const max = cornerOnly ? MAX_CORNER_SEGMENTS : MAX_BLEND_SEGMENTS
  const parse = (text: string): number | null | undefined => {
    if (text.trim() === '') return null
    if (!/^[+-]?\d+$/.test(text.trim())) return undefined
    const n = Number(text)
    return Number.isFinite(n) ? n : undefined
  }
  const finish = (): void => {
    if (!cancelled.current && draft !== null) {
      const n = parse(draft)
      setFilletSegmentsOverride(n === undefined ? before.current : n === null ? null : clamp(n))
    }
    cancelled.current = false
    setDraft(null)
  }
  return <input aria-label="Facets" type="text" inputMode="numeric" placeholder="Auto"
    value={draft ?? (value === null ? '' : String(value))}
    onFocus={() => { before.current = value; cancelled.current = false; setDraft(value === null ? '' : String(value)) }}
    onChange={(ev) => {
      const text = ev.target.value
      setDraft(text)
      const n = parse(text)
      // Intermediate digits can be outside the range (1 on the way to 12).
      // Keep the preview steady until a valid count exists or the edit ends.
      if (n === null || (n !== undefined && n >= min && n <= max)) setFilletSegmentsOverride(n)
    }}
    onBlur={finish}
    onKeyDown={(ev) => {
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
      borderRadius: 'var(--radius-control, 7px)', color: 'var(--text-primary)', font: 'inherit', fontSize: 12.5 }} />
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
  return <div style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
    <div>
      <OptionHeading label="Shape" info="Choose the shape of the edge." />
      <ShapeOptions sections={sections} />
    </div>
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
}

/* oxlint-disable lingui/no-unlocalized-strings -- True Curves strings are not yet wrapped for translation: this file is English-only until they are */
import { useId, useRef, useState, useSyncExternalStore } from 'react'
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
import { ProfileIcon } from './ProfileIcon'
import { FilletDimensionControl } from './FilletDimensionControl'
import { ToolOptionsInfo } from './ToolOptionsCard'

/** One profile or action in the inline library beyond the four basic kinds. */
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

/** The active profile name retained when the inline library is collapsed. */
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


function ShapeOptions({ sections = [] }: FilletOptionsProps) {
  const { t } = useLingui()
  const settings = useSyncExternalStore(subscribeFilletSettings, getFilletSettings, getFilletSettings)
  const cornerOnly = useSyncExternalStore(subscribeFilletCornerOnly, getFilletCornerOnly, getFilletCornerOnly)
  const [open, setOpen] = useState(false)
  const gridId = useId()
  const current = settings.profile
  const isCurrent = (p: FilletProfile): boolean =>
    p.kind === current.kind && (p.kind !== 'shape' || (current.kind === 'shape' && current.id === p.id))
  const buttonStyle = (active: boolean): React.CSSProperties => ({
    ...pillStyle(active), display: 'flex', alignItems: 'center', gap: 6,
    minHeight: 36, minWidth: 0, textAlign: 'left', overflowWrap: 'anywhere',
  })
  return (
    <div className="fillet-shapes" data-testid="fillet-shape-options">
      <div role="group" aria-label="Quick edge shape" className="fillet-profile-grid">
        {BASICS.map((b) => {
          const off = cornerOnly && !cornerTakes(b.profile)
          return <button key={b.label} type="button" aria-pressed={isCurrent(b.profile)}
            disabled={off} title={off ? CORNER_ONLY_TITLE : undefined}
            onClick={() => setFilletProfile(b.profile)} style={buttonStyle(isCurrent(b.profile))}>
            <ProfileIcon kind={b.profile.kind} />{b.label}
          </button>
        })}
      </div>
      <div className="fillet-profile-expansion" data-open={open} aria-hidden={!open} inert={!open}>
        <div id={gridId} className="fillet-profile-scroll" role="group" aria-label="Additional edge shapes">
          {sections.filter((s) => s.items.length > 0 || s.emptyHint !== undefined).map((s) => (
            <div key={s.id} role="group" aria-label={s.heading}>
              {s.heading !== undefined && <div className="fillet-profile-heading">{s.heading}</div>}
              {s.items.length === 0 && <div className="fillet-profile-heading">{s.emptyHint}</div>}
              <div className="fillet-profile-grid">
                {s.items.map((it) => {
                  const active = it.profile !== undefined && isCurrent(it.profile)
                  const cornerOff = cornerOnly && it.profile !== undefined && !cornerTakes(it.profile)
                  return <button key={it.id} type="button"
                    aria-pressed={it.profile !== undefined ? active : undefined}
                    disabled={it.disabled === true || cornerOff}
                    title={cornerOff ? CORNER_ONLY_TITLE : it.title}
                    onClick={() => { if (it.profile !== undefined) setFilletProfile(it.profile); else it.onSelect?.() }}
                    style={buttonStyle(active)}>
                    {it.thumbnail !== undefined && <img src={it.thumbnail} width={32} height={32} alt="" style={{ flexShrink: 0, borderRadius: 4 }} />}
                    <span>{it.label}{it.badge !== undefined && <small className="fillet-profile-badge">{it.badge}</small>}</span>
                  </button>
                })}
              </div>
            </div>
          ))}
        </div>
      </div>
      <button type="button" className="fillet-expansion-toggle" aria-expanded={open} aria-controls={gridId}
        title={t`Choose the shape of the edge`} onClick={() => setOpen(!open)}>
        {open ? 'Less tools ▼' : 'More tools ▲'}
      </button>
      {!open && current.kind === 'shape' && <div className="fillet-active-profile" role="status">{profileLabel(current)}</div>}
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
  return <div className="fillet-options">
    <div className="fillet-shape-section">
      <OptionHeading label="Shape" info="Choose the shape of the edge." />
      <ShapeOptions sections={sections} />
    </div>
    <FilletDimensionControl sections={sections} />
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

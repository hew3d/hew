/* oxlint-disable lingui/no-unlocalized-strings -- Preview-only UI copy */
import { useLayoutEffect, useId, useRef, useState, useSyncExternalStore } from 'react'
import { beginFilletDimension, getFilletDimension, subscribeFilletDimension, type DimensionGesture } from '../tools/fillet/dimensionStore'
import { parseCurveInput } from '../tools/curveInput'
import { formatLength, getLengthUnit, subscribe as subscribeUnits } from '../settings/units'
import { isComposingKey } from '../input/keyboard'
import { ProfileIcon, profileOutline } from './ProfileIcon'
import type { FilletProfile } from '../tools/filletSettings'

/** Non-interactive indicators: the workpiece frame stays fixed as the cut grows. */
function TreatmentIndicator({ profile, larger, label, alongEdge }: {
  profile: FilletProfile; larger: boolean; label: string; alongEdge: boolean
}) {
  return <span role="img" aria-label={label} className="fillet-dimension-indicator">
    <span aria-hidden="true" style={{ display: 'inline-flex', transform: `scale(${alongEdge || larger ? 1 : 0.65})`, transformOrigin: 'bottom left' }}>
      <ProfileIcon kind={profile.kind} shape={profile.kind === 'shape' ? profile.shape : undefined} />
    </span>
    {alongEdge && <svg aria-hidden="true" viewBox="0 0 24 8" width="24" height="8" style={{ position: 'absolute', bottom: 0, left: 0 }}>
      <path d="M1 6H23" stroke="currentColor" />
      <path d={larger ? 'M18 1V7' : 'M6 1V7'} stroke="currentColor" strokeWidth="2" />
    </svg>}
  </span>
}

export function FilletDimensionControl() {
  const snapshot = useSyncExternalStore(subscribeFilletDimension, getFilletDimension, getFilletDimension)
  // Reformat the numeric input if document units change; meters never round-trip through display text.
  const unit = useSyncExternalStore(subscribeUnits, getLengthUnit, getLengthUnit)
  const id = useId()
  const [draft, setDraft] = useState<string | null>(null)
  const [travel, setTravel] = useState(0)
  const [sensitivity, setSensitivity] = useState<'Fine' | 'Normal' | 'Coarse'>('Normal')
  const gain = { Fine: 0.25, Normal: 1, Coarse: 2 }[sensitivity]
  const gestureGain = useRef(1)
  const gesture = useRef<DimensionGesture | null>(null)
  const baseline = useRef(0)
  const invalid = useRef(false)
  const identity = snapshot.identity
  useLayoutEffect(() => {
    setDraft(null)
    setTravel(0)
    invalid.current = false
    return () => { gesture.current?.cancel(); gesture.current = null }
  }, [identity])
  const start = (input: 'slider' | 'typed' = 'slider'): boolean => {
    if (gesture.current !== null) return true
    baseline.current = snapshot.value ?? 0
    gestureGain.current = gain
    gesture.current = beginFilletDimension(identity, input)
    return gesture.current !== null
  }
  const finish = (cancel = false): void => {
    const active = gesture.current
    gesture.current = null
    if (cancel || invalid.current) active?.cancel()
    else active?.finish()
    invalid.current = false
    setDraft(null)
    setTravel(0)
  }
  const profile = snapshot.profile
  const reason = snapshot.reason ?? (profile?.kind === 'shape' && profileOutline(profile.shape) === null ? 'Profile outline unavailable; use Measurements for now.' : null)
  if (reason !== null || snapshot.value === null || profile === null) {
    return <p className="fillet-dimension-unavailable">{reason}</p>
  }
  const local = snapshot.grip !== null && snapshot.grip.kind !== 'radius'
  const noun = snapshot.label.toLowerCase()
  const alongEdge = snapshot.label === 'Stop' || snapshot.label === 'Position'
  const smaller = snapshot.label === 'Stop' ? 'Shorter stop' : snapshot.label === 'Position' ? 'Earlier radius point' : snapshot.label === 'Distance' ? 'Shorter chamfer' : `Smaller ${noun}`
  const larger = snapshot.label === 'Stop' ? 'Longer stop' : snapshot.label === 'Position' ? 'Later radius point' : snapshot.label === 'Distance' ? 'Longer chamfer' : `Larger ${noun}`
  const context = local ? ' at active grip' : ''
  return <div className="fillet-dimension-control">
    <label htmlFor={id}>{snapshot.label}{local ? ' · active grip' : ''}</label>
    <div className="fillet-dimension-slider">
      <TreatmentIndicator profile={profile} larger={false} label={smaller + context} alongEdge={alongEdge} />
      <input type="range" aria-label={`Adjust ${noun}${context}`} min={-1} max={1} step={0.01} value={travel}
        aria-valuetext={formatLength(snapshot.value)}
        onPointerDown={(ev) => { if (start()) ev.currentTarget.setPointerCapture(ev.pointerId) }}
        onChange={(ev) => {
          if (!start()) return
          const position = Number(ev.target.value)
          setTravel(position)
          // A relative input span, never a bound on model geometry. Recenter on
          // release: successive strokes extend the available travel indefinitely.
          const zeroStep = parseCurveInput('1', unit)
          const scale = baseline.current > 0 ? baseline.current : 'kind' in zeroStep && zeroStep.kind === 'length' ? zeroStep.meters : 0
          const value = baseline.current > 0 ? baseline.current * Math.pow(2, position * gestureGain.current) : Math.max(0, position * scale * gestureGain.current)
          gesture.current?.update(value)
        }}
        onPointerUp={() => finish()} onPointerCancel={() => finish(true)}
        onLostPointerCapture={() => { if (gesture.current !== null) finish(true) }}
        onKeyDown={(ev) => {
          ev.stopPropagation()
          if (ev.key === 'Escape') { ev.preventDefault(); finish(true); ev.currentTarget.blur() }
          if (ev.key === 'Enter') { ev.preventDefault(); finish(); ev.currentTarget.blur() }
        }}
        onKeyUp={(ev) => { ev.stopPropagation(); if (ev.key.startsWith('Arrow') || ['Home', 'End', 'PageUp', 'PageDown'].includes(ev.key)) finish() }}
        onBlur={() => finish()} />
      <TreatmentIndicator profile={profile} larger label={larger + context} alongEdge={alongEdge} />
    </div>
    <div role="group" aria-label="Slider sensitivity" className="fillet-sensitivity" onKeyDown={ev => ev.stopPropagation()} onKeyUp={ev => ev.stopPropagation()}>
      {(['Fine', 'Normal', 'Coarse'] as const).map(mode => <button key={mode} type="button"
        aria-label={`${mode} slider sensitivity`} aria-pressed={sensitivity === mode}
        onClick={() => setSensitivity(mode)}>{mode}</button>)}
    </div>
    <input id={id} type="text" inputMode="decimal" aria-label={`${snapshot.label} value`} aria-invalid={invalid.current || undefined}
      value={draft ?? snapshot.draft ?? formatLength(snapshot.value)}
      onFocus={() => { start('typed'); invalid.current = false; setDraft(snapshot.draft ?? formatLength(snapshot.value!)) }}
      onChange={(ev) => {
        if (!start('typed')) return
        const text = ev.target.value
        setDraft(text)
        const parsed = parseCurveInput(text, unit)
        invalid.current = 'error' in parsed || parsed.kind !== 'length' || !Number.isFinite(parsed.meters) || parsed.meters < 0
        if (!invalid.current && 'kind' in parsed && parsed.kind === 'length') gesture.current?.update(parsed.meters, text)
      }}
      onBlur={() => finish()}
      onKeyDown={(ev) => {
        ev.stopPropagation()
        if (isComposingKey(ev)) return
        if (ev.key === 'Escape') { ev.preventDefault(); finish(true); ev.currentTarget.blur() }
        if (ev.key === 'Enter') { ev.preventDefault(); ev.currentTarget.blur() }
      }} />
    <small>Release to keep the preview. Repeat a stroke to adjust further.</small>
    {snapshot.limit !== null && <small>Preview limit: {formatLength(snapshot.limit)}</small>}
  </div>
}

/**
 * ViewDie — the corner-cut die and four plane pips in the viewport's top
 * right (docs/design/view-die.md §2, §3). The die turns with the camera;
 * clicking a face or corner turns the view, dragging orbits. The pips show
 * which plane the next draw click lands on and lock or release it.
 *
 * Pure geometry lives in `viewDieGeometry.ts`. This file is the SVG, the
 * pointer handling and the plane state read from the viewport API.
 *
 * Input ownership: a press on the die or a pip never takes keyboard focus
 * and never reaches the canvas, so it cannot end a tool's gesture. A drag
 * keeps its own pointer capture on the die, so a press that starts here and
 * ends over the canvas (or the reverse) leaves no input state behind.
 */
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { msg } from '@lingui/core/macro'
import type { MessageDescriptor } from '@lingui/core'
import { useLingui } from '@lingui/react/macro'
import type { DrawPlaneState, PlanePip } from '../tools/drawPlane'
import {
  DIE_SIZE_PX,
  FACET_MIN_FACING_COS,
  FACET_MIN_RADIUS_PX,
  cornerParts,
  eyeForHit,
  facesOfAxis,
  hitTest,
  projectDie,
  stepTriangles,
  worldAxisOfNormal,
  type CornerSigns,
  type DieFaceId,
  type DieHit,
  type Quat,
  type V2,
} from './viewDieGeometry'

export type PipId = PlanePip
const PIPS: readonly PipId[] = ['red', 'green', 'blue', 'custom']

/**
 * What the die needs from the viewport API. Every member is optional so the
 * partial API doubles that App tests install keep working; the real
 * `ViewportApi` has all of them.
 */
export interface ViewDieApi {
  setViewDirection?: (eye: [number, number, number], opts?: { tween?: boolean }) => void
  orbitDrag?: (dxPx: number, dyPx: number) => void
  endOrbitDrag?: () => void
  subscribeCameraOrientation?: (cb: (q: [number, number, number, number]) => void) => () => void
  getDrawPlaneState?: () => DrawPlaneState | null
  pipClick?: (pip: PipId) => void
  pipEnabled?: (pip: PipId) => boolean
  /** True when a click on this pip would release a held plane. */
  pipWillRelease?: (pip: PipId) => boolean
  /** Fires when the draw-plane state changes (tool, lock, pin, hover plane). */
  subscribeDrawPlaneState?: (cb: (state: DrawPlaneState | null) => void) => () => void
}

export interface ViewDieProps {
  apiRef: { readonly current: ViewDieApi | null }
}

/** A press that moves less than this is a click (design §2.3). */
export const DRAG_THRESHOLD_PX = 4

const PIP_PX = 16
const PIP_GAP_PX = 6

const PIP_COLOR: Record<PipId, string> = {
  red: 'var(--axis-red)',
  green: 'var(--axis-green)',
  blue: 'var(--axis-blue)',
  custom: 'var(--plane-custom)',
}

const FACE_NAME: Record<DieFaceId, MessageDescriptor> = {
  top: msg`Top`,
  bottom: msg`Bottom`,
  front: msg`Front`,
  back: msg`Back`,
  left: msg`Left`,
  right: msg`Right`,
}

const CORNER_NAME = {
  'above-front-right': msg`Iso · front right`,
  'above-front-left': msg`Iso · front left`,
  'above-back-right': msg`Iso · back right`,
  'above-back-left': msg`Iso · back left`,
  'below-front-right': msg`Iso · from below, front right`,
  'below-front-left': msg`Iso · from below, front left`,
  'below-back-right': msg`Iso · from below, back right`,
  'below-back-left': msg`Iso · from below, back left`,
} as const

function cornerKey(c: CornerSigns): keyof typeof CORNER_NAME {
  const p = cornerParts(c)
  return `${p.below ? 'below' : 'above'}-${p.back ? 'back' : 'front'}-${p.right ? 'right' : 'left'}`
}

const PIP_LOCK_TIP: Record<PipId, MessageDescriptor> = {
  red: msg`Lock red plane  →`,
  green: msg`Lock green plane  ←`,
  blue: msg`Lock blue plane  ↑`,
  custom: msg`Pin the plane under the cursor  Shift`,
}
const PIP_RELEASE_TIP: Record<PipId, MessageDescriptor> = {
  red: msg`Release red plane  →`,
  green: msg`Release green plane  ←`,
  blue: msg`Release blue plane  ↑`,
  custom: msg`Release the pinned plane  Shift`,
}

type Hover =
  | { kind: 'die'; hit: DieHit; y: number }
  | { kind: 'pip'; pip: PipId; y: number }

function samePlane(a: DrawPlaneState | null, b: DrawPlaneState | null): boolean {
  if (a === b) return true
  if (a === null || b === null) return false
  return (
    a.pip === b.pip &&
    a.hold === b.hold &&
    a.plane.normal[0] === b.plane.normal[0] &&
    a.plane.normal[1] === b.plane.normal[1] &&
    a.plane.normal[2] === b.plane.normal[2]
  )
}

/**
 * Cancel the default of a press (no focus change, no text selection) without
 * stopping propagation: document-level outside-click handlers (an open menu)
 * must still see it.
 */
const noFocus = (e: { preventDefault: () => void }): void => {
  e.preventDefault()
}

export function ViewDie({ apiRef }: ViewDieProps) {
  const { i18n } = useLingui()
  const [q, setQ] = useState<Quat>([0, 0, 0, 1])
  const [plane, setPlane] = useState<DrawPlaneState | null>(null)
  const [hover, setHover] = useState<Hover | null>(null)
  const [inside, setInside] = useState(false)
  const svgRef = useRef<SVGSVGElement>(null)
  const wrapRef = useRef<HTMLDivElement>(null)

  const endPressRef = useRef<() => void>(() => {})
  const syncPlane = useCallback((): void => {
    const next = apiRef.current?.getDrawPlaneState?.() ?? null
    setPlane((prev) => (samePlane(prev, next) ? prev : next))
  }, [apiRef])

  // Subscribe once the viewport API exists (it is created after the first
  // mount of the viewport's own effect).
  useEffect(() => {
    let cancelled = false
    let raf = 0
    const offs: (() => void)[] = []
    const attach = (): void => {
      const api = apiRef.current
      if (api === null) {
        raf = requestAnimationFrame(attach)
        return
      }
      if (cancelled) return
      const offCam = api.subscribeCameraOrientation?.((nq) => {
        setQ(nq)
        syncPlane()
      })
      if (offCam !== undefined) offs.push(offCam)
      // The viewport fires this only on a real change of its own signature
      // (plane, pip, hold, custom enablement, which pip would release), some
      // of which `samePlane` does not see: always repaint.
      const offPlane = api.subscribeDrawPlaneState?.(() => {
        const next = apiRef.current?.getDrawPlaneState?.() ?? null
        setPlane(next === null ? null : { ...next })
      })
      if (offPlane !== undefined) offs.push(offPlane)
      syncPlane()
    }
    attach()
    // A press whose release happened outside the window: end it.
    const onBlur = (): void => endPressRef.current()
    window.addEventListener('blur', onBlur)
    return () => {
      window.removeEventListener('blur', onBlur)
      cancelled = true
      cancelAnimationFrame(raf)
      for (const off of offs) off()
      apiRef.current?.endOrbitDrag?.()
    }
  }, [apiRef, syncPlane])

  const proj = useMemo(() => projectDie(q), [q])
  const steps = useMemo(() => stepTriangles(proj), [proj])
  const projRef = useRef(proj)
  projRef.current = proj
  const stepsRef = useRef(steps)
  stepsRef.current = steps

  const drawActive = plane !== null
  const solid = proj.view.on

  // ---- plane fill (§2.5) ----
  const filledFaces = useMemo<ReadonlySet<DieFaceId>>(() => {
    if (plane === null || plane.pip === 'custom') return new Set()
    const axis = worldAxisOfNormal(plane.plane.normal)
    return axis === null ? new Set() : new Set(facesOfAxis(axis))
  }, [plane])
  const heldWeight = plane !== null && plane.hold !== 'inferred'
  const fillAlpha = heldWeight ? 0.55 : 0.35

  // ---- pips ----
  const pipEnabled = (pip: PipId): boolean => {
    return apiRef.current?.pipEnabled?.(pip) ?? false
  }
  const pipHeld = (pip: PipId): boolean => apiRef.current?.pipWillRelease?.(pip) ?? false
  const pipHoldKind = (pip: PipId): 'idle' | 'inferred' | 'locked' | 'pinned' | 'anchored' =>
    plane !== null && plane.pip === pip ? plane.hold : 'idle'

  // ---- pointer handling on the die ----
  const press = useRef<{
    id: number
    x0: number
    y0: number
    lastX: number
    lastY: number
    hit: DieHit | null
    dragging: boolean
  } | null>(null)

  const local = (e: { clientX: number; clientY: number }): V2 => {
    const r = svgRef.current!.getBoundingClientRect()
    return [e.clientX - r.left, e.clientY - r.top]
  }

  const finishPress = (commitClick: boolean): void => {
    const p = press.current
    press.current = null
    if (p === null) return
    if (p.dragging) {
      apiRef.current?.endOrbitDrag?.()
      return
    }
    if (commitClick && p.hit !== null) {
      const eye = eyeForHit(p.hit, projRef.current)
      apiRef.current?.setViewDirection?.([eye[0], eye[1], eye[2]], { tween: true })
    }
  }

  endPressRef.current = () => finishPress(false)

  const onDiePointerDown = (e: React.PointerEvent<SVGSVGElement>): void => {
    if (e.button !== 0) return
    // A press is already live: a second pointer is ignored; a repeat of the
    // same pointer means the release was lost, so settle the old one first.
    if (press.current !== null) {
      if (press.current.id !== e.pointerId) return
      finishPress(false)
    }
    // No preventDefault here: it would suppress the compatibility mousedown
    // that menus close on. `onMouseDown` cancels the focus change.
    const p = local(e)
    try {
      e.currentTarget.setPointerCapture(e.pointerId)
    } catch {
      // A synthetic pointer without an active id: the press still works.
    }
    press.current = {
      id: e.pointerId,
      x0: e.clientX,
      y0: e.clientY,
      lastX: e.clientX,
      lastY: e.clientY,
      hit: hitTest(projRef.current, p),
      dragging: false,
    }
  }

  const onDiePointerMove = (e: React.PointerEvent<SVGSVGElement>): void => {
    const pr = press.current
    // A mouse move with no button held means the release was lost (it
    // happened outside the window): end the press.
    if (pr !== null && pr.id === e.pointerId && e.pointerType === 'mouse' && e.buttons === 0) {
      finishPress(false)
      return
    }
    if (pr !== null && pr.id === e.pointerId) {
      if (!pr.dragging && Math.hypot(e.clientX - pr.x0, e.clientY - pr.y0) >= DRAG_THRESHOLD_PX) {
        pr.dragging = true
        setHover(null)
        apiRef.current?.orbitDrag?.(e.clientX - pr.x0, e.clientY - pr.y0)
        pr.lastX = e.clientX
        pr.lastY = e.clientY
      } else if (pr.dragging) {
        apiRef.current?.orbitDrag?.(e.clientX - pr.lastX, e.clientY - pr.lastY)
        pr.lastX = e.clientX
        pr.lastY = e.clientY
      }
      if (pr.dragging) return
    }
    const p = local(e)
    const hit = hitTest(projRef.current, p)
    setHover(hit === null ? null : { kind: 'die', hit, y: p[1] })
  }

  const onDiePointerUp = (e: React.PointerEvent<SVGSVGElement>): void => {
    const pr = press.current
    if (pr === null || pr.id !== e.pointerId || e.button !== 0) return
    // Finish first: releasing capture may dispatch lostpointercapture
    // synchronously, which must find the press already settled.
    finishPress(true)
    try {
      e.currentTarget.releasePointerCapture(e.pointerId)
    } catch {
      // Already released.
    }
  }

  const onDiePointerCancel = (e: React.PointerEvent<SVGSVGElement>): void => {
    if (press.current?.id === e.pointerId) finishPress(false)
  }

  // ---- pips ----
  const onPipClick = (pip: PipId): void => {
    if (!pipEnabled(pip)) return
    apiRef.current?.pipClick?.(pip)
    syncPlane()
  }

  // ---- tooltip text ----
  const tipText = (h: Hover | null): string | null => {
    if (h === null) return null
    if (h.kind === 'pip') {
      return i18n._(pipHeld(h.pip) ? PIP_RELEASE_TIP[h.pip] : PIP_LOCK_TIP[h.pip])
    }
    const hit = h.hit
    if (hit.kind === 'facet') return i18n._(CORNER_NAME[cornerKey(hit.corner)])
    if (hit.kind === 'step') return i18n._(FACE_NAME[hit.target])
    return i18n._(FACE_NAME[hit.face])
  }
  const tip = tipText(hover)

  const rootOpacity = drawActive || inside ? 1 : 0.7
  const pipOpacity = drawActive || inside ? 1 : 0.55

  const faces = [...proj.faces].filter((f) => f.facing).sort((a, b) => a.depth - b.depth)
  const facets = proj.facets.filter((f) => f.facing)
  const pts = (poly: readonly V2[]): string => poly.map((p) => `${p[0].toFixed(2)},${p[1].toFixed(2)}`).join(' ')

  const hoverPoly = ((): V2[] | null => {
    if (hover === null || hover.kind !== 'die') return null
    const hit = hover.hit
    // The view may have moved on since the pointer last did (a tween, a
    // step triangle that no longer exists): no target, no highlight.
    if (hit.kind === 'face') return proj.faces.find((f) => f.id === hit.face)?.polygon ?? null
    if (hit.kind === 'facet') return proj.facets.find((f) => f.corner.join() === hit.corner.join())?.polygon ?? null
    return steps.find((s) => s.side === hit.side)?.polygon ?? null
  })()

  const axisVar = ['var(--axis-red)', 'var(--axis-green)', 'var(--axis-blue)']
  const washFor = (id: DieFaceId): string => (id === 'top' || id === 'bottom' ? axisVar[2]! : id === 'front' || id === 'back' ? axisVar[1]! : axisVar[0]!)

  const viewName =
    proj.view.on ? (proj.view.kind === 'face' ? proj.view.face : `corner:${proj.view.corner.join(',')}`) : ''

  return (
    <div
      ref={wrapRef}
      data-testid="view-die"
      data-view={viewName}
      data-on-view={solid ? 'true' : 'false'}
      data-draw-active={drawActive ? 'true' : 'false'}
      onPointerEnter={() => { setInside(true); syncPlane() }}
      onPointerLeave={() => { setInside(false); if (press.current === null) setHover(null) }}
      onContextMenu={(e) => e.preventDefault()}
      style={{
        position: 'absolute',
        top: '16px',
        right: '16px',
        width: `${DIE_SIZE_PX}px`,
        zIndex: 20,
        // Only the die's shapes and the pips take presses (see below).
        pointerEvents: 'none',
        userSelect: 'none',
        WebkitUserSelect: 'none',
        touchAction: 'none',
      }}
    >
      <svg
        ref={svgRef}
        data-testid="view-die-svg"
        width={DIE_SIZE_PX}
        height={DIE_SIZE_PX}
        viewBox={`0 0 ${DIE_SIZE_PX} ${DIE_SIZE_PX}`}
        aria-hidden="true"
        focusable="false"
        onPointerDown={onDiePointerDown}
        onPointerMove={onDiePointerMove}
        onPointerUp={onDiePointerUp}
        onPointerCancel={onDiePointerCancel}
        onLostPointerCapture={onDiePointerCancel}
        onMouseDown={noFocus}
        onPointerLeave={() => { if (press.current === null) setHover(null) }}
        style={{
          // Only the drawn shapes take presses; the transparent corners of
          // the 76 px box belong to the canvas underneath.
          pointerEvents: 'none',
          display: 'block',
          overflow: 'visible',
          opacity: rootOpacity,
          transition: 'opacity 120ms ease',
          cursor: hover?.kind === 'die' ? 'pointer' : 'default',
        }}
      >
        {faces.map((f) => (
          <g key={f.id} data-face={f.id}>
            <polygon points={pts(f.polygon)} style={{ fill: 'var(--surface-overlay)', pointerEvents: 'auto' }} />
            <polygon points={pts(f.polygon)} style={{ fill: washFor(f.id), fillOpacity: 'var(--die-wash-alpha)' }} />
            {filledFaces.has(f.id) && plane !== null && (
              <polygon
                data-testid={`view-die-fill-${f.id}`}
                points={pts(f.polygon)}
                style={{ fill: PIP_COLOR[plane.pip], fillOpacity: fillAlpha }}
              />
            )}
            <polygon
              points={pts(f.polygon)}
              style={{ fill: 'none', stroke: 'var(--border-strong)', strokeWidth: 0.75, strokeLinejoin: 'round' }}
            />
          </g>
        ))}
        {facets.map((f) => (
          <g key={f.corner.join()} data-facet={f.corner.join()}>
            <polygon points={pts(f.polygon)} style={{ fill: 'var(--surface-overlay)', pointerEvents: 'auto' }} />
            <polygon points={pts(f.polygon)} style={{ fill: 'var(--text-primary)', fillOpacity: 0.09 }} />
            <polygon
              points={pts(f.polygon)}
              style={{ fill: 'none', stroke: 'var(--border-strong)', strokeWidth: 0.75, strokeLinejoin: 'round' }}
            />
          </g>
        ))}
        {faces.map((f) => {
          if (f.facingCos < 0.35) return null
          // Text lives in px at face-on size; the matrix turns and foreshortens it.
          const m = [...f.labelX, ...f.labelY, ...f.center].map((v) => v.toFixed(3)).join(' ')
          return (
            <text
              key={`l-${f.id}`}
              transform={`matrix(${m})`}
              textAnchor="middle"
              dominantBaseline="central"
              style={{
                fontFamily: 'var(--font-family-ui)',
                fontSize: '9px',
                fill: 'var(--text-secondary)',
                pointerEvents: 'none',
              }}
            >
              {i18n._(FACE_NAME[f.id])}
            </text>
          )
        })}
        <polygon
          data-testid="view-die-silhouette"
          data-solid={solid ? 'true' : 'false'}
          points={pts(proj.silhouette)}
          style={{
            fill: 'none',
            stroke: 'var(--text-secondary)',
            strokeWidth: 1,
            strokeLinejoin: 'round',
            strokeDasharray: solid ? 'none' : '3 2.5',
            pointerEvents: 'none',
          }}
        />
        {proj.stubs.map((s) => (
          <line
            key={`s-${s.axis}`}
            x1={s.from[0]}
            y1={s.from[1]}
            x2={s.to[0]}
            y2={s.to[1]}
            style={{ stroke: axisVar[s.axis], strokeWidth: 1.75, strokeLinecap: 'round', pointerEvents: 'none' }}
          />
        ))}
        {steps.map((s) => (
          <polygon
            key={`step-${s.side}`}
            data-testid={`view-die-step-${s.side}`}
            points={pts(s.polygon)}
            style={{
              fill: 'var(--text-secondary)',
              fillOpacity: 0.55,
              // A transparent stroke widens the target by the hit slop.
              stroke: 'transparent',
              strokeWidth: 3,
              pointerEvents: 'all',
            }}
          />
        ))}
        {facets.map((f) =>
          f.facingCos >= FACET_MIN_FACING_COS ? (
            <circle
              key={`d-${f.corner.join()}`}
              cx={f.center[0]}
              cy={f.center[1]}
              r={FACET_MIN_RADIUS_PX}
              style={{ fill: 'transparent', pointerEvents: 'all' }}
            />
          ) : null,
        )}
        {hoverPoly !== null && (
          <polygon
            data-testid="view-die-hover"
            points={pts(hoverPoly)}
            style={{
              fill: 'var(--text-primary)',
              fillOpacity: 0.22,
              stroke: 'var(--text-primary)',
              strokeWidth: 1.5,
              strokeLinejoin: 'round',
              pointerEvents: 'none',
            }}
          />
        )}
      </svg>

      <div
        data-testid="view-die-pips"
        style={{
          display: 'flex',
          justifyContent: 'center',
          gap: `${PIP_GAP_PX}px`,
          marginTop: `${PIP_GAP_PX}px`,
          opacity: pipOpacity,
          transition: 'opacity 120ms ease',
        }}
      >
        {PIPS.map((pip) => {
          const kind = pipHoldKind(pip)
          const filled = kind !== 'idle'
          const held = kind === 'locked' || kind === 'pinned'
          const ring = held || kind === 'anchored'
          const enabled = pipEnabled(pip)
          const color = PIP_COLOR[pip]
          return (
            <div
              key={pip}
              role="button"
              aria-label={i18n._(pipHeld(pip) ? PIP_RELEASE_TIP[pip] : PIP_LOCK_TIP[pip])}
              aria-disabled={!enabled}
              data-testid={`view-die-pip-${pip}`}
              data-state={kind}
              data-enabled={enabled ? 'true' : 'false'}
              onMouseDown={noFocus}
              onPointerEnter={(e) => {
                const r = wrapRef.current!.getBoundingClientRect()
                setHover({ kind: 'pip', pip, y: e.clientY - r.top })
              }}
              onPointerLeave={() => setHover((h) => (h?.kind === 'pip' && h.pip === pip ? null : h))}
              onClick={(e) => {
                e.preventDefault()
                onPipClick(pip)
              }}
              style={{
                position: 'relative',
                width: `${PIP_PX}px`,
                height: `${PIP_PX}px`,
                boxSizing: 'border-box',
                borderRadius: '3px',
                border: `1.5px solid ${color}`,
                background: filled ? color : 'transparent',
                pointerEvents: 'auto',
                cursor: enabled ? 'pointer' : 'not-allowed',
                display: 'flex',
                alignItems: 'center',
                justifyContent: 'center',
              }}
            >
              {held && (
                <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true" data-testid={`view-die-lock-${pip}`} style={{ display: 'block', pointerEvents: 'none' }}>
                  <path d="M3 4.5V3.2a2 2 0 0 1 4 0v1.3" fill="none" stroke="var(--die-pip-glyph)" strokeWidth="1.2" />
                  <rect x="1.8" y="4.5" width="6.4" height="4.6" rx="0.9" fill="var(--die-pip-glyph)" />
                </svg>
              )}
              {ring && (
                <span
                  data-testid={`view-die-ring-${pip}`}
                  aria-hidden="true"
                  style={{
                    position: 'absolute',
                    inset: '-5px',
                    border: `2px solid ${color}`,
                    borderRadius: '6px',
                    pointerEvents: 'none',
                  }}
                />
              )}
            </div>
          )
        })}
      </div>

      {tip !== null && hover !== null && (
        <div
          role="tooltip"
          data-testid="view-die-tooltip"
          style={{
            position: 'absolute',
            right: `${DIE_SIZE_PX + 8}px`,
            top: `${Math.max(0, Math.min(hover.y, DIE_SIZE_PX + PIP_GAP_PX + PIP_PX) - 10)}px`,
            padding: '3px 8px',
            background: 'var(--surface-overlay)',
            border: '1px solid var(--border-strong)',
            borderRadius: '7px',
            boxShadow: 'var(--shadow-chip, none)',
            color: 'var(--text-primary)',
            fontFamily: 'var(--font-family-ui)',
            fontSize: '11px',
            whiteSpace: 'pre',
            pointerEvents: 'none',
          }}
        >
          {tip}
        </div>
      )}
    </div>
  )
}

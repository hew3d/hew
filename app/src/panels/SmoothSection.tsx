/* oxlint-disable lingui/no-unlocalized-strings -- True Curves strings are not yet wrapped for translation: this file is English-only until they are */
/**
 * SmoothSection — Object Info's "Smooth" and "Soften edges" rows.
 *
 * Smooth turns an object's rough shape into a rounded one without changing
 * the shape you edit: pick Off, 1, 2 or 3. Bake keeps the rounded surface as
 * real geometry. Soften marks the gentle edges soft so they stop showing as
 * lines. Both apply to every object given; several objects are one undo step.
 */

import { useEffect, useMemo, useState } from 'react'
import { useLingui } from '@lingui/react/macro'
import type { Scene as WasmScene } from '../wasm/loader'
import {
  DEFAULT_SOFTEN_DEGREES,
  SMOOTH_LEVELS,
  bakeSmooth,
  parseSoftenDegrees,
  setSmoothLevel,
  smoothLevels,
  smoothRefusal,
  smoothSurfaceOff,
  softenEdges,
  softenToast,
} from './smoothActions'
import { parseKernelErrorCode, kernelErrorMessage } from '../kernelErrors'
import { actionOffered, BAKE_SMOOTH_ACTION, SOFTEN_EDGES_ACTION } from '../curveActions'
import { behaviorEnabled } from '../preview'

/** The Preview wall (src/preview): without the Smooth preview, the Smooth row
 *  is only for an object that already has a level, and offers only Off. With
 *  it, the full row. Soften and Bake follow their own commands. Fixed when
 *  the app loads. */
const SMOOTH_APPLY = behaviorEnabled('smooth-apply')
const BAKE_OFFERED = actionOffered(BAKE_SMOOTH_ACTION)
const SOFTEN_OFFERED = actionOffered(SOFTEN_EDGES_ACTION)

const LABEL_STYLE: React.CSSProperties = {
  fontSize: '10px',
  color: 'var(--text-faint, #888)',
  textTransform: 'uppercase',
  letterSpacing: '0.05em',
  marginBottom: '2px',
}

const CONTROL_STYLE: React.CSSProperties = {
  background: 'var(--surface-input, #1e1e1e)',
  border: '1px solid var(--border-strong, #444)',
  borderRadius: '3px',
  color: 'var(--text-primary, #eee)',
  fontFamily: 'var(--font-family-ui)',
  fontSize: '12px',
  padding: '3px 6px',
}

const ACTION_STYLE: React.CSSProperties = {
  background: 'none',
  border: '1px solid var(--border-strong, #444)',
  borderRadius: '3px',
  color: 'var(--text-primary, #eee)',
  cursor: 'pointer',
  fontFamily: 'var(--font-family-ui)',
  fontSize: '11px',
  padding: '2px 8px',
  whiteSpace: 'nowrap',
}

interface Props {
  scene: WasmScene
  /** Bumped on any document change to re-read the levels. */
  docRev: number
  /** The objects the rows act on. */
  objects: readonly bigint[]
  /** Called after a change that redraws geometry. */
  onGeometryChanged: () => void
  onToast?: (message: string, code?: string) => void
}

export function SmoothSection({ scene, docRev, objects, onGeometryChanged, onToast }: Props) {
  const { t } = useLingui()
  const [degrees, setDegrees] = useState(String(DEFAULT_SOFTEN_DEGREES))
  // A new selection starts the angle back at its default.
  const key = objects.map(String).join(',')
  // The levels are re-read on every document change.
  const levels = useMemo(() => {
    try {
      return smoothLevels(scene, objects)
    } catch {
      return []
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [scene, docRev, key])
  const mixed = levels.some((l) => l !== levels[0])
  const current = levels.length === 0 || mixed ? null : levels[0]
  const anySmooth = levels.some((l) => l > 0)
  // A level that is on but whose surface can no longer be drawn.
  const tooDense = useMemo(
    () => anySmooth && objects.some((id) => smoothSurfaceOff(scene, id)),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [scene, docRev, key, anySmooth],
  )

  useEffect(() => {
    setDegrees(String(DEFAULT_SOFTEN_DEGREES))
  }, [key])

  if (objects.length === 0) return null
  const showSmooth = SMOOTH_APPLY || anySmooth
  if (!showSmooth && !SOFTEN_OFFERED) return null

  const toastError = (err: unknown): void => {
    const code = parseKernelErrorCode(err)
    const raw = err instanceof Error ? err.message : String(err)
    onToast?.(kernelErrorMessage(code ?? 'Unknown', raw), code ?? undefined)
  }

  const onLevel = (level: number): void => {
    if (!SMOOTH_APPLY && level !== 0) return
    try {
      setSmoothLevel(scene, objects, level)
    } catch (err) {
      const r = smoothRefusal(err, level)
      onToast?.(r.message, r.code)
      return
    }
    onGeometryChanged()
  }

  const onBake = (): void => {
    try {
      bakeSmooth(scene, objects)
    } catch (err) {
      toastError(err)
      return
    }
    onGeometryChanged()
  }

  const onSoften = (): void => {
    const deg = parseSoftenDegrees(degrees)
    if (deg === null) {
      onToast?.('Type an angle between 1 and 179 degrees.')
      return
    }
    let n: number
    try {
      n = softenEdges(scene, objects, deg)
    } catch (err) {
      toastError(err)
      return
    }
    onToast?.(softenToast(n))
    if (n > 0) onGeometryChanged()
  }

  return (
    <>
      {showSmooth && (
      <div data-smooth-section>
        <div style={LABEL_STYLE}>Smooth</div>
        <div style={{ display: 'flex', alignItems: 'center', gap: '8px', flexWrap: 'wrap' }}>
          <select
            aria-label="Smooth"
            data-smooth-level
            value={current === null ? 'mixed' : String(current)}
            onChange={(e) => onLevel(parseInt(e.target.value, 10))}
            style={CONTROL_STYLE}
            title={t`Round the object's shape. The shape you edit stays as it is.`}
          >
            {current === null && <option value="mixed" disabled>Mixed</option>}
            {SMOOTH_LEVELS.filter((l) => SMOOTH_APPLY || l === 0 || l === current).map((l) => (
              <option key={l} value={l} disabled={!SMOOTH_APPLY && l !== 0}>{l === 0 ? 'Off' : String(l)}</option>
            ))}
          </select>
          {tooDense && (
            <span
              data-smooth-too-dense
              style={{ fontSize: '11px', color: 'var(--text-warn, #d9a441)' }}
              title={t`The shape grew too dense for this level. Lower the level or simplify the shape.`}
            >
              too dense for this level
            </span>
          )}
          {anySmooth && BAKE_OFFERED && (
            <button
              type="button"
              data-smooth-bake
              style={ACTION_STYLE}
              title={t`Keep the rounded shape as real faces. The shape you were editing is replaced.`}
              onClick={onBake}
            >
              Bake
            </button>
          )}
        </div>
      </div>
      )}
      {SOFTEN_OFFERED && (
      <div data-soften-section>
        <div style={LABEL_STYLE}>Soften edges</div>
        <div style={{ display: 'flex', alignItems: 'center', gap: '6px', flexWrap: 'wrap', fontSize: '12px', color: 'var(--text-secondary, #ccc)' }}>
          <span>Soften edges under</span>
          <input
            type="text"
            inputMode="decimal"
            aria-label="Soften angle in degrees"
            data-soften-angle
            value={degrees}
            onChange={(e) => setDegrees(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') onSoften()
              else if (e.key === 'Escape') {
                // Revert without also stepping out of a group or component.
                e.stopPropagation()
                setDegrees(String(DEFAULT_SOFTEN_DEGREES))
                e.currentTarget.blur()
              }
            }}
            style={{ ...CONTROL_STYLE, width: '44px' }}
            spellCheck={false}
          />
          <span>°</span>
          <button
            type="button"
            data-soften-apply
            style={ACTION_STYLE}
            title={t`Hide the lines on edges that bend less than this angle, so curved shapes look smooth.`}
            onClick={onSoften}
          >
            Apply
          </button>
        </div>
      </div>
      )}
    </>
  )
}

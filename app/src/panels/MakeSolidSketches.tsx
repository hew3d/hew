/**
 * MakeSolidSketches — Object ▸ Make Solid… on a selection that contains
 * sketches (docs/design/wireframe-faces-and-solids.md §6). The second input
 * of the Make Solid window: it seeds the solid search with every faced
 * region of the selected sketches, and when nothing closes it shows what is
 * missing — open edges in the rim color and lines that lie on no one plane
 * in the warn color — through the Model Health line overlay.
 *
 * A selection that holds Objects as well keeps today's Make Solid (the
 * Objects' wizard); the app decides, see `selectedSketchIds`.
 */
import { useCallback, useEffect, useRef, useState } from 'react'
import { plural, t } from '@lingui/core/macro'
import type { Scene as WasmScene } from '../wasm/loader'
import type { HealthOverlay } from '../health/healthReport'
import { WIZARD } from '../health/healthCopy'
import { kernelErrorMessage, parseKernelErrorCode } from '../kernelErrors'
import { cellOutcomeMessage, parseCell, solidToSelect, type CellOutcome, type DraftReport } from '../tools/draftCommit'
import type { NodeRef } from './treeModel'
import { edgesOfCurveRef, readPlaneUnits, type PlaneEdge, type PlaneUnitsScene } from '../tools/planeUnits'
import {
  BODY_STYLE,
  BUTTON_STYLE,
  FOOTER_STYLE,
  HEADER_STYLE,
  LINK_STYLE,
  PRIMARY_STYLE,
  WINDOW_STYLE,
  isTextField,
} from './MakeSolidWizard'

/** The sketches a selection stands for: whole sketches, and drawn units
 *  (islands, edges, curves, regions) of free sketches. A detached sketch's
 *  own context names it; a unit drawn on a plane is resolved through the
 *  plane's listing (`scene`) to the free sketches its edges live in. Drawing
 *  that lives in an Object's faces or an Object-owned sketch is not a free
 *  sketch and names none. */
export function selectedSketchIds(selection: readonly NodeRef[], scene?: PlaneUnitsScene): bigint[] {
  const out = new Set<bigint>()
  const addEdges = (edges: readonly PlaneEdge[]) => {
    for (const e of edges) if (e.store.kind === 'free') out.add(e.store.sketch)
  }
  for (const node of selection) {
    if (node.kind === 'sketch') {
      out.add(node.id)
      continue
    }
    if (node.ctx === undefined) continue
    let named: unknown
    try {
      named = (JSON.parse(node.ctx) as { sketch?: unknown }).sketch
    } catch {
      continue
    }
    if (typeof named === 'number') {
      out.add(BigInt(named))
      continue
    }
    if (scene === undefined || node.key === undefined) continue
    const pu = readPlaneUnits(scene, node.ctx)
    if (pu === null) continue
    switch (node.kind) {
      case 'draft-island': {
        const unit = pu.unitByKey.get(node.key)
        if (unit !== undefined) addEdges(unit.edges)
        break
      }
      case 'draft-edge': {
        const hit = pu.edgeByKey.get(node.key)
        if (hit !== undefined) addEdges([hit.edge])
        break
      }
      case 'draft-curve': {
        addEdges(edgesOfCurveRef(pu, node.key))
        break
      }
      case 'draft-region': {
        const region = pu.regionByKey.get(node.key)
        if (region?.extrude != null) out.add(region.extrude.sketch)
        break
      }
      default:
        break
    }
  }
  return [...out]
}

/** Why nothing closed, as the kernel reports it on a declined cell. */
export interface Decline {
  reason: string
  openEdges: number
  nonPlanarLoops: number
  /** Edges where two faces lie over each other (`overlapping_faces`). */
  overlappingEdges?: number
}

/** The first declined cell of a `make_solid_from_sketches` result. */
function firstDecline(cells: Array<Record<string, unknown>>): Decline | null {
  for (const c of cells) {
    const d = c.declined as { reason?: string; open_edges?: number; non_planar_loops?: number; overlapping_edges?: number } | undefined
    if (d !== undefined) {
      return {
        reason: d.reason ?? 'invalid',
        openEdges: d.open_edges ?? 0,
        nonPlanarLoops: d.non_planar_loops ?? 0,
        overlappingEdges: d.overlapping_edges ?? 0,
      }
    }
  }
  return null
}

/** The panel text for a decline: short, plain words. */
export function declineSentences(d: Decline): string[] {
  const out: string[] = []
  if (d.reason === 'not_closed' || d.openEdges > 0) {
    out.push(
      plural(d.openEdges, {
        one: 'Not closed yet. # open edge is marked in red.',
        other: 'Not closed yet. # open edges are marked in red.',
      }),
    )
  }
  // A non-flat loop (open or not) cannot take a single face. Printed after
  // the open-edge sentence; the plane half only appears when the kernel
  // actually counted a non-planar loop.
  if (d.nonPlanarLoops > 0) {
    out.push(
      d.openEdges > 0
        ? t`Those edges don't lie flat on one plane, so a single face can't close them. Draw a line across the opening to split it into flat faces.`
        : t`These lines do not lie on one plane, so they can't form a face.`,
    )
  }
  // Overlapping faces are marked red too; say so even when open edges are
  // also present, or the overlap marks go unexplained.
  if (d.reason === 'overlapping_faces' && (d.overlappingEdges ?? 0) > 0) {
    out.push(t`Two faces lie on top of each other where the red marks are; erase one of them.`)
  }
  if (out.length > 0) return out
  switch (d.reason) {
    case 'overlapping_faces':
      return [t`Two faces lie on top of each other where the red marks are; erase one of them.`]
    case 'inside_solid':
      return [t`This shape is inside a solid, so nothing was made. Push or pull its face to cut into the solid.`]
    case 'self_touching':
      return [t`The shape touches itself, so it can't be a solid. Pull the touching parts apart.`]
    default:
      return [t`Hew couldn't make a solid from this shape.`]
  }
}

interface Props {
  scene: WasmScene
  /** The selected sketches. */
  sketchIds: readonly bigint[]
  docRev?: number
  docGeneration?: number
  onClose: () => void
  onWidthChange?: (width: number) => void
  /** Draw (or, with null, stop drawing) the missing edges. */
  onHealthOverlay: (overlay: HealthOverlay | null) => void
  /** After a solid forms: re-tessellate and bump the document revision. */
  onGeometryChanged: () => void
  /** Select the Object a solid formed (after the scene refresh). */
  onSelectObject?: (object: bigint) => void
  onToast?: (message: string, code?: string) => void
  escapeBusy?: () => boolean
}

const WIDTH = 420

export function MakeSolidSketches({ scene, sketchIds, docRev = 0, docGeneration = 0, onClose, onWidthChange, onHealthOverlay, onGeometryChanged, onSelectObject, onToast, escapeBusy }: Props) {
  const [decline, setDecline] = useState<Decline | null>(null)
  const [working, setWorking] = useState(false)
  const overlayRef = useRef(onHealthOverlay)
  overlayRef.current = onHealthOverlay
  const shownRef = useRef(false)

  const clearOverlay = useCallback(() => {
    if (!shownRef.current) return
    shownRef.current = false
    overlayRef.current(null)
  }, [])
  // Closing clears what the window drew.
  useEffect(() => clearOverlay, [clearOverlay])

  const widthRef = useRef(onWidthChange)
  widthRef.current = onWidthChange
  useEffect(() => {
    widthRef.current?.(WIDTH)
    return () => widthRef.current?.(0)
  }, [])

  const showMissing = (rev: number): void => {
    const flat = scene.faces_open_edge_segments(BigUint64Array.from(sketchIds))
    const nOpen = flat.length >= 2 ? flat[0] : 0
    const body = flat.length >= 2 ? flat.subarray(2) : new Float32Array(0)
    const split = nOpen * 6
    shownRef.current = true
    overlayRef.current({
      ids: [],
      warnIds: [],
      current: null,
      rev,
      capPreview: null,
      // The edges of a non-planar loop are open edges too (the panel counts
      // them as such), and so are the edges where two faces lie over each
      // other (the tail after the loops, wireframe faces R22), so every one
      // is marked red; the loops also get the
      // warn color as a halo under the red.
      segments: { open: body.slice(), warn: body.slice(split) },
    })
  }

  const run = (): void => {
    if (working) return
    setWorking(true)
    try {
      let raw: { cells?: Array<Record<string, unknown>> }
      try {
        raw = JSON.parse(scene.make_solid_from_sketches(BigUint64Array.from(sketchIds)))
      } catch (err) {
        const code = parseKernelErrorCode(err)
        const msg = err instanceof Error ? err.message : String(err)
        onToast?.(code !== null ? kernelErrorMessage(code, msg) : msg, code ?? undefined)
        return
      }
      const rawCells = raw.cells ?? []
      const cells = rawCells.map(parseCell).filter((c): c is CellOutcome => c !== null)
      const report: DraftReport = { objects: [], sketches: [], keptAsSketch: [], facesClosed: [], cells }
      const message = cellOutcomeMessage(report)
      if (message !== null) {
        clearOverlay()
        onGeometryChanged()
        onToast?.(message)
        const formed = solidToSelect(report)
        if (formed !== null) onSelectObject?.(formed)
        onClose()
        return
      }
      // Nothing changed: say why, and show what is missing.
      const d = firstDecline(rawCells) ?? { reason: 'invalid', openEdges: 0, nonPlanarLoops: 0 }
      setDecline(d)
      showMissing(docRev)
    } finally {
      setWorking(false)
    }
  }

  const escRef = useRef(escapeBusy)
  escRef.current = escapeBusy
  useEffect(() => {
    const onKey = (e: KeyboardEvent): void => {
      if (e.key !== 'Escape' || isTextField(e.target)) return
      if (escRef.current?.() === true) return
      e.preventDefault()
      e.stopPropagation()
      onClose()
    }
    document.addEventListener('keydown', onKey)
    return () => document.removeEventListener('keydown', onKey)
  }, [onClose])

  // A different document closes the window; an edit makes what it marked stale.
  const generationRef = useRef(docGeneration)
  useEffect(() => {
    if (generationRef.current === docGeneration) return
    generationRef.current = docGeneration
    onClose()
  }, [docGeneration, onClose])
  const revRef = useRef(docRev)
  useEffect(() => {
    if (revRef.current === docRev) return
    revRef.current = docRev
    clearOverlay()
    setDecline(null)
  }, [docRev, clearOverlay])

  return (
    <div role="dialog" aria-modal="false" aria-label={WIZARD.title} data-make-solid-sketches style={{ ...WINDOW_STYLE, width: WIDTH }}>
      <div style={HEADER_STYLE}>
        <h2 style={{ margin: 0, flex: '1 1 auto', fontSize: '16px', fontWeight: 600, color: 'var(--text-primary)' }}>{WIZARD.overviewTitle('selection')}</h2>
        <button type="button" aria-label={WIZARD.close} title={WIZARD.close} onClick={onClose} style={{ ...LINK_STYLE, textDecoration: 'none', fontSize: '16px', color: 'var(--text-secondary)' }}>
          ✕
        </button>
      </div>
      <div style={BODY_STYLE}>
        <p>{t`Hew will turn the closed shapes you selected into solids.`}</p>
        {decline !== null &&
          declineSentences(decline).map((s) => (
            <p key={s} data-make-solid-decline style={{ color: 'var(--text-primary)' }}>
              {s}
            </p>
          ))}
      </div>
      <div style={FOOTER_STYLE}>
        <button type="button" style={BUTTON_STYLE} onClick={onClose}>
          {WIZARD.close}
        </button>
        <button type="button" style={PRIMARY_STYLE} disabled={working} onClick={run} autoFocus>
          {working ? WIZARD.working : WIZARD.fix}
        </button>
      </div>
    </div>
  )
}

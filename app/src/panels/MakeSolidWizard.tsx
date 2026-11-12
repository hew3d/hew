/**
 * MakeSolidWizard — the "Make Solid" window (docs/design/import-health.md,
 * Revisions 2 and 3). A tall window over the viewport's right side that
 * walks through what could be made solid one kind at a time, each page with
 * the recommended choice already picked. The common path is one click:
 * **Use recommended choices** applies every recommendation as ONE
 * `hew.doc.transact` frame (one undo step, labeled "Make Solid") and shows
 * the Done page. **Go step by step** walks the pages: Accept keeps a page's
 * choice and moves on, and nothing changes until **Make Solid** on the Ready
 * page. Not being solid is normal, so openings are left open by default on a
 * model or an import and closed by default on a selection.
 *
 * The window is non-modal for the viewport (orbit and zoom keep working),
 * resizable from its left edge (width remembered for the session), and
 * Escape or the close button cancels with no change. Clicking an item
 * isolates and frames it and draws its open edges (an item that can't be
 * closed draws in the warning color, with the reason under the list);
 * **Show all** isolates every item on the page; closing clears both.
 *
 * All wording is in `health/healthCopy.ts`; the plan is `health/fixPlan.ts`.
 */
import { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react'
import { i18n } from '@lingui/core'
import { useLingui } from '@lingui/react/macro'
import type { Scene as WasmScene } from '../wasm/loader'
import { openReason, readHealthReportFresh, type HealthOverlay } from '../health/healthReport'
import { runBatch, type BatchFailure } from '../health/healthBatch'
import {
  buildPlan,
  carryPerItem,
  defaultChoices,
  itemActs,
  itemDetail,
  leftByChoice,
  PAGE_ORDER,
  pagesWithItems,
  planCommands,
  readySentences,
  type FixChoices,
  type FixItem,
  type FixPlan,
} from '../health/fixPlan'
import {
  KIND_NAME,
  REASON_ON_PURPOSE,
  REASON_SENTENCE,
  THICKNESS,
  UNDO_LABEL,
  WIZARD,
  distinctLabels,
  scopeIsPicked,
  type KindKey,
  type WizardScope,
} from '../health/healthCopy'
import type { ThickenSide } from '../health/thickenSettings'
import { rowNode } from './ModelHealthPanel'
import { parseKernelErrorCode, kernelErrorMessage } from '../kernelErrors'
import type { IsolationTarget } from '../viewport/isolation'

interface Props {
  scene: WasmScene
  /** Where the wizard was opened from: the last import, the selection, or the model. */
  scope: WizardScope
  /** The object ids a `'selection'` scope covers (`'export'` too; ignored by the other scopes). */
  objectIds?: readonly bigint[]
  /** The document revision: the plan is read again whenever it changes. */
  docRev?: number
  /** Changes when a different document is loaded; the window closes itself. */
  docGeneration?: number
  /** The window closes (Done, the close button, Escape). */
  onClose: () => void
  /** Reports the window's width (0 once it closes), for chrome that centers in the uncovered part of the viewport. */
  onWidthChange?: (width: number) => void
  /** Show only `targets` in the viewport (view-only), framing them by default. */
  onIsolate: (targets: IsolationTarget[], opts?: { frame?: boolean }) => void
  onClearIsolation: () => void
  /** Draw (or, with null, stop drawing) the open edges of what the window has isolated. */
  onHealthOverlay: (overlay: HealthOverlay | null) => void
  /** After Make Solid: re-tessellate and bump the document revision. */
  onGeometryChanged: () => void
  onToast?: (message: string, code?: string) => void
  /** Bumped by the app whenever isolation ends or something else isolates:
   *  whatever this window had isolated is gone, so it forgets its selection. */
  isolationLost?: number
  /** True while Escape belongs to something else (an armed viewport gesture,
   *  a modal dialog): the window then leaves Escape alone. */
  escapeBusy?: () => boolean
}

type Page = 'overview' | KindKey | 'ready' | 'done'

// -------------------------------------------------------------------- width

export const WIZARD_DEFAULT_WIDTH = 560
export const WIZARD_MIN_WIDTH = 420
export const WIZARD_MAX_FRACTION = 0.8
/** The width the person last dragged it to, kept for the session. */
let rememberedWidth = WIZARD_DEFAULT_WIDTH

/** Clamp a width to [420 px, 80 % of the viewport]. */
export function clampWizardWidth(width: number, viewportWidth: number): number {
  const max = Math.max(WIZARD_MIN_WIDTH, Math.floor(viewportWidth * WIZARD_MAX_FRACTION))
  return Math.min(max, Math.max(WIZARD_MIN_WIDTH, Math.round(width)))
}

/** Test hook. */
export function resetWizardWidth(): void {
  rememberedWidth = WIZARD_DEFAULT_WIDTH
}

/** How many list items show before "Show 200 more". */
export const LIST_CHUNK = 200

// ------------------------------------------------------------------- styles

export const WINDOW_STYLE: React.CSSProperties = {
  position: 'absolute',
  top: 0,
  right: 0,
  bottom: 0,
  zIndex: 40,
  display: 'flex',
  flexDirection: 'column',
  background: 'var(--surface-window)',
  borderLeft: '1px solid var(--border-strong)',
  boxShadow: 'var(--shadow-window)',
  fontFamily: 'var(--font-family-ui)',
  color: 'var(--text-secondary)',
  boxSizing: 'border-box',
}

export const HANDLE_STYLE: React.CSSProperties = {
  position: 'absolute',
  left: -3,
  top: 0,
  bottom: 0,
  width: 7,
  cursor: 'ew-resize',
  touchAction: 'none',
}

export const HEADER_STYLE: React.CSSProperties = {
  display: 'flex',
  alignItems: 'center',
  gap: '8px',
  padding: '14px 16px 8px 20px',
}

export const BODY_STYLE: React.CSSProperties = {
  flex: '1 1 auto',
  overflowY: 'auto',
  padding: '4px 20px 12px',
  fontSize: '13px',
  lineHeight: 1.5,
}

export const FOOTER_STYLE: React.CSSProperties = {
  display: 'flex',
  justifyContent: 'flex-end',
  gap: '8px',
  padding: '12px 20px',
  borderTop: '1px solid var(--border-strong)',
}

export const BUTTON_STYLE: React.CSSProperties = {
  padding: '6px 18px',
  background: 'transparent',
  color: 'var(--text-secondary)',
  border: '1px solid var(--border-strong)',
  borderRadius: '4px',
  fontSize: '13px',
  fontFamily: 'var(--font-family-ui)',
  cursor: 'pointer',
}

export const PRIMARY_STYLE: React.CSSProperties = {
  ...BUTTON_STYLE,
  background: 'var(--accent-base)',
  color: '#fff',
  border: 'none',
}

export const LINK_STYLE: React.CSSProperties = {
  background: 'none',
  border: 'none',
  padding: 0,
  color: 'var(--accent-base)',
  cursor: 'pointer',
  fontFamily: 'inherit',
  fontSize: '12px',
  textDecoration: 'underline',
}

const TEXT_INPUT_STYLE: React.CSSProperties = {
  background: 'var(--surface-input, #1e1e1e)',
  border: '1px solid var(--border-strong, #444)',
  borderRadius: '3px',
  color: 'var(--text-primary, #eee)',
  fontFamily: 'var(--font-family-ui)',
  fontSize: '13px',
  padding: '4px 8px',
  outline: 'none',
  width: '120px',
}

// ---------------------------------------------------------------- helpers

export function isTextField(el: EventTarget | null): boolean {
  if (!(el instanceof HTMLElement)) return false
  return el.tagName === 'TEXTAREA' || el.isContentEditable || (el.tagName === 'INPUT' && !['radio', 'checkbox', 'button'].includes((el as HTMLInputElement).type))
}

/** The isolation targets for an item: every row it stands for. */
export function itemTargets(items: readonly FixItem[]): IsolationTarget[] {
  const out: IsolationTarget[] = []
  for (const item of items) {
    for (const row of item.rows) {
      const node = rowNode(row)
      out.push(node.kind === 'instance' ? { kind: 'member', object: row.object, instance: node.id } : { kind: 'object', id: row.object })
    }
  }
  return out
}

/**
 * The open edges to draw for `items`: those that can't be closed in the
 * warning color, the rest in the usual one (the two lists never overlap,
 * or a row would draw twice in two colors).
 */
export function overlayFor(items: readonly FixItem[], rev: number): HealthOverlay {
  const ids: bigint[] = []
  const warnIds: bigint[] = []
  for (const item of items) for (const r of item.rows) (item.kind === 'closerLook' ? warnIds : ids).push(r.object)
  return { ids, warnIds, current: null, rev, capPreview: null }
}

interface Outcome {
  /** Whether any change was made (false: nothing ran, or every step was refused). */
  ran: boolean
  solidNow: number
  /** Openings, split shapes and flat panels left as they are by choice. */
  left: number
  /** Items an op refused, with the reason in plain words. */
  refused: { item: FixItem; reason: string }[]
  /** Items Hew can't close on its own. */
  closer: FixItem[]
}

// --------------------------------------------------------------- component

export function MakeSolidWizard({ scene, scope, objectIds, docRev = 0, docGeneration = 0, onClose, onWidthChange, onIsolate, onClearIsolation, onHealthOverlay, onGeometryChanged, onToast, isolationLost = 0, escapeBusy }: Props) {
  const { t } = useLingui()
  const readScopeIds = (): bigint[] | null =>
    scope === 'import' ? Array.from(scene.last_import_object_ids()) : scopeIsPicked(scope) ? [...(objectIds ?? [])] : null
  const readPlan = (): FixPlan => buildPlan(readHealthReportFresh(scene, readScopeIds()))
  // The plan is what the person reviews and what Make Solid applies. It is
  // read again when the document changes (the window is non-modal) and right
  // before Make Solid builds its commands.
  const [plan, setPlan] = useState<FixPlan>(readPlan)
  const planRef = useRef(plan)
  planRef.current = plan
  const pages = pagesWithItems(plan)
  const [page, setPage] = useState<Page>('overview')
  const pageRef = useRef<Page>(page)
  pageRef.current = page
  const [choices, setChoices] = useState<FixChoices>(() => defaultChoices(scope))
  /** Pages whose list shows checkboxes ("Choose for each item"). */
  const [chooseEach, setChooseEach] = useState<Partial<Record<KindKey, boolean>>>({})
  /** How many items each page's list shows. */
  const [shown, setShown] = useState<Partial<Record<KindKey, number>>>({})
  /** How many still-open items the Done page lists. */
  const [doneShown, setDoneShown] = useState(LIST_CHUNK)
  const [active, setActive] = useState<string | null>(null)
  /** The item whose open edges are shown (a "can't be closed" one also shows its reason). */
  const [activeItem, setActiveItem] = useState<FixItem | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [working, setWorking] = useState(false)
  const [outcome, setOutcome] = useState<Outcome | null>(null)

  // ---- window geometry
  const rootRef = useRef<HTMLDivElement>(null)
  const [width, setWidth] = useState(rememberedWidth)
  const viewportWidth = (): number => rootRef.current?.parentElement?.clientWidth || window.innerWidth
  // The remembered width may not fit this viewport: clamp it on open too.
  useLayoutEffect(() => {
    setWidth((w) => clampWizardWidth(w, viewportWidth()))
  }, [])
  const dragRef = useRef<{ startX: number; startWidth: number } | null>(null)
  const onHandleDown = (e: React.PointerEvent): void => {
    dragRef.current = { startX: e.clientX, startWidth: width }
    ;(e.currentTarget as HTMLElement).setPointerCapture?.(e.pointerId)
    e.preventDefault()
  }
  const onHandleMove = (e: React.PointerEvent): void => {
    const drag = dragRef.current
    if (drag === null) return
    const next = clampWizardWidth(drag.startWidth + (drag.startX - e.clientX), viewportWidth())
    rememberedWidth = next
    setWidth(next)
  }
  const onHandleUp = (): void => {
    dragRef.current = null
  }
  // Keep the width inside the limits when the viewport itself resizes.
  useEffect(() => {
    const onResize = (): void => setWidth((w) => clampWizardWidth(w, viewportWidth()))
    window.addEventListener('resize', onResize)
    return () => window.removeEventListener('resize', onResize)
  }, [])

  const widthRef = useRef(onWidthChange)
  widthRef.current = onWidthChange
  useEffect(() => {
    widthRef.current?.(width)
  }, [width])
  useEffect(() => () => widthRef.current?.(0), [])

  // ---- closing
  const clearRef = useRef(onClearIsolation)
  clearRef.current = onClearIsolation
  const overlayRef = useRef(onHealthOverlay)
  overlayRef.current = onHealthOverlay
  // Leave alone any isolation this window didn't make (Model Health's, the API's).
  useEffect(
    () => () => {
      if (!isolatedRef.current) return
      isolatedRef.current = false
      clearRef.current()
      overlayRef.current(null)
    },
    [],
  )
  /** Set once the person cancels or the window goes away: a pending apply must not run. */
  const cancelledRef = useRef(false)
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const stopPending = useCallback(() => {
    cancelledRef.current = true
    if (timerRef.current !== null) clearTimeout(timerRef.current)
    timerRef.current = null
  }, [])
  useEffect(() => {
    cancelledRef.current = false
    return stopPending
  }, [stopPending])
  const cancel = useCallback(() => {
    stopPending()
    onClose()
  }, [onClose, stopPending])
  const escapeRef = useRef(escapeBusy)
  escapeRef.current = escapeBusy
  useEffect(() => {
    const onKey = (e: KeyboardEvent): void => {
      if (e.key !== 'Escape' || isTextField(e.target)) return
      // A drag or armed tool cancels first; a dialog closes itself.
      if (escapeRef.current?.() === true) return
      e.preventDefault()
      e.stopPropagation()
      cancel()
    }
    document.addEventListener('keydown', onKey)
    return () => document.removeEventListener('keydown', onKey)
  }, [cancel])

  // ---- freshness
  // A different document replaces the one the plan was read from: close.
  const generationRef = useRef(docGeneration)
  useEffect(() => {
    if (generationRef.current === docGeneration) return
    generationRef.current = docGeneration
    cancel()
  }, [docGeneration, cancel])
  // An edit while the window is open: read the plan again, keeping the
  // choices and the unticked items; items that need nothing now drop out.
  const revRef = useRef(docRev)
  /** Set once Make Solid has run: the revision bump that follows is its own. */
  const appliedRef = useRef(false)
  useEffect(() => {
    if (revRef.current === docRev) return
    revRef.current = docRev
    if (appliedRef.current) return
    const fresh = readPlan()
    setChoices((c) => ({ ...c, perItem: carryPerItem(planRef.current, fresh, c.perItem) }))
    setPlan(fresh)
    const current = pageRef.current
    if (current !== 'overview' && current !== 'ready' && current !== 'done' && !pagesWithItems(fresh).includes(current)) {
      setPage('overview')
      clearIsolationRef.current()
      return
    }
    // What is isolated is read again: its open edges at this revision, or
    // nothing when none of it needs Make Solid any more.
    if (isolatedRef.current) {
      const keys = new Set(isolatedItemsRef.current.map((i) => i.key))
      const still = PAGE_ORDER.flatMap((k) => fresh.items[k]).filter((i) => keys.has(i.key))
      if (still.length === 0) clearIsolationRef.current()
      else {
        isolatedItemsRef.current = still
        setActiveItem((a) => (a === null ? null : (still.find((i) => i.key === a.key && i.kind === 'closerLook') ?? null)))
        overlayRef.current(overlayFor(still, docRev))
      }
    }
  }, [docRev])

  // ---- isolation
  /** Whether this window has isolated something the viewport still shows. */
  const isolatedRef = useRef(false)
  /** The items this window isolated (re-read when the document changes). */
  const isolatedItemsRef = useRef<readonly FixItem[]>([])
  const clearIsolation = (): void => {
    setActive(null)
    setActiveItem(null)
    isolatedItemsRef.current = []
    if (isolatedRef.current) {
      isolatedRef.current = false
      onClearIsolation()
      onHealthOverlay(null)
    }
  }
  const clearIsolationRef = useRef(clearIsolation)
  clearIsolationRef.current = clearIsolation
  // Isolation ended elsewhere (Show everything, Escape in the viewport, a
  // click on nothing) or Model Health isolated something else: the
  // selection here no longer matches what the viewport shows.
  const lostRef = useRef(isolationLost)
  useEffect(() => {
    if (lostRef.current === isolationLost) return
    lostRef.current = isolationLost
    if (!isolatedRef.current) return
    isolatedRef.current = false
    isolatedItemsRef.current = []
    setActive(null)
    setActiveItem(null)
  }, [isolationLost])
  const isolateItems = (items: readonly FixItem[], key: string | null): void => {
    setActive(key)
    setActiveItem(items.length === 1 && items[0].kind === 'closerLook' ? items[0] : null)
    isolatedRef.current = true
    isolatedItemsRef.current = items
    onIsolate(itemTargets(items))
    onHealthOverlay(overlayFor(items, docRev))
  }

  // ---- applying
  const apply = (use: FixChoices): void => {
    // A thickness that doesn't parse is the person's to fix before anything runs.
    const first = planCommands(plan, use)
    if (!first.ok) {
      setError(first.error)
      return
    }
    setError(null)
    setWorking(true)
    // Let "Fixing…" paint before the synchronous run.
    timerRef.current = setTimeout(() => {
      timerRef.current = null
      // Cancelled before anything was sent: nothing changes.
      if (cancelledRef.current) return
      try {
        // Read the model again right before building the commands.
        const fresh = readPlan()
        const chosen: FixChoices = { ...use, perItem: carryPerItem(plan, fresh, use.perItem) }
        const commands = planCommands(fresh, chosen)
        if (!commands.ok) {
          setPlan(fresh)
          setError(commands.error)
          return
        }
        const result = runBatch(scene, () => UNDO_LABEL.makeSolid, commands.ops, commands.options)
        const itemOf = new Map<bigint, FixItem>()
        for (const kind of Object.keys(fresh.items) as KindKey[]) for (const item of fresh.items[kind]) for (const r of item.rows) itemOf.set(r.object, item)
        const refused: Outcome['refused'] = []
        const seen = new Set<FixItem>()
        for (const f of result.failed as BatchFailure[]) {
          const item = itemOf.get(f.object)
          if (item === undefined || seen.has(item)) continue
          seen.add(item)
          refused.push({ item, reason: f.explanation })
        }
        let solidNow = 0
        appliedRef.current = true
        if (result.done.length > 0) {
          onGeometryChanged()
          solidNow = Math.max(0, readHealthReportFresh(scene, readScopeIds()).counts.solids - fresh.solids)
        }
        setOutcome({
          ran: result.done.length > 0,
          solidNow,
          left: leftByChoice(fresh, chosen),
          refused,
          closer: fresh.items.closerLook,
        })
        clearIsolation()
        setPage('done')
      } catch (err) {
        const code = parseKernelErrorCode(err)
        const raw = err instanceof Error ? err.message : String(err)
        onToast?.(code !== null ? kernelErrorMessage(code, raw) : raw, code ?? undefined)
        setError(code !== null ? kernelErrorMessage(code, raw) : raw)
      } finally {
        setWorking(false)
      }
    }, 0)
  }

  // ---- navigation
  const steps: Page[] = ['overview', ...pages, 'ready']
  const go = (delta: number): void => {
    const i = steps.indexOf(page)
    const next = steps[Math.min(steps.length - 1, Math.max(0, i + delta))]
    setError(null)
    clearIsolation()
    setPage(next)
  }
  const accept = (): void => {
    if (page === 'flatPanels') {
      const commands = planCommands(plan, choices)
      if (!commands.ok) {
        setError(commands.error)
        return
      }
    }
    go(1)
  }

  const itemsOf = (kind: KindKey): FixItem[] => plan.items[kind]
  const setChoice = <K extends keyof FixChoices>(key: K, value: FixChoices[K]): void => {
    setError(null)
    setChoices((c) => ({ ...c, [key]: value }))
  }
  /** Tick or untick one item under "Choose for each item". */
  const toggleItem = (item: FixItem): void => {
    setError(null)
    setChoices((c) => {
      const next = new Map(c.perItem)
      next.set(item.key, !itemActs(item, c))
      return { ...c, perItem: next }
    })
  }
  /** A page's radio changed: its items follow the page again. */
  const setPageChoice = <K extends 'openings' | 'split' | 'flatPanels'>(kind: K, value: FixChoices[K]): void => {
    setError(null)
    setChoices((c) => {
      const mine = new Set(plan.items[kind].map((i) => i.key))
      const next = new Map([...c.perItem].filter(([k]) => !mine.has(k)))
      return { ...c, [kind]: value, perItem: next }
    })
  }

  // ---------------------------------------------------------------- pieces

  const radio = (name: string, label: string, checked: boolean, onChange: () => void, recommended: boolean, disabled = false): React.ReactNode => (
    <label key={label} style={{ display: 'flex', alignItems: 'baseline', gap: '8px', padding: '3px 0' }}>
      <input type="radio" name={name} checked={checked} disabled={disabled} onChange={onChange} />
      <span style={{ color: 'var(--text-primary)' }}>{label}</span>
      {recommended && <span style={{ color: 'var(--text-faint)', fontSize: '12px' }}>({WIZARD.recommended})</span>}
    </label>
  )

  /** Under a list: why the selected "can't be closed" item stays open, and that it is usually fine. */
  const reasonCallout = (): React.ReactNode =>
    activeItem === null ? null : (
      <div
        data-wizard-reason-callout
        style={{ margin: '8px 0 0', padding: '6px 10px', fontSize: '12px', border: '1px solid var(--border-strong)', borderRadius: '4px' }}
      >
        <p style={{ margin: 0 }}>
          {REASON_SENTENCE[openReason(activeItem.rows[0])]} {i18n._(REASON_ON_PURPOSE)}
        </p>
      </div>
    )

  const itemList = (kind: KindKey): React.ReactNode => {
    const items = itemsOf(kind)
    const limit = shown[kind] ?? LIST_CHUNK
    const each = chooseEach[kind] === true
    const canChoose = kind === 'openings' || kind === 'split' || kind === 'flatPanels'
    const labels = distinctLabels(items.map((i) => i.name))
    return (
      <div style={{ margin: '12px 0 4px' }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: '12px', marginBottom: '6px' }}>
          <span style={{ fontWeight: 600, color: 'var(--text-primary)' }}>
            {KIND_NAME[kind]} ({items.length})
          </span>
          <button type="button" style={LINK_STYLE} onClick={() => isolateItems(items, 'all')}>
            {WIZARD.showAll}
          </button>
        </div>
        <div
          role="list"
          data-wizard-list={kind}
          style={{ border: '1px solid var(--border-strong)', borderRadius: '4px', maxHeight: '38vh', overflowY: 'auto' }}
        >
          {items.slice(0, limit).map((item, index) => {
            const copies = WIZARD.copies(item.copies)
            const on = active === item.key
            return (
              <div
                key={item.key}
                role="listitem"
                style={{ display: 'flex', alignItems: 'center', gap: '8px', padding: '3px 8px', background: on ? 'var(--surface-row-selected, rgba(80,140,255,0.18))' : 'transparent' }}
              >
                {each && (
                  <input
                    type="checkbox"
                    aria-label={`${WIZARD.makeThisSolid} ${labels[index]}`}
                    checked={itemActs(item, choices)}
                    onChange={() => toggleItem(item)}
                  />
                )}
                <button
                  type="button"
                  data-wizard-item={item.key}
                  aria-pressed={on}
                  onClick={() => isolateItems([item], item.key)}
                  style={{ ...LINK_STYLE, flex: '1 1 auto', textAlign: 'left', textDecoration: 'none', color: 'var(--text-primary)', fontSize: '13px', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}
                >
                  {labels[index]}
                </button>
                {kind === 'closerLook' && (
                  <span data-wizard-detail style={{ color: 'var(--text-faint)', fontSize: '12px', flexShrink: 0 }}>
                    {itemDetail(item)}
                  </span>
                )}
                {copies !== '' && <span style={{ color: 'var(--text-faint)', fontSize: '12px', flexShrink: 0 }}>{copies}</span>}
              </div>
            )
          })}
          {items.length > limit && (
            <button type="button" style={{ ...LINK_STYLE, display: 'block', margin: '6px 8px' }} onClick={() => setShown((s) => ({ ...s, [kind]: limit + LIST_CHUNK }))}>
              {WIZARD.showMore}
            </button>
          )}
        </div>
        {reasonCallout()}
        {canChoose && (
          <div style={{ marginTop: '6px' }}>
            <button
              type="button"
              style={LINK_STYLE}
              aria-pressed={each}
              onClick={() => {
                setChooseEach((m) => ({ ...m, [kind]: !each }))
                if (each) {
                  const mine = new Set(items.map((i) => i.key))
                  setChoices((c) => ({ ...c, perItem: new Map([...c.perItem].filter(([k]) => !mine.has(k))) }))
                }
              }}
            >
              {WIZARD.chooseEach}
            </button>
          </div>
        )}
      </div>
    )
  }

  const side = (s: ThickenSide, label: string): React.ReactNode => (
    <label key={s} style={{ display: 'flex', alignItems: 'center', gap: '4px' }}>
      <input type="radio" name="wizard-thickness-side" checked={choices.thickness.side === s} onChange={() => setChoice('thickness', { ...choices.thickness, side: s })} />
      {label}
    </label>
  )

  const categoryBody = (kind: KindKey): React.ReactNode => {
    switch (kind) {
      case 'openings':
        return (
          <>
            <p>{WIZARD.openingsBody}</p>
            {(scopeIsPicked(scope)
              ? ['close', 'leave']
              : ['leave', 'close']
            ).map((v) =>
              v === 'close'
                ? radio('wz-openings', WIZARD.openingsClose, choices.openings === 'close', () => setPageChoice('openings', 'close'), scopeIsPicked(scope))
                : radio('wz-openings', WIZARD.openingsLeave, choices.openings === 'leave', () => setPageChoice('openings', 'leave'), !scopeIsPicked(scope)),
            )}
          </>
        )
      case 'split':
        return (
          <>
            <p>{WIZARD.splitBody}</p>
            {radio('wz-split', WIZARD.splitRejoin, choices.split === 'rejoin', () => setPageChoice('split', 'rejoin'), true)}
            {/* oxlint-disable-next-line lingui/no-unlocalized-strings -- wizard choice id, not copy */}
            {radio('wz-split', WIZARD.splitKeepWalls, choices.split === 'keepWalls', () => setPageChoice('split', 'keepWalls'), false)}
            {radio('wz-split', WIZARD.splitLeave, choices.split === 'leave', () => setPageChoice('split', 'leave'), false)}
          </>
        )
      case 'flatPanels':
        return (
          <>
            <p>{WIZARD.panelsBody}</p>
            {/* oxlint-disable-next-line lingui/no-unlocalized-strings -- wizard choice id, not copy */}
            {radio('wz-flat', WIZARD.panelsKeep, choices.flatPanels === 'keep', () => setPageChoice('flatPanels', 'keep'), true)}
            {/* oxlint-disable-next-line lingui/no-unlocalized-strings -- wizard choice id, not copy */}
            {radio('wz-flat', WIZARD.panelsThicken, choices.flatPanels === 'thicken', () => setPageChoice('flatPanels', 'thicken'), false)}
            {(choices.flatPanels === 'thicken' || plan.items.flatPanels.some((i) => itemActs(i, choices))) && (
              <div style={{ margin: '6px 0 4px 24px', display: 'flex', flexDirection: 'column', gap: '8px' }}>
                <label style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
                  {THICKNESS.field}
                  <input
                    type="text"
                    aria-label={THICKNESS.field}
                    value={choices.thickness.text}
                    spellCheck={false}
                    onChange={(e) => setChoice('thickness', { ...choices.thickness, text: e.target.value })}
                    onFocus={(e) => e.currentTarget.select()}
                    style={TEXT_INPUT_STYLE}
                  />
                </label>
                <div style={{ display: 'flex', gap: '14px' }}>
                  {side('front', THICKNESS.inFront)}
                  {side('behind', THICKNESS.behind)}
                  {side('centered', THICKNESS.centered)}
                </div>
              </div>
            )}
          </>
        )
      case 'curvedSheets':
        return (
          <>
            <p>{WIZARD.sheetsBody}</p>
            {radio('wz-sheets', WIZARD.sheetsKeep, true, () => undefined, true)}
          </>
        )
      case 'closerLook':
        return <p>{WIZARD.closerBody}</p>
      default:
        return null
    }
  }

  const titleOf = (p: Page): string => {
    switch (p) {
      case 'overview':
        return WIZARD.overviewTitle(scope)
      case 'openings':
        return WIZARD.openingsTitle
      case 'split':
        return WIZARD.splitTitle
      case 'flatPanels':
        return WIZARD.panelsTitle
      case 'curvedSheets':
        return WIZARD.sheetsTitle
      case 'closerLook':
        return WIZARD.closerTitle
      case 'ready':
        return WIZARD.readyTitle
      case 'done':
        return outcome !== null && !outcome.ran ? WIZARD.nothingChangedTitle : WIZARD.doneTitle
    }
  }

  const errorEl = error !== null && (
    <div role="alert" style={{ color: 'var(--status-warning, #e8c84a)', fontSize: '12px', margin: '8px 0' }}>
      {error}
    </div>
  )

  const renderBody = (): React.ReactNode => {
    if (page === 'overview') {
      return (
        <>
          <p>{WIZARD.overview(scope, plan.objects, plan.solids)}</p>
          {pages.length === 0 ? (
            <p>{WIZARD.nothingToFix}</p>
          ) : (
            <ul style={{ margin: '8px 0', paddingLeft: '18px' }}>
              {pages.map((k) => (
                <li key={k}>
                  {k === 'openings'
                    ? WIZARD.overviewLine.openings(itemsOf(k).length, defaultChoices(scope).openings === 'close')
                    : WIZARD.overviewLine[k](itemsOf(k).length)}
                </li>
              ))}
            </ul>
          )}
          {errorEl}
        </>
      )
    }
    if (page === 'ready') {
      return (
        <>
          <ul style={{ margin: '8px 0', paddingLeft: '18px' }} data-wizard-ready>
            {readySentences(plan, choices).map((s) => (
              <li key={s}>{s}</li>
            ))}
          </ul>
          {errorEl}
        </>
      )
    }
    if (page === 'done') {
      const o = outcome as Outcome
      const list = (name: string, rows: { item: FixItem; reason: string | null; warn?: boolean }[]): React.ReactNode => {
        const labels = distinctLabels(rows.map((r) => r.item.name))
        return (
          <>
            <div role="list" data-wizard-list={name} style={{ border: '1px solid var(--border-strong)', borderRadius: '4px', maxHeight: '38vh', overflowY: 'auto' }}>
              {rows.slice(0, doneShown).map(({ item, reason, warn }, index) => (
                <div key={item.key} role="listitem" style={{ padding: '3px 8px' }}>
                  <button
                    type="button"
                    data-wizard-item={item.key}
                    aria-pressed={active === item.key}
                    onClick={() => isolateItems([item], item.key)}
                    style={{ ...LINK_STYLE, textDecoration: 'none', color: 'var(--text-primary)', fontSize: '13px' }}
                  >
                    {labels[index]}
                  </button>
                  {reason !== null && (
                    <div data-wizard-reason style={{ fontSize: '12px', color: warn === true ? 'var(--status-warning, #e8c84a)' : 'var(--text-faint)' }}>
                      {reason}
                    </div>
                  )}
                </div>
              ))}
            </div>
            {rows.length > doneShown && (
              <button type="button" style={{ ...LINK_STYLE, marginTop: '6px' }} onClick={() => setDoneShown((n) => n + LIST_CHUNK)}>
                {WIZARD.showMore}
              </button>
            )}
          </>
        )
      }
      return (
        <>
          <p>{o.ran ? WIZARD.done(o.solidNow) : WIZARD.nothingChanged}</p>
          {o.left > 0 && <p>{WIZARD.leftAsChosen(o.left)}</p>}
          {o.refused.length > 0 && (
            <>
              <p style={{ marginBottom: '4px' }}>{WIZARD.couldntFix(o.refused.length)}</p>
              {list('couldnt-fix', o.refused.map((r) => ({ ...r, warn: true })))}
            </>
          )}
          {o.closer.length > 0 && (
            <>
              <p style={{ marginBottom: '4px' }}>{WIZARD.stillOpen(o.closer.length)}</p>
              {list('still-open', o.closer.map((item) => ({ item, reason: itemDetail(item) })))}
              {reasonCallout()}
            </>
          )}
        </>
      )
    }
    return (
      <>
        {categoryBody(page)}
        {itemList(page)}
        {errorEl}
      </>
    )
  }

  const renderFooter = (): React.ReactNode => {
    if (page === 'overview') {
      return pages.length === 0 ? (
        <button type="button" style={PRIMARY_STYLE} onClick={cancel}>
          {WIZARD.doneButton}
        </button>
      ) : (
        <>
          <button type="button" style={BUTTON_STYLE} onClick={() => go(1)}>
            {WIZARD.stepByStep}
          </button>
          <button type="button" style={PRIMARY_STYLE} disabled={working} onClick={() => apply(defaultChoices(scope))} autoFocus>
            {working ? WIZARD.working : WIZARD.acceptRecommended}
          </button>
        </>
      )
    }
    if (page === 'ready') {
      return (
        <>
          <button type="button" style={BUTTON_STYLE} disabled={working} onClick={() => go(-1)}>
            {WIZARD.back}
          </button>
          <button type="button" style={PRIMARY_STYLE} disabled={working} onClick={() => apply(choices)} autoFocus>
            {working ? WIZARD.working : WIZARD.fix}
          </button>
        </>
      )
    }
    if (page === 'done') {
      return (
        <button type="button" style={PRIMARY_STYLE} onClick={cancel} autoFocus>
          {WIZARD.doneButton}
        </button>
      )
    }
    return (
      <>
        <button type="button" style={BUTTON_STYLE} onClick={() => go(-1)}>
          {WIZARD.back}
        </button>
        <button type="button" style={PRIMARY_STYLE} onClick={accept} autoFocus>
          {WIZARD.accept}
        </button>
      </>
    )
  }

  return (
    <div
      ref={rootRef}
      role="dialog"
      aria-modal="false"
      aria-label={WIZARD.title}
      data-make-solid-wizard
      data-wizard-page={page}
      style={{ ...WINDOW_STYLE, width }}
    >
      <div
        role="separator"
        aria-orientation="vertical"
        aria-label={t`Resize`}
        data-wizard-resize
        style={HANDLE_STYLE}
        onPointerDown={onHandleDown}
        onPointerMove={onHandleMove}
        onPointerUp={onHandleUp}
        onPointerCancel={onHandleUp}
      />
      <div style={HEADER_STYLE}>
        <h2 style={{ margin: 0, flex: '1 1 auto', fontSize: '16px', fontWeight: 600, color: 'var(--text-primary)' }}>{titleOf(page)}</h2>
        <button type="button" aria-label={WIZARD.close} title={WIZARD.close} onClick={cancel} style={{ ...LINK_STYLE, textDecoration: 'none', fontSize: '16px', color: 'var(--text-secondary)' }}>
          ✕
        </button>
      </div>
      <div style={BODY_STYLE}>{renderBody()}</div>
      <div style={FOOTER_STYLE}>{renderFooter()}</div>
    </div>
  )
}


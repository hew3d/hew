/**
 * ShopApp — Shop Mode: a fullscreen, touch-first viewer/inspector shell for
 * a phone at the workbench. The use case: build a model at the desk, walk
 * to the workshop, and reference it on a phone — orbit it, tap a part for
 * its dimensions, check what to cut next — with or without wifi.
 *
 * A second hard-coded shell beside the editor (`App.tsx`), selected the
 * same hash/override way `main.tsx` already selects the Settings/Library
 * windows (`shop/shellMode.ts`). NOT a Studios/Workbenches framework — that
 * gets extracted post-1.0 once a third shell exists (rule of three).
 *
 * Non-goals, load-bearing for every design choice below:
 *   - NO editing. Shop Mode issues zero kernel transactions and never
 *     dirties the document — every bit of state it manipulates (selection,
 *     the isolate hidden-set, the Parts sheet's own hides, camera) is
 *     view-state, thrown away on close.
 *   - NO Scenes. The Parts sheet (`PartsSheet.tsx`), offline recents
 *     (`io/recents.ts`), and the "Open on Phone" E2E-encrypted relay
 *     handoff (in-app scanner `ScanSheet.tsx`, boot-time `#recv=` hash
 *     below, both funneling through `receiveDrop`) are built — see their
 *     own module docs.
 *
 * Composition: reuses the full `Viewport` component, unmodified — the same
 * pick/inference/snap layer the editor's Select and Tape Measure tools run
 * through, constrained here to a 3-tool registry (Select / Orbit / Tape
 * Measure) via `activeTool`. See the per-prop comments below for exactly
 * which editor-chrome callbacks are wired for real vs. deliberately
 * inert, and why.
 */
import { buildFrameThumbnail } from '../viewport/frameThumbnail'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { loadKernel, type Scene } from '../wasm/loader'
import Viewport, { type ViewportApi, type InferenceInfo, type StandardView } from '../viewport/Viewport'
import type { Projection } from '../viewport/cameraRig'
import { SnapDot } from '../viewport/SnapDot'
import { InferenceTooltip } from '../viewport/InferenceTooltip'
import { MeasurementBox } from '../viewport/MeasurementBox'
import { CAMERA_HANDOFF_TOOL_NAMES } from '../panels/cameraHandoffTools'
import { nodeKey, type NodeRef } from '../panels/treeModel'
import { tagPathKey } from '../panels/tagModel'
import { makeFileHost, type OpenPick } from '../io/fileHost'
import { anchorDownload } from '../io/webFileHost'
import { decrypt, fromBase64Url } from '../io/shareCrypto'
import { phoneRelayBase } from '../io/shareRelay'
import { loadHewBytes, isSceneEmpty, seedHiddenKeysFromRegistry, seedHiddenTagPathsFromRegistry, unionHiddenLeafIds } from '../io/documentLoad'
import { listRecents, recordRecent, type RecentEntry } from '../io/recents'
import { RecentsList } from './RecentsList'
import { RecentsSheet } from './RecentsSheet'
import { friendlyErrorText } from '../kernelErrors'
import { MultiClickTracker, type MultiClickPress } from '../viewport/multiClick'
import { worldBoundsForSelection } from '../panels/objectBounds'
import { resolveInspect, type InspectResult } from './inspect'
import { InspectCard } from './InspectCard'
import { PartsSheet } from './PartsSheet'
import { PEEK_HEIGHT_PX, type SheetDetent } from './sheetDetents'
import { isolateHiddenFor } from './isolate'
import { writeShellModeOverride, parseRecvParams, type RecvParams } from './shellMode'
import { installShopTestHarness } from './testHarness'
import { HintEngine, largestVisiblePart, testHintsSuppressed, type ActiveHint } from './hints'
import { HintOverlay } from './HintOverlay'
import { isArQuickLookCandidate, launchArQuickLook } from './arQuickLook'
import { seedLocaleLengthUnit } from './localeUnits'
import { buildPartsSheetSections, totalPartCount } from './partsSheetModel'
import { DocumentMenu } from './DocumentMenu'
import { SettingsMenu } from './SettingsMenu'
import { UnitPicker } from './UnitPicker'
import { ScanSheet } from './ScanSheet'
import { ViewsSheet } from './ViewsSheet'
import { ReceiveConfirmSheet } from './ReceiveConfirmSheet'
import { ScenePill } from './ScenePill'
import { getSceneTransitions, SCENE_TRANSITION_MS } from '../settings/sceneTransitions'
import {
  parseScenesJson,
  parseCameraJson,
  parseSectionJson,
  parseDriftJson,
  neighborScene,
  driftAny,
  type SceneEntry,
  type SceneDrift,
} from '../scenes/scenesModel'
import { useShopOrientation } from './orientation'
import {
  ChevronDownIcon,
  ChevronRightIcon,
  EllipsisIcon,
  HewMark,
  OrbitIcon,
  QrIcon,
  SelectIcon,
  TapeIcon,
  UploadIcon,
  ViewCubeIcon,
  ZoomExtentsIcon,
  type ShopIconProps,
} from './icons'
import type { Snap } from '../tools/types'

/** The 3-tool registry Shop Mode restricts `Viewport`'s `activeTool` to
 *  (design §"Composition") — Draw/Modify tools, Paint, Section Plane, etc.
 *  are all editor-only and unreachable here. */
type ShopToolName = 'Select' | 'Orbit' | 'Tape Measure'

/** Long-press duration (ms) before an untouched, unmoved press on a part
 *  triggers isolate — comfortably past the ~300-500ms a deliberate
 *  press-and-hold reads as, short enough not to feel unresponsive. Exported
 *  so PartsSheet.tsx's own per-row long-press (its "long-press: isolate"
 *  gesture) uses the exact same timing as the viewport's. */
export const LONG_PRESS_MS = 500
/** Movement tolerance (CSS px) before a held press is treated as the start
 *  of an orbit drag (viewport) / list scroll (PartsSheet) instead of a
 *  long-press. Deliberately tighter than `MULTI_CLICK_TOUCH_SLOP_PX`
 *  (`viewport/multiClick.ts`, shop-mode playtest finding 6): a long-press
 *  already gets a generous 500ms to distinguish "held still" from "started
 *  moving", so a tight position tolerance is what actually separates it
 *  from an orbit drag; a double-tap has no such time cushion between its
 *  two SEPARATE touch-downs, so it needs the position tolerance to do that
 *  job instead — different gesture shapes, correctly different slop
 *  budgets, not a shared constant. */
export const LONG_PRESS_SLOP_PX = 10
/** Isolate's opacity fade duration (design_handoff_shop_mode/README.md §4 /
 *  Motion summary: "parts fade 240ms ease") — the ONLY `pushHidden` callers
 *  that pass a `fadeMs` (`isolateNode`/`showAll` below); the Parts sheet's
 *  own eye toggles stay instant, per the same Motion summary row. */
const ISOLATE_FADE_MS = 240
/** How often the gesture hint engine's `tick()` re-checks its own
 *  time-driven conditions (`hints.ts`'s 8s no-orbit delay, and hint (b)'s
 *  own one-shot play-through ending) while a document is open and hints
 *  aren't gated off — well under either window's granularity, and cheap
 *  enough (a `Date.now()` diff, no DOM/kernel work) that a shorter period
 *  bought nothing. */
const HINT_TICK_MS = 500


/** The dock's segmented tool group (README §1 "Toolbar row"): icon + a
 *  visible label per segment, in `ShopToolName` order. Tape Measure's
 *  visible label is the prototype's shortened "Tape" (9.5px leaves no room
 *  for the full name at 70px segment width) — `ariaLabel` keeps the
 *  ACCESSIBLE name "Tape Measure" so screen readers and the existing
 *  test/E2E selectors (`getByRole('button', { name: /^tape measure$/i })`)
 *  see the same name as before the redesign. Select/Orbit need no
 *  `ariaLabel` override — their visible label already IS their full name. */
const TOOL_SEGMENTS: { name: ShopToolName; label: string; ariaLabel?: string; Icon: React.ComponentType<ShopIconProps> }[] = [
  { name: 'Select', label: 'Select', Icon: SelectIcon },
  { name: 'Orbit', label: 'Orbit', Icon: OrbitIcon },
  { name: 'Tape Measure', label: 'Tape', ariaLabel: 'Tape Measure', Icon: TapeIcon },
]

/** The same 3-name registry `TOOL_SEGMENTS` renders, as a `Set` for O(1)
 *  membership checks — `handleInternalToolChange`'s defense-in-depth
 *  allowlist (shop-mode adversarial-review finding 1, CRITICAL). Viewport's
 *  own `switchToolRef` is the primary enforcement point (it refuses any
 *  non-allowlisted switch under `readOnly` before anything else runs), but
 *  `onInternalToolChange` is Viewport reporting whatever it just settled on
 *  BACK to this component — a second, independent gate here means a defect
 *  in that primary check (or a future caller that reaches `setActiveTool`
 *  some other way) can't still land an editor-only tool name in Shop Mode's
 *  own `activeTool` state. */
const SHOP_TOOL_NAMES = new Set<string>(TOOL_SEGMENTS.map((s) => s.name))

/** Toast bottom offset above the fused dock+sheet object (design §9: "bottom-
 *  center above the dock"). The dock's own toolbar row is a fixed height
 *  (padding 10/12/8 around 52px content); PartsSheet reports its OWN
 *  current (detent- or drag-driven) height via `onHeightChange` so this can
 *  track the sheet riding up through half/full without the toast ending up
 *  underneath it — the same reason the dock itself rides above the sheet
 *  (module doc: the dock replaces the old hudBottomPx lift hack). */
const DOCK_ROW_HEIGHT_PX = 78

/** Landscape's right rail (design §5): icon-only 52×52 Select/Orbit/Tape
 *  segments (unlike the portrait dock's 70×50 icon+label ones — the rail
 *  has no room for a label at 52px wide) sharing the SAME `TOOL_SEGMENTS`
 *  order/icons/`ariaLabel`s as the portrait dock, so a screen reader (and
 *  the E2E's role queries) sees identical accessible names in either
 *  orientation even though the rail never renders the visible label. */
const RAIL_SEGMENT_PX = 52
/** Landscape's edge offsets (design §5): the title pill's top-left (and, via
 *  `DocumentMenu.tsx`, its own menu's anchor), the ⋯ button/rail's right
 *  edge (and, via `SettingsMenu.tsx`, its own menu's anchor). Playtest
 *  finding: the right rail was reported floating
 *  away from the edge on a phone with no cutout on that side — the ORIGINAL
 *  formula here (`max(16px, calc(env(safe-area-inset-right) + 8px))`)
 *  nests a fallback-less `env()` inside `calc()` inside `max()`, a
 *  composition some WebKit builds have historically mishandled (an
 *  unsupported/invalid `env()` reference invalidates the whole property at
 *  computed-value time per the CSS Env Variables spec, silently falling the
 *  offset back to `right: auto`'s default flow position instead of hugging
 *  the edge). Hardened to a flat `calc()` with an EXPLICIT `env()` fallback
 *  instead of the nested max/calc/env: `env(..., 0px)` never resolves to
 *  guaranteed-invalid, so the property can't collapse. Numerically
 *  equivalent at zero inset (both give 16px) but strictly more robust, and
 *  gives 8px MORE clearance than before once a real cutout is present. */
export const LANDSCAPE_LEFT_OFFSET_CSS = 'calc(env(safe-area-inset-left, 0px) + 16px)'
/** Exported alongside `TOP_STRIP_OFFSET_CSS` below — `SettingsMenu.tsx`'s
 *  landscape panel anchors off the same right edge the ⋯ button itself
 *  does. See `LANDSCAPE_LEFT_OFFSET_CSS`'s doc comment (mirrored) for why
 *  this dropped its old `max(16px, ...)` wrapper. */
export const LANDSCAPE_RIGHT_OFFSET_CSS = 'calc(env(safe-area-inset-right, 0px) + 16px)'

/** Horizontal clearance (css `right` value) the isolate banner/toast give
 *  the landscape right rail (design §5: "keep clamped inside the safe area
 *  between rail and sheet") — the rail's own content width (2 × 52px
 *  segments would be widest, but the rail is a single column so just one
 *  52px-ish button wide) plus its own padding (4px × 2) and edge offset,
 *  plus a small breathing gap so the banner/toast never touch it. Exported
 *  (adversarial-review finding 7) so `PartsSheet.tsx`'s landscape sheet
 *  width cap can subtract this SAME clearance from both sides rather than
 *  duplicating the math with its own flat pixel guess — a centered sheet
 *  that only budgets for the left edge collides with the rail on the right
 *  under a narrow-enough viewport. */
export const LANDSCAPE_RAIL_CLEARANCE_CSS = `calc(${RAIL_SEGMENT_PX}px + 8px + ${LANDSCAPE_RIGHT_OFFSET_CSS} + 12px)`

/** Recents thumbnail edge length (playtest finding 4) — small enough that a
 *  92×92 JPEG stays a few KB even for a busy model, comfortably inside the
 *  same IndexedDB record `recents.ts` already caps by total byte size;
 *  matches the 46px `recentThumbStyle` swatch's own CSS box at 2× for a
 *  crisp render on a typical phone's device pixel ratio. Exported so
 *  `ShopApp.test.tsx`'s own flip-fix test can assert the exact translate
 *  distance `buildRecentThumbnail`'s vertical-flip transform uses, rather
 *  than hardcoding the literal a second time. */
export const RECENT_THUMB_SIZE_PX = 92
/** JPEG quality for the recents thumbnail (playtest finding 4) — visibly
 *  fine for a small dock swatch, while keeping the encoded size trivial;
 *  no reason to spend more bits on a 92px image nobody zooms into. */
const RECENT_THUMB_QUALITY = 0.72

/**
 * Recents-row thumbnail: a `RECENT_THUMB_SIZE_PX` square "cover" crop of a
 * `captureFrame` result as a JPEG data URL (playtest finding 4) — the shared
 * `buildFrameThumbnail` with this shell's size/quality. `null` on failure;
 * callers treat that as "no thumbnail" (placeholder swatch), not an error.
 */
function buildRecentThumbnail(frame: { width: number; height: number; pixels: Uint8Array }): string | null {
  return buildFrameThumbnail(frame, RECENT_THUMB_SIZE_PX, RECENT_THUMB_SIZE_PX, RECENT_THUMB_QUALITY)
}

export function ShopApp() {
  const orientation = useShopOrientation()
  const isLandscape = orientation === 'landscape'

  const [scene, setScene] = useState<Scene | null>(null)
  const [kernelError, setKernelError] = useState<string | null>(null)
  const sceneRef = useRef<Scene | null>(null)
  sceneRef.current = scene

  const fileHostRef = useRef(makeFileHost())
  const viewportApi = useRef<ViewportApi | null>(null)

  // Document identity is display-only — Shop Mode tracks no dirty/save
  // state at all (it never mutates the document, so there is nothing to
  // save). `null` = nothing opened yet (the empty state below).
  const [docName, setDocName] = useState<string | null>(null)
  // The raw bytes behind whatever's currently loaded — kept around purely
  // so "Save a copy (.hew)" (docs/design/shop-mode.md §4's durable "Keep on
  // this phone" path) can hand them straight to an anchor-download without
  // re-serializing through the kernel (Shop Mode issues zero kernel
  // transactions — module doc — so there is no `scene.save()` call to make
  // here even if it wanted to). A ref, not state: read imperatively on a
  // menu click, never rendered.
  const lastBytesRef = useRef<Uint8Array | null>(null)

  const [activeTool, setActiveTool] = useState<ShopToolName>('Select')
  const [toolActivationSeq, setToolActivationSeq] = useState(0)
  // `activateTool`/`handleInternalToolChange` are declared further down
  // (just after `dismissInspectInstant` exists to call — see that
  // declaration's own doc comment for why playtest finding 8 needs it
  // here), but `activeTool` itself has to stay THIS early: the Tape
  // Measure touch-flow effect immediately below reads it.

  // ---------------------------------------------------------------- Tape Measure touch flow (finding 3)
  // The tool's own fixed gesture points (`Viewport`'s `onTapeMeasurePoints`
  // prop — `TapeMeasureTool.OnGesturePoint`'s doc has the full picture):
  // `[p0]` the instant a tap starts a new gesture, `[p0, p1]` the instant
  // the second tap commits it. Rendered as persistent world-anchored
  // markers below (`tapeAnchorsScreen`) since touch has no continuous hover
  // to drive the editor's own cursor-following dashed preview line.
  const [tapeAnchors, setTapeAnchors] = useState<[number, number, number][]>([])
  // A tool switch away from Tape Measure clears any markers left over from
  // the last gesture — they're meaningless (and visually stray) once the
  // tool they belong to is no longer active. Switching BACK to Tape
  // Measure starts clean too (the tool itself has no "current gesture" left
  // to report — the Viewport instantiates a fresh TapeMeasureTool on every
  // switch).
  useEffect(() => {
    if (activeTool !== 'Tape Measure') setTapeAnchors([])
  }, [activeTool])
  // A re-projection clock OWNED by this feature rather than borrowed from
  // the gesture-hint engine's own `hintTick`: that one's periodic component
  // is gated on `hintsAllowed` (hints.ts) and STOPS ticking entirely once
  // hints are exhausted/suppressed — the common steady state for a returning
  // user — which would leave these markers frozen at a stale screen position
  // for the whole duration of an orbit drag (`tapeAnchorsScreen`'s memo still
  // ALSO depends on `hintTick` for the immediate, unconditional camera-drag
  // start/end bump `handleCameraDragChange` fires regardless of
  // `hintsAllowed` — this interval is only for smooth tracking WHILE a drag
  // is in progress). Runs only while there's something to track.
  const [tapeReprojectTick, setTapeReprojectTick] = useState(0)
  useEffect(() => {
    if (tapeAnchors.length === 0) return
    const id = setInterval(() => setTapeReprojectTick((t) => t + 1), HINT_TICK_MS)
    return () => clearInterval(id)
  }, [tapeAnchors])

  const [activeContext, setActiveContext] = useState<NodeRef[]>([])
  const [selectedIds, setSelectedIds] = useState<NodeRef[]>([])

  // Long-press isolate's current target — declared up here (rather than down
  // by isolateNode/showAll below) because `pushHidden`/the Parts-sheet
  // toggles just below need to read it too (see pushHidden's doc comment).
  const [isolatedNode, setIsolatedNode] = useState<NodeRef | null>(null)

  // ---------------------------------------------------------------- view-state hides (Parts sheet + isolate)
  // Manual per-node hides and tag-section hides are Shop Mode view-state —
  // the Parts sheet's per-row/section eye toggles (PartsSheet.tsx), mirroring
  // App.tsx's DocumentTree/TagsPanel hiddenKeys/hiddenTagPaths but computed
  // through the SAME shared union walk (`unionHiddenLeafIds`,
  // documentLoad.ts) rather than a separate one. Long-press isolate composes
  // WITH these rather than replacing them — "Show all" only undoes the
  // isolate (see `pushHidden`/`showAll` below), so a part the sheet hid stays
  // hidden even after isolating and un-isolating something else. Never
  // persisted to the kernel (no `scene.set_node_user_hidden`/
  // `set_tag_hidden` calls, unlike App.tsx's equivalents) — module doc:
  // Shop Mode issues zero kernel transactions.
  const [hiddenKeys, setHiddenKeys] = useState<Set<string>>(new Set())
  const [hiddenTagPaths, setHiddenTagPaths] = useState<Set<string>>(new Set())

  // ---------------------------------------------------------------- Scenes (read-only — docs/design/scenes.md §6)
  // The document's Scenes never change during a Shop Mode session (zero
  // kernel mutations — module doc), so this is read ONCE per document open
  // (`applyOpenedBytes`) rather than re-derived on every render. `sid`s are
  // stable per document (kernel-minted), so a plain array is fine — no
  // Map/index needed for a handful of rows.
  const [sceneEntries, setSceneEntries] = useState<SceneEntry[]>([])
  const [activeSceneSid, setActiveSceneSid] = useState<number | null>(null)
  // Mirrors `activeSceneSid` synchronously (React state updates are async,
  // but `activateScene`/`refreshSceneDrift` below need the JUST-activated
  // sid immediately — same "state, plus a ref for same-tick reads" pattern
  // `useScenesController.ts`'s own `activeSidRef` uses for the identical
  // reason). Never read for rendering — `activeSceneSid` state is.
  const activeSceneSidRef = useRef<number | null>(null)
  const activateSceneRef = useRef<((sid: number, opts?: { instant?: boolean }) => void) | null>(null)
  const [sceneDrift, setSceneDrift] = useState<SceneDrift | null>(null)
  const activeSceneEntry = useMemo(
    () => (activeSceneSid === null ? null : (sceneEntries.find((e) => e.sid === activeSceneSid) ?? null)),
    [sceneEntries, activeSceneSid],
  )
  // A Scene reads as drifted from EITHER source (SPEC.md §2 "Drift & Show
  // all"): the kernel-computed camera/section/hidden-tag/hidden-node
  // mismatch (`sceneDrift`, refreshed on camera settle — see
  // `refreshSceneDrift` below), OR an active long-press isolate — which
  // `scene_drift` structurally CANNOT see (isolate is pure renderer state;
  // the kernel's own hidden-node/hidden-tag registries never change in Shop
  // Mode, module doc), so it's folded in here as a live boolean instead of
  // routed through the kernel at all.
  const activeSceneDrifted = activeSceneSid !== null && (driftAny(sceneDrift) || isolatedNode !== null)

  /**
   * Push the renderer-hidden set implied by (manual hides ∪ tag hides), plus
   * the long-press-isolate complement when `isolated` is non-null —
   * renderer-level only (`ViewportApi.setHidden`); never `scene.set_hidden`,
   * unlike App.tsx's `pushUnionHidden` (which also feeds the kernel's
   * inference-hide state — a real kernel call Shop Mode's view-state-only
   * posture never makes).
   *
   * All three set/node inputs are passed explicitly, not read from state, so
   * a caller can push the NEXT combination before its own `setState`
   * commits — `pushUnionHidden`'s same "state, not stale closure" reasoning.
   *
   * `fadeMs`, when given, opts into `ViewportApi.setHidden`'s isolate-fade
   * (design_handoff_shop_mode/README.md §4: "Other parts fade to opacity 0
   * over 240ms ease") — passed as a THIRD call argument only when defined,
   * never as an explicit `undefined`, so the Parts sheet's own eye-toggle
   * call sites (`toggleHiddenNode`/`toggleHiddenTagPath`, which never pass
   * it) keep calling `setHidden` with exactly the same two-argument shape
   * they always have (both for the editor's own call sites, unaffected by
   * this parameter entirely, and for this file's `setHidden` mock-assertion
   * tests, which check the exact argument list).
   */
  const pushHidden = useCallback(
    (nextHiddenKeys: Set<string>, nextHiddenTagPaths: Set<string>, isolated: NodeRef | null, fadeMs?: number) => {
      const scn = sceneRef.current
      if (scn === null) return
      const { objectIds, instanceIds } = unionHiddenLeafIds(scn, nextHiddenKeys, nextHiddenTagPaths)
      const hiddenObjectIds = new Set(objectIds)
      const hiddenInstanceIds = new Set(instanceIds)
      if (isolated !== null) {
        const getGroupMembers = (groupId: bigint): NodeRef[] =>
          Array.from(scn.group_members(groupId)).map((n) => ({ kind: n.kind as NodeRef['kind'], id: n.id }))
        const iso = isolateHiddenFor(
          isolated,
          Array.from(scn.object_ids()),
          Array.from(scn.instance_ids()),
          getGroupMembers,
        )
        for (const id of iso.hiddenObjectIds) hiddenObjectIds.add(id)
        for (const id of iso.hiddenInstanceIds) hiddenInstanceIds.add(id)
      }
      if (fadeMs !== undefined) {
        viewportApi.current?.setHidden([...hiddenObjectIds], [...hiddenInstanceIds], { fadeMs })
      } else {
        viewportApi.current?.setHidden([...hiddenObjectIds], [...hiddenInstanceIds])
      }
    },
    [],
  )

  /** Parts sheet: toggle one row's eye (a plain object/group/instance —
   *  the sheet never offers this for a sketch, which has no hidden-set
   *  membership to toggle in the first place). */
  const toggleHiddenNode = useCallback((node: NodeRef) => {
    setHiddenKeys((prev) => {
      const key = nodeKey(node)
      const next = new Set(prev)
      if (next.has(key)) next.delete(key)
      else next.add(key)
      pushHidden(next, hiddenTagPaths, isolatedNode)
      return next
    })
  }, [pushHidden, hiddenTagPaths, isolatedNode])

  /** Parts sheet: toggle a tag section's master eye. */
  const toggleHiddenTagPath = useCallback((path: string[]) => {
    setHiddenTagPaths((prev) => {
      const key = tagPathKey(path)
      const next = new Set(prev)
      if (next.has(key)) next.delete(key)
      else next.add(key)
      pushHidden(hiddenKeys, next, isolatedNode)
      return next
    })
  }, [pushHidden, hiddenKeys, isolatedNode])

  // ---------------------------------------------------------------- tap-to-inspect
  const [inspectResult, setInspectResult] = useState<InspectResult | null>(null)
  const [inspectPos, setInspectPos] = useState<{ x: number; y: number } | null>(null)
  // True while the card is running its 100ms exit fade (design §3 Motion
  // summary: "out 100ms ease-in") — `inspectResult`/`inspectPos` deliberately
  // stay non-null through that window (see `applyInspectResult` below) so
  // InspectCard keeps rendering the OUTGOING content while `leaving` swaps
  // its CSS class to the exit animation, the same `leaving`-prop pattern the
  // toast below already uses (a React unmount can't itself be animated).
  const [inspectLeaving, setInspectLeaving] = useState(false)
  // Bumped on every NEW (non-null) inspect result so InspectCard remounts —
  // forcing its `shop-inspect-in` entrance animation to replay even when the
  // same part is re-tapped back to back (a plain prop change wouldn't remount
  // it, so the CSS animation wouldn't restart).
  const [inspectSeq, setInspectSeq] = useState(0)
  const inspectDismissTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const clearInspectDismissTimer = useCallback(() => {
    if (inspectDismissTimerRef.current !== null) {
      clearTimeout(inspectDismissTimerRef.current)
      inspectDismissTimerRef.current = null
    }
  }, [])
  /** Every inspect-card state change funnels through here except the
   *  camera-movement instant kill (`dismissInspectInstant` below), which
   *  bypasses the exit fade entirely per the design's "camera movement
   *  dismisses instantly" rule. A non-null `result` replaces the card
   *  immediately (cancelling any pending exit) and bumps `inspectSeq` to
   *  replay the entrance; `null` starts the 100ms exit fade instead of
   *  clearing state right away. */
  const applyInspectResult = useCallback((result: InspectResult | null) => {
    clearInspectDismissTimer()
    if (result === null) {
      setInspectLeaving(true)
      inspectDismissTimerRef.current = setTimeout(() => {
        setInspectResult(null)
        setInspectPos(null)
        setInspectLeaving(false)
        inspectDismissTimerRef.current = null
      }, 100)
    } else {
      setInspectLeaving(false)
      setInspectResult(result)
      setInspectSeq((s) => s + 1)
    }
  }, [clearInspectDismissTimer])
  /** Camera movement's instant kill (design §3: "camera movement dismisses
   *  INSTANTLY — no exit animation") — skips `applyInspectResult`'s 100ms
   *  fade entirely. */
  const dismissInspectInstant = useCallback(() => {
    clearInspectDismissTimer()
    setInspectResult(null)
    setInspectLeaving(false)
  }, [clearInspectDismissTimer])

  // ---------------------------------------------------------------- dock/rail tool switch (playtest finding 8)
  /** The SAME function every one of the three portrait-dock/landscape-rail
   *  tool segments calls directly (`TOOL_SEGMENTS.map`'s own `onClick`,
   *  further down) — editing it here covers all six call sites (3 tools ×
   *  2 orientations) at once. Playtest finding 8: switching tools used to
   *  leave a stale inspect chip up, which visually "rode along" to
   *  whichever dock button was tapped next — cleared here (selection +
   *  instant inspect-card dismissal, the same "camera movement dismisses
   *  instantly" rule `dismissInspectInstant`'s own doc comment already
   *  documents) so no tool switch can ever leave one behind. Declared here,
   *  after `dismissInspectInstant` exists, rather than up by `activeTool`'s
   *  own state (this function used to live there) — it needs to CALL that
   *  function, which needs the tap-to-inspect section's state to exist
   *  first. */
  const activateTool = useCallback((name: ShopToolName) => {
    setActiveTool(name)
    if (CAMERA_HANDOFF_TOOL_NAMES.has(name)) setToolActivationSeq((s) => s + 1)
    setSelectedIds([])
    dismissInspectInstant()
    setInspectPos(null)
  }, [dismissInspectInstant])
  const handleInternalToolChange = useCallback((name: string) => {
    // Defense-in-depth allowlist (see `SHOP_TOOL_NAMES`'s own doc comment) —
    // ignore anything outside the 3-tool registry rather than blind-casting
    // it into `ShopToolName`.
    if (!SHOP_TOOL_NAMES.has(name)) return
    setActiveTool((prev) => (prev === name ? prev : (name as ShopToolName)))
  }, [])

  // The raw snap from the SAME tap `onSelect` just resolved a node from —
  // `inspect.ts`'s edge/face precedence needs both (see its module doc).
  const lastTapSnapRef = useRef<Snap | null>(null)
  const lastTapNodeRef = useRef<NodeRef | null>(null)
  // Declared here, above `applyOpenedBytes`, which disarms it on every
  // document swap (gesture section below owns arming/clearing).
  const longPressTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)

  const [inferenceInfo, setInferenceInfo] = useState<InferenceInfo | null>(null)
  const [measurement, setMeasurement] = useState('')
  const [measurementFrozen, setMeasurementFrozen] = useState(false)

  // Toast (design §9): single slot, in 180ms ease-out, auto-out after 4s
  // over 160ms (`.shop-toast-in`/`.shop-toast-out`, index.css). `leaving`
  // drives the exit animation — a React unmount can't itself be animated,
  // so the message stays mounted (opacity animating to 0 via the
  // `-out` class) for the 160ms fade before the timer clears it for real.
  const [toast, setToast] = useState<{ message: string; leaving: boolean } | null>(null)
  const toastTimersRef = useRef<{ hide: ReturnType<typeof setTimeout> | null; remove: ReturnType<typeof setTimeout> | null }>({ hide: null, remove: null })
  const showToast = useCallback((message: string) => {
    if (toastTimersRef.current.hide !== null) clearTimeout(toastTimersRef.current.hide)
    if (toastTimersRef.current.remove !== null) clearTimeout(toastTimersRef.current.remove)
    setToast({ message, leaving: false })
    toastTimersRef.current.hide = setTimeout(() => {
      setToast((cur) => (cur === null ? null : { ...cur, leaving: true }))
      toastTimersRef.current.remove = setTimeout(() => setToast(null), 160)
    }, 4000)
  }, [])

  // Two independent top-strip menus (maintainer-approved "Idea 2" split of
  // the old combined `OverflowMenu.tsx`) — the document pill's `DocumentMenu`
  // and the ⋯ button's `SettingsMenu`. Opening one closes the other (neither
  // needs to coexist, and both anchor to opposite top corners of the same
  // strip) rather than being strictly mutually exclusive by construction.
  const [documentMenuOpen, setDocumentMenuOpen] = useState(false)
  const [settingsMenuOpen, setSettingsMenuOpen] = useState(false)
  const openDocumentMenu = useCallback(() => {
    setSettingsMenuOpen(false)
    setDocumentMenuOpen((v) => !v)
  }, [])
  const openSettingsMenu = useCallback(() => {
    setDocumentMenuOpen(false)
    setSettingsMenuOpen((v) => !v)
  }, [])
  // Unit picker (design §7): a single shared modal reachable from TWO seams
  // — the Parts sheet header's own unit chip (PartsSheet.tsx's
  // `onOpenUnitPicker`) and the settings menu's Units row
  // (SettingsMenu.tsx's `onOpenUnitPicker`) — so its open/close state lives
  // up here rather than in either caller. It used to also carry a live
  // preview dimension (`previewDimensionM`, threaded up from PartsSheet's
  // sections) for a per-row preview value — Kurt's playtest cut that
  // feature entirely (rows are now unit name + active check only), so that
  // whole plumbing (this state, `onPreviewDimensionChange`, PartsSheet's
  // `firstVisibleLongestExtent`) is gone with it.
  const [unitPickerOpen, setUnitPickerOpen] = useState(false)
  const openUnitPicker = useCallback(() => {
    setSettingsMenuOpen(false)
    setUnitPickerOpen(true)
  }, [])
  // In-app QR scanner (`ScanSheet.tsx`) — reachable from the empty state's
  // "From your desktop…" button and the document menu's "Open from
  // desktop…" row alike, so its open state lives up here rather than in
  // either caller (same pattern as `unitPickerOpen` above).
  const [scanSheetOpen, setScanSheetOpen] = useState(false)
  const openScanner = useCallback(() => {
    setDocumentMenuOpen(false)
    setScanSheetOpen(true)
  }, [])
  // Recent models sheet (`RecentsSheet.tsx`) — the offline recents list
  // reachable WHILE a document is open (playtest: getting back to a
  // scanned handoff after opening a local file used to mean killing the
  // PWA to reach the empty state's Recents). Same open-state pattern as
  // the scanner above; reached from the Document menu.
  const [recentsSheetOpen, setRecentsSheetOpen] = useState(false)
  const openRecentsSheet = useCallback(() => {
    setDocumentMenuOpen(false)
    setRecentsSheetOpen(true)
  }, [])

  // ---------------------------------------------------------------- camera views (playtest finding 12)
  // `ViewsSheet.tsx`'s own module doc. Its open state lives up here, same
  // pattern as `unitPickerOpen`/`scanSheetOpen` above — reached from a NEW
  // dock action button in both orientations (the render below).
  const [viewsSheetOpen, setViewsSheetOpen] = useState(false)
  // Mirrors `ViewportApi.getProjection()` — `Viewport`'s own
  // `onProjectionChange` prop (already exists; wired below) reports it both
  // at mount and on every interactive/programmatic toggle, so this stays
  // accurate without ViewsSheet ever polling the api itself.
  const [projection, setProjection] = useState<Projection>('perspective')
  // Dock action button (playtest finding 8's own rule for every dock
  // action): clear any lingering selection/inspect chip on the tap, same as
  // Zoom Extents/AR — the sheet's own scrim then blocks further viewport
  // taps while it's up, so nothing NEW can appear before it closes.
  const openViewsSheet = useCallback(() => {
    setSelectedIds([])
    dismissInspectInstant()
    setInspectPos(null)
    setViewsSheetOpen(true)
  }, [dismissInspectInstant])
  const handleSelectStandardView = useCallback((view: StandardView) => {
    viewportApi.current?.setStandardView(view)
    setViewsSheetOpen(false)
  }, [])
  const toggleViewProjection = useCallback(() => {
    viewportApi.current?.toggleProjection()
  }, [])

  const containerRef = useRef<HTMLDivElement>(null)
  // `sheetDetents.ts`'s half/full fractions (deliberately left alone this
  // wave — see its own module doc) are fractions of "the container the
  // sheet floats over". Before the dock/top-strip restructure that WAS the
  // whole viewport wrapper, because the top strip used to be a normal-flow
  // sibling that shrank it; now the top strip floats OVER the (now
  // full-screen) `containerRef`, so measuring `containerRef` directly would
  // let a "full" sheet's dock ride up UNDER the top strip on a short
  // screen. This ref instead measures the space actually free for the
  // fused dock+sheet object — everything below the top strip's clearance —
  // preserving the pre-restructure proportions without touching
  // `sheetDetents.ts` itself. `containerRef` above is unchanged for
  // Viewport/InspectCard/gesture purposes, which legitimately DO want the
  // full screen.
  const sheetAreaRef = useRef<HTMLDivElement>(null)

  // The Parts sheet's SETTLED detent — lifted up here rather than a
  // `PartsSheet.tsx` local `useState` (an adversarial-review finding: the
  // portrait/landscape branches below render two STRUCTURALLY different
  // `PartsSheet` trees, so a rotation unmounts one and mounts the other —
  // React can't reconcile across that, and a local `useState('peek')` was
  // silently resetting to peek on every rotation instead of surviving it,
  // exactly the "detent lost on rotation" bug this state lives up here to
  // fix). `PartsSheet` is a controlled component for this value now
  // (`detent`/`onDetentChange` props); it still owns every bit of the
  // drag/snap MATH itself (live drag height, container measurement) —
  // only the settled value moved.
  const [detent, setDetent] = useState<SheetDetent>('peek')

  // The Parts sheet's own live height (detent- or drag-driven — PartsSheet
  // keeps owning that math, per the dock task's instruction) — read back up
  // only so the toast can float above the dock+sheet object wherever its
  // current top edge is (see DOCK_ROW_HEIGHT_PX's doc comment above).
  // Landscape used to have its own counterpart of this (`onSideWidthChange`
  // — the side sheet's live 44/340px width) so the isolate banner/toast
  // could stay clamped between it and the right rail. The side sheet is
  // gone (design correction — item 8): landscape now uses this SAME height
  // state, reported by the SAME `PartsSheet` component either orientation
  // renders (PartsSheet.tsx's module doc).
  const [sheetHeightPx, setSheetHeightPx] = useState(PEEK_HEIGHT_PX)

  // ---------------------------------------------------------------- gesture-discoverability hints (Wave 5)
  // `hints.ts`'s module doc for the full state machine — one `HintEngine`
  // for the session's lifetime (a ref, not state, same "stateful tracker
  // object" precedent as `multiClickRef` below); `activeHint` mirrors its
  // `getActive()` into React state so a change re-renders `HintOverlay`.
  // `hintTargetNodeRef` is the engine's own `targetPartId` (a plain string)
  // paired back up with the real `NodeRef` it came from, purely so the
  // render below can re-project its world bounds — the engine itself never
  // touches a `NodeRef`/`Scene` (module doc: UI/kernel-free, unit-testable
  // without either).
  const hintEngineRef = useRef<HintEngine | null>(null)
  if (hintEngineRef.current === null) hintEngineRef.current = new HintEngine()
  const [activeHint, setActiveHint] = useState<ActiveHint | null>(null)
  const hintTargetNodeRef = useRef<NodeRef | null>(null)
  const syncActiveHint = useCallback(() => {
    setActiveHint(hintEngineRef.current!.getActive())
  }, [])
  // Bumped on the periodic tick loop below AND on every camera-drag
  // start/end — the render's `hintDotScreen` memo depends on this, so it
  // re-projects (or hides, mid-drag) at each of those points rather than
  // only when `activeHint` itself changes identity.
  const [hintTick, setHintTick] = useState(0)
  // E2E-only: read once at mount (before-navigation `localStorage`, same
  // timing as `parseRecvParams`'s hash read below) — `hints.ts`'s
  // `HINT_TEST_SUPPRESS_KEY` doc comment covers why this exists and who
  // sets it.
  const [hintsSuppressedForTest] = useState(testHintsSuppressed)

  // ---------------------------------------------------------------- kernel boot
  useEffect(() => {
    loadKernel()
      .then((kernel) => {
        setScene(kernel.newScene())
      })
      .catch((err: unknown) => setKernelError(String(err)))
  }, [])

  // Locale-based unit default (design_handoff_shop_mode/README.md
  // "Decisions": "Default from locale (US/LR/MM -> Architectural, else
  // Meters)") — a ONE-TIME seed at Shop Mode's own boot, never touching
  // `settings/units.ts`'s own Meters default (see `localeUnits.ts`'s module
  // doc). Independent of the kernel boot above — the seed only needs to
  // land before anything reads `getLengthUnit()`, not before a document
  // loads.
  useEffect(() => {
    seedLocaleLengthUnit()
  }, [])

  // Shop-mode playtest finding 6: iOS Safari's double-tap-to-zoom ("smart
  // zoom") is a native UIKit gesture governed by the `<meta name="viewport">`
  // tag's `user-scalable`/`maximum-scale`, NOT by CSS `touch-action` — a
  // well-documented WebKit quirk (unlike Chrome/Android, where touch-action
  // alone is enough). `index.html`'s viewport tag (shared with the editor —
  // one HTML entry point, `shellMode.ts` picks the shell at runtime) has
  // neither set, so the OS gesture recognizer swallows the second tap of
  // every double-tap before `MultiClickTracker` ever sees it — this is why
  // the touch-action fix alone (the container's own `style` above) didn't
  // reliably help on a real phone despite passing in Playwright's touch
  // emulation, which doesn't model this native WebKit behavior at all. Set
  // ONLY while Shop Mode is mounted (never touches the shared HTML for the
  // editor) and restored on unmount for tidiness (Shop Mode is normally a
  // full-page session with no unmount in practice, but a Vite HMR reload in
  // dev shouldn't leave the tag stuck). JS-level content set/restore rather
  // than a second static tag (multiple `viewport` meta tags are
  // unreliable/inconsistently merged across browsers) or a build-time swap
  // (would need two HTML entry points for what's otherwise one shared page).
  useEffect(() => {
    const meta = document.querySelector('meta[name="viewport"]')
    if (meta === null) return
    const original = meta.getAttribute('content')
    if (original === null) return
    if (!original.includes('user-scalable')) {
      meta.setAttribute('content', `${original}, maximum-scale=1, user-scalable=no`)
    }
    return () => {
      meta.setAttribute('content', original)
    }
  }, [])

  // Offline recents (design §"Offline recents"): populate the empty state's
  // Recents list once at boot. Shop Mode is web-only (module doc), so unlike
  // App.tsx's equivalent this needs no isTauri gate. Best-effort — an
  // unavailable/failing IndexedDB already resolves to `[]` (recents.ts never
  // throws), so no catch is needed here either.
  const [recents, setRecents] = useState<RecentEntry[]>([])
  useEffect(() => {
    listRecents().then(setRecents)
  }, [])

  // ---------------------------------------------------------------- document open
  /**
   * Everything a successful `.hew` bytes load has in common regardless of
   * WHERE the bytes came from — the file picker (`openDocument`) or a tap
   * on a recents row (`openRecentEntry`, `PartsSheet`'s empty-state
   * sibling): reset the view-state that belonged to the PREVIOUS document,
   * seed + push the new one's hidden state, restore its camera, and record
   * it. Assumes `loadHewBytes(liveScene, bytes)` already ran and returned
   * ok — callers own the parse-failure branch themselves since their
   * failure messaging differs (a picker error vs. a corrupt recents entry).
   */
  const applyOpenedBytes = useCallback((liveScene: Scene, name: string, bytes: Uint8Array) => {
    // Round-3 playtest finding 2's real root cause: `Scene.load` (inside
    // `loadHewBytes`, called by every caller just before this) rebuilds the
    // kernel's inference state from scratch (`self.inference =
    // InferenceScene::new()`, wasm-api's own `load`), which resets
    // `axes_enabled` back to its default `true` — silently UNDOING the
    // `set_axes_snappable(false)` `Viewport`'s `showAxes={false}` prop
    // issued once at MOUNT, before any real document existed. Every actual
    // Shop Mode session opens at least one document, so axis snapping (and
    // its cue — CueLayer's `suppressAxisLine`/`excludeAxisSnapForSelect` in
    // Viewport.tsx are the OTHER, UI-level half of this same finding) was
    // silently back on for the entire time Kurt was ever looking at a real
    // model, not just some rare edge case. Re-issued here — the ONE choke
    // point every open path (picker, recents, QR) already funnels through —
    // rather than after each individual `loadHewBytes` call site.
    liveScene.set_axes_snappable(false)
    setSelectedIds([])
    setActiveContext([])
    dismissInspectInstant()
    setInspectPos(null)
    setIsolatedNode(null)
    // Scenes (docs/design/scenes.md §6): the PREVIOUS document's Scene
    // list/activation/drift belong to a sid space the new document reuses
    // for unrelated entities — reset before the new document's own list is
    // read further down, same "belongs to the outgoing document" reasoning
    // as `lastTapNodeRef`/`setTapeAnchors([])` just below.
    activeSceneSidRef.current = null
    setActiveSceneSid(null)
    setSceneDrift(null)
    // The last tap belongs to the OUTGOING document: its NodeRef/Snap key
    // dense ids the new document reuses for unrelated geometry, and this
    // open can land mid-gesture (the QR receive path resolves on a
    // background fetch). Drop them and disarm any pending long-press so a
    // held finger can't isolate — or an in-flight inspect can't describe —
    // the wrong document's part.
    lastTapNodeRef.current = null
    lastTapSnapRef.current = null
    if (longPressTimerRef.current !== null) {
      clearTimeout(longPressTimerRef.current)
      longPressTimerRef.current = null
    }
    // Finding 3: the outgoing document's Tape Measure markers, if any —
    // `viewportApi.current?.notifyLoaded()` below already rewinds a STILL-
    // active TapeMeasureTool via `onDocumentReset()`/`cancel()`, which now
    // fires `OnGesturePoint([])` itself (see that method's own doc comment),
    // so this is belt-and-suspenders for the same "stale world coordinates
    // from a document that's gone" reasoning as `lastTapNodeRef` just above,
    // not the only thing standing between a doc swap and a stray marker.
    setTapeAnchors([])
    // Seed from the just-loaded document's registries — the SAME
    // seedHiddenKeysFromRegistry/seedHiddenTagPathsFromRegistry App.tsx
    // uses — so Shop Mode opens with the same things hidden the full editor
    // would show: an imported .skp's hidden layers, or a re-opened .hew
    // with nodes/tags previously eye-toggled (in either shell).
    const seededHiddenKeys = seedHiddenKeysFromRegistry(liveScene)
    const seededHiddenTagPaths = seedHiddenTagPathsFromRegistry(liveScene)
    setHiddenKeys(seededHiddenKeys)
    setHiddenTagPaths(seededHiddenTagPaths)
    // Clear the renderer's hidden set first: it keys by dense ids the new
    // document reuses, so stale ids from the PREVIOUS document would
    // otherwise silently hide (and un-pick) unrelated objects for a frame
    // before the seeded push below lands (mirrors App.tsx's applyLoadedBytes).
    viewportApi.current?.setHidden([], [])
    viewportApi.current?.notifyLoaded()
    // The document's persisted section plane (docs/design/scenes.md §4) —
    // AFTER `notifyLoaded()`, which clears the viewport's own section state
    // as part of its reset. Independent of Scenes: a document can carry a
    // section plane with zero Scenes defined.
    const sectionPlaneJson = liveScene.section_plane_json()
    viewportApi.current?.setSectionPlane(
      sectionPlaneJson !== undefined ? (parseSectionJson(JSON.parse(sectionPlaneJson)) ?? null) : null,
    )
    // Scenes list (docs/design/scenes.md §6): read once — Shop Mode never
    // mutates it (module doc), so there is nothing to keep in sync with
    // later. The Views sheet's "SCENES" section and the pill both read
    // this; empty for a document with none.
    const openedScenes = parseScenesJson(liveScene.scenes_json())
    setSceneEntries(openedScenes)
    setDocName(name)
    lastBytesRef.current = bytes
    // Push the seeded hides now that the scene is tessellated (notifyLoaded
    // above), so hidden-by-default tags/nodes take effect on first render
    // instead of waiting for the user to touch an eye toggle.
    pushHidden(seededHiddenKeys, seededHiddenTagPaths, null)
    // Shop Mode is web-only (shellMode.ts — the desktop shell never enters
    // it), so unlike App.tsx's equivalent this needs no isTauri check.
    // Fire-and-forget: a lost write must never interrupt the open itself.
    // Re-recording an entry that was ITSELF opened from recents is
    // deliberate — it bumps that entry back to most-recently-used, the LRU
    // behavior a "recents" list is expected to have.
    // partCount (design §8's empty-state Recents rows: "2 hours ago · 9
    // parts") — the SAME buildPartsSheetSections/totalPartCount PartsSheet
    // itself renders from, computed once here at open time rather than
    // threaded out of the mounted component; App.tsx's own two call sites
    // are left passing no count (no equivalent "parts sheet" concept to
    // read one from there without a larger refactor). Reused below for
    // hint (a)'s target too, rather than walking the sections twice.
    const openSections = buildPartsSheetSections(liveScene, seededHiddenKeys, seededHiddenTagPaths)
    const partCount = totalPartCount(openSections)
    void recordRecent(bytes, name, 'open', undefined, partCount)

    // Recents thumbnail (playtest finding 4): captured AFTER a FITTED
    // framing call has posed the camera, in the SAME requestAnimationFrame
    // callback (not a separate/later one) — `ViewportApi.captureFrame`'s
    // own doc comment (Viewport.tsx) is why timing doesn't need to be any
    // more careful than that: it performs its own out-of-band
    // `renderer.render()` immediately before `gl.readPixels`, synchronously,
    // inside the call itself, so it always reads back exactly what IT just
    // drew — the "no preserveDrawingBuffer" hazard the loupe copy elsewhere
    // in Viewport.tsx has to worry about (a DIFFERENT capture path, reading
    // back the CONTINUOUS render loop's own last-drawn frame) doesn't apply
    // here. "Fitted" — `zoomExtents()` — regardless of which branch below
    // schedules the capture: a saved camera is often zoomed OUT (a wide
    // working view from the last edit session), so that branch fits the
    // camera JUST for this capture and then restores the saved view
    // immediately after (its own doc comment below has the detail). A
    // second `recordRecent` call for the SAME content-hash merges the
    // thumbnail into the row the call above just wrote (`recents.ts`'s own
    // dedupe doc comment) rather than creating a duplicate. Best-effort
    // throughout: an empty document (no framing call at all — the `else`
    // branch below), a missing `viewportApi`, or any capture/canvas failure
    // simply leaves `thumbnail` unset, falling back to the placeholder swatch.
    const captureRecentThumbnail = (): void => {
      // Bail if a newer document opened before this frame fired (shop-mode
      // playtest adversarial review): the rAF closures below are scheduled
      // per-open, but only the LATEST open is still current — capturing here
      // for a superseded document would read back the new scene and persist it
      // against the old document's content-hash. `bytes` is this open's own
      // buffer; `lastBytesRef.current` tracks the newest open.
      if (lastBytesRef.current !== bytes) return
      try {
        const frame = viewportApi.current?.captureFrame()
        if (frame === undefined) return
        const thumbnail = buildRecentThumbnail(frame)
        if (thumbnail !== null) void recordRecent(bytes, name, 'open', undefined, partCount, thumbnail)
      } catch {
        // Best-effort — falls back silently to the placeholder swatch.
      }
    }

    // Gesture-discoverability hint (a) (`hints.ts`'s module doc): point at
    // the largest currently-visible part, or nothing if there isn't one.
    // `hintTargetNodeRef` keeps the real NodeRef for the render below to
    // re-project; the engine itself only ever sees its stable string id.
    const hintTarget = largestVisiblePart(openSections)
    hintTargetNodeRef.current = hintTarget
    hintEngineRef.current!.documentOpened(hintTarget === null ? null : nodeKey(hintTarget))
    syncActiveHint()

    // Frame the freshly loaded model — the saved camera view if the
    // document carries one (docs/design/camera.md §5), else Zoom Extents
    // for a non-empty model, else the viewport's own default framing.
    // Mirrors App.tsx's applyLoadedBytes (the extraction this shares
    // `loadHewBytes`/`isSceneEmpty` with covers only the parse step; the
    // camera-restore choreography is chrome-specific and stays here).
    const savedCamera = liveScene.camera_state()
    if (savedCamera !== undefined) {
      try {
        const applyState = {
          projection: savedCamera.projection() as 'perspective' | 'parallel',
          fovDeg: savedCamera.fov_deg(),
          eye: [savedCamera.eye_x(), savedCamera.eye_y(), savedCamera.eye_z()] as [number, number, number],
          target: [savedCamera.target_x(), savedCamera.target_y(), savedCamera.target_z()] as [number, number, number],
          up: [savedCamera.up_x(), savedCamera.up_y(), savedCamera.up_z()] as [number, number, number],
        }
        requestAnimationFrame(() => {
          if (lastBytesRef.current !== bytes) return // superseded by a newer open
          // Playtest fix: the saved camera is often zoomed OUT (a wide
          // working view from the last edit session), which used to produce
          // an under-filled thumbnail. `zoomExtents()` re-poses the camera
          // SYNCHRONOUSLY (no tween — same guarantee the no-saved-camera
          // branch below already relies on for its own single-rAF capture),
          // so this briefly frames the model tight JUST for the capture,
          // then restores the user's own saved view as the LAST paint this
          // callback makes — the user still ends up exactly where they left
          // off; only the persisted thumbnail is framed differently.
          viewportApi.current?.zoomExtents()
          captureRecentThumbnail()
          viewportApi.current?.applyCameraState(applyState)
        })
      } finally {
        savedCamera.free()
      }
    } else if (!isSceneEmpty(liveScene)) {
      requestAnimationFrame(() => {
        if (lastBytesRef.current !== bytes) return // superseded by a newer open
        viewportApi.current?.zoomExtents()
        captureRecentThumbnail()
      })
    }
    // An empty scene frames nothing real (module doc's own reasoning for
    // the isSceneEmpty gate above) — no thumbnail worth capturing either;
    // that document's recents row just keeps the placeholder swatch.
    //
    // A document with Scenes opens on its FIRST Scene (docs/design/scenes.md
    // §5/§6, playtest; same as the desktop and SketchUp) — activated
    // instantly, no tween, in a rAF registered AFTER the camera-restore /
    // thumbnail rAF above so it runs after it and the Scene's camera wins.
    if (openedScenes.length > 0) {
      const firstSid = openedScenes[0].sid
      requestAnimationFrame(() => {
        if (lastBytesRef.current !== bytes) return // superseded by a newer open
        activateSceneRef.current?.(firstSid, { instant: true })
      })
    }
  }, [pushHidden, dismissInspectInstant, syncActiveHint])

  // ---------------------------------------------------------------- QR handoff receive path
  // "Open on Phone" (workers/share-relay/README.md, shareCrypto.ts's module
  // doc): the desktop encrypts the document client-side, uploads the
  // ciphertext to a one-shot dead-drop, and encodes `#recv=<token>.<key>.
  // <name>` on the URL the QR shows. `receiveDrop` — the shared
  // fetch-decrypt-load path — is called from TWO places, each REQUIRED to
  // declare its own `source` (adversarial-review finding 1, CRITICAL):
  // `confirmReceive` below (a LINK-ARRIVED handoff — the boot-time hash
  // effect further down, or any future non-scanner arrival — only ever
  // reaches this after the user explicitly taps "Open" on
  // `ReceiveConfirmSheet`) and `handleScanDecoded` (ScanSheet.tsx's in-app
  // scanner, wired further down — the scan act itself is the
  // authorization, so it calls straight through with no gate). share-
  // relay's `/drop` PUT has no auth (README.md's "Security model" — a
  // token is a bearer capability anyone who has it can redeem), so a
  // forged `#recv=…` link is otherwise indistinguishable from a real QR
  // scan by the time it reaches this function; `source` exists so THIS
  // function's two call sites are forced to state which trust story
  // applies, rather than the gate being enforced by convention alone — and,
  // since it's already in hand, it also picks the right noun ("code" vs.
  // "link") for the failure toasts below rather than telling a link-arrived
  // failure to go "scan the QR again".
  const receiveDrop = useCallback(async (params: RecvParams, source: 'scanner' | 'link') => {
    const noun = source === 'scanner' ? 'code' : 'link'
    let ciphertext: Uint8Array
    try {
      // The relay lives next to THIS page — `<origin>/relay/` (io/shareRelay.ts):
      // a self-hosted PWA reads from its own relay, the public one from the
      // Workers route on app.hew3d.com. Same origin, so no CORS either way.
      const response = await fetch(`${phoneRelayBase()}/drop/${params.token}`)
      if (!response.ok) {
        // A 404 here means the token was already consumed, expired (10
        // minutes — share-relay's own TTL), or never existed — from the
        // phone's point of view all three just read as "expired".
        showToast(`That ${noun} has expired — ask for a fresh one from the desktop.`)
        return
      }
      ciphertext = new Uint8Array(await response.arrayBuffer())
    } catch {
      showToast('Could not reach the share server — check your internet connection and try again.')
      return
    }
    let bytes: Uint8Array
    try {
      bytes = await decrypt(fromBase64Url(params.key), ciphertext)
    } catch {
      // SubtleCrypto's AES-GCM auth-tag check failed (shareCrypto.ts's
      // `decrypt` doc comment) — a wrong/mangled key, almost certainly from
      // a QR that didn't decode cleanly (or a link that got mangled in
      // transit). Nothing to recover from client-side (the ciphertext is
      // already consumed — share-relay's one-shot GET — so even a retry
      // can't help); point back at the desktop for a fresh one.
      showToast(`That ${noun} could not be decrypted — ask for a fresh one from the desktop.`)
      return
    }
    const liveScene = sceneRef.current
    if (liveScene === null) return
    const loaded = loadHewBytes(liveScene, bytes)
    if (!loaded.ok) {
      showToast(friendlyErrorText(loaded.error))
      return
    }
    applyOpenedBytes(liveScene, params.name, bytes)
  }, [showToast, applyOpenedBytes])

  // A LINK-ARRIVED `#recv=…` handoff awaiting the user's explicit
  // confirmation (adversarial-review finding 1, CRITICAL) — captured at
  // MOUNT (see the hash-stripping effect below), rendered by
  // `ReceiveConfirmSheet` once the kernel is ready (module doc's boot
  // effect gates every other open affordance the same way — `scene !==
  // null` in the render below). `null` means either nothing arrived on the
  // hash, or it already resolved (Open tapped → cleared here and handed to
  // `receiveDrop`; Cancel tapped → cleared here with no further action).
  const [pendingReceive, setPendingReceive] = useState<RecvParams | null>(null)

  // Boot-time `#recv=` hash — runs at MOUNT, independent of kernel
  // readiness (adversarial-review finding 8, MINOR): the token+key are
  // sensitive (share-relay's bearer-capability token, the AES key) and
  // must not sit in the address bar/history for the whole span of
  // `loadKernel()` (a real wasm fetch+instantiate, not instant) — captured
  // into `pendingReceive` state and stripped from the URL in the SAME
  // synchronous effect, not deferred until `scene` is ready. `#shop` is
  // preserved if it was already part of the hash (a boot straight into
  // Shop Mode that also happened to carry `#recv=…`); otherwise the hash
  // is stripped bare — the ordinary case for a camera-app scan landing on
  // the canonical origin, where `#shop` was never there to begin with (the
  // auto-detect heuristic in shellMode.ts routes a real phone into Shop
  // Mode regardless, and now also force-routes on a bare `#recv=…` itself
  // — see that file's own `resolveShellMode` doc, finding 2).
  //
  // This effect only ever CAPTURES params and strips the hash — it never
  // calls `receiveDrop` itself. That is finding 1's actual gate: a
  // link-arrived handoff has exactly ONE path to `receiveDrop`
  // (`confirmReceive`, reachable only from `ReceiveConfirmSheet`'s Open
  // button), so there is no code path by which this effect — or any
  // future one — can skip the confirmation UI, not just a convention that
  // says it shouldn't.
  useEffect(() => {
    const currentHash = window.location.hash
    const params = parseRecvParams(currentHash)
    if (params === null) return
    const strippedHash = currentHash.startsWith('#shop') ? '#shop' : ''
    window.history.replaceState(null, '', window.location.pathname + window.location.search + strippedHash)
    setPendingReceive(params)
    // Mount-only: `window.location.hash` is read exactly once, at boot —
    // matching the ORIGINAL effect's own single-run behavior (a `[scene]`
    // dependency that only ever transitioned null->non-null once per
    // session). A SPA never navigates to a second `#recv=…` without a full
    // reload, so there is no later hash for a re-run to observe anyway.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  const confirmReceive = useCallback(() => {
    const params = pendingReceive
    setPendingReceive(null)
    if (params === null) return
    void receiveDrop(params, 'link')
  }, [pendingReceive, receiveDrop])

  const cancelReceive = useCallback(() => {
    // The hash was already stripped at mount (the effect above), so
    // there's nothing further to undo here — clearing the pending state is
    // the whole cancellation. A reload after Cancel lands on the normal
    // empty/recents state, not a re-prompt (module doc's own "reload
    // doesn't re-prompt" invariant).
    setPendingReceive(null)
  }, [])

  // ScanSheet.tsx's in-app scanner hands back the parsed params the moment
  // it decodes a valid handoff (its own camera is already stopped by then)
  // — the scan act itself is the user's authorization (adversarial-review
  // finding 1's design), so this closes the sheet and runs `receiveDrop`
  // directly, with NO confirmation gate — unlike the link-arrived path
  // above.
  const handleScanDecoded = useCallback((params: RecvParams) => {
    setScanSheetOpen(false)
    void receiveDrop(params, 'scanner')
  }, [receiveDrop])

  // Semantic E2E harness `window.__hew_shop_test` (testHarness.ts) — loads a
  // fixture document without automating a real OS file picker, the same
  // problem `App.tsx`'s own harness solves for the editor. Debug/test builds
  // only, same gate as `App.tsx`'s `installTestHarness` call.
  const loadBytesViaHarness = useCallback((bytes: Uint8Array): boolean => {
    const liveScene = sceneRef.current
    if (liveScene === null) return false
    const loaded = loadHewBytes(liveScene, bytes)
    if (!loaded.ok) return false
    applyOpenedBytes(liveScene, 'Fixture.hew', bytes)
    return true
  }, [applyOpenedBytes])
  useEffect(() => {
    if (!(import.meta.env.DEV || import.meta.env.VITE_HEW_TEST === '1')) return
    return installShopTestHarness({
      getScene: () => sceneRef.current,
      getViewportApi: () => viewportApi.current,
      loadBytes: loadBytesViaHarness,
    })
  }, [loadBytesViaHarness])

  const openDocument = useCallback(async () => {
    const liveScene = sceneRef.current
    if (liveScene === null) return
    setDocumentMenuOpen(false)
    let pick: OpenPick | null
    try {
      pick = await fileHostRef.current.openAny()
    } catch (err: unknown) {
      showToast(`Open failed: ${friendlyErrorText(err)}`)
      return
    }
    if (pick === null) return // user cancelled
    if (pick.kind !== 'hew') {
      // Import formats (.skp/.dae/.gltf/.stl) all run through a kernel
      // import call that creates new objects — a real mutation. Shop Mode
      // issues none (module doc); point the user at the full editor rather
      // than silently downgrading to a read-only partial import.
      showToast('Shop Mode can only open .hew files — use the full editor to import other formats.')
      return
    }
    const loaded = loadHewBytes(liveScene, pick.bytes)
    if (!loaded.ok) {
      showToast(friendlyErrorText(loaded.error))
      return
    }
    applyOpenedBytes(liveScene, pick.name, pick.bytes)
  }, [showToast, applyOpenedBytes])

  // Offline recents (design §"Offline recents"): tap a row in the empty
  // state's Recents list. The bytes are already in hand (IndexedDB, not a
  // re-fetch), so this skips straight to loadHewBytes — no picker round trip.
  const openRecentEntry = useCallback((entry: RecentEntry) => {
    const liveScene = sceneRef.current
    if (liveScene === null) return
    setDocumentMenuOpen(false)
    const loaded = loadHewBytes(liveScene, entry.bytes)
    if (!loaded.ok) {
      showToast(friendlyErrorText(loaded.error))
      return
    }
    applyOpenedBytes(liveScene, entry.name, entry.bytes)
  }, [showToast, applyOpenedBytes])

  // "Use full editor": persist the override and reload — the editor's own
  // "Shop Mode" entry point (design's symmetric affordance) is a later
  // stage's addition to App.tsx, not this effort's.
  const useFullEditor = useCallback(() => {
    writeShellModeOverride('editor')
    window.location.reload()
  }, [])

  // "Save a copy (.hew)" (design §4's durable "Keep on this phone → Save
  // to Files" path): a plain anchor-download of whatever's currently
  // loaded, using the SAME bytes that were opened — no kernel involved
  // (module doc). A no-op if nothing is loaded (the menu item itself is
  // hidden in that case — see the render below).
  const saveCopy = useCallback(() => {
    setDocumentMenuOpen(false)
    const bytes = lastBytesRef.current
    if (bytes === null || docName === null) return
    anchorDownload(bytes, docName)
  }, [docName])

  // ---------------------------------------------------------------- selection / tap-to-inspect
  const handleSelect = useCallback((node: NodeRef | null, _additive: boolean) => {
    lastTapNodeRef.current = node
    setSelectedIds(node === null ? [] : [node])
    const scn = sceneRef.current
    const result = scn === null ? null : resolveInspect(scn, node, lastTapSnapRef.current)
    applyInspectResult(result)
    // Gesture hint (a) (`hints.ts`): a tap that resolved to something
    // inspectable (edge OR whole-part) is the gesture it teaches —
    // `tapped()` kills it (or pre-satisfies it if it hasn't fired yet).
    if (result !== null) {
      hintEngineRef.current!.tapped()
      syncActiveHint()
    }
  }, [applyInspectResult, syncActiveHint])

  const handleSelectSnap = useCallback((snap: Snap | null) => {
    lastTapSnapRef.current = snap
  }, [])

  // Anchor the card at the tap's container-relative position. `onSelect`/
  // `onSelectSnap` carry no screen coordinates (they report WHAT was under
  // the tap, not WHERE on screen) — a plain pointerdown-capture listener on
  // the viewport wrapper (below `Viewport`'s own canvas in the DOM, so this
  // runs AFTER Viewport's internal handling for the same physical tap,
  // same ordering `App.tsx`'s wrapper `onPointerLeave` already relies on)
  // supplies the coordinate half without touching Viewport at all.
  const handleWrapperPointerDown = useCallback((ev: React.PointerEvent<HTMLDivElement>) => {
    const rect = containerRef.current?.getBoundingClientRect()
    if (rect !== undefined) {
      setInspectPos({ x: ev.clientX - rect.left, y: ev.clientY - rect.top })
    }
  }, [])

  // Dismiss on camera movement (design: "Card dismisses on tap-elsewhere or
  // camera movement") — also doubles as the signal that cancels a long-press
  // isolate that's mid-timer once the same drag turns out to be an orbit.
  const cameraDragActiveRef = useRef(false)
  const handleCameraDragChange = useCallback((active: boolean) => {
    cameraDragActiveRef.current = active
    if (active) {
      dismissInspectInstant()
      setInspectPos(null)
      // Gesture hint (b) (`hints.ts`): a camera drag starting is treated as
      // "found the camera" — see `HintEngine.orbited`'s own doc comment for
      // why rotate/pan/pinch all routing through this one signal is an
      // accepted simplification. Also bumps `hintTick` so hint (a)'s dot
      // hides IMMEDIATELY rather than lagging up to one tick behind the
      // drag start (`hintDotScreen`'s memo, below).
      hintEngineRef.current!.orbited()
    }
    setHintTick((t) => t + 1)
    syncActiveHint()
  }, [dismissInspectInstant, syncActiveHint])

  // ---------------------------------------------------------------- gestures: double-tap + long-press
  const multiClickRef = useRef(new MultiClickTracker())
  // longPressTimerRef is declared up in the tap-to-inspect section so
  // applyOpenedBytes (defined between there and here) can disarm it.
  const pressStartRef = useRef<{ x: number; y: number } | null>(null)

  const clearLongPressTimer = useCallback(() => {
    if (longPressTimerRef.current !== null) {
      clearTimeout(longPressTimerRef.current)
      longPressTimerRef.current = null
    }
  }, [])

  // Every caller of this is a PROGRAMMATIC camera jump (a button/row tap,
  // never a drag) — design: "camera movement dismisses instantly", the same
  // rule `handleCameraDragChange`'s drag case already enforces. An
  // adversarial-review finding: this used to only cover drags, so the dock/
  // rail Zoom Extents button, a Parts-sheet row tap, and double-tap zoom all
  // left a now-stale inspect card up over the new framing. Dismissing here
  // — the one function every one of those call sites already routes
  // through (including the bare "zoom extents" case, called with `node:
  // null`) — covers all of them from one seam instead of repeating the
  // dismiss at each call site.
  //
  // Playtest finding 8: a `node === null` call is always a BARE zoom-extents
  // request — the dock/rail Zoom Extents button (nothing targeted), or a
  // double-tap that landed on empty space (which already cleared
  // `selectedIds` via `handleSelect(null, ...)` a moment earlier, so this is
  // a no-op re-clear there) — never a targeted zoom (`handleSheetSelectRow`/
  // a double-tap ON a part always pass a real `node`, and explicitly set
  // `selectedIds` to it themselves). Clearing here only in the null case
  // means the Zoom Extents button can never leave a stale selection/inspect
  // chip behind, without disturbing the targeted callers' own selection.
  const zoomToNodeOrExtents = useCallback((node: NodeRef | null) => {
    dismissInspectInstant()
    setInspectPos(null)
    if (node === null) setSelectedIds([])
    const scn = sceneRef.current
    if (node !== null && scn !== null) {
      const bounds = worldBoundsForSelection(scn, [node])
      if (bounds !== null) {
        viewportApi.current?.zoomToWorldBounds([...bounds.min], [...bounds.max])
        return
      }
    }
    viewportApi.current?.zoomExtents()
  }, [dismissInspectInstant])

  // Parts sheet: tap a row → "highlight the part + zoom to it" (design) —
  // the same selection state tap-to-inspect already drives (Viewport
  // renders `selectedIds` with a highlight) plus the existing zoom helper;
  // no separate highlight mechanism needed. Never opens an inspect card
  // itself — that's still the viewport tap gesture's own affordance
  // (module doc's "Tap to inspect" section), not the sheet's — but DOES
  // dismiss whatever card was already up, as a side effect of
  // `zoomToNodeOrExtents`'s own "camera movement dismisses instantly" rule
  // (that helper's own doc comment): a row tap moves the camera exactly
  // like the dock's Zoom Extents button does, so a stale card from a
  // BEFORE this tap must not survive it.
  const handleSheetSelectRow = useCallback((node: NodeRef) => {
    setSelectedIds([node])
    zoomToNodeOrExtents(node)
  }, [zoomToNodeOrExtents])

  const isolateNode = useCallback((node: NodeRef) => {
    setIsolatedNode(node)
    pushHidden(hiddenKeys, hiddenTagPaths, node, ISOLATE_FADE_MS)
  }, [pushHidden, hiddenKeys, hiddenTagPaths])

  // ---------------------------------------------------------------- Scenes activation (docs/design/scenes.md §6)
  // Applies a resolved Scene's CAPTURED hidden-objects/visible-tags
  // properties into `hiddenKeys`/`hiddenTagPaths` (App.tsx's equivalent
  // panel state — `useScenesController.ts`'s `applyResolved` mirrored,
  // minus its `scene.set_hidden` kernel call, which Shop Mode never makes),
  // then pushes the result through `pushHidden` — the SAME renderer-only
  // path long-press isolate uses, with the SAME fade. A property the Scene
  // did NOT capture is left exactly as it was (design's "None = not
  // captured, don't touch"). Shared by `activateScene` and `showAll`'s own
  // "return to the Scene's hidden set" branch below, so both stay in sync
  // by construction rather than by two independently-maintained copies.
  const applyResolvedHidden = useCallback((resolved: {
    has_hidden(): boolean
    has_hidden_tags(): boolean
    hidden_tag_paths(): string[]
    has_hidden_nodes(): boolean
    hidden_node_kinds(): Uint8Array
    hidden_node_ids(): BigUint64Array
  }) => {
    let nextHiddenKeys = hiddenKeys
    let nextHiddenTagPaths = hiddenTagPaths
    if (resolved.has_hidden_tags()) {
      nextHiddenTagPaths = new Set(
        resolved.hidden_tag_paths().map((p) => tagPathKey(p.split('/').map((seg) => seg.trim()).filter((seg) => seg.length > 0))),
      )
      setHiddenTagPaths(nextHiddenTagPaths)
    }
    if (resolved.has_hidden_nodes()) {
      const kinds = resolved.hidden_node_kinds()
      const ids = resolved.hidden_node_ids()
      const kindNames: NodeRef['kind'][] = ['object', 'group', 'instance']
      const keys = new Set<string>()
      for (let i = 0; i < kinds.length; i++) {
        const kind = kindNames[kinds[i]]
        if (kind !== undefined) keys.add(nodeKey({ kind, id: ids[i] }))
      }
      nextHiddenKeys = keys
      setHiddenKeys(keys)
    }
    if (resolved.has_hidden()) {
      pushHidden(nextHiddenKeys, nextHiddenTagPaths, null, ISOLATE_FADE_MS)
    }
  }, [hiddenKeys, hiddenTagPaths, pushHidden])

  // Recomputes the active Scene's drift (SPEC.md §2 "Drift & Show all") —
  // called from `activateScene`, `Viewport`'s `onCameraSettled` (fires
  // ~250ms after ANY camera move settles: orbit, a standard view, zoom
  // extents, or this file's own camera tween — docs/design/scenes.md §5),
  // and after `showAll`'s own re-resolve. Display is deliberately never
  // compared (`undefined` — SPEC.md §2 "Never on the phone": grid/axes/
  // guides are ignored in Shop Mode, so they can never legitimately drift
  // here). Reads `activeSceneSidRef` (not the `activeSceneSid` STATE) so a
  // just-activated sid is visible on the same tick `activateScene` sets it,
  // before React has committed the state update.
  const refreshSceneDrift = useCallback(() => {
    const scn = sceneRef.current
    const sid = activeSceneSidRef.current
    if (scn === null || sid === null) {
      setSceneDrift(null)
      return
    }
    try {
      const cam = viewportApi.current?.getCameraState()
      const json = scn.scene_drift(BigInt(sid), cam === undefined ? undefined : JSON.stringify(cam), undefined)
      setSceneDrift(parseDriftJson(json))
    } catch {
      setSceneDrift(null)
    }
  }, [])

  // Activation (SPEC.md §2, docs/design/scenes.md §6): PURE `resolve_scene`
  // — never `apply_scene` (that mutates the kernel's own document state;
  // Shop Mode issues zero kernel transactions, module doc) — feeding its
  // renderer-level leaf ids straight to `ViewportApi.setHidden`/
  // `setSectionPlane`/`tweenCameraState`. Isolate is cleared first (design's
  // "Isolate is cleared on activation"); display is never touched (SPEC.md
  // §2 "Never on the phone").
  const activateScene = useCallback((sid: number, opts?: { instant?: boolean }) => {
    const scn = sceneRef.current
    if (scn === null) return
    setIsolatedNode(null)
    let resolved
    try {
      resolved = scn.resolve_scene(BigInt(sid))
    } catch (err) {
      showToast(`Activate Scene failed: ${friendlyErrorText(err)}`)
      return
    }
    try {
      applyResolvedHidden(resolved)
      if (resolved.has_section()) {
        const sj = resolved.section_json()
        const plane = sj === undefined ? null : (parseSectionJson(JSON.parse(sj)) ?? null)
        viewportApi.current?.setSectionPlane(plane)
      }
      const cj = resolved.camera_json()
      const api = viewportApi.current
      if (cj !== undefined && api !== null) {
        const cam = parseCameraJson(JSON.parse(cj))
        if (cam !== undefined) {
          api.tweenCameraState(cam, getSceneTransitions() && opts?.instant !== true ? SCENE_TRANSITION_MS : 0, () => refreshSceneDrift())
        }
      }
    } finally {
      resolved.free()
    }
    activeSceneSidRef.current = sid
    setActiveSceneSid(sid)
    refreshSceneDrift()
  }, [applyResolvedHidden, showToast, refreshSceneDrift])
  // `applyOpenedBytes` (declared above) activates the first Scene on open
  // through this ref — same declaration-order reason as the other refs.
  activateSceneRef.current = activateScene

  /** Views sheet / pill chevrons: the neighbor in tab order, wrapping —
   *  `null` (no Scenes at all) is a no-op. */
  const stepScene = useCallback((dir: 1 | -1) => {
    const next = neighborScene(sceneEntries, activeSceneSidRef.current, dir)
    if (next !== null) activateScene(next.sid)
  }, [sceneEntries, activateScene])

  // Undoes ONLY the isolate — whatever the Parts sheet hid stays hidden
  // (pushHidden's doc comment). Under an ACTIVE Scene (SPEC.md §2 "Drift &
  // Show all"), this returns to the SCENE's hidden set — not
  // everything-visible — by re-resolving it fresh (rather than trusting
  // whatever `hiddenKeys`/`hiddenTagPaths` happen to hold, which could have
  // drifted from the Scene's own capture via a Parts-sheet eye toggle in
  // the meantime) and clears the drift isolate itself caused (the
  // `activeSceneDrifted` memo's `isolatedNode !== null` term clears the
  // instant `setIsolatedNode(null)` below commits; the explicit
  // `refreshSceneDrift()` catches any camera/section drift already present
  // too). No active Scene: unchanged prior behavior.
  const showAll = useCallback(() => {
    setIsolatedNode(null)
    const sid = activeSceneSidRef.current
    const scn = sceneRef.current
    if (sid !== null && scn !== null) {
      try {
        const resolved = scn.resolve_scene(BigInt(sid))
        try {
          applyResolvedHidden(resolved)
        } finally {
          resolved.free()
        }
      } catch {
        // Best-effort — the isolate is already cleared above regardless.
      }
      refreshSceneDrift()
      return
    }
    pushHidden(hiddenKeys, hiddenTagPaths, null, ISOLATE_FADE_MS)
  }, [pushHidden, hiddenKeys, hiddenTagPaths, applyResolvedHidden, refreshSceneDrift])

  const handleWrapperPointerDownCapture = useCallback((ev: React.PointerEvent<HTMLDivElement>) => {
    pressStartRef.current = { x: ev.clientX, y: ev.clientY }
    clearLongPressTimer()
    // Tape Measure's own HELD-press isolate now comes straight from the
    // Viewport (`onIsolateRequest` above), driven by the live press snap —
    // this timer instead isolates `lastTapNodeRef.current`, which only the
    // Select tool's own `onSelect` ever sets, so under Tape Measure it would
    // still be whatever was last tapped in Select mode (or null): a STALE
    // target with no relation to the current press. Arming it here too would
    // let a Tape-mode long-press isolate that stale part out from under the
    // Viewport's own live-snap isolate. Select/Orbit are unaffected — the
    // wrapper timer stays the only isolate path either of those tools has.
    if (activeTool === 'Tape Measure') return
    longPressTimerRef.current = setTimeout(() => {
      longPressTimerRef.current = null
      // Only a whole part isolates — see isWholePartKind's doc comment;
      // a long-press over empty space or a sketch is a no-op.
      const node = lastTapNodeRef.current
      if (node !== null && (node.kind === 'object' || node.kind === 'group' || node.kind === 'instance')) {
        isolateNode(node)
      }
    }, LONG_PRESS_MS)
  }, [clearLongPressTimer, isolateNode, activeTool])

  const handleWrapperPointerMove = useCallback((ev: React.PointerEvent<HTMLDivElement>) => {
    const start = pressStartRef.current
    if (start === null) return
    const dx = ev.clientX - start.x
    const dy = ev.clientY - start.y
    if (dx * dx + dy * dy > LONG_PRESS_SLOP_PX * LONG_PRESS_SLOP_PX) {
      clearLongPressTimer()
    }
  }, [clearLongPressTimer])

  const handleWrapperPointerUp = useCallback((ev: React.PointerEvent<HTMLDivElement>) => {
    clearLongPressTimer()
    pressStartRef.current = null
    // Shop-mode playtest finding 5: drop the shared SnapService's held
    // sticky snap now that this tap's own pick has already resolved
    // (Viewport's internal `pointerup` handler — a native listener straight
    // on the canvas — runs and calls `onSelect`/`onSelectSnap` BEFORE this
    // React-dispatched wrapper handler does, same ordering
    // `handleWrapperPointerDown`'s own doc comment relies on). Touch has no
    // continuous hover to organically release a held sticky snap between
    // taps the way a mouse would — see `SnapService.clearHold`'s doc.
    viewportApi.current?.clearSnapHold()
    const press: MultiClickPress = {
      timeStamp: ev.timeStamp,
      clientX: ev.clientX,
      clientY: ev.clientY,
      button: ev.button,
      pointerType: ev.pointerType,
    }
    // `press` (not `release`) decides the count: a double-tap's SECOND
    // touch is a fresh press-and-lift, and MultiClickTracker's own release
    // bookkeeping (drag-vs-click) is keyed off the PREVIOUS press, so
    // calling both in the natural down/up order here mirrors exactly how
    // Viewport's own multi-click detection drives the same tracker.
    const count = multiClickRef.current.press(press)
    multiClickRef.current.release(press)
    if (count === 2) {
      zoomToNodeOrExtents(lastTapNodeRef.current)
    }
  }, [clearLongPressTimer, zoomToNodeOrExtents])

  // ---------------------------------------------------------------- rescale confirmation (Tape Measure)
  // Tape Measure can arm a "resize the model?" confirmation when a typed
  // distance implies a scale change — a real kernel mutation
  // (rescale_document/rescale_session). Shop Mode issues none (module doc):
  // decline it immediately rather than stranding the tool waiting on a
  // confirmation UI Shop Mode has no chrome for.
  const handleRescaleArmed = useCallback(() => {
    viewportApi.current?.cancelPendingRescale()
  }, [])

  // ---------------------------------------------------------------- View in AR (iOS Quick Look)
  // design §"View in AR (Shop Mode, iOS)": the "View in AR…" row in
  // `DocumentMenu` (rendered only when `isArQuickLookCandidate()` — iOS
  // Safari only, arQuickLook.ts; used to be a dock/rail button, moved into
  // the document menu once the toolbar reshuffle dropped it — see
  // `DocumentMenu.tsx`'s own module doc) exports the live scene to USDZ
  // through the SAME `ViewportApi.exportUsdz` App.tsx's desktop Export
  // dialog calls, then hands the bytes to `launchArQuickLook` instead of a
  // save-picker. Non-iOS browsers never see the row at all — no
  // downloads-as-fallback here (design's fallback posture: the USDZ export
  // already lives in the desktop Export dialog).
  const [arBusy, setArBusy] = useState(false)
  const viewInAr = useCallback(async () => {
    // Busy-guard against double-taps: exportUsdz tessellates the whole
    // scene, so a second tap mid-export would just duplicate that work.
    if (arBusy) return
    const api = viewportApi.current
    if (api === null) return
    // Playtest finding 8: AR used to be a dock action button too — clear any
    // lingering selection/inspect chip on the tap, same as Zoom Extents and
    // every tool-switch segment. Harmless now that AR is reached through the
    // document menu instead (the menu's own scrim already blocks further
    // viewport taps while it's up), and still correct: nothing else clears
    // an inspect card left over from BEFORE the menu opened.
    setSelectedIds([])
    dismissInspectInstant()
    setInspectPos(null)
    setArBusy(true)
    try {
      let bytes: Uint8Array | null
      try {
        bytes = await api.exportUsdz()
      } catch (err: unknown) {
        showToast(`Export failed: ${friendlyErrorText(err)}`)
        return
      }
      if (bytes === null) {
        // Mirrors App.tsx's exportUsdz toast for the same null-bytes case
        // (crates/mesh-export's Scene::export refuses when the document has
        // no solids at all).
        showToast('Nothing to export — the model has no solids.')
        return
      }
      launchArQuickLook(bytes, (docName ?? 'Model').replace(/\.hew$/i, ''))
    } finally {
      setArBusy(false)
    }
  }, [arBusy, docName, showToast, dismissInspectInstant])

  // The document menu's own "View in AR…" row dispatches here — closes the
  // menu (same "act, then close" order `saveCopy`'s own doc comment
  // predates) and fires the real export/hand-off above. A separate wrapper
  // rather than passing `viewInAr` straight to `DocumentMenu` because that
  // row has no other seam to close the menu from (unlike `openScanner`'s own
  // wrapper around `setScanSheetOpen`, `viewInAr` itself has no menu-close
  // concern — it's also called from nowhere else that would want one).
  const viewInArFromDocumentMenu = useCallback(() => {
    setDocumentMenuOpen(false)
    void viewInAr()
  }, [viewInAr])

  const hasDocument = docName !== null

  // Isolate banner (design §4 "Isolate state"): the fused dock's toolbar
  // row (below) has no slot for the old floating "Show all" HUD chip it
  // replaces, so this is the isolate-undo affordance's new — and only —
  // home; without it, long-press isolate would have no escape once the old
  // HUD chips were removed. `resolveInspect` (already used for tap-to-
  // inspect above) is reused purely for its label resolution, with a null
  // snap — this never touches InspectCard itself.
  const isolatedLabel = useMemo(() => {
    if (isolatedNode === null || scene === null) return null
    const result = resolveInspect(scene, isolatedNode, null)
    return result !== null && result.kind === 'node' ? result.label : null
  }, [isolatedNode, scene])

  // The Scene pill no longer shares this row (portrait: bottom, above the
  // dock; landscape: on the top strip's own row), so the banner sits at
  // its original offset in both orientations.
  const isolateBannerTopCss = `calc(${TOP_STRIP_OFFSET_CSS} + 54px)`

  const inspectCardEl = useMemo(() => {
    if (inspectResult === null || inspectPos === null) return null
    const rect = containerRef.current?.getBoundingClientRect()
    return (
      <InspectCard
        key={inspectSeq}
        result={inspectResult}
        leaving={inspectLeaving}
        screenX={inspectPos.x}
        screenY={inspectPos.y}
        containerWidth={rect?.width ?? window.innerWidth}
        containerHeight={rect?.height ?? window.innerHeight}
      />
    )
  }, [inspectResult, inspectPos, inspectLeaving, inspectSeq])

  // ---------------------------------------------------------------- gesture-discoverability hints (Wave 5), continued
  // Whether the sheet is at its CLOSED/peek resting state — both
  // orientations now read this off the SAME `sheetHeightPx` (PartsSheet
  // reports it in either orientation, its own module doc), rather than a
  // new prop threaded out of `PartsSheet`'s own detent state.
  const sheetAtRest = sheetHeightPx <= PEEK_HEIGHT_PX
  // Hints never render in the empty state, during AR busy, or while a
  // menu/picker/sheet-half-or-full is covering the viewport (task's own
  // gating list) — reusing state this file already tracks for other
  // reasons rather than adding new props/flags for it.
  const hintsAllowed = hasDocument && !hintsSuppressedForTest && !documentMenuOpen && !settingsMenuOpen && !unitPickerOpen && !viewsSheetOpen && !scanSheetOpen && !arBusy && sheetAtRest

  // The periodic re-evaluation `HintEngine.tick()` needs for the 8s
  // no-orbit delay and hint (b)'s own one-shot play-through to end on their
  // own (module doc: the engine owns no timer itself). Runs ONLY while
  // hints are actually allowed to show — pausing while gated (rather than
  // ticking silently behind a menu) means a hint can never spend its one
  // "fires once per install" shot unseen (`hints.ts`'s own doc comment on
  // `tick()`).
  useEffect(() => {
    if (!hintsAllowed) return
    const id = setInterval(() => {
      hintEngineRef.current!.tick()
      syncActiveHint()
      setHintTick((t) => t + 1)
    }, HINT_TICK_MS)
    return () => clearInterval(id)
  }, [hintsAllowed, syncActiveHint])

  // Hint (a)'s dot position: re-projected via `ViewportApi.worldToScreen`
  // on every tick/camera-drag-edge/hint change (`hintTick`'s own doc
  // comment) — cheap (a single matrix multiply), so no dedicated animation-
  // frame loop is needed just for this (design's own "cheap: re-project on
  // the same camera-change signal... or hide during motion" allowance).
  // `null` while mid camera-drag, off-screen, or missing any input it needs
  // — `HintOverlay` renders nothing for 'tap' without one, rather than a
  // stale/wrong position.
  const hintDotScreen = useMemo(() => {
    if (activeHint?.name !== 'tap' || cameraDragActiveRef.current) return null
    const node = hintTargetNodeRef.current
    const scn = sceneRef.current
    const api = viewportApi.current
    if (node === null || scn === null || api === null) return null
    const bounds = worldBoundsForSelection(scn, [node])
    if (bounds === null) return null
    const center: [number, number, number] = [
      (bounds.min[0] + bounds.max[0]) / 2,
      (bounds.min[1] + bounds.max[1]) / 2,
      (bounds.min[2] + bounds.max[2]) / 2,
    ]
    const projected = api.worldToScreen(center)
    if (projected.behind) return null
    return { x: projected.x, y: projected.y }
    // eslint-disable-next-line react-hooks/exhaustive-deps -- `hintTick` is
    // the deliberate re-projection clock (module doc); `activeHint` is the
    // other real dependency. `sceneRef`/`viewportApi`/`hintTargetNodeRef`
    // are refs, read live, not reactive inputs.
  }, [activeHint, hintTick])

  // Tape Measure's anchor markers (finding 3), reprojected on the exact
  // same clock as the gesture-hint dot above (`hintTick` — camera-drag edges
  // + the periodic tick) — "world-anchored... like the hint dot's
  // projection" is the task's own description. Unlike the hint dot, these
  // don't hide mid camera-drag: they're not trying to draw attention to a
  // one-shot teaching moment, just staying visually pinned to the two
  // measured world points while the user orbits to check the result from
  // another angle. Off-screen points (`behind`) are dropped rather than
  // clamped — a marker for a point currently behind the camera has nothing
  // honest to show.
  const tapeAnchorsScreen = useMemo(() => {
    const api = viewportApi.current
    if (api === null || tapeAnchors.length === 0) return []
    const out: { x: number; y: number }[] = []
    for (const p of tapeAnchors) {
      const projected = api.worldToScreen(p)
      if (!projected.behind) out.push({ x: projected.x, y: projected.y })
    }
    return out
    // eslint-disable-next-line react-hooks/exhaustive-deps -- `hintTick`
    // catches the unconditional camera-drag start/end bump
    // (`handleCameraDragChange`, never gated on `hintsAllowed`);
    // `tapeReprojectTick` is this feature's OWN clock for smooth tracking
    // during an in-progress drag once hints are exhausted (its doc comment
    // above has the full reasoning). `viewportApi` is a ref, read live.
  }, [tapeAnchors, hintTick, tapeReprojectTick])

  const tapeMeasureOverlayEl = useMemo(() => {
    if (tapeAnchorsScreen.length === 0) return null
    const [a, b] = tapeAnchorsScreen
    return (
      <svg
        aria-hidden="true"
        data-testid="tape-measure-overlay"
        style={{ position: 'absolute', inset: 0, width: '100%', height: '100%', pointerEvents: 'none', zIndex: 25 }}
      >
        {b !== undefined && (
          <line
            x1={a.x} y1={a.y} x2={b.x} y2={b.y}
            stroke="var(--shop-accent)" strokeWidth={2} strokeDasharray="6 5" opacity={0.9}
          />
        )}
        {tapeAnchorsScreen.map((p, i) => (
          <circle
            // eslint-disable-next-line react/no-array-index-key -- fixed-length (0-2), order-stable pair (p0 first, p1 second); no key that survives reordering is available or needed.
            key={i}
            data-testid="tape-anchor-marker"
            cx={p.x} cy={p.y} r={7}
            fill="var(--shop-accent)" stroke="#fff" strokeWidth={3}
            style={{ filter: 'drop-shadow(0 1px 4px rgba(0,0,0,.3))' }}
          />
        ))}
      </svg>
    )
  }, [tapeAnchorsScreen])

  const hintOverlayEl = useMemo(() => {
    if (!hintsAllowed || activeHint === null) return null
    const rect = containerRef.current?.getBoundingClientRect()
    return (
      <HintOverlay
        hint={activeHint}
        dotScreen={hintDotScreen}
        containerWidth={rect?.width ?? window.innerWidth}
        containerHeight={rect?.height ?? window.innerHeight}
      />
    )
  }, [hintsAllowed, activeHint, hintDotScreen])

  return (
    <div style={{ position: 'fixed', inset: 0, overflow: 'hidden', background: 'var(--shop-dock)' }}>
      {/* Fullscreen viewport — canvas full-bleed behind every chrome overlay
          below (design §1 "Status bar area"). Everything else (top strip,
          isolate banner, the fused dock+sheet, toasts, the empty state) is
          an absolutely-positioned child of this SAME element rather than a
          layout-affecting sibling — this is what makes the canvas truly
          full-bleed instead of squeezed under a fixed-height top bar the
          way the pre-redesign chrome worked. */}
      <div
        ref={containerRef}
        data-testid="shop-viewport"
        // Shop-mode playtest finding 6: `OrbitControls` already sets
        // `touch-action: none` on the CANVAS itself (three.js's own
        // `connect()`/`dispose()`, unconditional, editor included) to fully
        // own pinch/pan-drag — that's why pinch-zoom already worked before
        // this fix. But iOS Safari's double-tap-to-zoom ("smart zoom") is a
        // native UIKit gesture keyed off the WKWebView's own zoomability
        // (the `<meta name="viewport">` tag's `user-scalable`), not
        // per-element `touch-action` — the mount effect near the top of this
        // component (the one that edits `meta[name="viewport"]`) is the fix
        // that actually matters there. `touch-action: none` here on the WRAPPER (not just
        // the canvas) is defense-in-depth for the corners/overlays around
        // the canvas and for browsers (Chrome/Android) that DO honor
        // touch-action for double-tap — matches the shop-only scope every
        // other finding-6 change keeps (the editor's own container never
        // sets this).
        style={{ position: 'relative', width: '100%', height: '100%', touchAction: 'none' }}
        onPointerDownCapture={handleWrapperPointerDownCapture}
        onPointerDown={handleWrapperPointerDown}
        onPointerMove={handleWrapperPointerMove}
        onPointerUp={handleWrapperPointerUp}
        onPointerLeave={() => setInferenceInfo(null)}
      >
        {scene !== null && (
          <Viewport
            wasmScene={scene}
            readOnly
            background="shop-gradient"
            // Shop Mode is a clean, product-photo-style viewer — every
            // design screenshot (design_handoff_shop_mode/screenshots/
            // 02-04, 06, 10-12) shows the model floating on the plain
            // gradient backdrop with no CAD ground grid or origin-axis
            // lines, unlike the full editor. Also load-bearing for the
            // gradient itself — see `showGrid`/`showAxes`'s doc comment on
            // `Viewport`'s Props for why this has to be a mount-time prop
            // rather than a post-mount `ViewportApi` call.
            showGrid={false}
            showAxes={false}
            apiRef={viewportApi}
            activeTool={activeTool}
            activeToolSeq={toolActivationSeq}
            onInternalToolChange={handleInternalToolChange}
            activeContext={activeContext}
            onEnterContext={(node) => setActiveContext((cur) => [...cur, node])}
            onExitContext={() => setActiveContext((cur) => cur.slice(0, -1))}
            onExitAllContexts={() => setActiveContext([])}
            selectedIds={selectedIds}
            onSelect={handleSelect}
            onSelectSnap={handleSelectSnap}
            // Shop-mode playtest: Tape Measure's own HELD-press isolate
            // (Viewport.tsx's own `onIsolateRequest` doc comment) — the
            // isolate TARGET only ever comes from THIS live press snap while
            // Tape Measure is active, never the wrapper's own long-press
            // timer (`handleWrapperPointerDownCapture` below disarms that
            // timer for the exact same tool, so the two isolate paths never
            // race each other). Mirrors that timer's own "only a whole part
            // isolates" rule.
            onIsolateRequest={(node) => {
              if (node.kind === 'object' || node.kind === 'group' || node.kind === 'instance') isolateNode(node)
            }}
            onInferenceChange={setInferenceInfo}
            onMeasurement={(text, frozen) => {
              setMeasurement(text)
              setMeasurementFrozen(frozen ?? false)
            }}
            onTapeMeasurePoints={(points) => setTapeAnchors([...points])}
            onRescaleArmed={handleRescaleArmed}
            onCameraDragChange={handleCameraDragChange}
            // Scenes drift (docs/design/scenes.md §5/§6): fires ~250ms after
            // ANY camera move settles (orbit, a standard view, zoom extents,
            // or this file's own camera tween) — the SAME debounced signal
            // the editor's `useScenesController` keys drift refresh off.
            // A no-op when no Scene is active (`refreshSceneDrift`'s own
            // early-return).
            onCameraSettled={refreshSceneDrift}
            onToast={(message) => showToast(message)}
            // Views sheet (playtest finding 12): mirrors the live
            // parallel/perspective state into React so `ViewsSheet` can
            // show which projection segment is active — an already-
            // existing optional Viewport prop (App.tsx's own Camera menu
            // checkbox reads it the same way), not new Viewport surface.
            onProjectionChange={setProjection}
            // The following editor-chrome callbacks are deliberately left
            // unwired (Viewport treats every one of them as optional and
            // no-ops internally when absent) rather than stubbed with an
            // explicit no-op function, because each is tied to a tool or
            // panel entirely outside Shop Mode's 3-tool registry and so is
            // structurally unreachable here: onOpenAnnotationEditor/
            // onSampleMaterial/currentMaterialId (Text/Paint),
            // onSessionChange (group/component session breadcrumb chrome —
            // sessions themselves still open/close via Viewport's own
            // internal double-click handling; Shop Mode just renders no
            // breadcrumb for it), onSectionChanged (Section Plane),
            // onToolReverted (Zoom Window's one-shot revert), selectedGuide/
            // onSelectGuide/selectedAnnotation/onSelectAnnotation/
            // onSelectMany (marquee/guide selection — Select still resolves
            // single-node taps fine without them), onDocumentChanged/
            // onHistoryChanged (no tree/undo chrome to refresh),
            // onHoverSketchRegionChange (no contextual dock).
            // `onProjectionChange` USED to be on this unwired list too (no
            // Camera menu checkbox to sync) — playtest finding 12's
            // `ViewsSheet` gave Shop Mode its own projection-aware chrome,
            // so it's wired above now.
          />
        )}

        <SnapDot info={inferenceInfo} />
        <InferenceTooltip info={inferenceInfo} />
        <MeasurementBox
          toolName={activeTool}
          value={measurement}
          frozen={measurementFrozen}
          variant="shop"
          orientation={orientation}
          // Playtest fix 5: both docking spots clear the live Parts sheet via
          // its tracked height. Portrait's lower-right spot also clears the
          // fused bottom dock row (`DOCK_ROW_HEIGHT_PX`); landscape's lower-
          // left spot has no bottom dock (the rail is on the side), so it
          // lifts by the sheet height alone — enough to clear the bottom-
          // center width-capped sheet whose left edge crowds this corner at
          // narrow widths (shop-mode playtest adversarial review).
          bottomOffsetPx={orientation === 'landscape' ? sheetHeightPx : sheetHeightPx + DOCK_ROW_HEIGHT_PX}
        />
        {inspectCardEl}
        {tapeMeasureOverlayEl}
        {hintOverlayEl}

        {/* Invisible — purely a measurement anchor for PartsSheet's detent
            math (see `sheetAreaRef`'s doc comment above). */}
        <div ref={sheetAreaRef} aria-hidden="true" style={{ position: 'absolute', top: `calc(${TOP_STRIP_OFFSET_CSS} + 54px)`, left: 0, right: 0, bottom: 0, pointerEvents: 'none' }} />

        {/* Empty state (design §8): nothing opened yet — charcoal
            full-bleed, mark + title + the two primary actions + Recents.
            Gated on `hasDocument` (not the old always-mounted placeholder)
            since it now fully replaces whatever's behind it, matching "No
            other copy" — a floating dock with nothing loaded to act on
            would contradict that. Recents (design §"Offline recents") reads
            the same IndexedDB store (`io/recents.ts`) the web WelcomeScreen's
            own Recents shelf does, so a model opened via the desktop
            editor's picker shows up here too. Each row's "N parts" suffix
            (design §8) reads `RecentEntry.partCount`, recorded at THIS
            shell's own open seam (`applyOpenedBytes` above) — a recent that
            predates that field, or one recorded by the desktop editor's own
            `App.tsx` call sites, simply omits the suffix. */}
        {!hasDocument && scene !== null && (
          <div
            style={{
              position: 'absolute', inset: 0, zIndex: 10, overflow: 'hidden',
              // --surface-sheet, not --shop-dock: the empty state is a
              // full-screen SURFACE, not floating chrome — it reads its
              // text/wash colors off --shop-text/--shop-text-muted below,
              // the same theme-aware pair the Parts sheet/menu/picker
              // already use on that surface (playtest theme-honoring fix).
              background: 'var(--surface-sheet)',
              display: 'flex', flexDirection: 'column', alignItems: 'center',
              padding: `max(24px, calc(env(safe-area-inset-top) + 24px)) 24px max(24px, calc(env(safe-area-inset-bottom) + 24px))`,
            }}
          >
            <div style={{ flex: '1 1 auto', minHeight: 0, display: 'flex', flexDirection: 'column', alignItems: 'center', justifyContent: 'center', gap: '18px' }}>
              <HewMark size={72} strokeWidth={4.6} />
              <span style={{ fontFamily: 'var(--font-family-ui)', fontSize: '22px', fontWeight: 600, color: 'var(--shop-text)' }}>
                Shop Mode
              </span>
              <div style={{ display: 'flex', flexDirection: 'column', gap: '10px', width: '100%', maxWidth: '320px', marginTop: '8px' }}>
                <button type="button" className="shop-press" onClick={openDocument} style={emptyPrimaryButtonStyle}>
                  <UploadIcon size={20} />
                  Open a model…
                </button>
                {/* Opens ScanSheet.tsx's in-app QR scanner — the empty
                    state's own seam onto the same "Open on Phone" handoff
                    the overflow menu's "Open from desktop…" row also
                    opens. */}
                <button
                  type="button"
                  className="shop-press"
                  onClick={() => setScanSheetOpen(true)}
                  style={emptyGhostButtonStyle}
                >
                  <QrIcon size={20} />
                  From your desktop…
                </button>
              </div>
            </div>

            {recents.length > 0 && (
              <div style={{ flex: '0 0 auto', width: '100%', maxWidth: '360px', display: 'flex', flexDirection: 'column', gap: '10px', paddingBottom: '8px', overflowY: 'auto' }}>
                <span style={{
                  fontFamily: 'var(--font-family-ui)', fontSize: '11px', fontWeight: 700,
                  letterSpacing: '0.08em', textTransform: 'uppercase', color: 'var(--shop-text-muted)',
                }}>
                  Recents
                </span>
                <RecentsList entries={recents} onOpen={openRecentEntry} limit={5} />
              </div>
            )}
          </div>
        )}

        <RecentsSheet
          open={recentsSheetOpen}
          orientation={orientation}
          entries={recents}
          onClose={() => setRecentsSheetOpen(false)}
          onOpen={openRecentEntry}
        />

        <ScanSheet
          open={scanSheetOpen}
          orientation={orientation}
          onClose={() => setScanSheetOpen(false)}
          onDecoded={handleScanDecoded}
        />

        {/* Adversarial-review finding 1 (CRITICAL): the confirmation gate a
            LINK-ARRIVED `#recv=…` handoff must clear before it ever loads —
            see `pendingReceive`'s own doc comment above for the full threat
            model. Gated on `scene !== null` like every other open
            affordance (the empty state's own buttons, just below, share the
            exact same gate) — the hash is already stripped by then
            regardless (finding 8's mount-time effect), so this is purely
            about not asking the user to decide before the kernel that would
            act on "Open" even exists yet. */}
        <ReceiveConfirmSheet
          name={scene !== null ? (pendingReceive?.name ?? null) : null}
          orientation={orientation}
          onCancel={cancelReceive}
          onOpen={confirmReceive}
        />

        {/* Top strip (design §1, split per the maintainer-approved "Idea 2"):
            a charcoal pill (mark + doc name — now itself a button opening
            `DocumentMenu`) and a charcoal ⋯ button (opening `SettingsMenu`),
            each floating independently rather than one continuous bar — the
            wrapper's own `pointerEvents:'none'` keeps the gap between them
            from stealing viewport taps/drags; the two children opt back into
            `pointerEvents:'auto'`. Rendered regardless of `hasDocument`
            (unlike the old top bar's "Hew — Shop Mode" placeholder text,
            this shows "Shop Mode" too) — the pill/`DocumentMenu` pair is the
            ONLY way to reach "Use full editor" before a document is even
            open, and the empty state's own charcoal background means it
            reads as one continuous surface with this strip rather than a
            visible seam. */}
        <div
          style={{
            position: 'absolute', top: TOP_STRIP_OFFSET_CSS,
            left: isLandscape ? LANDSCAPE_LEFT_OFFSET_CSS : '12px',
            right: isLandscape ? LANDSCAPE_RIGHT_OFFSET_CSS : '12px',
            display: 'flex', justifyContent: 'space-between', alignItems: 'center',
            zIndex: 40, pointerEvents: 'none',
          }}
        >
          {/* iOS document-app convention (Pages/Files): the pill is a real
              button opening `DocumentMenu`, mirroring the ⋯ button's own
              corner-occupying hide-while-open treatment just below — the
              panel's header takes over showing the filename the instant the
              pill itself hides. */}
          {!documentMenuOpen && (
            <button
              type="button"
              aria-label={`Document menu — ${docName ?? 'Shop Mode'}`}
              className="shop-press"
              onClick={openDocumentMenu}
              style={topStripPillStyle}
            >
              <HewMark size={17} />
              <span style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                {docName ?? 'Shop Mode'}
              </span>
              <ChevronDownIcon size={13} style={{ flexShrink: 0, opacity: 0.7 }} />
            </button>
          )}
          {/* iOS convention (playtest item 3): while the settings menu's own
              anchored panel is open, it visually takes over this button's
              top-right corner — so the ⋯ button itself hides while
              `settingsMenuOpen`, restored the instant the panel closes. */}
          {!settingsMenuOpen && (
            <div style={{ pointerEvents: 'auto' }}>
              <button type="button" aria-label="Settings" className="shop-press" onClick={openSettingsMenu} style={ellipsisButtonStyle}>
                <EllipsisIcon size={20} />
              </button>
            </div>
          )}
        </div>

        {/* Document menu — rendered as a sibling of the top strip above, NOT
            nested inside the pill's own small wrapper, so its scrim
            (`MenuPanel`'s `position:absolute; inset:0`) covers this whole
            relatively-positioned root rather than just the pill's own box. */}
        <DocumentMenu
          open={documentMenuOpen}
          docName={docName}
          orientation={orientation}
          onClose={() => setDocumentMenuOpen(false)}
          onOpen={openDocument}
          onOpenScanner={openScanner}
          onOpenRecents={openRecentsSheet}
          onSaveCopy={saveCopy}
          onUseFullEditor={useFullEditor}
          // Task 2 dropped the dock/rail AR button — this row is now its
          // only home (module doc). `showViewInAr` bundles BOTH gates the
          // old button had (iOS Safari, AND a document actually open) into
          // one flag, mirroring `docName !== null`'s own row-gating
          // convention just above this component's Save-a-copy row.
          showViewInAr={hasDocument && isArQuickLookCandidate()}
          arBusy={arBusy}
          onViewInAr={viewInArFromDocumentMenu}
        />

        {/* Settings menu — same sibling-of-top-strip rationale as
            `DocumentMenu` above, just anchored to the OTHER top corner. */}
        <SettingsMenu
          open={settingsMenuOpen}
          orientation={orientation}
          onClose={() => setSettingsMenuOpen(false)}
          onOpenUnitPicker={openUnitPicker}
        />

        {/* Unit picker (design §7) — the same shared modal the Parts
            sheet's own header unit chip opens (PartsSheet's
            `onOpenUnitPicker` below). */}
        <UnitPicker
          open={unitPickerOpen}
          orientation={orientation}
          onClose={() => setUnitPickerOpen(false)}
        />

        {/* Views sheet (design §7-style modal, playtest finding 12) — the
            NEW "Views" dock button's own destination, both orientations
            (the render below). */}
        <ViewsSheet
          open={viewsSheetOpen}
          orientation={orientation}
          onClose={() => setViewsSheetOpen(false)}
          onSelectView={handleSelectStandardView}
          projection={projection}
          onToggleProjection={toggleViewProjection}
          scenes={sceneEntries}
          activeSid={activeSceneSid}
          drifted={activeSceneDrifted}
          onSelectScene={activateScene}
        />

        {/* Active-Scene pill (SPEC.md §2; placement per playtest round 1):
            portrait — bottom-center, 12px above the workbench dock's tool
            row (which itself rides on the Parts sheet, so it lifts with the
            sheet's live height); landscape — top-center on the top strip's
            own row, between the document pill and the ⋯ menu. Hidden
            entirely when no Scene is active (`ScenePill`'s `entry === null`
            gate). */}
        <ScenePill
          entry={activeSceneEntry}
          drifted={activeSceneDrifted}
          onPrevious={() => stepScene(-1)}
          onNext={() => stepScene(1)}
          onOpenName={openViewsSheet}
          placement={
            isLandscape
              ? { kind: 'top', topCss: TOP_STRIP_OFFSET_CSS }
              : { kind: 'bottom', bottomPx: sheetHeightPx + DOCK_ROW_HEIGHT_PX }
          }
        />

        {/* Isolate banner (design §4) — see its doc comment above
            (`isolatedLabel`) for why this exists in a wave that otherwise
            doesn't touch isolate-state visuals. Landscape (design §5: "keep
            clamped inside the safe area between rail and sheet") wraps the
            SAME banner content in a `left`/`right`-bounded flex row that
            centers it in whatever room is free between the left edge and
            the right rail — the old left bound tracked the since-removed
            side sheet's live width (item 8); with that sheet gone the left
            bound is simply the viewport edge. Shifted down an extra pill's
            worth (`isolateBannerTopCss`) whenever the Scene pill above is
            ALSO showing — SPEC.md's own screenshots never depict both HUD
            elements up at once, so this is this file's own call rather than
            a spec value. */}
        {hasDocument && isolatedNode !== null && (
          isLandscape ? (
            <div
              style={{
                position: 'absolute', top: isolateBannerTopCss,
                left: 0, right: LANDSCAPE_RAIL_CLEARANCE_CSS,
                zIndex: 35, display: 'flex', justifyContent: 'center', pointerEvents: 'none',
              }}
            >
              <div className="shop-isolate-in" style={isolateBannerContentStyle}>
                <IsolateBannerBody label={isolatedLabel} onShowAll={showAll} />
              </div>
            </div>
          ) : (
            <div style={{ ...isolateBannerPositionStyle, top: isolateBannerTopCss }}>
              <div className="shop-isolate-in" style={isolateBannerContentStyle}>
                <IsolateBannerBody label={isolatedLabel} onShowAll={showAll} />
              </div>
            </div>
          )
        )}

        {/* Portrait: workbench dock (design §1) — one charcoal object
            fusing the toolbar row (Views / Select-Orbit-Tape / Zoom Extents)
            with the Parts sheet immediately below it (PartsSheet.tsx keeps
            owning the sheet's own surface/detent/drag state — see its
            module doc for the restructure this pairs with). This whole
            wrapper anchors to `bottom:0` with NO fixed height of its own —
            its height is just the sum of the fixed-height toolbar row plus
            whatever PartsSheet's OWN height currently is, so as the sheet
            grows toward "full" the dock rides up with it and is NEVER
            covered (this is what replaces the old hudBottomPx lift hack —
            there is no separate floating cluster left for the sheet to
            grow over). */}
        {!isLandscape && hasDocument && scene !== null && (
          <div
            style={{
              position: 'absolute', left: 0, right: 0, bottom: 0, zIndex: 25,
              display: 'flex', flexDirection: 'column',
              boxShadow: '0 -10px 34px -12px rgba(27,26,23,.55)',
            }}
          >
            <div
              style={{
                flex: '0 0 auto', display: 'flex', justifyContent: 'space-between', alignItems: 'center',
                padding: '10px 12px 8px', background: 'var(--shop-dock)', borderRadius: '18px 18px 0 0',
              }}
            >
              {/* "Views" (playtest finding 12), immediately LEFT of the tool
                  group — AR used to sit on the opposite side (now a
                  DocumentMenu row instead — see that component's module
                  doc), so the tool group no longer needs a same-width
                  invisible spacer to stay centered: a single icon button on
                  each side of `justify-content: space-between` is already
                  symmetric on its own. */}
              <button type="button" aria-label="Views" className="shop-press" onClick={openViewsSheet} style={dockIconButtonStyle}>
                <ViewCubeIcon size={21} />
              </button>

              <div style={{ display: 'flex', gap: '2px', background: 'color-mix(in srgb, var(--shop-dock-text-strong) 6%, transparent)', borderRadius: '13px', padding: '3px' }}>
                {TOOL_SEGMENTS.map(({ name, label, ariaLabel, Icon }) => {
                  const active = activeTool === name
                  return (
                    <button
                      key={name}
                      type="button"
                      aria-label={ariaLabel ?? label}
                      className="shop-press"
                      onClick={() => activateTool(name)}
                      style={segmentStyle(active)}
                    >
                      <Icon size={18} />
                      <span aria-hidden="true">{label}</span>
                    </button>
                  )
                })}
              </div>

              {/* `zoomToNodeOrExtents(null)`, not a bare `zoomExtents()` call
                  — same helper the Parts-sheet row tap/double-tap use, so
                  this button's own camera jump dismisses a stale inspect
                  card too (that helper's own doc comment). Immediately RIGHT
                  of the tool group, mirroring "Views" on the left. */}
              <button type="button" aria-label="Zoom Extents" className="shop-press" onClick={() => zoomToNodeOrExtents(null)} style={dockIconButtonStyle}>
                <ZoomExtentsIcon size={21} />
              </button>
            </div>

            <PartsSheet
              scene={scene}
              orientation={orientation}
              hiddenKeys={hiddenKeys}
              hiddenTagPaths={hiddenTagPaths}
              isolatedNode={isolatedNode}
              selectedNode={selectedIds[0] ?? null}
              containerRef={sheetAreaRef}
              detent={detent}
              onDetentChange={setDetent}
              onToggleNode={toggleHiddenNode}
              onToggleTagPath={toggleHiddenTagPath}
              onSelectRow={handleSheetSelectRow}
              onLongPressRow={isolateNode}
              onOpenUnitPicker={openUnitPicker}
              onHeightChange={setSheetHeightPx}
            />
          </div>
        )}

        {/* Landscape: right rail (design §5) — Views / Select-Orbit-Tape
            (icon only, unlike the portrait dock's icon+label segments — no
            room for a label at 52px wide) / Zoom Extents, vertically
            centered. A SEPARATE object from the bottom sheet below
            (unchanged composition — design §5's "Right rail" and "Side
            sheet" bullets were always two independent floating objects,
            never fused the way the portrait dock/sheet are; only the SECOND
            object changed shape — item 8's side-sheet removal). Translucent
            in landscape (playtest item 2: "the tool dock also gets the same
            treatment in LANDSCAPE" — the portrait dock above stays fully
            opaque). */}
        {isLandscape && hasDocument && scene !== null && (
          <div
            style={{
              position: 'absolute', right: LANDSCAPE_RIGHT_OFFSET_CSS, top: '50%', transform: 'translateY(-50%)',
              zIndex: 25, display: 'flex', flexDirection: 'column', alignItems: 'center', gap: '2px',
              background: 'color-mix(in srgb, var(--shop-dock) 88%, transparent)',
              border: '1px solid var(--shop-dock-border, transparent)',
              borderRadius: 'var(--radius-hud, 15px)', padding: '4px',
              boxShadow: '0 6px 18px -8px rgba(27,26,23,.6)',
            }}
          >
            {/* "Views" (playtest finding 12), ABOVE the tool group — the
                rail is a single vertical column, so "left/right of the tool
                group" (the portrait dock's own layout above) maps to
                "above/below" here. */}
            <button type="button" aria-label="Views" className="shop-press" onClick={openViewsSheet} style={railIconButtonStyle}>
              <ViewCubeIcon size={20} />
            </button>
            <div style={railDividerStyle} />
            {TOOL_SEGMENTS.map(({ name, ariaLabel, label, Icon }) => {
              const active = activeTool === name
              return (
                <button
                  key={name}
                  type="button"
                  aria-label={ariaLabel ?? label}
                  className="shop-press"
                  onClick={() => activateTool(name)}
                  style={railSegmentStyle(active)}
                >
                  <Icon size={20} />
                </button>
              )
            })}
            <div style={railDividerStyle} />
            {/* `zoomToNodeOrExtents(null)` — see the portrait dock's own
                identical button above for why. BELOW the tool group,
                mirroring "Views" above it. */}
            <button type="button" aria-label="Zoom Extents" className="shop-press" onClick={() => zoomToNodeOrExtents(null)} style={railIconButtonStyle}>
              <ZoomExtentsIcon size={20} />
            </button>
          </div>
        )}

        {/* Landscape: bottom sheet (design correction — item 8: the old
            left-edge side sheet is gone entirely; landscape now uses the
            SAME `PartsSheet` component as portrait, self-positioned,
            width-capped and centered over the bottom edge — see
            PartsSheet.tsx's own module doc for its root-style split).
            Dockless: unlike portrait, there is no toolbar row fused above
            it (the rail above already carries every tool). */}
        {isLandscape && hasDocument && scene !== null && (
          <PartsSheet
            scene={scene}
            orientation={orientation}
            hiddenKeys={hiddenKeys}
            hiddenTagPaths={hiddenTagPaths}
            isolatedNode={isolatedNode}
            selectedNode={selectedIds[0] ?? null}
            containerRef={sheetAreaRef}
            detent={detent}
            onDetentChange={setDetent}
            onToggleNode={toggleHiddenNode}
            onToggleTagPath={toggleHiddenTagPath}
            onSelectRow={handleSheetSelectRow}
            onLongPressRow={isolateNode}
            onOpenUnitPicker={openUnitPicker}
            onHeightChange={setSheetHeightPx}
          />
        )}

        {/* Toast (design §9): single slot. Portrait: bottom-center above
            the dock when one's showing (tracks PartsSheet's live height via
            `sheetHeightPx` — see DOCK_ROW_HEIGHT_PX's doc comment), else
            just above the safe area. Landscape has no dock to clear (its
            sheet is dockless — module doc above) but DOES need to clear the
            centered bottom sheet itself once it's showing, so it tracks the
            SAME `sheetHeightPx` state portrait uses, horizontally clamped
            clear of the right rail (design §5). */}
        {toast !== null && (
          isLandscape ? (
            <div
              style={{
                position: 'absolute',
                bottom: hasDocument ? `${sheetHeightPx + 12}px` : 'max(20px, calc(env(safe-area-inset-bottom) + 12px))',
                left: 0, right: LANDSCAPE_RAIL_CLEARANCE_CSS,
                zIndex: 45, display: 'flex', justifyContent: 'center', pointerEvents: 'none',
              }}
            >
              <div className={toast.leaving ? 'shop-toast-out' : 'shop-toast-in'} style={toastContentStyle}>
                {toast.message}
              </div>
            </div>
          ) : (
            <div
              className={toast.leaving ? 'shop-toast-out' : 'shop-toast-in'}
              style={{
                ...toastContentStyle,
                position: 'absolute', left: '50%', transform: 'translate(-50%, 0)',
                bottom: hasDocument ? `${sheetHeightPx + DOCK_ROW_HEIGHT_PX}px` : 'max(24px, calc(env(safe-area-inset-bottom) + 16px))',
                zIndex: 45,
              }}
            >
              {toast.message}
            </div>
          )
        )}

        {kernelError !== null && (
          <div style={{ position: 'absolute', inset: 0, display: 'flex', alignItems: 'center', justifyContent: 'center', color: '#fff', background: '#8b0000', zIndex: 100, padding: '24px', textAlign: 'center' }}>
            {kernelError}
          </div>
        )}
      </div>
    </div>
  )
}

/** `max(12px, env(safe-area-inset-top) + 8px)` (design §1) — shared by the
 *  top strip itself, and (exported) by `MenuPanel.tsx` to anchor both
 *  `DocumentMenu`'s and `SettingsMenu`'s panels just below it. A plain
 *  module-level string (not a token): it mixes a CSS environment function
 *  with arithmetic in a way that doesn't fit this app's custom-property
 *  token model. */
export const TOP_STRIP_OFFSET_CSS = 'max(12px, calc(env(safe-area-inset-top) + 8px))'

/** Mildly translucent (playtest item 2: ~0.85-0.9 alpha of the surface
 *  color, NO `backdrop-filter` — the WebGL compositing constraint stands)
 *  — `color-mix` against the (now theme-scoped) `--shop-dock` token rather
 *  than a hardcoded rgba, so this stays correct in both themes without a
 *  second literal to keep in sync. Now a real `<button>` (opens
 *  `DocumentMenu`) rather than inert chrome — `minHeight` keeps its hit
 *  target ≥44px (the "…"/settings button's own 44×44 square) even though
 *  its content alone (17px mark + 14px text) is shorter than that. */
const topStripPillStyle: React.CSSProperties = {
  display: 'flex', alignItems: 'center', gap: '8px',
  background: 'color-mix(in srgb, var(--shop-dock) 88%, transparent)',
  border: '1px solid var(--shop-dock-border, transparent)',
  borderRadius: 'var(--radius-hud, 15px)', padding: '11px 15px', minHeight: '44px',
  boxShadow: '0 6px 18px -8px rgba(27,26,23,.6)',
  fontFamily: 'var(--font-family-ui)', fontSize: '14px', fontWeight: 600, lineHeight: 1,
  color: 'var(--shop-dock-text-strong)', pointerEvents: 'auto', maxWidth: 'calc(100% - 60px)',
  cursor: 'pointer',
}

const ellipsisButtonStyle: React.CSSProperties = {
  width: '44px', height: '44px', borderRadius: 'var(--radius-hud, 15px)', border: '1px solid var(--shop-dock-border, transparent)', cursor: 'pointer',
  background: 'color-mix(in srgb, var(--shop-dock) 88%, transparent)', boxShadow: '0 6px 18px -8px rgba(27,26,23,.6)',
  display: 'flex', alignItems: 'center', justifyContent: 'center', color: 'var(--shop-dock-text)',
}

/** Isolate banner content, orientation-independent — factored out of the
 *  two positioning wrappers (`isolateBannerContentStyle`'s own doc comment
 *  below) so they share one body instead of duplicating its markup. */
function IsolateBannerBody({ label, onShowAll }: { label: string | null; onShowAll: () => void }) {
  return (
    <>
      <svg aria-hidden="true" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="var(--shop-accent-text)" strokeWidth={2}>
        <circle cx="12" cy="12" r="3" />
        <path d="M2 12s4-7 10-7 10 7 10 7-4 7-10 7-10-7-10-7z" />
      </svg>
      <span style={{ fontFamily: 'var(--font-family-ui)', fontSize: '13px', fontWeight: 500, color: 'var(--shop-dock-text-strong)', whiteSpace: 'nowrap' }}>
        Isolated{label !== null ? ` · ${label}` : ''}
      </span>
      <button type="button" className="shop-press" onClick={onShowAll} style={isolateShowAllStyle}>
        Show all
      </button>
    </>
  )
}

/** Shared visual chrome for the isolate banner's inner pill — deliberately
 *  a SEPARATE element from either orientation's positioning wrapper
 *  (`isolateBannerPositionStyle` (portrait) / the inline wrapper style in
 *  `ShopApp`'s landscape render) rather than merged into one via a spread,
 *  so the `.shop-isolate-in` entrance class (index.css: 200ms drop-in,
 *  Motion summary's "banner drops in 200ms") can animate THIS element's own
 *  `transform`/`opacity` without fighting the wrapper's `left`/`transform`
 *  centering — portrait's wrapper alone carries `transform: translateX(
 *  -50%)`; if the animation's `transform` keyframes landed on that same
 *  element they'd overwrite the centering for the animation's duration
 *  instead of composing with it. */
const isolateBannerContentStyle: React.CSSProperties = {
  display: 'flex', alignItems: 'center', gap: '6px', pointerEvents: 'auto',
  background: 'var(--shop-dock)', border: '1px solid var(--shop-dock-border, transparent)',
  borderRadius: 'var(--radius-hud, 15px)', padding: '5px 6px 5px 16px',
  boxShadow: '0 10px 26px -10px rgba(27,26,23,.6)', whiteSpace: 'nowrap',
}

/** Portrait's isolate banner positioning wrapper — landscape supplies its
 *  own inline equivalent instead (its `left`/`right` track the side
 *  sheet's live width, which this fixed `left: 50%` centering can't
 *  express). */
const isolateBannerPositionStyle: React.CSSProperties = {
  position: 'absolute', top: `calc(${TOP_STRIP_OFFSET_CSS} + 54px)`, left: '50%', transform: 'translateX(-50%)',
  zIndex: 35,
}

const isolateShowAllStyle: React.CSSProperties = {
  fontFamily: 'var(--font-family-ui)', fontSize: '13px', fontWeight: 600, whiteSpace: 'nowrap',
  // --shop-accent-fill (not the plain --shop-accent mark/wash color —
  // tokens.css's own doc comment): cream text sits directly on this fill,
  // which needs the darker, ≥4.5:1 terracotta.
  background: 'var(--shop-accent-fill)', color: 'var(--shop-on-accent)', border: 'none',
  borderRadius: '11px', padding: '9px 14px', cursor: 'pointer',
}

/** Toast bubble chrome, orientation-independent (design §9) — the caller
 *  (`ShopApp`'s render) supplies `position`/`bottom`/`left`/`transform`/
 *  `zIndex` around this, differently per orientation. */
const toastContentStyle: React.CSSProperties = {
  maxWidth: 'calc(100% - 32px)', padding: '12px 18px',
  background: 'var(--shop-dock)', color: 'var(--shop-dock-text-strong)',
  border: '1px solid var(--shop-dock-border, transparent)',
  borderRadius: '13px', boxShadow: '0 12px 30px -10px rgba(27,26,23,.6)',
  fontFamily: 'var(--font-family-ui)', fontSize: '13px', fontWeight: 500,
  whiteSpace: 'nowrap', pointerEvents: 'none',
}

const dockIconButtonStyle: React.CSSProperties = {
  width: '52px', height: '52px', borderRadius: '13px', border: 'none', cursor: 'pointer',
  background: 'color-mix(in srgb, var(--shop-dock-text-strong) 7%, transparent)', color: 'var(--shop-dock-text)',
  display: 'flex', alignItems: 'center', justifyContent: 'center', flexShrink: 0,
}

/** The segmented tool group's per-segment style (design §1 "center
 *  segmented tool group" — active = terracotta fill; Motion summary's
 *  "Chip state": background/color 120ms linear). */
function segmentStyle(active: boolean): React.CSSProperties {
  return {
    minWidth: '70px', height: '50px', borderRadius: '10px', border: 'none', cursor: 'pointer',
    // --shop-accent-fill, not --shop-accent (tokens.css's own doc comment)
    // — this segment carries a cream TEXT label (9.5px) on its active
    // fill, unlike the icon-only landscape rail's own `railSegmentStyle`
    // below (non-text, so plain --shop-accent's ~3.6:1 is fine there).
    background: active ? 'var(--shop-accent-fill)' : 'transparent',
    color: active ? 'var(--shop-on-accent)' : 'var(--shop-dock-text)',
    display: 'flex', flexDirection: 'column', alignItems: 'center', justifyContent: 'center', gap: '2px',
    fontFamily: 'var(--font-family-ui)',
    fontSize: 'var(--font-size-dock-chip, 9.5px)',
    fontWeight: active ? 600 : 500,
    lineHeight: 1,
    transition: 'background 120ms linear, color 120ms linear',
  }
}

/** Landscape right rail's zoom-extents/AR buttons (design §5: "zoom-extents
 *  52×48 ... AR 52×48") — a shorter rectangle than the portrait dock's
 *  52×52 `dockIconButtonStyle` (the rail is a narrow vertical column, not a
 *  wide horizontal row, so it borrows that height back for the tool
 *  segments below instead). */
const railIconButtonStyle: React.CSSProperties = {
  width: '52px', height: '48px', borderRadius: '11px', border: 'none', cursor: 'pointer',
  background: 'color-mix(in srgb, var(--shop-dock-text-strong) 7%, transparent)', color: 'var(--shop-dock-text)',
  display: 'flex', alignItems: 'center', justifyContent: 'center', flexShrink: 0,
}

/** Landscape right rail's Select/Orbit/Tape segments (design §5: "52×52,
 *  active terracotta") — icon-only, unlike the portrait dock's icon+label
 *  `segmentStyle` (a 52px-wide column has no room for a 9.5px label under
 *  the icon the way the portrait dock's 70px-wide ones do). */
function railSegmentStyle(active: boolean): React.CSSProperties {
  return {
    width: '52px', height: '52px', borderRadius: '11px', border: 'none', cursor: 'pointer',
    background: active ? 'var(--shop-accent)' : 'transparent',
    color: active ? 'var(--shop-on-accent)' : 'var(--shop-dock-text)',
    display: 'flex', alignItems: 'center', justifyContent: 'center', flexShrink: 0,
    transition: 'background 120ms linear, color 120ms linear',
  }
}

/** The rail's two hairline dividers (design §5: "36px hairline divider"),
 *  horizontal (the rail stacks vertically, unlike the portrait dock's own
 *  vertical dividers between horizontally-laid-out groups — this file has
 *  no portrait equivalent to share it with). */
const railDividerStyle: React.CSSProperties = {
  width: '36px', height: '1px', background: 'color-mix(in srgb, var(--shop-dock-text-strong) 12%, transparent)', margin: '3px 0', flexShrink: 0,
}

const emptyPrimaryButtonStyle: React.CSSProperties = {
  height: '56px', borderRadius: 'var(--radius-hud, 15px)', border: 'none', cursor: 'pointer',
  // --shop-accent-fill, not --shop-accent (tokens.css's own doc comment) —
  // "Open a model…"'s own 15px label is cream TEXT on this fill, below the
  // large-text threshold that would tolerate plain --shop-accent's ~3.6:1.
  background: 'var(--shop-accent-fill)', color: 'var(--shop-on-accent)',
  display: 'flex', alignItems: 'center', justifyContent: 'center', gap: '10px',
  fontFamily: 'var(--font-family-ui)', fontSize: '15px', fontWeight: 600,
}

const emptyGhostButtonStyle: React.CSSProperties = {
  height: '56px', borderRadius: 'var(--radius-hud, 15px)', border: 'none', cursor: 'pointer',
  // color-mix against --shop-text (not a flat rgba): the empty state's
  // wash needs to LIGHTEN in dark mode and DARKEN in light mode, which a
  // single hardcoded rgba(cream, alpha) can't do — mixing the already
  // theme-scoped --shop-text token in at low alpha gets both automatically.
  background: 'color-mix(in srgb, var(--shop-text) 7%, transparent)', color: 'var(--shop-text)',
  display: 'flex', alignItems: 'center', justifyContent: 'center', gap: '10px',
  fontFamily: 'var(--font-family-ui)', fontSize: '15px', fontWeight: 600,
}




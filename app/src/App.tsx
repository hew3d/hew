import { useEffect, useState, useCallback, useRef, useMemo } from 'react'
import { useClearSelectionOnDrawTool } from './useClearSelectionOnDrawTool'
import { i18n } from '@lingui/core'
import { ph, plural, t } from '@lingui/core/macro'
import { Trans, useLingui } from '@lingui/react/macro'
import { historyLabel } from './i18n/historyLabels'
import { useKeymap, useKeymapDispatch } from './keymap/useKeymap'
import { currentKeymap, defaultKeymapBindings, shortcutLabel, subscribeKeymap } from './keymap/keymap'
import { startNativeMenuSync } from './keymap/nativeMenu'
import { flushSync } from 'react-dom'
import { loadKernel, type Scene } from './wasm/loader'
import Viewport, {
  type ViewportApi,
  type InferenceInfo,
  type RescaleConfirmInfo,
  type FlyState,
} from './viewport/Viewport'
import { toggledRenderMode, type RenderMode } from './viewport/SceneRenderer'
import { InferenceTooltip } from './viewport/InferenceTooltip'
import { AnnotationEditor } from './viewport/AnnotationEditor'
import { SnapDot } from './viewport/SnapDot'
import { MeasurementBox } from './viewport/MeasurementBox'
import { ViewDie } from './viewport/ViewDie'
import { GeoReferenceInfo } from './panels/GeoReferenceInfo'
import { DocumentTree } from './panels/DocumentTree'
import { MaterialPalette } from './panels/MaterialPalette'
import { MenuBar } from './panels/MenuBar'
import { CAMERA_HANDOFF_TOOL_NAMES } from './panels/cameraHandoffTools'
import { TitleBar } from './TitleBar'
import { isCoarsePointer, isLinux, isMac, isWindows } from './platform'
import { forwardToTextField, isTextFieldFocused, pasteTextIntoFocusedField } from './clipboard/textFieldPaste'
import { nextPaint } from './paint'
import { beginOpenAttempt, clearOpenAttempt, forgetSettledFailure, openAttemptFailed } from './log/failedOpen'
import { getPanicCapture, recordKernelStop } from './log/panicCapture'
import { kernelMemoryBytes } from './wasm/kernelMemory'
import { formatBytes } from './log/reportBundle'

/** How long one step of a `.skp` open works before the overlay repaints:
 * long enough that the paint between steps costs little, short enough
 * that the bar moves several times a second. */
const OPEN_STEP_BUDGET_MS = 160

/** What `Scene.open_skp_begin`/`open_skp_step` hand back. */
interface OpenStepProgress {
  phase: 'shapes' | 'objects' | 'finishing' | 'source' | 'done'
  done: number
  total: number
  shapes: number
  report?: unknown
}
import { TagsPanel } from './panels/TagsPanel'
import { ComponentsPanel, PurgeUnusedButton } from './panels/ComponentsPanel'
import { ConfirmDialog } from './panels/ConfirmDialog'
import { BusyNotice } from './panels/BusyNotice'
import { ScenesPanel, ScenesAddButton, useSceneRenameState } from './panels/ScenesPanel'
import { ObjectInfoPanel } from './panels/ObjectInfoPanel'
import { TraySection } from './panels/TraySection'
import { ChangesPanel } from './panels/ChangesPanel'
import { ModelHealthPanel, selectionObjectIds } from './panels/ModelHealthPanel'
import { FloatingWindow } from './panels/FloatingWindow'
import { MakeSolidWizard } from './panels/MakeSolidWizard'
import { MakeSolidSketches, selectedSketchIds } from './panels/MakeSolidSketches'
import { BADGE_TOOLTIP_LIST, MENU, badgeText, importLogSplit, type WizardScope } from './health/healthCopy'
import { IsolationBar } from './viewport/IsolationBar'
import type { IsolationTarget } from './viewport/isolation'
import { EMPTY_SUMMARY, objectHealthStates, readHealthSummary, summarize, type HealthOverlay, type HealthScope } from './health/healthReport'
import { UnsavedChangesDialog, type UnsavedChangesDecision } from './panels/UnsavedChangesDialog'
import { parseHistoryEntries } from './panels/changesModel'
import { ToolRail } from './panels/ToolRail'
import { TrayHideRow, TrayShowTab } from './panels/TrayToggle'
import { ContextualDock } from './panels/ContextualDock'
import { nextSelection, mergeSelection, type SelectMode, canBoolean as canBooleanHelper, canBooleanInComponent, canMakeComponent, canPlaceInstance, canExplodeInstance, canMakeUnique, canGroup as canGroupHelper, canUngroup as canUngroupHelper, nodeEq, nodeKey, nodeKindToNumber, nodeRefFromJs, resolveLabel, entityLabel, buildTreeIndexMap, pruneDeadSelection, structuralSelection, isSketchScopedKind, makeLabelFor, stripTagSuffix, type NodeRef } from './panels/treeModel'
import { SelectionPathStrip } from './panels/SelectionPathStrip'
import { buildSelectionPath, frameClickAction, hoverLine, type HitLeaf } from './panels/selectionPath'
import { tagPathKey } from './panels/tagModel'
import { LogPanel } from './log/LogPanel'
import * as LogStore from './log/LogStore'
import { installTestHarness, type PrintRecorder } from './test/harness'
import { installLiveBridge, type LiveBridgeDeps, type LiveOpOutcome, type LiveOpRunner } from './api/liveBridge'
import { uiContextJson, uiIsolationJson, uiSelectionJson } from './api/viewMirror'
import { pumpEvents } from './api/eventPump'
import { install as installConsoleCapture, restore as restoreConsoleCapture } from './log/consoleCapture'
import { MATERIAL_SENTINEL } from './tools/PaintTool'
import { EDIT_CONTEXT_HINT_CODE } from './tools/faceDraw'
import { outlinerDrawing, resolveDraft, setKernelHidden, type OutlinerDrawing, type PlaneUnitsScene } from './tools/planeUnits'
import { makeFileHost, isTauri, FileReadError, type DocFormat, type ExportFileType, type FileHost, type FileRef, type ImportReport, type ImportPick, type OpenPick } from './io/fileHost'
import {
  panelIdFor,
  exportPanelIdFor,
  savePanelFormats,
  saveCompatFrom,
  exportChoiceFrom,
  exportPanelFormats,
  hasNativeSavePanel,
  pickSaveTarget,
  isSkpExportFormat,
  saveFormatFrom,
  type ExportFormat as PanelExportFormat,
  type SkpExportFormat,
} from './io/savePanel'
import { isSkpFormat, nameForFormat, normalizeDocFormat, saveFormatForPath, skpTarget, takesCompat } from './io/docFormat'
import {
  INITIAL_SESSION,
  deriveTitle,
  documentName,
  documentBaseName,
  saveStateLabel,
  afterMutation,
  markNonUndoableDirty,
  NON_UNDOABLE_REASON,
  afterSave, applyWriteThroughSave,
  afterOpen,
  canRevertToSaved,
  afterImport,
  afterRecovery,
  type DocSessionState,
} from './io/documentSession'
import { nextRelativeTimeBoundary } from './io/relativeTime'
import { makeRecoveryStore, shouldPromptRecovery, type RecoveryListing, type RecoverySnapshot, type RecoveryMeta } from './io/recoveryStore'
import { loadHewBytes, isPanicError, isSceneEmpty, seedHiddenKeysFromRegistry, seedHiddenTagPathsFromRegistry, unionHiddenLeafIds } from './io/documentLoad'
import { writeShellModeOverride } from './shop/shellMode'
import { listRecents, recordRecent, type RecentEntry } from './io/recents'
import { ImportReportDialog } from './panels/ImportReportDialog'
import { SaveFormatDialog, type SaveFormatAnswer } from './panels/SaveFormatDialog'
import { SkpLossNotice, type SkpLossNoticeData } from './panels/SkpLossNotice'
import { BusyOverlay, ImportingOverlay, type OpenProgress, ensureSpinnerKeyframes } from './panels/ImportingOverlay'
import { RecoveryDialog } from './panels/RecoveryDialog'
import { WelcomeScreen, type SampleEntry } from './panels/WelcomeScreen'
import { getShowWelcome, setShowWelcome } from './settings/welcomeScreen'
import { getLengthUnit, setLengthUnit, homeFramingScale, subscribe as subscribeLengthUnit, type LengthFormat } from './settings/units'
import { StlExportDialog } from './panels/StlExportDialog'
import { StlUnitsDialog } from './panels/StlUnitsDialog'
import { RescaleConfirmDialog } from './panels/RescaleConfirmDialog'
import { setLastStlImportUnit } from './settings/stlImportUnit'
import { ExportDialog, type ExportFormat } from './panels/ExportDialog'
import { PhoneShareDialog, type PhoneShareDocument } from './panels/PhoneShareDialog'
import { PrintDialog } from './panels/PrintDialog'
import { lineArtSvgDocument, projectAnnotationOverlay, requestLineDrawing } from './print/lineArt'
import { printViewDirection } from './print/printJob'
import { scaleRatio } from './print/scale'
import type { SvgExportOptions } from './panels/ExportDialog'
import { makePrintHost } from './print/printHost'
import { TextDialog, type TextDialogResult } from './panels/TextDialog'
import { layoutGlyphRun } from './text/glyphRun'
import { collectNonSolidObjects } from './io/solidGating'
import { friendlyErrorText, isErrorLevelCode, kernelErrorMessage, parseKernelErrorCode } from './kernelErrors'
import { CommandPalette } from './palette/CommandPalette'
import { toolHint, toolActionId, type PaletteEntry } from './palette/registry'
import type { TagReveal } from './panels/TagsPanel'
import { LibraryDialog } from './panels/LibraryDialog'
import { SaveToLibraryPopover } from './panels/SaveToLibraryPopover'
import { libraryStore } from './io/libraryStore'
import { buildItemGhost } from './library/ghostBuilder'
import { readItemSummary, renderItemThumbnail, sha256Hex } from './library/itemFiles'
import { loadThumbnails as loadSceneThumbnails, saveThumbnails as saveSceneThumbnails } from './scenes/sceneThumbnails'
import { buildLibraryItem } from './library/libraryModel'
import { itemFileName } from './library/fileNaming'
import type { LibraryCategory, LibraryItem } from './library/types'
import type { LibraryPlacement } from './tools/LibraryPlaceTool'
import { parseInsertResult } from './tools/LibraryPlaceTool'
import { readImageInfo, pictureName, isPictureFileName } from './tools/imageInfo'
import {
  getClipboard,
  hasClipboardContent,
  setClipboardBytes,
  refreshClipboardFromShell,
} from './clipboard/modelClipboard'
import { SettingsWindow } from './settings/SettingsWindow'
import { FluentSettingsPage } from './settings/FluentSettingsPage'
import { getDebugMode, subscribe as subscribeDebugMode } from './settings/debugMode'
import { getTrayLayout, setTrayLayout, subscribe as subscribeTrayLayout } from './settings/trayLayout'
import { getMeasurementsPosition, subscribeMeasurementsPosition } from './settings/view'
import { useViewSetting } from './settings/useViewSetting'
import { getSceneTransitions, setSceneTransitions, subscribe as subscribeSceneTransitions } from './settings/sceneTransitions'
import { useScenesController } from './scenes/useScenesController'
import { parseCameraJson, parseSectionJson, parseViewStyleJson, type ViewStyleJson } from './scenes/scenesModel'
import type { SceneSource } from './print/printJob'
import * as diagnosticLog from './log/diagnosticLog'
import * as inputRecorder from './recording/inputRecorder'
import { ReportBugDialog } from './panels/ReportBugDialog'
import { SentReportsDialog } from './panels/SentReportsDialog'
import { FollowNoticeBar, useFollowedReports } from './panels/FollowNoticeBar'
import { getFollowHost, totalUnread } from './io/followHost'
import { startWebLaunchCheck } from './io/webFollowHost'
import { TOOLS, isToolAction, toolLabel, type ToolName } from './tools/toolRegistry'

/** Autosave tick interval (ms). Exported as a test seam (App.autosave.test.tsx
 *  drives fake timers by this exact value rather than hardcoding it). */
export const AUTOSAVE_INTERVAL_MS = 12000
/** Right-tray width: default/bounds (px) + localStorage persistence key.
 *  Default is sized so typical nested tag/outliner labels fit untruncated. */
const TRAY_WIDTH_DEFAULT = 304
const TRAY_WIDTH_MIN = 220
const TRAY_WIDTH_MAX = 560
const TRAY_WIDTH_KEY = 'hew.trayWidth'
const RAIL_NARROW_KEY = 'hew.railNarrow'
const TRAY_COLLAPSED_KEY = 'hew.trayCollapsed'
const DOCK_HIDDEN_KEY = 'hew.dockHidden'
const clampTrayWidth = (w: number): number =>
  Math.min(TRAY_WIDTH_MAX, Math.max(TRAY_WIDTH_MIN, Math.round(w)))
/** Blur the focused element when it sits inside `el`, reporting whether it
 *  did. Called before a tray section or the whole tray unmounts from a menu
 *  item or shortcut: the tray's name and dimension fields commit on blur, and
 *  a macOS native-menu accelerator never moves DOM focus, so without this
 *  the field would unmount mid-edit and drop what was typed. */
function blurFocusWithin(el: Element | null): boolean {
  const active = document.activeElement
  if (el === null || !(active instanceof HTMLElement) || !el.contains(active)) return false
  active.blur()
  return true
}
/** Help ▸ Hew Help target — the online user guide's index. */
const USER_GUIDE_URL = 'https://hew3d.com/learn/'

interface AppState {
  kernelVersion: string
  scene: Scene
}

interface Toast {
  id: number
  message: string
  code?: string
  /** Error-level (red bubble, error log) vs warning — classified ONCE at
   * creation via kernelErrors' isErrorLevelCode, the single source. */
  isError: boolean
  /** Library save's thumbnail-render toast: 'pending' shows a spinner,
   * 'done' a checkmark. Undefined for every ordinary toast (no icon). */
  state?: 'pending' | 'done'
  /** True while the toast should stay up regardless of the normal 4s
   * auto-dismiss — the thumbnail-render toast's "still working" phase.
   * `updateToast` schedules its own short auto-dismiss once a patch turns
   * this back to false. */
  sticky?: boolean
  /** Guidance rather than a warning (`EDIT_CONTEXT_HINT_CODE`: "double-click
   * it to edit the component"): drawn in the accent tint, larger, and kept
   * up longer, so the way in is not missed at the bottom of the window. */
  emphasis?: boolean
}

let toastCounter = 0

/** Tools that create new geometry — picking one clears a lingering
 * top-level selection (the contextual dock should switch to draw context,
 * not stay pinned to the old object). */
// oxlint-disable-next-line lingui/no-unlocalized-strings -- ToolName identifiers, not copy
const DRAW_TOOLS: ReadonlySet<string> = new Set(['Line', 'Rectangle', 'Circle', 'Polygon', 'Arc'])

/** Tool name → native menu item id, for the macOS menu's radio checks.
 * Exported for the native-menu parity test (`nativeMenuParity.test.ts`):
 * every id here must exist as a real item in the Tauri shell's menu. */
export const TOOL_MENU_IDS: Record<string, string> = {
  Select: 'tool-select',
  Rectangle: 'draw-rectangle',
  Circle: 'draw-circle',
  Polygon: 'draw-polygon',
  Arc: 'draw-arc',
  Line: 'draw-line',
  'Push/Pull': 'tool-pushpull',
  'Follow Me': 'tool-follow-me',
  Offset: 'tool-offset',
  Paint: 'tool-paint',
  'Position Texture': 'tool-position-texture',
  Move: 'tool-move',
  Rotate: 'tool-rotate',
  Scale: 'tool-scale',
  'Tape Measure': 'tool-tape-measure',
  Dimension: 'tool-dimension',
  Protractor: 'tool-protractor',
  Slice: 'tool-slice',
  'Section Plane': 'tool-section-plane',
  'Edit Vertex': 'tool-edit-vertex',
  'Drawing Axes': 'tool-axes',
  Text: 'tool-text',
  Orbit: 'cam-orbit',
  Pan: 'cam-pan',
  Zoom: 'cam-zoom',
  'Zoom Window': 'cam-zoom-window',
  'Position Camera': 'cam-position-camera',
  'Look Around': 'cam-look-around',
  Walk: 'cam-walk',
}

/** What a report calls each kind of file a user opens or imports. */
/* oxlint-disable lingui/no-unlocalized-strings -- bug-report bundle text, English for the maintainer (see failedOpen.ts) */
const OPEN_FILE_LABELS = {
  skp: 'SketchUp model (.skp)',
  gltf: 'glTF model (.glb/.gltf)',
  stl: 'STL model (.stl)',
  dae: 'COLLADA model (.dae)',
  hew: 'Hew document (.hew)',
  recovered: 'Recovered document (.hew)',
} as const
/* oxlint-enable lingui/no-unlocalized-strings */

/** The user's own `.hew` behind a load, for applyLoadedBytes to keep should
 *  it fail (failedOpen.ts). Loads of Hew's own bytes (a blank document, a
 *  bundled sample, an API load) pass none. */
interface LoadedFile {
  name: string
  label: string
  /** The file's path on the desktop, which `document_opened` reports. */
  path?: string
}

/** Kernel memory this close to the 4 GiB a wasm32 kernel can address, when
 *  a trap ends an open, means the open ran out of memory rather than hitting
 *  some other abort. The bar sits well under the ceiling because a browser
 *  may refuse the last growth before it. */
const KERNEL_MEMORY_NEAR_CEILING = 3 * 1024 * 1024 * 1024

/** An open or import of `file` failed with `err`: keep the file for a bug
 *  report (failedOpen.ts) and log the file's size and the kernel's memory
 *  beside the toast's message.
 *
 *  A wasm trap (`WebAssembly.RuntimeError`, not a typed refusal) means the
 *  kernel is gone: its stack unwound without running anything, so the Scene
 *  stays borrowed and every later call throws "recursive use of an object".
 *  Rust's out-of-memory abort runs no panic hook, so nothing on the kernel
 *  side ever names the cause; this is where the page does, from the memory
 *  the kernel had reached, and hands over to the crash screen with that
 *  explanation instead of letting the next render trip over the borrow. */
/* oxlint-disable lingui/no-unlocalized-strings -- the open-failure note, log line, and kernel-stop summary go into the bug-report bundle and diagnostic log, English for the maintainer; only the crash-screen headline/detail are wrapped */
function noteOpenFailure(what: string, err: unknown, file: { name: string; bytes: Uint8Array }): void {
  const fileName = file.name
  const size = formatBytes(file.bytes.byteLength)
  const memory = kernelMemoryBytes()
  const memoryNote = memory !== null ? `; kernel memory ${formatBytes(memory)}` : ''
  // A Rust panic ends in the same trap, but its hook has already captured
  // the real message (and fired the crash screen) by the time the trap
  // reaches this catch: that capture names the cause, not the memory.
  const panic = getPanicCapture()
  const trapped = panic === null && err instanceof WebAssembly.RuntimeError
  const outOfMemory = trapped && memory !== null && memory >= KERNEL_MEMORY_NEAR_CEILING
  openAttemptFailed(
    panic !== null
      ? `Hew hit an internal error opening it (${panic.message}${memoryNote}).`
      : outOfMemory
        ? `Hew ran out of memory opening it${memoryNote}.`
        : trapped
          ? `Hew stopped while opening it (${err.message}${memoryNote}).`
          : friendlyErrorText(err),
  )
  const cause = panic !== null ? '; kernel panic' : outOfMemory ? '; out of memory' : trapped ? '; kernel trap' : ''
  LogStore.log.warn('app', `${what} failed on a ${size} file${memoryNote}${cause}`)
  if (!trapped) return
  if (outOfMemory) {
    recordKernelStop(`Out of memory opening ${file.name} (${size})${memoryNote}`, {
      headline: t`This model is too large for Hew to open`,
      detail: t`Hew ran out of memory opening ${fileName} (${size}). Hew can use up to 4 GB of memory, and this model needs more. A lighter copy may open: in SketchUp, run Purge Unused and remove or simplify the heaviest components. Reload to keep working.`,
    })
  } else {
    recordKernelStop(`Kernel trap opening ${file.name} (${size}): ${err.message}${memoryNote}`, {
      headline: t`Hew stopped while opening the file`,
      detail: t`Hew stopped while opening ${fileName} (${size}) and can't continue. Reload to keep working, and use Report this crash to send the file.`,
    })
  }
}
/* oxlint-enable lingui/no-unlocalized-strings */

/** Extract the filename from an absolute path (cross-platform / or \). */
function basenameOf(path: string): string {
  return path.replace(/[/\\]+/g, '/').split('/').filter(Boolean).pop() ?? path
}

/** True when a window holds nothing worth protecting — no named file, no
 *  unsaved edits, no geometry — so a File ▸ Open or File ▸ New pick may
 *  reuse it in place instead of opening a fresh window. Stricter than
 *  `isSceneEmpty` alone: a saved-then-emptied document (currentRef set, but
 *  the model has since been deleted down to nothing) still has a real file
 *  behind it and must not be silently abandoned by reusing the window. */
export function isPristineDocument(session: DocSessionState, scene: Scene): boolean {
  return session.currentRef === null && !session.dirty && isSceneEmpty(scene)
}


/**
 * Write an export's bytes: to the path the native save panel already
 * picked (`target`, consumed here), else through the host's own export
 * picker (the browser, and the PDF print path).
 */
async function writeExport(
  host: FileHost,
  target: { current: string | null },
  bytes: Uint8Array,
  name: string,
  fileType: ExportFileType,
): Promise<string | null> {
  const path = target.current
  target.current = null
  if (path !== null && host.writeTo !== undefined) {
    await host.writeTo(path, bytes)
    return path
  }
  return host.exportBinary(bytes, name, fileType)
}
/** Calls `scene.mark_saved()` ONLY when the document's undo depth hasn't
 * moved since `undoDepthAtCapture` — every save flow here grabs the bytes
 * to write, then awaits a host write that can take a while (Tauri fs,
 * a browser download, a Library write-through). An edit landing during
 * that await must never be silently marked clean: the written bytes
 * predate it. Always returns the CURRENT `scene.at_saved_mark()` reading —
 * a skipped mark correctly reports the document as still dirty, which
 * `afterSave`/`applyWriteThroughSave`'s `atSavedMark` parameter honors. */
function markSavedIfUnmoved(scene: Scene, undoDepthAtCapture: number): boolean {
  if (scene.undo_depth() === undoDepthAtCapture) scene.mark_saved()
  return scene.at_saved_mark()
}

/** What `Scene.save_skp` / `export_skp` return. */
interface SkpWrite {
  bytes: Uint8Array
  /** What the file could not hold exactly, as short phrases. */
  simplified: string[]
  writer_notes: string[]
}

/** "Undo Push/Pull" / "Redo Move" — the Edit menu's Lane C labels (docs/
 * design/v1.1-cycle.md), derived from the top undo/redo entry each render.
 * Falls back to the bare verb when there is nothing to undo/redo. */
function undoRedoMenuLabels(scene: Scene | null): { undo: string; redo: string } {
  const entries = parseHistoryEntries(scene?.history_entries_json())
  const topUndo = entries.undo[entries.undo.length - 1]
  const topRedo = entries.redo[0]
  const undoWhat = topUndo !== undefined ? historyLabel(topUndo.kind, topUndo.label) : null
  const redoWhat = topRedo !== undefined ? historyLabel(topRedo.kind, topRedo.label) : null
  return {
    undo: undoWhat !== null ? t`Undo ${undoWhat}` : t`Undo`,
    redo: redoWhat !== null ? t`Redo ${redoWhat}` : t`Redo`,
  }
}

/** Whether two id lists name the same set of ids, ignoring order and
 *  duplicates (a node hidden by two sources appears twice in a union). */
function sameIds(a: readonly bigint[], b: readonly bigint[]): boolean {
  const sa = new Set(a)
  const sb = new Set(b)
  return sa.size === sb.size && [...sa].every((id) => sb.has(id))
}

/** Whether two session stacks are the SAME sequence of frames (identity at
 *  every position, ignoring label — a label can legitimately change under a
 *  stack that is otherwise the same frame, e.g. a rename) — `handleSessionChange`'s
 *  pending-rescale invalidation (finding 5, group-session.md) fires whenever
 *  this is false, not just when the innermost frame's identity changes:
 *  undo/redo stays live behind the confirmation dialog, and a change ANYWHERE
 *  in the stack (an outer frame popping, a sibling frame's identity shifting)
 *  means the kernel's own idea of "the innermost frame" when Confirm is
 *  eventually clicked may no longer be the one the dialog named. */
export function sameSessionStackIdentity(
  a: { node: NodeRef }[],
  b: { node: NodeRef }[],
): boolean {
  if (a.length !== b.length) return false
  return a.every((f, i) => nodeEq(f.node, b[i].node))
}


/**
 * Multi-window dedupe (desktop): if some document window already has `path`
 * open, raise and focus it and answer `true` — the caller then skips its own
 * open. Two windows on one file would be confusing at best and, at worst,
 * two writers that don't know about each other. `false` on the web (single
 * window) or an older shell without the command.
 */
async function focusExistingWindowFor(path: string): Promise<boolean> {
  if (!isTauri) return false
  try {
    const { invoke } = await import('@tauri-apps/api/core')
    return await invoke<boolean>('focus_window_with_path', { path })
  } catch {
    return false
  }
}

export default function App() {
  const { t, i18n: activeI18n } = useLingui()
  /** Tray section titles: the heading text AND the key `toggleSection` uses
   *  to find a section's body, so every use shares these. */
  const trayTitle = {
    objectInfo: t`Object Info`,
    outliner: t`Outliner`,
    materials: t`Materials`,
    components: t`Components`,
    tags: t`Tags`,
    scenes: t`Scenes`,
    changes: t`Changes`,
  }
  const [state, setState] = useState<AppState | null>(null)
  const [error, setError] = useState<string | null>(null)
  // oxlint-disable-next-line lingui/no-unlocalized-strings -- ToolName identifier
  const [toolName, setToolName] = useState<string>('Select')
  /** Live stage-aware guidance from the active tool (Tool.statusHint);
   *  null = fall back to the palette's static tool description. */
  const [toolStageHint, setToolStageHint] = useState<string | null>(null)
  /** Fly's live state (docs/design/fly-navigation.md §4): `'hold'` while the
   * right button is down on the canvas, `'latched'` while Camera ▸ Fly is
   * on — drives the status-bar movement hint, the speed readout in the
   * Measurements box, and the Camera ▸ Fly check mark. */
  const [fly, setFly] = useState<FlyState>({ mode: 'off', speedText: '', locked: false, canLock: false })
  /** Precision snapping (the Ctrl/⌘+Alt chord held) — modal state, so it
   * gets a status-bar chip rather than living only in the viewport. */
  const [precisionSnap, setPrecisionSnap] = useState(false)
  const [snapKind, setSnapKind] = useState<string | null>(null)
  const [measurement, setMeasurement] = useState<string>('')
  /** True when `measurement` is a finished Tape Measure reading kept on
   *  screen for reference (tape-measure-rework part 1) rather than a live
   *  typed buffer — every other tool's callback never passes `frozen`, so
   *  this stays false for them. */
  const [measurementFrozen, setMeasurementFrozen] = useState(false)
  /** Live inference-cursor info for the tooltip chip. */
  const [inferenceInfo, setInferenceInfo] = useState<InferenceInfo | null>(null)
  const [toasts, setToasts] = useState<Toast[]>([])
  /** Per-object watertight map (bigint key) */
  const [watertightMap, setWatertightMap] = useState<Map<bigint, boolean>>(new Map())
  /** Tool driven from toolbar clicks */
  // oxlint-disable-next-line lingui/no-unlocalized-strings -- ToolName identifier
  const [activeTool, setActiveTool] = useState<ToolName>('Select')
  /** Bumped on every EXPLICIT camera-tool selection (menu/native
   * menu/keyboard) — see `activateTool` below. Forces Viewport to re-apply
   * the tool even when `name` is unchanged from `activeTool`'s current
   * value, which React's setState would otherwise treat as a no-op (it
   * bails out on an unchanged value, so re-choosing the menu's own already-
   * checked entry would silently do nothing without this). `activeTool`
   * itself stays truthful at all times now — Viewport's
   * `onInternalToolChange` (see `handleInternalToolChange` below) reports
   * every internal transition (auto-handoffs, Escape-to-Select, one-shot
   * reverts) the instant it happens — so this exists purely to force a real
   * re-application of an explicit reselect, not to paper over any drift. */
  const [toolActivationSeq, setToolActivationSeq] = useState(0)
  /** The one entry point every camera-tool selection (Camera menu, native
   * menu, keyboard shortcut) should go
   * through — see `toolActivationSeq`'s doc comment. Scoped to the camera
   * group: forcing every tool to re-apply on a same-name reselect from a
   * menu or the rail would also reset in-progress non-camera gestures (e.g.
   * a multi-click Line chain). Only a tool's keyboard shortcut does that,
   * on purpose (the keymap dispatcher's `run` below). */
  const activateTool = useCallback((name: ToolName) => {
    setActiveTool(name)
    if (CAMERA_HANDOFF_TOOL_NAMES.has(name)) setToolActivationSeq((s) => s + 1)
  }, [])
  /** The truthful source `activeTool` is built from for every INTERNAL tool
   * transition — Viewport's switchToolRef reports the tool that just
   * actually became active here on EVERY invocation, whether the parent
   * requested it (an echo, this then no-ops) or Viewport decided it
   * internally (an auto-handoff, an Escape, a one-shot revert). Skips the
   * update when the name already matches: both a harmless React bailout on
   * an echo, and — the important part — this must never fight
   * `toolActivationSeq` above, which forces a real reapply for an EXPLICIT
   * reselect; this callback only follows what Viewport already did, it
   * never itself requests a switch or touches that counter. */
  const handleInternalToolChange = useCallback((name: string) => {
    setActiveTool((prev) => (prev === name ? prev : (name as ToolName)))
  }, [])

  /** Sticky banner: true once a kernel panic / borrow-lock is detected */
  const [kernelPanicked, setKernelPanicked] = useState(false)
  /** Selected nodes (ordered; index 0 = primary). */
  const [selectedIds, setSelectedIds] = useState<NodeRef[]>([])
  /** Selected construction guide, mutually exclusive with node
   * selection. Deleted via the same Edit ▸ Delete / Delete-key path as nodes. */
  const [selectedGuide, setSelectedGuide] = useState<bigint | null>(null)
  /** Selected dimension/leader-text annotation, mutually exclusive with node
   * and guide selection (docs/design/dimensions-text.md). Deleted via the
   * same Edit ▸ Delete / Delete-key path. */
  const [selectedAnnotation, setSelectedAnnotation] = useState<bigint | null>(null)
  /** Session-only hidden node set (keyed by nodeKey). Cleared on load/new. */
  const [hiddenKeys, setHiddenKeys] = useState<Set<string>>(new Set())
  /** Whether Paste/Paste In Place have something to work with — gates the
   *  Edit menu items, the native menu (sync_menu_state), and the palette.
   *  Reflects THIS window's local clipboard cache; refreshed from the
   *  shared Tauri shell state on focus (see the effect below) so switching
   *  into a window after copying elsewhere picks it up promptly. */
  const [clipboardAvailable, setClipboardAvailable] = useState(() => hasClipboardContent())
  useEffect(() => {
    if (!isTauri) return
    const refresh = () => {
      void refreshClipboardFromShell().then(() => setClipboardAvailable(hasClipboardContent()))
    }
    refresh()
    window.addEventListener('focus', refresh)
    return () => window.removeEventListener('focus', refresh)
  }, [])
  /** Object-context path: app-only sticky editing of at most one plain
   *  object (or, via the K1/K2 fallback, a component instance whose pose/
   *  grouping can't support a real session), sitting logically inside the
   *  innermost open session frame, if any (docs/design/group-session.md).
   *  Empty = nothing pushed past the session stack (or top level, if no
   *  session is open either). A group NodeRef is never pushed here anymore
   *  — a group entry always opens a session instead (`sessionStack` below). */
  const [activeContext, setActiveContextState] = useState<NodeRef[]>([])
  /** The open kernel session stack, outermost first — mirrors the
   *  viewport's own belief (`ViewportApi.sessionStack`), pushed up via
   *  `onSessionChange` on open, close, and every undo/redo resync. Each
   *  frame is a group or a component instance (always innermost) plus its
   *  display label. Drives menu gating (Make Group/Make Component/Place
   *  Copy/Import/3D Text refuse only while the INNERMOST frame is a
   *  component; whole-model rescale refuses while ANY frame is open) and
   *  the breadcrumb/"editing" chips. Scoping/dimming stay inside the
   *  viewport, which owns the renderer and needs no cross-render state to
   *  apply them. */
  const [sessionStack, setSessionStack] = useState<{ node: NodeRef; label: string }[]>([])
  /** The leaf a viewport click hit, paired with the node that click selected
   *  (docs/design/selection-path.md "The strip"): the strip's crumbs below
   *  the selection exist only for such a selection. Any other route to the
   *  selection clears it (the effect below). */
  const [pathHit, setPathHit] = useState<{ leaf: HitLeaf; sel: NodeRef } | null>(null)
  /** The select snap of the click in flight — the viewport fires it just
   *  before `onSelect` (see `handleSelect` in Viewport.tsx). */
  const selectSnapLeafRef = useRef<HitLeaf | null>(null)
  /** The innermost open session frame's NodeRef, or `null`. */
  const innermostSessionNode: NodeRef | null =
    sessionStack.length > 0 ? sessionStack[sessionStack.length - 1].node : null
  /** Whether a COMPONENT frame is innermost — the gate for grouping/
   *  Make Component/Place Copy/Import/3D Text (docs/design/group-session.md
   *  §"Gate changes"): the kernel permits all of these during a group-only
   *  stack, so this is narrower than "any session open"
   *  (`sessionStack.length > 0`, still used for whole-model rescale). */
  const componentFrameOpen = innermostSessionNode?.kind === 'instance'
  /** Bumped on any document change so the tree re-queries entity lists. */
  const [docRev, setDocRev] = useState(0)
  /** View-only isolation (ViewportApi.isolate): how many objects are shown, or
   *  null when everything is. The viewport owns the targets; this feeds the bar. */
  const [isolatedCount, setIsolatedCount] = useState<number | null>(null)
  /** Width of the open Make Solid wizard (it covers the viewport's right side),
   *  so the isolation bar centers in the part left uncovered. */
  const [fixWizardWidth, setFixWizardWidth] = useState(0)
  // Bumped once per document LOAD (open/new/recover/library-open — every
  // path funnels through applyLoadedBytes), never on an in-document edit.
  // Panels that cache blob URLs keyed by a wasm handle use this to drop the
  // cache across loads: a fresh document restarts its SlotMap key versions,
  // so material #1 of document B collides with the cached handle of material
  // #1 of document A (audit q-web-robustness).
  const [docGeneration, setDocGeneration] = useState(0)
  /** Currently selected material id for the Paint tool. */
  const [currentMaterialId, setCurrentMaterialId] = useState<bigint>(MATERIAL_SENTINEL)
  // Picking a material makes it current AND activates the Paint tool, so the
  // next click paints with it (Ctrl/Cmd-click fills the whole object). This is
  // the whole-object path that the removed "Fill selected object" button used
  // to serve — the status bar spells out the Ctrl/Cmd-click shortcut.
  const handleSelectMaterial = (id: bigint) => {
    setCurrentMaterialId(id)
    setActiveTool('Paint')
  }
  /** Document session: currentRef + dirty flag. */
  const [docSession, setDocSession] = useState<DocSessionState>(INITIAL_SESSION)
  // File ▸ Revert to Saved has a file to return to and changes to throw away.
  const revertable = canRevertToSaved(docSession)
  /** Ticks purely to refresh the "Edited/Saved <relative time>" indicator —
   * nothing else reads this state. Scheduled on demand, at the exact next
   * moment the label's text would change (see the save-state-indicator-tick
   * effect below), not on a fixed interval. */
  const [nowTick, setNowTick] = useState(() => Date.now())
  /** Tray-section expanded state ( sections; the showX names predate the
   * tray — they used to mean floating-panel visibility). Initialized from and
   * written back to the trayLayout singleton so the layout survives
   * relaunches; the setters, shortcuts, and Window-menu checkmarks are
   * untouched. */
  /** Pane visibility: Model info (DocumentTree) */
  const [showModelInfo, setShowModelInfo] = useState(() => getTrayLayout().modelInfo)
  /** Pane visibility: Materials (MaterialPalette) */
  const [showMaterials, setShowMaterials] = useState(() => getTrayLayout().materials)
  /** Pane visibility: Components (ComponentsPanel) */
  const [showComponents, setShowComponents] = useState(() => getTrayLayout().components)
  /** Pane visibility: Tags */
  const [showTags, setShowTags] = useState(() => getTrayLayout().tags)
  /** Pane visibility: Scenes (docs/design/scenes.md §5) */
  const [showScenes, setShowScenes] = useState(() => getTrayLayout().scenes)
  /** Pane visibility: Changes (Lane C, docs/design/v1.1-cycle.md) */
  const [showChanges, setShowChanges] = useState(() => getTrayLayout().changes)
  /** Model Health's floating window (docs/design/import-health.md §5). Not
   * part of the persisted tray layout: it draws a viewport overlay, so it
   * starts closed every launch and is opened by the Object menu, Object
   * Info's Show link, or the status badge. */
  const [showModelHealth, setShowModelHealth] = useState(false)
  const showModelHealthRef = useRef(showModelHealth)
  showModelHealthRef.current = showModelHealth
  /** Model Health's scope select and its one-object filter (Object Info's
   * Show link) — lifted here so the entry points can set them. */
  const [healthScope, setHealthScope] = useState<HealthScope>('document')
  const [healthObjectFilter, setHealthObjectFilter] = useState<bigint | null>(null)
  /** The Make Solid wizard: null when closed, else where it was opened from
   *  (the last import, the selection with its object ids, or the whole model). */
  const [makeSolid, setMakeSolid] = useState<{ scope: WizardScope; objectIds: bigint[]; nonce: number } | null>(null)
  /** Make Solid on a selection of sketches (wireframe faces §6): the second
   *  input of Object ▸ Make Solid…; null when closed. */
  const [makeSolidSketches, setMakeSolidSketches] = useState<{ sketchIds: bigint[]; nonce: number } | null>(null)
  /** The sides of the viewport the Model Health window covers (zeros while
   *  it is closed). */
  const [healthCover, setHealthCover] = useState({ left: 0, right: 0 })
  // Framing an isolated item fits it beside the wizard and the Model Health
  // window, not under them.
  useEffect(() => {
    const wizard = makeSolid !== null || makeSolidSketches !== null ? fixWizardWidth : 0
    viewportApi.current?.setFramingInset?.(Math.max(wizard, healthCover.right), healthCover.left)
  }, [makeSolid, makeSolidSketches, fixWizardWidth, healthCover])
  const openMakeSolid = useCallback((scope: WizardScope, objectIds: bigint[] = []) => {
    setMakeSolidSketches(null)
    setMakeSolid((prev) => ({ scope, objectIds, nonce: (prev?.nonce ?? 0) + 1 }))
  }, [])
  const openMakeSolidSketches = useCallback((sketchIds: bigint[]) => {
    setMakeSolid(null)
    setMakeSolidSketches((prev) => ({ sketchIds, nonce: (prev?.nonce ?? 0) + 1 }))
  }, [])
  // The window is about the selection it opened on: a different one closes
  // it (and with it the overlay it drew).
  const sketchSelectionRef = useRef(selectedIds)
  useEffect(() => {
    if (sketchSelectionRef.current === selectedIds) return
    sketchSelectionRef.current = selectedIds
    setMakeSolidSketches(null)
  }, [selectedIds])
  /** Bumped on every request to open Model Health, to bring its window
   * forward (the window takes focus). */
  const [healthFocusNonce, setHealthFocusNonce] = useState(0)
  /** Pane visibility: Object Info */
  const [showObjectInfo, setShowObjectInfo] = useState(() => getTrayLayout().objectInfo)
  /** Debug Log panel visibility (default hidden — opt-in via Window menu only). */
  const [showDebugLog, setShowDebugLog] = useState(false)
  /** Library browser dialog visibility (Window ▸ Library, ⇧⌘L / bare ⇧L,
   * the tool rail's Library row, and the command palette's "Library" entry
   * all toggle/open this — see the `toggle-library`/`open-library`
   * menuActionRef cases). Not part of trayLayout — it's a modal, not a
   * docked pane. */
  const [showLibrary, setShowLibrary] = useState(false)
  /** View ▸ Axes / Grid / Guides visibility. Default all shown. */
  const [showAxes, setShowAxes] = useState(true)
  const [showGrid, setShowGrid] = useState(true)
  // Whole-model render mode (View ▸ X-ray / Wireframe): one value, so the
  // two are mutually exclusive. Per window, not saved in the file, not
  // persisted, so a new document starts Shaded.
  const [renderMode, setRenderMode] = useState<RenderMode>('shaded')
  const xrayOn = renderMode === 'xray'
  const wireframeOn = renderMode === 'wireframe'
  const toggleXray = useCallback(() => setRenderMode((m) => toggledRenderMode(m, 'xray')), [])
  const toggleWireframe = useCallback(() => setRenderMode((m) => toggledRenderMode(m, 'wireframe')), [])
  const [showGuides, setShowGuides] = useState(true)
  /** Camera ▸ Parallel Projection checkbox state (docs/design/camera.md §1)
   * — a RENDER CACHE of the rig's own truth, mirroring showAxes/showGrid's
   * pattern but populated from Viewport's `onProjectionChange` callback
   * (mount + every `toggleProjection`) rather than toggled directly here. */
  const [parallelProjection, setParallelProjection] = useState(false)
  /** View ▸ Section Cut's check/enabled state — a RENDER CACHE of the
   * section manager's own truth, never toggled directly by this component.
   * Populated only by `handleSectionChanged` re-reading
   * `viewportApi.current?.getSectionState()` (D3, section-plane-polish);
   * unlike showAxes/showGrid/showGuides (which this component itself owns
   * and pushes DOWN into the viewport), the section's existence/active flag
   * is owned by the viewport's session-only SectionManager and pulled UP
   * here, so it can never independently drift from kernel/session truth. */
  const [sectionPlaneMenuState, setSectionPlaneMenuState] = useState<{ checked: boolean; exists: boolean }>({
    checked: false,
    exists: false,
  })
  /** Settings modal visibility — web fallback only (Tauri opens a real OS window). */
  const [showSettingsModal, setShowSettingsModal] = useState(false)
  // Windows desktop settings surface (Fluent in-app page; see openSettings).
  const [showFluentSettings, setShowFluentSettings] = useState(false)
  /** Command palette visibility (⌘/ / Ctrl+/). */
  const [paletteOpen, setPaletteOpen] = useState(false)
  /** Query the palette opens with — '' everywhere except Help ▸ Search,
   *  which hands its typed text over (see MenuBar's onHelpSearch). */
  const [paletteSeed, setPaletteSeed] = useState('')
  const openPalette = useCallback((seed = '') => {
    setPaletteSeed(seed)
    setPaletteOpen(true)
  }, [])
  /** True while the user is pointer-dragging the camera (orbit/pan/dolly) —
   * fades the contextual dock out of the way. Fed by Viewport's
   * OrbitControls start/end events; never persisted. */
  const [cameraDragging, setCameraDragging] = useState(false)
  /** True while the cursor is aimed at a live sketch's extrudable region AND
   * nothing is selected ( hover-dock) — Viewport polls+throttles this
   * via SketchHoverGate and only calls back on a true/false transition. An
   * explicit selection's dock always wins; ContextualDock only consults this
   * when the derived context is 'empty'. */
  const [hoveringSketchRegion, setHoveringSketchRegion] = useState(false)
  /** Tag-path hide set: each entry is tagPathKey(path). Cleared on load/new. */
  const [hiddenTagPaths, setHiddenTagPaths] = useState<Set<string>>(new Set())
  /** Import report to display (null = no dialog). */
  const [importReport, setImportReport] = useState<ImportReport | null>(null)
  // Whether the report (and the busy overlay) belong to File ▸ Open of a
  // .skp — the document's own file, "Opening" — or to an import.
  const [reportOpened, setReportOpened] = useState(false)
  /** Result of a `.skp` write that could not hold everything exactly (a
   * Save / Save As / Export notice); null = none showing. */
  const [skpNotice, setSkpNotice] = useState<SkpLossNoticeData | null>(null)
  /** Save As format dialog: the format pre-selected, plus the resolver of
   * the promise `promptSaveFormat` hands the Save As flow; null = closed. */
  const [saveFormatPrompt, setSaveFormatPrompt] = useState<{ initial: DocFormat; compat: boolean } | null>(null)
  /** True while import_dae is running (blocks main thread). */
  const [isImporting, setIsImporting] = useState(false)
  /** How far a `.skp` open has come (the overlay's bar); null while an
   * import gives no progress. */
  const [openProgress, setOpenProgress] = useState<OpenProgress | null>(null)
  /** Display name of the file being imported (shown in the overlay). */
  const [importingName, setImportingName] = useState('')
  /** The busy card's line while a long edit runs (`runBusy`), else null. */
  const [busyNote, setBusyNote] = useState<string | null>(null)
  /** Show the busy card, let it paint, then run `work` — a long synchronous
   *  edit holds the main thread, so the card must be on screen first. */
  const busyRef = useRef(false)
  const runBusy = useCallback((message: string, work: () => void): boolean => {
    // One at a time, and no keys while the card paints: a second command or
    // an Undo in those frames would run against objects the first consumes.
    // A refused one is the caller's to tell (false).
    if (busyRef.current) return false
    busyRef.current = true
    // Keys pressed while the work freezes the main thread queue up and
    // fire once it returns, so the listener outlives the work and drops
    // every key stamped before the work ended.
    let endedAt = Infinity
    const swallow = (ev: KeyboardEvent) => {
      if (ev.timeStamp > endedAt) return
      ev.preventDefault()
      ev.stopImmediatePropagation()
    }
    window.addEventListener('keydown', swallow, true)
    flushSync(() => setBusyNote(message))
    void nextPaint().then(() => {
      try {
        work()
      } finally {
        endedAt = performance.now()
        window.setTimeout(() => window.removeEventListener('keydown', swallow, true), 100)
        busyRef.current = false
        setBusyNote(null)
      }
    })
    return true
  }, [])
  /** STL units-chooser modal: the picked file's name, or null when closed. */
  const [pendingStlImport, setPendingStlImport] = useState<{ name: string } | null>(null)
  /** Tape Measure "resize the model?" confirmation (design tool-parity §3,
   *  and group-session.md's scoped-rescale phase); null when closed. `scope`
   *  is the decision made HERE, in `handleRescaleArmed`, from the same
   *  `sessionStack` the confirm call below threads straight through — so the
   *  dialog's copy and the eventual `rescale_session`-vs-`rescale_document`
   *  call can never disagree. `null` = whole-model rescale. `scope.node` is
   *  the captured innermost frame's own identity (finding 5) — Ctrl/Cmd+Z
   *  and the native Edit>Undo both stay live while this dialog is up, so the
   *  session stack can change out from under it before Confirm; both
   *  `handleSessionChange` (any stack change while this is non-null) and the
   *  dialog's own onConfirm (a fresh re-read right before applying) use it to
   *  invalidate the pending arm rather than let the kernel rescale whatever
   *  frame is NOW innermost. */
  const [pendingRescaleConfirm, setPendingRescaleConfirm] = useState<{
    info: RescaleConfirmInfo
    scope: { label: string; isComponent: boolean; node: NodeRef } | null
  } | null>(null)
  /** Mirrors `pendingRescaleConfirm` for `handleSessionChange`'s call-time
   *  read — that callback has a stable (`[]`-deps) identity called from the
   *  viewport's own stable closures, so it can't read fresh render state
   *  directly; this ref is kept in lockstep by every `setPendingRescaleConfirm`
   *  call site (finding 5). */
  const pendingRescaleConfirmRef = useRef(pendingRescaleConfirm)
  pendingRescaleConfirmRef.current = pendingRescaleConfirm
  /** File ▸ Revert to Saved is waiting for its one confirmation. */
  const [revertPromptOpen, setRevertPromptOpen] = useState(false)
  /** A material delete pending confirmation (used on >0 faces/objects) —
   *  null once confirmed/cancelled, or when a delete of an UNUSED material
   *  never needed a modal at all (v1.1 assets lane). */
  const [pendingMaterialDelete, setPendingMaterialDelete] = useState<{
    id: bigint
    name: string
    usage: number
  } | null>(null)
  /** A component-definition delete pending confirmation (used by >0
   *  instances). */
  const [pendingComponentDelete, setPendingComponentDelete] = useState<{
    id: bigint
    name: string
    usage: number
  } | null>(null)
  /** Purge Unused's preview — what would be removed, shown before commit.
   *  null when no dialog is open (including the "nothing to purge" case,
   *  which never opens one — a toast alone is enough). */
  const [pendingPurgePreview, setPendingPurgePreview] = useState<{
    materialNames: string[]
    definitionNames: string[]
  } | null>(null)
  /** Recovery snapshot to offer at startup (null = no dialog). */
  const [recoveryPrompt, setRecoveryPrompt] = useState<RecoveryListing[] | null>(null)
  /** Welcome screen on a bare launch (startup-handoff effect decides). */
  const [welcomeOpen, setWelcomeOpen] = useState(false)
  /** True once any document has loaded — vetoes a late welcome-screen offer. */
  const documentLoadedRef = useRef(false)
  /** Mirror of welcomeOpen for the window-level key handlers (some have
   *  empty dep arrays, so they can't read the state directly). */
  const welcomeOpenRef = useRef(false)
  welcomeOpenRef.current = welcomeOpen
  /** Mirror of the persisted "show welcome on startup" flag, so the
   *  dialog's checkbox re-renders on toggle. */
  const [showWelcomeSetting, setShowWelcomeSetting] = useState(getShowWelcome)
  // The welcome screen's unit dropdown mirrors the units singleton; kept in
  // React state so the select re-renders, and re-synced from external
  // changes (the Settings surface) while the screen is up.
  const [welcomeUnit, setWelcomeUnit] = useState<LengthFormat>(getLengthUnit)
  useEffect(() => subscribeLengthUnit(setWelcomeUnit), [])
  /** Right-tray width (px), user-resizable via the drag handle; persisted. */
  const [trayWidth, setTrayWidth] = useState<number>(() => {
    const raw = window.localStorage.getItem(TRAY_WIDTH_KEY)
    const n = raw !== null ? Number(raw) : NaN
    return Number.isFinite(n) ? clampTrayWidth(n) : TRAY_WIDTH_DEFAULT
  })
  /** Icons-only tool rail (View ▸ Compact Tool Rail). Persisted like the
   *  tray width: read once at mount, so each open window keeps its own, and
   *  the last value set is what a new window or the next launch starts with
   *  (one localStorage origin serves every window). */
  const [railNarrow, setRailNarrow] = useState<boolean>(
    () => window.localStorage.getItem(RAIL_NARROW_KEY) === '1',
  )
  useEffect(() => {
    window.localStorage.setItem(RAIL_NARROW_KEY, railNarrow ? '1' : '0')
  }, [railNarrow])
  const toggleRailNarrow = useCallback(() => setRailNarrow((narrow) => !narrow), [])
  /** Tray hidden entirely (View ▸ Tray); persisted the same way as the rail
   *  above, and separately from its width so the width comes back
   *  unchanged. */
  const [trayCollapsed, setTrayCollapsed] = useState<boolean>(
    () => window.localStorage.getItem(TRAY_COLLAPSED_KEY) === '1',
  )
  useEffect(() => {
    window.localStorage.setItem(TRAY_COLLAPSED_KEY, trayCollapsed ? '1' : '0')
  }, [trayCollapsed])
  /** Contextual dock turned off (View ▸ Contextual Dock); persisted the
   *  same way as the rail and tray above. Everything on the dock also lives
   *  in the menus and palette, so hiding it takes nothing away. */
  const [dockHidden, setDockHidden] = useState<boolean>(
    () => window.localStorage.getItem(DOCK_HIDDEN_KEY) === '1',
  )
  useEffect(() => {
    window.localStorage.setItem(DOCK_HIDDEN_KEY, dockHidden ? '1' : '0')
  }, [dockHidden])
  const toggleDock = useCallback(() => setDockHidden((hidden) => !hidden), [])
  const trayCollapsedRef = useRef(trayCollapsed)
  trayCollapsedRef.current = trayCollapsed
  const trayRef = useRef<HTMLDivElement>(null)
  /** Whether the tray toggle that mounts next should take focus — true when
   *  the one it replaces (or anything else inside the hidden tray) had it. */
  const [trayToggleTakesFocus, setTrayToggleTakesFocus] = useState(false)
  const hideTray = useCallback(() => {
    if (trayCollapsedRef.current) return
    setTrayToggleTakesFocus(blurFocusWithin(trayRef.current))
    setTrayCollapsed(true)
  }, [])
  const showTray = useCallback(() => {
    if (!trayCollapsedRef.current) return
    const active = document.activeElement
    setTrayToggleTakesFocus(active instanceof HTMLElement && active.getAttribute('data-tray-show-tab') === 'true')
    setTrayCollapsed(false)
  }, [])
  const toggleTray = useCallback(() => {
    if (trayCollapsedRef.current) showTray()
    else hideTray()
  }, [hideTray, showTray])
  /** Every section shortcut and menu item goes through here. With the tray
   *  hidden, asking for a section brings the tray back with that section
   *  open, rather than flipping it unseen. Deliberately not an effect on the
   *  showX flags: those sync live across windows (settings/trayLayout.ts),
   *  while whether the tray is shown is each window's own.
   *  `title` names the TraySection, so a field focused inside it can commit
   *  before the section closes (see blurFocusWithin). */
  const toggleSection = useCallback((setShown: (update: (shown: boolean) => boolean) => void, title: string) => {
    if (trayCollapsedRef.current) {
      showTray()
      setShown(() => true)
    } else {
      blurFocusWithin(trayRef.current?.querySelector(`[data-tray-section-body="${CSS.escape(title)}"]`) ?? null)
      setShown((shown) => !shown)
    }
  }, [showTray])
  /** Open document windows (Tauri multi-window only) — the Window menu's
   *  tail of focus-this-window entries. Populated by `list_windows` at
   *  mount and kept fresh by the shell's `window-list` broadcast (window
   *  create/destroy/focus/title-change). Always empty on the web build
   *  (single window, no shell to ask) and on macOS Tauri, where the native
   *  menu renders its own tail directly — this only feeds the in-app
   *  MenuBar (Windows/Linux). */
  const [windowList, setWindowList] = useState<{ label: string; title: string; focused: boolean }[]>([])

  /** Imperative handle into the viewport (e.g. running a boolean). */
  const viewportApi = useRef<ViewportApi | null>(null)

  // Stable ref to the Scene for undo/redo button state queries
  const sceneRef = useRef<Scene | null>(null)
  /** The in-context entries, always current: `activeContext` above is the
   *  render copy. Every change goes through `setActiveContext`, which keeps
   *  this and the Scene's mirror (`Scene.set_ui_context`) in step in the
   *  same synchronous call — so a bus-driven entry (`hew.view.enter`) whose
   *  session opens and fallback push both land in one call stack reports
   *  them in ONE `context_changed`, not a stack event now and a fallback
   *  event a render later. */
  const activeContextRef = useRef<NodeRef[]>([])
  const setActiveContext = useCallback((next: NodeRef[] | ((prev: NodeRef[]) => NodeRef[])) => {
    const value = typeof next === 'function' ? next(activeContextRef.current) : next
    activeContextRef.current = value
    setActiveContextState(value)
    sceneRef.current?.set_ui_context(uiContextJson(value))
  }, [])
  // Bytes of a fresh blank scene, captured once after kernel load, used for New.
  const blankBytesRef = useRef<Uint8Array | null>(null)
  // Stable file host instance.
  const fileHostRef = useRef(makeFileHost())
  // The path the native save panel picked for the export under way, which
  // the export writes to instead of asking again (`writeExport`); the last
  // format exported, which the next Export opens on.
  const exportTargetRef = useRef<string | null>(null)
  const lastExportFormatRef = useRef<PanelExportFormat>('glb')
  const printHostRef = useRef(makePrintHost())
  // Test-only recording print host (harness.ts `setPrintRecorder`).
  const printRecorderRef = useRef<PrintRecorder | null>(null)
  // Resolver for the in-flight StlUnitsDialog promise (see promptStlUnits);
  // null when no chooser is open.
  const stlUnitsResolveRef = useRef<((unitScale: number | null) => void) | null>(null)
  // Guards openDocument/importDocument against re-entry: a second gesture
  // started while the first is mid-flight (e.g. Ctrl+K ▸ "import" ▸ Enter
  // behind the units modal) would otherwise orphan the first, hanging it
  // forever. Shared between both entry points — they converge on the same
  // post-pick import steps (runImportPick) and must not race each other. A
  // ref (not state) so the guard is synchronous, before the first await.
  const openInFlightRef = useRef(false)
  /** A `.skp` open is stepping between paints (see runOpenSkp): nothing
   * else may replace the document until it lands. A ref, so the check is
   * synchronous. */
  const skpOpenStepsRef = useRef(false)
  // The one-time `.skp` loss notice has been shown for the current document
  // (reset on every document load — Open / New — never on save).
  const skpNoticeShownRef = useRef(false)
  // The document's file is revision 17 of the 2021-and-later container (or was
  // last saved with "avoid features older SketchUp versions cannot open"): a
  // save keeps to it. Set by an open and by Save As; false for anything else.
  const skpCompatRef = useRef(false)
  // Resolver for the Save As format dialog's promise (see promptSaveFormat).
  const saveFormatResolveRef = useRef<((answer: SaveFormatAnswer | null) => void) | null>(null)
  // `.skp` source persistence for crash recovery: the source (100+ MB) is
  // written to the recovery store only when it changes, tracked by a
  // generation counter bumped wherever `Scene` replaces it (a load clears it,
  // open_skp / save_skp set it, adoptSnapshot restores it).
  const skpSourceGenRef = useRef(0)
  const persistedSourceGenRef = useRef(-1)
  // Starts true: a crashed or dismissed earlier session may have left a
  // source beside this window's recovery slot, and a document without one
  // must clear it on its first autosave rather than inherit it.
  const sourceOnDiskRef = useRef(true)
  // Mirror of docSession kept up-to-date so callbacks can read current state
  // without capturing stale closures or causing impure updater functions.
  const docSessionRef = useRef<DocSessionState>(INITIAL_SESSION)
  // When true, handleDocumentChanged suppresses the dirty-marking setState
  // (used during programmatic loads so the post-load afterOpen wins).
  const suppressDirtyRef = useRef(false)
  // saveDocument/saveAsDocument are defined much further down (they close
  // over docSession.currentRef); declared here (initial no-op, kept current
  // by an effect beside their real definitions) so confirmDiscard and the
  // unsaved-changes dialog's Save button — both defined earlier — can
  // trigger "the current save flow" without a forward reference.
  const saveDocumentRef = useRef<() => Promise<boolean>>(async () => false)
  const saveAsDocumentRef = useRef<() => Promise<boolean>>(async () => false)
  // Stable recovery-store instance (autosave / crash recovery).
  const recoveryStoreRef = useRef(makeRecoveryStore())
  // pushUnionHidden is defined further down (it depends on `state`); reach it
  // through a ref (kept current beside its definition) like reconcileRef /
  // applyLoadedBytesRef, so applyLoadedBytes and importDocument — both defined
  // earlier — can push freshly-seeded hidden state without a stale closure.
  const pushUnionHiddenRef = useRef<(nextHiddenKeys: Set<string>, nextHiddenTagPaths: Set<string>) => void>(() => {})
  // Re-seeds the session's hidden-tag set from the document's tag registry
  // and re-pushes the visibility union. Undo/redo can change the registry
  // (delete tag restores/removes hidden flags with it), so handleUndo/
  // handleRedo — defined before the seeding helpers — reach the latest
  // closure through this ref, same pattern as pushUnionHiddenRef.
  const resyncTagVisibilityRef = useRef<() => void>(() => {})
  // Same forward-reference pattern for `reconcileVisibility` (the cheap,
  // change-detecting variant) — the live bridge and the test harness install
  // once on mount and reach the latest closure through it.
  const reconcileVisibilityRef = useRef<() => void>(() => {})
  // Latest tag handlers for the test harness — it installs once on mount
  // (before these are defined further down), so it reaches them through
  // refs, the same pattern as reconcileRef/applyLoadedBytesRef.
  const toggleTagPathRef = useRef<(path: string[]) => void>(() => {})
  const deleteTagRef = useRef<(path: string[]) => void>(() => {})
  // True when a mutation has occurred since the last successful autosave tick
  // (or since the last explicit Save, which also resets it). Avoids redundant
  // writes when the document hasn't changed between ticks.
  const dirtySinceAutosaveRef = useRef(false)
  // The autosave write currently in flight (never rejects), or null before
  // the first write. clearRecoverySnapshot awaits it so a discard's clear
  // can't interleave with a write of the very snapshot being discarded.
  const autosaveWriteRef = useRef<Promise<void> | null>(null)
  // The pending autosave `setTimeout` handle, or null while disarmed. Used
  // to be an unconditional `setInterval` running for the component's whole
  // lifetime, polling every 12s forever even on a clean, unedited document
  // (Lane F, "Render loop on demand" — the same idle-cost concern the render
  // loop had, applied to this timer). Now armed only by `armAutosaveTick`
  // (called wherever a mutation used to set `dirtySinceAutosaveRef.current =
  // true` directly) and disarmed the moment there's nothing left to do: a
  // successful write, an explicit Save/discard (`clearRecoverySnapshot`), or
  // a tick that finds the document already clean.
  const autosaveTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  // Indirection for `armAutosaveTick` to reach the tick body defined by the
  // autosave effect below — same forward-reference-via-ref pattern as
  // `resyncTagVisibilityRef` above, needed because `armAutosaveTick` must be
  // callable from callbacks declared earlier in the component than that
  // effect.
  const runAutosaveTickRef = useRef<() => void>(() => {})
  const armAutosaveTick = useCallback((): void => {
    dirtySinceAutosaveRef.current = true
    if (autosaveTimerRef.current === null) {
      autosaveTimerRef.current = setTimeout(() => {
        autosaveTimerRef.current = null
        runAutosaveTickRef.current()
      }, AUTOSAVE_INTERVAL_MS)
    }
  }, [])
  // Resolved once this window's menu-open-path listener is registered (or
  // registration failed) — the startup-handoff effect awaits it before
  // telling the shell this webview is ready for live open delivery. Created
  // during render so the promise exists before any effect runs.
  const openListenerReadyRef = useRef<{ promise: Promise<void>; resolve: () => void } | null>(null)
  if (openListenerReadyRef.current === null) {
    let resolve!: () => void
    const promise = new Promise<void>((r) => { resolve = r })
    openListenerReadyRef.current = { promise, resolve }
  }

  // Install console capture on mount, restore on unmount.
  useEffect(() => {
    installConsoleCapture()
    return () => {
      restoreConsoleCapture()
    }
  }, [])

  useEffect(() => {
    loadKernel()
      .then((kernel) => {
        const kernelVersion = kernel.version()
        const scene = kernel.newScene()
        sceneRef.current = scene
        // Snapshot blank scene bytes for "New" resets.
        blankBytesRef.current = new Uint8Array(scene.save())
        setState({ kernelVersion, scene })
        LogStore.log.info('app', `Kernel loaded — version ${kernelVersion}`)
      })
      .catch((err: unknown) => {
        const msg = String(err)
        setError(msg)
        LogStore.log.error('app', `Kernel load failed: ${msg}`)
      })
  }, [])

  // ---------------------------------------------------------------- Debug Mode
  // Apply the GLOBAL half of Debug Mode's effects (file logging + input
  // recording + torture mode on the CURRENT scene) once on mount and on every
  // subsequent toggle. The PER-SCENE half — a freshly-created Scene (New/Open)
  // inheriting the current mode — is covered by wasm/loader.ts's newScene().
  useEffect(() => {
    const apply = (on: boolean): void => {
      diagnosticLog.setFileLogging(on)
      if (on) {
        inputRecorder.start()
      } else {
        inputRecorder.stop()
      }
      sceneRef.current?.set_torture_mode(on)
    }
    apply(getDebugMode())
    return subscribeDebugMode(apply)
  }, [])

  // View ▸ Scenes ▸ Scene Transitions checkmark (docs/design/scenes.md §5 —
  // a SPEC.md deviation: the design put this in Settings, but it lives in
  // the View menu at a fixed 600ms instead, same posture as `showDebugMode`
  // above). `useScenesController` reads the singleton directly for the tween
  // duration; this is only the menu/palette checkmark's own render cache.
  const measurementsPosition = useViewSetting(getMeasurementsPosition, subscribeMeasurementsPosition)
  const [sceneTransitionsOn, setSceneTransitionsOn] = useState(() => getSceneTransitions())
  useEffect(() => subscribeSceneTransitions(setSceneTransitionsOn), [])

  // ---------------------------------------------------------------- tray layout persistence
  // Write the four section flags back to the singleton whenever any of them
  // changes (also fires once on mount, writing the just-restored values —
  // harmless). The subscription mirrors the other settings singletons and
  // covers cross-window/tab changes; for locally-originated changes it echoes
  // the values React already has, so the setState calls bail out.
  useEffect(() => {
    setTrayLayout({
      modelInfo: showModelInfo,
      objectInfo: showObjectInfo,
      materials: showMaterials,
      components: showComponents,
      tags: showTags,
      scenes: showScenes,
      changes: showChanges,
    })
  }, [showModelInfo, showObjectInfo, showMaterials, showComponents, showTags, showScenes, showChanges])
  useEffect(() => {
    return subscribeTrayLayout((layout) => {
      setShowModelInfo(layout.modelInfo)
      setShowObjectInfo(layout.objectInfo)
      setShowMaterials(layout.materials)
      setShowComponents(layout.components)
      setShowTags(layout.tags)
      setShowScenes(layout.scenes)
      setShowChanges(layout.changes)
    })
  }, [])

  // Keep the ref in sync so side-effect callbacks can read current session state.
  useEffect(() => {
    docSessionRef.current = docSession
  }, [docSession])

  // Pushes the viewport's current camera view into the document (docs/
  // design/camera.md §5) immediately before a `scene.save()` call, so the
  // saved bytes carry "what you were looking at when you last saved" —
  // session view state, NOT undoable (Document::set_camera_state's own
  // doc). A no-op if the viewport hasn't mounted yet (nothing to read).
  const pushCameraStateToScene = useCallback((scene: Scene) => {
    const state = viewportApi.current?.getCameraState()
    if (state === undefined) return
    scene.set_camera_state(
      state.projection,
      state.fovDeg,
      state.parallelHeight,
      new Float64Array(state.eye),
      new Float64Array(state.target),
      new Float64Array(state.up),
    )
  }, [])

  // ---------------------------------------------------------------- autosave
  // Snapshot the scene to the RecoveryStore so a crash or forced quit
  // doesn't lose work. Only writes when the document is dirty AND has
  // mutated since the last successful write (avoids redundant IO on an idle,
  // already-autosaved document) — armed on the first dirty mutation
  // (`armAutosaveTick`) and disarmed the instant there's nothing left to do,
  // rather than polling on a fixed interval for the component's whole
  // lifetime.
  const runAutosaveTick = useCallback((): void => {
    const scene = sceneRef.current
    const session = docSessionRef.current
    if (scene === null || !session.dirty || !dirtySinceAutosaveRef.current) return
    // A plugin command holds the document: snapshot it once the command
    // has ended (or been cancelled), not halfway through.
    if (scene.busy()) {
      armAutosaveTick()
      return
    }
    // `Scene.save()` is transparent to an open explode session (it
    // serializes as-if-closed), so autosave needs no special-casing here.
    // Crash-recovered documents should reopen at the last view too —
    // same posture as an explicit Save (docs/design/camera.md §5).
    pushCameraStateToScene(scene)
    const bytes = new Uint8Array(scene.save())
    const meta: RecoveryMeta = {
      version: 1,
      name: session.currentRef?.name ?? session.importedName ?? 'Untitled',
      path: typeof session.currentRef?.handle === 'string' ? session.currentRef.handle : null,
      format: session.currentRef?.format ?? 'hew',
      savedAt: Date.now(),
    }
    // The `.skp` source rides beside the snapshot, but only when it CHANGED
    // (after an open or a save) — never rewritten per tick (100+ MB). Read
    // here, before the async write, so the generation matches the bytes.
    const sourceGen = skpSourceGenRef.current
    let sourceToWrite: { source: Uint8Array | null } | null = null
    if (sourceGen !== persistedSourceGenRef.current) {
      const source = scene.skp_source() ?? null
      // Nothing stored and nothing to store (a plain .hew document): just
      // record the generation as handled.
      sourceToWrite = source === null && !sourceOnDiskRef.current ? null : { source }
      if (sourceToWrite === null) persistedSourceGenRef.current = sourceGen
    }
    // Keep the in-flight write observable so clearRecoverySnapshot can
    // wait it out — the shell does not order recovery_write against
    // recovery_clear, so a clear overlapping a write can leave the
    // snapshot (or half of it) on disk. The stored promise never rejects.
    // Source first, snapshot second: a crash between the two leaves a stale
    // source beside an older snapshot (still a valid base), never a new
    // snapshot with no source where one is due.
    const write = (async () => {
      if (sourceToWrite !== null) {
        await recoveryStoreRef.current.writeSource(sourceToWrite.source)
        persistedSourceGenRef.current = sourceGen
        sourceOnDiskRef.current = sourceToWrite.source !== null
      }
      await recoveryStoreRef.current.write(bytes, meta)
    })().then(() => {
      // Only mark clean while this write is still the latest — a stale
      // disarm could suppress the re-write that follows a superseding
      // edit-plus-clear interleave.
      if (autosaveWriteRef.current === write) dirtySinceAutosaveRef.current = false
    }).catch(() => {
      // Best effort — re-arm so the next tick retries instead of going
      // permanently silent (the old setInterval retried every 12s
      // regardless; a one-shot timer needs to re-schedule itself here to
      // match).
      armAutosaveTick()
    })
    autosaveWriteRef.current = write
  }, [pushCameraStateToScene, armAutosaveTick])
  useEffect(() => {
    runAutosaveTickRef.current = runAutosaveTick
  }, [runAutosaveTick])
  // Clear any pending autosave timer on unmount — nothing left to arm it
  // again once the component is gone.
  useEffect(() => () => {
    if (autosaveTimerRef.current !== null) {
      clearTimeout(autosaveTimerRef.current)
      autosaveTimerRef.current = null
    }
  }, [])

  // ---------------------------------------------------------------- save-state indicator tick
  // The "Edited/Saved <relative time>" text in TitleBar/MenuBar needs to
  // advance even when nothing else changes (e.g. sitting idle after an edit,
  // "Edited just now" should become "Edited 2 minutes ago"). Used to be a
  // flat 30s `setInterval` for the App's whole lifetime — re-rendering this
  // (large) component every 30s forever, including on a blank document with
  // no label to show at all. Instead, tick only while there's a reference
  // point to report from (`lastEditAt` while dirty, `lastSavedAt` while
  // clean — exactly `saveStateLabel`'s own two cases; docs/design/
  // v1.1-cycle.md Lane F applies the same on-demand principle to this timer
  // as to the render loop), and schedule exactly one `setTimeout` at the
  // next moment the text would actually change (`nextRelativeTimeBoundary`)
  // rather than polling on a fixed cadence. Re-armed whenever the reference
  // point itself changes (a fresh edit, a save); goes idle entirely once the
  // label settles on a fixed date string past 24h.
  useEffect(() => {
    const refMs = docSession.dirty ? docSession.lastEditAt : docSession.lastSavedAt
    if (refMs === null) return
    let timer: ReturnType<typeof setTimeout> | null = null
    const scheduleNext = () => {
      const boundary = nextRelativeTimeBoundary(refMs, Date.now())
      if (boundary === null) return
      timer = setTimeout(() => {
        setNowTick(Date.now())
        scheduleNext()
      }, Math.max(0, boundary - Date.now()))
    }
    scheduleNext()
    return () => {
      if (timer !== null) clearTimeout(timer)
    }
  }, [docSession.dirty, docSession.lastEditAt, docSession.lastSavedAt])

  // (The startup recovery check lives in the startup-handoff effect below —
  // it must run strictly AFTER any pending file-association open or recovery
  // handoff has been consumed, or the dialog races the open and can appear
  // over, then replace, a document the user explicitly double-clicked.)

  // Update document.title whenever session state changes. Under Tauri, also
  // push the title to the shell via `set_window_title` — the in-app MenuBar
  // no longer renders a title for the native case, so the title now lives
  // solely in the OS title bar. `set_window_title` (rather than a bare
  // window.setTitle) also refreshes the shell's open-window list/menu, so
  // the Window-menu entry for this window tracks its document name and
  // dirty mark live.
  useEffect(() => {
    const title = deriveTitle(docSession)
    document.title = title
    if (isTauri) {
      import('@tauri-apps/api/core').then(({ invoke }) => {
        invoke('set_window_title', { title }).catch(() => { /* ignore */ })
      }).catch(() => { /* ignore */ })
    }
  }, [docSession, activeI18n.locale])
  // Tell the shell which file this window has open (`DocumentPaths` in
  // main.rs) so opening the same path again focuses this window instead of
  // minting a second one. A Tauri FileRef handle is the absolute path; a
  // web handle (or an untitled/imported document) reports nothing.
  //
  // The mount-time run (currentRef still null) reports NOTHING: a window
  // minted for a queued open already holds a claim on its path in the
  // shell (`take_pending_window_open` moved it into the registry), and a
  // `null` here would erase that claim in the very window it protects.
  // Only a real transition to/from a path is reported.
  const lastReportedPathRef = useRef<string | null | undefined>(undefined)
  useEffect(() => {
    if (!isTauri) return
    const handle = docSession.currentRef?.handle
    const path = typeof handle === 'string' ? handle : null
    if (lastReportedPathRef.current === undefined && path === null) {
      lastReportedPathRef.current = null
      return
    }
    if (lastReportedPathRef.current === path) return
    lastReportedPathRef.current = path
    import('@tauri-apps/api/core').then(({ invoke }) => {
      invoke('set_document_path', { path }).catch(() => { /* older shell — ignore */ })
    }).catch(() => { /* ignore */ })
  }, [docSession.currentRef])

  // Tell the shell whether this document has unsaved changes — the live
  // API's `hew.meta.documents` `dirty`, which only this window can know.
  useEffect(() => {
    if (!isTauri) return
    import('@tauri-apps/api/core').then(({ invoke }) => {
      invoke('set_document_dirty', { dirty: docSession.dirty }).catch(() => { /* older shell — ignore */ })
    }).catch(() => { /* ignore */ })
  }, [docSession.dirty])

  // Tell the shell this document's persistent id (`document_id`, minted
  // by the first save) — the live API's `hew.meta.documents`
  // `document_id`. A save awaits it once the file has landed, before the
  // save reports success, so a client that saves and then lists documents
  // sees the id; it is also re-read after every session change (open,
  // New, close) and right after a copy or a phone share mints one. Only a
  // change is reported.
  const lastReportedDocumentIdRef = useRef<string | null | undefined>(undefined)
  const reportDocumentId = useCallback(async (): Promise<void> => {
    if (!isTauri) return
    const scene = sceneRef.current
    if (scene === null) return
    const id = scene.document_id() ?? null
    if (lastReportedDocumentIdRef.current === id) return
    lastReportedDocumentIdRef.current = id
    try {
      const { invoke } = await import('@tauri-apps/api/core')
      await invoke('set_document_id', { id })
    } catch { /* older shell — ignore */ }
  }, [])
  useEffect(() => {
    void reportDocumentId()
  }, [docSession, reportDocumentId])

  // Warn before unload when there are unsaved changes. (The "also clear
  // the recovery snapshot on a CLEAN unload" half of this lives further
  // down, right after `clearRecoverySnapshot` is defined — referencing it
  // here would be a TDZ error, since this effect runs before that `const`
  // is reached during render.)
  useEffect(() => {
    const handler = (e: BeforeUnloadEvent) => {
      if (docSession.dirty) {
        e.preventDefault()
        // Legacy support
        e.returnValue = ''
      }
    }
    window.addEventListener('beforeunload', handler)
    return () => window.removeEventListener('beforeunload', handler)
  }, [docSession.dirty])

  const handleStatusChange = useCallback((name: string, kind: string | null) => {
    // LibraryPlaceTool (viewport-level one-shot, armed imperatively via
    // armLibraryPlacement rather than the rail's switchToolRef path) reports
    // its real internal name here on every pointer move — 'LibraryPlace' is
    // not a `ToolName` and would otherwise leak into the status bar
    // literally. The "Inserting <name>…" pill already names what's
    // happening; the tool-name label falls back to Select, matching the
    // rail's own `activeTool` state, which armLibraryPlacement never changes.
    // oxlint-disable-next-line lingui/no-unlocalized-strings -- ToolName identifiers
    setToolName(name === 'LibraryPlace' || name === 'PlaceImage' ? 'Select' : name)
    setSnapKind(kind)
  }, [])

  const handleMeasurement = useCallback((text: string, frozen = false) => {
    setMeasurement(text)
    setMeasurementFrozen(frozen)
  }, [])

  const handleInferenceChange = useCallback((info: InferenceInfo | null) => {
    setInferenceInfo(info)
  }, [])

  const handleSceneChange = useCallback((wtMap: Map<bigint, boolean>) => {
    setWatertightMap(new Map(wtMap))
  }, [])

  const handleSelect = useCallback((node: NodeRef | null, mode: SelectMode) => {
    // Every route to a selection starts without a hit leaf; the viewport
    // click and the crumb click set it again right after this returns.
    setPathHit(null)
    // Node, guide, and annotation selection are mutually exclusive.
    setSelectedGuide(null)
    setSelectedAnnotation(null)
    setSelectedIds((cur) => nextSelection(cur, node, mode))
  }, [])

  /** The viewport's own Select click: the plain selection change, plus the
   *  hit leaf the click's snap named so the strip can show what is below the
   *  selected container. Only a replace-mode click on a node qualifies. */
  const handleViewportSelect = useCallback((node: NodeRef | null, mode: SelectMode) => {
    const leaf = selectSnapLeafRef.current
    selectSnapLeafRef.current = null
    handleSelect(node, mode)
    setPathHit(node !== null && mode === 'replace' && leaf !== null ? { leaf, sel: node } : null)
  }, [handleSelect])

  const handleSelectSnap = useCallback((snap: { object?: bigint; instance?: bigint } | null) => {
    selectSnapLeafRef.current =
      snap !== null && snap.object !== undefined ? { object: snap.object, instance: snap.instance } : null
  }, [])

  /** Lift a multi-node selection (marquee, Select All, Invert Selection)
   * from the viewport. `mode` is the Select tool's modifier matrix
   * (`selectModeFor`): replace / toggle / add / subtract — see
   * `mergeSelection`. */
  const handleSelectMany = useCallback((nodes: NodeRef[], mode: SelectMode) => {
    setSelectedGuide(null)
    setSelectedAnnotation(null)
    setSelectedIds((cur) => mergeSelection(cur, nodes, mode))
  }, [])

  /** The Outliner's row click: its modifier reading is a list's (Shift or
   * ⌘/Ctrl = toggle the row), not the viewport's four-way matrix, so it
   * keeps a boolean and maps onto toggle/replace here. */
  const handleOutlinerSelect = useCallback(
    (node: NodeRef, additive: boolean) => handleSelect(node, additive ? 'toggle' : 'replace'),
    [handleSelect],
  )

  /** Edit ▸ Select None (⇧⌘A / Ctrl+Shift+A): clear every kind of selection
   * — nodes, guide, annotation — without touching the editing context. */
  const handleSelectNone = useCallback(() => {
    setSelectedGuide(null)
    setSelectedAnnotation(null)
    setSelectedIds([])
  }, [])

  /** Replace the selection outright (Object Info's "(N instances)" click). */
  const handleReplaceSelection = useCallback(
    (nodes: NodeRef[]) => handleSelectMany(nodes, 'replace'),
    [handleSelectMany],
  )

  /** Open the Model Health window, or bring it forward if it is already
   *  open. With no `view` an open window keeps its scope and filter; a
   *  closed one (or any call with a `view`) starts from the whole document
   *  unless told otherwise — Object Info's Show link asks for one object. */
  const openModelHealth = useCallback((view?: { scope?: HealthScope; objectFilter?: bigint | null }) => {
    if (view !== undefined || !showModelHealthRef.current) {
      setHealthScope(view?.scope ?? 'document')
      setHealthObjectFilter(view?.objectFilter ?? null)
    }
    setShowModelHealth(true)
    setHealthFocusNonce((n) => n + 1)
  }, [])

  // The viewport draws ONE health overlay. The Model Health panel and the
  // Make Solid wizard both ask for one; while the wizard has something
  // isolated its overlay wins, and when that ends the panel's comes back.
  // This pair of handlers is the only place the arbitration lives.
  const panelOverlayRef = useRef<HealthOverlay | null>(null)
  const wizardOverlayRef = useRef<HealthOverlay | null>(null)
  /** Bumped when the wizard's isolation ends or is replaced from outside it. */
  const [wizardIsolationLost, setWizardIsolationLost] = useState(0)
  const handleHealthOverlay = useCallback((overlay: HealthOverlay | null) => {
    panelOverlayRef.current = overlay
    if (wizardOverlayRef.current === null) viewportApi.current?.setHealthOverlay(overlay)
  }, [])
  const handleWizardOverlay = useCallback((overlay: HealthOverlay | null) => {
    wizardOverlayRef.current = overlay
    viewportApi.current?.setHealthOverlay(overlay ?? panelOverlayRef.current)
  }, [])
  const handleHealthFrame = useCallback(
    (min: [number, number, number], max: [number, number, number]) => {
      viewportApi.current?.zoomToWorldBounds(min, max)
    },
    [],
  )
  /** Isolate the targets (view only) and push the merged hidden sets to the
   *  kernel's snap/pick registration, which the viewport cannot reach. */
  const syncKernelHidden = useCallback(() => {
    const scene = sceneRef.current
    const api = viewportApi.current
    if (scene === null || api === null) return
    const eff = api.getEffectiveHiddenIds()
    setKernelHidden(scene, eff.objects, eff.instances)
  }, [])
  /** Whether the isolation on screen is Model Health's own (a row it
   *  isolated). Its window closing or its scope changing ends only that one,
   *  never the wizard's. */
  const panelOwnsIsolationRef = useRef(false)
  /** The viewport's isolation changed (Escape, a document change, the API). */
  const handleIsolationChange = useCallback(
    (shown: number | null) => {
      setIsolatedCount(shown)
      // Isolation ended (Show everything, Escape, a click on nothing): the
      // wizard's open-edge overlay and its selection go with it.
      if (shown === null) {
        panelOwnsIsolationRef.current = false
        if (wizardOverlayRef.current !== null) handleWizardOverlay(null)
        setWizardIsolationLost((n) => n + 1)
      }
      syncKernelHidden()
    },
    [syncKernelHidden, handleWizardOverlay],
  )
  /** The viewport's isolation targets changed: mirror them into the Scene,
   *  where the bus's `hew.view.hidden` reads them. */
  const handleIsolationTargetsChange = useCallback((targets: IsolationTarget[] | null) => {
    const scene = sceneRef.current
    if (scene === null) return
    scene.set_ui_isolation(uiIsolationJson(targets))
    pumpEvents(scene)
  }, [])
  const handleIsolate = useCallback(
    (targets: IsolationTarget[], opts?: { frame?: boolean }) => {
      panelOwnsIsolationRef.current = false
      viewportApi.current?.isolate(targets, opts)
      syncKernelHidden()
    },
    [syncKernelHidden],
  )
  const handleIsolateRef = useRef(handleIsolate)
  handleIsolateRef.current = handleIsolate
  /** Model Health isolates its own row: the wizard's overlay and selection
   *  give way, and the panel's overlay draws. */
  const handlePanelIsolate = useCallback(
    (targets: IsolationTarget[], opts?: { frame?: boolean }) => {
      if (wizardOverlayRef.current !== null) {
        wizardOverlayRef.current = null
        viewportApi.current?.setHealthOverlay(panelOverlayRef.current)
        setWizardIsolationLost((n) => n + 1)
      }
      handleIsolate(targets, opts)
      panelOwnsIsolationRef.current = true
    },
    [handleIsolate],
  )
  /** Escape belongs to an armed viewport gesture or an open modal dialog
   *  before it closes the Make Solid wizard. */
  const wizardEscapeBusy = useCallback((): boolean => {
    const api = viewportApi.current
    if (api?.hasArmedGesture?.() === true || api?.isCapturingInput?.() === true) return true
    return document.querySelector('[role="dialog"][aria-modal="true"]') !== null
  }, [])
  const handleClearIsolation = useCallback(() => {
    viewportApi.current?.clearIsolation()
    syncKernelHidden()
  }, [syncKernelHidden])
  /** Model Health ends its own isolation only (see panelOwnsIsolationRef). */
  const handlePanelClearIsolation = useCallback(() => {
    if (panelOwnsIsolationRef.current) handleClearIsolation()
  }, [handleClearIsolation])
  // A document change while isolated keeps the targets that still exist and
  // drops the rest (the viewport clears isolation when none are left).
  useEffect(() => {
    const api = viewportApi.current
    if (api === null || (api.getIsolation?.() ?? null) === null) return
    // Nothing resolved differently: skip the kernel's whole-registry rebuild.
    if (api.refreshIsolation() === false) return
    syncKernelHidden()
  }, [docRev, syncKernelHidden])
  const handleHealthSelectNode = useCallback(
    (node: NodeRef) => handleSelectMany([node], 'replace'),
    [handleSelectMany],
  )
  const handleShowObjectHealth = useCallback(
    (id: bigint) => openModelHealth({ scope: 'document', objectFilter: id }),
    [openModelHealth],
  )

  /** Lift a guide pick from the viewport; clears node/annotation selection. */
  const handleSelectGuide = useCallback((id: bigint | null) => {
    setSelectedGuide(id)
    setSelectedAnnotation(null)
    if (id !== null) setSelectedIds([])
  }, [])

  /** Lift an annotation pick from the viewport; clears node/guide selection. */
  const handleSelectAnnotation = useCallback((id: bigint | null) => {
    setSelectedAnnotation(id)
    setSelectedGuide(null)
    if (id !== null) setSelectedIds([])
  }, [])
  // The live bridge (installed once) reaches the selection handlers through
  // refs, like the Outliner eye's `handleToggleHiddenRef`.
  const handleSelectManyRef = useRef(handleSelectMany)
  handleSelectManyRef.current = handleSelectMany
  const handleSelectGuideRef = useRef(handleSelectGuide)
  handleSelectGuideRef.current = handleSelectGuide
  const handleSelectAnnotationRef = useRef(handleSelectAnnotation)
  handleSelectAnnotationRef.current = handleSelectAnnotation
  const handleSelectNoneRef = useRef(handleSelectNone)
  handleSelectNoneRef.current = handleSelectNone

  /** The in-viewport annotation text editor's pending target — either an
   * existing annotation being edited (`id` set, double-click) or a
   * brand-new leader the Text tool just placed (`id` null). `null` = the
   * editor is closed. Viewport.tsx tracks the actual anchor/geometry
   * internally; this only carries what the DOM editor needs to render
   * (docs/design/dimensions-text.md). */
  const [annotationEditor, setAnnotationEditor] = useState<{
    id: bigint | null
    screenX: number
    screenY: number
    initialText: string
    placeholder: string
  } | null>(null)

  const handleOpenAnnotationEditor = useCallback(
    (info: { id: bigint | null; screenX: number; screenY: number; initialText: string; placeholder: string }) => {
      setAnnotationEditor(info)
    },
    [],
  )

  const handleCommitAnnotationEditor = useCallback((text: string) => {
    viewportApi.current?.commitAnnotationEditorText(text)
    setAnnotationEditor(null)
    setDocRev((r) => r + 1)
  }, [])

  const handleCancelAnnotationEditor = useCallback(() => {
    viewportApi.current?.cancelAnnotationEditor()
    setAnnotationEditor(null)
  }, [])

  /** Push an object-context (or K1/K2 instance-fallback) entry — the
   *  viewport's own double-click gesture calls this directly (it already
   *  resolved one container level at a time, and only ever hands this a
   *  plain object or a K1/K2 instance fallback, never a group — groups
   *  route to a session open instead, `Viewport.tsx`'s dblclick handler).
   *  Object pushes are allowed even while a session is open now (design:
   *  an object context sits logically inside the innermost frame) — the
   *  old "no-op while a session is open" guard is gone. The Outliner and
   *  the contextual dock's "enter-context" verb go through `enterNode`
   *  below instead, since THEY can be handed any node kind including one
   *  buried behind session frames that aren't open yet. */
  const handleEnterContext = useCallback((node: NodeRef) => {
    setActiveContext((prev) => [...prev, node])
    setSelectedIds([node])
  }, [])

  const handleExitContext = useCallback(() => {
    setActiveContext((prev) => prev.slice(0, -1))
    // A definition member is selectable only through the instance context
    // that poses it. Never carry that scoped handle back to the parent level,
    // where Delete/transform commands would otherwise retain a route to
    // geometry that is no longer visibly selected.
    setSelectedIds([])
  }, [])

  /** Clear the whole OBJECT-context path (never the session stack) — the
   *  viewport's undo/redo reconcile calls this when a history step changes
   *  which session frames are open while an object context is still
   *  pushed on top (the object context's assumptions about "logically
   *  inside the innermost frame" no longer hold; the kernel's answer
   *  wins). */
  const handleExitAllContexts = useCallback(() => {
    setActiveContext([])
    setSelectedIds([])
  }, [])

  /**
   * The viewport's session stack changed: a frame opened, closed, or the
   * whole stack resynced across an undo/redo boundary
   * (`ViewportApi.sessionStack` crossing a boundary the same way
   * `activeContext` can). Clearing the selection whenever the INNERMOST
   * frame's identity actually changes — not just on close — matters
   * because opening a session on group/instance B while some OTHER node
   * was selected (e.g. from before B's session opened) would otherwise
   * leave that stale selection sitting outside the new session's scope,
   * lighting up commands (Make Unique, Place Copy, boolean's 2-operand
   * gate) for geometry the app is supposed to treat as unreachable while
   * the session is open. A mid-session commit that leaves the innermost
   * frame's IDENTITY unchanged (the common case: drawing/editing inside an
   * open group) must NOT clear the selection on every refresh.
   */
  /** Mirrors `sessionStack` for `handleSessionChange`'s call-time comparison
   *  (finding 4): the callback below reads and writes this ref directly,
   *  as ordinary call-time state, INSTEAD of diffing inside a
   *  `setSessionStack` updater. An updater runs at React's reconciliation
   *  time, not synchronously when `handleSessionChange` is called — so a
   *  `setSelectedIds([])` written inside one used to land on a LATER render
   *  pass, after `enterNode`'s own trailing `setSelectedIds([target])` (a
   *  plain call-time setState in the same synchronous call stack) had
   *  already resolved, silently clobbering it. Doing the diff and every
   *  clear here at call time, before `setSessionStack` even runs, restores
   *  ordinary call-order semantics: whichever `setSelectedIds` call happens
   *  LAST in the synchronous stack wins, so `enterNode`'s final selection
   *  sticks. */
  const prevSessionStackRef = useRef<{ node: NodeRef; label: string }[]>([])
  const handleSessionChange = useCallback(
    (frames: { node: NodeRef; label: string }[]) => {
      const prev = prevSessionStackRef.current
      const prevInnermost = prev.length > 0 ? prev[prev.length - 1].node : null
      const nextInnermost = frames.length > 0 ? frames[frames.length - 1].node : null
      const innermostChanged = prevInnermost === null || nextInnermost === null
        ? prevInnermost !== nextInnermost
        : !nodeEq(prevInnermost, nextInnermost)
      if (innermostChanged) {
        setSelectedIds([])
        setSelectedGuide(null)
        setSelectedAnnotation(null)
      }
      // Tape Measure scoped-rescale invalidation (finding 5,
      // group-session.md): the confirmation dialog captured a specific
      // innermost frame at arm time. Undo/redo (Ctrl/Cmd+Z and the native
      // Edit menu) stay live while it's up, so the stack can change before
      // Confirm — ANY change, not just the innermost frame's identity,
      // since even an outer frame changing means the captured scope no
      // longer names a session state the kernel actually has. Cancel the
      // arm outright rather than let a stale confirm apply to whatever is
      // innermost NOW.
      if (pendingRescaleConfirmRef.current !== null && !sameSessionStackIdentity(prev, frames)) {
        viewportApi.current?.cancelPendingRescale()
        pendingRescaleConfirmRef.current = null
        setPendingRescaleConfirm(null)
      }
      prevSessionStackRef.current = frames
      setSessionStack(frames)
    },
    [],
  )

  /**
   * Entry convergence (docs/design/group-session.md): the ONE shared
   * helper the Outliner's double-click rows and the contextual dock's
   * "enter-context" verb both call, given ANY target node — unlike the
   * viewport's own double-click (which resolves and enters one container
   * level at a time), these two callers can be handed a node buried behind
   * session frames that aren't open yet (a nested object two groups deep,
   * clicked directly in the Outliner). Builds the target's full container
   * chain root→leaf (walking `node_parent`), diffs it against the
   * currently open session stack (the common outermost prefix stays open),
   * closes whatever's open past the divergence innermost-first, opens the
   * remainder outermost-first (groups via `runOpenGroupSession`, an
   * instance via the same fallback-aware open the viewport's own double-
   * click uses), and finishes with an object-context push if the leaf is a
   * plain object. This is what makes the Outliner and the viewport reach
   * IDENTICAL state for the same target — before this helper, the Outliner
   * pushed the bare clicked node with no ancestor walk, producing
   * non-contiguous breadcrumbs like "Model → Object 1" for a nested object
   * with no Group shown.
   */
  const enterNode = useCallback((target: NodeRef, opts?: { keepFallback?: boolean }) => {
    const scene = sceneRef.current
    const api = viewportApi.current
    if (scene === null || api === null) return
    // An armed gesture owns this click, same as Escape's own refusal
    // posture (finding 3, adversarial review): re-homing session state out
    // from under a mid-flight Move/Rotate/PushPull/draw gesture would either
    // fold geometry back while the gesture still holds stale handles, or
    // silently retarget the gesture's eventual commit via the shallower
    // EditContext this would push. Silently ignore, no toast — exactly what
    // Escape does when `toolHasArmedGesture` is true.
    if (api.hasArmedGesture()) return
    if (isSketchScopedKind(target.kind)) {
      // No kernel NodeId / container chain — select-only, same as today.
      setSelectedIds([target])
      return
    }

    // Build the root→leaf container chain by walking node_parent upward,
    // then reversing.
    const ancestors: NodeRef[] = []
    let cur: NodeRef | undefined = target
    while (cur !== undefined) {
      ancestors.push(cur)
      const kindNum = cur.kind === 'group' ? 1 : cur.kind === 'instance' ? 2 : 0
      const parentGroupId = scene.node_parent(kindNum, cur.id)
      cur = parentGroupId !== undefined ? { kind: 'group', id: parentGroupId } : undefined
    }
    ancestors.reverse()

    // An open session frame erases exactly the parent links of its OWN
    // direct members (the ungroup posture / explode bake) — so walking
    // `node_parent` from any target reachable only THROUGH an open session
    // stops short, landing on one of the innermost frame's own live members
    // as the chain's (spurious) root, with every open frame above it
    // missing entirely (finding 1). Recognize that case via
    // `sessionMembers()` (kernel truth for the innermost frame's live direct
    // members) and splice the WHOLE open stack back in as the chain's
    // prefix, rather than let the short walked chain read as "outside every
    // open frame" and have the diff below close the entire stack. A target
    // that is genuinely outside the open stack (its walked root matches none
    // of the innermost frame's members) falls through unchanged — the
    // existing close-all behavior stands for that case.
    const openStack = api.sessionStack()
    const innermostMembers = api.sessionMembers()
    const walkedRoot = ancestors[0]
    const rootIsSessionMember = openStack.length > 0 && innermostMembers !== null &&
      walkedRoot !== undefined && innermostMembers.some((m) => nodeEq(m, walkedRoot))
    const fullChain = rootIsSessionMember ? [...openStack, ...ancestors] : ancestors

    // Split into the SESSIONABLE prefix (group/instance frames) and a
    // trailing plain-object leaf, if any.
    const leaf = fullChain[fullChain.length - 1]
    const objectLeaf = leaf?.kind === 'object' ? leaf : null
    const framesChain = objectLeaf !== null ? fullChain.slice(0, -1) : fullChain

    // Diff against the open stack — common outermost prefix stays open.
    let common = 0
    while (
      common < openStack.length && common < framesChain.length &&
      nodeEq(openStack[common], framesChain[common])
    ) common++

    // A later step of a bus path keeps the fallback entries the earlier
    // steps pushed: the path's instances that could not take a session stay
    // in the chain, so the window ends inside the leaf with them beneath it.
    // Only when the frames above them stay open, though; closing a session
    // drops the context tail that sat inside it.
    if (opts?.keepFallback !== true || openStack.length > common) setActiveContext([])
    for (let i = openStack.length; i > common; i--) {
      api.runCloseInnermostSession()
    }
    for (let i = common; i < framesChain.length; i++) {
      const frame = framesChain[i]
      if (frame.kind === 'group') {
        api.runOpenGroupSession(frame.id)
      } else if (frame.kind === 'instance') {
        // Fallback-aware: silently pushes the instance as a K1/K2 context
        // via the SAME `onEnterContext` callback wiring the viewport's own
        // double-click uses, on the two refusals the design treats as "no
        // session for this instance" — identical behavior reaching a
        // component frame through the Outliner/dock instead of a direct
        // viewport double-click.
        api.runOpenExplodeSessionOrFallback(frame.id, frame)
      }
    }
    if (objectLeaf !== null) {
      setActiveContext((prev) => [...prev, objectLeaf])
    }
    setSelectedIds([target])
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  /** Breadcrumb crumb click at combined depth `depth` (0 = outermost
   *  session frame … sessionStack.length + activeContext.length - 1 = the
   *  deepest object-context entry): close/pop everything past it. A crumb
   *  inside the session-stack region closes frames innermost-first (and
   *  drops any object-context tail, which sits inside whatever frame is
   *  closing); a crumb inside the object-context region just truncates
   *  `activeContext`. */
  const handleSetPathDepth = useCallback((depth: number) => {
    const api = viewportApi.current
    // An armed gesture owns the click — same refusal posture as Escape and
    // `enterNode` above (finding 3): a crumb click bypassed this guard
    // entirely before, so closing a session (or popping an object context)
    // mid-gesture could re-home geometry the gesture still holds stale
    // handles into. Silently ignore, no toast.
    if (api?.hasArmedGesture()) return
    // The synchronous ref, not the render copy: bus frames can land before
    // the render that follows a session change.
    const stackLen = prevSessionStackRef.current.length
    if (depth < stackLen) {
      setActiveContext([])
      setSelectedIds([])
      for (let i = stackLen; i > depth; i--) api?.runCloseInnermostSession()
    } else {
      setActiveContext((prev) => prev.slice(0, depth - stackLen))
      setSelectedIds([])
    }
  }, [])

  // The live bridge (installed once) reaches the entry convergence and the
  // breadcrumb truncation through refs, like the selection handlers.
  const enterNodeRef = useRef(enterNode)
  enterNodeRef.current = enterNode
  const setPathDepthRef = useRef(handleSetPathDepth)
  setPathDepthRef.current = handleSetPathDepth

  /** The root "Model" breadcrumb crumb: exit everything — every open
   *  session frame, innermost-first, plus the object-context tail. */
  const handleExitToModel = useCallback(() => {
    const api = viewportApi.current
    // Same armed-gesture refusal as `handleSetPathDepth`/`enterNode` above
    // (finding 3) — the Model crumb is a close path too.
    if (api?.hasArmedGesture()) return
    setActiveContext([])
    setSelectedIds([])
    for (let i = prevSessionStackRef.current.length; i > 0; i--) api?.runCloseInnermostSession()
  }, [])

  // ---------------------------------------------------------------- scenes
  // The Scenes controller (docs/design/scenes.md §5): entries, active Scene,
  // drift, thumbnails, and every Add/Update/Activate/... against the kernel
  // + viewport. The tray section, View ▸ Scenes menu, palette, and Page
  // Up/Down all drive this one object. Deps arrive as callbacks so the
  // controller never reaches into this component's state directly.
  const scenesDisplay = useMemo(() => ({ grid: showGrid, axes: showAxes, guides: showGuides }), [showGrid, showAxes, showGuides])
  const scenesSetDisplay = useCallback((d: { grid: boolean; axes: boolean; guides: boolean }) => {
    setShowGrid(d.grid)
    setShowAxes(d.axes)
    setShowGuides(d.guides)
  }, [])
  const scenesSetHiddenState = useCallback((next: { hiddenKeys?: Set<string>; hiddenTagPaths?: Set<string> }) => {
    if (next.hiddenKeys !== undefined) setHiddenKeys(next.hiddenKeys)
    if (next.hiddenTagPaths !== undefined) setHiddenTagPaths(next.hiddenTagPaths)
  }, [])
  const scenesMarkDirty = useCallback(() => {
    // Scenes state lives outside the kernel's undo stack (docs/design/
    // scenes.md) — `at_saved_mark()` can't see a Scene add/update/rename,
    // so this is a non-undoable-dirty reason, not a plain `afterMutation`.
    setDocSession((s) => markNonUndoableDirty(s, NON_UNDOABLE_REASON.scenesEdited, Date.now()))
    armAutosaveTick()
    // A Scene edit records no history entry; `scenes_changed` is how a
    // subscriber hears of it.
    if (sceneRef.current !== null) pumpEvents(sceneRef.current)
  }, [armAutosaveTick])
  const scenesToast = useCallback((message: string) => handleToastRef.current?.(message), [])
  // A Scene's view style lands in the viewport the way a loaded
  // document's does (the kernel already made it the document's own).
  const scenesSetViewStyle = useCallback((style: ViewStyleJson) => {
    viewportApi.current?.setViewStyle(style)
    setShowAxes(style.axes)
  }, [])
  const scenes = useScenesController({
    scene: state?.scene ?? null,
    viewportApi,
    docRev,
    display: scenesDisplay,
    setDisplay: scenesSetDisplay,
    setViewStyle: scenesSetViewStyle,
    setHiddenState: scenesSetHiddenState,
    exitAllContexts: handleExitToModel,
    markDirty: scenesMarkDirty,
    onToast: scenesToast,
  })
  const scenesRef = useRef(scenes)
  scenesRef.current = scenes
  // Inline-rename UI state shared between the Scenes tray section's header
  // ⊕ Add Scene button and its row list (ScenesPanel.tsx's own doc comment
  // on why this can't just be local state in one component).
  const scenesRename = useSceneRenameState(scenes)
  /** Add Scene from a menu: reveal the Scenes section first, so the new
   *  row's rename field is mounted when the hook focuses it. */
  const addSceneFromMenu = () => {
    showTray()
    setShowScenes(true)
    scenesRename.add()
  }

  /** Validate and trim the context path when the document changes. */
  const trimContextPath = useCallback((scene: Scene, path: NodeRef[]): NodeRef[] => {
    const objectIds = new Set(Array.from(scene.object_ids()))
    const groupIds = new Set(Array.from(scene.group_ids()))
    const instanceIds = new Set(Array.from(scene.instance_ids()))

    let trimIndex = path.length
    for (let i = 0; i < path.length; i++) {
      const node = path[i]
      let alive: boolean
      if (node.kind === 'object') {
        alive = objectIds.has(node.id)
      } else if (node.kind === 'instance') {
        alive = instanceIds.has(node.id)
      } else {
        alive = groupIds.has(node.id)
      }
      if (!alive) {
        trimIndex = i
        break
      }
    }
    return path.slice(0, trimIndex)
  }, [])

  // Bump the tree's revision; trim stale context path entries; mark dirty.
  // Every scene mutation flows through here via Viewport's handleSceneRefresh.
  // (The first-sketch auto-zoom that used to fire here was removed by
  // maintainer decision: the welcome screen's unit choice already sets a
  // sensible initial framing, and yanking the camera on the first draw was
  // more irritating than helpful. Zoom Extents still covers sketches.)
  const handleDocumentChanged = useCallback(() => {
    setDocRev((r) => r + 1)
    setActiveContext((ctx) => {
      const scene = sceneRef.current
      if (scene === null) return ctx
      return trimContextPath(scene, ctx)
    })
    // Prune dead handles out of the selection. Undo/redo is the headline
    // case (the maintainer's repro: undo an array copy and Object Info kept
    // saying "3 selected" while the dock sat in Multi mode), but every
    // mutation that can kill nodes funnels through here — deletes, booleans
    // consuming operands, extrusions consuming sketches — so they all
    // reconcile at this one choke point. pruneDeadSelection returns the
    // same array when nothing died, so this setState is a no-op then.
    setSelectedIds((sel) => {
      const scene = sceneRef.current
      if (scene === null) return sel
      const deepest = activeContext.length > 0
        ? activeContext[activeContext.length - 1]
        : undefined
      if (deepest?.kind !== 'instance') return pruneDeadSelection(scene, sel)
      const component = scene.instance_def(deepest.id)
      if (component === undefined) return pruneDeadSelection(scene, sel)
      return pruneDeadSelection(
        scene,
        sel,
        Array.from(scene.component_member_objects(component)),
        Array.from(scene.component_member_sketches(component)),
      )
    })
    // Recompute dirty on any mutation — but NOT during programmatic loads
    // (suppressDirtyRef is true while applyLoadedBytes calls notifyLoaded).
    // `atSavedMark` is the kernel's own answer (Lane C): this is the single
    // choke point every commit, undo, AND redo reconciles through (Viewport's
    // handleSceneRefresh doc comment), so undoing back to the saved depth
    // lands here with `atSavedMark: true` and cleans the document right back
    // up — no separate "undo to clean" plumbing needed.
    if (!suppressDirtyRef.current) {
      const atSavedMark = sceneRef.current?.at_saved_mark() ?? true
      setDocSession((s) => afterMutation(s, Date.now(), atSavedMark))
      armAutosaveTick()
    }
    // Every commit, undo, and redo reconciles here: subscribers hear of it
    // (docs/agents/HEW_API.md §4.5).
    if (sceneRef.current !== null) pumpEvents(sceneRef.current)
  }, [activeContext, trimContextPath, armAutosaveTick])

  /** Re-tessellate + re-render after a kernel mutation committed OUTSIDE a
   *  tool — a tray panel calling a `Scene` method directly (material/
   *  component delete, purge) rather than routing through a dedicated
   *  Viewport method the way `deleteSelection`'s `runDelete` does. Mirrors
   *  the harness's own `act()` dispatch (`app/src/test/harness.ts`):
   *  `refreshScene()` already calls `handleDocumentChanged` internally
   *  (`Viewport.tsx`'s `handleSceneRefresh`), so call ONLY that when a
   *  viewport is mounted — falling back to `handleDocumentChanged()`
   *  directly (no re-tessellation possible) when it isn't, matching every
   *  App.test.tsx render (Viewport is mocked to `null` there; App guards
   *  every `viewportApi` call with optional chaining) so docRev/dirty/
   *  selection bookkeeping still happens under test. Calling both would
   *  double-fire `handleDocumentChanged` — harmless (idempotent) but not
   *  the point of this helper. */
  const refreshAfterPanelMutation = useCallback(() => {
    const api = viewportApi.current
    if (api !== null) api.refreshScene()
    else handleDocumentChanged()
  }, [handleDocumentChanged])

  /** After a rename from a panel: nothing drawn changes, so no
   *  re-tessellation — but an open session's "Editing …" breadcrumb shows a
   *  component's live name, so re-derive the session labels too. */
  const handleNameChanged = useCallback(() => {
    handleDocumentChanged()
    const api = viewportApi.current
    if (api !== null && api.sessionStack().length > 0) api.refreshSessionScope()
  }, [handleDocumentChanged])

  // Re-derive the View ▸ Section Cut menu state from the section
  // manager's own truth (`getSectionState`) — called by the viewport
  // whenever a section is placed/offset-committed/toggled/deleted, or a
  // fresh document clears it (Viewport's `onSectionChanged`). Deliberately
  // NOT a toggle: this always re-READS live state rather than flipping a
  // local boolean, so it can never drift from what the session actually
  // holds (D3, section-plane-polish).
  const handleSectionChanged = useCallback(() => {
    const state = viewportApi.current?.getSectionState() ?? null
    setSectionPlaneMenuState({ checked: state !== null && state.active, exists: state !== null })
    // The kernel owns the persisted plane (docs/design/scenes.md §4); the
    // viewport's SectionManager is its mirror. Every committed change lands
    // here, so this is the one write-through point. View state — never
    // dirties, like the camera.
    const scene = sceneRef.current
    if (scene !== null) {
      try {
        if (state === null) scene.clear_section_plane()
        else scene.set_section_plane(state.origin[0], state.origin[1], state.origin[2], state.normal[0], state.normal[1], state.normal[2], state.active)
      } catch {
        /* a non-unit normal from a degenerate placement — the viewport keeps its own copy; nothing persists */
      }
    }
    scenesRef.current.refreshDrift()
  }, [])

  // Semantic test harness `window.__hew_test`, installed only in
  // debug/test builds (dev server, or a build with VITE_HEW_TEST=1 for the
  // Playwright E2E build). Live values are read through refs so the harness
  // object is installed once and never goes stale.
  const reconcileRef = useRef(handleDocumentChanged)
  reconcileRef.current = handleDocumentChanged
  const selectedIdsRef = useRef(selectedIds)
  selectedIdsRef.current = selectedIds
  // Read live for the harness's toggleNodeHidden/isNodeHidden (Lane D:
  // outliner.spec.ts) — handleToggleHidden is defined further down (it
  // depends on later state), so it's reached through a ref like
  // applyLoadedBytesRef below; hiddenKeys is simple state, kept current the
  // same way selectedIdsRef is.
  const hiddenKeysRef = useRef(hiddenKeys)
  hiddenKeysRef.current = hiddenKeys
  // The hidden tag paths, read the same way. Both refs are also written the
  // moment the hidden state is re-read from the document
  // (`reseedHiddenStateFromScene`), so a reconcile in the same tick — a bus
  // hide's re-seed then reconcile — works from the fresh sets, never the
  // last render's.
  const hiddenTagPathsRef = useRef(hiddenTagPaths)
  hiddenTagPathsRef.current = hiddenTagPaths
  const handleToggleHiddenRef = useRef<(node: NodeRef) => void>(() => {})
  // applyLoadedBytes is defined further down (it depends on later state setters);
  // the harness installs once on mount, so reach it through a ref (kept current
  // below, beside that definition) like reconcileRef.
  const applyLoadedBytesRef = useRef<((bytes: Uint8Array, file?: LoadedFile) => boolean) | null>(null)
  // The desktop half of `--live` (docs/agents/HEW_API.md §11.2) and the test
  // harness's `apiDispatch` apply a dispatch through the same deps.
  const liveOpRef = useRef<LiveOpRunner | null>(null)
  // `pushCameraStateToScene` is defined further up the component; the
  // bridge (installed once) reaches it through this ref.
  const pushCameraStateRef = useRef<(scene: Scene) => void>(() => {})
  const liveBridgeDepsRef = useRef<LiveBridgeDeps | null>(null)
  liveBridgeDepsRef.current ??= {
    pushCameraState: (scene) => pushCameraStateRef.current(scene),
    runOp: (op, args) =>
      liveOpRef.current !== null
        ? liveOpRef.current(op, args)
        // oxlint-disable-next-line lingui/no-unlocalized-strings -- live-API refusal text, English (developer surface)
        : Promise.resolve({ ok: false, refusal: 'window_not_ready', message: 'the window is still starting' }),
    getScene: () => sceneRef.current,
    getViewportApi: () => viewportApi.current,
    reconcile: () => reconcileRef.current(),
    // A live `hew.scenes.apply` drives the SAME activation path the Scenes
    // tray uses (docs/design/scenes.md §7): the kernel state is already
    // written by the time the directive arrives; this syncs panels, the
    // renderer, and the camera exactly as a row click would.
    activateScene: (sid) => scenesRef.current.activate(sid),
    reconcileVisibility: () => reconcileVisibilityRef.current(),
    // A live `hew.library.insert` of a material item: dirty for the same
    // reason as the Library's own "Add to palette".
    markLibraryMaterialsDirty: () =>
      setDocSession((s) => markNonUndoableDirty(s, NON_UNDOABLE_REASON.libraryMaterials, Date.now())),
    reseedViewState: () => {
      const scene = sceneRef.current
      if (scene !== null) reseedHiddenStateRef.current(scene)
    },
    // A bus `hew.selection.set`/`clear`: the merged selection, applied
    // through the app's own selection handlers.
    selectMany: (nodes, mode) => handleSelectManyRef.current(nodes, mode),
    selectGuide: (id) => handleSelectGuideRef.current(id),
    selectAnnotation: (id) => handleSelectAnnotationRef.current(id),
    selectNone: () => handleSelectNoneRef.current(),
    // A bus isolation is nobody's panel's: Model Health closing must not
    // end it (`handleIsolate` hands ownership back to the app).
    isolate: (targets, opts) => handleIsolateRef.current(targets, opts),
    // A bus `hew.view.enter`/`exit`/`exit_all` drives the Outliner's own
    // entry convergence and the breadcrumb's own truncation.
    enterNode: (node, opts) => enterNodeRef.current(node, opts),
    setPathDepth: (depth) => setPathDepthRef.current(depth),
    contextDepth: () => prevSessionStackRef.current.length + activeContextRef.current.length,
  }
  useEffect(() => {
    if (!(import.meta.env.DEV || import.meta.env.VITE_HEW_TEST === '1')) return
    return installTestHarness({
      liveBridge: liveBridgeDepsRef.current ?? undefined,
      getScene: () => sceneRef.current,
      getViewportApi: () => viewportApi.current,
      isolate: (targets) => {
        panelOwnsIsolationRef.current = false
        const n = viewportApi.current?.isolate(targets) ?? 0
        syncKernelHidden()
        return n
      },
      clearIsolation: () => handleClearIsolation(),
      reconcile: () => reconcileRef.current(),
      getSelection: () => selectedIdsRef.current,
      setSelectedObjects: (ids) =>
        setSelectedIds(ids.map((id) => ({ kind: 'object' as const, id }))),
      setSelection: (nodes) => {
        setSelectedGuide(null)
        setSelectedAnnotation(null)
        setSelectedIds(nodes)
      },
      loadBytes: (bytes) => applyLoadedBytesRef.current?.(bytes) ?? false,
      toggleTagPath: (path) => toggleTagPathRef.current(path),
      deleteTag: (path) => deleteTagRef.current(path),
      toggleNodeHidden: (node) => handleToggleHiddenRef.current(node),
      isNodeHidden: (node) => hiddenKeysRef.current.has(nodeKey(node)),
      reconcileVisibility: () => reconcileVisibilityRef.current(),
      setPrintRecorder: (r) => {
        printRecorderRef.current = r
      },
      reseedViewState: () => {
        const scene = sceneRef.current
        if (scene !== null) reseedHiddenStateRef.current(scene)
      },
    })
  }, [])

  // The desktop half of `--live` (docs/agents/HEW_API.md §11.2): wires the Tauri
  // shell's local-socket JSON-RPC frames to this document's own live
  // Scene, exactly like installTestHarness above — a no-op outside Tauri
  // (isTauri gate lives inside installLiveBridge itself).
  useEffect(() => {
    pushCameraStateRef.current = pushCameraStateToScene
  }, [pushCameraStateToScene])
  useEffect(() => {
    const deps = liveBridgeDepsRef.current
    if (deps === null) return
    return installLiveBridge(deps)
  }, [])

  // The bus reads the selection and the active tool from the Scene
  // (docs/agents/HEW_API.md §7, "Selection and view state"): mirror both
  // whenever they change, and pump, so `selection_changed` and
  // `tool_changed` reach subscribers.
  useEffect(() => {
    const scene = state?.scene
    if (scene === undefined) return
    scene.set_ui_selection(uiSelectionJson(selectedIds, selectedGuide, selectedAnnotation))
    pumpEvents(scene)
  }, [state?.scene, selectedIds, selectedGuide, selectedAnnotation])
  // The in-context entries: `setActiveContext` already mirrors every change
  // into the Scene; this re-seeds a replaced Scene and pumps, so
  // `context_changed` reaches subscribers when the user's own clicks (not a
  // bus entry, which the bridge pumps) moved the entries.
  useEffect(() => {
    const scene = state?.scene
    if (scene === undefined) return
    // From the ref: a bus entry that landed after this render committed
    // must not be overwritten by the render copy.
    scene.set_ui_context(uiContextJson(activeContextRef.current))
    pumpEvents(scene)
  }, [state?.scene, activeContext])
  useEffect(() => {
    const scene = state?.scene
    if (scene === undefined) return
    scene.note_tool_changed(activeTool)
    pumpEvents(scene)
  }, [state?.scene, activeTool])

  const handleToast = useCallback((message: string, code?: string) => {
    // Severity lives beside the copy table in kernelErrors.ts (one source; a
    // new code gets its level where it gets its message).
    const isError = code !== undefined && isErrorLevelCode(code)
    const level = isError ? 'error' : 'warn'
    const logMessage = code !== undefined ? `[${code}] ${message}` : message
    LogStore.log[level]('tool', logMessage)

    if (isPanicError(message)) {
      setKernelPanicked(true)
    }

    const id = ++toastCounter
    const emphasis = code === EDIT_CONTEXT_HINT_CODE
    setToasts((prev) => [...prev, { id, message, code, isError, emphasis }])
    setTimeout(() => {
      setToasts((prev) => prev.filter((t) => t.id !== id))
    }, emphasis ? 7000 : 4000)
  }, [])
  const handleToastRef = useRef(handleToast)
  handleToastRef.current = handleToast

  const dismissToast = useCallback((id: number) => {
    setToasts((prev) => prev.filter((t) => t.id !== id))
  }, [])

  // A sticky, updatable toast — the Library save flow's "Rendering
  // thumbnail…" → "Saved" progression. Bypasses handleToast's LogStore/
  // isPanicError/auto-dismiss machinery (none of that applies to a status
  // toast with no kernel error code) and starts with no auto-dismiss timer
  // at all; updateToast arms one once the caller flips `sticky` back off.
  const pushStickyToast = useCallback((message: string): number => {
    const id = ++toastCounter
    setToasts((prev) => [...prev, { id, message, isError: false, state: 'pending', sticky: true }])
    return id
  }, [])

  const updateToast = useCallback((id: number, patch: Partial<Toast>) => {
    setToasts((prev) => prev.map((t) => (t.id === id ? { ...t, ...patch } : t)))
    if (patch.sticky === false) {
      setTimeout(() => {
        setToasts((prev) => prev.filter((t) => t.id !== id))
      }, 2400)
    }
  }, [])

  const handleRescaleArmed = useCallback((info: RescaleConfirmInfo) => {
    // Tape Measure scoped rescale (docs/design/group-session.md): with a
    // session frame open, the confirmation names the innermost frame and
    // (on Confirm) resizes just its contents in place, rather than refusing
    // outright — the design's replacement for the old blanket
    // `ExplodeSessionScope` decline this block used to show. The decision
    // is made HERE, once, and threaded through `pendingRescaleConfirm` so
    // the dialog and the eventual commit read the same answer.
    const innermost = sessionStack[sessionStack.length - 1]
    const scope = innermost === undefined
      ? null
      : { label: innermost.label, isComponent: innermost.node.kind === 'instance', node: innermost.node }
    setPendingRescaleConfirm({ info, scope })
  }, [sessionStack])

  // Report Bug — assemble the bundle and tell the user what happened.
  // The write is otherwise silent (Tauri saves a file, web downloads), so
  // without a toast it looks like the menu item does nothing.
  // Help ▸ Hew Help — the online user guide. A Tauri webview swallows a
  // blank-target navigation, so on desktop the URL goes to the OS default
  // browser via the opener plugin (the WelcomeScreen guide-link pattern);
  // the web build just opens a tab.
  const handleOpenGuide = useCallback(() => {
    if (isTauri) {
      void import('@tauri-apps/plugin-opener')
        .then(({ openUrl }) => openUrl(USER_GUIDE_URL))
        .catch(() => {
          /* best effort — nothing useful to do if the OS can't open the URL */
        })
    } else {
      window.open(USER_GUIDE_URL, '_blank', 'noopener')
    }
  }, [])

  // Help ▸ Report Bug… / palette / native Help menu all just open the
  // dialog (docs/design/report-bug.md §2) — it reads sceneRef itself at
  // build time via the scene prop below, so there's nothing else to do here.
  const [reportBugOpen, setReportBugOpen] = useState(false)
  const handleReportBug = useCallback(() => {
    setReportBugOpen(true)
  }, [])

  // Help ▸ Sent Reports… (docs/design/follow-a-report.md §2.2). `id` selects a
  // report first (the notice bar's View with exactly one report).
  const [sentReports, setSentReports] = useState<{ id: string | null } | null>(null)
  const handleSentReports = useCallback((id: string | null = null) => {
    setSentReports({ id })
  }, [])
  const followedReports = useFollowedReports()
  const sentReportsNew = totalUnread(followedReports)
  // The web build checks followed reports once per launch (the desktop shell
  // does its own); a no-op when this build has nothing to follow.
  useEffect(() => {
    if (isTauri) return
    return startWebLaunchCheck(getFollowHost())
  }, [])

  // Whether this desktop build carries the auto-updater (package-manager
  // builds compile it out — see the shell's `updater` feature). The in-app
  // menu bar (Windows/Linux) shows "Check for Updates" only when true; macOS
  // reaches it through the native menu instead. Stays false on the web build,
  // where the invoke simply never resolves.
  const [updaterAvailable, setUpdaterAvailable] = useState(false)
  useEffect(() => {
    if (!isTauri) return
    let cancelled = false
    import('@tauri-apps/api/core')
      .then(({ invoke }) => invoke<boolean>('updater_available'))
      .then((ok) => { if (!cancelled) setUpdaterAvailable(ok) })
      .catch(() => { /* command absent (updater compiled out) — leave hidden */ })
    return () => { cancelled = true }
  }, [])

  // Manual "Check for Updates" — hands off to the shell, which runs the whole
  // flow (check → native confirm → download → restart prompt).
  const handleCheckForUpdates = useCallback(() => {
    import('@tauri-apps/api/core')
      .then(({ invoke }) => invoke('check_for_updates'))
      .catch(() => { /* ignore */ })
  }, [])

  // View ▸ Shop Mode: the mirror of ShopApp's own "Use full editor" — pin
  // the override and reload so `main.tsx`'s `resolveShellMode` picks it up
  // on the next boot. Never called from the desktop shell or a fine-pointer
  // session (MenuBars's `onEnterShopMode` prop is only wired up when neither
  // holds — see the render below), so this doesn't need to re-check either.
  const handleEnterShopMode = useCallback(() => {
    writeShellModeOverride('shop')
    window.location.reload()
  }, [])

  useEffect(() => {
    const unsub = LogStore.subscribe((entries) => {
      const latest = entries[entries.length - 1]
      if (latest === undefined) return
      if (latest.source === 'console' && latest.level === 'error') {
        if (isPanicError(latest.message)) {
          setKernelPanicked(true)
        }
      }
    })
    return unsub
  }, [])

  // The innermost open session frame's current direct-member list (any node
  // kind) — `ViewportApi.sessionMembers()`, re-read whenever the document or
  // the session stack itself changes (mid-session fold-ins/deletes shift
  // membership without changing the session stack's own identity). `null`
  // when no session is open. Threaded to the Outliner (DocumentTree) so it
  // can nest the innermost frame's members under its synthetic header row.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  const sessionMembers = useMemo(
    () => viewportApi.current?.sessionMembers?.() ?? null,
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [docRev, sessionStack],
  )

  // The hit leaf outlives only the selection it produced: any other route to
  // a different selection (Outliner, Object Info, palette, marquee, script)
  // drops it so the strip never shows crumbs from a stale click.
  useEffect(() => {
    if (pathHit === null) return
    if (selectedIds.length === 1 && nodeEq(selectedIds[0], pathHit.sel)) return
    setPathHit(null)
  }, [selectedIds, pathHit])

  // The Outliner's positional-label index (per parent container), built once
  // per document revision and shared by the palette entries and the path.
  const sharedTreeIndex = useMemo(() => {
    const scene = state?.scene
    if (scene == null) return null
    return buildTreeIndexMap(
      scene.top_level_nodes().map(nodeRefFromJs),
      (groupId) => scene.group_members(groupId).map(nodeRefFromJs),
    )
    // docRev: membership changes on every mutation without changing identity.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [state?.scene, docRev])

  // Selection path (docs/design/selection-path.md): one label resolver shared
  // with the Outliner, Object Info, and the hover tip line. The positional
  // fallback index is the Outliner's (per parent container), recomputed per
  // document revision.
  const pathLabels = useMemo(() => {
    const scene = state?.scene
    if (scene == null || sharedTreeIndex === null) return null
    const frameLabels = new Map(sessionStack.map((f) => [nodeKey(f.node), f.label]))
    // The Outliner's drawn-unit numbering, read once and only if a drawn or
    // free-sketch label is asked for.
    let drawing: OutlinerDrawing | undefined
    const drawingOnce = () =>
      (drawing ??= outlinerDrawing(scene as unknown as Parameters<typeof outlinerDrawing>[0], Array.from(scene.object_ids())))
    const labelFor = makeLabelFor(scene, sharedTreeIndex, frameLabels, drawingOnce)
    const parentOf = (n: NodeRef): NodeRef | undefined => {
      const kindNum = nodeKindToNumber(n.kind)
      if (kindNum < 0) return undefined
      const p = scene.node_parent(kindNum, n.id)
      return p === undefined ? undefined : { kind: 'group' as const, id: p }
    }
    const memberName = (objectId: bigint): string | undefined => {
      const name = scene.object_name(objectId)
      const stripped = name === undefined ? '' : stripTagSuffix(name)
      return stripped === '' ? undefined : stripped
    }
    return { scene, labelFor, parentOf, memberName }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [state?.scene, sharedTreeIndex, sessionStack])

  const pathCrumbs = useMemo(() => {
    if (pathLabels === null) return []
    const { scene, labelFor, parentOf, memberName } = pathLabels
    // The object or free sketch a drawn selection lives on.
    const draftOwner = (ref: NodeRef): NodeRef | null => {
      const r = resolveDraft(scene as unknown as PlaneUnitsScene, ref)
      if (r === null) return null
      let sketch: bigint | null = null
      for (const e of r.edges) {
        if (e.store.kind === 'free') sketch = sketch ?? e.store.sketch
        else return { kind: 'object', id: e.store.object }
      }
      return sketch !== null ? { kind: 'sketch', id: sketch } : null
    }
    return buildSelectionPath({
      sessionStack,
      activeContext,
      selected: selectedIds,
      hitLeaf: pathHit !== null && selectedIds.length === 1 && nodeEq(selectedIds[0], pathHit.sel) ? pathHit.leaf : null,
      parentOf,
      sessionMembers,
      labelFor,
      memberName,
      draftOwner,
    })
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [pathLabels, sessionStack, activeContext, selectedIds, pathHit, sessionMembers])

  /** The hover tip's second line for the object under the cursor (Select tool
   *  only — the viewport forwards `object` solely then). */
  const hoverDetail = useMemo(() => {
    if (pathLabels === null || inferenceInfo === null || inferenceInfo.object === undefined) return null
    // An open session frame erases its direct members' parent links, so a
    // member under the cursor names the innermost frame as its parent.
    const frame = sessionStack.length > 0 ? sessionStack[sessionStack.length - 1].node : null
    const parentOf = (n: NodeRef): NodeRef | undefined =>
      pathLabels.parentOf(n) ??
      (frame !== null && sessionMembers?.some((m) => nodeEq(m, n)) === true ? frame : undefined)
    // An instance that is an open frame (or in the object context) shows its
    // parts as ordinary objects, not as closed-definition members.
    const isOpenInstance = (id: bigint): boolean =>
      sessionStack.some((f) => f.node.kind === 'instance' && f.node.id === id) ||
      activeContext.some((n) => n.kind === 'instance' && n.id === id)
    return hoverLine(
      { object: inferenceInfo.object, instance: inferenceInfo.instance },
      parentOf, pathLabels.labelFor, pathLabels.memberName, isOpenInstance,
    )
  }, [pathLabels, inferenceInfo?.object, inferenceInfo?.instance, sessionStack, activeContext, sessionMembers])

  /** A strip crumb click: the selection becomes that node, nothing else
   *  changes. The hit leaf stays paired with it so the crumbs below survive. */
  const handlePathSelect = useCallback((node: NodeRef) => {
    const h = pathHit
    handleSelect(node, 'replace')
    setPathHit(h === null ? null : { leaf: h.leaf, sel: node })
  }, [handleSelect, pathHit])

  /** `Model` crumb: with anything open, exit everything; otherwise clear. */
  const handlePathModel = useCallback(() => {
    if (sessionStack.length > 0 || activeContext.length > 0) handleExitToModel()
    else handleSelectNone()
  }, [sessionStack, activeContext, handleExitToModel, handleSelectNone])

  // Compute the lit set for isolation.
  // When the context is non-empty, compute the leaf objects of the deepest context node.
  // When entering an object (deepest is an object), lit = {that object}.
  // When entering an instance (deepest is an instance), lit = member objects of the def.
  // NOTE: this is a hook, so it must run on every render — keep it above the
  // early returns below (Rules of Hooks). It guards `state === null` itself.
  const activeLitSet: Set<bigint> | null = useMemo(() => {
    if (state === null || activeContext.length === 0) return null
    const deepest = activeContext[activeContext.length - 1]
    if (deepest.kind === 'instance') {
      // Light all definition member objects when inside a component
      const componentId = state.scene.instance_def(deepest.id)
      if (componentId === undefined) return null
      const members = Array.from(state.scene.component_member_objects(componentId))
      return new Set(members)
    }
    const kind = deepest.kind === 'group' ? 1 : 0
    const leaves = Array.from(state.scene.node_leaf_objects(kind, deepest.id))
    return new Set(leaves)
  }, [activeContext, state])

  // Selection-dependent command availability — one derivation shared by the
  // render-body handlers, the native Edit menu (sync_menu_state), and the
  // in-app MenuBar, so every surface agrees on what's currently possible.
  // NOTE: hook — must stay above the early returns (Rules of Hooks).
  const menuGates = useMemo(() => {
    const scene = state?.scene
    if (scene == null) return null
    const objectIdSet = new Set(Array.from(scene.object_ids()))
    const groupIdSet = new Set(Array.from(scene.group_ids()))
    const instanceIdSet = new Set(Array.from(scene.instance_ids()))
    // Booleans take plain solids, whole groups, AND component instances
    // (boolean_nodes; the kernel owns eligibility — solidity, an instance
    // nested inside a group — and refuses typed). An instance operand is
    // allowed here even though the kernel refuses it directly
    // (`BooleanOperandHasInstance`): `Viewport.tsx`'s `runBoolean`
    // transparently explodes it and retries (playtest finding 4). Liveness
    // here; top-level-ness in canBooleanHelper below.
    const isBooleanOperand = (n: NodeRef) =>
      (n.kind === 'object' && objectIdSet.has(n.id)) ||
      (n.kind === 'group' && groupIdSet.has(n.id)) ||
      (n.kind === 'instance' && instanceIdSet.has(n.id))
    const deepest = activeContext.length > 0 ? activeContext[activeContext.length - 1] : undefined
    const activeComponent = deepest?.kind === 'instance'
      ? scene.instance_def(deepest.id)
      : undefined
    const componentMemberIds = activeComponent !== undefined
      ? new Set(Array.from(scene.component_member_objects(activeComponent)))
      : null
    const booleanOperands = selectedIds.filter((n) =>
      componentMemberIds !== null
        ? n.kind === 'object' && componentMemberIds.has(n.id)
        : isBooleanOperand(n),
    )
    const isSketchKind = (n: NodeRef) => isSketchScopedKind(n.kind)
    const parentOf = (n: NodeRef) => {
      if (isSketchKind(n)) return undefined
      const k = n.kind === 'group' ? 1 : n.kind === 'instance' ? 2 : 0
      return scene.node_parent(k, n.id)
    }
    // Sketch-scoped selections have no kernel NodeId — any in the selection
    // disqualifies the node-level commands.
    const hasSketch = selectedIds.some(isSketchKind)
    // The kernel refuses grouping, component creation, instance placement,
    // import, and 3D Text outright while a COMPONENT frame is open
    // (`DocumentError::ExplodeSessionScope` — each would either restructure
    // the document tree or place a new instance the session can't fold
    // back cleanly; a definition is flat, so nothing can host a session or
    // absorb a fold-in inside one). A GROUP-only stack does NOT refuse
    // these — their products fold into the open group at close, the same
    // "everything you make while editing goes into the context" semantic
    // replacing ops already get inside a group session
    // (docs/design/group-session.md) — so the gate keys on the INNERMOST
    // frame's kind, not "any session open". Selection is already scoped to
    // the innermost frame's own scope (`handleSessionChange` clears any
    // stale selection whenever the innermost frame's identity changes), so
    // these would rarely light up on their own inside a component frame —
    // this is the explicit, defensive gate the instructions call for,
    // matching `canMakeComponent`'s existing `activeContext` guard rather
    // than relying solely on that transitive effect. Boolean/ungroup/
    // explode/make-unique are deliberately NOT gated here: a boolean
    // between two session members is exactly the "unmodified tool set"
    // editing a session is for, and the other three all require an
    // 'instance' selection, which scoped picking already makes unreachable
    // while a component frame is open. `componentFrameOpen` itself is the
    // top-level derived const (shared with the import/3D-Text gates and the
    // native menu sync below).
    return {
      booleanOperands,
      // Same top-level rule the kernel enforces (GroupedOperand): a nested
      // node picked in the Outliner must not light the commands up only to
      // be refused on commit. Editing INSIDE a component instance's own
      // definition (component-edit-parity.md phase A2) is the one exception:
      // two or more selected members of that SAME definition may boolean
      // together via `boolean_in_component_many`, checked with `canBooleanInComponent`.
      canBoolean:
        activeContext.length === 0
          ? canBooleanHelper(selectedIds, parentOf, isBooleanOperand)
          : (() => {
              if (componentMemberIds === null) return false
              return canBooleanInComponent(selectedIds, (n) => componentMemberIds.has(n.id))
            })(),
      canGroup: !componentFrameOpen && !hasSketch && canGroupHelper(selectedIds, parentOf),
      canUngroup: !hasSketch && canUngroupHelper(selectedIds),
      canMakeComponent:
        !componentFrameOpen && activeContext.length === 0 && canMakeComponent(selectedIds, parentOf),
      canPlaceCopy: !componentFrameOpen && canPlaceInstance(selectedIds),
      canExplode: canExplodeInstance(selectedIds),
      canMakeUnique: canMakeUnique(selectedIds),
      // Object ▸ Save Selection to Library… — the dock's Save to Library
      // verb's gate (one object, group, or instance, on a platform with a
      // library backend), so the command survives the dock being hidden.
      canSaveSelectionToLibrary:
        libraryStore().available() &&
        selectedIds.length === 1 &&
        (selectedIds[0].kind === 'object' || selectedIds[0].kind === 'group' || selectedIds[0].kind === 'instance'),
      // Copy/Cut (Lane D): at least one STRUCTURAL node selected — a
      // sketch-only selection still refuses with a toast (doCopy), but the
      // command doesn't even light up for one, matching every other
      // selection-gated command's posture.
      hasStructuralSelection: selectedIds.some(
        (n) => n.kind === 'object' || n.kind === 'group' || n.kind === 'instance',
      ),
      // Paste/Paste In Place: whether this window's clipboard cache has
      // anything (kept fresh on focus — see the `clipboardAvailable` effect).
      clipboardHasContent: clipboardAvailable,
    }
    // docRev: entity lists change on every mutation without changing identity.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [state, selectedIds, activeContext, docRev, sessionStack, clipboardAvailable])

  // Choosing a Draw tool at top level clears the selection (see the hook).
  useClearSelectionOnDrawTool(activeTool, DRAW_TOOLS, activeContext.length > 0, () =>
    setSelectedIds((cur) => (cur.length > 0 ? [] : cur)),
  )

  // ---------------------------------------------------------------- palette: dynamic Model entries
  // Object/group/component/tag names as searchable palette entries. Selecting
  // one "jumps" there: nodes get selected + revealed in the Outliner/Entity
  // Info; tags get revealed + flashed in the Tags panel. Labels match the
  // Outliner exactly (same resolveLabel fallbacks).
  const paletteModelEntries = useMemo((): PaletteEntry[] => {
    const scene = state?.scene
    if (scene == null || sharedTreeIndex === null) return []
    const entries: PaletteEntry[] = []

    // Positional fallback indices ("Object N") must be the Outliner's — the
    // tree numbers nodes within their parent container, not within the flat
    // per-kind id lists these loops iterate.
    const treeIndex = sharedTreeIndex
    const indexOf = (kind: NodeRef['kind'], id: bigint) =>
      treeIndex.get(nodeKey({ kind, id })) ?? 0

    const pushNode = (kind: NodeRef['kind'], id: bigint, label: string) => {
      entries.push({
        id: `jump-node:${kind}:${id}`,
        label:
          kind === 'object'
            ? t({ comment: 'Command palette entry that jumps to a named object', message: `Object: ${label}` })
            : kind === 'group'
              ? t({ comment: 'Command palette entry that jumps to a named group', message: `Group: ${label}` })
              : t({ comment: 'Command palette entry that jumps to a named component instance', message: `Component: ${label}` }),
        description: t`Select it and reveal it in the Outliner.`,
        // oxlint-disable-next-line lingui/no-unlocalized-strings -- palette group ID
        group: 'Model',
        synonyms: [label],
      })
    }
    Array.from(scene.object_ids()).forEach((id) => {
      pushNode('object', id, resolveLabel(scene.object_name(id), undefined, 'object', indexOf('object', id)))
    })
    Array.from(scene.group_ids()).forEach((id) => {
      pushNode('group', id, resolveLabel(scene.group_name(id), undefined, 'group', indexOf('group', id)))
    })
    Array.from(scene.instance_ids()).forEach((id) => {
      const def = scene.instance_def(id)
      const defName = def !== undefined ? scene.component_name(def) : undefined
      pushNode('instance', id, resolveLabel(scene.instance_name(id), defName, 'instance', indexOf('instance', id)))
    })

    // Unique tag paths across all nodes (same walk the Tags panel does).
    const seenTags = new Set<string>()
    const allNodes: { kindNum: number; id: bigint }[] = [
      ...Array.from(scene.object_ids()).map((id) => ({ kindNum: 0, id })),
      ...Array.from(scene.group_ids()).map((id) => ({ kindNum: 1, id })),
      ...Array.from(scene.instance_ids()).map((id) => ({ kindNum: 2, id })),
    ]
    for (const { kindNum, id } of allNodes) {
      for (const rawTag of scene.node_tags(kindNum, id)) {
        const path = rawTag.split('/').map((s) => s.trim()).filter((s) => s.length > 0)
        if (path.length === 0) continue
        const key = tagPathKey(path)
        if (seenTags.has(key)) continue
        seenTags.add(key)
        const tagLabel = path.join(' / ')
        entries.push({
          id: `jump-tag:${key}`,
          label: t({ comment: 'Command palette entry that jumps to a tag; the placeholder is the tag path', message: `Tag: ${tagLabel}` }),
          description: t`Reveal this tag in the Tags panel.`,
          // oxlint-disable-next-line lingui/no-unlocalized-strings -- palette group ID
          group: 'Model',
          synonyms: path,
        })
      }
    }
    return entries
    // docRev: names/tags change on every mutation without changing identity.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [state, sharedTreeIndex, t])

  // Model Health's cheap whole-document summary, once per document change —
  // the status badge and the Outliner icons read it (component-definition
  // members included, which the tessellation-derived watertight map never
  // sees). The full report is the panel's, read only while its window is open.
  const documentHealth = useMemo(
    () => (state === null ? EMPTY_SUMMARY : readHealthSummary(state.scene)),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [state, docRev],
  )
  const objectHealth = useMemo(() => objectHealthStates(documentHealth), [documentHealth])
  const healthSummary = useMemo(() => summarize(documentHealth), [documentHealth])

  /** Active Tags-panel reveal (palette jump); cleared after a short flash. */
  const [revealTag, setRevealTag] = useState<TagReveal | null>(null)
  const revealNonceRef = useRef(0)
  /** Bumped on each Object Info → Materials reveal so the palette scrolls the
   *  swatch into view even when it was already the current material. */
  const [materialRevealNonce, setMaterialRevealNonce] = useState(0)

  /** The one "select and reveal" for Object Info's links: bring the tray back,
   *  open the pane, select the target in it, and have the pane expand and
   *  scroll to it. A tag target is a path (selected, parents expanded, row
   *  scrolled into view); a material target is a handle (made the palette's
   *  current material — without picking up the Paint tool — and scrolled
   *  into view). The scene selection is left alone, so Object Info keeps
   *  showing what the person had selected. */
  const revealInTray = useCallback((pane: 'tags' | 'materials', target: string[] | bigint) => {
    showTray()
    if (pane === 'tags' && Array.isArray(target)) {
      setShowTags(true)
      const nonce = ++revealNonceRef.current
      setRevealTag({ key: JSON.stringify(target), nonce, select: true })
      // The flash fades; the selection (the panel's active row) stays.
      setTimeout(() => {
        setRevealTag((cur) => (cur?.nonce === nonce ? null : cur))
      }, 2000)
    } else if (pane === 'materials' && typeof target === 'bigint') {
      setShowMaterials(true)
      setCurrentMaterialId(target)
      setMaterialRevealNonce((n) => n + 1)
    }
  }, [showTray])

  // ---------------------------------------------------------------- document lifecycle events
  // Tells the Scene a lifecycle step for its next event drain
  // (docs/agents/HEW_API.md §4.5): `document_will_save`, `_saved`,
  // `_opened`, `_closed`. The path is the file's on the desktop (a web
  // file handle has none to give); the caller pumps.
  const noteDocumentEvent = useCallback(
    (kind: 'will_save' | 'saved' | 'opened' | 'closed', title: string, ref: FileRef | null, format?: string) => {
      const scene = sceneRef.current
      if (scene === null) return
      const path = typeof ref?.handle === 'string' ? ref.handle : undefined
      scene.note_document_event(kind, title, path, format)
    },
    [],
  )

  // The display units live in Settings, not the document: mirror them into
  // the Scene (what a live `.skp` export opens in, and `units_changed`).
  useEffect(() => {
    const scene = state?.scene
    if (scene === undefined) return
    const mirror = (): void => {
      scene.set_display_units(getLengthUnit())
      pumpEvents(scene)
    }
    mirror()
    return subscribeLengthUnit(mirror)
  }, [state?.scene])

  // The page going away closes its document, for anyone still listening.
  useEffect(() => {
    const onPageHide = (): void => {
      const scene = sceneRef.current
      if (scene === null) return
      noteDocumentEvent('closed', documentName(docSessionRef.current), docSessionRef.current.currentRef)
      pumpEvents(scene)
    }
    window.addEventListener('pagehide', onPageHide)
    return () => window.removeEventListener('pagehide', onPageHide)
  }, [noteDocumentEvent])

  // ---------------------------------------------------------------- shared reset helper
  // Used by New, Open — loads bytes into the scene and resets all UI state.
  // Seeds the app's hidden-state mirrors (hidden nodes, hidden tag paths)
  // from the document's own registries, and with `push` sends them to the
  // renderer and the kernel's pick/snap exclusion. What a load does, and
  // what an aborted plugin command needs: its abort restores the whole
  // document, view state included (docs/agents/PLUGINS.md "Compounds").
  const reseedHiddenStateFromScene = useCallback(
    (scene: Scene, push = true): { keys: Set<string>; tagPaths: Set<string> } => {
      const keys = seedHiddenKeysFromRegistry(scene)
      setHiddenKeys(keys)
      hiddenKeysRef.current = keys
      const tagPaths = seedHiddenTagPathsFromRegistry(scene)
      setHiddenTagPaths(tagPaths)
      hiddenTagPathsRef.current = tagPaths
      if (push) pushUnionHiddenRef.current(keys, tagPaths)
      return { keys, tagPaths }
    },
    [],
  )
  const reseedHiddenStateRef = useRef(reseedHiddenStateFromScene)
  reseedHiddenStateRef.current = reseedHiddenStateFromScene

  const applyLoadedBytes = useCallback((bytes: Uint8Array, file?: LoadedFile, opts?: { keepCamera?: boolean; announce?: boolean }): boolean => {
    const scene = sceneRef.current
    if (scene === null) return false
    // Revert to Saved reloads the document under the person's feet: the view
    // stays where it is, over the camera the file was saved with.
    const keptCamera = opts?.keepCamera === true ? viewportApi.current?.getCameraState() : undefined
    if (file !== undefined) beginOpenAttempt({ name: file.name, label: file.label, bytes })
    const loaded = loadHewBytes(scene, bytes)
    if (!loaded.ok) {
      if (file !== undefined) noteOpenFailure('Open', loaded.error, { name: file.name, bytes })
      if (loaded.panicked) {
        setKernelPanicked(true)
      }
      handleToast(friendlyErrorText(loaded.error))
      return false
    }
    // Any successful load supersedes the welcome screen — e.g. a warm
    // file-association open delivered while it was showing. The ref covers
    // the other ordering: a load that lands while the startup-handoff effect
    // is still awaiting its recovery check must also veto the offer it makes
    // at the end (the state setters alone can't — the later one wins).
    // A document replaced in place: subscribers hear the old one closed
    // and the new one opened (the reset follows in the same drain).
    if (documentLoadedRef.current) noteDocumentEvent('closed', documentName(docSessionRef.current), docSessionRef.current.currentRef)
    // A `.skp` open loads a blank first and announces the real document
    // itself once the import finishes (`announce: false`).
    if (opts?.announce !== false) {
      noteDocumentEvent(
        'opened',
        file?.name ?? documentName(afterOpen(null, 0)),
        file?.path === undefined ? null : { name: file.name, format: 'hew', handle: file.path },
      )
    }
    documentLoadedRef.current = true
    if (file !== undefined) clearOpenAttempt()
    else forgetSettledFailure()
    // `Scene.load` cleared the `.skp` source, and this is a new document:
    // the one-time `.skp` loss notice is due again (never reset on save).
    skpSourceGenRef.current += 1
    skpNoticeShownRef.current = false
    skpCompatRef.current = false
    setSkpNotice(null)
    setDocGeneration((g) => g + 1)
    // A new document starts in normal rendering. Revert keeps the window as
    // it is, so it keeps the person's render mode too.
    if (opts?.keepCamera !== true) setRenderMode('shaded')
    setWelcomeOpen(false)
    setSelectedIds([])
    setActiveContext([])
    // The wasm handle is reused and mutated in place across New/Open, so
    // the Scenes controller must be told explicitly that this is a new
    // document (docs/design/scenes.md §5): no active Scene, no drift, no
    // thumbnails carried over — a stale sid could otherwise collide with a
    // same-numbered Scene in the newly opened file.
    scenesRef.current.resetForDocument()
    // Seed from the just-loaded document's registries rather than clearing —
    // hidden .skp layers/nodes (or a re-opened .hew with tags/nodes previously
    // hidden via the eye toggle) must come up hidden on first render, not
    // visible.
    const { keys: seededHiddenKeys, tagPaths: seededHiddenTagPaths } = reseedHiddenStateFromScene(scene, false)
    // Also clear the renderer's hidden set: it keys by dense ids that the new
    // document reuses, so stale ids would silently hide (and un-pick) unrelated
    // objects after a load. (No-op if the viewport isn't mounted yet.)
    viewportApi.current?.setHidden([], [])
    // The document's persisted section plane (docs/design/scenes.md §4) —
    // read BEFORE notifyLoaded, whose "new document, clean view state" reset
    // deletes the viewport's plane and, through handleSectionChanged, would
    // otherwise clear the kernel's freshly loaded one too.
    let loadedSection: { origin: [number, number, number]; normal: [number, number, number]; active: boolean } | null = null
    try {
      const sectionJson = scene.section_plane_json()
      if (sectionJson !== undefined) loadedSection = parseSectionJson(JSON.parse(sectionJson)) ?? null
    } catch {
      loadedSection = null
    }
    // Suppress dirty-marking while notifyLoaded triggers handleDocumentChanged;
    // the caller will commit the authoritative afterOpen state (dirty=false).
    suppressDirtyRef.current = true
    try {
      viewportApi.current?.notifyLoaded()
    } finally {
      suppressDirtyRef.current = false
    }
    // Re-establish the loaded plane in the viewport (fires onSectionChanged →
    // writes the same plane back into the kernel; idempotent).
    if (loadedSection !== null) viewportApi.current?.setSectionPlane(loadedSection)
    // Push the seeded hides now that the scene is tessellated (notifyLoaded
    // above), so hidden-by-default tags/nodes take effect on first render
    // instead of waiting for the user to touch an eye toggle.
    pushUnionHiddenRef.current(seededHiddenKeys, seededHiddenTagPaths)
    // The document's view style (kernel `ViewStyle`): a `.skp`'s backdrop
    // and axes follow the file; a document without one is shown Hew's way.
    const styleJson = scene.view_style_json()
    const style = styleJson === undefined ? null : parseViewStyleJson(JSON.parse(styleJson))
    viewportApi.current?.setViewStyle(style)
    if (style !== null) setShowAxes(style.axes)
    // Restore the saved camera view (docs/design/camera.md §5) if the
    // document carries one; absent (every pre-v13 file, or a v13+ document
    // that never saved a view — File ▸ New's blank bytes always land here)
    // falls back to the pre-existing default framing (zoomExtents for a
    // non-empty model, or the mount-time home view for a blank one).
    const savedCamera = scene.camera_state()
    if (keptCamera !== undefined) {
      savedCamera?.free()
    } else if (savedCamera !== undefined) {
      try {
        const applyState = {
          projection: savedCamera.projection() as 'perspective' | 'parallel',
          fovDeg: savedCamera.fov_deg(),
          parallelHeight: savedCamera.parallel_height(),
          eye: [savedCamera.eye_x(), savedCamera.eye_y(), savedCamera.eye_z()] as [number, number, number],
          target: [savedCamera.target_x(), savedCamera.target_y(), savedCamera.target_z()] as [number, number, number],
          up: [savedCamera.up_x(), savedCamera.up_y(), savedCamera.up_z()] as [number, number, number],
        }
        requestAnimationFrame(() => viewportApi.current?.applyCameraState(applyState))
      } finally {
        savedCamera.free()
      }
    } else if (!isSceneEmpty(scene)) {
      // Frame the freshly-loaded model (Open / Recover / drag-drop / file
      // association all funnel through here). Empty documents (File ▸ New's
      // blank bytes) keep the default framing.
      requestAnimationFrame(() => viewportApi.current?.zoomExtents())
    }
    // A document with Scenes opens on its first Scene (docs/design/scenes.md
    // §5, playtest; SketchUp's own behavior) — activated instantly, no tween,
    // AFTER the saved-camera/zoom-extents frame above so the Scene's camera
    // (if it captures one) wins. Activation is view state: never dirties.
    requestAnimationFrame(() => {
      scenesRef.current.activateFirstOnLoad()
      if (keptCamera !== undefined) viewportApi.current?.applyCameraState(keptCamera)
    })
    // Scene thumbnail cache (docs/design/scenes.md §5 "Add"): the derived
    // JPEGs live in IndexedDB, keyed by content hash + sid, never in the
    // file itself — restore whatever this exact document's bytes had cached
    // from a prior session. Fire-and-forget: a cache miss/failure just shows
    // placeholders, and must never delay or block the load it rides on.
    void (async () => {
      try {
        const hash = await sha256Hex(bytes)
        const cached = await loadSceneThumbnails(hash)
        if (cached.size > 0) scenesRef.current.mergeThumbnails(cached)
      } catch {
        /* best-effort — placeholders are fine */
      }
    })()
    pumpEvents(scene)
    return true
  }, [handleToast, reseedHiddenStateFromScene, noteDocumentEvent])
  // Keep the harness's Open path ( __hew_test.load) pointed at the latest
  // applyLoadedBytes closure.
  applyLoadedBytesRef.current = applyLoadedBytes

  // ---------------------------------------------------------------- unsaved-changes dialog
  // The in-app UnsavedChangesDialog (Lane C, docs/design/v1.1-cycle.md)
  // replaces the native `ask`/`confirm` this guard used to show — it lists
  // the entries since the saved mark AND the non-undoable reasons
  // (Scenes/Library/…), and offers Save (runs the current save flow and
  // only proceeds on success) as a third option alongside Discard/Cancel.
  // Promise-plus-ref-resolver, same shape as promptStlUnits above: the
  // dialog itself is rendered declaratively near the bottom of this
  // component; this bridges that to the imperative call sites (confirmDiscard,
  // the Tauri close guard) that need to await a decision.
  const unsavedChangesResolveRef = useRef<((decision: UnsavedChangesDecision) => void) | null>(null)
  const [unsavedChangesPromptOpen, setUnsavedChangesPromptOpen] = useState(false)

  const requestUnsavedChangesDecision = useCallback((): Promise<UnsavedChangesDecision> => {
    return new Promise((resolve) => {
      // Defense in depth, mirroring promptStlUnits: a clobbered prior
      // prompt resolves 'cancel' rather than hanging forever.
      if (unsavedChangesResolveRef.current !== null) {
        unsavedChangesResolveRef.current('cancel')
      }
      unsavedChangesResolveRef.current = resolve
      setUnsavedChangesPromptOpen(true)
    })
  }, [])

  const resolveUnsavedChangesPrompt = useCallback((decision: UnsavedChangesDecision) => {
    setUnsavedChangesPromptOpen(false)
    const resolve = unsavedChangesResolveRef.current
    unsavedChangesResolveRef.current = null
    resolve?.(decision)
  }, [])

  // ---------------------------------------------------------------- discard guard
  // Returns true if it's safe to proceed (no unsaved changes, the user chose
  // Don't Save, or Save just succeeded). Reads current session state from
  // docSessionRef to stay pure-function-safe.
  const confirmDiscard = useCallback(async (): Promise<boolean> => {
    if (!docSessionRef.current.dirty) return true
    const decision = await requestUnsavedChangesDecision()
    // 'save' arrives only once the dialog's own Save has written the file
    // (UnsavedChangesDialog's onSave below) — saving again here would write
    // it twice, and a document with no file yet would ask where twice.
    return decision !== 'cancel'
  }, [requestUnsavedChangesDecision])

  // Drop this window's autosave snapshot once its document has actually been
  // discarded (replaced in place, or the window is closing) — a discarded
  // document must not resurface in the next launch's recovery prompt. Called
  // only AFTER the discard is irreversible, never at confirm time: a
  // confirmed-then-cancelled Open leaves the dirty document (and its crash
  // snapshot) untouched. Replace-in-place call sites additionally gate on
  // the pre-replace dirty flag — replacing a clean document discards
  // nothing, and clearing anyway could delete an Escape-deferred startup
  // snapshot on the web's single, unprotected slot (desktop slots are
  // claim-protected by the shell).
  //
  // Disarms the autosave tick first, then waits out any write already in
  // flight — the shell does not order recovery_write against recovery_clear,
  // so an overlapping clear could strand the discarded snapshot (or half of
  // it) on disk. Also used after Save/Save As: the same ordering guarantee
  // applies to clearing the snapshot of a just-saved document.
  const clearRecoverySnapshot = useCallback(async (): Promise<void> => {
    dirtySinceAutosaveRef.current = false
    // Cancel any pending timer outright — a discard/Save has nothing left
    // for it to do, and letting it fire anyway (to find `dirtySinceAutosave
    // Ref.current` already false and no-op) would just be idle wakeups; the
    // next mutation re-arms via `armAutosaveTick`.
    if (autosaveTimerRef.current !== null) {
      clearTimeout(autosaveTimerRef.current)
      autosaveTimerRef.current = null
    }
    const inFlight = autosaveWriteRef.current
    await inFlight
    // A newer write while we awaited means the document changed under us
    // (e.g. a first edit to the replacement document re-armed the tick) —
    // the slot no longer holds the discarded document, so leave it alone.
    if (autosaveWriteRef.current !== inFlight) return
    await recoveryStoreRef.current.clear().catch(() => { /* best effort */ })
    // The slot's `.skp` source went with the snapshot: the next autosave of
    // a `.skp` document must write it again.
    persistedSourceGenRef.current = -1
    sourceOnDiskRef.current = false
  }, [])

  // Drop this window's autosave/recovery snapshot the moment the session
  // state transitions dirty → clean, wherever that happens. The common
  // case is `handleDocumentChanged` computing `atSavedMark: true` after an
  // undo (or a run of them) lands back on the saved depth — the document
  // reads clean, but autosave may already have written a snapshot for it
  // while it was still dirty, and nothing else clears it: Save/Open/
  // Import/discard flows each call `clearRecoverySnapshot` at their own
  // choke point, but "the user undid everything" isn't one of them. Left
  // alone, that stale snapshot survives a subsequent clean close (correctly
  // silent — nothing dirty to warn about) and falsely offers recovery on
  // the next launch (the maintainer's repro).
  //
  // `wasDirtyRef` (not `docSessionRef`, which this same render already
  // updated for the CURRENT state) holds the previous render's dirty flag
  // so this only fires on the actual transition, not on every clean
  // render (mount, a fresh document, a page that was never dirty). Runs
  // AFTER `clearRecoverySnapshot`'s own definition so it can close over it
  // directly; `clearRecoverySnapshot` itself is racesafe (it awaits any
  // write still in flight before clearing, and bails if a newer write
  // superseded it — see its own doc comment), so a write that completes
  // after this transition is either awaited-then-cleared or represents a
  // genuinely newer edit and is correctly left alone. `clear()` is a
  // scoped no-op when there's nothing to clear, so redundant calls here
  // (e.g. right after a save flow already cleared it) are harmless.
  const wasDirtyRef = useRef(false)
  useEffect(() => {
    if (wasDirtyRef.current && !docSession.dirty) {
      void clearRecoverySnapshot()
    }
    wasDirtyRef.current = docSession.dirty
  }, [docSession.dirty, clearRecoverySnapshot])

  // Web build: beforeunload can't await a save/discard decision the way
  // the Tauri close guard does, so a dirty document only gets the native
  // "leave site?" prompt above. But when the document is NOT dirty, this
  // is still a real close — best-effort clear this window's snapshot the
  // same way the Tauri close handler does unconditionally, in case the
  // dirty→clean effect above hasn't run yet (e.g. the transition and the
  // unload land in the same tick) or a stale snapshot predates this
  // session's own dirty tracking. Fire-and-forget: unload doesn't wait.
  useEffect(() => {
    const handler = () => {
      if (!docSessionRef.current.dirty) void clearRecoverySnapshot()
    }
    window.addEventListener('beforeunload', handler)
    return () => window.removeEventListener('beforeunload', handler)
  }, [clearRecoverySnapshot])

  // ---------------------------------------------------------------- document lifecycle

  const newDocument = useCallback(async () => {
    const scene = sceneRef.current
    if (scene === null) return
    if (!isPristineDocument(docSessionRef.current, scene)) {
      // Non-pristine (a named file, unsaved edits, or existing geometry):
      // never overwrite it. On the desktop, File ▸ New opens a fresh
      // document window (the macOS staple); the current window keeps its
      // state untouched, so no discard prompt is needed. Only the web
      // build, which can't open OS windows, falls back to replace-in-place.
      // (Note: this is stricter than a bare isSceneEmpty check — a
      // saved-then-emptied document, currentRef set but nothing left in the
      // model, is still not pristine and must not be reused silently.)
      if (isTauri) {
        try {
          const { invoke } = await import('@tauri-apps/api/core')
          await invoke('new_window')
          return
        } catch {
          // fall through to the in-window reset below
        }
      }
      if (!(await confirmDiscard())) return
    }
    // Pristine (or web fallback): reset in place — a pristine document is
    // reused rather than spawning a second empty one.
    const blank = blankBytesRef.current
    if (blank === null) return
    const discardsUnsaved = docSessionRef.current.dirty
    if (applyLoadedBytes(blank)) {
      setDocSession(afterOpen(null, Date.now()))
      if (discardsUnsaved) void clearRecoverySnapshot()
    }
  }, [confirmDiscard, applyLoadedBytes, clearRecoverySnapshot])

  // Show the STL units-chooser modal and resolve with the chosen unit_scale,
  // or null if the user cancels (Escape / backdrop click / Cancel button).
  // A promise-plus-ref-resolver bridges the imperative runImportPick flow
  // below to the declarative modal rendered near the bottom of this
  // component (see the `pendingStlImport` block).
  const promptStlUnits = useCallback((fileName: string): Promise<number | null> => {
    return new Promise((resolve) => {
      // Defense in depth: if a chooser is somehow already open, fail its
      // promise cleanly (null) before taking over the resolver slot, so a
      // clobbered import bails rather than hanging forever.
      if (stlUnitsResolveRef.current !== null) {
        stlUnitsResolveRef.current(null)
      }
      stlUnitsResolveRef.current = resolve
      setPendingStlImport({ name: fileName })
    })
  }, [])

  // The post-pick import steps, shared by openDocument (the unified Open
  // dialog's non-hew branch) and importDocument (File ▸ Import…'s explicit,
  // import-only dialog) — both converge here once a pick is in hand. The
  // caller has already confirmed discard and holds openInFlightRef for the
  // whole gesture; `scene` is the live Scene it captured at the top of that
  // gesture.
  const runImportPick = useCallback(async (scene: Scene, pick: ImportPick) => {
    // STL carries no units of its own — ask before doing any blocking work
    // (unlike the other formats, which get no prompt at all). Cancelling
    // here leaves the current document untouched, same as cancelling the
    // file picker.
    let stlUnitScale = 0.001 // unused for non-STL kinds
    if (pick.kind === 'stl') {
      const chosen = await promptStlUnits(pick.name)
      if (chosen === null) return
      stlUnitScale = chosen
    }

    // Show the overlay BEFORE any blocking work.
    //
    // flushSync forces a synchronous DOM commit so the overlay card is in the
    // DOM immediately, rather than waiting for React's next async render cycle.
    // nextPaint() then waits two animation frames before we freeze the main
    // thread.  One rAF is NOT enough: rAF callbacks run before their frame is
    // painted, so resolving there would let the awaiting continuation start
    // the synchronous import ahead of the paint and the overlay would not
    // show until partway through the freeze.  See nextPaint() in paint.ts.
    //
    // NOTE: import_dae runs synchronously on the main thread, so the CSS
    // spinner animation will freeze while it parses.  The text message still
    // communicates progress.  True smooth animation would require running the
    // import in a Web Worker (future work — needs a SharedArrayBuffer channel
    // to the WASM module).
    flushSync(() => {
      setImportingName(pick.name)
      setReportOpened(false)
      setIsImporting(true)
    })
    await nextPaint()

    let report: ImportReport
    try {
      // First, so the catch below only ever blames this file.
      beginOpenAttempt({ name: pick.name, label: OPEN_FILE_LABELS[pick.kind], bytes: pick.bytes })
      // Reset to a blank document first (replace semantics).
      //
      // applyLoadedBytes(blank) calls scene.load(), clears selection/context,
      // and calls notifyLoaded() under suppressDirtyRef so the dirty mark is
      // suppressed — the afterImport state below owns the dirty flag.
      // We must bail if the blank load fails (should never happen in practice).
      const blank = blankBytesRef.current
      if (blank === null) {
        clearOpenAttempt()
        return
      }
      const discardsUnsaved = docSessionRef.current.dirty
      const blankOk = applyLoadedBytes(blank)
      if (!blankOk) {
        clearOpenAttempt()
        return
      }
      // The previous document is gone as of the blank load above (even if
      // the import below fails) — drop its autosave snapshot with it.
      if (discardsUnsaved) void clearRecoverySnapshot()

      // Import into the now-empty document, dispatched by format. STL has no
      // internal object names, so name its Objects from the file stem (the
      // picked basename minus the .stl extension); a blank stem falls back
      // to "Imported" in the kernel.
      const stlStem = pick.name.replace(/\.stl$/i, '').trim() || undefined
      report = (
        pick.kind === 'gltf'
          ? scene.import_gltf(pick.bytes)
          : pick.kind === 'skp'
            ? scene.import_skp(pick.bytes)
            : pick.kind === 'stl'
              ? scene.import_stl(pick.bytes, stlUnitScale, stlStem)
              : scene.import_dae(
                  pick.bytes,
                  Object.keys(pick.images).length > 0 ? pick.images : null,
                )
      ) as ImportReport
    } catch (err: unknown) {
      noteOpenFailure('Import', err, pick)
      // A wasm trap mid-import (an out-of-memory abort on a model too big
      // for the browser kernel) hands the page to the crash screen inside
      // noteOpenFailure; the toast below is for ordinary refusals.
      handleToast(t`Import failed: ${ph({ reason: friendlyErrorText(err) })}`)
      // The blank-document replace above already committed (applyLoadedBytes
      // mutates the live scene directly, with no rollback), even though the
      // import into it just failed — the live scene really is blank now.
      // Reflect that in docSession (as a fresh untitled document, exactly
      // like newDocument's blank reset) rather than leaving it pointing at
      // whatever file/handle was open before this gesture: without this, the
      // next Save would write the now-blank scene straight to that stale
      // path with no prompt, silently destroying a perfectly good file.
      setDocSession(afterOpen(null, Date.now()))
      return
    } finally {
      // Always clear the overlay — even on throw, so it can never get stuck.
      setIsImporting(false)
    }
    clearOpenAttempt()

    // Tessellate the imported objects and update session state.
    //
    // notifyLoaded() calls handleSceneRefresh() which tessellates the new
    // objects and bumps docRev.  suppressDirtyRef is false here so the dirty
    // mark would normally fire — but we immediately set afterImport() which
    // owns dirty=true, so the net effect is correct.
    viewportApi.current?.notifyLoaded()

    // Re-seed hidden tags and hidden nodes from the registries the import just
    // populated — the applyLoadedBytes(blank) call above seeded from an empty
    // document, so hidden-by-default tags/nodes (e.g. a hidden .skp layer or
    // hidden group/component) only show up now that scene.import_* has
    // registered them.
    reseedHiddenStateFromScene(scene)

    // Commit the session state.
    //
    // afterImport() sets currentRef=null (so Save always prompts — no silent
    // overwrite risk on either WebFileHost or TauriFileHost) and dirty=true.
    // The importedName is used by deriveTitle (window title) and by
    // saveAsDocument's suggested filename.
    setDocSession(afterImport(pick.name, Date.now()))

    setImportReport(report)
    const fmt =
      // oxlint-disable-next-line lingui/no-unlocalized-strings -- diagnostic log line, English for the maintainer
      pick.kind === 'gltf' ? 'glTF' : pick.kind === 'skp' ? 'SKP' : pick.kind === 'stl' ? 'STL' : 'DAE'
    LogStore.log.info('app', `Imported ${fmt}: ${report.objects_created} objects (${importLogSplit(report.health.solids, report.health.flat_sheets + report.health.bent_sheets + report.health.cappable + report.health.holes + report.health.mixed + report.health.non_manifold)})`)
    requestAnimationFrame(() => { viewportApi.current?.zoomExtents() })
  }, [handleToast, applyLoadedBytes, clearRecoverySnapshot, promptStlUnits, reseedHiddenStateFromScene])

  // The ONE open-skp path: every route that opens a `.skp` as the document's
  // OWN file (File ▸ Open, Open Recent, drag-drop, argv / single-instance /
  // Open With, the web recents shelf) lands here. It loads blank bytes the
  // way runImportPick does, then `Scene.open_skp` (which imports AND keeps
  // the bytes as the source a later save builds on), runs the same post-import
  // UI steps (notify loaded, seed hidden keys/tags, scenes, the report), and
  // commits `afterOpen` with the container's format — NOT `afterImport`. The
  // result is a clean document titled `Model.skp` whose Save writes the file
  // back in its own container. File ▸ Import… stays runImportPick: additive
  // semantics, no rebinding. Resolves true when the document landed.
  const runOpenSkp = useCallback(async (
    scene: Scene,
    pick: { name: string; bytes: Uint8Array; handle: unknown },
    // Revert to Saved reloads the document in place: `keepCamera` leaves the
    // view where it is (no zoom-to-fit), `silentReport` skips the import
    // report dialog (the person is returning to a file they already opened).
    opts?: { keepCamera?: boolean; silentReport?: boolean },
  ): Promise<boolean> => {
    // The open yields to the event loop between steps, so a second open
    // (a drop, a recent, a path from the shell) could land while the
    // document is half built; refuse it. Every way out of here settles
    // the overlay and the guard.
    if (skpOpenStepsRef.current) return false
    skpOpenStepsRef.current = true
    const keptCamera = opts?.keepCamera === true ? viewportApi.current?.getCameraState() : undefined
    const settle = () => {
      skpOpenStepsRef.current = false
      setIsImporting(false)
      setOpenProgress(null)
    }
    flushSync(() => {
      setImportingName(pick.name)
      setReportOpened(true)
      setIsImporting(true)
      setOpenProgress({ phase: 'read', done: 0, total: 0, bytes: pick.bytes.byteLength })
    })
    await nextPaint()

    const blank = blankBytesRef.current
    if (blank === null) {
      settle()
      return false
    }
    // The open runs in steps between paints, so the overlay can say how
    // far it has come: reading the file is the one step that cannot be
    // split; the placements and then the objects come a budget of work
    // at a time (see `Scene.open_skp_step`). The file is read before the
    // blank load, so one the reader refuses (a SketchUp version it does
    // not read, a damaged file) leaves the open document as it was.
    // Noted before the kernel sees the file: a kernel that dies mid-open
    // never gets as far as a catch below.
    beginOpenAttempt({ name: pick.name, label: OPEN_FILE_LABELS.skp, bytes: pick.bytes })
    LogStore.log.info('app', `Opening SKP: ${formatBytes(pick.bytes.byteLength)}`)
    let step: OpenStepProgress
    try {
      step = scene.open_skp_begin(pick.bytes) as OpenStepProgress
    } catch (err: unknown) {
      noteOpenFailure('SKP open', err, pick)
      handleToast(t`Open failed: ${ph({ reason: friendlyErrorText(err) })}`)
      settle()
      return false
    }

    let report: ImportReport & { container: string; compat?: boolean }
    try {
      const discardsUnsaved = docSessionRef.current.dirty
      if (!applyLoadedBytes(blank, undefined, { keepCamera: opts?.keepCamera, announce: false })) {
        clearOpenAttempt()
        settle()
        return false
      }
      // The previous document is gone as of the blank load (even if the open
      // below fails) — drop its autosave snapshot with it.
      if (discardsUnsaved) void clearRecoverySnapshot()
      while (step.phase !== 'done') {
        flushSync(() => setOpenProgress({ phase: step.phase as OpenProgress['phase'], done: step.done, total: step.total, shapes: step.shapes }))
        await nextPaint()
        step = scene.open_skp_step(OPEN_STEP_BUDGET_MS) as OpenStepProgress
      }
      report = step.report as ImportReport & { container: string; compat?: boolean }
      flushSync(() => setOpenProgress({ phase: 'view', done: 0, total: 1 }))
      await nextPaint()
      // The file IS this state: its import actions are on the undo stack, but
      // nothing is unsaved. Without the mark the document would read dirty
      // (and Undo-to-clean would never land) right after opening.
      scene.mark_saved()
      skpSourceGenRef.current += 1
      clearOpenAttempt()
    } catch (err: unknown) {
      noteOpenFailure('SKP open', err, pick)
      handleToast(t`Open failed: ${ph({ reason: friendlyErrorText(err) })}`)
      // The blank load committed even though the open failed: reflect a fresh
      // untitled document so the next Save can never write to a stale file
      // (same reasoning as runImportPick's catch).
      setDocSession(afterOpen(null, Date.now()))
      noteDocumentEvent('opened', documentName(afterOpen(null, 0)), null)
      pumpEvents(scene)
      settle()
      return false
    }

    // Tessellate and re-seed hidden state — the blank load seeded from an
    // empty registry. The dirty mark is suppressed: afterOpen owns it.
    // The overlay stays up (its last phase) until the view is ready.
    suppressDirtyRef.current = true
    try {
      viewportApi.current?.notifyLoaded()
    } finally {
      suppressDirtyRef.current = false
      settle()
    }
    reseedHiddenStateFromScene(scene)

    const format = normalizeDocFormat(report.container)
    skpCompatRef.current = report.compat === true
    setDocSession(afterOpen({ name: pick.name, format, handle: pick.handle ?? null }, Date.now()))
    // The import is done: announce the real document, not the blank.
    noteDocumentEvent('opened', pick.name, { name: pick.name, format, handle: pick.handle ?? null })
    pumpEvents(scene)
    if (opts?.silentReport !== true) setImportReport(report)
    const kernelMemory = kernelMemoryBytes()
    LogStore.log.info('app', `Opened SKP: ${report.objects_created} objects (${importLogSplit(report.health.solids, report.health.flat_sheets + report.health.bent_sheets + report.health.cappable + report.health.holes + report.health.mixed + report.health.non_manifold)})${kernelMemory === null ? '' : `; kernel memory ${formatBytes(kernelMemory)}`}`)
    requestAnimationFrame(() => {
      if (keptCamera === undefined) viewportApi.current?.zoomExtents()
      // A document with Scenes opens on its first Scene — after the framing
      // above so the Scene's camera wins (view state; never dirties).
      scenesRef.current.activateFirstOnLoad()
      if (keptCamera !== undefined) viewportApi.current?.applyCameraState(keptCamera)
    })
    if (isTauri && typeof pick.handle === 'string') {
      const path = pick.handle
      import('@tauri-apps/api/core').then(({ invoke }) =>
        invoke('push_recent', { path })
      ).catch(() => { /* ignore */ })
    } else if (!isTauri) {
      void recordRecent(pick.bytes, pick.name, 'open', undefined, undefined, undefined, format)
    }
    return true
  }, [handleToast, applyLoadedBytes, clearRecoverySnapshot, reseedHiddenStateFromScene, noteDocumentEvent])

  // The unified Open dialog: ONE picker accepting `.hew` plus every import
  // format, dispatching on the extension the user picked (see
  // fileHost.ts's openAny doc comment). A `.hew` pick applies directly; every
  // other kind converges on runImportPick, the same post-pick steps
  // importDocument's explicit, import-only dialog feeds.
  const openDocument = useCallback(async () => {
    const scene = sceneRef.current
    if (scene === null) return
    // Re-entrancy guard: shared with importDocument (see openInFlightRef) —
    // a second gesture started while this one is mid-flight (e.g. Ctrl+K
    // behind the STL units modal) would otherwise orphan it. A ref (not
    // state) so the check is synchronous, before the first await.
    if (openInFlightRef.current || skpOpenStepsRef.current) return
    openInFlightRef.current = true
    try {
      // Reuse THIS window only when it's pristine (see isPristineDocument);
      // otherwise the pick lands in a fresh window on the desktop (Tauri),
      // leaving the current document completely untouched. Only the web
      // build, which can't open OS windows, falls back to a guarded
      // replace-in-place — and only that fallback needs the discard prompt:
      // a pristine reuse discards nothing (dirty is false by definition),
      // and the new-window path never touches this window at all.
      const pristine = isPristineDocument(docSessionRef.current, scene)
      const opensNewWindow = !pristine && isTauri
      if (!pristine && !opensNewWindow) {
        if (!(await confirmDiscard())) return
      }

      let pick: OpenPick | null
      try {
        pick = await fileHostRef.current.openAny()
      } catch (err: unknown) {
        handleToast(t`Open failed: ${ph({ reason: friendlyErrorText(err) })}`)
        return
      }
      if (pick === null) return // user cancelled — current document unchanged

      // Already open in another window? Focus it and stop — never a second
      // window on the same file (a `.hew` or `.skp` pick; an import lands as
      // a new untitled document and has no window to be "already open" in).
      if (
        (pick.kind === 'hew' || pick.kind === 'skp') &&
        typeof pick.handle === 'string' &&
        (await focusExistingWindowFor(pick.handle))
      ) return

      if (opensNewWindow) {
        // Every Tauri pick (hew or import) carries a real filesystem path —
        // hew's `handle`, import kinds' `path` — so this should always
        // resolve; the fallback below only matters against an older shell.
        const path = pick.kind === 'hew' ? (pick.handle as string | null) : (pick.path ?? null)
        if (typeof path === 'string') {
          try {
            const { invoke } = await import('@tauri-apps/api/core')
            await invoke('open_in_new_window', { path })
            return
          } catch {
            // fall through to the guarded in-place replace below
          }
        }
        if (!(await confirmDiscard())) return
      }

      if (pick.kind === 'hew') {
        const discardsUnsaved = docSessionRef.current.dirty
        const ok = applyLoadedBytes(pick.bytes, {
          name: pick.name,
          label: OPEN_FILE_LABELS.hew,
          path: typeof pick.handle === 'string' ? pick.handle : undefined,
        })
        if (!ok) return
        setDocSession(afterOpen({ name: pick.name, format: 'hew', handle: pick.handle }, Date.now()))
        if (discardsUnsaved) void clearRecoverySnapshot()
        if (isTauri && typeof pick.handle === 'string') {
          import('@tauri-apps/api/core').then(({ invoke }) =>
            invoke('push_recent', { path: pick.handle as string })
          ).catch(() => { /* ignore */ })
        } else if (!isTauri) {
          // Web build only — the Tauri desktop shell has its own path-based
          // recents (push_recent above); this is the IndexedDB-backed shelf
          // WelcomeScreen/ShopApp read (io/recents.ts). Fire-and-forget: a
          // lost write here must never interrupt the open that triggered it.
          void recordRecent(pick.bytes, pick.name)
        }
        return
      }

      // A `.skp` picked through Open becomes the document's own file.
      if (pick.kind === 'skp') {
        await runOpenSkp(scene, { name: pick.name, bytes: pick.bytes, handle: pick.handle ?? null })
        return
      }

      await runImportPick(scene, pick)
    } finally {
      openInFlightRef.current = false
    }
  }, [confirmDiscard, applyLoadedBytes, handleToast, clearRecoverySnapshot, runImportPick, runOpenSkp])

  // File ▸ Import…'s explicit, import-only dialog (no `.hew` in its filter) —
  // kept alongside the unified Open dialog above for users who specifically
  // want an import-labeled entry point. Converges on the same runImportPick
  // steps once a pick is in hand.
  const importDocument = useCallback(async () => {
    const scene = sceneRef.current
    if (scene === null) return
    // Importing into the current document restructures it (new top-level
    // nodes at minimum) — refused outright by the kernel only while a
    // COMPONENT frame is open (`ExplodeSessionScope`; a group-only stack
    // folds the import's products in at close, docs/design/group-session.md).
    // The menu item is disabled for this reason too (see
    // `editGates.canImport`); this re-check covers the keyboard shortcut and
    // command-palette dispatch, which bypass a disabled menu item the same
    // way `handleGroup`/`handleMakeComponent`'s re-checks do for their own
    // gates.
    if (componentFrameOpen) {
      // oxlint-disable-next-line lingui/no-unlocalized-strings -- kernel error code identifier
      handleToast(kernelErrorMessage('ExplodeSessionScope', ''), 'ExplodeSessionScope')
      return
    }
    // Re-entrancy guard: shared with openDocument (see openInFlightRef) — a
    // second gesture started while this one is mid-flight (e.g. Ctrl+K ▸
    // "import" ▸ Enter behind the units modal) would otherwise orphan it,
    // hanging it forever and silently discarding its work. Refuse it. A ref
    // (not state) so the check is synchronous, before the first await.
    if (openInFlightRef.current || skpOpenStepsRef.current) return
    openInFlightRef.current = true
    try {
      // Same pristine-else-new-window rule as openDocument (see
      // isPristineDocument): reuse THIS window only when it's pristine;
      // otherwise the pick lands in a fresh window on the desktop (Tauri),
      // leaving the current document completely untouched. Only the web
      // build, which can't open OS windows, falls back to a guarded
      // replace-in-place — and only that fallback needs the discard prompt
      // (a pristine reuse discards nothing, and the new-window path never
      // touches this window at all).
      const pristine = isPristineDocument(docSessionRef.current, scene)
      const opensNewWindow = !pristine && isTauri
      if (!pristine && !opensNewWindow) {
        if (!(await confirmDiscard())) return
      }

      // Known limitation: openForImport() couples the picker with the file read
      // (and, for COLLADA on the web, a texture-directory picker AFTER the
      // read), so reading a large file happens before the overlay below can
      // appear.  Separating "picker closed" from "bytes read" needs a FileHost
      // API change; until then the overlay covers everything from here on.
      let pick: ImportPick | null
      try {
        pick = await fileHostRef.current.openForImport()
      } catch (err: unknown) {
        handleToast(t`Import failed: ${ph({ reason: friendlyErrorText(err) })}`)
        return
      }
      if (pick === null) return // user cancelled — current document unchanged

      if (opensNewWindow) {
        // Every Tauri import pick carries a real filesystem path (see
        // openDocument's identical comment) — the fallback below only
        // matters against an older shell.
        if (typeof pick.path === 'string') {
          try {
            const { invoke } = await import('@tauri-apps/api/core')
            await invoke('open_in_new_window', { path: pick.path })
            return
          } catch {
            // fall through to the guarded in-place replace below
          }
        }
        if (!(await confirmDiscard())) return
      }

      await runImportPick(scene, pick)
    } finally {
      openInFlightRef.current = false
    }
  }, [confirmDiscard, handleToast, runImportPick, componentFrameOpen])

  // Place Image (File ▸ Import Image…, a picture dropped on the viewport):
  // arm the placement gesture with the picture. Unlike File ▸ Import…, it
  // adds to the current document — nothing is replaced. The picture's size
  // is read from its header first, so an unreadable or oversized picture is
  // refused with the same words the kernel would use, before anything is
  // decoded. `at` is a drop point (client pixels): the ghost starts there.
  const startImagePlacement = useCallback((fileName: string, bytes: Uint8Array, at?: { clientX: number; clientY: number }) => {
    // The kernel refuses an import while a component is open
    // (ExplodeSessionScope); say so now rather than after two clicks.
    if (componentFrameOpen) {
      // oxlint-disable-next-line lingui/no-unlocalized-strings -- kernel error code
      handleToast(kernelErrorMessage('ExplodeSessionScope', ''), 'ExplodeSessionScope')
      return
    }
    const read = readImageInfo(bytes)
    if (!read.ok) {
      handleToast(kernelErrorMessage(read.code, ''), read.code)
      return
    }
    viewportApi.current?.armImagePlacement(
      {
        bytes,
        format: read.info.format,
        name: pictureName(fileName),
        pixelWidth: read.info.width,
        pixelHeight: read.info.height,
      },
      at,
    )
  }, [componentFrameOpen, handleToast])

  const importImage = useCallback(async () => {
    if (componentFrameOpen) {
      // oxlint-disable-next-line lingui/no-unlocalized-strings -- kernel error code
      handleToast(kernelErrorMessage('ExplodeSessionScope', ''), 'ExplodeSessionScope')
      return
    }
    let picked: { name: string; bytes: Uint8Array } | null
    try {
      picked = (await fileHostRef.current.openImage?.()) ?? null
    } catch (err: unknown) {
      const reason = friendlyErrorText(err)
      handleToast(err instanceof FileReadError
        ? t`Couldn't read that picture. Check that the file is still there, then pick it again.`
        : t`Couldn't open the picture: ${reason}`)
      return
    }
    if (picked === null) return
    startImagePlacement(picked.name, picked.bytes)
  }, [componentFrameOpen, handleToast, startImagePlacement, t])

  // Scene thumbnail cache (docs/design/scenes.md §5 "Add"): persist the
  // controller's CURRENT thumbnail map under the just-written bytes' own
  // content hash, so a later re-open of this exact save finds them again
  // (`applyLoadedBytes`'s matching load). Re-keys on every save rather than
  // once — an edit since the last save changes the content hash, and Save
  // As always does. Fire-and-forget: never blocks or fails a save.
  const persistSceneThumbnails = useCallback((savedBytes: Uint8Array) => {
    const thumbs = scenesRef.current.thumbnails
    if (thumbs.size === 0) return
    void (async () => {
      try {
        const hash = await sha256Hex(savedBytes)
        await saveSceneThumbnails(hash, thumbs)
      } catch {
        /* best-effort */
      }
    })()
  }, [])

  /** Show the non-blocking "the SketchUp file can't hold everything exactly"
   * notice for a Save / Save As — at most once per document (the flag resets
   * on every document load, never on save; later saves are silent). */
  const noteSkpSimplified = useCallback((simplified: string[], writerNotes: string[] = [], always = false) => {
    if ((simplified.length === 0 && writerNotes.length === 0) || (skpNoticeShownRef.current && !always)) return
    skpNoticeShownRef.current = true
    setSkpNotice({ simplified, writerNotes, offerHewCopy: true })
  }, [])

  // Both return a promise resolving `true` iff the write actually landed —
  // false for a user cancel OR a failure (already toasted) — so the
  // unsaved-changes dialog's Save button can await one and only continue
  // the original discard-triggering action (Close/Open/New/Import) on
  // success. Ordinary trigger sites (menu, shortcut) call these
  // fire-and-forget and ignore the result, exactly as before.
  /** Why the last `saveToRef` failed — the live API reports it to its client. */
  // oxlint-disable-next-line lingui/no-unlocalized-strings -- live-API refusal text, English
  const lastSaveErrorRef = useRef('the document could not be saved')

  /** Writes the document to `ref`, in `ref.format`, and commits the post-save
   * session state. The one save path: Save is this with the document's own
   * ref; the live API's `hew.doc.save` is this with a ref for the path the
   * client named. A document with no file yet saves as Save As does, the
   * format chosen along with the place (SketchUp's File ▸ Save the same).
   * `units` names the display units a SketchUp file opens in, over the
   * file's own (the API's `units`); absent, a save back keeps the file's
   * units and a new file takes the app's. */
  const saveToRef = useCallback(
    async (ref: FileRef | null, units?: string, compatArg?: boolean): Promise<boolean> => {
      const scene = sceneRef.current
      if (scene === null) return false
      if (ref === null) return saveAsDocumentRef.current()
      // `Scene.save()` is transparent to an open explode session — it
      // serializes as-if-closed, so saving mid-session needs no special
      // handling here (the session, and its dimming/scoping, is untouched).
      pushCameraStateToScene(scene)
      // The first save mints the document's persistent id (a `.skp` save
      // inside `save_skp`); a canceled save leaves it on the document.
      scene.ensure_document_id()
      const undoDepthAtSave = scene.undo_depth()
      noteDocumentEvent('will_save', documentName(docSessionRef.current), ref, ref.format)
      pumpEvents(scene)
      try {
        // A `.skp` file saves in its own container — NEVER the `.hew` bytes
        // (they would corrupt the file). `save_skp` builds on the file the
        // document came from; the written file becomes the source of the
        // next save only once it is on disk (`commit_skp_save` below).
        let bytes: Uint8Array
        let simplified: string[] = []
        let writerNotes: string[] = []
        // Recorded in `skpCompatRef` only once the write lands.
        let savedCompat: boolean | null = null
        const skp = isSkpFormat(ref.format)
        if (skp) {
          const compat = takesCompat(ref.format) && (compatArg ?? skpCompatRef.current)
          const out = scene.save_skp(skpTarget(ref.format), units ?? getLengthUnit(), units !== undefined, compat) as SkpWrite
          savedCompat = compat
          bytes = out.bytes
          simplified = out.simplified
          writerNotes = out.writer_notes
        } else {
          bytes = new Uint8Array(scene.save())
        }
        const newRef = await fileHostRef.current.save(bytes, ref)
        if (newRef === null) return false // user cancelled
        if (savedCompat !== null) skpCompatRef.current = savedCompat
        // The document is now bound to `newRef`: a `.skp` takes the written
        // file as the source of its next save; a `.hew` has none.
        if (skp) scene.commit_skp_save()
        else scene.set_skp_source(undefined)
        skpSourceGenRef.current += 1
        // See `markSavedIfUnmoved`'s doc comment: only claim the kernel's own
        // saved mark when nothing has edited the document since the bytes
        // above were grabbed.
        const atSavedMark = markSavedIfUnmoved(scene, undoDepthAtSave)
        setDocSession(afterSave(newRef, Date.now(), atSavedMark))
        await reportDocumentId()
        noteDocumentEvent('saved', newRef.name, newRef, newRef.format)
        pumpEvents(scene)
        LogStore.log.info('app', `Saved: ${newRef.name}`)
        // The work is now safely on disk — drop the autosave snapshot.
        void clearRecoverySnapshot()
        if (isTauri && typeof newRef.handle === 'string') {
          import('@tauri-apps/api/core').then(({ invoke }) =>
            invoke('push_recent', { path: newRef.handle as string })
          ).catch(() => { /* ignore */ })
        }
        if (newRef.format === 'hew') persistSceneThumbnails(bytes)
        noteSkpSimplified(simplified, writerNotes)
        return true
      } catch (err: unknown) {
        lastSaveErrorRef.current = friendlyErrorText(err)
        handleToast(t`Save failed: ${ph({ reason: friendlyErrorText(err) })}`)
        return false
      }
    },
    [handleToast, clearRecoverySnapshot, pushCameraStateToScene, noteSkpSimplified, noteDocumentEvent, reportDocumentId],
  )

  const saveDocument = useCallback(
    (): Promise<boolean> => saveToRef(docSession.currentRef),
    [docSession.currentRef, saveToRef],
  )

  // Save As's in-app format choice (the native picker follows it). Resolves
  // with the chosen format, or null when the dialog is canceled.
  const promptSaveFormat = useCallback((initial: DocFormat, initialCompat: boolean): Promise<SaveFormatAnswer | null> => {
    return new Promise((resolve) => {
      // Defense in depth: fail a still-open prompt cleanly before taking
      // over the resolver slot.
      if (saveFormatResolveRef.current !== null) saveFormatResolveRef.current(null)
      saveFormatResolveRef.current = resolve
      setSaveFormatPrompt({ initial, compat: initialCompat })
    })
  }, [])
  const settleSaveFormat = useCallback((answer: SaveFormatAnswer | null) => {
    const resolve = saveFormatResolveRef.current
    saveFormatResolveRef.current = null
    setSaveFormatPrompt(null)
    resolve?.(answer)
  }, [])

  const saveAsDocument = useCallback(async (): Promise<boolean> => {
    const scene = sceneRef.current
    if (scene === null) return false
    // The format comes from the platform's save panel on the desktop (its
    // format list, with the path), from the in-app step in the browser.
    const initial = docSession.currentRef?.format ?? 'hew'
    let format: DocFormat
    let compat = false
    let target: string | null = null
    if (hasNativeSavePanel() && fileHostRef.current.writeTo !== undefined) {
      try {
        const name = nameForFormat(documentName(docSessionRef.current), initial)
        const pick = await pickSaveTarget(t`Save As`, name, savePanelFormats(initial, skpCompatRef.current), panelIdFor(initial))
        if (pick === null) return false // panel canceled
        format = saveFormatFrom(pick)
        compat = saveCompatFrom(pick)
        target = pick.path
      } catch (err: unknown) {
        handleToast(t`Save As failed: ${ph({ reason: friendlyErrorText(err) })}`)
        return false
      }
    } else {
      const chosen = await promptSaveFormat(initial, skpCompatRef.current)
      if (chosen === null) return false // dialog canceled
      format = chosen.format
      compat = takesCompat(chosen.format) && chosen.compat
    }
    pushCameraStateToScene(scene)
    scene.ensure_document_id()
    const undoDepthAtSave = scene.undo_depth()
    // When saving an imported model (currentRef=null, importedName set), suggest
    // the imported filename so the user sees a sensible default in the Save As
    // dialog — its known extension (.skp/.dae/…) swapped for the chosen
    // format's, never stacked (`Model.skp` suggests `Model.hew`).
    const suggestedName = nameForFormat(documentName(docSessionRef.current), format)
    noteDocumentEvent('will_save', documentName(docSessionRef.current), null, format)
    pumpEvents(scene)
    try {
      // `save_skp` builds on the document's source (if any) and stages the
      // written file as the next source; a canceled picker or failed write
      // never commits it, so the source moves only once the write landed.
      let bytes: Uint8Array
      let simplified: string[] = []
      let writerNotes: string[] = []
      if (isSkpFormat(format)) {
        const out = scene.save_skp(skpTarget(format), getLengthUnit(), undefined, compat) as SkpWrite
        bytes = out.bytes
        simplified = out.simplified
        writerNotes = out.writer_notes
      } else {
        bytes = new Uint8Array(scene.save())
      }
      const host = fileHostRef.current
      const newRef =
        target !== null && host.writeTo !== undefined
          ? await host.writeTo(target, bytes, format)
          : await host.saveAs(bytes, suggestedName, format)
      if (newRef === null) return false // user cancelled
      // The document is now bound to `newRef`: a `.skp` takes the written
      // file as the source of its next save; a `.hew` has none.
      if (isSkpFormat(format)) scene.commit_skp_save()
      else scene.set_skp_source(undefined)
      skpCompatRef.current = compat
      skpSourceGenRef.current += 1
      const atSavedMark = markSavedIfUnmoved(scene, undoDepthAtSave)
      setDocSession(afterSave(newRef, Date.now(), atSavedMark))
      await reportDocumentId()
      noteDocumentEvent('saved', newRef.name, newRef, newRef.format)
      pumpEvents(scene)
      LogStore.log.info('app', `Saved as: ${newRef.name}`)
      // The work is now safely on disk — drop the autosave snapshot.
      void clearRecoverySnapshot()
      if (isTauri && typeof newRef.handle === 'string') {
        import('@tauri-apps/api/core').then(({ invoke }) =>
          invoke('push_recent', { path: newRef.handle as string })
        ).catch(() => { /* ignore */ })
      }
      if (format === 'hew') persistSceneThumbnails(bytes)
      // A deliberate choice of format: what it drops is listed every time.
      noteSkpSimplified(simplified, writerNotes, true)
      return true
    } catch (err: unknown) {
      handleToast(t`Save As failed: ${ph({ reason: friendlyErrorText(err) })}`)
      return false
    }
  }, [docSession.currentRef, handleToast, clearRecoverySnapshot, pushCameraStateToScene, promptSaveFormat, noteSkpSimplified, noteDocumentEvent, reportDocumentId])

  // The loss notice's "Save a .hew copy": write the document as `.hew`
  // through the Save As picker WITHOUT rebinding — `currentRef` (and the
  // `.skp` source) stay exactly as they are.
  const saveHewCopy = useCallback(async () => {
    const scene = sceneRef.current
    if (scene === null) return
    pushCameraStateToScene(scene)
    scene.ensure_document_id()
    void reportDocumentId()
    const bytes = new Uint8Array(scene.save())
    const suggestedName = nameForFormat(documentName(docSessionRef.current), 'hew')
    try {
      const copy = await fileHostRef.current.saveAs(bytes, suggestedName, 'hew')
      if (copy !== null) {
        setSkpNotice(null)
        handleToast(t`Saved a .hew copy.`)
        LogStore.log.info('app', `Saved a .hew copy: ${copy.name}`)
      }
    } catch (err: unknown) {
      handleToast(t`Save a .hew copy failed: ${ph({ reason: friendlyErrorText(err) })}`)
    }
  }, [handleToast, pushCameraStateToScene, reportDocumentId])

  // ---------------------------------------------------------------- open by path (Tauri only)
  // Reads the file at `path` and applies it to the CURRENT window
  // unconditionally — no pristine check, no discard guard. Callers must have
  // already established it's safe: openPath below (the pristine-else-
  // new-window branch) after confirming pristine-ness or an explicit discard,
  // or the guaranteed-fresh window a queued open_in_new_window/take_pending_open
  // delivery lands in. Drag-drop, recents, and file-association/second-instance
  // opens hand openPath `.hew` or `.skp` paths (each filters/records only those
  // extensions upstream), but a pick delivered to a new window can be any
  // format the unified Open dialog accepts, so this dispatches on extension
  // the same way openDocument does.
  const openPathInPlace = useCallback(async (path: string): Promise<boolean> => {
    const scene = sceneRef.current
    if (scene === null) return false
    const { invoke } = await import('@tauri-apps/api/core')
    if (/\.hew$/i.test(path)) {
      // Claim the path in the shell BEFORE the read/apply, so a second open
      // of the same file racing this one focuses this window instead of
      // opening its own copy (`DocumentPaths` in main.rs). Released again if
      // the open fails to land — the session effect never re-reports an
      // unchanged null.
      invoke('set_document_path', { path }).catch(() => { /* older shell — ignore */ })
      lastReportedPathRef.current = path
      let landed = false
      try {
        const buf = await invoke<ArrayBuffer>('read_file', { path })
        const bytes = new Uint8Array(buf)
        const discardsUnsaved = docSessionRef.current.dirty
        if (applyLoadedBytes(bytes, { name: basenameOf(path), label: OPEN_FILE_LABELS.hew, path })) {
          landed = true
          setDocSession(afterOpen({ name: basenameOf(path), format: 'hew', handle: path }, Date.now()))
          if (discardsUnsaved) void clearRecoverySnapshot()
          invoke('push_recent', { path }).catch(() => { /* ignore */ })
        }
      } finally {
        if (!landed) {
          lastReportedPathRef.current = null
          invoke('set_document_path', { path: null }).catch(() => { /* ignore */ })
        }
      }
      return landed
    }
    if (/\.skp$/i.test(path)) {
      // A `.skp` path opens as the document's own file (Open Recent, drag-
      // drop, argv, Open With, a File ▸ Open pick delivered to a new
      // window): claim the path like `.hew`, release it if the open fails.
      invoke('set_document_path', { path }).catch(() => { /* older shell — ignore */ })
      lastReportedPathRef.current = path
      let landed = false
      try {
        const buf = await invoke<ArrayBuffer>('read_file', { path })
        landed = await runOpenSkp(scene, { name: basenameOf(path), bytes: new Uint8Array(buf), handle: path })
      } finally {
        if (!landed) {
          lastReportedPathRef.current = null
          invoke('set_document_path', { path: null }).catch(() => { /* ignore */ })
        }
      }
      return landed
    }
    const buf = await invoke<ArrayBuffer>('read_file', { path })
    const bytes = new Uint8Array(buf)
    const { resolveImportPickFromPath } = await import('./io/tauriFileHost')
    const pick = await resolveImportPickFromPath(path, bytes)
    await runImportPick(scene, pick)
    return true
  }, [applyLoadedBytes, clearRecoverySnapshot, runImportPick, runOpenSkp])

  // File ▸ Revert to Saved, after its confirmation (and the live API's
  // `hew.doc.revert`, which asks none): read the saved file again, load it
  // into THIS window through the same load an Open uses, and leave the
  // window, camera and panels where they are. A load starts a document with
  // no undo or redo history and no unsaved mark, so both stacks are empty and
  // the document reads clean. The file is read BEFORE anything is replaced,
  // so one that has gone missing leaves the document as it was.
  //
  // A refusal carries its message; `shown` marks one already toasted here (a
  // failed read or load), so the menu path does not say it twice.
  const revertToSaved = useCallback(async (): Promise<
    { ok: true } | { ok: false; message: string; shown?: true }
  > => {
    const session = docSessionRef.current
    const ref = session.currentRef
    const scene = sceneRef.current
    if (!canRevertToSaved(session) || ref === null || scene === null) {
      return { ok: false, message: t`There is nothing to revert.` }
    }
    // The kernel's undo depth moves with every edit: an edit made while the
    // read or a permission prompt is pending changes it, though neither the
    // file nor the dirty flag does.
    const depthAtStart = scene.undo_depth()
    let bytes: Uint8Array
    try {
      if (typeof ref.handle === 'string') {
        const { invoke } = await import('@tauri-apps/api/core')
        bytes = new Uint8Array(await invoke<ArrayBuffer>('read_file', { path: ref.handle }))
      } else {
        // A web file handle: a person's click on Revert is the gesture a
        // permission prompt needs.
        const handle = ref.handle as {
          getFile: () => Promise<File>
          queryPermission?: (d: { mode: 'read' }) => Promise<PermissionState>
          requestPermission?: (d: { mode: 'read' }) => Promise<PermissionState>
        }
        if (handle.queryPermission !== undefined && (await handle.queryPermission({ mode: 'read' })) !== 'granted') {
          await handle.requestPermission?.({ mode: 'read' })
        }
        bytes = new Uint8Array(await (await handle.getFile()).arrayBuffer())
      }
    } catch (err: unknown) {
      const message = friendlyErrorText(err)
      handleToast(t`Revert failed: ${message}`)
      return { ok: false, message, shown: true }
    }
    // The read (or a permission prompt) took time. If the person saved
    // elsewhere, closed, or opened another file meanwhile, this revert is
    // for a document that is gone; and if the document was saved or undone
    // back to clean, there is nothing left to throw away.
    if (docSessionRef.current.currentRef !== ref || sceneRef.current !== scene || scene.undo_depth() !== depthAtStart) {
      const message = t`Revert canceled: the document changed while the file was being read.`
      handleToast(message)
      return { ok: false, message, shown: true }
    }
    if (!docSessionRef.current.dirty) {
      const message = t`Nothing to revert: the document is already at its saved state.`
      handleToast(message)
      return { ok: false, message, shown: true }
    }
    let landed: boolean
    if (isSkpFormat(ref.format)) {
      if (skpOpenStepsRef.current) {
        return { ok: false, message: t`Another file is still opening; try again in a moment.` }
      }
      landed = await runOpenSkp(scene, { name: ref.name, bytes, handle: ref.handle }, { keepCamera: true, silentReport: true })
    } else {
      landed = applyLoadedBytes(
        bytes,
        { name: ref.name, label: OPEN_FILE_LABELS.hew, path: typeof ref.handle === 'string' ? ref.handle : undefined },
        { keepCamera: true },
      )
      if (landed) setDocSession(afterOpen(ref, Date.now()))
    }
    if (!landed) return { ok: false, message: t`The saved file could not be loaded.`, shown: true }
    // The unsaved work this replaced is gone: so is its crash snapshot.
    void clearRecoverySnapshot()
    if (isTauri && typeof ref.handle === 'string') {
      const path = ref.handle
      import('@tauri-apps/api/core')
        .then(({ invoke }) => invoke('push_recent', { path }))
        .catch(() => { /* ignore */ })
    } else if (!isTauri && !isSkpFormat(ref.format)) {
      void recordRecent(bytes, ref.name)
    }
    handleToast(t`Reverted to the saved file.`)
    return { ok: true }
  }, [applyLoadedBytes, runOpenSkp, clearRecoverySnapshot, handleToast, t])

  // Tauri-only. The entry point for every "open this path" gesture that can
  // land on a window the user may already be working in: native/in-app "Open
  // Recent", native drag-drop, and a live file-association or second-instance
  // open delivered via the `menu-open-path` listener below (which also covers
  // a warm macOS "open document" Apple event and a native recent-file click —
  // see that listener's own doc comment). It also covers the two
  // guaranteed-fresh-window deliveries in the startup-handoff effect
  // (open_in_new_window's queued path, and a cold-start take_pending_open
  // path): a freshly spawned window is pristine by construction, so the
  // pristine check below reduces to the same in-place apply those deliveries
  // always had — this is not a special case, it just falls out of the rule.
  //
  // Follows the same pristine-else-new-window rule as openDocument (see
  // isPristineDocument): a non-pristine window is never silently replaced —
  // the path opens in a fresh window instead, leaving this window's document
  // completely untouched. Only a pristine reuse (or an invoke failure against
  // an older shell) proceeds to replace in place; the discard prompt is
  // reachable ONLY from that fallback, never from an ordinary new-window open
  // (nothing is discarded when a new window opens).
  /** Opens `path`'s BYTES as an import-shaped copy named `name` — the
   * library Open-as-Document contract in a fresh window (see the pristine
   * route in `onLibraryOpenAsDocument`): Save prompts, no recents entry,
   * no path binding. */
  const openLibraryCopy = useCallback(
    async (path: string, name: string | null) => {
      const scene = sceneRef.current
      if (scene === null) return
      try {
        const { invoke } = await import('@tauri-apps/api/core')
        const buf = await invoke<ArrayBuffer>('read_file', { path })
        const bytes = new Uint8Array(buf)
        const ok = applyLoadedBytes(bytes, { name: basenameOf(path), label: OPEN_FILE_LABELS.hew })
        if (ok) setDocSession(afterImport(name ?? t`Library Item`, Date.now()))
      } catch (err) {
        handleToast(
          t`Couldn't open the library item: ${ph({ reason: err instanceof Error ? err.message : String(err) })}`,
        )
      }
    },
    [applyLoadedBytes, handleToast],
  )
  const openLibraryCopyRef = useRef(openLibraryCopy)
  useEffect(() => { openLibraryCopyRef.current = openLibraryCopy }, [openLibraryCopy])

  const openPath = useCallback(async (path: string) => {
    const scene = sceneRef.current
    if (scene === null) return
    // Already open in some window (this one included)? Focus it, done.
    if (await focusExistingWindowFor(path)) return
    const pristine = isPristineDocument(docSessionRef.current, scene)
    if (!pristine) {
      try {
        const { invoke } = await import('@tauri-apps/api/core')
        await invoke('open_in_new_window', { path })
        return
      } catch {
        // Older shell without the command, or the invoke itself failed —
        // fall through to a guarded in-place replace, same as openDocument.
      }
      if (!(await confirmDiscard())) return
    }
    await openPathInPlace(path)
  }, [confirmDiscard, openPathInPlace])

  // ---------------------------------------------------------------- Open Recent (in-app menu; Linux web menu,  port)
  // The recents list is owned by the Rust shell (recents.json). The native
  // macOS/Windows menu has its own "Open Recent" submenu; the in-app web menu
  // (used on Linux + web) reads the list via `get_recents` and re-fetches after
  // any open/save/import (those change the session AND call `push_recent`).
  const [recentFiles, setRecentFiles] = useState<string[]>([])
  useEffect(() => {
    if (!isTauri) return
    let cancelled = false
    import('@tauri-apps/api/core')
      .then(({ invoke }) => invoke<string[]>('get_recents'))
      .then((list) => { if (!cancelled) setRecentFiles(list) })
      .catch(() => { /* ignore */ })
    return () => { cancelled = true }
  }, [docSession.currentRef, docSession.importedName])

  // A recent entry can go stale (file moved or deleted since); surface the
  // failure instead of letting the rejection vanish as an unhandled promise —
  // "clicking did nothing" is the worst outcome for a welcome-screen row.
  const openRecent = useCallback((path: string) => {
    openPath(path).catch(() => {
      handleToast(t`Couldn't open ${ph({ fileName: basenameOf(path) })} — the file may have been moved or deleted.`)
    })
  }, [openPath, handleToast])

  // ---------------------------------------------------------------- bundled samples
  // Fetched from the app's own assets (app/public/samples/, generated by
  // `cargo run -p kernel --example build_samples`). A sample opens like an
  // import: no file handle, so Save always prompts for a location.
  const openSample = useCallback(async (sample: SampleEntry) => {
    try {
      const res = await fetch(`samples/${sample.file}`)
      if (!res.ok) throw new Error(`HTTP ${res.status}`)
      const bytes = new Uint8Array(await res.arrayBuffer())
      if (applyLoadedBytes(bytes)) {
        setDocSession(afterImport(sample.title, Date.now()))
        LogStore.log.info('app', `Opened sample ${sample.file}`)
      }
    } catch (err: unknown) {
      handleToast(t`Couldn't open the sample: ${ph({ reason: friendlyErrorText(err) })}`)
    }
  }, [applyLoadedBytes, handleToast])
  const clearRecent = useCallback(() => {
    import('@tauri-apps/api/core')
      .then(({ invoke }) => invoke('clear_recent'))
      .then(() => setRecentFiles([]))
      .catch(() => { /* ignore */ })
  }, [])

  // ---------------------------------------------------------------- offline recents (web build; io/recents.ts)
  // Distinct from recentFiles/openRecent above (Tauri's path-based shell
  // recents.json, re-read from disk): these already carry their bytes,
  // recorded on every successful web-build open (this component's own
  // applyLoadedBytes-adjacent open paths below, and ShopApp.tsx's). Loaded
  // once per welcome-screen showing so a model opened earlier in the SAME
  // session (or in Shop Mode, sharing the one IndexedDB store) shows up.
  const [webRecents, setWebRecents] = useState<RecentEntry[]>([])
  useEffect(() => {
    if (isTauri) return
    listRecents().then(setWebRecents)
  }, [welcomeOpen])

  const openWebRecent = useCallback((entry: RecentEntry) => {
    // A recorded `.skp` re-opens through the open-skp path (no handle: the
    // shelf stores bytes, so Save asks where to write).
    if (entry.format !== undefined && entry.format !== 'hew') {
      const scene = sceneRef.current
      if (scene !== null) void runOpenSkp(scene, { name: entry.name, bytes: entry.bytes, handle: null })
      return
    }
    if (applyLoadedBytes(entry.bytes, { name: entry.name, label: OPEN_FILE_LABELS.hew })) {
      setDocSession(afterImport(entry.name, Date.now()))
      // Bumps the entry back to most-recently-used — the LRU behavior a
      // "recents" list is expected to have, matching ShopApp.tsx's
      // identical re-record-on-reopen choice.
      void recordRecent(entry.bytes, entry.name)
    }
  }, [applyLoadedBytes, runOpenSkp])

  // ---------------------------------------------------------------- recovery prompt actions

  /** Load a claimed snapshot into THIS window's document and session. Shared
   *  by the startup dialog (first/newest snapshot) and by recovery windows
   *  spawned for the remaining snapshots (take_pending_recovery below). */
  const adoptSnapshot = useCallback((snapshot: RecoverySnapshot): boolean => {
    // The snapshot is .hew bytes whatever the document's display name is
    // (an imported "house.skp", a Library item, "Untitled"); only a file
    // extension comes off, so "Plan v1.2" stays whole.
    const stem = snapshot.meta.name.replace(/\.(hew|skp|dae|stl|glb|gltf)$/i, '')
    const baseName = stem === '' ? t`Untitled` : stem
    const name = `${baseName}.hew`
    const ok = applyLoadedBytes(snapshot.bytes, { name, label: OPEN_FILE_LABELS.recovered })
    if (!ok) return false
    const scene = sceneRef.current
    if (scene === null) return false
    const { meta } = snapshot
    // meta.savedAt is when the autosave snapshot was written — the best
    // available lower bound for "edits existed as of here". Always dirty
    // via the 'Recovered document' reason, not `at_saved_mark()`: `Scene.
    // load` is clean-at-depth-0 by kernel default, but this content was
    // never written to the real file, only to the recovery slot.
    // A `.skp` document recovers with the source it persisted beside the
    // snapshot (Scene.load, above, cleared it), so a Save still builds on the
    // original file; bumping the generation re-writes it under this window's
    // own slot on the next autosave.
    scene.set_skp_source(snapshot.source ?? undefined)
    // `applyLoadedBytes` reset the revision-17 choice; it is recomputed from
    // the recovered source, whose revision is the file's own.
    const SceneClass = scene.constructor as unknown as { skp_container_compat(b: Uint8Array): boolean }
    skpCompatRef.current =
      snapshot.source !== null &&
      takesCompat(normalizeDocFormat(meta.format)) &&
      SceneClass.skp_container_compat(snapshot.source)
    skpSourceGenRef.current += 1
    setDocSession(afterRecovery(
      meta.path !== null ? { name: meta.name, format: normalizeDocFormat(meta.format), handle: meta.path } : null,
      meta.path !== null ? undefined : meta.name,
      meta.savedAt,
    ))
    // The recovered document still only exists in the recovery snapshot —
    // claim() re-homed it to this window's own slot (the next autosave tick
    // refreshes it in place); arm the tick so it will actually fire if
    // nothing else changes.
    armAutosaveTick()
    return true
  }, [applyLoadedBytes, armAutosaveTick])
  const adoptSnapshotRef = useRef(adoptSnapshot)
  useEffect(() => { adoptSnapshotRef.current = adoptSnapshot }, [adoptSnapshot])

  const handleRecover = useCallback(() => {
    const listings = recoveryPrompt
    if (listings === null || listings.length === 0) return
    void (async () => {
      // One snapshot loads here; every other one opens in its own window
      // (new_window parks the slot for the new webview to claim at mount).
      // Claiming re-homes each snapshot, so nothing is shadowed, overwritten,
      // or silently discarded — with N crashed documents, all N come back.
      //
      // This window prefers its OWN slot when one exists (the dialog only
      // runs in the window labeled "main"): claiming a different slot while
      // recovery-main.hew still holds an unclaimed document would force the
      // shell's no-clobber fallbacks instead of the clean rename path.
      const ownIdx = listings.findIndex((l) => l.slot === 'main')
      const ordered =
        ownIdx > 0
          ? [listings[ownIdx], ...listings.filter((_, i) => i !== ownIdx)]
          : listings
      const [first, ...rest] = ordered
      try {
        const snapshot = await recoveryStoreRef.current.claim(first.slot)
        if (snapshot !== null) adoptSnapshot(snapshot)
      } catch {
        /* claim failed — leave this window blank rather than guessing */
      }
      if (isTauri && rest.length > 0) {
        try {
          const { invoke } = await import('@tauri-apps/api/core')
          for (const listing of rest) {
            await invoke('new_window', { recoverSlot: listing.slot })
          }
        } catch {
          /* window spawn failed — the slots stay on disk for next launch */
        }
      }
    })()
    setRecoveryPrompt(null)
  }, [recoveryPrompt, adoptSnapshot])

  const handleDiscardRecovery = useCallback(() => {
    // The dialog listed every snapshot, so this is an informed Discard All —
    // per-window saves use clear(), which drops only that window's own slot.
    recoveryStoreRef.current.discardAll().catch(() => { /* ignore */ })
    setRecoveryPrompt(null)
  }, [])

  // Escape closes the prompt WITHOUT clearing — the snapshot survives and is
  // re-offered next launch, so an accidental keypress can't lose work.
  const handleDismissRecovery = useCallback(() => {
    setRecoveryPrompt(null)
  }, [])

  // ---------------------------------------------------------------- undo/redo for Edit menu
  // Tag-visibility resync is NOT called here: it hangs off the viewport's
  // onHistoryChanged (handleHistoryChanged below), the choke point shared by
  // every undo/redo entry point — menu, palette, AND the viewport's own
  // Cmd+Z/Cmd+Shift+Z keydown, which never passes through these handlers.
  const handleUndo = useCallback(() => {
    viewportApi.current?.runUndo()
  }, [])

  const handleRedo = useCallback(() => {
    viewportApi.current?.runRedo()
  }, [])

  // Fired by the viewport after ANY successful undo/redo. Undoing/redoing a
  // tag deletion changes the tag registry (and its hidden flags): re-seed so
  // content hidden solely via a restored hidden tag re-hides — and stays
  // unpickable — no matter which entry point drove the history step.
  const handleHistoryChanged = useCallback(() => {
    resyncTagVisibilityRef.current()
  }, [])

  // ---------------------------------------------------------------- zoom extents
  const handleZoomExtents = useCallback(() => {
    viewportApi.current?.zoomExtents()
  }, [])

  // ---------------------------------------------------------------- glTF export
  const exportGltf = useCallback(async () => {
    const api = viewportApi.current
    if (api === null) {
      handleToast(t`Export failed: viewport not ready.`)
      return
    }
    let bytes: Uint8Array | null
    try {
      bytes = await api.exportGlb()
    } catch (err: unknown) {
      handleToast(t`Export failed: ${ph({ reason: friendlyErrorText(err) })}`)
      return
    }
    if (bytes === null) {
      handleToast(t`Nothing to export — the model has no solids.`)
      return
    }
    // Suggest a name derived from the current document, dropping its file extension.
    const base = documentBaseName(docSessionRef.current)
    try {
      const exportedTo = await writeExport(fileHostRef.current, exportTargetRef, bytes, base, {
        description: t`glTF Binary`,
        ext: 'glb',
        mime: 'model/gltf-binary',
      })
      if (exportedTo !== null) {
        handleToast(t`Exported glTF.`)
        LogStore.log.info('app', `Exported glTF (${bytes.length} bytes)`)
      }
    } catch (err: unknown) {
      handleToast(t`Export failed: ${ph({ reason: friendlyErrorText(err) })}`)
    }
  }, [docSession.currentRef, docSession.importedName, handleToast])

  // ---------------------------------------------------------------- USDZ export
  // Like glTF, USDZ is a visualization/AR format, not a slicer input — no
  // solid-gating dialog, same posture as exportGltf above (module doc,
  // crates/mesh-export's "Non-solid inclusion": Scene::export always
  // includes non-solid geometry, the gating dialogs only ever warn ahead of
  // the STL/3MF slicer formats below).
  const exportUsdz = useCallback(async () => {
    const api = viewportApi.current
    if (api === null) {
      handleToast(t`Export failed: viewport not ready.`)
      return
    }
    let bytes: Uint8Array | null
    try {
      bytes = await api.exportUsdz()
    } catch (err: unknown) {
      handleToast(t`Export failed: ${ph({ reason: friendlyErrorText(err) })}`)
      return
    }
    if (bytes === null) {
      handleToast(t`Nothing to export — the model has no solids.`)
      return
    }
    // Suggest a name derived from the current document, dropping its file extension.
    const base = documentBaseName(docSessionRef.current)
    try {
      const exportedTo = await writeExport(fileHostRef.current, exportTargetRef, bytes, base, {
        // oxlint-disable-next-line lingui/no-unlocalized-strings -- file-format name
        description: 'USDZ',
        ext: 'usdz',
        mime: 'model/vnd.usdz+zip',
      })
      if (exportedTo !== null) {
        handleToast(t`Exported USDZ.`)
        LogStore.log.info('app', `Exported USDZ (${bytes.length} bytes)`)
      }
    } catch (err: unknown) {
      handleToast(t`Export failed: ${ph({ reason: friendlyErrorText(err) })}`)
    }
  }, [docSession.currentRef, docSession.importedName, handleToast])

  // ------------------------------------------------- slicer exports (STL/3MF)
  // Non-solid objects pending the solid-gating confirmation, plus which
  // format's export is waiting on it; null = no dialog. STL and 3MF feed
  // slicers, so both warn — never repair (rule 4) — when any exported object
  // is not a watertight solid.
  const [solidWarning, setSolidWarning] = useState<
    { format: 'stl' | '3mf'; names: string[]; ids: bigint[] } | null
  >(null)

  /** Curve resolution (segments per full turn; 0 = stored facets) chosen in
   *  the Export dialog, held across the solid-gating confirmation step. */
  const stlSegmentsRef = useRef(48)

  /** The actual export — runs directly when all objects are solid, or after
   *  "Export Anyway" in the gating dialog. */
  const doExportStl = useCallback(async () => {
    const api = viewportApi.current
    if (api === null) {
      handleToast(t`Export failed: viewport not ready.`)
      return
    }
    let bytes: Uint8Array | null
    try {
      bytes = await api.exportStl(stlSegmentsRef.current)
    } catch (err: unknown) {
      handleToast(t`Export failed: ${ph({ reason: friendlyErrorText(err) })}`)
      return
    }
    if (bytes === null) {
      handleToast(t`Nothing to export — the model has no solids.`)
      return
    }
    // Suggest a name derived from the current document, dropping its file extension.
    const base = documentBaseName(docSessionRef.current)
    try {
      const exportedTo = await writeExport(fileHostRef.current, exportTargetRef, bytes, base, {
        description: t`STL (Binary)`,
        ext: 'stl',
        mime: 'model/stl',
      })
      if (exportedTo !== null) {
        handleToast(t`Exported STL.`)
        LogStore.log.info('app', `Exported STL (${bytes.length} bytes)`)
      }
    } catch (err: unknown) {
      handleToast(t`Export failed: ${ph({ reason: friendlyErrorText(err) })}`)
    }
  }, [docSession.currentRef, docSession.importedName, handleToast])

  /** The actual 3MF export — runs directly when all objects are solid, or
   *  after "Export Anyway" in the gating dialog. */
  const doExport3mf = useCallback(async () => {
    const api = viewportApi.current
    if (api === null) {
      handleToast(t`Export failed: viewport not ready.`)
      return
    }
    let bytes: Uint8Array | null
    try {
      bytes = await api.export3mf()
    } catch (err: unknown) {
      handleToast(t`Export failed: ${ph({ reason: friendlyErrorText(err) })}`)
      return
    }
    if (bytes === null) {
      handleToast(t`Nothing to export — the model has no solids.`)
      return
    }
    // Suggest a name derived from the current document, dropping its file extension.
    const base = documentBaseName(docSessionRef.current)
    try {
      const exportedTo = await writeExport(fileHostRef.current, exportTargetRef, bytes, base, {
        // oxlint-disable-next-line lingui/no-unlocalized-strings -- file-format name
        description: '3MF',
        ext: '3mf',
        mime: 'model/3mf',
      })
      if (exportedTo !== null) {
        handleToast(t`Exported 3MF.`)
        LogStore.log.info('app', `Exported 3MF (${bytes.length} bytes)`)
      }
    } catch (err: unknown) {
      handleToast(t`Export failed: ${ph({ reason: friendlyErrorText(err) })}`)
    }
  }, [docSession.currentRef, docSession.importedName, handleToast])

  /** Entry point (Export dialog, STL/3MF formats): gate on solid status
   *  first. STL carries its chosen curve resolution (segments per turn)
   *  into `stlSegmentsRef`, held across the confirmation step. */
  const exportSolidGated = useCallback(async (format: 'stl' | '3mf', stlSegmentsPerTurn?: number) => {
    if (stlSegmentsPerTurn !== undefined) stlSegmentsRef.current = stlSegmentsPerTurn
    const scene = sceneRef.current
    const offenders = scene !== null ? collectNonSolidObjects(scene) : []
    if (offenders.length > 0) {
      setSolidWarning({ format, names: offenders.map((o) => o.name), ids: offenders.map((o) => o.id) })
      return
    }
    await (format === 'stl' ? doExportStl() : doExport3mf())
  }, [doExportStl, doExport3mf])

  // ---------------------------------------------------------------- unified Export dialog
  // File ▸ Export… opens ONE dialog with a Format select (glTF/STL/3MF/USDZ) —
  // the dialog's Export dispatches to the picked format; the slicer formats'
  // solid-gating dialog remains the follow-on step (chain unchanged).
  const [exportDialogOpen, setExportDialogOpen] = useState(false)
  const [printDialogOpen, setPrintDialogOpen] = useState(false)

  // ---------------------------------------------------------------- "Open on Phone" (docs/design/shop-mode.md §4)
  const [phoneShareOpen, setPhoneShareOpen] = useState(false)

  // The bytes+name PhoneShareDialog serializes and sends to the shell,
  // read once when the dialog mounts — the exact same `pushCameraStateToScene`
  // + `scene.save()` pair `saveDocument`/`saveAsDocument` use, and the same
  // base-name-plus-`.hew` derivation `saveAsDocument` suggests for `.hew`,
  // so the phone receives precisely what a real Save would have written.
  const getPhoneShareDocument = useCallback((): PhoneShareDocument | null => {
    const scene = sceneRef.current
    if (scene === null) return null
    pushCameraStateToScene(scene)
    // A shared document carries its id, so Shop Mode's per-document state
    // keys on the same identity as this one.
    scene.ensure_document_id()
    void reportDocumentId()
    const bytes = new Uint8Array(scene.save())
    // Always `.hew` bytes (a `.skp` document included), named `<base>.hew`.
    const name = nameForFormat(documentName(docSessionRef.current), 'hew')
    return { bytes, name }
  }, [pushCameraStateToScene, reportDocumentId])

  // File ▸ Open on Phone… is disabled on an empty document (nothing worth
  // sharing) — re-derived on every `docRev` bump (the same "something in
  // the document changed" signal the tree/tag panels re-read on), so a
  // draw/undo/redo on a previously-blank document flips it live rather
  // than only at the next unrelated re-render.
  const phoneShareDisabled = useMemo(() => {
    const scene = sceneRef.current
    return scene === null || isSceneEmpty(scene)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [docRev])

  // -------------------------------------------------------------------- 3D Text
  // Draw ▸ 3D Text… (docs/design/3d-text.md): the dialog resolves text/font/
  // height/depth; OK lays out the glyph run (font-aware, app-side) and hands
  // the plain-geometry result to the viewport, which arms a placement tool —
  // the dialog and this handler never touch the kernel directly.
  const [textDialogOpen, setTextDialogOpen] = useState(false)

  // 3D Text placement is one of the ops the kernel refuses outright while a
  // COMPONENT frame is open (`ExplodeSessionScope`: placing a component
  // instance is exactly the "instance placement" a definition — flat, no
  // sessions inside it — can't host). A group-only stack folds the placed
  // text in at close instead (docs/design/group-session.md), so this only
  // gates on `componentFrameOpen`. Gating the dialog's OPEN keeps the user
  // from ever reaching the refusal in normal use; both the MenuBar item
  // (disabled via `editGates.canDrawText`) and the command-palette/native-
  // menu dispatch route through this one function.
  const openTextDialog = useCallback(() => {
    if (componentFrameOpen) {
      // oxlint-disable-next-line lingui/no-unlocalized-strings -- kernel error code identifier
      handleToast(kernelErrorMessage('ExplodeSessionScope', ''), 'ExplodeSessionScope')
      return
    }
    setTextDialogOpen(true)
  }, [componentFrameOpen, handleToast])

  const handleTextPlace = useCallback((result: TextDialogResult) => {
    setTextDialogOpen(false)
    const { text, font, heightMeters, depthMeters } = result
    const contours = layoutGlyphRun(font.font, text, heightMeters)
    if (contours.length === 0 || !contours.some((c) => c.fill)) {
      handleToast(t`Nothing to place — the text has no visible strokes at this size.`)
      return
    }
    const label = text.replace(/\s+/g, ' ').trim()
    viewportApi.current?.armTextPlacement({
      contours,
      scale: heightMeters / font.font.unitsPerEm,
      depth: depthMeters,
      name: `3D Text "${label}"`,
    })
  }, [handleToast])

  // -------------------------------------------------------------------- Library
  // The Library dialog (`LibraryDialog.tsx`) only reports intent — every
  // action that actually touches the open document or the storage seam is
  // wired here (the "later integration wave" its own doc comment names).

  /** "Inserting <name>…" pill (frame 1c) shown while a one-shot placement is
   * armed; cleared by the arm callback on both cancel and commit. */
  const [insertingItem, setInsertingItem] = useState<string | null>(null)
  // Arming a NEW placement cancels the previous tool, whose onDone fires
  // during the takeover — a generation stamp keeps that late callback from
  // clearing the pill the new placement just set (adversarial review S13).
  const insertingGenRef = useRef(0)

  /** What the SaveToLibraryPopover is currently prompting for — a single
   * structural selection (ContextualDock's `save-to-library` verb) or the
   * whole document (File ▸ Save to Library…). `null` = popover closed. */
  const [saveToLibraryTarget, setSaveToLibraryTarget] = useState<
    | { kind: 'selection'; nodeKinds: Uint8Array; nodeIds: BigUint64Array; defaultName: string }
    | { kind: 'document'; defaultName: string }
    | null
  >(null)

  /** Lightweight per-item summary for the command palette's dynamic 'Library'
   * group (frame 1e) — NOT the full `LibraryItem` the dialog builds (no
   * thumbnail, no content hash): the palette only ever needs a label, a
   * category, and enough provenance to show "N in this model". */
  interface LibraryIndexEntry {
    fileName: string
    displayName: string
    category: LibraryCategory
    sourceId: string | null
    keywords: string[]
  }
  const [libraryIndex, setLibraryIndex] = useState<LibraryIndexEntry[]>([])
  const libraryIndexLoadedRef = useRef(false)

  const loadLibraryIndex = useCallback(async () => {
    const store = libraryStore()
    if (!store.available()) return
    try {
      const files = await store.list()
      const built: LibraryIndexEntry[] = []
      for (const file of files) {
        try {
          const bytes = await store.read(file.fileName)
          const summary = await readItemSummary(bytes)
          // contentHash is unused by the palette (only the dialog's
          // materialInPalette/placement-count lookups need it) — '' avoids
          // an unnecessary SHA-256 pass over every item on every reload.
          const item = buildLibraryItem(file, summary, '')
          built.push({
            fileName: file.fileName,
            displayName: item.displayName,
            category: item.category,
            sourceId: item.meta.id ?? null,
            keywords: item.meta.keywords ?? [],
          })
        } catch {
          // Unparsable file — a browser-detail-pane concern (it shows an
          // error tile); the palette just omits it.
        }
      }
      setLibraryIndex(built)
    } catch {
      // best effort — a listing failure just leaves the palette's Library
      // group empty, same as an empty library folder.
    }
  }, [])

  // Loaded lazily (first palette or dialog open), then kept fresh via the
  // store's own change subscription (a save/rename/delete from this window
  // OR another one — cross-window Library edits are real on desktop).
  useEffect(() => {
    if (!isTauri) return
    return libraryStore().subscribe(() => {
      void loadLibraryIndex()
    })
  }, [loadLibraryIndex])
  useEffect(() => {
    if (!paletteOpen && !showLibrary) return
    if (libraryIndexLoadedRef.current) return
    libraryIndexLoadedRef.current = true
    void loadLibraryIndex()
  }, [paletteOpen, showLibrary, loadLibraryIndex])

  // `hew.library` source_id -> live placement count in the open document,
  // for the dialog's "In this model" scope AND the palette's "N in this
  // model" description line. Re-read on every docRev (an insert/undo/redo
  // changes this) rather than incrementally tracked — the wasm call is a
  // cheap document-registry walk, not a re-tessellation.
  // Every "open the Library" entry point (menu, rail, palette, ⇧L). On the
  // desktop shell this opens/focuses the REAL native Library window (the
  // Settings-window pattern, resizable); the web build keeps its in-app
  // modal.
  const openLibraryEntry = useCallback(() => {
    if (isTauri) {
      void import('@tauri-apps/api/core').then(({ invoke }) => invoke('open_library_window'))
    } else {
      setShowLibrary((v) => !v)
    }
  }, [])

  const libraryPlacements = useMemo((): Record<string, number> => {
    const scene = state?.scene
    if (scene == null) return {}
    try {
      return JSON.parse(scene.library_placements_json()) as Record<string, number>
    } catch {
      return {}
    }
    // docRev: an insert/undo/redo changes placement counts without changing
    // scene identity.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [state, docRev])

  const paletteLibraryEntries = useMemo((): PaletteEntry[] => {
    return libraryIndex.map((entry): PaletteEntry => {
      const inModel = entry.sourceId !== null ? (libraryPlacements[entry.sourceId] ?? 0) : 0
      const isMaterial = entry.category === 'material'
      const displayName = entry.displayName
      const descParts: string[] = [entry.category]
      if (entry.keywords.length > 0) descParts.push(entry.keywords.join(', '))
      if (inModel > 0) descParts.push(plural(inModel, { one: '# in this model', other: '# in this model' }))
      return {
        id: `insert-library:${entry.fileName}`,
        label: isMaterial ? t`Add "${displayName}" to palette` : t`Insert "${displayName}"`,
        description: descParts.join(' — '),
        // oxlint-disable-next-line lingui/no-unlocalized-strings -- palette group ID
        group: 'Library',
        synonyms: entry.keywords,
      }
    })
  }, [libraryIndex, libraryPlacements, t])

  const paletteExtraEntries = useMemo(
    () => [...paletteModelEntries, ...paletteLibraryEntries],
    [paletteModelEntries, paletteLibraryEntries],
  )

  /** The display name a freshly-opened SaveToLibraryPopover should pre-fill
   * for a single structural selection — mirrors the Outliner/palette's own
   * `resolveLabel` fallbacks (`paletteModelEntries` above does the identical
   * walk per-node across the whole document; this is the same lookup for
   * one already-known node). */
  const librarySelectionDisplayName = useCallback(
    (node: NodeRef): string => {
      const scene = state?.scene
      if (scene == null) return t`Item`
      const index = sharedTreeIndex?.get(nodeKey(node)) ?? 0
      if (node.kind === 'object') return resolveLabel(scene.object_name(node.id), undefined, 'object', index)
      if (node.kind === 'group') return resolveLabel(scene.group_name(node.id), undefined, 'group', index)
      if (node.kind === 'instance') {
        const def = scene.instance_def(node.id)
        const defName = def !== undefined ? scene.component_name(def) : undefined
        return resolveLabel(scene.instance_name(node.id), defName, 'instance', index)
      }
      return t`Item`
    },
    [state, sharedTreeIndex, t],
  )

  /** Fire-and-forget thumbnail render + write, resolving `toastId` from a
   * "Rendering…" spinner to a "Saved" checkmark either way — a thumbnail
   * that fails to render is an honest no-thumbnail item (see
   * `renderItemThumbnail`'s doc comment), not a failed save. */
  const renderAndWriteThumbnail = useCallback((toastId: number, name: string, bytes: Uint8Array) => {
    void (async () => {
      try {
        const hash = await sha256Hex(bytes)
        const png = await renderItemThumbnail(bytes, 256)
        if (png !== null) await libraryStore().writeThumbnail(hash, png)
      } catch {
        // Best effort — the item itself already saved successfully.
      } finally {
        updateToast(toastId, { message: t`Saved "${name}" to library`, state: 'done', sticky: false })
      }
    })()
  }, [updateToast])

  /** Commits the SaveToLibraryPopover's chosen name: extracts the selection
   * or whole document as item bytes, writes it under a collision-proofed
   * file name, and (selection/document items only — materials render their
   * own swatch, see `handleSaveMaterialToLibrary`) kicks off the thumbnail
   * toast. Kernel refusals (an open component session, …) route through
   * kernelErrors.ts like every other call site. */
  const handleSaveToLibraryCommit = useCallback(
    (name: string) => {
      const target = saveToLibraryTarget
      setSaveToLibraryTarget(null)
      const scene = sceneRef.current
      if (scene === null || target === null) return
      const id = crypto.randomUUID()
      const meta = {
        id,
        name,
        category: target.kind === 'document' ? 'model' : 'component',
        savedAt: new Date().toISOString(),
        sourceDoc: documentName(docSessionRef.current),
      }
      let bytes: Uint8Array
      try {
        bytes =
          target.kind === 'document'
            ? scene.extract_document_item(JSON.stringify(meta))
            : scene.extract_item(target.nodeKinds, target.nodeIds, true, name, JSON.stringify(meta))
      } catch (err) {
        const code = parseKernelErrorCode(err)
        const rawMsg = err instanceof Error ? err.message : String(err)
        handleToast(kernelErrorMessage(code ?? 'Unknown', rawMsg), code ?? undefined)
        return
      }
      const fileName = itemFileName(name, id, target.kind === 'document' ? 'model' : 'component')
      // The stamp below runs after an await — if the user replaced the
      // document meanwhile (File ▸ New/Open), the captured node handles
      // could resolve against RECYCLED slots in the new document and stamp
      // the wrong entities (adversarial review). The session object's
      // identity changes exactly on document replacement, so capture it
      // now and bail if it moved.
      const sessionAtSave = docSessionRef.current
      void (async () => {
        try {
          await libraryStore().write(fileName, bytes)
        } catch (err) {
          handleToast(t`Save to Library failed: ${ph({ reason: err instanceof Error ? err.message : String(err) })}`)
          return
        }
        // Mark the SOURCE: the model an item was saved FROM contains it, by
        // definition — the selection (and its definition, for instances)
        // gets the item's provenance so "In this model" counts it
        // immediately and a re-insert reuses the existing definition
        // (playtest finding). Selection items only: a whole-document model
        // item has no meaningful in-model selection to mark.
        if (target.kind !== 'document' && docSessionRef.current === sessionAtSave) {
          let stamped = false
          try {
            // Capture cleanliness BEFORE the stamp dirties the session.
            const wasClean = !docSessionRef.current.dirty
            const ref = docSessionRef.current.currentRef
            const hash = await sha256Hex(bytes)
            const summary = await readItemSummary(bytes)
            // `stamp_library_source` pushes no undo entry (it's session
            // metadata, not a document action) — capture the undo depth
            // now so a write-through below can tell whether some OTHER,
            // genuinely undoable edit raced ahead of it before claiming
            // the kernel's saved mark (`markSavedIfUnmoved`'s doc comment).
            const undoDepthAtStamp = scene.undo_depth()
            scene.stamp_library_source(
              target.nodeKinds,
              target.nodeIds,
              id,
              hash,
              summary.first_component_sid ?? undefined,
            )
            stamped = true
            // The stamp is the "in this model" association — if it only
            // lives in memory, closing without a save silently severs it
            // (and the stamp-only "unsaved changes?" prompt reads as
            // spurious — playtest round 3). When the document was CLEAN
            // and bound to a REAL FILESYSTEM PATH (Tauri, string handle),
            // write the stamped bytes straight back to that file and stay
            // clean: the sole delta is the bookkeeping this very action
            // created. Anywhere else — web (an FSAA save can pop a
            // permission prompt or Save-As picker and rebind the
            // document), a handle-less session — stay on the ordinary
            // path: mark dirty, the user's next save carries the stamp.
            // `ref.format === 'hew'` is data-safety critical: this path
            // writes `scene.save()` — `.hew` bytes — straight to the file,
            // which for a `.skp`-bound document would replace the SketchUp
            // file with the wrong format. A `.skp` takes the ordinary path
            // (dirty; the user's next save writes the right container).
            const writeThrough =
              wasClean &&
              isTauri &&
              typeof ref?.handle === 'string' &&
              ref.format === 'hew' &&
              docSessionRef.current === sessionAtSave
            if (writeThrough) {
              // Camera-state parity with every other save path.
              pushCameraStateToScene(scene)
              scene.ensure_document_id()
              const docBytes = new Uint8Array(scene.save())
              const newRef = await fileHostRef.current.save(docBytes, ref)
              if (newRef !== null) {
                // An edit may have landed DURING the save — its dirty
                // state must never be clobbered clean (that would be
                // silent, autosave-suppressed data loss: the written
                // bytes predate the edit). Apply afterSave only when the
                // session is still exactly the one this flow captured, AND
                // only claim the kernel's saved mark when nothing moved
                // the undo stack out from under the stamp.
                const atSavedMark = markSavedIfUnmoved(scene, undoDepthAtStamp)
                setDocSession((prev) => applyWriteThroughSave(prev, sessionAtSave, newRef, Date.now(), atSavedMark))
                setDocRev((r) => r + 1)
              } else {
                // The write-through didn't land (host declined/cancelled) —
                // the stamp is still only in memory: non-undoable dirty,
                // same as the ordinary path below.
                reconcileRef.current()
                setDocSession((prev) => markNonUndoableDirty(prev, NON_UNDOABLE_REASON.libraryMetadata, Date.now()))
                setDocRev((r) => r + 1)
              }
            } else {
              // Ordinary path: the stamp landed but nothing wrote it to
              // disk — `at_saved_mark()` can't see it (no undo entry), so
              // it needs its own non-undoable-dirty reason.
              reconcileRef.current()
              setDocSession((prev) => markNonUndoableDirty(prev, NON_UNDOABLE_REASON.libraryMetadata, Date.now()))
              setDocRev((r) => r + 1)
            }
          } catch (err) {
            if (stamped) {
              // The DOCUMENT changed (the stamp landed) but the
              // write-through failed — surface it like any save failure
              // and leave the session honestly dirty so nothing is lost
              // silently.
              reconcileRef.current()
              setDocSession((prev) => markNonUndoableDirty(prev, NON_UNDOABLE_REASON.libraryMetadata, Date.now()))
              setDocRev((r) => r + 1)
              handleToast(t`Save failed: ${ph({ reason: friendlyErrorText(err) })}`)
            }
            // A failed stamp itself only costs the badge.
          }
        }
        const toastId = pushStickyToast(t`Rendering thumbnail for "${name}"…`)
        renderAndWriteThumbnail(toastId, name, bytes)
      })()
    },
    [saveToLibraryTarget, handleToast, pushStickyToast, renderAndWriteThumbnail, pushCameraStateToScene],
  )

  /** MaterialPalette's "Save to Library" button — no thumbnail render (a
   * pure-material item has nothing to render; `renderItemThumbnail` would
   * honestly return null), so this just writes and shows a plain toast. */
  const handleSaveMaterialToLibrary = useCallback(
    (materialId: bigint) => {
      const scene = sceneRef.current
      if (scene === null) return
      const info = scene.material_info(materialId)
      const name = info?.name() ?? 'Material'
      const id = crypto.randomUUID()
      const meta = {
        id,
        name,
        category: 'material',
        savedAt: new Date().toISOString(),
        sourceDoc: documentName(docSessionRef.current),
      }
      let bytes: Uint8Array
      try {
        bytes = scene.extract_material_item(materialId, JSON.stringify(meta))
      } catch (err) {
        const code = parseKernelErrorCode(err)
        const rawMsg = err instanceof Error ? err.message : String(err)
        handleToast(kernelErrorMessage(code ?? 'Unknown', rawMsg), code ?? undefined)
        return
      }
      const fileName = itemFileName(name, id, 'material')
      void (async () => {
        try {
          await libraryStore().write(fileName, bytes)
        } catch (err) {
          handleToast(t`Save to Library failed: ${ph({ reason: err instanceof Error ? err.message : String(err) })}`)
          return
        }
        handleToast(t`Saved "${name}" to library.`)
      })()
    },
    [handleToast],
  )

  const onLibraryInsert = useCallback(
    (item: LibraryItem, bytes: Uint8Array) => {
      setShowLibrary(false)
      void (async () => {
        let ghost: Awaited<ReturnType<typeof buildItemGhost>>
        try {
          ghost = await buildItemGhost(bytes)
        } catch (err) {
          handleToast(t`Couldn't preview "${ph({ name: item.displayName })}": ${ph({ reason: err instanceof Error ? err.message : String(err) })}`)
          return
        }
        const gen = ++insertingGenRef.current
        setInsertingItem(item.displayName)
        viewportApi.current?.armLibraryPlacement(
          {
            bytes,
            sourceId: item.meta.id ?? null,
            contentHash: item.contentHash,
            displayName: item.displayName,
            ghost: ghost.group,
            bboxMin: ghost.bboxMin,
            bboxMax: ghost.bboxMax,
          },
          () => {
            if (insertingGenRef.current === gen) setInsertingItem(null)
          },
        )
      })()
    },
    [handleToast],
  )

  // Open as Document: reuses THIS window only when it's pristine (same rule
  // as File ▸ Open — see `isPristineDocument`/`openDocument`). A non-pristine
  // window never gets silently replaced; the library folder's absolute path
  // (`itemPath`, built server-side from the SAME `resolved_library_dir` join
  // every other `library_*` command uses — main.rs's `library_item_path`)
  // lets this route through the exact `open_in_new_window` command File ▸
  // Open already uses for its own non-pristine case. A store/shell without
  // that path (web, or an older shell) falls back to an honest toast rather
  // than silently discarding the current document.
  const onLibraryOpenAsDocument = useCallback(
    (item: LibraryItem, bytes: Uint8Array) => {
      setShowLibrary(false)
      void (async () => {
        const scene = sceneRef.current
        if (scene === null) return
        const pristine = isPristineDocument(docSessionRef.current, scene)
        if (pristine) {
          const ok = applyLoadedBytes(bytes, { name: `${item.displayName}.hew`, label: OPEN_FILE_LABELS.hew })
          if (ok) setDocSession(afterImport(item.displayName, Date.now()))
          return
        }
        const cantRoute = () =>
          handleToast(t`Open the Library from an empty window to open this item as its own document.`)
        if (!isTauri) {
          cantRoute()
          return
        }
        const path = await libraryStore().itemPath(item.file.fileName)
        if (path === null) {
          cantRoute()
          return
        }
        try {
          const { invoke } = await import('@tauri-apps/api/core')
          await invoke('open_in_new_window', {
            path,
            asCopy: true,
            name: item.displayName,
          })
        } catch {
          cantRoute()
        }
      })()
    },
    [applyLoadedBytes, handleToast],
  )

  const onLibraryPaintWith = useCallback(
    (_item: LibraryItem, bytes: Uint8Array) => {
      const scene = sceneRef.current
      if (scene === null) return
      let ids: BigUint64Array
      try {
        ids = scene.insert_item_palette(bytes)
      } catch (err) {
        const code = parseKernelErrorCode(err)
        const rawMsg = err instanceof Error ? err.message : String(err)
        handleToast(kernelErrorMessage(code ?? 'Unknown', rawMsg), code ?? undefined)
        return
      }
      // Palette entries serialize into the document — mark it dirty like
      // any other committed mutation (adversarial review S10), or a
      // close/quit after "Paint with this" silently drops the material.
      // The material-registry write itself pushes no undo entry
      // (conformance's `material_and_tag_registry_writes_add_no_undo_entry`),
      // so `handleDocumentChanged`'s `at_saved_mark()` read alone can't see
      // it — an explicit non-undoable reason, same as Scenes.
      handleDocumentChanged()
      setDocSession((s) => markNonUndoableDirty(s, NON_UNDOABLE_REASON.libraryMaterials, Date.now()))
      setDocRev((r) => r + 1)
      if (ids.length === 0) return
      setCurrentMaterialId(ids[0])
      setActiveTool('Paint')
      setShowLibrary(false)
    },
    [handleToast, handleDocumentChanged],
  )

  const onLibraryAddToPalette = useCallback(
    (item: LibraryItem, bytes: Uint8Array) => {
      const scene = sceneRef.current
      if (scene === null) return
      let ids: BigUint64Array
      try {
        ids = scene.insert_item_palette(bytes)
      } catch (err) {
        const code = parseKernelErrorCode(err)
        const rawMsg = err instanceof Error ? err.message : String(err)
        handleToast(kernelErrorMessage(code ?? 'Unknown', rawMsg), code ?? undefined)
        return
      }
      // Dirty for the same reason as Paint-with-this (adversarial review
      // S10): the palette rides the document, and the same non-undoable
      // registry write applies.
      handleDocumentChanged()
      setDocSession((s) => markNonUndoableDirty(s, NON_UNDOABLE_REASON.libraryMaterials, Date.now()))
      setDocRev((r) => r + 1)
      if (ids.length > 0) handleToast(t`Added "${ph({ name: item.displayName })}" to palette.`)
    },
    [handleToast, handleDocumentChanged],
  )

  const libraryMaterialInPalette = useCallback(
    async (bytes: Uint8Array): Promise<boolean> => {
      const scene = sceneRef.current
      if (scene === null) return false
      try {
        const matches = JSON.parse(scene.palette_matches_json(bytes)) as (string | null)[]
        return matches.length > 0 && matches.every((v) => v !== null)
      } catch {
        return false
      }
    },
    // `docRev` (not `[]`): LibraryDialog's "in palette" effect keys off THIS
    // callback's identity to know when to re-check (see its
    // `[open, items, materialInPalette]` deps) — a permanently-stable
    // identity meant "Add to palette" (or an undo/redo that adds/removes a
    // palette entry) never refreshed the modal's blue dot, "In this model"
    // line, or in-model scope filter until the dialog was closed and
    // reopened. `docRev` bumps on every palette mutation and on every
    // undo/redo (see `handleDocumentChanged`/`onLibraryAddToPalette`), so a
    // new identity here is exactly the live-refresh signal the window
    // variant already gets for free from its own inline-lambda prop.
    [docRev],
  )

  // SVG line drawing (docs/design/printing.md §7b): the kernel's hidden-line
  // drawing from a print camera, written as a true-size SVG at a drawing
  // scale — the same line work a Line-art print uses.
  const exportSvg = useCallback(async (svg: SvgExportOptions) => {
    const api = viewportApi.current
    const scene = sceneRef.current
    if (api === null || scene === null) {
      handleToast(t`Export failed: viewport not ready.`)
      return
    }
    const live = api.getPrintView()
    const { dir, up } = printViewDirection(svg.view, live)
    const extent = api.computePrintExtent(dir, up, { includeSketches: true, includeAnnotations: svg.includeDimensions, restrictTo: null })
    if (extent.empty) {
      handleToast(t`Nothing to export — the model has nothing visible.`)
      return
    }
    const camera = { kind: 'ortho' as const, center: extent.center, dir, up, rect: extent.rect, depth: extent.depth }
    const section = api.getSectionState()
    const res = requestLineDrawing({
      scene,
      camera,
      section: section !== null && section.active ? { origin: section.origin, normal: section.normal } : null,
      hidden: api.getHiddenIds(),
      only: null,
      includeHidden: svg.hiddenDashed,
    })
    if (res.kind !== 'ok') {
      handleToast(res.kind === 'too-complex' ? t`Export failed: the model is too complex for a vector line drawing.` : t`Export failed: ${ph({ reason: res.message })}`)
      return
    }
    const annotations = svg.includeDimensions ? projectAnnotationOverlay(api.collectAnnotationDrawing(), camera) : null
    const text = lineArtSvgDocument(res.drawing, scaleRatio(svg.scale), { hiddenDashed: svg.hiddenDashed, annotations })
    const bytes = new TextEncoder().encode(text)
    const base = documentBaseName(docSessionRef.current)
    try {
      const exportedTo = await writeExport(fileHostRef.current, exportTargetRef, bytes, base, { description: t`SVG line drawing`, ext: 'svg', mime: 'image/svg+xml' })
      if (exportedTo !== null) {
        handleToast(t`Exported SVG.`)
        LogStore.log.info('app', `Exported SVG line drawing (${bytes.length} bytes, ${res.drawing.kinds.length} segments)`)
      }
    } catch (err: unknown) {
      handleToast(t`Export failed: ${ph({ reason: friendlyErrorText(err) })}`)
    }
  }, [docSession.currentRef, docSession.importedName, handleToast])

  // SketchUp export (`.skp`, the 2026 container or a 2017 to 2020 release): a copy
  // written out through `Scene.export_skp`, which leaves the document's own
  // `.skp` source alone. Whatever the file could not hold exactly is listed
  // in the same notice a `.skp` Save shows — every time (an export is a
  // deliberate act, not a recurring save), and with no "Save a .hew copy".
  const exportSkp = useCallback(async (format: SkpExportFormat, compat = false) => {
    const scene = sceneRef.current
    if (scene === null) {
      handleToast(t`Export failed: viewport not ready.`)
      return
    }
    let out: SkpWrite
    try {
      pushCameraStateToScene(scene)
      // The dialog's `skp` is the 2026 container (the wasm target's name).
      out = scene.export_skp(format === 'skp' ? 'skp2026' : format, getLengthUnit(), format === 'skp' && compat) as SkpWrite
    } catch (err: unknown) {
      handleToast(t`Export failed: ${ph({ reason: friendlyErrorText(err) })}`)
      return
    }
    const base = documentBaseName(docSessionRef.current)
    try {
      const exportedTo = await writeExport(fileHostRef.current, exportTargetRef, out.bytes, base + '.skp', {
        description: t`SketchUp model`,
        ext: 'skp',
        mime: 'application/vnd.sketchup.skp',
      })
      if (exportedTo !== null) {
        handleToast(t`Exported SketchUp file.`)
        LogStore.log.info('app', `Exported ${format} (${out.bytes.length} bytes)`)
        if (out.simplified.length > 0 || out.writer_notes.length > 0) {
          setSkpNotice({ simplified: out.simplified, writerNotes: out.writer_notes, offerHewCopy: false })
        }
      }
    } catch (err: unknown) {
      handleToast(t`Export failed: ${ph({ reason: friendlyErrorText(err) })}`)
    }
  }, [handleToast, pushCameraStateToScene])

  const handleExportFormat = useCallback((format: ExportFormat, stlSegmentsPerTurn: number, svg?: SvgExportOptions, skpCompat = false) => {
    setExportDialogOpen(false)
    if (isSkpExportFormat(format)) {
      void exportSkp(format, skpCompat)
    } else if (format === 'glb') {
      void exportGltf()
    } else if (format === 'usdz') {
      void exportUsdz()
    } else if (format === 'svg') {
      if (svg !== undefined) void exportSvg(svg)
    } else {
      void exportSolidGated(format, format === 'stl' ? stlSegmentsPerTurn : undefined)
    }
  }, [exportGltf, exportUsdz, exportSolidGated, exportSvg, exportSkp])

  // File ▸ Export…: on the desktop, the platform's own save panel with the
  // format list and the chosen format's options in it (io/savePanel.ts);
  // in the browser, which has no such panel, the in-app Export dialog.
  const openExport = useCallback(async () => {
    if (!hasNativeSavePanel() || fileHostRef.current.writeTo === undefined) {
      setExportDialogOpen(true)
      return
    }
    exportTargetRef.current = null
    const unit = getLengthUnit()
    const initial = lastExportFormatRef.current
    const extension = isSkpExportFormat(initial) ? 'skp' : initial
    const name = `${documentBaseName(docSessionRef.current)}.${extension}`
    try {
      const pick = await pickSaveTarget(t`Export`, name, exportPanelFormats(unit, initial, skpCompatRef.current), exportPanelIdFor(initial))
      if (pick === null) return
      const choice = exportChoiceFrom(pick, unit)
      if (choice === null) return
      lastExportFormatRef.current = choice.format
      exportTargetRef.current = pick.path
      handleExportFormat(choice.format, choice.stlSegmentsPerTurn, choice.svg, choice.skpCompat)
    } catch (err: unknown) {
      handleToast(t`Export failed: ${ph({ reason: friendlyErrorText(err) })}`)
    }
  }, [handleExportFormat, handleToast])

  // ---------------------------------------------------------------- settings window
  // Per-platform settings surface:
  //  - Windows desktop: a full-window in-app page in the Windows 11 app-
  //    settings idiom (back arrow + settings cards, like Notepad/Paint) —
  //    FluentSettingsPage below.
  //  - macOS/Linux desktop: a separate, free-floating OS webview window
  //    (macOS HIG expects a standalone settings window).
  //  - Web: no concept of a second OS window — an in-app modal
  //    (showSettingsModal below).
  const openSettings = useCallback(() => {
    if (isTauri && isWindows) {
      setShowFluentSettings(true)
      return
    }
    if (isTauri) {
      // Created by the shell (open_settings_window) rather than the JS
      // WebviewWindow API, so the capability set carries no window-creation
      // grants — a compromised webview must not be able to mint windows.
      import('@tauri-apps/api/core')
        .then(({ invoke }) => invoke('open_settings_window'))
        .catch(() => { /* ignore */ })
      return
    }
    setShowSettingsModal(true)
  }, [])

  // ---------------------------------------------------------------- stable refs for Tauri event listeners
  // These refs always track the latest callbacks so Tauri event handlers
  // (registered once) don't capture stale closures.
  const newDocumentRef = useRef(newDocument)
  const openDocumentRef = useRef(openDocument)
  const importDocumentRef = useRef(importDocument)
  const importImageRef = useRef(importImage)
  const startImagePlacementRef = useRef(startImagePlacement)
  // saveDocumentRef/saveAsDocumentRef are declared earlier (with docSessionRef)
  // so confirmDiscard/the unsaved-changes dialog can use them too — this
  // block only keeps them current, like every other ref here.
  const handleUndoRef = useRef(handleUndo)
  const handleRedoRef = useRef(handleRedo)
  const handleZoomExtentsRef = useRef(handleZoomExtents)
  const openPathRef = useRef(openPath)
  const openSettingsRef = useRef(openSettings)
  useEffect(() => { newDocumentRef.current = newDocument }, [newDocument])
  useEffect(() => { openDocumentRef.current = openDocument }, [openDocument])
  useEffect(() => { importDocumentRef.current = importDocument }, [importDocument])
  useEffect(() => { importImageRef.current = importImage }, [importImage])
  useEffect(() => { startImagePlacementRef.current = startImagePlacement }, [startImagePlacement])
  useEffect(() => { saveDocumentRef.current = saveDocument }, [saveDocument])
  useEffect(() => { saveAsDocumentRef.current = saveAsDocument }, [saveAsDocument])
  useEffect(() => { handleUndoRef.current = handleUndo }, [handleUndo])
  useEffect(() => { handleRedoRef.current = handleRedo }, [handleRedo])
  useEffect(() => { handleZoomExtentsRef.current = handleZoomExtents }, [handleZoomExtents])
  useEffect(() => { openPathRef.current = openPath }, [openPath])
  useEffect(() => { openSettingsRef.current = openSettings }, [openSettings])

  // The webview half of the live API's window management (docs/agents/
  // HEW_API.md §4.2, `shells/tauri/src-tauri/src/live_router.rs`): the shell
  // creates and routes windows, but only this window knows whether its
  // document has unsaved work and can load a file through the same open
  // path the UI uses.
  /* oxlint-disable lingui/no-unlocalized-strings -- live-API refusal messages are a developer surface, English */
  useEffect(() => {
    liveOpRef.current = async (op, args): Promise<LiveOpOutcome> => {
      // A plugin command holds the document: saving would write it half
      // done, and open, revert, and close would replace it under the
      // command (docs/agents/PLUGINS.md "Compounds").
      if ((op === 'save' || op === 'open' || op === 'revert' || op === 'close') && sceneRef.current?.busy()) {
        return { ok: false, refusal: 'document_busy', message: 'a plugin command is running on this document' }
      }
      if (op === 'save') {
        // `hew.doc.save`: a named path saves (and rebinds) there; none
        // re-saves in place (the shell only asks when the document has a file).
        // A path saves in the format its extension (or `format`) says; none
        // re-saves the document's own file in its own format.
        const path = typeof args.path === 'string' ? args.path : null
        const current = docSession.currentRef
        const units = typeof args.units === 'string' ? args.units : undefined
        let ref: FileRef | null = current
        if (path !== null || args.format !== undefined) {
          const target = path ?? (typeof current?.handle === 'string' ? current.handle : null)
          if (target === null) {
            return { ok: false, refusal: 'save_failed', message: 'the document has no file to save to' }
          }
          const format = saveFormatForPath(target, args.format, current?.format)
          if (format === null) {
            return {
              ok: false,
              refusal: 'format_mismatch',
              message: `${String(args.format)} contradicts the extension of ${basenameOf(target)}`,
            }
          }
          ref = { name: basenameOf(target), format, handle: target }
        }
        if (ref === null) {
          return { ok: false, refusal: 'save_failed', message: 'the document has no file to save to' }
        }
        // `compat` is for the 2021-and-later container only; an explicit
        // `compat:false` is the same as absent (a revision-17 file stays so).
        if (args.compat === true && ref !== null && !takesCompat(ref.format)) {
          return {
            ok: false,
            refusal: 'format_mismatch',
            message: '`compat` applies to the 2021-and-later container (format "skp")',
          }
        }
        if (args.copy === true) {
          // Save a copy: write the bytes, change nothing about this document
          // (no rebinding to the file, no clean mark, no new SketchUp source:
          // `save_skp` only stages one, and nothing commits it).
          const scene = sceneRef.current
          if (scene === null) return { ok: false, refusal: 'save_failed', message: 'not ready' }
          pushCameraStateToScene(scene)
          try {
            // A copy carries the document's id (minted here if this is the
            // first write; `save_skp` mints its own).
            if (!isSkpFormat(ref.format)) scene.ensure_document_id()
            const bytes = isSkpFormat(ref.format)
              ? (scene.save_skp(skpTarget(ref.format), units ?? getLengthUnit(), units !== undefined, takesCompat(ref.format) && (args.compat === true || (args.format === undefined && skpCompatRef.current))) as SkpWrite).bytes
              : new Uint8Array(scene.save())
            void reportDocumentId()
            await fileHostRef.current.save(bytes, ref)
            return { ok: true }
          } catch (err) {
            return {
              ok: false,
              refusal: 'save_failed',
              message: err instanceof Error ? err.message : String(err),
            }
          }
        }
        lastSaveErrorRef.current = 'the document could not be saved'
        const saved = await saveToRef(ref, units, args.compat === true ? true : args.format !== undefined ? false : undefined)
        return saved
          ? { ok: true }
          : { ok: false, refusal: 'save_failed', message: lastSaveErrorRef.current }
      }
      if (op === 'open') {
        const path = typeof args.path === 'string' ? args.path : ''
        try {
          const landed = await openPathInPlace(path)
          return landed
            ? { ok: true }
            : { ok: false, refusal: 'load_failed', message: 'the file could not be loaded' }
        } catch (err) {
          return {
            ok: false,
            refusal: 'load_failed',
            message: err instanceof Error ? err.message : String(err),
          }
        }
      }
      if (op === 'revert') {
        if (!canRevertToSaved(docSessionRef.current)) {
          return {
            ok: false,
            refusal: 'nothing_to_revert',
            message: docSessionRef.current.dirty
              ? 'There is nothing to revert to: this document has not been saved to a file yet.'
              : 'There is nothing to revert: the document has no changes since the last save.',
          }
        }
        const reverted = await revertToSaved()
        return reverted.ok ? { ok: true } : { ok: false, refusal: 'load_failed', message: reverted.message }
      }
      if (op === 'close') {
        const refuse = (): LiveOpOutcome => ({
          ok: false,
          refusal: 'unsaved_changes',
          message: 'The document has unsaved changes; pass discard:true to close it anyway.',
        })
        if (docSessionRef.current.dirty && args.discard !== true) return refuse()
        // Dropping the autosave snapshot takes a moment; the user (or another
        // client) may edit meanwhile, so decide again afterwards, before the
        // answer that commits to destroying the window.
        await clearRecoverySnapshot()
        if (docSessionRef.current.dirty && args.discard !== true) return refuse()
        return {
          ok: true,
          afterReply: async () => {
            const { getCurrentWindow } = await import('@tauri-apps/api/window')
            await getCurrentWindow().destroy()
          },
        }
      }
      return { ok: false, refusal: 'host_capability_missing', message: `unknown operation ${op}` }
    }
  }, [openPathInPlace, clearRecoverySnapshot, saveToRef, docSession.currentRef, pushCameraStateToScene, revertToSaved, reportDocumentId])
  /* oxlint-enable lingui/no-unlocalized-strings */

  // ---------------------------------------------------------------- native menu-action dispatch
  // The dispatch switch lives in a ref refreshed every render. The listener
  // below is registered ONCE (so it survives StrictMode and re-renders without
  // re-subscribing), but Vite Fast Refresh re-renders this component WITHOUT
  // re-running a []-deps effect — so a switch captured inside that effect would
  // be frozen at its first-mount version and silently drop any case added later
  // (e.g. a newly-added pane toggle). Routing through this ref keeps it current.
  // Whole-node delete: deletes every currently-selected node
  // (Object/Group/Instance) and clears the selection. Shared by the Edit ▸
  // Delete menu item and the Delete/Backspace key handler below — both
  // dispatch through `menuActionRef.current('edit-delete')` so this always
  // sees the current `selectedIds` (the ref is reassigned fresh every render;
  // see the Fast-Refresh note above `menuActionRef`).
  const deleteSelection = () => {
    // A selected guide/annotation deletes via the same path; mutually
    // exclusive with node selection.
    if (selectedGuide !== null) {
      viewportApi.current?.runDeleteGuide(selectedGuide)
      setSelectedGuide(null)
      setDocRev((r) => r + 1)
      return
    }
    if (selectedAnnotation !== null) {
      viewportApi.current?.runDeleteAnnotation(selectedAnnotation)
      setSelectedAnnotation(null)
      setDocRev((r) => r + 1)
      return
    }
    if (selectedIds.length === 0) return
    viewportApi.current?.runDelete(selectedIds)
    setSelectedIds([])
    setDocRev((r) => r + 1)
  }

  // ── Copy / Cut / Paste / Paste In Place (Lane D) ─────────────────────────
  // A stable per-mount tag for the clipboard content's informational
  // `sourceDocId` (never read by `insert_item` itself — see modelClipboard's
  // doc comment).
  const clipboardSourceIdRef = useRef<string>(
    typeof crypto.randomUUID === 'function' ? crypto.randomUUID() : String(Date.now()),
  )

  const isStructuralNode = (n: NodeRef): boolean =>
    n.kind === 'object' || n.kind === 'group' || n.kind === 'instance'

  const SKETCH_COPY_TOAST = t`Sketch geometry can't be copied yet — use Move with Option/Alt held`

  /** Copy: `extract_item` (unwrapped — a copied plain box stays a plain box,
   *  unlike Save to Library's component wrap) of the selection's structural
   *  subset, then store it. Returns the copied nodes on success (so Cut can
   *  delete exactly what was actually captured) or `null` on refusal/error —
   *  Cut must NOT delete anything when this fails, or the geometry is lost
   *  with no backup on the clipboard. */
  const doCopy = (): NodeRef[] | null => {
    const structural = selectedIds.filter(isStructuralNode)
    if (structural.length === 0) {
      if (selectedIds.length > 0) handleToast(SKETCH_COPY_TOAST)
      return null
    }
    const scene = sceneRef.current
    if (scene === null) return null
    const sel = structuralSelection(structural)
    if (sel === null) return null // unreachable — every entry is already structural
    let bytes: Uint8Array
    try {
      // The window nonce makes this a CLIPBOARD copy: the kernel stamps
      // every definition the copy carries with where it came from, so a
      // paste back into this same document (handlePaste passes the same
      // nonce) places the ORIGINAL definition — a linked instance, as
      // Move+copy makes — instead of minting an unlinked duplicate. The
      // scene composes its document epoch in, so a copy taken before
      // File ▸ Open/New in this window never links into the new document.
      bytes = scene.extract_item(sel.kinds, sel.ids, false, undefined, undefined, clipboardSourceIdRef.current)
    } catch (err) {
      const code = parseKernelErrorCode(err)
      const rawMsg = err instanceof Error ? err.message : String(err)
      handleToast(kernelErrorMessage(code ?? 'Unknown', rawMsg), code ?? undefined)
      return null
    }
    const kinds = [...new Set(structural.map((n) => n.kind))]
    // Paste In Place's affine (see ClipboardContent.placementAffine): the
    // kernel's own reconstruction of this exact selection's original world
    // position — `Scene.extract_item_placement` runs the SAME logic
    // `extract_item` used above (crates/kernel's `extract_placement`/
    // `visible_world_bottom_center`), covering BOTH the identity-axes
    // re-origin case and the moved-axes case, so this can never drift from
    // what extract_item actually did (hidden-descendant selections
    // included) and Paste In Place never has to special-case either axes
    // state itself. `undefined` (no visible content) falls back to `null`.
    const placement = scene.extract_item_placement(sel.kinds, sel.ids)
    const placementAffine = placement === undefined ? null : Array.from(placement)
    const copiedCount = structural.length
    void setClipboardBytes(bytes, structural.length, kinds, clipboardSourceIdRef.current, placementAffine).then(() => {
      setClipboardAvailable(true)
      handleToast(plural(copiedCount, { one: 'Copied # object', other: 'Copied # objects' }))
    })
    return structural
  }

  const handleCopy = () => {
    doCopy()
  }

  const handleCut = () => {
    const copied = doCopy()
    if (copied === null) return
    viewportApi.current?.runDelete(copied)
    setSelectedIds((cur) => cur.filter((n) => !copied.some((c) => nodeEq(c, n))))
    setDocRev((r) => r + 1)
  }

  /** Paste (cursor placement) / Paste In Place (identity affine, no
   *  gesture). Both resolve the clipboard the same way: this window's own
   *  cache first, else a fresh pull from the shared Tauri shell state (a
   *  cross-window paste with nothing copied locally yet). */
  const handlePaste = (inPlace: boolean) => {
    void (async () => {
      // ALWAYS await a fresh pull before reading — never trust the local
      // cache outright. Under Tauri, the focus-triggered refresh effect
      // may still be in flight (switch straight into this window and hit
      // ⌘V before its `clipboard_get` round trip lands): reading local
      // `current` unconditionally here could hand back a stale
      // cross-window clipboard. `refreshClipboardFromShell` is a no-op on
      // the web build and cheap (and correctly a no-op) when this
      // window's own cache already matches the shell's bytes — see its
      // doc comment.
      await refreshClipboardFromShell()
      const content = getClipboard()
      if (content === null) return // nothing to paste; the shortcut/menu is a silent no-op
      const scene = sceneRef.current
      if (scene === null) return

      if (inPlace) {
        // The kernel-computed reconstruction affine — see
        // `ClipboardContent.placementAffine`'s doc comment for why a bare
        // identity affine would NOT overlap the original in EITHER axes
        // state extract_item has (identity re-origin, or a moved frame).
        // `null` (no content to anchor on) falls back to plain identity.
        const affine = new Float64Array(
          content.placementAffine ?? [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0],
        )
        let raw: unknown
        try {
          raw = scene.insert_item(
            content.bytes,
            affine,
            content.sourceId,
            content.contentHash,
            clipboardSourceIdRef.current,
          )
        } catch (err) {
          const code = parseKernelErrorCode(err)
          const rawMsg = err instanceof Error ? err.message : String(err)
          handleToast(kernelErrorMessage(code ?? 'Unknown', rawMsg), code ?? undefined)
          return
        }
        const result = parseInsertResult(raw)
        // Additive kernel mutation made outside a tool — refreshScene is the
        // harness's own choke point for exactly this (re-tessellate +
        // reconcile + docRev, see ViewportApi.refreshScene's doc comment).
        viewportApi.current?.refreshScene()
        const refs: NodeRef[] = result.rootKinds.map((k, i) => ({
          kind: k === 0 ? 'object' as const : k === 1 ? 'group' as const : 'instance' as const,
          id: result.rootIds[i],
        }))
        setSelectedGuide(null)
        setSelectedAnnotation(null)
        setSelectedIds(refs)
        return
      }

      // Cursor placement: arm the same one-shot LibraryPlaceTool the Library
      // browser uses, with a ghost built straight from the clipboard bytes.
      //
      // A linked paste places the LIVE definition, which may have been
      // edited since Copy, so the ghost (and the bounds the snap and the
      // origin cue use) come from the kernel's preview of what the paste
      // will place. No preview (nothing links, or it cannot be built)
      // means the copy-time bytes already are the paste.
      let previewBytes = content.bytes
      try {
        const live = scene.clipboard_paste_preview(content.bytes, clipboardSourceIdRef.current)
        if (live !== undefined) previewBytes = live
      } catch {
        // Fall back to the copy-time bytes; the placement itself decides.
      }
      let ghost: Awaited<ReturnType<typeof buildItemGhost>>
      try {
        ghost = await buildItemGhost(previewBytes)
      } catch (err) {
        handleToast(t`Couldn't preview the paste: ${ph({ reason: err instanceof Error ? err.message : String(err) })}`)
        return
      }
      viewportApi.current?.armLibraryPlacement({
        bytes: content.bytes,
        sourceId: content.sourceId,
        contentHash: content.contentHash,
        // The paste half of the clipboard link (see doCopy): the same
        // window nonce, so an instance copied from THIS document places
        // its original definition rather than an unlinked copy.
        origin: clipboardSourceIdRef.current,
        displayName: t`Pasted objects`,
        ghost: ghost.group,
        bboxMin: ghost.bboxMin,
        bboxMax: ghost.bboxMax,
      })
    })()
  }

  const menuActionRef = useRef<(payload: string) => void>(() => {})
  menuActionRef.current = (payload: string) => {
    // A plugin command holds the document (docs/agents/PLUGINS.md
    // "Compounds"): only window focus, settings, updates, and help run.
    // The palette's actions come through here too.
    if (
      sceneRef.current?.busy() &&
      !(
        payload.startsWith('win-focus:') ||
        payload === 'app-settings' ||
        payload === 'app-check-updates' ||
        payload.startsWith('help-')
      )
    ) {
      return
    }
    // Palette "jump" entries (dynamic Model group) carry their target in the
    // id. Nodes: select + reveal in the Outliner/Object Info (the tree
    // scrolls its primary selection into view). Tags: reveal + flash in the
    // Tags panel.
    if (payload.startsWith('jump-node:')) {
      const [, kindStr, idStr] = payload.split(':')
      const kind = kindStr as NodeRef['kind']
      if ((kind === 'object' || kind === 'group' || kind === 'instance') && /^\d+$/.test(idStr)) {
        setSelectedGuide(null)
        setSelectedAnnotation(null)
        setSelectedIds([{ kind, id: BigInt(idStr) }])
        setShowModelInfo(true)
        setShowObjectInfo(true)
        showTray()
      }
      return
    }
    if (payload.startsWith('jump-tag:')) {
      const key = payload.slice('jump-tag:'.length)
      setShowTags(true)
      showTray()
      const nonce = ++revealNonceRef.current
      setRevealTag({ key, nonce })
      // Let the highlight fade back out unless another jump superseded it.
      setTimeout(() => {
        setRevealTag((cur) => (cur?.nonce === nonce ? null : cur))
      }, 2000)
      return
    }
    // Command palette's dynamic 'Library' group (frame 1e): Enter goes
    // straight to cursor placement (materials: straight into the palette) —
    // the Library modal never opens for this path, unlike a tile click in
    // the dialog itself.
    if (payload.startsWith('insert-library:')) {
      const fileName = payload.slice('insert-library:'.length)
      const entry = libraryIndex.find((e) => e.fileName === fileName)
      void (async () => {
        const store = libraryStore()
        if (!store.available()) return
        let bytes: Uint8Array
        try {
          bytes = await store.read(fileName)
        } catch (err) {
          handleToast(t`Couldn't read library item: ${ph({ reason: err instanceof Error ? err.message : String(err) })}`)
          return
        }
        if (entry?.category === 'material') {
          const scene = sceneRef.current
          if (scene === null) return
          let ids: BigUint64Array
          try {
            ids = scene.insert_item_palette(bytes)
          } catch (err) {
            const code = parseKernelErrorCode(err)
            const rawMsg = err instanceof Error ? err.message : String(err)
            handleToast(kernelErrorMessage(code ?? 'Unknown', rawMsg), code ?? undefined)
            return
          }
          // Palette entries serialize into the document (adversarial
          // review S10) — same dirty-marking as the dialog's flows.
          reconcileRef.current()
          setDocRev((r) => r + 1)
          if (ids.length > 0) handleToast(t`Added "${ph({ name: entry.displayName })}" to palette.`)
          return
        }
        const displayName = entry?.displayName ?? fileName
        let ghost: Awaited<ReturnType<typeof buildItemGhost>>
        try {
          ghost = await buildItemGhost(bytes)
        } catch (err) {
          handleToast(t`Couldn't preview "${displayName}": ${ph({ reason: err instanceof Error ? err.message : String(err) })}`)
          return
        }
        const contentHash = await sha256Hex(bytes)
        const gen = ++insertingGenRef.current
        setInsertingItem(displayName)
        viewportApi.current?.armLibraryPlacement(
          {
            bytes,
            sourceId: entry?.sourceId ?? null,
            contentHash,
            displayName,
            ghost: ghost.group,
            bboxMin: ghost.bboxMin,
            bboxMax: ghost.bboxMax,
          },
          () => {
            if (insertingGenRef.current === gen) setInsertingItem(null)
          },
        )
      })()
      return
    }
    switch (payload) {
      case 'new':      newDocumentRef.current(); break
      case 'open':     openDocumentRef.current(); break
      case 'import':   importDocumentRef.current(); break
      case 'import-image': void importImageRef.current(); break
      case 'export':   void openExport(); break
      case 'print':    setPrintDialogOpen(true); break
      // macOS native "Open on Phone…" — always enabled natively (macOS has
      // no live disabled-state push for it, same posture as
      // save-to-library-doc), so the empty-document gate the web MenuBar
      // enforces via `openOnPhoneDisabled` is enforced here with a toast.
      case 'open-on-phone':
        if (phoneShareDisabled) handleToast(t`Nothing to share yet — the document is empty`)
        else setPhoneShareOpen(true)
        break
      case 'draw-3d-text': openTextDialog(); break
      case 'save':     saveDocumentRef.current(); break
      case 'save-as':  saveAsDocumentRef.current(); break
      case 'revert-to-saved':
        if (canRevertToSaved(docSessionRef.current)) setRevertPromptOpen(true)
        break
      case 'undo':     handleUndoRef.current(); break
      case 'redo':     handleRedoRef.current(); break
      // On macOS the native Edit items carry key equivalents and are
      // ungated, so a Cmd+C/X/A pressed while a text field has focus lands
      // HERE (WebKit gave the page the key first; the field itself has no
      // editing key equivalents of its own) — forward it to the field's
      // editing command instead of the scene. Paste In Place in a field is
      // simply not ours. See main.rs's Edit menu comment.
      case 'edit-select-all':
        // oxlint-disable-next-line lingui/no-unlocalized-strings -- editing command name
        if (forwardToTextField('selectAll')) break
        // The welcome screen is modal: on the macOS desktop AppKit runs the
        // native Select All key equivalent before the page can swallow it.
        if (welcomeOpenRef.current) break
        viewportApi.current?.selectAll()
        break
      // Select None / Invert Selection have no text-field analog: like
      // Paste In Place, a fire that lands while a field has focus (the
      // native macOS ⇧⌘A key equivalent reaches here regardless) simply
      // does nothing rather than changing the scene selection out from
      // under an in-progress rename.
      case 'edit-select-none':
        if (isTextFieldFocused() || welcomeOpenRef.current) break
        handleSelectNone()
        break
      case 'edit-invert-selection':
        if (isTextFieldFocused()) break
        viewportApi.current?.invertSelection()
        break
      case 'edit-delete': deleteSelection(); break
      case 'edit-copy':
        if (forwardToTextField('copy')) break
        handleCopy()
        break
      case 'edit-cut':
        if (forwardToTextField('cut')) break
        handleCut()
        break
      case 'edit-paste':
        // Desktop only: the native Edit item fires here even while a text
        // field has focus (see main.rs's Edit menu comment) — paste OS text
        // into the field through the clipboard plugin instead of the scene.
        // On the web the browser pastes into fields itself and this
        // dispatch only ever comes from the in-app menu.
        if (isTauri && isTextFieldFocused()) {
          void import('@tauri-apps/plugin-clipboard-manager').then(({ readText }) =>
            pasteTextIntoFocusedField(readText),
          )
          break
        }
        handlePaste(false)
        break
      case 'edit-paste-in-place':
        if (isTextFieldFocused()) break
        handlePaste(true)
        break
      case 'close':
        // Trigger the beforeunload / close-guard path by emitting the
        // Tauri window close request — handled by the close guard effect.
        import('@tauri-apps/api/window').then(({ getCurrentWindow }) => {
          getCurrentWindow().close().catch(() => { /* ignore */ })
        }).catch(() => { /* ignore */ })
        break
      // Tool activations from native menu
      /* oxlint-disable lingui/no-unlocalized-strings -- ToolName identifiers, not copy */
      case 'tool-select':    setActiveTool('Select'); break
      case 'tool-rectangle': setActiveTool('Rectangle'); break
      case 'tool-circle':    setActiveTool('Circle'); break
      case 'tool-polygon':   setActiveTool('Polygon'); break
      case 'tool-arc':       setActiveTool('Arc'); break
      case 'tool-line':      setActiveTool('Line'); break
      case 'tool-pushpull':  setActiveTool('Push/Pull'); break
      case 'tool-follow-me': setActiveTool('Follow Me'); break
      case 'tool-offset':    setActiveTool('Offset'); break
      case 'tool-paint':     setActiveTool('Paint'); break
      case 'tool-position-texture': setActiveTool('Position Texture'); break
      case 'tool-move':      setActiveTool('Move'); break
      case 'tool-rotate':    setActiveTool('Rotate'); break
      case 'tool-scale':     setActiveTool('Scale'); break
      case 'tool-tape-measure': setActiveTool('Tape Measure'); break
      case 'tool-dimension': setActiveTool('Dimension'); break
      case 'tool-protractor': setActiveTool('Protractor'); break
      case 'tool-slice':     setActiveTool('Slice'); break
      case 'tool-section-plane': setActiveTool('Section Plane'); break
      case 'tool-edit-vertex': setActiveTool('Edit Vertex'); break
      case 'tool-axes':        setActiveTool('Drawing Axes'); break
      case 'tool-text':      setActiveTool('Text'); break
      case 'tool-orbit':     activateTool('Orbit'); break
      case 'tool-pan':       activateTool('Pan'); break
      case 'tool-zoom':      activateTool('Zoom'); break
      case 'tool-position-camera': activateTool('Position Camera'); break
      case 'tool-walk':      activateTool('Walk'); break
      case 'tool-look-around': activateTool('Look Around'); break
      case 'tool-zoom-window': activateTool('Zoom Window'); break
      /* oxlint-enable lingui/no-unlocalized-strings */
      case 'toggle-parallel-projection': viewportApi.current?.toggleProjection(); break
      // Camera ▸ Fly (docs/design/fly-navigation.md §10): the LATCHED fly
      // mode for trackpads — a hold of the right button needs no menu.
      case 'toggle-fly':          viewportApi.current?.toggleFlyLatched(); break
      // Window pane toggles — must use functional updaters (StrictMode safe)
      case 'toggle-model-info':   toggleSection(setShowModelInfo, trayTitle.outliner); break
      case 'toggle-materials':    toggleSection(setShowMaterials, trayTitle.materials); break
      case 'toggle-components':   toggleSection(setShowComponents, trayTitle.components); break
      case 'toggle-tags':         toggleSection(setShowTags, trayTitle.tags); break
      case 'toggle-scenes':       toggleSection(setShowScenes, trayTitle.scenes); break
      case 'toggle-changes':      toggleSection(setShowChanges, trayTitle.changes); break
      case 'toggle-object-info':  toggleSection(setShowObjectInfo, trayTitle.objectInfo); break
      case 'toggle-tray':         toggleTray(); break
      case 'toggle-compact-rail': toggleRailNarrow(); break
      case 'toggle-dock':         toggleDock(); break
      case 'toggle-debug-log':    setShowDebugLog((v) => !v); break
      case 'toggle-axes':         setShowAxes((v) => !v); break
      case 'reset-axes':         viewportApi.current?.resetAxes(); break
      case 'toggle-grid':         setShowGrid((v) => !v); break
      case 'toggle-xray':         toggleXray(); break
      case 'toggle-wireframe':    toggleWireframe(); break
      case 'toggle-guides':       setShowGuides((v) => !v); break
      case 'toggle-section-active': viewportApi.current?.toggleSectionActive(); break
      case 'edit-delete-guides':  viewportApi.current?.deleteAllGuides(); break
      case 'zoom-extents':        handleZoomExtentsRef.current(); break
      case 'view-top':            viewportApi.current?.setStandardView('top'); break
      case 'view-bottom':         viewportApi.current?.setStandardView('bottom'); break
      case 'view-front':          viewportApi.current?.setStandardView('front'); break
      case 'view-back':           viewportApi.current?.setStandardView('back'); break
      case 'view-left':           viewportApi.current?.setStandardView('left'); break
      case 'view-right':          viewportApi.current?.setStandardView('right'); break
      case 'view-iso':            viewportApi.current?.setStandardView('iso'); break
      case 'open-settings':       openSettingsRef.current(); break
      case 'report-bug': handleReportBug(); break
      case 'sent-reports': handleSentReports(); break
      case 'open-guide': handleOpenGuide(); break
      case 'open-library':   openLibraryEntry(); break
      case 'toggle-library': openLibraryEntry(); break
      case 'purge-unused': requestPurgeUnused(); break
      case 'make-solid': {
        // Object ▸ Make Solid…: the selected objects when anything is
        // selected, else the whole model.
        const scene = sceneRef.current
        const ids = scene === null ? [] : selectionObjectIds(scene, selectedIds)
        // A selection of sketches is the second input; one that also holds
        // Objects keeps today's behavior (the Objects' wizard).
        const sketches = selectedSketchIds(selectedIds, scene ?? undefined)
        if (ids.length > 0) openMakeSolid('selection', ids)
        else if (sketches.length > 0) openMakeSolidSketches(sketches)
        else openMakeSolid('model')
        break
      }
      case 'model-health': openModelHealth(); break
      // Contextual dock only (needs a single selected object/group/instance)
      // — same posture as 'enter-context' above: excused from the palette
      // in registry.ts rather than registered, since it needs a picked node.
      case 'save-to-library': {
        if (selectedIds.length !== 1) break
        const node = selectedIds[0]
        const sel = structuralSelection([node])
        if (sel === null) {
          // oxlint-disable-next-line lingui/no-unlocalized-strings -- kernel error code identifier
          handleToast(kernelErrorMessage('InvalidSelection', ''), 'InvalidSelection')
          break
        }
        setSaveToLibraryTarget({
          kind: 'selection',
          nodeKinds: sel.kinds,
          nodeIds: sel.ids,
          defaultName: librarySelectionDisplayName(node),
        })
        break
      }
      case 'save-to-library-doc':
        setSaveToLibraryTarget({ kind: 'document', defaultName: documentName(docSession) })
        break
      case 'open-palette':        openPalette(); break
      // Contextual dock only — these need the current selection, not
      // just a bare trigger, so unlike every case above they aren't also
      // reachable from a static native-menu item; the dock and (once
      // adds outliner actions there too) other selection-aware UI are the
      // only callers. handleUngroup/handleMakeUnique already read
      // `selectedIds` from their own closure (no args needed); this case is
      // defined before their `const` declarations later in this render, but
      // the switch only ever *runs* on a later click, by which point this
      // render has fully executed and the closure sees the real function.
      case 'enter-context':
        // Entry convergence (docs/design/group-session.md): the dock can
        // hand this ANY selected node kind, including a group/instance
        // buried behind session frames that aren't open yet — `enterNode`
        // (not the simpler `handleEnterContext`) opens/closes exactly the
        // frames needed.
        if (selectedIds.length === 1) enterNode(selectedIds[0])
        break
      case 'ungroup': handleUngroup(); break
      case 'make-unique': handleMakeUnique(); break
      case 'explode-instance': handleExplodeInstance(); break
      // Edit-menu object commands (also reachable from the contextual dock
      // via the aliases above). Selection-gated: the native menu items are
      // enabled/disabled through sync_menu_state, and the handlers themselves
      // re-check the selection, so a stale click is a no-op.
      case 'edit-group': handleGroup(); break
      case 'edit-ungroup': handleUngroup(); break
      case 'edit-make-component': handleMakeComponent(); break
      case 'edit-place-copy': handlePlaceInstance(); break
      case 'edit-explode': handleExplodeInstance(); break
      case 'edit-make-unique': handleMakeUnique(); break
      case 'edit-union': handleBoolean(0); break
      case 'edit-subtract': handleBoolean(1); break
      case 'edit-intersect': handleBoolean(2); break
      // View ▸ Scenes (docs/design/scenes.md §5): Add/Update/Next/Previous
      // drive the ScenesController exclusively — see its own module doc on
      // why the UI never reaches the kernel directly for Scenes.
      case 'scenes-add': addSceneFromMenu(); break
      case 'scenes-update': {
        const sid = scenesRef.current.activeSid
        if (sid !== null) scenesRef.current.update(sid)
        break
      }
      case 'scenes-next': scenesRef.current.next(); break
      case 'scenes-previous': scenesRef.current.previous(); break
      case 'scenes-transitions': setSceneTransitions(!sceneTransitionsOn); break
    }
  }

  // ---------------------------------------------------------------- native menu-action listener (Tauri only)
  // Registered once; dispatches through menuActionRef so the handler is always
  // the latest one (HMR/StrictMode safe).
  useEffect(() => {
    if (!isTauri) return
    let unlisten: (() => void) | undefined
    let cancelled = false
    // Window-scoped listener (NOT the module-level `listen`, whose Any
    // target also receives events emit_to'd at OTHER document windows —
    // with File ▸ New's multi-window support, a menu action must only run
    // in the window it was routed to).
    import('@tauri-apps/api/webviewWindow').then(({ getCurrentWebviewWindow }) => {
      return getCurrentWebviewWindow().listen<string>('menu-action', (event) => {
        menuActionRef.current(event.payload)
      })
    }).then((fn) => { if (cancelled) fn(); else unlisten = fn }).catch(() => { /* ignore if not in Tauri */ })
    return () => { cancelled = true; unlisten?.() }
  }, [])

  // ---------------------------------------------------------------- library-window bridge (Tauri only)
  // The native Library window owns no document; its actions arrive here —
  // at the ACTIVE document window — via `library_dispatch` (main.rs), the
  // same routing as native menu actions. The item is re-resolved from the
  // shared store by name, then handed to the exact handlers the web
  // build's in-app modal uses, so the two hostings cannot drift.
  const libraryBridgeRef = useRef({
    onLibraryInsert,
    onLibraryOpenAsDocument,
    onLibraryPaintWith,
    onLibraryAddToPalette,
    handleToast,
  })
  libraryBridgeRef.current = {
    onLibraryInsert,
    onLibraryOpenAsDocument,
    onLibraryPaintWith,
    onLibraryAddToPalette,
    handleToast,
  }
  useEffect(() => {
    if (!isTauri) return
    let unlisten: (() => void) | undefined
    let cancelled = false
    import('@tauri-apps/api/webviewWindow').then(({ getCurrentWebviewWindow }) => {
      return getCurrentWebviewWindow().listen<string>('library-action', (event) => {
        void (async () => {
          const bridge = libraryBridgeRef.current
          let payload: { action?: string; fileName?: string }
          try {
            payload = JSON.parse(event.payload) as { action?: string; fileName?: string }
          } catch {
            return
          }
          const fileName = payload.fileName
          if (fileName === undefined) return
          const store = libraryStore()
          if (!store.available()) return
          try {
            const bytes = await store.read(fileName)
            const hash = await sha256Hex(bytes)
            const summary = await readItemSummary(bytes)
            const entries = await store.list()
            const file = entries.find((e) => e.fileName === fileName) ?? {
              fileName,
              size: bytes.length,
              mtimeMs: 0,
            }
            const item = buildLibraryItem(file, summary, hash)
            // if/else rather than switch: the palette drift-guard test
            // scrapes `case '…':` labels out of this file as menu action
            // ids, and these are library-window verbs, not palette actions.
            const action = payload.action
            if (action === 'insert') bridge.onLibraryInsert(item, bytes)
            else if (action === 'open') bridge.onLibraryOpenAsDocument(item, bytes)
            else if (action === 'paint') bridge.onLibraryPaintWith(item, bytes)
            else if (action === 'add-palette') bridge.onLibraryAddToPalette(item, bytes)
          } catch (err) {
            bridge.handleToast(
              t`Couldn't read library item: ${ph({ reason: err instanceof Error ? err.message : String(err) })}`,
            )
          }
        })()
      })
    }).then((fn) => { if (cancelled) fn(); else unlisten = fn }).catch(() => { /* ignore if not in Tauri */ })
    return () => { cancelled = true; unlisten?.() }
  }, [])

  // Push this document's placements to the Library window: on every change,
  // on that window's mount-time request, and when THIS window gains focus
  // (multi-window: the focused document's counts are the ones the Library
  // shows).
  useEffect(() => {
    if (!isTauri) return
    let cancelled = false
    const unlisteners: Array<() => void> = []
    void import('@tauri-apps/api/event').then(async ({ listen }) => {
      const push = (): void => {
        let paletteHashes: string[] = []
        try {
          const scene = sceneRef.current
          if (scene !== null) {
            paletteHashes = JSON.parse(scene.palette_content_hashes_json()) as string[]
          }
        } catch {
          /* hashes are a badge nicety — never block the placements push */
        }
        // Routed through the SHELL, which forwards only the ACTIVE document
        // window's push — a background window's data can never race the
        // active one's into the Library (adversarial review).
        void import('@tauri-apps/api/core').then(({ invoke }) =>
          invoke('library_push_placements', {
            payload: JSON.stringify({ placements: libraryPlacements, paletteHashes }),
          }),
        ).catch(() => { /* the Library window may simply not exist */ })
      }
      push()
      const u1 = await listen('library-placements-request', push)
      if (cancelled) { u1(); return }
      unlisteners.push(u1)
      const { getCurrentWebviewWindow } = await import('@tauri-apps/api/webviewWindow')
      const u2 = await getCurrentWebviewWindow().listen('tauri://focus', push)
      if (cancelled) { u2(); return }
      unlisteners.push(u2)
    }).catch(() => { /* ignore if not in Tauri */ })
    return () => {
      cancelled = true
      unlisteners.forEach((u) => u())
    }
  }, [libraryPlacements])

  // ---------------------------------------------------------------- menu-open-path listener (Tauri only)
  // Emitted by Rust when a recent-file menu item is clicked, or when a file
  // is opened via the macOS "open document" Apple event (warm case). The
  // startup-handoff effect below waits for this registration before telling
  // the shell the window is ready for live delivery.
  useEffect(() => {
    if (!isTauri) return
    let unlisten: (() => void) | undefined
    let cancelled = false
    // Window-scoped for the same multi-window reason as menu-action above.
    import('@tauri-apps/api/webviewWindow').then(({ getCurrentWebviewWindow }) => {
      return getCurrentWebviewWindow().listen<string>('menu-open-path', (event) => {
        openPathRef.current(event.payload)
      })
    }).then((fn) => {
      if (cancelled) { fn(); return }
      unlisten = fn
      openListenerReadyRef.current?.resolve()
    }).catch(() => {
      // Registration failed — resolve anyway so the handoff effect (which
      // only needs the poll path in that case) can proceed.
      openListenerReadyRef.current?.resolve()
    })
    return () => { cancelled = true; unlisten?.() }
  }, []) // openPath accessed via openPathRef — no dep needed

  // ---------------------------------------------------------------- open-window list (Tauri only)
  // Feeds the in-app MenuBar's Window-menu tail (Windows/Linux — macOS's
  // native menu renders its own tail shell-side). `list_windows` seeds this
  // window's initial view; the shell's `window-list` broadcast (emitted to
  // every window on create/destroy/focus/title-change — see
  // refresh_window_list in main.rs) keeps it live after that. A plain
  // (non-window-scoped) listener is correct here, unlike menu-action/
  // menu-open-path above: the list is identical for every window and
  // reading it is never destructive.
  useEffect(() => {
    if (!isTauri) return
    let unlisten: (() => void) | undefined
    let cancelled = false
    // Register the listener BEFORE doing any fetch. The reverse order (fetch
    // and listen started in parallel) raced this window's OWN startup
    // delivery: a freshly spawned window's docSession-title-push effect
    // (below) can invoke `set_window_title` — broadcasting the corrected
    // title — before this window's own `window-list` listener has finished
    // its dynamic import + subscribe round trip. That broadcast would then
    // be silently dropped (nobody listening yet), with nothing to correct it
    // until the next focus/create/destroy event fires a fresh one — exactly
    // the "stays Untitled until focused" symptom. Listening first means
    // nothing between mount and "listener ready" can be missed: anything
    // already committed shell-side by then is picked up by the catch-up
    // fetch below, and anything after is captured live by the listener.
    let broadcastSeenSinceListening = false
    import('@tauri-apps/api/event').then(({ listen }) =>
      listen<{ label: string; title: string; focused: boolean }[]>('window-list', (event) => {
        broadcastSeenSinceListening = true
        setWindowList(event.payload)
      }),
    ).then((fn) => {
      if (cancelled) { fn(); return }
      unlisten = fn
      // Catch-up fetch, now that the listener is definitely live. Guarded
      // the same way a fetch-vs-broadcast race is always guarded here: a
      // broadcast landing in the gap between "listener registered" and
      // "this fetch resolves" must win, not be clobbered by a slower,
      // now-stale snapshot.
      import('@tauri-apps/api/core')
        .then(({ invoke }) => invoke<{ label: string; title: string; focused: boolean }[]>('list_windows'))
        .then((list) => { if (!cancelled && !broadcastSeenSinceListening) setWindowList(list) })
        .catch(() => { /* older shell without the command — ignore */ })
    }).catch(() => { /* not in Tauri, or registration failed */ })
    return () => { cancelled = true; unlisten?.() }
  }, [])

  const focusWindow = useCallback((label: string) => {
    import('@tauri-apps/api/core')
      .then(({ invoke }) => invoke('focus_window', { label }))
      .catch(() => { /* ignore */ })
  }, [])

  // ---------------------------------------------------------------- startup handoff: recovery slot, buffered open, recovery prompt
  // Runs once, after the scene first becomes available (adopting a snapshot
  // or opening a buffered path into a still-loading kernel would silently
  // no-op), in strict order:
  //
  //  1. take_pending_recovery — this window was spawned to recover a
  //     specific crash snapshot; claim it and skip everything else.
  //  2. take_pending_window_open — this window was spawned by
  //     open_in_new_window (a File ▸ Open pick onto a non-pristine window);
  //     claim its queued path and skip everything else.
  //  3. take_pending_open — a cold-start file association buffered a path
  //     before this webview could listen; open it and skip the prompt (the
  //     user explicitly asked for that document).
  //  4. frontend_ready — from here on the shell delivers opens live (the
  //     open-path listener registration is awaited above, and the polls
  //     have drained the buffer, so nothing can be delivered twice or lost).
  //  5. Only then, and only in the primary window with nothing loaded, the
  //     crash-recovery offer. Ordering it after 1–3 (rather than racing
  //     them from a separate effect) is what keeps the dialog from
  //     appearing over — and its Recover then replacing — a document the
  //     user just double-clicked.
  //
  // Secondary document windows never prompt: a sibling window's live
  // autosave must not be "recovered" into them.
  const pendingHandoffCheckedRef = useRef(false)
  useEffect(() => {
    if (state === null || pendingHandoffCheckedRef.current) return
    pendingHandoffCheckedRef.current = true
    void (async () => {
      if (isTauri) {
        try {
          await openListenerReadyRef.current?.promise
          const { invoke } = await import('@tauri-apps/api/core')
          const slot = await invoke<string | null>('take_pending_recovery')
          const windowOpen =
            slot == null
              ? await invoke<{ path: string; as_copy: boolean; name: string | null } | null>(
                  'take_pending_window_open',
                )
              : null
          const path =
            slot == null && windowOpen == null
              ? await invoke<string | null>('take_pending_open')
              : null
          invoke('frontend_ready').catch(() => { /* older shell — ignore */ })
          if (slot != null) {
            const snapshot = await recoveryStoreRef.current.claim(slot)
            if (snapshot !== null) adoptSnapshotRef.current(snapshot)
            return
          }
          if (windowOpen != null) {
            if (windowOpen.as_copy) {
              // Library "Open as Document": COPY semantics, matching the
              // pristine route exactly — the document is import-shaped
              // (Save always prompts), never path-bound to the library
              // item, and the library path never enters recents (whose
              // launch-time seeding grants WRITE approval — the round-3
              // review's silent-overwrite path).
              await openLibraryCopyRef.current(windowOpen.path, windowOpen.name)
            } else {
              await openPathRef.current(windowOpen.path)
            }
            return
          }
          if (path != null) {
            await openPathRef.current(path)
            return
          }
          const { getCurrentWindow } = await import('@tauri-apps/api/window')
          if (getCurrentWindow().label !== 'main') return
        } catch {
          /* not fatal — fall through to the recovery check */
        }
      }
      try {
        const listings = await recoveryStoreRef.current.list()
        if (shouldPromptRecovery(docSessionRef.current, listings)) {
          setRecoveryPrompt(listings)
          return
        }
      } catch {
        /* ignore — no recovery prompt */
      }
      // Bare launch confirmed: nothing pending, nothing to recover, primary
      // window. Offer the welcome screen (unless the user turned it off, or
      // a live file-open landed while the recovery check above was in flight).
      if (getShowWelcome() && !documentLoadedRef.current) setWelcomeOpen(true)
    })()
  }, [state]) // runs once, after the kernel is ready; handlers accessed via refs

  // ---------------------------------------------------------------- close guard (Tauri only)
  // Intercepts the native window close to warn about unsaved changes.
  //
  // Tauri v2 cannot honor a non-prevent decision made *after* an await: by the
  // time the async handler resolves, the synchronous prevent window has closed.
  // Fix: always call event.preventDefault() immediately (synchronously), then
  // explicitly call win.destroy() when we decide to close.  win.destroy() force-
  // closes the window bypassing onCloseRequested, so there is no re-entrancy loop.
  useEffect(() => {
    if (!isTauri) return
    let unlisten: (() => void) | undefined
    let cancelled = false
    import('@tauri-apps/api/window').then(({ getCurrentWindow }) => {
      const win = getCurrentWindow()
      return win.onCloseRequested(async (event) => {
        // Always prevent the default close; we decide explicitly below.
        event.preventDefault()
        if (docSessionRef.current.dirty) {
          // In-app UnsavedChangesDialog (Lane C) replaces the native `ask`
          // — its Save runs the current save flow and only resolves 'save'
          // on an actual successful write.
          const decision = await requestUnsavedChangesDecision()
          if (decision === 'cancel') return // keep the window open
          // 'save' means the dialog's Save already wrote the file.
        }
        // Whether the document was already clean (no prompt needed) or
        // just got saved/discarded above, the close is proceeding
        // irreversibly — drop this window's autosave snapshot before the
        // webview (and its ability to invoke the shell) is destroyed.
        // Unconditional (not just the dirty branch above): a document that
        // reads clean right now can still have a stale snapshot on disk —
        // e.g. autosave wrote one while dirty, then an undo brought the
        // document back to its saved mark — and leaving it behind would
        // falsely offer recovery on the next launch even though there was
        // never an unsaved-changes prompt to justify one.
        // `clearRecoverySnapshot` is a scoped no-op when there's nothing
        // to clear.
        await clearRecoverySnapshot()
        // Force-close, bypassing onCloseRequested (no loop).
        await win.destroy()
      })
    }).then((fn) => { if (cancelled) fn(); else unlisten = fn }).catch(() => { /* ignore */ })
    return () => { cancelled = true; unlisten?.() }
    // docSessionRef/saveDocumentRef are always current; clearRecoverySnapshot
    // and requestUnsavedChangesDecision both have stable ([]) deps.
  }, [clearRecoverySnapshot, requestUnsavedChangesDecision])

  // ---------------------------------------------------------------- native drag-drop (Tauri only)
  // The OS delivers file drops to Tauri's webview event bus rather than the
  // browser's dataTransfer API, so React onDrop never fires in the desktop build.
  // This effect subscribes to the Tauri webview drag-drop event and forwards
  // the first .hew or .skp path to openPath.  The existing React onDrop handlers are
  // kept for the web build and remain unchanged.
  useEffect(() => {
    if (!isTauri) return
    let unlisten: (() => void) | undefined
    let cancelled = false
    import('@tauri-apps/api/webview')
      .then(({ getCurrentWebview }) =>
        getCurrentWebview().onDragDropEvent((event) => {
          if (event.payload.type === 'drop') {
            const doc = event.payload.paths.find((p) => /\.(hew|skp)$/i.test(p))
            if (doc) {
              openPathRef.current(doc)
              return
            }
            // A dropped picture arms Place Image at the drop point (the
            // shell approved the dropped path for reading).
            const picture = event.payload.paths.find(isPictureFileName)
            if (picture !== undefined) {
              const { x, y } = event.payload.position ?? { x: 0, y: 0 }
              const dpr = window.devicePixelRatio || 1
              import('@tauri-apps/api/core')
                .then(({ invoke }) => invoke<ArrayBuffer>('read_file', { path: picture }))
                .then((buf) => {
                  const name = picture.split(/[/\\]/).pop() ?? picture
                  startImagePlacementRef.current(name, new Uint8Array(buf), { clientX: x / dpr, clientY: y / dpr })
                })
                .catch((err: unknown) => {
                  const reason = friendlyErrorText(err)
                  handleToastRef.current(t`Couldn't open the picture: ${reason}`)
                })
            }
          }
        }),
      )
      .then((fn) => { if (cancelled) fn(); else unlisten = fn })
      .catch(() => { /* not in Tauri */ })
    return () => { cancelled = true; unlisten?.() }
  }, []) // openPath accessed via openPathRef — no dep needed

  // ---------------------------------------------------------------- global keyboard shortcuts
  // Every command shortcut — tools, File/Edit/Window commands, scenes, the
  // palette, Delete and the clipboard keys — runs through the keymap
  // (keymap/), which the person can change in Settings ▸ Controls. The
  // dispatcher keeps the guards the old hand-written handlers applied: IME
  // composition, the modal welcome screen, focused text fields, a tool typing
  // a measurement, open dialogs, and the macOS native menu's own
  // accelerators (keymap/dispatch.ts).
  // Re-render on a keymap change (Settings ▸ Controls, possibly in another
  // window) so the rail's palette chip and every shortcut shown stay true.
  useKeymap()
  // macOS desktop: keep the native menu's accelerators equal to the keymap.
  useEffect(() => {
    if (!(isTauri && isMac)) return
    return startNativeMenuSync({
      isMac,
      defaults: defaultKeymapBindings(),
      current: () => currentKeymap().bindings,
      subscribe: subscribeKeymap,
      invoke: (accelerators) =>
        import('@tauri-apps/api/core').then(({ invoke }) =>
          invoke<string[]>('set_menu_accelerators', { accelerators }),
        ),
    })
  }, [])
  useKeymapDispatch({
    isMac,
    nativeMenu: isTauri && isMac,
    run: (id) => {
      // A plugin command holds the document (docs/agents/PLUGINS.md
      // "Compounds"): no shortcut acts on it, a tool's key included. The
      // chord was already swallowed, so the browser does not act on it
      // either.
      if (sceneRef.current?.busy()) return
      // A tool's key restarts the tool even when it is already active
      // (SketchUp's habit): it drops a sticky idle plane lock or a half-made
      // gesture. A tool mid-measurement keeps its letters through
      // `toolCapturesKey`, so this never cuts a typed entry short.
      if (isToolAction(id)) setToolActivationSeq((s) => s + 1)
      menuActionRef.current(id)
    },
    welcomeOpen: () => welcomeOpenRef.current,
    toolCapturesKey: (key) => viewportApi.current?.isCapturingInput?.(key) ?? false,
  })

  // Mirror the View ▸ Axes / Grid / Guides toggles into the viewport. The
  // viewport API ref is populated once the viewport mounts; all default to
  // visible so an early run before the ref is ready is a harmless no-op.
  useEffect(() => {
    viewportApi.current?.setAxesVisible(showAxes)
    // A document with a view style records the choice (a save back to its
    // `.skp` writes it); one without keeps none. While a plugin command
    // holds the document the kernel refuses the write (DocumentBusy); the
    // viewport still follows the toggle.
    try {
      sceneRef.current?.set_view_style_axes(showAxes)
    } catch (err) {
      if (!String(err).startsWith('DocumentBusy')) throw err
    }
  }, [showAxes])
  useEffect(() => { viewportApi.current?.setGridVisible(showGrid) }, [showGrid])
  useEffect(() => { viewportApi.current?.setRenderMode(renderMode) }, [renderMode])
  useEffect(() => { viewportApi.current?.setGuidesVisible(showGuides) }, [showGuides])

  // Undo/redo availability (queried from scene each render for menu state)
  const canUndo = sceneRef.current?.can_scene_undo() ?? false
  const canRedo = sceneRef.current?.can_scene_redo() ?? false
  // Lane C (docs/design/v1.1-cycle.md): "Undo Push/Pull" / "Redo Move".
  const { undo: undoLabel, redo: redoLabel } = undoRedoMenuLabels(sceneRef.current)

  // ---------------------------------------------------------------- native menu state sync (macOS)
  // Reflect UI state into the native menu bar: the active tool's radio
  // check, the View/Window toggles' check marks, and the enabled state of
  // the selection-gated Edit commands. macOS is the only platform that
  // attaches the native menu (Linux/Windows use the in-app MenuBar, which
  // reads this state directly as props).
  //
  // There is ONE menu bar shared by every document window, so the state it
  // shows must follow focus: gaining focus re-pushes this window's state
  // (menuFocusTick), and the shell ignores pushes from unfocused windows —
  // without this, switching from a window with a selection to one without
  // left the old window's check marks and Edit gates on the menu.
  const [menuFocusTick, setMenuFocusTick] = useState(0)
  useEffect(() => {
    if (!isTauri || !isMac) return
    let unlisten: (() => void) | undefined
    let cancelled = false
    import('@tauri-apps/api/window').then(({ getCurrentWindow }) =>
      getCurrentWindow().onFocusChanged(({ payload: focused }) => {
        if (focused) setMenuFocusTick((t) => t + 1)
      }),
    ).then((fn) => { if (cancelled) fn(); else unlisten = fn }).catch(() => { /* ignore */ })
    return () => { cancelled = true; unlisten?.() }
  }, [])
  useEffect(() => {
    if (!isTauri || !isMac) return
    const checked: Record<string, boolean> = {
      'view-axes': showAxes,
      'view-grid': showGrid,
      'view-xray': xrayOn,
      'view-wireframe': wireframeOn,
      'view-guides': showGuides,
      'view-section-plane': sectionPlaneMenuState.checked,
      // Sections read "on screen", as in the web MenuBar (see its props).
      'win-model-info': showModelInfo && !trayCollapsed,
      'win-materials': showMaterials && !trayCollapsed,
      'win-components': showComponents && !trayCollapsed,
      'win-tags': showTags && !trayCollapsed,
      'win-scenes': showScenes && !trayCollapsed,
      'win-changes': showChanges && !trayCollapsed,
      'win-object-info': showObjectInfo && !trayCollapsed,
      'view-tray': !trayCollapsed,
      'view-compact-rail': railNarrow,
      'view-dock': !dockHidden,
      'win-debug-log': showDebugLog,
      'win-library': showLibrary,
      'cam-parallel-projection': parallelProjection,
      'cam-fly': fly.mode === 'latched',
      'scenes-transitions': sceneTransitionsOn,
    }
    for (const [tool, id] of Object.entries(TOOL_MENU_IDS)) {
      checked[id] = tool === activeTool
    }
    const enabled: Record<string, boolean> = {
      'edit-delete': selectedIds.length > 0 || selectedGuide !== null || selectedAnnotation !== null,
      'edit-group': menuGates?.canGroup ?? false,
      'edit-ungroup': menuGates?.canUngroup ?? false,
      'edit-make-component': menuGates?.canMakeComponent ?? false,
      'edit-place-copy': menuGates?.canPlaceCopy ?? false,
      'edit-save-selection-to-library': menuGates?.canSaveSelectionToLibrary ?? false,
      'edit-explode': menuGates?.canExplode ?? false,
      'edit-make-unique': menuGates?.canMakeUnique ?? false,
      'edit-union': menuGates?.canBoolean ?? false,
      'edit-subtract': menuGates?.canBoolean ?? false,
      'edit-intersect': menuGates?.canBoolean ?? false,
      'edit-cut': menuGates?.hasStructuralSelection ?? false,
      'edit-copy': menuGates?.hasStructuralSelection ?? false,
      'edit-paste': menuGates?.clipboardHasContent ?? false,
      'edit-paste-in-place': menuGates?.clipboardHasContent ?? false,
      'view-section-plane': sectionPlaneMenuState.exists,
      // Import and 3D Text refuse only while a COMPONENT frame is open
      // (ExplodeSessionScope) — gate the NATIVE items the same way the web
      // MenuBar/palette already are, so the kernel refusal stays a backstop
      // rather than a toast a user actually meets.
      'file-import': !componentFrameOpen,
      'file-import-image': !componentFrameOpen,
      // Revert to Saved: a file to return to and changes to throw away.
      'file-revert': revertable,
      'draw-3d-text': !componentFrameOpen,
      // View ▸ Scenes ▸ Update/Next/Previous (docs/design/scenes.md §5).
      'scenes-update': scenes.activeSid !== null,
      'scenes-next': scenes.entries.length > 0,
      'scenes-previous': scenes.entries.length > 0,
      // Lane C: native Undo/Redo now have a real handle (edit_undo/
      // edit_redo registered via gated_item — shells/tauri's own comment on
      // why), so sync their enabled state exactly like the web MenuBar's
      // `disabled={!canUndo}`.
      'edit-undo': canUndo,
      'edit-redo': canRedo,
    }
    // "Undo Push/Pull" / "Redo Move" (docs/design/v1.1-cycle.md).
    const text: Record<string, string> = { 'edit-undo': undoLabel, 'edit-redo': redoLabel }
    import('@tauri-apps/api/core')
      .then(({ invoke }) => invoke('sync_menu_state', { checked, enabled, text }))
      .catch(() => { /* shell without the command (older build) — ignore */ })
  }, [
    activeTool,
    showAxes,
    showGrid,
    xrayOn,
    wireframeOn,
    showGuides,
    sectionPlaneMenuState,
    showModelInfo,
    showMaterials,
    showComponents,
    showTags,
    showScenes,
    showChanges,
    showObjectInfo,
    trayCollapsed,
    railNarrow,
    dockHidden,
    showDebugLog,
    showLibrary,
    selectedIds,
    selectedGuide,
    selectedAnnotation,
    menuGates,
    menuFocusTick,
    parallelProjection,
    fly.mode,
    componentFrameOpen,
    revertable,
    canUndo,
    canRedo,
    undoLabel,
    redoLabel,
    sceneTransitionsOn,
    scenes.activeSid,
    scenes.entries.length,
  ])

  // ---------------------------------------------------------------- drag-drop open
  const handleDragOver = useCallback((ev: React.DragEvent) => {
    ev.preventDefault()
    ev.dataTransfer.dropEffect = 'copy'
  }, [])

  const handleDrop = useCallback(async (ev: React.DragEvent) => {
    ev.preventDefault()
    const file = ev.dataTransfer.files[0]
    if (file == null) return
    if (isPictureFileName(file.name)) {
      // A dropped picture arms Place Image at the drop point; it does not
      // place by itself, and nothing about the document is replaced.
      const at = { clientX: ev.clientX, clientY: ev.clientY }
      try {
        startImagePlacement(file.name, new Uint8Array(await file.arrayBuffer()), at)
      } catch (err: unknown) {
        const reason = friendlyErrorText(err)
        handleToast(t`Couldn't open the picture: ${reason}`)
      }
      return
    }
    if (/\.skp$/i.test(file.name)) {
      // A dropped `.skp` opens as the document's own file. The File System
      // Access handle (Chromium) must be requested synchronously — a
      // DataTransferItem is dead after the first await — so Save can write
      // back in place; without one, Save asks where to write.
      const item = ev.dataTransfer.items?.[0] as
        | (DataTransferItem & { getAsFileSystemHandle?: () => Promise<FileSystemHandle | null> })
        | undefined
      const handlePromise = item?.getAsFileSystemHandle?.() ?? Promise.resolve(null)
      if (!(await confirmDiscard())) return
      const scene = sceneRef.current
      if (scene === null) return
      try {
        const bytes = new Uint8Array(await file.arrayBuffer())
        const dropped = await handlePromise.catch(() => null)
        const handle = dropped !== null && dropped.kind === 'file' ? dropped : null
        await runOpenSkp(scene, { name: file.name, bytes, handle })
      } catch (err: unknown) {
        handleToast(t`Drop open failed: ${ph({ reason: friendlyErrorText(err) })}`)
      }
      return
    }
    if (!file.name.endsWith('.hew')) return
    if (!(await confirmDiscard())) return
    file.arrayBuffer().then((buf) => {
      const discardsUnsaved = docSessionRef.current.dirty
      const ok = applyLoadedBytes(new Uint8Array(buf), { name: file.name, label: OPEN_FILE_LABELS.hew })
      if (ok) {
        setDocSession(afterOpen({ name: file.name, format: 'hew', handle: null }, Date.now()))
        if (discardsUnsaved) void clearRecoverySnapshot()
      }
    }).catch((err: unknown) => {
      handleToast(t`Drop open failed: ${ph({ reason: friendlyErrorText(err) })}`)
    })
  }, [confirmDiscard, applyLoadedBytes, handleToast, clearRecoverySnapshot, runOpenSkp, startImagePlacement, t])

  // ── Hide/Show + Tags (must be declared BEFORE the early returns below so the
  // hook count is stable across the loading and loaded renders — Rules of Hooks).
  /**
   * Derive the union hidden id sets from both manual-hide and tag-hide
   * sources (`unionHiddenLeafIds`, documentLoad.ts — shared with Shop Mode's
   * boot-time seed so both shells compute the identical hidden set), then
   * push the result to the renderer AND the kernel's inference-hide state.
   *
   * Both sets are passed in explicitly so callers can pass the *next* set
   * (before setState is applied) without waiting for a re-render.
   */
  const pushUnionHidden = useCallback(
    (nextHiddenKeys: Set<string>, nextHiddenTagPaths: Set<string>, opts?: { onlyIfChanged?: boolean }) => {
      const scene = state?.scene
      if (scene === undefined) return
      const { objectIds, instanceIds } = unionHiddenLeafIds(scene, nextHiddenKeys, nextHiddenTagPaths)
      // `onlyIfChanged`: skip the push when the renderer already hides
      // exactly this set — `set_hidden` rebuilds the kernel's whole
      // snap/pick registration, too costly to pay after every mutation that
      // merely MIGHT have moved something in or out of hiding.
      const shown = opts?.onlyIfChanged === true ? viewportApi.current?.getHiddenIds() : undefined
      // Drafting hides with its own tags and its group's (the kernel's
      // `sketch_shown`), which the hidden leaf ids above do not cover: an
      // explicit visibility change rebuilds the sketch lines too.
      if (shown === undefined) viewportApi.current?.refreshSketches()
      if (shown !== undefined && sameIds(shown.objects, objectIds) && sameIds(shown.instances, instanceIds)) return
      // (1) Renderer: hide the meshes. (2) Kernel inference: drop the hidden
      // geometry so snap/pick_face skip it — otherwise you'd still snap to and
      // be unable to click past a hidden solid's edges/faces.
      viewportApi.current?.setHidden(objectIds, instanceIds)
      // While isolated, the kernel hides everything outside the isolated set
      // as well (the viewport merges it into its own hidden sets).
      const eff = viewportApi.current?.getEffectiveHiddenIds?.() ?? { objects: objectIds, instances: instanceIds }
      setKernelHidden(scene, eff.objects, eff.instances)
    },
    // state?.scene changes on every render; we capture it fresh via the closure.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [state],
  )
  // Keep the ref pointed at the latest pushUnionHidden closure so callbacks
  // defined earlier (applyLoadedBytes, importDocument) never call a stale one.
  pushUnionHiddenRef.current = pushUnionHidden

  const handleToggleHidden = (node: NodeRef) => {
    const key = nodeKey(node)
    // Compute the next hidden set purely (no setState updater side effects).
    const next = new Set(hiddenKeys)
    const nowHidden = !next.has(key)
    if (next.has(key)) {
      next.delete(key)
    } else {
      next.add(key)
    }
    // Persist the choice in the document's USER-hidden registry (view state,
    // not undoable) so it survives save/load, mirroring how
    // handleToggleTagPath persists via set_tag_hidden — first, since the
    // renderer reads a definition member's hidden state from it.
    const kindNum = nodeKindToNumber(node.kind)
    if (kindNum >= 0) sceneRef.current?.set_node_user_hidden(kindNum, node.id, nowHidden)
    setHiddenKeys(next)
    pushUnionHidden(next, hiddenTagPaths)
  }
  handleToggleHiddenRef.current = handleToggleHidden

  /**
   * Batch hide/show for many nodes at once (Lane D: the Outliner's
   * per-container "hide/show all children" control, plus the root Model
   * row's own eye, which has no single node of its own to toggle). One Set
   * mutation, one `pushUnionHidden` push, N `set_node_user_hidden` calls —
   * not N individual `handleToggleHidden` round trips.
   */
  const handleSetHiddenMany = useCallback((nodes: NodeRef[], hidden: boolean) => {
    if (nodes.length === 0) return
    const next = new Set(hiddenKeys)
    for (const n of nodes) {
      const k = nodeKey(n)
      if (hidden) next.add(k)
      else next.delete(k)
    }
    const scene = sceneRef.current
    if (scene !== null) {
      for (const n of nodes) {
        const kindNum = nodeKindToNumber(n.kind)
        if (kindNum >= 0) scene.set_node_user_hidden(kindNum, n.id, hidden)
      }
    }
    setHiddenKeys(next)
    pushUnionHidden(next, hiddenTagPaths)
  }, [hiddenKeys, hiddenTagPaths, pushUnionHidden])

  const handleToggleTagPath = useCallback((path: string[]) => {
    const key = tagPathKey(path)
    const nowHidden = !hiddenTagPaths.has(key)
    const next = new Set(hiddenTagPaths)
    if (next.has(key)) {
      next.delete(key)
    } else {
      next.add(key)
    }
    // Persist the choice in the document's tag registry (view state, not
    // undoable) so it survives save/load, in addition to the session-only
    // hiddenTagPaths set below (which drives this render's UI immediately)
    // — first, since the renderer reads a definition member's tags from it.
    sceneRef.current?.set_tag_hidden(path.join('/'), nowHidden)
    setHiddenTagPaths(next)
    pushUnionHidden(hiddenKeys, next)
  }, [hiddenTagPaths, hiddenKeys, pushUnionHidden])
  toggleTagPathRef.current = handleToggleTagPath

  // Re-seed hiddenTagPaths from the document's registry and re-push the
  // union — the shared tail of every operation that can change the registry
  // out from under the session set (delete tag, undo/redo of a deletion).
  const resyncTagVisibility = useCallback(() => {
    const scene = sceneRef.current
    if (scene === null) return
    const seeded = seedHiddenTagPathsFromRegistry(scene)
    setHiddenTagPaths(seeded)
    pushUnionHidden(hiddenKeys, seeded)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [hiddenKeys, pushUnionHidden])
  resyncTagVisibilityRef.current = resyncTagVisibility

  // After a mutation that may have moved a node in or out of hiding without
  // going through the Tags panel or the Outliner eye — an Object Info tag
  // edit, a live API command (a tag assigned, hidden, renamed or deleted; a
  // node reparented into a hidden group), the harness's addNodeTag.
  // Re-seeds tags from the registry like `resyncTagVisibility`, but
  // cheaply: nothing to do when nothing is hidden at all, and no push when
  // the hidden set on screen is already right. Otherwise a node stays
  // visible and pickable under a hidden tag or group, or hidden without
  // one, until an unrelated resync (undo, a Tags-panel toggle) catches up.
  const reconcileVisibility = useCallback(() => {
    const scene = sceneRef.current
    if (scene === null) return
    // The latest sets, not this render's: a re-seed earlier in the same tick
    // (a bus hide) has already replaced them.
    const keys = hiddenKeysRef.current
    const tagPaths = hiddenTagPathsRef.current
    const seeded = seedHiddenTagPathsFromRegistry(scene)
    if (seeded.size === 0 && tagPaths.size === 0 && keys.size === 0) return
    if (seeded.size !== tagPaths.size || [...seeded].some((p) => !tagPaths.has(p))) {
      setHiddenTagPaths(seeded)
      hiddenTagPathsRef.current = seeded
    }
    pushUnionHidden(keys, seeded, { onlyIfChanged: true })
  }, [pushUnionHidden])
  reconcileVisibilityRef.current = reconcileVisibility

  const handleObjectInfoTagsChanged = useCallback(() => {
    reconcileVisibility()
    handleDocumentChanged()
  }, [reconcileVisibility, handleDocumentChanged])

  // Delete a tag everywhere (undoable, kernel-side): unassigns it — and its
  // sub-tags — from every node and drops the registry entries. Geometry is
  // never deleted; content hidden solely via the deleted tag becomes visible
  // again through the registry re-seed.
  const handleDeleteTag = useCallback((path: string[]) => {
    const scene = sceneRef.current
    if (scene === null) return
    try {
      scene.delete_tag(path.join('/'))
    } catch (err: unknown) {
      handleToast(t`Delete tag failed: ${ph({ reason: friendlyErrorText(err) })}`)
      return
    }
    resyncTagVisibility()
    handleDocumentChanged()
  }, [resyncTagVisibility, handleDocumentChanged, handleToast])
  deleteTagRef.current = handleDeleteTag

  // Rename a tag in place (Tags panel: second click on the active row, or a
  // double-click) — the LAST segment only; parents stay. Undoable and
  // identity-preserving in the kernel (`rename_tag`): the registry entry
  // keeps its stable id and hidden flag, so `hiddenTagPaths` (keyed by
  // PATH) is re-seeded from the registry afterwards, exactly like delete.
  // Returns the inline error text for the panel, or null on success.
  const handleRenameTag = useCallback((path: string[], newSegment: string): string | null => {
    const scene = sceneRef.current
    if (scene === null) return t`No document.`
    const next = [...path.slice(0, -1), newSegment]
    try {
      scene.rename_tag(path.join('/'), next.join('/'))
    } catch (err: unknown) {
      return friendlyErrorText(err)
    }
    resyncTagVisibility()
    handleDocumentChanged()
    return null
  }, [resyncTagVisibility, handleDocumentChanged])

  // ---------------------------------------------------------------- materials: rename/delete

  /** Rename a palette material (MaterialPalette's inline Finder-style
   *  editor). Returns inline error text or null on success, mirroring
   *  handleRenameTag. */
  const handleRenameMaterial = useCallback((id: bigint, name: string): string | null => {
    const scene = sceneRef.current
    if (scene === null) return t`No document.`
    try {
      scene.set_material_name(id, name)
    } catch (err: unknown) {
      return friendlyErrorText(err)
    }
    handleDocumentChanged()
    return null
  }, [handleDocumentChanged])

  /** Actually deletes the material — the shared tail of both the
   *  immediate (unused) and confirmed (used) delete paths. */
  const doDeleteMaterial = useCallback((id: bigint, name: string) => {
    const scene = sceneRef.current
    if (scene === null) return
    try {
      scene.delete_material(id)
    } catch (err: unknown) {
      handleToast(t`Delete material failed: ${ph({ reason: friendlyErrorText(err) })}`)
      return
    }
    // Unpainting a face/object-base is a rendering change, not just
    // bookkeeping — re-tessellate so the viewport actually shows it.
    refreshAfterPanelMutation()
    // The Paint tool must never keep a deleted material as "current".
    setCurrentMaterialId((cur) => (cur === id ? MATERIAL_SENTINEL : cur))
    handleToast(t`Deleted material "${name}".`)
  }, [refreshAfterPanelMutation, handleToast])

  /** MaterialPalette's × button: an unused material deletes immediately;
   *  one used on any face/object-base opens the confirm dialog naming how
   *  many (docs/design/v1.1-cycle.md Lane A). */
  const handleDeleteMaterial = useCallback((id: bigint) => {
    const scene = sceneRef.current
    if (scene === null) return
    const info = scene.material_info(id)
    const name = info?.name() ?? 'Material'
    const usage = scene.material_usage(id)
    if (usage === 0) {
      doDeleteMaterial(id, name)
      return
    }
    setPendingMaterialDelete({ id, name, usage })
  }, [doDeleteMaterial])

  // ---------------------------------------------------------------- components: rename/delete

  /** Rename a component definition from the Components panel. */
  const handleRenameComponent = useCallback((id: bigint, name: string): string | null => {
    const scene = sceneRef.current
    if (scene === null) return t`No document.`
    try {
      scene.set_component_name(id, name)
    } catch (err: unknown) {
      return friendlyErrorText(err)
    }
    handleNameChanged()
    return null
  }, [handleNameChanged])

  const doDeleteComponent = useCallback((id: bigint, name: string) => {
    const scene = sceneRef.current
    if (scene === null) return
    try {
      scene.delete_definition(id)
    } catch (err: unknown) {
      handleToast(t`Delete component failed: ${ph({ reason: friendlyErrorText(err) })}`)
      return
    }
    // Every instance vanishing from the scene is a rendering change.
    refreshAfterPanelMutation()
    handleToast(t`Deleted component "${name}".`)
  }, [refreshAfterPanelMutation, handleToast])

  /** ComponentsPanel's × button: a definition with no live instances
   *  deletes immediately; one with instances opens the confirm dialog
   *  naming how many. */
  const handleDeleteComponent = useCallback((id: bigint) => {
    const scene = sceneRef.current
    if (scene === null) return
    const rawName = scene.component_name(id)
    // Same positional fallback ComponentsPanel's row uses, so the name in
    // the confirm dialog matches the button the user just clicked.
    const index = Array.from(scene.component_ids()).indexOf(id)
    const name = rawName !== undefined && rawName.length > 0 ? rawName : entityLabel('instance', index)
    const usage = scene.definition_usage(id)
    if (usage === 0) {
      doDeleteComponent(id, name)
      return
    }
    setPendingComponentDelete({ id, name, usage })
  }, [doDeleteComponent])

  // ---------------------------------------------------------------- purge unused

  /** File ▸ Purge Unused… / the Components panel's header button: preview
   *  what would go (docs/design/v1.1-cycle.md Lane A), or toast "Nothing
   *  to purge" outright when there is nothing pending — no modal for a
   *  no-op. */
  const requestPurgeUnused = useCallback(() => {
    const scene = sceneRef.current
    if (scene === null) return
    const materialIds = Array.from(scene.unused_materials())
    const definitionIds = Array.from(scene.unused_definitions())
    if (materialIds.length === 0 && definitionIds.length === 0) {
      handleToast(t`Nothing to purge.`)
      return
    }
    const materialNames = materialIds.map((id) => scene.material_info(id)?.name() ?? t`Material`)
    const definitionNames = definitionIds.map((id) => {
      const n = scene.component_name(id)
      return n !== undefined && n.length > 0 ? n : t`Component`
    })
    setPendingPurgePreview({ materialNames, definitionNames })
  }, [handleToast, t])

  const confirmPurgeUnused = useCallback(() => {
    const scene = sceneRef.current
    setPendingPurgePreview(null)
    if (scene === null) return
    let report: ReturnType<Scene['purge_unused']>
    try {
      report = scene.purge_unused()
    } catch (err: unknown) {
      handleToast(t`Purge failed: ${ph({ reason: friendlyErrorText(err) })}`)
      return
    }
    // Purged materials/definitions can remove geometry (a purged
    // definition's instances) or change face rendering — re-tessellate.
    refreshAfterPanelMutation()
    // The Paint tool must never keep a purged material as "current": a
    // purged handle no longer resolves in the palette at all.
    setCurrentMaterialId((cur) =>
      cur !== MATERIAL_SENTINEL && scene.material_info(cur) === undefined ? MATERIAL_SENTINEL : cur,
    )
    const purgedDefinitions = report.definitions()
    const purgedMaterials = report.materials()
    handleToast(
      t`Purged ${plural(purgedDefinitions, { one: '# definition', other: '# definitions' })}, ${plural(purgedMaterials, { one: '# material', other: '# materials' })}.`,
    )
  }, [refreshAfterPanelMutation, handleToast])

  if (error !== null) {
    return (
      <main style={{ fontFamily: 'sans-serif', padding: '1rem', color: 'var(--danger-base, red)' }}>
        <h1><Trans>Hew — kernel load error</Trans></h1>
        <pre>{error}</pre>
      </main>
    )
  }

  // Confirmation-dialog bodies: whole sentences with the counts pluralized.
  const matDeleteName = pendingMaterialDelete?.name ?? ''
  const matDeleteUsage = pendingMaterialDelete?.usage ?? 0
  const materialDeleteBody = t`"${matDeleteName}" — used on ${plural(matDeleteUsage, { one: '# face', other: '# faces' })} — delete and unpaint them?`
  const compDeleteName = pendingComponentDelete?.name ?? ''
  const compDeleteUsage = pendingComponentDelete?.usage ?? 0
  const componentDeleteBody = t`Delete "${compDeleteName}" and its ${plural(compDeleteUsage, { one: '# instance', other: '# instances' })}?`
  const purgeDefinitionCount = pendingPurgePreview?.definitionNames.length ?? 0
  const purgeDefinitionList = pendingPurgePreview?.definitionNames.join(', ') ?? ''
  const purgeDefinitionsLine =
    purgeDefinitionCount > 0
      ? t`${plural(purgeDefinitionCount, { one: '# definition', other: '# definitions' })}: ${purgeDefinitionList}`
      : null
  const purgeMaterialCount = pendingPurgePreview?.materialNames.length ?? 0
  const purgeMaterialList = pendingPurgePreview?.materialNames.join(', ') ?? ''
  const purgeMaterialsLine =
    purgeMaterialCount > 0
      ? t`${plural(purgeMaterialCount, { one: '# material', other: '# materials' })}: ${purgeMaterialList}`
      : null
  // Modifier chord shown in the Precision-snap chip: a keyboard shortcut label, not translated.
  // oxlint-disable-next-line lingui/no-unlocalized-strings -- keyboard shortcut label
  const precisionKeys = isMac ? '⌘⌥' : 'Ctrl+Alt'
  if (state === null) {
    return (
      <main style={{ fontFamily: 'sans-serif', padding: '1rem' }}>
        <p><Trans>Loading kernel…</Trans></p>
      </main>
    )
  }

  // Status badge summary, from the cheap whole-document health summary.
  // Not being solid is normal, so every object that isn't solid (panels and
  // sheets and kept ones included) counts, and the badge is neutral.
  const objectCount = healthSummary.objects
  const notSolidCount = healthSummary.open + healthSummary.surfaces

  // Selection-gated command availability — see the menuGates memo above.
  const booleanOperands = menuGates?.booleanOperands ?? []
  const canBoolean = menuGates?.canBoolean ?? false
  const canGroupNow = menuGates?.canGroup ?? false
  const canUngroupNow = menuGates?.canUngroup ?? false
  const canMakeComp = menuGates?.canMakeComponent ?? false
  const canPlace = menuGates?.canPlaceCopy ?? false
  const canExplode = menuGates?.canExplode ?? false
  const canUnique = menuGates?.canMakeUnique ?? false
  const handleBoolean = (op: number) => {
    // Re-check the full gate (the accelerator path dispatches here
    // unconditionally, like handleGroup): two or more operands, each a plain
    // solid, a group or an instance, at the top level, passed in selection
    // order so the first selected is what a Subtract keeps.
    if (!(menuGates?.canBoolean ?? false)) return
    const result = viewportApi.current?.runBoolean(op, booleanOperands)
    if (result === 'mutated-failed') {
      // The retried boolean failed AFTER auto-explode already committed
      // real mutations (make_unique/explode_instance/group_nodes) —
      // Viewport already refreshed the scene and pushed the exploded
      // pieces as the new selection (onSelectMany) and toasted the honest
      // recovery hint; this is not a no-op, so the outliner/watertight
      // status must still be re-derived from the document.
      setDocRev((r) => r + 1)
    } else if (result != null) {
      setSelectedIds([result])
      setDocRev((r) => r + 1)
    }
  }

  const handleGroup = () => {
    // Re-check eligibility: the keyboard accelerator (Ctrl+G) dispatches here
    // unconditionally, unlike the menu items sync_menu_state disables, so the
    // handler must enforce the same gate — ≥2 distinct siblings, no sketches.
    // Without this a single selected node becomes a silent 1-member group
    // (the kernel accepts any non-empty sibling list).
    if (!(menuGates?.canGroup ?? false)) return
    const newGroupId = viewportApi.current?.runGroup(selectedIds)
    if (newGroupId != null) {
      setSelectedIds([{ kind: 'group', id: newGroupId }])
      setDocRev((r) => r + 1)
    }
  }

  // Outliner drag-and-drop (DocumentTree's `dropTargetFor` already vetted
  // the drop before calling this — GroupCycle/ExplodeSessionScope are the
  // kernel's own backstop, surfaced as a toast by `runReparent` if a race
  // slips one through). Keeps the moved nodes selected — matches
  // `handleGroup`'s "select what you just acted on" posture — which also
  // auto-expands/reveals the target group via the existing
  // `ancestorGroupKeys` machinery (DocumentTree.tsx).
  const handleReparent = (nodes: NodeRef[], group: bigint | undefined) => {
    const ok = viewportApi.current?.runReparent(nodes, group)
    if (ok === true) {
      // Moving a node into (or out of) a hidden group hides (or shows) it.
      reconcileVisibility()
      setSelectedIds(nodes)
      setDocRev((r) => r + 1)
    }
  }

  const handleUngroup = () => {
    if (selectedIds.length === 1 && selectedIds[0].kind === 'group') {
      viewportApi.current?.runUngroup(selectedIds[0].id)
      setSelectedIds([])
      setDocRev((r) => r + 1)
    }
  }

  const handleMakeComponent = () => {
    // Same re-check handleGroup does: dispatchers without a disabled state
    // (dock verbs are hidden, but keyboard/palette paths aren't) must not
    // hand the kernel an ineligible selection.
    if (!(menuGates?.canMakeComponent ?? false)) return
    const instanceId = viewportApi.current?.runMakeComponent(selectedIds)
    if (instanceId != null) {
      setSelectedIds([{ kind: 'instance', id: instanceId }])
      setDocRev((r) => r + 1)
    }
  }

  const handlePlaceInstance = () => {
    if (selectedIds.length === 1 && selectedIds[0].kind === 'instance') {
      const newId = viewportApi.current?.runPlaceInstance(selectedIds[0].id)
      if (newId != null) {
        setSelectedIds([{ kind: 'instance', id: newId }])
        setDocRev((r) => r + 1)
      }
    }
  }

  const handleExplodeInstance = () => {
    if (selectedIds.length === 1 && selectedIds[0].kind === 'instance') {
      const objectIds = viewportApi.current?.runExplodeInstance(selectedIds[0].id)
      if (objectIds != null && objectIds.length > 0) {
        setSelectedIds(objectIds.map((id) => ({ kind: 'object' as const, id })))
        setDocRev((r) => r + 1)
      } else if (objectIds != null) {
        setSelectedIds([])
        setDocRev((r) => r + 1)
      }
    }
  }

  const handleMakeUnique = () => {
    if (selectedIds.length === 1 && selectedIds[0].kind === 'instance') {
      const kept = viewportApi.current?.runMakeUnique(selectedIds[0].id)
      if (kept != null) {
        // Keep the same instance selected — its id is unchanged.
        setDocRev((r) => r + 1)
      }
    }
  }

  return (
    <main
      style={{
        fontFamily: 'var(--font-family-ui, sans-serif)',
        position: 'relative',
        display: 'flex',
        flexDirection: 'column',
        height: '100%',
        boxSizing: 'border-box',
        overflow: 'hidden',
        background: 'var(--surface-canvas-page, #1a1a1a)',
      }}
    >
      {/* Linux desktop shell only: borderless window → draw our own title bar
          (KWin/WebKitGTK won't repaint the native one after setTitle). Windows
          and macOS keep native decorations, so their OS caption (which reflects
          setTitle) is the title bar — no custom one. */}
      {isTauri && isLinux && (
        <TitleBar name={documentName(docSession)} saveState={saveStateLabel(docSession, nowTick)} />
      )}

      {/* App bar / menu bar.
          On macOS Tauri the native OS menu bar owns File/Edit (this renders
          nothing). On the web build and the Linux/Windows borderless shells,
          the in-app bar renders the menus (Linux settled on in-app chrome
          over the native GTK menubar — trialed and rejected); the
          centered title is shown by TitleBar on Linux/Windows so it is
          hidden here in that case. */}
      <MenuBar
        name={documentName(docSession)}
        saveState={saveStateLabel(docSession, nowTick)}
        nativeMenuBar={isTauri && isMac}
        hideTitle={isTauri && (isLinux || isWindows)}
        onNew={newDocument}
        onOpen={openDocument}
        onSave={saveDocument}
        onSaveAs={saveAsDocument}
        onRevertToSaved={() => setRevertPromptOpen(true)}
        revertToSavedDisabled={!revertable}
        onImport={importDocument}
        onImportImage={() => void importImage()}
        onExport={() => void openExport()}
        onPrint={() => setPrintDialogOpen(true)}
        onSaveToLibrary={() => menuActionRef.current('save-to-library-doc')}
        saveToLibraryDisabled={!libraryStore().available()}
        onPurgeUnused={() => menuActionRef.current('purge-unused')}
        onDrawText={openTextDialog}
        onClose={
          isTauri && !isMac
            ? () => {
                import('@tauri-apps/api/window')
                  .then(({ getCurrentWindow }) => getCurrentWindow().close())
                  .catch(() => { /* ignore */ })
              }
            : undefined
        }
        onExit={
          isTauri && !isMac
            ? () => {
                import('@tauri-apps/api/window')
                  .then(async ({ getAllWindows }) => {
                    // Close every window (each document window's close-guard
                    // still prompts for its own unsaved changes); the app exits
                    // once the last one is gone.
                    for (const w of await getAllWindows()) {
                      await w.close().catch(() => { /* keep closing the rest */ })
                    }
                  })
                  .catch(() => { /* ignore */ })
              }
            : undefined
        }
        recentFiles={recentFiles}
        onOpenRecent={openRecent}
        onClearRecent={clearRecent}
        onUndo={handleUndo}
        onRedo={handleRedo}
        canUndo={canUndo}
        canRedo={canRedo}
        undoLabel={undoLabel}
        redoLabel={redoLabel}
        activeTool={activeTool}
        onSelectTool={(name) => activateTool(name as ToolName)}
        // A section's check mark means "on screen": with the tray hidden
        // every section reads unchecked, and choosing one brings the tray
        // back with it open (toggleSection).
        showModelInfo={showModelInfo && !trayCollapsed}
        showMaterials={showMaterials && !trayCollapsed}
        showComponents={showComponents && !trayCollapsed}
        showTags={showTags && !trayCollapsed}
        showScenes={showScenes && !trayCollapsed}
        showChanges={showChanges && !trayCollapsed}
        showObjectInfo={showObjectInfo && !trayCollapsed}
        showTray={!trayCollapsed}
        onToggleTray={toggleTray}
        compactRail={railNarrow}
        onToggleCompactRail={toggleRailNarrow}
        showDock={!dockHidden}
        onToggleDock={toggleDock}
        showDebugLog={showDebugLog}
        showLibrary={showLibrary}
        onToggleModelInfo={() => toggleSection(setShowModelInfo, trayTitle.outliner)}
        onToggleMaterials={() => toggleSection(setShowMaterials, trayTitle.materials)}
        onToggleComponents={() => toggleSection(setShowComponents, trayTitle.components)}
        onToggleTags={() => toggleSection(setShowTags, trayTitle.tags)}
        onToggleScenes={() => toggleSection(setShowScenes, trayTitle.scenes)}
        onToggleChanges={() => toggleSection(setShowChanges, trayTitle.changes)}
        onToggleObjectInfo={() => toggleSection(setShowObjectInfo, trayTitle.objectInfo)}
        onToggleDebugLog={() => setShowDebugLog((v) => !v)}
        onToggleLibrary={() => setShowLibrary((v) => !v)}
        onScenesAdd={addSceneFromMenu}
        onScenesUpdate={() => menuActionRef.current('scenes-update')}
        sceneUpdateEnabled={scenes.activeSid !== null}
        onScenesNext={() => scenes.next()}
        onScenesPrevious={() => scenes.previous()}
        scenesNavEnabled={scenes.entries.length > 0}
        sceneTransitionsChecked={sceneTransitionsOn}
        onToggleSceneTransitions={() => setSceneTransitions(!sceneTransitionsOn)}
        showAxes={showAxes}
        showGrid={showGrid}
        showGuides={showGuides}
        onToggleAxes={() => {
          // The axes flag is recorded in the document's view style, which a
          // running plugin command holds (docs/agents/PLUGINS.md "Compounds").
          if (sceneRef.current?.busy()) return
          setShowAxes((v) => !v)
        }}
        onResetAxes={() => menuActionRef.current('reset-axes')}
        onToggleGrid={() => setShowGrid((v) => !v)}
        xray={xrayOn}
        wireframe={wireframeOn}
        onToggleXray={toggleXray}
        onToggleWireframe={toggleWireframe}
        onToggleGuides={() => setShowGuides((v) => !v)}
        onDeleteGuides={() => viewportApi.current?.deleteAllGuides()}
        sectionPlaneChecked={sectionPlaneMenuState.checked}
        sectionPlaneExists={sectionPlaneMenuState.exists}
        onToggleSectionActive={() => viewportApi.current?.toggleSectionActive()}
        onDelete={deleteSelection}
        onEditAction={(id) => menuActionRef.current(id)}
        editGates={{
          canGroup: canGroupNow,
          canUngroup: canUngroupNow,
          canMakeComponent: canMakeComp,
          canPlaceCopy: canPlace,
          canExplode: canExplode,
          canMakeUnique: canUnique,
          canBoolean,
          canSaveSelectionToLibrary: menuGates?.canSaveSelectionToLibrary ?? false,
          canImport: !componentFrameOpen,
          canDrawText: !componentFrameOpen,
          hasStructuralSelection: menuGates?.hasStructuralSelection ?? false,
          clipboardHasContent: menuGates?.clipboardHasContent ?? false,
        }}
        onZoomExtents={handleZoomExtents}
        onStandardView={(view) => viewportApi.current?.setStandardView(view)}
        parallelProjectionChecked={parallelProjection}
        onToggleParallelProjection={() => viewportApi.current?.toggleProjection()}
        flyChecked={fly.mode === 'latched'}
        onToggleFly={() => viewportApi.current?.toggleFlyLatched()}
        onOpenSettings={openSettings}
        onOpenGuide={handleOpenGuide}
        onOpenPalette={() => openPalette()}
        onHelpSearch={(q) => openPalette(q)}
        onReportBug={handleReportBug}
        onSentReports={() => handleSentReports()}
        sentReportsNew={sentReportsNew}
        onCheckForUpdates={updaterAvailable ? handleCheckForUpdates : undefined}
        onOpenOnPhone={isTauri ? () => setPhoneShareOpen(true) : undefined}
        openOnPhoneDisabled={phoneShareDisabled}
        onEnterShopMode={!isTauri && isCoarsePointer() ? handleEnterShopMode : undefined}
        windowList={isTauri ? windowList : undefined}
        onFocusWindow={focusWindow}
      />

      {/* Reply notice for followed bug reports (follow-a-report §2.3). */}
      <FollowNoticeBar onView={handleSentReports} />

      {/* Kernel panic sticky banner */}
      {kernelPanicked && (
        <div
          style={{
            background: '#8b0000',
            color: '#fff',
            padding: '10px 16px',
            display: 'flex',
            alignItems: 'center',
            gap: '12px',
            fontFamily: 'monospace',
            fontSize: '13px',
            zIndex: 200,
            flexShrink: 0,
          }}
        >
          <span style={{ flex: 1 }}>
            <Trans>A kernel error occurred — the session is no longer usable. Reload the page to recover.</Trans>
          </span>
          {/* Before Reload, which forgets the file an open died on
              (failedOpen.ts) and the steps that led here. */}
          <button
            onClick={handleReportBug}
            style={{
              padding: '4px 14px',
              background: 'transparent',
              color: '#fff',
              border: '1px solid #fff',
              borderRadius: '3px',
              fontFamily: 'monospace',
              fontSize: '13px',
              fontWeight: 'bold',
              cursor: 'pointer',
            }}
          >
            <Trans>Report Bug…</Trans>
          </button>
          <button
            onClick={() => window.location.reload()}
            style={{
              padding: '4px 14px',
              background: '#fff',
              color: '#8b0000',
              border: 'none',
              borderRadius: '3px',
              fontFamily: 'monospace',
              fontSize: '13px',
              fontWeight: 'bold',
              cursor: 'pointer',
            }}
          >
            <Trans>Reload</Trans>
          </button>
        </div>
      )}

      {/* Body row: labeled left tool rail (`03_tool_rail.md`), the
          viewport, and the docked right tray (`06_docked_panels.md`)
          — the app-shell's full 3-column layout (`02_app_shell.md`). */}
      <div style={{ flex: 1, minHeight: 0, display: 'flex' }}>
        <ToolRail
          activeTool={activeTool}
          onSelectTool={(name) => activateTool(name)}
          // The resting palette field lives at the top of the rail on every
          // platform ( — macOS forced it here since it has no in-window
          // menu bar, and the rest follow for consistency). The chip shows
          // the palette's key from the keymap (Cmd/Ctrl+`/` by default).
          onOpenPalette={() => openPalette()}
          // oxlint-disable-next-line lingui/no-unlocalized-strings -- command id
          paletteKbd={shortcutLabel('open-palette') || undefined}
          // The BROWSE entry point stays visible on every platform — the
          // dialog itself reports the web build's storage honestly
          // unavailable (matching Settings ▸ Folders' posture); only the
          // SAVE affordances hide where no store exists.
          onOpenLibrary={openLibraryEntry}
          libraryOpen={showLibrary}
          narrow={railNarrow}
          onToggleNarrow={toggleRailNarrow}
        />
        <div
          style={{ flex: 1, minWidth: 0, position: 'relative' }}
          onDragOver={handleDragOver}
          onDrop={handleDrop}
          // The inference cursor (snap dot + tooltip chip) tracks the pointer
          // inside the viewport; once the pointer leaves for the rail/tray it
          // would otherwise freeze at the edge and its label could sit on top
          // of panel content. Clear it on the way out.
          onPointerLeave={() => setInferenceInfo(null)}
        >
          {trayCollapsed && (
            <TrayShowTab onClick={showTray} focusOnMount={trayToggleTakesFocus} />
          )}
          <Viewport
            wasmScene={state.scene}
            onStatusChange={handleStatusChange}
            onSceneChange={handleSceneChange}
            onToast={handleToast}
            onToolHint={setToolStageHint}
            onPrecisionChange={setPrecisionSnap}
            activeTool={activeTool}
            activeToolSeq={toolActivationSeq}
            onInternalToolChange={handleInternalToolChange}
            activeContext={activeContext}
            selectedIds={selectedIds}
            activeLitSet={activeLitSet}
            onSelect={handleViewportSelect}
            onSelectSnap={handleSelectSnap}
            onSelectMany={handleSelectMany}
            onSelectGuide={handleSelectGuide}
            selectedGuide={selectedGuide}
            onSelectAnnotation={handleSelectAnnotation}
            selectedAnnotation={selectedAnnotation}
            onOpenAnnotationEditor={handleOpenAnnotationEditor}
            onEnterContext={handleEnterContext}
            onExitContext={handleExitContext}
            onIsolationChange={handleIsolationChange}
            onIsolationTargetsChange={handleIsolationTargetsChange}
            onExitAllContexts={handleExitAllContexts}
            onSessionChange={handleSessionChange}
            onDocumentChanged={handleDocumentChanged}
            onSectionChanged={handleSectionChanged}
            onCameraSettled={scenes.refreshDrift}
            onHistoryChanged={handleHistoryChanged}
            apiRef={viewportApi}
            onMeasurement={handleMeasurement}
            onRescaleArmed={handleRescaleArmed}
            onInferenceChange={handleInferenceChange}
            onCameraDragChange={setCameraDragging}
            onHoverSketchRegionChange={setHoveringSketchRegion}
            currentMaterialId={currentMaterialId}
            onProjectionChange={(projection) => setParallelProjection(projection === 'parallel')}
            onToolReverted={() => setActiveTool('Select')}
            onSampleMaterial={setCurrentMaterialId}
            onFlyChange={setFly}
          />

          {/* Inference & viewport feedback (`07_inference_feedback.md`)
              — all net-new DOM overlays; Viewport.tsx rendered none of this
              before this milestone. */}
          <SnapDot info={inferenceInfo} />
          <InferenceTooltip info={inferenceInfo} detail={hoverDetail} />
          {/* The in-viewport annotation text editor (docs/design/
              dimensions-text.md): double-click-to-edit an existing dimension/
              leader, or the Text tool's second click wording a new leader. */}
          {annotationEditor !== null && (
            <AnnotationEditor
              screenX={annotationEditor.screenX}
              screenY={annotationEditor.screenY}
              initialText={annotationEditor.initialText}
              placeholder={annotationEditor.placeholder}
              onCommit={handleCommitAnnotationEditor}
              onCancel={handleCancelAnnotationEditor}
            />
          )}
          {/* Overlays that live INSIDE the viewport container: hovering them
              also hides the inference cursor (the container's pointerleave
              can't see moves into its own children). display:contents keeps
              them out of the layout while still catching the bubbled events. */}
          <div style={{ display: 'contents' }} onPointerOver={() => setInferenceInfo(null)}>
            {/* While flying with no tool reading live, the box shows the
                flight speed instead (fly-navigation.md §3.4) — a tool's own
                measurement (a Line's length mid-gesture) always wins, since
                that is the number the gesture is about. */}
            {measurement === '' && fly.mode !== 'off' ? (
              <MeasurementBox toolName={toolName} value={fly.speedText} label={t`Fly speed`} frozen position={measurementsPosition} />
            ) : (
              <MeasurementBox toolName={toolName} value={measurement} frozen={measurementFrozen} position={measurementsPosition} />
            )}
            <ViewDie apiRef={viewportApi} />
          </div>

          {/* Selection path (docs/design/selection-path.md): where you are
              editing and what is selected, in one strip. It replaces the old
              "Editing X" pill — open frames read as crumbs carrying the
              Outliner's `editing` chip. */}
          <div style={{ display: 'contents' }} onPointerOver={() => setInferenceInfo(null)}>
            <SelectionPathStrip
              crumbs={pathCrumbs}
              onSelect={handlePathSelect}
              onModel={handlePathModel}
              onFrame={(depth) => {
                // The innermost open frame has nothing to close.
                if (frameClickAction(depth, sessionStack.length, activeContext.length) === 'noop') return
                handleSetPathDepth(depth + 1)
              }}
              onEnter={enterNode}
            />
          </div>

          {/* Make Solid wizard: over the viewport's right side, non-modal. */}
          {makeSolid !== null && state !== null && (
            <MakeSolidWizard
              key={makeSolid.nonce}
              scene={state.scene}
              scope={makeSolid.scope}
              objectIds={makeSolid.objectIds}
              docRev={docRev}
              docGeneration={docGeneration}
              onClose={() => setMakeSolid(null)}
              onIsolate={handleIsolate}
              onClearIsolation={handleClearIsolation}
              onHealthOverlay={handleWizardOverlay}
              isolationLost={wizardIsolationLost}
              escapeBusy={wizardEscapeBusy}
              onGeometryChanged={refreshAfterPanelMutation}
              onToast={handleToast}
              onBusy={runBusy}
              onWidthChange={setFixWizardWidth}
            />
          )}

          {makeSolidSketches !== null && state !== null && (
            <MakeSolidSketches
              key={makeSolidSketches.nonce}
              scene={state.scene}
              sketchIds={makeSolidSketches.sketchIds}
              docRev={docRev}
              docGeneration={docGeneration}
              onClose={() => setMakeSolidSketches(null)}
              onHealthOverlay={handleWizardOverlay}
              escapeBusy={wizardEscapeBusy}
              onGeometryChanged={refreshAfterPanelMutation}
              onSelectObject={(id) => handleSelect({ kind: 'object', id }, 'replace')}
              onToast={handleToast}
              onWidthChange={setFixWizardWidth}
            />
          )}

          {/* Model Health: a floating, non-modal window over the viewport,
              closed at every launch. The panel unmounts with it, which clears
              its overlay and its isolation. */}
          {showModelHealth && state !== null && (
            <FloatingWindow
              title={MENU.modelHealth}
              storageKey="hew.ui.modelHealthWindow"
              onClose={() => setShowModelHealth(false)}
              reservedRight={makeSolid !== null || makeSolidSketches !== null ? fixWizardWidth : 0}
              onCoverChange={setHealthCover}
              focusNonce={healthFocusNonce}
              data-model-health-window
            >
              <ModelHealthPanel
                scene={state.scene}
                docRev={docRev}
                selectedIds={selectedIds}
                scope={healthScope}
                onScopeChange={setHealthScope}
                objectFilter={healthObjectFilter}
                onClearObjectFilter={() => setHealthObjectFilter(null)}
                onSelectNode={handleHealthSelectNode}
                onIsolate={handlePanelIsolate}
                onClearIsolation={handlePanelClearIsolation}
                onFrameBounds={handleHealthFrame}
                onOverlayChange={handleHealthOverlay}
                onGeometryChanged={refreshAfterPanelMutation}
                onToast={handleToast}
                onBusy={runBusy}
              />
            </FloatingWindow>
          )}

          {/* View-only isolation (Model Health): what is shown, and the way back. */}
          {isolatedCount !== null && (
            <IsolationBar
              count={isolatedCount}
              onShowEverything={handleClearIsolation}
              rightInset={makeSolid !== null || makeSolidSketches !== null ? fixWizardWidth : 0}
            />
          )}

          {/* Library placement pill (frame 1c): shown for the one-shot
              cursor-placement gesture LibraryPlaceTool drives — a
              viewport-level tool, not a rail selection, so the status bar's
              usual "active tool" chrome doesn't name it (see App.tsx's
              LibraryPlaceTool status-hint note). Cleared by the arm
              callback on both cancel and commit. Offset below the selection-path
              strip when it is showing (editing inside an open group/
              component while placing a library item) rather than overlap it. */}
          {insertingItem !== null && (
            <div
              style={{
                position: 'absolute',
                top: pathCrumbs.length > 0 ? '54px' : '14px',
                left: '50%',
                transform: 'translateX(-50%)',
                display: 'flex',
                alignItems: 'center',
                gap: '8px',
                padding: '6px 12px',
                background: 'var(--surface-overlay)',
                border: '1px solid var(--border-strong)',
                borderRadius: '7px',
                fontFamily: 'var(--font-family-ui)',
                fontSize: '11.5px',
                color: 'var(--text-secondary)',
                whiteSpace: 'nowrap',
                zIndex: 20,
                pointerEvents: 'none',
              }}
            >
              <span>
                <Trans>
                  Inserting <b style={{ color: 'var(--text-primary)' }}>{insertingItem}</b>
                </Trans>
              </span>
              <span
                style={{
                  fontFamily: 'var(--font-family-mono)',
                  fontSize: 'var(--font-size-kbd)',
                  fontWeight: 600,
                  padding: '1.5px 5px',
                  borderRadius: 'var(--radius-kbd)',
                  background: 'var(--kbd-bg)',
                  border: '1px solid var(--kbd-border)',
                  color: 'var(--kbd-text)',
                }}
              >
                <Trans comment="Escape key cap in the placement pill">Esc</Trans>
              </span>
              <span style={{ color: 'var(--text-faint)' }}><Trans>cancel</Trans></span>
            </div>
          )}

          {/* Contextual dock — bottom-center, self-hides only when
              there's no curated verb set for the current selection (a
              construction guide — a selected sketch gets its own 'sketch'
              context as of), and fades out while the camera is being
              dragged. Reuses the same menuActionRef dispatch the
              palette and every menu item already go through.
              activeToolId tells the dock which verb, if any,
              is the ACTUAL active tool — so it never shows a stale "selected"
              verb (e.g. Rectangle) while a different tool (e.g. Arc) is live.
              hoveringSketchRegion previews the Push/Pull verb when
              nothing is selected and the cursor is aimed at a sketch region
              — an explicit selection's dock always wins over this hint.
              View ▸ Contextual Dock turns it off entirely. */}
          {!dockHidden && (
          <div style={{ display: 'contents' }} onPointerOver={() => setInferenceInfo(null)}>
            <ContextualDock
              selectedIds={selectedIds}
              selectedGuide={selectedGuide}
              hidden={cameraDragging}
              activeToolId={toolActionId(activeTool)}
              hoveringSketchRegion={hoveringSketchRegion}
              gates={{ canGroup: canGroupNow, canMakeComponent: canMakeComp, canSaveToLibrary: libraryStore().available() }}
              onRun={(id) => menuActionRef.current(id)}
            />
          </div>
          )}

          {/* A plugin command's progress notice, with Cancel. */}
          <BusyNotice />

          {/* Toast stack — positioned inside the viewport container. */}
          <div
            style={{
              position: 'absolute',
              bottom: '16px',
              left: '50%',
              transform: 'translateX(-50%)',
              display: 'flex',
              flexDirection: 'column',
              gap: '6px',
              pointerEvents: 'none',
              zIndex: 100,
            }}
          >
            {toasts.map((toast) => {
              if (toast.state === 'pending') ensureSpinnerKeyframes()
              return (
                <div
                  key={toast.id}
                  data-testid="toast"
                  data-emphasis={toast.emphasis === true ? 'true' : undefined}
                  onClick={() => dismissToast(toast.id)}
                  style={{
                    display: 'flex',
                    alignItems: 'center',
                    gap: '8px',
                    padding: '8px 16px',
                    // Tokenized (Library effort): error stays the saturated
                    // status-leaky fill (white text reads fine on it in both
                    // themes); an ordinary toast now follows the surface-
                    // overlay/text-primary pair instead of a hardcoded dark
                    // chip, so it's no longer illegible against the light
                    // theme's own light canvas.
                    // A guidance toast (emphasis) keeps the opaque overlay
                    // surface — it can land over the floating dock — and
                    // reads as guidance through the accent border and text.
                    background: toast.isError ? 'var(--status-leaky-bg)' : 'var(--surface-overlay)',
                    color: toast.isError ? '#fff' : toast.emphasis === true ? 'var(--accent-text-strong)' : 'var(--text-primary)',
                    border: toast.isError ? 'none' : toast.emphasis === true ? '2px solid var(--accent-base)' : '1px solid var(--border-strong)',
                    borderRadius: '4px',
                    fontFamily: toast.emphasis === true ? 'var(--font-family-ui)' : 'monospace',
                    fontSize: toast.emphasis === true ? '15px' : '13px',
                    fontWeight: toast.emphasis === true ? 600 : undefined,
                    cursor: 'pointer',
                    pointerEvents: 'auto',
                    boxShadow: '0 2px 8px rgba(0,0,0,0.4)',
                    maxWidth: '400px',
                    textAlign: 'center',
                  }}
                >
                  {toast.state === 'pending' && (
                    <span
                      aria-hidden="true"
                      style={{
                        width: '12px',
                        height: '12px',
                        flexShrink: 0,
                        border: '2px solid var(--border-strong)',
                        borderTopColor: 'var(--accent-base)',
                        borderRadius: '50%',
                        animation: 'hew-spin 0.8s linear infinite',
                      }}
                    />
                  )}
                  {toast.state === 'done' && (
                    <span aria-hidden="true" style={{ color: 'var(--status-solid)', flexShrink: 0 }}>✓</span>
                  )}
                  {/* One flex item for the code and the message, so a long
                      message wraps under the code instead of in a narrow
                      column beside it. */}
                  <span>
                    {/* A kernel code is worth showing (it is what a bug report
                        quotes); a guidance toast's code is a styling tag, not
                        something a person should read. */}
                    {toast.code !== undefined && toast.emphasis !== true && (
                      <strong style={{ marginRight: '6px', opacity: 0.8 }}>[{toast.code}]</strong>
                    )}
                    {toast.message}
                  </span>
                </div>
              )
            })}
          </div>
        </div>

        {/* Docked right tray (`06_docked_panels.md`) — permanently
            present, collapsible sections; replaces the old floating,
            draggable panels entirely (FloatingPanel.tsx deleted). Default
            order per spec: Object Info -> Outliner -> Materials, plus Tags
            as a 4th section (not in the spec's default list, but a real
            shipped Hew feature — kept rather than dropped). The showX/setShowX
            state pairs are unchanged from the floating-panel era; they now
            mean "expanded" instead of "visible," so every existing keyboard
            shortcut (Shift+Cmd+I/C/T/O) and Window-menu checkbox keeps
            working with its original meaning. */}
        {/* Tray resize handle — drag to adjust the tray width; the width is
            clamped and persisted so complex models' tag/outliner labels can
            be given room once and keep it across launches. */}
        {!trayCollapsed && (<>
        <div
          role="separator"
          aria-orientation="vertical"
          aria-label={t`Resize panels`}
          onPointerDown={(ev) => {
            ev.preventDefault()
            const startX = ev.clientX
            const startWidth = trayWidth
            const el = ev.currentTarget
            try {
              el.setPointerCapture(ev.pointerId)
            } catch {
              /* a recovered press can name a pointer the engine has already dropped */
            }
            const drag = { width: startWidth }
            const onMove = (mv: PointerEvent) => {
              drag.width = clampTrayWidth(startWidth + (startX - mv.clientX))
              setTrayWidth(drag.width)
            }
            // One teardown for every way the drag can end. pointerup alone
            // is not enough: an interrupted drag (pointercancel, or the
            // browser revoking capture) would otherwise leave the move
            // listener attached with its stale startX — merely hovering the
            // handle afterwards keeps resizing with no button held, and the
            // width never persists.
            // Window-level, because pointer events bubble: this hears the
            // drag whether or not capture held (capture failing above must
            // not strand the move listener).
            const endDrag = () => {
              window.removeEventListener('pointermove', onMove)
              window.removeEventListener('pointerup', endDrag)
              window.removeEventListener('pointercancel', endDrag)
              window.removeEventListener('lostpointercapture', endDrag)
              try {
                el.releasePointerCapture(ev.pointerId)
              } catch {
                /* capture already gone (lostpointercapture path) */
              }
              window.localStorage.setItem(TRAY_WIDTH_KEY, String(drag.width))
            }
            window.addEventListener('pointermove', onMove)
            window.addEventListener('pointerup', endDrag)
            window.addEventListener('pointercancel', endDrag)
            window.addEventListener('lostpointercapture', endDrag)
          }}
          style={{
            width: '5px',
            marginRight: '-5px',
            flexShrink: 0,
            cursor: 'col-resize',
            zIndex: 5,
            // Invisible grab strip riding on the tray's hairline border.
            background: 'transparent',
          }}
        />
        <div
          ref={trayRef}
          role="complementary"
          aria-label={t`Tray`}
          style={{
            width: `${trayWidth}px`,
            flexShrink: 0,
            display: 'flex',
            flexDirection: 'column',
            minHeight: 0,
            overflowY: 'auto',
            background: 'var(--surface-panel)',
            borderLeft: '1px solid var(--border-hairline)',
          }}
        >
          <TrayHideRow onClick={hideTray} focusOnMount={trayToggleTakesFocus} />
          <TraySection title={trayTitle.objectInfo} collapsed={!showObjectInfo} onToggle={() => setShowObjectInfo((v) => !v)}>
            <ObjectInfoPanel
              scene={state.scene}
              docRev={docRev}
              selectedIds={selectedIds}
              selectedAnnotation={selectedAnnotation}
              selectedGuide={selectedGuide}
              // Segments rebuilds a circle's chords, so it needs the repaint
              // path like every other tray panel that edits geometry; names
              // change nothing drawn and skip the re-tessellation.
              onNameChanged={handleNameChanged}
              onGeometryChanged={refreshAfterPanelMutation}
              onTagsChanged={handleObjectInfoTagsChanged}
              onSelectMany={handleReplaceSelection}
              onSelectTag={(path) => revealInTray('tags', path)}
              onSelectMaterial={(id) => revealInTray('materials', id)}
              sessionStack={sessionStack}
              sessionMembers={sessionMembers}
              onShowHealth={handleShowObjectHealth}
              onToast={handleToast}
            />
          </TraySection>
          <TraySection title={trayTitle.outliner} collapsed={!showModelInfo} onToggle={() => setShowModelInfo((v) => !v)}>
            <GeoReferenceInfo scene={state.scene} docRev={docRev} />
            <DocumentTree
              scene={state.scene}
              docRev={docRev}
              docGeneration={docGeneration}
              watertightMap={watertightMap}
              objectHealth={objectHealth}
              selectedIds={selectedIds}
              activeContext={activeContext}
              sessionStack={sessionStack}
              sessionMembers={sessionMembers}
              onSelect={handleOutlinerSelect}
              onEnterContext={enterNode}
              onExitContext={handleExitToModel}
              onSetContextDepth={handleSetPathDepth}
              hiddenKeys={hiddenKeys}
              onToggleHidden={handleToggleHidden}
              onSetHiddenMany={handleSetHiddenMany}
              onReparent={handleReparent}
              onDropRefused={(reason) => handleToast(reason)}
            />
          </TraySection>
          <TraySection title={trayTitle.materials} collapsed={!showMaterials} onToggle={() => setShowMaterials((v) => !v)}>
            <MaterialPalette
              scene={state.scene}
              docRev={docRev}
              docGeneration={docGeneration}
              currentMaterialId={currentMaterialId}
              revealNonce={materialRevealNonce}
              onSelectMaterial={handleSelectMaterial}
              onMaterialCreated={setCurrentMaterialId}
              onDocumentChanged={handleDocumentChanged}
              onAlphaCommitted={() => viewportApi.current?.syncMaterialOpacity()}
              onSaveToLibrary={handleSaveMaterialToLibrary}
              onRenameMaterial={handleRenameMaterial}
              onDeleteMaterial={handleDeleteMaterial}
            />
          </TraySection>
          <TraySection
            title={trayTitle.components}
            collapsed={!showComponents}
            onToggle={() => setShowComponents((v) => !v)}
            headerRight={<PurgeUnusedButton onClick={requestPurgeUnused} />}
          >
            <ComponentsPanel
              scene={state.scene}
              docRev={docRev}
              docGeneration={docGeneration}
              onSelectNodes={handleReplaceSelection}
              onRenameComponent={handleRenameComponent}
              onDeleteComponent={handleDeleteComponent}
            />
          </TraySection>
          <TraySection title={trayTitle.tags} collapsed={!showTags} onToggle={() => setShowTags((v) => !v)}>
            <TagsPanel
              scene={state.scene}
              docRev={docRev}
              hiddenTagPaths={hiddenTagPaths}
              onToggleTagPath={handleToggleTagPath}
              onDeleteTag={handleDeleteTag}
              revealTag={revealTag}
              onSelectNodes={handleReplaceSelection}
              onRenameTag={handleRenameTag}
            />
          </TraySection>
          <TraySection
            title={trayTitle.scenes}
            collapsed={!showScenes}
            onToggle={() => setShowScenes((v) => !v)}
            headerRight={<ScenesAddButton rename={scenesRename} />}
          >
            <ScenesPanel scenes={scenes} rename={scenesRename} />
          </TraySection>
          <TraySection title={trayTitle.changes} collapsed={!showChanges} onToggle={() => setShowChanges((v) => !v)}>
            <ChangesPanel scene={state.scene} docRev={docRev} reasons={docSession.nonUndoableReasons} />
          </TraySection>
        </div>
        </>)}
      </div>

      {/* Status bar — the Studio instructor line (`02_app_shell.md`): active
          tool name in text/tertiary, a dim ·, then a one-line hint in
          text/faint. Snap state moved to the on-cursor inference dot and live
          Measurements to the top-right VCB + near-cursor readout, so both are
          dropped here (Refinement pass, issue E). Every color is now a theme
          token — the old bar hardcoded near-white text (`#eee`), which was
          unreadable on the light theme's light bar (the dark/light divergence
          the user flagged). Extra bottom padding avoids descender clipping
          under macOS Tahoe's rounded window corners. */}
      <div
        style={{
          height: 30,
          boxSizing: 'border-box',
          padding: '0 var(--space-6, 13px) 6px',
          background: 'var(--surface-bar)',
          fontFamily: 'var(--font-family-ui)',
          fontSize: 'var(--font-size-body, 12px)',
          borderTop: '1px solid var(--border-hairline)',
          display: 'flex',
          alignItems: 'center',
          gap: 'var(--space-3, 8px)',
          flexShrink: 0,
        }}
      >
        <span style={{ color: 'var(--text-tertiary)', fontWeight: 600 }}>{toolLabel(toolName)}</span>
        {/* While flying (fly-navigation.md §4), the movement keys replace
            the tool's guidance — shown the moment the right button goes
            down, so a right-drag that used to pan explains itself. The
            tool itself keeps working underneath (its name stays). */}
        {fly.mode !== 'off' ? (
          <>
            <span style={{ color: 'var(--text-section)' }} aria-hidden="true">·</span>
            <span style={{ color: 'var(--text-tertiary)', fontWeight: 600 }} data-testid="fly-hint"><Trans context="camera mode" comment="Status bar label while flying through the model">Fly</Trans></span>
            <span style={{ color: 'var(--text-faint)' }}>
              {fly.mode === 'hold'
                ? t`W S forward/back · A D strafe · E Q up/down · wheel speed · Shift fast`
                : fly.locked
                  ? t`W S forward/back · A D strafe · E Q up/down · mouse or trackpad looks · wheel or pinch sets speed · Shift fast · Esc exits`
                  : fly.canLock
                    ? t`click the view to look with the mouse · W S forward/back · A D strafe · E Q up/down · scroll looks · pinch sets speed · Shift fast · Esc exits`
                    : t`W S forward/back · A D strafe · E Q up/down · scroll looks · pinch sets speed · Shift fast · Esc exits`}
            </span>
            {/* The tool keeps working underneath, so its own stage guidance
                ("Click the next point…") stays visible after the keys —
                a latched flight can last a whole drawing session. */}
            {(toolStageHint ?? '') !== '' && (
              <>
                <span style={{ color: 'var(--text-section)' }} aria-hidden="true">·</span>
                {/* Truncated rather than wrapped: the legend plus a long stage
                    hint would otherwise grow the status bar to two lines. */}
                <span style={{ color: 'var(--text-faint)', minWidth: 0, overflow: 'hidden', whiteSpace: 'nowrap', textOverflow: 'ellipsis' }}>{toolStageHint}</span>
              </>
            )}
          </>
        ) : (toolStageHint ?? toolHint(toolName)) !== '' && (
          /* Stage-aware guidance from the active tool when it provides one
             (Tool.statusHint — "Click the opposite corner…"), else the
             palette's static description; the app always says what to do
             next. */
          <>
            <span style={{ color: 'var(--text-section)' }} aria-hidden="true">·</span>
            <span style={{ color: 'var(--text-faint)' }}>{toolStageHint ?? toolHint(toolName)}</span>
          </>
        )}
        {/* Precision-snapping chip — the only cue that the chord is doing
            anything, and the mode is invisible otherwise (it changes which
            candidate wins, not what is drawn). Sits with the tool guidance
            rather than right-aligned with the document-level watertight badge,
            because it is about the gesture in progress. */}
        {precisionSnap && (
          <>
            <span style={{ color: 'var(--text-section)' }} aria-hidden="true">·</span>
            <span
              style={{
                padding: '2px 8px',
                fontSize: 'var(--font-size-dock-chip, 11px)',
                borderRadius: 4,
                background: 'var(--surface-raised, rgba(127,127,127,0.18))',
                color: 'var(--text-primary)',
                fontWeight: 600,
              }}
            >
              {t`Precision snap (${precisionKeys})`}
            </span>
          </>
        )}
        {/* Model health badge — aggregate solids feedback; no other single-glance
            home (per-object status lives in the Outliner). Right-aligned and
            neutral: not being solid is normal for a SketchUp model. All solid:
            the plain "N objects ✓ solid". Otherwise "N objects · M not solid",
            and clicking it opens the Model Health window. */}
        {objectCount > 0 && (notSolidCount === 0 ? (
          <span
            style={{
              marginLeft: 'auto',
              padding: '2px 8px',
              fontSize: 'var(--font-size-dock-chip, 11px)',
              borderRadius: 4,
              background: 'var(--surface-overlay)',
              color: 'var(--text-secondary)',
            }}
          >
            {badgeText(objectCount, 0)}
          </span>
        ) : (
          <button
            type="button"
            data-health-badge="open"
            onClick={() => openModelHealth()}
            title={i18n._(BADGE_TOOLTIP_LIST)}
            style={{
              marginLeft: 'auto',
              padding: '2px 8px',
              fontSize: 'var(--font-size-dock-chip, 11px)',
              fontFamily: 'inherit',
              borderRadius: 4,
              border: 'none',
              cursor: 'pointer',
              background: 'var(--surface-overlay)',
              color: 'var(--text-secondary)',
            }}
          >
            {badgeText(objectCount, notSolidCount)}
          </button>
        ))}
      </div>

      {/* Debug Log panel — opt-in via Window menu, default hidden. */}
      {showDebugLog && <LogPanel panelHeight={160} />}

      {/* Importing overlay — shown while an import blocks the main thread,
          and while a .skp open steps between paints (with its progress).
          The overlay is painted before the blocking call via a double rAF in
          importDocument; runOpenSkp settles it on every way out, so a
          thrown error or a refused blank load never leaves it stuck. */}
      {isImporting && <ImportingOverlay fileName={importingName} opening={reportOpened} progress={openProgress} />}
      {busyNote !== null && <BusyOverlay message={busyNote} />}

      {/* STL units-chooser — shown before the overlay above, since STL
          carries no unit information (see promptStlUnits). */}
      {pendingStlImport !== null && (
        <StlUnitsDialog
          fileName={pendingStlImport.name}
          onChoose={(unitScale, value) => {
            setLastStlImportUnit(value)
            setPendingStlImport(null)
            stlUnitsResolveRef.current?.(unitScale)
            stlUnitsResolveRef.current = null
          }}
          onCancel={() => {
            setPendingStlImport(null)
            stlUnitsResolveRef.current?.(null)
            stlUnitsResolveRef.current = null
          }}
        />
      )}

      {/* Unsaved-changes prompt (Lane C, docs/design/v1.1-cycle.md) —
          replaces the native ask/confirm for every discard-triggering
          gesture: File ▸ Close/New/Open/Import's confirmDiscard, and the
          Tauri window close guard below. */}
      {unsavedChangesPromptOpen && (
        <UnsavedChangesDialog
          scene={state?.scene ?? null}
          reasons={docSession.nonUndoableReasons}
          onSave={async () => {
            const ok = await saveDocumentRef.current()
            // Only resolve (closing the dialog and letting the original
            // discard-triggering action proceed) on a REAL save — a
            // cancelled or failed save (already toasted by the save flow
            // itself) leaves the dialog up so the user can retry or fall
            // back to Don't Save/Cancel.
            if (ok) resolveUnsavedChangesPrompt('save')
            return ok
          }}
          onDontSave={() => resolveUnsavedChangesPrompt('dont-save')}
          onCancel={() => resolveUnsavedChangesPrompt('cancel')}
        />
      )}

      {/* Tape Measure resize-the-model / resize-in-context confirmation
          (design tool-parity §3; group-session.md's scoped-rescale phase). */}
      {pendingRescaleConfirm !== null && (
        <RescaleConfirmDialog
          currentDistance={pendingRescaleConfirm.info.currentDistance}
          typedDistance={pendingRescaleConfirm.info.typedDistance}
          factor={pendingRescaleConfirm.info.factor}
          scope={pendingRescaleConfirm.scope}
          onConfirm={() => {
            const { scope } = pendingRescaleConfirm
            setPendingRescaleConfirm(null)
            const api = viewportApi.current
            // Belt-and-suspenders re-check (finding 5, group-session.md):
            // `handleSessionChange`'s invalidation above already cancels this
            // arm the moment the stack changes while the dialog is up, but
            // re-read the LIVE stack right here too rather than trust that
            // every session mutation is guaranteed to route through
            // `onSessionChange` before this click resolves. If the captured
            // scope's innermost node no longer matches what's actually
            // innermost now, decline instead of letting the kernel rescale
            // whatever frame it finds — the dialog would then be lying about
            // what it just applied.
            const liveStack = api?.sessionStack() ?? []
            const liveInnermost = liveStack.length > 0 ? liveStack[liveStack.length - 1] : null
            const stillMatches = scope === null
              ? liveInnermost === null
              : liveInnermost !== null && nodeEq(liveInnermost, scope.node)
            if (!stillMatches) {
              api?.cancelPendingRescale()
              return
            }
            api?.confirmPendingRescale(scope !== null, scope?.label ?? null)
          }}
          onCancel={() => {
            setPendingRescaleConfirm(null)
            viewportApi.current?.cancelPendingRescale()
          }}
        />
      )}

      {/* File ▸ Revert to Saved: one confirmation, Cancel the default. */}
      {revertPromptOpen && (
        <ConfirmDialog
          heading={t`Revert to Saved?`}
          body={t`This throws away everything you changed since the last save.`}
          confirmLabel={t`Revert`}
          danger
          cancelDefault
          onConfirm={() => {
            setRevertPromptOpen(false)
            void revertToSaved().then((r) => {
              if (!r.ok && r.shown !== true) handleToast(r.message)
            })
          }}
          onCancel={() => setRevertPromptOpen(false)}
        />
      )}

      {/* Material delete confirmation — only shown when the material is
          actually used somewhere (v1.1 assets lane); an unused one deletes
          immediately with no modal. */}
      {pendingMaterialDelete !== null && (
        <ConfirmDialog
          heading={t`Delete material?`}
          body={materialDeleteBody}
          confirmLabel={t`Delete`}
          danger
          onConfirm={() => {
            const { id, name } = pendingMaterialDelete
            setPendingMaterialDelete(null)
            doDeleteMaterial(id, name)
          }}
          onCancel={() => setPendingMaterialDelete(null)}
        />
      )}

      {/* Component definition delete confirmation — only shown when it has
          live instances; a definition with none deletes immediately. */}
      {pendingComponentDelete !== null && (
        <ConfirmDialog
          heading={t`Delete definition?`}
          body={componentDeleteBody}
          confirmLabel={t`Delete`}
          danger
          onConfirm={() => {
            const { id, name } = pendingComponentDelete
            setPendingComponentDelete(null)
            doDeleteComponent(id, name)
          }}
          onCancel={() => setPendingComponentDelete(null)}
        />
      )}

      {/* Purge Unused preview — lists what would go before committing
          (docs/design/v1.1-cycle.md Lane A). Never shown when there is
          nothing to purge — requestPurgeUnused toasts that case directly. */}
      {pendingPurgePreview !== null && (
        <ConfirmDialog
          heading={t`Purge unused?`}
          body={
            <>
              {purgeDefinitionsLine !== null && <div>{purgeDefinitionsLine}</div>}
              {purgeMaterialsLine !== null && <div>{purgeMaterialsLine}</div>}
            </>
          }
          confirmLabel={t`Purge`}
          danger
          onConfirm={confirmPurgeUnused}
          onCancel={() => setPendingPurgePreview(null)}
        />
      )}

      {/* Import report modal — shown after a successful model import */}
      {importReport !== null && (
        <ImportReportDialog
          report={importReport}
          opened={reportOpened}
          onClose={() => setImportReport(null)}
          onMakeSolid={() => {
            setImportReport(null)
            openMakeSolid('import')
          }}
        />
      )}

      {/* Save As format choice (Hew / SketchUp / SketchUp 2017) — shown
          before the native save picker, which follows the choice. */}
      {saveFormatPrompt !== null && (
        <SaveFormatDialog
          initial={saveFormatPrompt.initial}
          initialCompat={saveFormatPrompt.compat}
          onChoose={settleSaveFormat}
          onCancel={() => settleSaveFormat(null)}
        />
      )}

      {/* Non-blocking notice: the SketchUp file could not hold everything
          exactly (a `.skp` Save / Save As, shown once per document, or an
          Export). */}
      {skpNotice !== null && (
        <SkpLossNotice
          notice={skpNotice}
          onSaveHewCopy={() => { void saveHewCopy() }}
          onDismiss={() => setSkpNotice(null)}
        />
      )}

      {/* Unified Export dialog  — File ▸ Export…'s one entry
          point; the Format select decides glTF vs. STL vs. 3MF. The slicer
          formats' solid-gating confirmation below remains the follow-on
          step. */}
      {exportDialogOpen && (
        <ExportDialog
          onExport={handleExportFormat}
          initialCompat={skpCompatRef.current}
          onCancel={() => setExportDialogOpen(false)}
        />
      )}

      {/* File ▸ Print… (docs/design/printing.md): one dialog for paper and
          PDF; the shell's PrintHost hands the composed pages to the OS print
          dialog. */}
      {printDialogOpen && (
        <PrintDialog
          getViewportApi={() => viewportApi.current}
          getScene={() => sceneRef.current}
          getScenes={() => {
            // Each Scene: resolve every Scene the way activating it would —
            // camera pose, authoritative hidden leaf sets, section plane —
            // read-only (resolve_scene never mutates the document).
            const scn = sceneRef.current
            if (scn === null) return []
            const out: SceneSource[] = []
            for (const e of scenes.entries) {
              let resolved
              try {
                resolved = scn.resolve_scene(BigInt(e.sid))
              } catch {
                continue
              }
              try {
                const cj = resolved.camera_json()
                const cam = cj === undefined ? undefined : parseCameraJson(JSON.parse(cj))
                const hidden = resolved.has_hidden() ? { objects: Array.from(resolved.hidden_object_ids()), instances: Array.from(resolved.hidden_instance_ids()) } : null
                let section: SceneSource['section'] = undefined
                if (resolved.has_section()) {
                  const sj = resolved.section_json()
                  section = sj === undefined ? null : (parseSectionJson(JSON.parse(sj)) ?? null)
                }
                out.push({
                  sid: e.sid,
                  name: e.name,
                  camera: cam === undefined ? null : { projection: cam.projection, eye: cam.eye, target: cam.target, up: cam.up, fovDeg: cam.fovDeg, parallelHeight: cam.parallelHeight },
                  hidden,
                  section,
                })
              } finally {
                resolved.free()
              }
            }
            return out
          }}
          documentName={documentName(docSession)}
          sceneName={scenes.activeSid !== null && !scenes.activeDrifted ? (scenes.entries.find((e) => e.sid === scenes.activeSid)?.name ?? null) : null}
          printHost={printRecorderRef.current?.host ?? printHostRef.current}
          savePdf={(bytes, name) =>
            printRecorderRef.current?.savePdf !== undefined
              ? printRecorderRef.current.savePdf(bytes, name)
              : // usePrintController's `savePdf` contract is `Promise<boolean>` — narrower
                // than exportBinary's now-richer `Promise<string | null>` (the print flow
                // has no use for the saved location), so this is the one caller that
                // adapts back down at the boundary rather than widening its own contract.
                fileHostRef.current
                  // oxlint-disable-next-line lingui/no-unlocalized-strings -- file-format name
                  .exportBinary(bytes, name, { description: 'PDF', ext: 'pdf', mime: 'application/pdf' })
                  .then((result) => result !== null)
          }
          onPagesReady={printRecorderRef.current?.onPages}
          onClose={() => setPrintDialogOpen(false)}
        />
      )}

      {/* File ▸ Open on Phone… (docs/design/shop-mode.md §4,
          workers/share-relay/README.md) — the QR handoff dialog. Encrypts
          and uploads to the cloud dead-drop on mount, best-effort
          invalidates it on unmount (PhoneShareDialog's own module doc);
          this render gate is the only thing that ever mounts or unmounts
          it. */}
      {phoneShareOpen && (
        <PhoneShareDialog
          getDocument={getPhoneShareDocument}
          onClose={() => setPhoneShareOpen(false)}
        />
      )}

      {/* Help ▸ Report Bug… / palette / native Help menu / the kernel-error
          banner (docs/design/report-bug.md §2). The crash screen's dialog
          renders inside ErrorBoundary instead, since by the time it's needed
          this whole tree is gone. After a panic the banner can still be up
          with the tree intact: report that panic as the crash screen would
          (an out-of-memory abort leaves no capture, so it stays a plain
          report). */}
      {reportBugOpen && (
        <ReportBugDialog
          scene={sceneRef.current}
          documentName={documentName(docSession)}
          crash={kernelPanicked ? getPanicCapture() : null}
          onClose={() => setReportBugOpen(false)}
        />
      )}

      {sentReports !== null && (
        <SentReportsDialog
          initialId={sentReports.id}
          onCheckForUpdates={updaterAvailable ? handleCheckForUpdates : undefined}
          onClose={() => setSentReports(null)}
        />
      )}

      {/* Draw ▸ 3D Text… (docs/design/3d-text.md) — resolves text/font/
          height/depth; OK arms the placement tool via handleTextPlace. */}
      {textDialogOpen && (
        <TextDialog
          onPlace={handleTextPlace}
          onCancel={() => setTextDialogOpen(false)}
        />
      )}

      {/* Slicer-format solid-gating confirmation — shown before an STL/3MF
          export when any exported object is not a watertight solid; never
          silent repair (rule 4). */}
      {solidWarning !== null && (
        <StlExportDialog
          offenders={solidWarning.names}
          // oxlint-disable-next-line lingui/no-unlocalized-strings -- file-format names
          formatLabel={solidWarning.format === 'stl' ? 'STL' : '3MF'}
          onExport={() => {
            const format = solidWarning.format
            setSolidWarning(null)
            void (format === 'stl' ? doExportStl() : doExport3mf())
          }}
          onMakeSolid={() => {
            // Cancels the export and opens Make Solid on the offenders
            // (component members are objects too, so their ids work as is).
            const ids = solidWarning.ids
            exportTargetRef.current = null
            setSolidWarning(null)
            openMakeSolid('export', ids)
          }}
          onCancel={() => {
            exportTargetRef.current = null
            setSolidWarning(null)
          }}
        />
      )}

      {/* Recovery prompt — shown once at startup when autosaved snapshots
          exist and nothing else was loaded yet. Lists every crashed
          document; Recover opens each beyond the first in its own window. */}
      {recoveryPrompt !== null && recoveryPrompt.length > 0 && (
        <RecoveryDialog
          listings={recoveryPrompt}
          onRecover={handleRecover}
          onDiscard={handleDiscardRecovery}
          onDismiss={handleDismissRecovery}
        />
      )}

      {/* Welcome screen — bare launches only (the startup-handoff effect
          gates on: no pending open, no recovery, primary window, and the
          persisted "show on startup" flag). Any successful load closes it. */}
      {welcomeOpen && (
        <WelcomeScreen
          recentFiles={recentFiles}
          onClose={() => setWelcomeOpen(false)}
          onOpen={() => {
            setWelcomeOpen(false)
            openDocumentRef.current()
          }}
          onOpenRecent={(path) => {
            setWelcomeOpen(false)
            openRecent(path)
          }}
          onOpenSample={(sample) => {
            setWelcomeOpen(false)
            void openSample(sample)
          }}
          webRecents={isTauri ? undefined : webRecents}
          onOpenWebRecent={(entry) => {
            setWelcomeOpen(false)
            openWebRecent(entry)
          }}
          showOnStartup={showWelcomeSetting}
          onShowOnStartupChange={(show) => {
            setShowWelcome(show)
            setShowWelcomeSetting(show)
          }}
          unit={welcomeUnit}
          onUnitChange={(format) => {
            // Persists + broadcasts; the subscribe effect above updates
            // welcomeUnit, so no local setState needed.
            setLengthUnit(format)
            // A small-scale unit implies a small model: re-frame the (still
            // blank) scene closer so the first shape lands at a sensible
            // size. Guarded on emptiness — choosing a unit must never move
            // the camera over real work.
            const scene = sceneRef.current
            if (scene !== null && isSceneEmpty(scene)) {
              viewportApi.current?.setHomeFraming(homeFramingScale(format))
            }
          }}
        />
      )}

      {/* Command palette (⌘/ / Ctrl+/, one key everywhere). Selecting a
          result reuses the exact same dispatch the native menu and every
          menu click already go through. */}
      <CommandPalette
        open={paletteOpen}
        initialQuery={paletteSeed}
        onClose={() => setPaletteOpen(false)}
        onRun={(id) => menuActionRef.current(id)}
        extraEntries={paletteExtraEntries}
        gates={{
          selection: selectedIds.length > 0 || selectedGuide !== null || selectedAnnotation !== null,
          canGroup: menuGates?.canGroup ?? false,
          canUngroup: menuGates?.canUngroup ?? false,
          canMakeComponent: menuGates?.canMakeComponent ?? false,
          canPlaceCopy: menuGates?.canPlaceCopy ?? false,
          canExplode: menuGates?.canExplode ?? false,
          canMakeUnique: menuGates?.canMakeUnique ?? false,
          canBoolean: menuGates?.canBoolean ?? false,
          canSaveSelectionToLibrary: menuGates?.canSaveSelectionToLibrary ?? false,
          canImport: !componentFrameOpen,
          canRevertToSaved: revertable,
          canDrawText: !componentFrameOpen,
          sceneActive: scenes.activeSid !== null,
          scenesAny: scenes.entries.length > 0,
          hasStructuralSelection: menuGates?.hasStructuralSelection ?? false,
          clipboardHasContent: menuGates?.clipboardHasContent ?? false,
        }}
      />

      {/* Settings modal — web-only fallback (Tauri opens a real, separate OS
          window instead; see openSettings in App.tsx). */}
      {showSettingsModal && (
        <div
          onClick={() => setShowSettingsModal(false)}
          style={{
            position: 'fixed',
            inset: 0,
            background: 'var(--backdrop-dim)',
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            zIndex: 300,
          }}
        >
          <div
            role="dialog"
            aria-modal="true"
            aria-label={t`Settings`}
            onClick={(e) => e.stopPropagation()}
            style={{
              position: 'relative',
              width: '520px',
              height: '380px',
              maxHeight: '80vh',
              overflow: 'hidden',
              background: 'var(--surface-window, #1a1a1a)',
              color: 'var(--text-secondary, #ddd)',
              border: '1px solid var(--border-hairline, #333)',
              borderRadius: '10px',
              boxShadow: 'var(--shadow-palette, 0 8px 32px rgba(0,0,0,0.6))',
            }}
          >
            {/* Same tabbed Settings UI the desktop's dedicated window shows. */}
            <SettingsWindow />
            <button
              aria-label={t`Close settings`}
              onClick={() => setShowSettingsModal(false)}
              style={{
                position: 'absolute',
                top: '8px',
                right: '10px',
                background: 'none',
                border: 'none',
                color: 'var(--text-faint, #888)',
                cursor: 'pointer',
                fontSize: '16px',
                lineHeight: 1,
              }}
            >
              ×
            </button>
          </div>
        </div>
      )}

      {/* Windows desktop settings surface — a full-window page in the
          Windows 11 app-settings idiom (back arrow returns to the document;
          see openSettings). Rendered last so it overlays every panel. */}
      {showFluentSettings && (
        <FluentSettingsPage onBack={() => setShowFluentSettings(false)} />
      )}

      {/* Library browser (Window ▸ Library, ⇧⌘L / bare ⇧L, the tool rail's
          Library row, and the command palette's "Library" entry all open
          this). The dialog only reports intent — every action that touches
          the live document or the storage seam is wired here (see the
          onLibraryInsert / onLibraryOpenAsDocument / onLibraryPaintWith /
          onLibraryAddToPalette / handleSaveToLibraryCommit callbacks above). */}
      <LibraryDialog
        open={showLibrary}
        onClose={() => setShowLibrary(false)}
        placements={libraryPlacements}
        onInsert={onLibraryInsert}
        onOpenAsDocument={onLibraryOpenAsDocument}
        onPaintWith={onLibraryPaintWith}
        onAddToPalette={onLibraryAddToPalette}
        materialInPalette={libraryMaterialInPalette}
      />

      {/* Save to Library name prompt (frame 1d) — the ContextualDock's
          save-to-library verb (dock-anchored) and File ▸ Save to Library…
          (centered, whole document) share this one popover. */}
      {saveToLibraryTarget !== null && (
        <SaveToLibraryPopover
          defaultName={saveToLibraryTarget.defaultName}
          variant={saveToLibraryTarget.kind === 'document' ? 'modal' : 'dock'}
          onSave={handleSaveToLibraryCommit}
          onClose={() => setSaveToLibraryTarget(null)}
        />
      )}
    </main>
  )
}

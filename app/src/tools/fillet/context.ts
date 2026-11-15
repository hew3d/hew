/* oxlint-disable lingui/no-unlocalized-strings -- True Curves strings are not yet wrapped for translation: this file is English-only until they are */
/** The Fillet tool's shared state and primitives, handed to every stage. */

import * as THREE from 'three'
import type { Scene as WasmScene } from '../../wasm/loader'
import type { Snap } from '../types'
import { type Ray } from '../../viewport/math'
import { RetypeWindow } from '../retypeWindow'
import { type CurveToken } from '../curveInput'
import { setFilletMeasurementLabelOverride, type FilletProfile } from '../filletSettings'
import { FilletGrips } from '../filletGrips'
import { FilletLabels } from '../filletLabels'
import { type Hover, type PickOptions } from '../filletPick'
import type { Pick, Stage, RetypeSpec, FilletHooks, FilletOptions } from './types'
import { MIN_COMMIT_MOVE_PX } from './constants'
import { SyncBlendPreviewer, type BlendPreviewer } from './previewer'
import type { PickStage } from './pick'
import type { SizeStage } from './size'
import type { PreviewStage } from './preview'
import type { CommitStage } from './commit'
import type { EditStageOps } from './edit'

export class FilletContext {
  stage: Stage = { kind: 'idle' }

  hover: Hover = { kind: 'none' }

  collected: Pick[] = []
  statusLine: string | null = null

  /** A one-line confirmation ("Facets set to 8.") shown until the next
   *  pointer move or key. `noteRay` is the pointer ray it was set under: the
   *  Viewport re-sends that same ray after a key (its snap re-lock), and
   *  that is not a move. */
  note: string | null = null
  noteRay: Ray | null = null

  /** The blend under the pointer (`object:id`) and whether it can still be
   *  edited; read when the hover lands on it, dropped when history moves. */
  hoverBlendKey: string | null = null
  hoverBlendIntact = true

  lastRay: Ray | null = null
  lastSnap: Snap | null = null

  screen: [number, number] | null = null

  /** The size the armed preview was held to by a limit, in world units, or
   *  null when it drew the size asked for. */
  clampedHeadline: number | null = null

  worldPerPixel: ((dist: number) => number) | null = null

  lastToken: CurveToken | null = null

  /** Where the pointer was when a click finished a pick or an edit, so the
   *  double-click that may follow is swallowed instead of reaching the app. */
  finishedAt: [number, number] | null = null

  /** Whether the last committed round had stops or radius points. */
  lastVaried = false
  lastLabel = 'Radius'

  pendingMove: { ray: Ray; snap: Snap | null } | null = null

  maskedObjects = new Set<bigint>()

  unsubSettings: (() => void) | null = null

  /** The profile a drawn corner displaced (Cove, Bullnose, a shape), and
   *  the Round put in its place: the displaced one comes back once the tool
   *  leaves the corner, unless a profile was picked there (any pick, Alt
   *  included, publishes a new profile object, so `forcedRound` is no longer
   *  the current one). */
  displacedProfile: FilletProfile | null = null
  forcedRound: FilletProfile | null = null

  readonly root = new THREE.Group()
  readonly gripsView = new FilletGrips()

  readonly labels = new FilletLabels()
  readonly retype: RetypeWindow<RetypeSpec>

  readonly schedule: (cb: () => void) => void

  readonly now: () => number

  /** Objects whose previews are too slow to follow the pointer. */
  readonly slowObjects = new Set<bigint>()

  /** This armed preview skipped a slow object (its hint shows when no
   *  other target has something to say). */
  slowSkipped = false

  /** The retype window's (or the collected arming's) typed buffer, idle. */
  retypeTyped = ''
  ctrlHeld = false

  readonly previewGroup: THREE.Group
  readonly wasmScene: WasmScene
  readonly hooks: FilletHooks
  /** The only path to the kernel's preview answers. */
  readonly previewer: BlendPreviewer
  /** Bumped whenever a preview round starts or is dropped: an answer that
   *  arrives under an older epoch is stale and is not applied. */
  epoch = 0
  /** Re-runs a pointer press through the tool (a click that finished one
   *  pick may also start the next). Set by the tool. */
  dispatchPointerDown: (snap: Snap | null, ray: Ray) => void = () => {}

  // Stages, wired by the tool.
  pick!: PickStage
  size!: SizeStage
  preview!: PreviewStage
  commit!: CommitStage
  edit!: EditStageOps

  constructor(wasmScene: WasmScene, previewGroup: THREE.Group, hooks: FilletHooks, opts: FilletOptions = {}) {
    this.wasmScene = wasmScene
    this.previewGroup = previewGroup
    this.hooks = hooks
    this.retype = new RetypeWindow(wasmScene)
    this.schedule = opts.schedule ?? ((cb) => { requestAnimationFrame(() => cb()) })
    this.now = opts.now ?? (() => performance.now())
    this.previewer = opts.previewer ?? new SyncBlendPreviewer(wasmScene, this.now)
    this.root.name = 'FilletPreview'
    this.previewGroup.add(this.root)
    this.root.add(this.gripsView.group)
    this.root.add(this.labels.group)
  }

  /** Every readout update also publishes the label that goes with it, so the
   *  measurement box names a held grip's value ("Stop", "Position"). */
  measure(text: string): void {
    setFilletMeasurementLabelOverride(this.heldGripLabel())
    this.hooks.onMeasurement(text)
  }

  heldGripLabel(): string | null {
    const s = this.stage
    if (s.kind !== 'armed' || s.active.kind === 'radius') return null
    switch (s.active.kind) {
      case 'stop':
        return 'Stop'
      case 'handlePos':
        return 'Position'
      case 'endRadius':
      case 'handleRadius':
        return 'Radius'
    }
  }

  pickOptions(): PickOptions {
    return { noPropagate: this.ctrlHeld, worldPerPixel: this.worldPerPixel }
  }

  movedEnough(armedAt: [number, number] | null, minPx = MIN_COMMIT_MOVE_PX): boolean {
    if (armedAt === null || this.screen === null) return false
    return Math.hypot(this.screen[0] - armedAt[0], this.screen[1] - armedAt[1]) >= minPx
  }
}

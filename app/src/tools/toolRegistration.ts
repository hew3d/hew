/**
 * toolRegistration — how a viewport tool plugs into the Viewport.
 *
 * Every tool the viewport can construct contributes ONE entry (see
 * `tools/registrations/`, collected by `registrations/index.ts`). The
 * Viewport builds a tool by looking its `name` up in that list and calling
 * `create` with a `ToolContext`, and the Viewport's tool switch and input
 * handlers dispatch through the optional hooks below, so adding, changing, or
 * hiding a tool touches its own registration file instead of Viewport.tsx.
 *
 * The context is an explicit interface (not the Viewport instance): a
 * registration can only reach what is listed here, which keeps the tools
 * testable and the Viewport free to change its internals.
 */

import type * as THREE from 'three'
import type { Scene as WasmScene } from '../wasm/loader'
import type { NodeRef } from '../panels/treeModel'
import type { RefreshTouched } from '../viewport/SceneRenderer'
import type { Ray } from '../viewport/math'
import type { TouchedIds } from './offsetShared'
import type { Snap, Tool } from './types'
import type { ToolName } from './toolIcons'

/** What a tool factory may use. Every member reads live viewport state at
 *  call time (selection, editing context), so a factory can run any time. */
export interface ToolContext {
  /** The kernel scene the tool edits. */
  readonly scene: WasmScene
  /** Transient preview geometry (cleared by the tool itself). */
  readonly previewGroup: THREE.Group
  /** The committed object meshes (tools that ghost or pick them). */
  readonly objectsGroup: THREE.Group
  /** The live mesh group of a placed instance, or null. */
  getInstanceGroup(instanceId: bigint): THREE.Group | null

  /** A fresh copy of the current selection. */
  selection(): NodeRef[]
  /** Replace the selection with one node (a tool selecting its result). */
  select(node: NodeRef): void

  /** Re-tessellate after a commit: only `touched` when given, else all. */
  refreshScene(touched?: RefreshTouched): void
  /** Rebuild after a commit that touched objects, instances, or components,
   *  plus the sketch and guide buffers. */
  refreshTouched(touched: TouchedIds): void
  /** Rebuild the sketch line and fill buffers. */
  refreshSketches(): void
  /** Rebuild the construction-guide lines. */
  refreshGuides(): void
  /** Tell the app the document changed (dirty flag, panels). */
  documentChanged(): void
  /** Ask for a frame (a preview that moves outside any pointer event). */
  scheduleRender(): void
  /** Re-poll the status-bar hint for the active tool. */
  reportToolHint(): void

  /** Show a toast; `code` is a kernel error code for log capture. */
  toast(message: string, code?: string): void
  /** Show the typed-value readout. */
  measurement(text: string): void
  /** Run a long edit behind the busy card; false when it was refused. */
  busy(message: string, work: () => void): boolean | void

  /** Scope a tool to the current editing context (a no-op for a tool with
   *  no `setEditContext`). */
  scopeToEditContext(tool: Tool): void
  /** Whether a face of this object/placement is directly drawable in the
   *  current editing context. */
  faceEligibility(objectId: bigint, instanceId: bigint | undefined): boolean

  /** Hit-test the node a click would select (editing-context aware). */
  resolveSelectable(snap: Snap | null, ray: Ray): NodeRef | null
  /** Delete nodes the way Edit ▸ Delete does (selection, undo step). */
  deleteNodes(nodes: NodeRef[]): void
  /** Select what is under the cursor so a gesture can start on it. */
  acquireTransformTargets(ray: Ray): NodeRef[] | null

  /** Set the canvas cursor to the tool's icon, with an optional `+` badge. */
  setToolCursor(name: ToolName, badge?: boolean): void
  /** Whether `tool` is the tool receiving input right now. */
  isActiveTool(tool: Tool): boolean

  /** Polygon side count, remembered across re-selection for the session. */
  readonly polygonSides: { get(): number; set(sides: number): void }
  /** Draw only some faces of an object (a fillet preview hides the faces it
   *  replaces). `set` returns whether the mask was applied. */
  readonly faceMask: {
    set(objectId: bigint, faceIds: readonly bigint[], maxTriangles: number): boolean
    clear(objectId: bigint): void
  }
}

/** The phase of a pointer event a registration's `onPointer` hook sees. */
export type ToolPointerPhase = 'move' | 'down' | 'up'

export interface ToolRegistration<T extends Tool = Tool> {
  /** The registry tool this entry builds (a `TOOL_REGISTRY` name). */
  name: ToolName
  /** Build a ready-to-use tool: construct it, wire its callbacks, and scope
   *  it to the editing context. A new instance per activation. */
  create(ctx: ToolContext): T
  /** After the tool became the active one (arm the selection, seed state). */
  onActivate?(tool: T, ctx: ToolContext): void
  /** The instance is being replaced or the viewport is shutting down:
   *  release subscriptions it holds. */
  dispose?(tool: T): void
  /** Live modifier reads off the real pointer event (Ctrl for crease, Alt
   *  for a rail): called for the active tool before it sees the event. */
  onPointer?(tool: T, ev: MouseEvent, phase: ToolPointerPhase): void
  /** Window key events while the tool is active (modifier taps and holds). */
  onKeyDown?(tool: T, ev: KeyboardEvent, ctx: ToolContext): void
  onKeyUp?(tool: T, ev: KeyboardEvent, ctx: ToolContext): void
  /** The window lost focus: a swallowed key-up must not leave a hold stuck. */
  onWindowBlur?(tool: T): void
  /** Cmd/Ctrl+A with no text field focused. Return true when the tool took
   *  it (the viewport then re-polls the hint and redraws). */
  onSelectAllChord?(tool: T): boolean
}

/** Type-erase a registration (the table holds many tool types). */
export function defineTool<T extends Tool>(reg: ToolRegistration<T>): ToolRegistration {
  return reg as unknown as ToolRegistration
}

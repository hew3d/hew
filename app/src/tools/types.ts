/**
 * Tool interface and supporting types for the Hew interaction layer.
 * See docs/DEVELOPMENT.md for the design.
 */

import type { Ray } from '../viewport/math'
import type { NodeRef } from '../panels/treeModel'

/** Snap result shape — mirrors SnapJs from wasm-api, but pure TypeScript */
export interface Snap {
  x: number
  y: number
  z: number
  kind: string
  direction?: [number, number, number]
  object?: bigint
  element?: bigint
  elementKind?: string
  /** Owning sketch handle when the snap derives from a committed sketch
   *  edge (`elementKind === 'sketch-edge'`; `element` is the edge). */
  sketch?: bigint
}

/**
 * One active tool at a time. ToolController owns the routing; each method
 * is called with the resolved snap result (or null when snap is unavailable
 * and fallback was also null).
 */
export interface Tool {
  /** Called on every pointer move — update preview / cues */
  onPointerMove(snap: Snap | null, ray: Ray): void
  /** Called on pointer down — advance the gesture */
  onPointerDown(snap: Snap | null, ray: Ray): void
  /** Called on key events — esc cancels current stage */
  onKey(ev: KeyboardEvent): void
  /** Full reset — clear all preview / ephemeral state */
  cancel(): void
  /** Human-readable name shown in the status bar */
  readonly name: string

  /**
   * (optional) Return snap constraints the tool wants injected into the next
   * snapService.resolve() call.  Viewport feature-detects with
   * `'snapConstraint' in tool`.
   *
   * - `anchor` + `lockAxis`: axis-lock for distance tools (e.g. MoveTool)
   * - `constraintPlane`: restrict candidates to a plane (e.g. RectangleTool
   *   in face mode, to avoid snapping to occluded off-plane geometry)
   *
   * The optional `ray` argument is the current pointer ray; tools that need to
   * pick the hovered face (e.g. RectangleTool idle) can use it.  Tools that
   * don't need it may omit the parameter.
   */
  snapConstraint?(ray?: Ray): {
    anchor?: [number, number, number]
    lockAxis?: 0 | 1 | 2
    constraintPlane?: { point: [number, number, number]; normal: [number, number, number] }
  } | null

  /**
   * (optional) When true the tool is capturing raw keyboard input (e.g. VCB
   * numeric entry) and the Viewport should route key events to it BEFORE any
   * tool-switch shortcuts.  Viewport feature-detects with
   * `'capturingInput' in tool`.
   */
  capturingInput?(): boolean

  /**
   * (optional) Per-key refinement of `capturingInput`: whether THIS key
   * belongs to the tool's input capture. Lets a tool capture only the keys
   * its buffer actually needs (e.g. Move's armed ×N / /N window takes
   * digits and mode tokens but must never eat Space, which always resets
   * to Select) while a full VCB gesture still captures the whole keyboard.
   * Consulted by the Viewport's key routing and by App-level shortcut
   * gates via `ViewportApi.isCapturingInput(key)`; when absent, the plain
   * `capturingInput()` verdict applies to every key. Feature-detected with
   * `'capturesKey' in tool`.
   */
  capturesKey?(key: string): boolean

  /**
   * (optional) Called on a double-click BEFORE the Viewport's default
   * "enter context" gesture. Return `true` if the tool consumed the
   * double-click (e.g. LineTool ending a chain) so the Viewport skips
   * entering a group/instance/object; return `false`/omit to fall through to
   * the default behavior. Viewport feature-detects with
   * `'onDoubleClick' in tool`.
   */
  onDoubleClick?(snap: Snap | null, ray: Ray): boolean

  /**
   * (optional) A new/loaded document has replaced the Scene. Tools that cache
   * kernel handles across gestures (e.g. a ground-sketch handle) must drop
   * them here — reusing a handle from the previous document throws
   * UnknownSketch. The Viewport calls this on the active tool from its
   * `notifyLoaded`. Feature-detected with `'onDocumentReset' in tool`.
   */
  onDocumentReset?(): void

  /**
   * (optional) The app selection changed while this tool is active. Tools
   * that snapshot the selection at creation (Move/Rotate/Scale) implement
   * this so the NEXT gesture starts from live handles — without it, an
   * undo that killed selected nodes left the tool committing against dead
   * handles (UnknownObject). The Viewport pushes every selection change
   * (clicks, Outliner, undo/redo pruning) into the active tool.
   * Feature-detected with `'setSelection' in tool`.
   */
  setSelection?(nodes: NodeRef[]): void

  /**
   * (optional) One live "what do I do next" line for the status bar,
   * reflecting the tool's CURRENT stage — "Click the opposite corner", not
   * a static tool description. The Viewport re-polls after every routed
   * event (move/down/key/double-click/cancel/switch) and pushes changes up;
   * tools without it fall back to the palette's static description.
   * Feature-detected with `'statusHint' in tool`.
   */
  statusHint?(): string
}

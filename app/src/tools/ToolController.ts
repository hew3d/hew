/**
 * ToolController — owns pointer event routing and esc handling.
 *
 * One active tool at a time. Camera controls (middle-drag orbit, right-drag
 * pan, wheel dolly) are NOT a tool — they're always live and handled by
 * OrbitControls separately.
 */

import type { Tool } from './types'
import { SelectTool } from './SelectTool'
import type { OnSelect } from './SelectTool'
import type { Scene as WasmScene } from '../wasm/loader'

/** Callback invoked when the active tool name changes */
export type ToolChangeListener = (toolName: string) => void

export class ToolController {
  private _activeTool: Tool
  private _listeners: ToolChangeListener[] = []
  private _wasmScene: WasmScene
  private _onSelect: OnSelect

  constructor(wasmScene: WasmScene, onSelect: OnSelect) {
    this._wasmScene = wasmScene
    this._onSelect = onSelect
    this._activeTool = new SelectTool(wasmScene, onSelect)
  }


  get activeTool(): Tool {
    return this._activeTool
  }

  get activeToolName(): string {
    return this._activeTool.name
  }

  setTool(tool: Tool): void {
    this._activeTool.cancel()
    this._activeTool = tool
    this._notifyListeners()
  }

  resetToSelect(): void {
    this.setTool(new SelectTool(this._wasmScene, this._onSelect))
  }

  onToolChange(listener: ToolChangeListener): () => void {
    this._listeners.push(listener)
    return () => {
      this._listeners = this._listeners.filter((l) => l !== listener)
    }
  }

  private _notifyListeners(): void {
    for (const l of this._listeners) {
      l(this._activeTool.name)
    }
  }
}

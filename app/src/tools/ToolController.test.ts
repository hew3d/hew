import { describe, it, expect, vi } from 'vitest'
import { ToolController } from './ToolController'
import type { Tool } from './types'

/** A minimal Tool stub recording lifecycle calls, with an optional activate(). */
function makeStubTool(name: string, calls: string[], withActivate: boolean): Tool {
  const tool: Tool & { activate?: () => void } = {
    name,
    onPointerMove: () => { /* no-op */ },
    onPointerDown: () => { /* no-op */ },
    onKey: () => { /* no-op */ },
    cancel: () => calls.push(`${name}.cancel`),
  }
  if (withActivate) tool.activate = () => calls.push(`${name}.activate`)
  return tool
}

describe('ToolController.setTool activation ordering', () => {
  const wasmScene = {} as never
  const onSelect = vi.fn()

  it('calls the incoming tool activate() AFTER the outgoing tool cancel()', () => {
    // This ordering is load-bearing for the Scale gizmo: the outgoing tool's
    // cancel() clears the SHARED preview group, so the incoming tool must draw
    // its at-rest overlay only afterwards — otherwise the outgoing cancel wipes
    // it (the invisible-gizmo bug).
    const calls: string[] = []
    const controller = new ToolController(wasmScene, onSelect)
    const outgoing = makeStubTool('Outgoing', calls, false)
    const incoming = makeStubTool('Incoming', calls, true)

    controller.setTool(outgoing) // SelectTool.cancel() runs (no record)
    calls.length = 0 // ignore the initial swap
    controller.setTool(incoming)

    expect(calls).toEqual(['Outgoing.cancel', 'Incoming.activate'])
    expect(controller.activeTool).toBe(incoming)
  })

  it('tools without activate() switch normally (feature-detected)', () => {
    const calls: string[] = []
    const controller = new ToolController(wasmScene, onSelect)
    const a = makeStubTool('A', calls, false)
    const b = makeStubTool('B', calls, false)
    controller.setTool(a)
    calls.length = 0
    expect(() => controller.setTool(b)).not.toThrow()
    expect(calls).toEqual(['A.cancel']) // no activate; just the outgoing cancel
  })
})

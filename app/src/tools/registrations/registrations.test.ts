import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { describe, expect, it } from 'vitest'
import { TOOL_REGISTRY, type ToolName } from '../toolRegistry'
import { TOOL_ACTIONS, toolActionFor } from '../toolActions'
import { TOOL_REGISTRATIONS, toolRegistration } from './index'
import { DRAW_MENU, TOOLS_MENU, menuEntryActionId, pruneMenuEntries, type MenuEntry } from '../../panels/toolsMenuModel'

/* oxlint-disable lingui/no-unlocalized-strings -- tool IDs and test fixtures, never shown */
/**
 * Registry tools the Viewport still builds itself, and why. Each is tied to
 * Viewport-internal machinery that a `ToolContext` would have to mirror
 * wholesale, so moving it would risk behavior for no gain yet.
 */
const NOT_REGISTERED: Record<string, string> = {
  Select: 'owned by ToolController (resetToSelect); the default tool',
  Paint: 'material, eyedropper cursor, instance-paint refresh and session-stack hooks',
  'Position Texture': 'live uv preview straight into the scene renderer',
  Move: 'shared with the select-drag handoff (beginDragMove), copy-mode cursor badge, guide targets',
  Rotate: 'copy-mode cursor badge, guide targets, transform-target acquirer',
  Scale: 'transform-target acquirer and release',
  Flip: 'copy-mode cursor badge, transform-target acquirer',
  'Tape Measure': 'rescale confirmation, loupe and session scope',
  Dimension: 'annotation layer and editor',
  Text: 'annotation layer and editor',
  Protractor: 'guide layer and measurement chrome',
  Slice: 'section and cut chrome',
  'Section Plane': 'section manager and clipping state',
  'Edit Vertex': 'vertex handles on the scene renderer',
  'Drawing Axes': 'drawing-axes frame state',
  Orbit: 'camera tool: OrbitControls mouse-button remap, no Tool instance',
  Pan: 'camera tool: OrbitControls mouse-button remap, no Tool instance',
  Zoom: 'camera tool: OrbitControls mouse-button remap, no Tool instance',
  'Zoom Window': 'camera tool: owns left-drag in the Viewport',
  'Position Camera': 'camera tool: disables OrbitControls, drives the rig',
  'Look Around': 'camera tool: disables OrbitControls, drives the rig',
  Walk: 'camera tool: disables OrbitControls, drives the rig',
}

const read = (rel: string): string => readFileSync(fileURLToPath(new URL(rel, import.meta.url)), 'utf8')

describe('viewport tool registrations', () => {
  const registryNames = new Set<string>(TOOL_REGISTRY.map((t) => t.name))
  const registered = new Set<string>(TOOL_REGISTRATIONS.map((r) => r.name))

  it('registers each tool once', () => {
    expect(registered.size).toBe(TOOL_REGISTRATIONS.length)
  })

  it('every registered tool has a TOOL_REGISTRY spec', () => {
    expect([...registered].filter((n) => !registryNames.has(n))).toEqual([])
  })

  it('every registry tool is registered or listed with a reason in NOT_REGISTERED', () => {
    const missing = [...registryNames].filter((n) => !registered.has(n) && NOT_REGISTERED[n] === undefined)
    expect(missing).toEqual([])
  })

  it('NOT_REGISTERED names real registry tools that are not registered', () => {
    for (const name of Object.keys(NOT_REGISTERED)) {
      expect(registryNames.has(name), `${name} is not a registry tool`).toBe(true)
      expect(registered.has(name), `${name} is registered; drop it from NOT_REGISTERED`).toBe(false)
    }
  })

  it('the Viewport switch still builds every unregistered named tool itself', () => {
    const viewport = read('../../viewport/Viewport.tsx')
    const camera = new Set(['Select', 'Orbit', 'Pan', 'Zoom', 'Zoom Window'])
    const unbuilt = Object.keys(NOT_REGISTERED).filter(
      (n) => !camera.has(n) && !viewport.includes(`case '${n}':`),
    )
    expect(unbuilt).toEqual([])
  })

  it('the Viewport switch has no case for a registered tool (the table builds it)', () => {
    const viewport = read('../../viewport/Viewport.tsx')
    expect([...registered].filter((n) => viewport.includes(`case '${n}':`))).toEqual([])
  })

  it('toolRegistration looks up by name', () => {
    expect(toolRegistration('Fillet')?.name).toBe('Fillet')
    expect(toolRegistration('Walk')).toBeUndefined()
  })
})

describe('tool command table', () => {
  it('has a command for every registry tool', () => {
    for (const spec of TOOL_REGISTRY) {
      expect(toolActionFor(spec.actionId)?.tool, spec.actionId).toBe(spec.name)
    }
  })

  it('command ids are unique and open registry tools', () => {
    const ids = TOOL_ACTIONS.map((a) => a.id)
    expect(new Set(ids).size).toBe(ids.length)
    const names = new Set<ToolName>(TOOL_REGISTRY.map((t) => t.name))
    for (const a of TOOL_ACTIONS) expect(names.has(a.tool), a.id).toBe(true)
  })

  it("App.tsx's dispatch has a case label for each tool command and none extra", () => {
    const app = read('../../App.tsx')
    const cases = new Set([...app.matchAll(/^\s*case '(tool-[a-z-]+)':/gm)].map((m) => m[1]))
    expect([...cases].sort()).toEqual(TOOL_ACTIONS.map((a) => a.id).sort())
  })
})

describe('Tools and Draw menu models', () => {
  const walk = (entries: readonly MenuEntry[]): MenuEntry[] =>
    entries.flatMap((e) => (e.kind === 'submenu' ? [e, ...walk(e.items)] : [e]))

  it('every menu entry opens a real tool command', () => {
    for (const entry of [...walk(TOOLS_MENU), ...walk(DRAW_MENU)]) {
      const id = menuEntryActionId(entry)
      if (id !== undefined) expect(toolActionFor(id), id).toBeDefined()
    }
  })

  it('pruning nothing changes nothing', () => {
    expect(pruneMenuEntries(TOOLS_MENU, () => true)).toEqual(TOOLS_MENU)
    expect(pruneMenuEntries(DRAW_MENU, () => true)).toEqual(DRAW_MENU)
  })

  it('pruning a tool drops it, empty submenus, and orphaned separators', () => {
    const shape = new Set(['tool-revolve', 'tool-loft', 'tool-shell', 'tool-project', 'tool-path-array'])
    const pruned = pruneMenuEntries(TOOLS_MENU, (id) => !shape.has(id))
    expect(walk(pruned).some((e) => e.kind === 'submenu' && e.id === 'shape')).toBe(false)
    const kinds = pruned.map((e) => e.kind)
    expect(kinds).not.toContain(undefined)
    for (let i = 1; i < kinds.length; i++) {
      expect(kinds[i] === 'separator' && kinds[i - 1] === 'separator').toBe(false)
    }
    expect(kinds[0]).not.toBe('separator')
    expect(kinds[kinds.length - 1]).not.toBe('separator')
  })
})

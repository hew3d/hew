/**
 * The Preview wall at the components that show or run a Preview feature:
 * rendered as a production build sees them (the build flag mocked off, a fresh
 * module graph per case) and, for the marks, as a preview build does. The
 * registry-wide list checks are in wall.test.ts.
 */

import { fireEvent, render, screen, cleanup } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import * as THREE from 'three'
import { readFileSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import type { PreviewFeature } from './registry'

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '../../..')
const FEATURES = (JSON.parse(readFileSync(join(ROOT, 'preview-features.json'), 'utf8')) as { features: PreviewFeature[] }).features
const gatedTools = FEATURES.flatMap((f) => f.tools)

async function fresh(build: boolean) {
  vi.resetModules()
  vi.doMock('./build', () => ({ previewBuild: build }))
}

afterEach(() => {
  cleanup()
  vi.doUnmock('./build')
  vi.resetModules()
})

const menuProps = {
  name: 'Untitled', saveState: '', nativeMenuBar: false,
  onNew: vi.fn(), onOpen: vi.fn(), onSave: vi.fn(), onSaveAs: vi.fn(), onImport: vi.fn(), onExport: vi.fn(),
  onPrint: vi.fn(), onUndo: vi.fn(), onRedo: vi.fn(), canUndo: false, canRedo: false,
}

describe('production build', () => {
  it('MenuBar: no Preview mark, no withheld tool in Tools, no Smooth or Soften in Object', async () => {
    await fresh(false)
    const { MenuBar } = await import('../panels/MenuBar')
    render(<MenuBar {...menuProps} editGates={{ canSmooth: true, canSoften: true, canBoolean: true } as never} />)
    expect(screen.queryByTestId('preview-mark')).toBeNull()
    fireEvent.click(screen.getByRole('button', { name: /^tools$/i }))
    for (const tool of gatedTools) expect(screen.queryByText(tool), tool).toBeNull()
    expect(screen.queryByText('Shape')).toBeNull()
    expect(screen.queryByText('Deform')).toBeNull()
    expect(screen.getByText('Push/Pull')).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: /^object$/i }))
    expect(screen.queryByText('Smooth')).toBeNull()
    expect(screen.queryByText('Soften Edges…')).toBeNull()
    expect(screen.getByText('Union')).toBeInTheDocument()
  })

  it('ToolRail: no withheld tool, and the Library row still there', async () => {
    await fresh(false)
    const { ToolRail } = await import('../panels/ToolRail')
    render(<ToolRail activeTool="Select" onSelectTool={vi.fn()} onOpenPalette={vi.fn()} onOpenLibrary={vi.fn()} narrow={false} onToggleNarrow={vi.fn()} />)
    const names = screen.getAllByRole('radio').map((r) => r.getAttribute('aria-label'))
    for (const tool of gatedTools) expect(names, tool).not.toContain(tool)
    expect(names).toContain('Push/Pull')
    expect(screen.getByRole('button', { name: 'Library' })).toBeInTheDocument()
  })

  it('Settings: no Preview tab', async () => {
    await fresh(false)
    const { SettingsWindow } = await import('../settings/SettingsWindow')
    render(<SettingsWindow />)
    expect(screen.queryByRole('tab', { name: 'Preview' })).toBeNull()
    expect(screen.getAllByRole('tab')).toHaveLength(9)
  })

  it('SmoothSection: nothing for an object with no level; only Off for one that has a level', async () => {
    await fresh(false)
    const { SmoothSection } = await import('../panels/SmoothSection')
    const levels: Record<string, number> = {}
    const scene = {
      smooth_level: (id: bigint) => levels[String(id)] ?? 0,
      smooth_surface_available: () => true,
      set_smooth_level: vi.fn((id: bigint, level: number) => { levels[String(id)] = level; return { free: vi.fn() } }),
      bake: vi.fn(() => ({ free: vi.fn() })),
      soften_by_angle: vi.fn(() => 0),
      history_bracket_open: vi.fn(), history_bracket_commit: vi.fn(), history_bracket_abort: vi.fn(),
    }
    const props = { scene: scene as never, docRev: 0, onGeometryChanged: vi.fn(), onToast: vi.fn() }
    const a = render(<SmoothSection {...props} objects={[1n]} />)
    expect(a.container.querySelector('[data-smooth-section]')).toBeNull()
    expect(a.container.querySelector('[data-soften-section]')).toBeNull()
    a.unmount()
    levels['1'] = 2
    const b = render(<SmoothSection {...props} objects={[1n]} />)
    const select = screen.getByRole('combobox', { name: 'Smooth' }) as HTMLSelectElement
    const choices = [...select.options].filter((o) => !o.disabled).map((o) => o.textContent)
    expect(choices).toEqual(['Off'])
    expect(b.container.querySelector('[data-smooth-bake]')).toBeNull()
    expect(b.container.querySelector('[data-soften-section]')).toBeNull()
    // Off works; a stale level request does nothing.
    fireEvent.change(select, { target: { value: '3' } })
    expect(scene.set_smooth_level).not.toHaveBeenCalled()
    fireEvent.change(select, { target: { value: '0' } })
    expect(scene.set_smooth_level).toHaveBeenCalledWith(1n, 0)
  })

  it('Follow Me: a typed twist or end size does nothing and is not offered', async () => {
    await fresh(false)
    const { FollowMeTool } = await import('../tools/FollowMeTool')
    const free = vi.fn()
    let n = 10n
    const scene = {
      pick_sketch_edge: vi.fn(() => ({ sketch: () => 9n, edge: () => 1n, free })),
      pick_face: vi.fn(() => undefined),
      pick_sketch_region: vi.fn(() => ({ sketch: () => 20n, region: () => 21n, free })),
      pick_sketch_region_in_instance: vi.fn(),
      sketch_edge_island: vi.fn(() => 5n),
      sketch_island_edges: vi.fn(() => new BigUint64Array([1n])),
      sketch_curve_edges: vi.fn(() => new BigUint64Array([])),
      sketch_edge_endpoints: vi.fn(() => new Float64Array([0, 0, 0, 1, 0, 0])),
      face_boundary: vi.fn(() => new Float32Array([])),
      node_parent: vi.fn(() => undefined),
      history_generation: vi.fn(() => 1n),
      follow_me_along_edges: vi.fn(() => ++n),
      follow_me_along_edges_with: vi.fn(() => ++n),
    }
    const tool = new FollowMeTool(scene as never, new THREE.Group(), vi.fn(), vi.fn(), vi.fn(), [])
    const ray = { origin: [0, 0, 5], direction: [0, 0, -1] } as never
    const key = (k: string) => ({ key: k, ctrlKey: false, metaKey: false }) as KeyboardEvent
    tool.onPointerDown(null, ray)
    expect(tool.statusHint()).not.toMatch(/twist|shrink/i)
    // The suffix letters are not typed into the box; a twist never lands.
    expect(tool.capturesKey('d')).toBe(false)
    expect(tool.capturesKey('%')).toBe(false)
    for (const c of '30°') tool.onKey(key(c))
    tool.onKey(key('Enter'))
    expect(tool.statusHint()).not.toMatch(/Twist/)
    tool.onPointerDown(null, ray)
    expect(scene.follow_me_along_edges_with).not.toHaveBeenCalled()
  })
})

describe('preview build', () => {
  it('MenuBar: the Preview mark, and the tag on a Preview tool but not on an ordinary one', async () => {
    await fresh(true)
    const { MenuBar } = await import('../panels/MenuBar')
    render(<MenuBar {...menuProps} />)
    const mark = screen.getByTestId('preview-mark')
    expect(mark).toHaveTextContent('Preview')
    expect(mark.getAttribute('title')).toMatch(/unfinished features/)
    fireEvent.click(screen.getByRole('button', { name: /^tools$/i }))
    const fillet = screen.getByText('Fillet')
    expect(fillet).toHaveAttribute('data-preview-tag', 'Preview')
    expect(screen.getByText('Push/Pull')).not.toHaveAttribute('data-preview-tag')
  })

  it('ToolRail: the tooltip of a Preview tool carries the tag; an ordinary tool does not', async () => {
    await fresh(true)
    const { ToolRail } = await import('../panels/ToolRail')
    render(<ToolRail activeTool="Select" onSelectTool={vi.fn()} onOpenPalette={vi.fn()} onOpenLibrary={vi.fn()} narrow={false} onToggleNarrow={vi.fn()} />)
    expect(screen.getByRole('radio', { name: 'Fillet' }).getAttribute('title')).toMatch(/· Preview$/)
    expect(screen.getByRole('radio', { name: 'Push/Pull' }).getAttribute('title')).not.toMatch(/Preview/)
  })

  it('Report Bug: the system block says it is a preview build and lists the features on', async () => {
    await fresh(true)
    const { gatherSystemInfo } = await import('../log/reportBundle')
    const info = gatherSystemInfo('1.0.0', 'web-linux')
    expect(info.preview).toBe(true)
    expect(info.previewFeatures).toEqual(FEATURES.filter((f) => f.defaultOn !== false).map((f) => f.id))
  })

  it('crash reproducer: the manifest carries the same stamp', async () => {
    await fresh(true)
    const mod = await import('../log/reproducerDump')
    const src = readFileSync(join(ROOT, 'app/src/log/reproducerDump.ts'), 'utf8')
    expect(src).toContain('previewFeatures: enabledFeatureIds()')
    expect(mod).toBeDefined()
  })
})

describe('production build stamps nothing', () => {
  it('Report Bug: no preview keys in the system block', async () => {
    await fresh(false)
    const { gatherSystemInfo } = await import('../log/reportBundle')
    const info = gatherSystemInfo('1.0.0', 'web-linux')
    expect(Object.keys(info)).not.toContain('preview')
    expect(Object.keys(info)).not.toContain('previewFeatures')
  })
})

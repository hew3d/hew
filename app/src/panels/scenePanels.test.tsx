/**
 *  — component tests for scene-dependent panels.
 *
 * Covers: ObjectInfoPanel, MaterialPalette, DocumentTree, TagsPanel.
 *
 * All panel components import Scene exclusively as `import type`, so the type
 * is erased at runtime and no wasm/loader mock is required — a plain JS object
 * with the right methods stands in as the Scene.
 */

import { render, screen, fireEvent } from '@testing-library/react'
import { describe, expect, it, vi, beforeEach } from 'vitest'
import { ObjectInfoPanel } from './ObjectInfoPanel'
import { MaterialPalette } from './MaterialPalette'
import { DocumentTree } from './DocumentTree'
import { TagsPanel } from './TagsPanel'
import type { Scene as WasmScene } from '../wasm/loader'
import { MATERIAL_SENTINEL } from '../tools/PaintTool'

// ---------------------------------------------------------------------------
// Mock Scene factory — provides only the methods the panels actually call.
// Each call-site passes overrides to customise the scene for that test.
// ---------------------------------------------------------------------------

// eslint-disable-next-line @typescript-eslint/no-explicit-any
function makeScene(overrides: Record<string, any> = {}): WasmScene {
  return {
    object_ids: () => new BigUint64Array(),
    group_ids: () => new BigUint64Array(),
    instance_ids: () => new BigUint64Array(),
    sketch_ids: () => new BigUint64Array(),
    top_level_nodes: () => [],
    object_name: (_id: bigint) => undefined as string | undefined,
    group_name: (_id: bigint) => undefined as string | undefined,
    instance_name: (_id: bigint) => undefined as string | undefined,
    node_tags: (_kind: number, _id: bigint) => [] as string[],
    tag_meta_paths: () => [] as string[],
    tag_meta_hidden: () => new Uint8Array(),
    set_tag_hidden: vi.fn(),
    object_solid: (_id: bigint) => true,
    set_node_name: vi.fn(),
    add_node_tag: vi.fn(),
    remove_node_tag: vi.fn(),
    material_ids: () => new BigUint64Array(),
    material_info: (_id: bigint) => undefined,
    material_texture_bytes: (_id: bigint) => undefined,
    add_material: vi.fn().mockReturnValue(1n),
    add_texture_material: vi.fn(),
    set_object_material: vi.fn(),
    node_leaf_objects: (_kind: number, _id: bigint) => new BigUint64Array(),
    group_members: (_id: bigint) => [] as { kind: string; id: bigint }[],
    node_parent: (_kind: number, _id: bigint) => undefined as bigint | undefined,
    instance_def: (_id: bigint) => undefined as bigint | undefined,
    component_name: (_id: bigint) => undefined as string | undefined,
    ...overrides,
  } as unknown as WasmScene
}

// ---------------------------------------------------------------------------
// ObjectInfoPanel
// ---------------------------------------------------------------------------

describe('ObjectInfoPanel', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('shows "Select an object." when nothing is selected', () => {
    render(
      <ObjectInfoPanel
        scene={makeScene()}
        docRev={0}
        selectedIds={[]}
        onDocumentChanged={vi.fn()}
      />,
    )
    expect(screen.getByText(/select an object/i)).toBeInTheDocument()
  })

  it('shows "Multiple nodes selected." with more than one selection', () => {
    render(
      <ObjectInfoPanel
        scene={makeScene()}
        docRev={0}
        selectedIds={[
          { kind: 'object', id: 1n },
          { kind: 'object', id: 2n },
        ]}
        onDocumentChanged={vi.fn()}
      />,
    )
    expect(screen.getByText(/multiple nodes selected/i)).toBeInTheDocument()
  })

  it('shows the type label "Object" for an object node', () => {
    render(
      <ObjectInfoPanel
        scene={makeScene({
          object_name: () => 'Arch',
          node_tags: () => [],
          object_solid: () => true,
        })}
        docRev={0}
        selectedIds={[{ kind: 'object', id: 1n }]}
        onDocumentChanged={vi.fn()}
      />,
    )
    expect(screen.getByText('Object')).toBeInTheDocument()
  })

  it('populates the name input with the kernel-supplied name', () => {
    render(
      <ObjectInfoPanel
        scene={makeScene({
          object_name: () => 'Bridge Deck',
          node_tags: () => [],
          object_solid: () => true,
        })}
        docRev={0}
        selectedIds={[{ kind: 'object', id: 1n }]}
        onDocumentChanged={vi.fn()}
      />,
    )
    const input = screen.getByPlaceholderText('(unnamed)') as HTMLInputElement
    expect(input.value).toBe('Bridge Deck')
  })

  it('shows "Solid" for a watertight object', () => {
    render(
      <ObjectInfoPanel
        scene={makeScene({
          object_name: () => undefined,
          node_tags: () => [],
          object_solid: () => true,
        })}
        docRev={0}
        selectedIds={[{ kind: 'object', id: 1n }]}
        onDocumentChanged={vi.fn()}
      />,
    )
    expect(screen.getByText('Solid')).toBeInTheDocument()
  })

  it('shows "Leaky" for a non-watertight object', () => {
    render(
      <ObjectInfoPanel
        scene={makeScene({
          object_name: () => undefined,
          node_tags: () => [],
          object_solid: () => false,
        })}
        docRev={0}
        selectedIds={[{ kind: 'object', id: 1n }]}
        onDocumentChanged={vi.fn()}
      />,
    )
    expect(screen.getByText('Leaky')).toBeInTheDocument()
  })

  it('shows "Group" for a group node with no solid/leaky indicator', () => {
    render(
      <ObjectInfoPanel
        scene={makeScene({
          group_name: () => undefined,
          node_tags: () => [],
        })}
        docRev={0}
        selectedIds={[{ kind: 'group', id: 2n }]}
        onDocumentChanged={vi.fn()}
      />,
    )
    expect(screen.getByText('Group')).toBeInTheDocument()
    expect(screen.queryByText('Solid')).not.toBeInTheDocument()
    expect(screen.queryByText('Leaky')).not.toBeInTheDocument()
  })

  it('renders a tag chip for each existing tag on the node', () => {
    render(
      <ObjectInfoPanel
        scene={makeScene({
          object_name: () => undefined,
          // "Structure/Roof" → path ['Structure','Roof'] → chip shows "Structure / Roof"
          node_tags: () => ['Structure/Roof'],
          object_solid: () => true,
        })}
        docRev={0}
        selectedIds={[{ kind: 'object', id: 1n }]}
        onDocumentChanged={vi.fn()}
      />,
    )
    expect(screen.getByText('Structure / Roof')).toBeInTheDocument()
  })

  it('calls remove_node_tag with the path when the × chip button is clicked', () => {
    const scene = makeScene({
      object_name: () => undefined,
      node_tags: () => ['Walls'],
      object_solid: () => true,
    })
    const onDocumentChanged = vi.fn()
    render(
      <ObjectInfoPanel
        scene={scene}
        docRev={0}
        selectedIds={[{ kind: 'object', id: 1n }]}
        onDocumentChanged={onDocumentChanged}
      />,
    )
    fireEvent.click(screen.getByTitle('Remove tag'))
    expect((scene as any).remove_node_tag).toHaveBeenCalledWith(0, 1n, ['Walls'])
    expect(onDocumentChanged).toHaveBeenCalled()
  })

  it('calls add_node_tag with split segments on Enter in the tag input', () => {
    const scene = makeScene({
      object_name: () => undefined,
      node_tags: () => [],
      object_solid: () => true,
    })
    const onDocumentChanged = vi.fn()
    render(
      <ObjectInfoPanel
        scene={scene}
        docRev={0}
        selectedIds={[{ kind: 'object', id: 1n }]}
        onDocumentChanged={onDocumentChanged}
      />,
    )
    const tagInput = screen.getByPlaceholderText('Structure/Roof')
    fireEvent.change(tagInput, { target: { value: 'Mech/HVAC' } })
    fireEvent.keyDown(tagInput, { key: 'Enter' })
    expect((scene as any).add_node_tag).toHaveBeenCalledWith(0, 1n, ['Mech', 'HVAC'])
    expect(onDocumentChanged).toHaveBeenCalled()
  })

  it('commits the edited name to the scene on blur', () => {
    const scene = makeScene({
      object_name: () => '',
      node_tags: () => [],
      object_solid: () => true,
    })
    const onDocumentChanged = vi.fn()
    render(
      <ObjectInfoPanel
        scene={scene}
        docRev={0}
        selectedIds={[{ kind: 'object', id: 1n }]}
        onDocumentChanged={onDocumentChanged}
      />,
    )
    const nameInput = screen.getByPlaceholderText('(unnamed)')
    fireEvent.change(nameInput, { target: { value: 'Pillar' } })
    fireEvent.blur(nameInput)
    expect((scene as any).set_node_name).toHaveBeenCalledWith(0, 1n, 'Pillar')
    expect(onDocumentChanged).toHaveBeenCalled()
  })

  it('shows "Sketch" + its positional label for a sketch selection, and never crashes', () => {
    const scene = makeScene({
      sketch_ids: () => new BigUint64Array([10n, 20n, 30n]),
    })
    render(
      <ObjectInfoPanel
        scene={scene}
        docRev={0}
        selectedIds={[{ kind: 'sketch', id: 20n }]}
        onDocumentChanged={vi.fn()}
      />,
    )
    expect(screen.getByText('Sketch')).toBeInTheDocument()
    // 20n is index 1 in sketch_ids() → "Sketch 2" (1-based, matching the tree's label).
    expect(screen.getByText('Sketch 2')).toBeInTheDocument()
    expect(screen.getByText(/naming and tags are not yet supported/i)).toBeInTheDocument()
    // No name input (sketches aren't renameable), no crash from nodeKindToNumber.
    expect(screen.queryByPlaceholderText('(unnamed)')).not.toBeInTheDocument()
  })
})

// ---------------------------------------------------------------------------
// MaterialPalette
// ---------------------------------------------------------------------------

describe('MaterialPalette', () => {
  const baseProps = {
    docRev: 0,
    currentMaterialId: MATERIAL_SENTINEL,
    onSelectMaterial: vi.fn(),
    onDocumentChanged: vi.fn(),
    selectedIds: [] as { kind: 'object' | 'group' | 'instance' | 'sketch'; id: bigint }[],
  }

  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('renders the Default swatch with title "Default (unpainted)"', () => {
    render(<MaterialPalette {...baseProps} scene={makeScene()} />)
    expect(screen.getByTitle('Default (unpainted)')).toBeInTheDocument()
  })

  it('"Fill selected object" is disabled when nothing is selected', () => {
    render(<MaterialPalette {...baseProps} scene={makeScene()} selectedIds={[]} />)
    expect(screen.getByRole('button', { name: /fill selected object/i })).toBeDisabled()
  })

  it('"Fill selected object" is enabled when an object is selected', () => {
    render(
      <MaterialPalette
        {...baseProps}
        scene={makeScene()}
        selectedIds={[{ kind: 'object', id: 1n }]}
      />,
    )
    expect(screen.getByRole('button', { name: /fill selected object/i })).not.toBeDisabled()
  })

  it('clicking the Default swatch calls onSelectMaterial(MATERIAL_SENTINEL)', () => {
    const onSelectMaterial = vi.fn()
    render(
      <MaterialPalette {...baseProps} scene={makeScene()} onSelectMaterial={onSelectMaterial} />,
    )
    fireEvent.click(screen.getByTitle('Default (unpainted)'))
    expect(onSelectMaterial).toHaveBeenCalledWith(MATERIAL_SENTINEL)
  })

  it('renders a "+ Add color" button', () => {
    render(<MaterialPalette {...baseProps} scene={makeScene()} />)
    expect(screen.getByRole('button', { name: /\+ add color/i })).toBeInTheDocument()
  })

  it('clicking "+ Add color" calls add_material, onSelectMaterial, and onDocumentChanged', () => {
    const scene = makeScene()
    const onSelectMaterial = vi.fn()
    const onDocumentChanged = vi.fn()
    render(
      <MaterialPalette
        {...baseProps}
        scene={scene}
        onSelectMaterial={onSelectMaterial}
        onDocumentChanged={onDocumentChanged}
      />,
    )
    fireEvent.click(screen.getByRole('button', { name: /\+ add color/i }))
    expect((scene as any).add_material).toHaveBeenCalled()
    expect(onSelectMaterial).toHaveBeenCalled()
    expect(onDocumentChanged).toHaveBeenCalled()
  })

  it('renders a named swatch for each material returned by the scene', () => {
    const mockInfo = {
      r: () => 255,
      g: () => 0,
      b: () => 0,
      name: () => 'Red Paint',
      has_texture: () => false,
    }
    const scene = makeScene({
      material_ids: () => new BigUint64Array([7n]),
      material_info: () => mockInfo,
    })
    render(<MaterialPalette {...baseProps} scene={scene} currentMaterialId={7n} />)
    expect(screen.getByTitle('Red Paint')).toBeInTheDocument()
    expect(screen.getByText('Red Paint')).toBeInTheDocument()
  })

  it('clicking a material swatch calls onSelectMaterial with its id', () => {
    const mockInfo = {
      r: () => 0,
      g: () => 128,
      b: () => 255,
      name: () => 'Sky Blue',
      has_texture: () => false,
    }
    const scene = makeScene({
      material_ids: () => new BigUint64Array([3n]),
      material_info: () => mockInfo,
    })
    const onSelectMaterial = vi.fn()
    render(
      <MaterialPalette
        {...baseProps}
        scene={scene}
        onSelectMaterial={onSelectMaterial}
      />,
    )
    fireEvent.click(screen.getByTitle('Sky Blue'))
    expect(onSelectMaterial).toHaveBeenCalledWith(3n)
  })
})

// ---------------------------------------------------------------------------
// DocumentTree
// ---------------------------------------------------------------------------

const docTreeBase = {
  docRev: 0,
  watertightMap: new Map<bigint, boolean>(),
  selectedIds: [] as { kind: 'object' | 'group' | 'instance' | 'sketch'; id: bigint }[],
  activeContext: [] as { kind: 'object' | 'group' | 'instance' | 'sketch'; id: bigint }[],
  onSelect: vi.fn(),
  onEnterContext: vi.fn(),
  onExitContext: vi.fn(),
  onSetContextDepth: vi.fn(),
  canBoolean: false,
  onBoolean: vi.fn(),
  onGroup: vi.fn(),
  onUngroup: vi.fn(),
  canMakeComponent: false,
  onMakeComponent: vi.fn(),
  canPlaceInstance: false,
  onPlaceInstance: vi.fn(),
  canExplodeInstance: false,
  onExplodeInstance: vi.fn(),
  canMakeUnique: false,
  onMakeUnique: vi.fn(),
  hiddenKeys: new Set<string>(),
  onToggleHidden: vi.fn(),
}

describe('DocumentTree', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    // jsdom does not implement scrollIntoView; polyfill so DocumentTree's
    // scroll-primary-selection-into-view effect doesn't throw.
    Element.prototype.scrollIntoView = vi.fn()
  })

  it('shows "(no solids yet)" when the scene is empty', () => {
    render(<DocumentTree {...docTreeBase} scene={makeScene()} />)
    expect(screen.getByText('(no solids yet)')).toBeInTheDocument()
  })

  it('shows "(no sketches yet)" when the scene has no sketches', () => {
    render(<DocumentTree {...docTreeBase} scene={makeScene()} />)
    expect(screen.getByText('(no sketches yet)')).toBeInTheDocument()
  })

  it('renders "Model" breadcrumb at the top level', () => {
    render(<DocumentTree {...docTreeBase} scene={makeScene()} />)
    expect(screen.getByText('Model')).toBeInTheDocument()
  })

  it('renders an object row using its positional label when unnamed', () => {
    const scene = makeScene({
      top_level_nodes: () => [{ kind: 'object', id: 1n }],
      object_ids: () => new BigUint64Array([1n]),
      object_name: () => undefined,
      node_parent: () => undefined,
    })
    render(
      <DocumentTree
        {...docTreeBase}
        scene={scene}
        watertightMap={new Map([[1n, true]])}
      />,
    )
    // resolveLabel(undefined, undefined, 'object', 0) → "Object 1"
    expect(screen.getByText('Object 1')).toBeInTheDocument()
  })

  it('uses the kernel name when the object has one', () => {
    const scene = makeScene({
      top_level_nodes: () => [{ kind: 'object', id: 1n }],
      object_ids: () => new BigUint64Array([1n]),
      object_name: () => 'Bridge Arch',
      node_parent: () => undefined,
    })
    render(
      <DocumentTree
        {...docTreeBase}
        scene={scene}
        watertightMap={new Map([[1n, true]])}
      />,
    )
    expect(screen.getByText('Bridge Arch')).toBeInTheDocument()
  })

  it('shows Union/Subtract/Intersect buttons when canBoolean=true', () => {
    render(
      <DocumentTree {...docTreeBase} scene={makeScene()} canBoolean={true} />,
    )
    expect(screen.getByRole('button', { name: /union/i })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: /subtract/i })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: /intersect/i })).toBeInTheDocument()
  })

  it('calls onBoolean(0) when Union is clicked', () => {
    const onBoolean = vi.fn()
    render(
      <DocumentTree
        {...docTreeBase}
        scene={makeScene()}
        canBoolean={true}
        onBoolean={onBoolean}
      />,
    )
    fireEvent.click(screen.getByRole('button', { name: /union/i }))
    expect(onBoolean).toHaveBeenCalledWith(0)
  })

  it('calls onBoolean(1) when Subtract is clicked', () => {
    const onBoolean = vi.fn()
    render(
      <DocumentTree
        {...docTreeBase}
        scene={makeScene()}
        canBoolean={true}
        onBoolean={onBoolean}
      />,
    )
    fireEvent.click(screen.getByRole('button', { name: /subtract/i }))
    expect(onBoolean).toHaveBeenCalledWith(1)
  })

  it('calls onSelect with the node ref when an object row is clicked', () => {
    const scene = makeScene({
      top_level_nodes: () => [{ kind: 'object', id: 1n }],
      object_ids: () => new BigUint64Array([1n]),
      object_name: () => 'Cube',
      node_parent: () => undefined,
    })
    const onSelect = vi.fn()
    render(
      <DocumentTree
        {...docTreeBase}
        scene={scene}
        watertightMap={new Map([[1n, true]])}
        onSelect={onSelect}
      />,
    )
    fireEvent.click(screen.getByText('Cube'))
    expect(onSelect).toHaveBeenCalledWith({ kind: 'object', id: 1n }, false)
  })

  it('calls onSelect with a sketch NodeRef when a Sketches-section row is clicked', () => {
    const scene = makeScene({
      sketch_ids: () => new BigUint64Array([5n]),
    })
    const onSelect = vi.fn()
    render(
      <DocumentTree
        {...docTreeBase}
        scene={scene}
        onSelect={onSelect}
      />,
    )
    fireEvent.click(screen.getByText('Sketch 1'))
    expect(onSelect).toHaveBeenCalledWith({ kind: 'sketch', id: 5n }, false)
  })

  it('highlights the sketch row selected from the canvas (canvas → tree)', () => {
    const scene = makeScene({
      sketch_ids: () => new BigUint64Array([5n]),
    })
    render(
      <DocumentTree
        {...docTreeBase}
        scene={scene}
        selectedIds={[{ kind: 'sketch', id: 5n }]}
      />,
    )
    const row = screen.getByText('Sketch 1').closest('div')
    // The primary-selection tint is applied via inline style — assert it isn't
    // the default transparent background a non-selected row would get.
    expect(row).toHaveStyle({ background: 'var(--accent-tint-18)' })
  })

  it('shows the Group button when two sibling objects are selected', () => {
    // canGroupHelper returns true when >=2 nodes all share the same parent
    // (undefined = top-level), which holds here.
    const scene = makeScene({
      top_level_nodes: () => [
        { kind: 'object', id: 1n },
        { kind: 'object', id: 2n },
      ],
      object_ids: () => new BigUint64Array([1n, 2n]),
      object_name: () => undefined,
      node_parent: () => undefined,
    })
    render(
      <DocumentTree
        {...docTreeBase}
        scene={scene}
        watertightMap={new Map([[1n, true], [2n, true]])}
        selectedIds={[
          { kind: 'object', id: 1n },
          { kind: 'object', id: 2n },
        ]}
      />,
    )
    expect(screen.getByRole('button', { name: /^group$/i })).toBeInTheDocument()
  })

  it('calls onGroup when the Group button is clicked', () => {
    const scene = makeScene({
      top_level_nodes: () => [
        { kind: 'object', id: 1n },
        { kind: 'object', id: 2n },
      ],
      object_ids: () => new BigUint64Array([1n, 2n]),
      object_name: () => undefined,
      node_parent: () => undefined,
    })
    const onGroup = vi.fn()
    render(
      <DocumentTree
        {...docTreeBase}
        scene={scene}
        watertightMap={new Map([[1n, true], [2n, true]])}
        selectedIds={[
          { kind: 'object', id: 1n },
          { kind: 'object', id: 2n },
        ]}
        onGroup={onGroup}
      />,
    )
    fireEvent.click(screen.getByRole('button', { name: /^group$/i }))
    expect(onGroup).toHaveBeenCalled()
  })
})

// ---------------------------------------------------------------------------
// TagsPanel
// ---------------------------------------------------------------------------

describe('TagsPanel', () => {
  const baseTagProps = {
    docRev: 0,
    hiddenTagPaths: new Set<string>(),
    onToggleTagPath: vi.fn(),
  }

  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('shows "No tags found." when the scene has no tagged objects', () => {
    render(<TagsPanel {...baseTagProps} scene={makeScene()} />)
    expect(screen.getByText(/no tags found/i)).toBeInTheDocument()
  })

  it('renders the root tag segment from node_tags', () => {
    const scene = makeScene({
      object_ids: () => new BigUint64Array([1n]),
      node_tags: (_kind: number, id: bigint) => (id === 1n ? ['Structure/Roof'] : []),
    })
    render(<TagsPanel {...baseTagProps} scene={scene} />)
    // buildTagTree builds "Structure" root with "Roof" child.
    // The root row renders node.segment = "Structure".
    expect(screen.getByText('Structure')).toBeInTheDocument()
  })

  it('calls onToggleTagPath with the tag path when the eye button is clicked', () => {
    const scene = makeScene({
      object_ids: () => new BigUint64Array([1n]),
      node_tags: (_kind: number, id: bigint) => (id === 1n ? ['Walls'] : []),
    })
    const onToggleTagPath = vi.fn()
    render(
      <TagsPanel
        {...baseTagProps}
        scene={scene}
        onToggleTagPath={onToggleTagPath}
      />,
    )
    fireEvent.click(screen.getByTitle('Hide tagged objects'))
    expect(onToggleTagPath).toHaveBeenCalledWith(['Walls'])
  })

  it('shows "Show tagged objects" title when the path is hidden', () => {
    const scene = makeScene({
      object_ids: () => new BigUint64Array([1n]),
      node_tags: (_kind: number, id: bigint) => (id === 1n ? ['Walls'] : []),
    })
    // tagPathKey(['Walls']) = JSON.stringify(['Walls']) = '["Walls"]'
    render(
      <TagsPanel
        {...baseTagProps}
        scene={scene}
        hiddenTagPaths={new Set(['["Walls"]'])}
      />,
    )
    expect(screen.getByTitle('Show tagged objects')).toBeInTheDocument()
  })

  it('renders nested child tags', () => {
    const scene = makeScene({
      object_ids: () => new BigUint64Array([1n]),
      node_tags: (_kind: number, id: bigint) =>
        id === 1n ? ['Structure/Roof'] : [],
    })
    render(<TagsPanel {...baseTagProps} scene={scene} />)
    // "Roof" is a child of "Structure" — both should appear in the expanded tree
    expect(screen.getByText('Roof')).toBeInTheDocument()
  })

  it('shows a registry tag path even when no node carries it (e.g. an empty imported .skp layer)', () => {
    const scene = makeScene({
      // No node is tagged at all — the tag only exists in the registry.
      tag_meta_paths: () => ['Imported/EmptyLayer'],
      tag_meta_hidden: () => new Uint8Array([0]),
    })
    render(<TagsPanel {...baseTagProps} scene={scene} />)
    expect(screen.queryByText(/no tags found/i)).not.toBeInTheDocument()
    expect(screen.getByText('Imported')).toBeInTheDocument()
    expect(screen.getByText('EmptyLayer')).toBeInTheDocument()
  })

  it('unions registry tags with node-derived tags rather than replacing them', () => {
    const scene = makeScene({
      object_ids: () => new BigUint64Array([1n]),
      node_tags: (_kind: number, id: bigint) => (id === 1n ? ['Walls'] : []),
      tag_meta_paths: () => ['Walls', 'EmptyLayer'],
      tag_meta_hidden: () => new Uint8Array([0, 0]),
    })
    render(<TagsPanel {...baseTagProps} scene={scene} />)
    expect(screen.getByText('Walls')).toBeInTheDocument()
    expect(screen.getByText('EmptyLayer')).toBeInTheDocument()
  })
})

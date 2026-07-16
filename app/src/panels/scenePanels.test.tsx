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
    sketch_island_ids: (sid: bigint) => new BigUint64Array([sid + 100n]),
    sketch_edge_island: () => undefined,
    sketch_curve_edges: () => new BigUint64Array(),
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
    set_material_alpha: vi.fn(),
    node_leaf_objects: (_kind: number, _id: bigint) => new BigUint64Array(),
    group_members: (_id: bigint) => [] as { kind: string; id: bigint }[],
    node_parent: (_kind: number, _id: bigint) => undefined as bigint | undefined,
    instance_def: (_id: bigint) => undefined as bigint | undefined,
    component_name: (_id: bigint) => undefined as string | undefined,
    instances_of: (_id: bigint) => new BigUint64Array(),
    set_component_name: vi.fn(),
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

  it('renders an empty panel (no text at all) when nothing is selected', () => {
    const { container } = render(
      <ObjectInfoPanel
        scene={makeScene()}
        docRev={0}
        selectedIds={[]}
        onDocumentChanged={vi.fn()}
        onSelectMany={vi.fn()}
      />,
    )
    expect(container.textContent).toBe('')
  })

  it('shows only a quiet count line for a multi-selection', () => {
    render(
      <ObjectInfoPanel
        scene={makeScene()}
        docRev={0}
        selectedIds={[
          { kind: 'object', id: 1n },
          { kind: 'object', id: 2n },
          { kind: 'object', id: 3n },
        ]}
        onDocumentChanged={vi.fn()}
        onSelectMany={vi.fn()}
      />,
    )
    expect(screen.getByText('3 selected')).toBeInTheDocument()
    expect(screen.queryByText(/multiple nodes selected/i)).not.toBeInTheDocument()
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
        onSelectMany={vi.fn()}
      />,
    )
    expect(screen.getByText('Object')).toBeInTheDocument()
  })

  it('populates the name input with the kernel-supplied name', () => {
    render(
      <ObjectInfoPanel
        scene={makeScene({
          object_ids: () => new BigUint64Array([1n]),
          object_name: () => 'Bridge Deck',
          node_tags: () => [],
          object_solid: () => true,
        })}
        docRev={0}
        selectedIds={[{ kind: 'object', id: 1n }]}
        onDocumentChanged={vi.fn()}
        onSelectMany={vi.fn()}
      />,
    )
    const input = screen.getByPlaceholderText('Object 1') as HTMLInputElement
    expect(input.value).toBe('Bridge Deck')
  })

  it('never shows "(unnamed)" — an unnamed object falls back to the Outliner default label', () => {
    render(
      <ObjectInfoPanel
        scene={makeScene({
          object_ids: () => new BigUint64Array([7n, 8n]),
          top_level_nodes: () => [
            { kind: 'object', id: 7n },
            { kind: 'object', id: 8n },
          ],
          object_name: () => undefined,
          node_tags: () => [],
          object_solid: () => true,
        })}
        docRev={0}
        selectedIds={[{ kind: 'object', id: 8n }]}
        onDocumentChanged={vi.fn()}
        onSelectMany={vi.fn()}
      />,
    )
    // 8n is the second top-level row → the same "Object 2" the Outliner shows.
    const input = screen.getByPlaceholderText('Object 2') as HTMLInputElement
    expect(input.value).toBe('')
    expect(screen.queryByPlaceholderText('(unnamed)')).not.toBeInTheDocument()
    expect(screen.queryByText('(unnamed)')).not.toBeInTheDocument()
  })

  it('numbers an unnamed nested object by its position in its group, like the Outliner', () => {
    render(
      <ObjectInfoPanel
        scene={makeScene({
          // Object 8n is globally the second object, but the FIRST member of
          // its group — the Outliner shows "Object 1", so this panel must
          // too (the flat object_ids() index would say "Object 2").
          object_ids: () => new BigUint64Array([7n, 8n]),
          top_level_nodes: () => [
            { kind: 'object', id: 7n },
            { kind: 'group', id: 20n },
          ],
          group_members: (id: bigint) =>
            id === 20n ? [{ kind: 'object', id: 8n }] : [],
          object_name: () => undefined,
          node_tags: () => [],
          object_solid: () => true,
        })}
        docRev={0}
        selectedIds={[{ kind: 'object', id: 8n }]}
        onDocumentChanged={vi.fn()}
        onSelectMany={vi.fn()}
      />,
    )
    expect(screen.getByPlaceholderText('Object 1')).toBeInTheDocument()
  })

  it('an unnamed instance falls back to its component definition name (Outliner parity)', () => {
    render(
      <ObjectInfoPanel
        scene={makeScene({
          instance_ids: () => new BigUint64Array([4n]),
          instance_name: () => undefined,
          instance_def: () => 9n,
          component_name: () => 'Door',
          node_tags: () => [],
        })}
        docRev={0}
        selectedIds={[{ kind: 'instance', id: 4n }]}
        onDocumentChanged={vi.fn()}
        onSelectMany={vi.fn()}
      />,
    )
    expect(screen.getByPlaceholderText('Door')).toBeInTheDocument()
  })

  it('shows Definition Name and Instance Name fields for an instance', () => {
    render(
      <ObjectInfoPanel
        scene={makeScene({
          instance_ids: () => new BigUint64Array([4n]),
          instance_name: () => 'Front Door',
          instance_def: () => 9n,
          component_name: () => 'Door',
          instances_of: () => new BigUint64Array([4n]),
          node_tags: () => [],
        })}
        docRev={0}
        selectedIds={[{ kind: 'instance', id: 4n }]}
        onDocumentChanged={vi.fn()}
        onSelectMany={vi.fn()}
      />,
    )
    const defInput = screen.getByLabelText('Definition Name') as HTMLInputElement
    expect(defInput.value).toBe('Door')
    const instInput = screen.getByLabelText('Instance Name') as HTMLInputElement
    expect(instInput.value).toBe('Front Door')
  })

  it('commits a Definition Name edit through set_component_name (renames every instance)', () => {
    const scene = makeScene({
      instance_ids: () => new BigUint64Array([4n]),
      instance_name: () => undefined,
      instance_def: () => 9n,
      component_name: () => 'Door',
      instances_of: () => new BigUint64Array([4n]),
      node_tags: () => [],
    })
    const onDocumentChanged = vi.fn()
    render(
      <ObjectInfoPanel
        scene={scene}
        docRev={0}
        selectedIds={[{ kind: 'instance', id: 4n }]}
        onDocumentChanged={onDocumentChanged}
        onSelectMany={vi.fn()}
      />,
    )
    const defInput = screen.getByLabelText('Definition Name')
    fireEvent.change(defInput, { target: { value: 'Oak Door' } })
    fireEvent.blur(defInput)
    expect((scene as any).set_component_name).toHaveBeenCalledWith(9n, 'Oak Door')
    expect(onDocumentChanged).toHaveBeenCalled()
  })

  it('shows "Component (N instances)" and the count selects every instance', () => {
    const onSelectMany = vi.fn()
    render(
      <ObjectInfoPanel
        scene={makeScene({
          instance_ids: () => new BigUint64Array([4n, 5n, 6n]),
          instance_name: () => undefined,
          instance_def: () => 9n,
          component_name: () => 'Door',
          instances_of: () => new BigUint64Array([4n, 5n, 6n]),
          node_tags: () => [],
        })}
        docRev={0}
        selectedIds={[{ kind: 'instance', id: 4n }]}
        onDocumentChanged={vi.fn()}
        onSelectMany={onSelectMany}
      />,
    )
    expect(screen.getByText('Component')).toBeInTheDocument()
    const count = screen.getByRole('button', { name: '(3 instances)' })
    fireEvent.click(count)
    expect(onSelectMany).toHaveBeenCalledWith([
      { kind: 'instance', id: 4n },
      { kind: 'instance', id: 5n },
      { kind: 'instance', id: 6n },
    ])
  })

  it('a lone instance reads "(1 instance)" — singular', () => {
    render(
      <ObjectInfoPanel
        scene={makeScene({
          instance_ids: () => new BigUint64Array([4n]),
          instance_name: () => undefined,
          instance_def: () => 9n,
          component_name: () => 'Door',
          instances_of: () => new BigUint64Array([4n]),
          node_tags: () => [],
        })}
        docRev={0}
        selectedIds={[{ kind: 'instance', id: 4n }]}
        onDocumentChanged={vi.fn()}
        onSelectMany={vi.fn()}
      />,
    )
    expect(screen.getByRole('button', { name: '(1 instance)' })).toBeInTheDocument()
  })

  it('does not leak uncommitted Name text across a selection change between same-valued nodes', () => {
    // Objects 1n and 2n are both unnamed — the kernel-name VALUE is identical
    // (undefined) across the selection change, so only an identity-keyed
    // reset clears the typed-but-uncommitted text.
    const scene = makeScene({
      object_ids: () => new BigUint64Array([1n, 2n]),
      top_level_nodes: () => [
        { kind: 'object', id: 1n },
        { kind: 'object', id: 2n },
      ],
      object_name: () => undefined,
      node_tags: () => [],
      object_solid: () => true,
    })
    const { rerender } = render(
      <ObjectInfoPanel
        scene={scene}
        docRev={0}
        selectedIds={[{ kind: 'object', id: 1n }]}
        onDocumentChanged={vi.fn()}
        onSelectMany={vi.fn()}
      />,
    )
    const inputA = screen.getByPlaceholderText('Object 1') as HTMLInputElement
    fireEvent.change(inputA, { target: { value: 'Sneaky' } })
    // Selection moves to object 2 WITHOUT a commit (no blur).
    rerender(
      <ObjectInfoPanel
        scene={scene}
        docRev={0}
        selectedIds={[{ kind: 'object', id: 2n }]}
        onDocumentChanged={vi.fn()}
        onSelectMany={vi.fn()}
      />,
    )
    const inputB = screen.getByPlaceholderText('Object 2') as HTMLInputElement
    expect(inputB.value).toBe('')
    fireEvent.blur(inputB)
    expect((scene as any).set_node_name).not.toHaveBeenCalledWith(0, 2n, 'Sneaky')
  })

  it('does not leak uncommitted Definition Name text across a selection change between instances of same-valued definitions', () => {
    // Instances 4n and 5n place two DIFFERENT definitions that are both
    // unnamed — the defName VALUE is identical across the selection change.
    const scene = makeScene({
      instance_ids: () => new BigUint64Array([4n, 5n]),
      top_level_nodes: () => [
        { kind: 'instance', id: 4n },
        { kind: 'instance', id: 5n },
      ],
      instance_name: () => undefined,
      instance_def: (id: bigint) => (id === 4n ? 9n : 10n),
      component_name: () => undefined,
      instances_of: (id: bigint) => new BigUint64Array([id === 9n ? 4n : 5n]),
      node_tags: () => [],
    })
    const { rerender } = render(
      <ObjectInfoPanel
        scene={scene}
        docRev={0}
        selectedIds={[{ kind: 'instance', id: 4n }]}
        onDocumentChanged={vi.fn()}
        onSelectMany={vi.fn()}
      />,
    )
    const defA = screen.getByLabelText('Definition Name') as HTMLInputElement
    fireEvent.change(defA, { target: { value: 'Sneaky' } })
    // Selection moves to instance 5 (of the other, equally unnamed def).
    rerender(
      <ObjectInfoPanel
        scene={scene}
        docRev={0}
        selectedIds={[{ kind: 'instance', id: 5n }]}
        onDocumentChanged={vi.fn()}
        onSelectMany={vi.fn()}
      />,
    )
    const defB = screen.getByLabelText('Definition Name') as HTMLInputElement
    expect(defB.value).toBe('')
    fireEvent.blur(defB)
    expect((scene as any).set_component_name).not.toHaveBeenCalledWith(10n, 'Sneaky')
  })

  it('preserves an uncommitted Definition Name edit across instances of the SAME definition', () => {
    // Instances 4n and 5n both place definition 9n. The field edits the
    // shared definition, so its identity join key is the DEFINITION —
    // switching between siblings must not wipe an in-progress edit, and the
    // eventual blur commits to the (one) definition either way.
    const scene = makeScene({
      instance_ids: () => new BigUint64Array([4n, 5n]),
      top_level_nodes: () => [
        { kind: 'instance', id: 4n },
        { kind: 'instance', id: 5n },
      ],
      instance_name: () => undefined,
      instance_def: () => 9n,
      component_name: () => undefined,
      instances_of: () => new BigUint64Array([4n, 5n]),
      node_tags: () => [],
    })
    const { rerender } = render(
      <ObjectInfoPanel
        scene={scene}
        docRev={0}
        selectedIds={[{ kind: 'instance', id: 4n }]}
        onDocumentChanged={vi.fn()}
        onSelectMany={vi.fn()}
      />,
    )
    const defA = screen.getByLabelText('Definition Name') as HTMLInputElement
    fireEvent.change(defA, { target: { value: 'Cabinet' } })
    // Selection moves to the sibling instance of the SAME definition.
    rerender(
      <ObjectInfoPanel
        scene={scene}
        docRev={0}
        selectedIds={[{ kind: 'instance', id: 5n }]}
        onDocumentChanged={vi.fn()}
        onSelectMany={vi.fn()}
      />,
    )
    const defB = screen.getByLabelText('Definition Name') as HTMLInputElement
    expect(defB.value).toBe('Cabinet')
    fireEvent.blur(defB)
    expect((scene as any).set_component_name).toHaveBeenCalledWith(9n, 'Cabinet')
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
        onSelectMany={vi.fn()}
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
        onSelectMany={vi.fn()}
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
        onSelectMany={vi.fn()}
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
        onSelectMany={vi.fn()}
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
        onSelectMany={vi.fn()}
      />,
    )
    fireEvent.click(screen.getByTitle('Remove tag'))
    expect((scene as any).remove_node_tag).toHaveBeenCalledWith(0, 1n, ['Walls'])
    expect(onDocumentChanged).toHaveBeenCalled()
  })

  it('has no "No tags" boilerplate — empty state is just the "+" button', () => {
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
        onSelectMany={vi.fn()}
      />,
    )
    expect(screen.queryByText(/no tags/i)).not.toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Add tag' })).toBeInTheDocument()
    // The add field is hidden until "+" is clicked.
    expect(screen.queryByPlaceholderText('Structure/Roof')).not.toBeInTheDocument()
  })

  it('clicking "+" reveals a focused tag field; Enter commits split segments and closes it', () => {
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
        onSelectMany={vi.fn()}
      />,
    )
    fireEvent.click(screen.getByRole('button', { name: 'Add tag' }))
    const tagInput = screen.getByPlaceholderText('Structure/Roof')
    expect(tagInput).toHaveFocus()
    fireEvent.change(tagInput, { target: { value: 'Mech/HVAC' } })
    fireEvent.keyDown(tagInput, { key: 'Enter' })
    expect((scene as any).add_node_tag).toHaveBeenCalledWith(0, 1n, ['Mech', 'HVAC'])
    expect(onDocumentChanged).toHaveBeenCalled()
    // Field closes after commit; the "+" affordance returns.
    expect(screen.queryByPlaceholderText('Structure/Roof')).not.toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Add tag' })).toBeInTheDocument()
  })

  it('Escape cancels the tag field without committing', () => {
    const scene = makeScene({
      object_name: () => undefined,
      node_tags: () => [],
      object_solid: () => true,
    })
    render(
      <ObjectInfoPanel
        scene={scene}
        docRev={0}
        selectedIds={[{ kind: 'object', id: 1n }]}
        onDocumentChanged={vi.fn()}
        onSelectMany={vi.fn()}
      />,
    )
    fireEvent.click(screen.getByRole('button', { name: 'Add tag' }))
    const tagInput = screen.getByPlaceholderText('Structure/Roof')
    fireEvent.change(tagInput, { target: { value: 'Half-typed' } })
    fireEvent.keyDown(tagInput, { key: 'Escape' })
    expect((scene as any).add_node_tag).not.toHaveBeenCalled()
    expect(screen.queryByPlaceholderText('Structure/Roof')).not.toBeInTheDocument()
  })

  it('blurring the empty tag field closes it without committing', () => {
    const scene = makeScene({
      object_name: () => undefined,
      node_tags: () => [],
      object_solid: () => true,
    })
    render(
      <ObjectInfoPanel
        scene={scene}
        docRev={0}
        selectedIds={[{ kind: 'object', id: 1n }]}
        onDocumentChanged={vi.fn()}
        onSelectMany={vi.fn()}
      />,
    )
    fireEvent.click(screen.getByRole('button', { name: 'Add tag' }))
    fireEvent.blur(screen.getByPlaceholderText('Structure/Roof'))
    expect((scene as any).add_node_tag).not.toHaveBeenCalled()
    expect(screen.queryByPlaceholderText('Structure/Roof')).not.toBeInTheDocument()
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
        onSelectMany={vi.fn()}
      />,
    )
    const nameInput = screen.getByPlaceholderText('Object 1')
    fireEvent.change(nameInput, { target: { value: 'Pillar' } })
    fireEvent.blur(nameInput)
    expect((scene as any).set_node_name).toHaveBeenCalledWith(0, 1n, 'Pillar')
    expect(onDocumentChanged).toHaveBeenCalled()
  })

  it('shows "Sketch" + its positional label for a sketch selection, with no help boilerplate', () => {
    const scene = makeScene({
      sketch_ids: () => new BigUint64Array([10n, 20n, 30n]),
    })
    render(
      <ObjectInfoPanel
        scene={scene}
        docRev={0}
        selectedIds={[{ kind: 'sketch-island', id: 120n, sketch: 20n }]}
        onDocumentChanged={vi.fn()}
        onSelectMany={vi.fn()}
      />,
    )
    expect(screen.getByText('Sketch')).toBeInTheDocument()
    // Island 120n of sketch 20n is row index 1 in the outliner's flattened
    // island list → "Sketch 2" (1-based, matching the tree's label).
    expect(screen.getByText('Sketch 2')).toBeInTheDocument()
    // The old explanatory boilerplate is gone — just the real fields.
    expect(screen.queryByText(/naming and tags are not yet supported/i)).not.toBeInTheDocument()
    expect(screen.queryByText(/a drawn line/i)).not.toBeInTheDocument()
    // No name input (sketches aren't renameable), no crash from nodeKindToNumber.
    expect(screen.queryByRole('textbox')).not.toBeInTheDocument()
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
    onMaterialCreated: vi.fn(),
    onDocumentChanged: vi.fn(),
    onAlphaCommitted: vi.fn(),
  }

  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('renders the Default swatch with title "Default (unpainted)"', () => {
    render(<MaterialPalette {...baseProps} scene={makeScene()} />)
    expect(screen.getByTitle('Default (unpainted)')).toBeInTheDocument()
  })

  it('has no "Fill selected object" button (whole-object paint is Ctrl/Cmd-click)', () => {
    render(<MaterialPalette {...baseProps} scene={makeScene()} />)
    expect(screen.queryByRole('button', { name: /fill selected object/i })).not.toBeInTheDocument()
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

  it('clicking "+ Add color" calls add_material, onMaterialCreated, and onDocumentChanged — but not onSelectMaterial (adding a color must not switch to the Paint tool)', () => {
    const scene = makeScene()
    const onSelectMaterial = vi.fn()
    const onMaterialCreated = vi.fn()
    const onDocumentChanged = vi.fn()
    render(
      <MaterialPalette
        {...baseProps}
        scene={scene}
        onSelectMaterial={onSelectMaterial}
        onMaterialCreated={onMaterialCreated}
        onDocumentChanged={onDocumentChanged}
      />,
    )
    fireEvent.click(screen.getByRole('button', { name: /\+ add color/i }))
    expect((scene as any).add_material).toHaveBeenCalled()
    expect(onMaterialCreated).toHaveBeenCalled()
    expect(onSelectMaterial).not.toHaveBeenCalled()
    expect(onDocumentChanged).toHaveBeenCalled()
  })

  it('renders a named swatch for each material returned by the scene', () => {
    const mockInfo = {
      r: () => 255,
      g: () => 0,
      b: () => 0,
      a: () => 255,
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

  it('opacity slider is disabled when the Default swatch is selected', () => {
    render(<MaterialPalette {...baseProps} scene={makeScene()} />)
    expect(screen.getByRole('slider')).toBeDisabled()
  })

  it('opacity slider reflects the selected material\'s current alpha', () => {
    const mockInfo = {
      r: () => 255,
      g: () => 0,
      b: () => 0,
      a: () => 128,
      name: () => 'Glass',
      has_texture: () => false,
    }
    const scene = makeScene({
      material_ids: () => new BigUint64Array([7n]),
      material_info: () => mockInfo,
    })
    render(<MaterialPalette {...baseProps} scene={scene} currentMaterialId={7n} />)
    const slider = screen.getByRole('slider') as HTMLInputElement
    expect(slider).not.toBeDisabled()
    expect(slider.value).toBe('128')
  })

  it('dragging the opacity slider previews without calling set_material_alpha', () => {
    const mockInfo = {
      r: () => 255,
      g: () => 0,
      b: () => 0,
      a: () => 255,
      name: () => 'Glass',
      has_texture: () => false,
    }
    const scene = makeScene({
      material_ids: () => new BigUint64Array([7n]),
      material_info: () => mockInfo,
    })
    render(<MaterialPalette {...baseProps} scene={scene} currentMaterialId={7n} />)
    fireEvent.change(screen.getByRole('slider'), { target: { value: '64' } })
    expect(screen.getByText('25%')).toBeInTheDocument()
    expect((scene as any).set_material_alpha).not.toHaveBeenCalled()
  })

  it('releasing the opacity slider commits set_material_alpha and onAlphaCommitted once, and does not double-fire onDocumentChanged', () => {
    const mockInfo = {
      r: () => 255,
      g: () => 0,
      b: () => 0,
      a: () => 255,
      name: () => 'Glass',
      has_texture: () => false,
    }
    const scene = makeScene({
      material_ids: () => new BigUint64Array([7n]),
      material_info: () => mockInfo,
    })
    const onDocumentChanged = vi.fn()
    const onAlphaCommitted = vi.fn()
    render(
      <MaterialPalette
        {...baseProps}
        scene={scene}
        currentMaterialId={7n}
        onDocumentChanged={onDocumentChanged}
        onAlphaCommitted={onAlphaCommitted}
      />,
    )
    const slider = screen.getByRole('slider')
    fireEvent.change(slider, { target: { value: '64' } })
    fireEvent.blur(slider)
    expect((scene as any).set_material_alpha).toHaveBeenCalledTimes(1)
    expect((scene as any).set_material_alpha).toHaveBeenCalledWith(7n, 64)
    expect(onAlphaCommitted).toHaveBeenCalledTimes(1)
    // onAlphaCommitted (Viewport.syncMaterialOpacity) already cascades into
    // onDocumentChanged in the real app; MaterialPalette must not also call
    // it directly, or every commit double-bumps docRev.
    expect(onDocumentChanged).not.toHaveBeenCalled()
    // A second blur with no intervening change is not a new gesture.
    fireEvent.blur(slider)
    expect((scene as any).set_material_alpha).toHaveBeenCalledTimes(1)
  })

  it('displays 100%/0% only at the exact alpha extremes, not from rounding', () => {
    const mockInfo = {
      r: () => 255,
      g: () => 0,
      b: () => 0,
      a: () => 254,
      name: () => 'Glass',
      has_texture: () => false,
    }
    const scene = makeScene({
      material_ids: () => new BigUint64Array([7n]),
      material_info: () => mockInfo,
    })
    render(<MaterialPalette {...baseProps} scene={scene} currentMaterialId={7n} />)
    // 254/255 rounds to 100% naively, but alpha=254 is not fully opaque.
    expect(screen.queryByText('100%')).not.toBeInTheDocument()
    expect(screen.getByText('99%')).toBeInTheDocument()
  })

  it('flushes an in-progress drag on unmount so the value is not silently lost', () => {
    const mockInfo = {
      r: () => 255,
      g: () => 0,
      b: () => 0,
      a: () => 255,
      name: () => 'Glass',
      has_texture: () => false,
    }
    const scene = makeScene({
      material_ids: () => new BigUint64Array([7n]),
      material_info: () => mockInfo,
    })
    const { unmount } = render(
      <MaterialPalette {...baseProps} scene={scene} currentMaterialId={7n} />,
    )
    fireEvent.change(screen.getByRole('slider'), { target: { value: '64' } })
    expect((scene as any).set_material_alpha).not.toHaveBeenCalled()
    unmount()
    expect((scene as any).set_material_alpha).toHaveBeenCalledWith(7n, 64)
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

  it('renders no placeholder text and no section headers for an empty scene', () => {
    render(<DocumentTree {...docTreeBase} scene={makeScene()} />)
    expect(screen.queryByText('(no solids yet)')).not.toBeInTheDocument()
    expect(screen.queryByText('(no sketches yet)')).not.toBeInTheDocument()
    expect(screen.queryByText(/^objects$/i)).not.toBeInTheDocument()
    expect(screen.queryByText(/^sketches$/i)).not.toBeInTheDocument()
  })

  it('renders "Model" breadcrumb at the top level', () => {
    render(<DocumentTree {...docTreeBase} scene={makeScene()} />)
    expect(screen.getByText('Model')).toBeInTheDocument()
  })

  it('has no action buttons — booleans/group/component ops live in the menus now', () => {
    const scene = makeScene({
      top_level_nodes: () => [
        { kind: 'object', id: 1n },
        { kind: 'object', id: 2n },
      ],
      object_ids: () => new BigUint64Array([1n, 2n]),
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
    for (const label of [/union/i, /subtract/i, /intersect/i, /^group$/i, /ungroup/i, /make component/i, /place copy/i, /explode/i, /make unique/i]) {
      expect(screen.queryByRole('button', { name: label })).not.toBeInTheDocument()
    }
  })

  it('auto-expands the ancestor groups of EVERY selected node, not just the primary', () => {
    // Two collapsed top-level groups, each holding one instance of the same
    // definition. Selecting both instances (the "(N instances)" click) must
    // reveal both rows — walking only selectedIds[0]'s ancestor chain leaves
    // the second group collapsed and its instance invisible.
    const scene = makeScene({
      top_level_nodes: () => [
        { kind: 'group', id: 20n },
        { kind: 'group', id: 21n },
      ],
      group_members: (id: bigint) =>
        id === 20n
          ? [{ kind: 'instance', id: 4n }]
          : id === 21n
            ? [{ kind: 'instance', id: 5n }]
            : [],
      node_parent: (kind: number, id: bigint) => {
        if (kind === 2) return id === 4n ? 20n : 21n
        return undefined // groups are top-level
      },
      instance_name: () => undefined,
      instance_def: () => 9n,
      component_name: () => 'Door',
    })
    render(
      <DocumentTree
        {...docTreeBase}
        scene={scene}
        selectedIds={[
          { kind: 'instance', id: 4n },
          { kind: 'instance', id: 5n },
        ]}
      />,
    )
    // Both instance rows are visible: each group in the union of the
    // selection's ancestor chains auto-expanded.
    expect(screen.getAllByText('Door')).toHaveLength(2)
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

  it('rows use the UI font, not monospace', () => {
    const scene = makeScene({
      top_level_nodes: () => [{ kind: 'object', id: 1n }],
      object_ids: () => new BigUint64Array([1n]),
    })
    render(
      <DocumentTree
        {...docTreeBase}
        scene={scene}
        watertightMap={new Map([[1n, true]])}
      />,
    )
    const row = screen.getByText('Object 1').closest('div') as HTMLElement
    expect(row.style.fontFamily).toBe('var(--font-family-ui)')
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

  it('renders sketches as rows in the same unified list and selects them on click', () => {
    const scene = makeScene({
      top_level_nodes: () => [{ kind: 'object', id: 1n }],
      object_ids: () => new BigUint64Array([1n]),
      sketch_ids: () => new BigUint64Array([5n]),
    })
    const onSelect = vi.fn()
    const { container } = render(
      <DocumentTree
        {...docTreeBase}
        scene={scene}
        watertightMap={new Map([[1n, true]])}
        onSelect={onSelect}
      />,
    )
    // Object and sketch rows share one list container (no section wrappers).
    const objectRow = screen.getByText('Object 1').closest('div')?.parentElement
    const sketchRow = screen.getByText('Sketch 1').closest('div')?.parentElement
    expect(objectRow).toBe(sketchRow)
    expect(container.querySelector('[data-node-icon="sketch"]')).not.toBeNull()
    fireEvent.click(screen.getByText('Sketch 1'))
    // Rows are ISLANDS — the connected-shape unit — carrying their sketch.
    expect(onSelect).toHaveBeenCalledWith(
      { kind: 'sketch-island', id: 105n, sketch: 5n },
      false,
    )
  })

  it('highlights the sketch row selected from the canvas (canvas → tree)', () => {
    const scene = makeScene({
      sketch_ids: () => new BigUint64Array([5n]),
    })
    render(
      <DocumentTree
        {...docTreeBase}
        scene={scene}
        selectedIds={[{ kind: 'sketch-island', id: 105n, sketch: 5n }]}
      />,
    )
    const row = screen.getByText('Sketch 1').closest('div')
    // The primary-selection tint is applied via inline style — assert it isn't
    // the default transparent background a non-selected row would get.
    expect(row).toHaveStyle({ background: 'var(--accent-tint-18)' })
  })

  it('renders distinct type icons: solid object, leaky object, group, instance', () => {
    const scene = makeScene({
      top_level_nodes: () => [
        { kind: 'object', id: 1n },
        { kind: 'object', id: 2n },
        { kind: 'group', id: 3n },
        { kind: 'instance', id: 4n },
      ],
      object_ids: () => new BigUint64Array([1n, 2n]),
      group_ids: () => new BigUint64Array([3n]),
      instance_ids: () => new BigUint64Array([4n]),
    })
    const { container } = render(
      <DocumentTree
        {...docTreeBase}
        scene={scene}
        watertightMap={new Map([[1n, true], [2n, false]])}
      />,
    )
    expect(container.querySelector('[data-node-icon="object-solid"]')).not.toBeNull()
    expect(container.querySelector('[data-node-icon="object-leaky"]')).not.toBeNull()
    expect(container.querySelector('[data-node-icon="group"]')).not.toBeNull()
    expect(container.querySelector('[data-node-icon="instance"]')).not.toBeNull()
    // The leaky state is carried by a dashed outline on the object icon.
    const leaky = container.querySelector('[data-node-icon="object-leaky"] path')
    expect(leaky?.getAttribute('stroke-dasharray')).toBeTruthy()
    const solid = container.querySelector('[data-node-icon="object-solid"] path')
    expect(solid?.getAttribute('stroke-dasharray')).toBeNull()
  })

  it('nested groups start collapsed; clicking the chevron expands them', () => {
    const scene = makeScene({
      top_level_nodes: () => [{ kind: 'group', id: 10n }],
      group_ids: () => new BigUint64Array([10n]),
      object_ids: () => new BigUint64Array([1n]),
      group_members: (id: bigint) => (id === 10n ? [{ kind: 'object', id: 1n }] : []),
    })
    render(
      <DocumentTree
        {...docTreeBase}
        scene={scene}
        watertightMap={new Map([[1n, true]])}
      />,
    )
    // Collapsed by default: the member row is not mounted.
    expect(screen.queryByText('Object 1')).not.toBeInTheDocument()
    fireEvent.click(screen.getByText('▸'))
    expect(screen.getByText('Object 1')).toBeInTheDocument()
  })

  it('auto-expands the group ancestors of the primary selection', () => {
    const scene = makeScene({
      top_level_nodes: () => [{ kind: 'group', id: 10n }],
      group_ids: () => new BigUint64Array([10n]),
      object_ids: () => new BigUint64Array([1n]),
      group_members: (id: bigint) => (id === 10n ? [{ kind: 'object', id: 1n }] : []),
      node_parent: (kind: number, id: bigint) =>
        kind === 0 && id === 1n ? 10n : undefined,
    })
    render(
      <DocumentTree
        {...docTreeBase}
        scene={scene}
        watertightMap={new Map([[1n, true]])}
        selectedIds={[{ kind: 'object', id: 1n }]}
      />,
    )
    // The selected object lives inside the (default-collapsed) group — its
    // ancestor chain is force-expanded so the selection is visible.
    expect(screen.getByText('Object 1')).toBeInTheDocument()
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

  it('renders nothing at all when the scene has no tagged objects', () => {
    const { container } = render(<TagsPanel {...baseTagProps} scene={makeScene()} />)
    expect(container.firstChild).toBeNull()
    expect(screen.queryByText(/no tags found/i)).not.toBeInTheDocument()
    expect(screen.queryByText(/hew_export_tags/i)).not.toBeInTheDocument()
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

  it('rows use the UI font, not monospace', () => {
    const scene = makeScene({
      object_ids: () => new BigUint64Array([1n]),
      node_tags: (_kind: number, id: bigint) => (id === 1n ? ['Walls'] : []),
    })
    render(<TagsPanel {...baseTagProps} scene={scene} />)
    const row = screen.getByText('Walls').closest('div') as HTMLElement
    expect(row.style.fontFamily).toContain('--font-family-ui')
    expect(row.style.fontFamily).not.toBe('monospace')
  })

  it('revealTag re-expands collapsed ancestors and highlights the target row', () => {
    const scene = makeScene({
      object_ids: () => new BigUint64Array([1n]),
      node_tags: (_kind: number, id: bigint) =>
        id === 1n ? ['Structure/Roof'] : [],
    })
    const { rerender } = render(<TagsPanel {...baseTagProps} scene={scene} />)
    // Collapse "Structure" so "Roof" disappears.
    fireEvent.click(screen.getByText('▾'))
    expect(screen.queryByText('Roof')).not.toBeInTheDocument()
    // A palette jump to Structure/Roof pops it back open and flags the row.
    rerender(
      <TagsPanel
        {...baseTagProps}
        scene={scene}
        revealTag={{ key: JSON.stringify(['Structure', 'Roof']), nonce: 1 }}
      />,
    )
    const roof = screen.getByText('Roof')
    expect(roof).toBeInTheDocument()
    const row = roof.closest('div') as HTMLElement
    expect(row.style.background).toContain('accent-tint-18')
    // Sibling/ancestor rows are not highlighted.
    const structureRow = screen.getByText('Structure').closest('div') as HTMLElement
    expect(structureRow.style.background).not.toContain('accent-tint-18')
  })
})

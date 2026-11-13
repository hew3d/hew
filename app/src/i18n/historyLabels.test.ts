import { describe, expect, it } from 'vitest'
import { historyLabel, type HistoryKind } from './historyLabels'

// Every case mirrors render_history_label in crates/kernel/src/document.rs:
// in English the localized rendering must equal the kernel's own label.
const k = (id: string, args: HistoryKind['args'] = {}): HistoryKind => ({ id, args })

describe('historyLabel (English must match the kernel renderer)', () => {
  it('simple kinds', () => {
    expect(historyLabel(k('push-pull'), 'x')).toBe('Push/Pull')
    expect(historyLabel(k('follow-me'), 'x')).toBe('Follow Me')
    expect(historyLabel(k('edit'), 'x')).toBe('Edit')
    expect(historyLabel(k('link-identical-copies'), 'x')).toBe('Link identical copies')
  })

  it('labeled, repeat and bundle', () => {
    expect(historyLabel(k('labeled', { label: 'Table leg' }), 'x')).toBe('Table leg')
    expect(historyLabel(k('repeat', { inner: k('push-pull'), count: 3 }), 'x')).toBe('Push/Pull ×3')
    expect(historyLabel(k('bundle', { inner: k('draw'), more: 2 }), 'x')).toBe('Draw (+2 more)')
  })

  it('counts pluralize', () => {
    expect(historyLabel(k('transform', { verb: 'move', noun: 'object', count: 1 }), 'x')).toBe('Move 1 object')
    expect(historyLabel(k('transform', { verb: 'rotate', noun: 'object', count: 3 }), 'x')).toBe('Rotate 3 objects')
    expect(historyLabel(k('transform', { verb: 'scale', noun: 'item', count: 2 }), 'x')).toBe('Scale 2 items')
    expect(historyLabel(k('transform', { verb: 'move', noun: 'sketch', count: 2 }), 'x')).toBe('Move 2 sketches')
    expect(historyLabel(k('close-openings', { count: 1 }), 'x')).toBe('Close an opening')
    expect(historyLabel(k('close-openings', { count: 4 }), 'x')).toBe('Close 4 openings')
    expect(historyLabel(k('delete-faces', { count: 1 }), 'x')).toBe('Delete 1 face')
    expect(historyLabel(k('replace-material', { count: 5 }), 'x')).toBe('Replace material on 5 faces')
    expect(historyLabel(k('import', { count: 12 }), 'x')).toBe('Import 12 items')
    expect(historyLabel(k('copy-array', { count: 6 }), 'x')).toBe('Copy ×6')
  })

  it('names and renames', () => {
    expect(historyLabel(k('delete-node', { node: 'group', name: 'Walls' }), 'x')).toBe("Delete group 'Walls'")
    expect(historyLabel(k('delete-node', { node: 'object', name: 'Base' }), 'x')).toBe("Delete 'Base'")
    expect(historyLabel(k('delete-node', { node: 'object' }), 'x')).toBe('Delete object')
    expect(historyLabel(k('make-component', { name: 'Leg' }), 'x')).toBe("Make component 'Leg'")
    expect(historyLabel(k('make-component'), 'x')).toBe('Make component')
    expect(historyLabel(k('change-component-axes', { name: 'Leg' }), 'x')).toBe(
      "Change component axes 'Leg'",
    )
    expect(historyLabel(k('rename', { from: 'Box', to: 'Base' }), 'x')).toBe("Rename 'Box' → 'Base'")
    expect(historyLabel(k('rename', { to: 'Base' }), 'x')).toBe("Rename to 'Base'")
    expect(historyLabel(k('rename'), 'x')).toBe('Rename')
    expect(historyLabel(k('clear-name', { from: 'Box' }), 'x')).toBe("Clear name 'Box'")
    expect(historyLabel(k('rename-component', { from: 'A', to: 'B' }), 'x')).toBe("Rename component 'A' → 'B'")
    expect(historyLabel(k('finish-editing-component', { name: 'Leg' }), 'x')).toBe("Finish editing component 'Leg'")
  })

  it('annotations, attributes, rescale', () => {
    expect(historyLabel(k('add-annotation', { what: 'dimension' }), 'x')).toBe('Dimension')
    expect(historyLabel(k('edit-annotation', { what: 'text' }), 'x')).toBe('Edit text')
    expect(historyLabel(k('delete-annotation', { what: 'annotation' }), 'x')).toBe('Delete annotation')
    expect(historyLabel(k('set-attribute', { namespace: 'hew', key: 'color' }), 'x')).toBe('Set attribute hew.color')
    expect(historyLabel(k('rescale', { factor: '2.5' }), 'x')).toBe('Resize model ×2.5')
  })

  it('unknown kinds and missing kinds fall back to the kernel label', () => {
    expect(historyLabel(k('from-the-future'), 'Kernel says')).toBe('Kernel says')
    expect(historyLabel(undefined, 'Kernel says')).toBe('Kernel says')
  })
})

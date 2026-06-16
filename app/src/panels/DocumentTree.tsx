/**
 * DocumentTree — the document outliner ( navigation).
 *
 * A flat list of the document's top-level entities (solid Objects + Sketches)
 * with a breadcrumb for the active editing context. Click a row to select it;
 * double-click an object to enter its context. Nesting/components, rename, and
 * delete are later slices.
 *
 * Reads the entity lists straight from the WASM Scene, re-querying whenever
 * `docRev` bumps (the parent increments it on any document change).
 */

import { useMemo } from 'react'
import type { Scene as WasmScene } from '../wasm/loader'
import { entityLabel, breadcrumb, isDimmed } from './treeModel'

interface Props {
  scene: WasmScene
  /** Bumped by the parent on any document change to trigger a re-query. */
  docRev: number
  /** Per-object watertight state, for the solid/leaky dot. */
  watertightMap: Map<bigint, boolean>
  /** Selected entities (ordered; index 0 = primary). */
  selectedIds: bigint[]
  activeContext: bigint | null
  /** `additive` = shift/ctrl-click (multi-select). */
  onSelect: (id: bigint, additive: boolean) => void
  onEnterContext: (objectId: bigint) => void
  onExitContext: () => void
  /** True when exactly two objects are selected at top level (boolean-ready). */
  canBoolean: boolean
  /** Run a boolean on the two selected objects (0=union,1=subtract,2=intersect). */
  onBoolean: (op: number) => void
}

const ROW_BASE: React.CSSProperties = {
  display: 'flex',
  alignItems: 'center',
  gap: '6px',
  padding: '3px 8px',
  fontSize: '12px',
  fontFamily: 'monospace',
  cursor: 'pointer',
  borderRadius: '3px',
  userSelect: 'none',
}

export function DocumentTree({
  scene,
  docRev,
  watertightMap,
  selectedIds,
  activeContext,
  onSelect,
  onEnterContext,
  onExitContext,
  canBoolean,
  onBoolean,
}: Props) {
  // Re-query the entity lists whenever the document changes.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  const objects = useMemo(() => Array.from(scene.object_ids()), [scene, docRev])
  // eslint-disable-next-line react-hooks/exhaustive-deps
  const sketches = useMemo(() => Array.from(scene.sketch_ids()), [scene, docRev])

  const crumbs = breadcrumb(activeContext, objects)
  const selected = new Set(selectedIds)

  return (
    <aside
      style={{
        width: '240px',
        flexShrink: 0,
        background: '#2a2a2a',
        color: '#ddd',
        borderRadius: '4px',
        padding: '8px',
        overflowY: 'auto',
        display: 'flex',
        flexDirection: 'column',
        gap: '8px',
      }}
    >
      {/* Breadcrumb */}
      <div style={{ display: 'flex', alignItems: 'center', flexWrap: 'wrap', gap: '2px', fontSize: '12px', fontFamily: 'monospace' }}>
        {crumbs.map((c, i) => (
          <span key={i} style={{ display: 'flex', alignItems: 'center', gap: '2px' }}>
            {i > 0 && <span style={{ color: '#777' }}>›</span>}
            {c.contextId === null ? (
              <button
                onClick={onExitContext}
                style={{
                  background: 'none',
                  border: 'none',
                  color: i === crumbs.length - 1 ? '#fff' : '#7aa7e0',
                  cursor: 'pointer',
                  fontFamily: 'monospace',
                  fontSize: '12px',
                  padding: 0,
                }}
              >
                {c.label}
              </button>
            ) : (
              <span style={{ color: '#fff', fontWeight: 'bold' }}>{c.label}</span>
            )}
          </span>
        ))}
      </div>

      {/* Boolean actions — active only with exactly two objects selected. */}
      {canBoolean && (
        <div style={{ display: 'flex', gap: '4px' }}>
          <BoolButton label="Union" title="Combine both (A ∪ B)" onClick={() => onBoolean(0)} />
          <BoolButton label="Subtract" title="First minus second (A − B)" onClick={() => onBoolean(1)} />
          <BoolButton label="Intersect" title="Overlap only (A ∩ B)" onClick={() => onBoolean(2)} />
        </div>
      )}

      <Section title="Objects" empty="(no solids yet)">
        {objects.map((id, index) => {
          const watertight = watertightMap.get(id) ?? true
          return (
            <Row
              key={String(id)}
              label={entityLabel('object', index)}
              selected={selected.has(id)}
              active={id === activeContext}
              dimmed={isDimmed(id, activeContext)}
              onClick={(additive) => onSelect(id, additive)}
              onDoubleClick={() => onEnterContext(id)}
              dot={watertight ? '#1a7a3a' : '#cc3322'}
            />
          )
        })}
      </Section>

      <Section title="Sketches" empty="(no sketches yet)">
        {sketches.map((id, index) => (
          <Row
            key={String(id)}
            label={entityLabel('sketch', index)}
            selected={selected.has(id)}
            active={false}
            dimmed={isDimmed(id, activeContext)}
            onClick={(additive) => onSelect(id, additive)}
          />
        ))}
      </Section>
    </aside>
  )
}

function BoolButton({ label, title, onClick }: { label: string; title: string; onClick: () => void }) {
  return (
    <button
      onClick={onClick}
      title={title}
      style={{
        flex: 1,
        padding: '4px 2px',
        fontSize: '11px',
        fontFamily: 'monospace',
        cursor: 'pointer',
        background: '#46618c',
        color: '#eee',
        border: 'none',
        borderRadius: '3px',
      }}
    >
      {label}
    </button>
  )
}

function Section({ title, empty, children }: { title: string; empty: string; children: React.ReactNode }) {
  const items = Array.isArray(children) ? children : [children]
  const isEmpty = items.flat().filter(Boolean).length === 0
  return (
    <div>
      <div style={{ fontSize: '10px', textTransform: 'uppercase', letterSpacing: '0.06em', color: '#888', marginBottom: '4px' }}>
        {title}
      </div>
      {isEmpty ? (
        <div style={{ fontSize: '11px', color: '#666', fontStyle: 'italic', padding: '2px 8px' }}>{empty}</div>
      ) : (
        children
      )}
    </div>
  )
}

function Row({
  label,
  selected,
  active,
  dimmed,
  dot,
  onClick,
  onDoubleClick,
}: {
  label: string
  selected: boolean
  active: boolean
  dimmed: boolean
  dot?: string
  onClick: (additive: boolean) => void
  onDoubleClick?: () => void
}) {
  const background = active ? '#3a567f' : selected ? '#39507040' : 'transparent'
  return (
    <div
      onClick={(e) => onClick(e.shiftKey || e.ctrlKey || e.metaKey)}
      onDoubleClick={onDoubleClick}
      style={{
        ...ROW_BASE,
        background,
        opacity: dimmed ? 0.5 : 1,
        fontWeight: active ? 'bold' : 'normal',
      }}
    >
      {dot !== undefined && (
        <span style={{ width: '7px', height: '7px', borderRadius: '50%', background: dot, flexShrink: 0 }} />
      )}
      <span>{label}</span>
      {active && <span style={{ marginLeft: 'auto', fontSize: '10px', color: '#cfe0f5' }}>editing</span>}
    </div>
  )
}

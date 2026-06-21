/**
 * TagsPanel — a closeable pane listing the nestable tag tree built from
 * @@HEWTAG@@-encoded node names (see tools/sketchup/hew_export_tags.rb).
 *
 * Each tag row has a show/hide eye toggle that hides every object/instance
 * tagged at or under that path. Hides compose with manual per-node hides
 * (DocumentTree eye toggles) — a node hidden by either stays hidden; unhiding
 * a tag does not un-hide a node that is also manually hidden.
 *
 * If the model has no encoded tags, a friendly empty state is shown explaining
 * the Ruby workflow.
 */

import { useMemo, useState } from 'react'
import type { Scene as WasmScene } from '../wasm/loader'
import { buildTagTree, tagPathKey, type TagTreeNode } from './tagModel'
import { nodeRefFromJs, nodeKindToNumber } from './treeModel'

interface Props {
  scene: WasmScene
  /** Bumped on any document change to trigger a re-query. */
  docRev: number
  /** The set of tag path keys (JSON-serialised path arrays) currently hidden. */
  hiddenTagPaths: Set<string>
  /** Toggle hide/show for a tag (and all its descendants). */
  onToggleTagPath: (path: string[]) => void
  /** Called when the user closes the panel. */
  onClose: () => void
}

const ROW_BASE: React.CSSProperties = {
  display: 'flex',
  alignItems: 'center',
  gap: '6px',
  padding: '3px 8px',
  fontSize: '12px',
  fontFamily: 'monospace',
  cursor: 'default',
  borderRadius: '3px',
  userSelect: 'none',
}

export function TagsPanel({ scene, docRev, hiddenTagPaths, onToggleTagPath, onClose }: Props) {
  // Re-query the scene on every docRev bump.
  const tagTree = useMemo(() => {
    // Collect all nodes (objects, groups, instances) and parse their names.
    const allNodes = [
      ...Array.from(scene.object_ids()).map((id) => ({ kind: 'object' as const, id })),
      ...Array.from(scene.group_ids()).map((id) => ({ kind: 'group' as const, id })),
      ...Array.from(scene.instance_ids()).map((id) => ({ kind: 'instance' as const, id })),
    ]

    const tagged: { node: ReturnType<typeof nodeRefFromJs>; path: string[] }[] = []
    for (const raw of allNodes) {
      const node = nodeRefFromJs(raw)
      const kindNum = nodeKindToNumber(node.kind)
      // node_tags returns each tag path joined with "/"; split to recover segments.
      const rawTags = scene.node_tags(kindNum, node.id)
      for (const rawTag of rawTags) {
        const path = rawTag.split('/').map((s) => s.trim()).filter((s) => s.length > 0)
        if (path.length > 0) {
          tagged.push({ node, path })
        }
      }
    }

    return buildTagTree(tagged)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [scene, docRev])

  return (
    <aside
      style={{
        width: '220px',
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
      {/* Panel header */}
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
        <span style={{ fontWeight: 'bold', fontSize: '12px', color: '#eee' }}>Tags</span>
        <button
          onClick={onClose}
          title="Close panel"
          style={{
            background: 'none',
            border: 'none',
            color: '#888',
            cursor: 'pointer',
            fontSize: '14px',
            lineHeight: 1,
            padding: '0 2px',
          }}
        >
          ×
        </button>
      </div>

      {tagTree.length === 0 ? (
        <EmptyState />
      ) : (
        <div style={{ display: 'flex', flexDirection: 'column', gap: '1px' }}>
          {tagTree.map((node) => (
            <TagRow
              key={tagPathKey(node.path)}
              node={node}
              depth={0}
              hiddenTagPaths={hiddenTagPaths}
              onToggleTagPath={onToggleTagPath}
            />
          ))}
        </div>
      )}
    </aside>
  )
}

// ---------------------------------------------------------------------------
// TagRow — recursive tree row for a tag node
// ---------------------------------------------------------------------------

function TagRow({
  node,
  depth,
  hiddenTagPaths,
  onToggleTagPath,
}: {
  node: TagTreeNode
  depth: number
  hiddenTagPaths: Set<string>
  onToggleTagPath: (path: string[]) => void
}) {
  const [expanded, setExpanded] = useState(true)
  const hasChildren = node.children.length > 0
  const key = tagPathKey(node.path)
  const hidden = hiddenTagPaths.has(key)

  // Compute the count of directly-tagged nodes (not counting descendants).
  const directCount = node.nodes.length

  // A tag row is visually dimmed when it (or an ancestor) is hidden.
  // We check both the exact path and all ancestor paths.
  const isHiddenByAncestorOrSelf = isHiddenByAny(node.path, hiddenTagPaths)

  return (
    <>
      <div
        style={{
          ...ROW_BASE,
          paddingLeft: `${8 + depth * 16}px`,
          paddingRight: '4px',
          background: 'transparent',
        }}
      >
        {/* Expand/collapse button for tag folders with children */}
        {hasChildren ? (
          <button
            onClick={() => setExpanded((e) => !e)}
            style={{
              background: 'none',
              border: 'none',
              color: '#aaa',
              cursor: 'pointer',
              padding: '0 2px',
              fontSize: '10px',
              lineHeight: 1,
            }}
          >
            {expanded ? '▾' : '▸'}
          </button>
        ) : (
          /* Spacer to keep text aligned when no expand button */
          <span style={{ width: '14px', flexShrink: 0 }} />
        )}

        {/* Tag folder/label icon */}
        <span style={{ fontSize: '11px', color: '#b07aaa', flexShrink: 0 }}>⬧</span>

        {/* Tag name */}
        <span
          style={{
            flex: 1,
            color: isHiddenByAncestorOrSelf ? '#555' : '#ccc',
            fontSize: '12px',
          }}
        >
          {node.segment}
        </span>

        {/* Direct node count badge */}
        {directCount > 0 && (
          <span
            style={{
              fontSize: '10px',
              color: '#666',
              minWidth: '16px',
              textAlign: 'right',
              flexShrink: 0,
            }}
          >
            {directCount}
          </span>
        )}

        {/* Eye toggle */}
        <button
          onClick={() => onToggleTagPath(node.path)}
          title={hidden ? 'Show tagged objects' : 'Hide tagged objects'}
          style={{
            background: 'none',
            border: 'none',
            color: hidden ? '#555' : '#888',
            cursor: 'pointer',
            padding: '0 2px',
            fontSize: '11px',
            lineHeight: 1,
            flexShrink: 0,
          }}
        >
          {hidden ? '○' : '●'}
        </button>
      </div>

      {/* Children — shown when expanded */}
      {hasChildren && expanded && node.children.map((child) => (
        <TagRow
          key={tagPathKey(child.path)}
          node={child}
          depth={depth + 1}
          hiddenTagPaths={hiddenTagPaths}
          onToggleTagPath={onToggleTagPath}
        />
      ))}
    </>
  )
}

/**
 * Return true if the given path is itself hidden OR if any ancestor path is
 * hidden (which also covers this tag's descendants in the renderer).
 */
function isHiddenByAny(path: string[], hiddenTagPaths: Set<string>): boolean {
  for (let len = 1; len <= path.length; len++) {
    if (hiddenTagPaths.has(tagPathKey(path.slice(0, len)))) return true
  }
  return false
}

// ---------------------------------------------------------------------------
// EmptyState — shown when no @@HEWTAG@@-encoded tags are present
// ---------------------------------------------------------------------------

function EmptyState() {
  return (
    <div
      style={{
        fontSize: '11px',
        color: '#666',
        fontStyle: 'italic',
        padding: '4px 8px',
        lineHeight: 1.5,
      }}
    >
      No tags found.
      <br />
      Use <code style={{ fontStyle: 'normal', color: '#888' }}>hew_export_tags.rb</code> in
      SketchUp before exporting to COLLADA to encode tags into node names.
    </div>
  )
}

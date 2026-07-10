/**
 * MaterialPalette — document-level material swatch panel.
 *
 * Shows all materials in the document palette as swatches. The selected swatch
 * is the "current material" fed to the Paint tool. Controls:
 *   - "Add color" — opens a color picker + name input → add_material()
 *   - "Add texture" — file input + world-size input → add_texture_material()
 *   - Textured swatches show a small thumbnail of the uploaded image.
 *   - Opacity slider — adjusts the selected swatch's alpha (color or
 *     texture alike) → set_material_alpha(). Live while dragging, but only
 *     commits to the kernel (one undo step) on release.
 *
 * Props:
 *   `scene`        — the WASM scene (for material queries / mutations)
 *   `docRev`       — bumped by the parent on any document change
 *   `currentMaterialId` — currently selected material handle
 *   `onSelectMaterial`  — called when the user picks a different swatch
 *   `onDocumentChanged` — called after a material is added or changed
 *   `onRefreshViewport` — called after an opacity commit to re-tessellate the
 *     viewport (alpha isn't tracked by the doc-change touched lists, since
 *     it's resolved live at render time rather than baked into geometry)
 */

import { useEffect, useRef, useState } from 'react'
import type { Scene as WasmScene } from '../wasm/loader'
import { MATERIAL_SENTINEL } from '../tools/PaintTool'
import { nodeKindToNumber, type NodeRef } from './treeModel'

interface Props {
  scene: WasmScene
  docRev: number
  currentMaterialId: bigint
  onSelectMaterial: (id: bigint) => void
  onDocumentChanged: () => void
  onRefreshViewport: () => void
  /** Currently selected document nodes — used by "Fill selected object". */
  selectedIds?: NodeRef[]
}

const PANEL_STYLE: React.CSSProperties = {
  display: 'flex',
  flexDirection: 'column',
  gap: '6px',
  fontFamily: 'monospace',
  fontSize: '11px',
  color: 'var(--text-secondary, #ccc)',
}

const SWATCH_STYLE: React.CSSProperties = {
  width: '36px',
  height: '36px',
  borderRadius: '3px',
  cursor: 'pointer',
  border: '2px solid transparent',
  flexShrink: 0,
  position: 'relative',
  overflow: 'hidden',
}

const INPUT_STYLE: React.CSSProperties = {
  width: '100%',
  fontSize: '11px',
  fontFamily: 'monospace',
  background: 'var(--surface-input, #444)',
  color: 'var(--text-primary, #eee)',
  border: '1px solid var(--border-strong, #555)',
  borderRadius: '3px',
  padding: '2px 4px',
  boxSizing: 'border-box',
}

const BTN_STYLE: React.CSSProperties = {
  fontSize: '11px',
  fontFamily: 'monospace',
  background: 'var(--surface-input, #444)',
  color: 'var(--text-primary, #eee)',
  border: '1px solid var(--border-strong, #555)',
  borderRadius: '3px',
  padding: '3px 8px',
  cursor: 'pointer',
  width: '100%',
}

/**
 * Alpha (0–255) as a display percentage, clamped so only the exact extremes
 * read as "0%"/"100%" — plain rounding would show e.g. alpha=254 as "100%",
 * misleadingly implying fully opaque.
 */
function alphaToDisplayPercent(alpha: number): number {
  if (alpha <= 0) return 0
  if (alpha >= 255) return 100
  return Math.min(99, Math.max(1, Math.round((alpha / 255) * 100)))
}

export function MaterialPalette({
  scene,
  docRev,
  currentMaterialId,
  onSelectMaterial,
  onDocumentChanged,
  onRefreshViewport,
  selectedIds = [],
}: Props) {
  // Suppress the docRev-triggers-re-render lint — we intentionally use it to
  // re-query material_ids from the WASM scene on each document change.
  void docRev

  const materialIds = Array.from(scene.material_ids())

  // --- Opacity state ---
  // Non-null only mid-drag/mid-keystroke, so the slider tracks the pointer
  // without a kernel round trip on every tick; committed (and cleared) on
  // release so a whole gesture is one undo step, not one per tick.
  const [draggingAlpha, setDraggingAlpha] = useState<number | null>(null)
  const selectedMaterialInfo =
    currentMaterialId === MATERIAL_SENTINEL ? undefined : scene.material_info(currentMaterialId)
  // A newly selected swatch starts from its own alpha, not a stale drag value
  // left over from whatever was selected before.
  useEffect(() => setDraggingAlpha(null), [currentMaterialId])

  // commitAlpha is redefined every render (closes over current state/props),
  // so a ref lets the unmount cleanup below always call the latest version —
  // otherwise an in-progress drag silently loses its value if this panel
  // unmounts (e.g. the Materials tray section collapses) before any of
  // onPointerUp/onKeyUp/onBlur has fired.
  function commitAlpha() {
    if (draggingAlpha === null || selectedMaterialInfo === undefined) return
    scene.set_material_alpha(currentMaterialId, draggingAlpha)
    setDraggingAlpha(null)
    // onRefreshViewport re-tessellates via Viewport's handleSceneRefresh,
    // which itself calls onDocumentChanged — calling it here too would
    // double-fire the doc-change bookkeeping (docRev, dirty-marking) per commit.
    onRefreshViewport()
  }
  const commitAlphaRef = useRef(commitAlpha)
  commitAlphaRef.current = commitAlpha
  useEffect(() => () => commitAlphaRef.current(), [])

  // --- Add color state ---
  const [newColorHex, setNewColorHex] = useState('#4488cc')
  const [newColorName, setNewColorName] = useState('')

  // --- Add texture state ---
  const [texName, setTexName] = useState('')
  const [texWorldW, setTexWorldW] = useState('1.0')
  const [texWorldH, setTexWorldH] = useState('1.0')
  const [texError, setTexError] = useState<string | null>(null)
  const fileInputRef = useRef<HTMLInputElement>(null)
  const [pendingFile, setPendingFile] = useState<File | null>(null)

  function handleAddColor() {
    const name = newColorName.trim() || `Color ${materialIds.length + 1}`
    const hex = newColorHex.replace('#', '')
    const r = parseInt(hex.substring(0, 2), 16)
    const g = parseInt(hex.substring(2, 4), 16)
    const b = parseInt(hex.substring(4, 6), 16)
    const id = scene.add_material(name, r, g, b, 255)
    onSelectMaterial(id)
    onDocumentChanged()
    setNewColorName('')
  }

  async function handleAddTexture() {
    setTexError(null)
    const file = pendingFile
    if (file === null) {
      setTexError('Choose an image file first.')
      return
    }
    const name = texName.trim() || file.name
    const ww = parseFloat(texWorldW)
    const wh = parseFloat(texWorldH)
    if (isNaN(ww) || ww <= 0 || isNaN(wh) || wh <= 0) {
      setTexError('World size must be positive numbers.')
      return
    }
    const format = file.type === 'image/jpeg' ? 1 : 0
    const bytes = new Uint8Array(await file.arrayBuffer())
    try {
      const id = scene.add_texture_material(name, 255, 255, 255, 255, bytes, format, ww, wh)
      onSelectMaterial(id)
      onDocumentChanged()
      setTexName('')
      setPendingFile(null)
      if (fileInputRef.current) fileInputRef.current.value = ''
    } catch (err) {
      setTexError(err instanceof Error ? err.message : String(err))
    }
  }

  // Texture thumbnail cache: object URLs keyed by material id (as string).
  // We hold them in module-level state to avoid re-creating blobs on re-render.
  const thumbCache = getThumbCache()

  /**
   * "Fill selected object" — applies `currentMaterialId` as the base material
   * to every leaf object under each selected node (via node_leaf_objects for
   * groups/instances; for an object node it resolves to itself).
   */
  function handleFillObject() {
    if (selectedIds.length === 0) return
    for (const node of selectedIds) {
      if (node.kind === 'instance' || node.kind === 'sketch') {
        // Instances hold geometry via a component definition; a sketch is a
        // drawn line with no faces and no kernel NodeId. Skip both — only
        // world objects / definition members support set_object_material.
        continue
      }
      const kind = nodeKindToNumber(node.kind)  // 0 = object, 1 = group
      const leafIds = Array.from(scene.node_leaf_objects(kind, node.id))
      for (const objId of leafIds) {
        scene.set_object_material(objId, currentMaterialId)
      }
    }
    onDocumentChanged()
  }

  const canFill =
    selectedIds.length > 0 &&
    selectedIds.some((n) => n.kind === 'object' || n.kind === 'group')

  return (
    <div style={PANEL_STYLE}>
      {/* Default swatch */}
      <div style={{ display: 'flex', alignItems: 'center', gap: '6px' }}>
        <div
          onClick={() => onSelectMaterial(MATERIAL_SENTINEL)}
          title="Default (unpainted)"
          style={{
            ...SWATCH_STYLE,
            background: '#cccccc',
            borderColor: currentMaterialId === MATERIAL_SENTINEL ? 'var(--accent-base)' : 'transparent',
          }}
        />
        <span style={{ color: 'var(--text-tertiary, #aaa)', fontSize: '10px' }}>Default</span>
      </div>

      {/* Fill selected object */}
      <button
        style={{
          ...BTN_STYLE,
          opacity: canFill ? 1 : 0.4,
          cursor: canFill ? 'pointer' : 'not-allowed',
        }}
        disabled={!canFill}
        onClick={handleFillObject}
        title="Apply current material as base color for selected object(s)"
      >
        Fill selected object
      </button>

      {/* Material swatches */}
      {materialIds.map((id) => {
        const info = scene.material_info(id)
        if (info === undefined) return null
        const hex =
          '#' +
          [info.r(), info.g(), info.b()]
            .map((c) => c.toString(16).padStart(2, '0'))
            .join('')
        const selected = id === currentMaterialId

        // Texture thumbnail
        let thumbUrl: string | undefined = undefined
        if (info.has_texture()) {
          const cacheKey = id.toString()
          if (!thumbCache.has(cacheKey)) {
            const bytes = scene.material_texture_bytes(id)
            if (bytes !== undefined) {
              const mime = info.name().toLowerCase().endsWith('.jpg') ? 'image/jpeg' : 'image/png'
              const blob = new Blob([new Uint8Array(bytes)], { type: mime })
              thumbCache.set(cacheKey, URL.createObjectURL(blob))
            }
          }
          thumbUrl = thumbCache.get(cacheKey)
        }

        return (
          <div
            key={id.toString()}
            style={{ display: 'flex', alignItems: 'center', gap: '6px' }}
          >
            <div
              onClick={() => onSelectMaterial(id)}
              title={info.name()}
              style={{
                ...SWATCH_STYLE,
                background: thumbUrl !== undefined ? `url(${thumbUrl}) center/cover` : hex,
                borderColor: selected ? 'var(--accent-base)' : 'var(--border-strong, #444)',
              }}
            />
            <span
              style={{
                overflow: 'hidden',
                textOverflow: 'ellipsis',
                whiteSpace: 'nowrap',
                flex: 1,
                color: selected ? 'var(--accent-base)' : 'var(--text-secondary, #ccc)',
                cursor: 'pointer',
              }}
              onClick={() => onSelectMaterial(id)}
            >
              {info.name()}
            </span>
          </div>
        )
      })}

      {/* Opacity */}
      <div style={{ borderTop: '1px solid var(--border-hairline, #444)', margin: '2px 0' }} />
      <div style={{ fontWeight: 'bold', color: 'var(--text-tertiary, #aaa)', fontSize: '10px' }}>
        Opacity{selectedMaterialInfo !== undefined ? ` — ${selectedMaterialInfo.name()}` : ''}
      </div>
      <div style={{ display: 'flex', gap: '6px', alignItems: 'center' }}>
        <input
          type="range"
          min="0"
          max="255"
          step="1"
          disabled={selectedMaterialInfo === undefined}
          value={draggingAlpha ?? selectedMaterialInfo?.a() ?? 255}
          onChange={(e) => setDraggingAlpha(Number(e.target.value))}
          onPointerUp={commitAlpha}
          onKeyUp={commitAlpha}
          onBlur={commitAlpha}
          aria-label={
            selectedMaterialInfo !== undefined
              ? `Opacity for ${selectedMaterialInfo.name()}`
              : 'Opacity (select a material swatch first)'
          }
          title={selectedMaterialInfo === undefined ? 'Select a material swatch to adjust its opacity' : undefined}
          style={{
            flex: 1,
            opacity: selectedMaterialInfo === undefined ? 0.4 : 1,
            cursor: selectedMaterialInfo === undefined ? 'not-allowed' : 'pointer',
          }}
        />
        <span style={{ width: '32px', textAlign: 'right', color: 'var(--text-tertiary, #aaa)' }}>
          {alphaToDisplayPercent(draggingAlpha ?? selectedMaterialInfo?.a() ?? 255)}%
        </span>
      </div>

      {/* Divider */}
      <div style={{ borderTop: '1px solid var(--border-hairline, #444)', margin: '2px 0' }} />

      {/* Add color */}
      <div style={{ fontWeight: 'bold', color: 'var(--text-tertiary, #aaa)', fontSize: '10px' }}>Add color</div>
      <div style={{ display: 'flex', gap: '4px', alignItems: 'center' }}>
        <input
          type="color"
          value={newColorHex}
          onChange={(e) => setNewColorHex(e.target.value)}
          style={{ width: '36px', height: '24px', padding: 0, border: 'none', cursor: 'pointer' }}
        />
        <input
          type="text"
          value={newColorName}
          onChange={(e) => setNewColorName(e.target.value)}
          placeholder="Name…"
          style={{ ...INPUT_STYLE, flex: 1 }}
          onKeyDown={(e) => { if (e.key === 'Enter') handleAddColor() }}
        />
      </div>
      <button style={BTN_STYLE} onClick={handleAddColor}>
        + Add color
      </button>

      {/* Add texture */}
      <div style={{ borderTop: '1px solid var(--border-hairline, #444)', margin: '2px 0' }} />
      <div style={{ fontWeight: 'bold', color: 'var(--text-tertiary, #aaa)', fontSize: '10px' }}>Add texture</div>
      <input
        type="text"
        value={texName}
        onChange={(e) => setTexName(e.target.value)}
        placeholder="Name…"
        style={INPUT_STYLE}
      />
      <input
        ref={fileInputRef}
        type="file"
        accept="image/png,image/jpeg"
        onChange={(e) => setPendingFile(e.target.files?.[0] ?? null)}
        style={{ fontSize: '10px', color: 'var(--text-tertiary, #aaa)' }}
      />
      <div style={{ display: 'flex', gap: '4px', alignItems: 'center' }}>
        <span style={{ color: 'var(--text-faint, #888)', flexShrink: 0 }}>W:</span>
        <input
          type="number"
          value={texWorldW}
          min="0.01"
          step="0.1"
          onChange={(e) => setTexWorldW(e.target.value)}
          style={{ ...INPUT_STYLE, width: '60px' }}
        />
        <span style={{ color: 'var(--text-faint, #888)', flexShrink: 0 }}>H:</span>
        <input
          type="number"
          value={texWorldH}
          min="0.01"
          step="0.1"
          onChange={(e) => setTexWorldH(e.target.value)}
          style={{ ...INPUT_STYLE, width: '60px' }}
        />
      </div>
      {pendingFile !== null && (
        <span style={{ color: 'var(--text-tertiary, #aaa)', fontSize: '10px', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
          {pendingFile.name}
        </span>
      )}
      {texError !== null && (
        <span style={{ color: 'var(--status-leaky)', fontSize: '10px' }}>{texError}</span>
      )}
      <button style={BTN_STYLE} onClick={() => { void handleAddTexture() }}>
        + Add texture
      </button>
    </div>
  )
}

// Module-level thumbnail URL cache so object URLs survive React re-renders.
const _thumbCache = new Map<string, string>()
function getThumbCache(): Map<string, string> {
  return _thumbCache
}

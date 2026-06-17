/**
 * MaterialPalette — document-level material swatch panel.
 *
 * Shows all materials in the document palette as swatches. The selected swatch
 * is the "current material" fed to the Paint tool. Controls:
 *   - "Add color" — opens a color picker + name input → add_material()
 *   - "Add texture" — file input + world-size input → add_texture_material()
 *   - Textured swatches show a small thumbnail of the uploaded image.
 *
 * Props:
 *   `scene`        — the WASM scene (for material queries / mutations)
 *   `docRev`       — bumped by the parent on any document change
 *   `currentMaterialId` — currently selected material handle
 *   `onSelectMaterial`  — called when the user picks a different swatch
 *   `onDocumentChanged` — called after a material is added
 */

import { useRef, useState } from 'react'
import type { Scene as WasmScene } from '../wasm/loader'
import { MATERIAL_SENTINEL } from '../tools/PaintTool'
import { nodeKindToNumber, type NodeRef } from './treeModel'

interface Props {
  scene: WasmScene
  docRev: number
  currentMaterialId: bigint
  onSelectMaterial: (id: bigint) => void
  onDocumentChanged: () => void
  /** Currently selected document nodes — used by "Fill selected object". */
  selectedIds?: NodeRef[]
}

const PANEL_STYLE: React.CSSProperties = {
  width: '180px',
  minWidth: '180px',
  background: '#2a2a2a',
  borderRadius: '4px',
  padding: '8px',
  display: 'flex',
  flexDirection: 'column',
  gap: '6px',
  overflowY: 'auto',
  fontFamily: 'monospace',
  fontSize: '11px',
  color: '#ccc',
  flexShrink: 0,
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
  background: '#444',
  color: '#eee',
  border: '1px solid #555',
  borderRadius: '3px',
  padding: '2px 4px',
  boxSizing: 'border-box',
}

const BTN_STYLE: React.CSSProperties = {
  fontSize: '11px',
  fontFamily: 'monospace',
  background: '#444',
  color: '#eee',
  border: '1px solid #555',
  borderRadius: '3px',
  padding: '3px 8px',
  cursor: 'pointer',
  width: '100%',
}

export function MaterialPalette({
  scene,
  docRev,
  currentMaterialId,
  onSelectMaterial,
  onDocumentChanged,
  selectedIds = [],
}: Props) {
  // Suppress the docRev-triggers-re-render lint — we intentionally use it to
  // re-query material_ids from the WASM scene on each document change.
  void docRev

  const materialIds = Array.from(scene.material_ids())

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
      if (node.kind === 'instance') {
        // Instances hold geometry via a component definition; skip — only
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

  const canFill = selectedIds.length > 0 && selectedIds.some((n) => n.kind !== 'instance')

  return (
    <div style={PANEL_STYLE}>
      <div style={{ fontWeight: 'bold', fontSize: '12px', color: '#eee', marginBottom: '2px' }}>
        Materials
      </div>

      {/* Default swatch */}
      <div style={{ display: 'flex', alignItems: 'center', gap: '6px' }}>
        <div
          onClick={() => onSelectMaterial(MATERIAL_SENTINEL)}
          title="Default (unpainted)"
          style={{
            ...SWATCH_STYLE,
            background: '#cccccc',
            borderColor: currentMaterialId === MATERIAL_SENTINEL ? '#ffaa00' : 'transparent',
          }}
        />
        <span style={{ color: '#aaa', fontSize: '10px' }}>Default</span>
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
                borderColor: selected ? '#ffaa00' : '#444',
              }}
            />
            <span
              style={{
                overflow: 'hidden',
                textOverflow: 'ellipsis',
                whiteSpace: 'nowrap',
                flex: 1,
                color: selected ? '#ffaa00' : '#ccc',
                cursor: 'pointer',
              }}
              onClick={() => onSelectMaterial(id)}
            >
              {info.name()}
            </span>
          </div>
        )
      })}

      {/* Divider */}
      <div style={{ borderTop: '1px solid #444', margin: '2px 0' }} />

      {/* Add color */}
      <div style={{ fontWeight: 'bold', color: '#aaa', fontSize: '10px' }}>Add color</div>
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
      <div style={{ borderTop: '1px solid #444', margin: '2px 0' }} />
      <div style={{ fontWeight: 'bold', color: '#aaa', fontSize: '10px' }}>Add texture</div>
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
        style={{ fontSize: '10px', color: '#aaa' }}
      />
      <div style={{ display: 'flex', gap: '4px', alignItems: 'center' }}>
        <span style={{ color: '#888', flexShrink: 0 }}>W:</span>
        <input
          type="number"
          value={texWorldW}
          min="0.01"
          step="0.1"
          onChange={(e) => setTexWorldW(e.target.value)}
          style={{ ...INPUT_STYLE, width: '60px' }}
        />
        <span style={{ color: '#888', flexShrink: 0 }}>H:</span>
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
        <span style={{ color: '#aaa', fontSize: '10px', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
          {pendingFile.name}
        </span>
      )}
      {texError !== null && (
        <span style={{ color: '#ff6666', fontSize: '10px' }}>{texError}</span>
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

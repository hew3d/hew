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
 *   `onAlphaCommitted` — called after an opacity commit so the viewport can
 *     apply the new alpha to its already-built materials in place (alpha is
 *     live render state, resolved from the palette at render time rather
 *     than baked into geometry — no re-tessellation needed or wanted)
 */

import { useEffect, useRef, useState } from 'react'
import type { Scene as WasmScene } from '../wasm/loader'
import { MATERIAL_SENTINEL } from '../tools/PaintTool'
import { libraryStore } from '../io/libraryStore'

interface Props {
  scene: WasmScene
  docRev: number
  /** Document-load generation from App — a change means a NEW document, so
   *  the handle-keyed thumbnail cache (whose keys collide across loads) must
   *  be revoked and rebuilt. Optional (defaults 0) for tests that don't
   *  exercise cross-load behavior. */
  docGeneration?: number
  currentMaterialId: bigint
  /** The user picked a swatch — make it current AND pick up the Paint tool. */
  onSelectMaterial: (id: bigint) => void
  /** A material was just added — make it current, but do NOT switch tools (the
   * user may be building a palette, not ready to paint). */
  onMaterialCreated: (id: bigint) => void
  onDocumentChanged: () => void
  onAlphaCommitted: () => void
  /** Save the selected material as a library item (App.tsx owns
   * `extract_material_item` + the write/toast flow — see its doc comment).
   * Omitted entirely on a platform with no library backend, same posture as
   * the ContextualDock's Save to Library verb; this component additionally
   * gates the button on `libraryStore().available()` itself since it has no
   * other signal of platform capability. */
  onSaveToLibrary?: (materialId: bigint) => void
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

/**
 * Toggle header for an in-panel collapsible sub-pane ("Add color" / "Add
 * texture") — the whole row is one <button>, chevron + label, matching the
 * TraySection/TagsPanel collapse idiom used elsewhere in the tray.
 */
function SubPaneHeader({
  label,
  expanded,
  onToggle,
}: {
  label: string
  expanded: boolean
  onToggle: () => void
}) {
  return (
    <button
      type="button"
      onClick={onToggle}
      aria-expanded={expanded}
      style={{
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'space-between',
        width: '100%',
        background: 'none',
        border: 'none',
        padding: 0,
        cursor: 'pointer',
        textAlign: 'left',
        fontWeight: 'bold',
        fontFamily: 'monospace',
        color: 'var(--text-tertiary, #aaa)',
        fontSize: '10px',
      }}
    >
      <span>{label}</span>
      <span aria-hidden="true">{expanded ? '▾' : '▸'}</span>
    </button>
  )
}

export function MaterialPalette({
  scene,
  docRev,
  docGeneration = 0,
  currentMaterialId,
  onSelectMaterial,
  onMaterialCreated,
  onDocumentChanged,
  onSaveToLibrary,
  onAlphaCommitted,
}: Props) {
  // Suppress the docRev-triggers-re-render lint — we intentionally use it to
  // re-query material_ids from the WASM scene on each document change.
  void docRev
  const thumbCacheRef = useRef<Map<string, string>>(new Map())
  const thumbGenRef = useRef(docGeneration)
  // Drop the cache SYNCHRONOUSLY on a document load, before the render below
  // reads it: a fresh document reuses the same material handles for different
  // textures, so a stale entry would otherwise paint the wrong thumbnail on
  // the very first post-load render (an effect-time clear runs too late).
  if (thumbGenRef.current !== docGeneration) {
    for (const url of thumbCacheRef.current.values()) URL.revokeObjectURL(url)
    thumbCacheRef.current.clear()
    thumbGenRef.current = docGeneration
  }
  // Revoke on unmount too, so the URLs never leak for the tab's lifetime
  // (audit q-web-robustness).
  useEffect(() => {
    const cache = thumbCacheRef.current
    return () => {
      for (const url of cache.values()) URL.revokeObjectURL(url)
      cache.clear()
    }
  }, [])

  const materialIds = Array.from(scene.material_ids())

  // --- Filter state ---
  // Live, case-insensitive substring match on material name. Applied at
  // render time against the already-fresh materialIds — filtering never
  // touches the scene and never clears the current selection (the Default
  // row and the opacity slider are unaffected).
  const [filter, setFilter] = useState('')
  const normalizedFilter = filter.toLowerCase()
  const filteredMaterialIds =
    normalizedFilter === ''
      ? materialIds
      : materialIds.filter((id) => {
          const info = scene.material_info(id)
          return info !== undefined && info.name().toLowerCase().includes(normalizedFilter)
        })

  // Scroll the selected swatch into view whenever the selection changes —
  // in particular the Alt-eyedropper's sample→select loop (paint-tool design
  // §1), which can land on a swatch far down a long list with no visible
  // cue that anything happened. `block: 'nearest'` (the same convention
  // DocumentTree's primary-selection scroll and TagsPanel's reveal-tag
  // scroll both use) is a no-op when the row is already fully visible, so an
  // ordinary click on a swatch already in view doesn't yank the scroll
  // position — only an off-screen selection actually scrolls. This effect
  // fires on every mount too (there's always a "current" selection — the
  // Default swatch at minimum, unlike DocumentTree's possibly-empty
  // selection) — TagsPanel's double-optional `?.scrollIntoView?.(...)` call
  // (rather than DocumentTree's bare call) so environments without a real
  // scrollIntoView (jsdom without a polyfill) degrade to a silent no-op
  // instead of throwing.
  //
  // A live filter can hide the row the eyedropper just landed on outright —
  // nothing to scroll to, and the pick would look like it silently did
  // nothing. `pendingScrollRef` bridges the two renders this needs: the
  // dedicated effect below (keyed on `currentMaterialId` alone, so it never
  // fires from the user just typing a filter over an already-selected
  // swatch — that swatch stays hidden without disturbing the selection, see
  // the "filtering out the selected material" spec) arms the flag on every
  // selection change; this deps-less effect runs after every commit,
  // consumes the flag once its target row is actually renderable — clearing
  // the filter first, and waiting for the resulting re-render, if the
  // current filter hides it — and then scrolls.
  const selectedRowRef = useRef<HTMLDivElement | null>(null)
  const pendingScrollRef = useRef(false)
  useEffect(() => {
    pendingScrollRef.current = true
  }, [currentMaterialId])
  useEffect(() => {
    if (!pendingScrollRef.current) return
    if (normalizedFilter !== '' && currentMaterialId !== MATERIAL_SENTINEL) {
      const info = scene.material_info(currentMaterialId)
      const hiddenByFilter = info === undefined || !info.name().toLowerCase().includes(normalizedFilter)
      if (hiddenByFilter) {
        setFilter('')
        return // wait for the filter-cleared re-render; flag stays armed
      }
    }
    pendingScrollRef.current = false
    selectedRowRef.current?.scrollIntoView?.({ block: 'nearest' })
  })

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
    // onAlphaCommitted (Viewport.syncMaterialOpacity) updates the built
    // THREE materials in place and itself calls onDocumentChanged — calling
    // it here too would double-fire the doc-change bookkeeping (docRev,
    // dirty-marking) per commit.
    onAlphaCommitted()
  }
  const commitAlphaRef = useRef(commitAlpha)
  commitAlphaRef.current = commitAlpha
  useEffect(() => () => commitAlphaRef.current(), [])

  // --- Add sub-pane collapse state ---
  // Independent per sub-pane, session-only (no persistence) — both start
  // collapsed so the pane opens compact and the user opts into either flow.
  const [colorOpen, setColorOpen] = useState(false)
  const [textureOpen, setTextureOpen] = useState(false)

  // --- Add color state ---
  // null until the user actually picks a color. The native <input
  // type="color"> still needs a seed value to render (`#4488cc`), but that
  // seed is never treated as "chosen" — the prompt-state overlay below covers
  // it until the first change event.
  const [newColorHex, setNewColorHex] = useState<string | null>(null)
  const [newColorName, setNewColorName] = useState('')
  // Focus target after a successful add — the natural next action is adding
  // another color, so focus returns to the Name field rather than dropping
  // to <body> when the just-clicked "+ Add color" button disables itself.
  const newColorNameInputRef = useRef<HTMLInputElement>(null)
  // Focus target after the filter is cleared via the × button — that button
  // unmounts the instant the filter becomes empty, which would otherwise
  // drop focus to <body>.
  const filterInputRef = useRef<HTMLInputElement>(null)

  // --- Add texture state ---
  const [texName, setTexName] = useState('')
  const [texWorldW, setTexWorldW] = useState('1.0')
  const [texWorldH, setTexWorldH] = useState('1.0')
  const [texError, setTexError] = useState<string | null>(null)
  const fileInputRef = useRef<HTMLInputElement>(null)
  const [pendingFile, setPendingFile] = useState<File | null>(null)

  function handleAddColor() {
    if (newColorHex === null) return // disabled in the UI until a color is chosen
    const name = newColorName.trim() || `Color ${materialIds.length + 1}`
    const hex = newColorHex.replace('#', '')
    const r = parseInt(hex.substring(0, 2), 16)
    const g = parseInt(hex.substring(2, 4), 16)
    const b = parseInt(hex.substring(4, 6), 16)
    const id = scene.add_material(name, r, g, b, 255)
    onMaterialCreated(id)
    onDocumentChanged()
    setNewColorName('')
    setNewColorHex(null)
    // The Name input is still mounted (the sub-pane stays expanded), so this
    // is a plain synchronous focus move — no timeout needed.
    newColorNameInputRef.current?.focus()
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
      onMaterialCreated(id)
      onDocumentChanged()
      setTexName('')
      setPendingFile(null)
      if (fileInputRef.current) fileInputRef.current.value = ''
    } catch (err) {
      setTexError(err instanceof Error ? err.message : String(err))
    }
  }

  // Texture thumbnail cache: object URLs keyed by material handle, held in a
  // mount-scoped ref (not module-global) so it can't outlive the palette and
  // leak, and is dropped wholesale on a document load — see the effect below.
  const thumbCache = thumbCacheRef.current

  return (
    <div style={PANEL_STYLE}>
      {/* Filter */}
      <div style={{ position: 'relative', display: 'flex', alignItems: 'center' }}>
        <span
          aria-hidden="true"
          style={{
            position: 'absolute',
            left: '6px',
            color: 'var(--text-faint, #888)',
            fontSize: '11px',
            pointerEvents: 'none',
          }}
        >
          ⌕
        </span>
        <input
          ref={filterInputRef}
          type="text"
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          placeholder="Filter materials…"
          aria-label="Filter materials"
          style={{
            flex: 1,
            fontSize: '11px',
            fontFamily: 'monospace',
            background: 'var(--surface-input, #444)',
            color: 'var(--text-primary, #eee)',
            border: 'none',
            borderRadius: '3px',
            padding: '3px 20px',
            boxSizing: 'border-box',
          }}
        />
        {filter !== '' && (
          <button
            type="button"
            onClick={() => {
              setFilter('')
              // This button unmounts the instant the filter becomes empty
              // (it only renders while filter !== ''); without this, focus
              // would drop to <body> rather than staying in the filter flow.
              filterInputRef.current?.focus()
            }}
            aria-label="Clear filter"
            style={{
              position: 'absolute',
              right: '4px',
              background: 'none',
              border: 'none',
              color: 'var(--text-faint, #888)',
              cursor: 'pointer',
              fontSize: '13px',
              lineHeight: 1,
              padding: '2px',
            }}
          >
            ×
          </button>
        )}
      </div>

      {/* Default swatch */}
      <div
        ref={currentMaterialId === MATERIAL_SENTINEL ? selectedRowRef : undefined}
        style={{ display: 'flex', alignItems: 'center', gap: '6px' }}
      >
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

      {/* Material swatches */}
      {filteredMaterialIds.map((id) => {
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
            ref={selected ? selectedRowRef : undefined}
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
      {normalizedFilter !== '' && filteredMaterialIds.length === 0 && (
        <div style={{ color: 'var(--text-faint, #888)', fontSize: '10px', padding: '4px 0' }}>
          No materials match
        </div>
      )}

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

      {/* Save to Library — real (non-sentinel) material only, and only
          with a working library backend (desktop, or a browser with
          origin-private storage). */}
      {selectedMaterialInfo !== undefined && onSaveToLibrary !== undefined && libraryStore().available() && (
        <button style={BTN_STYLE} onClick={() => onSaveToLibrary(currentMaterialId)}>
          Save to Library
        </button>
      )}

      {/* Divider */}
      <div style={{ borderTop: '1px solid var(--border-hairline, #444)', margin: '2px 0' }} />

      {/* Add color */}
      <SubPaneHeader label="Add color" expanded={colorOpen} onToggle={() => setColorOpen((v) => !v)} />
      {colorOpen && (
        <>
          <input
            ref={newColorNameInputRef}
            type="text"
            value={newColorName}
            onChange={(e) => setNewColorName(e.target.value)}
            placeholder="Name…"
            style={INPUT_STYLE}
            onKeyDown={(e) => { if (e.key === 'Enter') handleAddColor() }}
          />
          <div style={{ position: 'relative', width: '100%', height: '24px' }}>
            <input
              type="color"
              value={newColorHex ?? '#4488cc'}
              onClick={(e) => {
                // Picking exactly the #4488cc seed on first open fires no
                // input/change event (native same-value dedup + React's value
                // tracker), which would otherwise leave newColorHex stuck at
                // null with no feedback and "+ Add color" permanently
                // disabled. 'click' fires on activation (and on keyboard
                // activation of a native input too), so treat activation
                // itself as "chosen" the first time — onChange still refines
                // it if the user picks something else. Opening then
                // cancelling the OS picker counts the seed as chosen, which
                // is acceptable and predictable next to the alternative.
                if (newColorHex === null) setNewColorHex(e.currentTarget.value)
              }}
              onChange={(e) => setNewColorHex(e.target.value)}
              aria-label="Choose color"
              style={{
                width: '100%',
                height: '24px',
                padding: 0,
                border: '1px solid var(--border-strong, #555)',
                borderRadius: '3px',
                cursor: 'pointer',
                boxSizing: 'border-box',
              }}
            />
            {newColorHex === null && (
              // Un-chosen prompt state: fully covers the native swatch (which
              // would otherwise render the #4488cc seed as if already
              // "chosen") with a neutral, dashed-outline placeholder.
              // pointerEvents: 'none' lets clicks pass through to the native
              // input beneath, which still opens the OS color picker.
              <div
                aria-hidden="true"
                style={{
                  position: 'absolute',
                  inset: 0,
                  display: 'flex',
                  alignItems: 'center',
                  justifyContent: 'center',
                  pointerEvents: 'none',
                  borderRadius: '3px',
                  border: '1px dashed var(--border-strong, #555)',
                  background: 'var(--surface-input, #444)',
                  color: 'var(--text-faint, #888)',
                  fontSize: '10px',
                  fontFamily: 'monospace',
                }}
              >
                Choose color…
              </div>
            )}
          </div>
          <button
            style={{
              ...BTN_STYLE,
              opacity: newColorHex === null ? 0.5 : 1,
              cursor: newColorHex === null ? 'not-allowed' : 'pointer',
            }}
            disabled={newColorHex === null}
            onClick={handleAddColor}
          >
            + Add color
          </button>
        </>
      )}

      {/* Add texture */}
      <div style={{ borderTop: '1px solid var(--border-hairline, #444)', margin: '2px 0' }} />
      <SubPaneHeader label="Add texture" expanded={textureOpen} onToggle={() => setTextureOpen((v) => !v)} />
      {textureOpen && (
        <>
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
        </>
      )}
    </div>
  )
}


/**
 * TextDialog — the "3D Text…" dialog (docs/design/3d-text.md,
 * docs/design/3d-text-fonts.md).
 *
 * Styling follows the StlUnitsDialog / StlExportDialog / RecoveryDialog
 * family — theme tokens with the same dark fallbacks the rest of the token
 * consumers carry. Escape cancels. OK resolves `{ text, font, heightMeters,
 * depthMeters }` and hands off to the caller, which lays out the glyph run
 * and arms the placement tool — this dialog only ever picks parameters, it
 * never touches the kernel.
 *
 * Extruded text only (no flat/zero-depth option, v1 scope per the design
 * doc): a positive depth is enforced before OK is enabled.
 *
 * Font picker: a searchable list composed from whichever providers
 * `fontSources.ts` resolves for this environment (Bundled always; System —
 * desktop enumeration or `queryLocalFonts()` — when available; Loaded
 * files for the session). Rows are grouped by FAMILY, with a style
 * selector when a family has more than one real face — system/user fonts
 * can genuinely offer Bold/Italic now (their own real faces; synthesis is
 * still never done — see docs/design/3d-text.md's Fonts section). Previews
 * render each row in its own face: bundled/user fonts register a
 * `FontFace`; system fonts are already installed, so a plain CSS
 * `font-family` does the job for free.
 */

import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import type { LoadedFont } from '../text/fonts'
import { lastSelectedFont, loadFontEntry, rememberSelectedFont } from '../text/fonts'
import {
  LocalFontPermissionDeniedError,
  loadUserFontFile,
  resolveFontProviders,
  type FontFaceEntry,
  type FontProvider,
  type FontProviderSet,
} from '../text/fontSources'
import { validateGlyphOutlines } from '../text/outlineValidation'
import { formatLength, getLengthUnit, getLengthUnitSuffix, parseLengthToMeters } from '../settings/units'

export interface TextDialogResult {
  text: string
  font: LoadedFont
  heightMeters: number
  depthMeters: number
}

interface TextDialogProps {
  onPlace: (result: TextDialogResult) => void
  onCancel: () => void
}

const OVERLAY_STYLE: React.CSSProperties = {
  position: 'fixed',
  inset: 0,
  background: 'var(--backdrop-dim, rgba(0,0,0,0.6))',
  display: 'flex',
  alignItems: 'center',
  justifyContent: 'center',
  zIndex: 2000,
}

const DIALOG_STYLE: React.CSSProperties = {
  background: 'var(--surface-overlay, #2a2a2a)',
  border: '1px solid var(--border-strong, #4a4a4a)',
  borderRadius: 'var(--radius-control, 6px)',
  boxShadow: 'var(--shadow-palette, 0 8px 32px rgba(0,0,0,0.6))',
  padding: '20px 24px',
  minWidth: '420px',
  maxWidth: '560px',
  fontFamily: 'var(--font-family-ui, system-ui, sans-serif)',
  color: 'var(--text-secondary, #ddd)',
}

const HEADING_STYLE: React.CSSProperties = {
  fontSize: '15px',
  fontWeight: 600,
  color: 'var(--text-primary, #eee)',
  marginBottom: '14px',
}

const FIELD_ROW_STYLE: React.CSSProperties = {
  display: 'flex',
  flexDirection: 'column',
  gap: '4px',
  marginBottom: '14px',
}

const LABEL_STYLE: React.CSSProperties = {
  fontSize: 'var(--font-size-body, 13px)',
  color: 'var(--text-tertiary, #ccc)',
}

const INPUT_STYLE: React.CSSProperties = {
  background: 'var(--surface-input, #333)',
  color: 'var(--text-primary, #eee)',
  border: '1px solid var(--border-strong, #555)',
  borderRadius: 'var(--radius-control, 4px)',
  padding: '6px 8px',
  fontSize: 'var(--font-size-body, 13px)',
  fontFamily: 'var(--font-family-ui, system-ui, sans-serif)',
}

const TWO_COL_STYLE: React.CSSProperties = {
  display: 'flex',
  gap: '12px',
}

const ERROR_STYLE: React.CSSProperties = {
  fontSize: 'var(--font-size-body, 13px)',
  color: 'var(--danger-text, #e07a7a)',
  marginBottom: '10px',
}

const WARNING_STYLE: React.CSSProperties = {
  fontSize: 'var(--font-size-body, 13px)',
  color: 'var(--warning-text, #d9a441)',
  marginBottom: '10px',
}

const NOTE_STYLE: React.CSSProperties = {
  fontSize: 'var(--font-size-small, 12px)',
  color: 'var(--text-tertiary, #999)',
  padding: '6px 8px',
}

const BUTTON_ROW_STYLE: React.CSSProperties = {
  display: 'flex',
  justifyContent: 'flex-end',
  gap: '10px',
  marginTop: '6px',
}

const CANCEL_BUTTON_STYLE: React.CSSProperties = {
  padding: '6px 20px',
  background: 'var(--surface-input, #444)',
  color: 'var(--text-primary, #eee)',
  border: '1px solid var(--border-strong, transparent)',
  borderRadius: 'var(--radius-control, 4px)',
  fontSize: 'var(--font-size-menu-item, 13px)',
  fontFamily: 'var(--font-family-ui, system-ui, sans-serif)',
  cursor: 'pointer',
}

const PLACE_BUTTON_STYLE: React.CSSProperties = {
  padding: '6px 20px',
  background: 'var(--accent-base, #3a5e9e)',
  color: 'var(--accent-text-strong, #fff)',
  border: 'none',
  borderRadius: 'var(--radius-control, 4px)',
  fontSize: 'var(--font-size-menu-item, 13px)',
  fontFamily: 'var(--font-family-ui, system-ui, sans-serif)',
  cursor: 'pointer',
}

const SMALL_BUTTON_STYLE: React.CSSProperties = {
  ...CANCEL_BUTTON_STYLE,
  padding: '4px 10px',
  fontSize: 'var(--font-size-small, 12px)',
  whiteSpace: 'nowrap',
}

const PICKER_STYLE: React.CSSProperties = {
  border: '1px solid var(--border-strong, #555)',
  borderRadius: 'var(--radius-control, 4px)',
  background: 'var(--surface-input, #262626)',
}

const PICKER_LIST_STYLE: React.CSSProperties = {
  maxHeight: '220px',
  overflowY: 'auto',
}

const SECTION_HEADING_STYLE: React.CSSProperties = {
  fontSize: 'var(--font-size-small, 11px)',
  textTransform: 'uppercase',
  letterSpacing: '0.04em',
  color: 'var(--text-tertiary, #888)',
  padding: '6px 8px 2px',
}

const FAMILY_ROW_STYLE: React.CSSProperties = {
  display: 'flex',
  alignItems: 'center',
  justifyContent: 'space-between',
  gap: '8px',
  padding: '6px 8px',
  cursor: 'pointer',
}

const FAMILY_ROW_SELECTED_STYLE: React.CSSProperties = {
  ...FAMILY_ROW_STYLE,
  background: 'var(--accent-base, #3a5e9e)',
  color: 'var(--accent-text-strong, #fff)',
}

const STYLE_SELECT_STYLE: React.CSSProperties = {
  ...INPUT_STYLE,
  padding: '2px 4px',
  fontSize: 'var(--font-size-small, 12px)',
}

/** Height/depth default to a sign-scale 100 mm and 5 mm — a legible
 *  starting point for the maker use case the design doc targets; any typed
 *  length grammar (`parseLengthToMeters`) is accepted, matching the VCB. */
const DEFAULT_HEIGHT_M = 0.1
const DEFAULT_DEPTH_M = 0.005

/** Cap on rendered family rows per section, applied AFTER filtering — a
 *  system can carry several hundred families (docs/design/3d-text-fonts.md's
 *  "Long lists need windowing" requirement); this avoids ever mounting
 *  hundreds of rows at once without pulling in a virtualization dependency. */
const MAX_VISIBLE_FAMILIES = 150

/** Case- and diacritic-insensitive normalization for the filter box. */
function normalizeForFilter(s: string): string {
  return s
    .normalize('NFD')
    .replace(/[\u0300-\u036f]/g, '') // strip combining diacritical marks
    .toLowerCase()
}

/** One family's selectable faces, grouped from a flat `FontFaceEntry[]` —
 *  `faces` is sorted Regular-first, then by weight, so the style selector's
 *  default reads naturally. */
interface FamilyGroup {
  family: string
  faces: FontFaceEntry[]
}

function groupByFamily(entries: readonly FontFaceEntry[]): FamilyGroup[] {
  const byFamily = new Map<string, FontFaceEntry[]>()
  for (const entry of entries) {
    const list = byFamily.get(entry.family)
    if (list) list.push(entry)
    else byFamily.set(entry.family, [entry])
  }
  return Array.from(byFamily.entries()).map(([family, faces]) => ({
    family,
    faces: faces.slice().sort((a, b) => {
      if (a.style === 'Regular' && b.style !== 'Regular') return -1
      if (b.style === 'Regular' && a.style !== 'Regular') return 1
      return (a.weight ?? 400) - (b.weight ?? 400)
    }),
  }))
}

/** Entries whose family or style text matches `query` (already normalized),
 *  case-/diacritic-insensitive; an empty query matches everything. */
function filterEntries(entries: readonly FontFaceEntry[], query: string): FontFaceEntry[] {
  if (query === '') return entries.slice()
  return entries.filter((e) => normalizeForFilter(`${e.family} ${e.style}`).includes(query))
}

/** Merges `incoming` into `prev` by `id`, keeping `prev`'s own order and
 *  appending only entries `prev` doesn't already have. `userEntries` is
 *  written from two independent places that can race in real use — the
 *  mount-time restore of files loaded EARLIER this session
 *  (`providers.user.list()`, itself backed by the same shared
 *  `fontSources.ts` registry `loadUserFontFile` writes into) and
 *  `handleFileChosen`'s append of a file JUST loaded. If the user picks a
 *  file while the mount-time restore is still in flight, that restore can
 *  resolve to a list that ALREADY includes the just-picked file (both read
 *  the same underlying registry), and a plain overwrite/append on either
 *  side would double it — this makes both writers commutative and
 *  idempotent regardless of which lands first. */
function mergeEntriesById(prev: readonly FontFaceEntry[], incoming: readonly FontFaceEntry[]): FontFaceEntry[] {
  const seen = new Set(prev.map((e) => e.id))
  const merged = prev.slice()
  for (const e of incoming) {
    if (!seen.has(e.id)) {
      seen.add(e.id)
      merged.push(e)
    }
  }
  return merged
}

/** Registers a `FontFace` (a bundled asset URL, a `File` to read, or
 *  already-fetched bytes) under a CSS family name unique to `cssFamily` so
 *  picker rows can preview it via a plain inline `fontFamily` style.
 *  EVERYTHING here — including reading a `File`'s bytes, which can throw
 *  synchronously in an environment lacking `Blob.prototype.arrayBuffer` —
 *  is inside the one try/catch, so no call site needs its own wrapping to
 *  stay safe. Failures are swallowed entirely: a broken preview degrades
 *  to the picker's fallback UI font, never a toast; the actual font LOAD
 *  (`loadFontEntry`, used for placement) is what surfaces real parse
 *  errors. */
async function registerPreviewFontFace(cssFamily: string, source: URL | File | ArrayBuffer): Promise<void> {
  try {
    const bytes = source instanceof File ? await source.arrayBuffer() : source
    const face = new FontFace(cssFamily, bytes instanceof URL ? `url(${bytes.href})` : bytes)
    await face.load()
    document.fonts.add(face)
  } catch {
    // Preview-only; see doc comment above.
  }
}

/** Section identity for a family/style row — drives which provider `bytes()`
 *  a selection is resolved through. */
type SectionKind = 'bundled' | 'system' | 'user'

export function TextDialog({ onPlace, onCancel }: TextDialogProps) {
  const [text, setText] = useState('Hew')
  const [heightText, setHeightText] = useState(() => formatLength(DEFAULT_HEIGHT_M))
  const [depthText, setDepthText] = useState(() => formatLength(DEFAULT_DEPTH_M))
  const [error, setError] = useState<string | null>(null)
  const [placing, setPlacing] = useState(false)
  const fileInputRef = useRef<HTMLInputElement>(null)

  const [providers, setProviders] = useState<FontProviderSet | null>(null)
  const [bundledEntries, setBundledEntries] = useState<FontFaceEntry[]>([])
  const [userEntries, setUserEntries] = useState<FontFaceEntry[]>([])
  const [systemEntries, setSystemEntries] = useState<FontFaceEntry[]>([])
  const [systemState, setSystemState] = useState<
    'idle' | 'loading' | 'loaded' | 'empty' | 'denied' | 'unavailable' | 'error'
  >('idle')
  const [systemErrorMessage, setSystemErrorMessage] = useState<string | null>(null)
  const [systemProviderKind, setSystemProviderKind] = useState<FontProvider['kind'] | null>(null)

  const [filter, setFilter] = useState('')
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const [selectedFont, setSelectedFont] = useState<LoadedFont | null>(null)
  const [outlineWarning, setOutlineWarning] = useState<string | null>(null)

  // Resolve the provider set once, then list bundled + (on desktop) system
  // fonts immediately — `list_system_fonts` is a plain Tauri command with
  // no permission gate, unlike `queryLocalFonts()`, so desktop needs no
  // separate user-gesture affordance the way the web provider does below.
  useEffect(() => {
    let cancelled = false
    void resolveFontProviders().then(async (resolved) => {
      if (cancelled) return
      setProviders(resolved)
      setSystemProviderKind(resolved.system?.kind ?? null)
      const bundled = await resolved.bundled.list()
      if (cancelled) return
      setBundledEntries(bundled)
      for (const entry of bundled) {
        const { url } = entry.locator as { url: URL }
        void registerPreviewFontFace(entry.id, url)
      }
      // Restore any files loaded earlier THIS SESSION (`fontSources.ts`'s
      // `userFontEntries` outlives this component — re-opening the dialog
      // must still offer them, not just a freshly-loaded pick).
      const user = await resolved.user.list()
      if (cancelled) return
      setUserEntries((prev) => mergeEntriesById(prev, user))
      for (const entry of user) {
        const { file } = entry.locator as { file: File }
        void registerPreviewFontFace(entry.id, file)
      }
      if (resolved.system?.kind === 'system-tauri') {
        setSystemState('loading')
        try {
          const list = await resolved.system.list()
          if (cancelled) return
          setSystemEntries(list)
          setSystemState(list.length > 0 ? 'loaded' : 'empty')
        } catch (err) {
          if (cancelled) return
          setSystemState('error')
          setSystemErrorMessage(err instanceof Error ? err.message : String(err))
        }
      } else if (resolved.system?.kind === 'system-web') {
        setSystemState('idle') // needs the explicit "Use my system fonts…" gesture below
      } else {
        setSystemState('unavailable')
      }
    })
    return () => {
      cancelled = true
    }
  }, [])

  // Default selection: whatever the session last picked, if it's among the
  // entries currently on offer; otherwise the first bundled font. Re-runs
  // as each section finishes loading, so a remembered system/user pick
  // still wins once its section catches up — but never overrides an
  // explicit selection the user already made in THIS dialog instance.
  const explicitSelectionRef = useRef(false)
  useEffect(() => {
    if (explicitSelectionRef.current) return
    const remembered = lastSelectedFont()
    const allKnown = [...bundledEntries, ...systemEntries, ...userEntries]
    const match = remembered !== null ? allKnown.find((e) => e.id === remembered) : undefined
    const fallback = bundledEntries[0]
    const target = match ?? fallback
    if (target !== undefined && target.id !== selectedId) {
      setSelectedId(target.id)
    }
    // selectedId intentionally excluded — this effect decides the default
    // and must not re-fire just because ITS OWN write changed selectedId.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [bundledEntries, systemEntries, userEntries])

  // Resolve the parsed LoadedFont for whichever entry is selected.
  useEffect(() => {
    if (selectedId === null || providers === null) return
    const allKnown = [...bundledEntries, ...systemEntries, ...userEntries]
    const entry = allKnown.find((e) => e.id === selectedId)
    if (entry === undefined) return
    const provider = providerFor(entry, providers)
    if (provider === null) return
    let cancelled = false
    void loadFontEntry(entry, () => provider.bytes(entry))
      .then((loaded) => {
        if (!cancelled) {
          setSelectedFont(loaded)
          setError(null)
        }
      })
      .catch((err) => {
        if (!cancelled) {
          setError(`Couldn't load the selected font: ${err instanceof Error ? err.message : String(err)}`)
        }
      })
    return () => {
      cancelled = true
    }
  }, [selectedId, providers, bundledEntries, systemEntries, userEntries])

  // Outline-quality warning (never a block — docs/design/3d-text-fonts.md):
  // recompute whenever the resolved font, text, or height changes.
  useEffect(() => {
    if (selectedFont === null) {
      setOutlineWarning(null)
      return
    }
    const heightMeters = parseLengthToMeters(heightText) ?? DEFAULT_HEIGHT_M
    const warnings = validateGlyphOutlines(selectedFont.font, text, heightMeters)
    if (warnings.length === 0) {
      setOutlineWarning(null)
    } else {
      const chars = warnings.map((w) => `"${w.char}"`).join(', ')
      setOutlineWarning(
        `Some glyphs in this font overlap themselves (${chars}) and may not extrude cleanly — you can still place it.`,
      )
    }
  }, [selectedFont, text, heightText])

  const handleKeyDown = useCallback(
    (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault()
        onCancel()
      }
    },
    [onCancel],
  )
  useEffect(() => {
    document.addEventListener('keydown', handleKeyDown)
    return () => document.removeEventListener('keydown', handleKeyDown)
  }, [handleKeyDown])

  const unit = getLengthUnitSuffix(getLengthUnit())
  const heightMeters = parseLengthToMeters(heightText)
  const depthMeters = parseLengthToMeters(depthText)
  const canPlace =
    text.trim() !== '' &&
    selectedFont !== null &&
    heightMeters !== null &&
    heightMeters > 0 &&
    depthMeters !== null &&
    depthMeters > 0 &&
    !placing

  const selectEntry = useCallback((id: string) => {
    explicitSelectionRef.current = true
    setSelectedId(id)
    rememberSelectedFont(id)
  }, [])

  const handleLoadFontClick = () => fileInputRef.current?.click()

  const handleFileChosen = async (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0]
    e.target.value = '' // allow re-picking the same file later
    if (!file) return
    let newEntry: FontFaceEntry
    try {
      const { entry, loaded } = await loadUserFontFile(file)
      newEntry = entry
      setUserEntries((prev) => mergeEntriesById(prev, [entry]))
      setSelectedFont(loaded)
      selectEntry(entry.id)
      setError(null)
    } catch (err) {
      setError(`Couldn't load "${file.name}" as a font: ${err instanceof Error ? err.message : String(err)}`)
      return
    }
    // Preview registration (including reading `file`'s bytes) is entirely
    // self-contained inside `registerPreviewFontFace` — see its doc
    // comment — so it can't affect the real load/select flow above even if
    // the read itself fails.
    void registerPreviewFontFace(newEntry.id, file)
  }

  const handleUseSystemFonts = async () => {
    if (providers?.system === undefined || providers.system === null) return
    setSystemState('loading')
    setSystemErrorMessage(null)
    try {
      const list = await providers.system.list()
      setSystemEntries(list)
      setSystemState(list.length > 0 ? 'loaded' : 'empty')
    } catch (err) {
      if (err instanceof LocalFontPermissionDeniedError) {
        setSystemState('denied')
      } else {
        setSystemState('error')
        setSystemErrorMessage(err instanceof Error ? err.message : String(err))
      }
    }
  }

  const handlePlace = () => {
    if (!canPlace || heightMeters === null || depthMeters === null || selectedFont === null) return
    setPlacing(true)
    setError(null)
    onPlace({ text, font: selectedFont, heightMeters, depthMeters })
  }

  const normalizedFilter = normalizeForFilter(filter)
  const bundledGroups = useMemo(
    () => groupByFamily(filterEntries(bundledEntries, normalizedFilter)),
    [bundledEntries, normalizedFilter],
  )
  const systemGroups = useMemo(
    () => groupByFamily(filterEntries(systemEntries, normalizedFilter)),
    [systemEntries, normalizedFilter],
  )
  const userGroups = useMemo(
    () => groupByFamily(filterEntries(userEntries, normalizedFilter)),
    [userEntries, normalizedFilter],
  )

  const selectedLabel =
    selectedFont !== null
      ? selectedFont.label
      : selectedId !== null
        ? ([...bundledEntries, ...systemEntries, ...userEntries].find((e) => e.id === selectedId) ?? null)?.family
        : null

  return (
    <div style={OVERLAY_STYLE} onClick={onCancel}>
      <div
        style={DIALOG_STYLE}
        onClick={(e) => e.stopPropagation()}
        role="dialog"
        aria-modal="true"
        aria-label="3D Text"
      >
        <div style={HEADING_STYLE}>3D Text</div>

        {error !== null && <div style={ERROR_STYLE}>{error}</div>}
        {error === null && outlineWarning !== null && <div style={WARNING_STYLE}>{outlineWarning}</div>}

        <div style={FIELD_ROW_STYLE}>
          <label style={LABEL_STYLE} htmlFor="text-3d-input">
            Text
          </label>
          <textarea
            id="text-3d-input"
            style={{ ...INPUT_STYLE, minHeight: '64px', resize: 'vertical', fontFamily: 'inherit' }}
            value={text}
            onChange={(e) => setText(e.target.value)}
            autoFocus
          />
        </div>

        <div style={FIELD_ROW_STYLE}>
          <label style={LABEL_STYLE} htmlFor="text-3d-font-filter">
            Font{selectedLabel !== null ? ` — ${selectedLabel}` : ''}
          </label>
          <input
            id="text-3d-font-filter"
            type="text"
            placeholder="Filter fonts…"
            style={INPUT_STYLE}
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
          />
          <div style={PICKER_STYLE}>
            <div style={PICKER_LIST_STYLE} role="listbox" aria-label="Font family">
              <FamilySection
                heading="Bundled"
                groups={bundledGroups}
                section="bundled"
                selectedId={selectedId}
                onSelect={selectEntry}
              />
              {systemProviderKind !== null && (
                <>
                  <div style={SECTION_HEADING_STYLE}>System</div>
                  {systemState === 'idle' && (
                    <div style={{ ...NOTE_STYLE, display: 'flex', alignItems: 'center', gap: '8px' }}>
                      System fonts need your permission to read.
                      <button type="button" style={SMALL_BUTTON_STYLE} onClick={() => void handleUseSystemFonts()}>
                        Use my system fonts…
                      </button>
                    </div>
                  )}
                  {systemState === 'loading' && <div style={NOTE_STYLE}>Loading system fonts…</div>}
                  {systemState === 'empty' && (
                    <div style={{ ...NOTE_STYLE, display: 'flex', alignItems: 'center', gap: '8px' }}>
                      {systemProviderKind === 'system-web'
                        ? 'No system fonts were found — your browser may have blocked access without asking.'
                        : 'No system fonts were found on this system.'}
                      {systemProviderKind === 'system-web' && (
                        <button type="button" style={SMALL_BUTTON_STYLE} onClick={() => void handleUseSystemFonts()}>
                          Ask again…
                        </button>
                      )}
                    </div>
                  )}
                  {systemState === 'denied' && (
                    <div style={{ ...NOTE_STYLE, display: 'flex', alignItems: 'center', gap: '8px' }}>
                      Permission was denied.
                      <button type="button" style={SMALL_BUTTON_STYLE} onClick={() => void handleUseSystemFonts()}>
                        Ask again…
                      </button>
                    </div>
                  )}
                  {systemState === 'error' && (
                    <div style={NOTE_STYLE}>Couldn't list system fonts{systemErrorMessage ? `: ${systemErrorMessage}` : '.'}</div>
                  )}
                  {systemState === 'loaded' && (
                    <FamilySection
                      heading={null}
                      groups={systemGroups}
                      section="system"
                      selectedId={selectedId}
                      onSelect={selectEntry}
                    />
                  )}
                </>
              )}
              {userGroups.length > 0 && (
                <FamilySection
                  heading="Loaded this session"
                  groups={userGroups}
                  section="user"
                  selectedId={selectedId}
                  onSelect={selectEntry}
                />
              )}
            </div>
          </div>
          <div style={{ display: 'flex', justifyContent: 'flex-end' }}>
            <button type="button" style={SMALL_BUTTON_STYLE} onClick={handleLoadFontClick}>
              Load font file…
            </button>
            <input
              ref={fileInputRef}
              type="file"
              accept=".ttf,.otf,font/ttf,font/otf"
              style={{ display: 'none' }}
              onChange={(e) => {
                void handleFileChosen(e)
              }}
            />
          </div>
        </div>

        <div style={TWO_COL_STYLE}>
          <div style={{ ...FIELD_ROW_STYLE, flex: 1 }}>
            <label style={LABEL_STYLE} htmlFor="text-3d-height">
              Height ({unit})
            </label>
            <input
              id="text-3d-height"
              style={INPUT_STYLE}
              value={heightText}
              onChange={(e) => setHeightText(e.target.value)}
            />
          </div>
          <div style={{ ...FIELD_ROW_STYLE, flex: 1 }}>
            <label style={LABEL_STYLE} htmlFor="text-3d-depth">
              Extrusion depth ({unit})
            </label>
            <input
              id="text-3d-depth"
              style={INPUT_STYLE}
              value={depthText}
              onChange={(e) => setDepthText(e.target.value)}
            />
          </div>
        </div>

        <div style={BUTTON_ROW_STYLE}>
          <button style={CANCEL_BUTTON_STYLE} onClick={onCancel}>
            Cancel
          </button>
          <button
            style={{ ...PLACE_BUTTON_STYLE, opacity: canPlace ? 1 : 0.5, cursor: canPlace ? 'pointer' : 'default' }}
            disabled={!canPlace}
            onClick={handlePlace}
          >
            OK
          </button>
        </div>
      </div>
    </div>
  )
}

/** Resolves which `FontProvider` backs `entry`, from its `source`. */
function providerFor(entry: FontFaceEntry, providers: FontProviderSet): FontProvider | null {
  switch (entry.source) {
    case 'bundled':
      return providers.bundled
    case 'system':
      return providers.system
    case 'user':
      return providers.user
  }
}

interface FamilySectionProps {
  heading: string | null
  groups: FamilyGroup[]
  section: SectionKind
  selectedId: string | null
  onSelect: (id: string) => void
}

/** Renders one section's family rows, capped to `MAX_VISIBLE_FAMILIES`
 *  post-filter (docs/design/3d-text-fonts.md's windowing requirement) with
 *  an honest "N more" note rather than silently truncating. */
function FamilySection({ heading, groups, section, selectedId, onSelect }: FamilySectionProps) {
  if (groups.length === 0) return null
  const visible = groups.slice(0, MAX_VISIBLE_FAMILIES)
  const hiddenCount = groups.length - visible.length
  return (
    <div>
      {heading !== null && <div style={SECTION_HEADING_STYLE}>{heading}</div>}
      {visible.map((group) => (
        <FamilyRow key={`${section}-${group.family}`} group={group} selectedId={selectedId} onSelect={onSelect} />
      ))}
      {hiddenCount > 0 && <div style={NOTE_STYLE}>+{hiddenCount} more — refine your search</div>}
    </div>
  )
}

interface FamilyRowProps {
  group: FamilyGroup
  selectedId: string | null
  onSelect: (id: string) => void
}

function FamilyRow({ group, selectedId, onSelect }: FamilyRowProps) {
  const activeFace = group.faces.find((f) => f.id === selectedId) ?? group.faces[0]
  const isSelected = group.faces.some((f) => f.id === selectedId)
  const previewFamily = activeFace.source === 'system' ? activeFace.family : activeFace.id
  return (
    <div
      style={isSelected ? FAMILY_ROW_SELECTED_STYLE : FAMILY_ROW_STYLE}
      role="option"
      aria-selected={isSelected}
      onClick={() => onSelect(activeFace.id)}
    >
      <span
        style={{
          fontFamily: `"${previewFamily}"`,
          fontStyle: activeFace.italic ? 'italic' : 'normal',
          fontWeight: activeFace.weight ?? 400,
          overflow: 'hidden',
          textOverflow: 'ellipsis',
          whiteSpace: 'nowrap',
        }}
      >
        {group.family}
      </span>
      {group.faces.length > 1 && (
        <select
          aria-label={`${group.family} style`}
          value={activeFace.id}
          style={STYLE_SELECT_STYLE}
          onClick={(e) => e.stopPropagation()}
          onChange={(e) => onSelect(e.target.value)}
        >
          {group.faces.map((f) => (
            <option key={f.id} value={f.id}>
              {f.style}
            </option>
          ))}
        </select>
      )}
    </div>
  )
}

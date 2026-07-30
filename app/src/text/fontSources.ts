/**
 * fontSources — font DISCOVERY for 3D Text (docs/design/3d-text-fonts.md).
 *
 * `text/fonts.ts` owns parsing (`loadFontEntry`) and session memory; this
 * module owns finding out what fonts EXIST and how to fetch each one's raw
 * bytes, behind one small `FontProvider` interface shared by four sources:
 *
 *   - bundled       — the shipped OFL families (`fonts.ts`'s
 *                      `BUNDLED_MANIFEST`), always available.
 *   - system-tauri  — desktop: `list_system_fonts`/`read_font_file` Tauri
 *                      commands (`shells/tauri/src-tauri/src/fonts.rs`).
 *                      Reads real installed font files, enumerated
 *                      server-side; see that file's doc comment for why
 *                      `read_font_file` takes an opaque TOKEN and never a
 *                      path.
 *   - system-web    — web: `window.queryLocalFonts()` where the browser
 *                      supports it (Chromium ≥ 103, secure context,
 *                      `local-fonts` permission). Must only be invoked from
 *                      a user gesture (the design's requirement) — this
 *                      module never calls it at load time, only when the
 *                      picker's `list()` call is itself triggered by one.
 *   - user          — the existing "Load font file…" picked-`File` path.
 *
 * The picker composes whichever of `{bundled, system, user}` are on offer
 * (`resolveFontProviders`) — it never needs to know which concrete system
 * provider backs the "System" section beyond the section heading, matching
 * the design doc's "the dialog composes whatever is available" contract.
 */

import { isTauri } from '../io/fileHost'
import { BUNDLED_MANIFEST, familyStyleFromFont, fontDisplayLabel, loadFontEntry, seedFontEntry, type LoadedFont } from './fonts'

// `fonts.ts` imports this module's `FontFaceEntry` TYPE ONLY (erased at
// compile time), so this static value import back into `fonts.ts` is not a
// runtime cycle — only this module ever actually evaluates the other at
// load time.

/** One selectable font FACE, from any provider. `locator` is opaque and
 *  provider-specific (a Tauri token, a `FontData` handle, a bundled asset
 *  URL, a picked `File`) — never a bare path the renderer is trusted to
 *  widen; see `shells/tauri/src-tauri/src/fonts.rs`'s doc comment for why
 *  that distinction is load-bearing for the desktop provider specifically. */
export interface FontFaceEntry {
  /** Stable within a session (see each provider's `list()` for exactly how
   *  it's derived). */
  id: string
  family: string
  /** "Regular", "Bold Italic", … — the face's own style, never synthesized. */
  style: string
  source: 'bundled' | 'system' | 'user'
  weight?: number
  italic?: boolean
  /** Provider-specific handle used to fetch bytes; never a bare path the
   *  renderer is trusted to widen. */
  locator: unknown
}

export interface FontProvider {
  readonly kind: 'bundled' | 'system-tauri' | 'system-web' | 'user'
  available(): Promise<boolean>
  list(): Promise<FontFaceEntry[]>
  bytes(entry: FontFaceEntry): Promise<ArrayBuffer>
}

/** Thrown by the web system provider's `list()` when `queryLocalFonts()`
 *  itself rejects with a permission-denied outcome — distinct from
 *  "unsupported" (`available()` returning `false`, which the picker checks
 *  BEFORE ever calling `list()`) so the dialog can tell the two apart and
 *  render the design's honest note ("system fonts need permission and a
 *  way to ask again") rather than a generic error toast. */
export class LocalFontPermissionDeniedError extends Error {
  constructor() {
    super('Local font access permission was denied.')
    this.name = 'LocalFontPermissionDeniedError'
  }
}

// ─────────────────────────────────────────────────────────────── bundled

let bundledProvider: FontProvider | null = null

/** Lazily builds the bundled provider (deferred so this module never
 *  imports `./fonts`' bundled manifest data eagerly at a time that would
 *  matter — in practice `fonts.ts` is cheap, but this keeps the same
 *  "build providers lazily" shape as the other three, which have real
 *  reasons to defer). */
function getBundledProvider(): FontProvider {
  if (bundledProvider) return bundledProvider
  bundledProvider = {
    kind: 'bundled',
    available: async () => true,
    list: async () =>
      BUNDLED_MANIFEST.map((f) => ({
        id: f.id,
        family: f.label,
        style: 'Regular',
        source: 'bundled' as const,
        locator: { url: f.url },
      })),
    bytes: async (entry) => {
      const { url } = entry.locator as { url: URL }
      const res = await fetch(url)
      if (!res.ok) throw new Error(`failed to fetch bundled font ${entry.family}: HTTP ${res.status}`)
      return res.arrayBuffer()
    },
  }
  return bundledProvider
}

// ────────────────────────────────────────────────────────────────── user

/** User-loaded font entries, kept for the rest of THIS session (module-level
 *  state — cleared only by a page reload, matching "never persisted with
 *  the document"). Insertion order is the display order the picker offers
 *  them in (most-recently-loaded last). Each entry's `locator.file` is the
 *  original `File` — `Blob`/`File` bytes are immutable and re-readable, so
 *  `bytes()` can call `file.arrayBuffer()` again on every selection without
 *  re-prompting or re-picking. */
const userFontEntries: FontFaceEntry[] = []

const userProvider: FontProvider = {
  kind: 'user',
  available: async () => true,
  list: async () => userFontEntries.slice(),
  bytes: async (entry) => {
    const { file } = entry.locator as { file: File }
    return file.arrayBuffer()
  },
}

/**
 * Registers a user-picked `.ttf`/`.otf` file as a new session `FontFaceEntry`
 * — the "Load font file…" flow. Parses the file FIRST (rejecting, same as
 * before this module existed, on anything that doesn't parse as a font —
 * the dialog surfaces this as a toast and nothing is added to the session
 * list on failure) so the registered entry's `family`/`style` reflect the
 * font's own name table (`familyStyleFromFont`) rather than a filename
 * guess, then appends it to `userFontEntries` and returns BOTH the entry
 * and its already-parsed `LoadedFont` (the caller — the picker — needs the
 * parsed font immediately to select/preview it; re-deriving it through
 * `loadFontEntry` would just hit the memo this function itself seeds).
 *
 * `loadFontEntry`'s memoized `LoadedFont.label` is baked from whatever
 * `entry.family`/`style` it's called WITH — correct for bundled/system
 * providers (accurate at `list()` time already) but only a provisional,
 * filename-derived GUESS the first time it's called here, since the font's
 * real family/style aren't knowable until after parsing. Left alone, that
 * wrong label would stay memoized under this entry's `id` forever (past
 * regression: `loaded.label` — what the dialog's selected-font display
 * actually reads — stuck on the filename even after `entry.family`/`style`
 * were corrected below). `seedFontEntry` re-seeds the SAME cache slot with
 * a `LoadedFont` whose label is recomputed from the corrected family/style,
 * so every future read of this `id` (including this call's own returned
 * `loaded`) sees the real one.
 */
export async function loadUserFontFile(file: File): Promise<{ entry: FontFaceEntry; loaded: LoadedFont }> {
  const fallbackFamily = file.name.replace(/\.(ttf|otf)$/i, '')
  const provisionalId = `user-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`
  const provisional: FontFaceEntry = {
    id: provisionalId,
    family: fallbackFamily,
    style: 'Regular',
    source: 'user',
    locator: { file },
  }
  // Parses eagerly (via the shared `loadFontEntry` memo) so a malformed
  // file rejects before ever touching `userFontEntries` — a failed load
  // must never appear in the session list.
  const loadedProvisional = await loadFontEntry(provisional, () => file.arrayBuffer())
  const { family, style } = familyStyleFromFont(loadedProvisional.font, fallbackFamily)
  const entry: FontFaceEntry = { ...provisional, family, style }
  const loaded: LoadedFont = { ...loadedProvisional, label: fontDisplayLabel(family, style) }
  seedFontEntry(entry.id, loaded)
  userFontEntries.push(entry)
  return { entry, loaded }
}

/** Test-only: clears the session's loaded user font entries. */
export function resetUserFontEntriesForTests(): void {
  userFontEntries.length = 0
}

// ─────────────────────────────────────────────────────────── system: tauri

let systemTauriProvider: FontProvider | null = null

/** One face as returned by the `list_system_fonts` Tauri command — mirrors
 *  `shells/tauri/src-tauri/src/fonts.rs`'s `SystemFontFace` (serde
 *  `Serialize`), field-for-field. */
interface RustSystemFontFace {
  token: string
  family: string
  style: string
  weight: number
  italic: boolean
  monospace: boolean
}

function getSystemTauriProvider(): FontProvider {
  if (systemTauriProvider) return systemTauriProvider
  systemTauriProvider = {
    kind: 'system-tauri',
    available: async () => isTauri,
    list: async () => {
      const { invoke } = await import('@tauri-apps/api/core')
      const faces = await invoke<RustSystemFontFace[]>('list_system_fonts')
      return faces.map(
        (f): FontFaceEntry => ({
          id: `sys-${f.token}`,
          family: f.family,
          style: f.style,
          source: 'system',
          weight: f.weight,
          italic: f.italic,
          locator: { token: f.token },
        }),
      )
    },
    bytes: async (entry) => {
      const { invoke } = await import('@tauri-apps/api/core')
      const { token } = entry.locator as { token: string }
      // read_font_file returns a raw IPC response (see main.rs's read_file
      // for the same convention), which resolves to an ArrayBuffer.
      return invoke<ArrayBuffer>('read_font_file', { token })
    },
  }
  return systemTauriProvider
}

// ───────────────────────────────────────────────────────────── system: web

/** The subset of the Local Font Access API's `FontData` this module reads.
 *  Not in TypeScript's DOM lib (an experimental, Chromium-only API as of
 *  this writing) — declared locally rather than globally so it can't leak
 *  into unrelated files. */
interface WebFontData {
  readonly postscriptName: string
  readonly fullName: string
  readonly family: string
  readonly style: string
  blob(): Promise<Blob>
}

interface WindowWithLocalFonts {
  queryLocalFonts?: () => Promise<WebFontData[]>
}

function localFontsWindow(): WindowWithLocalFonts | null {
  return typeof window === 'undefined' ? null : (window as unknown as WindowWithLocalFonts)
}

/** `queryLocalFonts()`'s own returned objects, keyed by the id this
 *  module derives for them — `bytes()` looks the `WebFontData` handle back
 *  up here rather than trying to reconstruct one, since a `FontData`
 *  object (and its `blob()` method) can't be round-tripped through a plain
 *  string locator. Repopulated on every `list()` call (permission can't be
 *  revoked mid-session in practice, but re-querying is cheap and keeps this
 *  simple). */
let webFontDataById: Map<string, WebFontData> | null = null

/** True iff `err` is `queryLocalFonts()`'s permission-denied shape — a
 *  `DOMException` whose `name` is `SecurityError` (the spec's documented
 *  rejection for a denied/revoked permission) or `NotAllowedError`
 *  (Chromium's actual rejection name for a user-denied permission prompt,
 *  seen in practice) — versus any other, genuinely unexpected failure,
 *  which is rethrown as-is rather than misreported as a permission denial. */
function isPermissionDenied(err: unknown): boolean {
  return err instanceof DOMException && (err.name === 'SecurityError' || err.name === 'NotAllowedError')
}

const systemWebProvider: FontProvider = {
  kind: 'system-web',
  available: async () => typeof localFontsWindow()?.queryLocalFonts === 'function',
  list: async () => {
    const w = localFontsWindow()
    if (typeof w?.queryLocalFonts !== 'function') return []
    let faces: WebFontData[]
    try {
      faces = await w.queryLocalFonts()
    } catch (err) {
      if (isPermissionDenied(err)) throw new LocalFontPermissionDeniedError()
      throw err
    }
    const byId = new Map<string, WebFontData>()
    const entries = faces.map((f): FontFaceEntry => {
      const id = `sysweb-${f.postscriptName || `${f.family}-${f.style}`}`
      byId.set(id, f)
      return {
        id,
        family: f.family,
        style: f.style,
        source: 'system',
        italic: /italic|oblique/i.test(f.style),
        locator: { id },
      }
    })
    webFontDataById = byId
    return entries
  },
  bytes: async (entry) => {
    const { id } = entry.locator as { id: string }
    const data = webFontDataById?.get(id)
    if (!data) {
      throw new Error(`system font "${entry.family} ${entry.style}" is no longer available — reopen the picker`)
    }
    const blob = await data.blob()
    return blob.arrayBuffer()
  },
}

// ──────────────────────────────────────────────────────────────── compose

/** The provider set the picker composes sections from — `system` is
 *  whichever ONE of the two system providers applies to this environment
 *  (never both), or `null` when neither is available (a non-Tauri,
 *  non-`queryLocalFonts` web build — the picker then offers Bundled +
 *  Loaded files only, matching the design's fallback). */
export interface FontProviderSet {
  bundled: FontProvider
  system: FontProvider | null
  user: FontProvider
}

/**
 * Resolves which providers the current environment offers. Desktop
 * (`isTauri`) always gets the Tauri system provider — `list_system_fonts`
 * is a plain Tauri command with no user-gesture requirement, unlike
 * `queryLocalFonts()`. On the web, the system provider is offered only when
 * `queryLocalFonts` exists on `window` — its OWN `available()` never calls
 * the gesture-gated API itself, only checks for its presence, so calling
 * this function is always safe to do eagerly (e.g. to decide whether to
 * show a "System" section heading at all) without violating the "only from
 * a user gesture" rule; that rule applies to `system.list()`, which the
 * picker must only call in response to an explicit user action.
 */
export async function resolveFontProviders(): Promise<FontProviderSet> {
  const system = isTauri ? getSystemTauriProvider() : (await systemWebProvider.available()) ? systemWebProvider : null
  return { bundled: getBundledProvider(), system, user: userProvider }
}

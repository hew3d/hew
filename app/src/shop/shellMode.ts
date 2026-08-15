/**
 * shellMode — pure resolution of which top-level shell `main.tsx` renders:
 * the full editor (`App`) or Shop Mode (`ShopApp`, a fullscreen touch-first
 * viewer/inspector for a phone at the workbench — see `ShopApp.tsx`'s own
 * doc comment for what it does and does not do).
 *
 * Kept UI/DOM-free and unit-tested directly (no `matchMedia`/`localStorage`
 * mocking needed in the resolution logic itself) — `main.tsx` is the one
 * place that reads the real `window`/`localStorage` and feeds this pure
 * function its answers.
 */

/** The persisted override `localStorage['hew:shellMode']` holds: `'auto'`
 *  (the default — device-heuristic detection) or an explicit pin set by
 *  either shell's "Use full editor" / "Shop Mode" toggle. */
export type ShellModeOverride = 'auto' | 'editor' | 'shop'

/** localStorage key for the override — read by `main.tsx` at boot, and by
 *  ShopApp's "Use full editor" action before it reloads the page. */
export const SHELL_MODE_OVERRIDE_KEY = 'hew:shellMode'

/**
 * The auto-heuristic's viewport-dimension threshold (CSS px): a coarse
 * pointer alone isn't sufficient signal (a touchscreen laptop is still a
 * desktop-shaped session) — the smaller of width/height must ALSO read as
 * phone-sized. 600px comfortably separates a phone (portrait or landscape)
 * from a tablet, which should keep landing in the full editor by default.
 */
const AUTO_DIMENSION_THRESHOLD_PX = 600

export interface ResolveShellModeInput {
  /** `window.location.hash`, verbatim. `#shop` forces Shop Mode
   *  unconditionally — the one signal that outranks even the desktop-shell
   *  exclusion below, since it is also the E2E hook for testing Shop Mode
   *  from a desktop-shaped Playwright/Chromium session. A bare `#recv=…`
   *  (adversarial-review finding 2: a camera-app scan lands cold on the
   *  canonical origin with exactly this shape, never `#shop&recv=…`) forces
   *  Shop Mode too — below `#shop`/`isTauri` but above the persisted
   *  override, so a phone that once tapped "Use full editor" (or an
   *  atypical device the auto-heuristic misjudges) still reaches Shop
   *  Mode's own `#recv=` handling instead of landing in the full editor,
   *  which has none. */
  hash: string
  /** True under the Tauri desktop shell. The desktop shell NEVER enters
   *  Shop Mode — a mouse-and-keyboard desktop session has no workbench
   *  use case for a touch-only inspector shell, and Shop Mode's whole
   *  reason to exist (offline PWA install, phone-sized chrome) doesn't
   *  apply there either. Checked ahead of every other signal except the
   *  `#shop` E2E hook. */
  isTauri: boolean
  /** The persisted `hew:shellMode` override (`'auto'` if unset/unreadable). */
  override: ShellModeOverride
  /** `matchMedia('(pointer: coarse)').matches` — `platform.isCoarsePointer()`. */
  coarsePointer: boolean
  innerWidth: number
  innerHeight: number
}

/** Resolve which shell to render for this boot. Pure function of its input —
 *  see each field's doc comment on `ResolveShellModeInput` for the precedence. */
export function resolveShellMode(input: ResolveShellModeInput): 'editor' | 'shop' {
  if (input.hash.startsWith('#shop')) return 'shop'
  if (input.isTauri) return 'editor'
  // A bare `#recv=…` (adversarial-review finding 2) forces Shop Mode the
  // same way `#shop` does, just ranked below the Tauri exclusion — the
  // desktop shell is never a legitimate receiver of a handoff meant for a
  // phone. A cheap prefix check, not the full `parseRecvParams` validation:
  // an almost-valid `#recv=` hash should still land IN Shop Mode so its own
  // (more informative) parse failure surfaces there, rather than silently
  // falling through to the editor, which has no `#recv=` handling to fail
  // out of at all.
  if (input.hash.startsWith('#recv=')) return 'shop'
  if (input.override === 'editor') return 'editor'
  if (input.override === 'shop') return 'shop'
  // 'auto': coarse pointer AND a phone-sized viewport.
  const minDimension = Math.min(input.innerWidth, input.innerHeight)
  return input.coarsePointer && minDimension < AUTO_DIMENSION_THRESHOLD_PX ? 'shop' : 'editor'
}

/** Read the persisted override, defaulting to `'auto'` for an unset,
 *  unrecognized, or unreadable (storage disabled/threw) value. Takes
 *  `storage` as a parameter — defaulting to the real `localStorage` — so
 *  tests can pass a stub without touching global state. */
export function readShellModeOverride(storage: Pick<Storage, 'getItem'> | undefined = safeLocalStorage()): ShellModeOverride {
  if (storage === undefined) return 'auto'
  try {
    const value = storage.getItem(SHELL_MODE_OVERRIDE_KEY)
    return value === 'editor' || value === 'shop' ? value : 'auto'
  } catch {
    return 'auto'
  }
}

/** Persist a new override. Same `storage`-injection pattern as
 *  `readShellModeOverride`; best-effort — a storage failure (private
 *  browsing, quota) is swallowed rather than thrown, matching every other
 *  localStorage write in this codebase (settings/units.ts, trayLayout.ts, …). */
export function writeShellModeOverride(
  value: ShellModeOverride,
  storage: Pick<Storage, 'setItem'> | undefined = safeLocalStorage(),
): void {
  if (storage === undefined) return
  try {
    storage.setItem(SHELL_MODE_OVERRIDE_KEY, value)
  } catch {
    /* ignore — best effort, matches every other settings write */
  }
}

/** `localStorage` if present (absent under SSR/non-browser test runners),
 *  else `undefined` — the shared guard both read/write helpers default to. */
function safeLocalStorage(): Storage | undefined {
  return typeof localStorage === 'undefined' ? undefined : localStorage
}

/** The `#recv=…` E2E-encrypted handoff's decoded fields
 *  (`shareCrypto.ts`'s module doc has the exact wire grammar this parses):
 *  `token` identifies the one-shot ciphertext drop on `share.hew3d.com`,
 *  `key` is the still-base64url-encoded AES key (callers pass it through
 *  `fromBase64Url` before handing it to `shareCrypto.decrypt`), and `name`
 *  is the desktop's display name, already percent-DEcoded. */
export interface RecvParams {
  token: string
  key: string
  name: string
}

/** 128 random bits, base64url, no padding (share-relay's own token shape —
 *  see workers/share-relay/README.md's "Token" bullet). */
const TOKEN_RE = /^[A-Za-z0-9_-]{22}$/
/** 256-bit raw AES key, base64url, no padding (`shareCrypto.ts`'s
 *  `toBase64Url` output — 32 bytes is 43 base64url characters). */
const KEY_RE = /^[A-Za-z0-9_-]{43}$/

/**
 * Parses the "Open on Phone" E2E-encrypted handoff off either a full URL
 * (`https://app.hew3d.com/#recv=…` — what a QR decodes to, or what a
 * camera-app scan lands Safari on) or a bare fragment (`#recv=…` — what
 * `window.location.hash` reads, and what `ScanSheet.tsx`'s in-app decoder
 * also hands this after stripping a scanned URL down to just its hash).
 * Both shapes are handled by the same code path here: this only ever looks
 * at the text from the FIRST `#` onward, so a full URL's origin/path prefix
 * is simply ignored rather than validated (the fragment is the only part
 * that ever reaches this parser for a bare-hash caller anyway, and for a
 * scanned URL the origin is whatever the QR said — trusting THAT is no
 * different from trusting the QR's token/key, which this function already
 * must).
 *
 * `token`/`key` are base64url and, per `shareCrypto.ts`'s module doc,
 * NEVER contain a literal `.` — but the urlencoded `name` segment MAY
 * (`encodeURIComponent` doesn't escape `.`), so this splits on only the
 * FIRST TWO `.` characters (token, then key) and treats everything after
 * the second as the (still-urlencoded) name, whole — never re-splitting
 * that segment on its own embedded dots.
 *
 * Also accepts an optional leading `shop&` before `recv=` (`#shop&recv=…`)
 * — the shape a hash that was ALREADY forcing Shop Mode (`#shop`) would
 * carry if something appended a handoff onto it, mirroring the retired
 * LAN-era grammar's identical `#shop&open=…` prefix. `ShopApp.tsx`'s
 * boot-time effect uses this to decide whether to restore `#shop` (this
 * shape) or strip the hash bare (the plain `#recv=…` shape a camera-app
 * scan landing cold on the canonical origin produces) once it's done.
 *
 * Returns `null` for: no `#` at all; a fragment not starting with `recv=`
 * (or `shop&recv=`); fewer than two `.`-separated segments after that (a
 * missing key or name); a token/key that isn't exactly the expected
 * base64url shape (including a truncated or padded one); an empty name
 * segment; or a name segment that isn't valid percent-encoding
 * (`decodeURIComponent` throws). Pure and DOM-free, so it's unit-tested
 * directly like the rest of this file.
 */
export function parseRecvParams(hashOrUrl: string): RecvParams | null {
  const hashStart = hashOrUrl.indexOf('#')
  if (hashStart === -1) return null
  const rawFragment = hashOrUrl.slice(hashStart + 1)
  const fragment = rawFragment.startsWith('shop&') ? rawFragment.slice('shop&'.length) : rawFragment
  if (!fragment.startsWith('recv=')) return null
  const payload = fragment.slice('recv='.length)

  const firstDot = payload.indexOf('.')
  if (firstDot === -1) return null
  const secondDot = payload.indexOf('.', firstDot + 1)
  if (secondDot === -1) return null

  const token = payload.slice(0, firstDot)
  const key = payload.slice(firstDot + 1, secondDot)
  const nameEncoded = payload.slice(secondDot + 1)
  if (!TOKEN_RE.test(token)) return null
  if (!KEY_RE.test(key)) return null
  if (nameEncoded.length === 0) return null

  let name: string
  try {
    name = decodeURIComponent(nameEncoded)
  } catch {
    return null
  }
  if (name.length === 0) return null

  return { token, key, name }
}

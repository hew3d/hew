/**
 * AR Quick Look — Shop Mode's "View in AR" HUD button (docs/design/shop-mode.md
 * §"View in AR (Shop Mode, iOS)"). Two pure, independently-testable pieces:
 *
 *   - `isArQuickLookCandidate()` decides whether the button should exist at
 *     all (iOS Safari only — every other browser gets the USDZ export via
 *     the desktop Export dialog instead, per the design's fallback posture).
 *   - `launchArQuickLook()` does the actual hand-off once ShopApp.tsx has
 *     USDZ bytes in hand (`ViewportApi.exportUsdz()`).
 *
 * Neither touches React or ShopApp state — both are plain DOM/Navigator
 * calls, kept here instead of inline so they're unit-testable under jsdom
 * without mounting the shell.
 */

/**
 * True on iOS/iPadOS Safari — the only environment that recognizes an
 * `<a rel="ar">` hand-off to AR Quick Look. Deliberately narrower than "any
 * WebKit engine on iOS": other iOS browsers (Chrome/Firefox/Edge for iOS)
 * are WebKit under the hood too, App Store policy requires it, but they
 * stamp their own token into the UA string (CriOS/FxiOS/EdgiOS/OPiOS) and
 * don't wire up the system Quick Look hand-off the way Safari itself does,
 * so they're excluded here.
 *
 * `navigator.platform`-based detection (the precedent in `platform.ts`'s
 * `isMac`) isn't enough on its own: iPadOS 13+ masquerades as desktop
 * Safari, reporting `platform === 'MacIntel'` and a `Macintosh` UA token
 * identical to a real Mac. The documented way to tell them apart is touch
 * support — real macOS Safari reports `maxTouchPoints === 0` even with a
 * Magic Trackpad attached, an iPad reports more than one.
 */
export function isArQuickLookCandidate(): boolean {
  if (typeof navigator === 'undefined') return false
  const ua = navigator.userAgent
  const isIPhoneOrIPod = /iPhone|iPod/.test(ua)
  const isIPadUA = /iPad/.test(ua)
  // iPadOS 13+'s macOS-masquerading UA — see the doc comment above.
  const isMasqueradingIPad = /Macintosh/.test(ua) && navigator.maxTouchPoints > 1
  if (!isIPhoneOrIPod && !isIPadUA && !isMasqueradingIPad) return false
  // Other iOS browsers embed "Safari" in their UA string too (WebKit
  // compatibility token), so their own token has to be excluded explicitly
  // rather than relying on /Safari/ alone to rule them out.
  if (/CriOS|FxiOS|EdgiOS|OPiOS/.test(ua)) return false
  // A home-screen web app ("Add to Home Screen", standalone display mode)
  // IS Safari's engine with the full Quick Look hand-off — but its UA
  // drops the trailing "Safari/xxx" token, so the token test alone would
  // hide the button exactly where a workshop user most likely lives.
  // `navigator.standalone` is Safari-only (true in that mode, false in
  // tab Safari, undefined everywhere else), making it the reliable signal.
  if ((navigator as { standalone?: boolean }).standalone === true) return true
  return /Safari/.test(ua)
}

/**
 * Launch AR Quick Look for `bytes` (a USDZ container produced by
 * `ViewportApi.exportUsdz()`), named for a user-facing `.usdz` download.
 *
 * Apple's documented mechanism: wrap the bytes in a Blob URL, point an
 * `<a rel="ar">` at it, and click it programmatically. Two easy-to-miss
 * details this bakes in:
 *
 * - The anchor MUST contain an `<img>` child. Safari's `rel="ar"` handling
 *   sniffs for one before it hands off to Quick Look at all — an anchor
 *   with no children just downloads the Blob like any other link instead
 *   of launching AR. A 1x1 transparent PNG satisfies the sniff without
 *   drawing anything.
 * - The object URL is revoked on a delay, not synchronously after
 *   `click()`. Quick Look's fetch of the Blob URL happens asynchronously
 *   from Safari's point of view (the URL is handed off to the system
 *   viewer rather than read inline before `click()` returns), so revoking
 *   immediately can race that fetch and leave the viewer unable to load
 *   the model. A few seconds is comfortably past the hand-off, and nothing
 *   else holds the URL alive after that.
 */
export function launchArQuickLook(bytes: Uint8Array, name: string): void {
  const blob = new Blob([new Uint8Array(bytes)], { type: 'model/vnd.usdz+zip' })
  const url = URL.createObjectURL(blob)

  const anchor = document.createElement('a')
  anchor.setAttribute('rel', 'ar')
  anchor.href = url
  anchor.download = name.endsWith('.usdz') ? name : name + '.usdz'

  // Required by Safari's rel="ar" sniff — see the doc comment above. No
  // visual footprint; only its presence as a child element matters.
  const img = document.createElement('img')
  img.src =
    'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII='
  img.alt = ''
  anchor.appendChild(img)

  // Unlike webFileHost.ts's plain anchor-downloads (which click an
  // never-attached element), Safari only honors the rel="ar" hand-off for
  // an anchor that's actually in the document at click time — an
  // AR-Quick-Look-specific requirement, not a general anchor-click one.
  document.body.appendChild(anchor)
  anchor.click()
  document.body.removeChild(anchor)

  // Delayed revoke — see the doc comment above for why this can't be
  // synchronous.
  setTimeout(() => URL.revokeObjectURL(url), 5000)
}

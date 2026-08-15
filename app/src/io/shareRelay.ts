/**
 * shareRelay — the one place the "Open on Phone" relay endpoints are
 * spelled out. The desktop dialog (panels/PhoneShareDialog.tsx) PUTs
 * ciphertext to `SHARE_RELAY_BASE`, and Shop Mode's receive path
 * (shop/ShopApp.tsx) GETs it back from the same base; the desktop mints the
 * QR against `RECEIVE_ORIGIN` so a camera-app scan opens the right web app.
 *
 * Both values default to production and can be overridden at BUILD time via
 * Vite env vars, so a self-hosted HTTPS test origin — a homelab
 * `https://hew.granroth.xyz` serving this build behind a real cert — can be
 * exercised end to end without editing (or committing) that origin:
 *
 *   - `VITE_HEW_RECEIVE_ORIGIN` — the origin QR links point at. Set this on
 *     the DESKTOP build so the QR (and the camera-app fallback that opens it
 *     in Safari) lands on the test app instead of app.hew3d.com. The web
 *     build doesn't mint QRs, so it ignores this.
 *   - `VITE_HEW_SHARE_RELAY` — the relay drop endpoint. Leave it at the
 *     default unless the Cloudflare Worker itself is also self-hosted; the
 *     relay is a dumb encrypted pipe and normally stays on Cloudflare even
 *     when the web app is served locally.
 *
 * Whatever origin the web build is served from must ALSO be added to the
 * Worker's CORS allowlist (its `EXTRA_ALLOWED_ORIGINS` var) or the receive
 * GET is blocked — see workers/share-relay/src/handlers.ts. The web shell's
 * own CSP `connect-src` (shells/web/inject-csp.mjs) already admits the relay
 * host; a non-default `VITE_HEW_SHARE_RELAY` host would need adding there too.
 */

/** Base URL of the share-relay Worker's drop store (workers/share-relay). */
export const SHARE_RELAY_BASE =
  import.meta.env.VITE_HEW_SHARE_RELAY ?? 'https://share.hew3d.com/drop'

/** The web-app origin QR links are minted against — scanning with the OS
 *  camera opens this origin in Safari, so the fragment loads even without
 *  the installed app. */
export const RECEIVE_ORIGIN =
  import.meta.env.VITE_HEW_RECEIVE_ORIGIN ?? 'https://app.hew3d.com'

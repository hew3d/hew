/**
 * shareRelay — where the "Open on Phone" relay lives, relative to the origin
 * that serves the Hew web app: always `<origin>/relay/` (docs/design/
 * self-hosting-relay.md §2). That one convention replaces every hard-wired
 * hostname this module used to carry:
 *
 *   - the PHONE (Shop Mode's receive path, `shop/ShopApp.tsx`) fetches the
 *     ciphertext from its OWN origin — `phoneRelayBase()` — so a self-hosted
 *     PWA talks to the self-hosted relay next to it, and the public PWA at
 *     app.hew3d.com talks to the Workers route on app.hew3d.com. No runtime
 *     config file, no env var, no CORS (same origin);
 *   - the DESKTOP (`panels/PhoneShareDialog.tsx`) has one setting, the server
 *     origin (`settings/server.ts`), and the Rust relay client derives the
 *     upload URL from it; this module only derives the QR's receive URL,
 *     `<origin>/#recv=…`, from the same setting.
 *
 * The public deployment follows the same rule: `app.hew3d.com/relay/*` is a
 * Workers route to the share-relay Worker (`share.hew3d.com` keeps routing
 * there too, for older builds — workers/share-relay/README.md).
 */

/** The relay's path under any origin that serves the app. No trailing slash
 *  here — callers append `/drop`, `/drop/<token>`, or `/` (the identity
 *  route, whose trailing slash IS part of the contract). */
export const RELAY_PATH = '/relay'

/** `<origin>/relay` for the given app origin. */
export function relayBaseFor(origin: string): string {
  return `${origin.replace(/\/+$/, '')}${RELAY_PATH}`
}

/** The relay next to THIS page — the phone side's one and only relay. */
export function phoneRelayBase(): string {
  return relayBaseFor(window.location.origin)
}

/** The `#recv=…` receive URL a desktop mints for `appOrigin`. */
export function receiveUrlFor(appOrigin: string, fragment: string): string {
  return `${appOrigin.replace(/\/+$/, '')}/#${fragment}`
}

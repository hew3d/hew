/**
 * ReceiveConfirmSheet — the confirmation gate a LINK-ARRIVED "Open on
 * Phone" handoff must clear before `ShopApp.tsx` ever fetches/decrypts/
 * loads it (adversarial-review finding 1, CRITICAL).
 *
 * The threat this closes: share-relay's `/drop/<token>` PUT is
 * unauthenticated (workers/share-relay/README.md's "Security model" —
 * possession of the token is the only credential the server can check),
 * so anyone can mint a token, encrypt arbitrary bytes under it, and hand a
 * victim a `https://app.hew3d.com/#recv=<forged token>.<key>.<name>` link.
 * Before this fix, `ShopApp.tsx`'s boot-time effect treated a scanned QR
 * and a clicked/pasted link identically — both loaded immediately, no
 * confirmation. A scan is a deliberate physical act (the user pointed
 * their own camera at a code someone showed them) and stays instant; a
 * link can arrive from anywhere (a text, a chat, a malicious page) with no
 * equivalent proof of intent, so it stops here first. See
 * `ShopApp.tsx`'s own `pendingReceive`/`confirmReceive`/`cancelReceive`
 * for how the gate is wired to the two arrival paths.
 *
 * Same visual family as `UnitPicker.tsx`/`ScanSheet.tsx` — portrait bottom
 * sheet, landscape centered 360px card, same scrim/handle/radius/shadow
 * chrome. `name` is untrusted text straight off the URL fragment (the
 * sender chose it, not this app) — rendered as a plain React text child
 * (never `dangerouslySetInnerHTML`, so no HTML/script injection is
 * possible through it either way) and length-capped so a pathologically
 * long name can't stretch the sheet or bloat the DOM.
 */
import type { ShopOrientation } from './orientation'
import { QrIcon } from './icons'

export interface ReceiveConfirmSheetProps {
  /** Non-null renders the sheet; `null` renders nothing — same "open" gate
   *  shape as `ScanSheet`/`UnitPicker`, just keyed off the pending params
   *  instead of a boolean, since the sheet's own body needs the name. */
  name: string | null
  orientation: ShopOrientation
  onCancel: () => void
  onOpen: () => void
}

/** Display-only cap on the untrusted shared name — long enough that a real
 *  document name never visibly truncates, short enough that a hostile
 *  fragment can't turn this into an unbounded string in the DOM. The
 *  fragment parser (`shellMode.ts`'s `parseRecvParams`) itself imposes no
 *  length limit on the name segment, so this is the one place that does. */
const MAX_DISPLAY_NAME_LENGTH = 120

/** Truncates `name` to `MAX_DISPLAY_NAME_LENGTH`, appending an ellipsis
 *  when it does — pure string truncation, not HTML-aware (there is no HTML
 *  to be aware of: `name` is rendered as a plain text child everywhere it
 *  reaches the DOM). */
function truncateForDisplay(name: string): string {
  return name.length > MAX_DISPLAY_NAME_LENGTH
    ? `${name.slice(0, MAX_DISPLAY_NAME_LENGTH)}…`
    : name
}

export function ReceiveConfirmSheet({ name, orientation, onCancel, onOpen }: ReceiveConfirmSheetProps) {
  if (name === null) return null

  const isLandscape = orientation === 'landscape'
  const displayName = truncateForDisplay(name)

  return (
    <>
      <div
        data-testid="shop-receive-confirm-scrim"
        aria-hidden="true"
        onClick={onCancel}
        style={{ position: 'absolute', inset: 0, background: 'rgba(27,26,23,.35)', zIndex: 55 }}
      />
      <div
        role="dialog"
        aria-label="Open shared model?"
        style={
          isLandscape
            ? {
                position: 'absolute', left: '50%', transform: 'translateX(-50%)',
                top: 'max(20px, env(safe-area-inset-top))', bottom: 'max(20px, env(safe-area-inset-bottom))',
                width: '360px', zIndex: 56,
                background: 'var(--surface-sheet)', borderRadius: '18px',
                padding: '12px 10px', boxShadow: '0 18px 48px -14px rgba(27,26,23,.5)',
                overflowY: 'auto', display: 'flex', flexDirection: 'column', justifyContent: 'center',
              }
            : {
                position: 'absolute', left: 0, right: 0, bottom: 0, zIndex: 56,
                background: 'var(--surface-sheet)', borderRadius: '18px 18px 0 0',
                padding: '10px 10px max(20px, calc(env(safe-area-inset-bottom) + 10px))',
                boxShadow: '0 -14px 40px -12px rgba(27,26,23,.5)',
              }
        }
      >
        <div style={{ width: '40px', height: '5px', borderRadius: '3px', background: 'var(--shop-hairline)', margin: '0 auto 10px' }} />
        <div style={{ display: 'flex', alignItems: 'center', gap: '10px', padding: '0 14px 4px' }}>
          <span aria-hidden="true" style={{ display: 'flex', color: 'var(--shop-accent)' }}>
            <QrIcon size={22} />
          </span>
          <span style={{ fontFamily: 'var(--font-family-ui)', fontSize: '17px', fontWeight: 600, color: 'var(--shop-text)' }}>
            Open shared model?
          </span>
        </div>

        <p style={{
          margin: '8px 14px 4px', fontFamily: 'var(--font-family-ui)', fontSize: '14px', lineHeight: 1.5,
          color: 'var(--shop-text-muted)', overflowWrap: 'anywhere',
        }}>
          Someone shared <strong style={{ color: 'var(--shop-text)', fontWeight: 600 }}>&ldquo;{displayName}&rdquo;</strong> with you.
          Only open models from people you trust.
        </p>

        <div style={{ display: 'flex', gap: '8px', padding: '10px 14px 4px' }}>
          <button
            type="button"
            onClick={onCancel}
            style={{
              flex: 1, height: '48px',
              background: 'color-mix(in srgb, var(--shop-text) 7%, transparent)', color: 'var(--shop-text)',
              border: 'none', borderRadius: '13px', cursor: 'pointer',
              fontFamily: 'var(--font-family-ui)', fontSize: '15px', fontWeight: 600,
            }}
          >
            Cancel
          </button>
          <button
            type="button"
            onClick={onOpen}
            style={{
              flex: 1, height: '48px',
              // --shop-accent-fill, not --shop-accent (tokens.css's own doc
              // comment) — cream TEXT sits directly on this fill, which
              // needs the darker, >=4.5:1 terracotta, same reasoning as
              // every other primary-action button in Shop Mode's chrome.
              background: 'var(--shop-accent-fill)', color: 'var(--shop-on-accent)',
              border: 'none', borderRadius: '13px', cursor: 'pointer',
              fontFamily: 'var(--font-family-ui)', fontSize: '15px', fontWeight: 600,
            }}
          >
            Open
          </button>
        </div>
      </div>
    </>
  )
}

# share-relay

A tiny Cloudflare Worker that is the whole server side of Hew's "Open on
Phone" handoff: a one-shot, end-to-end-encrypted ciphertext dead-drop. The
desktop app encrypts a document in the webview, uploads the ciphertext
here, and shows a QR code encoding a URL that carries both the drop's
token and its decryption key. This Worker never sees the key, never sees
plaintext, and forgets the ciphertext the moment it's been read once.

## Why this exists

"Open on Phone" used to work by having the desktop app serve the whole PWA
plus the document over plain LAN HTTP, because an HTTPS-hosted PWA can
never `fetch` from an `http://` LAN address (mixed content, no browser
exception for private ranges). That design is gone. This Worker replaces
it: the desktop uploads encrypted bytes to a normal HTTPS endpoint, and the
QR points at the hosted app itself (`https://app.hew3d.com/#recv=…`), which
downloads and decrypts client-side. No LAN, no local server, no
same-network requirement.

## API surface

This is **the relay contract** — the same one the self-hostable Rust binary
(`crates/hew-relay`) implements, and the black-box conformance suite in
`contract/` checks both against (see "Testing"). Every route also answers
under an optional `/relay` prefix (`/relay/drop`, `/relay/drop/<token>`,
`/relay/`), stripped exactly once: self-hosting serves the relay from the
same origin as the web app under `/relay/`, and the public deployment
routes `app.hew3d.com/relay/*` here alongside the legacy `share.hew3d.com/*`.

| Route | Method | Body | Response |
|---|---|---|---|
| `/` (also `/relay`, `/relay/`) | `GET` | — | `200 {"service":"hew-relay","contract":1,"maxBytes":33554432,"ttlMs":600000,"auth":"none"\|"bearer"}` — the identity route a desktop's *Test connection* hits, `cache-control: no-store` |
| `/drop` | `PUT` | opaque ciphertext, ≤32 MiB | `200 {"token": "…"}` / `413` if too large / `400` if empty / `401 {"error":"unauthorized"}` when an upload key is configured and the `Authorization: Bearer <key>` header is missing or wrong |
| `/drop/<token>` | `GET` | — | `200` ciphertext bytes (one-shot — deleted on read) / `404` if unknown, expired, or already consumed |
| `/drop/<token>` | `HEAD` | — | `200` empty body if a drop is present and unexpired / `404` otherwise — never consumes it (see below) |
| `/drop/<token>` | `DELETE` | — | `204` always (deletes if present; best-effort client-side invalidation) |
| any path | `OPTIONS` | — | CORS preflight (`204` for an allowed origin, `403` otherwise) |

Every other path/method is a `404`. `hew-relay` additionally answers `PUT
/drop` with `503 {"error":"relay full"}` + `Retry-After` when its memory
cap would be exceeded (this Worker has no such cap — Durable Object storage
is the bound, and its free-tier limits fail closed).

- **Token**: 128 random bits, base64url-encoded (22 characters, no
  padding), generated server-side on `PUT`. It's a bearer capability, not a
  guessable id.
- **Expiry**: a drop is treated as expired 10 minutes after upload. Its
  Durable Object (below) arms a self-destruct alarm for that instant when
  the drop is written, so an upload nobody ever fetches cleans itself up
  with no cron, no sweep, and no separate backstop process needed.
- **One-shot, genuinely atomic**: `GET` calls the drop's Durable Object
  `consume()` method, which reads and wipes the drop's storage in one
  request. A Durable Object instance is single-threaded per id, so two
  `GET`s for the same token cannot interleave — whichever request's
  `consume()` runs first empties the storage before the second one's
  `consume()` ever gets to look at it, and the second one deterministically
  gets `null` (404). This replaces an earlier R2-backed design, which could
  only narrow that race (R2 has no atomic get-and-delete), never close it —
  see "Storage" below for the full story.
- **Non-consuming existence check**: `HEAD` calls the drop's `peek()`
  method instead of `consume()` — it applies the same "populated and not
  past the TTL" test but only reads the `meta` row, never wipes storage.
  This is what lets the desktop dialog poll for pickup (has the phone
  fetched it yet?) without racing — or losing to — the phone's own `GET`.
- **CORS**: only `https://app.hew3d.com`, `tauri://localhost`, and
  `https://tauri.localhost` are allowed (the three origins the hosted app
  and the two Tauri webview flavors run under), plus anything in the
  `EXTRA_ALLOWED_ORIGINS` var. No wildcard, ever. A same-origin
  self-hosted phone never needs CORS at all, and the desktop's Rust relay
  client sends no `Origin`.
- **Optional upload key**: set the `HEW_RELAY_UPLOAD_KEY` secret and `PUT`
  requires `Authorization: Bearer <key>` (constant-time compared, never
  logged); `GET`/`HEAD`/`DELETE` stay keyless — the token is the capability
  and the phone never holds the key. Unset in production: the public relay
  is open by design (next section). The identity route reports which mode
  is in effect so a desktop can say "the server rejected the upload key"
  instead of a bare 401.

## Security model — no auth, by design

There is no API key, no signature, nothing to configure. That's deliberate,
and it means this Worker's trust boundary sits entirely on the RECEIVING
side, not here:

- **The token is a bearer capability, not an identity.** `PUT /drop` is
  UNAUTHENTICATED — anyone can upload ciphertext and mint a fresh token for
  it. Whoever holds a token can redeem it; this Worker has no way to tell
  a legitimate desktop's token from one an attacker minted and handed to a
  victim as a forged `#recv=…` link. That is exactly why possessing a
  token is not, on its own, treated as authorization to auto-load
  something on the receiving phone — see the next bullet.
- **Possession of the QR code — the SCAN act itself — is the actual
  authorization for the "scanner" receive path.** Shop Mode's in-app
  scanner (`app/src/shop/ScanSheet.tsx`) loads whatever it decodes
  immediately, no further confirmation: pointing your own camera at a code
  someone showed you is a deliberate physical act with no equivalent for a
  link. A **link-arrived** `#recv=…` (typed, pasted, texted, or — the real
  threat this closes — forged and sent to a victim) has no such act behind
  it, so the phone-side app (`ShopApp.tsx`'s boot-time `#recv=` handling)
  shows an explicit "Open shared model?" confirmation before it ever
  fetches this Worker, naming the (untrusted, sender-chosen) shared name
  and warning to only open models from people you trust. This Worker
  cannot enforce that distinction itself — it has no idea whether a given
  `GET` came from a scan or a click — so the gate lives entirely on the
  receiving app, and this Worker's own job stays exactly what it's always
  been: an untrusted encrypted pipe.
- **The server only ever handles ciphertext.** `PUT`'s body is opaque
  bytes from the desktop's AES-256-GCM encryption
  (`app/src/io/shareCrypto.ts`); this Worker has no way to read it even if
  it wanted to.
- **The decryption key never reaches this Worker at all.** It rides the
  *fragment* (`#recv=…`) of the URL the QR code encodes. Fragments are
  never sent in an HTTP request — not to this Worker, not to
  `app.hew3d.com`'s own server, not to any CDN or proxy in between. Only
  client-side JavaScript on the receiving page ever sees it.
- **A stolen/leaked/forged token is a bounded, self-healing problem**: it's
  only useful during a ≤10-minute window, only until SOME reader consumes
  it (after which it's already gone), and — even if redeemed — decrypting
  a forged drop's ciphertext still needs the matching key from the same
  fragment, and loading the result on a phone still needs either a real
  scan or an explicit "Open" tap past the confirmation gate above.
- **The kernel parser is the last line of defense against hostile bytes.**
  Even a confirmed "Open" only ever hands the decrypted bytes to the same
  `.hew` parser every other open path in the app uses
  (`app/src/io/documentLoad.ts`'s `loadHewBytes`) — malformed or
  adversarially-crafted content is refused the same way a corrupt file
  opened from disk would be, per docs/DEVELOPMENT.md's "no silent geometry
  repair" rule. Nothing about the relay or the receive flow grants a drop's
  bytes any more trust than a file the user picked themselves.

The trust boundary this design assumes, restated: this Worker is an
untrusted encrypted pipe with no authentication of its own. Authorization
happens entirely on the receiving phone, keyed on HOW the handoff arrived
— a scan is self-authorizing, a link is not and must be confirmed — with
the kernel's own parser as the final backstop regardless of which path a
document took to get there.

## Storage: a Durable Object per token, not R2 or KV

Cloudflare KV is eventually consistent across PoPs — a value written in one
data center is not guaranteed to be immediately visible from a `GET` in
another. For a flow where the phone scans the QR and fetches within
seconds of the desktop's upload, that gap could silently lose the very
first real-world use. KV was never a real option for that reason.

An earlier version of this Worker used an R2 bucket instead, which does
give read-after-write consistency. It worked, but R2 is a **live billing
surface**: Class A operations (`put`, `list`, and — critically — the
cron-driven `list`/`delete` sweep this design used to run) bill per-million
past R2's free monthly allowance. A traffic spike, an abuse burst, or
simply more real usage than expected could turn this Worker into a bill,
which contradicts the whole point of a free, disposable side project.

This Worker now stores each drop in its own **Durable Object**
(`ShareDrop`, `src/shareDrop.ts`) — `env.SHARE_DROP.idFromName(token)` maps
every token to a dedicated DO instance, backed by that instance's own
private SQLite database (`new_sqlite_classes` in `wrangler.toml`'s
migration). This closes the billing question structurally rather than by
staying under a limit:

- **SQLite-backed Durable Objects are on the Workers FREE plan** — no paid
  plan is required to use them at all.
- **Every free-tier limit fails CLOSED.** Cloudflare's free allowance for
  Durable Objects is, account-wide and per day/limit: 100,000 DO requests,
  100,000 rows written, 5,000,000 rows read, and 5 GB of total storage.
  Exceeding any of these makes the *operation error* — this Worker's `PUT`
  or `GET` fails with a `500`/`503` and the user sees a failed upload or
  download. It does **not** silently start billing. There is no metered
  overage tier on the free plan to fall into.
- **R2 is gone entirely.** No bucket, no lifecycle rule, no Class A/B
  operation counting to reason about. A Worker with no R2 binding and no
  paid-plan Durable Objects namespace has no code path that can ever
  generate a Cloudflare invoice for this project.

The tradeoff for that guarantee is the 2 MB per-row/BLOB cap and 100 KB
per-statement-text cap SQLite-backed DO storage imposes — which is why a
drop's ciphertext is chunked before it's written (see "32 MB via chunking"
below) rather than stored as a single BLOB.

### 32 MB via chunking

A drop can be up to `MAX_BYTES` (32 MiB), comfortably over the 2 MB
per-row cap. `handlers.ts`'s `handlePutDrop` splits the buffered upload
into `CHUNK_BYTES` (1,900,000 byte) pieces via `chunkBytes` before handing
them to `ShareDrop.store`, which writes each piece as its own row —
`INSERT INTO chunk (idx, data) VALUES (?, ?)`, with the chunk bound as a
parameter, never inlined into the SQL text (a multi-MB BLOB as literal SQL
would blow the 100 KB statement-text cap almost immediately). A full 32 MiB
upload is at most `⌈32 MiB / 1.9 MB⌉ = 17` chunk rows — nowhere near the
100,000-rows-written/day free-tier ceiling for any realistic usage.
`ShareDrop.consume` reads the rows back in `idx` order and concatenates
them before returning, so `handlers.ts` and everything on the client side
sees one contiguous blob, exactly as before.

### Batched RPC: no single call carries a whole drop

Workers RPC caps one call's serialized arguments or return value at
32 MiB — exactly the contract's per-drop maximum, so a maximum-size drop
handed to the DO in one `store(...)` call (or handed back in one
`consume()` return) fails with "Serialized RPC arguments or return values
are limited to 32MiB". The unit suite's fake namespace never serializes, so
it could not see this; the black-box conformance suite against real
`workerd` did. Both directions are therefore batched at
`RPC_BATCH_CHUNKS` (8 chunks ≈ 15 MB per call): the upload rides
`store(name, chunkCount, totalBytes, firstBatch)` then `append(batch)*`,
and a drop is invisible to `GET`/`HEAD` until every declared chunk row is
present; the download rides `consume()` (the one-shot claim, below) then
`take(from, count)*`, each of which deletes what it returns and the last
of which wipes the drop. Chunks are COPIED at the RPC boundary — they are
`subarray` views of the one upload buffer, and structured clone of a view
ships its whole backing buffer.

### One-shot atomicity, for real this time

The old R2-backed `GET` dispatched R2's `get` and `delete` back-to-back to
narrow (not close) a race where two concurrent `GET`s for the same token
could both observe the object before either delete completed. Durable
Objects close that race structurally: a DO instance is single-threaded per
id, so two `consume()` calls against the same token's `ShareDrop` cannot
interleave. Whichever request's `consume()` runs first marks the drop
claimed in one synchronous burst (relative to any other request to that
same DO — this is a runtime guarantee, the input/output gates described in
Cloudflare's Durable Objects documentation); the second one always finds
it claimed and returns `null`. `handleGetDrop` turns that `null` into a
404, indistinguishable from a token that never existed or already expired
— same "no information leak either way" property the old design had, now
on a foundation that's actually atomic instead of merely narrowed.

## Structure

- `src/types.ts` — minimal hand-rolled shims for the Durable Object
  storage/namespace surface this Worker touches, instead of depending on
  `@cloudflare/workers-types`. Keeps the unit suite dependency-free.
- `src/shareDrop.ts` — the `ShareDrop` Durable Object class: `store`,
  `consume`, `peek`, `destroy`, and the `alarm` handler. This is the drop
  store itself; see "Storage" above for the design.
- `src/handlers.ts` — all the HTTP-facing logic (routing, CORS, the three
  `/drop` operations), written as plain functions over a `DropEnv` binding
  interface. No global state, no direct dependency on the `fetch` Worker
  entry point.
- `src/index.ts` — the real Worker entry point; wires `fetch` to
  `handlers.ts` and re-exports `ShareDrop` (wrangler resolves the DO
  binding's `class_name` against this module's exports).
- `src/testSupport/fakeDurableObject.ts` — a fake DO namespace/storage
  stack backed by Node's built-in `node:sqlite`, used only by the tests
  below (not part of the deployed Worker).
- `src/handlers.test.ts`, `src/dropStore.test.ts` — unit tests (see below).
- `contract/relay.contract.test.ts` — the black-box conformance suite, run
  against a live server (this Worker under `wrangler dev`, or `hew-relay`).

## Testing

```sh
cd workers/share-relay
npm test   # node --test src/*.test.ts
```

No install step needed to run the tests: they use Node's built-in test
runner and native TypeScript support (Node ≥23.6; this repo's
`.node-version` pins 26.7.0), and `src/testSupport/fakeDurableObject.ts`
stands in for the real Durable Object binding — no `miniflare`, no
network, no Cloudflare account required. It's backed by `node:sqlite`
(Node's built-in SQLite engine, also dependency-free), so `shareDrop.ts`'s
actual SQL runs for real rather than against a hand-rolled query matcher.
`npm install` is only needed for `wrangler dev`/`wrangler deploy` (below),
which pull in the real `wrangler` CLI.

### Conformance suite (both implementations)

`contract/relay.contract.test.ts` exercises the contract table above over
real HTTP against whatever `HEW_RELAY_URL` points at — one-shot consume,
HEAD peek, DELETE idempotence, token grammar, both size caps, CORS with
allowed/denied/absent origins, prefix and no-prefix routing, the identity
route with and without its slash, the bearer key on and off, plus (server
runs that enable them) TTL expiry and the full-relay 503. Its header
comment lists every `HEW_RELAY_*` knob. Against this Worker:

```sh
npx wrangler dev --local --port 8787 &            # optionally: --var HEW_RELAY_UPLOAD_KEY:<key>
HEW_RELAY_URL=http://127.0.0.1:8787 HEW_RELAY_BUFFERS_BODY=1 npm run test:contract
```

`HEW_RELAY_BUFFERS_BODY=1` skips the two header-only fail-fast cases:
`wrangler dev`'s local proxy buffers request bodies to completion before
the Worker sees them, so a PUT whose declared `Content-Length` is never met
gets no answer there (and the client's eventual disconnect crashes the dev
server). Production Cloudflare needs the flag as well: its edge holds the
Worker's response until the whole request body has arrived, so an
oversized upload gets its 413 only after it has been sent in full (the
desktop never sends one — it reads the cap from the identity route first).
`hew-relay` must pass those cases without the flag. `.github/workflows/ci.yml`'s `relay-contract` job
runs the unit suite, this suite against `wrangler dev`, and this suite
against `hew-relay`, all blocking; `scripts/verify-full.sh` runs the same.

## Deploy checklist (maintainer)

This Worker is not wired into the existing CI/CD — deploying it is a
manual, one-time-per-environment task. There is **no R2 bucket to create
anymore** — the Durable Object namespace is provisioned automatically by
the migration in `wrangler.toml` on first deploy.

> **Already deployed the earlier R2 version?** See "Cleaning up a prior R2
> deploy" at the end of this section — a `wrangler deploy` of this version
> updates the existing Worker in place (swapping the R2 binding and cron
> trigger for the Durable Object), but the orphaned R2 bucket must be
> deleted by hand.

1. **Install dependencies**: `cd workers/share-relay && npm install`
   (pulls in `wrangler`; this directory is intentionally outside the
   pnpm workspace, so it manages its own `node_modules`).
2. **Authenticate**: `npx wrangler login` (or set `CLOUDFLARE_API_TOKEN`).
3. **First deploy**: `npx wrangler deploy`. This registers the Worker,
   applies the `new_sqlite_classes = ["ShareDrop"]` migration (creating the
   Durable Object namespace), and binds `SHARE_DROP` — all from
   `wrangler.toml`, no manual dashboard step. The Worker is not yet
   reachable at a public hostname.
4. **Wire the route**: `wrangler.toml`'s `[[routes]]` block
   (`share.hew3d.com/*` on the `hew3d.com` zone) takes effect on this same
   deploy — it requires `hew3d.com` to already be a Cloudflare-managed
   zone on the same account. If it isn't yet, comment the block out,
   deploy, set up the zone, then uncomment and redeploy.
5. **Confirm the account/Worker is on the Free plan.** SQLite-backed
   Durable Objects don't require a paid plan, and staying on Free is what
   makes every limit fail closed instead of billing (see "Storage" above).
   Workers & Pages → Overview in the dashboard shows the current plan.
6. **Set up the dashboard-only safety nets**: `DASHBOARD-SETUP.md` in this
   directory is the exact click-path checklist (rate limiting rule,
   billing notification tripwire) — do this once, right after the first
   deploy.
7. **Verify**: `curl -i -X PUT --data-binary @somefile
   https://share.hew3d.com/drop -H 'Origin: https://app.hew3d.com'`
   should return `{"token":"…"}`; a follow-up `GET` on
   `https://share.hew3d.com/drop/<token>` should return the same bytes
   once, then `404` on a second try.

No environment variables or secrets are needed for the public relay — see
"Security model" above for why. (`HEW_RELAY_UPLOAD_KEY` is a self-hoster's
knob; do not set it on the production Worker.)

8. **Add the same-origin routes**: alongside `share.hew3d.com/*`, route
   `app.hew3d.com/relay/*` **and** the bare `app.hew3d.com/relay` to this
   Worker (dashboard → the Worker → Settings → Domains & Routes, zone
   `hew3d.com`; the wildcard pattern does not cover the slash-less identity
   URL, which the contract requires) — new desktops and the
   phone app reach the relay as `<origin>/relay/…` (`DASHBOARD-SETUP.md`
   has the checklist, including the rate-limit rule re-key). Both hostnames
   reach the same Worker and the same Durable Object namespace — the DO id
   derives from the token, not the hostname — so old and new clients
   interoperate. Verify on the first request that the Workers route wins
   over the Pages custom domain on the same hostname (Cloudflare documents
   that precedence; confirm rather than trust).

### Cleaning up a prior R2 deploy

If an earlier revision of this Worker (the one that used an R2 bucket
named `hew-share-relay`) was already deployed, do this once when moving
to the Durable Object version. Nothing here is destructive to the
Worker itself — it only removes the now-unused R2 resources.

1. **Redeploy this version**: `npx wrangler deploy`. Because the Worker
   keeps the same `name` (`share-relay`), this **updates it in place** —
   it does not create a second Worker. The deploy drops the old `[[r2_buckets]]`
   binding and the old cron `[triggers]` (neither is in this
   `wrangler.toml` anymore) and applies the `v1` `new_sqlite_classes`
   migration. The R2 version carried no migrations, so `v1` is the first
   migration this Worker has ever seen and applies without conflict.
2. **Delete the orphaned R2 bucket** — `wrangler deploy` never deletes
   account resources, so the bucket lingers (and, empty, still occupies a
   name):
   ```
   npx wrangler r2 bucket delete hew-share-relay
   ```
   R2 refuses to delete a non-empty bucket. Any objects the old Worker
   wrote had a 10-minute TTL, so it is almost certainly empty by now; if
   the delete errors as non-empty, empty it first (R2 → the bucket →
   *Empty bucket* in the dashboard, or `wrangler r2 object delete` per
   key) and retry.
3. **Confirm the cron trigger is gone**: dashboard → Workers & Pages →
   `share-relay` → Settings → Triggers should show no Cron Triggers (the
   redeploy removes the old sweep). Nothing to pay for either way on the
   Free plan, but it should be gone for tidiness.
4. **Leave R2 itself enabled** — an R2 subscription with zero buckets and
   zero usage costs nothing, and there is no "disable R2" step to do.
   Deleting the bucket is the whole cleanup.
5. **Verify**: `npx wrangler r2 bucket list` no longer shows
   `hew-share-relay`, and dashboard → `share-relay` → Settings → Bindings
   shows the `SHARE_DROP` Durable Object binding with no R2 binding
   present.

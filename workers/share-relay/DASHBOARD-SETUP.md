# Dashboard setup (maintainer-only, manual)

None of this is expressible in `wrangler.toml` or code — it's Cloudflare
dashboard configuration that has to be clicked through by hand, once, after
the first deploy (see `README.md`'s deploy checklist, step 6). This is a
checklist, not a tutorial: follow it top to bottom.

Context: `share-relay` is designed so a billing surface can't exist at all
(no R2, SQLite-backed Durable Objects on the Free plan, every free-tier
limit fails closed — see `README.md`'s "Storage" section). Everything below
is belt-and-suspenders on top of that structural guarantee, not a
requirement for the $0 argument to hold.

## (a) Rate limiting rule

The Free plan allows exactly **one** Rate Limiting Rule per zone. Spend it
here.

- [ ] Dashboard → the `hew3d.com` zone → **Security** → **WAF** → **Rate
      limiting rules** → **Create rule**.
- [ ] **Rule name**: `share-relay-put-throttle` (or anything you'll
      recognize later).
- [ ] **If incoming requests match…**
  - Field: **Hostname** — equals `share.hew3d.com`
  - AND Field: **URI Path** — starts with `/drop`
- [ ] **When rate exceeds**: pick a threshold above normal usage but well
      below anything that could stress the DO free tier from one IP — e.g.
      **> 15 requests per 10 seconds**.
- [ ] **Period**: **10 seconds** — this is the only period the Free plan
      offers; it's fixed, not a dropdown choice you're skipping past.
- [ ] **Characteristics**: **IP address** — the only counting key the Free
      plan supports (no "IP + path", no custom characteristics).
- [ ] **Then**: **Block**.
- [ ] **Duration**: whatever the UI defaults to is fine (typically 10
      seconds to 1 minute) — this is a throttle, not a ban list.
- [ ] Save and deploy the rule.
- [ ] Sanity check: hit `/drop` from one machine faster than the threshold
      (e.g. a quick shell loop of `curl`s) and confirm you start getting
      blocked, then stop and confirm it clears.

Known Free-plan constraints (not bugs, just what's available — don't go
looking for a way around these):

- Exactly one rule per zone.
- Counting key is IP address only.
- Period is fixed at 10 seconds.

## (b) Billing/usage notification tripwire

This should never fire — the whole design goal is that nothing here can
ever bill. Set it anyway, as a tripwire in case that assumption is ever
wrong (a Cloudflare pricing change, a plan get bumped by accident, etc.).

- [ ] Dashboard → account **Notifications** (top-level, not inside the
      zone) → **Add**.
- [ ] Notification type: **Billing / Usage** — pick the billing alert type
      the dashboard offers (wording varies by account; look for "spend" or
      "billing threshold").
- [ ] Threshold: the **lowest** amount the dashboard allows — typically
      **$0.01** or the smallest non-zero value offered. Do not accept a
      default that's higher just because it's pre-filled.
- [ ] Delivery: your email (and Slack/webhook too, if you already have one
      wired up for other Cloudflare alerts — not required to set up fresh
      for this).
- [ ] Save.
- [ ] Confirm it shows as **Active** in the notification list.

If this notification EVER fires, treat it as a signal that the structural
$0 guarantee broke somewhere (wrong plan, a binding that snuck in R2/KV
again, a Cloudflare pricing change) — go find out why before dismissing it.

## (c) Confirm Workers + Durable Objects stay on Free

- [ ] Dashboard → **Workers & Pages** → **Overview** (or **Plans**) →
      confirm the account is on the **Free** plan, not **Paid**.
- [ ] Confirm `share-relay` itself has no plan override — Workers don't
      have a separate per-Worker plan, but double-check there's no
      Workers Paid subscription active on this account that you didn't
      intend.
- [ ] This is what makes overages **fail closed**: on Free, exceeding a
      Durable Objects limit (100k requests/day, 100k rows written/day, 5M
      rows read/day, 5 GB storage — all account-wide) returns an error to
      the client (Cloudflare error 1027, "Worker exceeded resource
      limits", or a plain request failure) instead of being metered and
      billed. On a Paid plan, the same overage would instead be billed.
      **Staying on Free is not a performance choice here — it's the
      billing guarantee itself.**
- [ ] If anything on the account ever needs a paid plan for an unrelated
      reason, revisit this Worker specifically before that happens — the
      $0 argument in `README.md` stops holding the moment this account (or
      this Worker's Durable Objects) leaves Free.

## Optional: allow a self-hosted HTTPS test origin

To exercise the phone flow against a self-hosted build (e.g. a homelab
`https://hew.granroth.xyz` serving `shells/web/dist` behind a real cert)
rather than production `app.hew3d.com`, the relay must let that origin read
its responses. It is NOT in the committed allowlist; add it as a Worker var:

1. Cloudflare dashboard → Workers & Pages → `share-relay` → Settings →
   Variables and Secrets → add a plaintext variable
   `EXTRA_ALLOWED_ORIGINS` = `https://hew.granroth.xyz` (comma-separate
   several). Deploy/save.
   - Or, for a local `wrangler dev`: `wrangler dev --var
     'EXTRA_ALLOWED_ORIGINS:https://hew.granroth.xyz'`.
2. Serve the shop web build (`pnpm --dir shells/web build` →
   `shells/web/dist`) at that origin.
3. Build the DESKTOP with `VITE_HEW_RECEIVE_ORIGIN=https://hew.granroth.xyz`
   so the QR (and a camera-app scan of it) points at the test origin. The
   in-app scanner works regardless of this — it reads the token off the QR
   and fetches the relay directly — so this only matters for the
   camera-app fallback path.

Remove the var when done; production needs only the three base origins.

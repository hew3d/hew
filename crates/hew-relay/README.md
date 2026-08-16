# hew-relay

The self-hostable "Open on Phone" relay for Hew: a one-shot, in-memory,
end-to-end-encrypted dead-drop. The desktop app encrypts a document in
its own process, uploads the ciphertext here, and shows a QR code whose
URL carries the decryption key in its fragment — so this server never
sees a key or a plaintext byte, and forgets the ciphertext the moment the
phone reads it once (or after ten minutes).

It speaks exactly the contract of the public relay Worker
(`workers/share-relay/README.md` in the Hew repository is the reference;
`workers/share-relay/contract/` is the black-box suite both must pass) and
is meant to sit behind the same web server that serves the Hew web app,
under `/relay/` on the same origin. Full setup — nginx, Docker, Proxmox,
HTTPS, and the desktop's *Settings ▸ Advanced ▸ Server* setting — is in
`docs/SELF_HOSTING.md`.

## Running

```
hew-relay [--listen 127.0.0.1:8787] [--allow-origin https://x]... [--upload-key KEY]
          [--max-bytes 33554432] [--max-total-bytes 268435456] [--ttl-secs 600]
```

Every flag has an environment twin — `HEW_RELAY_LISTEN`,
`HEW_RELAY_ALLOW_ORIGINS` (comma-separated), `HEW_RELAY_UPLOAD_KEY`,
`HEW_RELAY_MAX_BYTES`, `HEW_RELAY_MAX_TOTAL_BYTES`, `HEW_RELAY_TTL_SECS` —
which is how the shipped `hew-relay.service` unit configures it (through
`/etc/hew/relay.env`). There is no config file.

- `--upload-key` makes `PUT` require `Authorization: Bearer <key>` (the
  desktop setting sends it); reads stay keyless because the token is the
  capability. Set it if the relay is reachable from the internet.
- `--max-total-bytes` bounds memory: past it, uploads get
  `503 {"error":"relay full"}` with `Retry-After` and the desktop says so.
- `--allow-origin` is only for a phone app served from a DIFFERENT origin
  than the relay; same-origin (the recommended layout) never needs it.

Logs one line per request — method, route kind, status, size — never a
token, a body, or the key. `RUST_LOG` filters as usual.

## Files in this archive

- `hew-relay` — static binary (musl), install to `/usr/local/bin/`.
- `hew-relay.service` — systemd unit, install to `/etc/systemd/system/`.
- `README.md` — this file.

Licensed AGPL-3.0-only, like the rest of Hew.

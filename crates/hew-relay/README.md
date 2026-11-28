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
- `--max-total-bytes` is the memory budget for documents: uploads in
  flight, stored drops, and responses still being sent. See *Limits*.
- `--allow-origin` is only for a phone app served from a DIFFERENT origin
  than the relay; same-origin (the recommended layout) never needs it.

Logs one line per request — method, route kind, status, size — never a
token, a body, or the key. `RUST_LOG` filters as usual.

## Limits

**Byte budget** (`--max-total-bytes`). An upload claims nothing until body
data arrives, whatever length its headers declare. Each buffer is then
charged its allocated size, and at least 4 KiB, so the budget also bounds
the number of drops: 65,536 at the default. There is no separate entry
limit. An upload with a `Content-Length` never holds more than that length
(or 4 KiB). One without can hold up to twice what it has sent, capped at
`--max-bytes`, until it finishes. Past the budget, uploads get
`503 {"error":"relay full"}` with `Retry-After`.

**Time.** The relay serves HTTP/1.1 only.

- A request head must arrive within 15 seconds of the connection opening,
  or of the previous response on it.
- A request, body included, has 60 seconds. After that it gets a `408`.
- While a response waits for the client to read, the connection must
  accept another 1 MiB every 2 minutes, or it is closed and its budget
  released. A client that reads the bare minimum can hold a 32 MiB
  document for up to 64 minutes, well past the drop's TTL. A download
  that is cut off cannot be retried, because a drop is gone once read.

  A client reading steadily at 16 KiB/s or faster is not cut off, however
  large the document, on a host with Linux's default send buffer limit
  (`net.ipv4.tcp_wmem` maximum of 4 MiB). That was measured over loopback
  with a copy of this logic. The kernel reports progress in steps that
  grow with the send buffer, so a host that raises the maximum above
  roughly 5.5 MiB could cut off a reader at that rate. The 5.5 MiB figure
  is extrapolated, not measured.

**What the relay does not limit.** It cannot tell clients apart, because
behind a proxy they all share one address, so it has no per-client limits.
One client with the bandwidth can fill the budget with real uploads; an
upload key stops strangers from doing so. The relay has no connection
limit of its own either. The process's open-file limit is the ceiling,
typically 1024 under systemd, and a client that reaches it keeps others
from connecting. Each connection costs memory outside the budget, with its
read buffer capped at 64 KiB. If the relay is reachable from the internet,
set per-client connection and request limits in the proxy (nginx
`limit_conn` and `limit_req`). A proxy that buffers responses, as nginx
does by default, reads a download from the relay at once and applies its
own timeouts to the client.

## Files in this archive

- `hew-relay` — static binary (musl), install to `/usr/local/bin/`.
- `hew-relay.service` — systemd unit, install to `/etc/systemd/system/`.
- `README.md` — this file.

Licensed AGPL-3.0-only, like the rest of Hew.

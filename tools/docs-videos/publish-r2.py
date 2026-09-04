#!/usr/bin/env python3
"""Publish the docs-video fleet to the hew-media R2 bucket and emit the
manifest the site will consume.

Keys are content-hashed (docs/<slug>-<sha256[:10]>.<ext>) so every object is
immutable: cache-control is set to a year + immutable, and republishing a
changed clip mints a NEW key while the manifest flips the pointer. Old keys
are left in place (cheap; old HTML keeps working) — prune manually if ever
needed.

Idempotent: keys already present in the bucket are skipped, so re-runs only
upload what changed. State of the bucket is read once via `r2 object get`
probes? No — via the manifest from the previous run (videos.json next to
this script) plus a --force flag to re-upload anyway.

  publish-r2.py [--dry-run] [--force]

Requires wrangler auth (uses the share-relay node_modules install).
"""
import argparse, hashlib, json, os, subprocess, sys

HERE = os.path.dirname(os.path.abspath(__file__))
MEDIA = os.path.join(HERE, "../../site/public/videos")
MANIFEST = os.path.join(HERE, "../../site/src/data/videos.json")
WRANGLER = os.path.join(HERE, "../../workers/share-relay/node_modules/.bin/wrangler")
BUCKET = "hew-media"
BASE_URL = "https://media.hew3d.com"
MIME = {".webm": 'video/webm; codecs="av01.0.08M.08"',
        ".mp4": 'video/mp4; codecs="avc1.640028"',
        ".jpg": "image/jpeg"}
CACHE = "public, max-age=31536000, immutable"

ap = argparse.ArgumentParser()
ap.add_argument("--dry-run", action="store_true")
ap.add_argument("--force", action="store_true")
a = ap.parse_args()

prev = set()
if os.path.exists(MANIFEST):
    m = json.load(open(MANIFEST))
    prev = {e["key"] for slug in m["videos"]
            for e in m["videos"][slug].values() if isinstance(e, dict)}

def duration(path):
    out = subprocess.run(["ffprobe", "-v", "error", "-show_entries", "format=duration",
                          "-of", "csv=p=0", path], capture_output=True, text=True).stdout.strip()
    return round(float(out), 1) if out else None

slugs = sorted({f[len("docs-"):-len("-dark.webm")]
                for f in os.listdir(MEDIA) if f.endswith("-dark.webm")})
videos, uploads = {}, []
for slug in slugs:
    entry = {}
    for ext in (".webm", ".mp4", ".jpg"):
        path = os.path.join(MEDIA, f"docs-{slug}-dark{ext}")
        if not os.path.exists(path):
            sys.exit(f"missing {path}")
        digest = hashlib.sha256(open(path, "rb").read()).hexdigest()
        key = f"docs/{slug}-{digest[:10]}{ext}"
        entry[{".webm": "webm", ".mp4": "mp4", ".jpg": "poster"}[ext]] = {
            "key": key, "url": f"{BASE_URL}/{key}",
            "bytes": os.path.getsize(path), "sha256": digest,
        }
        if a.force or key not in prev:
            uploads.append((path, key, MIME[ext]))
    entry["seconds"] = duration(os.path.join(MEDIA, f"docs-{slug}-dark.webm"))
    entry["theme"] = "dark"
    videos[slug] = entry

print(f"{len(slugs)} clips, {len(uploads)} objects to upload"
      f" ({sum(os.path.getsize(p) for p, _, _ in uploads) / 1e6:.0f} MB)")
if a.dry_run:
    for _, key, _ in uploads: print("  would put", key)
    sys.exit(0)

for i, (path, key, mime) in enumerate(uploads, 1):
    print(f"[{i}/{len(uploads)}] {key}")
    r = subprocess.run([WRANGLER, "r2", "object", "put", f"{BUCKET}/{key}",
                        "--file", path, "--content-type", mime,
                        "--cache-control", CACHE, "--remote"],
                       capture_output=True, text=True)
    if r.returncode:
        sys.exit(f"upload failed for {key}:\n{r.stdout}\n{r.stderr}")

json.dump({"base": BASE_URL, "videos": videos}, open(MANIFEST, "w"), indent=2)
print(f"manifest → {MANIFEST}")

#!/usr/bin/env python3
"""Publish the docs-video fleet and the launch reel to the hew-media R2
bucket and emit the manifest the site will consume.

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
        ".jpg": "image/jpeg", ".webp": "image/webp", ".gif": "image/gif"}
CACHE = "public, max-age=31536000, immutable"

ap = argparse.ArgumentParser()
ap.add_argument("--dry-run", action="store_true")
ap.add_argument("--force", action="store_true")
ap.add_argument("--allow-stale-readme", action="store_true",
                help="publish the reel even if README.md does not reference its excerpt URLs")
a = ap.parse_args()

prev, published = set(), {}
if os.path.exists(MANIFEST):
    m = json.load(open(MANIFEST))
    published = m["videos"]
    prev = {e["key"] for slug in published
            for e in published[slug].values() if isinstance(e, dict)}
    prev |= {e["key"] for e in m.get("reel", {}).values() if isinstance(e, dict)}

def duration(path):
    out = subprocess.run(["ffprobe", "-v", "error", "-show_entries", "format=duration",
                          "-of", "csv=p=0", path], capture_output=True, text=True).stdout.strip()
    return round(float(out), 1) if out else None

os.makedirs(MEDIA, exist_ok=True)
slugs = sorted({f[len("docs-"):-len("-dark.webm")]
                for f in os.listdir(MEDIA) if f.endswith("-dark.webm")})
# Clips not rendered on this machine keep their published entries: the
# rendered media is gitignored, so a checkout that only re-renders the reel
# (or one chapter) must not drop the rest of the fleet from the manifest.
videos, uploads = dict(published), []

def hashed(path, prefix):
    """(key, manifest entry) for one file; content-hashed like the docs clips."""
    digest = hashlib.sha256(open(path, "rb").read()).hexdigest()
    ext = os.path.splitext(path)[1]
    key = f"{prefix}-{digest[:10]}{ext}"
    return key, {"key": key, "url": f"{BASE_URL}/{key}",
                 "bytes": os.path.getsize(path), "sha256": digest}

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

# The launch reel (tools/launch-reel/assemble-reel.py writes reel.* here):
# the landing page's <video> pair + poster, and the README excerpt. The
# README references the excerpt by its hashed URL, so a re-rendered excerpt
# means a README edit — the check below says so instead of publishing a URL
# the README does not use.
REEL = {"webm": "reel.webm", "mp4": "reel.mp4", "poster": "reel.jpg",
        "readme_webp": "reel-readme.webp", "readme_gif": "reel-readme.gif"}
reel = None
if os.path.exists(os.path.join(MEDIA, REEL["webm"])):
    reel = {}
    for field, name in REEL.items():
        path = os.path.join(MEDIA, name)
        if not os.path.exists(path):
            sys.exit(f"missing {path} (assemble-reel.py writes the whole set)")
        key, entry = hashed(path, "reel/" + ("readme" if field.startswith("readme") else "reel"))
        reel[field] = entry
        if a.force or key not in prev:
            uploads.append((path, key, MIME[os.path.splitext(name)[1]]))
    reel["seconds"] = duration(os.path.join(MEDIA, REEL["webm"]))
    readme = open(os.path.join(HERE, "../../README.md")).read()
    stale = [f for f in ("readme_webp", "readme_gif") if reel[f]["url"] not in readme]
    if stale and not a.allow_stale_readme:
        sys.exit("README.md does not reference the README excerpt about to be published:\n  "
                 + "\n  ".join(reel[f]["url"] for f in stale)
                 + "\nPut these URLs in README.md's <picture> block (or pass --allow-stale-readme).")
elif os.path.exists(MANIFEST) and "reel" in json.load(open(MANIFEST)):
    reel = json.load(open(MANIFEST))["reel"]   # keep the published reel as is

print(f"{len(videos)} clips in manifest ({len(slugs)} rendered here){' + reel' if reel else ''}, "
      f"{len(uploads)} objects to upload"
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

manifest = {"base": BASE_URL, "videos": videos}
if reel:
    manifest["reel"] = reel
json.dump(manifest, open(MANIFEST, "w"), indent=2)
print(f"manifest → {MANIFEST}")

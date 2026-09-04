#!/usr/bin/env python3
"""Insert <docs-video> embeds into Learn chapters for every clip in the
committed manifest (site/src/data/videos.json). Idempotent: skips chapters
that already carry a docs-video tag. The element lands after the first
prose paragraph following the frontmatter (the chapter intro), so the
reader gets one sentence of context before the moving picture.

Embeds are slug-addressed — the BaseLayout component resolves the slug to
the manifest's content-hashed media URLs at build time, so re-publishing a
clip never touches the markdown.

  embed-fleet.py [--site <site-dir>] [--dry-run]
"""
import argparse, json, os, re, sys

HERE = os.path.dirname(os.path.abspath(__file__))

ap = argparse.ArgumentParser()
ap.add_argument("--site", default=os.path.join(HERE, "../../site"))
ap.add_argument("--dry-run", action="store_true")
a = ap.parse_args()

manifest = json.load(open(os.path.join(a.site, "src/data/videos.json")))
changed, skipped = [], []
for slug in sorted(manifest["videos"]):
    md = os.path.join(a.site, "src/content/learn", f"{slug}.md")
    if not os.path.exists(md):
        skipped.append((slug, "no chapter file")); continue
    s = open(md).read()
    if "<docs-video" in s:
        skipped.append((slug, "already embedded")); continue
    m = re.match(r"(?s)^(---\n.*?\n---\n\s*\n)(.*?\n)\n", s)
    if not m:
        skipped.append((slug, "no intro paragraph found")); continue
    tag = (f'<docs-video theme="dark" slug="{slug}"'
           f' label="{slug.replace("-", " ")} demonstration"></docs-video>\n\n')
    if not a.dry_run:
        open(md, "w").write(s[:m.end()] + tag + s[m.end():])
    changed.append(slug)

print("embedded:", ", ".join(changed) or "(none)")
for slug, why in skipped:
    print(f"skipped {slug}: {why}")
sys.exit(0)

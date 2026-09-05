#!/usr/bin/env python3
"""Assemble the launch reel from per-shot captures.

reel.json lists the shots in order. Each entry names a capture directory
under out/ and the two marks that bound the cut, with optional offsets and a
speed factor:

  { "shot": "carve", "in": "box", "out": "hole", "in_off": 1.2,
    "out_off": 0.5, "speed": 2.6 }

Mark times come from capture-meta.json (seconds since the harness started);
they are aligned to the recording by the same rule as the docs fleet: the
`end` mark is taken to coincide with the end of the video, so
video_time = mark + (duration - marks.end).

Outputs, next to reel.json unless --out is given:
  <out>.webm   AV1 (svt-av1 CRF 38), the landing page's primary source
  <out>.mp4    H.264 (CRF via --h264-crf), the Safari fallback
  <out>.jpg    poster frame
  <out>-readme.gif / .webp   the README excerpt: the shots flagged
               "readme": true, at --readme-width px, looping. A shot can
               carry its own README cut ("readme_in"/"readme_out" plus
               "readme_in_off"/"readme_out_off") when the excerpt wants a
               shorter piece of it than the reel does.

  assemble-reel.py [--only SHOT] [--readme-only] [--h264-crf 20]
"""
import argparse, json, os, shutil, subprocess, sys, tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
ap = argparse.ArgumentParser()
ap.add_argument("--reel", default=os.path.join(HERE, "reel.json"))
ap.add_argument("--out", default=None, help="output basename (default: reel.json's \"out\")")
ap.add_argument("--only", help="build a single shot as <out>-<shot>.mp4 for review")
ap.add_argument("--h264-crf", type=int, default=23)
# 560 px at 12 fps decodes in real time on ordinary machines; a 720 px
# 15 fps excerpt stalled on long pans (animated images delay frames when
# decoding falls behind rather than dropping them)
ap.add_argument("--readme-width", type=int, default=560)
ap.add_argument("--readme-fps", type=int, default=12)
ap.add_argument("--readme-only", action="store_true")
a = ap.parse_args()

reel = json.load(open(a.reel))
fps = reel.get("fps", 30)
out_base = a.out or os.path.join(HERE, reel.get("out", "out/reel"))
os.makedirs(os.path.dirname(out_base), exist_ok=True)

def run(cmd):
    r = subprocess.run(cmd, capture_output=True, text=True)
    if r.returncode:
        sys.exit(f"ffmpeg failed:\n  {' '.join(cmd)}\n{r.stderr}")

def duration(path):
    return float(subprocess.run(["ffprobe", "-v", "error", "-show_entries", "format=duration",
                                 "-of", "csv=p=0", path], capture_output=True, text=True).stdout)

def cut(entry, dest, key=""):
    """Cut one shot; key="readme_" reads the README-specific bounds when set."""
    meta_path = os.path.join(HERE, "out", entry["shot"], "capture-meta.json")
    meta = json.load(open(meta_path))
    video, marks = meta["video"], meta["marks"]
    dur = duration(video)
    off = dur - marks["end"]
    g = lambda name, default: entry.get(key + name, entry.get(name, default)) if key else entry.get(name, default)
    t_in = marks[g("in", "start")] + off + g("in_off", 0.0)
    t_out = marks[g("out", "end")] + off + g("out_off", 0.0)
    speed = entry.get("speed", 1.0)
    # ffmpeg's trim silently clamps an out-of-range start to 0 and produces a
    # shorter, shifted clip with a zero exit code — refuse instead.
    if not (0.0 <= t_in < t_out <= dur + 0.05):
        sys.exit(f"{entry['shot']}: cut {t_in:.2f} → {t_out:.2f} is outside the "
                 f"{dur:.2f}s capture (check in_off/out_off against the marks)")
    vf = (f"trim=start={t_in:.3f}:end={t_out:.3f},setpts=(PTS-STARTPTS)/{speed},"
          f"fps={fps},format=yuv420p")
    run(["ffmpeg", "-y", "-v", "error", "-i", video, "-vf", vf, "-an",
         "-c:v", "libx264", "-preset", "fast", "-crf", "12", dest])
    secs = (t_out - t_in) / speed
    print(f"  {entry['shot']:<18} {t_in:6.2f} → {t_out:6.2f}  x{speed:<4} = {secs:4.1f}s")
    return secs

shots = reel["shots"]
if a.only:
    shots = [s for s in shots if s["shot"] == a.only]
    if not shots:
        sys.exit(f"no shot named {a.only} in {a.reel}")

tmp = tempfile.mkdtemp(prefix="hew-reel-")
try:
    parts, total = [], 0.0
    if not a.readme_only:
        for i, entry in enumerate(shots):
            dest = os.path.join(tmp, f"{i:02d}-{entry['shot']}.mp4")
            total += cut(entry, dest)
            parts.append((entry, dest))
        print(f"  total {total:.1f}s over {len(parts)} shots")

    def concat(entries, dest):
        lst = os.path.join(tmp, "list.txt")
        with open(lst, "w") as f:
            for _, p in entries:
                f.write("file '" + p.replace("'", "'\\''") + "'\n")
        run(["ffmpeg", "-y", "-v", "error", "-f", "concat", "-safe", "0", "-i", lst,
             "-c", "copy", dest])

    if a.only:
        dest = f"{out_base}-{a.only}.mp4"
        shutil.copy(parts[0][1], dest)
        print("wrote", dest)
        sys.exit(0)

    if not a.readme_only:
        master = os.path.join(tmp, "master.mp4")
        concat(parts, master)
        run(["ffmpeg", "-y", "-v", "error", "-i", master, "-an",
             "-c:v", "libsvtav1", "-crf", "38", "-preset", "8", "-g", str(fps * 5),
             f"{out_base}.webm"])
        run(["ffmpeg", "-y", "-v", "error", "-i", master, "-an",
             "-c:v", "libx264", "-preset", "medium", "-crf", str(a.h264_crf),
             "-movflags", "+faststart", f"{out_base}.mp4"])
        run(["ffmpeg", "-y", "-v", "error", "-i", master, "-ss", str(reel.get("poster_at", 1.0)),
             "-frames:v", "1", "-q:v", "3", f"{out_base}.jpg"])
        print("wrote", f"{out_base}.webm", f"{out_base}.mp4", f"{out_base}.jpg")

    readme_parts, rtotal = [], 0.0
    for i, entry in enumerate(e for e in shots if e.get("readme")):
        dest = os.path.join(tmp, f"readme-{i:02d}-{entry['shot']}.mp4")
        rtotal += cut(entry, dest, key="readme_")
        readme_parts.append((entry, dest))
    if readme_parts:
        print(f"  README excerpt {rtotal:.1f}s over {len(readme_parts)} shots")
        excerpt = os.path.join(tmp, "readme.mp4")
        concat(readme_parts, excerpt)
        w = a.readme_width
        scale = f"fps={a.readme_fps},scale={w}:-2:flags=lanczos"
        run(["ffmpeg", "-y", "-v", "error", "-i", excerpt, "-vf",
             f"{scale},split[a][b];[a]palettegen=max_colors=128:stats_mode=diff[p];"
             f"[b][p]paletteuse=dither=bayer:bayer_scale=4:diff_mode=rectangle",
             "-loop", "0", f"{out_base}-readme.gif"])
        # Homebrew's ffmpeg ships without libwebp; gif2webp (the webp
        # formula) turns the GIF into a far smaller animated WebP.
        if shutil.which("gif2webp"):
            run(["gif2webp", "-q", "70", "-m", "4", "-mixed", f"{out_base}-readme.gif",
                 "-o", f"{out_base}-readme.webp"])
        for ext in ("gif", "webp"):
            p = f"{out_base}-readme.{ext}"
            if not os.path.exists(p): continue
            print(f"wrote {p} ({os.path.getsize(p) / 1e6:.1f} MB)")
finally:
    shutil.rmtree(tmp, ignore_errors=True)

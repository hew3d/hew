#!/usr/bin/env python3
"""Assemble a docs-fleet clip: trim the capture to the scene (keeping the
intro caption), no audio, web-embed ready.

Emits TWO files per clip for a <video><source> pair:
  <out>.webm  — AV1 (svt-av1 CRF 38): primary; dark-theme UIs cost the same
                as light here, unlike H.264, whose adaptive quantization
                spends ~2x bits protecting dark gradients from banding
  <out>.mp4   — H.264 fallback for older Safari (CRF via --h264-crf;
                use ~22 for dark-theme captures to tame the dark premium)

  assemble-docs.py --meta <capture-meta.json> --out <clip-basename> \
      [--lead 1.55] [--h264-crf 19]
"""
import argparse, json, subprocess, sys

ap = argparse.ArgumentParser()
ap.add_argument("--meta", required=True)
ap.add_argument("--out", required=True)
ap.add_argument("--lead", type=float, default=1.55, help="seconds kept before scene-start")
ap.add_argument("--h264-crf", type=int, default=19)
a = ap.parse_args()

meta = json.load(open(a.meta))
video, beats = meta["video"], meta["beats"]
dur = float(subprocess.run(["ffprobe", "-v", "error", "-show_entries", "format=duration",
                            "-of", "csv=p=0", video], capture_output=True, text=True).stdout)
off = dur - beats["scene-end"]
start = max(0.0, beats["scene-start"] + off - a.lead)
base = a.out.removesuffix(".mp4").removesuffix(".webm")
vf = f"trim=start={start:.3f},setpts=PTS-STARTPTS,fps=30,format=yuv420p"
print(f"trim {start:.2f}s → end ({dur - start:.1f}s clip)")
rc = subprocess.run(["ffmpeg", "-y", "-v", "error", "-i", video, "-vf", vf, "-an",
                     "-c:v", "libsvtav1", "-crf", "38", "-preset", "6",
                     f"{base}.webm"]).returncode
rc |= subprocess.run(["ffmpeg", "-y", "-v", "error", "-i", video, "-vf", vf, "-an",
                      "-c:v", "libx264", "-preset", "medium", "-crf", str(a.h264_crf),
                      "-movflags", "+faststart", f"{base}.mp4"]).returncode
rc |= subprocess.run(["ffmpeg", "-y", "-v", "error", "-i", f"{base}.webm", "-ss", "1",
                      "-frames:v", "1", "-q:v", "4", f"{base}.jpg"]).returncode
sys.exit(rc)
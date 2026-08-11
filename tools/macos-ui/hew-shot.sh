#!/usr/bin/env bash
#
# hew-shot.sh — take a documentation-standard screenshot of the live Hew window.
#
# Matches what app/scripts/capture-docs-screenshots.mjs produces for
# site/public/docs/: 1440x900 of app content at 2x, written as an 8-bit RGB PNG
# with no alpha channel and no embedded colour profile (2880x1800). Those three
# normalisations are why the generated shots are a fraction of the size of the
# same picture saved out of Preview, which keeps an alpha channel and a Display
# P3 profile and doesn't optimise the deflate stream.
#
# The window is resized for the capture and put back afterwards.
#
# Usage:
#   hew-shot.sh [out.png] [options]
#
#   --size WxH     content size in points to capture (default 1440x900)
#   --titlebar     include the macOS title bar; WxH then covers the whole
#                  window, as the generated shots never do
#   --scale N      output = capture size x N (default 2, the docs standard)
#   --native       don't rescale; write exactly what the display produced
#   --keep         leave the window at capture size instead of restoring it
#                  (use when taking a series, so the framing stays identical)
#   --delay S      seconds to wait after resizing before capturing (default 1.0)
#
# On a Retina display a 1440x900 region captures as 2880x1800 real pixels with
# no resampling — the same kind of image the generated shots are. On a
# non-Retina display (a 4K panel run at its native 3840x2160, say) the capture
# is only 1440x900 pixels and has to be upsampled to reach the standard, which
# is visibly softer and compresses worse. The script says so when that happens;
# for true 2x pixels on such a display, drive the app in a browser instead —
# see app/scripts/capture-live-shot.mjs.
#
# Requires the same Accessibility permission as hew-ui.sh (System Events needs
# to read and set the window bounds). Override the target process with
# HEW_UI_PROC=Name; the default `hew` covers both the installed app and
# `pnpm --dir shells/tauri dev`.
set -euo pipefail

PROC="${HEW_UI_PROC:-hew}"
OUT=""
CAP_W=1440
CAP_H=900
SCALE=2
NATIVE=0
KEEP=0
TITLEBAR=0
DELAY=1.0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --size)     [[ "$2" =~ ^([0-9]+)[xX]([0-9]+)$ ]] || { echo "--size wants WxH" >&2; exit 2; }
                CAP_W="${BASH_REMATCH[1]}"; CAP_H="${BASH_REMATCH[2]}"; shift 2 ;;
    --scale)    SCALE="$2"; shift 2 ;;
    --delay)    DELAY="$2"; shift 2 ;;
    --native)   NATIVE=1; shift ;;
    --keep)     KEEP=1; shift ;;
    --titlebar) TITLEBAR=1; shift ;;
    -h|--help)  grep -E '^#( |$)' "$0" | sed -E 's/^# ?//'; exit 0 ;;
    -*)         echo "unknown option: $1" >&2; exit 2 ;;
    *)          OUT="$1"; shift ;;
  esac
done
OUT="${OUT:-$HOME/Desktop/hew-shot.png}"
[[ "$OUT" == *.png ]] || OUT="$OUT.png"

# --- window geometry ---------------------------------------------------------
read_bounds() {
  osascript <<OSA
tell application "System Events" to tell process "$PROC"
  set win to front window
  set {px, py} to position of win
  set {sw, sh} to size of win
end tell
return (px as string) & " " & (py as string) & " " & (sw as string) & " " & (sh as string)
OSA
}

set_bounds() { # x y w h
  osascript <<OSA >/dev/null
tell application "System Events" to tell process "$PROC"
  set win to front window
  set position of win to {$1, $2}
  set size of win to {$3, $4}
end tell
OSA
}

# Title-bar height, derived from the live window rather than assumed: the close
# button is inset from the window's top-left by the same margin on both axes, so
# the bar is that inset above and below a 16pt button. Falls back to the stock
# 28pt if the traffic lights aren't where we expect.
read_titlebar() {
  osascript <<OSA 2>/dev/null || echo 28
tell application "System Events" to tell process "$PROC"
  set win to front window
  set {wx, wy} to position of win
  set b to button 1 of win
  set {bx, by} to position of b
  set {bw, bh} to size of b
  return ((by - wy) * 2 + bh) as string
end tell
OSA
}

osascript -e "tell application \"System Events\" to tell process \"$PROC\" to set frontmost to true"
sleep 0.4

read -r ORIG_X ORIG_Y ORIG_W ORIG_H <<<"$(read_bounds)"

restore() {
  (( KEEP )) || set_bounds "$ORIG_X" "$ORIG_Y" "$ORIG_W" "$ORIG_H" || true
  [[ -n "${TMP:-}" ]] && rm -f "$TMP" "$TMP.png"
  return 0
}
trap restore EXIT

CHROME=0
if (( ! TITLEBAR )); then
  CHROME=$(read_titlebar | tr -dc '0-9')
  [[ -n "$CHROME" ]] || CHROME=28
fi
WIN_W=$CAP_W
WIN_H=$(( CAP_H + CHROME ))

# Keep the window fully on-screen at the capture size: a region running off the
# display captures black, and a window under the menu bar loses its top rows.
SCREEN=$(osascript -e 'tell application "Finder" to get bounds of window of desktop')
SCREEN_W=$(awk -F', ' '{print $3}' <<<"$SCREEN")
SCREEN_H=$(awk -F', ' '{print $4}' <<<"$SCREEN")
MENU_H=$(osascript -e 'tell application "System Events" to get item 2 of (size of menu bar 1 of process "Finder")' 2>/dev/null || echo 25)

X=$ORIG_X; Y=$ORIG_Y
(( X + WIN_W > SCREEN_W )) && X=$(( SCREEN_W - WIN_W ))
(( Y + WIN_H > SCREEN_H )) && Y=$(( SCREEN_H - WIN_H ))
(( X < 0 )) && X=0
(( Y < MENU_H )) && Y=$MENU_H

if (( WIN_W > SCREEN_W || Y + WIN_H > SCREEN_H )); then
  echo "warning: ${WIN_W}x${WIN_H} points doesn't fit on this ${SCREEN_W}x${SCREEN_H} display" >&2
fi

set_bounds "$X" "$Y" "$WIN_W" "$WIN_H"
sleep "$DELAY"

# Re-read: a window can refuse the exact size (minimum size, tiling) or be moved
# by the window server, and the capture region has to follow the real geometry.
read -r BX BY BW BH <<<"$(read_bounds)"
RX=$BX
RY=$(( BY + CHROME ))
RW=$BW
RH=$(( BH - CHROME ))

TMP="$(mktemp -t hew-shot)"
RAW="$TMP.png"
screencapture -x -R "${RX},${RY},${RW},${RH}" -t png "$RAW"

# --- normalise to the docs standard -----------------------------------------
if (( NATIVE )); then
  TARGET_W=0; TARGET_H=0
else
  TARGET_W=$(( RW * SCALE )); TARGET_H=$(( RH * SCALE ))
fi

PY=""
for c in /usr/bin/python3 python3; do
  if command -v "$c" >/dev/null 2>&1 && "$c" -c "import PIL" >/dev/null 2>&1; then PY="$c"; break; fi
done

if [[ -n "$PY" ]]; then
  "$PY" - "$RAW" "$OUT" "$TARGET_W" "$TARGET_H" <<'PY'
import sys
from PIL import Image

src, dst, tw, th = sys.argv[1], sys.argv[2], int(sys.argv[3]), int(sys.argv[4])
img = Image.open(src)
# Screen captures come back RGBA with a display profile attached; the generated
# docs shots are plain 8-bit RGB with neither. Compose onto black rather than
# dropping the channel, so any translucent chrome resolves the way the screen
# showed it.
if img.mode != "RGB":
    img = img.convert("RGBA")
    flat = Image.new("RGB", img.size, (0, 0, 0))
    flat.paste(img, mask=img.split()[-1])
    img = flat
if tw and (img.width, img.height) != (tw, th):
    img = img.resize((tw, th), Image.LANCZOS)
img.save(dst, "PNG", optimize=True)
PY
else
  # No Pillow: sips can resize and drop the profile but keeps the alpha channel,
  # so the file lands larger than the generated shots.
  cp "$RAW" "$OUT"
  if (( TARGET_W )); then sips -z "$TARGET_H" "$TARGET_W" "$OUT" >/dev/null; fi
  sips --deleteColorManagementProperties "$OUT" >/dev/null 2>&1 || true
  echo "note: no python3 with Pillow found; used sips (alpha channel retained)" >&2
fi

px() { sips -g pixelWidth -g pixelHeight "$1" | awk '/pixelWidth/{w=$2} /pixelHeight/{h=$2} END{print w "x" h}'; }
RAW_PX=$(px "$RAW")
OUT_PX=$(px "$OUT")

echo "$OUT"
echo "  ${OUT_PX}, $(( $(stat -f%z "$OUT") / 1024 )) KiB   (region ${RW}x${RH} pt captured as ${RAW_PX} px)"

RAW_W=${RAW_PX%x*}
if (( ! NATIVE && RAW_W < TARGET_W )); then
  cat >&2 <<MSG
  note: this display renders Hew at $(awk "BEGIN{printf \"%.3g\", $RAW_W/$RW}")x, so the image was upsampled to
        reach ${TARGET_W}x${TARGET_H} and is softer than the generated docs shots. For true
        2x pixels, use a Retina display or app/scripts/capture-live-shot.mjs.
MSG
fi

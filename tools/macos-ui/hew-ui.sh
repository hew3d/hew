#!/usr/bin/env bash
#
# hew-ui.sh — drive the Hew desktop app for UI/UX testing on macOS.
#
# Wraps the three macOS automation primitives that work against a Tauri webview
# (see README.md for why each is needed and what permissions it requires):
#   - screencapture        window screenshots
#   - System Events        native menu bar + keystrokes (Accessibility perm)
#   - mouse.swift (CGEvent) clicks/drags inside the webview (Accessibility perm)
#
# COORDINATES ARE WINDOW-RELATIVE POINTS. The window can move between calls, so
# every command re-reads the live window bounds and offsets to absolute screen
# coordinates itself. `shot` produces an image whose pixel size equals the
# window's point size, so a pixel you eyeball in the screenshot at (ix, iy) is
# exactly the argument to `click ix iy`. No Retina math on your side.
#
# Usage:
#   hew-ui.sh bounds                     print "x y w h" (screen points)
#   hew-ui.sh activate                   bring Hew to the front
#   hew-ui.sh shot [out.png]             screenshot the window (1px == 1 click pt)
#   hew-ui.sh click  X Y                 left click  at window-relative (X,Y)
#   hew-ui.sh dblclick X Y               double click
#   hew-ui.sh rclick X Y                 right click
#   hew-ui.sh move   X Y                 move cursor only
#   hew-ui.sh drag   X1 Y1 X2 Y2         press-drag-release (draw in viewport)
#   hew-ui.sh menus                      list native menu names
#   hew-ui.sh menu   "Tools" "Rotate"    click a menu item (1 or 2 levels deep)
#   hew-ui.sh key    "z" using {command down}   send a keystroke (raw AppleScript)
#   hew-ui.sh type   "hello"             type literal text
#
# Override the target process (default hew-desktop) with HEW_UI_PROC=Name.
set -euo pipefail

PROC="${HEW_UI_PROC:-hew-desktop}"
DIR="$(cd "$(dirname "$0")" && pwd)"
BIN="$DIR/.bin"
MOUSE="$BIN/mouse"

# --- compile the CGEvent helper on first use / when the source changes --------
ensure_mouse() {
  mkdir -p "$BIN"
  if [[ ! -x "$MOUSE" || "$DIR/mouse.swift" -nt "$MOUSE" ]]; then
    swiftc -O "$DIR/mouse.swift" -o "$MOUSE"
  fi
}

# --- live window bounds as "x y w h" (screen points) --------------------------
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

activate() {
  osascript -e "tell application \"System Events\" to tell process \"$PROC\" to set frontmost to true"
  sleep 0.3
}

# window-relative point -> absolute screen point (echoes "absX absY")
to_abs() { # $1=relX $2=relY
  read -r bx by _ _ <<<"$(read_bounds)"
  echo "$((bx + $1)) $((by + $2))"
}

cmd="${1:-}"; shift || true
case "$cmd" in
  bounds)
    read_bounds
    ;;
  activate)
    activate
    ;;
  shot)
    out="${1:-/tmp/hew-shot.png}"
    activate
    read -r bx by bw bh <<<"$(read_bounds)"
    screencapture -x -R "${bx},${by},${bw},${bh}" -t png "$out"
    # Downscale the (possibly Retina 2x) capture to exactly window points so
    # image pixels map 1:1 to click coordinates.
    sips -z "$bh" "$bw" "$out" >/dev/null
    echo "$out  (${bw}x${bh}; image px == window-relative click coords)"
    ;;
  click|dblclick|rclick|move)
    ensure_mouse
    read -r ax ay <<<"$(to_abs "$1" "$2")"
    "$MOUSE" "$cmd" "$ax" "$ay"
    ;;
  drag)
    ensure_mouse
    read -r ax1 ay1 <<<"$(to_abs "$1" "$2")"
    read -r ax2 ay2 <<<"$(to_abs "$3" "$4")"
    "$MOUSE" drag "$ax1" "$ay1" "$ax2" "$ay2"
    ;;
  menus)
    osascript -e "tell application \"System Events\" to tell process \"$PROC\" to return name of every menu of menu bar 1"
    ;;
  menu)
    activate
    if [[ $# -eq 1 ]]; then
      echo "menu: need at least <menu> <item>" >&2; exit 2
    elif [[ $# -eq 2 ]]; then
      # Top-level menu -> item
      osascript -e "tell application \"System Events\" to tell process \"$PROC\" to click menu item \"$2\" of menu 1 of menu bar item \"$1\" of menu bar 1"
    else
      # menu -> submenu -> item (e.g. Draw > Shapes > Rectangle)
      osascript -e "tell application \"System Events\" to tell process \"$PROC\" to click menu item \"$3\" of menu 1 of menu item \"$2\" of menu 1 of menu bar item \"$1\" of menu bar 1"
    fi
    ;;
  key)
    activate
    osascript -e "tell application \"System Events\" to keystroke $*"
    ;;
  type)
    activate
    osascript -e "tell application \"System Events\" to keystroke \"$1\""
    ;;
  *)
    grep -E '^#( |$)' "$0" | sed -E 's/^# ?//' | sed -n '1,40p'
    exit 2
    ;;
esac

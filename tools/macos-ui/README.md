# macOS UI driver for Hew (`hew-ui.sh`)

Lets an agent (or you) drive the **running Hew desktop app** on macOS for
UX testing — screenshot it, click toolbar/webview buttons, draw in the viewport,
and pull down native menus — without a human at the keyboard.

This exists because the desktop shell is Hew's primary testing target, but a
Tauri webview blocks the easy automation paths. The notes below record what
works, what doesn't, and why, so we don't re-derive it every session.

## One-time setup (permissions)

The **terminal app that runs these commands** (e.g. iTerm2) needs two macOS
privacy permissions. Grant them in System Settings ▸ Privacy & Security, then
fully restart the terminal:

- **Accessibility** — required for System Events (native menus, keystrokes) and
  for CGEvent mouse clicks to be delivered.
- **Screen Recording** — required for `screencapture` to capture window pixels
  (without it, captures come back blank/desktop-only).

Quick check that both are live:

```sh
# Accessibility: should print the frontmost app's name, not an error
osascript -e 'tell application "System Events" to name of first process whose frontmost is true'
# Screen Recording: open the PNG and confirm it shows real window content
screencapture -x /tmp/t.png && open /tmp/t.png
```

## Usage

Start the app first (`pnpm --dir shells/tauri dev`), then:

```sh
tools/macos-ui/hew-ui.sh shot /tmp/s.png       # screenshot the window
tools/macos-ui/hew-ui.sh click 188 88          # click toolbar "Rectangle"
tools/macos-ui/hew-ui.sh drag 250 300 480 470  # drag to draw in the viewport
tools/macos-ui/hew-ui.sh menus                 # list native menus
tools/macos-ui/hew-ui.sh menu "Tools" "Rotate" # click a native menu item
tools/macos-ui/hew-ui.sh menu "Draw" "Shapes" "Rectangle"   # nested
tools/macos-ui/hew-ui.sh key '"z" using {command down}'     # Cmd-Z (undo)
```

### Coordinates are window-relative points

Every command re-reads the live window position before acting, so a window that
has moved Just Works. **`shot` writes an image whose pixel dimensions equal the
window's point dimensions**, so a pixel you see at `(x, y)` in the screenshot is
exactly the argument to `click x y` — no Retina/backing-scale math. (A 1280×800
window → a 1280×800 PNG even on a 2× Retina display.)

Override the target process with `HEW_UI_PROC=Name` (default `hew-desktop`).

## How it works / why each piece

| Need | Tool | Notes |
|------|------|-------|
| Screenshot | `screencapture -R x,y,w,h` | window region, then `sips` downscales 2× Retina → 1× points |
| Native menus, keystrokes | AppleScript / System Events | Tauri menus are real `NSMenu`s, fully AX-accessible |
| Clicks & drags in the webview | `mouse.swift` (CGEvent) | **see below** |

**Why CGEvent for clicks, not System Events.** System Events' `click at {x, y}`
posts through the Accessibility API, which WKWebView content (Tauri's webview)
rejects — it fails with error **-25208**. Synthesizing low-level HID events with
CGEvent (`mouse.swift`) lands clicks anywhere, webview included. `mouse.swift`
is compiled to `.bin/mouse` on first use (git-ignored) and recompiled when the
source changes.

## Known app observations (from the first driving session)

- The toolbar/menu **arms** a tool (button highlights) but the **"Tool:" status
  label lags** — it only catches up after the first viewport interaction.
  Cosmetic, but a reactivity smell.
- Startup logs a benign `wasm-bindgen` deprecation: "using deprecated parameters
  for the initialization function; pass a single object instead."

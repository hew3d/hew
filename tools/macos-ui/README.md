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

Override the target process with `HEW_UI_PROC=Name` (default `hew`).

## Documentation screenshots (`hew-shot.sh`)

`hew-ui.sh shot` is built for automation, so its output is deliberately 1×.
For a picture headed for the user guide, use `hew-shot.sh` instead: it resizes
the window so the app content is exactly 1440×900 points, captures that region
without the title bar, and normalises the PNG the way
`app/scripts/capture-docs-screenshots.mjs` does — 8-bit RGB, no alpha, no
colour profile, deflate optimised. The result is a 2880×1800 file that sits
alongside the generated shots in `site/public/docs/` without looking out of
place. (Preview keeps the alpha channel and a Display P3 profile, which is why
its exports are several times larger.) The window goes back to its original
size and position afterwards.

```sh
tools/macos-ui/hew-shot.sh ~/Desktop/push-pull.png
tools/macos-ui/hew-shot.sh shot.png --titlebar    # include the macOS title bar
tools/macos-ui/hew-shot.sh shot.png --keep        # stay resized for a series
tools/macos-ui/hew-shot.sh shot.png --native      # no rescale to 2×
```

**The 2× only means something on a Retina display.** A 1440×900 region there is
2880×1800 real pixels; on a display running at its native resolution (a 4K panel
at 3840×2160, say) the same region is 1440×900 pixels and has to be upsampled to
reach the standard, which is softer than a generated shot and compresses worse.
The script prints a note when it upsamples. To get true 2× pixels regardless of
the display, drive the app in a browser and capture through Chromium instead:

```sh
pnpm --dir app dev
pnpm --dir app exec node scripts/capture-live-shot.mjs
```

That opens the dev app in a real window at 1440×900 with a device pixel ratio of
2 and waits; model whatever the shot needs, then type a name at the `shot>`
prompt to write `site/public/docs/<name>.png` at 2880×1800. The pixel ratio is
the browser's, not the screen's, so the output is identical in kind to the
generated shots.

It probes both `localhost` and `127.0.0.1` on ports 5173–5175 and 4173, because
those two names are **not** interchangeable here: `pnpm dev` leaves Vite on its
default host, which binds `[::1]` only, so `localhost` answers and `127.0.0.1`
is refused. When more than one server answers — a stale one from another
worktree is easy to leave running — it lists them and asks which, rather than
silently shooting the wrong build. `--url` skips all of that.

### Getting the pointer into the shot

A page screenshot renders the document, and the pointer is an OS compositor
layer, so it is never in one. `--cursor` injects an overlay that mirrors the
pointer into the page where the capture can see it. It reads the computed
`cursor` under the pointer, so the viewport's tool cursors — the
`url(data:image/svg+xml,…)` values `src/tools/toolIcons.ts` builds, copy badge
included — are drawn from the same image the app is showing, on the same
hotspot; only the plain CSS keywords are stand-ins.

Pair it with `--delay` to catch a gesture in progress:

```sh
pnpm --dir app exec node scripts/capture-live-shot.mjs --cursor --delay 4
```

Type a name, then take the mouse and be part-way through the push/pull drag when
the shutter goes — the inference chip, the hover highlight and the drag preview
are all live app state, so they land in the frame too. `<name> <secs>` overrides
the delay for one shot, and `cursor on|off` / `delay <secs>` change the standing
settings mid-session. The overlay sits on top of the real pointer, so you see
double while working; only the drawn one is in the PNG.

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

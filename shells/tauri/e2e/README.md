# Hew desktop E2E (WebdriverIO + tauri-driver)

End-to-end tests against the **real desktop binary** — the Tauri shell + WebKitGTK
webview + WASM kernel, exactly as a user runs it. This is the top of the test
pyramid (docs/DEVELOPMENT.md); keep it to a thin desktop-*wiring* smoke, because
modeling logic is already covered deterministically by the kernel/proptest suites
and the headless `window.__hew_test` harness.

The spec drives that same semantic harness, so it asserts logic (object counts,
`state_hash` round-trip), not canvas pixels.

## How it works

[`tauri-driver`](https://tauri.app/develop/tests/webdriver/) is Tauri's official
WebDriver intermediary: it starts the app and shims a W3C WebDriver session onto
the platform's webview driver. On **Linux** that driver is `WebKitWebDriver`;
WebdriverIO (`wdio.conf.ts`) talks to tauri-driver on port 4444 and runs the
Mocha specs in `specs/`.

> **macOS is unsupported** — WKWebView has no WebDriver. That's fine: CI is Linux.
> For local macOS repro use `tools/macos-ui/hew-ui.sh` (the CGEvent driver).

The binary under test is a **debug build with the semantic harness compiled in**
(`VITE_HEW_TEST=1`, so `window.__hew_test` installs in the otherwise-production
webview — see `app/src/App.tsx`). `wdio.conf.ts` derives the binary name from
`src-tauri/Cargo.toml`, so it survives the planned `hew-desktop` → `hew` rename.

## One-time host setup (Linux)

```bash
# WebKitWebDriver — the native driver tauri-driver delegates to (needs sudo).
# The package was renamed; both ship the same /usr/bin/WebKitWebDriver binary:
sudo apt-get install -y webkitgtk-webdriver   # newer Ubuntu
# sudo apt-get install -y webkit2gtk-driver   # Debian / older Ubuntu
# A headless session also needs a virtual display:
sudo apt-get install -y xvfb
# tauri-driver itself (no sudo; installs into ~/.cargo/bin):
cargo install tauri-driver --locked
```

The binary is always `WebKitWebDriver` regardless of package name; tauri-driver
finds it on `PATH`. If it isn't, point at it with
`WEBKIT_WEBDRIVER=/path/to/WebKitWebDriver`.

## Running

```bash
# 1. Build the harness-enabled debug binary (rebuilds the web app with the harness)
pnpm --dir shells/tauri e2e:build

# 2. Run the desktop E2E (start tauri-driver is handled by wdio.conf.ts)
pnpm --dir shells/tauri e2e
#    headless (no desktop session):  xvfb-run -a pnpm --dir shells/tauri e2e

# typecheck the config + specs without running them:
pnpm --dir shells/tauri e2e:typecheck
```

`e2e:build` runs `VITE_HEW_TEST=1 tauri build --debug --no-bundle`; the binary
lands at `src-tauri/target/debug/<name>` and `wdio.conf.ts` finds it (override
with `HEW_DESKTOP_BIN`). Without it, the run fails fast in `onPrepare` telling you
to build.

## CI

Heavier than the web E2E (a full desktop Rust build + Xvfb + the native driver),
and the roadmap scopes it "optional". It lives in its own workflow
(`.GitHub/workflows/desktop-e2e.yml`, `workflow_dispatch` — manual) rather than
the per-push pipeline, so it never slows or reds normal CI. The integrator wires
it onto a runner once the desktop image carries the system deps above.

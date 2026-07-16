import { defineConfig, devices } from '@playwright/test'

/**
 * Playwright config for Hew's web-build E2E (docs/DEVELOPMENT.md).
 *
 * Scope today: scaffolding + a single boot smoke test. The fuller smoke
 * flow (draw → push/pull → save/reload) is and drives through the
 * semantic harness (`window.__hew_test`) where possible, pixels otherwise.
 *
 * Browsers: WebKit is primary — it approximates the macOS Tauri WKWebView, our
 * least-tested-by-CI target (tauri-driver can't drive WKWebView).
 * Chromium is included for the input/render path. Firefox is available but kept
 * out of the default set to keep CI lean; add it when a regression warrants it.
 */

const PORT = 4173
const HOST = '127.0.0.1'
const baseURL = `http://${HOST}:${PORT}`
const isCI = !!process.env.CI

// Force Chromium onto the SwiftShader software rasterizer: headless CI has no
// GPU, and (for the visual goldens) software GL is deterministic enough to
// diff across machines where a real GPU is not. Harmless locally.
const SWIFTSHADER_ARGS = ['--enable-unsafe-swiftshader', '--use-gl=angle', '--use-angle=swiftshader']

export default defineConfig({
  testDir: './e2e',
  // Vitest owns *.test.ts; Playwright owns *.spec.ts — disjoint by extension so
  // neither runner ever loads the other's files. (vitest.config.ts mirrors this.)
  testMatch: '**/*.spec.ts',
  // The visual-goldens suite lives under e2e/visual/ and runs ONLY in
  // the pinned `visual` project below — the functional browser projects skip it
  // (goldens are GPU/runner-specific; see that project + e2e/visual/README.md).
  testIgnore: '**/visual/**',
  // Serial: the harness specs drive a stateful kernel + a software-GL (SwiftShader)
  // WebGL context; multiple parallel contexts contend and flake (docs/DEVELOPMENT.md).
  // The E2E suite is intentionally thin, so one worker is plenty.
  fullyParallel: false,
  workers: 1,
  forbidOnly: isCI,
  retries: isCI ? 1 : 0,
  // The opaque-WebGL-canvas smoke is GPU/timing sensitive; keep a real timeout.
  timeout: 30_000,
  expect: {
    timeout: 10_000,
    // Visual-goldens defaults. Software-GL (SwiftShader) is far more
    // deterministic than a real GPU but not bit-exact across machines, so allow
    // a small per-pixel + whole-image tolerance and freeze animations. Goldens
    // are authoritative only on the pinned runner (see e2e/visual/README.md).
    toHaveScreenshot: {
      animations: 'disabled',
      maxDiffPixelRatio: 0.02,
      threshold: 0.2,
    },
  },
  reporter: isCI ? [['list'], ['html', { open: 'never' }]] : 'list',
  use: {
    baseURL,
    trace: 'on-first-retry',
    screenshot: 'only-on-failure',
    // Suppress the welcome screen on bare launches: every spec boots a fresh
    // browser context (empty localStorage), and the dialog would otherwise
    // overlay the viewport in all of them. Specs that test the welcome screen
    // itself clear this key explicitly.
    storageState: {
      cookies: [],
      origins: [
        {
          origin: baseURL,
          localStorage: [
            { name: 'hew.settings.showWelcome', value: 'false' },
            // This suite runs Chromium on SwiftShader ON PURPOSE (see
            // SWIFTSHADER_ARGS); without this pin, the viewport's software-GL
            // detection (src/viewport/gpuCapability.ts) would fire in every
            // spec — a notice overlaying the canvas, antialias off, pixel
            // ratio capped — changing the visual goldens and screenshots.
            // Force the hardware profile so the suite keeps testing the
            // pinned rendering path; the detection logic itself is
            // unit-tested (gpuCapability.test.ts).
            { name: 'hew.debug.gpuProfile', value: 'hardware' },
          ],
        },
      ],
    },
  },

  projects: [
    {
      name: 'chromium',
      use: {
        ...devices['Desktop Chrome'],
        launchOptions: { args: SWIFTSHADER_ARGS },
      },
    },
    {
      name: 'webkit',
      use: { ...devices['Desktop Safari'] },
    },
    //  — visual-regression goldens on a *pinned* configuration: fixed
    // viewport + DPR=1 + SwiftShader software GL, so a golden PNG is reproducible
    // off a real GPU (docs/DEVELOPMENT.md). Runs ONLY the e2e/visual/ specs (the global
    // testIgnore keeps them out of the functional projects). Goldens are
    // authoritative on the pinned CI runner — see e2e/visual/README.md.
    {
      name: 'visual',
      testDir: './e2e/visual',
      testMatch: '**/*.spec.ts',
      testIgnore: [],
      use: {
        ...devices['Desktop Chrome'],
        viewport: { width: 800, height: 600 },
        deviceScaleFactor: 1,
        launchOptions: { args: SWIFTSHADER_ARGS },
      },
    },
  ],

  // Serve the app via the Vite dev server. Dev mode auto-enables the
  // `window.__hew_test` harness (`import.meta.env.DEV`), which the semantic E2E
  // tests drive — without shipping the harness in a production bundle or
  // plumbing a build flag. The production build is validated separately by
  // `vite build` in scripts/verify.sh. The dev server needs the WASM pkg built
  // (scripts/verify.sh / wasm-pack does this; present locally).
  webServer: {
    command: `pnpm exec vite --host ${HOST} --port ${PORT} --strictPort`,
    url: baseURL,
    reuseExistingServer: !isCI,
    timeout: 180_000,
  },
})

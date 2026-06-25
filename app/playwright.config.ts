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

export default defineConfig({
  testDir: './e2e',
  // Vitest owns *.test.ts; Playwright owns *.spec.ts — disjoint by extension so
  // neither runner ever loads the other's files. (vitest.config.ts mirrors this.)
  testMatch: '**/*.spec.ts',
  fullyParallel: true,
  forbidOnly: isCI,
  retries: isCI ? 1 : 0,
  // The opaque-WebGL-canvas smoke is GPU/timing sensitive; keep a real timeout.
  timeout: 30_000,
  expect: { timeout: 10_000 },
  reporter: isCI ? [['list'], ['html', { open: 'never' }]] : 'list',
  use: {
    baseURL,
    trace: 'on-first-retry',
    screenshot: 'only-on-failure',
  },

  projects: [
    {
      name: 'chromium',
      use: {
        ...devices['Desktop Chrome'],
        launchOptions: {
          // Headless CI has no GPU; allow the SwiftShader software-GL fallback so
          // the WebGL2 viewport context still initializes (docs/DEVELOPMENT.md — the
          // "pinned runner" software-GL approach). Harmless locally with a GPU.
          args: ['--enable-unsafe-swiftshader', '--use-gl=angle', '--use-angle=swiftshader'],
        },
      },
    },
    {
      name: 'webkit',
      use: { ...devices['Desktop Safari'] },
    },
  ],

  // Serve the built web app. In CI the build is produced by an earlier pipeline
  // step (so just preview the existing dist); locally we build first for a
  // one-command run. reuseExistingServer keeps local iteration fast.
  webServer: {
    command: isCI
      ? `pnpm exec vite preview --host ${HOST} --port ${PORT} --strictPort`
      : `pnpm build && pnpm exec vite preview --host ${HOST} --port ${PORT} --strictPort`,
    url: baseURL,
    reuseExistingServer: !isCI,
    timeout: 180_000,
  },
})

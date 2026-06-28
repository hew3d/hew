import { defineConfig } from 'vitest/config'
import react from '@vitejs/plugin-react'

export default defineConfig({
  plugins: [react()],
  test: {
    // Pure-logic suites (`.test.ts`) run on the lean `node` env; React component
    // suites (`.test.tsx`) need a DOM, so they run on `jsdom`. Keeping the
    // split avoids paying jsdom's setup cost on the ~440 logic tests.
    environment: 'node',
    environmentMatchGlobs: [['**/*.test.tsx', 'jsdom']],
    // jsdom defaults to the opaque `about:blank` origin, under which `localStorage`
    // is undefined; give it a real origin so storage-backed components work.
    environmentOptions: { jsdom: { url: 'http://localhost/' } },
    // jest-dom matchers + post-test React unmount. Harmless for the node suites.
    setupFiles: ['src/test/setup.ts'],
    // Vitest owns *.test.ts(x); Playwright owns e2e/*.spec.ts. Scoping the include
    // to *.test.ts(x) keeps Vitest from ever loading a Playwright spec (which
    // imports @playwright/test and would crash the unit run). The pure
    // canvas-projection helper under e2e/ is unit-tested here via its .test.ts.
    include: ['src/**/*.test.{ts,tsx}', 'e2e/**/*.test.ts'],
  },
})

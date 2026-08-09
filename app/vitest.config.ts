import { defineConfig } from 'vitest/config'
import react from '@vitejs/plugin-react'

// Pure-logic suites (`.test.ts`) run on the lean `node` env; React component
// suites (`.test.tsx`) need a DOM, so they run on `jsdom`. Keeping the split
// avoids paying jsdom's setup cost on the ~440 logic tests.
//
// This was one `environmentMatchGlobs` entry until Vitest 4 removed that
// option; `projects` is its replacement, and expresses the same split as two
// named runs. A `.test.ts` file that genuinely needs a DOM still opts in with
// a `// @vitest-environment jsdom` docblock, as `api/liveBridge.test.ts` and
// `text/fontSources.test.ts` do — that per-file override still takes
// precedence over the project's environment.
export default defineConfig({
  test: {
    projects: [
      {
        plugins: [react()],
        test: {
          name: 'node',
          environment: 'node',
          // Vitest owns *.test.ts(x); Playwright owns e2e/*.spec.ts. Scoping
          // the include to *.test.ts(x) keeps Vitest from ever loading a
          // Playwright spec (which imports @playwright/test and would crash
          // the unit run). The pure canvas-projection helper under e2e/ is
          // unit-tested here via its .test.ts.
          include: ['src/**/*.test.ts', 'e2e/**/*.test.ts'],
          // jsdom defaults to the opaque `about:blank` origin, under which
          // `localStorage` is undefined; give it a real origin so
          // storage-backed components work. Needed here too, for the
          // docblock-opted-in files above.
          environmentOptions: { jsdom: { url: 'http://localhost/' } },
          // jest-dom matchers + post-test React unmount. Harmless for the
          // node suites.
          setupFiles: ['src/test/setup.ts'],
        },
      },
      {
        plugins: [react()],
        test: {
          name: 'jsdom',
          environment: 'jsdom',
          include: ['src/**/*.test.tsx'],
          environmentOptions: { jsdom: { url: 'http://localhost/' } },
          setupFiles: ['src/test/setup.ts'],
        },
      },
    ],
  },
})

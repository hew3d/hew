import { defineConfig } from 'vitest/config'
import react from '@vitejs/plugin-react'

export default defineConfig({
  plugins: [react()],
  test: {
    environment: 'node',
    // Vitest owns *.test.ts; Playwright owns e2e/*.spec.ts. Scoping the include
    // to *.test.ts keeps Vitest from ever loading a Playwright spec (which
    // imports @playwright/test and would crash the unit run). The pure
    // canvas-projection helper under e2e/ is unit-tested here via its .test.ts.
    include: ['src/**/*.test.{ts,tsx}', 'e2e/**/*.test.ts'],
  },
})

import { defineConfig } from '@lingui/cli'
import { formatter } from '@lingui/format-po'
import { ALL_LOCALES, PSEUDO_LOCALE, SOURCE_LOCALE } from './src/i18n/locales'

// Lingui extraction + compilation config (docs/LOCALIZATION.md). The locale
// list lives in src/i18n/locales.ts so the app, this config, and the verify
// gate's coverage report cannot disagree about which languages exist.
export default defineConfig({
  sourceLocale: SOURCE_LOCALE,
  locales: [...ALL_LOCALES],
  catalogs: [
    {
      path: '<rootDir>/src/locales/{locale}/messages',
      include: ['src'],
      exclude: ['**/*.test.ts', '**/*.test.tsx', '**/wasm/pkg/**'],
    },
  ],
  // Line numbers in `#:` references churn on every unrelated edit and would
  // turn each catalog diff into noise; the file path alone is what a
  // translator needs to find a string.
  format: formatter({ lineNumbers: false }),
  // Generated from the English catalog at compile time: accented letters,
  // bracket markers, and 30% padding — anything still in plain English in
  // the app was never wrapped; anything clipped is a layout bug.
  pseudoLocale: {
    locale: PSEUDO_LOCALE,
    prepend: '⟦ ',
    append: ' ⟧',
    extend: 0.3,
    extendCharacter: '.',
  },
  fallbackLocales: { default: SOURCE_LOCALE },
})

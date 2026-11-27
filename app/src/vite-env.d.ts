/// <reference types="vite/client" />
/// <reference types="vite-plugin-pwa/client" />

declare module '*.svg?raw' {
  const content: string
  export default content
}

// Gettext catalogs imported through @lingui/vite-plugin (docs/LOCALIZATION.md):
// the plugin compiles the .po on the fly into the runtime message table.
declare module '*.po' {
  import type { Messages } from '@lingui/core'
  export const messages: Messages
}


// The Preview wall's build flag: a literal true/false defined at build time by
// vite.config.ts (and vitest.config.ts). Read only in src/preview/build.ts.
declare const __HEW_PREVIEW__: boolean

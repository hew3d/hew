/// <reference types="vite/client" />
/// <reference types="vite-plugin-pwa/client" />

interface ImportMetaEnv {
  /** Override the "Open on Phone" relay drop endpoint and QR receive
   *  origin at build time — see app/src/io/shareRelay.ts. Both optional;
   *  unset means the production defaults. */
  readonly VITE_HEW_SHARE_RELAY?: string
  readonly VITE_HEW_RECEIVE_ORIGIN?: string
}

declare module '*.svg?raw' {
  const content: string
  export default content
}

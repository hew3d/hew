import init, { version, demo_mesh, Scene, type DemoMesh } from './pkg/wasm_api.js'

// Memoize: only one initialization promise across the lifetime of the module.
let initPromise: Promise<void> | null = null

function getInitPromise(): Promise<void> {
  if (initPromise === null) {
    const wasmUrl = new URL('./pkg/wasm_api_bg.wasm', import.meta.url)
    initPromise = init(wasmUrl).then(() => undefined)
  }
  return initPromise
}

export interface KernelApi {
  version(): string
  demo_mesh(): DemoMesh
  /** Create a new authoritative Scene (docs/DEVELOPMENT.md B1) */
  newScene(): Scene
}

export async function loadKernel(): Promise<KernelApi> {
  await getInitPromise()
  return {
    version,
    demo_mesh,
    newScene: () => new Scene(),
  }
}

// Re-export Scene type so consumers can type their references without
// importing directly from the pkg (which requires the wasm init to have run).
export type { Scene } from './pkg/wasm_api.js'

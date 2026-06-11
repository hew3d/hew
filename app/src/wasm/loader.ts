import init, { version, demo_mesh, type DemoMesh } from './pkg/wasm_api.js'

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
}

export async function loadKernel(): Promise<KernelApi> {
  await getInitPromise()
  return { version, demo_mesh }
}

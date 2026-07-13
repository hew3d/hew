import init, {
  version,
  demo_mesh,
  Scene,
  type DemoMesh,
  init_logging,
  set_log_drain,
} from './pkg/wasm_api.js'
import { installKernelDrain, bridgeLogStore } from '../log/diagnosticLog'
import { registerScene, installFailureHandlers } from '../log/reproducerDump'
import { getDebugMode } from '../settings/debugMode'

// Memoize: only one initialization promise across the lifetime of the module.
let initPromise: Promise<void> | null = null

// Guards the log-drain install so it only ever runs once per session, even if
// loadKernel() is awaited from multiple call sites.
let logDrainInstalled = false

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
  if (!logDrainInstalled) {
    logDrainInstalled = true
    // : install the diagnostic-log sink — route kernel LogRecords into
    // the unified ring buffer and bridge the existing app LogStore into it.
    installKernelDrain(init_logging, set_log_drain)
    bridgeLogStore()
    // : install the auto-reproducer dump's uncaught error/rejection
    // handlers once per session (catches panics/traps without an
    // App/ErrorBoundary edit).
    installFailureHandlers()
  }
  return {
    version,
    demo_mesh,
    newScene: () => {
      const scene = new Scene()
      // : register every created Scene as the reproducer dump's
      // command-stream source and start recording it immediately.
      registerScene(scene)
      // : a freshly-created Scene (New/Open recreate it) inherits the
      // current Debug Mode setting, since torture mode is per-Scene state.
      scene.set_torture_mode(getDebugMode())
      return scene
    },
  }
}

// Re-export Scene type so consumers can type their references without
// importing directly from the pkg (which requires the wasm init to have run).
export type { Scene, DocChangeJs } from './pkg/wasm_api.js'

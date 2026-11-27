// The SDK's npm entry. Inside a Hew realm the runtime has already defined
// these globals before the plugin's own script runs; a plugin bundles this
// shim and gets the live objects.
export const hew = globalThis.hew
export const HewRefusal = globalThis.HewRefusal

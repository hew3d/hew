// Shared helpers for the headless Node replay runner.
//
// The wasm-pack `--target nodejs` output (`pkg-node/wasm_api.js`) is
// CommonJS (`require()`-based, uses `__dirname`). This package is ESM
// (`"type": "module"` in package.json), so we bridge with `createRequire`
// rather than dynamic `import()` — `import()` of a CJS module that itself
// calls `require('fs')`/`__dirname` internally works too, but `createRequire`
// is the documented Node interop path and keeps `require.resolve` semantics
// (relative path errors are clearer) for the generated bindings.
import { createRequire } from 'node:module';
import { existsSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
export const PKG_DIR = path.join(__dirname, 'pkg-node');
export const FIXTURES_DIR = path.join(__dirname, 'fixtures');

const PKG_JS = path.join(PKG_DIR, 'wasm_api.js');

/**
 * Loads the wasm-pack nodejs-target bindings, throwing a clear error if the
 * package hasn't been built yet (`npm run build`).
 */
export function loadWasm() {
  if (!existsSync(PKG_JS)) {
    throw new Error(
      `wasm bindings not found at ${PKG_JS}\n` +
        `Run "npm run build" (or the wasm-pack command in README.md) first.`,
    );
  }
  const require = createRequire(import.meta.url);
  return require(PKG_JS);
}

#!/usr/bin/env node
// Prints which flavor a built kernel package is, as the module itself
// reports it: `1` for a preview build's (`HEW_PREVIEW=1`), `0` for a
// production build's (docs/dev/DEVELOPMENT.md "Preview features"). Asks the
// compiled module (`preview_build()`), not a stamp file, so it reads the
// truth of whatever was built. Release jobs compare it with what the tag
// needs.
//
// usage: node scripts/wasm-flavor-of.mjs <wasm-pack --target web package dir>

import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { pathToFileURL } from "node:url";

const dir = process.argv[2];
if (!dir) {
  console.error("usage: node scripts/wasm-flavor-of.mjs <package dir>");
  process.exit(2);
}
const pkg = resolve(dir);
const mod = await import(pathToFileURL(resolve(pkg, "wasm_api.js")).href);
mod.initSync({ module: readFileSync(resolve(pkg, "wasm_api_bg.wasm")) });
if (typeof mod.preview_build !== "function") {
  console.error(`wasm-flavor-of: ${pkg} has no preview_build(); it predates the Preview wall`);
  process.exit(1);
}
console.log(mod.preview_build() ? "1" : "0");

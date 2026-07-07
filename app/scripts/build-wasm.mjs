#!/usr/bin/env node
// Build the Rust kernel to WASM for the app, but only when it is missing or
// stale. The compiled package (app/src/wasm/pkg) is a generated wasm-pack
// artifact and is gitignored, so a fresh checkout has none — running `vite`
// there fails with "Failed to resolve import ./pkg/wasm_api.js". This script
// runs automatically before `pnpm dev` / `pnpm build` so the kernel is always
// built, while skipping the ~30s rebuild when nothing in the kernel changed.
//
// Staleness rule: rebuild if the output is absent, or if any Rust source
// (*.rs, Cargo.toml/lock, rust-toolchain.toml) is newer than the built
// wasm_api.js. Pure-UI edits leave the WASM untouched and start instantly.

import { spawnSync } from "node:child_process";
import { readdirSync, statSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const appDir = dirname(dirname(fileURLToPath(import.meta.url))); // app/
const repoRoot = dirname(appDir);
const cratesDir = join(repoRoot, "crates");
const outFile = join(appDir, "src", "wasm", "pkg", "wasm_api.js");
const crate = join(cratesDir, "wasm-api");
// wasm-pack resolves --out-dir relative to the crate manifest dir, not cwd.
const outDir = join("..", "..", "app", "src", "wasm", "pkg");

function mtime(path) {
  try {
    return statSync(path).mtimeMs;
  } catch {
    return 0; // missing → treat as "not built yet"
  }
}

// Newest mtime among the kernel's build inputs.
function newestSourceMtime() {
  let newest = Math.max(
    mtime(join(repoRoot, "Cargo.toml")),
    mtime(join(repoRoot, "Cargo.lock")),
    mtime(join(repoRoot, "rust-toolchain.toml")),
  );
  const stack = [cratesDir];
  while (stack.length) {
    const dir = stack.pop();
    let entries;
    try {
      entries = readdirSync(dir, { withFileTypes: true });
    } catch {
      continue;
    }
    for (const entry of entries) {
      if (entry.name === "target" || entry.name === "node_modules") continue;
      const full = join(dir, entry.name);
      if (entry.isDirectory()) {
        stack.push(full);
      } else if (entry.name.endsWith(".rs") || entry.name === "Cargo.toml") {
        const m = mtime(full);
        if (m > newest) newest = m;
      }
    }
  }
  return newest;
}

const built = mtime(outFile);
if (built !== 0 && built >= newestSourceMtime()) {
  console.log("[build-wasm] kernel WASM is up to date — skipping build");
  process.exit(0);
}

console.log(
  built === 0
    ? "[build-wasm] kernel WASM not found — building (first run may take ~30s)…"
    : "[build-wasm] kernel source changed — rebuilding WASM…",
);

const result = spawnSync(
  "wasm-pack",
  ["build", crate, "--target", "web", "--out-dir", outDir],
  { stdio: "inherit", cwd: repoRoot },
);

if (result.error) {
  if (result.error.code === "ENOENT") {
    console.error(
      "\n[build-wasm] `wasm-pack` was not found on PATH.\n" +
        "Install it (https://rustwasm.github.io/wasm-pack/installer/), or build\n" +
        "the kernel manually:\n" +
        "  wasm-pack build crates/wasm-api --target web --out-dir ../../app/src/wasm/pkg\n",
    );
  } else {
    console.error("[build-wasm]", result.error.message);
  }
  process.exit(1);
}
process.exit(result.status ?? 1);

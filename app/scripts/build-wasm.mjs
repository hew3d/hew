#!/usr/bin/env node
// Build the Rust kernel to WASM for the app, but only when it is missing or
// stale. The compiled package (app/src/wasm/pkg) is a generated wasm-pack
// artifact and is gitignored, so a fresh checkout has none — running `vite`
// there fails with "Failed to resolve import ./pkg/wasm_api.js". This script
// runs automatically before `pnpm dev` / `pnpm build` so the kernel is always
// built, while skipping the rebuild when nothing in the kernel changed.
//
// The kernel has two flavors (docs/dev/DEVELOPMENT.md "Preview features"):
// a preview module and a production one, fixed at compile time by
// HEW_PREVIEW. The app refuses to start on a module of the other flavor, so
// this script builds the flavor the command that follows it will want, by
// the same rule Vite uses (app/src/preview/parseFlag.ts): `1` preview, `0`
// production, unset or empty the command's default. `--dev` (the dev
// server) defaults to preview; anything else (`vite build`) to production.
//
// Rebuild rule: rebuild if the output is absent, or if any build input
// (*.rs, Cargo.toml/lock, rust-toolchain.toml, preview-features.json, which
// crates/api compiles in, or this script) is newer than the built
// wasm_api.js, or any other generated file is missing from the package
// directory, or the module in place is the other flavor. The flavor is asked
// of the compiled module itself (scripts/wasm-flavor-of.mjs), so a package
// put there by any route is read correctly. Pure-UI edits leave the WASM
// untouched and start instantly.
//
// `--dev` builds skip wasm-opt (`--no-opt`), as the gate's preview builds do:
// it is the slowest step and only a shipped module needs it. Such a build
// leaves a `dev-build` mark beside the module, and a later `build` rebuilds
// over a marked module rather than bundle an unoptimized kernel. The gate's
// unoptimized modules carry the same mark (scripts/wasm-flavors.sh use); its
// own app build, which ships nothing, sets HEW_UNOPTIMIZED_WASM=1 to build
// on one as it is.

import { spawnSync } from "node:child_process";
import { readFileSync, readdirSync, rmSync, statSync, writeFileSync } from "node:fs";
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
    mtime(fileURLToPath(import.meta.url)),
    mtime(join(repoRoot, "Cargo.toml")),
    mtime(join(repoRoot, "Cargo.lock")),
    mtime(join(repoRoot, "rust-toolchain.toml")),
    mtime(join(repoRoot, "preview-features.json")),
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

const dev = process.argv.includes("--dev");

// The flavor the following command wants. Mirrors parsePreviewFlag in
// app/src/preview/parseFlag.ts, which this plain-Node script cannot import.
function wantsPreview() {
  const raw = process.env.HEW_PREVIEW;
  if (raw === undefined || raw === "") return dev;
  if (raw === "1") return true;
  if (raw === "0") return false;
  console.error(
    `[build-wasm] HEW_PREVIEW must be 1 (preview build), 0 (production build), or unset/empty (the command's default); got "${raw}".`,
  );
  process.exit(1);
}

// "preview" or "production" as the module in place reports it, or null when
// it cannot be asked (it predates the Preview wall, or does not load).
function flavorInPlace(packageDir) {
  const probe = spawnSync(
    process.execPath,
    [join(repoRoot, "scripts", "wasm-flavor-of.mjs"), packageDir],
    { encoding: "utf8", stdio: ["ignore", "pipe", "inherit"] },
  );
  if (probe.status !== 0) return null;
  const out = probe.stdout.trim();
  return out === "1" ? "preview" : out === "0" ? "production" : null;
}

const want = wantsPreview() ? "preview" : "production";
const built = mtime(outFile);
const packageDir = dirname(outFile);
const outputsPresent = ["wasm_api.js", "wasm_api.d.ts", "wasm_api_bg.wasm", "wasm_api_bg.wasm.d.ts", "package.json"]
  .every(name => mtime(join(packageDir, name)) !== 0);
const wasmFile = join(packageDir, "wasm_api_bg.wasm");
const devMark = join(packageDir, "dev-build");
const fresh = outputsPresent && built >= newestSourceMtime();
const have = fresh ? flavorInPlace(packageDir) : null;
// The mark names the module it was written for by that module's mtime, so a
// mark left behind by a build that some other route has since replaced does
// not count.
function devBuilt() {
  try {
    return readFileSync(devMark, "utf8").trim() === String(mtime(wasmFile));
  } catch {
    return false;
  }
}
const acceptUnoptimized = process.env.HEW_UNOPTIMIZED_WASM === "1";
const unoptimized = !dev && !acceptUnoptimized && fresh && have === want && devBuilt();
if (fresh && have === want && !unoptimized) {
  console.log(`[build-wasm] kernel WASM is up to date (${want}) — skipping build`);
  process.exit(0);
}

console.log(
  built === 0
    ? `[build-wasm] kernel WASM not found — building the ${want} module (the first build takes several minutes)…`
    : !outputsPresent
      ? `[build-wasm] the kernel WASM package is incomplete — rebuilding the ${want} module…`
      : !fresh
        ? `[build-wasm] kernel source changed — rebuilding the ${want} module…`
        : unoptimized
          ? `[build-wasm] the kernel WASM in place was built without optimization (for the dev server or the gate) — rebuilding the ${want} module…`
          : `[build-wasm] the kernel WASM in place is ${have ?? "not a module this build can read"}; this command needs ${want} — rebuilding…`,
);

// The Rust build reads HEW_PREVIEW itself and treats only exactly `1` as on,
// so the variable is set for a preview module and removed for a production one.
const env = { ...process.env };
if (want === "preview") env.HEW_PREVIEW = "1";
else delete env.HEW_PREVIEW;

const result = spawnSync(
  "wasm-pack",
  ["build", crate, "--target", "web", ...(dev ? ["--no-opt"] : []), "--out-dir", outDir],
  { stdio: "inherit", cwd: repoRoot, env },
);

if (result.error) {
  if (result.error.code === "ENOENT") {
    console.error(
      "\n[build-wasm] `wasm-pack` was not found on PATH.\n" +
        "Install the pinned version with scripts/install-wasm-pack.sh, then run\n" +
        "this command again.\n",
    );
  } else {
    console.error("[build-wasm]", result.error.message);
  }
  process.exit(1);
}
if (result.status === 0) {
  // The stamp scripts/wasm-flavors.sh leaves beside a package it built.
  writeFileSync(join(packageDir, "preview-build"), want === "preview" ? "1\n" : "0\n");
  if (dev) writeFileSync(devMark, `${mtime(wasmFile)}\n`);
  else rmSync(devMark, { force: true });
}
process.exit(result.status ?? 1);

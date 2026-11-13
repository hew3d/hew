#!/usr/bin/env node
// Build hew-cli and stage it as this Tauri build's `externalBin` sidecar,
// named for the host target triple the way tauri-build's build.rs requires
// (bundle.externalBin: ["binaries/hew-cli"] in tauri.conf.json). Runs before
// every `tauri dev`/`build`/`e2e:build` — declaring externalBin makes the
// build script hard-fail the moment the Tauri crate is compiled if the
// matching binaries/hew-cli-<triple>[.exe] file isn't already on disk, so
// staging it has to happen before that, not just before bundling.
//
// `cargo build` is its own staleness check (fast no-op when hew-cli's
// sources haven't changed), so unlike build-wasm.mjs this doesn't need a
// separate mtime comparison.

import { spawnSync } from "node:child_process";
import { mkdirSync, copyFileSync, existsSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const tauriDir = dirname(dirname(fileURLToPath(import.meta.url))); // shells/tauri/
const repoRoot = dirname(dirname(tauriDir));
const binariesDir = join(tauriDir, "src-tauri", "binaries");

function run(cmd, args) {
  const result = spawnSync(cmd, args, {
    cwd: repoRoot,
    encoding: "utf8",
    maxBuffer: 32 * 1024 * 1024,
    stdio: ["ignore", "pipe", "inherit"],
  });
  if (result.error) {
    console.error(`[stage-cli-sidecar] failed to run \`${cmd}\`: ${result.error.message}`);
    process.exit(1);
  }
  if (result.status !== 0) {
    console.error(`[stage-cli-sidecar] \`${cmd} ${args.join(" ")}\` exited ${result.status}`);
    process.exit(result.status ?? 1);
  }
  return result.stdout;
}

console.log("[stage-cli-sidecar] building hew-cli (release)…");
const messages = run("cargo", ["build", "--release", "-p", "hew-cli", "--message-format=json-render-diagnostics"]);
// Cargo reports the actual executable, including custom target directories.
const artifact = messages.split("\n").filter(Boolean).map(line => JSON.parse(line))
  .find(message => message.reason === "compiler-artifact" && message.target.name === "hew-cli" && message.executable);
if (!artifact) {
  console.error("[stage-cli-sidecar] Cargo did not report the hew-cli executable");
  process.exit(1);
}

const hostLine = run("rustc", ["-vV"])
  .split("\n")
  .find((line) => line.startsWith("host:"));
if (!hostLine) {
  console.error("[stage-cli-sidecar] could not determine host target triple from `rustc -vV`");
  process.exit(1);
}
const triple = hostLine.slice("host:".length).trim();
const ext = triple.includes("windows") ? ".exe" : "";

mkdirSync(binariesDir, { recursive: true });
const destination = join(binariesDir, `hew-cli-${triple}${ext}`);
if (existsSync(destination) && readFileSync(destination).equals(readFileSync(artifact.executable))) {
  console.log(`[stage-cli-sidecar] binaries/hew-cli-${triple}${ext} is unchanged`);
} else {
  copyFileSync(artifact.executable, destination);
  console.log(`[stage-cli-sidecar] staged binaries/hew-cli-${triple}${ext}`);
}

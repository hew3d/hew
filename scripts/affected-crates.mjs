#!/usr/bin/env node
// Which workspace crates does a set of changed files reach?
//
// Reads repo-relative paths on stdin, one per line, and prints the package
// name of every workspace member that owns one of them or depends on one
// that does (normal, build and dev dependencies, transitively), one per
// line, sorted. scripts/verify-changed.sh tests exactly that set.
//
// With `--owners` it prints only the crates that own a changed file, without
// their dependents.
//
// A change to a file every crate builds from (the root manifest, the lock
// file, the toolchain pin, cargo's own config) reaches the whole workspace.
// So does a change under crates/ that no member owns (a vendored path
// dependency). preview-features.json is compiled into crates/api, so it
// counts as a change to that crate, and a fixture another crate's tests read
// counts as a change to that crate too (READ_ACROSS).

import { spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { dirname, relative, sep } from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = dirname(dirname(fileURLToPath(import.meta.url)));
const ownersOnly = process.argv.includes("--owners");

const changed = readFileSync(0, "utf8")
  .split("\n")
  .map(line => line.trim())
  .filter(Boolean);

const meta = spawnSync("cargo", ["metadata", "--format-version", "1", "--no-deps", "--offline"], {
  cwd: repoRoot,
  encoding: "utf8",
  maxBuffer: 256 * 1024 * 1024,
  stdio: ["ignore", "pipe", "inherit"],
});
if (meta.status !== 0) {
  console.error("affected-crates: cargo metadata failed");
  process.exit(1);
}
const { packages, workspace_members: memberIds } = JSON.parse(meta.stdout);
const members = packages.filter(pkg => memberIds.includes(pkg.id));
const names = new Set(members.map(pkg => pkg.name));

// Each member's directory, repo-relative with forward slashes and a trailing
// slash, longest first so a nested crate wins over the one that contains it.
const dirs = members
  .map(pkg => ({
    name: pkg.name,
    dir: relative(repoRoot, dirname(pkg.manifest_path)).split(sep).join("/") + "/",
  }))
  .sort((a, b) => b.dir.length - a.dir.length);

const GLOBAL = [
  /^Cargo\.toml$/,
  /^Cargo\.lock$/,
  /^rust-toolchain\.toml$/,
  /^clippy\.toml$/,
  /^\.?rustfmt\.toml$/,
  /^\.cargo\//,
];

// Files a crate's tests read from outside its own directory. The kernel
// loads every .hew fixture in the repository under its strictest validator
// (crates/kernel/tests/tangent_loop_specs.rs), and crates/wasm-api, which
// depends on the kernel, reads the app's E2E fixtures.
const READ_ACROSS = [
  { re: /^app\/e2e\/fixtures\//, crate: "kernel" },
  { re: /^app\/public\/samples\//, crate: "kernel" },
  { re: /^crates\/[^/]+\/tests\/fixtures\/.*\.hew$/, crate: "kernel" },
];

const owners = new Set();
let everything = false;
for (const file of changed) {
  if (GLOBAL.some(re => re.test(file))) {
    everything = true;
    continue;
  }
  if (file === "preview-features.json") {
    if (names.has("api")) owners.add("api");
    continue;
  }
  for (const { re, crate } of READ_ACROSS) {
    if (re.test(file) && names.has(crate)) owners.add(crate);
  }
  const owner = dirs.find(({ dir }) => file.startsWith(dir));
  if (owner) owners.add(owner.name);
  // Under crates/ but in no member: a vendored path dependency, whose
  // dependents this script does not track.
  else if (file.startsWith("crates/")) everything = true;
}

let result;
if (everything) {
  result = names;
} else if (ownersOnly) {
  result = owners;
} else {
  // dependents[x] = the members that depend on x directly.
  const dependents = new Map([...names].map(name => [name, []]));
  for (const pkg of members) {
    for (const dep of pkg.dependencies) {
      if (names.has(dep.name) && dep.name !== pkg.name) dependents.get(dep.name).push(pkg.name);
    }
  }
  result = new Set(owners);
  const queue = [...owners];
  while (queue.length) {
    for (const dependent of dependents.get(queue.pop())) {
      if (!result.has(dependent)) {
        result.add(dependent);
        queue.push(dependent);
      }
    }
  }
}

for (const name of [...result].sort()) console.log(name);

#!/usr/bin/env node
//  headless Node replay runner — the CI regression gate for the
// recording/replay surface (docs/DIAGNOSTICS.md). Also the
// bug -> test pipeline entry point (`--freeze`).
//
// Default mode: loads every tools/replay-runner/fixtures/*.json, replays each
// into a fresh Scene, and asserts the returned state_hash matches the
// fixture's golden_hash. Prints PASS/FAIL per fixture; exits 0 iff all pass.
//
// `--generate` delegates to generate.mjs (drives a fresh Scene to build a
// representative model, records it, and writes a new fixture file).
//
// `--freeze <recording.json> <fixture-name> [--force]` delegates to
// freeze.mjs ( bug -> test pipeline: validates a captured reproducer
// replays to its own golden, then commits it verbatim as a fixture).
import { readdir, readFile } from 'node:fs/promises';
import path from 'node:path';
import { loadWasm, FIXTURES_DIR } from './lib.mjs';

async function main() {
  if (process.argv.includes('--generate')) {
    const { generateAll } = await import('./generate.mjs');
    await generateAll();
    return;
  }

  if (process.argv.includes('--freeze')) {
    const { freeze } = await import('./freeze.mjs');
    const flagIndex = process.argv.indexOf('--freeze');
    const force = process.argv.includes('--force');
    const rest = process.argv.slice(flagIndex + 1).filter((a) => a !== '--force');
    const [recordingPath, fixtureName] = rest;
    if (!recordingPath || !fixtureName) {
      console.error('usage: node run.mjs --freeze <recording.json> <fixture-name> [--force]');
      process.exitCode = 1;
      return;
    }
    await freeze(recordingPath, fixtureName, { force });
    return;
  }

  const wasm = loadWasm();

  let names;
  try {
    names = (await readdir(FIXTURES_DIR)).filter((f) => f.endsWith('.json')).sort();
  } catch (err) {
    if (err.code === 'ENOENT') {
      console.error(`No fixtures directory at ${FIXTURES_DIR}`);
      process.exitCode = 1;
      return;
    }
    throw err;
  }

  if (names.length === 0) {
    console.error(`No fixtures found in ${FIXTURES_DIR} — run "npm run generate" first.`);
    process.exitCode = 1;
    return;
  }

  let allPassed = true;
  for (const name of names) {
    const fixturePath = path.join(FIXTURES_DIR, name);
    const json = await readFile(fixturePath, 'utf8');

    // `golden_hash` is a u64 (docs/DIAGNOSTICS.md); values above
    // Number.MAX_SAFE_INTEGER lose precision through JSON.parse's double-
    // precision number type, so pull the literal digits out of the raw text
    // with a regex and hand them to BigInt directly. (Sanity-checked: e.g.
    // 16685420354669910861 round-trips through JSON.parse as
    // 16685420354669910016 — a real, silent corruption.)
    const match = json.match(/"golden_hash"\s*:\s*(\d+)/);
    if (!match) {
      console.log(`FAIL  ${name}  (no golden_hash field found)`);
      allPassed = false;
      continue;
    }
    const golden = BigInt(match[1]);

    const scene = new wasm.Scene();
    try {
      const got = scene.replay(json);
      if (got === golden) {
        console.log(`PASS  ${name}  state_hash=${got}`);
      } else {
        console.log(`FAIL  ${name}  expected golden_hash=${golden} got state_hash=${got}`);
        allPassed = false;
      }
    } catch (err) {
      console.log(`FAIL  ${name}  (replay threw: ${err})`);
      allPassed = false;
    } finally {
      scene.free?.();
    }
  }

  if (!allPassed) {
    process.exitCode = 1;
  }
}

main().catch((err) => {
  console.error(err);
  process.exitCode = 1;
});

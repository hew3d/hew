#!/usr/bin/env node
//  headless Node replay runner — the CI regression gate for the
// recording/replay surface (docs/dev/DIAGNOSTICS.md). Also the
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

/**
 * Post-replay probes, by fixture name. The golden state_hash covers neither
 * the pick scope nor the render mode, so a fixture whose point is a scoped
 * pick asserts it here: after the replay the scene must still hold the
 * recorded scope and answer the same picks. Returns a failure message or null.
 */
const POST_REPLAY_PROBES = {
  'xray-edit-scope': (scene) => {
    const pick = (x, y) => {
      const hit = scene.pick_face(x, y, 8, 0, 0, -1);
      if (hit === undefined) return null;
      const object = hit.object();
      hit.free();
      return object;
    };
    // The recorded scope is still set after the replay. Ask the scoped
    // picks first, then clear it to learn the nearest face (A's top).
    scene.set_render_mode('shaded');
    const scopedShaded = pick(1.5, 1);
    const behindShaded = pick(0.4, 1);
    scene.set_render_mode('xray');
    const scopedXray = pick(1.5, 1);
    const behindXray = pick(0.4, 1);
    scene.clear_pick_scope();
    const nearest = pick(1.5, 1);
    if (nearest === null) return 'X-ray pick through A found nothing';
    if (scopedXray === null || scopedXray === nearest) {
      return `X-ray pick through A with the replayed scope returned ${scopedXray}, expected the scoped box (not ${nearest})`;
    }
    if (scopedShaded !== scopedXray) {
      return `Shaded pick through A with the replayed scope returned ${scopedShaded}, expected the scoped box ${scopedXray}`;
    }
    if (behindXray !== nearest) return `X-ray pick with nothing in scope behind returned ${behindXray}, expected ${nearest}`;
    if (behindShaded !== nearest) return `Shaded pick with nothing in scope behind returned ${behindShaded}, expected ${nearest}`;
    scene.set_render_mode('shaded');
    if (pick(1.5, 1) !== nearest) return 'Shaded pick after clear_pick_scope must return the nearest face';
    scene.set_render_mode('wireframe');
    scene.set_pick_scope(new BigUint64Array([scopedXray]), new BigUint64Array([]), false);
    if (pick(1.5, 1) !== null) return 'Wireframe picks no face, scope or not';
    scene.clear_pick_scope();
    return null;
  },
};

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

    // `golden_hash` is a u64 (docs/dev/DIAGNOSTICS.md); values above
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
      const probe = POST_REPLAY_PROBES[path.basename(name, '.json')];
      const probeFailure = got === golden && probe ? probe(scene) : null;
      if (probeFailure !== null) {
        console.log(`FAIL  ${name}  (post-replay probe: ${probeFailure})`);
        allPassed = false;
      } else if (got === golden) {
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

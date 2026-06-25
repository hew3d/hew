#!/usr/bin/env node
//  bug -> test pipeline: freezes a captured reproducer (a `Recording`
// JSON — what `Scene.take_recording()` emits, and what a future bug-report
// bundle, M17, will contain) into a committed replay regression fixture.
//
// Flow (docs/DEVELOPMENT.md): a bug is reported with its recording -> the
// recording is reproduced -> the bug is fixed -> `freeze` validates the
// recording still replays to its own golden under the fixed kernel and, if
// so, commits it verbatim as `fixtures/<name>.json` -> `run.mjs`/CI replays
// it forever.
//
// Can be run directly (`node freeze.mjs <recording.json> <fixture-name>
// [--force]`) or via `node run.mjs --freeze <recording.json> <fixture-name>
// [--force]`, which dynamically imports `freeze()` below (mirrors how
// `run.mjs --generate` delegates to `generate.mjs`).
import { readFile, writeFile, mkdir } from 'node:fs/promises';
import path from 'node:path';
import { loadWasm, FIXTURES_DIR } from './lib.mjs';

// Must match RECORDING_FORMAT_VERSION (docs/DIAGNOSTICS.md,
// crates/wasm-api/src/recording.rs). v2 = typed replayable calls.
const RECORDING_FORMAT_VERSION = 2;

/**
 * Sanitizes a fixture name into a safe `<name>.json` basename: rejects empty
 * names and path separators/traversal, and appends `.json` if not already
 * present. Throws on anything unsafe rather than silently coercing it.
 */
function sanitizeFixtureName(fixtureName) {
  if (!fixtureName || typeof fixtureName !== 'string') {
    throw new Error('fixture name must be a non-empty string');
  }
  if (fixtureName.includes('/') || fixtureName.includes('\\') || fixtureName.includes('\0')) {
    throw new Error(`fixture name must not contain path separators: "${fixtureName}"`);
  }
  if (fixtureName === '.' || fixtureName === '..') {
    throw new Error(`invalid fixture name: "${fixtureName}"`);
  }
  const withExt = fixtureName.endsWith('.json') ? fixtureName : `${fixtureName}.json`;
  // path.basename strips any residual directory components defensively;
  // re-check it matches what we built (catches anything the separator check
  // above missed, e.g. encoded traversal).
  const base = path.basename(withExt);
  if (base !== withExt) {
    throw new Error(`invalid fixture name: "${fixtureName}"`);
  }
  return base;
}

/**
 * Reads, validates, and replay-checks a captured reproducer at
 * `recordingPath`, then writes it verbatim to `fixtures/<fixtureName>.json`.
 *
 * Refuses (throws) if:
 * - the recording's `version` isn't the current `RECORDING_FORMAT_VERSION`,
 * - it's missing a `golden_hash` or a `calls` array,
 * - replaying it into a fresh `Scene` doesn't reproduce its own
 *   `golden_hash` (non-deterministic, or recorded against different kernel
 *   code — freezing it would commit a fixture that can't reproduce), or
 * - the target fixture already exists and `force` is not set.
 *
 * On success, writes the exact bytes read (never a re-serialized copy) to
 * `fixtures/<fixtureName>.json` and returns the output path.
 */
export async function freeze(recordingPath, fixtureName, { force = false } = {}) {
  const json = await readFile(recordingPath, 'utf8');

  // Parse for structural validation only — the bytes we eventually write are
  // the raw `json` text read above, never this parsed-and-reserialized copy.
  let parsed;
  try {
    parsed = JSON.parse(json);
  } catch (err) {
    throw new Error(`${recordingPath} is not valid JSON: ${err.message}`);
  }

  if (parsed.version !== RECORDING_FORMAT_VERSION) {
    throw new Error(
      `${recordingPath} has version=${parsed.version}, expected ` +
        `${RECORDING_FORMAT_VERSION} (docs/DIAGNOSTICS.md) — refusing to freeze ` +
        `a recording from a different format version.`,
    );
  }
  if (!Array.isArray(parsed.calls)) {
    throw new Error(`${recordingPath} is missing a "calls" array`);
  }

  // golden_hash is a u64 (docs/DIAGNOSTICS.md); JSON.parse silently
  // corrupts values above Number.MAX_SAFE_INTEGER (e.g.
  // 16685420354669910861 becomes 16685420354669910016), so pull the literal
  // digits out of the raw text via regex and hand them to BigInt directly —
  // never trust `parsed.golden_hash` (run.mjs/generate.mjs document the same
  // trap).
  const match = json.match(/"golden_hash"\s*:\s*(\d+)/);
  if (!match) {
    throw new Error(`${recordingPath} is missing a "golden_hash" field`);
  }
  const golden = BigInt(match[1]);

  const fixtureBase = sanitizeFixtureName(fixtureName);
  const outPath = path.join(FIXTURES_DIR, fixtureBase);

  if (!force) {
    try {
      await readFile(outPath);
      throw new Error(
        `${outPath} already exists — refusing to overwrite without --force.`,
      );
    } catch (err) {
      if (err.code !== 'ENOENT') {
        throw err;
      }
      // ENOENT is the expected case (no existing fixture); fall through.
    }
  }

  // Replay into a fresh Scene and require the replayed state_hash to match
  // the recording's own declared golden. A mismatch means this reproducer is
  // non-deterministic or was captured against different kernel code — either
  // way it cannot be trusted as a regression fixture.
  const wasm = loadWasm();
  const scene = new wasm.Scene();
  let got;
  try {
    got = scene.replay(json);
  } catch (err) {
    throw new Error(
      `${recordingPath} failed to replay — refusing to freeze a reproducer ` +
        `that doesn't replay cleanly: ${err}`,
    );
  } finally {
    scene.free?.();
  }

  if (got !== golden) {
    throw new Error(
      `${recordingPath} does NOT reproduce its own golden_hash ` +
        `(declared golden_hash=${golden}, replay produced state_hash=${got}) — ` +
        `refusing to freeze. This reproducer is non-deterministic or was ` +
        `captured against different kernel code; a fixture that can't ` +
        `reproduce its own golden is worthless as a regression test.`,
    );
  }

  await mkdir(FIXTURES_DIR, { recursive: true });
  // Write the exact bytes read from recordingPath — never a re-serialized
  // copy — so golden_hash's u64 digits never pass through JSON.stringify.
  await writeFile(outPath, json, 'utf8');

  console.log(
    `Froze ${recordingPath} -> ${outPath} (golden_hash=${golden} confirmed by replay)`,
  );
  console.log(`${fixtureBase} is now part of the CI gate (run.mjs replays it on every run).`);
  return outPath;
}

// Allow `node freeze.mjs <recording.json> <fixture-name> [--force]` directly,
// in addition to `run.mjs --freeze`.
if (import.meta.url === `file://${process.argv[1]}`) {
  const args = process.argv.slice(2).filter((a) => a !== '--force');
  const force = process.argv.includes('--force');
  const [recordingPath, fixtureName] = args;
  if (!recordingPath || !fixtureName) {
    console.error('usage: node freeze.mjs <recording.json> <fixture-name> [--force]');
    process.exitCode = 1;
  } else {
    freeze(recordingPath, fixtureName, { force }).catch((err) => {
      console.error(err.message ?? err);
      process.exitCode = 1;
    });
  }
}

#!/usr/bin/env node
// The wasm leg of the determinism golden (docs/dev/DEVELOPMENT.md, "The
// determinism golden"). Drives the same models through the wasm kernel that
// crates/api/tests/determinism_golden.rs drives natively, and compares each
// row's state hash and tessellation hash with corpus/determinism/golden.tsv.
//
//   node determinism.mjs                 print every differing row; exit 0
//   node determinism.mjs --strict        the same, but exit 1 if any differ,
//                                        or if a preview module leaves a row
//                                        unavailable
//   node determinism.mjs --out <file>    also write this run's table to <file>
//
// The wasm module must be built first and copied into pkg-node:
//
//   scripts/wasm-flavors.sh build preview nodejs
//   scripts/wasm-flavors.sh use-node preview
//
// A production module has no Preview commands, so a row that needs one is
// reported as unavailable, not as a difference, and `--strict` checks the
// rest. A preview module has every command, so there an unavailable row
// fails `--strict`.
import { createRequire } from 'node:module';
import { readFileSync, readdirSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { PKG_DIR, loadWasm } from './lib.mjs';

const ROOT = path.join(path.dirname(fileURLToPath(import.meta.url)), '..', '..');
const CURVES_DIR = path.join(ROOT, 'corpus', 'curves');
const DETERMINISM_DIR = path.join(ROOT, 'corpus', 'determinism');

// ------------------------------------------------------------------ hash

// FNV-1a 64 over bytes, as two uint32 halves (a BigInt per byte is too slow
// for a mesh). The prime is 2^40 + 0x1b3.
class Fnv {
  constructor() {
    this.hi = 0xcbf29ce4;
    this.lo = 0x84222325;
  }
  bytes(u8) {
    let { hi, lo } = this;
    for (let i = 0; i < u8.length; i++) {
      lo = (lo ^ u8[i]) >>> 0;
      const low = lo * 0x1b3;
      const carry = Math.floor(low / 4294967296);
      const newHi = (hi * 0x1b3 + carry + (lo & 0xffffff) * 256) % 4294967296;
      lo = low % 4294967296;
      hi = newHi;
    }
    this.hi = hi;
    this.lo = lo;
  }
  u64(n) {
    const b = new Uint8Array(8);
    new DataView(b.buffer).setBigUint64(0, BigInt(n), true);
    this.bytes(b);
  }
  typed(a) {
    this.u64(a.length);
    this.bytes(new Uint8Array(a.buffer, a.byteOffset, a.byteLength));
  }
  hex() {
    return this.hi.toString(16).padStart(8, '0') + this.lo.toString(16).padStart(8, '0');
  }
}

// Returns { hash, failed }: failed says an object would not tessellate, which
// the row's outcome then names.
function meshHash(scene) {
  const h = new Fnv();
  let failed = false;
  const ids = scene.object_ids();
  h.u64(ids.length);
  for (const id of ids) {
    let mesh;
    try {
      mesh = scene.object_mesh(id);
    } catch {
      h.u64(0);
      failed = true;
      continue;
    }
    h.u64(1);
    for (const read of ['positions', 'normals', 'indices', 'uvs', 'edge_positions', 'soft_edge_positions', 'tangent_edge_positions']) {
      h.typed(mesh[read]());
    }
    mesh.free();
  }
  return { hash: h.hex(), failed };
}

// -------------------------------------------------------------- frames

// A length with a unit, in meters: 3mm, 2.5cm, 0.1m, 1/8in.
const NUMBER = /^[+-]?(\d+\.?\d*|\.\d+)([eE][+-]?\d+)?$/;
function parseLength(s) {
  s = s.trim();
  let number;
  let scale;
  if (s.endsWith('mm')) [number, scale] = [s.slice(0, -2), 1e-3];
  else if (s.endsWith('cm')) [number, scale] = [s.slice(0, -2), 1e-2];
  else if (s.endsWith('in')) [number, scale] = [s.slice(0, -2), 0.0254];
  else if (s.endsWith('m')) [number, scale] = [s.slice(0, -1), 1.0];
  else return null;
  number = number.trim();
  let value;
  const slash = number.indexOf('/');
  if (slash >= 0) {
    const a = number.slice(0, slash).trim();
    const b = number.slice(slash + 1).trim();
    if (!NUMBER.test(a) || !NUMBER.test(b) || Number(b) === 0) return null;
    value = Number(a) / Number(b);
  } else {
    if (!NUMBER.test(number)) return null;
    value = Number(number);
  }
  return Number.isFinite(value) ? value * scale : null;
}

// Replaces {"$ref": "label#/pointer"} with the labeled result's value and
// every string that is a length with a unit with meters.
function substitute(v, labels) {
  if (Array.isArray(v)) return v.map((x) => substitute(x, labels));
  if (typeof v === 'string') {
    const m = parseLength(v);
    return m !== null && /^[0-9-]/.test(v) ? m : v;
  }
  if (v && typeof v === 'object') {
    const keys = Object.keys(v);
    if (keys.length === 1 && keys[0] === '$ref') {
      const [label, pointer] = v.$ref.split('#');
      let cur = labels.get(label);
      if (cur === undefined) throw new Error(`no earlier frame is labeled ${label}`);
      for (const part of pointer.split('/').slice(1)) {
        cur = cur?.[part.replace(/~1/g, '/').replace(/~0/g, '~')];
        if (cur === undefined) throw new Error(`${v.$ref} names nothing`);
      }
      return cur;
    }
    return Object.fromEntries(keys.map((k) => [k, substitute(v[k], labels)]));
  }
  return v;
}

// Raised for what makes the whole run meaningless (no connection), not one
// row: it is not turned into a panic row.
class Fatal extends Error {}

// The text of a frame. JSON.stringify writes -0 as 0, but the files hold
// numbers like -0.0 and the native leg keeps the sign, so -0 is written as
// -0.0. Every other number prints as its shortest round-trip form, which the
// kernel's JSON parser (float_roundtrip) reads back as the same f64.
const NEG_ZERO = '__negative_zero__';
function frameText(frame) {
  const text = JSON.stringify(frame, (_k, v) => (Object.is(v, -0) ? NEG_ZERO : v));
  return text.replaceAll(`"${NEG_ZERO}"`, '-0.0');
}

class Session {
  constructor(wasm) {
    this.scene = new wasm.Scene();
    this.conn = this.scene.api_connection_open();
    this.nextId = 0;
    this.labels = new Map();
    for (const [method, params] of [['hew.meta.hello', { protocol: 1 }], ['hew.doc.attach', {}]]) {
      const out = this.call(method, params);
      if (out.ok === undefined) throw new Fatal(`${method} failed: ${out.refused ?? out.broken}`);
    }
  }
  // { ok: result } | { refused: code } | { broken: text }
  call(method, params) {
    this.nextId += 1;
    const reply = this.scene.api_dispatch(
      this.conn,
      frameText({ jsonrpc: '2.0', id: this.nextId, method, params }),
    );
    if (reply === undefined || reply === null) throw new Error(`${method} did not reply`);
    // The wasm boundary tags the JSON-RPC response: {"reply": "<json>"},
    // plus "dry_run", or {"pending": ...} for a forward. Native expects a
    // plain reply.
    const tagged = JSON.parse(reply);
    if (tagged.pending || tagged.dry_run || typeof tagged.reply !== 'string') {
      throw new Error(`${method} did not reply`);
    }
    const r = JSON.parse(tagged.reply);
    if (!r.error) {
      const res = r.result ?? null;
      // A mutating command's own result sits inside its envelope.
      if (res !== null && typeof res === 'object' && 'results' in res) {
        return { ok: res.results[0] ?? null };
      }
      return { ok: res };
    }
    const refusal = r.error.data?.refusal;
    if (typeof refusal === 'string') return { refused: refusal };
    return { broken: `${r.error.code}: ${r.error.message}` };
  }
  frame(f) {
    let params;
    try {
      params = substitute(f.params ?? {}, this.labels);
    } catch (e) {
      return { broken: String(e.message) };
    }
    const out = this.call(f.method, params);
    if (out.ok !== undefined && f.as) this.labels.set(f.as, out.ok);
    return out;
  }
  free() {
    try {
      this.scene.api_connection_close(this.conn);
      this.scene.free();
    } catch {
      // A scene that trapped may not free; the instance is replaced anyway.
    }
  }
}

// ------------------------------------------------------------------ rows

function row(id, outcome, scene) {
  const state = BigInt.asUintN(64, scene.state_hash()).toString(16).padStart(16, '0');
  const { hash, failed } = meshHash(scene);
  return { id, outcome: failed ? `${outcome}+mesh_error` : outcome, state, mesh: hash };
}
const HEADER_LINES =
  '# The determinism golden. Written by scripts/determinism.sh; do not edit.\n' +
  '# state_hash is Document::state_hash; mesh_hash is FNV-1a 64 over the exact bits of every\n' +
  "# visible object's tessellation.\n";
const line = (r) => `${r.id}\t${r.outcome}\t${r.state}\t${r.mesh}`;

// Only a Preview command in a production module cannot run, which is not a
// difference from the table. Anything else is a row, like native.
const PREVIEW_ONLY = 'preview_only';
const unavailable = (id, why) => ({ id, unavailable: why });
const isUnavailable = (out) => out.refused === PREVIEW_ONLY;

// Builds a model: { session } or { session, outcome } when a frame stopped it,
// or { unavailable }.
function build(wasm, model) {
  const s = new Session(wasm);
  for (const [i, f] of model.build.entries()) {
    const out = s.frame(f);
    if (out.ok !== undefined) continue;
    if (isUnavailable(out)) return { s, unavailable: `build frame ${i} (${f.method}): ${PREVIEW_ONLY}` };
    if (out.refused !== undefined) return { s, outcome: `build_refused:${out.refused}@${i}` };
    console.error(`build frame ${i} (${f.method}) failed: ${out.broken}`);
    return { s, outcome: `build_broken@${i}` };
  }
  return { s };
}

function runBuilt(wasm, name, model) {
  const b = build(wasm, model);
  try {
    if (b.unavailable) return unavailable(name, b.unavailable);
    return row(name, b.outcome ?? 'built', b.s.scene);
  } finally {
    b.s.free();
  }
}

function runCase(wasm, name, model, pick) {
  const id = `${name}/${pick.case}@${pick.size}`;
  const kase = model.cases.find((c) => c.id === pick.case);
  if (!kase) throw new Fatal(`cases.tsv: ${name} has no case ${pick.case}`);
  const b = build(wasm, model);
  const s = b.s;
  try {
    if (b.unavailable) return unavailable(id, b.unavailable);
    if (b.outcome) return row(id, b.outcome, s.scene);
    for (const [i, f] of (kase.before ?? []).entries()) {
      const out = s.frame(f);
      if (out.ok !== undefined) continue;
      if (isUnavailable(out)) return unavailable(id, `before[${i}] (${f.method}): ${PREVIEW_ONLY}`);
      if (out.refused !== undefined) return row(id, `before[${i}] refused:${out.refused}`, s.scene);
      console.error(`${id}: before[${i}] failed: ${out.broken}`);
      return row(id, 'broken', s.scene);
    }
    const sweep = kase.sweep ?? (kase.method === 'hew.solid.chamfer' ? 'distance' : 'radius');
    let params;
    try {
      params = substitute(kase.params ?? {}, s.labels);
    } catch (e) {
      console.error(`${id}: ${e.message}`);
      return row(id, 'broken', s.scene);
    }
    params[sweep] = parseLength(pick.size);
    const out = s.call(kase.method, params);
    if (isUnavailable(out)) return unavailable(id, `${kase.method}: ${PREVIEW_ONLY}`);
    if (out.broken !== undefined) console.error(`${id}: ${out.broken}`);
    return row(id, out.ok !== undefined ? 'ok' : out.refused !== undefined ? `refused:${out.refused}` : 'broken', s.scene);
  } finally {
    s.free();
  }
}

// Runs one row. A throw (a wasm trap, an unexpected reply) is a panic row;
// the module instance is then replaced, since a trap can leave it unusable.
function guarded(ctx, id, fn) {
  try {
    return fn(ctx.wasm);
  } catch (e) {
    if (e instanceof Fatal) throw e;
    console.error(`${id}: panic: ${e?.stack ?? e}`);
    ctx.wasm = reloadWasm();
    return { id, outcome: 'panic', state: '0'.repeat(16), mesh: '0'.repeat(16) };
  }
}

// A fresh instance of the same module: the CommonJS bindings instantiate
// when first required, so dropping the cache entry and requiring again
// builds a new one.
function reloadWasm() {
  const require = createRequire(import.meta.url);
  const entry = path.join(PKG_DIR, 'wasm_api.js');
  delete require.cache[require.resolve(entry)];
  return require(entry);
}

// ------------------------------------------------------------------ main

function loadModels() {
  const models = new Map();
  for (const dir of [CURVES_DIR, DETERMINISM_DIR]) {
    for (const f of readdirSync(dir).filter((f) => f.endsWith('.json')).sort()) {
      const name = f.slice(0, -'.json'.length);
      if (models.has(name)) throw new Error(`two models are named ${name}`);
      models.set(name, JSON.parse(readFileSync(path.join(dir, f), 'utf8')));
    }
  }
  return new Map([...models].sort((a, b) => (a[0] < b[0] ? -1 : a[0] > b[0] ? 1 : 0)));
}

function loadPicks() {
  return readFileSync(path.join(DETERMINISM_DIR, 'cases.tsv'), 'utf8')
    .split(/\r?\n/)
    .filter((l) => l && !l.startsWith('#') && !l.startsWith('model\t'))
    .map((l) => {
      const [model, kase, size] = l.split('\t');
      return { model, case: kase, size };
    });
}

function loadGolden() {
  const map = new Map();
  for (const l of readFileSync(path.join(DETERMINISM_DIR, 'golden.tsv'), 'utf8').split(/\r?\n/)) {
    if (!l || l.startsWith('#') || l.startsWith('id\t')) continue;
    map.set(l.split('\t')[0], l);
  }
  return map;
}

function main() {
  const args = process.argv.slice(2);
  const strict = args.includes('--strict');
  const outAt = args.indexOf('--out');
  const outPath = outAt >= 0 ? args[outAt + 1] : null;
  if (outAt >= 0 && !outPath) {
    console.error('usage: node determinism.mjs [--strict] [--out <file>]');
    process.exit(2);
  }

  const ctx = { wasm: loadWasm() };
  const models = loadModels();
  const picks = loadPicks();
  const rows = [];
  for (const [name, model] of models) {
    rows.push(guarded(ctx, name, (wasm) => runBuilt(wasm, name, model)));
    for (const pick of picks.filter((p) => p.model === name)) {
      const id = `${name}/${pick.case}@${pick.size}`;
      rows.push(guarded(ctx, id, (wasm) => runCase(wasm, name, model, pick)));
    }
  }

  const golden = loadGolden();
  const diffs = [];
  const skipped = [];
  const seen = new Set();
  for (const r of rows) {
    seen.add(r.id);
    if (r.unavailable) {
      skipped.push(r);
      continue;
    }
    const expected = golden.get(r.id);
    if (expected === undefined) {
      diffs.push({ id: r.id, kind: 'NEW', actual: line(r) });
    } else if (expected !== line(r)) {
      const e = expected.split('\t');
      diffs.push({
        id: r.id,
        kind: 'DIFF',
        expected,
        actual: line(r),
        outcome: e[1] !== r.outcome,
        stateDiffers: e[2] !== r.state,
        meshDiffers: e[3] !== r.mesh,
      });
    }
  }
  for (const [id, expected] of golden) {
    if (!seen.has(id)) diffs.push({ id, kind: 'GONE', expected });
  }

  if (outPath) {
    const body = rows.filter((r) => !r.unavailable).map(line).join('\n');
    // The same header lines the native test writes, so the two files compare
    // byte for byte.
    writeFileSync(outPath, `${HEADER_LINES}id\toutcome\tstate_hash\tmesh_hash\n${body}\n`);
  }

  for (const d of diffs) {
    console.log(`${d.kind} ${d.id}`);
    if (d.expected) console.log(`  expected ${d.expected}`);
    if (d.actual) console.log(`  actual   ${d.actual}`);
  }
  for (const s of skipped) console.log(`UNAVAILABLE ${s.id}: ${s.unavailable}`);

  const preview = ctx.wasm.preview_build();
  const real = diffs.filter((d) => d.kind === 'DIFF');
  const stateOnly = real.filter((d) => d.stateDiffers).length;
  const meshOnly = real.filter((d) => d.meshDiffers).length;
  console.log(
    `determinism (wasm, ${preview ? 'preview' : 'production'}): ${rows.length} rows, ${diffs.length} differ ` +
      `(${stateOnly} state hash, ${meshOnly} mesh hash, ${real.filter((d) => d.outcome).length} outcome), ` +
      `${skipped.length} unavailable`,
  );
  console.log(`differing rows: ${diffs.map((d) => d.id).join(', ') || 'none'}`);
  process.exit(strict && (diffs.length > 0 || (preview && skipped.length > 0)) ? 1 : 0);
}

main();

#!/usr/bin/env node
// The browser gate's skip rule, for both builds (docs/dev/DEVELOPMENT.md
// "Preview features"). Reads a Playwright JSON report
// (PLAYWRIGHT_JSON_OUTPUT_FILE) and fails when:
//
//   - any test was skipped for a reason other than the Preview wall's. A
//     skipped test reports green while asserting nothing, so the only skips
//     allowed are the ones the wall makes, each declared through the app's
//     registry-driven helpers (app/e2e/helpers/preview.ts), whose reasons
//     start with "preview-wall:". Anything else is a test to fix, or a
//     case to argue for here;
//   - with --wall (a whole, unsharded run): the wall spec
//     (app/e2e/preview-wall.spec.ts) did not run at least one test to a pass
//     in this build, so it can never be skipped wholesale;
//   - nothing ran at all.
//
// usage: node scripts/check-e2e-skips.mjs <report.json> <production|preview> [--wall]

import { readFileSync } from "node:fs";

const [reportPath, build, ...flags] = process.argv.slice(2);
if (!reportPath || !["production", "preview"].includes(build)) {
  console.error("usage: node scripts/check-e2e-skips.mjs <report.json> <production|preview> [--wall]");
  process.exit(2);
}
const MARK = "preview-wall:";
const WALL_SPEC = "preview-wall.spec.ts";
const report = JSON.parse(readFileSync(reportPath, "utf8"));

const tests = [];
const walk = (suite, file) => {
  const here = suite.file ?? file;
  for (const spec of suite.specs ?? []) {
    for (const t of spec.tests ?? []) tests.push({ file: spec.file ?? here, title: spec.title, test: t });
  }
  for (const child of suite.suites ?? []) walk(child, here);
};
for (const suite of report.suites ?? []) walk(suite, suite.file);

const skipReasons = (t) =>
  [...(t.annotations ?? []), ...(t.results ?? []).flatMap((r) => r.annotations ?? [])]
    .filter((a) => a.type === "skip" || a.type === "fixme")
    .map((a) => a.description ?? "");

const problems = [];
let ran = 0;
let wallPassed = 0;
for (const { file, title, test } of tests) {
  if (test.status === "skipped") {
    const reasons = skipReasons(test);
    if (reasons.length === 0 || !reasons.every((r) => r.startsWith(MARK))) {
      problems.push(`skipped without the Preview wall's reason: ${file} > ${title} (${reasons.join("; ") || "no reason"})`);
    }
    continue;
  }
  ran += 1;
  if (file.endsWith(WALL_SPEC) && test.status === "expected") wallPassed += 1;
}
if (ran === 0) problems.push("no test ran");
if (flags.includes("--wall") && wallPassed === 0) {
  problems.push(`${WALL_SPEC} passed no test in the ${build} build: the wall was not checked`);
}

if (problems.length > 0) {
  console.error(`check-e2e-skips (${build}): FAILED`);
  for (const p of problems) console.error(`  ${p}`);
  process.exit(1);
}
const skipped = tests.length - ran;
console.log(`check-e2e-skips (${build}): ${ran} ran, ${skipped} skipped by the Preview wall${flags.includes("--wall") ? `, ${wallPassed} wall checks passed` : ""}`);

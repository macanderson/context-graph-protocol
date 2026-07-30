#!/usr/bin/env node
/**
 * Assert this provider is conformant: run `contextgraph-inspect` against the
 * built stdio provider and fail (non-zero exit) if any check is not `pass`.
 * This is what the bundled CI workflow runs — and what you can run locally.
 *
 * The inspect binary is found via the CONTEXTGRAPH_INSPECT env var if set,
 * otherwise `contextgraph-inspect` on PATH (install it with
 * `cargo install contextgraph-conformance`).
 */
import { spawnSync } from "node:child_process";
import { existsSync } from "node:fs";

const inspect = process.env.CONTEXTGRAPH_INSPECT ?? "contextgraph-inspect";

if (!existsSync("dist/stdio.js")) {
  console.error("dist/stdio.js not found — run `npm run build` first.");
  process.exit(1);
}

const result = spawnSync(
  inspect,
  ["stdio", "--json", "--", process.execPath, "dist/stdio.js"],
  { encoding: "utf8" },
);

if (result.error) {
  console.error(
    `could not run "${inspect}": ${result.error.message}\n` +
      "Install it with `cargo install contextgraph-conformance`, or set " +
      "CONTEXTGRAPH_INSPECT to a prebuilt binary.",
  );
  process.exit(1);
}

// The report is the JSON block after the human-readable probe output; parse
// from the first line that starts with `{` (CLICOLOR_FORCE-safe: the JSON block
// itself carries no ANSI).
const lines = (result.stdout ?? "").split("\n");
const start = lines.findIndex((line) => line.trimStart().startsWith("{"));
if (start === -1) {
  console.error("no JSON report in inspect output:\n" + result.stdout + result.stderr);
  process.exit(1);
}

let report;
try {
  report = JSON.parse(lines.slice(start).join("\n"));
} catch (error) {
  console.error(`could not parse inspect report: ${error.message}`);
  process.exit(1);
}

const checks = report.checks ?? [];
const failed = checks.filter((c) => c.status === "fail");
for (const c of checks) {
  const mark = c.status === "pass" ? "OK" : c.status === "skipped" ? "--" : "XX";
  console.log(`  ${mark} ${c.name}: ${c.evidence}`);
}

if (failed.length > 0) {
  console.error(`\nNOT conformant: ${failed.map((c) => c.name).join(", ")}`);
  process.exit(1);
}
console.log(`\nAll ${checks.length} checks passed — provider is conformant.`);

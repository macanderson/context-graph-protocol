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

for (const c of checks) {
  const mark = c.status === "pass" ? "OK" : c.status === "skipped" ? "--" : "XX";
  console.log(`  ${mark} ${c.name}: ${c.evidence}`);
}

// A report with no checks is not a passing report. Before this, an empty list
// printed "All 0 checks passed — provider is conformant" and exited 0, so an
// inspect run that probed nothing was indistinguishable from a clean one. This
// is the first quality signal a provider author ever sees; it has to be able to
// tell "everything passed" from "nothing ran".
if (checks.length === 0) {
  console.error(
    "\nNOT conformant: the report contains no checks at all, so nothing was " +
      "verified. This usually means inspect could not reach the provider, or " +
      "the provider exited before the handshake. Run `contextgraph-inspect " +
      "stdio -- node dist/stdio.js` by hand and read the probe output above " +
      "the JSON.",
  );
  process.exit(1);
}

// Anything that is not `pass` and not `skipped` counts against the provider,
// rather than only the exact string "fail". A status this script does not know
// — an `error`, or one a future inspect adds — must not be read as success by
// a check whose whole job is to be strict.
const passed = checks.filter((c) => c.status === "pass");
const skipped = checks.filter((c) => c.status === "skipped");
const failed = checks.filter(
  (c) => c.status !== "pass" && c.status !== "skipped",
);

if (failed.length > 0) {
  console.error(
    `\nNOT conformant: ${failed
      .map((c) => `${c.name} (${c.status})`)
      .join(", ")}`,
  );
  process.exit(1);
}

// Skipped checks are reported separately rather than folded into the total.
// A transport legitimately skips some checks — an HTTP provider cannot answer
// the three stdio-only ones — but "13 checks passed" when 8 were skipped
// overstates what was verified, and the number is what an author quotes.
const summary =
  skipped.length > 0
    ? `${passed.length} passed, ${skipped.length} skipped`
    : `all ${passed.length} checks passed`;
console.log(`\nConformant — ${summary}.`);

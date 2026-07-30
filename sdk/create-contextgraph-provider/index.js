#!/usr/bin/env node
/**
 * create-contextgraph-provider — scaffold a conformant Context Graph Protocol
 * provider. It copies a language template, substitutes the project name and SDK
 * dependency, and drops a GitHub Actions workflow that runs `contextgraph-inspect`
 * against the generated provider in its own CI *from the first commit* — the
 * literal acceptance criterion of issue #17.
 *
 * Usage:
 *   npm create contextgraph-provider@latest my-provider
 *   create-contextgraph-provider <target-dir> [options]
 *
 * Options:
 *   --lang <typescript|python>   Language template (default: typescript).
 *   --name <package-name>        Project/package name (default: target dir basename).
 *   --sdk  <dependency-spec>     Override the SDK dependency. Defaults to the
 *                                published version range; point it at a local
 *                                checkout to try an unpublished SDK, e.g.
 *                                `--sdk file:../context-graph-protocol/sdk/typescript`
 *                                (TypeScript) or a local path (Python).
 *   --force                      Write into a non-empty target directory.
 *
 * Zero runtime dependencies — stdlib Node only.
 */
import { existsSync, mkdirSync, readdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { basename, dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));

/** The published dependency each template resolves by default (see #59). */
const DEFAULT_SDK = {
  typescript: "^0.1.0",
  python: "contextgraph-sdk>=0.1.0",
};

function fail(message) {
  process.stderr.write(`create-contextgraph-provider: ${message}\n`);
  process.exit(1);
}

function parseArgs(argv) {
  const opts = { lang: "typescript", force: false };
  const positional = [];
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    if (arg === "--force") opts.force = true;
    else if (arg === "--lang") opts.lang = argv[++i];
    else if (arg === "--name") opts.name = argv[++i];
    else if (arg === "--sdk") opts.sdk = argv[++i];
    else if (arg === "--help" || arg === "-h") opts.help = true;
    else if (arg.startsWith("--")) fail(`unknown option ${arg}`);
    else positional.push(arg);
  }
  opts.target = positional[0];
  return opts;
}

const HELP = `create-contextgraph-provider — scaffold a conformant CGP provider

Usage:
  create-contextgraph-provider <target-dir> [--lang typescript|python]
                                            [--name <package-name>]
                                            [--sdk <dependency-spec>] [--force]

The generated project bundles a GitHub Actions workflow that runs
contextgraph-inspect against it in CI from day one.`;

/** Recursively list files under `dir`, returned as paths relative to `dir`. */
function listFiles(dir, prefix = "") {
  const out = [];
  for (const entry of readdirSync(dir)) {
    const abs = join(dir, entry);
    const rel = prefix ? `${prefix}/${entry}` : entry;
    if (statSync(abs).isDirectory()) out.push(...listFiles(abs, rel));
    else out.push(rel);
  }
  return out;
}

/**
 * Map a template path to its written path: a leading `_` on any segment becomes
 * a `.` (so `_gitignore` → `.gitignore`, `_github/workflows` → `.github/...`),
 * which keeps dotfiles from being swallowed by npm packaging.
 */
function outputPath(relPath) {
  return relPath
    .split("/")
    .map((segment) => (segment.startsWith("_") ? `.${segment.slice(1)}` : segment))
    .join("/");
}

function substitute(content, vars) {
  return content.replace(/\{\{(\w+)\}\}/g, (match, key) =>
    key in vars ? vars[key] : match,
  );
}

function main() {
  const opts = parseArgs(process.argv.slice(2));
  if (opts.help) {
    process.stdout.write(`${HELP}\n`);
    return;
  }
  if (!opts.target) fail(`no target directory given\n\n${HELP}`);
  if (!["typescript", "python"].includes(opts.lang)) {
    fail(`--lang must be "typescript" or "python", got "${opts.lang}"`);
  }

  const targetDir = resolve(process.cwd(), opts.target);
  const name = opts.name ?? basename(targetDir);
  const sdk = opts.sdk ?? DEFAULT_SDK[opts.lang];
  const templateDir = join(HERE, "templates", opts.lang);
  if (!existsSync(templateDir)) fail(`missing template for ${opts.lang}`);

  if (existsSync(targetDir) && readdirSync(targetDir).length > 0 && !opts.force) {
    fail(`target ${targetDir} is not empty (pass --force to write anyway)`);
  }

  const vars = {
    PROJECT_NAME: name,
    SDK_SPEC: sdk,
    YEAR: String(new Date().getFullYear()),
  };

  const files = listFiles(templateDir);
  for (const rel of files) {
    const raw = readFileSync(join(templateDir, rel), "utf8");
    const dest = join(targetDir, outputPath(rel));
    mkdirSync(dirname(dest), { recursive: true });
    writeFileSync(dest, substitute(raw, vars));
  }

  const nextSteps =
    opts.lang === "typescript"
      ? ["npm install", "npm run build", "npm run conformance"]
      : ["python3 -m venv .venv && . .venv/bin/activate", "pip install -e .", "python scripts/check_conformance.py"];

  process.stdout.write(
    `\nScaffolded ${opts.lang} provider "${name}" in ${targetDir}\n` +
      `  SDK dependency: ${sdk}\n\n` +
      `Next steps:\n  cd ${opts.target}\n` +
      nextSteps.map((s) => `  ${s}`).join("\n") +
      `\n\nIts .github/workflows/conformance.yml runs contextgraph-inspect in CI from the first push.\n`,
  );
}

main();

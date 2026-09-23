#!/usr/bin/env python3
"""
Downstream canary, oxagen half (#104, #139): replay the CGP fixtures oxagen
vendors through THIS repository's HEAD.

Usage (matches downstream-canary-stella.sh — env vars, no flags, safe to run
twice):

    CGP_DIR=/path/to/context-graph-protocol \\
    OXAGEN_DIR=/path/to/oxagen \\
    python3 .github/scripts/downstream-canary-oxagen.py

Exits 0 when every vendored fixture set still passes this checkout's own
fixture tests, 1 otherwise. Needs python3 (stdlib only) and cargo.

What oxagen pins, and why a build is the wrong canary for it:

  stella consumes the contextgraph-* crates as Rust dependencies, so its
  canary builds stella against HEAD. `macanderson/oxagen` (public; the
  repository this job once called `oxagen-platform`, which does not exist)
  is a TypeScript monorepo. What it takes from this repository is data:
  golden fixture sets copied into its packages, each beside a `manifest.json`
  that names this repository as `upstream_repository`, the commit it was
  copied at, and a sha256 per file. oxagen's own tests validate its code
  against those copies. So the break that reaches oxagen is not a compile
  error — it is this repository's contract moving out from under a copy that
  oxagen still treats as the truth.

What this script checks:

  1. Discovery. Every `manifest.json` under OXAGEN_DIR whose
     `upstream_repository` is this repository is a vendored set. Finding none
     is a failure, not a pass: it means the sparse checkout or the layout
     moved, and a canary that silently checks nothing is the failure mode this
     script replaced.
  2. Mapping. Each set is mapped to the fixture directory it was copied from
     (its `upstream_path`, or the directory its `generation_command`
     regenerates) and to the test target that owns that directory. A set this
     script does not know how to replay is a failure, so a newly vendored set
     cannot go unchecked by default.
  3. Replay. The vendored bytes are overlaid onto this checkout's fixture
     directory and that directory's own test target is run unmodified. The
     tests are HEAD's; the data is oxagen's. A pass means oxagen's pinned
     copy is still a true statement of this repository's contract; a failure
     names the test and the rule that its copy now violates. The one test
     skipped is the upstream manifest's hash check, which asserts HEAD's own
     bytes and fails by construction once they are overlaid (ci.yml runs it
     against the real tree).
  4. Removal. A vendored file this checkout no longer ships fails: oxagen
     is pinned to a fixture the contract dropped.

  Byte drift (HEAD's copy differs from oxagen's) is reported but is not
  itself a failure: an additive change to a fixture is not a break, and the
  replay is what decides whether the difference matters.

The overlay mutates CGP_DIR, so every overlaid file's original bytes are held
in memory and written back in a `finally`, including on Ctrl-C. That keeps the
script safe on a contributor's working tree, not only on a disposable runner
checkout.
"""
import hashlib
import json
import os
import subprocess
import sys
from pathlib import Path

CGP_URL = "https://github.com/macanderson/context-graph-protocol"

# Upstream fixture directory -> (cargo package, test target, tests to skip).
# Adding a vendored set upstream means adding its row here; step 2 above makes
# forgetting to do so a red run rather than a silent gap.
REPLAYS = {
    "contextgraph-conformance/fixtures/contextgraph-1.0": (
        "contextgraph-conformance",
        "golden_fixtures",
        ["manifest_has_strict_coverage_and_correct_file_hashes"],
    ),
    "contextgraph-trace/fixtures": ("contextgraph-trace", "fixture_suite", []),
}

# A manifest without `upstream_path` names the command that regenerates it;
# this maps that command to the directory it writes.
GENERATION_COMMANDS = {
    "cargo test -p contextgraph-conformance --test golden_fixtures": (
        "contextgraph-conformance/fixtures/contextgraph-1.0"
    ),
}

SKIP_DIRS = {".git", "node_modules", "target", "dist", "build", ".turbo"}


def sha256(path: Path) -> str:
    return "sha256:" + hashlib.sha256(path.read_bytes()).hexdigest()


def normalize_url(url: str) -> str:
    url = url.strip().rstrip("/")
    return url[: -len(".git")] if url.endswith(".git") else url


def discover(oxagen_dir: Path) -> list[tuple[Path, dict]]:
    found = []
    for root, dirs, files in os.walk(oxagen_dir):
        dirs[:] = sorted(d for d in dirs if d not in SKIP_DIRS)
        if "manifest.json" not in files:
            continue
        path = Path(root) / "manifest.json"
        try:
            manifest = json.loads(path.read_text(encoding="utf-8"))
        except (OSError, ValueError):
            continue  # not every manifest.json in a monorepo is ours
        if not isinstance(manifest, dict):
            continue
        upstream = manifest.get("upstream_repository")
        if isinstance(upstream, str) and normalize_url(upstream) == CGP_URL:
            found.append((path, manifest))
    return found


def upstream_dir_of(manifest: dict) -> str | None:
    if isinstance(manifest.get("upstream_path"), str):
        return manifest["upstream_path"].strip("/")
    return GENERATION_COMMANDS.get(manifest.get("generation_command", ""))


def summary(lines: list[str]) -> None:
    target = os.environ.get("GITHUB_STEP_SUMMARY")
    if target:
        with open(target, "a", encoding="utf-8") as handle:
            handle.write("\n".join(lines) + "\n")


def main() -> int:
    cgp_dir = Path(
        os.environ.get("CGP_DIR") or Path(__file__).resolve().parent.parent.parent
    ).resolve()
    oxagen_env = os.environ.get("OXAGEN_DIR")
    if not oxagen_env:
        print("::error::OXAGEN_DIR is not set — point it at a checkout of macanderson/oxagen")
        return 1
    oxagen_dir = Path(oxagen_env).resolve()
    if not oxagen_dir.is_dir():
        print(f"::error::{oxagen_dir} is not a directory")
        return 1

    sets = discover(oxagen_dir)
    if not sets:
        print(
            f"::error::no manifest.json under {oxagen_dir} names {CGP_URL} as its "
            "upstream_repository — the sparse checkout or oxagen's layout moved, so "
            "this canary would check nothing"
        )
        return 1

    failures: list[str] = []
    report = ["### Downstream canary: `oxagen` vendored fixtures vs this HEAD", ""]

    for manifest_path, manifest in sets:
        vendored_dir = manifest_path.parent
        label = str(vendored_dir.relative_to(oxagen_dir))
        pinned = manifest.get("upstream_commit", "an unrecorded commit")
        upstream = upstream_dir_of(manifest)
        print(f"--- {label} (pinned at {pinned}) ---")

        if upstream is None or upstream not in REPLAYS:
            failures.append(
                f"{label}: vendors a CGP fixture set this canary cannot replay "
                f"(upstream {upstream!r}); add it to REPLAYS"
            )
            continue
        package, test_target, skips = REPLAYS[upstream]
        upstream_path = cgp_dir / upstream

        files = manifest.get("files")
        if not isinstance(files, dict) or not files:
            failures.append(f"{label}: manifest lists no files")
            continue

        overlay: list[tuple[Path, Path]] = []
        for name in sorted(files):
            source = vendored_dir / name
            dest = upstream_path / name
            if not source.is_file():
                failures.append(f"{label}/{name}: listed in oxagen's manifest but absent")
                continue
            if not dest.is_file():
                failures.append(
                    f"{label}/{name}: this HEAD no longer ships {upstream}/{name}"
                )
                continue
            drift = "identical to HEAD" if sha256(source) == sha256(dest) else "differs from HEAD"
            print(f"  {name}: {drift}")
            overlay.append((source, dest))

        if not overlay:
            continue

        originals = {dest: dest.read_bytes() for _, dest in overlay}
        command = ["cargo", "test", "-p", package, "--test", test_target, "--"]
        for skip in skips:
            command += ["--skip", skip]
        try:
            for source, dest in overlay:
                dest.write_bytes(source.read_bytes())
            print(f"  replaying: {' '.join(command)}", flush=True)
            result = subprocess.run(command, cwd=cgp_dir, check=False)
        finally:
            for dest, original in originals.items():
                dest.write_bytes(original)

        if result.returncode == 0:
            report.append(f"- :white_check_mark: `{label}` (pinned at `{pinned}`) passes `{test_target}` at HEAD")
        else:
            failures.append(
                f"{label} (pinned at {pinned}) fails {package}'s `{test_target}` at HEAD — "
                "see the test output above for the rule its copy now violates"
            )

    if failures:
        for failure in failures:
            print(f"::error title=downstream canary (oxagen)::{failure}")
            report.append(f"- :x: {failure}")
        summary(report)
        return 1

    summary(report)
    print(f"All {len(sets)} vendored fixture set(s) replay green against {cgp_dir}.")
    return 0


if __name__ == "__main__":
    sys.exit(main())

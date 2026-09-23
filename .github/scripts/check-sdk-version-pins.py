#!/usr/bin/env python3
"""
Hold the scaffolder's default SDK pins to the SDK manifests they name.

Usage:
    python3 .github/scripts/check-sdk-version-pins.py

Exits 0 if every version one manifest states about a package owned by another
manifest is consistent with that package's own manifest, and every version
tracked Markdown writes for such a package is too. Exits 1 otherwise. Stdlib
only, offline — it reads no registry, so it cannot depend on a publish.

Why this exists (see docs/adr/0012-sdk-version-pins-share-a-major.md):

  `sdk/create-contextgraph-provider/index.js` carries a `DEFAULT_SDK` table —
  the dependency every scaffolded project resolves when the caller passes no
  `--sdk`. It names versions of two packages it does not own, and it sat at
  `^0.1.0` for TypeScript while `sdk/typescript/package.json` shipped `1.0.0`,
  a full major behind, for an unknown length of time (#98).

  Nothing detected it, and the job that looks closest could not have. The
  `create-contextgraph-provider scaffolds a conformant project` job in
  `ci.yml` overrides both published pins with local paths
  (`--sdk "file:$GITHUB_WORKSPACE/sdk/typescript"`, `pip install ./sdk/python`)
  so that CI never depends on a registry. That is the right call for that job:
  it proves the *templates* are conformant, and says nothing about whether the
  versions they name exist. So the guard belongs here instead.

The rule, and why it is not equality:

  The two pins are deliberately ranges — a caret (`^2.0.0`) and a floor
  (`contextgraph-sdk>=2.0.0`) — so that a scaffold picks up an SDK patch
  release without a commit here. Equality would defeat that and would make
  every SDK patch a two-repo-file change nobody would remember to make.

  What a scaffold cannot survive is a **major**: majors are where the SDK's
  API changes, and ADR 0011's open-`FrameKind` break is the worked example —
  a project scaffolded against a `1.x` pin generates code against the closed
  vocabulary `2.0.0` retired. So the pin's floor must share the manifest's
  major.

  The floor must also not run *ahead* of the manifest, which is drift in the
  other direction: a pin naming a version that has never been published
  resolves to nothing at all. The manifest version is the newest release that
  can exist, because these manifests are what `publish-sdks.yml` publishes.

What is deliberately not checked, and why:

  * `sdk/go` carries no package version. A Go module is versioned by its git
    tag (`sdk/go/v…`), not by a field in `go.mod`, and the scaffolder emits no
    Go template, so there is no in-tree pin to compare against. Its
    `ProtocolVersion` constant is a *protocol* version — a different axis from
    package version, governed by SPEC.md §3.1. The tag is not compared with
    the crates' major either: the Go SDK has its own version line, because Go
    ties a module's major to its import path, and a CI checkout fetches no
    tags (docs/adr/0026-versions-in-prose-and-the-go-sdk-tag.md). What *is*
    checked is that no prose pins a concrete Go version, since nothing offline
    could keep one true.
  * CHANGELOG.md and docs/adr/ quote versions as history, and are exempt from
    the prose check for the reason check-deploy-hygiene.py exempts them.
  * `schema/reference-vectors.ndjson` carries `"version": "1.0.0"` strings.
    Those are the *provider* versions of the fixtures in the vectors, not
    package versions of anything in this repository, and they must stay put:
    rewriting them would change the bytes the reference vectors pin.
  * `create-contextgraph-provider`'s own `version` is a version it owns, and
    it is free to move on its own cadence.
"""
import json
import re
import subprocess
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent

SCAFFOLDER = ROOT / "sdk/create-contextgraph-provider/index.js"
TS_MANIFEST = ROOT / "sdk/typescript/package.json"
PY_MANIFEST = ROOT / "sdk/python/pyproject.toml"
TS_TEMPLATE = ROOT / "sdk/create-contextgraph-provider/templates/typescript/package.json"
PY_TEMPLATE = ROOT / "sdk/create-contextgraph-provider/templates/python/pyproject.toml"
CARGO = ROOT / "Cargo.toml"

# The placeholder `index.js` substitutes the resolved pin into. If a template
# ever hardcodes a version instead, the pin this script checks stops being the
# pin that ships, and the guard would pass while guarding nothing.
PLACEHOLDER = "{{SDK_SPEC}}"

# Range operators whose leading version is a floor — the only shape the rule
# below can reason about. An unlisted operator (`<`, `!=`, a compound range)
# fails rather than being silently read as a floor.
TS_OPERATORS = ("^", "~", ">=", "")
PY_OPERATORS = (">=", "~=", "==")

DEFAULT_SDK_BLOCK = re.compile(r"^const DEFAULT_SDK = \{(.*?)^\};", re.S | re.M)
DEFAULT_SDK_ENTRY = re.compile(r'^\s*(\w+):\s*"([^"]+)",?\s*$', re.M)
SEMVER = re.compile(r"^(\d+)\.(\d+)\.(\d+)(?:[-+].*)?$")
PY_REQUIREMENT = re.compile(
    rf"^([A-Za-z0-9._-]+)\s*({'|'.join(re.escape(op) for op in PY_OPERATORS)})\s*(.+)$"
)

failures = 0


def check(label: str, ok: bool, detail: str = "") -> bool:
    global failures
    print(f"  {'PASS' if ok else 'FAIL'}  {label}")
    if not ok:
        failures += 1
        if detail:
            for line in detail.splitlines():
                print(f"        {line}")
    return ok


def parse_semver(raw: str) -> tuple[int, int, int] | None:
    match = SEMVER.match(raw.strip())
    return (int(match[1]), int(match[2]), int(match[3])) if match else None


def default_sdk_pins() -> dict[str, str]:
    """The `DEFAULT_SDK` table in the scaffolder, as `{lang: dependency-spec}`."""
    block = DEFAULT_SDK_BLOCK.search(SCAFFOLDER.read_text(encoding="utf-8"))
    if block is None:
        return {}
    return {lang: spec for lang, spec in DEFAULT_SDK_ENTRY.findall(block[1])}


def split_range(spec: str, operators: tuple[str, ...]) -> tuple[str, str] | None:
    """Split `^2.0.0` into `("^", "2.0.0")`, or return None on an unknown operator."""
    for operator in sorted(operators, key=len, reverse=True):
        if operator and spec.startswith(operator):
            return operator, spec[len(operator):].strip()
    return ("", spec) if "" in operators and spec[:1].isdigit() else None


def check_pin(label: str, pin: str, floor: str | None, shipped: str, source: str) -> None:
    """A pin's floor shares `shipped`'s major and does not run ahead of it."""
    parsed_floor = parse_semver(floor) if floor is not None else None
    parsed_shipped = parse_semver(shipped)
    if parsed_floor is None or parsed_shipped is None:
        check(
            f"{label} pin is a floor this check can read",
            False,
            f"pin {pin!r} against {source} version {shipped!r}\n"
            f"remedy: write the pin as a floor over an x.y.z version"
            f" (e.g. ^{shipped} or >={shipped}).",
        )
        return

    remedy = (
        f"pin {pin!r} names major {parsed_floor[0]}; {source} ships {shipped}\n"
        f"remedy: move the pin in {SCAFFOLDER.relative_to(ROOT)}'s DEFAULT_SDK"
        f" onto {shipped}, or explain the split in"
        f" docs/adr/0012-sdk-version-pins-share-a-major.md."
    )
    check(
        f"{label} pin shares the major {source} ships",
        parsed_floor[0] == parsed_shipped[0],
        remedy,
    )
    check(
        f"{label} pin does not name a version {source} has never shipped",
        parsed_floor <= parsed_shipped,
        f"pin {pin!r} floors at {floor}, ahead of the {shipped} in {source}\n"
        f"remedy: publish {floor} first, or lower the pin.",
    )


print("the scaffolder's default pins agree with the SDK manifests they name")

pins = default_sdk_pins()
if not check(
    "DEFAULT_SDK is readable in the scaffolder",
    set(pins) == {"typescript", "python"},
    f"parsed {sorted(pins)} from {SCAFFOLDER.relative_to(ROOT)}, expected"
    " typescript and python\n"
    "remedy: keep DEFAULT_SDK a flat `lang: \"spec\"` object literal, or teach"
    " this parser the new shape.",
):
    sys.exit(1)

ts_manifest = json.loads(TS_MANIFEST.read_text(encoding="utf-8"))
py_manifest = tomllib.loads(PY_MANIFEST.read_text(encoding="utf-8"))["project"]
ts_template = json.loads(TS_TEMPLATE.read_text(encoding="utf-8"))
py_template = tomllib.loads(PY_TEMPLATE.read_text(encoding="utf-8"))["project"]

# Half of what makes the pin above the *shipped* pin: the templates have to
# take it from DEFAULT_SDK rather than restating a version of their own.
check(
    "the TypeScript template takes its SDK dependency from DEFAULT_SDK",
    ts_template.get("dependencies", {}).get(ts_manifest["name"]) == PLACEHOLDER,
    f"expected {ts_manifest['name']!r}: {PLACEHOLDER!r} in"
    f" {TS_TEMPLATE.relative_to(ROOT)}, found"
    f" {ts_template.get('dependencies')!r}",
)
check(
    "the Python template takes its SDK dependency from DEFAULT_SDK",
    py_template.get("dependencies") == [PLACEHOLDER],
    f"expected [{PLACEHOLDER!r}] in {PY_TEMPLATE.relative_to(ROOT)}, found"
    f" {py_template.get('dependencies')!r}",
)

ts_pin = pins["typescript"]
ts_split = split_range(ts_pin, TS_OPERATORS)
check_pin(
    "typescript",
    ts_pin,
    ts_split[1] if ts_split else None,
    ts_manifest["version"],
    TS_MANIFEST.relative_to(ROOT).as_posix(),
)

py_pin = pins["python"]
py_requirement = PY_REQUIREMENT.match(py_pin)
if check(
    "the Python pin names the package sdk/python publishes",
    py_requirement is not None and py_requirement[1] == py_manifest["name"],
    f"pin {py_pin!r} does not read as {py_manifest['name']} followed by one of"
    f" {', '.join(PY_OPERATORS)} and an x.y.z version",
):
    assert py_requirement is not None
    check_pin(
        "python",
        py_pin,
        py_requirement[3],
        py_manifest["version"],
        PY_MANIFEST.relative_to(ROOT).as_posix(),
    )

# The lockstep the scaffolder's own DEFAULT_SDK comment and MIGRATION.md §5.4
# both assert: an SDK major and a crate major are the same release. The pins
# above are only meaningful while it holds — a `^2.0.0` pin protects a
# scaffold from ADR 0011 exactly because SDK 2 and crate 2 are one break.
print("\nthe SDK majors move in lockstep with the crates")

workspace_version = tomllib.loads(CARGO.read_text(encoding="utf-8"))["workspace"]["package"][
    "version"
]
majors = {
    "Cargo.toml [workspace.package]": workspace_version,
    TS_MANIFEST.relative_to(ROOT).as_posix(): ts_manifest["version"],
    PY_MANIFEST.relative_to(ROOT).as_posix(): py_manifest["version"],
}
parsed = {source: parse_semver(version) for source, version in majors.items()}
unreadable = [source for source, version in parsed.items() if version is None]
if check(
    "every versioned manifest states an x.y.z version",
    not unreadable,
    "\n".join(f"{source} -> {majors[source]!r}" for source in unreadable),
):
    check(
        "the SDKs and the crates share one major",
        len({version[0] for version in parsed.values() if version}) == 1,  # type: ignore[index]
        "\n".join(f"{source} -> {version}" for source, version in majors.items())
        + "\nremedy: bump them together, as MIGRATION.md §5.4 says they move.",
    )

# Versions written in prose (#108, ADR 0026).
#
# Everything above compares manifests. A version written in a README or a
# runbook is invisible to that, and two had already gone stale: a
# `go get …@v0.1.0` acceptance command beside two unpinned siblings, and a
# `contextgraph-types = "=0.1.0"` example with the crates on 2.x. So install
# commands and dependency lines in tracked Markdown are held to the same rule
# as the pins above: a version of a package this repository publishes shares
# that package's major and is not ahead of it.
#
# The Go SDK is the exception. Its only version is its git tag, which an
# offline check cannot read (a CI checkout fetches no tags). So prose never
# pins a concrete Go version. Write `@latest`, or an `@vX.Y.Z` placeholder. A
# version nothing can check is a version nothing keeps true.
#
# Historical records are exempt, as check-deploy-hygiene.py exempts them: a
# changelog or an ADR that rewrote the version it reports would be lying about
# the past.
print("\nversions written in prose name a version their manifest ships")

PROSE_EXEMPT = ("CHANGELOG.md", "docs/adr/")
PROSE_VERSION = re.compile(r"^[=^~<>!\s]*v?(\d+)(?:\.(\d+))?(?:\.(\d+))?(?:[-+.]\S*)?$")
QUOTED_REQ = r'"(?P<spec>[^"]+)"'
UNQUOTED_REQ = r"(?P<spec>[=^~<>]*v?\d[\w.+\-]*)"


def published_crates() -> set[str]:
    """Names of the workspace crates that set `publish = true`."""
    members = tomllib.loads(CARGO.read_text(encoding="utf-8"))["workspace"].get("members", [])
    names = set()
    for member in members:
        manifest = ROOT / member / "Cargo.toml"
        if not manifest.is_file():
            continue
        package = tomllib.loads(manifest.read_text(encoding="utf-8")).get("package", {})
        if package.get("publish") is True and "name" in package:
            names.add(package["name"])
    return names


def tracked_markdown() -> list[Path]:
    out = subprocess.run(
        ["git", "-C", str(ROOT), "ls-files", "-z", "--", "*.md"],
        capture_output=True, text=True, check=True,
    ).stdout
    return [ROOT / name for name in out.split("\0")
            if name and not name.startswith(PROSE_EXEMPT) and (ROOT / name).is_file()]


def prose_floor(spec: str) -> tuple[int, int, int] | None:
    """The version a requirement floors at: `^2`, `=2.0.0`, `>=2.1` -> (2, 0, 0) etc."""
    first = spec.split(",")[0].strip()
    match = PROSE_VERSION.match(first)
    if match is None:
        return None
    return tuple(int(part or 0) for part in match.groups())  # type: ignore[return-value]


scaffolder_manifest = json.loads(
    (ROOT / "sdk/create-contextgraph-provider/package.json").read_text(encoding="utf-8"))
crate_names = published_crates()
crate_alternation = "|".join(sorted(map(re.escape, crate_names))) or r"(?!)"

# (pattern, owner label, the version the owner ships)
PROSE_PINS: list[tuple[re.Pattern[str], str, str]] = [
    (re.compile(re.escape(ts_manifest["name"]) + r"@" + UNQUOTED_REQ),
     TS_MANIFEST.relative_to(ROOT).as_posix(), ts_manifest["version"]),
    (re.compile(r"(?:\bcreate-contextgraph-provider|\bnpm (?:create|init) contextgraph-provider)@"
                + UNQUOTED_REQ),
     "sdk/create-contextgraph-provider/package.json", scaffolder_manifest["version"]),
    (re.compile(r"(?<![\w-])" + re.escape(py_manifest["name"]) + r"\s*(?:==|>=|~=)\s*"
                + UNQUOTED_REQ),
     PY_MANIFEST.relative_to(ROOT).as_posix(), py_manifest["version"]),
    (re.compile(r"(?<![\w-])(?:" + crate_alternation + r")\s*=\s*(?:\{[^}\n]*?version\s*=\s*)?"
                + QUOTED_REQ),
     "Cargo.toml [workspace.package]", workspace_version),
    (re.compile(r"cargo (?:add|install) (?:" + crate_alternation + r")@" + UNQUOTED_REQ),
     "Cargo.toml [workspace.package]", workspace_version),
]
GO_PIN = re.compile(r"go (?:get|install) \S*/sdk/go\S*@(v\d[\w.+\-]*)")

stale: list[str] = []
go_pins: list[str] = []
for path in tracked_markdown():
    rel = path.relative_to(ROOT).as_posix()
    for lineno, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        for pattern, owner, shipped in PROSE_PINS:
            for match in pattern.finditer(line):
                floor = prose_floor(match["spec"])
                shipped_version = parse_semver(shipped)
                if floor is None or shipped_version is None:
                    stale.append(f"{rel}:{lineno}  {match[0]!r} is not a version this"
                                 f" check can read against {owner} {shipped}")
                elif floor[0] != shipped_version[0] or floor > shipped_version:
                    stale.append(f"{rel}:{lineno}  {match[0]!r} — {owner} ships {shipped}")
        for match in GO_PIN.finditer(line):
            go_pins.append(f"{rel}:{lineno}  {match[0]!r}")

check(
    "every version written in prose shares its manifest's major and is not ahead of it",
    not stale,
    "\n".join(stale)
    + "\nremedy: move the version onto the shipped major, drop the version so the"
      "\ncommand resolves the current release, or, if the line records history,"
      "\nmove it into CHANGELOG.md or an ADR.",
)
check(
    "no prose pins a concrete Go SDK version (its only version is a git tag)",
    not go_pins,
    "\n".join(go_pins)
    + "\nremedy: write @latest, or an @vX.Y.Z placeholder in a runbook step."
      "\nWhy: docs/adr/0026-versions-in-prose-and-the-go-sdk-tag.md",
)

print(f"\n{'OK — no version drift' if failures == 0 else f'{failures} failure(s)'}")
sys.exit(1 if failures else 0)

#!/usr/bin/env python3
"""
Hold CI to a deliberately pinned Rust toolchain.

Usage:
    python3 .github/scripts/check-toolchain-pin.py

Exits 0 if `rust-toolchain.toml` names one concrete release, no workflow
installs a floating channel behind its back, and the `msrv` job still overrides
the pin. Exits 1 otherwise. Stdlib only, offline.

Why this exists (see #160 and docs/toolchain.md):

  * Every lint and test job used to run `dtolnay/rust-toolchain@stable`, so the
    compiler moved on Rust's release schedule rather than ours. clippy runs at
    `-D warnings`, so each new warn-by-default lint arrived as an unannounced
    red `main` — landing on whichever pull request happened to be open, whose
    author then paid for a change they had not made. Rust 1.98's
    `clippy::chunks_exact_to_as_chunks` did precisely that to
    `contextgraph-types/src/attest.rs`, code green and untouched since #87.

  * Reverting that fix does not look like a revert. Re-adding
    `dtolnay/rust-toolchain@stable` to one new job reads as ordinary
    copy-paste from the twenty jobs that used to have it, and nothing goes red
    until the *next* Rust release — by which time the cause is weeks behind the
    symptom. The same is true of loosening the pin to `"1.98"`, which rustup
    resolves to the newest 1.98.x and so is not a pin at all.

  * The `msrv` job is the one job that must NOT use the pin, and since
    `rust-toolchain.toml` exists it needs an explicit override to stay honest:
    rustup ranks the file above the rustup default the job sets, so without
    `RUSTUP_TOOLCHAIN` that job would build on the newest compiler while
    reporting that the crates build on the oldest. Deleting the override is
    invisible — the job keeps passing, it just stops meaning anything.

Each of those is a silent regression, which is what makes them worth a gate
rather than a review note.
"""

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent
WORKFLOWS = ROOT / ".github" / "workflows"
PIN_FILE = ROOT / "rust-toolchain.toml"
CARGO_TOML = ROOT / "Cargo.toml"

# rustup treats these as moving targets. `1.98` is included deliberately: a
# two-component version resolves to the latest patch in that series, so it
# floats exactly like `stable` does, only more slowly and less visibly.
CONCRETE_VERSION = re.compile(r"^\d+\.\d+\.\d+$")
# Anchored to a `uses:` directive rather than matched anywhere on the line.
#
# The looser form was wrong in a way worth keeping a note about: it fired on the
# comment *above this job* explaining the rule, because that comment names
# `dtolnay/rust-toolchain@stable` in order to say not to write it. A guard that
# forbids a string cannot be written with a grep for that string, or it forbids
# its own documentation — and the reflex fix, deleting the sentence, would take
# out the one place a reader learns why the rule exists.
#
# `uses:` may be a step key or a list item, and the value may be quoted, so all
# three shapes are accepted. Anything not in a `uses:` value is prose.
FLOATING_ACTION = re.compile(
    r"""^\s*(?:-\s*)?uses:\s*['"]?dtolnay/rust-toolchain@(stable|beta|nightly)\b"""
)


def fail(message: str) -> None:
    print(f"::error::{message}", file=sys.stderr)


def read_channel() -> str | None:
    """The `channel` from rust-toolchain.toml, or None if it is unreadable."""
    for line in PIN_FILE.read_text().splitlines():
        match = re.match(r'^\s*channel\s*=\s*"([^"]*)"', line)
        if match:
            return match.group(1)
    return None


def check_pin_is_concrete(problems: list[str]) -> str | None:
    if not PIN_FILE.exists():
        problems.append(
            "rust-toolchain.toml is missing. It is the single source of truth "
            "for the toolchain CI lints and tests with; without it every job "
            "falls back to whatever stable happens to be that week (#160)."
        )
        return None

    channel = read_channel()
    if channel is None:
        problems.append(
            'rust-toolchain.toml has no quoted `channel = "..."`. The composite '
            "action at .github/actions/rust parses it with the same expression "
            "and would fail the same way."
        )
        return None

    if not CONCRETE_VERSION.match(channel):
        problems.append(
            f'rust-toolchain.toml pins channel "{channel}", which is not one '
            "concrete release. Use a full x.y.z version: `stable`/`beta`/"
            "`nightly` move on Rust's schedule, and a two-component `1.98` "
            "resolves to the newest 1.98.x, so neither is a pin. See "
            "docs/toolchain.md for the bump procedure."
        )
        return None

    return channel


def check_no_floating_installs(problems: list[str]) -> None:
    for workflow in sorted(WORKFLOWS.glob("*.yml")):
        for number, line in enumerate(workflow.read_text().splitlines(), start=1):
            if FLOATING_ACTION.search(line):
                problems.append(
                    f".github/workflows/{workflow.name}:{number} installs a "
                    "floating toolchain. Use `./.github/actions/rust`, which "
                    "installs the release named by rust-toolchain.toml. "
                    "(The `msrv` job's `@master` is correct and exempt — it "
                    "pins to the MSRV read from Cargo.toml.)"
                )


def check_msrv_job_overrides_the_pin(problems: list[str]) -> None:
    ci = WORKFLOWS / "ci.yml"
    text = ci.read_text()
    match = re.search(r"^  msrv:$(.*?)(?=^  \S+:$)", text, re.MULTILINE | re.DOTALL)
    if match is None:
        problems.append(
            "ci.yml has no `msrv:` job. The MSRV in Cargo.toml is a promise to "
            "downstream consumers, and this repo proves it in CI rather than "
            "asserting it."
        )
        return

    if "RUSTUP_TOOLCHAIN" not in match.group(1):
        problems.append(
            "ci.yml's `msrv` job no longer sets RUSTUP_TOOLCHAIN. rustup ranks "
            "rust-toolchain.toml above the rustup default the job installs, so "
            "without that override the job builds on the pinned toolchain "
            "while reporting the crates build on their oldest supported one. "
            "It would keep passing and stop meaning anything."
        )


def check_pin_is_not_below_the_msrv(channel: str, problems: list[str]) -> None:
    match = re.search(r'^rust-version\s*=\s*"([^"]+)"', CARGO_TOML.read_text(), re.MULTILINE)
    if match is None:
        problems.append("Cargo.toml has no `[workspace.package] rust-version`.")
        return

    msrv = match.group(1)

    def as_version(value: str) -> tuple[int, int, int]:
        """Pad to major.minor.patch so `1.90` and `1.90.0` compare equal."""
        parts = [int(part) for part in value.split(".")]
        parts += [0] * (3 - len(parts))
        return tuple(parts[:3])

    if as_version(channel) < as_version(msrv):
        problems.append(
            f"rust-toolchain.toml pins {channel}, older than the MSRV {msrv} "
            "the crates advertise. CI would lint and test on a compiler that "
            "predates the floor it promises to support."
        )


def main() -> int:
    problems: list[str] = []

    channel = check_pin_is_concrete(problems)
    check_no_floating_installs(problems)
    check_msrv_job_overrides_the_pin(problems)
    if channel is not None:
        check_pin_is_not_below_the_msrv(channel, problems)

    if problems:
        for problem in problems:
            fail(problem)
        print(f"\n{len(problems)} problem(s). See docs/toolchain.md.", file=sys.stderr)
        return 1

    print(f"Toolchain pin is concrete ({channel}), no workflow floats, msrv overrides it.")
    return 0


if __name__ == "__main__":
    sys.exit(main())

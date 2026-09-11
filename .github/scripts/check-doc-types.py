#!/usr/bin/env python3
"""A document that prints a Rust type must print the type the crates have.

## Why this is a guard and not a one-time correction

The provider-facing type blocks are duplicated across `README.md`,
`docs/overview.md`, `docs/protocol-surface.md`, `docs/GUIDE.md` and
`docs/implementing-a-provider.md`. Nothing tied them to the source, so they
drifted, and #151 found four separate ways they had — two of which would have
made someone build a **non-conformant** provider:

* `FrameKind` printed as a closed set of seven. It is open: `Unknown(String)`
  exists, `from_wire` never fails, and `SPEC.md` §13 U2 makes it a MUST that a
  receiver tolerate a kind added by a later `1.x`. Someone porting the enum as
  printed writes the flag day ADR 0011 exists to prevent.
* `ContextFrame.content` printed as `String`. It is `Option<String>`, and it has
  to be: a `reference` frame carries no content at all.
* The `ContextProvider` trait printed with five methods when it has six — the
  same page told you sixty lines later to implement the missing one.
* A `capabilities()` doc comment describing `upsert`, `subscriptions` and
  `filters`, which ADR 0004 removed. The guide had copied the sentence verbatim
  from the source, so the code documented a type the code did not have.

A one-time correction fixes the four. This stops the fifth.

## What it checks

1. Any document printing `FrameKind`'s variants shows `Unknown`, **or** says
   beside it that the vocabulary is open. The rule is what matters, not the
   variant, so either satisfies it.
2. Any document printing a `content` field of `ContextFrame` prints it as
   `Option<String>`.
3. `docs/implementing-a-provider.md`'s `ContextProvider` block names every
   method the real trait declares.
4. No document, and no source doc comment, describes a `Capabilities` field the
   struct does not have.

It deliberately does not diff whole type blocks. The docs elide fields on
purpose — that is what makes them readable — and a guard that forbade eliding
would be turned off within a month. These four are the claims that were wrong
and that a reader would act on.

Standard library only: this runs in CI on a container that may have nothing
installed, and a guard whose dependency is missing is a guard that does not run.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

FRAME_RS = ROOT / "contextgraph-types" / "src" / "frame.rs"
CAPABILITY_RS = ROOT / "contextgraph-types" / "src" / "capability.rs"
PROVIDER_RS = ROOT / "contextgraph-host" / "src" / "provider.rs"

PROVIDER_GUIDE = ROOT / "docs" / "implementing-a-provider.md"

# Every document that prints protocol types for a reader to port.
TYPE_DOCS = [
    ROOT / "README.md",
    ROOT / "docs" / "overview.md",
    ROOT / "docs" / "protocol-surface.md",
    ROOT / "docs" / "GUIDE.md",
    PROVIDER_GUIDE,
]

# Capability names that were removed by ADR 0004 and kept turning up in prose.
RETIRED_CAPABILITIES = ("upsert", "subscription", "filters")

failures: list[str] = []
checks = 0


def rel(path: Path) -> str:
    return str(path.relative_to(ROOT))


def source_struct_fields(path: Path, struct: str) -> dict[str, str]:
    """Field name -> declared type, for one `pub struct` in one file."""
    body = re.search(
        rf"pub struct {struct} \{{(.*?)\n\}}", path.read_text(), re.DOTALL
    )
    if body is None:
        raise SystemExit(f"check-doc-types: no `pub struct {struct}` in {rel(path)}")
    return {
        name: ty.strip()
        for name, ty in re.findall(r"^\s*pub (\w+): ([^,\n]+),", body.group(1), re.M)
    }


def source_trait_methods(path: Path, trait: str) -> list[str]:
    body = re.search(
        rf"pub trait {trait}[^{{]*\{{(.*?)\n\}}\n", path.read_text(), re.DOTALL
    )
    if body is None:
        raise SystemExit(f"check-doc-types: no `pub trait {trait}` in {rel(path)}")
    return re.findall(r"^\s+(?:async )?fn (\w+)", body.group(1), re.M)


def frame_kind_variants() -> list[str]:
    body = re.search(r"pub enum FrameKind \{(.*?)\n\}", FRAME_RS.read_text(), re.DOTALL)
    if body is None:
        raise SystemExit("check-doc-types: no `pub enum FrameKind` in frame.rs")
    return re.findall(r"^\s{4}(\w+)", body.group(1), re.M)


# --- 1. FrameKind is open, and a document printing it must not imply otherwise.
variants = frame_kind_variants()
if "Unknown" not in variants:
    raise SystemExit(
        "check-doc-types: FrameKind no longer has an `Unknown` variant. If the "
        "vocabulary really closed, retire this check and ADR 0011 with it."
    )

# The named kinds, in declaration order, minus the open-vocabulary escape hatch.
NAMED = [v for v in variants if v != "Unknown"]
# A doc "prints the variants" if it lists the first and last named kind close
# together — the closed-set spelling this check exists to catch.
PRINTS_VARIANTS = re.compile(
    rf"{NAMED[0]},\s*(?:{NAMED[1]},\s*)?.*?{NAMED[-1]}\b", re.DOTALL
)
OPENNESS_STATED = re.compile(
    r"(?i)\bopen\b.{0,80}\bvocabular|vocabular.{0,80}\bopen\b|Unknown\(String\)|`?Unknown`?\b"
)

for doc in TYPE_DOCS:
    text = doc.read_text()
    for match in PRINTS_VARIANTS.finditer(text):
        checks += 1
        # Look in the printed block itself and the paragraph that follows it.
        window = text[match.start() : match.end() + 1200]
        if not OPENNESS_STATED.search(window):
            failures.append(
                f"{rel(doc)}: prints FrameKind's variants as a closed set. "
                "Show `Unknown(String)`, or say beside the list that the "
                "vocabulary is open and a receiver MUST NOT reject an "
                "unrecognised kind (SPEC.md §13 U2, ADR 0011)."
            )

# --- 2. `content` is optional wherever a document prints it.
frame_fields = source_struct_fields(FRAME_RS, "ContextFrame")
content_ty = frame_fields.get("content")
if content_ty is None:
    raise SystemExit("check-doc-types: ContextFrame has no `content` field")

for doc in TYPE_DOCS:
    for line in doc.read_text().splitlines():
        printed = re.match(r"\s*pub content: ([^,]+),", line)
        if printed is None:
            continue
        checks += 1
        if printed.group(1).strip() != content_ty:
            failures.append(
                f"{rel(doc)}: prints `content: {printed.group(1).strip()}`; "
                f"{rel(FRAME_RS)} declares `{content_ty}`. It has to be "
                "optional — a `reference` frame carries no content (SPEC.md §P3)."
            )

# --- 3. The provider guide's trait block names every method the trait has.
methods = source_trait_methods(PROVIDER_RS, "ContextProvider")
guide = PROVIDER_GUIDE.read_text()
block = re.search(r"pub trait ContextProvider[^{]*\{(.*?)\n\}\n```", guide, re.DOTALL)
if block is None:
    failures.append(
        f"{rel(PROVIDER_GUIDE)}: no `pub trait ContextProvider` block found. If "
        "the page stopped printing the trait, drop this check with it."
    )
else:
    printed_methods = set(re.findall(r"^\s+(?:async )?fn (\w+)", block.group(1), re.M))
    for method in methods:
        checks += 1
        if method not in printed_methods:
            failures.append(
                f"{rel(PROVIDER_GUIDE)}: the printed `ContextProvider` omits "
                f"`{method}`, which {rel(PROVIDER_RS)} declares. A provider "
                "author reads this block as the contract."
            )

# --- 4. Nothing describes a Capabilities field that does not exist.
capability_fields = set(source_struct_fields(CAPABILITY_RS, "Capabilities"))
for path in [*TYPE_DOCS, PROVIDER_RS]:
    text = path.read_text()
    if path is PROVIDER_RS:
        # Only the doc comments, so the check never trips on real code.
        text = "\n".join(
            line for line in text.splitlines() if line.lstrip().startswith("///")
        )
    for retired in RETIRED_CAPABILITIES:
        for match in re.finditer(rf"\b{retired}\w*\b", text, re.I):
            checks += 1
            window = text[max(0, match.start() - 400) : match.end() + 400]
            # A mention is fine when it says the thing is gone. ADR 0004 is the
            # citation that says so, and the CHANGELOG/ADR index quote it.
            if re.search(r"(?i)ADR 0004|0004-dead-capability|removed|retired|no\b.{0,20}\bwrite method", window):
                continue
            if retired in capability_fields:
                continue
            failures.append(
                f"{rel(path)}: describes a `{match.group(0)}` capability. "
                "`Capabilities` has "
                f"{', '.join(sorted(capability_fields))} and nothing else — "
                "ADR 0004 removed the rest. Cite ADR 0004 if you are naming it "
                "to say it is gone."
            )

if failures:
    print("check-doc-types: FAILED", file=sys.stderr)
    for failure in failures:
        print(f"  - {failure}", file=sys.stderr)
    sys.exit(1)

print(
    f"check-doc-types: OK — {checks} claims checked against "
    "contextgraph-types and contextgraph-host; every printed type matches "
    "the type the crates declare."
)

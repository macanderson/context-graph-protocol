# 20. The `range` of `file` provenance is a published line grammar, and its end clamps

- Status: accepted
- Date: 2026-09-23
- Resolves #183. Adds `SPEC.md` §6.2.1 and requirement **F17**. Builds on the
  §6.2 no-normalization clause and on the F5-bytes verifier of #12.

## Context

F5 requires a digest over "the exact UTF-8 source bytes addressed by `uri` +
`range`". `SPEC.md` used `range` five times and defined it nowhere. The
reference host's `extract_line_range` had quietly chosen answers to every
question a definition would have to settle — prefix, indexing base, whether the
end is inclusive, whether a line owns its newline, what happens past the end of
the file — and its own module docs said so: "`SPEC.md` §6.2 does not fix a
`range` grammar".

A digest is compared byte for byte, so two implementations that answer any of
those questions differently compute different digests over the same file, and
the verifier reports the difference as a **mismatch** — the signal §6.5.4 and
the F5-bytes check treat as tampering. The failure mode of the missing grammar
was a false accusation, not a parse error. And because `range` is
length-prefixed into the attestation preimage (§6.5.1), two implementations
that disagree about it do not merely disagree about a digest; they are one
careless edit from disagreeing about a signature.

It also escaped conformance entirely. A provider emitting `range: "120-160"`
got `Unreadable` from the host, and `provenance-fixture-consistency` skips an
unreadable link — correctly, since a remote provider's files are not on the
host's disk. So a provider whose digests nobody could check passed the whole
suite.

The issue named four decisions that belong to the maintainer: the grammar
itself, whether an end past the last line clamps or errors, whether non-line
ranges are reserved or forbidden, and whether `range` gets its own anchor.
SCR-002 says to take the durable option and record it; this is that record.

## Decision

**1. The grammar is the one every producer in the ecosystem already emits.**

```abnf
line-range  = %x4C line-number [ "-" line-number ]  ; "L", uppercase only
line-number = %x31-39 *DIGIT                         ; decimal, >= 1, no leading zero
```

Lines split on LF only; a line includes its LF; a final LF opens no extra line;
CR is content; 1-indexed; end inclusive; `L5` means `L5-5`. The ripgrep and
tree-sitter reference providers, `contextgraph-refprov`, the example-docs
fixture, the four SDK examples, `examples/reference-messages.json` and the
`SPEC.md` §6 example all emit exactly this. Codifying it changes the meaning of
no range any of them has ever produced.

**2. An end past the last line clamps. A start past it is unverifiable.** This
is the asymmetry the issue called out as one "no one would reproduce by
guessing" — so it is now written down rather than reversed, for three reasons:

- *The digest, not the range, is the integrity check.* A clamped span must
  still hash to the declared digest, so clamping can never make altered bytes
  verify. The only thing the choice decides is how a file that shrank beneath a
  range is reported: clamping reports a **mismatch**, which is true — the bytes
  changed — where erroring would report *unverifiable* and hide the change
  behind a softer verdict.
- *A start past the end is different in kind.* It addresses no bytes at all,
  and "the digest of nothing" is not a thing a range can mean. There is no
  clamped reading of it that is not a guess.
- *The ecosystem depends on it.* The reference fixture and all four SDK
  examples declare `L1-40` over a four-line file and digest the whole file.
  Making an out-of-range end an error would turn every reference example in the
  repository red on the day the grammar was published, which is the clearest
  possible evidence of which behaviour is the contract.

A producer **SHOULD** still emit an end within the resource. The rule is about
what a verifier does, not an invitation.

**3. Line numbers are unbounded.** An end too large for the verifier's integer
type clamps like any other end past the last line; a start that large lies
past the last line of every resource that can exist. The reference
implementation saturates to `usize::MAX` rather than failing to parse, so the
meaning of `L1-99999999999999999999999` does not depend on whether the verifier
is 32- or 64-bit.

**4. Every other spelling is reserved, and reported unverifiable.** Byte,
character and column ranges are not forbidden forever; `contextgraph/1`
simply does not define one. Reservation is what makes a later definition safe:
a verifier built before it reports the new form unverifiable, which degrades
the digest check rather than faking it — the stance F8 takes toward an unknown
signature algorithm. A verifier **MUST NOT** fall back to the whole resource
(that confirms bytes the range never named) and **MUST NOT** report a mismatch
(that calls a grammar disagreement tampering). Forbidding the forms outright
would have had the same effect today and closed a door tomorrow; reserving them
costs nothing.

**5. `range` gets its own anchor, F17, rather than riding F5.** F5 is a rule
about the digest's *spelling* and is verified by a shape check. F17 is a rule
about *which bytes* and is verified two ways — statically by `frame-validity`
(the grammar) and dynamically by `provenance-fixture-consistency` (the bytes) —
plus a reference vector file. One anchor per independently checkable claim is
how the rest of the table is organised.

**6. One implementation, shared.** The grammar lives in
`contextgraph_types::LineRange`, beside `is_well_formed_digest`, because a
non-Rust implementer reads that crate to know what to build and because the
conformance suite's grammar check and the host's byte verifier must not be able
to drift apart. `contextgraph-host`'s `extract_line_range` now delegates to it,
and `frame-validity` calls `ContextFrame::provenance_with_unrecognised_ranges`.

## Consequences

- `tests/vectors/range-vectors.json` publishes the addressed bytes and the
  `sha256:` digest for every rule, and the reason for every unverifiable case.
  The values were computed from the prose by a separate script, not read off
  the Rust code, and two Rust suites reconcile against them:
  `contextgraph-types/tests/range_vectors.rs` (addressing) and
  `contextgraph-host/tests/range_vectors.rs` (the digest, end to end through
  `verify_provenance_digest`, including a bait whole-resource digest that a
  fallback would wrongly confirm).
- `frame-validity` now fails a `file` link whose `range` is outside the grammar
  or inverted. The example-docs fixture gains `--misbehave unrecognised-range`
  (`1-40` for `L1-40`, nothing else wrong) as its witness; no conformance check
  was added, so no published count changes.
- Two inputs the host used to accept are now unverifiable: a zero-padded line
  number (`L05`) and a signed one (`L+5`). Both were artefacts of Rust's
  `str::parse`, not decisions — a verifier in any other language written from
  the grammar would already have rejected them — and no producer in this
  repository or its SDKs emits either. A single spelling per line number is
  what lets hosts compare `range` as a string, which cross-provider dedup does.
- The attestation vectors carry `range` values of the GitHub `L10-L20` shape.
  They are left exactly as they are: §6.5.1 encodes `range` as opaque bytes and
  never parses it, those values are encoding inputs rather than digest claims,
  and changing a published vector is itself a wire break. `SPEC.md` §6.2.1 says
  so, so nobody copies them as an example of the grammar.

## Alternatives considered

**An end past the last line is an error.** Symmetric and easy to explain. It
is also the choice that breaks every reference example in the repository and
turns a shrunken file from a mismatch into an unverifiable link — trading an
accurate finding for a vaguer one, to buy symmetry nobody needed.

**Accept GitHub's `L10-L20` as well.** It is a familiar convention and the
attestation vectors already contain it. It would give one span two spellings,
and hosts compare `range` as a string when deduplicating citations, so two
providers citing the same lines would stop matching. §6.1 makes the same
argument for timestamps — one spelling per instant — and it holds here.

**Leave the grammar host-defined, as it was.** Cheapest, and exactly the
defect: §1 promises a provider can be written from the spec alone, and a
digest that depends on an unpublished convention makes that false in the one
place where the consequence is a false tampering report.

# Cross-language reference vectors

Two fixtures live here: the provenance-attestation vectors and the
provenance `range` vectors. Both are data a port reads, never a transcription
it copies.

## Attestation vectors

`attestation-vectors.json` is the single copy of the provenance-attestation
reference vectors (`SPEC.md` §6.5). Four test suites read it:

| Suite | File |
| --- | --- |
| Rust (reference) | [`contextgraph-types/tests/attestation_vectors.rs`](../../contextgraph-types/tests/attestation_vectors.rs) |
| TypeScript | [`sdk/typescript/test/attest.test.ts`](../../sdk/typescript/test/attest.test.ts) |
| Python | [`sdk/python/tests/test_attest.py`](../../sdk/python/tests/test_attest.py) |
| Go | [`sdk/go/contextgraph/attest/vectors_test.go`](../../sdk/go/contextgraph/attest/vectors_test.go) |

The Rust suite still writes every value out inline and then asserts this file
agrees with it, so the reference implementation stays readable as a
specification while the three ports cannot reconcile against a stale
transcription. A port that copied these digests into its own source would go
green forever after someone corrected one here.

**A diff to any value in this file is a wire-breaking change** and needs a new
major family (`SPEC.md` §15). Adding a *new* vector is not: it publishes a case
the set could not previously distinguish, which is what #93 did for non-ASCII
input, non-power-of-two Merkle trees, inclusion proofs and signatures, and what
#124 did for the presence byte. The `empty_uri` / `absent_uri` pair differs in
nothing a JSON reader returning a plain string can see, so a port whose optional
type is just `string` publishes one chain head where this file publishes two —
which is exactly how the Go SDK came to report `commitment_mismatch` on honest
frames.

All four suites assert that pair's exact encoding bytes **and** chain heads, not
merely that the two differ (#125), so a port collapsing `""` to absent fails a
named test in each language:

| Suite | Test |
| --- | --- |
| Rust | `the_published_empty_versus_absent_vectors_hold`, `the_shared_fixture_publishes_exactly_these_values` |
| TypeScript | `a present-but-empty field encodes to the published bytes, distinct from an absent one` |
| Python | `test_a_present_but_empty_field_encodes_to_the_published_bytes`, `test_a_present_but_empty_field_has_its_own_published_chain_head` |
| Go | `TestLinkEncodingMatchesThePublishedBytes`, `TestADecodedEmptyURIKeepsTheChainHeadItsSignerComputed` |

Each also checks that the fixture still spells `"uri": ""` out, because a loader
that normalized it away would test the absent case twice and pass.

This directory is deliberately not `tests/fixtures/`, which
[`schema/validate-examples.py`](../../schema/validate-examples.py) globs and
validates against the lifecycle **record** schema. These are not records.

### Regenerating

There is no regeneration command, on purpose. The values are fixtures rather
than an assertion about the current code — recomputing them would let an
encoding change rewrite its own oracle. To publish a new vector, add it to the
Rust suite with a placeholder, run

```sh
cargo test -p contextgraph-types --features attestation --test attestation_vectors
```

read the value out of the failure, and write it into both files.

## Range vectors

`range-vectors.json` pins which bytes a `file` provenance `range` addresses
(`SPEC.md` §6.2.1, F17). Each case names a resource, a `range` (absent means
the whole resource), and either the exact addressed `bytes` with their
`sha256:` `digest`, or the reason the range is `unverifiable`. Resources and
bytes are JSON strings, so CR, LF and non-ASCII are exact; hash the UTF-8
encoding of the string with no normalization.

| Suite | File | Checks |
| --- | --- | --- |
| Rust (addressing) | [`contextgraph-types/tests/range_vectors.rs`](../../contextgraph-types/tests/range_vectors.rs) | `LineRange` selects exactly the published bytes and refuses exactly the unverifiable cases, for the stated reason |
| Rust (digest, end to end) | [`contextgraph-host/tests/range_vectors.rs`](../../contextgraph-host/tests/range_vectors.rs) | the host re-reads a real file and verifies every published digest, and reports every unverifiable case `Unreadable` even when handed the whole-resource digest |

The values were computed from the §6.2.1 prose by a script independent of the
Rust code, so the Rust suites reconcile against a second opinion rather than a
snapshot of their own output. As with the attestation vectors, **a diff to any
published value changes which bytes a digest covers** and is wire-breaking;
adding a case is not.

The `range` strings inside `attestation-vectors.json` (`L10-L20`, `L1-L2`) are
not examples of this grammar. §6.5.1 encodes `range` as opaque bytes and never
parses it, so those values are encoding inputs, not digest claims — and they
stay as published, because changing a published vector is itself a wire break.

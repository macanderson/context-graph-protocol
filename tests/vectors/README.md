# Cross-language attestation vectors

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
input, non-power-of-two Merkle trees, inclusion proofs and signatures.

This directory is deliberately not `tests/fixtures/`, which
[`schema/validate-examples.py`](../../schema/validate-examples.py) globs and
validates against the lifecycle **record** schema. These are not records.

## Regenerating

There is no regeneration command, on purpose. The values are fixtures rather
than an assertion about the current code — recomputing them would let an
encoding change rewrite its own oracle. To publish a new vector, add it to the
Rust suite with a placeholder, run

```sh
cargo test -p contextgraph-types --features attestation --test attestation_vectors
```

read the value out of the failure, and write it into both files.

# Running the Context Graph Protocol conformance suite

"Context Graph Protocol conformant" means *green on `contextgraph-conformance`'s suite for your declared
capability set* — a checkable claim, which is what makes third-party
adoption safe. This page covers both ways to run it: the `contextgraph-inspect` CLI
binary, and calling the suite as a library from your own test harness.

## The checks

Fourteen provider checks. The authoritative list is the `CHECK_*` constants in
`contextgraph-conformance/src/lib.rs`; this table is kept honest against them by
`.github/scripts/check-conformance-counts.py`, which fails CI when a count or a
check name here stops matching the code.

| check | what it proves | skipped when |
|---|---|---|
| `handshake` | the provider completes the handshake and reports a non-empty identity and capabilities | never — a failed handshake skips the checks that depend on it |
| `consent-scope` | declared egress scopes are well-formed and consistent with the `egress` flag | never |
| `frame-validity` | every returned frame is citable and scored honestly: `score` in `[0, 1]`, non-empty `title` and `citation_label`, and every `file` provenance `range` in the §6.2.1 line grammar (§F17) | never |
| `verify-honesty` | a provider advertising `verify` answers about digests it actually served | the provider does not advertise `verify`, or served no frame carrying a `content_digest` |
| `budget-honesty` | returned frames' summed `token_cost` never exceeds the query's `max_tokens` | never |
| `as-of-temporal` | at each of two `as_of` pins, every returned frame's `[valid_from, valid_to)` window contains the pin — nothing not yet true, nothing no longer true (SPEC.md Q2) | never |
| `kinds-filter` | a kind-filtered query narrows to that kind (§Q1) | the provider declares no query kinds, or declares one outside the base `FrameKind` vocabulary |
| `anchor-relevance` | a graph provider's frames anchor on a `uri` or a relation target (§G3/§G4) | the provider does not declare `capabilities.graph`, or served no anchorable frame |
| `provenance-fixture-consistency` | `file` provenance digests match the bytes they name — catching a stale or forged digest that passes §F5's grammar | never |
| `shutdown-clean` | the provider acknowledges shutdown and tears down without error | never |
| `malformed-input-tolerance` | malformed input does not crash the provider — an unparseable line, a JSON value that is not an envelope object (`42`), and a `query` envelope with its payload missing — and each is answered `bad_request` or, for garbage with no id to answer, ignored (§R1) | **stdio only** — the probe is wire-level |
| `embedding-fingerprint` | a declared `embeddings_fingerprint` is not contradicted by a `bad_request` (§E1) | **stdio only**, and when the provider declares no fingerprint |
| `correlation` | request ids are echoed back (§H4) | **stdio only**, and when the provider does not declare `capabilities.correlation` |
| `attestation` | a provider offering attestations produces ones that verify (§6.5); one in a scheme this build cannot check fails as *uncheckable* (`UnknownAlgorithm`), never as forged and never as a skip ([ADR 0027](./adr/0027-an-attestation-the-suite-cannot-check-is-not-certified.md)) | **stdio only**, and when the handshake failed (the `handshake` check owns that) |

A run's overall verdict, `ConformanceReport::passed()`, is true iff **no check
failed**. A skipped check never fails a run — which is why the "skipped when"
column matters: an HTTP or in-process provider legitimately skips the four
wire-level probes, and a provider that declares no graph capability legitimately
skips `anchor-relevance`. Skipping is not passing, and the report distinguishes
them.

A skip always means *does not apply*, never *could not decide*. A check that
binds what the provider declared but cannot reach a verdict fails, with evidence
saying why — otherwise `passed()` would certify what nobody checked. The case
that settled it is an attestation signed in a scheme this build does not know:
the provider published keys and served signatures, so `attestation` applies,
and it reports `fail` naming `UnknownAlgorithm` rather than skipping
([ADR 0027](./adr/0027-an-attestation-the-suite-cannot-check-is-not-certified.md)).
CI's `conformance-green.sh` and `conformance-external.sh` go one step further
and fail on any skip, because the providers they judge declare every capability.

The suite is deliberately adversarial. Pointed at a provider that lies about
costs, emits an out-of-range score, omits a citation label, serves a digest that
does not match its bytes, or dies mid-query, the matching check fails loudly with
an evidence string naming the exact violation — never a bare "not conformant".

## Option A: the `contextgraph-inspect` CLI

Install the binary (it ships inside the `contextgraph-conformance` crate):

```bash
cargo install contextgraph-conformance
```

Run it against a stdio provider:

```bash
contextgraph-inspect stdio -- ./my-provider --some-flag
```

Or a remote HTTP provider:

```bash
contextgraph-inspect http https://my-provider.example.com/contextgraph
```

Add `--query "some goal text"` to also fire an interactive test query before
the conformance run, and `--json` to get the report as machine-readable JSON
(handy for CI) instead of colored terminal output.

`contextgraph-inspect` exits with a non-zero status when the provider is **not**
conformant, so it's directly usable as a CI gate:

```bash
contextgraph-inspect stdio --json -- ./my-provider > report.json || {
  echo "provider is not Context Graph Protocol conformant"; cat report.json; exit 1;
}
```

Sample colored output for a fully conformant provider:

```
── conformance: stdio: ./my-provider ──
  ✓ handshake
      provider 'my-provider' v0.1.0 — data-flow reads=true writes=false egress=false; query kinds=["doc"], graph=false
  ✓ consent-scope
      declared egress scopes [] are well-formed and consistent with egress=false
  ✓ frame-validity
      2 frame(s) — all scores in [0,1], titles + citation labels present
  ✓ budget-honesty
      2 frame(s) sum to 128 token(s), within the 4096-token budget
  ✓ as-of-temporal
      as_of pin respected — none of the 2 returned frame(s) is dated after it
  ✓ provenance-fixture-consistency
      every file-provenance digest matches the bytes it names
  ✓ shutdown-clean
      provider acknowledged shutdown and tore down cleanly
  ✓ malformed-input-tolerance
      provider survived 3 malformed input(s) and answered a valid query after each: answered `bad_request` to an unparseable line; …
  – verify-honesty
      provider does not advertise `verify`; a host falls back to re-querying its frames (§4)
  – kinds-filter
      provider declares no query kinds, so §Q1 has no kind to narrow to
  – anchor-relevance
      provider does not declare capabilities.graph, so §G3/§G4 do not bind it
  – embedding-fingerprint
      provider declares no embeddings_fingerprint, so §E1 has no dimension to contradict
  – correlation
      provider does not declare capabilities.correlation, so §H4 does not bind it
  – attestation
      provider offered no attestations, so there is nothing to verify
  CONFORMANT — 8 passed, 6 skipped
```

### Checking a record and its attestation

A Context Exchange Provider has a second thing to check, one that needs no
running provider: the records it appends. `contextgraph-inspect record` computes
a record's `record_hash` (profile `LH1`), prints the canonical bytes that hash
is taken over, checks a stored hash, and verifies a detached record attestation
(profile `LC3`, [ADR 0017](./adr/0017-record-hash-and-record-attestation.md)).
Every verb calls the same `contextgraph_types::record_attest` functions the
reference suite does, so its answer is the reference answer.

```bash
contextgraph-inspect record hash record.json       # print the record_hash
contextgraph-inspect record preimage record.json   # print the canonical RFC 8785 (JCS) bytes
contextgraph-inspect record verify record.json     # is the stored record_hash current?
contextgraph-inspect record attest attestation.json --record record.json --key <public key hex>
```

Any file argument may be `-` to read standard input.

`preimage` is the one to reach for when your implementation and the reference
disagree on a digest. The digest says only that two canonicalizations differ;
the preimage says where. It prints the exact bytes with no trailing newline, so
`contextgraph-inspect record preimage record.json | sha256sum` reproduces the
hex of the `record_hash`, and a byte diff against your own canonicalization
locates the member that sorts or prints differently:

```bash
diff <(contextgraph-inspect record preimage record.json) <(my-cep canonicalize record.json)
```

`verify` prints `current` with the hash, or `stale` with both the stored and
the recomputed hash, or `unhashed` when the record carries no `record_hash` at
all.

`attest` recomputes the record's hash from its content rather than trusting the
stored member, so a record edited after it was signed reports a commitment
mismatch rather than passing. An auditor who holds the hash but not the record
passes `--record-hash sha256:<hex>` in place of `--record`. `--key` is the
attester's Ed25519 public key, 32 bytes as 64 hex digits. The verdict names the
failure: commitment mismatch, bad signature, unknown algorithm, or a malformed
key, signature, or `signed_record_hash`.

The fixtures in [`tests/fixtures/`](../tests/fixtures/) make a worked example.
`record-attestation.json` signs `observation.json` under the published test key
in `record-attestation-key.json`:

```bash
contextgraph-inspect record attest tests/fixtures/record-attestation.json \
  --record tests/fixtures/observation.json \
  --key 495b4a0a4a16c5444d8626a7ae0bc6eca613676b51fb947238cb8238baa9fde5
```

Exit statuses are stable, for scripts:

| status | meaning |
|---|---|
| `0` | positive answer: the hash or preimage printed, the stored hash is current, or the attestation is valid |
| `1` | negative answer: the stored hash is stale or absent, or the attestation does not verify |
| `2` | no answer: the input could not be read, is not JSON, is not a record, or the arguments are wrong |

`contextgraph-conformance/tests/inspect_record.rs` runs each verb against the
fixtures in `tests/fixtures/`.

## Option B: as a library, from your own test suite

`run_conformance` is a plain async function that returns a typed
`ConformanceReport` — call it directly from an integration test:

```rust,no_run
use contextgraph_conformance::{ProviderTarget, run_conformance};

#[tokio::test]
async fn my_provider_is_contextgraph_conformant() {
    let report = run_conformance(ProviderTarget::Stdio {
        program: env!("CARGO_BIN_EXE_my_provider").to_string(),
        args: vec![],
    })
    .await;

    assert!(
        report.passed(),
        "not conformant: {:?}",
        report.failures().collect::<Vec<_>>()
    );
}
```

`ProviderTarget` has three variants:

- `ProviderTarget::Stdio { program, args }` — spawn a child process. Every check
  can run; which ones actually do still depends on what the provider declares.
- `ProviderTarget::Http { url }` — POST to a remote endpoint. The four
  wire-level probes are skipped: `malformed-input-tolerance`,
  `embedding-fingerprint`, `correlation` and `attestation`.
- `ProviderTarget::InProcess(Box<dyn ContextProvider>)` — an
  already-constructed in-process provider, e.g. a built-in you want to
  regression-test in your own workspace without spawning anything
  (`malformed-input-tolerance` is skipped for the same reason).

`ConformanceReport` gives you `passed()`, `failures()` (an iterator over just
the failed checks), and `tally()` (`(passed, failed, skipped)` counts) for
building your own reporting on top.

## The `contextgraph-example-docs` fixture, for reference

`contextgraph-conformance`'s own test suite
(`contextgraph-conformance/tests/conformance_suite.rs`) runs the checks against a real
bundled reference provider, `contextgraph-example-docs`, including a `--misbehave
<mode>` flag that deliberately trips one check at a time (`lying-costs`,
`bad-score`, `empty-citation`, `bad-version`, `crash-on-query`,
`crash-on-garbage`, `crash-on-missing-payload`). Reading those tests is the fastest way to see exactly
what evidence string each failure mode produces, and doubles as proof that
the suite genuinely catches a broken provider rather than rubber-stamping
everything.

The two digest-integrity modes are deliberately distinct: `malformed-digest`
emits an ungrammatical stub that `frame-validity` (§F5 grammar) rejects before
any bytes are read, while `stale-digest` emits a *well-formed* `sha256:` digest
that simply does not match the backing file's bytes — caught only by
`provenance-fixture-consistency`, which re-reads the fixture's own files and
re-hashes them (§6.2). A third mode, `unrecognised-range`, keeps the digest
honest and writes the range as `1-40` instead of `L1-40`: no conforming
verifier can locate those bytes, so `frame-validity` fails it on the §6.2.1
grammar (§F17) — before that rule existed, the host reported the link
unreadable, the byte check skipped it, and the provider passed.

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
| `frame-validity` | every returned frame is citable and scored honestly: `score` in `[0, 1]`, non-empty `title` and `citation_label` | never |
| `verify-honesty` | a provider advertising `verify` answers about digests it actually served | the provider does not advertise `verify`, or served no frame carrying a `content_digest` |
| `budget-honesty` | returned frames' summed `token_cost` never exceeds the query's `max_tokens` | never |
| `as-of-temporal` | no returned frame is dated after the `as_of` pin — content that was not yet true | never |
| `kinds-filter` | a kind-filtered query narrows to that kind (§Q1) | the provider declares no query kinds, or declares one outside the base `FrameKind` vocabulary |
| `anchor-relevance` | a graph provider's frames anchor on a `uri` or a relation target (§G3/§G4) | the provider does not declare `capabilities.graph`, or served no anchorable frame |
| `provenance-fixture-consistency` | `file` provenance digests match the bytes they name — catching a stale or forged digest that passes §F5's grammar | never |
| `shutdown-clean` | the provider acknowledges shutdown and tears down without error | never |
| `malformed-input-tolerance` | a garbage line on the wire does not crash the provider | **stdio only** — the probe is wire-level |
| `embedding-fingerprint` | a declared `embeddings_fingerprint` is not contradicted by a `bad_request` (§E1) | **stdio only**, and when the provider declares no fingerprint |
| `correlation` | request ids are echoed back (§H4) | **stdio only**, and when the provider does not declare `capabilities.correlation` |
| `attestation` | a provider offering attestations produces ones that verify (§6.5) | **stdio only**, and when the provider returned no frames to attest |

A run's overall verdict, `ConformanceReport::passed()`, is true iff **no check
failed**. A skipped check never fails a run — which is why the "skipped when"
column matters: an HTTP or in-process provider legitimately skips the four
wire-level probes, and a provider that declares no graph capability legitimately
skips `anchor-relevance`. Skipping is not passing, and the report distinguishes
them.

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
      provider ignored a malformed line and still answered a valid query
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
`crash-on-garbage`). Reading those tests is the fastest way to see exactly
what evidence string each failure mode produces, and doubles as proof that
the suite genuinely catches a broken provider rather than rubber-stamping
everything.

The two digest-integrity modes are deliberately distinct: `malformed-digest`
emits an ungrammatical stub that `frame-validity` (§F5 grammar) rejects before
any bytes are read, while `stale-digest` emits a *well-formed* `sha256:` digest
that simply does not match the backing file's bytes — caught only by
`provenance-fixture-consistency`, which re-reads the fixture's own files and
re-hashes them (§6.2).

# Lifecycle-profile fixtures (canonical home)

This directory is the **canonical home** for the Context Exchange Provider
profile's example records and golden `record_hash` vectors (issue
[#28](https://github.com/macanderson/context-graph-protocol/issues/28),
requirement `LF1`). It resolves the earlier draft's open question of *which repo
owns the vectors*: they live here, in the protocol repo, so a third-party CEP and
the reference conformance suite validate against the **same bytes**.

## Layout

- One fixture per portable `record_kind` (12 of them):
  `observation.json`, `knowledge.json`, `memory.json`, `directive.json`,
  `record_proposal.json`, `evidence.json`, `artifact_contract.json`,
  `contract_validation.json`, `outcome_assessment.json`, `promotion_event.json`,
  `context_use.json`, `context_use_feedback.json`. The filename stem **is** the
  `record_kind`.
- `record-hash-vectors.json` — the canonical RFC 8785 (JCS) preimage **text** of
  every record fixture, beside the hash it produces. The text is the part that
  makes a vector usable: when a third-party implementation computes a different
  digest, the digest cannot say where the two canonicalizations diverged and a
  byte diff of the preimage can.
- `record-attestation.json` — a **detached** `RecordAttestation` (it is not a
  record kind; it is ledger metadata beside a record, so it is validated against
  `#/$defs/RecordAttestation`, never the root record schema). Its
  `signed_record_hash` signs `observation.json`'s `record_hash`, and its
  `signature` is a real Ed25519 signature that the conformance suite verifies.
- `record-attestation-key.json` — the **published test key** that signs it: the
  seed, the public key it derives, and the exact message the signature covers.
  It is committed to a public repository and forgeable by anyone, which is the
  point — a vector nobody can reproduce is a shape example. Never use it for
  anything real.

## What validates these

- **Structure and cross-language vector checks:**
  `python3 schema/validate-examples.py` validates every record fixture against
  [`schema/contextgraph-lifecycle-record.schema.json`](../../schema/contextgraph-lifecycle-record.schema.json)
  and the attestation against `#/$defs/RecordAttestation`. It then checks, in
  Python and with no JCS library, that each published canonical text hashes to
  its published `record_hash` and parses back to the fixture with `record_hash`
  removed — the two properties an implementer who has neither Rust nor a
  canonicalizer actually relies on.
- **Round-trip, envelope invariants, hash, and signature:**
  [`contextgraph-conformance/tests/lifecycle_profile_examples.rs`](../../contextgraph-conformance/tests/lifecycle_profile_examples.rs)
  deserializes each fixture through `contextgraph_types::ContextRecord`, checks
  the profile invariants, **recomputes** `record_hash` and the canonical
  preimage, and **verifies** the attestation under the published key. Every
  recomputation calls `contextgraph_types::record_attest` rather than a copy of
  the rule kept in the test.

## Regenerating

`record_hash` is content-addressed (profile `LH1`). If you edit a fixture's
content, refresh everything derived from it:

```sh
REGENERATE_LIFECYCLE_HASHES=1 cargo test -p contextgraph-conformance \
  --test lifecycle_profile_examples
```

This rewrites each fixture's `record_hash` in place preserving field order,
re-signs `record-attestation.json`, and rebuilds `record-hash-vectors.json` and
the derived members of `record-attestation-key.json`. Then re-run without the
env var to verify.

## Worked example — how `record_hash` is computed (RFC 8785 JCS)

`record_hash = "sha256:" + hex(sha256(JCS(record without its record_hash member)))`.

Take `observation.json`. **Step 1** — remove its own top-level `record_hash`
member from the preimage. Removed, not blanked: the record hashes the same
whether the member is absent, correct, or wrong, so a producer never has to
invent a placeholder. **Step 2** — canonicalize the remaining object with **RFC
8785 (JCS)**: sort object members by UTF-16 code unit, minimal separators (`,`
and `:`), no insignificant whitespace, and the RFC 8785 number form (ECMAScript
shortest round-trip — `0.82`, not `0.820`). For `observation.json` that yields
exactly these bytes (one line, shown wrapped here):

```
{"confidence":0.82,"lineage_id":"lin_obs_0001","observed_at":"2026-07-29T14:00:00Z","origin":"observed","provenance":{"origin_authority_id":"authority_acme","origin_provider_id":"provider_example","producer_kind":"agent","producer_ref":"agent://trace-miner"},"record_id":"rec_obs_0001","record_kind":"observation","record_status":"active","schema_version":"contextgraph/lifecycle/1.0-draft","scope":{"repository_id":"repo_stella","session_id":"sess_412","workspace_id":"ws_main"},"sensitivity":"internal","sharing_scope":"repository","statement":"the api handler retries three times before surfacing a 502","subject_ref":"trace_run_991"}
```

**Step 3** — SHA-256 the UTF-8 of that string and prefix `sha256:`:

```
sha256:b45eebfdfe7e6e5056bf25d84864cf9acd731eef120a1f6de129fb788c3b34dc
```

which is exactly the `record_hash` stored in `observation.json`, and the entry
`record-hash-vectors.json` publishes for it.

Reproducing those bytes without a JCS library is possible for *these* fixtures
and is not the same thing as implementing RFC 8785. A Python
`json.dumps(sort_keys=True, separators=(",", ":"), ensure_ascii=False)` matches
here because every fixture's member names are ASCII and every number round-trips
identically under CPython's `repr` and ECMAScript's `Number::toString`. Neither
is guaranteed by the profile — a member name outside the BMP would sort
differently, and a number near a precision boundary would print differently — so
compute `record_hash` with a conforming canonicalizer and use the shortcut only
to sanity-check a vector.

> The **detached attestation is never part of the preimage** (profile `LH3`):
> `record_hash` is computed over the record alone, so signing or rotating a key
> never changes a record's identity.

## Worked example — what the attestation signs

The signature does **not** cover the bare digest. It covers the domain tag
`contextgraph/attest/1/record` followed by the digest's 32 raw bytes (profile
`LC4`), which is what stops a signature produced at the frame layer — or by an
unrelated system that hashed the same JSON — from being presented as a record
attestation. `record-attestation-key.json` publishes those bytes as
`signed_message_hex`, so an implementation in any language can build them and
check the signature with any Ed25519 library.

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
- `record-attestation.json` — a **detached** `RecordAttestation` (it is not a
  record kind; it is ledger metadata beside a record, so it is validated against
  `#/$defs/RecordAttestation`, never the root record schema). Its
  `signed_record_hash` signs `observation.json`'s `record_hash`.

## What validates these

- **Structure:** `python3 schema/validate-examples.py` validates every record
  fixture against
  [`schema/contextgraph-lifecycle-record.schema.json`](../../schema/contextgraph-lifecycle-record.schema.json)
  and the attestation against `#/$defs/RecordAttestation`.
- **Round-trip + envelope invariants + hash:**
  [`contextgraph-conformance/tests/lifecycle_profile_examples.rs`](../../contextgraph-conformance/tests/lifecycle_profile_examples.rs)
  deserializes each fixture through `contextgraph_types::ContextRecord`, checks
  the profile invariants, and **recomputes** `record_hash`.

## Regenerating the hashes

`record_hash` is content-addressed (profile `LH1`). If you edit a fixture's
content, refresh its hash:

```sh
REGENERATE_LIFECYCLE_HASHES=1 cargo test -p contextgraph-conformance \
  --test lifecycle_profile_examples
```

This rewrites each fixture's `record_hash` (and the attestation's
`signed_record_hash`) in place, preserving field order, then re-run without the
env var to verify.

## Worked example — how `record_hash` is computed (RFC 8785 JCS)

`record_hash = "sha256:" + hex(sha256(JCS(record without its record_hash member)))`.

Take `observation.json`. **Step 1** — remove its own `record_hash` member from
the preimage. **Step 2** — canonicalize the remaining object with **RFC 8785
(JCS)**: sort object members by code point, minimal separators (`,` and `:`), no
insignificant whitespace, and the RFC 8785 number form (ECMAScript shortest
round-trip — `0.82`, not `0.820`). For `observation.json` that yields exactly
these 637 bytes (one line, shown wrapped here):

```
{"confidence":0.82,"lineage_id":"lin_obs_0001","observed_at":"2026-07-29T14:00:00Z","origin":"observed","provenance":{"origin_authority_id":"authority_acme","origin_provider_id":"provider_example","producer_kind":"agent","producer_ref":"agent://trace-miner"},"record_id":"rec_obs_0001","record_kind":"observation","record_status":"active","schema_version":"contextgraph/lifecycle/1.0-draft","scope":{"repository_id":"repo_stella","session_id":"sess_412","workspace_id":"ws_main"},"sensitivity":"internal","sharing_scope":"repository","statement":"the api handler retries three times before surfacing a 502","subject_ref":"trace_run_991"}
```

**Step 3** — SHA-256 the UTF-8 of that string and prefix `sha256:`:

```
sha256:b45eebfdfe7e6e5056bf25d84864cf9acd731eef120a1f6de129fb788c3b34dc
```

which is exactly the `record_hash` stored in `observation.json`. The reference
Rust `serde_json_canonicalizer` and a
`json.dumps(sort_keys=True, separators=(",", ":"), ensure_ascii=False)` Python
canonicalizer both reproduce these bytes and this hash — that byte-agreement is
the interop guarantee the vectors exist to pin (profile `LH2`).

> The **detached attestation is never part of the preimage** (profile `LH3`):
> `record_hash` is computed over the record alone, so signing or rotating a key
> never changes a record's identity.

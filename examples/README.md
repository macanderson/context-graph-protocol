# Context Graph Protocol wire examples

Reference wire transcripts for the Context Graph Protocol. These are the exact
JSON shapes a host and provider exchange — useful when implementing a provider
in **any language**, because you can diff your own output against them.

There is no separate IDL; the [`contextgraph-types`](https://crates.io/crates/contextgraph-types)
structs serialized by `serde_json` *are* the protocol. For the type definitions
see [protocol-surface.md](../docs/protocol-surface.md); for the build guide see
[implementing-a-provider.md](../docs/implementing-a-provider.md); for the
normative rules these examples follow see
[protocol-surface.md § Conformance requirements](../docs/protocol-surface.md#conformance-requirements).

**These examples are machine-checked** against the [JSON Schema](../schema/contextgraph-envelope.schema.json).
The same script checks that the annotated session below quotes the transcript
exactly, and that every other JSON block on this page is a valid envelope.
Run `python3 schema/validate-examples.py` to verify they conform, or point your
own validator (any language — `ajv`, Python `jsonschema`, Rust `jsonschema`
crate, Go `gojsonschema`) at the schema to validate your provider's output.

## Files

- [`full-stdio-session.ndjson`](./full-stdio-session.ndjson) — a complete
  stdio session: one compact JSON object per line, in wire order. This is what
  actually travels over the pipe — diff your provider's output against it.
- [`reference-messages.json`](./reference-messages.json) — the same shapes,
  pretty-printed, with one example of each message type (including an `egress`
  provider variant).

## Framing

Every message is one Context Graph Protocol **envelope** — an internally-tagged enum
(`#[serde(tag = "type", rename_all = "snake_case")]`). The `type` field selects
the variant and sits at the same level as the payload fields:

| `type`          | direction        | payload                                        |
| --------------- | ---------------- | ---------------------------------------------- |
| `handshake`     | host → provider  | `protocol_version`                             |
| `handshake_ack` | provider → host  | `protocol_version`, `provider`, `capabilities` |
| `query`         | host → provider  | `query` (a `ContextQuery`)                     |
| `frames`        | provider → host  | `result` (a `ContextQueryResult`)              |
| `shutdown`      | host → provider  | *(none)*                                       |
| `error`         | provider → host  | `message`                                      |

Over **stdio**, each envelope is one line of compact JSON (NDJSON) on the
provider's stdin/stdout. Over **streamable HTTP**, each exchange is one POST
whose body is one envelope, with one envelope returned as the response.

Optional (`Option<T>`) fields may be omitted or sent as `null` — both are
valid on the wire, and a conforming implementation should accept either. The
examples below show both: `embeddings_fingerprint` is sent as an explicit
`null`, and other absent optional fields are simply omitted.

## A complete stdio session (annotated)

The steps below are `full-stdio-session.ndjson`, one step per line, in wire
order. Labels show direction; they are not part of the wire data. Each quoted
block **is** that line of the transcript, re-spaced compactly — every member,
including the correlation `id`s — and `python3 schema/validate-examples.py`
fails if a block and its line ever disagree, or if the transcript gains a line
this walkthrough does not show.

**1. host → provider — `handshake`.** The host names the protocol version it
speaks.

```json
{"type":"handshake","protocol_version":"contextgraph/1.0"}
```

**2. provider → host — `handshake_ack`.** The provider replies with its own
version, its identity, and its capabilities. This provider reads workspace
content locally and has **no egress**, so a host may auto-enable it. It
declares `correlation` (it echoes a request's `id`, so a host may pipeline
requests) and `verify` (it answers step 5).

```json
{"type":"handshake_ack","protocol_version":"contextgraph/1.0","provider":{"name":"repo-graph","version":"0.2.0","data_flow":{"reads":true,"writes":false,"egress":false}},"capabilities":{"query":{"kinds":["doc","symbol"]},"correlation":true,"graph":true,"embeddings_fingerprint":null,"verify":true}}
```

**3. host → provider — `query`.** A retrieval request carrying a hard token
budget (`max_tokens`) and a correlation `id`.

```json
{"type":"query","query":{"goal":"how do I configure the retry policy?","query_text":"retry policy configuration","kinds":["doc","symbol"],"anchors":["src/config.rs"],"max_frames":5,"max_tokens":1024},"id":"q1"}
```

**4. provider → host — `frames`.** The answer echoes the query's `id`, which is
how a host matches it to its request (`SPEC.md` §H4). Its frames' `token_cost`
values sum to no more than the query's `max_tokens`; each frame carries a
non-empty `title` and `citation_label`, a score in `[0,1]`, a `content_digest`
naming its exact bytes, and a `file` provenance chain.

```json
{"type":"frames","result":{"frames":[{"id":"repo-graph:retry-doc","kind":"doc","title":"Retry policy","content":"Retry behavior is set in Config::retry. max_attempts bounds the tries; backoff_ms is the initial delay, doubled each attempt.","content_digest":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","uri":"file:///repo/docs/retry.md","score":0.92,"token_cost":32,"provenance":[{"type":"file","uri":"file:///repo/docs/retry.md","range":"L1-20","digest":"sha256:0101010101010101010101010101010101010101010101010101010101010101","method":"file-read","by":"repo-graph"}],"citation_label":"retry.md L1-20","relations":[],"recorded_at":"2026-07-20T18:00:00Z"},{"id":"repo-graph:retry-sym","kind":"symbol","title":"Config::retry","content":"pub struct RetryPolicy { pub max_attempts: u32, pub backoff_ms: u64 }","content_digest":"sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","uri":"file:///repo/src/config.rs","score":0.81,"token_cost":18,"provenance":[{"type":"file","uri":"file:///repo/src/config.rs","range":"L42-44","digest":"sha256:0808080808080808080808080808080808080808080808080808080808080808","method":"tree-sitter-symbol-extraction","by":"repo-graph"}],"citation_label":"config.rs L42-44","relations":[],"recorded_at":"2026-07-20T18:00:00Z"}],"truncated":false},"id":"q1"}
```

**5. host → provider — `verify`.** Turns later, the host asks whether the
frames it still holds are current. It sends identities only —
`(provider_id, frame_id, content_digest)` — never a frame body. `verify` carries
no `id`: correlation belongs to `query`, `frames` and `error` alone.

```json
{"type":"verify","request":{"frames":[{"provider_id":"repo-graph","frame_id":"repo-graph:retry-doc","content_digest":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},{"provider_id":"repo-graph","frame_id":"repo-graph:retry-sym","content_digest":"sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}]}}
```

**6. provider → host — `verified`.** One verdict per identity. `retry-doc` is
`valid`, so the host may keep reusing it; `retry-sym` is `stale`, and the
`replacement_digest` says what its bytes are now.

```json
{"type":"verified","response":{"verdicts":[{"frame":{"provider_id":"repo-graph","frame_id":"repo-graph:retry-doc","content_digest":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},"status":"valid"},{"frame":{"provider_id":"repo-graph","frame_id":"repo-graph:retry-sym","content_digest":"sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"},"status":"stale","replacement_digest":"sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"}]}}
```

**7. host → provider — `query`.** The host re-queries for fresh content, under
a new `id`.

```json
{"type":"query","query":{"goal":"how do I configure the retry policy?","query_text":"retry policy configuration","kinds":["doc","symbol"],"anchors":["src/config.rs"],"max_frames":5,"max_tokens":1024},"id":"q2"}
```

**8. provider → host — `frames`.** This answer is **attested**: beside the
frames it carries `frame_attestations` and a `result_attestation`, described in
[Attesting an answer](#attesting-an-answer) below.

```json
{"type":"frames","result":{"frames":[{"id":"repo-graph:retry-doc","kind":"doc","title":"Retry policy","content":"Retry behavior is set in Config::retry. max_attempts bounds the tries; backoff_ms is the initial delay, doubled each attempt.","content_digest":"sha256:9f2c3e7a1b","uri":"file:///repo/docs/retry.md","score":0.92,"token_cost":32,"provenance":[{"type":"file","uri":"file:///repo/docs/retry.md","range":"L1-20","digest":"sha256:0101010101010101010101010101010101010101010101010101010101010101","method":"file-read","by":"repo-graph"}],"citation_label":"retry.md L1-20","relations":[],"recorded_at":"2026-07-20T18:00:00Z"},{"id":"repo-graph:retry-sym","kind":"symbol","title":"Config::retry","content":"pub struct RetryPolicy { pub max_attempts: u32, pub backoff_ms: u64 }","content_digest":"sha256:1a2b3c4d5e","uri":"file:///repo/src/config.rs","score":0.81,"token_cost":18,"provenance":[{"type":"file","uri":"file:///repo/src/config.rs","range":"L42-44","digest":"sha256:0808080808080808080808080808080808080808080808080808080808080808","method":"tree-sitter-symbol-extraction","by":"repo-graph"}],"citation_label":"config.rs L42-44","relations":[],"recorded_at":"2026-07-20T18:00:00Z"}],"truncated":false,"frame_attestations":[{"frame":{"provider_id":"repo-graph","frame_id":"repo-graph:retry-doc","content_digest":"sha256:9f2c3e7a1b"},"attestation":{"signed_commitment":"sha256:7951637413208edb0ade6e2a26d3a4cae01cc7b89ff5274529551a417b7f442d","key_id":"repo-graph-2026-08","algorithm":"ed25519","attester_id":"repo-graph","signature":"7794b4b9438757ff7d2eedfcc2b39a72f5042bb932e81599a05daa52a6ac3b698f4fb524881f32550e45eb885a0ba434711bcb4a057f6a744f577e6a75dacc05","issued_at":"2026-08-29T12:00:00Z"},"inclusion_proof":{"leaf_index":0,"leaf_count":2,"path":[{"sibling":"sha256:516b096c17a42fdf136c075707c125a3afbebda2511e1d53c7c7e65c088e7233","sibling_is_left":false}]}},{"frame":{"provider_id":"repo-graph","frame_id":"repo-graph:retry-sym","content_digest":"sha256:1a2b3c4d5e"},"inclusion_proof":{"leaf_index":1,"leaf_count":2,"path":[{"sibling":"sha256:274026c23d58d52a61f965ced41577bfe53199c75fb1f68f3ad91abc4ac0c4f3","sibling_is_left":true}]}}],"result_attestation":{"signed_commitment":"sha256:a6eb9a20c7ef7b4fb633d073c3ca917253fa925a663a0176915c0643de7e75ed","key_id":"repo-graph-2026-08","algorithm":"ed25519","attester_id":"repo-graph","signature":"1afe9bca1d05835f06e00e59e6623ad61cfb53d5a77ee93576d7edadd355c87f72620030888e1cd5cedbb7503c5cd09fb633a0c27cac63e49e18c68e88d5f90f","issued_at":"2026-08-29T12:00:00Z"}},"id":"q2"}
```

**9. host → provider — `shutdown`.** The host is done; a well-behaved provider
exits cleanly (stdio) or simply expects no further requests (HTTP).

```json
{"type":"shutdown"}
```

## Attesting an answer

A provider that signs what it serves attaches the evidence to the **result**,
beside the frames and never inside one
([`SPEC.md` §6.5.5](../SPEC.md), F11–F13). Both members are optional and are
omitted when absent, so an unsigned answer is byte-identical to one from a
provider written before attestation existed.

- `frame_attestations` — one entry per attested frame. Each names the
  `(provider_id, frame_id, content_digest)` identity it covers **in full**,
  because that triple is what the signature binds to and because array position
  is not identity. An entry carries a per-frame `attestation`, an
  `inclusion_proof` tying the frame to the signed result-set root, or both.
- `result_attestation` — one signature over the RFC 6962 Merkle root of every
  frame in the answer, in canonical `FrameId` order.

The attested exchange in both transcripts (`id: "q3"` in
`reference-messages.json`, `id: "q2"` in the NDJSON) shows both entry shapes:
`repo-graph:retry-doc` carries its own signature *and* a proof, while
`repo-graph:retry-sym` is attested only through the root — the cheapest honest
shape, one signature instead of one per frame.

**The example key is published so you can check the signatures.** They are
Ed25519 over a signing key derived from a seed of thirty-two `0x2a` bytes; the
matching public key is
`197f6b23e16c8532c6abc838facd5ea789be0c76b2920334039bfa8b3d368d61`. An example
signature nobody can verify demonstrates nothing. This key signs the fixtures in
this repository and must never sign anything else.
`contextgraph-conformance/tests/attestation_wire.rs` recomputes every commitment
and root in these files and verifies every signature against it, so a forged or
stale example is a red build rather than something an implementer copies.

## The `egress` variant

If a provider sends data off the local machine — a cloud documentation search,
a remote embedding API — it declares `egress: true`:

```json
{"type":"handshake_ack","protocol_version":"contextgraph/1.0","provider":{"name":"cloud-docs","version":"1.4.0","data_flow":{"reads":true,"writes":false,"egress":true}},"capabilities":{"query":{"kinds":["doc"]},"correlation":true,"graph":false,"embeddings_fingerprint":null}}
```

A conforming host **does not auto-enable** this provider. It gates the provider
behind explicit, named, one-time consent and **never transmits the query
payload before consent is recorded**. The host's HTTP transport treats *every*
remote provider as `egress` regardless of this claim, so a remote cannot lie
its way past the gate. This is the single most security-relevant shape in Context Graph Protocol;
see [protocol-surface.md § Conformance requirements](../docs/protocol-surface.md#conformance-requirements).

## Reporting an error without dying

A bad `query` should be answered with `error`, not a crash. A provider that
exits on a bad request fails the `malformed-input-tolerance` conformance check.

```json
{"type":"error","message":"unsupported frame kind: 'image'"}
```

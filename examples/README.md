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
examples below omit `null` optional fields to show the minimal valid form.

## A complete stdio session (annotated)

`full-stdio-session.ndjson` contains exactly these five lines, in this order.
Labels show direction; they are **not** part of the wire data.

**1. host → provider — `handshake`.** The host names the protocol version it
speaks.

```json
{"type":"handshake","protocol_version":"contextgraph/1.0-draft"}
```

**2. provider → host — `handshake_ack`.** The provider replies with its own
version, its identity, and its capabilities. This provider reads workspace
content locally and has **no egress**, so a host may auto-enable it.

```json
{"type":"handshake_ack","protocol_version":"contextgraph/1.0-draft","provider":{"name":"repo-graph","version":"0.2.0","data_flow":{"reads":true,"writes":false,"egress":false}},"capabilities":{"query":{"kinds":["doc","symbol"]},"correlation":true,"graph":true,"embeddings_fingerprint":null,"verify":true}}
```

**3. host → provider — `query`.** A retrieval request carrying a hard token
budget (`max_tokens`).

```json
{"type":"query","query":{"goal":"how do I configure the retry policy?","query_text":"retry policy configuration","kinds":["doc","symbol"],"anchors":["src/config.rs"],"max_frames":5,"max_tokens":1024}}
```

**4. provider → host — `frames`.** The answer: two frames whose `token_cost`
sums to 64, within the 1024-token budget, each with a non-empty `title` and
`citation_label`, scores in `[0,1]`, and a `file` provenance chain.

```json
{"type":"frames","result":{"frames":[{"id":"repo-graph:retry-doc","kind":"doc","title":"Retry policy","content":"Retry behavior is set in Config::retry. max_attempts bounds the tries; backoff_ms is the initial delay, doubled each attempt.","uri":"file:///repo/docs/retry.md","score":0.92,"token_cost":41,"provenance":[{"type":"file","uri":"file:///repo/docs/retry.md","range":"L1-20","digest":"sha256:9f2c3e7a","method":"file-read","by":"repo-graph"}],"citation_label":"retry.md L1-20"},{"id":"repo-graph:retry-sym","kind":"symbol","title":"Config::retry","content":"pub struct RetryPolicy { pub max_attempts: u32, pub backoff_ms: u64 }","uri":"file:///repo/src/config.rs","score":0.81,"token_cost":23,"provenance":[{"type":"file","uri":"file:///repo/src/config.rs","range":"L42-44","digest":"sha256:1a2b3c4d","method":"tree-sitter-symbol-extraction","by":"repo-graph"}],"citation_label":"config.rs L42-44"}],"truncated":false}}
```

**5. host → provider — `shutdown`.** The host is done; a well-behaved provider
exits cleanly (stdio) or simply expects no further requests (HTTP).

```json
{"type":"shutdown"}
```

## The `egress` variant

If a provider sends data off the local machine — a cloud documentation search,
a remote embedding API — it declares `egress: true`:

```json
{"type":"handshake_ack","protocol_version":"contextgraph/1.0-draft","provider":{"name":"cloud-docs","version":"1.4.0","data_flow":{"reads":true,"writes":false,"egress":true}},"capabilities":{"query":{"kinds":["doc"]},"correlation":true,"graph":false,"embeddings_fingerprint":null}}
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

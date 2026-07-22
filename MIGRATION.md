# Migration guide

For downstreams pinning this repository before it had tags, a published name,
or a normative spec.

---

## ⚠️ The redirect hazard — read this first

Downstreams currently pin the **pre-rename** URL:

```toml
ocp-types = { git = "https://github.com/macanderson/opencontextprotocol", rev = "7912257c…" }
```

That resolves today **only because GitHub redirects a renamed repository**. The
redirect is not a guarantee:

> If anyone later creates a new repository named `opencontextprotocol` under the
> `macanderson` account, GitHub stops redirecting and the old URL resolves to
> **that** repository instead. Every pin above would then silently fetch code
> from a different project — with no error, no warning, and a `Cargo.lock` that
> still looks plausible.

This is a supply-chain footgun, not a cosmetic issue. **Repoint every pin to the
canonical URL**, whether or not you also take the version bump:

```
https://github.com/macanderson/context-graph-protocol
```

---

## 1. Repository and crate renames

| Before | After |
| --- | --- |
| `github.com/macanderson/opencontextprotocol` | `github.com/macanderson/context-graph-protocol` |
| `ocp-types` | `contextgraph-types` |
| `ocp-host` | `contextgraph-host` |
| `ocp-conformance` | `contextgraph-conformance` |
| `ocp/1.0-draft` (protocol version) | `contextgraph/1.0-draft` |

Rust paths follow the crate names: `use ocp_types::…` → `use contextgraph_types::…`.

The rename landed in commit `d6768a8`; the crates were originally imported in
`7912257`.

## 2. Pin by tag, not by SHA

Until the crates are on crates.io (issue #16), pin the tag:

```toml
[dependencies]
contextgraph-types = { git = "https://github.com/macanderson/context-graph-protocol", tag = "v0.0.2" }
contextgraph-host  = { git = "https://github.com/macanderson/context-graph-protocol", tag = "v0.0.2" }
```

A tag is stable, greppable, and shows up in `cargo tree`; a bare SHA tells the
next reader nothing about how far behind they are.

### Cutting the tag

*(Pending maintainer authorization — the commands, not an instruction to run
them unattended.)*

```bash
git checkout main && git pull
git tag -a v0.0.2 -m "Pre-0.1.0 checkpoint: normative SPEC.md, canonical token accounting, CI"
git push origin v0.0.2
```

## 3. Breaking changes since `7912257`

### 3.1 Removed capability fields

Removed by [ADR 0004](./docs/adr/0004-dead-capability-surface.md) because each
was negotiable at handshake but unreachable — no wire method, no host API, no
conformance check:

| Removed | Replacement |
| --- | --- |
| `Capabilities.upsert` | none — see `docs/sketches/write-path.md` |
| `Capabilities.subscribe` | pull-based revalidation; see `docs/sketches/push-invalidation.md` |
| `QueryCapability.filters` | none — see `docs/sketches/query-filters.md` |

**The wire is unaffected.** These fields carried `#[serde(default)]`, and
deserialization ignores unknown fields, so a provider still emitting them
handshakes successfully. The break is at the Rust API only.

**Fix:** delete the field from your struct literals. In `stella` that is three
`filters: Vec::new(),` lines in `stella-cli/src/ocp.rs`.

`DataFlow.writes` is **kept** (with a corrected definition) — it is a
consent-surface declaration, not a capability flag.

### 3.2 Added capability field

`Capabilities.correlation: bool` — declares that the provider echoes a request's
`id`. Defaults to `false`, so existing struct literals using
`..Default::default()` compile unchanged and such providers are queried in
lock-step.

### 3.3 Envelope changes

`Envelope::Query`, `Frames`, and `Error` gained an optional `id`;
`Envelope::Error` gained an optional `code`. Pattern matches that destructured
these variants exhaustively need `..`:

```rust
// before
Envelope::Error { message } => …
// after
Envelope::Error { message, .. } => …
```

### 3.4 `token_cost` is now a defined quantity — **behavioural break**

`ContextFrame.token_cost` **MUST** now equal
`ceil(utf8_byte_length(content) / 4)` (`SPEC.md` §B3, and
[ADR 0003](./docs/adr/0003-canonical-token-accounting.md)).

This is the change most likely to turn a previously-green provider red, and
that is deliberate: the old check verified that declared costs summed within
budget, which a provider declaring `token_cost: 1` on a ten-thousand-token frame
satisfied perfectly.

**Fix:** call `contextgraph_types::budget_tokens(&content)` when building a
frame. Do not hand-roll the count.

Hosts: a budget token is an **accounting unit, not a model token**. It
under-estimates source code and CJK text. Convert your real model budget with
`budget_from_model_tokens(model_tokens, SUGGESTED_HOST_SAFETY_FACTOR)`.

### 3.5 Temporal fields are validated

`valid_from` / `valid_to` / `recorded_at` / `as_of` **MUST** match
`YYYY-MM-DDTHH:MM:SS(.f+)?Z` — a UTC-only subset of RFC 3339 (`SPEC.md` §F4).
Offsets like `+02:00` and lowercase `t`/`z` are **not** conformant.

### 3.6 File provenance digests are validated

Provenance of kind `file` **MUST** carry `sha256:<64 lowercase hex>`
(`SPEC.md` §F5). The `sha256:abc` placeholder used in early fixtures no longer
passes.

## 4. Where the spec lives now

Code comments used to cite `docs/specs/stella-rust-cli/06-context-protocol.md`,
which lives in a private repository — unresolvable for anyone outside it.

All normative text now lives in [`SPEC.md`](./SPEC.md) in this repository, with
stable anchors (`H1`, `B3`, `F5`, …) that will not be renumbered within the
`contextgraph/1` family. Cite those.

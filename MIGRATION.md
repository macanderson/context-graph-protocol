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

## 5. Crate `1.x` → `2.0.0` — a Rust break, not a wire break

**Nothing changes on the wire.** `PROTOCOL_VERSION` is still `contextgraph/1.0`,
the bytes are unchanged, and a `1.x` peer and a `2.x` peer interoperate in both
directions. The major is spent entirely on the Rust API, because
[ADR 0011](./docs/adr/0011-open-frame-kind-vocabulary.md) opens the `FrameKind`
vocabulary and there is no way to do that compatibly in an `enum`. See
[docs/stability.md](./docs/stability.md) for why the two axes are allowed to
disagree.

Take the bump with:

```toml
contextgraph-types = "2"
contextgraph-host  = "2"
```

Then fix the call-site shapes below. Each is a compile error, so the compiler
enumerates the work for you — none of this fails silently at runtime. The first
three affect everyone; §5.4 and §5.5 affect only a host or provider that touched
the attestation types.

### 5.1 `match` on `FrameKind` needs a wildcard arm

`FrameKind` gained an `Unknown(String)` variant and is now `#[non_exhaustive]`,
so an exhaustive match no longer compiles:

```rust
match frame.kind {
    FrameKind::Snippet => …,
    // …the other six…
    FrameKind::Unknown(ref kind) => render_opaque(kind),  // if you can use it
    _ => render_opaque(frame.kind.as_str()),              // otherwise
}
```

The wildcard is not boilerplate you are being made to write: per SPEC.md §13 U2
a receiver **MUST NOT** reject a frame for carrying a kind it does not know. The
arm is where that obligation now lives, and the type system is what stops you
from forgetting it when `contextgraph/1.1` names an eighth kind.

### 5.2 `FrameKind` is no longer `Copy`

An unknown kind owns its wire string, and a `String` cannot be `Copy`. Where you
relied on the implicit copy, borrow — or `.clone()` when you need an owned value:

```rust
let kind = frame.kind.clone();     // was: let kind = frame.kind;
if matches!(&frame.kind, FrameKind::Doc) { … }
```

Prefer `frame.kind.as_str()` where you only wanted the name; it borrows and
allocates nothing.

### 5.3 `frame_kind_name` takes a reference

```rust
contextgraph_host::frame_kind_name(&kind)   // was: frame_kind_name(kind)
```

It used to return `&'static str`, which duplicated the vocabulary in a second
place and could not name a kind the host did not know. The returned lifetime is
now tied to the kind, because an unknown kind owns its string.

### 5.4 The attestation seam has one shape, and one home

Only affects a host or provider written against the attestation work that landed
after `1.x`. If you have never touched a `FrameAttestation`, skip to §5.5.

`contextgraph-host` briefly defined two of its own `FrameAttestation` types
beside the canonical one in `contextgraph-types`, and the `frames` envelope
carried an `attestations` member while the query result already carried
`frame_attestations`. One signed answer had two encodings and no rule for which
won ([ADR 0019](./docs/adr/0019-one-home-for-an-attestation.md), #161). Four
compile errors follow:

```rust
// The type: one definition, re-exported from contextgraph-host for convenience.
use contextgraph_types::FrameAttestation;          // was: contextgraph_host::wire / ::trust
FrameAttestation::signed(frame.identity(provider_id), attestation)  // was: ::new(frame_id, …)

// The envelope: no attestations member. Put the evidence on the result.
Envelope::Frames { id, result }                    // was: { id, result, attestations }

// A signing provider populates the result `query` already returns.
// `ContextProvider::query_attested` and `AttestedQueryResult` are gone.
async fn query(&self, q: &ContextQuery) -> Result<ContextQueryResult, HostError> {
    Ok(ContextQueryResult {
        frame_attestations: self.sign(&frames),
        ..ContextQueryResult::unattested(frames, false, None)
    })
}

// The store reads the evidence off the result, so a mismatched pair is
// no longer expressible.
trust.check_result(provider_id, &result)           // was: (provider_id, &result, &attestations)
```

`Host::query_provider_attested` now returns `(ContextQueryResult, Vec<FrameAttestationOutcome>)`
rather than an `AttestedQueryResult` in the first slot, so `.0.result` becomes
`.0`.

`AttestationState` gained `UnusableEvidence` for an entry that named a frame and
could not be turned into a check. If you match that enum exhaustively, add an
arm; F9 means it is treated as unattested for every decision.

Nothing here changes the wire for a provider that was already putting its
evidence on the result, which is where `SPEC.md` §6.5.5 has always put it.

### 5.5 A caller that builds `ContextQueryResult` by hand

Adding `frame_attestations` and `result_attestation` broke every three-field
struct literal. Two one-line fixes, either is fine:

```rust
ContextQueryResult::unattested(frames, truncated, dropped_estimate)
// or
ContextQueryResult { frames, truncated, dropped_estimate, ..Default::default() }
```

### 5.6 SDKs move in lockstep

`contextgraph-sdk` (Python) and `@contextgraphprotocol/typescript-sdk`
(TypeScript) also go to `2.0.0`, for the same reason in their own type systems:
`FrameKind` widens to accept any string, so an exhaustive `switch` that relied
on `never`-narrowing stops type-checking. Narrow with the exported
`isKnownFrameKind` / `KNOWN_FRAME_KINDS` when you need to branch only on kinds
you understand. The Go SDK is unchanged in this release — porting it is tracked
in issue #93. It takes a break of its own later; see §7.

## 6. The JSON Schemas moved to a branded `$id` — no action required

**Nothing you have to do.** No bytes changed, no URL stopped working, and no
`$ref` resolves differently. Read this only so the new URL is not a surprise.

Both schemas' `$id` now names the protocol's own domain, versioned by major
family:

| | `$id` |
| --- | --- |
| was | `https://raw.githubusercontent.com/macanderson/context-graph-protocol/main/schema/<name>` |
| now | `https://contextgraphprotocol.org/schema/v1/<name>` |

`v1` is the `contextgraph/1` wire family, not the crate version — the crates are
already on `2.x` against that same wire ([docs/stability.md](./docs/stability.md)).
The old URL pinned `main`, a git branch, so an additive `1.x` minor silently
changed what a resolver holding it saw. A family is bounded: within
`contextgraph/1` changes are additive-only, so an older cached copy stays valid,
merely less complete. [ADR 0013](./docs/adr/0013-schema-identity-on-a-branded-versioned-url.md)
carries the reasoning.

**If you fetch the schema by URL,** the `raw.githubusercontent.com` URL still
returns 200 and the same bytes, and will keep doing so — it is in the wild, and
a schema URL that 404s is worse than a stale one. Move to the branded URL when
convenient, not urgently.

**If you pin a local copy,** it stays valid. Every `$ref` in both schemas is a
same-document pointer (`#/$defs/…`) and neither schema references the other, so
`$id` has no bearing on resolution — the schemas validate fully offline, exactly
as before.

**If you compare `$id` as a string,** that is the one thing that changed. A test
asserting the old literal needs the new one. `contextgraph-conformance` does not
do this, and neither does any SDK in this repository.

## 7. Go SDK `v0.1.0` → `v0.2.0` — optional strings become `*string`

**Nothing changes on the wire, and this is a compile error rather than a silent
one — with one exception, in §7.3.** The Go SDK's optional string fields that a
peer can observe as *absent* or as *present and empty* are now `*string`:

| Type | Fields |
| --- | --- |
| `contextgraph.Provenance` | `URI`, `Range`, `Digest`, `Method`, `By` |
| `contextgraph.ContextFrame` | `Content`, `ContentDigest` |

Every other optional string in the package is unchanged.

### 7.1 Why the break was worth taking

`SPEC.md` §6.5.1 makes the presence byte normative: `enc_opt(None)` is `0x00`
and `enc_opt(Some(s))` is `0x01 ‖ enc_str(s)`, so `"uri": null` and `"uri": ""`
**must** hash differently — without that, a link's URI could be deleted from a
signed chain without disturbing the hash. Go's `encoding/json` decodes an absent
member and an explicit `""` into the same `string`, and `omitempty` drops an
empty one on the way out, so the old wire struct could represent only one of the
two states.

That was survivable for a Go provider building its own links and fatal for a Go
*verifier*. Handed an honest frame from a Rust, TypeScript or Python signer that
carried `"uri": ""` — perfectly representable in all three — a Go verifier
computed the chain head for an *absent* URI, and answered `commitment_mismatch`
on evidence that was in fact intact (issue #124). `ContextFrame.ContentDigest`
had the same shape and the same consequence, being an `enc_opt` field of the
§6.5.2 frame commitment.

The alternative was a decode-side wrapper type that preserved presence without
changing the exported struct. Cheaper, and it would have left two ways to spell
a provenance link in one SDK — the shape that produced this bug in the first
place. One representation, correct by construction, was worth a major-shaped
break in a `v0` module.

`ContextFrame.Content` moves for a related reason: `omitempty` on a `string`
silently omitted the member for a `full` frame carrying an empty document,
producing a frame the schema rejects (its `full` branch requires `content`), and
a `reference` frame is defined by omitting `content` entirely rather than by
sending `""`.

### 7.2 Fixing your call sites

Build a present value with the new `contextgraph.Ptr` helper — generic, so it
also builds the `*uint32` that `CanonicalTokenCost` takes:

```go
// before
frame := cg.ContextFrame{
    Content:       content,
    ContentDigest: digest,
    Provenance: []cg.Provenance{{Type: "file", URI: uri, Range: rng, Digest: digest}},
}

// after
frame := cg.ContextFrame{
    Content:       cg.Ptr(content),
    ContentDigest: cg.Ptr(digest),
    Provenance: []cg.Provenance{{Type: "file", URI: cg.Ptr(uri), Range: cg.Ptr(rng), Digest: cg.Ptr(digest)}},
}
```

Reading a field now means checking presence, which is the point:

```go
if frame.ContentDigest == nil {
    // Not verifiable — re-query rather than reuse (docs/context-reuse.md §4).
}
```

Verifying somebody else's frame no longer needs a hand-rolled conversion:

```go
var frame cg.ContextFrame
json.Unmarshal(body, &frame)
result := attest.VerifyFrameAttestation(providerID,
    attest.FrameFromContextFrame(frame), attestation, publicKey)
```

`attest.LinkFromProvenance` no longer collapses an empty string to absent. That
collapse was documented rather than hidden, and `TestLinkFromProvenanceStatesItsCollapse`
pinned it; both are gone, replaced by a vector that pins the faithful behaviour
against `tests/vectors/attestation-vectors.json`.

### 7.3 The one thing that does not fail loudly

If your code used `""` to *mean* absent, wrapping it blindly changes your output
bytes: `cg.Ptr("")` now emits `"digest": ""` where the old struct omitted the
member. Do not blanket-wrap. Pass `nil` where you meant absent:

```go
uri := cg.Ptr(candidate)
if candidate == "" {
    uri = nil          // absent, not present-and-empty
}
```

A provider that never set an empty optional emits byte-identical JSON before and
after this change.

### 7.4 What the missing `/vN` suffix means for you

The module path is `github.com/macanderson/context-graph-protocol/sdk/go`, with
**no `/vN` suffix**. Go's import-compatibility rule only requires a suffix at
`v2` and above, and this module is on `v0` — where semantic versioning makes no
compatibility promise at all and the Go toolchain permits a breaking minor.

Said plainly:

- The break ships as tag `sdk/go/v0.2.0` **at the same import path**. Not one
  import line in your code changes.
- Because the path is unchanged, `go get -u` will move a `v0.1.0` consumer onto
  it. Your build then fails to compile, which is the loud outcome you want —
  see §7.3 for the one case that is quieter.
- There is no `/v2` escape hatch here. A `v2+` module path lets an old and a new
  major coexist in one build; a `v0` module has no such path, so `v0.1.0` and
  `v0.2.0` of this SDK cannot both be linked into the same binary.
- To stay on the old behaviour, pin it and do not `-u`:

  ```
  require github.com/macanderson/context-graph-protocol/sdk/go v0.1.0
  ```

  Understand what you are pinning: `v0.1.0` is the version that reports
  `commitment_mismatch` on honest frames carrying an empty optional. It is a
  place to pause, not a place to stay.

The SDK stays at `v0.x` deliberately. Its wire behaviour is pinned by the
cross-language vectors and the conformance suite, not by its module version, and
`v0` is the honest label for a surface still being reconciled port by port
against the Rust reference.

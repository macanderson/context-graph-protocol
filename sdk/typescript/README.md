# @contextgraphprotocol/typescript-sdk — TypeScript

A zero-dependency TypeScript SDK for building **conformant** Context Graph
Protocol providers. Implement one interface, hand it to the runtime, and you
have a provider that speaks the line-oriented JSON wire over stdio and passes
the same conformance suite that judges the Rust reference provider.

> This is the second independent implementation (after the Rust reference), and
> it passes the full conformance suite — the concrete evidence behind the
> protocol's "enforced by contract, not convention" claim. See
> [`sdk/README.md`](../README.md) for the multi-language picture.

## Install

```sh
npm install @contextgraphprotocol/typescript-sdk
```

## Write a provider

```ts
import { runStdioProvider, budgetTokens, type Provider } from "@contextgraphprotocol/typescript-sdk";

const provider: Provider = {
  info: () => ({
    name: "my-docs-provider",
    version: "0.1.0",
    // Nothing leaves the machine ⇒ declare the honest local-only egress scope.
    data_flow: { reads: true, writes: false, egress: false, egress_scopes: ["local-only"] },
  }),
  capabilities: () => ({ query: { kinds: ["doc"] }, correlation: true, verify: true }),
  query: () => ({
    frames: [
      {
        id: "doc:1",
        kind: "doc",
        title: "Getting started",
        content: "Install the binding, then implement the required methods.",
        content_digest: `sha256:${"11".repeat(32)}`,
        score: 0.9,
        // token_cost MUST equal ceil(utf8_len(content)/4) — the SDK computes it.
        token_cost: budgetTokens("Install the binding, then implement the required methods."),
        valid_from: "2026-01-01T00:00:00Z",
        provenance: [{ type: "file", uri: "file:///docs/start.md", range: "L1-10", digest: `sha256:${"11".repeat(32)}` }],
        citation_label: "start.md L1-10",
        relations: [],
      },
    ],
    truncated: false,
  }),
};

runStdioProvider(provider);
```

The runtime handles the whole lifecycle a host drives — handshake, query
(echoing the correlation `id`), verify, shutdown — and stays alive with a typed
error on a malformed line rather than crashing.

## What it gives you

- **Wire types** (`ContextFrame`, `ContextQuery`, `Capabilities`, `Envelope`, …)
  mirrored from the JSON Schema — the language-neutral source of truth.
- **`runStdioProvider(provider)`** — the stdio lifecycle loop.
- **`createHttpHandler(provider)`** — the same lifecycle behind one HTTP POST
  endpoint (see below).
- **`budgetTokens(content)`** — the canonical B3 cost, `ceil(utf8_len/4)`.
- **Provenance attestation** (`SPEC.md` §6.5) — verify and sign frames and
  result sets.
- **Record hashing and attestation** (the lifecycle profile) — `recordHash`
  over RFC 8785, and `RecordAttestation` verify and sign.
- Runnable **example providers** — `examples/example-docs.ts` (stdio) and
  `examples/example-docs-http.ts` (HTTP) — that pass the conformance suite.

## Host it over HTTP

The same `provider` runs behind a single POST endpoint (the streamable-HTTP
transport, SPEC.md §3) — write the provider once, change only the transport:

```ts
import { createServer } from "node:http";
import { createHttpHandler } from "@contextgraphprotocol/typescript-sdk";

createServer(createHttpHandler(provider)).listen(8787);
// Express:  app.post("/contextgraph", createHttpHandler(provider))  // no JSON body-parser on that route
// Fastify:  reply with respondToEnvelopeBody(provider, request.body)
```

`handleEnvelope(provider, envelope)` is the transport-free state machine if you
want to wire it into a framework yourself. Confirm it green with
`contextgraph-inspect http http://127.0.0.1:8787` (the `malformed-input-tolerance`,
`embedding-fingerprint`, and `correlation` probes report *skipped* over HTTP —
they inspect raw framing this transport doesn't expose).

## Verify a provenance attestation

`SPEC.md` §6.5 makes a frame's provenance *evidence* rather than merely
tamper-evident: a detached Ed25519 signature over a commitment to the frame's
identity and its provenance chain. The `attest` surface implements the whole
construction — the length-prefixed link encoding, the source-first chain fold,
the frame commitment, an RFC 6962 Merkle root over a result set with inclusion
proofs, and verification.

```ts
import { frameCommitment, verifyFrameAttestation, digestString }
  from "@contextgraphprotocol/typescript-sdk";

const verdict = verifyFrameAttestation("repo-graph", frame, attestation, publicKey);
if (verdict.verdict !== "valid") {
  // Never a boolean: "the frame changed after signing" and "the key is wrong"
  // call for opposite responses, and F9 says an unverifiable attestation
  // degrades a frame to unattested rather than disqualifying it.
  console.warn(verdict);
}
```

A frame that declares no `content_digest` verifies as `valid_identity_only`,
not `valid` ([ADR 0018](../../docs/adr/0018-signing-a-frame-requires-a-content-digest.md)):
the signature binds who served it and its provenance, but not its bytes.
`isValid` is `false` for it; `signatureVerifies` is `true`.

A frame attested through a signed result-set root rather than its own signature
is checked with `verifyFrameInclusion(providerId, frame, proof, rootAttestation,
publicKey)`. It applies the same rule, and refuses a proof as
`malformed_commitment` before hashing anything when its path is longer than
`MAX_INCLUSION_PATH_STEPS` (64) or does not have exactly the length and
`sibling_is_left` sides RFC 6962 gives its `(leaf_index, leaf_count)`
(`SPEC.md` §6.5.3). `isWellShaped(proof)` runs that check alone, and
`inclusionPathSides(leafIndex, leafCount)` returns the expected sides. The
shape cannot tell every tree size apart (leaf 3 of 5 and leaf 3 of 7 share a
path), so a host checking a live answer also compares `leaf_count` with the
number of frames the answer carries.

## Sign a provenance attestation

The SDK signs as well as verifies
([ADR 0033](../../docs/adr/0033-sdks-can-sign-provenance-attestations.md)),
mirroring the Rust reference's `sign_frame_attestation`, `sign_commitment` and
`public_key_for`:

```ts
import { publicKeyFor, signFrameAttestation, signingKeyFromSeed }
  from "@contextgraphprotocol/typescript-sdk";

// Once, at startup. A KeyObject keeps the key in OpenSSL's memory rather than
// in a JavaScript Uint8Array (see "Key custody" below).
const signingKey = signingKeyFromSeed(seedFromYourSecretStore); // 32 raw bytes
const publicKey = publicKeyFor(signingKey); // hand this to hosts out of band

const attestation = signFrameAttestation(
  "repo-graph", frame, signingKey, "key-2026-09", "acme-docs", "2026-09-28T00:00:00Z",
);
```

`signCommitment(commitment, signingKey, keyId, attesterId, issuedAt)` signs any
32-byte commitment, including a `merkleRoot(...)` over a whole result set.
`signFrameAttestation` refuses a frame with no `content_digest`, or one that is
not `sha256:<64 lowercase hex>` (§D1), because ADR 0018 forbids signing a frame
whose bytes the signature would not bind.

The test suite signs the published commitment with the published seed from
`tests/vectors/attestation-vectors.json` and compares the result to the
published signature byte for byte. Ed25519 is deterministic, so that is exact
equality, not "it also verifies".

### Key custody

The protocol specifies the preimage and never the custody of the key. The
in-process signers are for a provider that has decided to hold its key in
application memory, and that decision has a cost:

- **Whatever can read the process can sign as you.** A heap snapshot, a core
  dump, a debugger, a dependency with a supply-chain compromise, or a log line
  that serialized the wrong object each hand over the key. Nothing on the wire
  tells a host that a signature came from a stolen copy.
- **A long-lived key makes every leak retroactive and ongoing.** Everything the
  key ever signed stays verifiable, and everything an attacker signs with it
  verifies too, until every host that trusts it has removed it from its trust
  store ([ADR 0016](../../docs/adr/0016-attestation-trust-roots.md)).
  Rotation issues a new `key_id`; it never reuses one. Rotate on a schedule
  short enough that you would accept that window of forgery.
- **Prefer a `KeyObject` to a raw seed.** `signingKeyFromSeed` zeroes the
  buffer it builds, and the `KeyObject` it returns keeps the key outside the
  JavaScript heap. It does not remove the key from the process.

A provider whose key lives in an HSM or a KMS never passes it to this SDK.
It computes the 32 bytes, has the backend sign them, and assembles the
attestation:

```ts
import { ALGORITHM_ED25519, digestString, frameCommitment, toHex }
  from "@contextgraphprotocol/typescript-sdk";

const commitment = frameCommitment("repo-graph", frame); // 32 bytes
const signature: Uint8Array = await kms.signEd25519(keyRef, commitment); // your backend, 64 bytes

const attestation = {
  signed_commitment: digestString(commitment),
  key_id: "key-2026-09",
  algorithm: ALGORITHM_ED25519,
  attester_id: "acme-docs",
  signature: toHex(signature),
  issued_at: "2026-09-28T00:00:00Z",
};
```

The backend must produce a pure Ed25519 signature (RFC 8032, not Ed25519ph)
over exactly those 32 bytes. Check the first one against `verifyCommitment`
before you ship it.

### What a port of the attestation encoding gets wrong

Two things a port of this encoding gets wrong, both of which the test suite
catches:

- **`String.prototype.length` is not a UTF-8 byte count.** The §6.5.1 length
  prefix is bytes; `.length` is UTF-16 code units, which differs for every
  non-Latin-1 string and by an extra one per astral-plane character. This SDK
  measures what `TextEncoder` produced.
- **Node's Ed25519 accepts a small-order public key.** §6.5.4 asks for a
  strict verifier, so `verifyCommitment` declines those keys — and any key
  whose `y` is not reduced — before OpenSSL ever sees them.

The vectors are shared across every language:

```sh
npm run build && node --test "dist/test/*.test.js"
```

They come from `tests/vectors/attestation-vectors.json`, which the Rust
reference publishes and pins.

## Hash and attest a lifecycle record

The Context Exchange Provider profile
([`docs/profiles/context-exchange-provider.md`](../../docs/profiles/context-exchange-provider.md))
content-addresses a record by `record_hash` and signs that hash with a
`RecordAttestation` ([ADR 0017](../../docs/adr/0017-record-hash-and-record-attestation.md)).

```ts
import { recordHash, signRecord, verifyRecordAttestation }
  from "@contextgraphprotocol/typescript-sdk";

record.record_hash = recordHash(record); // LH1: JCS, with record_hash itself left out
const attestation = signRecord(record, signingKey, "cep-key-2026-09", "acme", "2026-09-28T00:00:00Z");

const verdict = verifyRecordAttestation(record, attestation, publicKey);
// Recomputes the hash (LC5), so rewriting record_hash to match edited
// content is still caught as `commitment_mismatch`.
```

- **`recordHash(record)`** is `sha256:` over the RFC 8785 (JCS)
  canonicalization of the record, with its **top-level** `record_hash` member
  removed. A nested `record_hash` is content and stays in. When two
  implementations disagree, diff `recordHashPreimage(record)`.
- **`canonicalizeJson(value)`** is the RFC 8785 canonicalizer on its own. It
  refuses `NaN`, the infinities and lone surrogates with a `RecordHashError`
  instead of coercing them the way `JSON.stringify` does, and refuses a value
  that contains itself instead of overflowing the stack.
- **The signed message is `"contextgraph/attest/1/record"` followed by the
  digest's 32 raw bytes** (`recordAttestationMessage`), so a frame-layer
  signature over the same digest never verifies as a record attestation. An
  HSM or KMS signs those bytes itself, as in "Key custody" above.

The tests reproduce every `jcs_utf8` and `record_hash` in
`tests/fixtures/record-hash-vectors.json` byte for byte, verify
`record-attestation.json` under `record-attestation-key.json`, and re-sign it
with the published test seed:

```sh
npm run build && node --test "dist/test/record/*.test.js"
```

## Prove it conformant

Build, then run the reference suite against your provider:

```sh
npm run build
# from the repository root, with the Rust bins built (cargo build --workspace --bins):
./.github/scripts/conformance-external.sh -- node sdk/typescript/dist/examples/example-docs.js
```

A green run is the machine-checkable claim that your provider honors the
protocol — the same bar every implementation is held to.

## License

MIT OR Apache-2.0, matching the Context Graph Protocol crates.

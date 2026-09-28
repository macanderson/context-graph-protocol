# 33. SDKs can sign provenance attestations

- Status: accepted
- Date: 2026-09-28
- Decides #127. Builds on [ADR 0010](0010-provenance-attestation.md),
  [ADR 0016](0016-attestation-trust-roots.md),
  [ADR 0017](0017-record-hash-and-record-attestation.md) and
  [ADR 0018](0018-signing-a-frame-requires-a-content-digest.md). Changes no
  wire format and no normative requirement.

## Context

#93 ported the §6.5 constructions to the TypeScript, Python and Go SDKs. Each
of them can compute a frame commitment and a Merkle root, and each can verify
an attestation. None of them can produce one. The Rust reference can:
`sign_commitment`, `sign_frame_attestation` and `public_key_for` exist "for
providers content to hold key material in memory", beside a doc comment
telling a provider with an HSM or a KMS to sign the 32 commitment bytes itself.

The SDKs gave a provider only the second half of that choice, with no worked
example. The protocol specifies the preimage and never the custody of the key,
so stopping there was defensible. It left a real asymmetry, though. A Rust
provider attests with one call. A TypeScript, Python or Go provider has to find
an Ed25519 library, work out which bytes to sign, and assemble the struct by
hand, with nothing to check the result against. An ecosystem where the only
path is hand assembly is one where the first mistakes happen in production.

Two facts shape the answer:

- `tests/vectors/attestation-vectors.json` publishes a seed, its public key,
  and the signature it produces over a published commitment.
  `tests/fixtures/record-attestation-key.json` does the same for the record
  layer. Ed25519 is deterministic (RFC 8032), so a signer can be pinned to
  those bytes exactly rather than round-tripped against its own verifier.
- Each manifest promises zero runtime dependencies. For Python, that promise is
  why `contextgraph_sdk._ed25519` exists: a hand-written, strict, verify-only
  Ed25519. It holds no secrets and touches only public inputs, and that is what
  makes a hand-written implementation defensible.

## Decision

**Every SDK ships in-process signing that mirrors the Rust reference. It is
pinned to the published vectors, it sits beside a documented HSM or KMS path,
and it never widens the zero-dependency promise.**

### 1. The surface mirrors Rust, spelled in each language's convention

| Rust reference | TypeScript | Python | Go |
|---|---|---|---|
| `sign_commitment` | `signCommitment` | `sign_commitment` | `SignCommitment` |
| `sign_frame_attestation` | `signFrameAttestation` | `sign_frame_attestation` | `SignFrameAttestation` |
| `public_key_for` | `publicKeyFor` | `public_key_for` | `PublicKeyFor` |

Each entry point takes the raw 32-byte Ed25519 seed. That is the form the Rust
reference and both published vectors use, so the witness test below is the
same test in every language. An SDK may also accept the language's own private
key type, where one exists, so a key loaded once need not live as a byte array.
TypeScript accepts a private Ed25519 `KeyObject` and exports
`signingKeyFromSeed`.

An SDK that ports the record layer (#119) signs records under the same rules:
`sign_record_attestation` and `sign_record`, over the domain-separated message
of ADR 0017 §3. The TypeScript SDK ships both.

### 2. A frame signer refuses a frame with no `content_digest`

ADR 0018 already says an attester **MUST** populate `content_digest` on any
frame it signs. A signing entry point is the one place that rule can be
enforced before a signature exists, so a frame signer refuses such a frame with
an error instead of signing it. `sign_commitment` stays unconditional: it signs
32 bytes, and it is how a caller re-signs an old identity-only commitment on
purpose.

### 3. Each language uses the platform's Ed25519, never a hand-written signer

- **TypeScript**: `node:crypto`. A raw seed becomes a private `KeyObject`
  behind the 16-byte DER PKCS#8 prefix (RFC 8410), the signing-side twin of the
  SPKI prefix the verifier already uses. The DER buffer is zeroed once the
  `KeyObject` exists. No dependency is added.
- **Go**: `crypto/ed25519`, `NewKeyFromSeed` and `Sign`. Standard library. No
  dependency is added.
- **Python**: the signing path takes an **optional** dependency on
  `cryptography`, declared as an extra (`contextgraph-sdk[signing]`), and
  imports it only when a signing function is called. The core install stays
  zero-dependency, and the verifier stays the in-package `_ed25519` module.
  Without the extra, a signing call raises a named error that says which extra
  to install. It never falls back to another implementation, and verification
  never depends on what is installed.

Python gets an optional dependency rather than a hand-written signer because
signing handles a secret and verifying does not. A signer written with Python
integers does scalar multiplication on the private key using big-integer
arithmetic that is not constant time. Its timing depends on the key, and that
is a side channel no amount of vector testing can rule out. The verify-only
module is defensible because nothing it touches is secret. That argument does
not extend to signing, and `_ed25519` keeps saying so.

The error rule comes from the issue. A missing optional dependency must
produce a clear error, never a different answer. A signer that silently
switched backends would produce the same bytes, since Ed25519 is deterministic,
but a verifier whose result depended on what happened to be installed would
not. So the optional dependency sits on the side where its absence can only
ever be an error.

### 4. The witness is the published signature, byte for byte

Each SDK's test signs the published commitment in
`tests/vectors/attestation-vectors.json` with the published seed and compares
the result to the published signature with exact equality. It also derives the
published public key from the published seed. A signer checked only against
its own verifier proves that the two agree with each other, and a shared bug
passes that check. An SDK that ports the record layer does the same with
`tests/fixtures/record-attestation-key.json`.

### 5. Custody is documented beside the convenience

Each SDK README carries a key custody section with two parts:

- **The HSM or KMS recipe.** Compute the commitment (or the record attestation
  message), have the backend produce a pure Ed25519 signature over exactly
  those bytes, and assemble the attestation by hand. The recipe is a worked
  example, not a sentence.
- **What an in-memory key costs.** Anything that can read the process can sign
  as the provider: a heap snapshot, a core dump, a compromised dependency, a
  log line that serialized the wrong object. Nothing on the wire shows that a
  signature came from a stolen copy. A long-lived key keeps that exposure open
  until every host that trusts it removes it from its trust store (ADR 0016).
  Rotate on a schedule short enough that you would accept that window.

## Alternatives rejected

- **Decline, and document a recipe in each README.** This is the other outcome
  #127 allowed. It keeps the SDKs free of signing code, but the recipe would be
  copied by hand into every provider, where assembling the struct, hex-encoding
  the signature, or picking the right bytes to sign (the frame commitment, not
  the chain head; the record message, not the bare digest) can go wrong
  silently. A signer pinned to a published vector gets that right once.
- **A pluggable asynchronous `Signer` interface in place of seeds.** It would
  model an HSM directly, but it adds an async surface that three languages
  must keep in step, and it adds no capability. The preimage functions are
  already public, and an HSM path is three lines around them. If a real need
  appears, it can be added without breaking anything decided here.
- **A hand-written Python signer, to keep one install everywhere.** Rejected
  for the constant-time reason in §3.
- **A required `cryptography` dependency for Python.** This would break the
  zero-dependency promise for every provider, including the majority that
  never sign.

## Consequences

- A provider in any of the four languages can produce an attestation that the
  published key verifies, and each SDK proves it on every pull request.
- The Python SDK has its first optional dependency. `pip install
  contextgraph-sdk` is unchanged. Signing requires
  `pip install contextgraph-sdk[signing]`, and the error says so.
- The TypeScript `AttestationVerdict` gains `valid_identity_only`, and the
  SDK gains `verifyFrameInclusion` (the Rust reference's
  `verify_frame_inclusion`, with the same 64-step path bound), bringing its
  verifier in line with ADR 0018 for a frame signed directly and for one
  signed through a result-set root. It is additive, and `isValid` stays
  `false` for the new verdict.
- The Rust reference's `sign_frame_attestation` predates ADR 0018 and still
  signs a digest-less frame. §2 applies to it too. Aligning it is a change to
  `contextgraph-types`, not to any SDK, and until it lands the SDKs are
  stricter than the reference here and nowhere else.
- Signing adds a way to misuse a key that did not exist before. The README
  sections are there so that nobody takes on that risk without being told
  what it costs.

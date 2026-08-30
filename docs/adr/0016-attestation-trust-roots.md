# 0016 — Trust roots for provenance attestation: the operator, not a registry

**Status:** Accepted (`contextgraph/1.0`; host-only, no wire or spec change)

## Context

[ADR 0010](./0010-provenance-attestation.md) defines the *preimage* a provider
signs and stops there, on purpose: `frame_commitment` and `merkle_root` are
public 32-byte values, so a provider holding keys in an HSM or a KMS signs them
with its own backend and `contextgraph-types` never touches a secret. §6 of that
ADR names key distribution as out of scope and its Consequences list it as
follow-up work.

That boundary is right and this ADR does not move it. What it leaves unanswered
is the question on the other side of the wire: **a host holds a
`ProvenanceAttestation` and a frame — where does the public key come from?**

Until that has an answer, `contextgraph_types::attest` verifies nothing in
practice. Verification works only where the same operator configured both sides
and hard-coded the key into their own host, which is most of the value gone: the
buyer for attestation is the auditor who was not in the room when either side
was installed.

The pressure here runs toward a registry. Every scheme that makes a key
discoverable — a well-known endpoint, a transparency log, a signed provider
directory, a CA — works by introducing a party both sides already trust. That
party is an organization. [`GOVERNANCE.md`](../../GOVERNANCE.md)'s consent
boundary rules it out in advance and says why: "a host that needs an
organization behind it to function is no longer a protocol implementation an
individual can run, and 'conformant' would quietly come to mean 'connected to
someone's control plane.'" The rejection there is not about quality. A
transparency log is an excellent design. It is out of scope for the host, and
the answer does not change because the alternative is less convenient.

One further constraint is a fact about today's wire rather than a principle: a
`ProvenanceAttestation` carries a `key_id`, an `algorithm` and a signature. It
does **not** carry a public key, and neither does `handshake_ack`. So the
"trust the key the provider hands you" option is not merely weak — it is not
implementable at all without a normative addition to the wire types, which is a
different decision from this one.

## Decision

**The operator is the trust root. Trust is host-local, keyed by
`(provider_id, key_id)`, and the protocol specifies no key distribution.**

Four parts.

### 1. A host-side trust store, mirroring the consent store

`contextgraph_host::TrustStore` maps a `provider_id` to the keys that provider
may sign under, each named by its `key_id`. It is in-memory, serde-able and
persistable, exactly like [`ConsentStore`](../../contextgraph-host/src/consent.rs)
— because it is the same kind of object: a local record of a decision a person
made about one provider on one machine. A host that already persists consent
persists trust with the code it already has.

Nothing populates it implicitly. A key is in the store because the operator put
it there, in the same act and from the same material as the provider's command
line or URL. That is the whole distribution mechanism, and it is the one that
needs no organization: it is how `ssh` knows a host key, how `minisign` knows a
signer, and how an `age` recipient is chosen.

### 2. The trust decision belongs beside the consent decision

A `ConsentReceipt` already pins provider identity — name and version — at the
moment a person grants egress. "I consent to this provider" and "I trust this
provider's signing key" are the same judgement about the same party, made from
the same evidence, and a host should present them together: the key's
fingerprint (`TrustedKey::fingerprint`, a `sha256:` over the key bytes) shown in
the consent prompt, recorded when the grant is recorded.

This is a **host convention, deliberately not a wire field.** Putting a public
key inside `ConsentReceipt` would make every host that stores a receipt store a
key, would make key rotation a receipt rewrite against an append-only ledger,
and would spend a normative wire change on something the operator has to supply
out of band regardless. The receipt records *what was agreed*; the trust store
records *what is currently believed about a key*. They have different lifetimes
and the append-only one must not carry the mutable one.

### 3. An unknown key is unattested evidence, never rejected evidence

`SPEC.md` F9 is the security-critical half and this ADR does not soften it. A
frame whose attestation cannot be checked — no key held, bad signature,
unrecognised algorithm, garbage bytes — is composed exactly as a frame carrying
no attestation is. The host records *why* and keeps serving the evidence.

A host that dropped such frames would hand any peer a denial-of-service
primitive: attach a malformed attestation to a rival provider's shape of
evidence and watch it vanish from the prompt. Verification adds a fact to the
audit. It never subtracts evidence, and it never reranks.

### 4. Named outcomes, because "I have no key" and "this is forged" are different

`AttestationState` distinguishes: no check performed, no attestation offered,
attested, no trusted key for that `key_id`, an unrecognised algorithm, and a
named `AttestationVerdict` failure. This follows ADR 0010 §7 for the same
reason: `NoTrustedKey` is a configuration gap the operator can close in a
minute, and `Invalid { CommitmentMismatch }` is an incident. A boolean sends an
operator hunting the wrong one.

The state rides on every `AuditEntry`, not only the included ones. A frame
dropped as a cross-provider duplicate has an attestation state too, and it is
worth seeing: dedup keeps the higher-scored copy, which may be the unsigned one.

## Alternatives considered

**Public key in the `handshake_ack`, pinned trust-on-first-use.** Rejected for
this revision, on two grounds. It proves *continuity* — the same key that
answered last time is answering now — and never *identity*; an attacker present
at first contact is trusted forever, and the audit record cannot tell an
operator which of their providers were verified against a key a person actually
checked. It is also not implementable today: no wire field carries a public key,
so adopting it means a normative addition, which is its own ADR and its own
conformance requirement. Worth having eventually as a **labelled second tier**
below a configured key, never as the only tier. Tracked as follow-up.

**A key registry, well-known endpoint, or transparency log.** Rejected as out of
scope, per `GOVERNANCE.md`'s consent boundary. These are the *right* answer for
a fleet product and the wrong answer for a host, and the protocol already
supplies the primitives one would be built from — the attestation is portable
and offline-checkable by anyone holding the key, so a registry can be built
*over* this without being built *into* it.

**Bind the public key into `ConsentReceipt` as a wire field.** Rejected; see
Decision §2. The receipt is an append-only audit artifact and a key is mutable
state. Rotation would either rewrite history or accumulate contradictory
receipts, and neither is a ledger anyone should read.

**Derive the key from the provider id.** Rejected. A `provider_id` is a host's
local routing key, chosen by the host operator and not globally unique; making
it cryptographic material would silently overload the one identifier a person is
free to rename.

**Declare key distribution out of scope and ship nothing.** Rejected. It is a
defensible protocol position and it was already the state of the tree — with the
result that `contextgraph_types::attest` shipped complete and unreachable. The
protocol can decline to specify distribution while the reference host still
needs *some* answer, and "the operator supplies it" is an answer, written down.

## Consequences

- `contextgraph-host` gains `trust::TrustStore`, `TrustedKey`,
  `AttestationState`, and an `AttestationLedger` produced by a fan-out. It
  enables `contextgraph-types`' `attestation` feature unconditionally; the
  types crate's own zero-dependency default is unchanged.
- `AuditEntry` gains an `attestation` field, and `ContextProvider` gains a
  default `query_attested` method. Both are Rust-semver breaking for
  `contextgraph-host` and neither touches the wire.
- **What this does not prove, stated plainly:**
  - Trust is **local and non-transitive.** A frame this host marks `Attested`
    carries no weight for a second host that holds no key. The attestation
    itself is portable — anyone with the key can check it offline, which is the
    point — but the *decision to trust the key* travels with nobody.
  - There is **no revocation.** A key stays trusted until an operator removes
    it. Nothing here notices a compromise, and no expiry is enforced.
  - There is **no rotation protocol.** A new `key_id` is trusted when the
    operator adds it; until then the provider's frames read as `NoTrustedKey`.
    That degradation is safe (F9) and silent to the model — it is visible only
    in the audit, so a host that never reads its audit learns nothing.
  - `Attested` means **"signed by a key this operator chose to trust"** and
    nothing more. It does not mean the content is true, that the provider is
    who it claims to be to any third party, or that the operator checked the
    key against anything at all. A trust store populated carelessly produces
    confident-looking green marks over nothing, and the protocol cannot tell.
  - The **default posture is unattested.** A host that configures no keys gets
    `NoTrustedKey` on every signed frame and loses nothing it had before.

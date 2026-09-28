# 29. The trust store is a documented file at a path the operator names

- Status: accepted
- Date: 2026-09-28
- Resolves #134. Builds on [ADR 0016](0016-attestation-trust-roots.md) (the
  operator is the trust root) and
  [ADR 0028](0028-key-validity-windows-are-evaluated-at-receipt.md) (key
  validity windows). Host-only: no wire, schema or `SPEC.md` change.

## Context

`contextgraph_host::TrustStore` was serde-able and `Host::set_trust_store`
restored one, and nothing in the repository ever read or wrote one. Every host
started empty, so in practice a consumer verified nothing unless it called
`Host::trust_key` in code. `ConsentStore` has the same gap, but consent fails
*closed* — an unconsented egress provider is not queried — while an empty trust
store fails to a silent `NoTrustedKey` on every frame, which F9 makes correct
and which is easy never to notice.

The issue was clear that the shape matters less than the fact that one exists
and is written down. Two constraints came from ADR 0016: loading a key is an
**operator act**, so only a path the operator names is in scope, and a host
should show `TrustedKey::fingerprint()` beside the consent prompt, so a loader
that never surfaces the fingerprint loses that.

## Decision

**1. A versioned JSON document, `contextgraph-trust/1`, distinct from the
store's in-memory serde form.**

```json
{
  "format": "contextgraph-trust/1",
  "providers": {
    "docs": [
      {
        "key_id": "docs-2026-08",
        "algorithm": "ed25519",
        "public_key": "<64 hex>",
        "fingerprint": "sha256:<64 hex>",
        "not_before": "2026-08-01T00:00:00Z",
        "not_after": "2027-07-31T23:59:59Z"
      }
    ]
  }
}
```

`providers` is keyed by the provider's **local** id — the one the operator
registered it under and records consent under — because that is the id trust
is keyed on (the two-id split in `TrustStore::check_result_signed_as`). Keys
are a list rather than a map keyed by `key_id`, so a `key_id` is written once
and cannot disagree with a map key. The `format` member names the document, so
a later format is refused as a later format rather than misread as this one.
The in-memory serde form of `TrustStore` stays what it was and is not the file
format: a Rust derive is an implementation detail, and a file a person edits is
a contract.

**2. Strict on read.** Every member is known, and any other is an error — a
misspelt `not_afer` silently ignored would widen a windowed key's trust to
forever. The loader refuses, by name: an unreadable or missing file
(`TrustFileError::Unreadable`), one over 1 MiB, text that is not JSON, a
missing or foreign `format`, an unknown or mistyped member, an empty provider
id or `key_id`, an algorithm other than `ed25519`, a key that is not 64 hex
characters, a malformed or inverted window, the same `key_id` twice for one
provider, and a recorded fingerprint that does not match its key
(`TrustFileError::Invalid { problem }`). None of them is ever turned into an
empty or partial store.

**3. The fingerprint travels with the key.** The saver writes every key's
`fingerprint`. The loader accepts a file without one — an operator pasting a
key from a provider's README should not have to compute a hash first — and,
when one is present, requires it to match. So a person who compared
fingerprints with a provider's operator out of band can paste the one they
compared, and the file then refuses a key edited afterwards. A host showing the
consent prompt renders `TrustedKey::fingerprint()` for each of
`TrustStore::keys_for(provider_id)`, which is the same string the file records.

**4. Only a path the operator names.** `TrustStore::load(path)` and
`TrustStore::save(path)` take a path and do nothing else: no default location,
no search path, no environment variable, no discovery. A missing file is an
error like any other; a host that wants "empty when absent" writes that choice
itself, where a reviewer can see it.

**5. Writes are atomic and never produce an unreadable file.** `save` writes a
temporary file beside the target, flushes it, and renames it into place, so a
crash mid-write leaves the previous file whole. It also refuses a store holding
a key the loader would refuse, so a file this host writes is always one it can
read back.

**6. Beside consent, not inside it.** The file is its own document rather than
a member of whatever holds consent, for ADR 0016 §2's reason: consent is an
append-only record of what was agreed, and trust is mutable belief about a
key. A host that persists both keeps two files, loaded at the same moment.

## Consequences

- `contextgraph-host` gains `trust_file` with `TrustStore::load`, `save`,
  `from_trust_file_json` and `to_trust_file_json`, `TrustFileError`,
  `TrustFileProblem`, `TRUST_FILE_FORMAT` and `MAX_TRUST_FILE_BYTES`, and
  `TrustStore::providers()` for rendering what an operator has trusted. All
  additive.
- Witnesses: `tests/trust_file.rs`'s
  `a_store_written_to_a_file_verifies_frames_after_it_is_read_back` (the
  round trip, through a host) and the malformed-file tests there and in
  `trust_file`'s unit tests.
- `docs/composing-frames-into-a-prompt.md` documents the file, the path rule,
  and the fingerprint in the consent flow.
- The file holds public keys only and is not secret. Whoever can write it
  decides what the host believes, so it should be writable by the operator
  alone; this ADR states that and does not enforce file modes, which are a
  platform concern.
- `ConsentStore` still has no file of its own. It is the same kind of gap and
  a different document, and it is not decided here.

## Alternatives considered

**Serialize `TrustStore` as it already derives.** One line of code, and it
would make an internal `HashMap` layout the thing operators edit, with no
version, no strictness and no fingerprint. The first refactor of the struct
would silently change the file format.

**A `[[trusted_key]]` block in an existing provider config.** It would put the
key beside the command line that launches the provider, which is appealing.
This repository has no host configuration file for it to live in — each host
has its own — so it would be a format the reference host does not read. A host
that has such a config can embed this document's entries in it; the entry
shape is the contract.

**Lenient reading: skip a bad entry, load the rest.** Friendlier on a bad day,
and exactly the "silently empty" failure the issue exists to prevent, one key
at a time.

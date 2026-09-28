# contextgraph-sdk — Python

A zero-dependency (stdlib-only) Python SDK for building **conformant** Context
Graph Protocol providers. Implement one small interface, hand it to the runtime,
and you have a provider that speaks the line-oriented JSON wire over stdio and
passes the same conformance suite that judges the Rust reference provider.

> Third independent implementation (after Rust and TypeScript); passes the full
> conformance suite. See [`sdk/README.md`](../README.md) for the whole picture.

## Install

```sh
pip install contextgraph-sdk
```

Signing attestations needs one optional package; everything else needs none
(see [Sign an attestation](#sign-an-attestation-optional)):

```sh
pip install "contextgraph-sdk[signing]"
```

Python 3.9 or newer. CI runs the SDK's tests on the declared floor (3.9) and
on the newest CPython release, and typechecks it with `mypy --strict` against
the floor as well as 3.10.

## Write a provider

```python
from contextgraph_sdk import run_stdio_provider, budget_tokens


class MyDocsProvider:
    def info(self):
        # Nothing leaves the machine -> declare the honest local-only egress scope.
        return {
            "name": "my-docs-provider",
            "version": "0.1.0",
            "data_flow": {"reads": True, "writes": False, "egress": False,
                          "egress_scopes": ["local-only"]},
        }

    def capabilities(self):
        return {"query": {"kinds": ["doc"]}, "correlation": True, "verify": True}

    def query(self, query):
        content = "Install the binding, then implement the required methods."
        return {
            "frames": [{
                "id": "doc:1", "kind": "doc", "title": "Getting started",
                "content": content,
                "content_digest": "sha256:" + ("11" * 32),
                "score": 0.9,
                # token_cost MUST equal ceil(utf8_len(content)/4).
                "token_cost": budget_tokens(content),
                "valid_from": "2026-01-01T00:00:00Z",
                "provenance": [{"type": "file", "uri": "file:///docs/start.md",
                                "range": "L1-10", "digest": "sha256:" + ("11" * 32)}],
                "citation_label": "start.md L1-10", "relations": [],
            }],
            "truncated": False,
        }


run_stdio_provider(MyDocsProvider())
```

`verify` is optional. The runtime handles the whole lifecycle — handshake, query
(echoing the correlation `id`), verify, shutdown — and stays alive with a typed
error on a malformed line rather than crashing.

## Host it over HTTP

The same provider runs behind a single POST endpoint (the streamable-HTTP
transport, SPEC.md §3) via a WSGI app — runnable on the stdlib server or any WSGI
host (gunicorn, Flask):

```python
from wsgiref.simple_server import make_server
from contextgraph_sdk import make_wsgi_app

make_server("127.0.0.1", 8788, make_wsgi_app(MyDocsProvider())).serve_forever()
# Flask:           app.wsgi_app = make_wsgi_app(provider)
# FastAPI (ASGI):  reply with respond_to_body(provider, await request.body()) in your route
```

`handle_envelope(provider, envelope)` is the transport-free state machine if you
want to wire it into a framework yourself. A runnable HTTP example lives at
`examples/example_docs_http.py`; confirm it green with
`contextgraph-inspect http http://127.0.0.1:8788` (the `malformed-input-tolerance`,
`embedding-fingerprint`, and `correlation` probes report *skipped* over HTTP —
they inspect raw framing this transport doesn't expose).

## Verify a provenance attestation

`SPEC.md` §6.5 makes a frame's provenance *evidence* rather than merely
tamper-evident: a detached Ed25519 signature over a commitment to the frame's
identity and its provenance chain. `contextgraph_sdk.attest` implements the
whole construction — the length-prefixed link encoding, the source-first chain
fold, the frame commitment, an RFC 6962 Merkle root over a result set with
inclusion proofs, and verification.

```python
from contextgraph_sdk import verify_frame_attestation, Verdict

result = verify_frame_attestation("repo-graph", frame, attestation, public_key)
if result.verdict != Verdict.VALID:
    # Never a boolean: "the frame changed after signing" and "the key is
    # wrong" call for opposite responses, and F9 says an unverifiable
    # attestation degrades a frame to unattested rather than disqualifying it.
    print(result)
```

Two things worth knowing:

- **`len(s)` is not a UTF-8 byte count.** The §6.5.1 length prefix is bytes;
  `len` on a `str` counts code points, which differs for every non-ASCII
  string. This SDK measures what `s.encode("utf-8")` produced.
- **Ed25519 verification carries no dependency.** The standard library has no
  Ed25519 and this SDK promises no third-party packages, so
  `contextgraph_sdk._ed25519` is a self-contained RFC 8032 **verifier** —
  never a signer — matching `ed25519-dalek`'s `verify_strict`. It is checked
  against RFC 8032 §7.1's own vectors, against a dalek-produced signature, and
  differentially against the `cryptography` package wherever that happens to be
  installed.

The vectors are shared across every language:

```sh
cd sdk/python && python3 -m unittest discover -s tests -v
```

They come from `tests/vectors/attestation-vectors.json`, which the Rust
reference publishes and pins.

## Sign an attestation (optional)

Producing an attestation needs a signer, and this SDK does not sign in pure
Python. Signing multiplies by a secret scalar, and big-integer arithmetic in
Python leaks that scalar through timing. The in-package code is a verifier
only. Signing goes through the `cryptography` package instead, which uses
OpenSSL's constant-time Ed25519. It ships as an optional extra:

```sh
pip install "contextgraph-sdk[signing]"
```

```python
from contextgraph_sdk import public_key_for, sign_frame_attestation

attestation = sign_frame_attestation(
    "repo-graph", frame, seed, key_id="key-1", attester_id="acme",
    issued_at="2026-09-28T00:00:00Z",
)
wire = attestation.to_wire()          # the JSON object that travels
public_key = public_key_for(seed)     # what a verifier needs
```

`sign_commitment(...)` signs any 32-byte commitment, such as a `merkle_root(...)`
over a result set. `sign_frame_attestation` refuses a frame that declares no
`content_digest`: `SPEC.md` §6.5.2 requires one on every frame you sign,
because without it the signature covers the frame's name and none of its
content (ADR 0018).

Without the extra, every signing function raises `SigningUnavailableError`,
and the message names the extra to install. There is no fallback, so you never
get a different answer depending on what is installed. Verification, hashing
and the provider runtime never need the extra.

The signers are pinned, not round-tripped. `tests/test_signing.py` signs the
published commitment and the published record with the published seeds. It
compares the result to the published signatures byte for byte, and CI runs it
with the extra installed on every pull request.

### Key custody

These functions take a raw 32-byte seed, and holding a long-lived signing seed
in application memory has a cost. Anything that can read the process can read
the seed: a core dump, a debugger, a heap-disclosure bug, a compromised
dependency. Python `bytes` cannot be wiped, so the seed stays readable until
the garbage collector reclaims it, and possibly after. A key that leaks lets
anyone sign as you until every verifier stops trusting its `key_id`.

That is acceptable for tests, local tools, and short-lived keys you rotate
often. For a long-lived key, keep it in an HSM, a cloud KMS, or a signing
service, and do not call these functions at all. The protocol specifies the
preimage, never the custody of the key:

```python
from contextgraph_sdk import ProvenanceAttestation, digest_string, frame_commitment

commitment = frame_commitment("repo-graph", frame)   # 32 bytes
signature = kms.sign_ed25519(key_ref, commitment)    # your backend, 64 bytes
attestation = ProvenanceAttestation(
    signed_commitment=digest_string(commitment), key_id="key-1",
    algorithm="ed25519", attester_id="acme", signature=signature.hex(),
    issued_at="2026-09-28T00:00:00Z",
)
```

For a record attestation, the bytes to sign are
`record_attestation_message(record_hash(record))`.

## Hash and verify a lifecycle record

The lifecycle profile (`docs/profiles/context-exchange-provider.md`) addresses
a record by its `record_hash`: SHA-256 over the RFC 8785 (JCS)
canonicalization of the record, with its own top-level `record_hash` member
removed. A `RecordAttestation` is a detached Ed25519 signature over
`"contextgraph/attest/1/record"` followed by that hash's 32 raw bytes.

```python
from contextgraph_sdk import (
    RecordAttestation, Verdict, record_hash, verify_record_attestation,
)

digest = record_hash(record)          # "sha256:…", the record's identity
result = verify_record_attestation(record, RecordAttestation.from_wire(att), key)
assert result.verdict == Verdict.VALID
```

`verify_record_attestation` recomputes the hash from the record's content. It
never trusts the stored `record_hash` member, so a record edited after signing
and then given a matching `record_hash` is still reported as
`commitment_mismatch`.

The canonicalizer is a conforming RFC 8785 implementation in the standard
library alone. `json.dumps(sort_keys=True, separators=(",", ":"))` is not one,
for three reasons:

- **Numbers.** It lays numbers out as Python's `repr` does, and JCS requires
  ECMAScript's `Number::toString`. The two disagree at `1e16`–`1e21`, below
  `1e-4`, and on every whole-number float (`1.0` against `1`).
- **Member order.** It sorts names by code point, and JCS sorts by UTF-16
  code unit.
- **Strings.** It escapes differently, and it lets a lone surrogate through
  where JCS requires a refusal.

`canonicalize(value)` and `record_hash_preimage(record)` return the exact
bytes, for diffing against another implementation. `tests/test_record.py`
checks the RFC's Appendix B number table and its worked example, and
reproduces every vector in `tests/fixtures/record-hash-vectors.json`.

## Prove it conformant

From the repository root, with the Rust bins built:

```sh
cargo build --workspace --bins
./.github/scripts/conformance-external.sh -- python3 sdk/python/examples/example_docs.py
```

A green run is the machine-checkable claim that your provider honors the protocol.

## License

MIT OR Apache-2.0, matching the Context Graph Protocol crates.

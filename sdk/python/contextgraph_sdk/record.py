"""Record content addressing and record attestation, in Python.

A port of ``contextgraph_types::record_attest`` — the lifecycle profile's
``record_hash`` and ``RecordAttestation``
(``docs/profiles/context-exchange-provider.md`` ``LH1``, ``LH2``, ``LH5``,
``LC4``, ``LC5``; ADR 0017). The Rust crate is the reference, and
``sdk/python/tests/test_record.py`` reconciles this module against the vectors
it publishes in ``tests/fixtures/``.

Where :mod:`contextgraph_sdk.attest` covers the *frame* layer a
``context/query`` returns, this covers the *record* layer a Context Exchange
Provider appends, gets and resolves:

1. :func:`record_hash` — ``sha256:<hex>`` over the RFC 8785 (JCS)
   canonicalization of a record **with its own top-level ``record_hash``
   member removed** (``LH1``). Removed, not blanked: a record hashes
   identically whether it carries no ``record_hash``, the right one, or a
   wrong one, so a producer never invents a placeholder and a verifier never
   has to know which one was chosen. A ``record_hash`` nested inside
   ``extensions`` or a body member is ordinary content and stays.
2. :func:`verify_record_attestation` — a detached Ed25519 signature over
   :data:`RECORD_ATTESTATION_DOMAIN` followed by the hash's 32 raw bytes
   (``LC4``), checked against a hash **recomputed** from the record rather
   than the stored member (``LC5``), so a record whose ``record_hash`` was
   rewritten to match a stolen signature is caught.

The canonicalizer is :mod:`contextgraph_sdk._jcs`, a conforming RFC 8785
implementation in the standard library alone; its header says why
``json.dumps(sort_keys=True)`` is not one. Verification uses the in-package
:mod:`contextgraph_sdk._ed25519` verifier, so neither half needs a dependency.
Signing (:func:`sign_record_attestation`, :func:`sign_record`) needs the
optional ``cryptography`` backend, exactly as in :mod:`contextgraph_sdk.attest`.
"""

from __future__ import annotations

import hashlib
from dataclasses import dataclass
from typing import Any, Dict, Mapping

from . import _ed25519, _jcs, _signing
from ._jcs import CanonicalizationError
from .attest import (
    ALGORITHM_ED25519,
    AttestationVerdict,
    Verdict,
    _from_strict_hex,
    digest_string,
    parse_digest,
)

__all__ = [
    "RECORD_ATTESTATION_DOMAIN",
    "RECORD_HASH_MEMBER",
    "CanonicalizationError",
    "RecordAttestation",
    "RecordHashError",
    "canonicalize",
    "record_attestation_message",
    "record_hash",
    "record_hash_is_current",
    "record_hash_preimage",
    "sign_record",
    "sign_record_attestation",
    "verify_record_attestation",
    "verify_signed_record_hash",
]

#: The envelope member a record's own hash lives in, and the one member removed
#: from its preimage (``LH1``).
RECORD_HASH_MEMBER = "record_hash"

#: The domain-separation tag a record attestation signs under (``LC4``).
#:
#: Normative: the signed message is these bytes followed by the 32 raw bytes of
#: ``signed_record_hash``. A ``record_hash`` is a plain SHA-256 over a JSON
#: document, which unrelated systems also compute; signing it bare would let one
#: signature mean whatever its presenter says. Both halves are fixed length, so
#: the concatenation is injective without a length prefix.
RECORD_ATTESTATION_DOMAIN = b"contextgraph/attest/1/record"

#: RFC 8785 canonicalization of any JSON value, re-exported: the function an
#: implementer reaches for when their hash differs and a byte diff is needed.
canonicalize = _jcs.canonicalize


class RecordHashError(ValueError):
    """Why a ``record_hash`` could not be computed or used.

    :attr:`kind` names the case, because the three call for different
    responses: a non-object is a caller bug, a canonicalization failure is a
    record carrying something JCS refuses (a NaN, a lone surrogate), and a
    malformed digest is a wire value that failed its grammar. Mirrors the
    Rust reference's ``RecordHashError``.
    """

    NOT_AN_OBJECT = "not_an_object"
    NOT_CANONICALIZABLE = "not_canonicalizable"
    MALFORMED_DIGEST = "malformed_digest"

    def __init__(self, kind: str, detail: str) -> None:
        super().__init__(detail)
        #: One of :attr:`NOT_AN_OBJECT`, :attr:`NOT_CANONICALIZABLE`,
        #: :attr:`MALFORMED_DIGEST`.
        self.kind = kind


@dataclass(frozen=True)
class RecordAttestation:
    """A detached attestation over one record's ``record_hash`` (``LC4``).

    Detached, like a frame's: it never travels inside the preimage it signs,
    so re-signing after a key rotation cannot perturb the record's identity.
    """

    #: The ``sha256:<hex>`` record hash this attestation signs.
    signed_record_hash: str
    #: The signing key's id. Rotation issues a new id; it never reuses one.
    key_id: str
    #: The signature scheme, e.g. :data:`~contextgraph_sdk.attest.ALGORITHM_ED25519`.
    algorithm: str
    #: The attesting authority, as distinct from the key that signed.
    attester_id: str
    #: The detached signature, lowercase hex.
    signature: str
    #: When the attestation was issued (a ``SPEC.md`` §F4 protocol timestamp).
    issued_at: str

    @classmethod
    def from_wire(cls, obj: Mapping[str, Any]) -> "RecordAttestation":
        """Build one from a decoded JSON object, ignoring unknown members."""
        return cls(
            signed_record_hash=obj["signed_record_hash"],
            key_id=obj["key_id"],
            algorithm=obj["algorithm"],
            attester_id=obj["attester_id"],
            signature=obj["signature"],
            issued_at=obj["issued_at"],
        )

    def to_wire(self) -> Dict[str, str]:
        """The JSON object this attestation travels as — :meth:`from_wire`'s
        inverse."""
        return {
            "signed_record_hash": self.signed_record_hash,
            "key_id": self.key_id,
            "algorithm": self.algorithm,
            "attester_id": self.attester_id,
            "signature": self.signature,
            "issued_at": self.issued_at,
        }

    def uses_known_algorithm(self) -> bool:
        """Whether this names a scheme this revision defines."""
        return self.algorithm == ALGORITHM_ED25519


# ---------------------------------------------------------------------------
# Hashing (``LH1``)
# ---------------------------------------------------------------------------


def record_hash_preimage(record: Mapping[str, Any]) -> bytes:
    """The exact bytes a record's ``record_hash`` is taken over.

    RFC 8785 canonicalization of the record with its **top-level**
    ``record_hash`` member removed. Exposed beside :func:`record_hash` because a
    hash mismatch between two implementations is unreadable and a byte diff of
    the preimage is not — and it is what the golden vectors' ``jcs_utf8`` pins.

    :raises RecordHashError: the record is not a JSON object, or holds
        something RFC 8785 refuses.
    """
    if not isinstance(record, Mapping):
        raise RecordHashError(
            RecordHashError.NOT_AN_OBJECT, "a record must be a JSON object"
        )
    # A shallow copy is enough: only a top-level member is removed, and nested
    # values are read, never written.
    preimage = {
        name: value for name, value in record.items() if name != RECORD_HASH_MEMBER
    }
    try:
        return _jcs.canonicalize(preimage)
    except CanonicalizationError as error:
        raise RecordHashError(
            RecordHashError.NOT_CANONICALIZABLE,
            f"record is not canonicalizable under RFC 8785: {error}",
        ) from error


def record_hash(record: Mapping[str, Any]) -> str:
    """A record's content-addressed identity (``LH1``):
    ``"sha256:" + hex(sha256(JCS(record without its record_hash member)))``.

    :raises RecordHashError: as :func:`record_hash_preimage`.
    """
    return digest_string(hashlib.sha256(record_hash_preimage(record)).digest())


def record_hash_is_current(record: Mapping[str, Any]) -> bool:
    """Whether a record's stored ``record_hash`` is the one its content produces.

    ``False`` is the interesting answer: the record was edited after it was
    hashed, or was hashed by an implementation that canonicalizes differently.
    A record with no ``record_hash`` member is ``False`` rather than an error —
    it is unhashed, not malformed.

    :raises RecordHashError: as :func:`record_hash_preimage`.
    """
    computed = record_hash(record)
    stored = record.get(RECORD_HASH_MEMBER)
    return isinstance(stored, str) and stored == computed


def _raw_digest(digest: str) -> bytes:
    raw = parse_digest(digest) if isinstance(digest, str) else None
    if raw is None:
        raise RecordHashError(
            RecordHashError.MALFORMED_DIGEST,
            f"expected a sha256:<64 lowercase hex> digest, found {digest!r}",
        )
    return raw


def record_attestation_message(signed_record_hash: str) -> bytes:
    """The message an Ed25519 record attestation signs (``LC4``):
    :data:`RECORD_ATTESTATION_DOMAIN` followed by the digest's 32 raw bytes.

    The normative rule, not an implementation detail: a provider signing in an
    HSM or KMS builds these bytes, signs them with its own backend, and never
    hands this SDK a secret.

    :raises RecordHashError: the digest is not ``sha256:<64 lowercase hex>``.
    """
    return RECORD_ATTESTATION_DOMAIN + _raw_digest(signed_record_hash)


# ---------------------------------------------------------------------------
# Verification (``LC4``, ``LC5``)
# ---------------------------------------------------------------------------


def verify_signed_record_hash(
    expected_record_hash: str,
    attestation: RecordAttestation,
    public_key: bytes,
) -> AttestationVerdict:
    """Verify a detached attestation against an already-computed ``record_hash``.

    The primitive an auditor uses when they hold the hash and the signature but
    not the record — the point of a detached attestation over a content
    address. Every failure is named, exactly as for a frame attestation; the
    ``expected`` and ``signed`` members of a ``commitment_mismatch`` verdict
    carry record hashes here.
    """
    if attestation.algorithm != ALGORITHM_ED25519:
        return AttestationVerdict(
            Verdict.UNKNOWN_ALGORITHM, algorithm=attestation.algorithm
        )
    try:
        message = record_attestation_message(attestation.signed_record_hash)
    except RecordHashError:
        return AttestationVerdict(Verdict.MALFORMED_COMMITMENT)

    # Compare hashes *before* touching the signature: a mismatch means the
    # record changed after signing, and reporting it as a bad signature would
    # send an operator after a key-management bug instead.
    if attestation.signed_record_hash != expected_record_hash:
        return AttestationVerdict(
            Verdict.COMMITMENT_MISMATCH,
            expected=expected_record_hash,
            signed=attestation.signed_record_hash,
        )

    if not _ed25519.is_usable_public_key(public_key):
        return AttestationVerdict(Verdict.MALFORMED_KEY)
    signature = _from_strict_hex(attestation.signature)
    if signature is None or len(signature) != 64:
        return AttestationVerdict(Verdict.MALFORMED_SIGNATURE)

    ok = _ed25519.verify(public_key, message, signature)
    return AttestationVerdict(Verdict.VALID if ok else Verdict.BAD_SIGNATURE)


def verify_record_attestation(
    record: Mapping[str, Any],
    attestation: RecordAttestation,
    public_key: bytes,
) -> AttestationVerdict:
    """Verify a detached attestation against the record it claims to sign.

    Recomputes the record's hash rather than trusting its stored
    ``record_hash`` member (``LC5``): an attacker who edits the content and
    then rewrites ``record_hash`` to match produces a record that is internally
    consistent, and only the recompute catches it.

    :raises RecordHashError: the record cannot be hashed at all — a distinct
        outcome from every verdict, because "this document is not a record" is
        not a statement about the signature.
    """
    return verify_signed_record_hash(record_hash(record), attestation, public_key)


# ---------------------------------------------------------------------------
# Signing — optional, needs the ``cryptography`` backend (``[signing]`` extra)
# ---------------------------------------------------------------------------


def sign_record_attestation(
    signed_record_hash: str,
    signing_key_seed: bytes,
    key_id: str,
    attester_id: str,
    issued_at: str,
) -> RecordAttestation:
    """Sign a ``record_hash`` in-process, for providers content to hold key
    material in memory.

    A provider using an HSM or KMS calls :func:`record_attestation_message`
    instead, signs those bytes with its own backend, and builds the
    :class:`RecordAttestation` itself.

    :raises RecordHashError: the hash is not ``sha256:<64 lowercase hex>``.
    :raises ValueError: the seed is not 32 bytes.
    :raises ~contextgraph_sdk.attest.SigningUnavailableError: ``cryptography``
        is not installed.
    """
    message = record_attestation_message(signed_record_hash)
    signature = _signing.sign(signing_key_seed, message)
    return RecordAttestation(
        signed_record_hash=signed_record_hash,
        key_id=key_id,
        algorithm=ALGORITHM_ED25519,
        attester_id=attester_id,
        signature=signature.hex(),
        issued_at=issued_at,
    )


def sign_record(
    record: Mapping[str, Any],
    signing_key_seed: bytes,
    key_id: str,
    attester_id: str,
    issued_at: str,
) -> RecordAttestation:
    """Sign the record's own **recomputed** hash — the convenience a provider
    appending a record wants, so the signed hash cannot drift from the content
    by a copy-paste. A stored ``record_hash`` member is ignored, as ``LH1``
    requires.

    :raises RecordHashError: the record cannot be hashed.
    :raises ValueError: the seed is not 32 bytes.
    :raises ~contextgraph_sdk.attest.SigningUnavailableError: ``cryptography``
        is not installed.
    """
    return sign_record_attestation(
        record_hash(record), signing_key_seed, key_id, attester_id, issued_at
    )

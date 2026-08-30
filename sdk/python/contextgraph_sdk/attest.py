"""Provenance attestation — the ``SPEC.md`` §6.5 constructions, in Python.

This is a port of ``contextgraph_types::attest``, and the Rust crate is the
reference: the vectors in ``tests/vectors/attestation-vectors.json`` come from
it, and ``sdk/python/tests/test_attest.py`` reconciles every function here
against them.

The trap this file exists to avoid
----------------------------------

§6.5.1 length-prefixes each field with the **UTF-8 byte length** of its value.
``len(s)`` on a Python 3 ``str`` counts *code points*, which is a different
number for every string outside ASCII. A port that reaches for ``len`` produces
a self-consistent chain head that no other implementation agrees with, and an
ASCII-only test suite never notices. Nothing here measures a ``str``;
:func:`_enc_str` measures the bytes ``.encode("utf-8")`` produced.

No JSON canonicalizer
---------------------

The encoding was chosen over RFC 8785 (JCS) precisely so this port needs none
(ADR 0010). A provenance link is six optional strings; if you find yourself
reaching for ``json.dumps`` here, re-read §6.5.1.

No third-party dependency
-------------------------

The SDK promises zero dependencies, and Python's standard library ships SHA-256
but no Ed25519. Verification therefore uses :mod:`contextgraph_sdk._ed25519`, a
self-contained RFC 8032 verifier in this package — see that module's header for
why a verifier (and only a verifier) is a defensible thing to carry.
"""

from __future__ import annotations

import hashlib
from dataclasses import dataclass, field
from typing import Iterable, Optional, Sequence, Union

from . import _ed25519
from .types import Provenance

__all__ = [
    "ALGORITHM_ED25519",
    "AttestableFrame",
    "AttestationVerdict",
    "InclusionProof",
    "InclusionStep",
    "ProvenanceAttestation",
    "Verdict",
    "digest_string",
    "encode_provenance_link",
    "frame_commitment",
    "inclusion_proof",
    "merkle_root",
    "parse_digest",
    "provenance_chain_head",
    "root_from_proof",
    "verify_commitment",
    "verify_frame_attestation",
]

#: The signature algorithm this revision defines (``SPEC.md`` §6.5).
ALGORITHM_ED25519 = "ed25519"

# The domain-separation tags and Merkle prefixes the hashing rules use
# (``SPEC.md`` §6.5.1). These exact byte strings are normative — a port that
# spells one differently computes different commitments and interoperates with
# nothing.
_DOMAIN_GENESIS = b"contextgraph/attest/1/genesis"
_DOMAIN_LINK = b"contextgraph/attest/1/link"
_DOMAIN_FRAME = b"contextgraph/attest/1/frame"
_DOMAIN_MERKLE_EMPTY = b"contextgraph/attest/1/merkle-empty"

# RFC 6962 prefixes. Distinct so a leaf hash can never be reinterpreted as an
# interior node — the second-preimage defense that makes a proof mean what it
# claims.
_MERKLE_LEAF = b"\x00"
_MERKLE_NODE = b"\x01"

#: The largest value a four-byte unsigned prefix can carry.
_MAX_PREFIX = 0xFFFFFFFF


@dataclass(frozen=True)
class ProvenanceAttestation:
    """A detached attestation binding one frame's provenance to a signer.

    Detached, always: it never travels inside the preimage it signs, so
    re-signing after a key rotation cannot perturb a frame's identity.
    """

    #: The ``sha256:<hex>`` commitment this attestation signs.
    signed_commitment: str
    #: The signing key's id. Rotation issues a new id; it never reuses one.
    key_id: str
    #: The signature scheme, e.g. :data:`ALGORITHM_ED25519`.
    algorithm: str
    #: The attesting authority, as distinct from the key that signed.
    attester_id: str
    #: The detached signature, lowercase hex.
    signature: str
    #: When the attestation was issued (a ``SPEC.md`` §F4 protocol timestamp).
    issued_at: str

    @classmethod
    def from_wire(cls, obj: dict) -> "ProvenanceAttestation":
        """Build one from a decoded JSON object, ignoring unknown members."""
        return cls(
            signed_commitment=obj["signed_commitment"],
            key_id=obj["key_id"],
            algorithm=obj["algorithm"],
            attester_id=obj["attester_id"],
            signature=obj["signature"],
            issued_at=obj["issued_at"],
        )

    def uses_known_algorithm(self) -> bool:
        """Whether this names a scheme this revision defines."""
        return self.algorithm == ALGORITHM_ED25519


@dataclass(frozen=True)
class AttestableFrame:
    """The part of a frame a commitment covers.

    Only these three fields enter the preimage, so a caller holding a frame
    from elsewhere does not have to fabricate a ``score`` and a ``token_cost``
    to compute a commitment.
    """

    id: str
    content_digest: Optional[str] = None
    provenance: Sequence[Provenance] = field(default_factory=tuple)


@dataclass(frozen=True)
class InclusionStep:
    """One step of an :class:`InclusionProof`."""

    #: The sibling subtree hash, ``sha256:<hex>``.
    sibling: str
    #: Whether the sibling is the **left** operand at this level.
    sibling_is_left: bool


@dataclass(frozen=True)
class InclusionProof:
    """A proof that one commitment is a leaf of a signed :func:`merkle_root`."""

    #: The leaf's index in canonical order.
    leaf_index: int
    #: How many leaves the tree held — a root alone does not pin its size.
    leaf_count: int
    #: Sibling hashes from the leaf upward.
    path: Sequence[InclusionStep]


@dataclass(frozen=True)
class AttestationVerdict:
    """The outcome of checking a :class:`ProvenanceAttestation` (§6.5.4).

    Every failure is *named*. §6.5.4 requires a verifier to distinguish them:
    "the frame changed after signing" and "the key is wrong" send an operator
    in opposite directions, and F8 treats "I cannot check this" as a third
    answer again — so a boolean is not an acceptable return type here.
    """

    #: One of the :class:`Verdict` constants.
    verdict: str
    #: For ``commitment_mismatch``, the commitment recomputed from the frame.
    expected: Optional[str] = None
    #: For ``commitment_mismatch``, the commitment the attestation claims.
    signed: Optional[str] = None
    #: For ``unknown_algorithm``, the scheme that was named.
    algorithm: Optional[str] = None

    def is_valid(self) -> bool:
        """Whether this verdict is :data:`Verdict.VALID`.

        No other verdict is provisionally acceptable: the point of an
        attestation is that "I could not check it" and "it is good" are never
        the same answer.
        """
        return self.verdict == Verdict.VALID


class Verdict:
    """The named outcomes :class:`AttestationVerdict` can carry."""

    VALID = "valid"
    COMMITMENT_MISMATCH = "commitment_mismatch"
    BAD_SIGNATURE = "bad_signature"
    UNKNOWN_ALGORITHM = "unknown_algorithm"
    MALFORMED_KEY = "malformed_key"
    MALFORMED_SIGNATURE = "malformed_signature"
    MALFORMED_COMMITMENT = "malformed_commitment"


# ---------------------------------------------------------------------------
# Canonical encoding (``SPEC.md`` §6.5.1)
# ---------------------------------------------------------------------------


def _enc_str(s: str) -> bytes:
    """``uint32be(utf8_byte_length(s)) || utf8(s)``.

    The length comes from the encoded bytes, never from ``len(s)``. Unsigned
    and big-endian, both normative — ``to_bytes`` is told both explicitly
    rather than left to a default.
    """
    raw = s.encode("utf-8")
    if len(raw) > _MAX_PREFIX:
        raise ValueError(
            f"a provenance field of {len(raw)} bytes overflows the "
            "§6.5.1 uint32 length prefix"
        )
    return len(raw).to_bytes(4, "big", signed=False) + raw


def _enc_opt(s: Optional[str]) -> bytes:
    """``0x00`` for absent, ``0x01 || _enc_str(s)`` for present.

    The presence byte is what keeps absent distinct from empty. Without it
    ``uri=None`` and ``uri=""`` encode identically, and a URI could be deleted
    from a signed chain without disturbing the hash.
    """
    if s is None:
        return b"\x00"
    return b"\x01" + _enc_str(s)


def _link_field(link: Union[Provenance, dict], name: str) -> Optional[str]:
    """Read one optional field from a link.

    A missing key and an explicit ``None`` are both absent; ``""`` is present.
    """
    value = link.get(name)
    return None if value is None else str(value)


def encode_provenance_link(link: Union[Provenance, dict]) -> bytes:
    """The canonical encoding of one provenance link (``SPEC.md`` §6.5.1).

    Field order is normative: ``type``, ``uri``, ``range``, ``digest``,
    ``method``, ``by``.
    """
    kind = link.get("type")
    if kind is None:
        raise ValueError("a provenance link must state its type")
    return b"".join(
        (
            _enc_str(str(kind)),
            _enc_opt(_link_field(link, "uri")),
            _enc_opt(_link_field(link, "range")),
            _enc_opt(_link_field(link, "digest")),
            _enc_opt(_link_field(link, "method")),
            _enc_opt(_link_field(link, "by")),
        )
    )


def _sha256(*parts: bytes) -> bytes:
    h = hashlib.sha256()
    for part in parts:
        h.update(part)
    return h.digest()


def digest_string(raw: bytes) -> str:
    """Render 32 raw bytes as this protocol's ``sha256:<hex>`` digest string."""
    return "sha256:" + raw.hex()


def parse_digest(digest: str) -> Optional[bytes]:
    """Parse a ``sha256:<hex>`` digest string. ``None`` if malformed."""
    prefix = "sha256:"
    if not digest.startswith(prefix):
        return None
    hex_part = digest[len(prefix) :]
    if len(hex_part) != 64 or hex_part != hex_part.lower():
        return None
    try:
        return bytes.fromhex(hex_part)
    except ValueError:
        return None


# ---------------------------------------------------------------------------
# Chain head, frame commitment, Merkle tree (``SPEC.md`` §6.5.2–§6.5.3)
# ---------------------------------------------------------------------------


def provenance_chain_head(links: Iterable[Union[Provenance, dict]] = ()) -> bytes:
    """The head of a frame's provenance hash chain (``SPEC.md`` §6.5.2).

    Links fold **source-first**, in the order §6 requires them to be carried,
    so each step consumes the previous head and no link can be inserted,
    dropped, reordered or edited without changing the result. An empty chain
    hashes to the genesis value rather than to zero, so "no provenance" is a
    stated claim a signature can cover.
    """
    head = _sha256(_DOMAIN_GENESIS)
    for link in links:
        head = _sha256(_DOMAIN_LINK, head, encode_provenance_link(link))
    return head


def frame_commitment(
    provider_id: str, frame: Union[AttestableFrame, dict]
) -> bytes:
    """The commitment binding one frame's identity to its provenance chain.

    The ``(provider_id, frame id, content_digest)`` triple is not optional
    (``SPEC.md`` §6.5.2): two frames citing the same source share a chain head,
    so a signature over the head alone lifts from one frame onto another.
    """
    if isinstance(frame, AttestableFrame):
        frame_id, content_digest, provenance = (
            frame.id,
            frame.content_digest,
            frame.provenance,
        )
    else:
        frame_id = frame["id"]
        content_digest = frame.get("content_digest")
        provenance = frame.get("provenance") or ()
    preimage = (
        _enc_str(provider_id)
        + _enc_str(frame_id)
        + _enc_opt(content_digest)
    )
    return _sha256(_DOMAIN_FRAME, preimage, provenance_chain_head(provenance))


def _leaf_hash(commitment: bytes) -> bytes:
    return _sha256(_MERKLE_LEAF, commitment)


def _node_hash(left: bytes, right: bytes) -> bytes:
    return _sha256(_MERKLE_NODE, left, right)


def _split_point(n: int) -> int:
    """The largest power of two strictly less than ``n`` (RFC 6962's split)."""
    k = 1
    while k * 2 < n:
        k *= 2
    return k


def merkle_root(commitments: Sequence[bytes]) -> bytes:
    """The Merkle root over a set of frame commitments (``SPEC.md`` §6.5.3).

    RFC 6962's shape, not the "duplicate the last leaf on an odd level"
    shortcut, which admits two distinct leaf sets with the same root. The two
    agree on any power-of-two leaf count, which is why the published vectors
    include three and seven.
    """
    if len(commitments) == 0:
        return _sha256(_DOMAIN_MERKLE_EMPTY)
    if len(commitments) == 1:
        return _leaf_hash(commitments[0])
    k = _split_point(len(commitments))
    return _node_hash(merkle_root(commitments[:k]), merkle_root(commitments[k:]))


def inclusion_proof(
    commitments: Sequence[bytes], leaf_index: int
) -> Optional[InclusionProof]:
    """Build an :class:`InclusionProof`. ``None`` if the index is out of range."""
    if leaf_index < 0 or leaf_index >= len(commitments):
        return None
    path: list[InclusionStep] = []
    _collect_path(commitments, leaf_index, path)
    return InclusionProof(
        leaf_index=leaf_index, leaf_count=len(commitments), path=tuple(path)
    )


def _collect_path(
    commitments: Sequence[bytes], index: int, path: list
) -> None:
    """Walk down the tree accumulating sibling hashes, leaf-upward."""
    if len(commitments) <= 1:
        return
    k = _split_point(len(commitments))
    if index < k:
        _collect_path(commitments[:k], index, path)
        path.append(
            InclusionStep(
                sibling=digest_string(merkle_root(commitments[k:])),
                sibling_is_left=False,
            )
        )
    else:
        _collect_path(commitments[k:], index - k, path)
        path.append(
            InclusionStep(
                sibling=digest_string(merkle_root(commitments[:k])),
                sibling_is_left=True,
            )
        )


def root_from_proof(commitment: bytes, proof: InclusionProof) -> Optional[bytes]:
    """Recompute a Merkle root from a leaf commitment and its proof.

    The whole offline story: an auditor holding one frame, its proof and a
    signed root needs nothing else. ``None`` if any sibling is malformed or the
    index does not sit inside the stated leaf count.
    """
    if proof.leaf_index >= proof.leaf_count:
        return None
    acc = _leaf_hash(commitment)
    for step in proof.path:
        sibling = parse_digest(step.sibling)
        if sibling is None:
            return None
        acc = (
            _node_hash(sibling, acc)
            if step.sibling_is_left
            else _node_hash(acc, sibling)
        )
    return acc


# ---------------------------------------------------------------------------
# Verification (``SPEC.md`` §6.5.4)
# ---------------------------------------------------------------------------


def verify_commitment(
    expected: bytes,
    attestation: ProvenanceAttestation,
    public_key: bytes,
) -> AttestationVerdict:
    """Verify a detached attestation over an already-computed commitment.

    Pure and offline: a commitment, an attestation and a public key are
    sufficient. ``public_key`` is the raw 32 bytes, matching the Rust
    reference.
    """
    if attestation.algorithm != ALGORITHM_ED25519:
        return AttestationVerdict(
            Verdict.UNKNOWN_ALGORITHM, algorithm=attestation.algorithm
        )
    signed = parse_digest(attestation.signed_commitment)
    if signed is None:
        return AttestationVerdict(Verdict.MALFORMED_COMMITMENT)

    # Compare commitments *before* touching the signature. A mismatch means the
    # frame changed after signing, and reporting that as a bad signature sends
    # an operator hunting a key-management bug when the finding is tampering.
    if signed != expected:
        return AttestationVerdict(
            Verdict.COMMITMENT_MISMATCH,
            expected=digest_string(expected),
            signed=attestation.signed_commitment,
        )

    if not _ed25519.is_usable_public_key(public_key):
        return AttestationVerdict(Verdict.MALFORMED_KEY)
    try:
        signature = bytes.fromhex(attestation.signature)
    except ValueError:
        return AttestationVerdict(Verdict.MALFORMED_SIGNATURE)
    if len(signature) != 64:
        return AttestationVerdict(Verdict.MALFORMED_SIGNATURE)

    ok = _ed25519.verify(public_key, signed, signature)
    return AttestationVerdict(Verdict.VALID if ok else Verdict.BAD_SIGNATURE)


def verify_frame_attestation(
    provider_id: str,
    frame: Union[AttestableFrame, dict],
    attestation: ProvenanceAttestation,
    public_key: bytes,
) -> AttestationVerdict:
    """Verify a detached attestation over a single frame (``SPEC.md`` §6.5.4)."""
    return verify_commitment(
        frame_commitment(provider_id, frame), attestation, public_key
    )

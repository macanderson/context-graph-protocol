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

Signing is optional
-------------------

:func:`sign_commitment`, :func:`sign_frame_attestation` and
:func:`public_key_for` mirror the Rust reference's in-process signers, and they
need the optional ``cryptography`` backend (``pip install
"contextgraph-sdk[signing]"``). Without it they raise
:class:`~contextgraph_sdk._signing.SigningUnavailableError` — never a different
answer. :mod:`contextgraph_sdk._signing` says why the SDK does not sign in pure
Python, and what holding a seed in memory costs; a provider whose key lives in
an HSM or KMS signs :func:`frame_commitment`'s 32 bytes with that backend
instead and never calls these.
"""

from __future__ import annotations

import hashlib
from dataclasses import dataclass, field
from typing import Any, Dict, Iterable, List, Mapping, Optional, Sequence, Union

from . import _ed25519, _signing
from ._signing import SigningUnavailableError
from .types import Provenance

__all__ = [
    "ALGORITHM_ED25519",
    "AttestableFrame",
    "AttestationVerdict",
    "InclusionProof",
    "InclusionStep",
    "MAX_INCLUSION_PATH_STEPS",
    "LinkLike",
    "FrameLike",
    "ProvenanceAttestation",
    "SigningUnavailableError",
    "Verdict",
    "digest_string",
    "encode_provenance_link",
    "frame_commitment",
    "inclusion_path_sides",
    "inclusion_proof",
    "merkle_root",
    "parse_digest",
    "provenance_chain_head",
    "public_key_for",
    "root_from_proof",
    "sign_commitment",
    "sign_frame_attestation",
    "verify_commitment",
    "verify_frame_attestation",
    "verify_frame_inclusion",
]

#: The signature algorithm this revision defines (``SPEC.md`` §6.5).
ALGORITHM_ED25519 = "ed25519"

#: A provenance link, as the typed :class:`~contextgraph_sdk.types.Provenance`
#: or as any decoded JSON mapping with the same members. A missing key and an
#: explicit ``None`` are both the encoding's absent; ``""`` is present.
LinkLike = Union[Provenance, Mapping[str, Any]]

#: A frame, as :class:`AttestableFrame` or as any mapping carrying ``id``,
#: ``content_digest`` and ``provenance``.
FrameLike = Union["AttestableFrame", Mapping[str, Any]]

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
    def from_wire(cls, obj: Mapping[str, Any]) -> "ProvenanceAttestation":
        """Build one from a decoded JSON object, ignoring unknown members."""
        return cls(
            signed_commitment=obj["signed_commitment"],
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
            "signed_commitment": self.signed_commitment,
            "key_id": self.key_id,
            "algorithm": self.algorithm,
            "attester_id": self.attester_id,
            "signature": self.signature,
            "issued_at": self.issued_at,
        }

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

    def is_well_shaped(self) -> bool:
        """Whether this proof has exactly the shape RFC 6962 gives its
        ``(leaf_index, leaf_count)`` (``SPEC.md`` §6.5.3, ADR 0031).

        The path must be exactly as long as the one
        :func:`inclusion_path_sides` computes, with every step's
        ``sibling_is_left`` equal to it. Mirrors
        ``contextgraph_types::InclusionProof::is_well_shaped``.

        This is what makes ``leaf_count`` mean something. Without it a genuine
        path could be presented under a false ``leaf_index`` or ``leaf_count``
        and still recompute the signed root, because the walk never reads
        either field. The check hashes nothing and is bounded: a path longer
        than :data:`MAX_INCLUSION_PATH_STEPS` is refused on its length before
        the shape is computed.
        """
        path = self.path
        if len(path) > MAX_INCLUSION_PATH_STEPS:
            return False
        sides = inclusion_path_sides(self.leaf_index, self.leaf_count)
        if sides is None or len(sides) != len(path):
            return False
        return all(
            step.sibling_is_left is left for left, step in zip(sides, path)
        )


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
        the same answer. That includes :data:`Verdict.VALID_IDENTITY_ONLY`,
        whose signature verifies over a preimage that says nothing about the
        bytes in hand (ADR 0018).
        """
        return self.verdict == Verdict.VALID

    def signature_verifies(self) -> bool:
        """Whether the signature itself checked out, whatever it covers.

        True for :data:`Verdict.VALID` and :data:`Verdict.VALID_IDENTITY_ONLY`.
        The question to ask on purpose when a caller wants provider identity
        and provenance without a claim about content; a separate method rather
        than a looser :meth:`is_valid` so the choice is legible at the call
        site. Mirrors the Rust reference's ``signature_verifies``.
        """
        return self.verdict in (Verdict.VALID, Verdict.VALID_IDENTITY_ONLY)

    def binds_content(self) -> bool:
        """Whether the verified commitment binds the frame's content bytes.

        Only :data:`Verdict.VALID` does; a verdict that did not verify at all
        binds nothing. Mirrors the Rust reference's ``binds_content``.
        """
        return self.verdict == Verdict.VALID


class Verdict:
    """The named outcomes :class:`AttestationVerdict` can carry."""

    VALID = "valid"
    #: The signature verifies, but over a frame that declares no
    #: ``content_digest``, so it binds the frame's identity and provenance and
    #: none of its bytes (``SPEC.md`` §6.5.2, ADR 0018).
    #: :meth:`AttestationVerdict.is_valid` is false for it;
    #: :meth:`AttestationVerdict.signature_verifies` is true.
    VALID_IDENTITY_ONLY = "valid_identity_only"
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


def _link_field(link: LinkLike, name: str) -> Optional[str]:
    """Read one optional field from a link.

    A missing key and an explicit ``None`` are both absent; ``""`` is present.
    """
    value = link.get(name)
    return None if value is None else str(value)


def encode_provenance_link(link: LinkLike) -> bytes:
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


def _from_strict_hex(text: str) -> Optional[bytes]:
    """Decode lowercase hex, or ``None``.

    Strict on purpose. ``bytes.fromhex`` accepts uppercase *and* skips ASCII
    whitespace between byte pairs, so ``"AB CD"`` would parse — and the
    protocol's grammar is 64 lowercase hex characters
    (``contextgraph_types::is_well_formed_digest``). One spelling per value is
    what keeps two implementations from disagreeing about whether a given
    attestation is well-formed.
    """
    if len(text) % 2 != 0:
        return None
    if any(c not in "0123456789abcdef" for c in text):
        return None
    return bytes.fromhex(text)


def parse_digest(digest: str) -> Optional[bytes]:
    """Parse a ``sha256:<hex>`` digest string. ``None`` if malformed."""
    prefix = "sha256:"
    if not digest.startswith(prefix):
        return None
    hex_part = digest[len(prefix) :]
    if len(hex_part) != 64:
        return None
    return _from_strict_hex(hex_part)


# ---------------------------------------------------------------------------
# Chain head, frame commitment, Merkle tree (``SPEC.md`` §6.5.2–§6.5.3)
# ---------------------------------------------------------------------------


def provenance_chain_head(links: Iterable[LinkLike] = ()) -> bytes:
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


def frame_commitment(provider_id: str, frame: FrameLike) -> bytes:
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
    """The largest power of two strictly less than ``n`` (RFC 6962's split),
    for ``n >= 2``. Bit arithmetic rather than a doubling loop, so its cost
    does not grow with a provider-supplied ``leaf_count``."""
    return 1 << ((n - 1).bit_length() - 1)


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
    commitments: Sequence[bytes], index: int, path: list[InclusionStep]
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


#: The longest inclusion path a verifier walks (``SPEC.md`` §6.5.3).
#:
#: Matches ``contextgraph_types::MAX_INCLUSION_PATH_STEPS``: 64 steps covers
#: any tree a 64-bit ``leaf_count`` can describe. The cap is about work, not
#: correctness — each step costs a hash and the path arrives from the provider.
MAX_INCLUSION_PATH_STEPS = 64

#: The largest ``leaf_count`` a proof may state. The Rust reference carries it
#: as a ``usize``; a Python ``int`` is unbounded, so the bound is explicit here
#: to keep the shape walk at most :data:`MAX_INCLUSION_PATH_STEPS` iterations
#: and to refuse exactly what the reference cannot represent.
_MAX_LEAF_COUNT = (1 << 64) - 1


def _is_proof_int(value: object) -> bool:
    """A proof index or count: a non-negative ``int`` a ``u64`` can hold.

    ``bool`` is an ``int`` subclass in Python and is refused, as serde refuses
    ``true`` for a ``usize``.
    """
    return (
        isinstance(value, int)
        and not isinstance(value, bool)
        and 0 <= value <= _MAX_LEAF_COUNT
    )


def inclusion_path_sides(leaf_index: int, leaf_count: int) -> Optional[List[bool]]:
    """The RFC 6962 inclusion-path shape for leaf ``leaf_index`` of a tree of
    ``leaf_count`` leaves (``SPEC.md`` §6.5.3).

    One entry per step, **leaf upward**, each ``True`` when that step's
    sibling is the left operand — exactly the ``sibling_is_left`` sequence
    :func:`inclusion_proof` emits. Mirrors
    ``contextgraph_types::inclusion_path_sides``.

    ``None`` when ``leaf_index >= leaf_count`` (including an empty tree, which
    has no leaves to prove), when either is negative or not an ``int``, or when
    ``leaf_count`` exceeds what a 64-bit count can hold. The walk descends the
    tree's split points from the root, so it takes at most 64 steps, and it
    hashes nothing.

    >>> inclusion_path_sides(0, 1)
    []
    >>> inclusion_path_sides(3, 7)
    [True, True, False]
    >>> inclusion_path_sides(6, 7)
    [True, True]
    >>> inclusion_path_sides(7, 7) is None
    True
    """
    if not (_is_proof_int(leaf_index) and _is_proof_int(leaf_count)):
        return None
    if leaf_index >= leaf_count:
        return None
    sides: List[bool] = []
    index, count = leaf_index, leaf_count
    while count > 1:
        split = _split_point(count)
        if index < split:
            # In the left subtree: the sibling is the right one.
            sides.append(False)
            count = split
        else:
            sides.append(True)
            index -= split
            count -= split
    # Collected root-downward; a proof lists its steps leaf-upward.
    sides.reverse()
    return sides


def root_from_proof(commitment: bytes, proof: InclusionProof) -> Optional[bytes]:
    """Recompute a Merkle root from a leaf commitment and its proof.

    The whole offline story: an auditor holding one frame, its proof and a
    signed root needs nothing else.

    ``None`` if any sibling is malformed, or if the proof is not
    :meth:`well-shaped <InclusionProof.is_well_shaped>` for the tree it
    states: an index outside ``leaf_count``, or a path whose length or sides
    are not the ones RFC 6962 gives that ``(leaf_index, leaf_count)``. That
    check runs first and hashes nothing, and it is what makes ``leaf_count``
    mean something — a verifier that ignored it could be shown a proof from a
    differently-shaped tree (``SPEC.md`` §6.5.3, ADR 0031).
    """
    if not proof.is_well_shaped():
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
    signature = _from_strict_hex(attestation.signature)
    if signature is None or len(signature) != 64:
        return AttestationVerdict(Verdict.MALFORMED_SIGNATURE)

    ok = _ed25519.verify(public_key, signed, signature)
    return AttestationVerdict(Verdict.VALID if ok else Verdict.BAD_SIGNATURE)


def verify_frame_attestation(
    provider_id: str,
    frame: FrameLike,
    attestation: ProvenanceAttestation,
    public_key: bytes,
) -> AttestationVerdict:
    """Verify a detached attestation over a single frame (``SPEC.md`` §6.5.4).

    A frame that declares no ``content_digest`` was committed to by id and
    provenance alone, so a signature that checks out over it says nothing
    about the bytes. That case is reported as
    :data:`Verdict.VALID_IDENTITY_ONLY` rather than :data:`Verdict.VALID`
    (ADR 0018), exactly as the Rust reference does; every failing verdict is
    left as it is, because it is already the more specific answer.
    """
    verdict = verify_commitment(
        frame_commitment(provider_id, frame), attestation, public_key
    )
    if verdict.verdict == Verdict.VALID and _frame_content_digest(frame) is None:
        return AttestationVerdict(Verdict.VALID_IDENTITY_ONLY)
    return verdict


def verify_frame_inclusion(
    provider_id: str,
    frame: FrameLike,
    proof: InclusionProof,
    result_attestation: ProvenanceAttestation,
    public_key: bytes,
) -> AttestationVerdict:
    """Verify that a frame was a leaf of a signed result-set root
    (``SPEC.md`` §6.5.3, F13). Mirrors
    ``contextgraph_types::attest::verify_frame_inclusion``.

    The other half of §6.5: a provider that signs one Merkle root and ships a
    per-frame :class:`InclusionProof` has attested every frame with a single
    signature. This recomputes the root from the frame's own
    :func:`frame_commitment` and its proof, then checks ``result_attestation``
    over that root. The content-binding rule is
    :func:`verify_frame_attestation`'s (ADR 0018): the leaf is a frame
    commitment, so a frame with no ``content_digest`` is
    :data:`Verdict.VALID_IDENTITY_ONLY` however many hashes sit above it.

    A proof that is not :meth:`well-shaped <InclusionProof.is_well_shaped>` —
    a path longer than :data:`MAX_INCLUSION_PATH_STEPS`, a leaf index outside
    the stated tree, or a path whose length or sides disagree with its
    ``(leaf_index, leaf_count)`` — or one with a malformed sibling is
    :data:`Verdict.MALFORMED_COMMITMENT`: there is no root to compare against.
    The length and shape are decided before anything is hashed, the frame
    commitment included (ADR 0031).

    A host checking an answer as it arrived also compares ``proof.leaf_count``
    with the number of frames the answer carries (``SPEC.md`` §6.5.3): the
    shape cannot tell every tree size apart, and that comparison needs the
    whole answer, which this function never sees.
    """
    if not proof.is_well_shaped():
        return AttestationVerdict(Verdict.MALFORMED_COMMITMENT)
    root = root_from_proof(frame_commitment(provider_id, frame), proof)
    if root is None:
        return AttestationVerdict(Verdict.MALFORMED_COMMITMENT)
    verdict = verify_commitment(root, result_attestation, public_key)
    if verdict.verdict == Verdict.VALID and _frame_content_digest(frame) is None:
        return AttestationVerdict(Verdict.VALID_IDENTITY_ONLY)
    return verdict


def _frame_content_digest(frame: FrameLike) -> Any:
    """A frame's declared ``content_digest``, or ``None`` when it declares none.

    ``Any`` because a mapping frame is decoded JSON and may hold anything;
    callers decide what a non-string means for them.
    """
    if isinstance(frame, AttestableFrame):
        return frame.content_digest
    return frame.get("content_digest")


# ---------------------------------------------------------------------------
# Signing — optional, needs the ``cryptography`` backend (``[signing]`` extra)
# ---------------------------------------------------------------------------


def public_key_for(signing_key_seed: bytes) -> bytes:
    """The raw 32-byte public key matching a signing seed — the form
    :func:`verify_frame_attestation` accepts.

    Needs the optional backend: deriving a public key multiplies by the secret
    scalar, so it is signing's risk and goes where signing goes.

    :raises SigningUnavailableError: ``cryptography`` is not installed.
    :raises ValueError: the seed is not 32 bytes.
    """
    return _signing.public_key(signing_key_seed)


def sign_commitment(
    commitment: bytes,
    signing_key_seed: bytes,
    key_id: str,
    attester_id: str,
    issued_at: str,
) -> ProvenanceAttestation:
    """Sign an arbitrary commitment (a frame commitment or a Merkle root)
    in-process, for providers content to hold key material in memory.

    Mirrors ``contextgraph_types::attest::sign_commitment``: Ed25519 over the
    commitment's 32 raw bytes, the signature rendered as lowercase hex. Ed25519
    is deterministic, so the same seed and commitment always produce the same
    attestation — which is how ``tests/test_signing.py`` pins this against the
    published vector byte for byte.

    A provider whose key lives in an HSM or KMS does not call this: it signs
    the same 32 bytes with its own backend and builds the
    :class:`ProvenanceAttestation` itself. The protocol specifies the preimage,
    never the custody of the key.

    :raises SigningUnavailableError: ``cryptography`` is not installed.
    :raises ValueError: the commitment or the seed is not 32 bytes.
    """
    if len(commitment) != 32:
        raise ValueError(
            f"a commitment is 32 raw bytes, got {len(commitment)}; pass "
            "frame_commitment(...) or merkle_root(...), not a digest string"
        )
    signature = _signing.sign(signing_key_seed, bytes(commitment))
    return ProvenanceAttestation(
        signed_commitment=digest_string(bytes(commitment)),
        key_id=key_id,
        algorithm=ALGORITHM_ED25519,
        attester_id=attester_id,
        signature=signature.hex(),
        issued_at=issued_at,
    )


def sign_frame_attestation(
    provider_id: str,
    frame: FrameLike,
    signing_key_seed: bytes,
    key_id: str,
    attester_id: str,
    issued_at: str,
) -> ProvenanceAttestation:
    """Sign one frame's :func:`frame_commitment` in-process.

    **Refuses a frame that declares no ``content_digest``.** ``SPEC.md`` §6.5.2
    requires an attester to populate it on any frame it signs (ADR 0018): a
    digest-less commitment binds the frame's identity and provenance but none
    of its content, so the provider could re-serve entirely different bytes
    under the same id with the signature still checking out. The reference
    verifier reports such an attestation as identity-only rather than valid;
    this signer declines to produce one. A ``content_digest`` that is present
    but not ``sha256:<64 lowercase hex>`` is refused too: such a frame already
    fails ``SPEC.md`` D1, and a signature over it binds a string rather than
    the bytes it claims to name. :func:`sign_commitment` remains for a caller
    with a reason to sign an arbitrary commitment.

    :raises ValueError: the frame declares no well-formed ``content_digest``,
        or the seed is not 32 bytes.
    :raises SigningUnavailableError: ``cryptography`` is not installed.
    """
    content_digest = _frame_content_digest(frame)
    if content_digest is None:
        raise ValueError(
            "refusing to sign a frame that declares no content_digest: the "
            "signature would cover its identity but none of its content "
            "(SPEC.md §6.5.2, ADR 0018)"
        )
    if not isinstance(content_digest, str) or parse_digest(content_digest) is None:
        raise ValueError(
            "refusing to sign a frame whose content_digest is not "
            f"sha256:<64 lowercase hex> (SPEC.md D1): {content_digest!r}"
        )
    return sign_commitment(
        frame_commitment(provider_id, frame),
        signing_key_seed,
        key_id,
        attester_id,
        issued_at,
    )

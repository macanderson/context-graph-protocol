"""The optional signing path, pinned to the published vectors byte for byte.

Two published seeds, two published signatures:

- ``tests/vectors/attestation-vectors.json`` — ``signature.signing_key_seed_hex``
  signs the frame commitment ``signature.attestation.signed_commitment``;
- ``tests/fixtures/record-attestation-key.json`` — ``signing_key_seed`` signs
  ``observation.json``'s ``record_hash`` as ``record-attestation.json``.

Ed25519 is deterministic (RFC 8032 §5.1.6), so signing the published input with
the published seed must reproduce the published signature exactly. A signer
checked only by round-tripping through its own verifier would pass while
disagreeing with every other implementation; this one is compared to bytes
``ed25519-dalek`` produced.

Signing needs the optional ``cryptography`` backend. Where it is absent these
tests skip, **unless** ``CONTEXTGRAPH_REQUIRE_SIGNING=1`` is set — which the
``sdk (python)`` CI job does after installing ``contextgraph-sdk[signing]``, so
a skip there is a failure rather than a silent pass. The absent-backend tests
at the bottom run everywhere: they hide the backend and check that every entry
point raises :class:`SigningUnavailableError` instead of answering differently.
"""

from __future__ import annotations

import json
import os
import unittest
from pathlib import Path
from typing import Any
from unittest import mock

from contextgraph_sdk.attest import (
    AttestableFrame,
    ProvenanceAttestation,
    SigningUnavailableError,
    Verdict,
    frame_commitment,
    merkle_root,
    parse_digest,
    public_key_for,
    sign_commitment,
    sign_frame_attestation,
    verify_commitment,
    verify_frame_attestation,
)
from contextgraph_sdk.record import (
    RecordAttestation,
    record_hash,
    sign_record,
    sign_record_attestation,
    verify_record_attestation,
)


def _find(relative: str) -> Path:
    """Walk up from this file until ``relative`` appears."""
    here = Path(__file__).resolve()
    for parent in [here] + list(here.parents):
        candidate = parent / relative
        if candidate.is_file():
            return candidate
    raise AssertionError(f"{relative} not found above {__file__}")


def _load(relative: str) -> Any:
    return json.loads(_find(relative).read_text(encoding="utf-8"))


V = _load("tests/vectors/attestation-vectors.json")
RECORD_ATTESTATION = _load("tests/fixtures/record-attestation.json")
RECORD_KEY = _load("tests/fixtures/record-attestation-key.json")
SIGNED_RECORD = _load("tests/fixtures/" + RECORD_KEY["signs"])

FRAME_SEED = bytes.fromhex(V["signature"]["signing_key_seed_hex"])
RECORD_SEED = bytes.fromhex(RECORD_KEY["signing_key_seed"])

#: Set by CI after installing the ``[signing]`` extra, so the pinned tests
#: cannot quietly skip on the one runner that is meant to prove them.
REQUIRE_SIGNING = os.environ.get("CONTEXTGRAPH_REQUIRE_SIGNING") == "1"

#: The backend module ``contextgraph_sdk._signing`` imports. Mapping it to
#: ``None`` in ``sys.modules`` makes any import of it raise ImportError, which
#: is how the absent-backend tests hide an installed ``cryptography``.
_BACKEND_MODULE = "cryptography.hazmat.primitives.asymmetric.ed25519"


def _backend_available() -> bool:
    try:
        import cryptography.hazmat.primitives.asymmetric.ed25519  # noqa: F401
    except ImportError:
        return False
    return True


def published_frame() -> AttestableFrame:
    spec = V["frame_commitment"]
    return AttestableFrame(
        id=spec["frame"]["id"],
        content_digest=spec["frame"]["content_digest"],
        provenance=[V["links"][name] for name in spec["frame"]["provenance"]],
    )


class _NeedsBackend(unittest.TestCase):
    def setUp(self) -> None:
        if _backend_available():
            return
        if REQUIRE_SIGNING:
            self.fail(
                "CONTEXTGRAPH_REQUIRE_SIGNING=1 but 'cryptography' is not "
                'installed: pip install "contextgraph-sdk[signing]"'
            )
        self.skipTest(
            "the optional signing backend is not installed "
            '(pip install "contextgraph-sdk[signing]"); the pinned signing '
            "vectors run in CI's sdk (python) job"
        )


class FrameSigningVectors(_NeedsBackend):
    def test_the_published_seed_derives_the_published_public_key(self) -> None:
        self.assertEqual(public_key_for(FRAME_SEED).hex(), V["signature"]["public_key_hex"])

    def test_signing_the_published_commitment_reproduces_the_published_signature(
        self,
    ) -> None:
        wire = V["signature"]["attestation"]
        commitment = parse_digest(wire["signed_commitment"])
        assert commitment is not None
        signed = sign_commitment(
            commitment,
            FRAME_SEED,
            key_id=wire["key_id"],
            attester_id=wire["attester_id"],
            issued_at=wire["issued_at"],
        )
        # Byte for byte: the signature, and the whole attestation around it.
        self.assertEqual(signed.signature, wire["signature"])
        self.assertEqual(signed.to_wire(), wire)
        self.assertEqual(signed, ProvenanceAttestation.from_wire(wire))

    def test_signing_the_published_frame_reproduces_the_published_attestation(
        self,
    ) -> None:
        wire = V["signature"]["attestation"]
        signed = sign_frame_attestation(
            V["frame_commitment"]["provider_id"],
            published_frame(),
            FRAME_SEED,
            wire["key_id"],
            wire["attester_id"],
            wire["issued_at"],
        )
        self.assertEqual(signed.to_wire(), wire)
        verdict = verify_frame_attestation(
            V["frame_commitment"]["provider_id"],
            published_frame(),
            signed,
            public_key_for(FRAME_SEED),
        )
        self.assertEqual(verdict.verdict, Verdict.VALID)
        self.assertTrue(verdict.binds_content())

    def test_a_digest_less_frame_verifies_as_identity_only(self) -> None:
        # ADR 0018's verifier half. sign_frame_attestation refuses this frame,
        # so the non-conformant attester is played by sign_commitment over its
        # commitment. The signature is genuine; it binds no bytes, and the
        # verdict has to say so rather than report VALID.
        provider_id = V["frame_commitment"]["provider_id"]
        digest_less = AttestableFrame(
            id=V["frame_commitment"]["frame"]["id"],
            provenance=published_frame().provenance,
        )
        signed = sign_commitment(
            frame_commitment(provider_id, digest_less),
            FRAME_SEED,
            "key-1",
            "oxagen",
            "2026-08-27T00:00:00Z",
        )
        as_mapping = {"id": digest_less.id, "provenance": list(digest_less.provenance)}
        for frame in (digest_less, as_mapping):
            with self.subTest(frame=type(frame).__name__):
                verdict = verify_frame_attestation(
                    provider_id, frame, signed, public_key_for(FRAME_SEED)
                )
                self.assertEqual(verdict.verdict, Verdict.VALID_IDENTITY_ONLY)
                self.assertFalse(verdict.is_valid())
                self.assertTrue(verdict.signature_verifies())
                self.assertFalse(verdict.binds_content())

    def test_a_merkle_root_signs_and_verifies_like_any_commitment(self) -> None:
        root = merkle_root(
            [
                frame_commitment(V["merkle"]["provider_id"], frame)
                for frame in V["merkle"]["leaf_frames"][:3]
            ]
        )
        self.assertEqual(
            "sha256:" + root.hex(), V["merkle"]["roots_by_leaf_count"]["3"]
        )
        signed = sign_commitment(root, FRAME_SEED, "key-1", "oxagen", "2026-08-27T00:00:00Z")
        self.assertEqual(
            verify_commitment(root, signed, public_key_for(FRAME_SEED)).verdict,
            Verdict.VALID,
        )


class RecordSigningVectors(_NeedsBackend):
    def test_the_published_record_seed_derives_the_published_public_key(self) -> None:
        self.assertEqual(public_key_for(RECORD_SEED).hex(), RECORD_KEY["public_key"])

    def test_signing_the_published_record_reproduces_the_published_attestation(
        self,
    ) -> None:
        signed = sign_record(
            SIGNED_RECORD,
            RECORD_SEED,
            RECORD_ATTESTATION["key_id"],
            RECORD_ATTESTATION["attester_id"],
            RECORD_ATTESTATION["issued_at"],
        )
        self.assertEqual(signed.to_wire(), RECORD_ATTESTATION)
        self.assertEqual(signed, RecordAttestation.from_wire(RECORD_ATTESTATION))

    def test_signing_the_published_hash_reproduces_the_published_signature(
        self,
    ) -> None:
        signed = sign_record_attestation(
            RECORD_ATTESTATION["signed_record_hash"],
            RECORD_SEED,
            RECORD_ATTESTATION["key_id"],
            RECORD_ATTESTATION["attester_id"],
            RECORD_ATTESTATION["issued_at"],
        )
        self.assertEqual(signed.signature, RECORD_ATTESTATION["signature"])

    def test_a_frame_signature_over_the_same_digest_is_not_a_record_attestation(
        self,
    ) -> None:
        # What the domain tag buys: sign the record's hash bytes through the
        # frame layer, relabel the result as a record attestation, and it must
        # not verify.
        digest = record_hash(SIGNED_RECORD)
        raw = parse_digest(digest)
        assert raw is not None
        frame_signed = sign_commitment(raw, RECORD_SEED, "key-1", "oxagen", "2026-07-29T14:00:05Z")
        lifted = RecordAttestation(
            signed_record_hash=digest,
            key_id=frame_signed.key_id,
            algorithm=frame_signed.algorithm,
            attester_id=frame_signed.attester_id,
            signature=frame_signed.signature,
            issued_at=frame_signed.issued_at,
        )
        self.assertEqual(
            verify_record_attestation(
                SIGNED_RECORD, lifted, bytes.fromhex(RECORD_KEY["public_key"])
            ).verdict,
            Verdict.BAD_SIGNATURE,
        )


class SigningRefusals(unittest.TestCase):
    """Refusals that come before any backend, so they hold on every machine."""

    def test_a_frame_with_no_content_digest_is_not_signed(self) -> None:
        # SPEC.md §6.5.2 / ADR 0018: the signature would bind the frame's
        # identity and none of its content.
        for frame in (
            AttestableFrame(id="retry-policy"),
            {"id": "retry-policy", "provenance": []},
        ):
            with self.subTest(frame=type(frame).__name__):
                with self.assertRaises(ValueError) as refused:
                    sign_frame_attestation(
                        "repo-graph", frame, FRAME_SEED, "k", "a", "2026-08-27T00:00:00Z"
                    )
                self.assertNotIsInstance(refused.exception, SigningUnavailableError)
                self.assertIn("content_digest", str(refused.exception))

    def test_a_frame_with_a_malformed_content_digest_is_not_signed(self) -> None:
        # SPEC.md D1: a present digest is sha256:<64 lowercase hex>. Anything
        # else is a string, not a binding to bytes, and is refused before the
        # backend is consulted.
        malformed = ("", "sha256:short", "SHA256:" + "ab" * 32, "sha256:" + "AB" * 32, 42)
        for digest in malformed:
            with self.subTest(digest=digest):
                with self.assertRaises(ValueError) as refused:
                    sign_frame_attestation(
                        "repo-graph",
                        {"id": "retry-policy", "content_digest": digest, "provenance": []},
                        FRAME_SEED,
                        "k",
                        "a",
                        "2026-08-27T00:00:00Z",
                    )
                self.assertNotIsInstance(refused.exception, SigningUnavailableError)
                self.assertIn("content_digest", str(refused.exception))

    def test_a_malformed_seed_or_commitment_is_a_value_error_everywhere(self) -> None:
        commitment = bytes(32)
        with self.assertRaises(ValueError):
            sign_commitment(commitment, bytes(31), "k", "a", "t")
        with self.assertRaises(ValueError):
            sign_commitment(b"sha256:not-raw-bytes", FRAME_SEED, "k", "a", "t")
        with self.assertRaises(ValueError):
            sign_record_attestation("sha256:short", RECORD_SEED, "k", "a", "t")


class AbsentBackend(unittest.TestCase):
    """With ``cryptography`` hidden, every signer names what is missing."""

    def test_every_signing_entry_point_raises_a_named_error(self) -> None:
        commitment = bytes(32)
        entry_points = {
            "public_key_for": lambda: public_key_for(FRAME_SEED),
            "sign_commitment": lambda: sign_commitment(
                commitment, FRAME_SEED, "k", "a", "t"
            ),
            "sign_frame_attestation": lambda: sign_frame_attestation(
                "repo-graph", published_frame(), FRAME_SEED, "k", "a", "t"
            ),
            "sign_record": lambda: sign_record(SIGNED_RECORD, RECORD_SEED, "k", "a", "t"),
            "sign_record_attestation": lambda: sign_record_attestation(
                RECORD_ATTESTATION["signed_record_hash"], RECORD_SEED, "k", "a", "t"
            ),
        }
        with mock.patch.dict("sys.modules", {_BACKEND_MODULE: None}):
            for name, call in entry_points.items():
                with self.subTest(entry_point=name):
                    with self.assertRaises(SigningUnavailableError) as missing:
                        call()
                    # The error says how to fix it, in the words pyproject uses.
                    self.assertIn("contextgraph-sdk[signing]", str(missing.exception))

    def test_verification_never_needs_the_backend(self) -> None:
        # The zero-dependency half keeps working with the backend hidden.
        wire = V["signature"]["attestation"]
        commitment = parse_digest(wire["signed_commitment"])
        assert commitment is not None
        with mock.patch.dict("sys.modules", {_BACKEND_MODULE: None}):
            self.assertEqual(
                verify_commitment(
                    commitment,
                    ProvenanceAttestation.from_wire(wire),
                    bytes.fromhex(V["signature"]["public_key_hex"]),
                ).verdict,
                Verdict.VALID,
            )
            self.assertEqual(
                verify_record_attestation(
                    SIGNED_RECORD,
                    RecordAttestation.from_wire(RECORD_ATTESTATION),
                    bytes.fromhex(RECORD_KEY["public_key"]),
                ).verdict,
                Verdict.VALID,
            )


if __name__ == "__main__":  # pragma: no cover
    unittest.main()

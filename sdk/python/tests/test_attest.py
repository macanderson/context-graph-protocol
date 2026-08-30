"""The Python attestation port, reconciled against the published vectors.

Every expected value is read from ``tests/vectors/attestation-vectors.json``,
which ``contextgraph-types/tests/attestation_vectors.rs`` mirrors and pins.
Nothing here asserts against a value this file computed: a port that agrees
with itself is what this suite exists to catch.

Written against :mod:`unittest` rather than pytest so it runs on a bare
``python3`` with nothing installed, which is the same promise the SDK makes to
its users. ``python3 -m pytest`` collects it too.
"""

from __future__ import annotations

import json
import unittest
from pathlib import Path

from contextgraph_sdk import _ed25519
from contextgraph_sdk.attest import (
    ALGORITHM_ED25519,
    AttestableFrame,
    ProvenanceAttestation,
    Verdict,
    digest_string,
    encode_provenance_link,
    frame_commitment,
    inclusion_proof,
    merkle_root,
    parse_digest,
    provenance_chain_head,
    root_from_proof,
    verify_commitment,
    verify_frame_attestation,
)


def _load_vectors() -> dict:
    """Walk up from this file until the shared fixture appears.

    A relative depth would be right for exactly one of "run from the repo
    root" and "run from sdk/python", and both happen.
    """
    for parent in [Path(__file__).resolve()] + list(Path(__file__).resolve().parents):
        candidate = parent / "tests" / "vectors" / "attestation-vectors.json"
        if candidate.is_file():
            return json.loads(candidate.read_text(encoding="utf-8"))
    raise AssertionError(
        "tests/vectors/attestation-vectors.json not found above " + __file__
    )


V = _load_vectors()


def link(name: str) -> dict:
    return V["links"][name]


def merkle_leaves(count: int) -> list:
    provider_id = V["merkle"]["provider_id"]
    return [
        frame_commitment(provider_id, frame)
        for frame in V["merkle"]["leaf_frames"][:count]
    ]


def attestation() -> ProvenanceAttestation:
    return ProvenanceAttestation.from_wire(V["signature"]["attestation"])


def public_key() -> bytes:
    return bytes.fromhex(V["signature"]["public_key_hex"])


def signed_commitment() -> bytes:
    parsed = parse_digest(V["signature"]["attestation"]["signed_commitment"])
    assert parsed is not None
    return parsed


class EncodingVectors(unittest.TestCase):
    def test_link_encoding_matches_the_published_bytes(self) -> None:
        self.assertEqual(
            encode_provenance_link(link("ascii_minimal")).hex(),
            V["link_encodings_hex"]["ascii_minimal"],
        )
        self.assertEqual(
            encode_provenance_link(link("unicode")).hex(),
            V["link_encodings_hex"]["unicode"],
        )

    def test_the_length_prefix_counts_utf8_bytes_not_len(self) -> None:
        # The trap, demonstrated in this runtime rather than asserted about it:
        # if these numbers were equal, the vector above could not tell a
        # correct port from one using len().
        trap = V["unicode_length_trap"]
        uri = link("unicode")["uri"]
        by = link("unicode")["by"]
        self.assertEqual(len(uri.encode("utf-8")), trap["uri"]["utf8_bytes"])
        self.assertEqual(len(uri), trap["uri"]["code_points"])
        self.assertEqual(len(by.encode("utf-8")), trap["by"]["utf8_bytes"])
        self.assertEqual(len(by), trap["by"]["code_points"])
        self.assertNotEqual(
            trap["uri"]["utf8_bytes"],
            trap["uri"]["code_points"],
            "the multi-byte characters must make the two answers differ, "
            "or this proves nothing",
        )

    def test_an_absent_field_never_encodes_like_an_empty_one(self) -> None:
        self.assertNotEqual(
            encode_provenance_link({"type": "file"}),
            encode_provenance_link({"type": "file", "uri": ""}),
        )

    def test_an_explicit_none_is_absent(self) -> None:
        self.assertEqual(
            encode_provenance_link({"type": "file", "uri": None}),
            encode_provenance_link({"type": "file"}),
        )


class ChainVectors(unittest.TestCase):
    def test_chain_heads_match_the_published_vectors(self) -> None:
        heads = V["chain_heads"]
        self.assertEqual(digest_string(provenance_chain_head([])), heads["empty"])
        self.assertEqual(
            digest_string(provenance_chain_head([link("file")])), heads["file"]
        )
        self.assertEqual(
            digest_string(provenance_chain_head([link("file"), link("derivation")])),
            heads["file_then_derivation"],
        )
        self.assertEqual(
            digest_string(provenance_chain_head([link("unicode")])), heads["unicode"]
        )

    def test_reordering_the_chain_changes_the_head(self) -> None:
        self.assertNotEqual(
            provenance_chain_head([link("file"), link("derivation")]),
            provenance_chain_head([link("derivation"), link("file")]),
        )

    def test_frame_commitment_matches_the_published_vector(self) -> None:
        spec = V["frame_commitment"]
        frame = AttestableFrame(
            id=spec["frame"]["id"],
            content_digest=spec["frame"]["content_digest"],
            provenance=[link(name) for name in spec["frame"]["provenance"]],
        )
        self.assertEqual(
            digest_string(frame_commitment(spec["provider_id"], frame)),
            spec["commitment"],
        )

    def test_a_plain_dict_frame_commits_identically(self) -> None:
        spec = V["frame_commitment"]
        as_dict = {
            "id": spec["frame"]["id"],
            "content_digest": spec["frame"]["content_digest"],
            "provenance": [link(name) for name in spec["frame"]["provenance"]],
        }
        self.assertEqual(
            digest_string(frame_commitment(spec["provider_id"], as_dict)),
            spec["commitment"],
        )


class MerkleVectors(unittest.TestCase):
    def test_roots_match_the_published_vectors_odd_counts_included(self) -> None:
        for count, root in V["merkle"]["roots_by_leaf_count"].items():
            with self.subTest(leaves=count):
                self.assertEqual(
                    digest_string(merkle_root(merkle_leaves(int(count)))), root
                )

    def test_inclusion_proof_matches_and_recomputes_the_root(self) -> None:
        spec = V["merkle"]["inclusion_proof"]
        leaves = merkle_leaves(spec["leaf_count"])
        proof = inclusion_proof(leaves, spec["leaf_index"])
        assert proof is not None
        self.assertEqual(proof.leaf_count, spec["leaf_count"])
        self.assertEqual(proof.leaf_index, spec["leaf_index"])
        self.assertEqual(
            [
                {"sibling": s.sibling, "sibling_is_left": s.sibling_is_left}
                for s in proof.path
            ],
            spec["path"],
        )
        recomputed = root_from_proof(leaves[spec["leaf_index"]], proof)
        assert recomputed is not None
        self.assertEqual(
            digest_string(recomputed),
            V["merkle"]["roots_by_leaf_count"][str(spec["leaf_count"])],
        )

    def test_a_proof_does_not_validate_an_outsider(self) -> None:
        leaves = merkle_leaves(7)
        proof = inclusion_proof(leaves, 3)
        assert proof is not None
        outsider = frame_commitment("repo-graph", {"id": "intruder"})
        recomputed = root_from_proof(outsider, proof)
        assert recomputed is not None
        self.assertNotEqual(
            digest_string(recomputed), V["merkle"]["roots_by_leaf_count"]["7"]
        )

    def test_out_of_range_index_has_no_proof(self) -> None:
        self.assertIsNone(inclusion_proof(merkle_leaves(3), 3))
        self.assertIsNone(inclusion_proof([], 0))


class SignatureVectors(unittest.TestCase):
    def test_the_published_signature_verifies(self) -> None:
        verdict = verify_commitment(signed_commitment(), attestation(), public_key())
        self.assertEqual(verdict.verdict, Verdict.VALID)
        self.assertTrue(verdict.is_valid())
        self.assertEqual(attestation().algorithm, ALGORITHM_ED25519)
        self.assertTrue(attestation().uses_known_algorithm())

    def test_perturbing_the_commitment_is_a_mismatch_not_a_bad_signature(self) -> None:
        tampered = bytearray(signed_commitment())
        tampered[0] ^= 0x01
        verdict = verify_commitment(bytes(tampered), attestation(), public_key())
        # §6.5.4's ordering rule: the frame changed after signing, and saying
        # "bad signature" would send an operator after a key-management bug.
        self.assertEqual(verdict.verdict, Verdict.COMMITMENT_MISMATCH)
        self.assertFalse(verdict.is_valid())
        self.assertEqual(
            verdict.signed, V["signature"]["attestation"]["signed_commitment"]
        )

    def test_perturbing_the_signature_is_a_bad_signature(self) -> None:
        raw = bytearray(bytes.fromhex(V["signature"]["attestation"]["signature"]))
        raw[0] ^= 0x01
        forged = ProvenanceAttestation.from_wire(
            {**V["signature"]["attestation"], "signature": bytes(raw).hex()}
        )
        self.assertEqual(
            verify_commitment(signed_commitment(), forged, public_key()).verdict,
            Verdict.BAD_SIGNATURE,
        )

    def test_perturbing_the_frame_is_caught_through_verify_frame_attestation(
        self,
    ) -> None:
        spec = V["frame_commitment"]
        provenance = [link(name) for name in spec["frame"]["provenance"]]
        honest = AttestableFrame(
            id=spec["frame"]["id"],
            content_digest=spec["frame"]["content_digest"],
            provenance=provenance,
        )
        self.assertEqual(
            verify_frame_attestation(
                spec["provider_id"], honest, attestation(), public_key()
            ).verdict,
            Verdict.VALID,
            "precondition: the published attestation signs this exact frame",
        )
        # The tamper a bare digest cannot see, because the tamperer rewrites
        # the digest too.
        tampered = AttestableFrame(
            id=honest.id,
            content_digest=honest.content_digest,
            provenance=[{**provenance[0], "uri": "src/evil.rs"}],
        )
        self.assertEqual(
            verify_frame_attestation(
                spec["provider_id"], tampered, attestation(), public_key()
            ).verdict,
            Verdict.COMMITMENT_MISMATCH,
        )
        # And the identity binding: same bytes, different provider.
        self.assertEqual(
            verify_frame_attestation(
                "impostor", honest, attestation(), public_key()
            ).verdict,
            Verdict.COMMITMENT_MISMATCH,
        )

    def test_every_failure_is_named(self) -> None:
        commitment = signed_commitment()
        key = public_key()
        wire = V["signature"]["attestation"]

        unknown = verify_commitment(
            commitment,
            ProvenanceAttestation.from_wire({**wire, "algorithm": "dilithium3"}),
            key,
        )
        self.assertEqual(unknown.verdict, Verdict.UNKNOWN_ALGORITHM)
        self.assertEqual(unknown.algorithm, "dilithium3")

        self.assertEqual(
            verify_commitment(
                commitment,
                ProvenanceAttestation.from_wire(
                    {**wire, "signed_commitment": "not-a-digest"}
                ),
                key,
            ).verdict,
            Verdict.MALFORMED_COMMITMENT,
        )
        self.assertEqual(
            verify_commitment(
                commitment,
                ProvenanceAttestation.from_wire({**wire, "signature": "abcd"}),
                key,
            ).verdict,
            Verdict.MALFORMED_SIGNATURE,
        )
        self.assertEqual(
            verify_commitment(commitment, attestation(), bytes(5)).verdict,
            Verdict.MALFORMED_KEY,
        )

    def test_hex_is_accepted_in_exactly_one_spelling(self) -> None:
        # The protocol's grammar is lowercase (is_well_formed_digest).
        # Accepting uppercase would mean two implementations disagreeing about
        # whether the same attestation is well-formed, which is the class of
        # divergence this whole port exists to close. bytes.fromhex would also
        # skip whitespace between byte pairs, so both are checked.
        commitment = signed_commitment()
        wire = V["signature"]["attestation"]
        for bad in (
            wire["signature"].upper(),
            wire["signature"][:2] + " " + wire["signature"][2:],
        ):
            with self.subTest(signature=bad[:8]):
                self.assertEqual(
                    verify_commitment(
                        commitment,
                        ProvenanceAttestation.from_wire({**wire, "signature": bad}),
                        public_key(),
                    ).verdict,
                    Verdict.MALFORMED_SIGNATURE,
                )
        upper = "sha256:" + wire["signed_commitment"][7:].upper()
        self.assertIsNone(parse_digest(upper))
        self.assertEqual(
            verify_commitment(
                commitment,
                ProvenanceAttestation.from_wire(
                    {**wire, "signed_commitment": upper}
                ),
                public_key(),
            ).verdict,
            Verdict.MALFORMED_COMMITMENT,
        )

    def test_a_strict_verifier_declines_weak_keys(self) -> None:
        commitment = signed_commitment()
        strictness = V["verifier_strictness"]
        rejectable = (
            strictness["small_order_public_keys_hex"]
            + strictness["non_canonical_public_keys_hex"]
        )
        self.assertTrue(rejectable)
        for hex_key in rejectable:
            with self.subTest(key=hex_key):
                self.assertEqual(
                    verify_commitment(
                        commitment, attestation(), bytes.fromhex(hex_key)
                    ).verdict,
                    Verdict.MALFORMED_KEY,
                )


class Ed25519Verifier(unittest.TestCase):
    """The verifier itself, checked against something other than this repo."""

    # RFC 8032 §7.1 TEST 1, TEST 2 and TEST 3: (public key, message,
    # signature), all hex. Cited rather than generated, so a bug in this
    # module cannot also produce its own expectation.
    RFC_8032_VECTORS = [
        (
            "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a",
            "",
            "e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e06522490155"
            "5fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b",
        ),
        (
            "3d4017c3e843895a92b70aa74d1b7ebc9c982ccf2ec4968cc0cd55f12af4660c",
            "72",
            "92a009a9f0d4cab8720e820b5f642540a2b27b5416503f8fb3762223ebdb69da"
            "085ac1e43e15996e458f3613d0f11d8c387b2eaeb4302aeeb00d291612bb0c00",
        ),
        (
            "fc51cd8e6218a1a38da47ed00230f0580816ed13ba3303ac5deb911548908025",
            "af82",
            "6291d657deec24024827e69c3abe01a30ce548a284743a445e3680d7db5ac3ac"
            "18ff9b538d16f290ae67f760984dc6594a7c15e9716ed28dc027beceea1ec40a",
        ),
    ]

    def test_rfc_8032_vectors_verify(self) -> None:
        for key, message, signature in self.RFC_8032_VECTORS:
            with self.subTest(message=message or "(empty)"):
                self.assertTrue(
                    _ed25519.verify(
                        bytes.fromhex(key),
                        bytes.fromhex(message),
                        bytes.fromhex(signature),
                    )
                )

    def test_every_perturbation_of_an_rfc_vector_is_rejected(self) -> None:
        key, message, signature = self.RFC_8032_VECTORS[2]
        raw_key = bytes.fromhex(key)
        raw_msg = bytes.fromhex(message)
        raw_sig = bytearray(bytes.fromhex(signature))
        # One flipped bit in R, one in S, one in the message, one in the key.
        for index in (0, 40):
            flipped = bytearray(raw_sig)
            flipped[index] ^= 0x01
            with self.subTest(signature_byte=index):
                self.assertFalse(_ed25519.verify(raw_key, raw_msg, bytes(flipped)))
        flipped_msg = bytearray(raw_msg)
        flipped_msg[0] ^= 0x01
        self.assertFalse(_ed25519.verify(raw_key, bytes(flipped_msg), bytes(raw_sig)))
        flipped_key = bytearray(raw_key)
        flipped_key[0] ^= 0x01
        self.assertFalse(_ed25519.verify(bytes(flipped_key), raw_msg, bytes(raw_sig)))

    def test_the_published_small_order_table_is_exactly_the_small_order_set(
        self,
    ) -> None:
        # The TypeScript and Go ports carry this table because they have no
        # field arithmetic of their own. Here it is recomputed: every entry
        # must satisfy 8P = identity, and the base point must not.
        strictness = V["verifier_strictness"]
        for hex_key in strictness["small_order_public_keys_hex"]:
            with self.subTest(key=hex_key):
                point = _ed25519._decompress(bytes.fromhex(hex_key))
                self.assertIsNotNone(point, "a listed key must decode to a point")
                self.assertTrue(_ed25519._is_small_order(point))
                self.assertFalse(_ed25519.is_usable_public_key(bytes.fromhex(hex_key)))
        for hex_key in strictness["non_canonical_public_keys_hex"]:
            with self.subTest(key=hex_key):
                self.assertIsNone(_ed25519._decompress(bytes.fromhex(hex_key)))
        # The honest key this repository publishes is neither.
        self.assertTrue(_ed25519.is_usable_public_key(public_key()))
        self.assertFalse(_ed25519._is_small_order(_ed25519.B))

    def test_a_non_reduced_scalar_is_rejected(self) -> None:
        # S + L is a second encoding of a signature a lax verifier accepts for
        # the same message (RFC 8032 §8.4).
        raw = bytes.fromhex(V["signature"]["attestation"]["signature"])
        s = int.from_bytes(raw[32:], "little")
        malleable = raw[:32] + (s + _ed25519.L).to_bytes(32, "little")
        self.assertTrue(_ed25519.verify(public_key(), signed_commitment(), raw))
        self.assertFalse(_ed25519.verify(public_key(), signed_commitment(), malleable))


class DifferentialAgainstCryptography(unittest.TestCase):
    """Cross-check the in-package verifier against a vetted implementation.

    Skipped where ``cryptography`` is absent, which is the normal case for this
    SDK's users and for CI — the RFC 8032 vectors above are what runs there.
    This adds the one thing they cannot: agreement on inputs nobody published
    an answer for.
    """

    def setUp(self) -> None:
        try:
            from cryptography.hazmat.primitives.asymmetric import (  # noqa: F401
                ed25519 as _backend,
            )
        except ImportError:  # pragma: no cover - depends on the environment
            self.skipTest("cryptography is not installed")

    def test_the_two_verifiers_agree_on_every_case(self) -> None:
        from cryptography.exceptions import InvalidSignature
        from cryptography.hazmat.primitives.asymmetric import ed25519 as backend

        def reference(key: bytes, message: bytes, signature: bytes) -> bool:
            try:
                backend.Ed25519PublicKey.from_public_bytes(key).verify(
                    signature, message
                )
                return True
            except (InvalidSignature, ValueError):
                return False

        cases = [
            (public_key(), signed_commitment(), bytes.fromhex(
                V["signature"]["attestation"]["signature"]
            )),
        ]
        for key, message, signature in Ed25519Verifier.RFC_8032_VECTORS:
            cases.append(
                (bytes.fromhex(key), bytes.fromhex(message), bytes.fromhex(signature))
            )
        # Perturbations of each, so the agreement covers rejections too.
        for key, message, signature in list(cases):
            flipped = bytearray(signature)
            flipped[0] ^= 0x01
            cases.append((key, message, bytes(flipped)))
            cases.append((key, message + b"\x00", signature))

        for index, (key, message, signature) in enumerate(cases):
            with self.subTest(case=index):
                self.assertEqual(
                    _ed25519.verify(key, message, signature),
                    reference(key, message, signature),
                )


if __name__ == "__main__":  # pragma: no cover
    unittest.main()

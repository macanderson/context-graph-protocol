"""The Python record-layer port, reconciled against the published vectors.

Every ``record_hash`` and every canonical preimage asserted here is read from
``tests/fixtures/record-hash-vectors.json``, and the attestation from
``tests/fixtures/record-attestation.json`` with its key in
``record-attestation-key.json`` — all published by the Rust reference
(``contextgraph_types::record_attest``) and recomputed by
``contextgraph-conformance``. RFC 8785's own examples are entered verbatim from
the RFC. Nothing is asserted against a value this file computed.

The RFC 8785 Appendix B number table comes first, as #119 asks: Python's
``repr`` and ECMAScript's ``Number::toString`` disagree at exactly the
exponent thresholds that table pins, so it is where a Python JCS port breaks.

Standard-library :mod:`unittest` only, like ``test_attest.py``, so it runs on a
bare ``python3``.
"""

from __future__ import annotations

import json
import struct
import unittest
from pathlib import Path
from typing import Any

from contextgraph_sdk import _ed25519
from contextgraph_sdk.attest import Verdict
from contextgraph_sdk.record import (
    RECORD_ATTESTATION_DOMAIN,
    RECORD_HASH_MEMBER,
    CanonicalizationError,
    RecordAttestation,
    RecordHashError,
    canonicalize,
    record_attestation_message,
    record_hash,
    record_hash_is_current,
    record_hash_preimage,
    verify_record_attestation,
    verify_signed_record_hash,
)


def _fixtures_dir() -> Path:
    """Walk up from this file until ``tests/fixtures`` appears.

    A relative depth would be right for exactly one of "run from the repo
    root" and "run from sdk/python", and both happen.
    """
    here = Path(__file__).resolve()
    for parent in [here] + list(here.parents):
        candidate = parent / "tests" / "fixtures" / "record-hash-vectors.json"
        if candidate.is_file():
            return candidate.parent
    raise AssertionError("tests/fixtures/record-hash-vectors.json not found above " + __file__)


FIXTURES = _fixtures_dir()


def load(name: str) -> Any:
    return json.loads((FIXTURES / name).read_text(encoding="utf-8"))


VECTORS = load("record-hash-vectors.json")
ATTESTATION = load("record-attestation.json")
KEY = load("record-attestation-key.json")


def signed_record() -> Any:
    return load(KEY["signs"])


def published_key() -> bytes:
    return bytes.fromhex(KEY["public_key"])


def published_attestation() -> RecordAttestation:
    return RecordAttestation.from_wire(ATTESTATION)


def double(bits: int) -> float:
    """The IEEE 754 double with these 64 bits."""
    value: float = struct.unpack(">d", bits.to_bytes(8, "big"))[0]
    return value


class Rfc8785Numbers(unittest.TestCase):
    """RFC 8785 Appendix B, Table 1 — ECMAScript number text, bit for bit."""

    # (IEEE 754 bits, the text RFC 8785 Appendix B requires). NaN and the
    # infinities are covered separately: JCS must refuse them, not print them.
    APPENDIX_B = [
        (0x0000000000000000, "0"),
        (0x8000000000000000, "0"),
        (0x0000000000000001, "5e-324"),
        (0x8000000000000001, "-5e-324"),
        (0x7FEFFFFFFFFFFFFF, "1.7976931348623157e+308"),
        (0xFFEFFFFFFFFFFFFF, "-1.7976931348623157e+308"),
        (0x4340000000000000, "9007199254740992"),
        (0xC340000000000000, "-9007199254740992"),
        (0x4430000000000000, "295147905179352830000"),
        (0x44B52D02C7E14AF5, "9.999999999999997e+22"),
        (0x44B52D02C7E14AF6, "1e+23"),
        (0x44B52D02C7E14AF7, "1.0000000000000001e+23"),
        (0x444B1AE4D6E2EF4E, "999999999999999700000"),
        (0x444B1AE4D6E2EF4F, "999999999999999900000"),
        (0x444B1AE4D6E2EF50, "1e+21"),
        (0x3EB0C6F7A0B5ED8C, "9.999999999999997e-7"),
        (0x3EB0C6F7A0B5ED8D, "0.000001"),
        (0x41B3DE4355555553, "333333333.3333332"),
        (0x41B3DE4355555554, "333333333.33333325"),
        (0x41B3DE4355555555, "333333333.3333333"),
        (0x41B3DE4355555556, "333333333.3333334"),
        (0x41B3DE4355555557, "333333333.33333343"),
        (0xBECBF647612F3696, "-0.0000033333333333333333"),
        (0x43143FF3C1CB0959, "1424953923781206.2"),
    ]

    def test_appendix_b_number_serialization_samples(self) -> None:
        for bits, expected in self.APPENDIX_B:
            with self.subTest(bits=f"{bits:#018x}"):
                self.assertEqual(
                    canonicalize({"n": double(bits)}),
                    ('{"n":' + expected + "}").encode("ascii"),
                )

    def test_the_table_really_crosses_the_thresholds_repr_gets_wrong(self) -> None:
        # Guard against the table passing for the wrong reason: for these
        # entries Python's own spelling differs from ECMAScript's, so an
        # implementation that used repr() would fail the test above.
        for bits, expected in (
            (0x3EB0C6F7A0B5ED8D, "0.000001"),
            (0x3EB0C6F7A0B5ED8C, "9.999999999999997e-7"),
            (0x4340000000000000, "9007199254740992"),
            (0x444B1AE4D6E2EF4E, "999999999999999700000"),
        ):
            with self.subTest(expected=expected):
                self.assertNotEqual(repr(double(bits)), expected)

    def test_nan_and_the_infinities_are_refused(self) -> None:
        for bits in (0x7FF8000000000000, 0x7FF0000000000000, 0xFFF0000000000000):
            with self.subTest(bits=f"{bits:#018x}"):
                with self.assertRaises(CanonicalizationError):
                    canonicalize({"n": double(bits)})

    def test_an_integer_is_a_double_first(self) -> None:
        # JCS has one number type, the IEEE 754 double. 2**53 + 1 is not
        # representable, and a JSON parser producing doubles rounds it to 2**53.
        self.assertEqual(canonicalize(2**53 + 1), b"9007199254740992")
        self.assertEqual(canonicalize(10**21), b"1e+21")
        self.assertEqual(canonicalize(-7), b"-7")
        # And an integer past the double range has no JSON meaning at all.
        with self.assertRaises(CanonicalizationError):
            canonicalize(10**400)

    def test_booleans_are_literals_not_numbers(self) -> None:
        # bool is an int subclass in Python; to JSON it is a literal.
        self.assertEqual(canonicalize([True, False, None, 1]), b"[true,false,null,1]")


class Rfc8785Structure(unittest.TestCase):
    def test_section_3_2_4_worked_example_is_byte_for_byte(self) -> None:
        # The RFC's input (§3.2.2) and its canonical bytes (§3.2.4's hex
        # listing), entered verbatim from https://www.rfc-editor.org/rfc/rfc8785.txt.
        source = (
            "{\n"
            '  "numbers": [333333333.33333329, 1E30, 4.50,\n'
            "              2e-3, 0.000000000000000000000000001],\n"
            '  "string": "\\u20ac$\\u000F\\u000aA\'\\u0042\\u0022\\u005c\\\\\\"\\/",\n'
            '  "literals": [null, true, false]\n'
            "}"
        )
        expected = bytes.fromhex(
            "7b226c69746572616c73223a5b6e756c6c2c747275652c66616c73655d2c226e"
            "756d62657273223a5b3333333333333333332e333333333333332c31652b3330"
            "2c342e352c302e3030322c31652d32375d2c22737472696e67223a22e282ac24"
            "5c75303030665c6e4127425c225c5c5c5c5c222f227d"
        )
        self.assertEqual(canonicalize(json.loads(source)), expected)

    def test_section_3_2_3_sorts_member_names_by_utf16_code_unit(self) -> None:
        # The emoji is a surrogate pair (leading unit 0xD83D), so it sorts
        # *before* U+FB33 although its code point is far higher. A port that
        # sorted by code point — Python's default — puts it last.
        source = (
            "{"
            '"\\u20ac": "Euro Sign",'
            '"\\r": "Carriage Return",'
            '"\\ufb33": "Hebrew Letter Dalet With Dagesh",'
            '"1": "One",'
            '"\\ud83d\\ude00": "Emoji: Grinning Face",'
            '"\\u0080": "Control",'
            '"\\u00f6": "Latin Small Letter O With Diaeresis"'
            "}"
        )
        canonical = canonicalize(json.loads(source)).decode("utf-8")
        positions = [
            canonical.index(label)
            for label in (
                "Carriage Return",
                "One",
                "Control",
                "Latin Small Letter O With Diaeresis",
                "Euro Sign",
                "Emoji: Grinning Face",
                "Hebrew Letter Dalet With Dagesh",
            )
        ]
        self.assertEqual(positions, sorted(positions), canonical)

    def test_strings_escape_exactly_what_the_rfc_lists(self) -> None:
        # The short escapes, lowercase \u00xx for the other C0 controls, and
        # literal UTF-8 for everything else — U+007F and U+2028 included.
        self.assertEqual(
            canonicalize("\b\t\n\f\r\"\\\x01\x1f\x7f /é"),
            '"\\b\\t\\n\\f\\r\\"\\\\\\u0001\\u001f\x7f /é"'.encode("utf-8"),
        )

    def test_a_lone_surrogate_is_refused(self) -> None:
        # json.loads produces one happily; RFC 8785 requires a refusal.
        with self.assertRaises(CanonicalizationError):
            canonicalize(json.loads('"\\ud800"'))
        with self.assertRaises(CanonicalizationError):
            canonicalize(json.loads('{"\\udc00": 1}'))

    def test_non_json_values_are_refused(self) -> None:
        with self.assertRaises(CanonicalizationError):
            canonicalize({"when": object()})
        with self.assertRaises(CanonicalizationError):
            canonicalize({1: "a non-string member name"})


class RecordHashVectors(unittest.TestCase):
    def test_every_published_preimage_and_hash_is_reproduced(self) -> None:
        self.assertTrue(VECTORS["vectors"], "the vector file must not be empty")
        for vector in VECTORS["vectors"]:
            with self.subTest(record=vector["record_file"]):
                record = load(vector["record_file"])
                self.assertEqual(
                    record_hash_preimage(record).decode("utf-8"), vector["jcs_utf8"]
                )
                self.assertEqual(record_hash(record), vector["record_hash"])

    def test_a_published_record_carrying_its_hash_is_current(self) -> None:
        record = signed_record()
        self.assertIn(RECORD_HASH_MEMBER, record, "precondition: the fixture carries one")
        self.assertTrue(record_hash_is_current(record))


class OmitSelf(unittest.TestCase):
    def test_the_hash_member_is_removed_not_blanked(self) -> None:
        record = signed_record()
        absent = {k: v for k, v in record.items() if k != RECORD_HASH_MEMBER}
        wrong = {**record, RECORD_HASH_MEMBER: "sha256:" + "f" * 64}
        self.assertEqual(record_hash_preimage(record), record_hash_preimage(absent))
        self.assertEqual(record_hash_preimage(record), record_hash_preimage(wrong))
        self.assertNotIn(
            RECORD_HASH_MEMBER.encode("utf-8"), record_hash_preimage(record)
        )

    def test_only_the_top_level_member_is_removed(self) -> None:
        record = signed_record()
        nested = {**record, "extensions": {"record_hash": "sha256:nested"}}
        self.assertIn(b"sha256:nested", record_hash_preimage(nested))
        self.assertNotEqual(record_hash(nested), record_hash(record))

    def test_member_order_does_not_change_the_hash(self) -> None:
        forward = json.loads('{"a":1,"b":2,"record_hash":"x"}')
        reverse = json.loads('{"record_hash":"x","b":2,"a":1}')
        self.assertEqual(record_hash(forward), record_hash(reverse))

    def test_editing_content_changes_the_hash_and_staleness_is_visible(self) -> None:
        edited = {**signed_record(), "statement": "edited after hashing"}
        self.assertNotEqual(record_hash(edited), record_hash(signed_record()))
        self.assertFalse(record_hash_is_current(edited))
        unhashed = {
            k: v for k, v in signed_record().items() if k != RECORD_HASH_MEMBER
        }
        self.assertFalse(
            record_hash_is_current(unhashed),
            "an unhashed record is not current; it is unhashed",
        )

    def test_failures_are_named(self) -> None:
        with self.assertRaises(RecordHashError) as not_object:
            record_hash([1, 2, 3])  # type: ignore[arg-type]
        self.assertEqual(not_object.exception.kind, RecordHashError.NOT_AN_OBJECT)

        with self.assertRaises(RecordHashError) as nan:
            record_hash({"confidence": float("nan")})
        self.assertEqual(nan.exception.kind, RecordHashError.NOT_CANONICALIZABLE)

        with self.assertRaises(RecordHashError) as short:
            record_attestation_message("sha256:short")
        self.assertEqual(short.exception.kind, RecordHashError.MALFORMED_DIGEST)


class RecordAttestationVectors(unittest.TestCase):
    def test_the_published_attestation_verifies_under_the_published_key(self) -> None:
        verdict = verify_record_attestation(
            signed_record(), published_attestation(), published_key()
        )
        self.assertEqual(verdict.verdict, Verdict.VALID)
        self.assertTrue(verdict.is_valid())
        self.assertTrue(published_attestation().uses_known_algorithm())
        self.assertEqual(
            published_attestation().signed_record_hash, record_hash(signed_record())
        )

    def test_the_signed_message_is_the_published_bytes(self) -> None:
        message = record_attestation_message(ATTESTATION["signed_record_hash"])
        self.assertEqual(message.hex(), KEY["signed_message_hex"])
        self.assertTrue(message.startswith(RECORD_ATTESTATION_DOMAIN))
        self.assertEqual(len(message), len(RECORD_ATTESTATION_DOMAIN) + 32)
        # And the verifier accepts exactly those bytes, directly.
        self.assertTrue(
            _ed25519.verify(
                published_key(), message, bytes.fromhex(ATTESTATION["signature"])
            )
        )

    def test_the_wire_form_round_trips(self) -> None:
        self.assertEqual(published_attestation().to_wire(), ATTESTATION)

    def test_editing_the_record_after_signing_is_a_mismatch(self) -> None:
        tampered = {**signed_record(), "statement": "the api handler never retries"}
        verdict = verify_record_attestation(
            tampered, published_attestation(), published_key()
        )
        self.assertEqual(verdict.verdict, Verdict.COMMITMENT_MISMATCH)
        self.assertEqual(verdict.signed, ATTESTATION["signed_record_hash"])
        self.assertEqual(verdict.expected, record_hash(tampered))

    def test_rewriting_the_stored_hash_does_not_launder_a_tampered_record(
        self,
    ) -> None:
        # LC5: edit the content, then restate record_hash so the record is
        # internally consistent. Only the recompute catches it.
        laundered = {**signed_record(), "statement": "the api handler never retries"}
        laundered[RECORD_HASH_MEMBER] = record_hash(laundered)
        self.assertTrue(record_hash_is_current(laundered))
        self.assertEqual(
            verify_record_attestation(
                laundered, published_attestation(), published_key()
            ).verdict,
            Verdict.COMMITMENT_MISMATCH,
        )

    def test_a_forged_stored_hash_is_ignored_in_favour_of_the_content(self) -> None:
        # The converse: a wrong stored member does not break an honest record,
        # because the stored member is never part of the preimage.
        relabelled = {**signed_record(), RECORD_HASH_MEMBER: "sha256:" + "0" * 64}
        self.assertEqual(
            verify_record_attestation(
                relabelled, published_attestation(), published_key()
            ).verdict,
            Verdict.VALID,
        )

    def test_a_perturbed_signature_is_a_bad_signature(self) -> None:
        raw = bytearray(bytes.fromhex(ATTESTATION["signature"]))
        raw[0] ^= 0x01
        forged = RecordAttestation.from_wire({**ATTESTATION, "signature": raw.hex()})
        self.assertEqual(
            verify_record_attestation(signed_record(), forged, published_key()).verdict,
            Verdict.BAD_SIGNATURE,
        )

    def test_a_signature_over_the_bare_digest_is_not_a_record_attestation(
        self,
    ) -> None:
        # What the domain tag buys: the published signature verifies over the
        # tagged message, and must not also verify over the bare 32 bytes.
        signature = bytes.fromhex(ATTESTATION["signature"])
        bare = bytes.fromhex(ATTESTATION["signed_record_hash"][len("sha256:") :])
        self.assertFalse(_ed25519.verify(published_key(), bare, signature))

    def test_every_failure_is_named(self) -> None:
        record = signed_record()
        key = published_key()

        unknown = verify_record_attestation(
            record,
            RecordAttestation.from_wire({**ATTESTATION, "algorithm": "dilithium3"}),
            key,
        )
        self.assertEqual(unknown.verdict, Verdict.UNKNOWN_ALGORITHM)
        self.assertEqual(unknown.algorithm, "dilithium3")
        self.assertFalse(unknown.is_valid(), "declining is still not accepting")

        self.assertEqual(
            verify_record_attestation(
                record,
                RecordAttestation.from_wire(
                    {**ATTESTATION, "signed_record_hash": "not-a-digest"}
                ),
                key,
            ).verdict,
            Verdict.MALFORMED_COMMITMENT,
        )
        for bad in ("abcd", ATTESTATION["signature"].upper()):
            with self.subTest(signature=bad[:8]):
                self.assertEqual(
                    verify_record_attestation(
                        record,
                        RecordAttestation.from_wire({**ATTESTATION, "signature": bad}),
                        key,
                    ).verdict,
                    Verdict.MALFORMED_SIGNATURE,
                )
        self.assertEqual(
            verify_record_attestation(record, published_attestation(), bytes(5)).verdict,
            Verdict.MALFORMED_KEY,
        )

    def test_a_wrong_key_is_a_bad_signature_not_a_mismatch(self) -> None:
        # The provenance-vector key is a real, usable key that did not sign
        # this record: the hash is intact, only the key is wrong.
        other = bytes.fromhex(
            "ea4a6c63e29c520abef5507b132ec5f9954776aebebe7b92421eea691446d22c"
        )
        self.assertEqual(
            verify_signed_record_hash(
                ATTESTATION["signed_record_hash"], published_attestation(), other
            ).verdict,
            Verdict.BAD_SIGNATURE,
        )

    def test_a_non_object_cannot_be_verified_at_all(self) -> None:
        with self.assertRaises(RecordHashError):
            verify_record_attestation(
                "not a record",  # type: ignore[arg-type]
                published_attestation(),
                published_key(),
            )


if __name__ == "__main__":  # pragma: no cover
    unittest.main()

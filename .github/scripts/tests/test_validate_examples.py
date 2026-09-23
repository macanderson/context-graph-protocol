"""Self-test for `schema/validate-examples.py` (#110).

The fixture tree is synthetic — two tiny schemas, one example per surface, one
record — rather than a copy of the real `schema/` and `tests/fixtures/`. That
keeps each test about the guard: a real example going invalid must redden the
`schema` job, not this one, and a test here cannot pass or fail because of what
the repository currently holds.

Needs `jsonschema`, exactly as the gate does; CI installs it for both.
"""
from __future__ import annotations

import hashlib
import json
import unittest

from _gate_harness import GateTestCase

BASE = "https://contextgraphprotocol.org/schema/v1/"
ENVELOPE = "contextgraph-envelope.schema.json"
RECORD = "contextgraph-lifecycle-record.schema.json"
DRAFT = "https://json-schema.org/draft/2020-12/schema"

ENVELOPE_SCHEMA = {
    "$schema": DRAFT,
    "$id": BASE + ENVELOPE,
    "oneOf": [{"$ref": "#/$defs/Handshake"}],
    "$defs": {
        "Handshake": {
            "type": "object",
            "properties": {
                "type": {"const": "handshake"},
                "protocol_version": {"type": "string"},
            },
            "required": ["type", "protocol_version"],
            "additionalProperties": False,
        },
        "ContextFrame": {
            "type": "object",
            "properties": {"title": {"type": "string", "minLength": 1}},
            "required": ["title"],
            "additionalProperties": False,
        },
    },
}

RECORD_KINDS = ["observation", "knowledge"]

RECORD_SCHEMA = {
    "$schema": DRAFT,
    "$id": BASE + RECORD,
    "type": "object",
    "properties": {
        "record_kind": {"$ref": "#/$defs/recordKind"},
        "statement": {"type": "string", "minLength": 1},
        "record_hash": {"type": "string"},
    },
    "required": ["record_kind", "statement", "record_hash"],
    "additionalProperties": False,
    "$defs": {
        "recordKind": {"type": "string", "enum": RECORD_KINDS},
        "RecordAttestation": {
            "type": "object",
            "properties": {"signed_record_hash": {"type": "string"}},
            "required": ["signed_record_hash"],
            "additionalProperties": False,
        },
    },
}

HANDSHAKE = {"type": "handshake", "protocol_version": "contextgraph/1.0"}
DOMAIN_TAG = b"contextgraph/attest/1/record"


def canonical(value: dict) -> str:
    # Adequate for these ASCII-only fixtures; see tests/fixtures/README.md on
    # why this is not RFC 8785 in general.
    return json.dumps(value, sort_keys=True, separators=(",", ":"))


def sha256(text: str) -> str:
    return "sha256:" + hashlib.sha256(text.encode("utf-8")).hexdigest()


class ValidateExamplesTest(GateTestCase):
    SCRIPT = "schema/validate-examples.py"

    @classmethod
    def setUpClass(cls) -> None:
        # Fail loudly rather than skip: a skipped suite is exactly the silent
        # PASS these tests exist to rule out.
        try:
            import jsonschema  # noqa: F401
        except ImportError:
            raise RuntimeError(
                "jsonschema is not installed, and schema/validate-examples.py needs it."
                " The job running these self-tests must `pip install jsonschema`,"
                " as the `schema` job does.") from None

    def setUp(self) -> None:
        super().setUp()
        self.envelope_schema = json.loads(json.dumps(ENVELOPE_SCHEMA))
        self.record_schema = json.loads(json.dumps(RECORD_SCHEMA))
        self.spec = (
            "# Spec\n\n```jsonc\n"
            '{ "type": "handshake", "protocol_version": "contextgraph/1.0" } // a comment\n'
            "```\n\n```jsonc\n"
            '{ "title": "…" }\n'
            "```\n"
        )
        # The walkthrough section must quote the transcript line for line (#144).
        self.readme = (
            "# Examples\n\n## A complete stdio session (annotated)\n\n```json\n"
            + json.dumps(HANDSHAKE) + "\n```\n"
        )
        self.records = {
            kind: {"record_kind": kind, "statement": f"a {kind}"} for kind in RECORD_KINDS
        }

    def build(self) -> None:
        """Write the whole world from the current (possibly mutated) state."""
        self.write(f"schema/{ENVELOPE}", json.dumps(self.envelope_schema))
        self.write(f"schema/{RECORD}", json.dumps(self.record_schema))
        self.write("schema/reference-vectors.ndjson", json.dumps(HANDSHAKE) + "\n")
        self.write("examples/full-stdio-session.ndjson", json.dumps(HANDSHAKE) + "\n")
        self.write("examples/reference-messages.json", json.dumps([HANDSHAKE]))
        self.write("examples/README.md", self.readme)
        self.write("SPEC.md", self.spec)

        vectors = []
        for kind, body in self.records.items():
            hashless = {k: v for k, v in body.items() if k != "record_hash"}
            text = canonical(hashless)
            # A test may plant a wrong stored hash; the vector always publishes
            # the true one, so the disagreement is the fixture's alone.
            record = {**body, "record_hash": body.get("record_hash", sha256(text))}
            self.write(f"tests/fixtures/{kind}.json", json.dumps(record))
            vectors.append({"record_file": f"{kind}.json", "jcs_utf8": text,
                            "record_hash": sha256(text)})
        self.write("tests/fixtures/record-hash-vectors.json",
                   json.dumps({"vectors": vectors}))

        signed = vectors[0]["record_hash"]
        self.write("tests/fixtures/record-attestation.json",
                   json.dumps({"signed_record_hash": signed}))
        self.write("tests/fixtures/record-attestation-key.json", json.dumps({
            "signs": vectors[0]["record_file"],
            "signed_message_hex": DOMAIN_TAG.hex() + signed.removeprefix("sha256:"),
        }))

    def gate(self):
        self.build()
        return self.run_gate()

    # --- the baseline, so every failure below is attributable --------------

    def test_a_consistent_world_passes(self):
        self.assertPasses(self.gate())

    # --- the examples README walkthrough (#144) ------------------------------

    def test_a_walkthrough_that_drifts_from_the_transcript_fails(self):
        drifted = {**HANDSHAKE, "protocol_version": "contextgraph/0.9"}
        self.readme = self.readme.replace(json.dumps(HANDSHAKE), json.dumps(drifted))
        self.assertFailsWith(self.gate(), "FAIL  README.md:5 is transcript line 1")

    # --- `$id` --------------------------------------------------------------

    def test_a_wrong_envelope_id_fails(self):
        self.envelope_schema["$id"] = (
            "https://raw.githubusercontent.com/macanderson/context-graph-protocol/main/schema/"
            + ENVELOPE)
        self.assertFailsWith(self.gate(), f"FAIL  $id is {BASE}{ENVELOPE}")

    def test_a_wrong_record_schema_id_fails(self):
        self.record_schema["$id"] = f"https://contextgraphprotocol.org/schema/{RECORD}"
        self.assertFailsWith(self.gate(), f"FAIL  $id is {BASE}{RECORD}")

    # --- fixtures -----------------------------------------------------------

    def test_a_record_fixture_that_violates_the_schema_fails(self):
        self.records["knowledge"]["surprise"] = True
        self.assertFailsWith(self.gate(), "FAIL  tests/fixtures/knowledge.json")

    def test_an_example_message_that_violates_the_schema_fails(self):
        self.build()
        self.write("examples/reference-messages.json",
                   json.dumps([{**HANDSHAKE, "id": "not-on-a-handshake"}]))
        self.assertFailsWith(self.run_gate(), "FAIL  reference-messages.json message 0")

    def test_a_record_hash_that_disagrees_with_its_vector_fails(self):
        self.records["observation"]["record_hash"] = "sha256:" + "0" * 64
        self.assertFailsWith(
            self.gate(),
            "FAIL  observation.json: the published record_hash is the one the fixture stores")

    # --- which fixtures are records (#126) ----------------------------------

    def test_a_non_record_json_fails_naming_the_convention(self):
        self.build()
        self.write("tests/fixtures/scratch.json", '{"hello": "world"}')
        result = self.run_gate()
        self.assertFailsWith(
            result,
            "FAIL  every JSON file in tests/fixtures is a record kind or a declared non-record",
            "tests/fixtures/scratch.json: 'scratch' is not a record_kind",
            "the filename stem is the record_kind",
        )
        # Reported as a stray, never validated as a record: the old failure
        # mode was a record-schema error blaming the wrong thing.
        self.assertNotIn("FAIL  tests/fixtures/scratch.json", result.stdout)

    def test_the_declared_non_record_fixtures_are_not_validated_as_records(self):
        result = self.gate()
        self.assertPasses(result)
        self.assertNotIn("tests/fixtures/record-hash-vectors.json (", result.stdout)

    def test_a_record_fixture_that_disappears_is_caught(self):
        self.build()
        self.remove("tests/fixtures/knowledge.json")
        self.assertFailsWith(
            self.run_gate(),
            "FAIL  every record_kind the schema declares has a fixture",
            "tests/fixtures/knowledge.json is missing",
        )

    def test_every_record_fixture_disappearing_is_caught(self):
        self.build()
        for kind in RECORD_KINDS:
            self.remove(f"tests/fixtures/{kind}.json")
        self.assertFailsWith(
            self.run_gate(), "FAIL  tests/fixtures holds lifecycle record examples")

    def test_a_record_filed_under_another_kind_fails(self):
        self.records["knowledge"]["record_kind"] = "observation"
        self.assertFailsWith(
            self.gate(),
            "FAIL  tests/fixtures/knowledge.json (observation)",
            "the filename stem is the record_kind",
        )

    def test_a_record_schema_without_its_kind_enum_is_reported(self):
        del self.record_schema["$defs"]["recordKind"]["enum"]
        self.record_schema["$defs"]["recordKind"]["type"] = "string"
        self.assertFailsWith(
            self.gate(), "FAIL  the record schema declares its record kinds")

    # --- SPEC.md ------------------------------------------------------------

    def test_a_spec_block_with_an_undeclared_member_fails(self):
        # The #54-era defect: an `id` on an envelope that grants none.
        self.spec = self.spec.replace(
            '"protocol_version": "contextgraph/1.0" }',
            '"protocol_version": "contextgraph/1.0", "id": "r1" }')
        self.assertFailsWith(self.gate(), "FAIL  SPEC.md:3 (envelope handshake)")

    def test_a_spec_frame_example_is_held_to_the_frame_schema(self):
        self.spec = self.spec.replace('{ "title": "…" }', '{ "title": "…", "bogus": 1 }')
        self.assertFailsWith(self.gate(), "FAIL  SPEC.md:7 (ContextFrame)")

    def test_a_changed_fence_style_is_reported_rather_than_passing_vacuously(self):
        self.spec = self.spec.replace("```jsonc", "```json")
        self.assertFailsWith(
            self.gate(),
            "FAIL  SPEC.md contains fenced jsonc examples",
            "no ```jsonc blocks found")


if __name__ == "__main__":
    unittest.main()

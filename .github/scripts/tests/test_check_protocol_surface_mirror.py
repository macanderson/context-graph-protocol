"""Self-test for `.github/scripts/check-protocol-surface-mirror.py` (#158).

The guard exists because docs/protocol-surface.md silently fell 34 requirements
behind SPEC.md. Each test below builds a small SPEC.md and mirror pair and
proves the guard goes red in one direction it must: an id the spec adds, an id
the mirror invents, a row with no enforcement, a table shape it cannot read.
"""
from __future__ import annotations

import unittest

from _gate_harness import GateTestCase

SPEC_HEADER = "| # | Requirement | Verified by |\n| - | ----------- | ----------- |\n"
MIRROR_HEADER = "| # | Requirement | Enforced / verified by |\n| - | ----------- | ---------------------- |\n"


def spec(*rows: str) -> str:
    return "# Spec\n\n## 3. Handshake\n\n" + SPEC_HEADER + "".join(rows) + "\nprose\n"


def mirror(*rows: str, heading: str = "## Conformance requirements") -> str:
    return (
        "# CGP protocol surface\n\n"
        "## Context frame\n\n"
        # A table outside the mirrored section. Its ids must not count.
        + MIRROR_HEADER + "| Z9 | not a requirement row of the mirror | tour |\n\n"
        + heading + "\n\n### Handshake\n\n"
        + MIRROR_HEADER + "".join(rows)
        + "\n## Version strings\n\n"
        + MIRROR_HEADER + "| Z8 | after the section | tour |\n"
    )


class ProtocolSurfaceMirrorTest(GateTestCase):
    SCRIPT = ".github/scripts/check-protocol-surface-mirror.py"

    def gate(self, spec_text: str, mirror_text: str):
        self.write("SPEC.md", spec_text)
        self.write("docs/protocol-surface.md", mirror_text)
        return self.run_gate()

    # --- the baseline --------------------------------------------------------

    def test_a_complete_mirror_passes(self):
        self.assertPasses(self.gate(
            spec("| **H1** | reply | `handshake` |\n", "| **UR1** | report | host |\n",
                 "| **X1** | open codes |\n"),
            mirror("| H1 | reply | `handshake` check |\n", "| UR1 | report | host |\n",
                   "| X1 | open codes | `ErrorCode` |\n"),
        ))

    # --- the direction #158 was filed about ------------------------------------

    def test_a_requirement_added_to_the_spec_is_caught(self):
        result = self.gate(
            spec("| **H1** | reply | `handshake` |\n", "| **Q1** | kinds bind | `kinds-filter` |\n"),
            mirror("| H1 | reply | `handshake` check |\n"),
        )
        self.assertFailsWith(
            result, "FAIL  every SPEC.md requirement has a mirror row", "missing: Q1")

    def test_a_new_id_family_is_caught_without_being_listed(self):
        # Nothing in the guard names the families, so a letter nobody has used
        # yet is checked the day it lands.
        result = self.gate(
            spec("| **H1** | reply | x |\n", "| **ZZ12** | brand new | x |\n"),
            mirror("| H1 | reply | x |\n"),
        )
        self.assertFailsWith(result, "missing: ZZ12")

    def test_missing_ids_are_listed_in_spec_order(self):
        result = self.gate(
            spec("| **F2** | a | x |\n", "| **F10** | b | x |\n", "| **F9** | c | x |\n"),
            mirror("| F2 | a | x |\n"),
        )
        self.assertFailsWith(result, "missing: F9 F10")

    # --- the reverse direction ---------------------------------------------------

    def test_a_mirror_row_the_spec_does_not_define_is_caught(self):
        result = self.gate(
            spec("| **H1** | reply | x |\n"),
            mirror("| H1 | reply | x |\n", "| H9 | invented | x |\n"),
        )
        self.assertFailsWith(
            result,
            "FAIL  the mirror names no requirement SPEC.md does not define",
            "not in SPEC.md: H9",
        )

    def test_rows_outside_the_mirrored_section_do_not_count(self):
        # Z8/Z9 in the fixture sit outside the section. Counting them would
        # report them as invented; ignoring the section would let them stand in
        # for a real row.
        result = self.gate(spec("| **H1** | reply | x |\n"), mirror("| H1 | reply | x |\n"))
        self.assertPasses(result)
        self.assertNotIn("Z9", result.stdout)

    # --- duplicates and enforcement ---------------------------------------------

    def test_a_duplicated_mirror_row_is_caught(self):
        result = self.gate(
            spec("| **H1** | reply | x |\n"),
            mirror("| H1 | reply | x |\n", "| H1 | again | y |\n"),
        )
        self.assertFailsWith(result, "FAIL  no requirement id has two mirror rows", "H1 on lines")

    def test_a_duplicated_spec_id_is_caught(self):
        result = self.gate(
            spec("| **H1** | reply | x |\n", "| **H1** | reply again | x |\n"),
            mirror("| H1 | reply | x |\n"),
        )
        self.assertFailsWith(result, "FAIL  no requirement id is stated twice in SPEC.md")

    def test_a_row_naming_no_enforcement_is_caught(self):
        result = self.gate(
            spec("| **H1** | reply | x |\n", "| **H2** | names | x |\n"),
            mirror("| H1 | reply | x |\n", "| H2 | names |  |\n"),
        )
        self.assertFailsWith(
            result, "FAIL  every mirror row names what enforces or verifies it", "H2 (line")

    def test_a_two_column_row_is_caught_as_unenforced(self):
        result = self.gate(spec("| **H1** | reply |\n"), mirror("| H1 | reply |\n"))
        self.assertFailsWith(result, "FAIL  every mirror row names what enforces or verifies it")

    # --- both row shapes, and the blind spots -----------------------------------

    def test_both_row_shapes_are_read_on_both_sides(self):
        self.assertPasses(self.gate(
            spec("| **H1** | bold | x |\n", "| H2 | plain | x |\n"),
            mirror("| **H1** | bold | x |\n", "| H2 | plain | x |\n"),
        ))

    def test_a_spec_with_no_readable_rows_is_a_failure_not_a_pass(self):
        # If the spec's table shape changed, an empty id set would make every
        # comparison vacuously true.
        result = self.gate(
            spec("| *H1* | italic, a shape the gate does not read | x |\n"),
            mirror("| H1 | reply | x |\n"),
        )
        self.assertFailsWith(
            result, "FAIL  SPEC.md states numbered requirements this gate can read")

    def test_a_renamed_mirror_section_is_reported(self):
        result = self.gate(
            spec("| **H1** | reply | x |\n"),
            mirror("| H1 | reply | x |\n", heading="## Requirements index"),
        )
        self.assertFailsWith(
            result, "FAIL  docs/protocol-surface.md has a '## Conformance requirements' section")

    def test_an_empty_mirror_section_is_reported(self):
        result = self.gate(spec("| **H1** | reply | x |\n"), mirror())
        self.assertFailsWith(
            result, "FAIL  that section holds requirement rows this gate can read")


if __name__ == "__main__":
    unittest.main()

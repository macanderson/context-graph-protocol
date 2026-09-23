"""Shared scaffolding for the Python gate self-tests.

Every gate in this repository finds its subject relative to its own file
(`ROOT = Path(__file__).resolve()...`). So the hermetic way to test one is to
build a throwaway tree shaped like the repository, copy the **real, unmodified**
script into it at the same relative path, and run it there as a subprocess. The
gate then sees only the fixture tree: nothing in the checkout is read, nothing
is mutated, and a test cannot pass because of the state the repository happens
to be in.

That is also why no gate was refactored to become testable. Its behaviour under
test is its behaviour in CI, byte for byte, including its exit code and output.

Standard library only: this runs in CI with no test runner installed beyond
`python3 -m unittest`.
"""
from __future__ import annotations

import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

# The checkout these tests live in. Used for one thing only: to find the real
# script under test, which is copied into each fixture tree.
REPO = Path(__file__).resolve().parents[3]


class GateTestCase(unittest.TestCase):
    """A test case that runs one gate script against a fixture tree.

    Subclasses set `SCRIPT` to the gate's repository-relative path, and build a
    tree with `self.write(...)` before calling `self.run_gate()`.
    """

    SCRIPT: str = ""

    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory(prefix="cgp-gate-test-")
        self.root = Path(self._tmp.name)
        target = self.root / self.SCRIPT
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(REPO / self.SCRIPT, target)

    def tearDown(self) -> None:
        self._tmp.cleanup()

    def write(self, rel: str, text: str) -> Path:
        path = self.root / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")
        return path

    def remove(self, rel: str) -> None:
        (self.root / rel).unlink()

    def git_init(self) -> None:
        """Make the fixture tree a git repository with everything tracked.

        For gates that enumerate their subject with `git ls-files`.
        """
        for args in (
            ["init", "-q"],
            ["add", "-A"],
        ):
            subprocess.run(["git", "-C", str(self.root), *args], check=True,
                           capture_output=True)

    def run_gate(self) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [sys.executable, str(self.root / self.SCRIPT)],
            cwd=self.root,
            capture_output=True,
            text=True,
            timeout=60,
        )

    # --- assertions -------------------------------------------------------

    def assertPasses(self, result: subprocess.CompletedProcess[str]) -> None:
        self.assertEqual(result.returncode, 0, self._explain(result))

    def assertFailsWith(self, result: subprocess.CompletedProcess[str],
                        *needles: str) -> None:
        """The gate exited non-zero, and its output names *why*.

        A bare non-zero exit is not enough: a gate that crashed, or failed an
        unrelated check, would satisfy it. Each needle must appear in the
        output, so the test pins the specific check that went red.
        """
        self.assertNotEqual(result.returncode, 0, self._explain(result))
        output = result.stdout + result.stderr
        for needle in needles:
            self.assertIn(needle, output, self._explain(result))

    @staticmethod
    def _explain(result: subprocess.CompletedProcess[str]) -> str:
        return (f"\n--- exit {result.returncode} ---\n--- stdout ---\n"
                f"{result.stdout}\n--- stderr ---\n{result.stderr}")

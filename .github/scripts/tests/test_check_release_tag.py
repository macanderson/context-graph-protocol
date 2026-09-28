"""Self-test for check-release-tag.py (#103, ADR 0034).

Each rule is driven red by the drift it names and green once the drift is
fixed. The workspace is a scratch Cargo layout (manifests only, no Rust), the
"on main" rule runs against real commits in a scratch git repository, and the
registry rule is fed a fake index instead of the network.

The witness #103 asks for is `test_a_tag_off_main_is_refused` together with
`test_a_tag_that_misnames_the_version_is_refused`: `release.yml` cannot reach
`cargo publish` from a tag that is not `contextgraph-v<version>` on `main`.
"""
from __future__ import annotations

import importlib.util
import io
import json
import os
import subprocess
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path
from unittest import mock

SCRIPT = Path(__file__).resolve().parent.parent / "check-release-tag.py"
spec = importlib.util.spec_from_file_location("check_release_tag", SCRIPT)
assert spec and spec.loader
guard = importlib.util.module_from_spec(spec)
spec.loader.exec_module(guard)

WHO = {
    "GIT_AUTHOR_NAME": "Ada Lovelace",
    "GIT_AUTHOR_EMAIL": "ada@example.invalid",
    "GIT_COMMITTER_NAME": "Ada Lovelace",
    "GIT_COMMITTER_EMAIL": "ada@example.invalid",
    "GIT_CONFIG_GLOBAL": os.devnull,
    "GIT_CONFIG_NOSYSTEM": "1",
}


def index_body(*versions: str) -> str:
    """A sparse-index file listing `versions`, one JSON object per line."""
    return "\n".join(json.dumps({"name": "x", "vers": v, "deps": [], "cksum": "0" * 64}) for v in versions) + "\n"


class Workspace(unittest.TestCase):
    """A scratch workspace in a scratch git repository, `main` checked out."""

    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory()
        self.root = Path(self._tmp.name)
        self.write_workspace("2.0.0")
        self.git("init", "-q", "-b", "main")
        self.git("add", "-A")
        self.git("commit", "-q", "-m", "release 2.0.0")
        self.release_commit = self.git("rev-parse", "HEAD").strip()

    def tearDown(self) -> None:
        self._tmp.cleanup()

    def git(self, *args: str) -> str:
        return subprocess.run(
            ["git", "-C", str(self.root), *args],
            env={**os.environ, **WHO},
            check=True,
            capture_output=True,
            text=True,
        ).stdout

    def write_workspace(self, version: str, trace_version: str | None = None) -> None:
        (self.root / "Cargo.toml").write_text(
            "[workspace]\n"
            'members = ["contextgraph-types", "contextgraph-trace", "contextgraph-refprov"]\n\n'
            "[workspace.package]\n"
            f'version = "{version}"\n'
            "publish = false\n",
            encoding="utf-8",
        )
        crates = {
            "contextgraph-types": ("version.workspace = true", "publish = true"),
            "contextgraph-trace": (
                "version.workspace = true" if trace_version is None else f'version = "{trace_version}"',
                "publish = true",
            ),
            # Unpublished: its version never counts.
            "contextgraph-refprov": ('version = "0.0.1"', "publish.workspace = true"),
        }
        for name, (version_line, publish_line) in crates.items():
            (self.root / name).mkdir(exist_ok=True)
            (self.root / name / "Cargo.toml").write_text(
                f'[package]\nname = "{name}"\n{version_line}\n{publish_line}\n',
                encoding="utf-8",
            )

    def run_guard(self, tag: str, commit: str, fetch=None, registry: bool = False) -> tuple[int, str]:
        argv = ["--tag", tag, "--commit", commit, "--main", "main", "--root", str(self.root)]
        if not registry:
            argv.append("--skip-registry")
        out, err = io.StringIO(), io.StringIO()
        with redirect_stdout(out), redirect_stderr(err), mock.patch.dict(os.environ, {}, clear=False):
            os.environ.pop("GITHUB_OUTPUT", None)
            code = guard.main(argv, fetch=fetch or (lambda url: (404, "")))
        return code, out.getvalue() + err.getvalue()


class TagNamesTheVersion(Workspace):
    def test_the_matching_tag_on_main_passes(self) -> None:
        code, out = self.run_guard("contextgraph-v2.0.0", self.release_commit)
        self.assertEqual(code, 0, out)
        self.assertIn("releases 2.0.0", out)

    def test_a_tag_that_misnames_the_version_is_refused(self) -> None:
        for tag in ("contextgraph-v2.0.1", "v2.0.0", "ocp-v2.0.0", "contextgraph-2.0.0", "contextgraph-v2.0.0 "):
            with self.subTest(tag=tag):
                code, out = self.run_guard(tag, self.release_commit)
                self.assertEqual(code, 1, out)
                self.assertIn("must be `contextgraph-v2.0.0`", out)

    def test_crates_out_of_lockstep_are_refused(self) -> None:
        self.write_workspace("2.0.0", trace_version="2.1.0")
        code, out = self.run_guard("contextgraph-v2.0.0", self.release_commit)
        self.assertEqual(code, 1, out)
        self.assertIn("lockstep", out)
        self.assertIn("contextgraph-trace 2.1.0", out)

    def test_an_unpublished_crate_does_not_count(self) -> None:
        versions = guard.published_crate_versions(self.root)
        self.assertEqual(versions, {"contextgraph-types": "2.0.0", "contextgraph-trace": "2.0.0"})

    def test_no_published_crate_fails_rather_than_passing_vacuously(self) -> None:
        self.assertIn("scan is broken", guard.tag_problems("contextgraph-v2.0.0", {})[0])

    def test_a_version_that_is_not_semver_is_refused(self) -> None:
        problems = guard.tag_problems("contextgraph-v2.0", {"contextgraph-types": "2.0"})
        self.assertIn("not a semantic version", problems[0])


class TaggedCommitIsOnMain(Workspace):
    def test_a_tag_off_main_is_refused(self) -> None:
        self.git("checkout", "-q", "-b", "feature")
        (self.root / "unreviewed.txt").write_text("never merged\n", encoding="utf-8")
        self.git("add", "-A")
        self.git("commit", "-q", "-m", "unreviewed")
        off_main = self.git("rev-parse", "HEAD").strip()

        code, out = self.run_guard("contextgraph-v2.0.0", off_main)
        self.assertEqual(code, 1, out)
        self.assertIn("is not on `main`", out)

        # The same tree, once merged to main, passes.
        self.git("checkout", "-q", "main")
        self.git("merge", "-q", "--ff-only", "feature")
        code, out = self.run_guard("contextgraph-v2.0.0", off_main)
        self.assertEqual(code, 0, out)

    def test_an_older_commit_on_main_passes(self) -> None:
        (self.root / "later.txt").write_text("later\n", encoding="utf-8")
        self.git("add", "-A")
        self.git("commit", "-q", "-m", "later work")
        code, out = self.run_guard("contextgraph-v2.0.0", self.release_commit)
        self.assertEqual(code, 0, out)

    def test_an_unknown_commit_fails(self) -> None:
        code, out = self.run_guard("contextgraph-v2.0.0", "0" * 40)
        self.assertEqual(code, 1, out)


class VersionIsNotAlreadyPublished(Workspace):
    def test_a_first_release_passes(self) -> None:
        code, out = self.run_guard("contextgraph-v2.0.0", self.release_commit, registry=True)
        self.assertEqual(code, 0, out)

    def test_a_new_version_of_a_published_crate_passes(self) -> None:
        fetch = lambda url: (200, index_body("0.1.2", "1.0.0"))  # noqa: E731
        code, out = self.run_guard("contextgraph-v2.0.0", self.release_commit, fetch=fetch, registry=True)
        self.assertEqual(code, 0, out)

    def test_a_version_already_live_is_refused(self) -> None:
        # A tag re-pushed for a live version: pushing `contextgraph-v2.0.0`
        # again after 2.0.0 shipped must stop in preflight, not wait for an
        # approval click that can only end in crates.io's rejection.
        fetch = lambda url: (200, index_body("0.1.2", "2.0.0"))  # noqa: E731
        code, out = self.run_guard("contextgraph-v2.0.0", self.release_commit, fetch=fetch, registry=True)
        self.assertEqual(code, 1, out)
        self.assertIn("contextgraph-types 2.0.0 is already on crates.io", out)

    def test_an_unreadable_index_fails_closed(self) -> None:
        fetch = lambda url: (None, "connection reset")  # noqa: E731
        code, out = self.run_guard("contextgraph-v2.0.0", self.release_commit, fetch=fetch, registry=True)
        self.assertEqual(code, 1, out)
        self.assertIn("fails closed", out)

    def test_the_index_path_follows_cargos_layout(self) -> None:
        self.assertEqual(guard.index_path("contextgraph-types"), "co/nt/contextgraph-types")
        self.assertEqual(guard.index_path("abc"), "3/a/abc")
        self.assertEqual(guard.index_path("ab"), "2/ab")
        self.assertEqual(guard.index_path("A"), "1/a")


class WritesTheVersionForThePublishJob(Workspace):
    def test_the_version_is_written_to_github_output(self) -> None:
        output = self.root / "github_output"
        argv = [
            "--tag", "contextgraph-v2.0.0", "--commit", self.release_commit,
            "--main", "main", "--root", str(self.root), "--skip-registry",
        ]
        with redirect_stdout(io.StringIO()), mock.patch.dict(os.environ, {"GITHUB_OUTPUT": str(output)}):
            self.assertEqual(guard.main(argv), 0)
        self.assertEqual(output.read_text(encoding="utf-8"), "version=2.0.0\n")


class TheWorkflowTriggersOnTheSamePrefix(unittest.TestCase):
    def test_release_yml_triggers_on_the_prefix_this_script_enforces(self) -> None:
        workflow = (Path(__file__).resolve().parents[2] / "workflows" / "release.yml").read_text(encoding="utf-8")
        self.assertIn(f'- "{guard.TAG_PREFIX}*"', workflow)
        self.assertIn("check-release-tag.py", workflow)


if __name__ == "__main__":
    unittest.main()

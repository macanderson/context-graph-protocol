"""Self-test for `.github/scripts/check-sdk-metadata.py` (#112).

The defect the guard exists for is a package that ships with no metadata, or
different metadata from the crates. The tests plant each form of that in a
throwaway `sdk/` tree and check the guard names it. They include a brand-new
SDK directory, because the guard exists so the next SDK cannot start with
nothing again.
"""
from __future__ import annotations

import json
import unittest

from _gate_harness import GateTestCase

HOME = "https://contextgraphprotocol.org"
REPO = "https://github.com/macanderson/context-graph-protocol"
CARGO = f"""[workspace]
members = []

[workspace.package]
version = "2.0.0"
repository = "{REPO}"
homepage = "{HOME}"
"""


def npm(subdir: str, **overrides) -> str:
    manifest = {
        "name": subdir,
        "version": "2.0.0",
        "homepage": HOME,
        "repository": {"type": "git", "url": f"git+{REPO}.git", "directory": f"sdk/{subdir}"},
        "bugs": {"url": f"{REPO}/issues"},
    }
    for key, value in overrides.items():
        if value is None:
            manifest.pop(key, None)
        else:
            manifest[key] = value
    return json.dumps(manifest, indent=2)


def pyproject(**urls) -> str:
    table = {"Homepage": HOME, "Repository": REPO, "Issues": f"{REPO}/issues", **urls}
    lines = "".join(f'{k} = "{v}"\n' for k, v in table.items() if v is not None)
    return f'[project]\nname = "sdk"\nversion = "2.0.0"\n\n[project.urls]\n{lines}'


GO_MOD = "module github.com/macanderson/context-graph-protocol/sdk/go\n\ngo 1.21\n"


class SdkMetadataTest(GateTestCase):
    SCRIPT = ".github/scripts/check-sdk-metadata.py"

    def setUp(self) -> None:
        super().setUp()
        self.write("Cargo.toml", CARGO)
        self.write("sdk/typescript/package.json", npm("typescript"))
        self.write("sdk/python/pyproject.toml", pyproject())
        self.write("sdk/go/go.mod", GO_MOD)

    def test_matching_metadata_passes(self):
        self.assertPasses(self.run_gate())

    # --- npm ------------------------------------------------------------------

    def test_a_missing_npm_homepage_fails(self):
        self.write("sdk/typescript/package.json", npm("typescript", homepage=None))
        self.assertFailsWith(self.run_gate(),
                             f"FAIL  sdk/typescript/package.json: homepage is {HOME}")

    def test_an_off_apex_homepage_fails(self):
        self.write("sdk/typescript/package.json",
                   npm("typescript", homepage="https://cgp.oxagen.sh"))
        self.assertFailsWith(self.run_gate(), "homepage is", "cgp.oxagen.sh")

    def test_a_string_repository_fails_because_it_cannot_deep_link(self):
        self.write("sdk/typescript/package.json", npm("typescript", repository=REPO))
        self.assertFailsWith(self.run_gate(), "FAIL  sdk/typescript/package.json: repository names")

    def test_a_repository_with_the_wrong_directory_fails(self):
        self.write("sdk/typescript/package.json", npm("typescript", repository={
            "type": "git", "url": f"git+{REPO}.git", "directory": "sdk/python"}))
        self.assertFailsWith(self.run_gate(), "at directory sdk/typescript")

    def test_a_repository_naming_another_repo_fails(self):
        self.write("sdk/typescript/package.json", npm("typescript", repository={
            "type": "git", "url": "git+https://github.com/someone/fork.git",
            "directory": "sdk/typescript"}))
        self.assertFailsWith(self.run_gate(), "FAIL  sdk/typescript/package.json: repository names")

    def test_missing_bugs_fails(self):
        self.write("sdk/typescript/package.json", npm("typescript", bugs=None))
        self.assertFailsWith(self.run_gate(), "FAIL  sdk/typescript/package.json: bugs names")

    def test_a_new_sdk_with_no_metadata_is_caught_without_being_listed(self):
        self.write("sdk/rust-lite/package.json", json.dumps({"name": "x", "version": "2.0.0"}))
        self.assertFailsWith(self.run_gate(), "FAIL  sdk/rust-lite/package.json: homepage is")

    def test_a_private_package_is_skipped(self):
        self.write("sdk/internal/package.json", json.dumps({"name": "x", "private": True}))
        result = self.run_gate()
        self.assertPasses(result)
        self.assertIn("SKIP  sdk/internal/package.json", result.stdout)

    def test_scaffold_templates_are_not_held_to_it(self):
        self.write("sdk/typescript/templates/typescript/package.json",
                   json.dumps({"name": "{{NAME}}"}))
        self.assertPasses(self.run_gate())

    # --- PyPI -----------------------------------------------------------------

    def test_a_python_homepage_pointing_at_github_fails(self):
        # The exact disagreement #112 was filed about.
        self.write("sdk/python/pyproject.toml", pyproject(Homepage=REPO))
        self.assertFailsWith(
            self.run_gate(), f"FAIL  sdk/python/pyproject.toml: [project.urls] Homepage is {HOME}")

    def test_a_missing_python_repository_url_fails(self):
        self.write("sdk/python/pyproject.toml", pyproject(Repository=None))
        self.assertFailsWith(self.run_gate(), "[project.urls] Repository is")

    # --- Go and the reference -------------------------------------------------

    def test_a_go_module_outside_the_repository_fails(self):
        self.write("sdk/go/go.mod", "module github.com/someone/cgp-go\n")
        self.assertFailsWith(self.run_gate(), "FAIL  sdk/go/go.mod: module path is")

    def test_the_reference_comes_from_cargo_toml(self):
        # Change the reference and leave the packages alone: every package now
        # disagrees, which proves the values are read, not hardcoded.
        self.write("Cargo.toml", CARGO.replace(HOME, "https://example.org"))
        self.assertFailsWith(self.run_gate(), "homepage is https://example.org")

    def test_an_empty_sdk_directory_is_reported(self):
        for rel in ("sdk/typescript/package.json", "sdk/python/pyproject.toml", "sdk/go/go.mod"):
            self.remove(rel)
        self.assertFailsWith(self.run_gate(),
                             "FAIL  sdk/ holds package manifests this check can read")


if __name__ == "__main__":
    unittest.main()

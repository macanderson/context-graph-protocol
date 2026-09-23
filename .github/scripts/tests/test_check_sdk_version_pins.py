"""Self-test for `.github/scripts/check-sdk-version-pins.py` (#98, #108).

The manifest half has guarded the scaffolder's DEFAULT_SDK pins since #98. The
prose half (#108) reads versions written in tracked Markdown, which had gone
stale where no manifest comparison could see them. Each test plants one kind
of drift in a throwaway tree and checks the guard names it.
"""
from __future__ import annotations

import json
import unittest

from _gate_harness import GateTestCase

TS_NAME = "@contextgraphprotocol/typescript-sdk"
GO_MODULE = "github.com/macanderson/context-graph-protocol/sdk/go/contextgraph"


def scaffolder(ts: str = "^2.0.0", py: str = "contextgraph-sdk>=2.0.0") -> str:
    return (
        "#!/usr/bin/env node\n"
        "const DEFAULT_SDK = {\n"
        f'  typescript: "{ts}",\n'
        f'  python: "{py}",\n'
        "};\n"
    )


class SdkVersionPinsTest(GateTestCase):
    SCRIPT = ".github/scripts/check-sdk-version-pins.py"

    def setUp(self) -> None:
        super().setUp()
        self.write("Cargo.toml",
                   '[workspace]\nmembers = ["contextgraph-types", "contextgraph-refprov"]\n\n'
                   '[workspace.package]\nversion = "2.0.0"\n')
        self.write("contextgraph-types/Cargo.toml",
                   '[package]\nname = "contextgraph-types"\nversion.workspace = true\n'
                   'publish = true\n')
        # Unpublished: nobody can depend on it, so prose about it is not a pin.
        self.write("contextgraph-refprov/Cargo.toml",
                   '[package]\nname = "contextgraph-refprov"\nversion.workspace = true\n'
                   'publish = false\n')
        self.write("sdk/typescript/package.json",
                   json.dumps({"name": TS_NAME, "version": "2.0.0"}))
        self.write("sdk/python/pyproject.toml",
                   '[project]\nname = "contextgraph-sdk"\nversion = "2.0.0"\n')
        self.write("sdk/create-contextgraph-provider/package.json",
                   json.dumps({"name": "create-contextgraph-provider", "version": "2.1.0"}))
        self.write("sdk/create-contextgraph-provider/index.js", scaffolder())
        self.write("sdk/create-contextgraph-provider/templates/typescript/package.json",
                   json.dumps({"dependencies": {TS_NAME: "{{SDK_SPEC}}"}}))
        self.write("sdk/create-contextgraph-provider/templates/python/pyproject.toml",
                   '[project]\nname = "x"\ndependencies = ["{{SDK_SPEC}}"]\n')

    def gate_with(self, **docs: str):
        for rel, text in docs.items():
            self.write(rel, text)
        self.git_init()
        return self.run_gate()

    # --- the baseline ---------------------------------------------------------

    def test_consistent_manifests_and_prose_pass(self):
        self.assertPasses(self.gate_with(**{
            "README.md": (
                f"npm install {TS_NAME}\n"
                f"npm install {TS_NAME}@^2.0.0\n"
                "pip install contextgraph-sdk>=2.0\n"
                'contextgraph-types = "2"\n'
                'contextgraph-types = { version = "=2.0.0", features = ["attestation"] }\n'
                "cargo add contextgraph-types@2\n"
                "npm create contextgraph-provider@latest my-provider\n"
                "npx create-contextgraph-provider@2.1.0 my-provider\n"
                f"go get {GO_MODULE}\n"
                f"go get {GO_MODULE}@latest\n"
                f"go get {GO_MODULE}@vX.Y.Z\n"
            ),
        }))

    # --- the manifest half (#98) ----------------------------------------------

    def test_a_default_sdk_pin_a_major_behind_fails(self):
        self.write("sdk/create-contextgraph-provider/index.js", scaffolder(ts="^1.0.0"))
        self.assertFailsWith(self.gate_with(),
                             "FAIL  typescript pin shares the major sdk/typescript/package.json ships")

    def test_an_sdk_major_split_from_the_crates_fails(self):
        self.write("sdk/python/pyproject.toml",
                   '[project]\nname = "contextgraph-sdk"\nversion = "3.0.0"\n')
        self.write("sdk/create-contextgraph-provider/index.js",
                   scaffolder(py="contextgraph-sdk>=3.0.0"))
        self.assertFailsWith(self.gate_with(), "FAIL  the SDKs and the crates share one major")

    # --- the prose half (#108) ------------------------------------------------

    def test_a_stale_npm_pin_in_prose_fails(self):
        result = self.gate_with(**{"docs/install.md": f"npm install {TS_NAME}@0.1.0\n"})
        self.assertFailsWith(
            result,
            "FAIL  every version written in prose shares its manifest's major",
            "docs/install.md:1",
            "sdk/typescript/package.json ships 2.0.0",
        )

    def test_a_pip_pin_ahead_of_the_manifest_fails(self):
        result = self.gate_with(**{"README.md": "pip install contextgraph-sdk==2.4.0\n"})
        self.assertFailsWith(result, "README.md:1", "sdk/python/pyproject.toml ships 2.0.0")

    def test_a_crate_dependency_line_on_an_old_major_fails(self):
        # The exact line #108's sweep found in docs/protocol-advantages.md.
        result = self.gate_with(**{
            "docs/protocol-advantages.md": 'pin `contextgraph-types = "=0.1.0"` for a guarantee\n',
        })
        self.assertFailsWith(result, "docs/protocol-advantages.md:1",
                             "Cargo.toml [workspace.package] ships 2.0.0")

    def test_a_table_form_crate_dependency_is_read(self):
        result = self.gate_with(**{
            "README.md": 'contextgraph-types = { version = "1.4", features = ["x"] }\n',
        })
        self.assertFailsWith(result, "README.md:1")

    def test_cargo_add_with_an_old_version_fails(self):
        result = self.gate_with(**{"README.md": "cargo add contextgraph-types@1.0.0\n"})
        self.assertFailsWith(result, "README.md:1")

    def test_an_unpublished_crate_is_not_a_pin(self):
        self.assertPasses(self.gate_with(**{"README.md": 'contextgraph-refprov = "0.1"\n'}))

    def test_the_scaffolder_is_held_to_its_own_manifest(self):
        result = self.gate_with(**{"README.md": "npm create contextgraph-provider@1 x\n"})
        self.assertFailsWith(result, "sdk/create-contextgraph-provider/package.json ships 2.1.0")

    def test_a_concrete_go_version_in_prose_fails(self):
        # The acceptance-bar line #108 was filed about.
        result = self.gate_with(**{
            "sdk/PUBLISHING.md": f"go get {GO_MODULE}@v0.1.0\n",
        })
        self.assertFailsWith(
            result,
            "FAIL  no prose pins a concrete Go SDK version",
            "sdk/PUBLISHING.md:1",
        )

    def test_a_go_pin_matching_the_crates_still_fails(self):
        # Go has no manifest version to agree with, so even a "right-looking"
        # version is unverifiable offline and is refused.
        result = self.gate_with(**{"README.md": f"go get {GO_MODULE}@v2.0.0\n"})
        self.assertFailsWith(result, "FAIL  no prose pins a concrete Go SDK version")

    def test_historical_records_are_exempt(self):
        stale = f"npm install {TS_NAME}@0.1.0\ngo get {GO_MODULE}@v0.1.0\n"
        self.assertPasses(self.gate_with(**{
            "CHANGELOG.md": stale,
            "docs/adr/0012-sdk-version-pins.md": stale,
        }))

    def test_the_exemption_does_not_leak(self):
        result = self.gate_with(**{"docs/adrs.md": f"npm install {TS_NAME}@0.1.0\n"})
        self.assertFailsWith(result, "docs/adrs.md:1")

    def test_only_tracked_markdown_is_read(self):
        self.git_init()
        # Untracked, as a node_modules README would be.
        self.write("sdk/typescript/node_modules/dep/README.md",
                   f"npm install {TS_NAME}@0.1.0\n")
        self.assertPasses(self.run_gate())


if __name__ == "__main__":
    unittest.main()

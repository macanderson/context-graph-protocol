"""Self-test for `.github/scripts/check-deploy-hygiene.py` (#110).

The guard fails open when it narrows: a URL its pattern cannot see is neither an
offender nor a missing artifact, so the run is a green PASS that looks exactly
like a healthy one. That already happened once — the pattern required the file
to sit directly under `schema/`, so the versioned `…/schema/v1/…` identity URLs
matched nothing (#79). These tests pin each direction the guard must fail in, so
the next narrowing goes red here instead of silently.
"""
from __future__ import annotations

import json
import unittest

from _gate_harness import GateTestCase

APEX = "https://contextgraphprotocol.org"
RAW = "https://raw.githubusercontent.com/macanderson/context-graph-protocol/main"
SCHEMA = "contextgraph-envelope.schema.json"
VECTORS = "reference-vectors.ndjson"
# A host this repository does not serve. Every URL in this file is assembled
# from these constants rather than written out: the real hygiene gate scans
# this file too, and a literal offender here would fail it.
UNSERVED = "https://cgp.oxagen.sh"


class DeployHygieneTest(GateTestCase):
    SCRIPT = ".github/scripts/check-deploy-hygiene.py"

    def setUp(self) -> None:
        super().setUp()
        # The one artifact the served-prefix URLs below resolve to.
        self.write(f"schema/{SCHEMA}", "{}\n")

    def gate_with(self, **files: str):
        for rel, text in files.items():
            self.write(rel, text)
        self.git_init()
        return self.run_gate()

    # --- the baseline, so every failure below is attributable --------------

    def test_a_tree_advertising_only_served_existing_artifacts_passes(self):
        self.assertPasses(self.gate_with(**{
            "README.md": f"raw: {RAW}/schema/{SCHEMA}\n"
                         f"apex: {APEX}/schema/{SCHEMA}\n"
                         f"identity: {APEX}/schema/v1/{SCHEMA}\n",
        }))

    # --- the offender direction ---------------------------------------------

    def test_an_artifact_url_on_an_unserved_prefix_is_an_offender(self):
        result = self.gate_with(**{
            "docs/registry.md": f"badge: {APEX}/badges/conformant.svg\n",
        })
        self.assertFailsWith(
            result,
            "FAIL  no artifact URL on a prefix this repo does not publish",
            "docs/registry.md:1",
        )

    def test_an_unserved_host_is_an_offender(self):
        result = self.gate_with(**{
            "README.md": f"{UNSERVED}/schema/{SCHEMA}\n",
        })
        self.assertFailsWith(
            result, "FAIL  no artifact URL on a prefix this repo does not publish")

    # --- the missing-artifact direction ------------------------------------

    def test_a_served_url_naming_a_missing_file_is_reported_missing(self):
        result = self.gate_with(**{
            "README.md": f"{RAW}/schema/not-a-real.schema.json\n",
        })
        self.assertFailsWith(
            result,
            "FAIL  every advertised artifact exists at the path its URL names",
            "README.md:1",
        )

    # --- the blind spot that motivated this suite --------------------------

    def test_a_versioned_url_is_seen_at_all(self):
        # On an unserved host, so the only way to pass is to not match it.
        result = self.gate_with(**{
            "README.md": f"{UNSERVED}/schema/v1/{SCHEMA}\n",
        })
        self.assertFailsWith(
            result,
            "FAIL  no artifact URL on a prefix this repo does not publish",
            f"/schema/v1/{SCHEMA}",
        )

    def test_a_versioned_url_naming_a_missing_file_is_reported_missing(self):
        result = self.gate_with(**{
            "README.md": f"{APEX}/schema/v1/gone.schema.json\n",
        })
        self.assertFailsWith(
            result,
            "FAIL  every advertised artifact exists at the path its URL names",
            "/schema/v1/gone.schema.json",
        )

    def test_a_nested_prefix_resolves_against_the_longest_matching_row(self):
        # `/schema/` also prefixes this URL. Resolved against that row it would
        # look for `schema/v1/<name>`, which deliberately does not exist, and
        # report a healthy URL missing.
        self.assertPasses(self.gate_with(**{
            "README.md": f"{APEX}/schema/v1/{SCHEMA}\n",
        }))

    # --- the reference vectors (#111) ---------------------------------------

    def test_a_published_vectors_url_resolves_on_both_schema_paths(self):
        self.write(f"schema/{VECTORS}", "{}\n")
        self.assertPasses(self.gate_with(**{
            "PUBLISHING.md": f"{APEX}/schema/v1/{VECTORS}\n"
                             f"{APEX}/schema/{VECTORS}\n",
        }))

    def test_an_ndjson_url_is_seen_at_all(self):
        result = self.gate_with(**{
            "README.md": f"{UNSERVED}/schema/v1/{VECTORS}\n",
        })
        self.assertFailsWith(
            result,
            "FAIL  no artifact URL on a prefix this repo does not publish",
            "reference-vectors.ndjson",
        )

    def test_an_ndjson_url_naming_a_missing_file_is_reported_missing(self):
        result = self.gate_with(**{
            "README.md": f"{APEX}/schema/v1/{VECTORS}\n",
        })
        self.assertFailsWith(
            result, "FAIL  every advertised artifact exists at the path its URL names")

    # --- historical records are exempt -------------------------------------

    def test_adr_and_changelog_citations_are_exempt(self):
        bad = f"{APEX}/badges/conformant.svg and {RAW}/schema/gone.json\n"
        self.assertPasses(self.gate_with(**{
            "docs/adr/0008-deploy-topology.md": bad,
            "CHANGELOG.md": bad,
        }))

    def test_the_exemption_does_not_leak_to_other_docs(self):
        result = self.gate_with(**{
            "docs/adrs-are-not-this.md": f"{APEX}/badges/conformant.svg\n",
        })
        self.assertFailsWith(result, "docs/adrs-are-not-this.md:1")

    # --- the Vercel link half ----------------------------------------------

    def test_a_committed_vercel_link_fails(self):
        result = self.gate_with(**{
            ".vercel/project.json": json.dumps({"projectId": "prj_other"}),
        })
        self.assertFailsWith(result, "FAIL  no Vercel link is committed")

    def test_a_local_link_to_the_apex_project_fails(self):
        self.git_init()
        # Written after `git add`, so it is untracked — the invisible case.
        self.write("site/.vercel/project.json",
                   json.dumps({"projectId": "prj_s3lfCDvK9H9PwgvkpXiho1juiR63"}))
        result = self.run_gate()
        self.assertFailsWith(
            result, "FAIL  no local checkout is linked to the apex project")


if __name__ == "__main__":
    unittest.main()

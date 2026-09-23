"""Self-test for `.github/scripts/check-doc-links.py` (#153).

Each case builds a scratch git tree with a known answer and runs the checker's
`dangling()` over it, so a regex change that silently stops matching — or
starts matching everything — fails here before it can report a false green
against the real tree.
"""

from __future__ import annotations

import importlib.util
import subprocess
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[1] / "check-doc-links.py"
_spec = importlib.util.spec_from_file_location("check_doc_links", SCRIPT)
assert _spec is not None and _spec.loader is not None
check_doc_links = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(check_doc_links)

REAL = {"docs/real.md": "# real\n", "docs/adr/0001-x.md": "# x\n"}


def run(tree: dict[str, str], foreign: dict[str, str] | None = None) -> list[str]:
    """Problems reported for `tree`, each reduced to `file:line` or the
    FOREIGN key it concerns."""
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        for rel, body in {**REAL, **tree}.items():
            path = root / rel
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(body, encoding="utf-8")
        subprocess.run(["git", "init", "-q"], cwd=root, check=True)
        subprocess.run(["git", "add", "-A"], cwd=root, check=True)
        problems = check_doc_links.dangling(root, foreign or {})
    return [
        p.split("`")[1] if p.startswith("FOREIGN") else p.split(": ", 1)[0]
        for p in problems
    ]


class CheckDocLinks(unittest.TestCase):
    def test_a_clean_tree_passes(self) -> None:
        self.assertEqual(run({"README.md": "See docs/real.md and docs/adr/.\n"}), [])

    def test_a_missing_file_fails_naming_file_and_line(self) -> None:
        self.assertEqual(run({"README.md": "ok\nSee `docs/gone/away.md`.\n"}), ["README.md:2"])

    def test_a_cargo_description_is_read(self) -> None:
        tree = {"crate/Cargo.toml": 'description = "see docs/gone.md."\n'}
        self.assertEqual(run(tree), ["crate/Cargo.toml:1"])

    def test_a_rust_doc_comment_is_read(self) -> None:
        tree = {"crate/src/lib.rs": "//! Implements docs/gone.md.\n"}
        self.assertEqual(run(tree), ["crate/src/lib.rs:1"])

    def test_a_rust_string_literal_is_test_data(self) -> None:
        tree = {"crate/src/lib.rs": 'let p = "docs/adr/0006-x.md";\n'}
        self.assertEqual(run(tree), [])

    def test_a_test_sources_comments_describe_fixtures(self) -> None:
        tree = {"crate/tests/vectors.rs": '// uri "docs/fixture/x.md"\n'}
        self.assertEqual(run(tree), [])

    def test_a_python_test_source_is_not_read(self) -> None:
        tree = {".github/scripts/tests/test_x.py": 'TREE = {"README.md": "docs/gone.md"}\n'}
        self.assertEqual(run(tree), [])

    def test_json_is_data(self) -> None:
        self.assertEqual(run({"vectors.json": '{"path": "docs/gone.md"}\n'}), [])

    def test_fragment_stripped_and_punctuation_not_swallowed(self) -> None:
        tree = {"README.md": "See docs/real.md#section. Also docs/real.md.\n"}
        self.assertEqual(run(tree), [])

    def test_a_relative_link_resolves_against_the_citing_file(self) -> None:
        self.assertEqual(run({"crate/src/lib.rs": "//! [a](../../docs/real.md)\n"}), [])

    def test_a_relative_link_with_the_wrong_depth_fails(self) -> None:
        tree = {"crate/src/lib.rs": "//! [a](../docs/real.md)\n"}
        self.assertEqual(run(tree), ["crate/src/lib.rs:1"])

    def test_an_absolute_link_into_this_repository_is_checked(self) -> None:
        url = "https://github.com/macanderson/context-graph-protocol/blob/main/docs/gone.md"
        self.assertEqual(run({"README.md": url + "\n"}), ["README.md:1"])

    def test_a_docs_path_in_another_repositorys_url_is_skipped(self) -> None:
        tree = {"README.md": "https://github.com/someone/else/blob/main/docs/gone.md\n"}
        self.assertEqual(run(tree), [])

    def test_a_glob_or_placeholder_is_a_pattern(self) -> None:
        tree = {"README.md": "Each docs/adr/*.md, docs/adr/NNNN-<slug>.md, docs/{a,b}.md\n"}
        self.assertEqual(run(tree), [])

    def test_the_changelog_is_history(self) -> None:
        self.assertEqual(run({"CHANGELOG.md": "Removed docs/gone/.\n"}), [])

    def test_a_foreign_prefix_excuses_another_repositorys_path(self) -> None:
        tree = {"README.md": "stella's docs/design/spec.md\n"}
        self.assertEqual(run(tree, {"docs/design/": "stella"}), [])

    def test_a_foreign_entry_nothing_cites_is_an_error(self) -> None:
        tree = {"README.md": "See docs/real.md.\n"}
        self.assertEqual(run(tree, {"docs/design/": "stella"}), ["docs/design/"])

    def test_a_foreign_entry_naming_a_local_path_is_an_error(self) -> None:
        tree = {"README.md": "See docs/real.md.\n"}
        # Reported twice: it matches a local path, and (being local) it is
        # never the reason a reference passed, so nothing uses it either.
        self.assertEqual(
            run(tree, {"docs/real.md": "not foreign"}), ["docs/real.md", "docs/real.md"]
        )

    def test_the_real_foreign_entries_are_all_foreign(self) -> None:
        # The shipped list must never name a path this repository has.
        root = SCRIPT.parents[2]
        for prefix in check_doc_links.FOREIGN:
            self.assertFalse((root / prefix).exists(), prefix)


if __name__ == "__main__":
    unittest.main()

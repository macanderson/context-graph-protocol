"""Unit tests for `.github/scripts/check-published-crates.py` (#143)."""

from __future__ import annotations

import importlib.util
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[1] / "check-published-crates.py"
_spec = importlib.util.spec_from_file_location("check_published_crates", SCRIPT)
assert _spec is not None and _spec.loader is not None
check_published_crates = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(check_published_crates)


def manifest(name: str, publish: str | None) -> str:
    lines = ["[package]", f'name = "{name}"', 'version = "2.0.0"']
    if publish is not None:
        lines.append(f"publish = {publish}  # why")
    lines += ["", "[dependencies]", 'serde = "1"']
    return "\n".join(lines) + "\n"


class PublishedCrates(unittest.TestCase):
    def scan(self, manifests: dict[str, str]) -> set[str]:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            for directory, text in manifests.items():
                (root / directory).mkdir()
                (root / directory / "Cargo.toml").write_text(text, encoding="utf-8")
            return check_published_crates.published_crates(root)

    def test_only_publish_true_counts(self) -> None:
        found = self.scan(
            {
                "a": manifest("crate-a", "true"),
                "b": manifest("crate-b", "false"),
                "c": manifest("crate-c", None),
            }
        )
        self.assertEqual(found, {"crate-a"})

    def test_a_commented_out_publish_does_not_count(self) -> None:
        text = manifest("crate-a", None).replace("[dependencies]", "# publish = true\n[dependencies]")
        self.assertEqual(self.scan({"a": text}), set())


class Missing(unittest.TestCase):
    CRATES = {"contextgraph-types", "contextgraph-trace"}

    def test_a_document_naming_every_crate_passes(self) -> None:
        text = "Covered: `contextgraph-types` and `contextgraph-trace`."
        self.assertEqual(check_published_crates.missing(self.CRATES, text), [])

    def test_an_omitted_crate_is_reported(self) -> None:
        text = "Covered: `contextgraph-types`."
        self.assertEqual(check_published_crates.missing(self.CRATES, text), ["contextgraph-trace"])

    def test_a_longer_name_is_not_a_mention(self) -> None:
        text = "`contextgraph-types-extra` and `contextgraph-trace`"
        self.assertEqual(check_published_crates.missing(self.CRATES, text), ["contextgraph-types"])


if __name__ == "__main__":
    unittest.main()

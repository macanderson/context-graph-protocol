"""Witness for the Next.js/Fumadocs documentation site requested at repository root."""

import json
import re
import unittest
from pathlib import Path


REPO = Path(__file__).resolve().parents[1]
DOC_NAMES = {path.name for path in (REPO / "docs").glob("*.md")}


class DocumentationSiteWitness(unittest.TestCase):
    def test_fumadocs_site_exposes_existing_docs_logo_and_neutral_theme(self):
        problems = []
        manifests = []
        for manifest in REPO.rglob("package.json"):
            if "node_modules" in manifest.parts:
                continue
            try:
                package = json.loads(manifest.read_text(encoding="utf-8"))
            except (OSError, json.JSONDecodeError):
                continue
            dependencies = {
                **package.get("dependencies", {}),
                **package.get("devDependencies", {}),
            }
            if "next" in dependencies:
                manifests.append((manifest, dependencies))

        if not manifests:
            problems.append("no Next.js package.json exists")
        else:
            manifest, dependencies = manifests[0]
            site = manifest.parent
            if not any(name.startswith("fumadocs-") for name in dependencies):
                problems.append("the Next.js app does not depend on Fumadocs")

            authored = [
                path
                for path in site.rglob("*")
                if path.is_file()
                and "node_modules" not in path.parts
                and path.suffix in {".ts", ".tsx", ".js", ".jsx", ".md", ".mdx", ".css"}
            ]
            source = "\n".join(
                path.read_text(encoding="utf-8", errors="ignore") for path in authored
            )

            represented_docs = {
                path.name
                for path in site.rglob("*")
                if path.is_file() and path.suffix in {".md", ".mdx"}
            }
            uses_repository_docs = bool(
                re.search(r"(?:dir|directory|path)\s*:\s*['\"](?:\.\./)+docs/?['\"]", source)
            )
            if not (DOC_NAMES <= represented_docs or uses_repository_docs):
                missing = sorted(DOC_NAMES - represented_docs)
                problems.append(
                    "the Fumadocs content source does not include every docs/ page: "
                    + ", ".join(missing)
                )

            if "contextgraph-logo.svg" not in source:
                problems.append("the site never renders the repository's Context Graph Protocol logo")

            theme_files = [path for path in authored if path.suffix == ".css"]
            theme = "\n".join(
                path.read_text(encoding="utf-8", errors="ignore") for path in theme_files
            )
            for role in ("primary", "accent", "foreground"):
                if not re.search(rf"--(?:color-(?:fd-)?)?{role}\s*:", theme):
                    problems.append(f"the design system does not define a --{role} theme token")

            chromatic_tailwind = re.search(
                r"(?:bg|text|border|from|to|via)-(?:red|orange|amber|yellow|lime|green|emerald|teal|cyan|sky|blue|indigo|violet|purple|fuchsia|pink|rose)-\d+",
                source,
            )
            if chromatic_tailwind:
                problems.append(
                    f"the neutral-only theme uses chromatic utility {chromatic_tailwind.group(0)!r}"
                )

        self.assertFalse(problems, "Documentation site is incomplete:\n- " + "\n- ".join(problems))


if __name__ == "__main__":
    unittest.main()

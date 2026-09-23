"""Self-test for check-dco.py (#141, ADR 0025).

Real commits in a scratch repository, so the trailer parsing is exercised the
way CI exercises it (`git log --format=%(trailers…)`), not through a mock. The
witness #141 asks for is `test_an_unsigned_commit_fails_and_signing_it_passes`:
a range with one unsigned commit goes red, and the same range with that commit
signed off goes green.
"""
from __future__ import annotations

import importlib.util
import io
import os
import subprocess
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent.parent / "check-dco.py"
spec = importlib.util.spec_from_file_location("check_dco", SCRIPT)
assert spec and spec.loader
guard = importlib.util.module_from_spec(spec)
spec.loader.exec_module(guard)

ADA = ("Ada Lovelace", "ada@example.invalid")
BOT = ("some-agent[bot]", "123+some-agent[bot]@users.noreply.github.com")


class DcoTest(unittest.TestCase):
    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory()
        self.root = Path(self._tmp.name)
        self.base_env = {
            **os.environ,
            "GIT_CONFIG_GLOBAL": os.devnull,
            "GIT_CONFIG_NOSYSTEM": "1",
        }
        self.git(ADA, "init", "-q", "-b", "main")
        self.commit(ADA, "base", signoff=None)
        self.base = self.git(ADA, "rev-parse", "HEAD").strip()
        self.counter = 0

    def tearDown(self) -> None:
        self._tmp.cleanup()

    def git(self, who: tuple[str, str], *args: str) -> str:
        name, email = who
        env = {
            **self.base_env,
            "GIT_AUTHOR_NAME": name,
            "GIT_AUTHOR_EMAIL": email,
            "GIT_COMMITTER_NAME": name,
            "GIT_COMMITTER_EMAIL": email,
        }
        return subprocess.run(
            ["git", "-C", str(self.root), *args],
            env=env,
            check=True,
            capture_output=True,
            text=True,
        ).stdout

    def commit(self, author: tuple[str, str], subject: str, signoff: str | None) -> None:
        path = self.root / f"{subject.replace(' ', '-')}.txt"
        path.write_text(subject, encoding="utf-8")
        self.git(author, "add", "-A")
        message = subject if signoff is None else f"{subject}\n\nSigned-off-by: {signoff}"
        self.git(author, "commit", "-q", "-m", message)

    def check(self) -> tuple[int, str]:
        out = io.StringIO()
        with redirect_stdout(out):
            code = guard.main(["--root", str(self.root), "--range", f"{self.base}..HEAD"])
        return code, out.getvalue()

    def test_a_commit_signed_off_by_its_author_passes(self) -> None:
        self.commit(ADA, "signed", signoff="Ada Lovelace <ada@example.invalid>")
        code, out = self.check()
        self.assertEqual(code, 0, out)

    def test_email_comparison_ignores_case(self) -> None:
        self.commit(ADA, "signed", signoff="Ada Lovelace <Ada@Example.invalid>")
        code, out = self.check()
        self.assertEqual(code, 0, out)

    def test_an_unsigned_commit_fails_and_signing_it_passes(self) -> None:
        self.commit(ADA, "signed", signoff="Ada Lovelace <ada@example.invalid>")
        self.commit(ADA, "unsigned", signoff=None)
        code, out = self.check()
        self.assertEqual(code, 1)
        self.assertIn("1 of 2 commit(s)", out)
        self.assertIn("'unsigned' has no Signed-off-by trailer", out)
        self.assertIn("git rebase --signoff", out)

        # The documented remedy, applied: every commit in the range signed.
        self.git(ADA, "rebase", "-q", "--signoff", self.base)
        code, out = self.check()
        self.assertEqual(code, 0, out)

    def test_a_sign_off_by_someone_else_fails(self) -> None:
        self.commit(ADA, "borrowed", signoff="Grace Hopper <grace@example.invalid>")
        code, out = self.check()
        self.assertEqual(code, 1)
        self.assertIn("but authored by <ada@example.invalid>", out)

    def test_a_sign_off_mentioned_in_the_body_is_not_a_trailer(self) -> None:
        self.commit(
            ADA,
            "prose",
            signoff=None,
        )
        # Amend with a body that talks about sign-off but ends in prose, so
        # git does not parse a trailer block.
        self.git(
            ADA,
            "commit",
            "-q",
            "--amend",
            "-m",
            "prose\n\nSigned-off-by: Ada Lovelace <ada@example.invalid>\nis what I forgot to add.\n\nThe end.",
        )
        code, out = self.check()
        self.assertEqual(code, 1, out)

    def test_an_app_commit_needs_a_sign_off_but_not_its_own(self) -> None:
        self.commit(BOT, "app unsigned", signoff=None)
        code, out = self.check()
        self.assertEqual(code, 1)
        self.assertIn("authored by a GitHub App", out)

        self.git(BOT, "commit", "-q", "--amend", "-m", "app signed\n\nSigned-off-by: Ada Lovelace <ada@example.invalid>")
        code, out = self.check()
        self.assertEqual(code, 0, out)

    def test_merge_commits_are_skipped(self) -> None:
        self.git(ADA, "checkout", "-q", "-b", "side")
        self.commit(ADA, "side work", signoff="Ada Lovelace <ada@example.invalid>")
        self.git(ADA, "checkout", "-q", "main")
        self.commit(ADA, "main work", signoff="Ada Lovelace <ada@example.invalid>")
        self.git(ADA, "merge", "-q", "--no-ff", "-m", "unsigned merge", "side")
        code, out = self.check()
        self.assertEqual(code, 0, out)
        self.assertIn("2 commit(s)", out)

    def test_an_empty_range_passes(self) -> None:
        code, out = self.check()
        self.assertEqual(code, 0, out)


if __name__ == "__main__":
    unittest.main()

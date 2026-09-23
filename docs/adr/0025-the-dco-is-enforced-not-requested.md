# 25. The DCO is enforced, not requested

- Status: accepted
- Date: 2026-09-23
- Resolves [#141](https://github.com/macanderson/context-graph-protocol/issues/141).
  Keeps the licensing model `CONTRIBUTING.md` and `GOVERNANCE.md` already
  describe (DCO, no CLA, no copyright assignment) and makes it operate.

## Context

This repository tells every contributor to sign off every commit
(`git commit -s`) to certify the [Developer Certificate of
Origin](https://developercertificate.org/). It makes the sign-off load-bearing:
`CONTRIBUTING.md`'s License section and the pull request template both say a
contribution is licensed MIT OR Apache-2.0 "as certified by your DCO sign-off",
and `GOVERNANCE.md` says contributions land "under the DCO".

Nothing checked it. When #141 was filed, 21 of the 81 commits on `main` carried
a `Signed-off-by` trailer, and none of the nine merges after #76 did. No
workflow read commit trailers and no DCO app reported on merged pull requests.

A licensing rule stated as mandatory, not enforced, and not followed by the
maintainer's own recent merges is worse than no rule. It reads to an adopter's
lawyer as a policy in force, and the history does not support it. For a project
whose pitch is "enforced by contract, not convention", it was the largest
convention-only claim left in the tree.

#141 named the two honest resolutions: **enforce** the rule, or **drop** it and
rewrite the License paragraph so the grant rests on the contribution itself.

## Decision

**1. The rule stays, and a required check enforces it.** The `dco` job in
`ci.yml` runs `.github/scripts/check-dco.py` over the commits a pull request
adds (`base..head`). Every non-merge commit must carry a `Signed-off-by:`
trailer naming its author's email, compared case-insensitively. Merge commits
are skipped; they add no authored change of their own.

**2. A GitHub App's commit needs a sign-off, but not its own.** A commit
authored by an app (`name[bot]`) passes with any `Signed-off-by:` trailer, and
fails with none. An app cannot certify anything. The trailer names the human,
or the operator (Dependabot signs as `support@github.com`), who does. This
matters here more than in most repositories, because agents author much of this
history. Exempting apps would leave the largest share of new commits
uncertified, which would recreate the gap this record closes.

**3. The cut-over is this change, and history is not rewritten.** The check
reads only the commits each pull request adds, so every commit that lands after
this record is checked and none before it is. Rewriting merged history is not
an option: it would break every SHA cited in issues, changelogs and downstream
pins. The commits before the cut-over that lack a sign-off are contributions
made under the stated terms, as far as those terms went at the time. This
record does not retroactively certify them.

**4. The certification is made on the pull request's commits.** The
repository squash-merges, and GitHub writes the squashed commit on `main`
itself. That commit may or may not carry the trailers, depending on the
repository's squash-message setting. The sign-off that counts is the one the
contributor made on the commits they pushed. Those commits stay addressable
under the pull request (`refs/pull/N/head`), and the check result is recorded
on the pull request. A check on `push` to `main` would be judging GitHub's
rewrite, not the contributor's act, so the check does not run there.

**5. The fix for a failing pull request is mechanical and documented.**
`git rebase --signoff <base>` followed by `git push --force-with-lease`. The
check prints both commands, and `CONTRIBUTING.md` states them.

## Why not drop the requirement

Dropping it is a real change to the licensing story, not a documentation tidy.
The DCO was chosen over a CLA on purpose, so that contributors keep their
copyright and certify provenance per change instead of signing an agreement
once. Removing it would leave the license grant resting only on the pull
request template's paragraph. That is a weaker and less standard basis, and
adopters would have to re-evaluate it.

Enforcing costs one trailer per commit, which `-s` adds automatically, and a
rebase when it is forgotten. The DCO app, the Linux kernel and most
foundation-hosted projects make the same trade. Of the two options, enforcing
is the one still right in ten years.

The sibling repository `macanderson/stella` made the opposite choice: a CLA,
under which a `Signed-off-by` trailer carries no meaning. The two repositories
have different contributors and different licenses at stake, so they may
legitimately differ. This record decides only for this repository.

## Consequences

- `CONTRIBUTING.md`, `GOVERNANCE.md` and the pull request template describe
  the rule that now operates. `CONTRIBUTING.md` names the cut-over and the fix.
- Every author must sign off, including automated authors and agents. For an
  app, that means the human or operator behind it. Tooling that opens pull
  requests (agents, bots) must add the trailer, or its pull requests stay red.
- The check verifies that a trailer is present and matches the author's
  identity. It cannot verify the legal capacity of whoever signed, and no
  mechanical check can. The certification is the signer's, and the check makes
  sure one was made.
- To make the check blocking, the maintainer marks `every commit is signed off
  (DCO)` as a required status check in branch protection. That is a repository
  setting, outside the tree.

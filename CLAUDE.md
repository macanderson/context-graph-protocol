# CLAUDE.md

@AGENTS.md

Repo-wide agent guidance lives in `AGENTS.md` (imported above) so that every
agent — Claude Code and Stella alike — loads the same steering context,
including the standing-decisions block. The records live in the oxagen
workspace (`.oxagen/rules/`); this repository does not carry a copy.

The Local execution section in `AGENTS.md` bars local builds, tests, dev servers, Docker, Biome, and git hooks on this machine.

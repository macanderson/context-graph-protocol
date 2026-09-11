# The CI toolchain, and how to bump it

This repository pins the Rust toolchain that CI lints and tests with.
[`rust-toolchain.toml`](../rust-toolchain.toml) at the root names one concrete
release, and every workflow job installs exactly that release through the
[`./.github/actions/rust`](../.github/actions/rust/action.yml) composite action.

## Two numbers that are not the same number

| | Where | What it means | Who moves it |
|---|---|---|---|
| **Toolchain pin** | `rust-toolchain.toml` → `channel` | The compiler CI lints and tests with, and the one you get locally | Us, in a deliberate bump PR |
| **MSRV** | `Cargo.toml` → `[workspace.package] rust-version` | The oldest compiler the published crates promise to support | Us, as a semver-relevant decision |

They answer different questions and move independently. Raising the pin says
nothing about what a downstream consumer needs; it is not a signal to raise the
MSRV, and doing so would break a promise to consumers who have not upgraded.
The `msrv` CI job proves the floor separately, building the workspace on
`rust-version` rather than on the pin.

## Why pin at all

Before this, every job ran `dtolnay/rust-toolchain@stable`, so the compiler
moved on Rust's release schedule rather than ours. clippy runs at
`-D warnings`, so each new warn-by-default lint became an unannounced red
`main` — landing on whichever pull request happened to be open at the time,
whose author then paid for a change they had not made. Rust 1.98's
`clippy::chunks_exact_to_as_chunks` did exactly that to
`contextgraph-types/src/attest.rs`, code that had been green and untouched
since #87; #114 absorbed the fix only because its own CI could not go green
otherwise. `rustfmt` has the same shape and is harder to diagnose, because a
new stable rustfmt reformats previously-clean files and the diff reads as the
author's work.

Pinning does not stop us tracking stable. It makes tracking stable a decision
with a commit attached, instead of an ambush on a contributor (#160).

## Bumping the pin

Do this as **one pull request that does nothing else**, so the lint and
formatting churn a new compiler brings is attributable to the bump rather than
smeared across an unrelated change.

1. Edit `channel` in `rust-toolchain.toml` to the new release. Use a full
   `x.y.z` version — `stable` floats, and a two-component `1.98` resolves to
   the newest `1.98.x`, so neither is a pin. CI rejects both.

2. Install it and re-run the gates locally:

   ```sh
   rustup toolchain install "$(sed -n 's/^channel = "\(.*\)"/\1/p' rust-toolchain.toml)"
   cargo fmt --all
   cargo clippy --workspace --all-targets -- -D warnings
   cargo clippy -p contextgraph-types --all-features --all-targets -- -D warnings
   ```

3. **Commit the `cargo fmt --all` churn in this same PR.** A new stable
   rustfmt reformats files nobody touched; letting that land later attaches it
   to an unrelated diff and hides the real change inside it.

4. Fix the new clippy lints here too, in the same PR. If a lint is wrong for
   this codebase, `#[allow]` it at the narrowest scope with a comment saying
   why — not at the crate root, and not by lowering `-D warnings`.

5. Sanity-check the MSRV job is still green. It builds on `rust-version`, not
   the pin, so a new pin should not move it. If it does, something set
   `RUSTUP_TOOLCHAIN` or the override was dropped — see below.

6. Note the bump under `## [Unreleased]` in `CHANGELOG.md`. It is a repository
   milestone, not a crate release: the published crates are unaffected, because
   the pin is not the MSRV.

## Working locally

rustup reads `rust-toolchain.toml` automatically, so `cargo build` in a fresh
clone installs and uses the pinned release with `rustfmt` and `clippy`
attached. You do not need to select a toolchain by hand, and you should not
`rustup override set` in this checkout — that outranks the file and puts you on
a compiler CI is not using.

To check what you are actually on:

```sh
rustup show active-toolchain
```

## The one job that must not use the pin

rustup resolves overrides in this order, highest first:

1. `cargo +1.90.0 …` on the command line
2. the `RUSTUP_TOOLCHAIN` environment variable
3. a directory override from `rustup override set`
4. **`rust-toolchain.toml`**
5. the rustup default toolchain

The `msrv` job installs the MSRV and makes it the rustup *default* — rank 5,
which `rust-toolchain.toml` at rank 4 outranks. So the moment this pin existed,
that job would have built on the pinned compiler while reporting that the
crates build on their oldest supported one: still green, no longer meaning
anything. It sets `RUSTUP_TOOLCHAIN` (rank 2) to the MSRV to override the file.

`.github/scripts/check-toolchain-pin.py` enforces all of this on every PR: the
pin is one concrete release and not older than the MSRV, no workflow installs a
floating channel, and the `msrv` job keeps its override. Each of those
regressions is invisible in review and silent at merge time, which is why they
are a gate rather than a convention.

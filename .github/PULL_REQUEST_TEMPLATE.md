# Pull request

<!-- Thanks for contributing! Keep it one logical change per PR. -->

## Summary

<!-- What does this change do, and why? One or two sentences. -->

## What changed

<!-- Bullet list of the substantive changes. Link any relevant issue (`Closes #123`). -->

## Checklist

- [ ] One logical change per PR (smaller lands faster)
- [ ] Gate is green locally — `fmt`, `clippy -D warnings`, `test`
- [ ] A witness test is included, or a reason there isn't one is stated below
- [ ] Docs updated in the same PR if behavior or flags changed (`README.md`,
      `docs/`, doc comments, `--help` text)
- [ ] All commits signed off (`git commit -s`, DCO)
- [ ] `CHANGELOG.md` updated under `[Unreleased]` if user-visible

## Protocol-stability impact (if a spec/wire change)

- [ ] Not applicable — no wire or spec change
- [ ] Additive (new optional field/check) — safe within `contextgraph/1`
- [ ] Wire-breaking — requires `contextgraph/2`; explain below

<!-- If this touches the wire shape or a conformance check, note it here and
     say how it stays compatible with contextgraph/1.0-draft providers. -->

## License

By submitting this pull request, I agree to dual-license this contribution
under **MIT OR Apache-2.0**, as certified by my DCO sign-off.

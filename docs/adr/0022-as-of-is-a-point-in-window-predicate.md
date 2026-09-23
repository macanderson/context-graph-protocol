# 22. `as_of` is a point-in-window predicate on valid time

- Status: accepted
- Date: 2026-09-23
- Issue: #184. Follows the precedent `SPEC.md` §5.1 set for `kinds` (Q1).

## Context

`ContextQuery.as_of` shipped with a format rule (F4) and one clause of prose in
`SPEC.md` §6.1: "`as_of` pins retrieval to an instant." No **MUST**, no
**SHOULD**, no anchor.

The conformance suite enforced a rule anyway. `check_as_of` failed a provider
for returning a frame whose `valid_from` postdated the pin, and cited §6.1 as
its authority. It was the only check in the suite that could not name a numbered
requirement, because there was none. A provider implemented from the spec
alone — which §1 says is sufficient — could fail it without breaking anything
published.

It also enforced half of the window §6.1 describes. A frame whose `valid_to` was
years before the pin, a fact that had stopped being true, came back for a pinned
query and the check reported "none of the returned frame(s) is dated after the
pin". The half that keeps stale facts out was unchecked, and the half that keeps
premature facts out was enforced without being specified.

README listed **Temporal validity** among the seven guarantees, enforced by
"`ContextFrame` temporal fields". Those fields carry F4, which constrains their
format. So the guarantee on offer was that `as_of` is well-formed, not that it
does anything.

Writing the requirement meant deciding four things: its strength, which fields
it binds, how it relates to `recorded_at`, and what a provider that cannot serve
a pinned query replies.

## Decision

**1. Q2 is a MUST over the valid-time window, half-open.** When `as_of` is
present, every returned frame satisfies `valid_from <= as_of` if it carries
`valid_from`, and `as_of < valid_to` if it carries `valid_to`. An absent bound is
unbounded on that side. A frame with neither bound makes no temporal claim and is
eligible. The anchor is `Q2` in `SPEC.md` §5, beside Q1, with the rationale in
§5.3.

Half-open, `[valid_from, valid_to)`, because it is the only convention under
which a fact and the fact that superseded it, sharing a boundary instant, never
both answer for that instant. It is the convention of SQL:2011 application-time
periods and of every bitemporal store we would expect an implementer to have
met.

**2. Instants compare as instants.** F4 permits an optional fractional part, so
`…:00.5Z` sorts before `…:00Z` as a string and `…:00Z` and `…:00.0Z` spell one
instant two ways. Q2 says "compared as instants", and the check does exactly
that (`compare_instants`), comparing the fixed-width seconds prefix
lexicographically and the fraction as a decimal.

**3. `as_of` pins valid time only.** §6.1 already distinguishes when content was
*true* (`valid_from`/`valid_to`) from when the provider *learned* it
(`recorded_at`). A historical query asks what was true then as best the provider
knows now, and a fact recorded after the pin answers that correctly. "What did
the provider believe at that instant" is the transaction-time question, and it
gets its own field in a `1.x` minor if a consumer needs it, not a second meaning
for this one.

**4. No capability flag and no error code.** The predicate ranges over fields
the provider itself emits and is applied to frames it was already going to
return, so every provider can comply. A provider without a temporal index
serves undated frames, which the predicate admits, and a host that needs dated
evidence filters on the fields' presence. Following Q1, a provider with nothing
valid at the pin returns zero frames.

**5. An absent `as_of` constrains nothing.** Whether an unpinned query answers
with a current view or with history is provider-private, like ranking.

**6. The check probes both halves, at two pins.** One pin cannot give both halves
observable work against a provider with content on both sides of a boundary.
`check_as_of` now pins `2026-07-01` and `2026-10-01`, and the reference fixture
serves one frame valid over `[2026-01-01, 2026-09-01)` and one valid from
`2026-09-01`. At the first pin an honest answer omits the second frame. At the
second it omits the first. `ignore-as-of` skips the filter and trips the first.
`ignore-valid-to` applies only the `valid_from` half, which is the obvious first
implementation, and trips the second. The probe moved into its own module
(`contextgraph-conformance/src/as_of.rs`) rather than growing `lib.rs`.

## Consequences

- A provider that filtered on `valid_from` alone, and serves frames with a
  `valid_to`, now fails `as-of-temporal`. No provider in this repository or the
  three SDKs served a `valid_to`, so none turned red. The reference provider
  (`contextgraph-refprov`) and the TypeScript and Python example providers now
  apply the whole predicate anyway, so an author copying them does not copy the
  bug.
- The check never fails a provider for returning fewer frames. Q2 says which
  frames are eligible, never how many must come back.
- README's Temporal validity row points at Q2 and the check rather than at the
  fields.
- The reference fixture's first frame now carries a `valid_to`. It is not in the
  §6.5.2 commitment preimage, so no attestation changes.

## Alternatives considered

**SHOULD, with a `capabilities.as_of` flag.** This is where a provider with no
temporal index seemed to point. But nothing a provider emits would have to
change to satisfy the MUST, so the flag would gate nothing. It would also be the
dead capability surface ADR 0004 removed: a declaration no host behaviour
depends on.

**A not-yet-true filter only** (`valid_from` alone). This is what the check
enforced. It lets every historical query return facts that had already been
superseded, and superseded facts are the ones a long-running agent is most
likely to be holding. §6.1 already calls the fields a window. Specifying half
of it would have written the defect into the spec.

**A closed window, `[valid_from, valid_to]`.** It admits both a fact and its
successor at the boundary instant, which is exactly the ambiguity a pin exists
to remove.

**An `unsupported_as_of` error code.** `unsupported_kind` exists because a
provider may serve no frames of a requested kind and the host benefits from
knowing that the request, rather than the corpus, was the problem. Under Q2
there is no request a provider cannot honour, so the code would never have a
correct use.

**Pin `recorded_at` too.** It would give `as_of` two meanings at once and make
every historical query drop facts learned after the pin, which is the opposite
of what a historical query wants.

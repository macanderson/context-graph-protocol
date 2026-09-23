# 27. An attestation the suite cannot check is not certified

- Status: accepted
- Date: 2026-09-23
- Decides the open question in #159. Builds on
  [ADR 0010](0010-provenance-attestation.md) and
  [ADR 0019](0019-one-home-for-an-attestation.md). Changes no wire format and
  no normative requirement.

## Context

`SPEC.md` §6.5 F8 says a verifier that does not recognise an attestation's
`algorithm` **MUST** report it as uncheckable and **MUST NOT** treat the frame as
attested: "I cannot check this" is never "this is good". PR #154 added the
`attestation` conformance check and five `--misbehave` modes for F6, F7 and F9.
F8's path through that check was written and exercised by nothing. When every
attestation a provider served named an unknown scheme, the check reported
`Skipped`.

Three consumers read that report, and they disagreed about a skip:

| Consumer | A `Skipped` check is |
|---|---|
| `ConformanceReport::passed()` | a pass |
| `.github/scripts/conformance-green.sh`, `conformance-external.sh` | a failure |
| `.github/scripts/conformance-red.sh` | a catch (it counted anything not `pass`) |

So a provider that wrote `"algorithm": "magic"` on every attestation was
conformant to the library and not conformant to CI. Nothing had ever produced
that input, so the disagreement had never shown.

F8 binds the **verifier**. It says what a verifier must not conclude. It does not
say what a conformance verdict should conclude about the provider that served
the attestation. That question is the one this ADR answers.

## Decision

**1. An attestation the suite cannot check fails the `attestation` check. It
does not skip.** This holds whether every attestation is uncheckable or only
some are.

**2. The evidence says *uncheckable*, never *invalid*.** The failure names the
verdict, `UnknownAlgorithm`, and the scheme. It is reported apart from the
§6.5.4 failures (`BadSignature`, `CommitmentMismatch`, and the rest), so a
provider that is ahead of this build learns what it is waiting for, a suite that
knows its scheme, and is not told it forged anything. F9 is still checked: the
frames must still be served, degraded to unattested.

**3. `Skipped` means "does not apply", never "could not decide".** A skip passes
in `ConformanceReport::passed()` because a check that does not bind the declared
capability set has nothing to certify. Once a check applies, it must reach a
verdict. If it cannot, it fails and gives the reason. This is written on
`CheckStatus::Skipped` and on `passed()`.

**4. The three consumers agree, by statement and not by accident.**

- `conformance-red.sh` counts a mode as caught only by a check whose status is
  `fail`. A skip is not a catch.
- `conformance-green.sh` and `conformance-external.sh` keep their stricter bar:
  every check must `pass`. The providers they judge declare every capability the
  suite probes, so a skip there can only mean a check stopped running. Each
  script now says so in its header.
- `passed()` is unchanged. The uncheckable case is a `fail` in the report
  itself, so all three give it the same answer.

**5. `--misbehave unknown-algorithm` is the witness.** It signs every frame
honestly and then relabels each attestation `dilithium3`. The commitment and
the signature bytes are the honest ones, so the verdict can only come from F8.
A test asserts that the check fails, that it alone fails, that the evidence
names `UnknownAlgorithm` and `dilithium3` and none of the forgery verdicts, and
that the JSON status CI reads is `fail`.

## Why fail, and not skip

The case for skip is real. `CheckStatus::Skipped` already means "not applicable
here", and a provider on a post-quantum scheme is not broken. It is only
uncertified by this build. Three things outweigh it:

- **A skip is a pass.** "Context Graph Protocol conformant" means green on this
  suite for your declared capability set. A provider that publishes keys and
  serves signatures has declared the capability this check exists for. Passing
  it on signatures the suite never checked is the self-attestation that §11.1
  exists to rule out.
- **A skip is an opt-out.** If an unknown scheme skipped, relabelling a forged
  signature `"magic"` would turn a `BadSignature` failure into a certified
  provider. The same applies to a mixed answer: if one honest frame could make
  the check pass with a note, it would launder every frame served beside it.
- **The probe already took this position next door.** A published key with no
  attestation fails, and so does an attestation with no published key. Both are
  signing claims nothing here can exercise. An attestation in an unknown scheme
  is the third member of that set.

The cost falls on a provider that adopts a new scheme before the suite does. It
stays uncertified until the suite learns the scheme, and the evidence says so in
exactly those terms. That is the right cost. Learning a scheme is an additive
change to the suite, and certifying without checking cannot be undone once
somebody has relied on it.

## Consequences

- `attestation_stdio_probe` has no skip branch for uncheckable attestations,
  and no pass-with-note for a mixed answer. Its doc comment carries the
  reasoning above.
- Every mode `conformance-red.sh` discovers is caught by a `fail` from a check
  it names, and none by a skip. This was verified when the rule changed.
- No provider CI judges today signs in an unknown scheme. The SDK examples and
  the reference providers publish no key, so the check passes for them, and
  `contextgraph-example-docs` signs with Ed25519. No green job changes.
- When the suite learns a new algorithm, a provider using it moves from `fail`
  to `pass` with no change on its side.

## Alternatives considered

**Keep the skip and make `passed()` treat an attestation skip as a failure.**
That spends one status on two meanings and makes `passed()` special-case a
single check by name. The next check with the same shape would need the same
exception. Keeping "does not apply" and "could not decide" separate fixes the
cause.

**Add a fourth status, `Uncheckable`.** It is honest, but it widens the
serialized report that CI and third-party harnesses parse, and it still has to
answer the same question: is it a pass or a failure? Once the answer is
"failure", a fourth status only restates `fail` with evidence attached, and
`fail` already has evidence.

**Fail only when every attestation is uncheckable.** This is the narrowest
reading of the issue, and it leaves the laundering hole described above.

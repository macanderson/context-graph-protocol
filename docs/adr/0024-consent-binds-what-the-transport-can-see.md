# 24. Consent binds what the transport can see: C3 is a MUST, stdio confinement is the deployment's

- Status: accepted
- Date: 2026-09-23
- Issue: #187.

## Context

README listed **Consent enforcement** among the seven guarantees, stated without
conditions: "A provider that sends data off-machine is never queried until you
record named, revocable consent."

The spec was narrower. C3, "a provider **SHOULD** declare `egress: true`
honestly", was marked advisory. C4 made the HTTP transport treat every
non-loopback provider as egress regardless of its claim, and §4 said why: "C3
is a claim … The transport overrides the declaration because the transport
*knows*."

That reasoning implies its converse, and nobody had written it down. Over stdio
the transport does not know. A stdio provider is a child process with its own
sockets, and nothing on the NDJSON pipe reveals whether it opens one. stdio is
the primary binding, used by every reference provider and all four SDKs. On
stdio, a provider that declares `egress: false` and sends workspace content to
the internet is queried without consent and never detected. §11.1 lists what
the suite "genuinely" cannot check, and it did not list this hole, which is
larger than the three it did list.

The reference host also differed from C4 in the other direction, and that was
undocumented. `HttpProvider` forces `egress: true` for every HTTP provider,
loopback included. C4 exempts loopback.

Three decisions were needed: what the guarantee claims, how strong C3 is, and
whether the reference host takes on network confinement for child processes.

## Decision

**1. The guarantee claims what is enforced, per transport.** README,
`docs/overview.md`, `docs/protocol-advantages.md` and
`docs/implementing-a-provider.md` now say the following. A provider that
declares egress, and every provider reached over HTTP whatever it declares, is
never queried without consent. Over stdio, `egress: false` is a declaration the
host trusts and cannot observe. `SPEC.md` §4.3 sets this out case by case, and
§11.1 lists the stdio egress hole beside the HTTP transport rules.

**2. C3 is a MUST.** Whether a requirement is strong and whether it can be
checked are separate questions. A dishonest `egress: false` can no more be
checked under a MUST than under a SHOULD. The MUST makes it a conformance
violation, which a deployment, a registry, or a contract can act on. That is how
every unverifiable but load-bearing declaration is handled, and A2's `cited` is
the precedent inside this specification. The new text also names relaying
through a process, service, or proxy as "indirectly", because a local proxy is
the obvious way to launder egress. Under the SHOULD, exfiltration behind an
honest-looking handshake was merely discouraged.

**3. The reference host does not confine stdio children's network. That is the
deployment's job, and the host provides the seam.** Network confinement is
platform-specific: network namespaces or seccomp on Linux, a sandbox profile on
macOS (whose tooling Apple has deprecated), AppContainer on Windows. Doing it in
the reference host would give a guarantee that holds on some platforms and
silently not on others, which is the same overstatement this ADR removes from
README. What the host provides instead composes with every mechanism.
`Host::add_stdio` spawns whatever program it is given, under the scrubbed
environment `stdio.rs` already applies, so an operator passes `bwrap
--unshare-net`, `unshare -n`, `sandbox-exec -p …`, or `docker run --network
none` as the program. A confined child makes C4's argument true for stdio again,
because the transport knows. If a deployment asks for built-in confinement
later, it can arrive as an opt-in spawn policy on the same seam without changing
the protocol.

**4. Loopback HTTP is egress in the reference host, deliberately.** A loopback
listener is a process of unknown provenance that can relay every query onward,
and a local proxy is the cheapest way to walk a remote provider past C4's
loopback exemption. C4 stays the floor a conformant host must meet. The
reference host's stricter behaviour is policy. The code comment in `http.rs`
and the `http_transport_forces_egress_even_when_the_remote_claims_local` test,
which now asserts that it runs against loopback, both say so, so the behaviour
is not read as a bug and relaxed later.

**5. The gap has a witness.** `contextgraph-host/tests/stdio_egress_gap.rs`
spawns a stdio child that declares `egress: false`, then opens a TCP connection
and writes the query line to it before answering. The test asserts that the
host requires no consent, raises no error, and that the payload arrives. It
pins today's behaviour so that any later fix, confinement or detection, has a
fail→pass case to prove against.

## Consequences

- README no longer offers a guarantee the protocol cannot keep, and the
  seven-guarantees table points at §4.3.
- A provider that was already honest is unaffected by C3 becoming a MUST. One
  that under-declares was never conformant in spirit, and is now non-conformant
  in letter too.
- Operators get a concrete instruction for untrusted stdio providers instead of
  an implied promise.
- The witness test depends on bash's `/dev/tcp`, like the other stdio fixtures,
  and is Unix-only.

## Alternatives considered

**Keep C3 a SHOULD, because it cannot be checked.** This is the §6.6 argument
against mandating score calibration. It does not carry over. Calibration has no
agreed meaning to comply with, so a MUST there would be vacuous. Egress has a
precise one (did bytes leave the machine?), and the only thing missing is an
observer.

**Sandbox stdio children in the reference host.** This is the strongest
guarantee on paper. It would be platform-partial in practice, and every host
built on `contextgraph-host` would inherit a security boundary it does not
control and cannot audit. The deployment layer, whether containers, OS
sandboxes or MDM profiles, already owns this, does it better, and can be
composed with the host today through `Host::add_stdio`.

**Make `egress` attestable.** A signature proves who made a claim, not that the
claim is true. An attested `egress: false` from a dishonest provider is still
false. This would add machinery without closing the hole.

**Relax the reference host's loopback handling to match C4.** It would align the
code with the letter of the floor and reopen the local-proxy route. The floor
exists so that a host is not required to be stricter. It does not require a
host to be laxer.

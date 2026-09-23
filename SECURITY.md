# Security Policy

Context Graph Protocol is a protocol whose central promise is data-flow accountability — that
workspace content does not leave your machine without recorded, named consent.
Reporting a vulnerability responsibly keeps that promise credible.

## Reporting a vulnerability

**Do not open a public GitHub issue for a security vulnerability.**

Please report it privately:

1. Open a **private security advisory** via GitHub's "Report a vulnerability"
   flow on this repository (Security tab → "Report a vulnerability"), **or**
2. Email the maintainer at **macanderson@users.noreply.github.com** with the
   subject `Context Graph Protocol security: <short summary>`.

Include as much of the following as you can:

- A description of the issue and its security impact.
- The Context Graph Protocol crate(s) and version(s) affected. Every crate this
  repository publishes to crates.io is covered: `contextgraph-types`,
  `contextgraph-host`, `contextgraph-conformance`, and `contextgraph-trace`.
  `contextgraph-trace`'s journal format is sketch stage
  ([`docs/host-trace.md`](./docs/host-trace.md)); that changes what counts as a
  breaking change to it, not whether a vulnerability in it is fixed.
- The protocol version (e.g. `contextgraph/1.0`).
- A minimal repro: a malformed envelope, a misbehaving provider, or a
  bypassed consent gate.
- Any mitigations you have identified.

## What is in scope

- Bypass of the **consent gate** — an `egress: true` (or remote) provider
  receiving a query payload before consent is recorded.
- **Budget-honesty** evasion — a provider whose frames exceed `max_tokens`
  undetected by a conforming host.
- **Untrusted-data** handling — frame content treated as instructions by a
  conforming host (prompt-injection by way of the wire protocol).
- Wire-level **denial of service** against `contextgraph-host`'s stdio or HTTP
  transports (a malformed line or oversized envelope crashing or hanging the
  host).
- Forgery or stripping of **provenance** digests in a way a conforming host
  fails to detect.

## What is out of scope

- Vulnerabilities in a specific third-party Context Graph Protocol provider (report those to the
  provider, not here).
- Issues that require the user to already run untrusted code (the protocol
  cannot prevent a compromised host process).
- Theoretical cost overruns a host already detects and drops loudly.

## Response timeline

We aim to acknowledge a report within **5 business days** and to publish a
fix and advisory within **90 days**, coordinating disclosure with the reporter.
A fix lands as a patch release of the affected crate, with a GitHub Security
Advisory and, where applicable, a CVE.

## Supported versions

The project has two version axes, and they are stated separately
([`docs/stability.md`](./docs/stability.md) explains why they differ):

- **Wire protocol:** `contextgraph/1.0`, frozen on 2026-08-11
  ([`GOVERNANCE.md`](./GOVERNANCE.md)). Changes within the `contextgraph/1`
  family are additive only. Every supported crate release speaks it.
- **Crates:** the published crates share one version, currently `2.x`.

Security fixes land as a patch release on the **latest published crate major
line**, today `2.x`, and nowhere else:

| Crate line | Receives security fixes |
| --- | --- |
| `2.x` | Yes — the latest `2.x` release. |
| `0.1.x` | No. These predate the freeze; upgrade to `2.x` ([`MIGRATION.md`](./MIGRATION.md)). |

No `1.x` crate was ever published to crates.io, so there is no `1.x` line to
support. When a future crate major ships, fixes move to it; whether the
previous major also receives them will be stated here at that time.

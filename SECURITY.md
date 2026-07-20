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
- The Context Graph Protocol crate(s) and version(s) affected (`contextgraph-types`, `contextgraph-host`,
  `contextgraph-conformance`).
- The protocol version (e.g. `contextgraph/1.0-draft`).
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

Context Graph Protocol is pre-1.0 (`contextgraph/1.0-draft`). Only the latest published crate release
receives security fixes until the `contextgraph/1.0` freeze.

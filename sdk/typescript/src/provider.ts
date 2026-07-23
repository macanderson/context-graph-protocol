/**
 * The provider runtime: implement {@link Provider}, hand it to
 * {@link runStdioProvider}, and you have a conformant Context Graph Protocol
 * provider speaking the line-oriented JSON wire over stdio.
 *
 * The runtime handles the whole lifecycle a host drives — handshake, query
 * (echoing the correlation `id`), verify, shutdown — and, crucially, stays
 * alive and replies with a typed error on a malformed line rather than
 * crashing (the `malformed-input-tolerance` guarantee).
 */
import * as readline from "node:readline";

import {
  type Capabilities,
  type ContextQuery,
  type ContextQueryResult,
  type Envelope,
  type ProviderInfo,
  type VerifyRequest,
  type VerifyResponse,
  PROTOCOL_VERSION,
} from "./types.js";

/**
 * A Context Graph Protocol provider. `query` is mandatory; `verify` is optional
 * (a provider that omits it is treated as unable to vouch for its frames, and
 * the host re-queries). Both may be sync or async.
 */
export interface Provider {
  /** Identity and data-flow posture, reported at handshake. */
  info(): ProviderInfo;
  /** What this provider can do, negotiated at handshake. */
  capabilities(): Capabilities;
  /** Answer a retrieval request with budgeted, provenance-carrying frames. */
  query(query: ContextQuery): ContextQueryResult | Promise<ContextQueryResult>;
  /** Revalidate frames a host already holds (identities only — never bodies). */
  verify?(request: VerifyRequest): VerifyResponse | Promise<VerifyResponse>;
}

function writeEnvelope(envelope: Envelope): void {
  process.stdout.write(`${JSON.stringify(envelope)}\n`);
}

/**
 * Run `provider` as a stdio child process, the shape the reference host and the
 * conformance suite drive. Reads one envelope per line from stdin and writes
 * one envelope per line to stdout until a `shutdown` (or EOF).
 */
export function runStdioProvider(provider: Provider): void {
  const rl = readline.createInterface({ input: process.stdin, terminal: false });

  // Serialize line handling: readline emits buffered lines synchronously in a
  // single tick, but `handleLine` is async (a `query`/`verify` handler suspends
  // at its `await` before writing its reply). Chaining each line's handler
  // after the previous one's promise ensures a later `shutdown` line only runs
  // — and calls `process.exit(0)` — after all prior replies have been written,
  // instead of racing ahead of their pending microtasks (which pipelined hosts
  // rely on, since this SDK advertises `correlation: true`).
  let queue: Promise<void> = Promise.resolve();
  rl.on("line", (line: string) => {
    // Keep the chain alive even if a handler rejects, so a single failing line
    // never drops the replies for lines that follow it.
    queue = queue.then(() => handleLine(provider, line)).catch(() => {});
  });
  // EOF / a broken pipe means the host is gone; exit cleanly.
  rl.on("close", () => process.exit(0));
}

async function handleLine(provider: Provider, line: string): Promise<void> {
  const trimmed = line.trim();
  if (trimmed.length === 0) return;

  let envelope: Envelope;
  try {
    envelope = JSON.parse(trimmed) as Envelope;
  } catch {
    // A malformed line: a robust provider stays alive and says so with a code
    // rather than crashing (the `malformed-input-tolerance` guarantee).
    writeEnvelope({
      type: "error",
      code: "bad_request",
      message: "line was not a valid CGP envelope",
    });
    return;
  }

  switch (envelope.type) {
    case "handshake":
      writeEnvelope({
        type: "handshake_ack",
        protocol_version: PROTOCOL_VERSION,
        provider: provider.info(),
        capabilities: provider.capabilities(),
      });
      break;

    case "query": {
      const result = await provider.query(envelope.query);
      // Echo the correlation id so the host can match reply to request (H4).
      const reply: Envelope = { type: "frames", result };
      if (envelope.id !== undefined) reply.id = envelope.id;
      writeEnvelope(reply);
      break;
    }

    case "verify": {
      const response: VerifyResponse = provider.verify
        ? await provider.verify(envelope.request)
        : {
            // No verify support ⇒ vouch for nothing; the host re-queries.
            verdicts: envelope.request.frames.map((frame) => ({
              frame,
              status: "unknown" as const,
            })),
          };
      writeEnvelope({ type: "verified", response });
      break;
    }

    case "shutdown":
      process.exit(0);
      break;

    default:
      // handshake_ack / frames / verified / error are host→provider-invalid
      // inputs; a provider ignores them.
      break;
  }
}
